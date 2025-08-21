pub mod certificate;
pub mod deploy;
pub mod login;
pub mod machine;
pub mod namespace;
pub mod profile;
pub mod service;
pub mod volume;

use std::fs::File;

use anyhow::Result;
use atty::Stream;
use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use ignition::resources::metadata::Namespace;

use crate::{
    cmd::machine::{MachineLogsArgs, RestartNamespacedArgs},
    config::Config,
    ui::message::{message_info, message_warn},
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
    /// Connect to a ignitiond server
    Login(login::LoginArgs),

    /// Print the current user
    Whoami,

    /// Config profile management
    #[command(subcommand)]
    Profile(ProfileCommand),

    /// Namespace management (short: ns)
    #[command(subcommand, alias = "ns")]
    Namespace(NamespaceCommand),

    /// Deploy resources from a file
    Deploy(deploy::DeployArgs),

    /// Machine management
    #[command(subcommand)]
    Machine(MachineCommand),

    /// Volume management
    #[command(subcommand)]
    Volume(VolumeCommand),

    /// Service management (short: svc)
    #[command(subcommand, alias = "svc")]
    Service(ServiceCommand),

    /// Certificate management (short: cert)
    #[command(subcommand, alias = "cert")]
    Certificate(CertificateCommand),

    /// Install completions for your shell (run with root permissions)
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand)]
pub enum MachineCommand {
    /// List machines (short: ls)
    #[command(alias = "ls")]
    List(ListNamespacedArgs),

    /// Get a machine
    Get(GetNamespacedArgs),

    /// Get logs for a machine
    Logs(MachineLogsArgs),

    /// Execute a command in a machine
    Exec(machine::MachineExecArgs),

    /// Delete a machine (short: rm)
    #[command(alias = "rm")]
    Delete(DeleteNamespacedArgs),

    /// Restart a machine
    Restart(RestartNamespacedArgs),
}

#[derive(Subcommand)]
pub enum ServiceCommand {
    /// List services (short: ls)
    #[command(alias = "ls")]
    List(ListNamespacedArgs),

    /// Get a service
    Get(GetNamespacedArgs),

    /// Delete a service (short: rm)
    #[command(alias = "rm")]
    Delete(DeleteNamespacedArgs),
}

#[derive(Subcommand)]
pub enum VolumeCommand {
    /// List volumes (short: ls)
    #[command(alias = "ls")]
    List(ListNamespacedArgs),

    /// Get a volume
    Get(GetNamespacedArgs),

    /// Delete a volume (short: rm)
    #[command(alias = "rm")]
    Delete(DeleteNamespacedArgs),
}

#[derive(Subcommand)]
pub enum CertificateCommand {
    /// List certificates (short: ls)
    #[command(alias = "ls")]
    List(ListNamespacedArgs),

    /// Get a certificate
    Get(GetNamespacedArgs),

    /// Delete a certificate (short: rm)
    #[command(alias = "rm")]
    Delete(DeleteNamespacedArgs),
}

#[derive(Subcommand)]
pub enum ProfileCommand {
    /// Current profile
    Current,

    /// List profiles (short: ls)
    #[command(alias = "ls")]
    List,

    /// Set a profile
    Set(profile::ProfileSetArgs),

    /// Delete a profile (short: rm)
    #[command(alias = "rm")]
    Delete(profile::ProfileDeleteArgs),
}

#[derive(Subcommand)]
pub enum NamespaceCommand {
    /// List namespaces (short: ls)
    #[command(alias = "ls")]
    List,
}

pub async fn run_cli() -> Result<()> {
    let cli = Cli::parse();
    let mut cmd = Cli::command();

    if let Command::Completions { shell } = &cli.command {
        if let Some(file_path) = match &shell {
            Shell::Bash => Some("/etc/bash_completion.d/lttle"),
            Shell::Zsh => Some("~/.local/share/zsh/site-functions/lttle"),
            _ => None,
        } {
            let mut file = File::create(file_path)?;
            clap_complete::generate(shell.clone(), &mut cmd, "lttle", &mut file);
            message_info(format!(
                "Installed completions for {} to {}",
                shell.to_string(),
                file_path
            ));

            return Ok(());
        }

        if atty::is(Stream::Stdout) {
            message_warn(format!(
                "Automatic installation is not supported for {}. \
                 Please check the documentation for manual installation instructions. \
                 To get the completion script, pipe this command to your shell's specific completion file. \
                 For example in bash: `lttle completions bash > /etc/bash_completion.d/lttle`",
                shell.to_string()
            ));

            return Ok(());
        }

        clap_complete::generate(shell.clone(), &mut cmd, "lttle", &mut std::io::stdout());
        return Ok(());
    }

    let config = Config::load().await?;

    match cli.command {
        Command::Login(args) => login::run_login(&config, args).await,
        Command::Whoami => login::run_whoami(&config).await,
        Command::Profile(cmd) => match cmd {
            ProfileCommand::Current => profile::run_profile_current(&config).await,
            ProfileCommand::List => profile::run_profile_list(&config).await,
            ProfileCommand::Set(args) => profile::run_profile_set(&config, args).await,
            ProfileCommand::Delete(args) => profile::run_profile_delete(&config, args).await,
        },
        Command::Namespace(cmd) => match cmd {
            NamespaceCommand::List => namespace::run_namespace_list(&config).await,
        },
        Command::Deploy(args) => deploy::run_deploy(&config, args).await,
        Command::Machine(cmd) => match cmd {
            MachineCommand::List(args) => machine::run_machine_list(&config, args).await,
            MachineCommand::Get(args) => machine::run_machine_get(&config, args).await,
            MachineCommand::Logs(args) => machine::run_machine_get_logs(&config, args).await,
            MachineCommand::Exec(args) => machine::run_machine_exec(&config, args).await,
            MachineCommand::Delete(args) => machine::run_machine_delete(&config, args).await,
            MachineCommand::Restart(args) => machine::run_machine_restart(&config, args).await,
        },
        Command::Service(cmd) => match cmd {
            ServiceCommand::List(args) => service::run_service_list(&config, args).await,
            ServiceCommand::Get(args) => service::run_service_get(&config, args).await,
            ServiceCommand::Delete(args) => service::run_service_delete(&config, args).await,
        },
        Command::Volume(cmd) => match cmd {
            VolumeCommand::List(args) => volume::run_volume_list(&config, args).await,
            VolumeCommand::Get(args) => volume::run_volume_get(&config, args).await,
            VolumeCommand::Delete(args) => volume::run_volume_delete(&config, args).await,
        },
        Command::Certificate(cmd) => match cmd {
            CertificateCommand::List(args) => {
                certificate::run_certificate_list(&config, args).await
            }
            CertificateCommand::Get(args) => certificate::run_certificate_get(&config, args).await,
            CertificateCommand::Delete(args) => {
                certificate::run_certificate_delete(&config, args).await
            }
        },
        Command::Completions { .. } => unreachable!(),
    }
}

#[derive(Clone, Debug, Args)]
pub struct ListNamespacedArgs {
    /// List resources in a specific namespace (short: --ns)
    #[arg(long = "namespace", alias = "ns")]
    namespace: Option<String>,

    /// Skip namespace filtering and list resources from all namespaces
    #[arg(long = "all-namespaces", short = 'a')]
    all_namespaces: bool,
}

impl From<ListNamespacedArgs> for Namespace {
    fn from(args: ListNamespacedArgs) -> Self {
        if args.all_namespaces {
            Namespace::Unspecified
        } else {
            Namespace::from_value_or_default(args.namespace)
        }
    }
}

#[derive(Clone, Debug, Args)]
pub struct GetNamespacedArgs {
    /// List resources in a specific namespace (short: --ns)
    #[arg(long = "namespace", alias = "ns")]
    namespace: Option<String>,

    /// Name of the resource to fetch
    name: String,
}

impl From<GetNamespacedArgs> for Namespace {
    fn from(args: GetNamespacedArgs) -> Self {
        Namespace::from_value_or_default(args.namespace)
    }
}

#[derive(Clone, Debug, Args)]
pub struct DeleteNamespacedArgs {
    /// List resources in a specific namespace (short: --ns)
    #[arg(long = "namespace", alias = "ns")]
    namespace: Option<String>,

    /// Name of the resource to delete
    name: String,

    /// Confirm deletion of object
    #[arg(long = "yes", short = 'y')]
    confirm: bool,
}

impl From<DeleteNamespacedArgs> for Namespace {
    fn from(args: DeleteNamespacedArgs) -> Self {
        Namespace::from_value_or_default(args.namespace)
    }
}
