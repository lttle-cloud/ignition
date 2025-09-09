use anyhow::Result;
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use ignition::{api::auth::AuthHandler, utils::tracing::init_tracing};
use tracing::{error, info};

// TODO: get this from config

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = std::env::args().collect::<Vec<String>>();

    if args.len() != 4 {
        error!("Usage: generate-token-tool <jwt-secret> <tenant> <subject>");
        return Ok(());
    }

    let hmac_secret = BASE64_URL_SAFE_NO_PAD.encode(vec![0; 32]);

    let jwt_secret = args[1].clone();
    let tenant = args[2].clone();
    let subject = args[3].clone();

    let token = AuthHandler::new(
        jwt_secret,
        hmac_secret,
        "",
        Option::<&std::path::Path>::None,
        Option::<&std::path::Path>::None,
    )?
    .generate_token(&tenant, &subject)?;
    info!("token = {}", token);

    Ok(())
}
