use crate::model::Probe;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Query {
    GpuSize,
}

struct Rule {
    field: Query,
    os: &'static str,
    arch: &'static str,
    paths: &'static [&'static str],
    args: &'static [&'static str],
}

const NVIDIA_QUERY: &[&str] = &[
    "--query-gpu=pci.bus_id,memory.total",
    "--format=csv,noheader,nounits",
];

const RULES: &[Rule] = &[
    Rule {
        field: Query::GpuSize,
        os: "linux",
        arch: "x86_64",
        paths: &[
            "/usr/bin/nvidia-smi",
            "/usr/local/nvidia/bin/nvidia-smi",
            "/usr/lib/wsl/lib/nvidia-smi",
        ],
        args: NVIDIA_QUERY,
    },
    Rule {
        field: Query::GpuSize,
        os: "linux",
        arch: "aarch64",
        paths: &["/usr/bin/nvidia-smi", "/usr/local/nvidia/bin/nvidia-smi"],
        args: NVIDIA_QUERY,
    },
];

const TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 64 * 1024;

fn rule(field: Query, os: &str, arch: &str) -> Option<&'static Rule> {
    RULES
        .iter()
        .find(|rule| rule.field == field && rule.os == os && rule.arch == arch)
}

/// Returns None when there is no command for this field and target.
pub fn execute(field: Query) -> Option<Probe<String>> {
    let rule = rule(field, std::env::consts::OS, std::env::consts::ARCH)?;
    Some(run(rule))
}

fn run(rule: &Rule) -> Probe<String> {
    let mut failures = Vec::new();
    for path in rule.paths {
        let child = Command::new(path)
            .args(rule.args)
            .env_clear()
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        match child {
            Ok(mut child) => {
                let result = capture(&mut child, TIMEOUT);
                if result.is_err() {
                    stop_child(child);
                }
                return result.map_err(|error| format!("{path}: {error}"));
            }
            Err(error) => failures.push(format!("{path}: {error}")),
        }
    }
    Err(format!(
        "no allowlisted executable could be started ({})",
        failures.join(", ")
    ))
}

fn capture(child: &mut Child, timeout: Duration) -> Probe<String> {
    let mut stdout = Capture::new(child.stdout.take().ok_or("stdout pipe unavailable")?)?;
    let mut stderr = Capture::new(child.stderr.take().ok_or("stderr pipe unavailable")?)?;
    let start = Instant::now();
    loop {
        stdout.read_available()?;
        stderr.read_available()?;
        let status = child.try_wait().map_err(|error| error.to_string())?;
        if let Some(status) = status
            && stdout.closed
            && stderr.closed
        {
            if status.success() {
                return String::from_utf8(stdout.bytes).map_err(|error| error.to_string());
            }
            let detail = [stderr.bytes, stdout.bytes].into_iter().find_map(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
                Some(line.chars().take(240).collect::<String>())
            });
            return Err(match detail {
                Some(detail) => format!("{status}: {detail}"),
                None => status.to_string(),
            });
        }

        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return Err(format!(
                "execution timed out after {} ms",
                timeout.as_millis()
            ));
        }
        thread::sleep(remaining.min(Duration::from_millis(5)));
    }
}

fn stop_child(mut child: Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    let _ = child.kill();
    // A driver call can leave a child blocked in the kernel. Reap it without
    // making the inventory wait for termination.
    let _ = thread::Builder::new().spawn(move || {
        let _ = child.wait();
    });
}

struct Capture<T> {
    pipe: T,
    bytes: Vec<u8>,
    closed: bool,
}

impl<T: Read + AsRawFd> Capture<T> {
    fn new(pipe: T) -> Probe<Self> {
        let fd = pipe.as_raw_fd();
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        Ok(Self {
            pipe,
            bytes: Vec::new(),
            closed: false,
        })
    }

    fn read_available(&mut self) -> Probe<()> {
        if self.closed {
            return Ok(());
        }
        let mut chunk = [0u8; 4096];
        loop {
            match self.pipe.read(&mut chunk) {
                Ok(0) => {
                    self.closed = true;
                    return Ok(());
                }
                Ok(count) if self.bytes.len() + count <= OUTPUT_LIMIT => {
                    self.bytes.extend_from_slice(&chunk[..count]);
                }
                Ok(_) => return Err("command output exceeded the 64 KiB limit".into()),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{OUTPUT_LIMIT, Query, capture, rule, stop_child};
    use std::io::Write;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    const FIXTURE: &str = "SYS_INFO_BINARY_TEST_FIXTURE";

    fn fixture(mode: &str) -> Child {
        Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "execute_binary::tests::child_fixture",
                "--nocapture",
            ])
            .env(FIXTURE, mode)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn fixture")
    }

    #[test]
    fn child_fixture() {
        match std::env::var(FIXTURE).as_deref() {
            Ok("timeout") => std::thread::sleep(Duration::from_secs(10)),
            Ok("oversize") => {
                let _ = std::io::stdout().write_all(&vec![b'x'; OUTPUT_LIMIT + 1]);
            }
            Ok("failure") => {
                eprintln!("GPU query failed");
                std::process::exit(7);
            }
            _ => {}
        }
    }

    #[test]
    fn command_failure_is_bounded_and_captures_exit_diagnostics() {
        for (mode, timeout, expected) in [
            ("timeout", Duration::from_millis(50), "timed out"),
            ("oversize", Duration::from_secs(2), "output exceeded"),
            ("failure", Duration::from_secs(2), "GPU query failed"),
        ] {
            let mut child = fixture(mode);
            let result = capture(&mut child, timeout);
            stop_child(child);
            let error = result.expect_err("fixture must fail");
            assert!(error.contains(expected), "{mode}: {error}");
        }
    }

    #[test]
    fn only_the_gpu_size_query_on_supported_linux_targets_is_allowlisted() {
        for arch in ["x86_64", "aarch64"] {
            let rule = rule(Query::GpuSize, "linux", arch).expect("known target");
            assert!(rule.paths.iter().all(|path| path.starts_with('/')));
            assert_eq!(
                rule.args,
                [
                    "--query-gpu=pci.bus_id,memory.total",
                    "--format=csv,noheader,nounits"
                ]
            );
        }
        assert!(rule(Query::GpuSize, "linux", "riscv64").is_none());
        assert!(rule(Query::GpuSize, "macos", "aarch64").is_none());
        assert!(rule(Query::GpuSize, "windows", "x86_64").is_none());
    }
}
