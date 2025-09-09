use anyhow::Result;
use base64::{Engine, prelude::BASE64_URL_SAFE_NO_PAD};
use ignition::utils::tracing::init_tracing;
use tracing::{error, info};

// TODO: get this from config

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let args = std::env::args().collect::<Vec<String>>();

    if args.len() != 2 {
        error!("Usage: generate-hmac-secret-tool <context>");
        return Ok(());
    }

    let random: [u8; 32] = rand::random();
    let context = args[1].clone();

    let hmac_key = blake3::derive_key(&context, &random);
    let hmac_secret = BASE64_URL_SAFE_NO_PAD.encode(hmac_key);

    info!("hmac_secret = {}", hmac_secret);

    Ok(())
}
