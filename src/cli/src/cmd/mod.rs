mod admin;
mod deploy;
mod login;
mod machine;
mod service;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use util::result::Result;

use crate::{
    cmd::{
        admin::{
            run_admin_login, run_admin_user_create, run_admin_user_disable, run_admin_user_enable,
            run_admin_user_list, run_admin_user_sign,
        },
        deploy::run_deploy,
        login::run_login,
        machine::run_machine_list,
        service::run_service_list,
    },
    config::Config,
};

#[derive(Parser)]
#[command(name = "lttle")]
#[command(author = "lttle.cloud")]
#[command(about = "lttle.cloud CLI", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Deploy an app from a file
    Deploy(Deploy),

    /// Manage machines
    #[command(alias = "m")]
    Machine(MachineCommand),

    /// Manage services
    #[command(alias = "svc")]
    Service(ServiceCommand),

    /// Admin commands
    #[command(alias = "adm")]
    Admin(AdminCommand),

    /// Login as a user
    Login {
        /// User token
        token: String,
    },

    /// Show current user info
    Whoami,
}

#[derive(Args)]
pub struct Deploy {
    #[arg(short = 'f', long = "file")]
    pub file: PathBuf,
}

#[derive(Args)]
pub struct MachineCommand {
    #[command(subcommand)]
    pub command: MachineSubcommand,
}

#[derive(Subcommand)]
pub enum MachineSubcommand {
    /// List machines
    #[command(alias = "ls")]
    List,

    /// Get machine details
    Get {
        /// Machine ID
        id: String,
    },

    /// Start machine
    Start {
        /// Machine ID
        id: String,
    },

    /// Stop machine
    Stop {
        /// Machine ID
        id: String,
    },

    /// Delete machine
    #[command(alias = "del", alias = "rm")]
    Delete {
        /// Machine ID
        id: String,
    },
}

#[derive(Args)]
pub struct ServiceCommand {
    #[command(subcommand)]
    pub command: ServiceSubcommand,
}

#[derive(Subcommand)]
pub enum ServiceSubcommand {
    /// List services
    #[command(alias = "ls")]
    List,

    /// Get service details
    Get {
        /// Service ID
        id: String,
    },

    /// Delete service
    #[command(alias = "rm")]
    Delete {
        /// Service ID
        id: String,
    },
}

#[derive(Args)]
pub struct AdminCommand {
    #[command(subcommand)]
    pub command: AdminSubcommand,
}

#[derive(Subcommand)]
pub enum AdminSubcommand {
    /// Manage users
    User(UserCommand),

    /// Login as admin
    Login {
        /// Admin token
        token: String,
    },
}

#[derive(Args)]
pub struct UserCommand {
    #[command(subcommand)]
    pub command: UserSubcommand,
}

#[derive(Subcommand)]
pub enum UserSubcommand {
    /// Create a new user
    Create {
        /// Username
        username: String,
    },

    /// List users
    #[command(alias = "ls")]
    List,

    /// Disable a user
    Disable {
        /// Username
        username: String,
    },

    /// Enable a user
    Enable {
        /// Username
        username: String,
    },

    /// Sign a user token
    Sign {
        /// Username
        username: String,
    },
}

pub async fn run_cli(config: Config) -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Deploy(deploy) => {
            return run_deploy(config, deploy.file).await;
        }
        Command::Machine(mc) => match mc.command {
            MachineSubcommand::List => {
                return run_machine_list(config).await;
            }
            MachineSubcommand::Get { id } => println!("Getting machine: {}", id),
            MachineSubcommand::Start { id } => println!("Starting machine: {}", id),
            MachineSubcommand::Stop { id } => println!("Stopping machine: {}", id),
            MachineSubcommand::Delete { id } => println!("Deleting machine: {}", id),
        },
        Command::Service(sc) => match sc.command {
            ServiceSubcommand::List => {
                return run_service_list(config).await;
            }
            ServiceSubcommand::Get { id } => println!("Getting service: {}", id),
            ServiceSubcommand::Delete { id } => println!("Deleting service: {}", id),
        },
        Command::Admin(ac) => match ac.command {
            AdminSubcommand::User(uc) => match uc.command {
                UserSubcommand::Create { username } => {
                    return run_admin_user_create(config, username).await;
                }
                UserSubcommand::List => {
                    return run_admin_user_list(config).await;
                }
                UserSubcommand::Disable { username } => {
                    return run_admin_user_disable(config, username).await;
                }
                UserSubcommand::Enable { username } => {
                    return run_admin_user_enable(config, username).await;
                }
                UserSubcommand::Sign { username } => {
                    return run_admin_user_sign(config, username).await;
                }
            },
            AdminSubcommand::Login { token } => {
                return run_admin_login(config, token).await;
            }
        },
        Command::Login { token } => {
            return run_login(config, token).await;
        }
        Command::Whoami => println!("Showing current user info"),
    };

    Ok(())
}
