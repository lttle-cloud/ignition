use std::sync::Arc;

use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};

use crate::{api::ApiState, resources::metadata::Namespace};

#[derive(Debug, Clone)]
pub struct ServiceRequestContext {
    pub tenant: String,
    pub namespace: Namespace,
}

pub enum ServiceRequestContextError {
    InvalidToken,
    InvalidNamespace,
}

impl IntoResponse for ServiceRequestContextError {
    fn into_response(self) -> Response {
        match self {
            ServiceRequestContextError::InvalidToken => {
                (StatusCode::UNAUTHORIZED, "Invalid token").into_response()
            }
            ServiceRequestContextError::InvalidNamespace => {
                (StatusCode::BAD_REQUEST, "Invalid namespace").into_response()
            }
        }
    }
}

impl FromRequestParts<Arc<ApiState>> for ServiceRequestContext {
    type Rejection = ServiceRequestContextError;

    async fn from_request_parts(
        parts: &mut Parts,
        _state: &Arc<ApiState>,
    ) -> Result<Self, Self::Rejection> {
        let namespace_header = parts.headers.get("x-ignition-namespace");
        let namespace = if let Some(namespace_header) = namespace_header {
            Namespace::from_value(
                namespace_header
                    .to_str()
                    .map_err(|_| ServiceRequestContextError::InvalidNamespace)?
                    .to_string()
                    .into(),
            )
        } else {
            Namespace::Unspecified
        };

        Ok(ServiceRequestContext {
            tenant: "test_tenant".to_string(),
            namespace,
        })
    }
}
