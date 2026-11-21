use crate::dynamic_library::{DynamicLibrary, LibraryId};
use crate::hardware;
use crate::host_type::{self, HostType};
use crate::model::{
    DefaultRoute, Disk, Distribution, Errors, Gpu, Host, Network, Power, Probe, best_routes,
    nonempty,
};
use crate::mount::{NetworkMount, is_network_filesystem};
use crate::unix;
use core_foundation::array::{CFArray, CFArrayRef};
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::bundle::{CFBundle, CFBundleGetValueForInfoDictionaryKey};
use core_foundation::data::CFData;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_foundation::url::{CFURL, CFURLRef};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{Bool, NSObject, NSObjectProtocol};
use objc2::{msg_send, sel};
use objc2_foundation::{NSArray, NSString};
use std::collections::BTreeSet;
use std::ffi::{CStr, CString, c_char, c_void};
use std::io;
use std::net::Ipv4Addr;
use std::ptr;
use sysinfo::System;

#[link(name = "SystemConfiguration", kind = "framework")]
unsafe extern "C" {
    fn SCNetworkInterfaceCopyAll() -> CFArrayRef;
    fn SCNetworkInterfaceGetBSDName(interface: CFTypeRef) -> CFStringRef;
    fn SCNetworkInterfaceGetInterfaceType(interface: CFTypeRef) -> CFStringRef;
    fn SCDynamicStoreCreate(
        allocator: *const c_void,
        name: CFStringRef,
        callback: Option<unsafe extern "C" fn(*const c_void, CFArrayRef, *mut c_void)>,
        context: *mut c_void,
    ) -> CFTypeRef;
    fn SCDynamicStoreCopyLocalHostName(store: CFTypeRef) -> CFStringRef;
    fn SCDynamicStoreCopyMultiple(
        store: CFTypeRef,
        keys: CFArrayRef,
        patterns: CFArrayRef,
    ) -> CFDictionaryRef;
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOServiceMatching(name: *const c_char) -> CFDictionaryRef;
    fn IORegistryEntryIDMatching(id: u64) -> CFDictionaryRef;
    fn IOServiceGetMatchingService(port: u32, matching: CFDictionaryRef) -> u32;
    fn IOServiceGetMatchingServices(port: u32, matching: CFDictionaryRef, iter: *mut u32) -> i32;
    fn IOIteratorNext(iter: u32) -> u32;
    fn IORegistryEntryGetRegistryEntryID(entry: u32, id: *mut u64) -> i32;
    fn IOObjectRelease(object: u32) -> i32;
    fn KextManagerCreateURLForBundleIdentifier(
        allocator: *const c_void,
        identifier: CFStringRef,
    ) -> CFURLRef;
    fn IORegistryEntryCreateCFProperty(
        entry: u32,
        key: CFStringRef,
        allocator: *const c_void,
        options: u32,
    ) -> CFTypeRef;
    fn IORegistryEntrySearchCFProperty(
        entry: u32,
        plane: *const c_char,
        key: CFStringRef,
        allocator: *const c_void,
        options: u32,
    ) -> CFTypeRef;
}

pub fn architecture() -> String {
    // hw.optional.arm64 also describes the host while running under Rosetta.
    let mut arm64 = 0i32;
    let mut size = std::mem::size_of_val(&arm64);
    let result = unsafe {
        libc::sysctlbyname(
            c"hw.optional.arm64".as_ptr(),
            (&mut arm64 as *mut i32).cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if result == 0 && arm64 == 1 {
        "aarch64".into()
    } else {
        System::cpu_arch()
    }
}

pub fn host_type() -> Probe<HostType> {
    let mut model = [0u8; 128];
    let mut size = model.len();
    let model_read = unsafe {
        libc::sysctlbyname(
            c"hw.model".as_ptr(),
            model.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    let model = (model_read == 0)
        .then(|| CStr::from_bytes_until_nul(&model).ok())
        .flatten()
        .and_then(|name| nonempty(&name.to_string_lossy()));
    if let Some(kind) = model.as_deref().and_then(host_type::virtual_machine) {
        return Ok(kind);
    }
    let mut guest = 0i32;
    let mut size = std::mem::size_of_val(&guest);
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.hv_vmm_present".as_ptr(),
            (&raw mut guest).cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    if status == 0 {
        return Ok(if guest == 0 {
            HostType::Baremetal
        } else {
            HostType::Vm
        });
    }
    model
        .map(|_| HostType::Baremetal)
        .ok_or_else(|| "host virtualization could not be identified".into())
}

pub fn distribution(errors: &mut Errors) -> Distribution {
    hardware::distribution(
        "macos",
        Some("macos".into()),
        Some("macOS".into()),
        System::os_version().and_then(|version| nonempty(&version)),
        errors,
    )
}

pub fn uptime() -> Probe<u64> {
    let mut time = std::mem::MaybeUninit::uninit();
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, time.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    u64::try_from(unsafe { time.assume_init() }.tv_sec).map_err(|error| error.to_string())
}

pub fn boot_id() -> Probe<String> {
    let mut buffer = [0u8; 64];
    let mut size = buffer.len();
    let status = unsafe {
        libc::sysctlbyname(
            c"kern.bootsessionuuid".as_ptr(),
            buffer.as_mut_ptr().cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    let sysctl_id = if status == 0 {
        CStr::from_bytes_until_nul(&buffer)
            .ok()
            .and_then(|value| nonempty(&value.to_string_lossy()))
            .ok_or_else(|| "empty macOS boot session UUID".to_owned())
    } else {
        Err(io::Error::last_os_error().to_string())
    };
    sysctl_id
        .or_else(|error| {
            IoService::from_matching(unsafe { IOServiceMatching(c"IOPMrootDomain".as_ptr()) })
                .and_then(|domain| domain.text("BootSessionUUID"))
                .ok_or(error)
        })
        .map(|uuid| uuid.to_ascii_lowercase())
}

pub fn host(errors: &mut Errors) -> Host {
    unix::host(errors, || {
        let name = unsafe { SCDynamicStoreCopyLocalHostName(ptr::null()) };
        if name.is_null() {
            return None;
        }
        let name = unsafe { CFString::wrap_under_create_rule(name) };
        nonempty(&name.to_string()).map(|name| format!("{name}.local"))
    })
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
    if let Err(error) = collect_dns(&mut network, errors) {
        errors.add("network.dns_servers", None, error);
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

fn physical_interfaces() -> Probe<BTreeSet<String>> {
    let interfaces = unsafe { SCNetworkInterfaceCopyAll() };
    if interfaces.is_null() {
        return Err("cannot enumerate System Configuration interfaces".into());
    }
    let interfaces = unsafe { CFArray::<CFType>::wrap_under_create_rule(interfaces) };
    let mut physical = BTreeSet::new();
    for interface in interfaces.iter() {
        let interface = interface.as_CFTypeRef();
        let kind = unsafe { SCNetworkInterfaceGetInterfaceType(interface) };
        if kind.is_null() {
            continue;
        }
        let kind = unsafe { CFString::wrap_under_get_rule(kind) }.to_string();
        if matches!(
            kind.as_str(),
            "Ethernet" | "IEEE80211" | "FireWire" | "WWAN"
        ) {
            let name = unsafe { SCNetworkInterfaceGetBSDName(interface) };
            if !name.is_null() {
                physical.insert(unsafe { CFString::wrap_under_get_rule(name) }.to_string());
            }
        }
    }
    Ok(physical)
}

fn default_interfaces() -> Probe<Vec<DefaultRoute<String>>> {
    use libc::{AF_INET, CTL_NET, NET_RT_DUMP2, PF_ROUTE};
    let mut mib = [CTL_NET, PF_ROUTE, 0, AF_INET, NET_RT_DUMP2, 0];
    for _ in 0..3 {
        let mut len = 0;
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                6,
                ptr::null_mut(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        } != 0
        {
            return Err(io::Error::last_os_error().to_string());
        }
        if len == 0 {
            return Ok(Vec::new());
        }
        let mut table = vec![0u8; len];
        if unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                6,
                table.as_mut_ptr().cast(),
                &mut len,
                ptr::null_mut(),
                0,
            )
        } == 0
        {
            table.truncate(len);
            let indexes = default_route_indexes(&table)?;
            return indexes
                .into_iter()
                .map(|route| {
                    let mut name = [0u8; libc::IF_NAMESIZE];
                    let result = unsafe {
                        libc::if_indextoname(u32::from(route.interface), name.as_mut_ptr().cast())
                    };
                    if result.is_null() {
                        return Err(io::Error::last_os_error().to_string());
                    }
                    CStr::from_bytes_until_nul(&name)
                        .map(|name| DefaultRoute {
                            interface: name.to_string_lossy().into_owned(),
                            gateway: route.gateway,
                        })
                        .map_err(|error| error.to_string())
                })
                .collect();
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ENOMEM) {
            return Err(error.to_string());
        }
    }
    Err("IPv4 routing table changed during collection".into())
}

fn default_route_indexes(mut table: &[u8]) -> Probe<Vec<DefaultRoute<u16>>> {
    let header_len = std::mem::size_of::<libc::rt_msghdr2>();
    let mut routes = Vec::new();
    'messages: while !table.is_empty() {
        if table.len() < header_len {
            return Err("truncated IPv4 routing message".into());
        }
        // The kernel exposes native-layout routing messages with variable-length addresses.
        let header = unsafe { table.as_ptr().cast::<libc::rt_msghdr2>().read_unaligned() };
        let end = usize::from(header.rtm_msglen);
        if end < header_len || end > table.len() {
            return Err("invalid IPv4 routing message length".into());
        }
        let fields = &table[header_len..end];
        table = &table[end..];
        let unusable = libc::RTF_HOST | libc::RTF_REJECT | libc::RTF_BLACKHOLE | libc::RTF_DEAD;
        if i32::from(header.rtm_version) != libc::RTM_VERSION
            || header.rtm_flags & (libc::RTF_UP | unusable) != libc::RTF_UP
            || header.rtm_addrs & libc::RTA_DST == 0
        {
            continue;
        }
        let mut fields = fields;
        let mut gateway = None;
        for field in [libc::RTA_DST, libc::RTA_GATEWAY, libc::RTA_NETMASK] {
            if header.rtm_addrs & field == 0 {
                continue;
            }
            let Some(&len) = fields.first() else {
                return Err("missing route address".into());
            };
            // XNU aligns routing socket addresses to 32-bit boundaries.
            let aligned = usize::from(len).max(1).next_multiple_of(4);
            let Some((addr, rest)) = fields.split_at_checked(aligned) else {
                return Err("truncated route address".into());
            };
            let addr = &addr[..usize::from(len)];
            fields = rest;
            if field == libc::RTA_DST
                && (addr.get(1) != Some(&(libc::AF_INET as u8)) || addr.get(4..8) != Some(&[0; 4]))
            {
                continue 'messages;
            }
            if field == libc::RTA_GATEWAY
                && addr.get(1) == Some(&(libc::AF_INET as u8))
                && let Some(octets) = addr
                    .get(4..8)
                    .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
            {
                gateway = Some(Ipv4Addr::from(octets)).filter(|address| !address.is_unspecified());
            }
            if field == libc::RTA_NETMASK
                && addr
                    .get(4..)
                    .unwrap_or_default()
                    .iter()
                    .any(|&byte| byte != 0)
            {
                continue 'messages;
            }
        }
        if header.rtm_index != 0 {
            let scoped = header.rtm_flags & libc::RTF_IFSCOPE != 0;
            routes.push((
                u64::from(scoped),
                DefaultRoute {
                    interface: header.rtm_index,
                    gateway,
                },
            ));
        }
    }
    Ok(best_routes(routes))
}

fn collect_dns(network: &mut Network, errors: &mut Errors) -> Probe<()> {
    let name = CFString::from_static_string("sys-info");
    let raw_store = unsafe {
        SCDynamicStoreCreate(
            ptr::null(),
            name.as_concrete_TypeRef(),
            None,
            ptr::null_mut(),
        )
    };
    if raw_store.is_null() {
        return Err("cannot open the System Configuration store".into());
    }
    let store = unsafe { CFType::wrap_under_create_rule(raw_store) };
    let global_dns = CFString::from_static_string("State:/Network/Global/DNS");
    let keys = CFArray::from_CFTypes(std::slice::from_ref(&global_dns));
    let patterns = CFArray::from_CFTypes(&[CFString::from_static_string(
        "State:/Network/Service/[^/]+/DNS",
    )]);
    let values = unsafe {
        SCDynamicStoreCopyMultiple(
            store.as_CFTypeRef(),
            keys.as_concrete_TypeRef(),
            patterns.as_concrete_TypeRef(),
        )
    };
    if values.is_null() {
        return Err("cannot read DNS configuration from System Configuration".into());
    }
    let values = unsafe { CFDictionary::<CFString, CFType>::wrap_under_create_rule(values) };
    let server_key = CFString::from_static_string("ServerAddresses");
    let (keys, _) = values.get_keys_and_values();
    let mut keys: Vec<_> = keys
        .into_iter()
        .map(|key| unsafe { CFString::wrap_under_get_rule(key.cast()) })
        .collect();
    // Preserve the primary resolver's order before adding scoped service resolvers.
    keys.sort_by_key(|key| (key != &global_dns, key.to_string()));
    for key in &keys {
        let Some(config) = values
            .find(key)
            .and_then(|value| value.downcast::<CFDictionary>())
        else {
            errors.add(
                "network.dns_servers",
                None,
                "invalid DNS configuration entry",
            );
            continue;
        };
        let Some(servers) = config
            .find(server_key.as_CFTypeRef())
            .and_then(|server_array| {
                let value = unsafe { CFType::wrap_under_get_rule(*server_array) };
                value.downcast::<CFArray>()
            })
        else {
            continue;
        };
        for server in servers.iter() {
            let value = unsafe { CFType::wrap_under_get_rule(*server) };
            match value
                .downcast::<CFString>()
                .map(|value| unix::dns_address(&value.to_string()))
            {
                Some(Ok(Some(server))) => network.add_dns(server),
                Some(Ok(None)) => {}
                _ => errors.add("network.dns_servers", None, "invalid DNS server address"),
            }
        }
    }
    Ok(())
}

#[link(name = "DiskArbitration", kind = "framework")]
unsafe extern "C" {
    fn DASessionCreate(allocator: *const c_void) -> CFTypeRef;
    fn DADiskCreateFromBSDName(
        allocator: *const c_void,
        session: CFTypeRef,
        name: *const c_char,
    ) -> CFTypeRef;
    fn DADiskCopyWholeDisk(disk: CFTypeRef) -> CFTypeRef;
    fn DADiskCopyDescription(disk: CFTypeRef) -> CFDictionaryRef;
    static kDADiskDescriptionDeviceInternalKey: CFStringRef;
}

fn internal_disk(session: Option<&CFType>, name: &[c_char]) -> Probe<bool> {
    let session = session.ok_or_else(|| "Disk Arbitration session unavailable".to_owned())?;
    let bytes = unsafe { std::slice::from_raw_parts(name.as_ptr().cast::<u8>(), name.len()) };
    let name = CStr::from_bytes_until_nul(bytes).map_err(|error| error.to_string())?;
    if let Some(internal) = disk_internal_property(session, name) {
        return Ok(internal);
    }
    // An APFS boot snapshot can appear as diskXsYsZ. Disk Arbitration describes
    // the volume diskXsY even when it does not describe the snapshot.
    if let Ok(name) = name.to_str()
        && let Some(disk) = name.strip_prefix("/dev/disk")
        && disk.split('s').count() == 3
        && let Some((volume, _)) = name.rsplit_once('s')
        && let Ok(volume) = CString::new(volume)
        && let Some(internal) = disk_internal_property(session, &volume)
    {
        return Ok(internal);
    }
    Err("Disk Arbitration did not report whether the device is internal".into())
}

fn disk_internal_property(session: &CFType, name: &CStr) -> Option<bool> {
    let disk =
        unsafe { DADiskCreateFromBSDName(ptr::null(), session.as_CFTypeRef(), name.as_ptr()) };
    if disk.is_null() {
        return None;
    }
    let disk = unsafe { CFType::wrap_under_create_rule(disk) };
    let description_internal = |disk: &CFType| {
        let details = unsafe { DADiskCopyDescription(disk.as_CFTypeRef()) };
        if details.is_null() {
            return None;
        }
        let details = unsafe { CFDictionary::<CFString, CFType>::wrap_under_create_rule(details) };
        let key = unsafe { CFString::wrap_under_get_rule(kDADiskDescriptionDeviceInternalKey) };
        let internal = details.find(&key)?.downcast::<CFBoolean>()?;
        Some(internal == CFBoolean::true_value())
    };
    description_internal(&disk).or_else(|| {
        let whole = unsafe { DADiskCopyWholeDisk(disk.as_CFTypeRef()) };
        if whole.is_null() {
            return None;
        }
        description_internal(&unsafe { CFType::wrap_under_create_rule(whole) })
    })
}

fn mounted_filesystems() -> Probe<Vec<libc::statfs>> {
    let mut mounts = ptr::null_mut();
    let count = unsafe { libc::getmntinfo(&mut mounts, libc::MNT_NOWAIT) };
    if count <= 0 || mounts.is_null() {
        return Err(io::Error::last_os_error().to_string());
    }
    // getmntinfo owns the buffer until the next call. Copy it for the caller.
    Ok(unsafe { std::slice::from_raw_parts(mounts, count as usize) }.to_vec())
}

pub fn mount(errors: &mut Errors) -> Vec<NetworkMount> {
    let Some(filesystems) = errors.record("mount", None, mounted_filesystems()) else {
        return Vec::new();
    };
    let mut mounts = Vec::new();
    for info in filesystems {
        let fs = c_field(&info.f_fstypename);
        let source = c_field(&info.f_mntfromname).unwrap_or_default();
        let local = info.f_flags & libc::MNT_LOCAL as u32 != 0;
        let remote_source =
            source.starts_with("//") || source.contains("://") || source.contains(":/");
        let network = fs.as_deref().is_some_and(is_network_filesystem)
            || (!local && remote_source && fs.as_deref() != Some("autofs"));
        if network {
            if let Some(path) = c_field(&info.f_mntonname) {
                mounts.push(NetworkMount::new(path, fs, &source));
            } else {
                errors.add("mount.mount_point", None, "invalid mount point");
            }
        }
    }
    mounts.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    mounts
}

pub fn disk(errors: &mut Errors) -> Vec<Disk> {
    let Some(mounts) = errors.record("disk", None, mounted_filesystems()) else {
        return Vec::new();
    };
    let session = unsafe { DASessionCreate(ptr::null()) };
    let session = (!session.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(session) });
    let mut disks = Vec::new();
    for mount in mounts {
        if mount.f_flags & libc::MNT_LOCAL as u32 == 0 {
            continue;
        }
        let Some(mount_point) = c_field(&mount.f_mntonname) else {
            errors.add("disk.mount_point", None, "invalid mount point");
            continue;
        };
        let os = mount_point == "/";
        if mount_point.starts_with("/System/Volumes/") {
            continue;
        }
        let fs = c_field(&mount.f_fstypename);
        if matches!(fs.as_deref(), Some("devfs" | "autofs" | "fdesc" | "volfs"))
            || fs.as_deref().is_some_and(is_network_filesystem)
        {
            continue;
        }
        let mut stats = std::mem::MaybeUninit::<libc::statfs>::uninit();
        let space = if unsafe { libc::statfs(mount.f_mntonname.as_ptr(), stats.as_mut_ptr()) } == 0
        {
            let stats = unsafe { stats.assume_init() };
            let size = u64::from(stats.f_bsize);
            stats
                .f_blocks
                .checked_mul(size)
                .zip(stats.f_bfree.checked_mul(size))
                .ok_or_else(|| "filesystem capacity overflowed".into())
        } else {
            Err(io::Error::last_os_error().to_string())
        };
        if !Disk::eligible(os, &mount_point, &space, errors) {
            continue;
        }
        let internal = errors.record(
            "disk.device",
            Some(&mount_point),
            internal_disk(session.as_ref(), &mount.f_mntfromname),
        );
        if internal == Some(false) || (!os && internal.is_none()) {
            continue;
        }
        let fs = fs.ok_or_else(|| "filesystem type unavailable".into());
        disks.push(Disk::from_space(mount_point, fs, space, errors));
    }
    disks.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    if disks.is_empty() {
        errors.add("disk", None, "no mounted local filesystems available");
    }
    disks
}

fn c_field(field: &[c_char]) -> Option<String> {
    let bytes = unsafe { std::slice::from_raw_parts(field.as_ptr().cast::<u8>(), field.len()) };
    let value = CStr::from_bytes_until_nul(bytes).ok()?.to_string_lossy();
    (!value.is_empty()).then(|| value.into_owned())
}

pub fn gpu(errors: &mut Errors, ram_bytes: Option<u64>) -> Vec<Gpu> {
    match metal_devices(errors, ram_bytes) {
        Ok(gpus) if !gpus.is_empty() => gpus,
        result => {
            let reason = result
                .err()
                .unwrap_or_else(|| "Metal returned no usable GPU devices".into());
            errors.add("gpu.library", Some("Metal"), reason);
            let fallback = registry_gpus(errors, ram_bytes);
            errors
                .record("gpu", Some("IOKit"), fallback)
                .unwrap_or_default()
        }
    }
}

fn metal_devices(errors: &mut Errors, ram_bytes: Option<u64>) -> Probe<Vec<Gpu>> {
    type Devices = NSArray<NSObject>;
    type CopyDevices = unsafe extern "C" fn() -> *mut Devices;
    let library = DynamicLibrary::load(LibraryId::Metal)?;
    let copy_devices = unsafe { library.symbol::<CopyDevices>(c"MTLCopyAllDevices") }?;
    // Release Metal objects and the autorelease pool while the library is still loaded.
    autoreleasepool(|_| {
        let devices = unsafe { Retained::from_raw(copy_devices()) }
            .ok_or_else(|| "MTLCopyAllDevices returned null".to_owned())?;
        let mut gpus = Vec::new();
        for device in devices.iter() {
            if !device.respondsToSelector(sel!(registryID)) {
                errors.add(
                    "gpu.library",
                    Some("Metal"),
                    "GPU does not expose registryID",
                );
                continue;
            }
            let registry_id: u64 = unsafe { msg_send![&*device, registryID] };
            let id = format!("metal:{registry_id:x}");
            let service =
                IoService::from_matching(unsafe { IORegistryEntryIDMatching(registry_id) });

            let model = if device.respondsToSelector(sel!(name)) {
                let name: Option<Retained<NSString>> = unsafe { msg_send![&*device, name] };
                name.and_then(|name| nonempty(&name.to_string()))
            } else {
                None
            };
            if model.is_none() {
                errors.add("gpu.library", Some(&id), "Metal GPU name unavailable");
            }

            let unified = if device.respondsToSelector(sel!(hasUnifiedMemory)) {
                let unified: Bool = unsafe { msg_send![&*device, hasUnifiedMemory] };
                Some(unified.as_bool())
            } else {
                errors.add(
                    "gpu.library",
                    Some(&id),
                    "Metal unified memory status unavailable",
                );
                None
            };
            let shared_limit = if unified == Some(true)
                && device.respondsToSelector(sel!(recommendedMaxWorkingSetSize))
            {
                let size: u64 = unsafe { msg_send![&*device, recommendedMaxWorkingSetSize] };
                (size > 0).then_some(size)
            } else {
                None
            };
            gpus.push(registry_gpu(
                id,
                service.as_ref(),
                model,
                unified,
                shared_limit,
                ram_bytes,
                errors,
            ));
        }
        Ok(gpus)
    })
}

fn registry_gpus(errors: &mut Errors, ram_bytes: Option<u64>) -> Probe<Vec<Gpu>> {
    let matching = unsafe { IOServiceMatching(c"IOAccelerator".as_ptr()) };
    if matching.is_null() {
        return Err("cannot create IOKit GPU matching dictionary".into());
    }
    let mut iterator = 0;
    // IOServiceGetMatchingServices consumes the matching dictionary.
    let status = unsafe { IOServiceGetMatchingServices(0, matching, &mut iterator) };
    if status != 0 || iterator == 0 {
        return Err(format!(
            "cannot enumerate IOKit accelerators (status {status:#x})"
        ));
    }
    let iterator = IoService(iterator);
    let mut gpus = Vec::new();
    loop {
        let service = unsafe { IOIteratorNext(iterator.0) };
        if service == 0 {
            break;
        }
        let service = IoService(service);
        let mut registry_id = 0;
        let status = unsafe { IORegistryEntryGetRegistryEntryID(service.0, &mut registry_id) };
        if status != 0 {
            errors.add(
                "gpu",
                Some("IOKit"),
                format!("GPU registry ID unavailable ({status:#x})"),
            );
            continue;
        }
        gpus.push(registry_gpu(
            format!("iokit:{registry_id:x}"),
            Some(&service),
            None,
            None,
            None,
            ram_bytes,
            errors,
        ));
    }
    if gpus.is_empty() {
        return Err("IOKit returned no usable GPU devices".into());
    }
    Ok(gpus)
}

fn registry_gpu(
    id: String,
    service: Option<&IoService>,
    model: Option<String>,
    unified: Option<bool>,
    shared_limit: Option<u64>,
    ram_bytes: Option<u64>,
    errors: &mut Errors,
) -> Gpu {
    let model = model.or_else(|| {
        let property = service?.property("model", false)?;
        if let Some(text) = property.downcast::<CFString>() {
            return nonempty(&text.to_string());
        }
        let data = property.downcast::<CFData>()?;
        nonempty(String::from_utf8_lossy(data.bytes()).trim_end_matches('\0'))
    });
    let unified = unified.or_else(|| {
        model
            .as_deref()
            .is_some_and(|model| model.starts_with("Apple M"))
            .then_some(true)
    });
    let vram = || {
        let megabytes = service?.number("VRAM,totalMB", true)?;
        u64::try_from(megabytes)
            .ok()?
            .checked_mul(1024 * 1024)
            .filter(|&bytes| bytes > 0)
    };
    let size = if unified == Some(true) {
        ram_bytes
    } else {
        vram()
    };
    let driver = service.and_then(|service| {
        service
            .text("CFBundleIdentifierKernel")
            .or_else(|| service.text("CFBundleIdentifier"))
    });
    let version = service
        .and_then(|service| service.text("CFBundleVersion"))
        .or_else(|| kext_bundle_version(driver.as_deref()?));
    let band = hardware::gpu_band(None, model.as_deref());
    Gpu {
        compute_capability: None,
        band: errors.require("gpu.band", Some(&id), band, "GPU manufacturer unavailable"),
        model: errors.require(
            "gpu.type",
            Some(&id),
            model.as_deref().map(hardware::simple_gpu_type),
            "GPU model unavailable",
        ),
        pci_id: None,
        driver,
        version: errors.require(
            "gpu.version",
            Some(&id),
            version,
            "GPU driver version unavailable from IOKit",
        ),
        size: errors.require(
            "gpu.size",
            Some(&id),
            size,
            "GPU memory capacity unavailable",
        ),
        memory_kind: match unified {
            Some(true) => Some("unified"),
            Some(false) => Some("dedicated"),
            None => size.map(|_| "dedicated"),
        },
        shared_limit,
        id,
    }
}

fn kext_bundle_version(identifier: &str) -> Option<String> {
    let identifier = CFString::new(identifier);
    let url = unsafe {
        KextManagerCreateURLForBundleIdentifier(ptr::null(), identifier.as_concrete_TypeRef())
    };
    if url.is_null() {
        return None;
    }
    // Read the installed driver bundle's metadata without loading its executable.
    let bundle = CFBundle::new(unsafe { CFURL::wrap_under_create_rule(url) })?;
    let key = CFString::from_static_string("CFBundleVersion");
    let value = unsafe {
        CFBundleGetValueForInfoDictionaryKey(
            bundle.as_concrete_TypeRef(),
            key.as_concrete_TypeRef(),
        )
    };
    if value.is_null() {
        return None;
    }
    let value = unsafe { CFType::wrap_under_get_rule(value) }.downcast_into::<CFString>()?;
    nonempty(&value.to_string())
}

pub fn power(errors: &mut Errors) -> Vec<Power> {
    let Some(battery) =
        IoService::from_matching(unsafe { IOServiceMatching(c"AppleSmartBattery".as_ptr()) })
    else {
        errors.add(
            "power.watt",
            None,
            "no AppleSmartBattery telemetry available",
        );
        return Vec::new();
    };
    let voltage = battery
        .number("Voltage", false)
        .filter(|&voltage| voltage > 0);
    let current = battery
        .number("InstantAmperage", false)
        .or_else(|| battery.number("Amperage", false));
    let watt = voltage
        .zip(current)
        .map(|(mv, ma)| mv as f64 * f64::from((ma as i32).unsigned_abs()) / 1_000_000.0);
    let source = "battery";
    vec![Power {
        source: source.into(),
        watt: errors.require(
            "power.watt",
            Some(source),
            watt,
            "battery voltage or current unavailable",
        ),
    }]
}

struct IoService(u32);

impl IoService {
    fn from_matching(matching: CFDictionaryRef) -> Option<Self> {
        if matching.is_null() {
            return None;
        }
        // IOServiceGetMatchingService consumes the matching dictionary.
        let service = unsafe { IOServiceGetMatchingService(0, matching) };
        (service != 0).then(|| Self(service))
    }

    fn property(&self, key: &str, search_parents: bool) -> Option<CFType> {
        let key = CFString::new(key);
        const RECURSIVE_PARENTS: u32 = 1 | 2;
        let value = unsafe {
            if search_parents {
                IORegistryEntrySearchCFProperty(
                    self.0,
                    c"IOService".as_ptr(),
                    key.as_concrete_TypeRef(),
                    ptr::null(),
                    RECURSIVE_PARENTS,
                )
            } else {
                IORegistryEntryCreateCFProperty(self.0, key.as_concrete_TypeRef(), ptr::null(), 0)
            }
        };
        (!value.is_null()).then(|| unsafe { CFType::wrap_under_create_rule(value) })
    }

    fn number(&self, key: &str, search_parents: bool) -> Option<i64> {
        self.property(key, search_parents)?
            .downcast::<CFNumber>()?
            .to_i64()
    }

    fn text(&self, key: &str) -> Option<String> {
        nonempty(
            &self
                .property(key, false)?
                .downcast::<CFString>()?
                .to_string(),
        )
    }
}

impl Drop for IoService {
    fn drop(&mut self) {
        unsafe { IOObjectRelease(self.0) };
    }
}
