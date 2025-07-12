use anyhow::Result;
use ignition::api::auth::AuthHandler;

// TODO: get this from config

#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<String>>();

    if args.len() != 3 {
        eprintln!("Usage: generate-token-tool <jwt-secret> <tenant>");
        return Ok(());
    }

    let jwt_secret = args[1].clone();
    let tenant = args[2].clone();
    let token = AuthHandler::new(jwt_secret).generate_token(&tenant)?;
    println!("{}", token);

    Ok(())
}
