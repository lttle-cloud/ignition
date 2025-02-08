use sds::{Collection, Store};
use tonic::{Request, Response, Status};
use util::result::Result;

use crate::data::user::{User, UserStatus};
use crate::ignition_proto::admin::{
    self, CreateUserRequest, CreateUserResponse, CreateUserTokenRequest, CreateUserTokenResponse,
    DisableUserRequest, DisableUserResponse, EnableUserRequest, EnableUserResponse,
    ListUsersResponse,
};
use crate::ignition_proto::admin_server::Admin;
use crate::ignition_proto::util::Empty;

pub struct AdminService {
    store: Store,
    user_collection: Collection<User>,
}

impl AdminService {
    pub fn new(store: Store) -> Result<Self> {
        let user_collection = store.collection("users")?;

        Ok(Self {
            store,
            user_collection,
        })
    }
}

#[tonic::async_trait]
impl Admin for AdminService {
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
            name: request.name,
            status: UserStatus::Active,
        };

        // check if user with this name already exists
        if let Some(_existing_user) = txn
            .prefix_iter(&self.user_collection, &user.name)
            .map_err(|_| Status::internal("failed to iterate users"))?
            .next()
        {
            return Err(Status::already_exists("User already exists"));
        };

        let user_key = format!("{}:{}", &user.name, &user.id);
        txn.put(&self.user_collection, &user_key, &user)
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

    async fn disable_user(
        &self,
        request: Request<DisableUserRequest>,
    ) -> Result<Response<DisableUserResponse>, Status> {
        todo!("Implement disable_user")
    }

    async fn enable_user(
        &self,
        request: Request<EnableUserRequest>,
    ) -> Result<Response<EnableUserResponse>, Status> {
        todo!("Implement enable_user")
    }

    async fn create_user_token(
        &self,
        request: Request<CreateUserTokenRequest>,
    ) -> Result<Response<CreateUserTokenResponse>, Status> {
        todo!("Implement create_user_token")
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
