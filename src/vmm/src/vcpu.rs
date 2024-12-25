use crate::cpu_ref;
use kvm_bindings::{CpuId, Msrs};
use kvm_ioctls::Kvm;
use util::result::{bail, Result};

pub struct VmConfig {
    pub vcpus_count: u8,
    pub vcpus_config: VcpusConfigList,
    pub max_irq: u32,
}

impl VmConfig {
    pub fn new(kvm: &Kvm, vcpus_count: u8, max_irq: u32) -> Result<Self> {
        Ok(VmConfig {
            vcpus_count,
            vcpus_config: VcpusConfigList::new(kvm, vcpus_count)?,
            max_irq,
        })
    }
}

pub struct VcpuConfig {
    pub id: u8,
    pub cpuid: CpuId,
    pub msrs: Msrs,
}

pub struct VcpusConfigList {
    pub configs: Vec<VcpuConfig>,
}

impl VcpusConfigList {
    pub fn new(kvm: &Kvm, vcpus_count: u8) -> Result<Self> {
        if vcpus_count == 0 {
            bail!("At least one vCPU must be defined");
        }

        let base_cpuid = kvm.get_supported_cpuid(kvm_bindings::KVM_MAX_CPUID_ENTRIES)?;
        let supported_msrs = cpu_ref::msrs::supported_guest_msrs(kvm)?;

        let mut configs = vec![];
        for index in 0..vcpus_count {
            let mut cpuid = base_cpuid.clone();
            cpu_ref::cpuid::filter_cpuid(kvm, index, vcpus_count, &mut cpuid);

            let vcpu_config = VcpuConfig {
                id: index,
                cpuid,
                msrs: supported_msrs.clone(),
            };

            configs.push(vcpu_config);
        }

        return Ok(VcpusConfigList { configs });
    }
}

pub trait ExitHandler: Clone {
    fn kick(&self) -> Result<()>;
}
