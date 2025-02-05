use ignition_client::{Client, ClientConfig, HelloRequest};
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
    };

    let client = Client::new(client_config).await?;

    let reply = client
        .greeter()
        .say_hello(HelloRequest {
            name: "Mark".to_owned(),
            age: Some(23),
        })
        .await?
        .into_inner();

    dbg!(reply);

    let reply = client
        .greeter()
        .say_hello(HelloRequest {
            name: "Harry".to_owned(),
            age: None,
        })
        .await?
        .into_inner();

    dbg!(reply);

    Ok(())
}

fn main() -> Result<()> {
    async_runtime::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(ignition())?;

    Ok(())
}
