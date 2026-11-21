use flate2::read::GzDecoder;
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::process::Command;
use std::time::Duration;

#[test]
fn cli_emits_inventory_in_all_four_formats() {
    let mut previous_boot_id: Option<Value> = None;
    for (args, cbor, gzip) in [
        (&[][..], false, false),
        (&["-t", "cbor"][..], true, false),
        (
            &["--content-type=json-gzip", "-p", "stdout"][..],
            false,
            true,
        ),
        (&["--content-type", "cbor-gzip"][..], true, true),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sys-info"))
            .args(args)
            .output()
            .expect("run sys-info");
        assert!(output.status.success(), "{:?}", output.stderr);
        assert!(output.stderr.is_empty());
        let body = if gzip {
            let mut body = Vec::new();
            GzDecoder::new(output.stdout.as_slice())
                .read_to_end(&mut body)
                .expect("decompress gzip report");
            body
        } else {
            output.stdout
        };
        let report: Value = if cbor {
            let mut input = body.as_slice();
            let report = ciborium::from_reader(&mut input).expect("decode CBOR");
            assert!(input.is_empty(), "expected one CBOR document");
            report
        } else {
            if !gzip {
                assert!(body.ends_with(b"\n"));
            }
            serde_json::from_slice(&body).expect("decode JSON")
        };
        assert_eq!(report["schema_version"], 4);
        assert!(report["runtime"]["system_time"].is_u64());
        assert!(report["runtime"]["uptime"].is_u64());
        if let Some(boot_id) = report["runtime"].get("boot_id") {
            assert!(boot_id.as_str().is_some_and(|value| !value.is_empty()));
            if let Some(previous) = &previous_boot_id {
                assert_eq!(boot_id, previous);
            }
            previous_boot_id = Some(boot_id.clone());
        } else {
            assert!(
                report["error"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|error| error["field"] == "runtime.boot_id")
            );
        }
        assert!(report.get("uptime_seconds").is_none());
        assert!(report["runtime"].get("uptime_seconds").is_none());
        assert!(report["runtime"].get("system_time_unix_seconds").is_none());
        assert_has_no_empty_fields(&report);
        if let Some(user) = report["host"].get("user") {
            assert!(user.as_str().is_some_and(|name| !name.is_empty()));
        } else {
            assert!(
                report["error"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|error| error["field"] == "host.user")
            );
        }
        #[cfg(unix)]
        {
            assert_eq!(report["host"]["uid"], unsafe { libc::geteuid() });
            assert_eq!(report["host"]["gid"], unsafe { libc::getegid() });
        }
        #[cfg(windows)]
        {
            assert!(report["host"].get("uid").is_none());
            assert!(report["host"].get("gid").is_none());
        }
        if let Some(kind) = report["host"].get("type") {
            assert!(matches!(
                kind.as_str(),
                Some("baremetal" | "vm" | "microvm" | "container" | "lambda" | "wsl")
            ));
        }
        for mount in report["mount"].as_array().into_iter().flatten() {
            assert!(mount["mount_point"].is_string());
            assert!(
                mount
                    .as_object()
                    .expect("mount")
                    .keys()
                    .all(|key| matches!(key.as_str(), "mount_point" | "filesystem" | "source"))
            );
        }
        let cpu = &report["cpu"];
        assert!(cpu.get("architecture").is_none());
        for key in ["band", "type", "arch"] {
            if let Some(value) = cpu.get(key) {
                assert!(value.is_string(), "{key}");
            }
        }
        for key in ["frequency", "count"] {
            if let Some(value) = cpu.get(key) {
                assert!(value.as_u64().is_some_and(|value| value > 0), "{key}");
            }
        }
        for gpu in report["gpu"].as_array().into_iter().flatten() {
            assert!(gpu.get("model").is_none());
            for key in ["band", "type"] {
                if let Some(value) = gpu.get(key) {
                    assert!(value.is_string(), "{key}");
                }
            }
        }

        let is_ipv4 = |ip: &Value| {
            ip.as_str()
                .is_some_and(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok())
        };
        let network = &report["network"];
        let external_ip = network.get("external_ip");
        if let Some(gateway) = network.get("default_gateway") {
            assert!(is_ipv4(gateway));
            assert_ne!(gateway, "0.0.0.0");
        }
        let mut found_external_ip = false;
        for interface in network["interfaces"].as_array().into_iter().flatten() {
            assert!(interface.get("up").is_none());
            assert!(interface.get("loopback").is_none());
            for address in interface["addresses"].as_array().into_iter().flatten() {
                assert!(is_ipv4(&address["ip"]));
                found_external_ip |= external_ip == Some(&address["ip"]);
                if let Some(netmask) = address.get("netmask") {
                    assert!(netmask.as_u64().is_some_and(|prefix| prefix <= 32));
                }
            }
        }
        if let Some(ip) = external_ip {
            assert!(is_ipv4(ip) && found_external_ip);
        }
        assert!(network.get("dns_servers").is_none());
        if let Some(server) = network.get("dns_server") {
            assert!(
                server
                    .as_str()
                    .and_then(|ip| ip.parse::<std::net::Ipv4Addr>().ok())
                    .is_some_and(|ip| !ip.is_loopback())
            );
        }
        for error in report["error"].as_array().into_iter().flatten() {
            assert!(error["field"].is_string());
            assert!(error["message"].is_string());
        }
    }
}

fn assert_has_no_empty_fields(value: &Value) {
    match value {
        Value::Object(object) => {
            assert!(!object.is_empty());
            for (key, value) in object {
                assert!(!key.ends_with("_bytes"), "old capacity key {key}");
                if matches!(key.as_str(), "size" | "total" | "used" | "shared_limit") {
                    assert!(value.as_u64().is_some_and(|bytes| bytes > 0), "{key}");
                }
                assert_has_no_empty_fields(value);
            }
        }
        Value::Array(items) => {
            assert!(!items.is_empty());
            items.iter().for_each(assert_has_no_empty_fields);
        }
        Value::String(text) => assert!(!text.is_empty()),
        Value::Null => panic!("null value in inventory"),
        _ => {}
    }
}

#[test]
fn host_user_uses_the_system_account_instead_of_environment_variables() {
    let run = |user: &str| {
        let output = Command::new(env!("CARGO_BIN_EXE_sys-info"))
            .env("USER", user)
            .env("LOGNAME", user)
            .env("USERNAME", user)
            .output()
            .expect("run sys-info");
        assert!(output.status.success());
        serde_json::from_slice::<Value>(&output.stdout).expect("decode JSON")
    };
    let first = run("sys-info-test-account-1");
    let second = run("sys-info-test-account-2");
    assert_eq!(first["host"].get("user"), second["host"].get("user"));
    if let Some(user) = first["host"].get("user") {
        assert!(
            !user
                .as_str()
                .unwrap_or_default()
                .starts_with("sys-info-test-account-")
        );
    }
}

#[test]
fn lambda_runtime_takes_priority_in_host_type() {
    let output = Command::new(env!("CARGO_BIN_EXE_sys-info"))
        .env("AWS_LAMBDA_FUNCTION_NAME", "inventory-test")
        .env("AWS_LAMBDA_RUNTIME_API", "127.0.0.1:9001")
        .output()
        .expect("run sys-info");
    assert!(output.status.success());
    let report: Value = serde_json::from_slice(&output.stdout).expect("decode JSON");
    assert_eq!(report["host"]["type"], "lambda");
}

#[test]
fn invalid_options_have_no_inventory_output() {
    for args in [
        &["--content-type", "xml"][..],
        &["--protocol=udp"],
        &["-p", "http"],
        &["-p", "coap://user:password@localhost/inventory"],
        &["-p", "mqtt://localhost/"],
        &["-p", "mqtt://localhost/sensors/%23"],
        &["-t", "json", "--content-type=cbor"],
        &["--endpoint", "http://localhost/inventory"],
        &["-e", "http://localhost/inventory"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_sys-info"))
            .args(args)
            .output()
            .expect("run sys-info");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert!(!output.stderr.is_empty(), "{args:?}");
    }
}

#[test]
fn repeated_protocols_deliver_in_order_and_continue_after_rejection() {
    let server = TcpListener::bind("127.0.0.1:0").expect("bind HTTP test server");
    let address = server.local_addr().expect("get HTTP test server address");
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let reports = ["409 Conflict", "204 No Content"].map(|status| {
            let (mut connection, _) = server.accept().expect("accept HTTP request");
            connection
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("set HTTP test timeout");
            let mut reader = BufReader::new(&mut connection);
            let mut request_line = String::new();
            reader
                .read_line(&mut request_line)
                .expect("read request line");
            let mut length = None;
            let mut authorization = None;
            loop {
                let mut header = String::new();
                assert_ne!(reader.read_line(&mut header).expect("read header"), 0);
                if header == "\r\n" {
                    break;
                }
                let (name, value) = header.split_once(':').expect("parse request header");
                if name.eq_ignore_ascii_case("Content-Length") {
                    length = Some(value.trim().parse().expect("parse body length"));
                }
                if name.eq_ignore_ascii_case("Authorization") {
                    authorization = Some(value.trim().to_owned());
                }
            }
            let mut body = vec![0; length.expect("request body length")];
            reader.read_exact(&mut body).expect("read HTTP report");
            write!(
                connection,
                "HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .expect("send HTTP response");
            (request_line, authorization, body)
        });
        sender.send(reports).expect("return received reports");
    });
    let output = Command::new(env!("CARGO_BIN_EXE_sys-info"))
        .args([
            "-p",
            &format!("http://user:p%40ss@{address}/first"),
            "--protocol",
            &format!("http://{address}/second?source=cli"),
            "-p",
            "stdout",
        ])
        .env("NO_PROXY", "127.0.0.1")
        .env("no_proxy", "127.0.0.1")
        .env_remove("HTTP_PROXY")
        .env_remove("http_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .output()
        .expect("run sys-info");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).expect("decode diagnostics");
    assert!(stderr.contains("destination 1 (http)") && stderr.contains("409"));
    assert!(!stderr.contains("p%40ss") && !stderr.contains("p@ss"));
    let [first, second] = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("receive both HTTP reports");
    assert_eq!(first.0, "POST /first HTTP/1.1\r\n");
    assert_eq!(first.1.as_deref(), Some("Basic dXNlcjpwQHNz"));
    assert_eq!(second.0, "POST /second?source=cli HTTP/1.1\r\n");
    assert_eq!(second.1, None);
    assert_eq!(first.2, second.2);
    assert_eq!(output.stdout.strip_suffix(b"\n"), Some(first.2.as_slice()));
}
