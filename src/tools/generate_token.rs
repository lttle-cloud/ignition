use anyhow::Result;
use ignition::{api::auth::AuthHandler, utils::tracing::init_tracing};
use tracing::{error, info};

// TODO: get this from config

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = std::env::args().collect::<Vec<String>>();

    if args.len() != 3 {
        error!("Usage: generate-token-tool <jwt-secret> <tenant> <subject>");
        return Ok(());
    }

    let jwt_secret = args[1].clone();
    let tenant = args[2].clone();
    let subject = args[3].clone();
    let token = AuthHandler::new(jwt_secret).generate_token(&tenant, &subject)?;
    info!("token = {}", token);

    Ok(())
}
