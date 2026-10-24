use std::fmt::Write;
use std::process::Command;

pub fn http_request(
    method: &str,
    url: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
    timeout_secs: u64,
) -> Result<String, String> {
    let mut cmd = Command::new("curl");
    cmd.arg("-sS")
        .arg("-L")
        .arg("-m")
        .arg(timeout_secs.to_string())
        .arg("-X")
        .arg(method);

    for (k, v) in headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }

    if let Some(b) = body {
        cmd.arg("--data-raw").arg(b);
    }

    cmd.arg("-w").arg("\n__HTTP_STATUS__:%{http_code}").arg(url);

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to execute curl for {url}: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "curl command failed ({}): {}",
            output.status,
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let marker = "\n__HTTP_STATUS__:";
    let Some(idx) = stdout.rfind(marker) else {
        return Err("Missing HTTP status marker from curl output".to_string());
    };

    let response_body = &stdout[..idx];
    let status_str = stdout[idx + marker.len()..].trim();
    let status_code: u16 = status_str
        .parse()
        .map_err(|_| format!("Invalid HTTP status code '{status_str}'"))?;

    if !(200..300).contains(&status_code) {
        let preview: String = response_body.chars().take(240).collect();
        return Err(format!("HTTP {status_code}: {}", preview.trim()));
    }

    Ok(response_body.to_string())
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
