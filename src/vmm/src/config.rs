use std::path::PathBuf;

use linux_loader::loader::Cmdline;
use util::{encoding::codec, result::Result};

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

    pub fn with_init_envs(mut self, value: Vec<impl AsRef<str>>) -> Result<Self> {
        for env in value {
            self.cmdline
                .insert_str(format!("--takeoff-env={}", env.as_ref()))?;
        }
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
        }
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
    pub log_file_path: Option<String>,
    pub snapshot_policy: Option<SnapshotPolicy>,
}

pub struct Set;
pub struct NotSet;

#[codec]
#[derive(Clone, Debug)]
pub enum SnapshotPolicy {
    OnNthListenSyscall(u32),
    OnListenOnPort(u16),
    OnUserspaceReady,
    Manual,
}

pub struct ConfigBuilder<V, M, K> {
    memory: Option<MemoryConfig>,
    vcpu: Option<VcpuConfig>,
    kernel: Option<KernelConfig>,
    net: Option<NetConfig>,
    block: Vec<BlockConfig>,
    log_file_path: Option<String>,
    snapshot_policy: Option<SnapshotPolicy>,
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
            log_file_path: self.log_file_path,
            snapshot_policy: self.snapshot_policy,
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
            log_file_path: self.log_file_path,
            snapshot_policy: self.snapshot_policy,
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
            log_file_path: self.log_file_path,
            snapshot_policy: self.snapshot_policy,
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
            log_file_path: self.log_file_path,
            snapshot_policy: self.snapshot_policy,
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
            log_file_path: self.log_file_path,
            snapshot_policy: self.snapshot_policy,
            _marker: std::marker::PhantomData,
        }
    }

    pub fn with_log_file_path(mut self, value: impl AsRef<str>) -> Self {
        self.log_file_path = Some(value.as_ref().to_string());
        self
    }

    pub fn with_snapshot_policy(mut self, policy: SnapshotPolicy) -> Self {
        self.snapshot_policy = Some(policy);
        self
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
            log_file_path: builder.log_file_path,
            snapshot_policy: builder.snapshot_policy,
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
            log_file_path: None,
            snapshot_policy: None,
            _marker: std::marker::PhantomData,
        }
    }
}
