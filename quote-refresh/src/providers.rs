use crate::http::{http_request, url_encode};
use crate::json::{self, Json};
use crate::time_util::{now_unix, parse_timestamp};
use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// 假設恢復週期為五小時
pub const WINDOW_SECS: i64 = 5 * 3600;

const PROMPT: &str = "Reply with only: ok";
const CLI_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Codex,
    Claude,
    Cursor,
    Antigravity,
}

impl Agent {
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "codex" => Some(Self::Codex),
            "claude" => Some(Self::Claude),
            "cursor" => Some(Self::Cursor),
            "antigravity" | "anti" => Some(Self::Antigravity),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Antigravity => "anti",
        }
    }

    /// 查詢目前是否處於「額度有消耗、待恢復」狀態；是則回傳恢復時間
    pub fn pending_reset(self) -> Result<Option<i64>, String> {
        match self {
            Self::Codex => codex_pending(),
            Self::Claude => claude_pending(),
            Self::Cursor => cursor_pending(),
            Self::Antigravity => antigravity_pending(),
        }
    }

    /// 憑證已知過期時直接跳過查詢 (目前僅 Claude 憑證帶有 expiresAt)
    pub fn token_expired(self) -> bool {
        match self {
            Self::Claude => claude_token_expired(),
            _ => false,
        }
    }

    /// 發出最簡單的問答以啟動恢復週期
    pub fn kick(self) -> Result<(), String> {
        match self {
            Self::Codex => {
                let mut args = vec![
                    "exec".to_string(),
                    "--skip-git-repo-check".into(),
                    "--ephemeral".into(),
                    "-s".into(),
                    "read-only".into(),
                    "-c".into(),
                    "model_reasoning_effort=\"low\"".into(),
                ];
                push_model(&mut args, "-m", "QUOTE_REFRESH_CODEX_MODEL", None);
                args.push(PROMPT.into());
                run_cli("codex", &args)
            }
            Self::Claude => {
                let mut args = vec![
                    "-p".to_string(),
                    "--no-session-persistence".into(),
                    "--strict-mcp-config".into(),
                    "--disable-slash-commands".into(),
                    "--tools".into(),
                    "".into(),
                ];
                push_model(&mut args, "--model", "QUOTE_REFRESH_CLAUDE_MODEL", Some("haiku"));
                args.push(PROMPT.into());
                run_cli("claude", &args)
            }
            Self::Cursor => {
                let mut args = vec!["-p".to_string(), "--mode".into(), "ask".into(), "--trust".into()];
                push_model(&mut args, "--model", "QUOTE_REFRESH_CURSOR_MODEL", None);
                args.push(PROMPT.into());
                run_cli("cursor-agent", &args)?;
                // Cursor 無五小時視窗 API，以本地紀錄推估
                fs::create_dir_all(state_dir()?).map_err(|e| format!("建立狀態目錄失敗: {e}"))?;
                fs::write(cursor_state_path()?, now_unix().to_string())
                    .map_err(|e| format!("寫入 Cursor 狀態失敗: {e}"))
            }
            Self::Antigravity => antigravity_kick(),
        }
    }
}

fn home_dir() -> Result<PathBuf, String> {
    env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "Environment variable HOME is not set".to_string())
}

fn read_json(path: &PathBuf) -> Result<Json, String> {
    let raw = fs::read_to_string(path).map_err(|e| format!("無法讀取 {}: {e}", path.display()))?;
    json::parse(&raw).map_err(|e| format!("解析 {} 失敗: {e}", path.display()))
}

fn future(ts: Option<i64>) -> Option<i64> {
    ts.filter(|&t| t > now_unix())
}

// ---------- Claude ----------

fn claude_token_expired() -> bool {
    let Ok(doc) = home_dir().and_then(|h| read_json(&h.join(".claude").join(".credentials.json"))) else {
        return false;
    };
    doc.get("claudeAiOauth")
        .and_then(|o| o.get("expiresAt"))
        .and_then(|v| v.as_f64())
        .is_some_and(|ms| (ms / 1000.0) as i64 <= now_unix() + 60)
}

fn claude_pending() -> Result<Option<i64>, String> {
    let doc = read_json(&home_dir()?.join(".claude").join(".credentials.json"))?;
    let token = doc
        .get("claudeAiOauth")
        .and_then(|o| o.get("accessToken"))
        .and_then(|t| t.as_str())
        .filter(|s| !s.is_empty())
        .ok_or("找不到 claudeAiOauth.accessToken")?;
    let auth = format!("Bearer {token}");
    let body = http_request(
        "GET",
        "https://api.anthropic.com/api/oauth/usage",
        &[
            ("Authorization", &auth),
            ("anthropic-beta", "oauth-2025-04-20"),
            ("User-Agent", "claude-code/1.0.0"),
        ],
        None,
        15,
    )?;
    let data = json::parse(&body).map_err(|e| format!("解析回應 JSON 失敗: {e}"))?;
    // 視窗未啟動時 resets_at 為 null
    Ok(future(
        data.get("five_hour")
            .and_then(|w| w.get("resets_at"))
            .and_then(parse_timestamp),
    ))
}

// ---------- Codex ----------

fn codex_pending() -> Result<Option<i64>, String> {
    let doc = read_json(&home_dir()?.join(".codex").join("auth.json"))?;
    let token = doc
        .get("tokens")
        .and_then(|t| t.get("access_token"))
        .and_then(|v| v.as_str())
        .or_else(|| doc.get("access_token").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .ok_or("找不到 tokens.access_token")?;
    let auth = format!("Bearer {token}");
    let body = http_request(
        "GET",
        "https://chatgpt.com/backend-api/wham/usage",
        &[
            ("Authorization", &auth),
            ("Accept", "application/json"),
            ("User-Agent", "codex-cli"),
        ],
        None,
        15,
    )?;
    let data = json::parse(&body).map_err(|e| format!("解析回應 JSON 失敗: {e}"))?;
    let w = data
        .get("rate_limit")
        .and_then(|r| r.get("primary_window"))
        .ok_or("找不到 rate_limit.primary_window")?;
    let num = |k: &str| w.get(k).and_then(|v| v.as_f64());
    let used = num("used_percent").unwrap_or(0.0);
    let after = num("reset_after_seconds");
    let window = num("limit_window_seconds");
    // 視窗未啟動時 reset_after_seconds == limit_window_seconds 且用量為 0
    let started = used > 0.0 || matches!((after, window), (Some(a), Some(l)) if a < l);
    if !started {
        return Ok(None);
    }
    let reset = w
        .get("reset_at")
        .and_then(parse_timestamp)
        .or_else(|| after.map(|a| now_unix() + a as i64));
    Ok(future(reset))
}

// ---------- Cursor ----------

fn state_dir() -> Result<PathBuf, String> {
    Ok(env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|_| home_dir().map(|h| h.join(".cache")))?
        .join("quote-refresh"))
}

fn cursor_state_path() -> Result<PathBuf, String> {
    Ok(state_dir()?.join("cursor"))
}

fn cursor_pending() -> Result<Option<i64>, String> {
    let Ok(raw) = fs::read_to_string(cursor_state_path()?) else {
        return Ok(None);
    };
    Ok(future(raw.trim().parse::<i64>().ok().map(|t| t + WINDOW_SECS)))
}

// ---------- Antigravity ----------

const CLOUDCODE: &str = "https://cloudcode-pa.googleapis.com/v1internal";

fn antigravity_token() -> Result<String, String> {
    let home = home_dir()?;
    let acp_path = home.join(".gemini").join("antigravity-acp").join("acp_token.json");
    if acp_path.exists() {
        let doc = read_json(&acp_path)?;
        let field = |k: &str| {
            doc.get(k)
                .and_then(|v| v.as_str())
                .ok_or_else(|| format!("acp_token.json 缺少 {k}"))
        };
        let form = format!(
            "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
            url_encode(field("client_id")?),
            url_encode(field("client_secret")?),
            url_encode(field("refresh_token")?)
        );
        let token_uri = doc
            .get("token_uri")
            .and_then(|v| v.as_str())
            .unwrap_or("https://oauth2.googleapis.com/token");
        let body = http_request(
            "POST",
            token_uri,
            &[("Content-Type", "application/x-www-form-urlencoded")],
            Some(&form),
            15,
        )?;
        let tok = json::parse(&body).map_err(|e| format!("解析 token 失敗: {e}"))?;
        if let Some(t) = tok.get("access_token").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return Ok(t.to_string());
        }
    }
    let oauth_path = home.join(".gemini").join("oauth_creds.json");
    if oauth_path.exists() {
        let doc = read_json(&oauth_path)?;
        if let Some(t) = doc.get("access_token").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
            return Ok(t.to_string());
        }
    }
    Err("找不到 Antigravity 憑證".to_string())
}

fn cloudcode_post(auth: &str, method: &str, body: &str) -> Result<Json, String> {
    let url = format!("{CLOUDCODE}:{method}");
    let resp = http_request(
        "POST",
        &url,
        &[
            ("Authorization", auth),
            ("Content-Type", "application/json"),
            ("User-Agent", "antigravity/1.0"),
        ],
        Some(body),
        30,
    )?;
    json::parse(&resp).map_err(|e| format!("解析 {method} 回應失敗: {e}"))
}

fn gemini_models(auth: &str) -> Result<Vec<(String, Json)>, String> {
    let data = cloudcode_post(auth, "fetchAvailableModels", "{}")?;
    let models = data
        .get("models")
        .and_then(|m| m.as_object())
        .ok_or("找不到 models")?;
    Ok(models
        .iter()
        .filter(|(id, info)| id.starts_with("gemini-") && info.get("quotaInfo").is_some())
        .cloned()
        .collect())
}

fn antigravity_pending() -> Result<Option<i64>, String> {
    let auth = format!("Bearer {}", antigravity_token()?);
    let now = now_unix();
    let mut earliest: Option<i64> = None;
    for (_, info) in gemini_models(&auth)? {
        let q = info.get("quotaInfo").unwrap();
        let remaining = q.get("remainingFraction").and_then(|v| v.as_f64()).unwrap_or(1.0);
        let Some(reset) = future(q.get("resetTime").and_then(parse_timestamp)) else {
            continue;
        };
        // 未消耗時 resetTime 約等於 now + 五小時
        if remaining < 1.0 || reset < now + WINDOW_SECS - 120 {
            earliest = Some(earliest.map_or(reset, |e| e.min(reset)));
        }
    }
    Ok(earliest)
}

fn antigravity_kick() -> Result<(), String> {
    let auth = format!("Bearer {}", antigravity_token()?);
    let load = cloudcode_post(
        &auth,
        "loadCodeAssist",
        r#"{"metadata":{"ideType":"ANTIGRAVITY","platform":"PLATFORM_UNSPECIFIED","pluginType":"GEMINI"}}"#,
    )?;
    let project = load
        .get("cloudaicompanionProject")
        .and_then(|p| p.as_str().or_else(|| p.get("id").and_then(|v| v.as_str())))
        .ok_or("loadCodeAssist 未回傳 cloudaicompanionProject")?
        .to_string();
    let model = match env::var("QUOTE_REFRESH_ANTI_MODEL") {
        Ok(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => gemini_models(&auth)?
            .into_iter()
            .map(|(id, _)| id)
            .filter(|id| id.contains("flash") && !id.contains("image"))
            .min_by_key(|id| id.len())
            .ok_or("找不到可用的 Gemini flash 模型")?,
    };
    let body = format!(
        r#"{{"project":{},"model":{},"request":{{"contents":[{{"role":"user","parts":[{{"text":{}}}]}}],"generationConfig":{{"maxOutputTokens":8}}}}}}"#,
        Json::Str(project).to_string_compact(),
        Json::Str(model).to_string_compact(),
        Json::Str(PROMPT.into()).to_string_compact(),
    );
    cloudcode_post(&auth, "generateContent", &body).map(|_| ())
}

// ---------- CLI ----------

fn push_model(args: &mut Vec<String>, flag: &str, var: &str, default: Option<&str>) {
    let model = env::var(var)
        .ok()
        .filter(|m| !m.trim().is_empty())
        .or_else(|| default.map(str::to_string));
    if let Some(m) = model {
        args.push(flag.to_string());
        args.push(m);
    }
}

/// cron 的 PATH 很精簡，額外搜尋常見的使用者安裝位置
fn find_bin(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = env::var_os("PATH")
        .map(|p| env::split_paths(&p).collect())
        .unwrap_or_default();
    if let Ok(home) = home_dir() {
        for d in [".local/bin", "bin", ".npm-global/bin", ".bun/bin", ".cargo/bin"] {
            dirs.push(home.join(d));
        }
        if let Ok(entries) = fs::read_dir(home.join(".nvm/versions/node")) {
            dirs.extend(entries.flatten().map(|e| e.path().join("bin")));
        }
    }
    dirs.push("/usr/local/bin".into());
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

fn run_cli(name: &str, args: &[String]) -> Result<(), String> {
    let bin = find_bin(name).ok_or_else(|| format!("找不到 {name} 執行檔"))?;
    let workdir = env::temp_dir().join("quote-refresh");
    fs::create_dir_all(&workdir).map_err(|e| format!("建立工作目錄失敗: {e}"))?;

    // 讓子程序也能找到 node 等相依執行檔
    let mut path = bin.parent().map(|p| p.as_os_str().to_owned()).unwrap_or_default();
    if let Some(p) = env::var_os("PATH") {
        path.push(":");
        path.push(p);
    }

    let mut child = Command::new(&bin)
        .args(args)
        .current_dir(&workdir)
        .env("PATH", path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("執行 {} 失敗: {e}", bin.display()))?;

    let mut stderr = child.stderr.take().unwrap();
    let reader = thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() > CLI_TIMEOUT => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{name} 逾時 ({}s)", CLI_TIMEOUT.as_secs()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("等待 {name} 失敗: {e}")),
        }
    };
    let err = reader.join().unwrap_or_default();
    if status.success() {
        Ok(())
    } else {
        let tail: Vec<&str> = err.lines().rev().take(3).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        Err(format!("{name} 結束碼 {status}: {}", tail.join(" | ").trim()))
    }
}
