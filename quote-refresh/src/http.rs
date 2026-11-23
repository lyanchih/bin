use std::fmt::Write;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::Agent;

fn shared_agent(timeout_secs: u64) -> Agent {
    static TLS_CFG: OnceLock<TlsConfig> = OnceLock::new();
    let tls_cfg = TLS_CFG
        .get_or_init(|| {
            TlsConfig::builder()
                .provider(TlsProvider::Rustls)
                .root_certs(RootCerts::WebPki)
                .unversioned_rustls_crypto_provider(Arc::new(rustls_rustcrypto::provider()))
                .build()
        })
        .clone();

    Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .tls_config(tls_cfg)
        .build()
        .into()
}

pub fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout_secs: u64,
) -> Result<String, String> {
    let agent = shared_agent(timeout_secs);

    let response = match (method, body) {
        ("POST", Some(b)) => {
            let mut req = agent.post(url);
            for &(k, v) in headers {
                req = req.header(k, v);
            }
            req.send(b)
        }
        ("POST", None) => {
            let mut req = agent.post(url);
            for &(k, v) in headers {
                req = req.header(k, v);
            }
            req.send_empty()
        }
        _ => {
            let mut req = agent.get(url);
            for &(k, v) in headers {
                req = req.header(k, v);
            }
            req.call()
        }
    };

    let mut resp = response.map_err(|e| format!("網路連線失敗 ({url}): {e}"))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("讀取回應內容失敗 ({url}): {e}"))?;

    if !(200..300).contains(&status) {
        let preview: String = text.chars().take(240).collect();
        return Err(format!("HTTP {status}: {}", preview.trim()));
    }

    Ok(text)
}

pub fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}
