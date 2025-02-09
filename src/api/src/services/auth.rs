use jsonwebtoken::{decode, DecodingKey, Validation};
use sds::{Collection, Store};
use tonic::{Request, Status};
use util::result::Result;

use crate::data::token::UserTokenClaims;
use crate::data::user::{User, UserStatus};

#[derive(Clone)]
pub struct AuthInterceptorConfig {
    pub jwt_secret: String,
}

#[derive(Clone)]
pub struct AuthInterceptor {
    store: Store,
    user_collection: Collection<User>,
    config: AuthInterceptorConfig,
}

impl AuthInterceptor {
    pub fn new(store: Store, config: AuthInterceptorConfig) -> Result<Self> {
        let user_collection = store.collection("users")?;

        Ok(Self {
            store,
            user_collection,
            config,
        })
    }

    pub fn validate_request(&self, req: Request<()>) -> Result<Request<()>, Status> {
        // Extract token from authorization header
        let token = match req.metadata().get("authorization") {
            Some(t) => t
                .to_str()
                .map_err(|_| Status::unauthenticated("Invalid authorization header"))?,
            None => return Err(Status::unauthenticated("Missing authorization header")),
        };

        // Validate JWT token
        let token_data = decode::<UserTokenClaims>(
            token,
            &DecodingKey::from_base64_secret(&self.config.jwt_secret)
                .map_err(|_| Status::internal("invalid jwt secret"))?,
            &Validation::default(),
        )
        .map_err(|_| Status::unauthenticated("Invalid token"))?;

        let claims = token_data.claims;

        // Check if user exists and is active
        let txn = self
            .store
            .read_txn()
            .map_err(|_| Status::internal("failed to create read txn"))?;

        match txn
            .get(&self.user_collection, &claims.sub)
            .ok_or_else(|| Status::unauthenticated("User not found"))?
        {
            user if user.status == UserStatus::Active => (),
            _ => return Err(Status::permission_denied("User is not active")),
        }

        Ok(req)
    }
}

pub fn user_auth_interceptor(
    interceptor: AuthInterceptor,
) -> impl Fn(Request<()>) -> Result<Request<()>, Status> {
    move |req| {
        let interceptor = interceptor.clone();
        interceptor.validate_request(req)
    }
}
