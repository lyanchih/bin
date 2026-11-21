use super::ContentType;
use coap::UdpCoAPClient;
use coap_lite::{CoapOption, CoapRequest, MessageType, RequestType, option_value::OptionValueU16};
use percent_encoding::percent_decode_str;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use url::{Host, Url};

const DEFAULT_PORT: u16 = 5683;

pub async fn send(url: Url, content_type: ContentType, body: Vec<u8>) -> Result<(), String> {
    let port = url.port().unwrap_or(DEFAULT_PORT);
    let peer = match url.host() {
        Some(Host::Ipv4(ip)) => SocketAddr::new(ip.into(), port),
        Some(Host::Ipv6(ip)) => SocketAddr::new(ip.into(), port),
        Some(Host::Domain(host)) => tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| format!("CoAP address lookup failed: {error}"))?
            .next()
            .ok_or("CoAP address lookup returned no addresses")?,
        None => return Err("CoAP endpoint is missing a host".into()),
    };
    let source = SocketAddr::new(
        if peer.is_ipv4() {
            Ipv4Addr::UNSPECIFIED.into()
        } else {
            Ipv6Addr::UNSPECIFIED.into()
        },
        0,
    );
    let client = UdpCoAPClient::new_with_specific_source(source, peer)
        .await
        .map_err(|error| format!("CoAP socket failed: {error}"))?;

    let mut request = CoapRequest::new();
    request.set_method(RequestType::Post);
    let packet = &mut request.message;
    packet.header.set_type(MessageType::Confirmable);
    let mut token = [0; 8];
    getrandom::fill(&mut token)
        .map_err(|error| format!("could not generate a CoAP token: {error}"))?;
    packet.set_token(token.to_vec());
    if let Some(Host::Domain(host)) = url.host()
        && host.parse::<Ipv4Addr>().is_err()
    {
        packet.add_option(CoapOption::UriHost, host.as_bytes().to_vec());
    }
    if port != DEFAULT_PORT {
        packet.add_option_as(CoapOption::UriPort, OptionValueU16(port));
    }
    if url.path() != "/"
        && let Some(segments) = url.path_segments()
    {
        for segment in segments {
            packet.add_option(CoapOption::UriPath, percent_decode_str(segment).collect());
        }
    }
    if let Some(query) = url.query() {
        for argument in query.split('&') {
            packet.add_option(CoapOption::UriQuery, percent_decode_str(argument).collect());
        }
    }
    let format = match content_type {
        ContentType::Json => 50,
        ContentType::Cbor => 60,
        // Experimental CoAP Content-Formats. Receivers must use the same IDs.
        ContentType::JsonGzip => 65000,
        ContentType::CborGzip => 65001,
    };
    packet.add_option_as(CoapOption::ContentFormat, OptionValueU16(format));
    packet.payload = body;

    let response = client
        .send(request)
        .await
        .map_err(|error| format!("CoAP delivery failed: {error}"))?;
    let code = response.message.header.get_code();
    if code.starts_with("2.") && code != "2.31" {
        Ok(())
    } else {
        Err(format!("CoAP delivery returned {code}"))
    }
}
