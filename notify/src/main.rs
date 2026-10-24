mod http;
mod json;
mod ntfy;
mod providers;
mod state;
mod time_util;

use ntfy::{build_notification_payload, send_ntfy};
use providers::{fetch_antigravity, fetch_claude, fetch_codex, fetch_cursor, AgentQuota};
use state::QuotaStateStore;
use std::env;
use std::path::PathBuf;
use std::process;
use std::thread;
use std::time::Duration;
use time_util::{format_local_datetime, format_local_hhmm, format_remaining, now_unix};

struct Config {
    interval_mins: u64,
    state_file: PathBuf,
    ntfy_server: String,
    ntfy_topic: String,
    run_once: bool,
    force_notify: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval_mins: 5,
            state_file: PathBuf::from("/tmp/ai-quota-notify-state.json"),
            ntfy_server: "https://ntfy.sh".to_string(),
            ntfy_topic: "lyan-ai-quota".to_string(),
            run_once: false,
            force_notify: false,
        }
    }
}

fn print_usage(prog: &str) {
    println!(
        "AI 訂閱額度監控與通知程序 (支援 Claude, Codex, Antigravity, Cursor)

用法:
  {prog} [選項]

選項:
  -i, --interval <分鐘>   設定抓取間隔分鐘數 (預設: 5)
  -s, --state <路徑>      設定暫存檔路徑 (預設: /tmp/ai-quota-notify-state.json)
      --topic <topic>     設定 ntfy topic (預設: lyan-ai-quota)
      --server <url>      設定 ntfy 伺服器 (預設: https://ntfy.sh)
  -1, --once              只執行一次抓取與檢查即結束
  -t, --test-notify       強制發送一次目前額度狀態通知至 ntfy (測試用)
  -h, --help              顯示此說明"
    );
}

fn parse_args() -> Config {
    let mut cfg = Config::default();
    let args: Vec<String> = env::args().collect();
    let prog = args.first().map(|s| s.as_str()).unwrap_or("notify");

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                print_usage(prog);
                process::exit(0);
            }
            "-i" | "--interval" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("[ERROR] -i / --interval 需要提供分鐘數參數");
                    process::exit(1);
                };
                match val.parse::<u64>() {
                    Ok(m) if m > 0 => cfg.interval_mins = m,
                    _ => {
                        eprintln!("[ERROR] 無效的分鐘數參數: {val} (必須為正整數)");
                        process::exit(1);
                    }
                }
            }
            arg if arg.starts_with("-i") && arg.len() > 2 => {
                let val = &arg[2..];
                match val.parse::<u64>() {
                    Ok(m) if m > 0 => cfg.interval_mins = m,
                    _ => {
                        eprintln!("[ERROR] 無效的分鐘數參數: {val} (必須為正整數)");
                        process::exit(1);
                    }
                }
            }
            "-s" | "--state" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("[ERROR] -s / --state 需要提供檔案路徑");
                    process::exit(1);
                };
                cfg.state_file = PathBuf::from(val);
            }
            "--topic" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("[ERROR] --topic 需要提供 topic 名稱");
                    process::exit(1);
                };
                cfg.ntfy_topic = val.clone();
            }
            "--server" => {
                i += 1;
                let Some(val) = args.get(i) else {
                    eprintln!("[ERROR] --server 需要提供 URL");
                    process::exit(1);
                };
                cfg.ntfy_server = val.clone();
            }
            "-1" | "--once" => {
                cfg.run_once = true;
            }
            "-t" | "--test-notify" => {
                cfg.force_notify = true;
            }
            other => {
                eprintln!("[ERROR] 未知的參數: {other}");
                print_usage(prog);
                process::exit(1);
            }
        }
        i += 1;
    }

    cfg
}

fn poll_once(cfg: &Config) {
    let now_ts = now_unix();
    let now_str = format_local_datetime(now_ts);
    println!("[{now_str}] 開始抓取各 Agent 訂閱額度...");

    let fetchers: [(&str, fn() -> Result<AgentQuota, String>); 4] = [
        ("claude", fetch_claude),
        ("codex", fetch_codex),
        ("antigravity", fetch_antigravity),
        ("cursor", fetch_cursor),
    ];

    let mut fresh_quotas = Vec::new();
    for (name, fetcher) in fetchers {
        match fetcher() {
            Ok(q) => {
                let summary: Vec<String> = q
                    .windows
                    .iter()
                    .map(|w| {
                        let rst = w
                            .reset_at
                            .map(|ts| {
                                format!(
                                    "{} ({})",
                                    format_remaining(ts, now_ts),
                                    format_local_hhmm(ts)
                                )
                            })
                            .unwrap_or_else(|| "-".to_string());
                        format!(
                            "{}: 剩餘 {:.1}% / 已用 {:.1}% [刷新: {}]",
                            w.window, w.remaining_percent, w.used_percent, rst
                        )
                    })
                    .collect();
                println!(
                    "[{}] [INFO] [{}] {}",
                    format_local_datetime(now_unix()),
                    q.display_name,
                    summary.join(" | ")
                );
                fresh_quotas.push(q);
            }
            Err(err) => {
                eprintln!(
                    "[{}] [ERROR] [{}] 抓取訂閱額度失敗: {}",
                    format_local_datetime(now_unix()),
                    name,
                    err
                );
            }
        }
    }

    let prev_store = QuotaStateStore::load(&cfg.state_file);
    let mut store = prev_store.clone();
    let alerts = store.update_and_evaluate(fresh_quotas, now_ts);

    if !alerts.is_empty() || cfg.force_notify {
        let (title, message, priority, tags) =
            build_notification_payload(&alerts, &store.agents, now_ts);

        match send_ntfy(
            &cfg.ntfy_server,
            &cfg.ntfy_topic,
            &title,
            &message,
            priority,
            &tags,
        ) {
            Ok(()) => {
                println!(
                    "[{}] [INFO] 已發送 ntfy 通知至 {}/{} ({})",
                    format_local_datetime(now_unix()),
                    cfg.ntfy_server.trim_end_matches('/'),
                    cfg.ntfy_topic,
                    title
                );
            }
            Err(err) => {
                eprintln!(
                    "[{}] [ERROR] 發送 ntfy 通知失敗 ({}/{}): {}",
                    format_local_datetime(now_unix()),
                    cfg.ntfy_server.trim_end_matches('/'),
                    cfg.ntfy_topic,
                    err
                );
                // Roll back last_notified_threshold on send failure so next cycle retries
                for alert in &alerts {
                    let prev_thresh = prev_store
                        .agents
                        .iter()
                        .find(|a| a.agent == alert.agent)
                        .and_then(|a| a.windows.iter().find(|w| w.window == alert.window))
                        .and_then(|w| w.last_notified_threshold);
                    if let Some(agent_mut) =
                        store.agents.iter_mut().find(|a| a.agent == alert.agent)
                    {
                        if let Some(win_mut) = agent_mut
                            .windows
                            .iter_mut()
                            .find(|w| w.window == alert.window)
                        {
                            win_mut.last_notified_threshold = prev_thresh;
                        }
                    }
                }
            }
        }
    }

    if let Err(err) = store.save(&cfg.state_file) {
        eprintln!(
            "[{}] [ERROR] 寫入暫存檔失敗 ({}): {}",
            format_local_datetime(now_unix()),
            cfg.state_file.display(),
            err
        );
    }
}

fn main() {
    let cfg = parse_args();
    println!(
        "[{}] AI 額度監控程序啟動｜間隔: {} 分鐘｜暫存檔: {}｜ntfy: {}/{}",
        format_local_datetime(now_unix()),
        cfg.interval_mins,
        cfg.state_file.display(),
        cfg.ntfy_server.trim_end_matches('/'),
        cfg.ntfy_topic
    );

    loop {
        poll_once(&cfg);
        if cfg.run_once || cfg.force_notify {
            break;
        }
        thread::sleep(Duration::from_secs(cfg.interval_mins * 60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::QuotaWindow;

    #[test]
    fn test_rfc3339_and_remaining_time() {
        let ts = time_util::parse_rfc3339("2026-10-23T23:00:00.307723+00:00").unwrap();
        let ts_z = time_util::parse_rfc3339("2026-10-23T23:00:00Z").unwrap();
        assert_eq!(ts, ts_z);

        let ts_tz = time_util::parse_rfc3339("2026-10-24T07:00:00+08:00").unwrap();
        assert_eq!(ts, ts_tz);

        assert_eq!(time_util::format_remaining(ts + 3660, ts), "1h01m");
        assert_eq!(
            time_util::format_remaining(ts + 86400 + 7200, ts),
            "1d02h00m"
        );
        assert_eq!(time_util::format_remaining(ts - 10, ts), "即將刷新");
    }

    #[test]
    fn test_threshold_dedup_and_reset() {
        let mut store = QuotaStateStore::default();
        let now = 1_700_000_000;
        let reset_t1 = now + 3600;

        let make_quota = |used: f64, reset_at: i64| {
            vec![AgentQuota {
                agent: "claude".to_string(),
                display_name: "Claude".to_string(),
                fetched_at: now,
                windows: vec![QuotaWindow {
                    window: "5h".to_string(),
                    used_percent: used,
                    remaining_percent: 100.0 - used,
                    reset_at: Some(reset_at),
                }],
            }]
        };

        // 1. 79% -> no alert
        let alerts = store.update_and_evaluate(make_quota(79.0, reset_t1), now);
        assert!(alerts.is_empty());

        // 2. 81% -> crosses 80% threshold -> 1 alert (threshold 80)
        let alerts = store.update_and_evaluate(make_quota(81.0, reset_t1), now + 60);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].threshold, 80);

        // 3. 84% in same reset cycle -> already notified for 80%, not yet 85% -> no alert
        let alerts = store.update_and_evaluate(make_quota(84.0, reset_t1), now + 120);
        assert!(alerts.is_empty());

        // 4. 86% in same reset cycle -> crosses 85% threshold -> 1 alert (threshold 85)
        let alerts = store.update_and_evaluate(make_quota(86.0, reset_t1), now + 180);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].threshold, 85);

        // 5. 96% in same reset cycle -> jumps straight to 95% -> 1 alert (threshold 95)
        let alerts = store.update_and_evaluate(make_quota(96.0, reset_t1), now + 240);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].threshold, 95);

        // 6. 97% in same reset cycle -> already notified at 95% -> no alert
        let alerts = store.update_and_evaluate(make_quota(97.0, reset_t1), now + 300);
        assert!(alerts.is_empty());

        // 7. Quota refreshes (new reset time and usage drops to 10%, then climbs to 82%)
        let reset_t2 = reset_t1 + 18000;
        let alerts = store.update_and_evaluate(make_quota(10.0, reset_t2), reset_t1 + 10);
        assert!(alerts.is_empty());

        let alerts = store.update_and_evaluate(make_quota(82.0, reset_t2), reset_t1 + 60);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].threshold, 80);
    }
}
