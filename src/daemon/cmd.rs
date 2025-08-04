use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(name = "ignitiond")]
#[command(author = "lttle.cloud")]
#[command(about = "ignition daemon", long_about = None)]
pub struct Cli {
    /// Path to the config file. If not provided, the daemon will look for a config file in the
    /// current working directory, in the home config dir ($HOME/.config/lttle/ignition.toml) or
    /// in the system config dir (/etc/lttle/ignition.toml)
    #[arg(long = "config", short = 'c')]
    pub config_path: Option<PathBuf>,
}
