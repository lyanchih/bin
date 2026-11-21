use crate::model::{Cpu, Distribution, Errors, Ram, nonempty};
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

pub fn collect(errors: &mut Errors) -> (Cpu, Ram) {
    let system = System::new_with_specifics(
        RefreshKind::nothing()
            .with_cpu(CpuRefreshKind::nothing())
            .with_memory(MemoryRefreshKind::nothing().with_ram()),
    );
    let mut models = Vec::new();
    let mut vendors = Vec::new();
    for cpu in system.cpus() {
        if let Some(model) = nonempty(cpu.brand())
            && !models.contains(&model)
        {
            models.push(model);
        }
        let vendor = match cpu.vendor_id().trim() {
            "GenuineIntel" => Some("Intel".into()),
            "AuthenticAMD" => Some("AMD".into()),
            "HygonGenuine" => Some("Hygon".into()),
            "" | "Unknown" | "unknown" => None,
            vendor => nonempty(vendor),
        };
        if let Some(vendor) = vendor
            && !vendors.contains(&vendor)
        {
            vendors.push(vendor);
        }
    }
    let model = nonempty(&models.join(" / "));
    #[cfg(target_os = "windows")]
    let model = crate::platform::cpu_model().ok().or(model);
    let band = model
        .as_deref()
        .and_then(|model| match model.split_whitespace().next()? {
            "Qualcomm" | "Snapdragon" => Some("Qualcomm".to_owned()),
            "Apple" | "Ampere" | "MediaTek" => model.split_whitespace().next().map(str::to_owned),
            _ => None,
        })
        .or_else(|| nonempty(&vendors.join(" / ")));
    let frequency = base_frequency(model.as_deref());
    let model = model.map(|models| {
        models
            .split(" / ")
            .map(simple_cpu_type)
            .collect::<Vec<_>>()
            .join(" / ")
    });
    let count = system.cpus().len();
    let architecture = crate::platform::architecture();
    let architecture = match architecture.trim().to_ascii_lowercase().as_str() {
        "amd64" | "x64" => Some("x86_64".into()),
        "arm64" => Some("aarch64".into()),
        "unknown" => None,
        _ => nonempty(&architecture),
    };
    let bytes = system.total_memory();
    let cpu = Cpu {
        model: errors.require("cpu.type", None, model, "processor model unavailable"),
        band: errors.require("cpu.band", None, band, "processor manufacturer unavailable"),
        frequency: errors.require(
            "cpu.frequency",
            None,
            frequency,
            "CPU base frequency unavailable",
        ),
        arch: errors.require(
            "cpu.arch",
            None,
            architecture,
            "CPU architecture unavailable",
        ),
        count: errors.require(
            "cpu.count",
            None,
            (count > 0).then_some(count),
            "logical CPU count unavailable",
        ),
        features: cpu_features(),
    };
    let ram = Ram {
        total: errors.require(
            "ram.total",
            None,
            (bytes > 0).then_some(bytes),
            "physical RAM size unavailable",
        ),
    };
    (cpu, ram)
}

fn cpu_features() -> Vec<&'static str> {
    #[cfg(target_arch = "x86_64")]
    let features = [
        ("sse2", std::is_x86_feature_detected!("sse2")),
        ("sse4.1", std::is_x86_feature_detected!("sse4.1")),
        ("avx", std::is_x86_feature_detected!("avx")),
        ("avx2", std::is_x86_feature_detected!("avx2")),
        ("f16c", std::is_x86_feature_detected!("f16c")),
        ("fma", std::is_x86_feature_detected!("fma")),
        ("avx512f", std::is_x86_feature_detected!("avx512f")),
        ("avx512vl", std::is_x86_feature_detected!("avx512vl")),
        ("avx512bw", std::is_x86_feature_detected!("avx512bw")),
        ("avx512bf16", std::is_x86_feature_detected!("avx512bf16")),
        ("avx512fp16", std::is_x86_feature_detected!("avx512fp16")),
        ("avx512vnni", std::is_x86_feature_detected!("avx512vnni")),
    ];
    #[cfg(target_arch = "aarch64")]
    let features = [
        ("neon", std::arch::is_aarch64_feature_detected!("neon")),
        ("fp16", std::arch::is_aarch64_feature_detected!("fp16")),
        (
            "dotprod",
            std::arch::is_aarch64_feature_detected!("dotprod"),
        ),
        ("sve", std::arch::is_aarch64_feature_detected!("sve")),
        ("sve2", std::arch::is_aarch64_feature_detected!("sve2")),
        ("bf16", std::arch::is_aarch64_feature_detected!("bf16")),
        ("i8mm", std::arch::is_aarch64_feature_detected!("i8mm")),
    ];
    features
        .into_iter()
        .filter_map(|(name, supported)| supported.then_some(name))
        .collect()
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub fn compute_capability(major: i32, minor: i32) -> crate::model::Probe<String> {
    if major > 0 && minor >= 0 {
        Ok(format!("{major}.{minor}"))
    } else {
        Err("invalid CUDA compute capability".into())
    }
}

fn base_frequency(models: Option<&str>) -> Option<u64> {
    #[cfg(target_os = "linux")]
    if let Some(frequency) = crate::platform::cpu_base_frequency() {
        return Some(frequency);
    }
    let advertised = models
        .into_iter()
        .flat_map(|models| models.split(" / "))
        .filter_map(advertised_base_frequency)
        .max();
    advertised.or_else(cpuid_base_frequency)
}

fn advertised_base_frequency(model: &str) -> Option<u64> {
    let (_, frequency) = model.rsplit_once('@')?;
    let frequency = frequency.trim();
    let (value, scale) = [
        ("GHz", 1_000_000_000u64),
        ("MHz", 1_000_000),
        ("kHz", 1_000),
    ]
    .into_iter()
    .find_map(|(unit, scale)| Some((frequency.strip_suffix(unit)?.trim(), scale)))?;
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let mut hz = whole.parse::<u64>().ok()?.checked_mul(scale)?;
    if !fraction.is_empty() {
        let digits = u32::try_from(fraction.len()).ok()?;
        let divisor = 10u64.checked_pow(digits)?;
        hz = hz.checked_add(fraction.parse::<u64>().ok()?.checked_mul(scale)? / divisor)?;
    }
    (hz > 0).then_some(hz)
}

fn cpuid_base_frequency() -> Option<u64> {
    #[cfg(target_arch = "x86_64")]
    {
        // Leaf 0x16 EAX starts with the reported base frequency in MHz.
        if cpuid(0).eax >= 0x16 {
            let mhz = cpuid(0x16).eax & 0xffff;
            return (mhz > 0).then(|| u64::from(mhz) * 1_000_000);
        }
    }
    None
}

#[cfg(target_arch = "x86_64")]
pub fn cpuid(leaf: u32) -> std::arch::x86_64::CpuidResult {
    // SAFETY: All x86_64 processors support CPUID. Rust 1.88 marks it unsafe.
    #[allow(unused_unsafe)]
    unsafe {
        std::arch::x86_64::__cpuid(leaf)
    }
}

fn normalized_hardware_name(name: &str) -> String {
    name.replace(['®', '™'], "")
        .replace("(R)", "")
        .replace("(TM)", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn simple_cpu_type(model: &str) -> String {
    let normalized = normalized_hardware_name(model);
    let mut name = normalized.split(" @ ").next().unwrap_or(&normalized);
    if let Some((part, _)) = name.split_once(" - Qualcomm") {
        name = part;
    }
    let tokens: Vec<_> = name.split_whitespace().collect();
    // Core i-series already encodes the family in the model number.
    if tokens.contains(&"Intel")
        && let Some(part) = tokens.iter().find(|part| {
            part.as_bytes()
                .get(..3)
                .is_some_and(|prefix| matches!(prefix, b"i3-" | b"i5-" | b"i7-" | b"i9-"))
                && part.as_bytes().get(3).is_some_and(u8::is_ascii_digit)
        })
    {
        return (*part).to_owned();
    }
    let mut words: Vec<_> = tokens
        .iter()
        .copied()
        .filter(|&word| word != "CPU" && word != "Processor")
        .collect();
    if words.len() > 1 && matches!(words[0], "Intel" | "AMD" | "Apple" | "Qualcomm") {
        words.remove(0);
    }
    if words.last().is_some_and(|word| {
        word.strip_suffix("-Core")
            .is_some_and(|count| count.parse::<u32>().is_ok())
    }) {
        words.pop();
    }
    nonempty(&words.join(" ")).unwrap_or_else(|| model.to_owned())
}

pub fn simple_gpu_type(model: &str) -> String {
    let normalized = normalized_hardware_name(model);
    let product = normalized
        .split('[')
        .skip(1)
        .filter_map(|suffix| suffix.split_once(']').map(|(inside, _)| inside))
        .find(|part| {
            matches!(
                part.split_whitespace().next(),
                Some("GeForce" | "RTX" | "GTX" | "Radeon" | "Arc" | "Quadro" | "Tesla")
            )
        });
    let name = product.unwrap_or(&normalized);
    let name = [
        "NVIDIA Corporation ",
        "Intel Corporation ",
        "Advanced Micro Devices, Inc. [AMD/ATI] ",
        "NVIDIA ",
        "AMD ",
        "Intel ",
        "Apple ",
        "Qualcomm ",
    ]
    .into_iter()
    .find_map(|vendor| name.strip_prefix(vendor))
    .unwrap_or(name);
    let name = if name.starts_with("Radeon RX ") {
        name.strip_prefix("Radeon ").unwrap_or(name)
    } else if ["GeForce RTX ", "GeForce GTX ", "GeForce GT "]
        .iter()
        .any(|prefix| name.starts_with(prefix))
    {
        name.strip_prefix("GeForce ").unwrap_or(name)
    } else {
        name
    };
    nonempty(name).unwrap_or_else(|| model.to_owned())
}

pub fn gpu_band(vendor_id: Option<u32>, model: Option<&str>) -> Option<String> {
    let vendor = match vendor_id {
        Some(0x1002 | 0x1022) => "AMD",
        Some(0x10de) => "NVIDIA",
        Some(0x8086) => "Intel",
        Some(0x106b) => "Apple",
        Some(0x5143 | 0x17cb) => "Qualcomm",
        Some(0x13b5) => "Arm",
        Some(0x1010) => "Imagination",
        Some(0x1414) => "Microsoft",
        Some(0x15ad) => "VMware",
        Some(0x1af4) => "Red Hat",
        Some(0x1a03) => "ASPEED",
        _ => match model?.split_whitespace().next()? {
            "AMD" | "ATI" | "Radeon" => "AMD",
            "NVIDIA" | "GeForce" => "NVIDIA",
            "Intel" => "Intel",
            "Apple" => "Apple",
            "Qualcomm" | "Adreno" => "Qualcomm",
            "Arm" | "ARM" | "Mali" => "Arm",
            "PowerVR" | "Imagination" => "Imagination",
            "ASPEED" => "ASPEED",
            _ => return None,
        },
    };
    Some(vendor.to_owned())
}

pub fn distribution(
    os: &'static str,
    dis: Option<String>,
    name: Option<String>,
    version: Option<String>,
    errors: &mut Errors,
) -> Distribution {
    Distribution {
        os,
        libc: None,
        libc_version: None,
        dis: errors.require(
            "distribution.dis",
            None,
            dis,
            "distribution identifier unavailable",
        ),
        name: errors.require("distribution.name", None, name, "OS name unavailable"),
        version: errors.require(
            "distribution.version",
            None,
            version,
            "OS version unavailable",
        ),
        kernel_version: errors.require(
            "distribution.kernel_version",
            None,
            System::kernel_version().and_then(|value| nonempty(&value)),
            "kernel version unavailable",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{advertised_base_frequency, simple_cpu_type, simple_gpu_type};

    #[test]
    fn only_an_advertised_base_clock_is_parsed_from_a_model() {
        assert_eq!(
            advertised_base_frequency("Intel Core i5-9400 CPU @ 2.90GHz"),
            Some(2_900_000_000)
        );
        assert_eq!(
            advertised_base_frequency("CPU @ 2300 MHz"),
            Some(2_300_000_000)
        );
        assert_eq!(advertised_base_frequency("CPU up to 4.10GHz"), None);
        assert_eq!(advertised_base_frequency("Apple M4 Max"), None);
        assert_eq!(advertised_base_frequency("CPU @ 0.00GHz"), None);
    }

    #[test]
    fn model_names_keep_identifiers_and_distinguishing_suffixes() {
        assert_eq!(
            simple_cpu_type("Intel(R) Core(TM) i5-9400 CPU @ 2.90GHz"),
            "i5-9400"
        );
        assert_eq!(
            simple_cpu_type("12th Gen Intel(R) Core(TM) i7-12700K"),
            "i7-12700K"
        );
        assert_eq!(
            simple_cpu_type("Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz"),
            "Xeon Gold 6338"
        );
        assert_eq!(
            simple_cpu_type("AMD Ryzen 7 7800X3D 8-Core Processor"),
            "Ryzen 7 7800X3D"
        );
        assert_eq!(simple_cpu_type("Apple M4 Max"), "M4 Max");
        assert_eq!(simple_gpu_type("NVIDIA GeForce RTX 3060 Ti"), "RTX 3060 Ti");
        assert_eq!(
            simple_gpu_type("NVIDIA Corporation GA106M [GeForce RTX 3060 Laptop GPU]"),
            "RTX 3060 Laptop GPU"
        );
        assert_eq!(simple_gpu_type("AMD Radeon RX 7900 XTX"), "RX 7900 XTX");
        assert_eq!(
            simple_gpu_type("Intel(R) Arc(TM) A770 Graphics"),
            "Arc A770 Graphics"
        );
        assert_eq!(simple_gpu_type("Apple M4 Max"), "M4 Max");
        assert_eq!(
            simple_gpu_type("Example Video Adapter"),
            "Example Video Adapter"
        );
    }
}
