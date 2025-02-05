pub use hello_world::{greeter_client::GreeterClient, HelloReply, HelloRequest};
use tonic::transport::{Channel, Endpoint};
use util::result::Result;

mod hello_world {
    tonic::include_proto!("helloworld");
}

pub struct ClientConfig {
    pub addr: String,
}

pub struct Client {
    transport: Channel,
}

impl Client {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let transport = Endpoint::new(config.addr.clone())?.connect().await?;

        Ok(Self { transport })
    }

    pub fn greeter(&self) -> GreeterClient<Channel> {
        let greeter_client = GreeterClient::new(self.transport.clone());
        greeter_client
    }
}
