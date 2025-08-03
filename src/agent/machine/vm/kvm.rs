use anyhow::{Result, bail};
use kvm_ioctls::Kvm;

const REQUIRED_CAPS: &[kvm_ioctls::Cap] = &[
    kvm_ioctls::Cap::Irqchip,
    kvm_ioctls::Cap::Ioeventfd,
    kvm_ioctls::Cap::Irqfd,
    kvm_ioctls::Cap::UserMemory,
];

pub fn create_and_verify_kvm() -> Result<Kvm> {
    let kvm = Kvm::new()?;

    for cap in REQUIRED_CAPS.iter() {
        if !kvm.check_extension(*cap) {
            bail!("required KVM cap not supported: {:?}", cap);
        }
    }

    Ok(kvm)
}
