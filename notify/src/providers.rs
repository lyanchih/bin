use crate::http::{http_request, url_encode};
use crate::json::{self, Json};
use crate::time_util::{now_unix, parse_timestamp};
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct QuotaWindow {
    pub window: String,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub reset_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AgentQuota {
    pub agent: String,
    pub display_name: String,
    pub fetched_at: i64,
    pub windows: Vec<QuotaWindow>,
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Environment variable HOME is not set".to_string())
}

pub fn fetch_claude() -> Result<AgentQuota, String> {
    let cred_path = home_dir()?.join(".claude").join(".credentials.json");
    let raw = fs::read_to_string(&cred_path)
        .map_err(|e| format!("無法讀取 {}: {e}", cred_path.display()))?;
    let doc = json::parse(&raw).map_err(|e| format!("解析憑證 JSON 失敗: {e}"))?;

    let token = doc
        .get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "找不到 claudeAiOauth.accessToken".to_string())?;

    let auth_header = format!("Bearer {token}");
    let body = http_request(
        "GET",
        "https://api.anthropic.com/api/oauth/usage",
        &[
            ("Authorization", &auth_header),
            ("anthropic-beta", "oauth-2025-04-20"),
            ("User-Agent", "claude-code/1.0.0"),
        ],
        None,
        15,
    )?;

    let data = json::parse(&body).map_err(|e| format!("解析回應 JSON 失敗: {e}"))?;
    let mut windows = Vec::new();

    for (key, label) in [("five_hour", "5h"), ("seven_day", "7d")] {
        if let Some(w) = data.get(key) {
            if matches!(w, Json::Object(_)) {
                let used = w
                    .get("utilization")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0);
                let rem = (100.0 - used).clamp(0.0, 100.0);
                let reset_at = w.get("resets_at").and_then(parse_timestamp);
                windows.push(QuotaWindow {
                    window: label.to_string(),
                    used_percent: used,
                    remaining_percent: rem,
                    reset_at,
                });
            }
        }
    }

    if windows.is_empty() {
        return Err("API 回應中不包含 five_hour 或 seven_day 額度資訊".to_string());
    }

    Ok(AgentQuota {
        agent: "claude".to_string(),
        display_name: "Claude".to_string(),
        fetched_at: now_unix(),
        windows,
    })
}

pub fn fetch_codex() -> Result<AgentQuota, String> {
    let auth_path = home_dir()?.join(".codex").join("auth.json");
    let raw = fs::read_to_string(&auth_path)
        .map_err(|e| format!("無法讀取 {}: {e}", auth_path.display()))?;
    let doc = json::parse(&raw).map_err(|e| format!("解析憑證 JSON 失敗: {e}"))?;

    let token = doc
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .or_else(|| doc.get("access_token").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "找不到 tokens.access_token".to_string())?;

    let auth_header = format!("Bearer {token}");
    let body = http_request(
        "GET",
        "https://chatgpt.com/backend-api/wham/usage",
        &[
            ("Authorization", &auth_header),
            ("Accept", "application/json"),
            ("User-Agent", "codex-cli"),
        ],
        None,
        15,
    )?;

    let data = json::parse(&body).map_err(|e| format!("解析回應 JSON 失敗: {e}"))?;
    let rl = data
        .get("rate_limit")
        .ok_or_else(|| "API 回應中缺少 rate_limit 欄位".to_string())?;

    let mut windows = Vec::new();
    for (key, label) in [("primary_window", "5h"), ("secondary_window", "7d")] {
        if let Some(w) = rl.get(key) {
            if matches!(w, Json::Object(_)) {
                let used = w
                    .get("used_percent")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0);
                let rem = (100.0 - used).clamp(0.0, 100.0);
                let reset_at = w.get("reset_at").and_then(parse_timestamp);
                windows.push(QuotaWindow {
                    window: label.to_string(),
                    used_percent: used,
                    remaining_percent: rem,
                    reset_at,
                });
            }
        }
    }

    if windows.is_empty() {
        return Err("rate_limit 中不包含 primary_window 或 secondary_window".to_string());
    }

    Ok(AgentQuota {
        agent: "codex".to_string(),
        display_name: "Codex".to_string(),
        fetched_at: now_unix(),
        windows,
    })
}

fn obtain_antigravity_access_token() -> Result<String, String> {
    let home = home_dir()?;
    let acp_path = home
        .join(".gemini")
        .join("antigravity-acp")
        .join("acp_token.json");

    if acp_path.exists() {
        let raw = fs::read_to_string(&acp_path)
            .map_err(|e| format!("無法讀取 {}: {e}", acp_path.display()))?;
        let doc = json::parse(&raw).map_err(|e| format!("解析 acp_token.json 失敗: {e}"))?;

        let client_id = doc
            .get("client_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "acp_token.json 缺少 client_id".to_string())?;
        let client_secret = doc
            .get("client_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "acp_token.json 缺少 client_secret".to_string())?;
        let refresh_token = doc
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "acp_token.json 缺少 refresh_token".to_string())?;
        let token_uri = doc
            .get("token_uri")
            .and_then(|v| v.as_str())
            .unwrap_or("https://oauth2.googleapis.com/token");

        let form_body = format!(
            "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
            url_encode(client_id),
            url_encode(client_secret),
            url_encode(refresh_token)
        );

        let resp_body = http_request(
            "POST",
            token_uri,
            &[("Content-Type", "application/x-www-form-urlencoded")],
            Some(&form_body),
            15,
        )?;
        let tok_json =
            json::parse(&resp_body).map_err(|e| format!("解析 OAuth token 回應失敗: {e}"))?;
        if let Some(tok) = tok_json.get("access_token").and_then(|v| v.as_str()) {
            if !tok.is_empty() {
                return Ok(tok.to_string());
            }
        }
    }

    let oauth_path = home.join(".gemini").join("oauth_creds.json");
    if oauth_path.exists() {
        let raw = fs::read_to_string(&oauth_path)
            .map_err(|e| format!("無法讀取 {}: {e}", oauth_path.display()))?;
        let doc = json::parse(&raw).map_err(|e| format!("解析 oauth_creds.json 失敗: {e}"))?;
        if let Some(tok) = doc.get("access_token").and_then(|v| v.as_str()) {
            if !tok.is_empty() {
                return Ok(tok.to_string());
            }
        }
    }

    Err("找不到有效的 Antigravity 憑證 (~/.gemini/antigravity-acp/acp_token.json)".to_string())
}

pub fn fetch_antigravity() -> Result<AgentQuota, String> {
    let token = obtain_antigravity_access_token()?;
    let auth_header = format!("Bearer {token}");

    let endpoints = [
        "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
        "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    ];

    let mut last_err = String::new();
    let mut response_json = None;

    for url in endpoints {
        match http_request(
            "POST",
            url,
            &[
                ("Authorization", &auth_header),
                ("Content-Type", "application/json"),
                ("User-Agent", "antigravity/1.0"),
            ],
            Some("{}"),
            15,
        ) {
            Ok(body) => match json::parse(&body) {
                Ok(parsed) => {
                    response_json = Some(parsed);
                    break;
                }
                Err(e) => {
                    last_err = format!("解析 {url} JSON 失敗: {e}");
                }
            },
            Err(e) => {
                last_err = format!("{url} 請求失敗: {e}");
            }
        }
    }

    let data = response_json.ok_or(last_err)?;
    let models = data
        .get("models")
        .and_then(|m| m.as_object())
        .ok_or_else(|| "回應中缺少 models 欄位".to_string())?;

    // Only track the Gemini pool for Antigravity
    let mut gemini_pool: Option<QuotaWindow> = None;

    for (model_id, model_info) in models {
        if !model_id.starts_with("gemini-") {
            continue;
        }
        let has_display_name = model_info
            .get("displayName")
            .and_then(|v| v.as_str())
            .is_some();
        if !has_display_name {
            continue;
        }
        let Some(q) = model_info.get("quotaInfo") else {
            continue;
        };
        let rem_frac = q
            .get("remainingFraction")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let rem_pct = (rem_frac * 100.0).clamp(0.0, 100.0);
        let used_pct = (100.0 - rem_pct).clamp(0.0, 100.0);
        let reset_at = q.get("resetTime").and_then(parse_timestamp);

        match &mut gemini_pool {
            None => {
                gemini_pool = Some(QuotaWindow {
                    window: "Gemini".to_string(),
                    used_percent: used_pct,
                    remaining_percent: rem_pct,
                    reset_at,
                });
            }
            Some(existing) => {
                if used_pct > existing.used_percent {
                    existing.used_percent = used_pct;
                    existing.remaining_percent = rem_pct;
                }
                if existing.reset_at.is_none() && reset_at.is_some() {
                    existing.reset_at = reset_at;
                }
            }
        }
    }

    let Some(w) = gemini_pool else {
        return Err("Antigravity models 中找不到有效的 Gemini quotaInfo".to_string());
    };

    Ok(AgentQuota {
        agent: "antigravity".to_string(),
        display_name: "Antigravity".to_string(),
        fetched_at: now_unix(),
        windows: vec![w],
    })
}
