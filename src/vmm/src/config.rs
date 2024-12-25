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
    pub cmdline: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub memory: MemoryConfig,
    pub vcpu: VcpuConfig,
    pub kernel: KernelConfig,
}
