#[path = "linux/gpu_api.rs"]
mod gpu_api;

use crate::hardware;
use crate::host_type::{self, HostType};
use crate::model::{
    DefaultRoute, Disk, Distribution, Errors, Gpu, Host, Network, Power, Probe, best_routes,
    nonempty,
};
use crate::mount::{NetworkMount, is_network_filesystem};
use crate::unix;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::CString;
use std::fs;
use std::io;
use std::net::Ipv4Addr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub fn architecture() -> String {
    sysinfo::System::cpu_arch()
}

pub fn cpu_base_frequency() -> Option<u64> {
    let policies = fs::read_dir("/sys/devices/system/cpu/cpufreq").ok()?;
    policies
        .filter_map(Result::ok)
        .filter_map(|policy| {
            if !policy.file_name().as_encoded_bytes().starts_with(b"policy") {
                return None;
            }
            let khz = read(&policy.path().join("base_frequency"))
                .ok()?
                .parse::<u64>()
                .ok()?;
            (khz > 0).then_some(khz.checked_mul(1_000)?)
        })
        .max()
}

pub fn host_type() -> Probe<HostType> {
    let container_env = host_type::env_value("container")
        .is_some_and(|value| !matches!(value.as_str(), "none" | "wsl"));
    let container_file = ["/.dockerenv", "/run/.containerenv"]
        .iter()
        .any(|path| Path::new(path).exists())
        || read(Path::new("/run/systemd/container"))
            .is_ok_and(|kind| !matches!(kind.as_str(), "none" | "wsl"));
    let container_cgroup = fs::read_to_string("/proc/self/cgroup").is_ok_and(|value| {
        [
            "/docker/",
            "/docker-",
            "/kubepods",
            "/libpod-",
            "/lxc/",
            "/lxc.payload.",
            "/cri-containerd-",
        ]
        .iter()
        .any(|marker| value.contains(marker))
    });
    if container_env || container_file || container_cgroup {
        return Ok(HostType::Container);
    }
    let wsl = read(Path::new("/proc/sys/kernel/osrelease"))
        .is_ok_and(|release| release.to_ascii_lowercase().contains("microsoft"))
        || host_type::env_value("WSL_INTEROP").is_some();
    if wsl {
        return Ok(HostType::Wsl);
    }

    let identity = [
        "/sys/class/dmi/id/sys_vendor",
        "/sys/class/dmi/id/product_name",
        "/sys/class/dmi/id/bios_vendor",
        "/sys/hypervisor/type",
        "/sys/firmware/devicetree/base/model",
        "/sys/firmware/devicetree/base/compatible",
    ]
    .iter()
    .filter_map(|path| read(Path::new(path)).ok())
    .collect::<Vec<_>>()
    .join(" ");
    if let Some(kind) = host_type::virtual_machine(&identity) {
        return Ok(kind);
    }

    #[cfg(target_arch = "x86_64")]
    if crate::hardware::cpuid(1).ecx & (1 << 31) != 0 {
        return Ok(HostType::Vm);
    }
    if identity.trim_matches(['\0', ' ']).is_empty() {
        return Err("host virtualization could not be identified".into());
    }
    Ok(HostType::Baremetal)
}

pub fn distribution(errors: &mut Errors) -> Distribution {
    let release = fs::read_to_string("/etc/os-release").or_else(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            fs::read_to_string("/usr/lib/os-release")
        } else {
            Err(error)
        }
    });
    let release = errors.record(
        "distribution",
        None,
        release.map_err(|error| error.to_string()),
    );
    let values = release
        .as_deref()
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key, release_value(value)?))
        })
        .collect::<BTreeMap<_, _>>();
    let mut distribution = hardware::distribution(
        "linux",
        values.get("ID").cloned(),
        values.get("NAME").cloned(),
        values
            .get("VERSION_ID")
            .or_else(|| values.get("VERSION"))
            .cloned(),
        errors,
    );
    if let Some((kind, version)) = errors.record("distribution.libc", None, system_libc()) {
        distribution.libc = Some(kind);
        distribution.libc_version = errors.require(
            "distribution.libc_version",
            None,
            version,
            "system libc version unavailable",
        );
    } else {
        errors.add(
            "distribution.libc_version",
            None,
            "system libc could not be identified",
        );
    }
    distribution
}

fn system_libc() -> Probe<(&'static str, Option<String>)> {
    use crate::execute_binary::{Query, execute};

    let output =
        execute(Query::LibcVersion).ok_or("no system libc query for this architecture")??;
    parse_system_libc(&output)
}

fn parse_system_libc(output: &str) -> Probe<(&'static str, Option<String>)> {
    let first = output.lines().next().unwrap_or_default();
    let (kind, version) = if first.starts_with("musl libc (") {
        (
            "musl",
            output
                .lines()
                .find_map(|line| line.strip_prefix("Version ")),
        )
    } else if first.starts_with("ldd (")
        && [
            "glibc",
            "GNU libc",
            "GNU C Library",
            "GNU libc6",
            "Debian",
            "Ubuntu",
            "GLIBC",
        ]
        .iter()
        .any(|label| first.contains(label))
    {
        ("glibc", first.split_whitespace().last())
    } else {
        return Err("could not recognize the system libc from ldd".into());
    };
    let version = version
        .filter(|value| {
            value.contains('.')
                && value
                    .split('.')
                    .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        })
        .map(str::to_owned);
    Ok((kind, version))
}

fn release_value(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with('\'') {
        return nonempty(value.strip_prefix('\'')?.strip_suffix('\'')?);
    }
    let quoted = value.starts_with('"');
    let value = if quoted {
        value.strip_prefix('"')?.strip_suffix('"')?
    } else {
        value
    };
    let mut chars = value.chars();
    let mut decoded = String::new();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let escaped = chars.next()?;
            if quoted && !matches!(escaped, '"' | '\\' | '`' | '$') {
                decoded.push(ch);
            }
            decoded.push(escaped);
        } else {
            decoded.push(ch);
        }
    }
    nonempty(&decoded)
}

pub fn host(errors: &mut Errors) -> Host {
    unix::host(errors, || None)
}

pub fn network(errors: &mut Errors) -> Network {
    let routes = errors
        .record("network.default_route", None, default_interfaces())
        .unwrap_or_default();
    let interfaces = unix::selected_interfaces(errors, &routes, physical_interfaces);
    let mut network = Network {
        interfaces: errors
            .record("network.interfaces", None, interfaces)
            .unwrap_or_default(),
        ..Default::default()
    };
    match fs::read_to_string("/etc/resolv.conf") {
        Ok(resolver) => {
            for line in resolver.lines() {
                let mut words = line
                    .split(['#', ';'])
                    .next()
                    .unwrap_or_default()
                    .split_whitespace();
                if words.next() == Some("nameserver") {
                    match unix::dns_address(words.next().unwrap_or_default()) {
                        Ok(Some(address)) => network.consider_dns(address),
                        Ok(None) => {}
                        Err(_) => errors.add(
                            "network.dns_server",
                            None,
                            "invalid resolver nameserver entry",
                        ),
                    }
                }
            }
        }
        Err(error) => errors.add("network.dns_server", None, error.to_string()),
    }
    if let Some(route) = routes.first() {
        let ip = network
            .interfaces
            .iter()
            .find(|interface| interface.name == route.interface)
            .and_then(|interface| interface.addresses.first())
            .map(|address| address.ip.clone());
        network.set_default_route(ip, route.gateway, errors);
    }
    network.check(errors);
    network
}

fn default_interfaces() -> Probe<Vec<DefaultRoute<String>>> {
    let routes = fs::read_to_string("/proc/net/route").map_err(|error| error.to_string())?;
    parse_routes(&routes)
}

fn parse_routes(table: &str) -> Probe<Vec<DefaultRoute<String>>> {
    const RTF_UP: u32 = 0x1;
    const RTF_REJECT: u32 = 0x200;
    let mut lines = table.lines();
    if !lines
        .next()
        .is_some_and(|header| header.starts_with("Iface"))
    {
        return Err("invalid IPv4 routing table header".into());
    }
    let mut routes = Vec::new();
    for line in lines {
        let columns: Vec<_> = line.split_ascii_whitespace().collect();
        let [name, destination, gateway, flags, _, _, metric, mask, ..] = columns.as_slice() else {
            return Err("incomplete IPv4 routing table entry".into());
        };
        if *destination != "00000000" || *mask != "00000000" {
            continue;
        }
        let flags = u32::from_str_radix(flags, 16).map_err(|error| error.to_string())?;
        if flags & (RTF_UP | RTF_REJECT) == RTF_UP {
            let metric = metric
                .parse()
                .map_err(|error: std::num::ParseIntError| error.to_string())?;
            let gateway = u32::from_str_radix(gateway, 16).map_err(|error| error.to_string())?;
            // /proc/net/route renders IPv4 addresses as native-endian hexadecimal integers.
            let gateway = (gateway != 0).then(|| Ipv4Addr::from(gateway.to_ne_bytes()));
            routes.push((
                metric,
                DefaultRoute {
                    interface: (*name).to_owned(),
                    gateway,
                },
            ));
        }
    }
    Ok(best_routes(routes))
}

fn physical_interfaces() -> Probe<BTreeSet<String>> {
    let devices = fs::read_dir("/sys/class/net").map_err(|error| error.to_string())?;
    let mut physical = BTreeSet::new();
    for device in devices {
        let device = device.map_err(|error| error.to_string())?;
        match fs::canonicalize(device.path()) {
            Ok(path)
                if path.starts_with("/sys/devices")
                    && !path.starts_with("/sys/devices/virtual") =>
            {
                physical.insert(device.file_name().to_string_lossy().into_owned());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(physical)
}

pub fn uptime() -> Probe<u64> {
    let mut time = std::mem::MaybeUninit::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, time.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    u64::try_from(unsafe { time.assume_init() }.tv_sec).map_err(|error| error.to_string())
}

pub fn boot_id() -> Probe<String> {
    read(Path::new("/proc/sys/kernel/random/boot_id")).map_err(|error| error.to_string())
}

struct MountInfo<'a> {
    filesystem: &'a [u8],
    source: &'a [u8],
    device: &'a [u8],
    path: &'a [u8],
    options: &'a [u8],
}

fn mount_table() -> Probe<Vec<u8>> {
    fs::read("/proc/self/mountinfo").map_err(|error| error.to_string())
}

fn mount_entries(table: &[u8]) -> impl Iterator<Item = Probe<MountInfo<'_>>> {
    table
        .split(|&byte| byte == b'\n')
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            let separator = entry
                .windows(3)
                .position(|bytes| bytes == b" - ")
                .ok_or_else(|| "invalid mountinfo entry".to_owned())?;
            let mut info = entry[separator + 3..].split(|&byte| byte == b' ');
            let mut fields = entry[..separator].split(|&byte| byte == b' ').skip(2);
            let (Some(filesystem), Some(source), Some(options), Some(device), Some(path)) = (
                info.next(),
                info.next(),
                info.next(),
                fields.next(),
                fields.nth(1),
            ) else {
                return Err("incomplete mountinfo entry".into());
            };
            Ok(MountInfo {
                filesystem,
                source,
                device,
                path,
                options,
            })
        })
}

pub fn mount(errors: &mut Errors) -> Vec<NetworkMount> {
    let Some(table) = errors.record("mount", None, mount_table()) else {
        return Vec::new();
    };
    network_mounts(&table, errors)
}

fn network_mounts(table: &[u8], errors: &mut Errors) -> Vec<NetworkMount> {
    let mut mounts = BTreeMap::new();
    for entry in mount_entries(table) {
        let Some(entry) = errors.record("mount", None, entry) else {
            continue;
        };
        let filesystem = String::from_utf8_lossy(entry.filesystem);
        let network_9p = filesystem == "9p"
            && entry
                .options
                .split(|&byte| byte == b',')
                .any(|option| matches!(option, b"trans=tcp" | b"trans=rdma"));
        if !is_network_filesystem(&filesystem) && !network_9p {
            continue;
        }
        let target = String::from_utf8_lossy(&mount_path(entry.path)).into_owned();
        let source = mount_path(entry.source);
        mounts.insert(
            target.clone(),
            NetworkMount::new(
                target,
                Some(filesystem.into_owned()),
                &String::from_utf8_lossy(&source),
            ),
        );
    }
    mounts.into_values().collect()
}

pub fn disk(errors: &mut Errors) -> Vec<Disk> {
    let Some(mounts) = errors.record("disk", None, mount_table()) else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for entry in mount_entries(&mounts) {
        let Some(MountInfo {
            filesystem,
            source,
            device,
            path,
            ..
        }) = errors.record("disk", None, entry)
        else {
            continue;
        };
        let root = path == b"/";
        let path = mount_path(path);
        let mount = Path::new(std::ffi::OsStr::from_bytes(&path));
        if mount.starts_with("/boot")
            || mount.starts_with("/efi")
            || source.starts_with(b"/dev/loop")
            || is_network_filesystem(&String::from_utf8_lossy(filesystem))
            || filesystem == b"9p"
        {
            continue;
        }
        if source.starts_with(b"/dev/")
            || filesystem == b"zfs"
            || (root && filesystem == b"overlay")
        {
            candidates.push((!root, path, filesystem, source, device));
        }
    }
    candidates.sort_unstable();
    let mut disks = Vec::new();
    let mut seen = BTreeSet::new();
    for (data, path, filesystem, source, device) in candidates {
        let mount = String::from_utf8_lossy(&path).into_owned();
        let volume = (
            filesystem,
            if filesystem == b"btrfs" {
                source
            } else {
                device
            },
        );
        if seen.contains(&volume) {
            continue;
        }
        let space = disk_space(path);
        if !Disk::eligible(!data, &mount, &space, errors) {
            continue;
        }
        if data && boot_partition(device) {
            continue;
        }
        if filesystem != b"overlay" {
            let internal = errors.record(
                "disk.device",
                Some(&mount),
                internal_device(device, source, filesystem),
            );
            if internal == Some(false) || (data && internal.is_none()) {
                continue;
            }
        }
        seen.insert(volume);
        disks.push(Disk::from_space(
            mount,
            nonempty(&String::from_utf8_lossy(filesystem))
                .ok_or_else(|| "filesystem type unavailable".into()),
            space,
            errors,
        ));
    }
    if disks.is_empty() {
        errors.add("disk", None, "no eligible mounted filesystems available");
    }
    disks
}

fn boot_partition(device: &[u8]) -> bool {
    let Ok(device) = std::str::from_utf8(device) else {
        return false;
    };
    let Ok(properties) = fs::read_to_string(format!("/run/udev/data/b{device}")) else {
        return false;
    };
    properties
        .lines()
        .filter_map(|line| line.strip_prefix("E:ID_PART_ENTRY_TYPE="))
        .any(|kind| {
            matches!(
                kind.to_ascii_lowercase().as_str(),
                "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"
                    | "21686148-6449-6e6f-744e-656564454649"
                    | "de94bba4-06d1-4d40-a16a-bfd50179d6ac"
                    | "0xef"
                    | "0x27"
            )
        })
}

fn internal_device(device: &[u8], source: &[u8], filesystem: &[u8]) -> Probe<bool> {
    let device = std::str::from_utf8(device).map_err(|error| error.to_string())?;
    let path = fs::canonicalize(Path::new("/sys/dev/block").join(device))
        // Filesystems such as Btrfs can report an anonymous mount device.
        .or_else(|_| {
            let source =
                fs::canonicalize(Path::new(std::ffi::OsStr::from_bytes(&mount_path(source))))?;
            let name = source
                .file_name()
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
            fs::canonicalize(Path::new("/sys/class/block").join(name))
        })
        .map_err(|error| format!("block device topology unavailable: {error}"))?;
    let mut visited = BTreeSet::new();
    if filesystem == b"btrfs" {
        let members = btrfs_devices(&path)?;
        for member in members {
            if !internal_block(&member, &mut visited)? {
                return Ok(false);
            }
        }
        return Ok(true);
    }
    internal_block(&path, &mut visited)
}

fn btrfs_devices(device: &Path) -> Probe<Vec<PathBuf>> {
    let pools = fs::read_dir("/sys/fs/btrfs").map_err(|error| error.to_string())?;
    for pool in pools {
        let pool = pool.map_err(|error| error.to_string())?;
        let members = match fs::read_dir(pool.path().join("devices")) {
            Ok(members) => members,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        let members: Vec<_> = members
            .map(|entry| fs::canonicalize(entry?.path()))
            .collect::<io::Result<_>>()
            .map_err(|error| error.to_string())?;
        if members.iter().any(|member| member == device) {
            return Ok(members);
        }
    }
    Err("Btrfs backing devices unavailable".into())
}

fn internal_block(path: &Path, visited: &mut BTreeSet<PathBuf>) -> Probe<bool> {
    let path = if path.join("partition").exists() {
        path.parent()
            .ok_or_else(|| "missing parent block device".to_owned())?
    } else {
        path
    };
    if !visited.insert(path.to_owned()) {
        return Ok(true);
    }
    let mut has_slaves = false;
    let slaves = fs::read_dir(path.join("slaves")).map_err(|error| error.to_string())?;
    for slave in slaves {
        let slave = slave
            .and_then(|entry| fs::canonicalize(entry.path()))
            .map_err(|error| error.to_string())?;
        has_slaves = true;
        if !internal_block(&slave, visited)? {
            return Ok(false);
        }
    }
    if has_slaves {
        return Ok(true);
    }
    if path.starts_with("/sys/devices/virtual") {
        return Ok(false);
    }
    let removable = read(&path.join("removable")).map_err(|error| error.to_string())?;
    if removable != "0" || read(&path.join("device/type")).is_ok_and(|kind| kind == "SD") {
        return Ok(false);
    }
    for node in path
        .ancestors()
        .take_while(|node| node.starts_with("/sys/devices"))
    {
        let external_bus = fs::read_link(node.join("subsystem"))
            .ok()
            .and_then(|bus| bus.file_name().map(|name| name.to_owned()))
            .is_some_and(|name| {
                matches!(
                    name.to_str(),
                    Some("usb" | "firewire" | "thunderbolt" | "iscsi_session" | "iscsi_connection")
                )
            });
        let network_transport = read(&node.join("transport"))
            .is_ok_and(|transport| matches!(transport.as_str(), "tcp" | "rdma" | "fc" | "loop"));
        if external_bus
            || network_transport
            || read(&node.join("external_facing")).is_ok_and(|flag| flag == "1")
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn mount_path(mut bytes: &[u8]) -> Vec<u8> {
    let mut path = Vec::new();
    while let Some((&first, rest)) = bytes.split_first() {
        if let [
            b'\\',
            a @ b'0'..=b'3',
            b @ b'0'..=b'7',
            c @ b'0'..=b'7',
            tail @ ..,
        ] = bytes
        {
            path.push((a - b'0') * 64 + (b - b'0') * 8 + (c - b'0'));
            bytes = tail;
        } else {
            path.push(first);
            bytes = rest;
        }
    }
    path
}

fn disk_space(path: Vec<u8>) -> Probe<(u64, u64)> {
    let path = CString::new(path).map_err(|error| error.to_string())?;
    let mut stats = std::mem::MaybeUninit::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    let stats = unsafe { stats.assume_init() };
    let size = stats.f_frsize;
    stats
        .f_blocks
        .checked_mul(size)
        .zip(stats.f_bfree.checked_mul(size))
        .ok_or_else(|| "filesystem capacity overflowed".into())
}

pub fn gpu(errors: &mut Errors, _ram_bytes: Option<u64>) -> Vec<Gpu> {
    let mut devices = BTreeSet::new();
    for entry in directory("/sys/bus/pci/devices", "gpu", errors) {
        match read(&entry.join("class")) {
            Ok(class) if class.starts_with("0x03") => {
                if let Some(path) = gpu_path(&entry, errors) {
                    devices.insert(path);
                }
            }
            Ok(_) => {}
            Err(error) => errors.add(
                "gpu",
                entry.file_name().and_then(|name| name.to_str()),
                error.to_string(),
            ),
        }
    }
    for entry in directory("/sys/class/drm", "gpu", errors) {
        let card = entry
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("card"))
            .is_some_and(|index| {
                !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
            });
        if card && let Some(path) = gpu_path(&entry.join("device"), errors) {
            devices.insert(path);
        }
    }

    let pci_ids = [
        "/usr/share/hwdata/pci.ids",
        "/usr/share/misc/pci.ids",
        "/usr/share/pci.ids",
    ]
    .into_iter()
    .find_map(|path| fs::read_to_string(path).ok());
    let api = gpu_api::GpuApi::default();
    let gpus = devices
        .into_iter()
        .map(|path| {
            let id = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned();
            let driver = fs::read_link(path.join("driver")).ok().and_then(|link| {
                link.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            });
            let vendor = read(&path.join("vendor")).ok();
            let device = read(&path.join("device")).ok();
            let dynamic = if driver.as_deref() == Some("nvidia") {
                api.nvidia(&id, errors)
            } else {
                gpu_api::NvidiaDetails::default()
            };
            let model = dynamic
                .model
                .or_else(|| read(&path.join("product_name")).ok())
                .or_else(|| {
                    let info = fs::read_to_string(
                        Path::new("/proc/driver/nvidia/gpus")
                            .join(&id)
                            .join("information"),
                    )
                    .ok()?;
                    info.lines()
                        .find_map(|line| nonempty(line.strip_prefix("Model:")?))
                })
                .or_else(|| pci_name(pci_ids.as_deref()?, vendor.as_deref()?, device.as_deref()?))
                .or_else(|| {
                    let model = fs::read(path.join("of_node/model")).ok()?;
                    nonempty(String::from_utf8_lossy(&model).trim_end_matches('\0'))
                });
            let version = dynamic
                .version
                .or_else(|| read(&path.join("driver/module/version")).ok())
                .or_else(|| {
                    let module = driver.as_deref()?;
                    read(&Path::new("/sys/module").join(module).join("version")).ok()
                });
            let memory = dynamic
                .memory
                .or_else(|| {
                    if driver.as_deref() == Some("nvidia") {
                        api.binary_memory(&id, errors)
                    } else {
                        None
                    }
                })
                .or_else(|| {
                    ["mem_info_vram_total", "vram_total"]
                        .into_iter()
                        .find_map(|name| {
                            read(&path.join(name))
                                .ok()?
                                .parse::<u64>()
                                .ok()
                                .filter(|&size| size > 0)
                        })
                });
            let memory = match (memory, driver.as_deref()) {
                (Some(size), _) => Ok(size),
                (None, Some("nouveau")) => gpu_api::nouveau_memory(&path),
                (None, _) => {
                    Err("GPU memory capacity unavailable from kernel or driver API".into())
                }
            };
            let size_bytes = errors.record("gpu.size", Some(&id), memory);
            let vendor_id = vendor
                .as_deref()
                .and_then(|id| u32::from_str_radix(id.strip_prefix("0x")?, 16).ok());
            let band = hardware::gpu_band(vendor_id, model.as_deref())
                .or_else(|| pci_vendor(pci_ids.as_deref()?, vendor.as_deref()?));
            let compute_capability = if vendor_id == Some(0x10de) {
                let capability = dynamic.compute_capability.or_else(|| {
                    if driver.as_deref() == Some("nvidia") {
                        api.binary_compute(&id, errors)
                    } else {
                        None
                    }
                });
                errors.require(
                    "gpu.compute_capability",
                    Some(&id),
                    capability,
                    "CUDA compute capability unavailable",
                )
            } else {
                None
            };
            Gpu {
                band: errors.require("gpu.band", Some(&id), band, "GPU manufacturer unavailable"),
                model: errors.require(
                    "gpu.type",
                    Some(&id),
                    model.as_deref().map(hardware::simple_gpu_type),
                    "GPU model unavailable",
                ),
                pci_id: vendor.as_deref().and_then(|vendor| {
                    Some(format!(
                        "{}:{}",
                        vendor.strip_prefix("0x")?,
                        device.as_deref()?.strip_prefix("0x")?,
                    ))
                }),
                driver,
                version: errors.require(
                    "gpu.version",
                    Some(&id),
                    version,
                    "GPU driver version unavailable",
                ),
                size: size_bytes,
                memory_kind: size_bytes.map(|_| "dedicated"),
                shared_limit: None,
                compute_capability,
                id,
            }
        })
        .collect::<Vec<_>>();
    #[cfg(target_env = "gnu")]
    api.finish(errors);
    if gpus.is_empty() {
        errors.add("gpu", None, "no GPU detected through PCI or DRM");
    }
    gpus
}

fn gpu_path(path: &Path, errors: &mut Errors) -> Option<PathBuf> {
    errors.record(
        "gpu",
        path.to_str(),
        fs::canonicalize(path).map_err(|error| error.to_string()),
    )
}

fn pci_vendor(table: &str, vendor: &str) -> Option<String> {
    let vendor = vendor.strip_prefix("0x")?;
    table
        .lines()
        .filter_map(|line| line.strip_prefix(vendor))
        .find(|rest| rest.starts_with(char::is_whitespace))
        .and_then(nonempty)
}

fn pci_name(table: &str, vendor: &str, device: &str) -> Option<String> {
    let vendor = vendor.strip_prefix("0x")?;
    let device = device.strip_prefix("0x")?;
    let mut lines = table.lines().skip_while(|line| {
        line.starts_with('\t') || line.split_whitespace().next() != Some(vendor)
    });
    let vendor_line = lines.next()?;
    let vendor_name = vendor_line.strip_prefix(vendor)?.trim();
    for line in lines {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix('\t')?;
        if let Some(model) = line
            .strip_prefix(device)
            .filter(|rest| rest.starts_with(char::is_whitespace))
        {
            return nonempty(&format!("{vendor_name} {}", model.trim()));
        }
    }
    None
}

pub fn power(errors: &mut Errors) -> Vec<Power> {
    let mut power = Vec::new();
    for supply in directory("/sys/class/power_supply", "power", errors) {
        let source = format!(
            "power_supply:{}",
            supply
                .file_name()
                .unwrap_or(supply.as_os_str())
                .to_string_lossy()
        );
        let kind = read(&supply.join("type"));
        let Some(kind) = errors.record(
            "power",
            Some(&source),
            kind.map_err(|error| error.to_string()),
        ) else {
            continue;
        };
        let sample = read_number(&supply.join("power_now"))
            .map(|micro_watt| micro_watt.abs() / 1_000_000.0)
            .or_else(|error| {
                if kind == "Battery" {
                    let current = read_number(&supply.join("current_now"))?;
                    let voltage = read_number(&supply.join("voltage_now"))?;
                    Ok(current.abs() * voltage.abs() / 1_000_000_000_000.0)
                } else {
                    Err(error)
                }
            });
        // Most AC adapters report only whether they are online.
        if kind == "Battery" || sample.is_ok() {
            power.push(Power {
                watt: record_watt(sample, &source, errors),
                source,
            });
        }
    }
    for monitor in directory("/sys/class/hwmon", "power", errors) {
        let name = read(&monitor.join("name")).unwrap_or_else(|_| "unknown".into());
        let id = monitor
            .file_name()
            .unwrap_or(monitor.as_os_str())
            .to_string_lossy();
        let Some(sensors) = errors.record(
            "power",
            Some(&id),
            fs::read_dir(&monitor).map_err(|error| error.to_string()),
        ) else {
            continue;
        };
        for sensor in sensors {
            let sensor = match sensor {
                Ok(sensor) => sensor.path(),
                Err(error) => {
                    errors.add("power", Some(&id), error.to_string());
                    continue;
                }
            };
            let Some(channel) = sensor
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("power"))
                .and_then(|name| name.strip_suffix("_input"))
            else {
                continue;
            };
            let label = read(&monitor.join(format!("power{channel}_label")))
                .unwrap_or_else(|_| format!("power{channel}"));
            let source = format!("hwmon:{id}:{name}:{label}");
            let sample = read_number(&sensor).map(|micro_watt| micro_watt.abs() / 1_000_000.0);
            power.push(Power {
                watt: record_watt(sample, &source, errors),
                source,
            });
        }
    }
    power.sort_by(|a, b| a.source.cmp(&b.source));
    if power.is_empty() {
        errors.add(
            "power.watt",
            None,
            "no battery or hwmon power telemetry available",
        );
    }
    power
}

fn record_watt(sample: Probe<f64>, source: &str, errors: &mut Errors) -> Option<f64> {
    errors.record(
        "power.watt",
        Some(source),
        sample.and_then(|sample| {
            sample
                .is_finite()
                .then_some(sample)
                .ok_or_else(|| "invalid wattage".into())
        }),
    )
}

fn read(path: &Path) -> io::Result<String> {
    fs::read_to_string(path).and_then(|text| {
        nonempty(&text).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "empty value"))
    })
}

fn read_number(path: &Path) -> Probe<f64> {
    let number = read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    number
        .parse::<i64>()
        .map(|value| value as f64)
        .map_err(|error| format!("{}: {error}", path.display()))
}

fn directory(path: &str, field: &'static str, errors: &mut Errors) -> Vec<PathBuf> {
    match fs::read_dir(path) {
        Ok(entries) => entries
            .filter_map(|entry| {
                errors.record(
                    field,
                    Some(path),
                    entry
                        .map(|entry| entry.path())
                        .map_err(|error| error.to_string()),
                )
            })
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            errors.add(field, Some(path), error.to_string());
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{network_mounts, parse_routes, parse_system_libc};
    use crate::model::Errors;

    #[test]
    fn lists_network_mounts_without_probing_remote_capacity() {
        let table = br"1 0 8:1 / / rw - ext4 /dev/sda1 rw
2 1 0:2 / /run rw - tmpfs tmpfs rw
3 1 0:3 / /mnt/team\040share rw - nfs4 nas:/export/team rw
4 1 0:4 / /mnt/smb rw - cifs //alice@files/share rw
5 1 0:5 / /mnt/guest rw - 9p hostshare rw,trans=virtio
6 1 0:6 / /mnt/remote9p rw - 9p server rw,trans=tcp
7 1 0:7 / /mnt/pending rw - autofs systemd-1 rw
";
        let mounts = network_mounts(table, &mut Errors::default());
        assert_eq!(
            mounts
                .iter()
                .map(|mount| mount.mount_point.as_str())
                .collect::<Vec<_>>(),
            ["/mnt/remote9p", "/mnt/smb", "/mnt/team share"]
        );
        assert_eq!(mounts[1].filesystem.as_deref(), Some("cifs"));
        assert_eq!(mounts[1].source.as_deref(), Some("//files/share"));
        assert_eq!(mounts[2].source.as_deref(), Some("nas:/export/team"));
    }

    #[test]
    fn default_routes_use_only_the_lowest_metric_and_keep_ties() {
        let table = "Iface Destination Gateway Flags RefCnt Use Metric Mask MTU Window IRTT\n\
                     en0 00000000 0101A8C0 0003 0 0 100 00000000 0 0 0\n\
                     tun0 00000000 00000000 0001 0 0 50 00000000 0 0 0\n\
                     en1 00000000 0101A8C0 0003 0 0 50 00000000 0 0 0\n\
                     down0 00000000 00000000 0002 0 0 0 00000000 0 0 0\n\
                     reject0 00000000 00000000 0201 0 0 0 00000000 0 0 0\n\
                     en2 00000000 00000000 0001 0 0 0 00000080 0 0 0";
        let routes = parse_routes(table).expect("valid IPv4 routing table");
        assert_eq!(
            routes
                .iter()
                .map(|route| route.interface.as_str())
                .collect::<Vec<_>>(),
            ["tun0", "en1"]
        );
        assert_eq!(routes[0].gateway, None);
        assert_eq!(routes[1].gateway, Some("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn route_table_can_have_no_ipv4_default() {
        let table = "Iface Destination Gateway Flags RefCnt Use Metric Mask\n\
                     en0 0001A8C0 00000000 0001 0 0 100 00FFFFFF";
        assert!(
            parse_routes(table)
                .expect("valid IPv4 routing table")
                .is_empty()
        );
        assert!(parse_routes("Iface\nen0").is_err());
    }
    #[test]
    fn recognizes_glibc_and_musl_from_system_version_output() {
        assert_eq!(
            parse_system_libc("ldd (Ubuntu GLIBC 2.39-0ubuntu8.7) 2.39\n").unwrap(),
            ("glibc", Some("2.39".into()))
        );
        assert_eq!(
            parse_system_libc("musl libc (aarch64)\nVersion 1.2.5\nDynamic Program Loader\n")
                .unwrap(),
            ("musl", Some("1.2.5".into()))
        );
        assert!(parse_system_libc("BusyBox ldd\n").is_err());
    }
}
