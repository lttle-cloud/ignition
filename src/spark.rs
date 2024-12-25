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
        kernel: KernelConfig {
            path: "../linux/vmlinux".into(),
            cmdline: "".into(),
        },
    };

    let start_time = std::time::Instant::now();
    let vm = Vmm::new(config)?;
    let elapsed_us = start_time.elapsed().as_micros();

    println!("VM created in {}us", elapsed_us);

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
