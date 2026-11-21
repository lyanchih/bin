use crate::dynamic_library::{DynamicLibrary, LibraryId};
use crate::execute_binary::{self, Query};
use crate::model::{Errors, Probe};
use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::path::Path;

#[cfg(target_env = "gnu")]
use std::{
    ffi::{CStr, CString, c_char, c_uint, c_void},
    ptr,
};

#[derive(Default)]
pub struct GpuApi {
    #[cfg(target_env = "gnu")]
    nvml: OnceCell<Probe<Nvml>>,
    binary_memory: OnceCell<Option<Probe<NvidiaMemory>>>,
}

#[derive(Default)]
pub struct NvidiaDetails {
    pub model: Option<String>,
    pub version: Option<String>,
    pub memory: Option<u64>,
}

impl GpuApi {
    pub fn nvidia(&self, pci_address: &str, errors: &mut Errors) -> NvidiaDetails {
        #[cfg(target_env = "gnu")]
        {
            let library = self.nvml.get_or_init(Nvml::new).as_ref();
            let result = library
                .map_err(Clone::clone)
                .and_then(|library| library.device(pci_address));
            let Some((library, device)) = errors.record("gpu.library", Some(pci_address), result)
            else {
                return NvidiaDetails::default();
            };
            NvidiaDetails {
                model: errors.record("gpu.library", Some(pci_address), library.name(device)),
                version: errors.record("gpu.library", Some(pci_address), library.driver_version()),
                memory: errors.record("gpu.library", Some(pci_address), library.memory(device)),
            }
        }
        #[cfg(not(target_env = "gnu"))]
        {
            errors.record(
                "gpu.library",
                Some(pci_address),
                DynamicLibrary::load(LibraryId::Nvml),
            );
            NvidiaDetails::default()
        }
    }

    pub fn binary_memory(&self, pci_address: &str, errors: &mut Errors) -> Option<u64> {
        let memory = self.binary_memory.get_or_init(|| {
            execute_binary::execute(Query::GpuSize)
                .map(|output| output.and_then(|output| parse_nvidia_memory(&output)))
        });
        let result = memory
            .as_ref()?
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|memory| {
                let pci = parse_pci_address(pci_address)
                    .ok_or_else(|| format!("invalid GPU PCI address {pci_address}"))?;
                memory
                    .get(&pci)
                    .cloned()
                    .ok_or_else(|| format!("nvidia-smi did not report PCI device {pci_address}"))?
            });
        errors.record("gpu.binary", Some(pci_address), result)
    }

    #[cfg(target_env = "gnu")]
    pub fn finish(self, errors: &mut Errors) {
        if let Some(Ok(nvml)) = self.nvml.into_inner() {
            errors.record("gpu.library", Some("NVML"), nvml.shutdown());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct PciAddress {
    domain: u32,
    bus: u8,
    slot: u8,
    function: u8,
}

type NvidiaMemory = BTreeMap<PciAddress, Probe<u64>>;

fn parse_pci_address(address: &str) -> Option<PciAddress> {
    let (domain, rest) = address.trim().split_once(':')?;
    let (bus, rest) = rest.split_once(':')?;
    let (slot, function) = rest.split_once('.')?;
    let address = PciAddress {
        domain: u32::from_str_radix(domain, 16).ok()?,
        bus: u8::from_str_radix(bus, 16).ok()?,
        slot: u8::from_str_radix(slot, 16).ok()?,
        function: u8::from_str_radix(function, 16).ok()?,
    };
    (address.slot < 32 && address.function < 8).then_some(address)
}

fn parse_nvidia_memory(output: &str) -> Probe<NvidiaMemory> {
    use std::collections::btree_map::Entry;

    let mut memory = BTreeMap::new();
    for line in output.lines() {
        let Some((pci, value)) = line.split_once(',') else {
            continue;
        };
        let Some(address) = parse_pci_address(pci) else {
            continue;
        };
        let size = value
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|&mib| mib > 0)
            .and_then(|mib| mib.checked_mul(1024 * 1024))
            .ok_or_else(|| {
                format!(
                    "nvidia-smi did not return a valid memory size for {}",
                    pci.trim()
                )
            });
        match memory.entry(address) {
            Entry::Vacant(entry) => {
                entry.insert(size);
            }
            Entry::Occupied(mut entry) if entry.get() != &size => {
                *entry.get_mut() = Err(format!(
                    "nvidia-smi returned conflicting memory sizes for {}",
                    pci.trim()
                ));
            }
            Entry::Occupied(_) => {}
        }
    }
    if memory.is_empty() {
        return Err("nvidia-smi returned no usable PCI device records".into());
    }
    Ok(memory)
}

#[cfg(target_env = "gnu")]
type Device = *mut c_void;
#[cfg(target_env = "gnu")]
type NvmlCall = unsafe extern "C" fn() -> c_uint;

#[cfg(target_env = "gnu")]
struct Nvml {
    library: DynamicLibrary,
    shutdown: NvmlCall,
}

#[cfg(target_env = "gnu")]
impl Nvml {
    fn new() -> Probe<Self> {
        let library = DynamicLibrary::load(LibraryId::Nvml)?;
        let shutdown = *unsafe { library.symbol::<NvmlCall>(c"nvmlShutdown") }?;
        let init = unsafe { library.symbol::<NvmlCall>(c"nvmlInit_v2") }?;
        nvml_status(unsafe { init() }, "initialize")?;
        Ok(Self { library, shutdown })
    }

    fn shutdown(self) -> Probe<()> {
        nvml_status(unsafe { (self.shutdown)() }, "shutdown")
    }

    fn device(&self, pci_address: &str) -> Probe<(&Self, Device)> {
        type Find = unsafe extern "C" fn(*const c_char, *mut Device) -> c_uint;
        let find = unsafe {
            self.library
                .symbol::<Find>(c"nvmlDeviceGetHandleByPciBusId_v2")
        }?;
        let address = CString::new(pci_address).map_err(|error| error.to_string())?;
        let mut device = ptr::null_mut();
        nvml_status(
            unsafe { find(address.as_ptr(), &mut device) },
            "lookup PCI device",
        )?;
        if device.is_null() {
            return Err("NVML returned a null device handle".into());
        }
        Ok((self, device))
    }

    fn name(&self, device: Device) -> Probe<String> {
        type Query = unsafe extern "C" fn(Device, *mut c_char, c_uint) -> c_uint;
        let query = unsafe { self.library.symbol::<Query>(c"nvmlDeviceGetName") }?;
        let mut name = [0u8; 64];
        nvml_status(
            unsafe { query(device, name.as_mut_ptr().cast(), name.len() as c_uint) },
            "GPU name",
        )?;
        nvml_string(&name, "GPU name")
    }

    fn driver_version(&self) -> Probe<String> {
        type Query = unsafe extern "C" fn(*mut c_char, c_uint) -> c_uint;
        let query = unsafe { self.library.symbol::<Query>(c"nvmlSystemGetDriverVersion") }?;
        let mut version = [0u8; 80];
        nvml_status(
            unsafe { query(version.as_mut_ptr().cast(), version.len() as c_uint) },
            "driver version",
        )?;
        nvml_string(&version, "driver version")
    }

    fn memory(&self, device: Device) -> Probe<u64> {
        #[repr(C)]
        #[derive(Default)]
        struct Memory {
            total: u64,
            free: u64,
            used: u64,
        }
        type Query = unsafe extern "C" fn(Device, *mut Memory) -> c_uint;
        let query = unsafe { self.library.symbol::<Query>(c"nvmlDeviceGetMemoryInfo") }?;
        let mut memory = Memory::default();
        nvml_status(unsafe { query(device, &mut memory) }, "GPU memory size")?;
        positive_memory(memory.total).map_err(|error| format!("NVML: {error}"))
    }
}

#[cfg(target_env = "gnu")]
fn nvml_status(status: c_uint, operation: &str) -> Probe<()> {
    if status == 0 {
        Ok(())
    } else {
        Err(format!("NVML {operation} failed with status {status}"))
    }
}

#[cfg(target_env = "gnu")]
fn nvml_string(bytes: &[u8], field: &str) -> Probe<String> {
    CStr::from_bytes_until_nul(bytes)
        .ok()
        .and_then(|value| crate::model::nonempty(&value.to_string_lossy()))
        .ok_or_else(|| format!("NVML returned an empty or truncated {field}"))
}

pub fn nouveau_memory(device: &Path) -> Probe<u64> {
    let nodes =
        fs::read_dir(device.join("drm")).map_err(|error| format!("Nouveau DRM nodes: {error}"))?;
    for node in nodes {
        let name = node.map_err(|error| error.to_string())?.file_name();
        if name.to_str().is_some_and(|name| {
            name.strip_prefix("renderD").is_some_and(|index| {
                !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
            })
        }) {
            let file = File::open(Path::new("/dev/dri").join(name))
                .map_err(|error| format!("Nouveau render node: {error}"))?;
            return query_nouveau_memory(&file);
        }
    }
    Err("Nouveau render node unavailable".into())
}

fn query_nouveau_memory(file: &File) -> Probe<u64> {
    // Linux UAPI drm_nouveau_getparam, DRM_IOCTL_NOUVEAU_GETPARAM, and NOUVEAU_GETPARAM_FB_SIZE.
    #[repr(C)]
    struct GetParam {
        param: u64,
        value: u64,
    }
    let mut query = GetParam { param: 8, value: 0 };
    let request = libc::_IOWR::<GetParam>(u32::from(b'd'), 0x40);
    if unsafe { libc::ioctl(file.as_raw_fd(), request, &mut query as *mut GetParam) } != 0 {
        return Err(format!(
            "Nouveau memory query: {}",
            io::Error::last_os_error()
        ));
    }
    positive_memory(query.value)
}

fn positive_memory(bytes: u64) -> Probe<u64> {
    (bytes > 0)
        .then_some(bytes)
        .ok_or_else(|| "GPU API reported no dedicated memory".into())
}

#[cfg(test)]
mod tests {
    use super::{parse_nvidia_memory, parse_pci_address};

    #[test]
    fn binary_sizes_match_both_pci_domain_formats_and_keep_partial_results() {
        let memory = parse_nvidia_memory(
            "00000000:01:00.0, 8192\n000000AB:0F:1F.7, [N/A]\ninvalid\n00000000:03:00.0, 0\n",
        )
        .expect("valid device rows");
        let address = |value| parse_pci_address(value).expect("valid PCI address");
        assert_eq!(memory[&address("0000:01:00.0")], Ok(8_589_934_592));
        assert!(memory[&address("00ab:0f:1f.7")].is_err());
        assert!(memory[&address("0000:03:00.0")].is_err());
        assert!(parse_nvidia_memory("no devices").is_err());
        assert!(parse_pci_address("0000:01:00.9").is_none());
    }

    #[test]
    fn rejects_conflicting_sizes_for_the_same_pci_device() {
        let memory = parse_nvidia_memory("0000:01:00.0, 8192\n00000000:01:00.0, 4096\n")
            .expect("device rows");
        assert!(memory[&parse_pci_address("0000:01:00.0").expect("PCI address")].is_err());
    }
}
