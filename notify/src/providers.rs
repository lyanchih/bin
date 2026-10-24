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

    let endpoints = [
        "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
        "https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels",
    ];

    let mut last_err = String::new();
    let mut response_json = None;

    let auth_header = format!("Bearer {token}");
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

enum CursorAuth {
    Bearer(String),
    Cookie(String),
}

fn obtain_cursor_auth() -> Result<CursorAuth, String> {
    // 1. Env vars
    if let Ok(tok) = env::var("CURSOR_AUTH_TOKEN").or_else(|_| env::var("CURSOR_API_KEY")) {
        if !tok.trim().is_empty() {
            return Ok(CursorAuth::Bearer(tok.trim().to_string()));
        }
    }
    if let Ok(cookie) = env::var("CURSOR_COOKIE").or_else(|_| env::var("CURSOR_SESSION_TOKEN")) {
        if !cookie.trim().is_empty() {
            let val = cookie.trim();
            let c = if val.starts_with("WorkosCursorSessionToken=") {
                val.to_string()
            } else {
                format!("WorkosCursorSessionToken={val}")
            };
            return Ok(CursorAuth::Cookie(c));
        }
    }

    // 2. Candidate Files
    let home = home_dir()?;
    let candidate_files = [
        home.join(".cursor").join("auth.json"),
        home.join(".cursor").join("credentials.json"),
        home.join(".config").join("cursor").join("auth.json"),
        home.join(".config").join("Cursor").join("auth.json"),
    ];

    for path in candidate_files {
        if path.exists() {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(doc) = json::parse(&raw) {
                    if let Some(t) = doc.get("accessToken").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            return Ok(CursorAuth::Bearer(t.to_string()));
                        }
                    }
                    if let Some(t) = doc.get("token").and_then(|v| v.as_str()) {
                        if !t.is_empty() {
                            return Ok(CursorAuth::Bearer(t.to_string()));
                        }
                    }
                    if let Some(c) = doc.get("cookie").and_then(|v| v.as_str()) {
                        if !c.is_empty() {
                            let val = c.trim();
                            let cookie = if val.starts_with("WorkosCursorSessionToken=") {
                                val.to_string()
                            } else {
                                format!("WorkosCursorSessionToken={val}")
                            };
                            return Ok(CursorAuth::Cookie(cookie));
                        }
                    }
                }
            }
        }
    }

    Err("找不到 Cursor 憑證 (~/.cursor/auth.json 或 CURSOR_AUTH_TOKEN / CURSOR_COOKIE)".to_string())
}

pub fn fetch_cursor() -> Result<AgentQuota, String> {
    let auth = obtain_cursor_auth()?;
    match auth {
        CursorAuth::Bearer(token) => {
            let auth_header = format!("Bearer {token}");
            let body = http_request(
                "GET",
                "https://api2.cursor.sh/auth/usage",
                &[
                    ("Authorization", &auth_header),
                    ("Accept", "application/json"),
                    ("User-Agent", "cursor-client"),
                ],
                None,
                15,
            )?;

            let data = json::parse(&body).map_err(|e| format!("解析 Cursor API 回應 JSON 失敗: {e}"))?;
            let reset_at = data.get("startOfMonth").and_then(parse_timestamp);

            let mut used_pct = 0.0;
            if let Some(gpt4) = data.get("gpt-4") {
                let num_req = gpt4.get("numRequests").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let max_req = gpt4.get("maxRequestUsage").and_then(|v| v.as_f64());
                if let Some(limit) = max_req {
                    if limit > 0.0 {
                        used_pct = ((num_req / limit) * 100.0).clamp(0.0, 100.0);
                    }
                }
            }

            let rem_pct = (100.0 - used_pct).clamp(0.0, 100.0);
            Ok(AgentQuota {
                agent: "cursor".to_string(),
                display_name: "Cursor".to_string(),
                fetched_at: now_unix(),
                windows: vec![QuotaWindow {
                    window: "Fast Requests".to_string(),
                    used_percent: used_pct,
                    remaining_percent: rem_pct,
                    reset_at,
                }],
            })
        }
        CursorAuth::Cookie(cookie) => {
            let body = http_request(
                "GET",
                "https://cursor.com/api/usage-summary",
                &[
                    ("Cookie", &cookie),
                    ("Accept", "application/json"),
                    ("User-Agent", "Mozilla/5.0"),
                ],
                None,
                15,
            )?;

            let data = json::parse(&body).map_err(|e| format!("解析 Cursor Dashboard JSON 失敗: {e}"))?;
            let reset_at = data.get("billingCycleEnd").and_then(parse_timestamp);

            let plan = data
                .get("individualUsage")
                .and_then(|u| u.get("plan"));

            let used_pct = if let Some(p) = plan {
                p.get("totalPercentUsed")
                    .and_then(|v| v.as_f64())
                    .or_else(|| {
                        let used = p.get("used").and_then(|v| v.as_f64())?;
                        let limit = p.get("limit").and_then(|v| v.as_f64())?;
                        if limit > 0.0 {
                            Some((used / limit) * 100.0)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(0.0)
                    .clamp(0.0, 100.0)
            } else {
                0.0
            };

            let rem_pct = (100.0 - used_pct).clamp(0.0, 100.0);
            Ok(AgentQuota {
                agent: "cursor".to_string(),
                display_name: "Cursor".to_string(),
                fetched_at: now_unix(),
                windows: vec![QuotaWindow {
                    window: "Plan Usage".to_string(),
                    used_percent: used_pct,
                    remaining_percent: rem_pct,
                    reset_at,
                }],
            })
        }
    }
}
