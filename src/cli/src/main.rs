pub(crate) mod client;
mod cmd;
mod config;
mod resource;

use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime,
    result::{bail, Context, Result},
    tracing::{self, error},
};

use crate::{cmd::run_cli, config::Config};

async fn ignition() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let Ok(config) = Config::load().await else {
        bail!("Failed to load config");
    };

    if let Err(e) = run_cli(config).await {
        error!("Error: {}", e);
        std::process::exit(1);
    };

    // let client = Client::new(client_config).await?;

    // info!("Pulling image");
    // let image = client
    //     .image()
    //     .pull(PullImageRequest {
    //         image: "nginx:latest".into(),
    //     })
    //     .await?;
    // info!("Image: {:?}", image);

    // let machines = client.machine().list(Empty {}).await?;
    // info!("Machines: {:?}", machines);

    // client
    //     .machine()
    //     .deploy(DeployMachineRequest {
    //         machine: Some(Machine {
    //             name: "test".into(),
    //             image: "nginx:latest".into(),
    //             memory: 128,
    //             vcpus: 1,
    //             environment: vec![],
    //             snapshot_policy: None,
    //         }),
    //     })
    //     .await?;

    // let machines = client.machine().list(Empty {}).await?;
    // info!("Machines: {:?}", machines);

    // let secret = client
    //     .image()
    //     .create_pull_secret(CreatePullSecretRequest {
    //         name: "test_secret".into(),
    //         username: "test_user".into(),
    //         password: "test_password".into(),
    //     })
    //     .await?;

    // info!("Created secret: {:?}", secret);

    // let secrets = client.image().list_pull_secrets(Empty {}).await?;
    // info!("Secrets: {:?}", secrets);

    // client
    //     .image()
    //     .delete_pull_secret(DeletePullSecretRequest {
    //         name: "test_secret".into(),
    //     })
    //     .await?;

    // let secrets = client.image().list_pull_secrets(Empty {}).await?;
    // info!("Secrets: {:?}", secrets);

    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
