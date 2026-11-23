use crate::json::Json;
use std::time::{SystemTime, UNIX_EPOCH};

#[repr(C)]
#[derive(Default)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

unsafe extern "C" {
    fn localtime_r(timep: *const i64, result: *mut Tm) -> *mut Tm;
}

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn to_local_tm(ts: i64) -> Option<Tm> {
    let mut tm = Tm::default();
    let ptr = unsafe { localtime_r(&ts as *const i64, &mut tm as *mut Tm) };
    if ptr.is_null() {
        None
    } else {
        Some(tm)
    }
}

/// Format unix timestamp to local `HH:MM` (絕對時間只需要到時分不用日)
pub fn format_local_hhmm(ts: i64) -> String {
    if let Some(tm) = to_local_tm(ts) {
        format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
    } else {
        "--:--".to_string()
    }
}

/// Format unix timestamp to local `YYYY-MM-DD HH:MM:SS` for console logs & state file
pub fn format_local_datetime(ts: i64) -> String {
    if let Some(tm) = to_local_tm(ts) {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    } else {
        ts.to_string()
    }
}

/// Format remaining duration until `reset_ts` relative to `now_ts`
pub fn format_remaining(reset_ts: i64, now_ts: i64) -> String {
    let diff = reset_ts - now_ts;
    if diff <= 0 {
        return "即將刷新".to_string();
    }
    let days = diff / 86400;
    let rem = diff % 86400;
    let hours = rem / 3600;
    let mins = (rem % 3600) / 60;
    if days > 0 {
        format!("{}d{:02}h{:02}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h{:02}m", hours, mins)
    } else {
        format!("{}m", mins.max(1))
    }
}

/// Parse either a numeric Unix timestamp or an ISO-8601 / RFC3339 timestamp string into Unix epoch seconds.
pub fn parse_timestamp(val: &Json) -> Option<i64> {
    match val {
        Json::Number(n) => {
            let v = *n as i64;
            if v > 0 {
                Some(v)
            } else {
                None
            }
        }
        Json::Str(s) => {
            if let Ok(v) = s.trim().parse::<i64>() {
                return Some(v);
            }
            parse_rfc3339(s.trim())
        }
        _ => None,
    }
}

/// Parse RFC3339 / ISO-8601 strings such as:
/// - "2026-10-23T23:00:00.307723+00:00"
/// - "2026-10-23T21:18:07Z"
/// - "2026-10-24T06:00:00+08:00"
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    if s.len() < 19 {
        return None;
    }
    let year: i64 = s.get(0..4)?.parse().ok()?;
    if s.as_bytes().get(4).copied()? != b'-' {
        return None;
    }
    let month: i64 = s.get(5..7)?.parse().ok()?;
    if s.as_bytes().get(7).copied()? != b'-' {
        return None;
    }
    let day: i64 = s.get(8..10)?.parse().ok()?;
    let sep = s.as_bytes().get(10).copied()?;
    if sep != b'T' && sep != b't' && sep != b' ' {
        return None;
    }
    let hour: i64 = s.get(11..13)?.parse().ok()?;
    if s.as_bytes().get(13).copied()? != b':' {
        return None;
    }
    let minute: i64 = s.get(14..16)?.parse().ok()?;
    if s.as_bytes().get(16).copied()? != b':' {
        return None;
    }
    let second: i64 = s.get(17..19)?.parse().ok()?;

    let mut rest = &s[19..];
    // Skip fractional seconds if present (.123456)
    if let Some(stripped) = rest.strip_prefix('.') {
        let digits_len = stripped
            .bytes()
            .take_while(|b| b.is_ascii_digit())
            .count();
        rest = &stripped[digits_len..];
    }

    let offset_secs: i64 = if rest.is_empty() || rest.eq_ignore_ascii_case("Z") {
        0
    } else {
        let sign = match rest.as_bytes().first().copied()? {
            b'+' => 1_i64,
            b'-' => -1_i64,
            _ => return None,
        };
        let tz = &rest[1..];
        let (tz_h, tz_m) = if let Some((h, m)) = tz.split_once(':') {
            (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?)
        } else if tz.len() == 4 {
            (
                tz.get(0..2)?.parse::<i64>().ok()?,
                tz.get(2..4)?.parse::<i64>().ok()?,
            )
        } else if tz.len() == 2 {
            (tz.parse::<i64>().ok()?, 0)
        } else {
            return None;
        };
        sign * (tz_h * 3600 + tz_m * 60)
    };

    let days = days_from_civil(year, month, day);
    let utc_secs = days * 86400 + hour * 3600 + minute * 60 + second - offset_secs;
    Some(utc_secs)
}

/// Howard Hinnant's civil-to-days algorithm (days since 1970-01-01 UTC)
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m_adj = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * m_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}
