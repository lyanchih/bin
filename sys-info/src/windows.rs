use crate::dynamic_library::{DynamicLibrary, LibraryId};
use crate::hardware;
use crate::host_type::{self, HostType};
use crate::model::{
    Address, DefaultRoute, Disk, Distribution, Errors, Gpu, Host, Interface, Network, Power, Probe,
    best_routes, mac_address, nonempty,
};
use crate::mount::NetworkMount;
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::ffi::c_void;
use std::io;
use std::mem::size_of;
use std::net::Ipv4Addr;
use std::ptr;
use sysinfo::System;
use windows::Win32::Graphics::Dxgi::{
    DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND, IDXGIDevice, IDXGIFactory1,
};
use windows::core::{Interface as _, PCWSTR};
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_PRESENT, GUID_DEVCLASS_DISPLAY, HDEVINFO, SP_DEVINFO_DATA, SPDRP_DEVICEDESC,
    SPDRP_HARDWAREID, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDevicePropertyW, SetupDiGetDeviceRegistryPropertyW,
};
use windows_sys::Win32::Devices::Properties::{DEVPKEY_Device_DriverVersion, DEVPROP_TYPE_STRING};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BUFFER_OVERFLOW, ERROR_CONNECTION_UNAVAIL, ERROR_MORE_DATA, ERROR_NO_DATA,
    ERROR_NO_MORE_FILES, ERROR_NO_MORE_ITEMS, ERROR_NOT_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    NO_ERROR,
};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    FreeMibTable, GAA_FLAG_INCLUDE_ALL_INTERFACES, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_MULTICAST,
    GetAdaptersAddresses, GetIfEntry2, GetIpForwardTable2, IF_TYPE_SOFTWARE_LOOPBACK,
    IP_ADAPTER_ADDRESSES_LH, MIB_IF_ROW2, MIB_IPFORWARD_TABLE2,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::NetworkManagement::WNet::WNetGetConnectionW;
use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6, SOCKADDR_IN, SOCKET_ADDRESS};
use windows_sys::Win32::Storage::FileSystem::{
    BusType1394, BusTypeFibre, BusTypeMmc, BusTypeSd, BusTypeUnknown, BusTypeUsb, BusTypeiScsi,
    CreateFileW, DISK_SPACE_INFORMATION, FILE_SHARE_READ, FILE_SHARE_WRITE, FindFirstVolumeW,
    FindNextVolumeW, FindVolumeClose, GetDiskSpaceInformationW, GetDriveTypeW,
    GetLogicalDriveStringsW, GetVolumeInformationW, GetVolumeNameForVolumeMountPointW,
    GetVolumePathNameW, GetVolumePathNamesForVolumeNameW, OPEN_EXISTING,
};
use windows_sys::Win32::System::IO::DeviceIoControl;
use windows_sys::Win32::System::Ioctl::{
    IOCTL_DISK_GET_PARTITION_INFO_EX, IOCTL_STORAGE_QUERY_PROPERTY, PARTITION_INFORMATION_EX,
    PARTITION_STYLE_GPT, PARTITION_STYLE_MBR, PropertyStandardQuery, STORAGE_DEVICE_DESCRIPTOR,
    STORAGE_PROPERTY_QUERY, StorageDeviceProperty,
};
use windows_sys::Win32::System::Power::{
    BATTERY_UNKNOWN_RATE, CallNtPowerInformation, SYSTEM_BATTERY_STATE, SystemBatteryState,
};
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows_sys::Win32::System::SystemInformation::{
    COMPUTER_NAME_FORMAT, ComputerNamePhysicalDnsDomain, ComputerNamePhysicalDnsFullyQualified,
    ComputerNamePhysicalDnsHostname, GetComputerNameExW, GetSystemWindowsDirectoryW,
    GetTickCount64, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, IsWow64Process2};
use windows_sys::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOTE, GetUserNameW};

pub fn architecture() -> String {
    let mut machine = 0;
    let mut process_machine = 0;
    let success =
        unsafe { IsWow64Process2(GetCurrentProcess(), &mut process_machine, &mut machine) } != 0;
    match (success, machine) {
        (true, IMAGE_FILE_MACHINE_AMD64) => "x86_64".into(),
        (true, IMAGE_FILE_MACHINE_ARM64) => "aarch64".into(),
        _ => System::cpu_arch(),
    }
}

pub fn cpu_model() -> Probe<String> {
    registry_string(
        windows::core::w!(r"HARDWARE\DESCRIPTION\System\CentralProcessor\0"),
        windows::core::w!("ProcessorNameString"),
    )
}

fn registry_string(subkey: PCWSTR, key: PCWSTR) -> Probe<String> {
    let mut value = [0u16; 256];
    let mut size = size_of::<[u16; 256]>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            key.as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != NO_ERROR {
        return Err(io::Error::from_raw_os_error(status as i32).to_string());
    }
    nonempty(&wide_name(&value)).ok_or_else(|| "Windows registry value is empty".into())
}

pub fn host_type() -> Probe<HostType> {
    if host_type::env_value("CONTAINER_SANDBOX_MOUNT_POINT").is_some()
        || registry_dword(
            windows::core::w!(r"SYSTEM\CurrentControlSet\Control"),
            windows::core::w!("ContainerType"),
        )
        .is_ok()
    {
        return Ok(HostType::Container);
    }
    let bios = windows::core::w!(r"HARDWARE\DESCRIPTION\System\BIOS");
    let identity = [
        windows::core::w!("SystemManufacturer"),
        windows::core::w!("SystemProductName"),
        windows::core::w!("BIOSVendor"),
    ]
    .into_iter()
    .filter_map(|key| registry_string(bios, key).ok())
    .collect::<Vec<_>>()
    .join(" ");
    // Hyper-V can be active on the physical host through VBS. Use guest SMBIOS
    // identities rather than treating the presence of a hypervisor as a VM.
    if identity.is_empty() {
        return Err("Windows SMBIOS identity unavailable".into());
    }
    Ok(host_type::virtual_machine(&identity).unwrap_or(HostType::Baremetal))
}

pub fn distribution(errors: &mut Errors) -> Distribution {
    hardware::distribution(
        "windows",
        Some("windows".into()),
        System::name().and_then(|name| nonempty(&name)),
        System::os_version().and_then(|version| nonempty(&version)),
        errors,
    )
}

pub fn uptime() -> Probe<u64> {
    Ok(unsafe { GetTickCount64() } / 1_000)
}

pub fn boot_id() -> Probe<String> {
    registry_dword(
        windows::core::w!(
            r"SYSTEM\CurrentControlSet\Control\Session Manager\Memory Management\PrefetchParameters"
        ),
        windows::core::w!("BootId"),
    )
    .map(|counter| counter.to_string())
}

fn registry_dword(subkey: PCWSTR, key: PCWSTR) -> Probe<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            key.as_ptr(),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&raw mut value).cast(),
            &mut size,
        )
    };
    if status != NO_ERROR {
        return Err(io::Error::from_raw_os_error(status as i32).to_string());
    }
    Ok(value)
}

pub fn host(errors: &mut Errors) -> Host {
    let hostname = computer_name(ComputerNamePhysicalDnsHostname);
    let domain = computer_name(ComputerNamePhysicalDnsDomain);
    let fqdn = computer_name(ComputerNamePhysicalDnsFullyQualified).and_then(|name| {
        if name.trim_end_matches('.').contains('.') {
            Ok(name)
        } else {
            Err("no fully qualified host name configured".into())
        }
    });
    Host {
        kind: None,
        user: errors.record("host.user", None, effective_user()),
        uid: None,
        gid: None,
        hostname: errors.record("host.hostname", None, hostname),
        domain: errors.record("host.domain", None, domain),
        fqdn: errors.record("host.fqdn", None, fqdn),
    }
}

fn effective_user() -> Probe<String> {
    // GetUserNameW documents UNLEN + 1 UTF-16 code units, including the terminator.
    let mut buffer = [0u16; 257];
    let mut size = buffer.len() as u32;
    if unsafe { GetUserNameW(buffer.as_mut_ptr(), &mut size) } == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    nonempty(&wide_name(&buffer)).ok_or_else(|| "Windows returned an empty account name".into())
}

fn computer_name(format: COMPUTER_NAME_FORMAT) -> Probe<String> {
    let mut len = 0;
    let status = unsafe { GetComputerNameExW(format, ptr::null_mut(), &mut len) };
    if status != 0 {
        return Err("computer name is empty".into());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) || len == 0 {
        return Err(error.to_string());
    }
    let mut name = vec![0; len as usize];
    if unsafe { GetComputerNameExW(format, name.as_mut_ptr(), &mut len) } == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    name.truncate(len as usize);
    nonempty(&String::from_utf16_lossy(&name)).ok_or_else(|| "computer name is empty".into())
}

pub fn network(errors: &mut Errors) -> Network {
    let mut network = Network::default();
    match adapter_buffer() {
        Ok(buffer) => collect_adapters(&buffer, &mut network, errors),
        Err(error) => errors.add("network.interfaces", None, error),
    }
    network.check(errors);
    network
}

fn adapter_buffer() -> Probe<Vec<u64>> {
    let mut bytes = 15_360;
    let flags = GAA_FLAG_INCLUDE_ALL_INTERFACES | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST;
    // The list contains pointers to records in this allocation and must stay in place.
    // uint64 alignment is sufficient for all structures in GetAdaptersAddresses' buffer.
    for _ in 0..3 {
        let words = (bytes as usize).div_ceil(size_of::<u64>());
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(words)
            .map_err(|error| error.to_string())?;
        buffer.resize(words, 0);
        let status = unsafe {
            GetAdaptersAddresses(
                u32::from(AF_INET),
                flags,
                ptr::null(),
                buffer.as_mut_ptr().cast(),
                &mut bytes,
            )
        };
        match status {
            NO_ERROR => return Ok(buffer),
            ERROR_NO_DATA => return Ok(Vec::new()),
            ERROR_BUFFER_OVERFLOW => {}
            _ => return Err(io::Error::from_raw_os_error(status as i32).to_string()),
        }
    }
    Err("adapter list changed during collection".into())
}

fn adapters(buffer: &[u64]) -> impl Iterator<Item = &IP_ADAPTER_ADDRESSES_LH> {
    let mut current: *const IP_ADAPTER_ADDRESSES_LH = if buffer.is_empty() {
        ptr::null()
    } else {
        buffer.as_ptr().cast()
    };
    // The pointers belong to the buffer produced by GetAdaptersAddresses.
    std::iter::from_fn(move || {
        let adapter: &IP_ADAPTER_ADDRESSES_LH = unsafe { current.as_ref() }?;
        current = adapter.Next;
        Some(adapter)
    })
}

fn default_interfaces(buffer: &[u64]) -> Probe<Vec<DefaultRoute<u32>>> {
    let metrics: BTreeMap<_, _> = adapters(buffer)
        .map(|adapter| {
            (
                unsafe { adapter.Anonymous1.Anonymous.IfIndex },
                adapter.Ipv4Metric,
            )
        })
        .collect();
    struct RouteTable(*mut MIB_IPFORWARD_TABLE2);
    impl Drop for RouteTable {
        fn drop(&mut self) {
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }

    let mut table = ptr::null_mut();
    let status = unsafe { GetIpForwardTable2(AF_INET, &mut table) };
    if status == ERROR_NO_DATA {
        return Ok(Vec::new());
    }
    if status != NO_ERROR {
        return Err(io::Error::from_raw_os_error(status as i32).to_string());
    }
    if table.is_null() {
        return Err("Windows returned a null IPv4 routing table".into());
    }
    let table = RouteTable(table);
    // Table is a flexible array of NumEntries rows owned by the API.
    let rows = unsafe {
        let count = (*table.0).NumEntries as usize;
        std::slice::from_raw_parts(ptr::addr_of!((*table.0).Table).cast(), count)
    };
    Ok(best_routes(rows.iter().filter_map(
        |row: &windows_sys::Win32::NetworkManagement::IpHelper::MIB_IPFORWARD_ROW2| {
            let prefix = &row.DestinationPrefix;
            if prefix.PrefixLength != 0
                || row.Loopback
                || unsafe { prefix.Prefix.si_family } != AF_INET
            {
                return None;
            }
            let interface = row.InterfaceIndex;
            let metric = *metrics.get(&interface)?;
            let gateway = if unsafe { row.NextHop.si_family } == AF_INET {
                let address =
                    Ipv4Addr::from(unsafe { row.NextHop.Ipv4.sin_addr.S_un.S_addr }.to_ne_bytes());
                (!address.is_unspecified()).then_some(address)
            } else {
                None
            };
            Some((
                u64::from(row.Metric) + u64::from(metric),
                DefaultRoute { interface, gateway },
            ))
        },
    )))
}

fn is_physical(adapter: &IP_ADAPTER_ADDRESSES_LH) -> Probe<bool> {
    let mut interface = MIB_IF_ROW2 {
        InterfaceLuid: adapter.Luid,
        ..Default::default()
    };
    let result = unsafe { GetIfEntry2(&mut interface) };
    if result != NO_ERROR {
        return Err(io::Error::from_raw_os_error(result as i32).to_string());
    }
    // HardwareInterface is the least significant bit of InterfaceAndOperStatusFlags.
    Ok(interface.InterfaceAndOperStatusFlags._bitfield & 1 != 0)
}

fn collect_adapters(buffer: &[u64], network: &mut Network, errors: &mut Errors) {
    let routes = errors
        .record("network.default_route", None, default_interfaces(buffer))
        .unwrap_or_default();
    let mut external_ip = None;
    for adapter in adapters(buffer) {
        let mut dns = adapter.FirstDnsServerAddress;
        while let Some(entry) = unsafe { dns.as_ref() } {
            dns = entry.Next;
            match unsafe { socket_ipv4(&entry.Address) } {
                Ok(Some(ip)) => network.add_dns(ip.to_string()),
                Ok(None) => {}
                Err(error) => errors.add("network.dns_servers", None, error),
            }
        }
        let up = adapter.OperStatus == IfOperStatusUp;
        let loopback = adapter.IfType == IF_TYPE_SOFTWARE_LOOPBACK;
        let index = unsafe { adapter.Anonymous1.Anonymous.IfIndex };
        if (!routes.is_empty() && !routes.iter().any(|route| route.interface == index))
            || (routes.is_empty() && (!up || loopback))
        {
            continue;
        }
        let friendly = if adapter.FriendlyName.is_null() {
            None
        } else {
            unsafe { PCWSTR(adapter.FriendlyName).to_string() }
                .ok()
                .and_then(|name| nonempty(&name))
        };
        let name = friendly.or_else(|| {
            if adapter.AdapterName.is_null() {
                return None;
            }
            nonempty(&unsafe { CStr::from_ptr(adapter.AdapterName.cast()) }.to_string_lossy())
        });
        let Some(name) = errors.require("network.name", None, name, "interface name unavailable")
        else {
            continue;
        };
        if routes.is_empty()
            && !errors
                .record("network.interface_type", Some(&name), is_physical(adapter))
                .unwrap_or(false)
        {
            continue;
        }
        let mut interface = Interface {
            name,
            loopback,
            macaddress: adapter
                .PhysicalAddress
                .get(..adapter.PhysicalAddressLength as usize)
                .and_then(mac_address),
            ..Default::default()
        };
        let mut address = adapter.FirstUnicastAddress;
        while let Some(entry) = unsafe { address.as_ref() } {
            address = entry.Next;
            let ip = match unsafe { socket_ipv4(&entry.Address) } {
                Ok(Some(ip)) => ip,
                Ok(None) => continue,
                Err(error) => {
                    errors.add("network.ip", Some(&interface.name), error);
                    continue;
                }
            };
            let prefix = entry.OnLinkPrefixLength;
            let netmask = errors.require(
                "network.netmask",
                Some(&interface.name),
                (prefix <= 32).then_some(prefix),
                "invalid IPv4 subnet prefix length",
            );
            let address = Address {
                ip: ip.to_string(),
                netmask,
            };
            if !interface.addresses.contains(&address) {
                interface.addresses.push(address);
            }
        }
        if routes.first().is_some_and(|route| route.interface == index) {
            external_ip = interface
                .addresses
                .first()
                .map(|address| address.ip.clone());
        }
        network.interfaces.push(interface);
    }
    if let Some(route) = routes.first() {
        network.set_default_route(external_ip, route.gateway, errors);
    }
    network.interfaces.sort_by(|a, b| a.name.cmp(&b.name));
}

unsafe fn socket_ipv4(address: &SOCKET_ADDRESS) -> Probe<Option<Ipv4Addr>> {
    let len = usize::try_from(address.iSockaddrLength).unwrap_or(0);
    let socket = address.lpSockaddr;
    if len >= size_of::<u16>() && !socket.is_null() {
        match unsafe { (*socket).sa_family } {
            AF_INET if len >= size_of::<SOCKADDR_IN>() => {
                let address = unsafe { &*socket.cast::<SOCKADDR_IN>() };
                let bytes = unsafe { address.sin_addr.S_un.S_addr }.to_ne_bytes();
                return Ok(Some(Ipv4Addr::from(bytes)));
            }
            AF_INET6 => return Ok(None),
            _ => {}
        }
    }
    Err("invalid network address".into())
}

fn windows_volume() -> Probe<String> {
    let mut directory = [0u16; 260];
    let length =
        unsafe { GetSystemWindowsDirectoryW(directory.as_mut_ptr(), directory.len() as u32) };
    if length == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    if length as usize >= directory.len() {
        return Err("Windows system directory exceeded the buffer".into());
    }
    let mut mount = [0u16; 260];
    let mut volume = [0u16; 64];
    let success = unsafe {
        GetVolumePathNameW(directory.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) != 0
            && GetVolumeNameForVolumeMountPointW(
                mount.as_ptr(),
                volume.as_mut_ptr(),
                volume.len() as u32,
            ) != 0
    };
    if !success {
        return Err(io::Error::last_os_error().to_string());
    }
    Ok(wide_name(&volume).to_ascii_lowercase())
}

fn wide_name(value: &[u16]) -> String {
    let len = value
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..len])
}

fn preferred_mount(volume: &[u16]) -> Probe<Option<Vec<u16>>> {
    let mut paths = vec![0u16; 260];
    for _ in 0..3 {
        let mut required = 0;
        let status = unsafe {
            GetVolumePathNamesForVolumeNameW(
                volume.as_ptr(),
                paths.as_mut_ptr(),
                paths.len() as u32,
                &mut required,
            )
        };
        if status != 0 {
            return Ok(paths
                .split_inclusive(|&unit| unit == 0)
                .take_while(|path| path.len() > 1)
                .min_by(|a, b| (a.len(), a).cmp(&(b.len(), b)))
                .map(<[u16]>::to_vec));
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_MORE_DATA as i32) || required as usize <= paths.len()
        {
            return Err(error.to_string());
        }
        paths.resize(required as usize, 0);
    }
    Err("volume mount paths changed during collection".into())
}

fn mounted_volumes(errors: &mut Errors) -> Vec<(Vec<u16>, Vec<u16>)> {
    struct VolumeSearch(HANDLE);
    impl Drop for VolumeSearch {
        fn drop(&mut self) {
            unsafe { FindVolumeClose(self.0) };
        }
    }

    let mut volume = vec![0u16; 64];
    let handle = unsafe { FindFirstVolumeW(volume.as_mut_ptr(), volume.len() as u32) };
    if handle == INVALID_HANDLE_VALUE {
        errors.add("disk", None, io::Error::last_os_error().to_string());
        return Vec::new();
    }
    let search = VolumeSearch(handle);
    let mut mounted = Vec::new();
    loop {
        if unsafe { GetDriveTypeW(volume.as_ptr()) } == DRIVE_FIXED {
            match preferred_mount(&volume) {
                Ok(Some(mount)) => mounted.push((volume.clone(), mount)),
                Ok(None) => {}
                Err(error) => errors.add("disk.mount_point", Some(&wide_name(&volume)), error),
            }
        }
        if unsafe { FindNextVolumeW(search.0, volume.as_mut_ptr(), volume.len() as u32) } == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() != Some(ERROR_NO_MORE_FILES as i32) {
                errors.add("disk", None, error.to_string());
            }
            break;
        }
    }
    mounted
}

fn internal_data_volume(volume: &[u16], os: bool) -> Probe<bool> {
    struct Handle(HANDLE);
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    let mut path = volume.to_vec();
    let end = path
        .iter()
        .position(|&unit| unit == 0)
        .ok_or_else(|| "invalid volume GUID path".to_owned())?;
    if end == 0 || path[end - 1] != u16::from(b'\\') {
        return Err("invalid volume GUID path".into());
    }
    // CreateFile opens the volume itself when the trailing separator is omitted.
    path[end - 1] = 0;
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error().to_string());
    }
    let handle = Handle(handle);
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        ..Default::default()
    };
    let mut response = [0u64; 128];
    let mut received = 0;
    let status = unsafe {
        DeviceIoControl(
            handle.0,
            IOCTL_STORAGE_QUERY_PROPERTY,
            (&raw const query).cast::<c_void>(),
            size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            response.as_mut_ptr().cast(),
            size_of::<[u64; 128]>() as u32,
            &mut received,
            ptr::null_mut(),
        )
    };
    if status == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    if (received as usize) < size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
        return Err("truncated Windows storage descriptor".into());
    }
    let storage = unsafe { &*response.as_ptr().cast::<STORAGE_DEVICE_DESCRIPTOR>() };
    if storage.RemovableMedia
        || [
            BusTypeUsb,
            BusType1394,
            BusTypeSd,
            BusTypeMmc,
            BusTypeiScsi,
            BusTypeFibre,
        ]
        .contains(&storage.BusType)
    {
        return Ok(false);
    }
    if storage.BusType == BusTypeUnknown {
        return Err("Windows did not report the storage transport".into());
    }
    if os {
        return Ok(true);
    }
    let mut partition = PARTITION_INFORMATION_EX::default();
    let status = unsafe {
        DeviceIoControl(
            handle.0,
            IOCTL_DISK_GET_PARTITION_INFO_EX,
            ptr::null(),
            0,
            (&raw mut partition).cast(),
            size_of::<PARTITION_INFORMATION_EX>() as u32,
            &mut received,
            ptr::null_mut(),
        )
    };
    // Dynamic and pooled volumes may not expose individual partitions.
    if status == 0 || (received as usize) < size_of::<PARTITION_INFORMATION_EX>() {
        return Ok(true);
    }
    let boot = match partition.PartitionStyle {
        PARTITION_STYLE_MBR => matches!(
            unsafe { partition.Anonymous.Mbr.PartitionType },
            0x27 | 0xef
        ),
        PARTITION_STYLE_GPT => {
            let kind = unsafe { partition.Anonymous.Gpt.PartitionType };
            let kind = windows::core::GUID {
                data1: kind.data1,
                data2: kind.data2,
                data3: kind.data3,
                data4: kind.data4,
            };
            [
                windows::core::GUID::from_u128(0xc12a7328_f81f_11d2_ba4b_00a0c93ec93b),
                windows::core::GUID::from_u128(0xe3c9e316_0b5c_4db8_817d_f92df00215ae),
                windows::core::GUID::from_u128(0xde94bba4_06d1_4d40_a16a_bfd50179d6ac),
            ]
            .contains(&kind)
        }
        _ => false,
    };
    Ok(!boot && !partition.IsServicePartition)
}

pub fn mount(errors: &mut Errors) -> Vec<NetworkMount> {
    let mut roots = [0u16; 26 * 4 + 1];
    let count = unsafe { GetLogicalDriveStringsW(roots.len() as u32, roots.as_mut_ptr()) } as usize;
    if count == 0 || count >= roots.len() {
        let message = if count == 0 {
            io::Error::last_os_error().to_string()
        } else {
            "drive list exceeded the buffer".into()
        };
        errors.add("mount", None, message);
        return Vec::new();
    }
    let mut mounts = Vec::new();
    for root in roots[..count].split_inclusive(|&unit| unit == 0) {
        if root.len() != 4 || unsafe { GetDriveTypeW(root.as_ptr()) } != DRIVE_REMOTE {
            continue;
        }
        let local_name = [root[0], root[1], 0];
        let path = wide_name(root);
        match remote_drive_source(&local_name) {
            Ok(Some(source)) => mounts.push(NetworkMount::new(path, None, &source)),
            Ok(None) => {}
            Err(error) => {
                errors.add("mount.source", Some(&path), error);
                mounts.push(NetworkMount::new(path, None, ""));
            }
        }
    }
    mounts.sort_by(|a, b| a.mount_point.cmp(&b.mount_point));
    mounts
}

fn remote_drive_source(local_name: &[u16; 3]) -> Probe<Option<String>> {
    let mut remote = vec![0u16; 260];
    for _ in 0..3 {
        let mut size = remote.len() as u32;
        let status =
            unsafe { WNetGetConnectionW(local_name.as_ptr(), remote.as_mut_ptr(), &mut size) };
        match status {
            NO_ERROR => {
                return nonempty(&wide_name(&remote))
                    .map(Some)
                    .ok_or_else(|| "Windows returned an empty network drive source".into());
            }
            ERROR_CONNECTION_UNAVAIL | ERROR_NOT_CONNECTED => return Ok(None),
            ERROR_MORE_DATA if size as usize > remote.len() => remote.resize(size as usize, 0),
            _ => return Err(io::Error::from_raw_os_error(status as i32).to_string()),
        }
    }
    Err("network drive source changed during collection".into())
}

pub fn disk(errors: &mut Errors) -> Vec<Disk> {
    let os_volume = errors.record("disk.os_volume", None, windows_volume());
    let mut disks = Vec::new();
    for (volume, mount) in mounted_volumes(errors) {
        let path = mount.as_ptr();
        let mount_point = wide_name(&mount);
        let os = os_volume
            .as_ref()
            .is_some_and(|os| os.eq_ignore_ascii_case(&wide_name(&volume)));
        let mut fs = [0u16; 256];
        let status = unsafe {
            GetVolumeInformationW(
                path,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        let filesystem = if status == 0 {
            Err(io::Error::last_os_error().to_string())
        } else {
            let count = fs.iter().position(|&unit| unit == 0).unwrap_or(fs.len());
            nonempty(&String::from_utf16_lossy(&fs[..count]))
                .ok_or_else(|| "filesystem type unavailable".into())
        };
        let mut space = DISK_SPACE_INFORMATION::default();
        let status = unsafe { GetDiskSpaceInformationW(path, &mut space) };
        let usage = if status < 0 {
            Err(windows::core::HRESULT(status).message())
        } else {
            let block = u64::from(space.SectorsPerAllocationUnit) * u64::from(space.BytesPerSector);
            space
                .ActualTotalAllocationUnits
                .checked_mul(block)
                .zip(space.UsedAllocationUnits.checked_mul(block))
                .ok_or_else(|| "filesystem capacity overflowed".into())
        };
        if !Disk::eligible(os, &mount_point, &usage, errors) {
            continue;
        }
        let internal = errors.record(
            "disk.device",
            Some(&mount_point),
            internal_data_volume(&volume, os),
        );
        if internal == Some(false) || (!os && internal.is_none()) {
            continue;
        }
        disks.push((
            !os,
            Disk::from_usage(mount_point, filesystem, usage, errors),
        ));
    }
    if disks.is_empty() {
        errors.add("disk", None, "no eligible mounted volumes available");
    }
    disks.sort_by(|(a_os, a), (b_os, b)| (a_os, &a.mount_point).cmp(&(b_os, &b.mount_point)));
    disks.into_iter().map(|(_, disk)| disk).collect()
}

pub fn gpu(errors: &mut Errors, _ram_bytes: Option<u64>) -> Vec<Gpu> {
    match dxgi_gpus(errors) {
        Ok(mut gpus) if !gpus.is_empty() => {
            fill_display_properties(&mut gpus, errors);
            gpus
        }
        result => {
            let reason = result
                .err()
                .unwrap_or_else(|| "DXGI returned no usable GPU devices".into());
            errors.add("gpu.library", Some("DXGI"), reason);
            let fallback = setupapi_gpus(errors);
            errors
                .record("gpu", Some("SetupAPI"), fallback)
                .unwrap_or_default()
        }
    }
}

fn dxgi_gpus(errors: &mut Errors) -> Probe<Vec<Gpu>> {
    type Create = unsafe extern "system" fn(
        *const windows::core::GUID,
        *mut *mut c_void,
    ) -> windows::core::HRESULT;
    let library = DynamicLibrary::load(LibraryId::Dxgi)?;
    let create = unsafe { library.symbol::<Create>(c"CreateDXGIFactory1") }?;
    let mut factory = ptr::null_mut();
    unsafe { create(&IDXGIFactory1::IID, &mut factory) }
        .ok()
        .map_err(|error| error.to_string())?;
    if factory.is_null() {
        return Err("DXGI factory creation returned null".into());
    }
    // The factory and adapters must release their COM handles before unloading DXGI.
    let factory = unsafe { IDXGIFactory1::from_raw(factory) };
    let mut gpus = Vec::new();
    for index in 0.. {
        let adapter = match unsafe { factory.EnumAdapters1(index) } {
            Ok(adapter) => adapter,
            Err(error) if error.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(error) => {
                if gpus.is_empty() {
                    return Err(error.to_string());
                }
                errors.add("gpu.library", Some("DXGI"), error.to_string());
                break;
            }
        };
        let index = format!("dxgi:{index}");
        let desc = unsafe { adapter.GetDesc1() };
        let Some(desc) = errors.record(
            "gpu.library",
            Some(&index),
            desc.map_err(|error| error.to_string()),
        ) else {
            continue;
        };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let id = format!(
            "dxgi:{:08x}{:08x}",
            desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
        );
        let len = desc
            .Description
            .iter()
            .position(|&unit| unit == 0)
            .unwrap_or(desc.Description.len());
        let model = nonempty(&String::from_utf16_lossy(&desc.Description[..len]));
        // IDXGIDevice requests the display driver package's user-mode version.
        let version = unsafe { adapter.CheckInterfaceSupport(&IDXGIDevice::IID) }
            .map(|version| {
                let version = version as u64;
                format!(
                    "{}.{}.{}.{}",
                    version >> 48,
                    (version >> 32) & 0xffff,
                    (version >> 16) & 0xffff,
                    version & 0xffff
                )
            })
            .map_err(|error| error.to_string());
        let (size, kind) = if desc.DedicatedVideoMemory > 0 {
            (Some(desc.DedicatedVideoMemory as u64), Some("dedicated"))
        } else if desc.DedicatedSystemMemory > 0 {
            (
                Some(desc.DedicatedSystemMemory as u64),
                Some("dedicated_system"),
            )
        } else {
            (None, None)
        };
        let band = hardware::gpu_band(Some(desc.VendorId), model.as_deref());
        gpus.push(Gpu {
            band: errors.require("gpu.band", Some(&id), band, "GPU manufacturer unavailable"),
            model: errors.require(
                "gpu.type",
                Some(&id),
                model.as_deref().map(hardware::simple_gpu_type),
                "GPU model unavailable",
            ),
            pci_id: (desc.VendorId != 0 && desc.DeviceId != 0)
                .then(|| format!("{:04x}:{:04x}", desc.VendorId, desc.DeviceId)),
            driver: None,
            version: errors.record("gpu.version", Some(&id), version),
            size: errors.require(
                "gpu.size",
                Some(&id),
                size,
                "dedicated GPU memory unavailable",
            ),
            memory_kind: kind,
            shared_limit: (desc.SharedSystemMemory > 0).then_some(desc.SharedSystemMemory as u64),
            id,
        });
    }
    Ok(gpus)
}

struct DisplayDevices {
    handle: HDEVINFO,
    entries: Vec<SP_DEVINFO_DATA>,
}

impl DisplayDevices {
    fn query(errors: &mut Errors) -> Probe<Self> {
        let handle = unsafe {
            SetupDiGetClassDevsW(
                &GUID_DEVCLASS_DISPLAY,
                ptr::null(),
                ptr::null_mut(),
                DIGCF_PRESENT,
            )
        };
        if handle == -1 {
            return Err(io::Error::last_os_error().to_string());
        }
        let mut devices = Self {
            handle,
            entries: Vec::new(),
        };
        for index in 0.. {
            let mut device = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if unsafe { SetupDiEnumDeviceInfo(handle, index, &mut device) } == 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_NO_MORE_ITEMS as i32) {
                    errors.add("gpu", Some("SetupAPI"), error.to_string());
                }
                break;
            }
            devices.entries.push(device);
        }
        if devices.entries.is_empty() {
            return Err("no present Windows display devices".into());
        }
        Ok(devices)
    }
}

impl Drop for DisplayDevices {
    fn drop(&mut self) {
        unsafe { SetupDiDestroyDeviceInfoList(self.handle) };
    }
}

fn fill_display_properties(gpus: &mut [Gpu], errors: &mut Errors) {
    let incomplete =
        |gpu: &Gpu| gpu.pci_id.is_some() && (gpu.model.is_none() || gpu.version.is_none());
    if !gpus.iter().any(incomplete) {
        return;
    }
    let devices = DisplayDevices::query(errors);
    let Some(devices) = errors.record("gpu", Some("SetupAPI"), devices) else {
        return;
    };
    let identified: Vec<_> = devices
        .entries
        .iter()
        .filter_map(|device| {
            let (_, pci_id) = display_pci_id(devices.handle, device);
            Some((pci_id?, device))
        })
        .collect();
    for gpu in gpus.iter_mut().filter(|gpu| incomplete(gpu)) {
        let mut matches = identified
            .iter()
            .filter(|(pci_id, _)| gpu.pci_id.as_ref() == Some(pci_id));
        // With identical cards the PCI vendor/device pair cannot identify an instance.
        let (Some((_, device)), None) = (matches.next(), matches.next()) else {
            continue;
        };
        if gpu.model.is_none() {
            let model = display_property(devices.handle, device, SPDRP_DEVICEDESC);
            gpu.model = errors.record(
                "gpu.type",
                Some(&gpu.id),
                model.map(|name| hardware::simple_gpu_type(&name)),
            );
        }
        if gpu.version.is_none() {
            let version = display_driver_version(devices.handle, device);
            gpu.version = errors.record("gpu.version", Some(&gpu.id), version);
        }
    }
}

fn setupapi_gpus(errors: &mut Errors) -> Probe<Vec<Gpu>> {
    let devices = DisplayDevices::query(errors)?;
    let mut gpus = Vec::new();
    for (index, device) in devices.entries.iter().enumerate() {
        let id = format!("setupapi:{index}");
        let model = display_property(devices.handle, device, SPDRP_DEVICEDESC);
        let (pci_vendor, pci_id) = display_pci_id(devices.handle, device);
        let band = hardware::gpu_band(pci_vendor, model.as_deref().ok());
        let version = display_driver_version(devices.handle, device);
        errors.add(
            "gpu.size",
            Some(&id),
            "Windows display device inventory does not expose GPU memory capacity",
        );
        gpus.push(Gpu {
            band: errors.require("gpu.band", Some(&id), band, "GPU manufacturer unavailable"),
            model: errors.record(
                "gpu.type",
                Some(&id),
                model.map(|name| hardware::simple_gpu_type(&name)),
            ),
            pci_id,
            driver: None,
            version: errors.record("gpu.version", Some(&id), version),
            size: None,
            memory_kind: None,
            shared_limit: None,
            id,
        });
    }
    Ok(gpus)
}

fn display_pci_id(devices: HDEVINFO, device: &SP_DEVINFO_DATA) -> (Option<u32>, Option<String>) {
    let hardware_id = display_property(devices, device, SPDRP_HARDWAREID)
        .ok()
        .map(|id| id.to_ascii_uppercase());
    let vendor = hardware_id
        .as_deref()
        .and_then(|id| pci_component(id, "VEN_"));
    let device = hardware_id
        .as_deref()
        .and_then(|id| pci_component(id, "DEV_"));
    let combined = vendor
        .zip(device)
        .map(|(vendor, device)| format!("{vendor:04x}:{device:04x}"));
    (vendor, combined)
}

fn display_property(devices: HDEVINFO, device: &SP_DEVINFO_DATA, key: u32) -> Probe<String> {
    let mut value = [0u16; 512];
    let status = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            devices,
            device,
            key,
            ptr::null_mut(),
            value.as_mut_ptr().cast(),
            size_of::<[u16; 512]>() as u32,
            ptr::null_mut(),
        )
    };
    if status == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    nonempty(&wide_name(&value))
        .ok_or_else(|| "Windows returned an empty display device property".into())
}

fn display_driver_version(devices: HDEVINFO, device: &SP_DEVINFO_DATA) -> Probe<String> {
    let mut value = [0u16; 80];
    let mut property_type = 0;
    let status = unsafe {
        SetupDiGetDevicePropertyW(
            devices,
            device,
            &DEVPKEY_Device_DriverVersion,
            &mut property_type,
            value.as_mut_ptr().cast(),
            size_of::<[u16; 80]>() as u32,
            ptr::null_mut(),
            0,
        )
    };
    if status == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    if property_type != DEVPROP_TYPE_STRING {
        return Err("Windows display driver version has unexpected property type".into());
    }
    nonempty(&wide_name(&value))
        .ok_or_else(|| "Windows returned an empty display driver version".into())
}

fn pci_component(hardware_id: &str, prefix: &str) -> Option<u32> {
    let (_, value) = hardware_id.split_once(prefix)?;
    u32::from_str_radix(value.get(..4)?, 16).ok()
}

pub fn power(errors: &mut Errors) -> Vec<Power> {
    let mut battery = SYSTEM_BATTERY_STATE::default();
    let status = unsafe {
        CallNtPowerInformation(
            SystemBatteryState,
            ptr::null(),
            0,
            (&mut battery as *mut SYSTEM_BATTERY_STATE).cast(),
            size_of::<SYSTEM_BATTERY_STATE>() as u32,
        )
    };
    if status != 0 {
        errors.add(
            "power.watt",
            None,
            format!("battery state NTSTATUS 0x{:08x}", status as u32),
        );
        return Vec::new();
    }
    if !battery.BatteryPresent {
        errors.add("power.watt", None, "no system battery available");
        return Vec::new();
    }
    let source = "battery";
    let watt = (battery.Rate != BATTERY_UNKNOWN_RATE)
        .then(|| f64::from((battery.Rate as i32).unsigned_abs()) / 1000.0);
    vec![Power {
        source: source.into(),
        watt: errors.require("power.watt", Some(source), watt, "battery rate unavailable"),
    }]
}
