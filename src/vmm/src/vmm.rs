use crate::{
    config::Config,
    constants,
    device::SharedDeviceManager,
    memory::{self, Memory},
    vcpu::{VcpusConfigList, VmConfig},
};
use kvm_ioctls::Kvm;
use util::result::{bail, Result};

pub struct Vmm {
    config: Config,
    memory: Memory,
    device_manager: SharedDeviceManager,
}

impl Vmm {
    pub fn new(config: Config) -> Result<Self> {
        let kvm = Kvm::new()?;
        Vmm::check_kvm_caps(&kvm)?;

        let memory = Memory::new(config.memory.clone())?;
        let device_manager = SharedDeviceManager::new();

        let vm_config = VmConfig::new(&kvm, config.vcpu.num, constants::MAX_IRQ)?;

        Ok(Vmm {
            config,
            memory,
            device_manager,
        })
    }

    fn check_kvm_caps(kvm: &Kvm) -> Result<()> {
        let required_caps = vec![
            kvm_ioctls::Cap::Irqchip,
            kvm_ioctls::Cap::Ioeventfd,
            kvm_ioctls::Cap::Irqfd,
            kvm_ioctls::Cap::UserMemory,
        ];

        for cap in required_caps {
            if !kvm.check_extension(cap) {
                bail!("required KVM cap not supported: {:?}", cap);
            }
        }

        Ok(())
    }
}
