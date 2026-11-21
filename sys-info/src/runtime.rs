use crate::model::Errors;

#[cfg(unix)]
pub fn open_files_limit() -> crate::model::Probe<Option<libc::rlim_t>> {
    let mut limit = std::mem::MaybeUninit::<libc::rlimit>::uninit();
    // On success getrlimit initializes the result.
    if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let limit = unsafe { limit.assume_init() }.rlim_cur;
    Ok((limit != libc::RLIM_INFINITY).then_some(limit))
}

pub fn time_sync(errors: &mut Errors) -> (Option<String>, Option<u64>) {
    #[cfg(target_os = "linux")]
    {
        use crate::execute_binary::{Query, execute};
        use std::fs;
        use std::path::Path;

        let source = if Path::new("/run/systemd/system").is_dir() {
            execute(Query::TimeSource)
                .ok_or_else(|| "no allowlisted time service query for this architecture".to_owned())
                .and_then(|result| result)
                .and_then(|result| {
                    let value = result.trim();
                    (!matches!(value, "" | "n/a") && !value.contains(char::is_whitespace))
                        .then(|| value.to_owned())
                        .ok_or_else(|| "systemd-timesyncd has no active NTP server".into())
                })
        } else {
            Err("systemd time service unavailable".into())
        };
        let age = fs::metadata("/run/systemd/timesync/synchronized")
            .and_then(|metadata| metadata.modified())
            .map_err(|error| format!("systemd-timesyncd synchronization marker: {error}"))
            .and_then(|timestamp| {
                timestamp
                    .elapsed()
                    .map(|duration| duration.as_secs())
                    .map_err(|_| "last synchronization is later than the system clock".into())
            });
        (
            errors.record("runtime.time_source", None, source),
            errors.record("runtime.last_sync", None, age),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        for field in ["runtime.time_source", "runtime.last_sync"] {
            errors.add(
                field,
                None,
                "active NTP synchronization details unavailable on this platform",
            );
        }
        (None, None)
    }
}
