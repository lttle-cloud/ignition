use linux_loader::loader::Cmdline;
use util::result::Result;

use crate::constants::CMDLINE_CAPACITY;

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub size_mib: usize,
}

#[derive(Debug, Clone)]
pub struct VcpuConfig {
    pub num: u8,
}

#[derive(Debug, Clone)]
pub struct KernelConfig {
    pub path: String,
    pub cmdline: Cmdline,
}

impl KernelConfig {
    pub fn new(path: impl AsRef<str>, cmd: impl AsRef<str>) -> Result<Self> {
        let mut cmdline = Cmdline::new(CMDLINE_CAPACITY)?;
        cmdline.insert_str(cmd)?;

        Ok(KernelConfig {
            path: path.as_ref().to_string(),
            cmdline,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub memory: MemoryConfig,
    pub vcpu: VcpuConfig,
    pub kernel: KernelConfig,
}
