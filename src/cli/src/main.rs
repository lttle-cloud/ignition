use ignition_client::{
    ignition_proto::{admin::CreateUserRequest, util::Empty},
    ClientConfig, PrivilegedClient,
};
use tracing::info;
use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime,
    result::{Context, Result},
};

async fn ignition() -> Result<()> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(tracing::Level::INFO)
        .finish();
    tracing::subscriber::set_global_default(subscriber)
        .context("Failed to set global default subscriber")?;

    let client_config = ClientConfig {
        addr: "tcp://127.0.0.1:5100".into(),
        // TODO(@laurci): get this from env
        token: "temp_admin_token".into(),
    };

    let client = PrivilegedClient::new(client_config).await?;

    let create_user = false;
    let list_users = true;

    if create_user {
        info!("Creating user");

        let create_user = CreateUserRequest {
            name: "laurci_test".into(),
        };

        let user = client.admin().create_user(create_user).await?.into_inner();

        dbg!(user);
    }

    if list_users {
        info!("Listing users");

        let users_list = client.admin().list_users(Empty {}).await?.into_inner();

        dbg!(users_list);
    }

    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
