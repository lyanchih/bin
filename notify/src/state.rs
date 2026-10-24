use crate::json::{self, Json};
use crate::providers::AgentQuota;
use crate::time_util::{format_local_datetime, format_local_hhmm, format_remaining, now_unix};
use std::fs;
use std::path::Path;

pub const THRESHOLDS: [u8; 4] = [80, 85, 90, 95];

#[derive(Debug, Clone)]
pub struct WindowState {
    pub window: String,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub reset_at: Option<i64>,
    pub last_notified_threshold: Option<u8>,
    pub last_notified_reset_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AgentState {
    pub agent: String,
    pub display_name: String,
    pub fetched_at: i64,
    pub windows: Vec<WindowState>,
}

#[derive(Debug, Clone, Default)]
pub struct QuotaStateStore {
    pub updated_at: i64,
    pub agents: Vec<AgentState>,
}

#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub agent: String,
    pub display_name: String,
    pub window: String,
    pub threshold: u8,
    pub used_percent: f64,
    pub remaining_percent: f64,
    pub reset_at: Option<i64>,
}

pub fn highest_threshold_reached(used_percent: f64) -> Option<u8> {
    let mut reached = None;
    for &t in &THRESHOLDS {
        if used_percent + 1e-6 >= (t as f64) {
            reached = Some(t);
        }
    }
    reached
}

/// Determines whether the quota window has refreshed since the last notification.
fn is_quota_refreshed(
    prev: &WindowState,
    new_used_percent: f64,
    new_reset_at: Option<i64>,
    now_ts: i64,
) -> bool {
    // 1. If usage dropped below the lowest threshold (80%), reset notification state
    if highest_threshold_reached(new_used_percent).is_none() {
        return true;
    }

    // 2. Check if the previous reset timestamp has already passed or moved forward significantly
    if let Some(prev_rst) = prev.last_notified_reset_at.or(prev.reset_at) {
        if now_ts >= prev_rst {
            return true;
        }
        if let Some(curr_rst) = new_reset_at {
            // If reset_at jumped forward by more than 10 minutes, it's a new reset window
            if curr_rst > prev_rst + 600 {
                return true;
            }
        }
    }

    // 3. If usage dropped significantly (> 10%) compared to previous recorded usage
    if new_used_percent + 10.0 < prev.used_percent {
        return true;
    }

    false
}

impl QuotaStateStore {
    pub fn load(path: &Path) -> Self {
        let Ok(raw) = fs::read_to_string(path) else {
            return Self::default();
        };
        let Ok(doc) = json::parse(&raw) else {
            return Self::default();
        };
        Self::from_json(&doc).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let now_ts = now_unix();
        let json_val = self.to_json(now_ts);
        let content = json_val.to_string_pretty();

        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                let _ = fs::create_dir_all(parent);
            }
        }

        let tmp_path = path.with_extension("json.tmp");
        fs::write(&tmp_path, &content)
            .map_err(|e| format!("無法寫入暫存檔 {}: {e}", tmp_path.display()))?;
        fs::rename(&tmp_path, path)
            .map_err(|e| format!("無法更新暫存檔 {}: {e}", path.display()))?;
        Ok(())
    }

    /// Update state with freshly fetched quotas and return any newly triggered threshold alerts.
    pub fn update_and_evaluate(
        &mut self,
        fresh_quotas: Vec<AgentQuota>,
        now_ts: i64,
    ) -> Vec<AlertEvent> {
        let mut alerts = Vec::new();
        self.updated_at = now_ts;

        for fresh in fresh_quotas {
            let prev_agent = self.agents.iter().find(|a| a.agent == fresh.agent).cloned();
            let mut new_windows = Vec::new();

            for fw in fresh.windows {
                let prev_win = prev_agent
                    .as_ref()
                    .and_then(|a| a.windows.iter().find(|w| w.window == fw.window));

                let (mut last_notified_threshold, mut last_notified_reset_at) = match prev_win {
                    Some(pw) => {
                        if is_quota_refreshed(pw, fw.used_percent, fw.reset_at, now_ts) {
                            (None, None)
                        } else {
                            (pw.last_notified_threshold, pw.last_notified_reset_at)
                        }
                    }
                    None => (None, None),
                };

                if let Some(tier) = highest_threshold_reached(fw.used_percent) {
                    let prev_tier = last_notified_threshold.unwrap_or(0);
                    if tier > prev_tier {
                        alerts.push(AlertEvent {
                            agent: fresh.agent.clone(),
                            display_name: fresh.display_name.clone(),
                            window: fw.window.clone(),
                            threshold: tier,
                            used_percent: fw.used_percent,
                            remaining_percent: fw.remaining_percent,
                            reset_at: fw.reset_at,
                        });
                        last_notified_threshold = Some(tier);
                        last_notified_reset_at = fw.reset_at;
                    }
                }

                new_windows.push(WindowState {
                    window: fw.window,
                    used_percent: (fw.used_percent * 100.0).round() / 100.0,
                    remaining_percent: (fw.remaining_percent * 100.0).round() / 100.0,
                    reset_at: fw.reset_at,
                    last_notified_threshold,
                    last_notified_reset_at,
                });
            }

            let new_agent_state = AgentState {
                agent: fresh.agent.clone(),
                display_name: fresh.display_name,
                fetched_at: fresh.fetched_at,
                windows: new_windows,
            };

            if let Some(slot) = self.agents.iter_mut().find(|a| a.agent == fresh.agent) {
                *slot = new_agent_state;
            } else {
                self.agents.push(new_agent_state);
            }
        }

        // Keep consistent display order: claude, codex, antigravity
        self.agents.sort_by_key(|a| match a.agent.as_str() {
            "claude" => 0,
            "codex" => 1,
            "antigravity" => 2,
            _ => 3,
        });

        alerts
    }

    fn to_json(&self, now_ts: i64) -> Json {
        let agent_items: Vec<Json> = self
            .agents
            .iter()
            .map(|a| {
                let win_items: Vec<Json> = a
                    .windows
                    .iter()
                    .map(|w| {
                        let reset_hhmm = w
                            .reset_at
                            .map(format_local_hhmm)
                            .unwrap_or_else(|| "-".to_string());
                        let reset_rem = w
                            .reset_at
                            .map(|ts| format_remaining(ts, now_ts))
                            .unwrap_or_else(|| "-".to_string());

                        Json::Object(vec![
                            ("window".to_string(), Json::Str(w.window.clone())),
                            ("used_percent".to_string(), Json::Number(w.used_percent)),
                            (
                                "remaining_percent".to_string(),
                                Json::Number(w.remaining_percent),
                            ),
                            (
                                "reset_at_unix".to_string(),
                                w.reset_at
                                    .map(|v| Json::Number(v as f64))
                                    .unwrap_or(Json::Null),
                            ),
                            ("reset_at_hhmm".to_string(), Json::Str(reset_hhmm)),
                            ("reset_remaining".to_string(), Json::Str(reset_rem)),
                            (
                                "last_notified_threshold".to_string(),
                                w.last_notified_threshold
                                    .map(|v| Json::Number(v as f64))
                                    .unwrap_or(Json::Null),
                            ),
                            (
                                "last_notified_reset_at".to_string(),
                                w.last_notified_reset_at
                                    .map(|v| Json::Number(v as f64))
                                    .unwrap_or(Json::Null),
                            ),
                        ])
                    })
                    .collect();

                Json::Object(vec![
                    ("agent".to_string(), Json::Str(a.agent.clone())),
                    ("display_name".to_string(), Json::Str(a.display_name.clone())),
                    (
                        "fetched_at".to_string(),
                        Json::Str(format_local_datetime(a.fetched_at)),
                    ),
                    (
                        "fetched_at_unix".to_string(),
                        Json::Number(a.fetched_at as f64),
                    ),
                    ("windows".to_string(), Json::Array(win_items)),
                ])
            })
            .collect();

        Json::Object(vec![
            (
                "updated_at".to_string(),
                Json::Str(format_local_datetime(self.updated_at)),
            ),
            (
                "updated_at_unix".to_string(),
                Json::Number(self.updated_at as f64),
            ),
            ("agents".to_string(), Json::Array(agent_items)),
        ])
    }

    fn from_json(doc: &Json) -> Option<Self> {
        let updated_at = doc
            .get("updated_at_unix")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);
        let agents_arr = doc.get("agents")?.as_array()?;
        let mut agents = Vec::new();

        for item in agents_arr {
            let agent = item.get("agent")?.as_str()?.to_string();
            let display_name = item
                .get("display_name")
                .and_then(|v| v.as_str())
                .unwrap_or(&agent)
                .to_string();
            let fetched_at = item
                .get("fetched_at_unix")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let win_arr = item.get("windows")?.as_array()?;
            let mut windows = Vec::new();

            for w in win_arr {
                let window = w.get("window")?.as_str()?.to_string();
                let used_percent = w
                    .get("used_percent")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0);
                let remaining_percent = w
                    .get("remaining_percent")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.0 - used_percent);
                let reset_at = w.get("reset_at_unix").and_then(|v| v.as_i64());
                let last_notified_threshold =
                    w.get("last_notified_threshold").and_then(|v| v.as_u8());
                let last_notified_reset_at =
                    w.get("last_notified_reset_at").and_then(|v| v.as_i64());

                windows.push(WindowState {
                    window,
                    used_percent,
                    remaining_percent,
                    reset_at,
                    last_notified_threshold,
                    last_notified_reset_at,
                });
            }

            agents.push(AgentState {
                agent,
                display_name,
                fetched_at,
                windows,
            });
        }

        Some(QuotaStateStore { updated_at, agents })
    }
}
