use comfy_table::Table;
use ignition_client::ignition_proto::{
    admin::{user, CreateUserRequest, CreateUserTokenRequest, SetStatusRequest},
    util::Empty,
};
use util::{
    result::{bail, Result},
    tracing::info,
};

use crate::{client::get_admin_client, config::Config};

pub async fn run_admin_login(config: Config, admin_token: String) -> Result<()> {
    let mut updated_config = config.clone();
    updated_config.admin_token = Some(admin_token);
    updated_config.save().await?;

    info!("Admin token saved to config");

    Ok(())
}

pub async fn run_admin_user_create(config: Config, username: String) -> Result<()> {
    let client = get_admin_client(&config).await?;
    let Ok(response) = client
        .admin()
        .create_user(CreateUserRequest { name: username })
        .await
    else {
        bail!("Failed to create user");
    };

    let Some(user) = response.into_inner().user else {
        bail!("Failed to create user");
    };

    info!("user created: {} ({})", user.name, user.id);

    Ok(())
}

pub async fn run_admin_user_list(config: Config) -> Result<()> {
    let client = get_admin_client(&config).await?;
    let Ok(response) = client.admin().list_users(Empty {}).await else {
        bail!("Failed to list users");
    };

    let users = response.into_inner().users;

    let mut table = Table::new();
    table.set_header(vec!["ID", "Name", "Status"]);

    for user in users {
        let status = user::Status::try_from(user.status)?;
        table.add_row(vec![user.id, user.name, status.as_str_name().to_string()]);
    }

    println!("{table}");

    Ok(())
}

pub async fn run_admin_user_disable(config: Config, username: String) -> Result<()> {
    let client = get_admin_client(&config).await?;
    let Ok(response) = client
        .admin()
        .set_status(SetStatusRequest {
            id: username,
            status: user::Status::Inactive as i32,
        })
        .await
    else {
        bail!("Failed to disable user");
    };

    let Some(user) = response.into_inner().user else {
        bail!("Failed to disable user");
    };

    info!("user disabled: {} ({})", user.name, user.id);

    Ok(())
}

pub async fn run_admin_user_enable(config: Config, username: String) -> Result<()> {
    let client = get_admin_client(&config).await?;
    let Ok(response) = client
        .admin()
        .set_status(SetStatusRequest {
            id: username,
            status: user::Status::Active as i32,
        })
        .await
    else {
        bail!("Failed to enable user");
    };

    let Some(user) = response.into_inner().user else {
        bail!("Failed to enable user");
    };

    info!("user enabled: {} ({})", user.name, user.id);

    Ok(())
}

pub async fn run_admin_user_sign(config: Config, username: String) -> Result<()> {
    let client = get_admin_client(&config).await?;
    let Ok(response) = client
        .admin()
        .create_user_token(CreateUserTokenRequest {
            id: username,
            duration_seconds: Some(5 * 24 * 60 * 60), // 5 days
        })
        .await
    else {
        bail!("Failed to sign user token");
    };

    let token = response.into_inner().token;
    info!("user token: {}", token);

    Ok(())
}
