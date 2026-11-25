mod http;
#[allow(dead_code)]
mod json;
mod providers;
mod time_util;

use providers::Agent;
use std::env;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;
use time_util::{format_local_datetime, format_local_hhmm, format_remaining, now_unix};

const USAGE: &str = "用法: quote-refresh [AGENT...]

於額度未消耗時發出最小問答，啟動 agent 的五小時恢復週期並顯示剩餘時間。

AGENT: codex | claude | cursor | antigravity (anti)
預設: codex claude

環境變數 (選用):
  QUOTE_REFRESH_CODEX_MODEL / QUOTE_REFRESH_CLAUDE_MODEL (預設 haiku)
  QUOTE_REFRESH_CURSOR_MODEL / QUOTE_REFRESH_ANTI_MODEL";

/// 問答送出後，用量 API 可能延遲反映
const VERIFY_ATTEMPTS: u32 = 6;
const VERIFY_DELAY: Duration = Duration::from_secs(5);

fn main() -> ExitCode {
    let mut agents = Vec::new();
    for arg in env::args().skip(1) {
        if arg == "-h" || arg == "--help" {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        match Agent::parse(&arg) {
            Some(a) if !agents.contains(&a) => agents.push(a),
            Some(_) => {}
            None => {
                eprintln!("未知的 agent: {arg}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }
    if agents.is_empty() {
        agents = vec![Agent::Codex, Agent::Claude];
    }

    println!("[{}] quote-refresh", format_local_datetime(now_unix()));
    let mut failed = false;
    for agent in agents {
        match refresh(agent) {
            Ok((reset, kicked)) => {
                let tag = if kicked { "已啟動恢復週期" } else { "恢復中" };
                println!(
                    "  {:<7} {tag}  剩餘 {}  ({} 恢復)",
                    agent.name(),
                    format_remaining(reset, now_unix()),
                    format_local_hhmm(reset)
                );
            }
            Err(e) => {
                failed = true;
                eprintln!("  {:<7} 錯誤: {e}", agent.name());
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// 回傳 (恢復時間, 是否由本次問答啟動)
fn refresh(agent: Agent) -> Result<(i64, bool), String> {
    // 查詢失敗 (如 token 過期) 也照樣發問：CLI 執行時會自行更新 token
    let probe_err = if agent.token_expired() {
        Some("token 已過期".to_string())
    } else {
        match agent.pending_reset() {
            Ok(Some(reset)) => return Ok((reset, false)),
            Ok(None) => None,
            Err(e) => Some(e),
        }
    };
    agent.kick().map_err(|e| match &probe_err {
        Some(p) => format!("{e} (先前查詢失敗: {p})"),
        None => e,
    })?;
    for attempt in 0..VERIFY_ATTEMPTS {
        if attempt > 0 {
            thread::sleep(VERIFY_DELAY);
        }
        if let Some(reset) = agent.pending_reset()? {
            return Ok((reset, true));
        }
    }
    Err("已送出問答，但額度仍未進入恢復週期".to_string())
}
