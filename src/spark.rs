use util::result::Result;
use vmm::{
    config::{Config, KernelConfig, MemoryConfig, VcpuConfig},
    vmm::Vmm,
};

async fn spark() -> Result<()> {
    println!("Sparkling...");

    let config = Config {
        memory: MemoryConfig { size_mib: 128 },
        vcpu: VcpuConfig { num: 1 },
        kernel: KernelConfig::builder("../linux/vmlinux")?
            .with_initrd("./target/takeoff.cpio")
            .with_cmdline("i8042.nokbd reboot=t panic=1 pci=off")?
            .build(),
    };

    let start_time = std::time::Instant::now();
    let mut vm = Vmm::new(config)?;
    let elapsed_us = start_time.elapsed().as_micros();
    println!("VM creation took {}us", elapsed_us);

    let start_time = std::time::Instant::now();
    vm.run()?;
    let elapsed_us = start_time.elapsed().as_micros();
    println!("VM run took {}us", elapsed_us);

    println!("Sparkled!");

    Ok(())
}

fn main() -> Result<()> {
    util::async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(spark())?;

    Ok(())
}
