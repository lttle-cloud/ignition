use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};

use crate::api::ApiState;

#[derive(Debug, Clone)]
pub struct ServiceRequestContext {
    pub tenant: String,
    pub namespace: Option<String>,
}

pub enum ServiceRequestContextError {
    InvalidToken,
}

impl IntoResponse for ServiceRequestContextError {
    fn into_response(self) -> Response {
        match self {
            ServiceRequestContextError::InvalidToken => {
                (StatusCode::UNAUTHORIZED, "Invalid token").into_response()
            }
        }
    }
}

impl FromRequestParts<Arc<ApiState>> for ServiceRequestContext {
    type Rejection = ServiceRequestContextError;

    async fn from_request_parts(
        _parts: &mut Parts,
        _state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        Ok(ServiceRequestContext {
            tenant: "test_tenant".to_string(),
            namespace: None,
        })
    }
}
