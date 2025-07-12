pub mod login;
pub mod machine;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use ignition::resources::metadata::Namespace;

use crate::config::Config;

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
    /// Login to a ignition API server
    Login(login::LoginArgs),

    /// Machine management
    #[command(subcommand)]
    Machine(MachineCommand),
}

#[derive(Subcommand)]
pub enum MachineCommand {
    /// List machines (short: ls)
    #[command(alias = "ls")]
    List(ListNamespacedArgs),

    /// Get a machine
    Get(GetNamespacedArgs),
}

pub async fn run_cli(config: &Config) -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Login(args) => login::run_login(config, args).await,
        Command::Machine(cmd) => match cmd {
            MachineCommand::List(args) => machine::run_list_machines(config, args).await,
            MachineCommand::Get(args) => machine::run_get_machine(config, args).await,
        },
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
