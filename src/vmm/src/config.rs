use std::path::PathBuf;

use linux_loader::loader::Cmdline;
use util::result::Result;

use crate::constants::CMDLINE_CAPACITY;

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub size_mib: usize,
    pub path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VcpuConfig {
    pub num: u8,
}

#[derive(Debug, Clone)]
pub struct KernelConfig {
    pub path: String,
    pub cmdline: Cmdline,
    pub initrd_path: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NetConfig {
    pub tap_name: String,
    pub ip_addr: String,
    pub netmask: String,
    pub gateway: String,
    pub mac_addr: String,
}

#[derive(Debug, Clone)]
pub struct BlockConfig {
    pub file_path: PathBuf,
    pub read_only: bool,
    pub root: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub memory: MemoryConfig,
    pub vcpu: VcpuConfig,
    pub kernel: KernelConfig,
    pub net: Option<NetConfig>,
    pub block: Vec<BlockConfig>,
}

impl KernelConfig {
    pub fn builder(kernel_path: impl AsRef<str>) -> Result<KernelConfigBuilder> {
        Ok(KernelConfigBuilder {
            path: kernel_path.as_ref().to_string(),
            cmdline: Cmdline::new(CMDLINE_CAPACITY)?,
            initrd_path: None,
        })
    }
}

pub struct KernelConfigBuilder {
    path: String,
    cmdline: Cmdline,
    initrd_path: Option<String>,
}

impl KernelConfigBuilder {
    pub fn with_cmdline(&mut self, cmd: impl AsRef<str>) -> Result<&mut Self> {
        self.cmdline.insert_str(cmd)?;
        Ok(self)
    }

    pub fn with_initrd(&mut self, initrd_path: impl AsRef<str>) -> &mut Self {
        self.initrd_path = Some(initrd_path.as_ref().to_string());
        self
    }

    pub fn build(&self) -> KernelConfig {
        KernelConfig {
            path: self.path.clone(),
            cmdline: self.cmdline.clone(),
            initrd_path: self.initrd_path.clone(),
        }
    }
}
