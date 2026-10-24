use crate::http::http_request;
use crate::json::Json;
use crate::state::{AgentState, AlertEvent};
use crate::time_util::{format_local_hhmm, format_remaining};
use std::fmt::Write;

fn status_badge(used_percent: f64) -> &'static str {
    if used_percent >= 95.0 {
        "🚨"
    } else if used_percent >= 90.0 {
        "🔴"
    } else if used_percent >= 85.0 {
        "🟠"
    } else if used_percent >= 80.0 {
        "🟡"
    } else {
        "🟢"
    }
}

fn agent_icon(agent: &str) -> &'static str {
    match agent {
        "claude" => "🟣",
        "codex" => "🟢",
        "antigravity" => "🔵",
        _ => "🤖",
    }
}

fn progress_bar(used_percent: f64, width: usize) -> String {
    let clamped = used_percent.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * (width as f64)).round() as usize;
    let filled = filled.min(width);
    let empty = width.saturating_sub(filled);
    let mut bar = String::with_capacity(width * 3 + 2);
    for _ in 0..filled {
        bar.push('█');
    }
    for _ in 0..empty {
        bar.push('░');
    }
    bar
}

pub fn build_notification_payload(
    alerts: &[AlertEvent],
    agents: &[AgentState],
    now_ts: i64,
) -> (String, String, u8, Vec<&'static str>) {
    let max_tier = alerts.iter().map(|a| a.threshold).max().unwrap_or(80);

    let priority = match max_tier {
        95..=100 => 5,
        90..=94 => 4,
        85..=89 => 4,
        _ => 3,
    };

    let tags = match max_tier {
        95..=100 => vec!["rotating_light", "warning", "robot"],
        90..=94 => vec!["red_circle", "warning", "robot"],
        85..=89 => vec!["orange_circle", "warning", "robot"],
        _ => vec!["yellow_circle", "robot"],
    };

    let title = if alerts.is_empty() {
        "📊 AI 訂閱額度狀態總覽".to_string()
    } else if alerts.len() == 1 {
        let a = &alerts[0];
        format!(
            "{} AI 額度告警｜{} ({}) 已用 {:.0}% (超過 {}%)",
            status_badge(a.used_percent),
            a.display_name,
            a.window,
            a.used_percent,
            a.threshold
        )
    } else {
        let names: Vec<String> = alerts
            .iter()
            .map(|a| format!("{}({})≥{}%", a.display_name, a.window, a.threshold))
            .collect();
        format!(
            "{} AI 額度告警｜{}",
            status_badge(max_tier as f64),
            names.join("、")
        )
    };

    let mut msg = String::new();

    if !alerts.is_empty() {
        let _ = writeln!(msg, "🔔 超額觸發通知：");
        for a in alerts {
            let rst_info = match a.reset_at {
                Some(ts) => format!(
                    "剩餘 {} ({})",
                    format_remaining(ts, now_ts),
                    format_local_hhmm(ts)
                ),
                None => "-".to_string(),
            };
            let _ = writeln!(
                msg,
                "• {} {} [{}]：已用 {:.1}% / 剩餘 {:.1}%（跨越 {}% 門檻｜刷新：{}）",
                status_badge(a.used_percent),
                a.display_name,
                a.window,
                a.used_percent,
                a.remaining_percent,
                a.threshold,
                rst_info
            );
        }
        let _ = writeln!(msg, "\n────────────────────────");
    }

    let _ = writeln!(msg, "📊 所有 Agent 目前額度狀態：");

    for agent in agents {
        let _ = writeln!(
            msg,
            "\n{} {}",
            agent_icon(&agent.agent),
            agent.display_name
        );
        let len = agent.windows.len();
        for (idx, w) in agent.windows.iter().enumerate() {
            let branch = if idx + 1 == len { "└─" } else { "├─" };
            let badge = status_badge(w.used_percent);
            let bar = progress_bar(w.used_percent, 10);
            let reset_str = match w.reset_at {
                Some(ts) => format!(
                    "⏳ {} ({})",
                    format_remaining(ts, now_ts),
                    format_local_hhmm(ts)
                ),
                None => "⏳ -".to_string(),
            };
            let warn_suffix = if w.used_percent >= 80.0 {
                format!(" ⚠️≥{}%", w.last_notified_threshold.unwrap_or(80))
            } else {
                String::new()
            };
            let _ = writeln!(
                msg,
                "{} {:<10} {} [{}] 剩 {:>3.0}% (用 {:>3.0}%)｜{}{}",
                branch,
                format!("{}:", w.window),
                badge,
                bar,
                w.remaining_percent,
                w.used_percent,
                reset_str,
                warn_suffix
            );
        }
    }

    (title, msg.trim_end().to_string(), priority, tags)
}

pub fn send_ntfy(
    ntfy_server: &str,
    topic: &str,
    title: &str,
    message: &str,
    priority: u8,
    tags: &[&str],
) -> Result<(), String> {
    let url = ntfy_server.trim_end_matches('/');
    let payload = Json::Object(vec![
        ("topic".to_string(), Json::Str(topic.to_string())),
        ("title".to_string(), Json::Str(title.to_string())),
        ("message".to_string(), Json::Str(message.to_string())),
        ("priority".to_string(), Json::Number(priority as f64)),
        (
            "tags".to_string(),
            Json::Array(tags.iter().map(|t| Json::Str((*t).to_string())).collect()),
        ),
    ]);

    let body = payload.to_string_compact();
    http_request(
        "POST",
        url,
        &[("Content-Type", "application/json; charset=utf-8")],
        Some(&body),
        15,
    )
    .map(|_| ())
}
