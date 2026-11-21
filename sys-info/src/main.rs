mod delivery;
mod dynamic_library;
#[cfg(target_os = "linux")]
mod execute_binary;
mod hardware;
mod host_type;
mod model;
mod mount;

#[cfg(unix)]
mod unix;

#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;

use delivery::{ContentType, Destination};
use model::Report;
use std::io::{self, Write};
use std::process::ExitCode;

const USAGE: &str = "Usage: sys-info [OPTIONS]

  -t, --content-type <TYPE>  json (default), cbor, json-gzip, cbor-gzip
  -p, --protocol <URI>       http://, https://, coap://, mqtt://, or stdout
                             Repeat to deliver in order (default: stdout)
  -h, --help                 Show this help
";

struct Options {
    content_type: ContentType,
    destinations: Vec<Destination>,
}

fn parse_options(args: &[String]) -> Result<Option<Options>, String> {
    let mut content_type = None;
    let mut destinations = Vec::new();
    let mut args = args.iter().map(String::as_str);
    while let Some(arg) = args.next() {
        let (flag, value) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (arg, None),
        };
        let mut next_value = || {
            value
                .or_else(|| args.next().filter(|next| !next.starts_with('-')))
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag {
            "-h" | "--help" if value.is_none() => return Ok(None),
            "-t" | "--content-type" if content_type.is_none() => {
                content_type = Some(ContentType::parse(next_value()?)?);
            }
            "-p" | "--protocol" => destinations.push(Destination::parse(next_value()?)?),
            _ => return Err("unrecognized or repeated option".into()),
        }
    }
    if destinations.is_empty() {
        destinations.push(Destination::Stdout);
    }
    Ok(Some(Options {
        content_type: content_type.unwrap_or_default(),
        destinations,
    }))
}

fn main() -> ExitCode {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let options = match parse_options(&args) {
        Ok(options) => options,
        Err(error) => {
            let _ = write!(io::stderr().lock(), "sys-info: {error}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let result = match options {
        Some(options) => return deliver(options),
        None => {
            let mut output = io::stdout().lock();
            output
                .write_all(USAGE.as_bytes())
                .and_then(|()| output.flush())
                .map_err(|error| error.to_string())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "sys-info: {error}");
            ExitCode::FAILURE
        }
    }
}

fn deliver(options: Options) -> ExitCode {
    let body = match options.content_type.encode(&Report::collect()) {
        Ok(body) => body,
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "sys-info: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut status = ExitCode::SUCCESS;
    for (index, destination) in options.destinations.into_iter().enumerate() {
        let protocol = destination.protocol();
        if let Err(error) = destination.deliver(options.content_type, &body) {
            let _ = writeln!(
                io::stderr().lock(),
                "sys-info: destination {} ({protocol}): {error}",
                index + 1
            );
            status = ExitCode::FAILURE;
        }
    }
    status
}
