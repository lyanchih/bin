use super::ContentType;
use percent_encoding::percent_decode_str;
use rumqttc::Outgoing;
use rumqttc::v5::mqttbytes::{
    QoS,
    v5::{Packet, PubAckReason, PublishProperties},
};
use rumqttc::v5::{AsyncClient, ConnectionError, Event, MqttOptions, StateError};
use url::{Host, Url};

pub struct Endpoint {
    host: String,
    port: u16,
    topic: String,
    credentials: Option<(String, String)>,
}

impl Endpoint {
    pub fn parse(url: &Url) -> Result<Self, String> {
        let host = match url.host() {
            Some(Host::Domain(host)) => host.into(),
            Some(Host::Ipv4(host)) => host.to_string(),
            Some(Host::Ipv6(host)) => host.to_string(),
            None => return Err("MQTT endpoint is missing a host".into()),
        };
        if url.query().is_some() {
            return Err("MQTT endpoint does not accept a query string".into());
        }
        let path = url.path().strip_prefix('/').unwrap_or(url.path());
        let topic = decode_field(path, "topic")?;
        if topic.is_empty() || topic.contains(['#', '+']) {
            return Err("MQTT endpoint requires a topic without wildcards".into());
        }
        let username = decode_field(url.username(), "username")?;
        let password = url
            .password()
            .map(|value| decode_field(value, "password"))
            .transpose()?;
        let credentials = if username.is_empty() && password.is_none() {
            None
        } else {
            Some((username, password.unwrap_or_default()))
        };
        Ok(Self {
            host,
            port: url.port().unwrap_or(1883),
            topic,
            credentials,
        })
    }
}

fn decode_field(value: &str, field: &str) -> Result<String, String> {
    let value = percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| format!("invalid UTF-8 in MQTT {field}"))?;
    if value.contains('\0') || value.len() > usize::from(u16::MAX) {
        return Err(format!("invalid MQTT {field}"));
    }
    Ok(value.into_owned())
}

pub async fn send(
    endpoint: Endpoint,
    content_type: ContentType,
    body: Vec<u8>,
) -> Result<(), String> {
    let mut random = [0; 8];
    getrandom::fill(&mut random)
        .map_err(|error| format!("could not generate an MQTT client ID: {error}"))?;
    let id = format!("sysinfo{:016x}", u64::from_ne_bytes(random));
    let mut options = MqttOptions::new(id, endpoint.host, endpoint.port);
    if let Some((username, password)) = endpoint.credentials {
        options.set_credentials(username, password);
    }
    let (client, mut event_loop) = AsyncClient::new(options, 2);
    let mut properties = PublishProperties {
        content_type: Some(content_type.media_type().into()),
        payload_format_indicator: Some(u8::from(content_type == ContentType::Json)),
        ..PublishProperties::default()
    };
    if content_type.is_gzip() {
        properties
            .user_properties
            .push(("content-encoding".into(), "gzip".into()));
    }
    client
        .try_publish_with_properties(endpoint.topic, QoS::AtLeastOnce, false, body, properties)
        .map_err(|_| "could not queue MQTT publish")?;

    let mut publish_id = None;
    loop {
        let event = match event_loop.poll().await {
            Ok(event) => event,
            Err(error @ ConnectionError::MqttState(StateError::ConnectionAborted)) => {
                // The batched read can queue PUBACK before observing EOF.
                event_loop
                    .state
                    .events
                    .drain(..)
                    .find(|event| {
                        matches!(event, Event::Incoming(Packet::PubAck(ack))
                            if publish_id == Some(ack.pkid))
                    })
                    .ok_or_else(|| format!("MQTT delivery failed: {error}"))?
            }
            Err(error) => return Err(format!("MQTT delivery failed: {error}")),
        };
        match event {
            Event::Outgoing(Outgoing::Publish(id)) => publish_id = Some(id),
            Event::Incoming(Packet::PubAck(ack)) if publish_id == Some(ack.pkid) => {
                return match ack.reason {
                    PubAckReason::Success | PubAckReason::NoMatchingSubscribers => Ok(()),
                    reason => Err(format!("MQTT broker rejected publish: {reason:?}")),
                };
            }
            _ => {}
        }
    }
}
