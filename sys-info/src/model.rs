use crate::{hardware, host_type, mount::NetworkMount, platform, runtime};
use serde::Serialize;
use std::fmt::Write as _;
use std::net::Ipv4Addr;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Probe<T> = Result<T, String>;

#[derive(Serialize)]
pub struct Report {
    schema_version: u32,
    #[serde(skip_serializing_if = "Cpu::is_empty")]
    cpu: Cpu,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    gpu: Vec<Gpu>,
    #[serde(skip_serializing_if = "Ram::is_empty")]
    ram: Ram,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    disk: Vec<Disk>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    mount: Vec<NetworkMount>,
    distribution: Distribution,
    #[serde(skip_serializing_if = "Host::is_empty")]
    host: Host,
    #[serde(skip_serializing_if = "Network::is_empty")]
    network: Network,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    power: Vec<Power>,
    #[serde(skip_serializing_if = "Runtime::is_empty")]
    runtime: Runtime,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    error: Vec<CollectionError>,
}

impl Report {
    pub fn collect() -> Self {
        let mut errors = Errors::default();
        let (cpu, ram) = hardware::collect(&mut errors);
        let gpu = platform::gpu(&mut errors, ram.total);
        let disk = platform::disk(&mut errors);
        let mount = platform::mount(&mut errors);
        let distribution = platform::distribution(&mut errors);
        let mut host = platform::host(&mut errors);
        host.kind = errors.record("host.type", None, host_type::detect());
        let network = platform::network(&mut errors);
        let power = platform::power(&mut errors);
        let uptime = errors.record("runtime.uptime", None, platform::uptime());
        let boot_id = errors.record("runtime.boot_id", None, platform::boot_id());
        let (time_source, last_sync) = runtime::time_sync(&mut errors);
        #[cfg(unix)]
        let open_files_limit = errors
            .record(
                "runtime.open_files_limit",
                None,
                runtime::open_files_limit(),
            )
            .flatten();
        let system_time = errors.record(
            "runtime.system_time",
            None,
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs())
                .map_err(|error| error.to_string()),
        );
        Self {
            schema_version: 4,
            cpu,
            gpu,
            ram,
            disk,
            mount,
            distribution,
            host,
            network,
            power,
            runtime: Runtime {
                system_time,
                uptime,
                boot_id,
                time_source,
                last_sync,
                #[cfg(unix)]
                open_files_limit,
            },
            error: errors.0,
        }
    }
}

#[derive(Serialize)]
struct Runtime {
    #[serde(skip_serializing_if = "Option::is_none")]
    system_time: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uptime: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    boot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time_source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_sync: Option<u64>,
    #[cfg(unix)]
    #[serde(skip_serializing_if = "Option::is_none")]
    open_files_limit: Option<libc::rlim_t>,
}

impl Runtime {
    fn is_empty(&self) -> bool {
        #[cfg(unix)]
        if self.open_files_limit.is_some() {
            return false;
        }
        self.system_time.is_none()
            && self.uptime.is_none()
            && self.boot_id.is_none()
            && self.time_source.is_none()
            && self.last_sync.is_none()
    }
}

#[derive(Serialize)]
pub struct Cpu {
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<&'static str>,
}

impl Cpu {
    fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.band.is_none()
            && self.frequency.is_none()
            && self.arch.is_none()
            && self.count.is_none()
            && self.features.is_empty()
    }
}

#[derive(Serialize)]
pub struct Ram {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
}

impl Ram {
    fn is_empty(&self) -> bool {
        self.total.is_none()
    }
}

#[derive(Serialize)]
pub struct Gpu {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pci_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_kind: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_limit: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compute_capability: Option<String>,
}

#[derive(Serialize)]
pub struct Disk {
    pub mount_point: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesystem: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<u64>,
}

impl Disk {
    pub fn eligible(
        os: bool,
        mount: &str,
        capacity: &Probe<(u64, u64)>,
        errors: &mut Errors,
    ) -> bool {
        match capacity {
            Ok((total, _)) if os => *total >= 1_000_000_000,
            Ok((total, _)) => *total > 500_000_000_000,
            Err(_) if os => true,
            Err(error) => {
                errors.add("disk.space", Some(mount), error.as_str());
                false
            }
        }
    }

    #[cfg(unix)]
    pub fn from_space(
        mount_point: String,
        filesystem: Probe<String>,
        space: Probe<(u64, u64)>,
        errors: &mut Errors,
    ) -> Self {
        let usage = space.and_then(|(total, free)| {
            total
                .checked_sub(free)
                .map(|used| (total, used))
                .ok_or_else(|| "filesystem reported inconsistent capacity".into())
        });
        Self::from_usage(mount_point, filesystem, usage, errors)
    }

    pub fn from_usage(
        mount_point: String,
        filesystem: Probe<String>,
        usage: Probe<(u64, u64)>,
        errors: &mut Errors,
    ) -> Self {
        let filesystem = errors.record("disk.filesystem", Some(&mount_point), filesystem);
        let space = errors.record(
            "disk.space",
            Some(&mount_point),
            usage.and_then(|(total, used)| {
                if total == 0 || used > total {
                    Err("filesystem reported inconsistent capacity".into())
                } else {
                    Ok((total, used))
                }
            }),
        );
        Self {
            mount_point,
            filesystem,
            total: space.map(|(total, _)| total),
            used: space.map(|(_, used)| used).filter(|&used| used > 0),
        }
    }
}

#[derive(Serialize)]
pub struct Distribution {
    pub os: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dis: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libc: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub libc_version: Option<String>,
}

#[derive(Serialize)]
pub struct Host {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<host_type::HostType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fqdn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub uid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gid: Option<u32>,
}

impl Host {
    fn is_empty(&self) -> bool {
        self.kind.is_none()
            && self.fqdn.is_none()
            && self.domain.is_none()
            && self.hostname.is_none()
            && self.user.is_none()
            && self.uid.is_none()
            && self.gid.is_none()
    }
}

#[derive(Serialize, Default)]
pub struct Network {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_gateway: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<Interface>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dns_server: Option<String>,
}

impl Network {
    pub fn set_default_route(
        &mut self,
        ip: Option<String>,
        gateway: Option<Ipv4Addr>,
        errors: &mut Errors,
    ) {
        self.default_gateway = gateway.map(|gateway| gateway.to_string());
        self.external_ip = errors.require(
            "network.external_ip",
            None,
            ip,
            "the first IPv4 default route interface has no IPv4 address",
        );
    }

    pub fn is_empty(&self) -> bool {
        self.external_ip.is_none()
            && self.default_gateway.is_none()
            && self.interfaces.is_empty()
            && self.dns_server.is_none()
    }

    pub fn consider_dns(&mut self, server: Ipv4Addr) {
        if self.dns_server.is_none() && !server.is_loopback() {
            self.dns_server = Some(server.to_string());
        }
    }

    pub fn check(&self, errors: &mut Errors) {
        if self.interfaces.is_empty() {
            errors.add(
                "network.interfaces",
                None,
                "no network interfaces available",
            );
        }
        if self.dns_server.is_none() {
            errors.add(
                "network.dns_server",
                None,
                "no non-loopback IPv4 DNS servers available",
            );
        }
        for interface in &self.interfaces {
            if interface.addresses.is_empty() {
                errors.add(
                    "network.ip",
                    Some(&interface.name),
                    "no IPv4 addresses assigned",
                );
            }
            if !interface.loopback && interface.macaddress.is_none() {
                errors.add(
                    "network.macaddress",
                    Some(&interface.name),
                    "link-layer address unavailable",
                );
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct DefaultRoute<T> {
    pub interface: T,
    pub gateway: Option<Ipv4Addr>,
}

// Preserve priority and the order of tied interfaces as reported by the OS.
pub fn best_routes<T: PartialEq>(
    routes: impl IntoIterator<Item = (u64, DefaultRoute<T>)>,
) -> Vec<DefaultRoute<T>> {
    let mut best: Vec<DefaultRoute<T>> = Vec::new();
    let mut lowest = u64::MAX;
    for (cost, route) in routes {
        if cost < lowest {
            best.clear();
            lowest = cost;
        }
        if cost == lowest
            && !best
                .iter()
                .any(|previous| previous.interface == route.interface)
        {
            best.push(route);
        }
    }
    best
}

#[derive(Serialize, Default)]
pub struct Interface {
    pub name: String,
    #[serde(skip)]
    pub loopback: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macaddress: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<Address>,
}

#[derive(Serialize, PartialEq, Eq)]
pub struct Address {
    pub ip: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub netmask: Option<u8>,
}

#[derive(Serialize)]
pub struct Power {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watt: Option<f64>,
}

#[derive(Default)]
pub struct Errors(Vec<CollectionError>);

#[derive(Serialize)]
struct CollectionError {
    field: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    message: String,
}

impl Errors {
    pub fn add(&mut self, field: &'static str, subject: Option<&str>, message: impl Into<String>) {
        self.0.push(CollectionError {
            field,
            subject: subject.map(str::to_owned),
            message: message.into(),
        });
    }

    pub fn record<T>(
        &mut self,
        field: &'static str,
        subject: Option<&str>,
        value: Probe<T>,
    ) -> Option<T> {
        match value {
            Ok(value) => Some(value),
            Err(message) => {
                self.add(field, subject, message);
                None
            }
        }
    }

    pub fn require<T>(
        &mut self,
        field: &'static str,
        subject: Option<&str>,
        value: Option<T>,
        message: &str,
    ) -> Option<T> {
        self.record(field, subject, value.ok_or_else(|| message.to_owned()))
    }
}

pub fn nonempty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub fn mac_address(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.iter().all(|&byte| byte == 0) {
        return None;
    }
    let mut text = String::new();
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 {
            text.push(':');
        }
        let _ = write!(text, "{byte:02x}");
    }
    Some(text)
}

#[cfg(test)]
mod disk_tests {
    use super::{Disk, Errors};

    #[test]
    fn disk_capacity_thresholds_prioritize_os_and_keep_large_data_volumes() {
        let mut errors = Errors::default();
        let mut eligible = |os, bytes| Disk::eligible(os, "/", &Ok((bytes, 0)), &mut errors);
        assert!(!eligible(true, 999_999_999));
        assert!(eligible(true, 1_000_000_000));
        assert!(!eligible(false, 500_000_000_000));
        assert!(eligible(false, 500_000_000_001));
    }
}

#[cfg(test)]
mod network_tests {
    use super::{Ipv4Addr, Network};

    #[test]
    fn dns_reports_only_the_first_non_loopback_ipv4_address() {
        let mut network = Network::default();
        for octets in [
            [127, 0, 0, 0],
            [127, 0, 0, 1],
            [127, 0, 0, 53],
            [127, 255, 255, 255],
        ] {
            network.consider_dns(Ipv4Addr::from(octets));
        }
        assert!(
            serde_json::to_value(&network)
                .unwrap()
                .get("dns_server")
                .is_none()
        );
        for octets in [[10, 0, 0, 53], [1, 1, 1, 1], [10, 0, 0, 53]] {
            network.consider_dns(Ipv4Addr::from(octets));
        }
        assert_eq!(
            serde_json::to_value(&network).unwrap(),
            serde_json::json!({"dns_server": "10.0.0.53"})
        );
    }
}
