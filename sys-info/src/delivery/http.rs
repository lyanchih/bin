use super::{ContentType, DELIVERY_TIMEOUT};
use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;
use url::Url;

pub struct Endpoint {
    url: Url,
    authorization: Option<String>,
}

impl Endpoint {
    pub fn parse(mut url: Url) -> Result<Self, String> {
        let authorization = if url.username().is_empty() && url.password().is_none() {
            None
        } else {
            let mut credentials: Vec<_> = percent_decode_str(url.username()).collect();
            if credentials.contains(&b':') || credentials.iter().any(u8::is_ascii_control) {
                return Err("invalid HTTP Basic username".into());
            }
            let password: Vec<_> = percent_decode_str(url.password().unwrap_or_default()).collect();
            if password.iter().any(u8::is_ascii_control) {
                return Err("invalid HTTP Basic password".into());
            }
            credentials.push(b':');
            credentials.extend(password);
            url.set_password(None)
                .map_err(|_| "could not remove HTTP URI password")?;
            url.set_username("")
                .map_err(|_| "could not remove HTTP URI username")?;
            Some(format!("Basic {}", STANDARD.encode(credentials)))
        };
        Ok(Self { url, authorization })
    }

    pub fn scheme(&self) -> &'static str {
        if self.url.scheme() == "https" {
            "https"
        } else {
            "http"
        }
    }

    pub fn send(self, content_type: ContentType, body: &[u8]) -> Result<(), String> {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(DELIVERY_TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .into();
        let mut request = agent
            .post(self.url.as_str())
            .header("Content-Type", content_type.media_type());
        if let Some(authorization) = self.authorization {
            request = request.header("Authorization", authorization);
        }
        if content_type.is_gzip() {
            request = request.header("Content-Encoding", "gzip");
        }
        let response = request
            .send(body)
            .map_err(|error| format!("HTTP delivery failed: {error}"))?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(format!("HTTP delivery returned {}", response.status()))
        }
    }
}
