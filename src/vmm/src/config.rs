use std::path::PathBuf;

use linux_loader::loader::Cmdline;
use util::result::Result;

use crate::constants::CMDLINE_CAPACITY;

#[derive(Debug, Clone)]
pub struct MemoryConfig {
    pub size_mib: usize,
    pub path: Option<String>,
}

impl MemoryConfig {
    pub fn new(size_mib: usize) -> MemoryConfig {
        MemoryConfig {
            size_mib,
            path: None,
        }
    }

    pub fn with_path(mut self, value: impl AsRef<str>) -> Self {
        self.path = Some(value.as_ref().to_string());
        self
    }
}

#[derive(Debug, Clone)]
pub struct VcpuConfig {
    pub num: u8,
}

impl VcpuConfig {
    pub fn new(num: u8) -> VcpuConfig {
        VcpuConfig { num }
    }
}

#[derive(Debug, Clone)]
pub struct KernelConfig {
    pub path: String,
    pub cmdline: Cmdline,
    pub initrd_path: Option<String>,
}

impl KernelConfig {
    pub fn new(kernel_path: impl AsRef<str>) -> Result<Self> {
        let cmdline = Cmdline::new(CMDLINE_CAPACITY)?;

        Ok(KernelConfig {
            path: kernel_path.as_ref().to_string(),
            cmdline,
            initrd_path: None,
        })
    }

    pub fn with_cmdline(mut self, value: impl AsRef<str>) -> Result<Self> {
        self.cmdline.insert_str(value)?;
        Ok(self)
    }

    pub fn with_initrd(mut self, value: impl AsRef<str>) -> Self {
        self.initrd_path = Some(value.as_ref().to_string());
        self
    }
}

#[derive(Debug, Clone)]
pub struct NetConfig {
    pub tap_name: String,
    pub ip_addr: String,
    pub netmask: String,
    pub gateway: String,
    pub mac_addr: String,
    pub listen_trigger_count: u32,
}

impl NetConfig {
    pub fn new(
        tap_name: impl AsRef<str>,
        ip_addr: impl AsRef<str>,
        netmask: impl AsRef<str>,
        gateway: impl AsRef<str>,
        mac_addr: impl AsRef<str>,
    ) -> Self {
        NetConfig {
            tap_name: tap_name.as_ref().to_string(),
            ip_addr: ip_addr.as_ref().to_string(),
            netmask: netmask.as_ref().to_string(),
            gateway: gateway.as_ref().to_string(),
            mac_addr: mac_addr.as_ref().to_string(),
            listen_trigger_count: 1,
        }
    }

    pub fn with_listen_trigger_count(mut self, value: u32) -> Self {
        self.listen_trigger_count = value;
        self
    }
}

#[derive(Debug, Clone)]
pub struct BlockConfig {
    pub file_path: PathBuf,
    pub read_only: bool,
    pub root: bool,
    pub mount_at: Option<String>,
}

impl BlockConfig {
    pub fn new(file_path: impl Into<PathBuf>) -> Self {
        BlockConfig {
            file_path: file_path.into(),
            read_only: true,
            root: false,
            mount_at: None,
        }
    }

    pub fn writeable(mut self) -> Self {
        self.read_only = false;
        self
    }

    pub fn root(mut self) -> Self {
        self.root = true;
        self
    }

    pub fn mount_at(mut self, value: impl AsRef<str>) -> Self {
        self.mount_at = Some(value.as_ref().to_string());
        self
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub memory: MemoryConfig,
    pub vcpu: VcpuConfig,
    pub kernel: KernelConfig,
    pub net: Option<NetConfig>,
    pub block: Vec<BlockConfig>,
}

pub struct Set;
pub struct NotSet;

pub struct ConfigBuilder<V, M, K> {
    memory: Option<MemoryConfig>,
    vcpu: Option<VcpuConfig>,
    kernel: Option<KernelConfig>,
    net: Option<NetConfig>,
    block: Vec<BlockConfig>,

    _marker: std::marker::PhantomData<(V, M, K)>,
}

impl<M, V, K> ConfigBuilder<M, V, K> {
    pub fn memory(self, mem: MemoryConfig) -> ConfigBuilder<Set, V, K> {
        ConfigBuilder {
            memory: Some(mem),
            vcpu: self.vcpu,
            kernel: self.kernel,
            net: self.net,
            block: self.block,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn vcpu(self, vcpu: VcpuConfig) -> ConfigBuilder<M, Set, K> {
        ConfigBuilder {
            memory: self.memory,
            vcpu: Some(vcpu),
            kernel: self.kernel,
            net: self.net,
            block: self.block,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn kernel(self, kernel: KernelConfig) -> ConfigBuilder<M, V, Set> {
        ConfigBuilder {
            memory: self.memory,
            vcpu: self.vcpu,
            kernel: Some(kernel),
            net: self.net,
            block: self.block,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_net(self, net: NetConfig) -> Self {
        ConfigBuilder {
            memory: self.memory,
            vcpu: self.vcpu,
            kernel: self.kernel,
            net: Some(net),
            block: self.block,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_block(mut self, block: BlockConfig) -> Self {
        self.block.push(block);

        ConfigBuilder {
            memory: self.memory,
            vcpu: self.vcpu,
            kernel: self.kernel,
            net: self.net,
            block: self.block,
            _marker: std::marker::PhantomData,
        }
    }
}

impl From<ConfigBuilder<Set, Set, Set>> for Config {
    fn from(builder: ConfigBuilder<Set, Set, Set>) -> Self {
        Config {
            memory: builder.memory.unwrap(),
            vcpu: builder.vcpu.unwrap(),
            kernel: builder.kernel.unwrap(),
            net: builder.net,
            block: builder.block,
        }
    }
}

impl Config {
    pub fn new() -> ConfigBuilder<NotSet, NotSet, NotSet> {
        ConfigBuilder {
            memory: None,
            vcpu: None,
            kernel: None,
            net: None,
            block: Vec::new(),
            _marker: std::marker::PhantomData,
        }
    }
}
