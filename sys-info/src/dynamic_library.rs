use crate::model::Probe;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryId {
    #[cfg(target_os = "linux")]
    Nvml,
    #[cfg(target_os = "macos")]
    Metal,
    #[cfg(target_os = "windows")]
    Dxgi,
    #[cfg(target_os = "windows")]
    Cuda,
}

impl LibraryId {
    pub fn name(self) -> &'static str {
        match self {
            #[cfg(target_os = "linux")]
            Self::Nvml => "NVML",
            #[cfg(target_os = "macos")]
            Self::Metal => "Metal",
            #[cfg(target_os = "windows")]
            Self::Dxgi => "DXGI",
            #[cfg(target_os = "windows")]
            Self::Cuda => "CUDA",
        }
    }
}

fn candidates(os: &str, arch: &str, library: LibraryId) -> &'static [&'static str] {
    match (os, arch, library) {
        #[cfg(target_os = "linux")]
        ("linux", "x86_64", LibraryId::Nvml) => &[
            "/usr/lib/x86_64-linux-gnu/libnvidia-ml.so.1",
            "/lib/x86_64-linux-gnu/libnvidia-ml.so.1",
            "/usr/lib64/libnvidia-ml.so.1",
            "/usr/lib64/nvidia/libnvidia-ml.so.1",
            "/usr/lib/libnvidia-ml.so.1",
            "/usr/local/nvidia/lib64/libnvidia-ml.so.1",
            "/usr/lib/wsl/lib/libnvidia-ml.so.1",
        ],
        #[cfg(target_os = "linux")]
        ("linux", "aarch64", LibraryId::Nvml) => &[
            "/usr/lib/aarch64-linux-gnu/libnvidia-ml.so.1",
            "/lib/aarch64-linux-gnu/libnvidia-ml.so.1",
            "/usr/lib/aarch64-linux-gnu/nvidia/libnvidia-ml.so.1",
            "/usr/lib/aarch64-linux-gnu/tegra/libnvidia-ml.so.1",
            "/usr/lib64/libnvidia-ml.so.1",
            "/usr/lib/libnvidia-ml.so.1",
            "/usr/local/nvidia/lib64/libnvidia-ml.so.1",
        ],
        #[cfg(target_os = "macos")]
        ("macos", "x86_64" | "aarch64", LibraryId::Metal) => {
            &["/System/Library/Frameworks/Metal.framework/Versions/A/Metal"]
        }
        #[cfg(target_os = "windows")]
        ("windows", "x86_64" | "aarch64", LibraryId::Dxgi) => &["dxgi.dll"],
        #[cfg(target_os = "windows")]
        ("windows", "x86_64" | "aarch64", LibraryId::Cuda) => &["nvcuda.dll"],
        _ => &[],
    }
}

#[cfg(not(all(target_os = "linux", not(target_env = "gnu"))))]
pub struct DynamicLibrary {
    handle: libloading::Library,
    path: &'static str,
}

#[cfg(all(target_os = "linux", not(target_env = "gnu")))]
pub struct DynamicLibrary;

impl DynamicLibrary {
    pub fn load(library: LibraryId) -> Probe<Self> {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let paths = candidates(os, arch, library);
        if paths.is_empty() {
            return Err(format!(
                "{} has no allowlisted library for {os}/{arch}",
                library.name()
            ));
        }

        #[cfg(all(target_os = "linux", not(target_env = "gnu")))]
        return Err(format!(
            "{} cannot be loaded by this static Linux build",
            library.name()
        ));

        #[cfg(not(all(target_os = "linux", not(target_env = "gnu"))))]
        {
            let mut failures = Vec::new();
            for &path in paths {
                // The requested library and path come exclusively from the built-in table.
                #[cfg(unix)]
                let result = unsafe {
                    libloading::os::unix::Library::open(
                        Some(path),
                        libloading::os::unix::RTLD_NOW | libloading::os::unix::RTLD_LOCAL,
                    )
                };
                #[cfg(windows)]
                let result = unsafe {
                    libloading::os::windows::Library::load_with_flags(
                        path,
                        libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32,
                    )
                };
                match result {
                    Ok(handle) => {
                        return Ok(Self {
                            handle: handle.into(),
                            path,
                        });
                    }
                    Err(error) => failures.push(format!("{path}: {error}")),
                }
            }
            Err(format!(
                "cannot load allowlisted {} library ({})",
                library.name(),
                failures.join(", ")
            ))
        }
    }

    /// The caller must supply the function signature defined by the library ABI.
    #[cfg(not(all(target_os = "linux", not(target_env = "gnu"))))]
    pub unsafe fn symbol<T>(&self, name: &std::ffi::CStr) -> Probe<libloading::Symbol<'_, T>> {
        unsafe { self.handle.get(name.to_bytes_with_nul()) }
            .map_err(|error| format!("{} export {}: {error}", self.path, name.to_string_lossy()))
    }
}

#[cfg(test)]
mod tests {
    use super::{LibraryId, candidates};

    #[test]
    fn allows_only_expected_os_arch_and_paths() {
        #[cfg(target_os = "linux")]
        let library = LibraryId::Nvml;
        #[cfg(target_os = "macos")]
        let library = LibraryId::Metal;
        #[cfg(target_os = "windows")]
        let library = LibraryId::Dxgi;

        let os = std::env::consts::OS;
        for arch in ["x86_64", "aarch64"] {
            let paths = candidates(os, arch, library);
            assert!(!paths.is_empty());
            #[cfg(unix)]
            assert!(paths.iter().all(|path| path.starts_with('/')));
            #[cfg(windows)]
            assert_eq!(paths, ["dxgi.dll"]);
        }
        assert!(candidates(os, "riscv64", library).is_empty());
        assert!(candidates("unknown", "x86_64", library).is_empty());
    }
}
