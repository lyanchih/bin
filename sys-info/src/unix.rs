use crate::model::{Address, DefaultRoute, Errors, Host, Interface, Probe, mac_address, nonempty};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::ptr;

pub fn host(errors: &mut Errors, fallback_fqdn: impl FnOnce() -> Option<String>) -> Host {
    let mut buffer = [0u8; 256];
    let hostname = if unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) } == 0 {
        CStr::from_bytes_until_nul(&buffer)
            .ok()
            .and_then(|name| nonempty(&name.to_string_lossy()))
            .ok_or_else(|| "system hostname is empty or truncated".into())
    } else {
        Err(io::Error::last_os_error().to_string())
    };
    let hostname = errors.record("host.hostname", None, hostname);
    let fqdn = hostname
        .as_deref()
        .and_then(canonical_name)
        .or_else(fallback_fqdn);
    let domain = fqdn
        .as_deref()
        .and_then(|name| name.split_once('.'))
        .map(|(_, domain)| domain.to_owned());
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    Host {
        kind: None,
        user: errors.record("host.user", None, effective_user(uid)),
        uid: Some(uid),
        gid: Some(gid),
        hostname,
        fqdn: errors.require(
            "host.fqdn",
            None,
            fqdn,
            "no fully qualified host name available",
        ),
        domain: errors.require("host.domain", None, domain, "no host DNS domain available"),
    }
}

fn effective_user(uid: libc::uid_t) -> Probe<String> {
    let suggested = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    let mut size = usize::try_from(suggested)
        .ok()
        .filter(|&size| size > 0)
        .unwrap_or(1024);
    let mut buffer = Vec::new();
    loop {
        buffer
            .try_reserve_exact(size - buffer.len())
            .map_err(|error| error.to_string())?;
        buffer.resize(size, 0);
        let mut entry = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = ptr::null_mut();
        let status = unsafe {
            libc::getpwuid_r(
                uid,
                entry.as_mut_ptr(),
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut result,
            )
        };
        match status {
            0 if result.is_null() => {
                return Err(format!("no system account for effective UID {uid}"));
            }
            0 => {
                // A successful lookup owns the name in the caller's buffer.
                let name = unsafe { (*result).pw_name };
                if name.is_null() {
                    return Err(format!("account name unavailable for effective UID {uid}"));
                }
                let name = unsafe { CStr::from_ptr(name) };
                return nonempty(&name.to_string_lossy())
                    .ok_or_else(|| format!("empty account name for effective UID {uid}"));
            }
            libc::ERANGE => {
                size = size
                    .checked_mul(2)
                    .ok_or_else(|| "system account buffer size overflowed".to_owned())?;
            }
            error => return Err(io::Error::from_raw_os_error(error).to_string()),
        }
    }
}

fn canonical_name(hostname: &str) -> Option<String> {
    fn qualified(name: &str) -> Option<String> {
        let name = name.trim_end_matches('.');
        let (host, domain) = name.split_once('.')?;
        (!host.is_empty() && !domain.is_empty() && name.parse::<IpAddr>().is_err())
            .then(|| name.to_owned())
    }
    if let Some(fqdn) = qualified(hostname) {
        return Some(fqdn);
    }

    let hostname = CString::new(hostname).ok()?;
    let mut hints: libc::addrinfo = unsafe { std::mem::zeroed() };
    hints.ai_flags = libc::AI_CANONNAME;
    hints.ai_socktype = libc::SOCK_STREAM;
    let mut result = ptr::null_mut();
    if unsafe { libc::getaddrinfo(hostname.as_ptr(), ptr::null(), &hints, &mut result) } != 0 {
        return None;
    }
    unsafe {
        let name = result.as_ref().and_then(|info| {
            (!info.ai_canonname.is_null()).then(|| CStr::from_ptr(info.ai_canonname))
        });
        let fqdn = name.and_then(|name| qualified(&name.to_string_lossy()));
        if !result.is_null() {
            libc::freeaddrinfo(result);
        }
        fqdn
    }
}

pub fn selected_interfaces(
    errors: &mut Errors,
    routes: &[DefaultRoute<String>],
    physical_interfaces: impl FnOnce() -> Probe<BTreeSet<String>>,
) -> Probe<Vec<Interface>> {
    let physical = if routes.is_empty() {
        errors
            .record("network.interface_type", None, physical_interfaces())
            .unwrap_or_default()
    } else {
        BTreeSet::new()
    };
    interfaces(errors, |name, up, loopback| {
        include_interface(name, up, loopback, routes, &physical)
    })
}

fn include_interface(
    name: &str,
    up: bool,
    loopback: bool,
    routes: &[DefaultRoute<String>],
    physical: &BTreeSet<String>,
) -> bool {
    if routes.is_empty() {
        up && !loopback && physical.contains(name)
    } else {
        routes.iter().any(|route| route.interface == name)
    }
}

fn interfaces(
    errors: &mut Errors,
    include: impl Fn(&str, bool, bool) -> bool,
) -> Probe<Vec<Interface>> {
    struct IfAddrs(*mut libc::ifaddrs);
    impl Drop for IfAddrs {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { libc::freeifaddrs(self.0) };
            }
        }
    }

    let mut head = ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut head) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    let list = IfAddrs(head);
    let mut current = list.0;
    let mut interfaces = BTreeMap::new();
    // The linked list, C strings and socket addresses are owned by getifaddrs.
    while let Some(entry) = unsafe { current.as_ref() } {
        current = entry.ifa_next;
        if entry.ifa_name.is_null() {
            errors.add("network.name", None, "interface name unavailable");
            continue;
        }
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let up = entry.ifa_flags & libc::IFF_UP as u32 != 0;
        let loopback = entry.ifa_flags & libc::IFF_LOOPBACK as u32 != 0;
        if !include(&name, up, loopback) {
            continue;
        }
        let interface = interfaces.entry(name.clone()).or_insert_with(|| Interface {
            name,
            loopback,
            ..Default::default()
        });
        let Some(family) = (unsafe { socket_family(entry.ifa_addr) }) else {
            continue;
        };
        match family {
            libc::AF_INET => {
                let Some(ip) = (unsafe { socket_ipv4(entry.ifa_addr, false) }) else {
                    errors.add(
                        "network.ip",
                        Some(&interface.name),
                        "invalid interface address",
                    );
                    continue;
                };
                let netmask = errors.require(
                    "network.netmask",
                    Some(&interface.name),
                    unsafe { socket_ipv4(entry.ifa_netmask, true) }.and_then(prefix_length),
                    "subnet mask unavailable",
                );
                let address = Address {
                    ip: ip.to_string(),
                    netmask,
                };
                if !interface.addresses.contains(&address) {
                    interface.addresses.push(address);
                }
            }
            libc::AF_INET6 => {}
            _ => {
                if interface.macaddress.is_none() {
                    interface.macaddress = unsafe { socket_mac(entry.ifa_addr) };
                }
            }
        }
    }
    Ok(interfaces.into_values().collect())
}

fn prefix_length(mask: Ipv4Addr) -> Option<u8> {
    let mask = u32::from(mask);
    (mask.leading_ones() + mask.trailing_zeros() == 32).then(|| mask.leading_ones() as u8)
}

// The caller guarantees the pointer is null or points into the getifaddrs list.
unsafe fn socket_ipv4(addr: *const libc::sockaddr, compact_mask: bool) -> Option<Ipv4Addr> {
    let family = unsafe { socket_family(addr) }?;
    if family != libc::AF_INET && !(compact_mask && family == libc::AF_UNSPEC) {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        if family != libc::AF_INET {
            return None;
        }
        let addr = unsafe { &*addr.cast::<libc::sockaddr_in>() };
        Some(Ipv4Addr::from(addr.sin_addr.s_addr.to_ne_bytes()))
    }
    #[cfg(target_os = "macos")]
    {
        // Darwin can omit trailing zero bytes from a netmask.
        let offset = std::mem::offset_of!(libc::sockaddr_in, sin_addr);
        let count = usize::from(unsafe { (*addr).sa_len })
            .saturating_sub(offset)
            .min(4);
        if count != 4 && !compact_mask {
            return None;
        }
        let mut bytes = [0; 4];
        if count > 0 {
            let address =
                unsafe { std::slice::from_raw_parts(addr.cast::<u8>().add(offset), count) };
            bytes[..count].copy_from_slice(address);
        }
        Some(Ipv4Addr::from(bytes))
    }
}

unsafe fn socket_family(addr: *const libc::sockaddr) -> Option<i32> {
    (!addr.is_null()).then(|| i32::from(unsafe { (*addr).sa_family }))
}

#[cfg(target_os = "linux")]
unsafe fn socket_mac(addr: *const libc::sockaddr) -> Option<String> {
    if unsafe { socket_family(addr) }? != libc::AF_PACKET {
        return None;
    }
    let link = unsafe { &*addr.cast::<libc::sockaddr_ll>() };
    mac_address(link.sll_addr.get(..usize::from(link.sll_halen))?)
}

#[cfg(target_os = "macos")]
unsafe fn socket_mac(addr: *const libc::sockaddr) -> Option<String> {
    if unsafe { socket_family(addr) }? != libc::AF_LINK {
        return None;
    }
    // sockaddr_dl is variable-length. Read only its header and populated address bytes.
    let link = addr.cast::<libc::sockaddr_dl>();
    let start = std::mem::offset_of!(libc::sockaddr_dl, sdl_data);
    let len = usize::from(unsafe { (*addr).sa_len });
    if len < start {
        return None;
    }
    let offset = start + usize::from(unsafe { (*link).sdl_nlen });
    let count = usize::from(unsafe { (*link).sdl_alen });
    if offset + count > len {
        return None;
    }
    mac_address(unsafe { std::slice::from_raw_parts(addr.cast::<u8>().add(offset), count) })
}

pub fn dns_address(value: &str) -> Probe<Option<Ipv4Addr>> {
    if let Some((address, zone)) = value.split_once('%')
        && !zone.is_empty()
        && !zone.contains('%')
        && address.parse::<Ipv6Addr>().is_ok()
    {
        return Ok(None);
    }
    value
        .parse::<IpAddr>()
        .map(|address| match address {
            IpAddr::V4(ip) => Some(ip),
            IpAddr::V6(_) => None,
        })
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{DefaultRoute, Ipv4Addr, dns_address, include_interface, prefix_length};
    use std::collections::BTreeSet;

    #[test]
    fn interface_selection_prefers_default_route_or_uses_up_physical_links() {
        let physical = BTreeSet::from(["eth0".into(), "eth1".into()]);
        let routes = vec![DefaultRoute {
            interface: "tun0".into(),
            gateway: None,
        }];
        assert!(include_interface("tun0", true, false, &routes, &physical));
        assert!(!include_interface("eth0", true, false, &routes, &physical));

        let no_routes = [];
        assert!(include_interface(
            "eth0", true, false, &no_routes, &physical
        ));
        assert!(!include_interface(
            "eth1", false, false, &no_routes, &physical
        ));
        assert!(!include_interface(
            "tun0", true, false, &no_routes, &physical
        ));
        assert!(!include_interface("lo", true, true, &no_routes, &physical));
    }

    #[test]
    fn ipv4_prefix_handles_boundaries_and_noncontiguous_masks() {
        assert_eq!(prefix_length([0, 0, 0, 0].into()), Some(0));
        assert_eq!(prefix_length([255, 255, 255, 0].into()), Some(24));
        assert_eq!(prefix_length([255, 255, 255, 255].into()), Some(32));
        assert_eq!(prefix_length([255, 0, 255, 0].into()), None);
    }

    #[test]
    fn dns_ignores_ipv6_addresses_and_reports_invalid_addresses() {
        assert_eq!(
            dns_address("127.0.0.53"),
            Ok(Some(Ipv4Addr::new(127, 0, 0, 53)))
        );
        assert_eq!(dns_address("2001:db8::53"), Ok(None));
        assert_eq!(dns_address("fe80::1%en0"), Ok(None));
        assert!(dns_address("invalid").is_err());
    }
}
