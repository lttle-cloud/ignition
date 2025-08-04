#![allow(dead_code)]

pub const DEFAULT_KERNEL_CMD_LINE_INIT: &'static str =
    "i8042.nokbd reboot=t panic=1 noapic clocksource=kvm-clock tsc=reliable console=ttyS0";

pub const DEFAULT_SUSPEND_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_TRAFFIC_AWARE_INACTIVITY_TIMEOUT_SECS: u64 = 5;
pub const DEFAULT_NAMESPACE: &str = "default";
pub const DEFAULT_AGENT_TENANT: &str = "agent";
