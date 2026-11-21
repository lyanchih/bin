mod coap;
mod http;
mod mqtt;

use crate::model::Report;
use flate2::{Compression, write::GzEncoder};
use std::future::Future;
use std::io::{self, Write};
use std::time::Duration;
use url::Url;

const DELIVERY_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum ContentType {
    #[default]
    Json,
    Cbor,
    JsonGzip,
    CborGzip,
}

impl ContentType {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "json" => Ok(Self::Json),
            "cbor" => Ok(Self::Cbor),
            "json-gzip" => Ok(Self::JsonGzip),
            "cbor-gzip" => Ok(Self::CborGzip),
            _ => Err("invalid content type, expected json, cbor, json-gzip, or cbor-gzip".into()),
        }
    }

    fn media_type(self) -> &'static str {
        match self {
            Self::Json | Self::JsonGzip => "application/json",
            Self::Cbor | Self::CborGzip => "application/cbor",
        }
    }

    fn is_gzip(self) -> bool {
        matches!(self, Self::JsonGzip | Self::CborGzip)
    }

    pub fn encode(self, report: &Report) -> Result<Vec<u8>, String> {
        if self.is_gzip() {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            self.serialize(report, &mut encoder)?;
            encoder.finish().map_err(|error| error.to_string())
        } else {
            let mut body = Vec::new();
            self.serialize(report, &mut body)?;
            Ok(body)
        }
    }

    fn serialize(self, report: &Report, writer: impl Write) -> Result<(), String> {
        match self {
            Self::Json | Self::JsonGzip => {
                serde_json::to_writer(writer, report).map_err(|error| error.to_string())
            }
            Self::Cbor | Self::CborGzip => {
                ciborium::into_writer(report, writer).map_err(|error| error.to_string())
            }
        }
    }
}

pub enum Destination {
    Http(http::Endpoint),
    Coap(Url),
    Mqtt(mqtt::Endpoint),
    Stdout,
}

impl Destination {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value == "stdout" {
            return Ok(Self::Stdout);
        }

        let url = Url::parse(value).map_err(|error| format!("invalid protocol URI: {error}"))?;
        if url.host().is_none() || url.port() == Some(0) || url.fragment().is_some() {
            return Err("protocol URI requires a host and a valid port, with no fragment".into());
        }
        match url.scheme() {
            "http" | "https" => http::Endpoint::parse(url).map(Self::Http),
            "coap" if url.username().is_empty() && url.password().is_none() => Ok(Self::Coap(url)),
            "coap" => Err("CoAP URI does not support username or password".into()),
            "mqtt" => mqtt::Endpoint::parse(&url).map(Self::Mqtt),
            _ => Err("expected an http://, https://, coap://, or mqtt:// URI, or stdout".into()),
        }
    }

    pub fn protocol(&self) -> &'static str {
        match self {
            Self::Http(endpoint) => endpoint.scheme(),
            Self::Coap(_) => "coap",
            Self::Mqtt(_) => "mqtt",
            Self::Stdout => "stdout",
        }
    }

    pub fn deliver(self, content_type: ContentType, body: &[u8]) -> Result<(), String> {
        match self {
            Self::Http(endpoint) => endpoint.send(content_type, body),
            Self::Coap(url) => run_async("CoAP", coap::send(url, content_type, body.to_vec())),
            Self::Mqtt(endpoint) => {
                run_async("MQTT", mqtt::send(endpoint, content_type, body.to_vec()))
            }
            Self::Stdout => {
                let mut output = io::stdout().lock();
                (|| -> io::Result<()> {
                    output.write_all(body)?;
                    if content_type == ContentType::Json {
                        output.write_all(b"\n")?;
                    }
                    output.flush()
                })()
                .map_err(|error| error.to_string())
            }
        }
    }
}

fn run_async(
    protocol: &str,
    delivery: impl Future<Output = Result<(), String>>,
) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("{protocol} runtime: {error}"))?;
    let result = runtime.block_on(async {
        tokio::time::timeout(DELIVERY_TIMEOUT, delivery)
            .await
            .map_err(|_| format!("{protocol} delivery timed out"))?
    });
    runtime.shutdown_background();
    result
}
