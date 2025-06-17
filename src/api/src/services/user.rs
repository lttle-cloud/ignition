use tonic::{Request, Response, Status};
use util::result::Result;

use crate::ignition_proto::user::WhoAmIResponse;
use crate::ignition_proto::user_server;
use crate::ignition_proto::util::Empty;
use crate::services::auth::get_authenticated_user;

pub struct UserApiConfig {}

pub struct UserApi {
    config: UserApiConfig,
}

impl UserApi {
    pub fn new(config: UserApiConfig) -> Result<Self> {
        Ok(Self { config })
    }
}

#[tonic::async_trait]
impl user_server::User for UserApi {
    async fn who_am_i(&self, request: Request<Empty>) -> Result<Response<WhoAmIResponse>, Status> {
        let current_user = get_authenticated_user(&request)?;
        Ok(Response::new(WhoAmIResponse {
            user: Some(current_user.into()),
        }))
    }
}
