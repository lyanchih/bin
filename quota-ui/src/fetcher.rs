use crate::http::{http_request, url_encode};
use crate::json;
use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct QuotaSummary {
    pub claude_5h: Option<f64>,
    pub codex_5h: Option<f64>,
    pub gemini: Option<f64>,
    pub cursor: Option<f64>,
    pub error_claude: bool,
    pub error_codex: bool,
    pub error_gemini: bool,
    pub error_cursor: bool,
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Environment variable HOME is not set".to_string())
}

pub fn fetch_claude_5h() -> Result<f64, String> {
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
        10,
    )?;

    let data = json::parse(&body).map_err(|e| format!("解析回應 JSON 失敗: {e}"))?;
    let used = data
        .get("five_hour")
        .and_then(|w| w.get("utilization"))
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "找不到 five_hour.utilization".to_string())?;

    Ok(used.clamp(0.0, 100.0))
}

pub fn fetch_codex_5h() -> Result<f64, String> {
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
        10,
    )?;

    let data = json::parse(&body).map_err(|e| format!("解析回應 JSON 失敗: {e}"))?;
    let used = data
        .get("rate_limit")
        .and_then(|rl| rl.get("primary_window"))
        .and_then(|pw| pw.get("used_percent"))
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "找不到 primary_window.used_percent".to_string())?;

    Ok(used.clamp(0.0, 100.0))
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
            .ok_or_else(|| "缺少 client_id".to_string())?;
        let client_secret = doc
            .get("client_secret")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "缺少 client_secret".to_string())?;
        let refresh_token = doc
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "缺少 refresh_token".to_string())?;
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
            10,
        )?;
        let tok_json = json::parse(&resp_body).map_err(|e| format!("解析 token 失敗: {e}"))?;
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

    Err("找不到 Antigravity 憑證".to_string())
}

pub fn fetch_gemini() -> Result<f64, String> {
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
            10,
        ) {
            Ok(body) => match json::parse(&body) {
                Ok(parsed) => {
                    response_json = Some(parsed);
                    break;
                }
                Err(e) => last_err = format!("解析 JSON 失敗: {e}"),
            },
            Err(e) => last_err = format!("請求失敗: {e}"),
        }
    }

    let data = response_json.ok_or(last_err)?;
    let models = data
        .get("models")
        .and_then(|m| m.as_object())
        .ok_or_else(|| "找不到 models".to_string())?;

    let mut max_used: Option<f64> = None;

    for (model_id, model_info) in models {
        if !model_id.starts_with("gemini-") {
            continue;
        }
        if model_info.get("displayName").and_then(|v| v.as_str()).is_none() {
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
        let rem_pct = rem_frac * 100.0;
        let used_pct = (100.0 - rem_pct).clamp(0.0, 100.0);

        match max_used {
            None => max_used = Some(used_pct),
            Some(v) if used_pct > v => max_used = Some(used_pct),
            _ => {}
        }
    }

    max_used.ok_or_else(|| "找不到 Gemini 額度資訊".to_string())
}

enum CursorAuth {
    Bearer(String),
    Cookie(String),
}

fn obtain_cursor_auth() -> Result<CursorAuth, String> {
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

pub fn fetch_cursor() -> Result<f64, String> {
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
                10,
            )?;
            let data = json::parse(&body).map_err(|e| format!("解析 JSON 失敗: {e}"))?;
            if let Some(gpt4) = data.get("gpt-4") {
                let num_req = gpt4.get("numRequests").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let max_req = gpt4.get("maxRequestUsage").and_then(|v| v.as_f64());
                if let Some(limit) = max_req {
                    if limit > 0.0 {
                        return Ok(((num_req / limit) * 100.0).clamp(0.0, 100.0));
                    }
                }
            }
            Ok(0.0)
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
                10,
            )?;
            let data = json::parse(&body).map_err(|e| format!("解析 JSON 失敗: {e}"))?;
            if let Some(plan) = data.get("individualUsage").and_then(|u| u.get("plan")) {
                if let Some(pct) = plan.get("totalPercentUsed").and_then(|v| v.as_f64()) {
                    return Ok(pct.clamp(0.0, 100.0));
                }
                if let (Some(used), Some(limit)) = (
                    plan.get("used").and_then(|v| v.as_f64()),
                    plan.get("limit").and_then(|v| v.as_f64()),
                ) {
                    if limit > 0.0 {
                        return Ok(((used / limit) * 100.0).clamp(0.0, 100.0));
                    }
                }
            }
            Ok(0.0)
        }
    }
}

pub fn fetch_all() -> QuotaSummary {
    let mut s = QuotaSummary::default();

    match fetch_claude_5h() {
        Ok(v) => s.claude_5h = Some(v),
        Err(e) => {
            eprintln!("[quota-ui] Claude fetch error: {e}");
            s.error_claude = true;
        }
    }

    match fetch_codex_5h() {
        Ok(v) => s.codex_5h = Some(v),
        Err(e) => {
            eprintln!("[quota-ui] Codex fetch error: {e}");
            s.error_codex = true;
        }
    }

    match fetch_gemini() {
        Ok(v) => s.gemini = Some(v),
        Err(e) => {
            eprintln!("[quota-ui] Gemini fetch error: {e}");
            s.error_gemini = true;
        }
    }

    match fetch_cursor() {
        Ok(v) => s.cursor = Some(v),
        Err(e) => {
            eprintln!("[quota-ui] Cursor fetch error: {e}");
            s.error_cursor = true;
        }
    }

    s
}
