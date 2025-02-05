use std::net::SocketAddr;

use hello_world::{
    greeter_server::{Greeter, GreeterServer},
    HelloReply, HelloRequest,
};
use tonic::{transport::Server, Request, Response, Status};
use tracing::info;
use util::result::Result;

pub mod hello_world {
    tonic::include_proto!("helloworld");
}

#[derive(Debug, Default)]
pub struct MyGreeter {}

#[tonic::async_trait]
impl Greeter for MyGreeter {
    async fn say_hello(
        &self,
        request: Request<HelloRequest>,
    ) -> Result<Response<HelloReply>, Status> {
        let request = request.into_inner();
        println!("Got a request: {:?}", request);

        let mut message = format!("Hello {}!", request.name);
        if let Some(age) = request.age {
            message.push_str(&format!(" You are {age} years old!"));
        };

        let reply = HelloReply { message };

        Ok(Response::new(reply))
    }
}

pub struct ApiServerConfig {
    pub addr: SocketAddr,
}

pub async fn start_api_server(config: ApiServerConfig) -> Result<()> {
    let greeter = MyGreeter::default();

    info!("api server listening on {:?}", config.addr);

    Server::builder()
        .add_service(GreeterServer::new(greeter))
        .serve(config.addr)
        .await?;

    Ok(())
}
