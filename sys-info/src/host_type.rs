use crate::{model::Probe, platform};
use serde::Serialize;
use std::env;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum HostType {
    Baremetal,
    Vm,
    Microvm,
    #[cfg(not(target_os = "macos"))]
    Container,
    Lambda,
    #[cfg(target_os = "linux")]
    Wsl,
}

pub fn detect() -> Probe<HostType> {
    let lambda = env_value("AWS_LAMBDA_FUNCTION_NAME").is_some()
        && (env_value("AWS_LAMBDA_RUNTIME_API").is_some()
            || env_value("AWS_EXECUTION_ENV")
                .is_some_and(|runtime| runtime.starts_with("AWS_Lambda_")));
    if lambda {
        return Ok(HostType::Lambda);
    }
    platform::host_type()
}

pub fn env_value(name: &str) -> Option<String> {
    let value = env::var_os(name)?;
    crate::model::nonempty(&value.to_string_lossy())
}

pub fn virtual_machine(identity: &str) -> Option<HostType> {
    let identity = identity.to_ascii_lowercase();
    if ["firecracker", "cloud hypervisor", "microvm"]
        .iter()
        .any(|name| identity.contains(name))
    {
        return Some(HostType::Microvm);
    }
    // EC2 also exposes physical .metal instances with the same manufacturer.
    if identity.contains("amazon ec2") && identity.contains(".metal") {
        return Some(HostType::Baremetal);
    }
    [
        "kvm",
        "qemu",
        "vmware",
        "virtualbox",
        "innotek",
        "virtual machine",
        "virtualmac",
        "hyper-v",
        "xen",
        "bhyve",
        "parallels",
        "amazon ec2",
        "google compute engine",
        "openstack",
        "bochs",
        "digitalocean",
        "dummy-virt",
    ]
    .iter()
    .any(|name| identity.contains(name))
    .then_some(HostType::Vm)
}

#[cfg(test)]
mod tests {
    use super::{HostType, virtual_machine};

    #[test]
    fn distinguishes_vm_and_microvm_signatures_from_physical_hardware() {
        assert_eq!(
            virtual_machine("Firecracker microVM"),
            Some(HostType::Microvm)
        );
        assert_eq!(
            virtual_machine("Apple Inc. VirtualMac2,1"),
            Some(HostType::Vm)
        );
        assert_eq!(
            virtual_machine("Amazon EC2 c7i.metal-24xl"),
            Some(HostType::Baremetal)
        );
        assert_eq!(
            virtual_machine("Amazon EC2 c7i.4xlarge"),
            Some(HostType::Vm)
        );
        assert_eq!(virtual_machine("Dell PowerEdge R760"), None);
    }
}
