use jsonwebtoken::{encode, EncodingKey, Header};
use sds::{Collection, Store};
use std::time::{SystemTime, UNIX_EPOCH};
use tonic::{Request, Response, Status};
use util::result::Result;

use crate::data::token::UserTokenClaims;
use crate::data::user::{User, UserStatus};
use crate::ignition_proto::admin::{
    self, CreateUserRequest, CreateUserResponse, CreateUserTokenRequest, CreateUserTokenResponse,
    ListUsersResponse, SetStatusRequest, SetStatusResponse,
};
use crate::ignition_proto::admin_server::Admin;
use crate::ignition_proto::util::Empty;

pub struct AdminApiConfig {
    pub jwt_secret: String,
    pub default_token_duration: u32,
}

pub struct AdminApi {
    store: Store,
    user_collection: Collection<User>,
    config: AdminApiConfig,
}

impl AdminApi {
    pub fn new(store: Store, config: AdminApiConfig) -> Result<Self> {
        let user_collection = store.collection("users")?;

        Ok(Self {
            store,
            user_collection,
            config,
        })
    }
}

#[tonic::async_trait]
impl Admin for AdminApi {
    async fn list_users(
        &self,
        _request: Request<Empty>,
    ) -> Result<Response<ListUsersResponse>, Status> {
        let txn = self
            .store
            .read_txn()
            .map_err(|_| Status::internal("failed to create read txn"))?;

        let users = txn
            .iter(&self.user_collection)
            .map_err(|_| Status::internal("failed to iterate users"))?
            .collect::<Result<Vec<_>, sds::Error>>()
            .map_err(|_| Status::internal("failed to collect users"))?
            .iter()
            .map(|(_, user)| {
                let admin_user: admin::User = user.into();
                admin_user
            })
            .collect::<Vec<_>>();

        Ok(Response::new(ListUsersResponse { users }))
    }

    async fn create_user(
        &self,
        request: Request<CreateUserRequest>,
    ) -> Result<Response<CreateUserResponse>, Status> {
        let request = request.into_inner();

        if !request
            .name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_')
        {
            return Err(Status::invalid_argument("Invalid user name"));
        }

        if request.name.starts_with('_') || request.name.ends_with('_') {
            return Err(Status::invalid_argument("Invalid user name"));
        }

        let mut txn = self
            .store
            .write_txn()
            .map_err(|_| Status::internal("failed to create read txn"))?;

        let user = User {
            id: cuid::cuid2(),
            name: request.name.clone(),
            status: UserStatus::Active,
        };

        // check if user with this name already exists
        if let Ok(Some((_, user))) =
            txn.find_first(&self.user_collection, |_, user| user.name == request.name)
        {
            return Err(Status::already_exists(format!(
                "User {} already exists",
                user.name
            )));
        }

        txn.put(&self.user_collection, &user.id, &user)
            .map_err(|_| Status::internal("failed to put user"))?;

        txn.commit()
            .map_err(|_| Status::internal("failed to commit txn"))?;

        Ok(Response::new(CreateUserResponse {
            user: Some(admin::User {
                id: user.id,
                status: admin::user::Status::Active as i32,
                name: user.name,
            }),
        }))
    }

    async fn set_status(
        &self,
        request: Request<SetStatusRequest>,
    ) -> Result<Response<SetStatusResponse>, Status> {
        let request = request.into_inner();
        let new_status = match request.status() {
            admin::user::Status::Active => UserStatus::Active,
            admin::user::Status::Inactive => UserStatus::Inactive,
        };

        let mut txn = self
            .store
            .write_txn()
            .map_err(|_| Status::internal("failed to create write txn"))?;

        let mut user = txn
            .get(&self.user_collection, &request.id)
            .ok_or_else(|| Status::not_found("User not found"))?;

        if user.status == new_status {
            return Err(Status::failed_precondition(match new_status {
                UserStatus::Active => "User is already enabled",
                UserStatus::Inactive => "User is already disabled",
            }));
        }

        user.status = new_status;

        txn.put(&self.user_collection, &request.id, &user)
            .map_err(|_| Status::internal("failed to update user"))?;

        txn.commit()
            .map_err(|_| Status::internal("failed to commit txn"))?;

        Ok(Response::new(SetStatusResponse {
            user: Some((&user).into()),
        }))
    }

    async fn create_user_token(
        &self,
        request: Request<CreateUserTokenRequest>,
    ) -> Result<Response<CreateUserTokenResponse>, Status> {
        let request = request.into_inner();

        let txn = self
            .store
            .read_txn()
            .map_err(|_| Status::internal("failed to create read txn"))?;

        match txn
            .find_first(&self.user_collection, |_, user| user.id == request.id)
            .map_err(|_| Status::internal("failed to find user"))?
        {
            Some((_, user)) if user.status == UserStatus::Active => (),
            Some(_) => return Err(Status::failed_precondition("User is not active")),
            None => return Err(Status::not_found("User not found")),
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Status::internal("failed to get timestamp"))?
            .as_secs();

        let duration = request
            .duration_seconds
            .unwrap_or(self.config.default_token_duration);
        let claims = UserTokenClaims {
            sub: request.id,
            iat: now,
            exp: now + duration as u64,
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_base64_secret(&self.config.jwt_secret)
                .map_err(|_| Status::internal("invalid jwt secret"))?,
        )
        .map_err(|_| Status::internal("failed to create token"))?;

        Ok(Response::new(CreateUserTokenResponse { token }))
    }
}

pub(crate) fn admin_auth_interceptor(
    req: Request<()>,
    admin_token: String,
) -> Result<Request<()>, Status> {
    let token = match req.metadata().get("authorization") {
        Some(t) => t
            .to_str()
            .map_err(|_| Status::unauthenticated("Invalid authorization header"))?,
        None => return Err(Status::unauthenticated("Missing authorization header")),
    };

    if token != admin_token {
        return Err(Status::unauthenticated("Invalid admin token"));
    }

    Ok(req)
}
