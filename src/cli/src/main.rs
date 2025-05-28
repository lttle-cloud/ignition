use ignition_client::{
    ignition_proto::{
        admin::{CreateUserRequest, CreateUserTokenRequest, SetStatusRequest},
        image::{CreatePullSecretRequest, DeletePullSecretRequest},
        util::Empty,
    },
    Client, ClientConfig, PrivilegedClient,
};
use tracing_subscriber::FmtSubscriber;
use util::{
    async_runtime,
    result::{Context, Result},
    tracing::{self, info},
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

    let admin_client = PrivilegedClient::new(client_config).await?;

    // Test creating a user
    info!("Creating test user");
    let user = admin_client
        .admin()
        .create_user(CreateUserRequest {
            name: "test_user".into(),
        })
        .await?
        .into_inner()
        .user
        .expect("user should be returned");

    info!("Created user: {:?}", user);

    // Test creating a token for the user
    info!("Creating token for user");
    let token = admin_client
        .admin()
        .create_user_token(CreateUserTokenRequest {
            id: user.id.clone(),
            duration_seconds: Some(7200), // 2 hours
        })
        .await?
        .into_inner()
        .token;

    info!("Created token: {}", token);

    // Test disabling the user
    info!("Disabling user");
    let disabled_user = admin_client
        .admin()
        .set_status(SetStatusRequest {
            id: user.id.clone(),
            status: ignition_client::ignition_proto::admin::user::Status::Inactive as i32,
        })
        .await?
        .into_inner()
        .user
        .expect("user should be returned");

    info!("Disabled user: {:?}", disabled_user);

    // Try to create a token for disabled user (should fail)
    info!("Attempting to create token for disabled user (should fail)");
    let token_result = admin_client
        .admin()
        .create_user_token(CreateUserTokenRequest {
            id: user.id.clone(),
            duration_seconds: None,
        })
        .await;

    info!("Create token result: {:?}", token_result);

    // Re-enable the user
    info!("Re-enabling user");
    let enabled_user = admin_client
        .admin()
        .set_status(SetStatusRequest {
            id: user.id.clone(),
            status: ignition_client::ignition_proto::admin::user::Status::Active as i32,
        })
        .await?
        .into_inner()
        .user
        .expect("user should be returned");

    info!("Re-enabled user: {:?}", enabled_user);

    // List all users to verify the changes
    info!("Listing all users");
    let users = admin_client
        .admin()
        .list_users(Empty {})
        .await?
        .into_inner()
        .users;

    info!("All users: {:?}", users);

    info!("Create token for user");
    let token_result = admin_client
        .admin()
        .create_user_token(CreateUserTokenRequest {
            id: user.id.clone(),
            duration_seconds: None,
        })
        .await;
    info!("Create token result: {:?}", token_result);

    let client_config = ClientConfig {
        addr: "tcp://127.0.0.1:5100".into(),
        // TODO(@laurci): get this from env
        token: token_result
            .expect("token should be returned")
            .into_inner()
            .token,
    };

    let client = Client::new(client_config).await?;

    let secret = client
        .image()
        .create_pull_secret(CreatePullSecretRequest {
            name: "test_secret".into(),
            username: "test_user".into(),
            password: "test_password".into(),
        })
        .await?;

    info!("Created secret: {:?}", secret);

    let secrets = client.image().list_pull_secrets(Empty {}).await?;
    info!("Secrets: {:?}", secrets);

    client
        .image()
        .delete_pull_secret(DeletePullSecretRequest {
            name: "test_secret".into(),
        })
        .await?;

    let secrets = client.image().list_pull_secrets(Empty {}).await?;
    info!("Secrets: {:?}", secrets);

    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
