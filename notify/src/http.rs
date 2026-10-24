use std::fmt::Write;
use std::io::Read;
use std::time::Duration;

pub fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout_secs: u64,
) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(timeout_secs))
        .build();

    let mut req = agent.request(method, url);
    for &(k, v) in headers {
        req = req.set(k, v);
    }

    let result = match body {
        Some(b) => req.send_string(b),
        None => req.call(),
    };

    match result {
        Ok(resp) => resp
            .into_string()
            .map_err(|e| format!("讀取回應內容失敗 ({url}): {e}")),
        Err(ureq::Error::Status(code, resp)) => {
            let mut err_body = String::new();
            let _ = resp.into_reader().take(240).read_to_string(&mut err_body);
            Err(format!("HTTP {code}: {}", err_body.trim()))
        }
        Err(ureq::Error::Transport(t)) => Err(format!("網路連線失敗 ({url}): {t}")),
    }
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
