use ignition_proto::{admin_client::AdminClient, image_client::ImageClient};
use tonic::{
    service::interceptor::InterceptedService,
    transport::{Channel, Endpoint},
    Request, Status,
};
use util::result::Result;

pub mod ignition_proto {
    tonic::include_proto!("ignition");
    pub mod util {
        tonic::include_proto!("ignition.util");
    }
    pub mod admin {
        tonic::include_proto!("ignition.admin");
    }
    pub mod image {
        tonic::include_proto!("ignition.image");
    }
    pub mod deployment {
        tonic::include_proto!("ignition.deployment");
    }
}

pub struct ClientConfig {
    pub addr: String,
    pub token: String,
}

pub struct Client {
    transport: Channel,
    token: String,
}

impl Client {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let endpoint = Endpoint::new(config.addr.clone())?;
        let transport = endpoint.connect().await?;

        Ok(Self {
            transport,
            token: config.token,
        })
    }

    pub fn image(
        &self,
    ) -> ImageClient<InterceptedService<Channel, impl Fn(Request<()>) -> Result<Request<()>, Status>>>
    {
        let token = self.token.clone();
        let interceptor = move |mut req: Request<()>| {
            req.metadata_mut()
                .insert("authorization", token.parse().unwrap());
            Ok(req)
        };

        ImageClient::with_interceptor(self.transport.clone(), interceptor)
    }
}

pub struct PrivilegedClient {
    transport: Channel,
    token: String,
}

impl PrivilegedClient {
    pub async fn new(config: ClientConfig) -> Result<Self> {
        let endpoint = Endpoint::new(config.addr.clone())?;
        let transport = endpoint.connect().await?;

        Ok(Self {
            transport,
            token: config.token,
        })
    }

    pub fn admin(
        &self,
    ) -> AdminClient<InterceptedService<Channel, impl Fn(Request<()>) -> Result<Request<()>, Status>>>
    {
        let token = self.token.clone();
        let interceptor = move |mut req: Request<()>| {
            req.metadata_mut()
                .insert("authorization", token.parse().unwrap());
            Ok(req)
        };

        AdminClient::with_interceptor(self.transport.clone(), interceptor)
    }
}

impl Client {}
