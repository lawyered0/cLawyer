//! Bearer token authentication middleware for the web gateway.

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

use crate::db::UserRole;

/// Authenticated request principal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPrincipal {
    pub user_id: String,
    pub role: UserRole,
}

impl AuthPrincipal {
    pub fn new(user_id: impl Into<String>, role: UserRole) -> Self {
        Self {
            user_id: user_id.into(),
            role,
        }
    }
}

/// Shared auth state injected via axum middleware state.
#[derive(Clone)]
pub struct AuthState {
    pub token: String,
    pub principal: AuthPrincipal,
}

/// Auth middleware that validates bearer token from header or query param.
///
/// SSE connections can't set headers from `EventSource`, so we also accept
/// `?token=xxx` as a query parameter.
pub async fn auth_middleware(
    State(auth): State<AuthState>,
    headers: HeaderMap,
    mut request: Request,
    next: Next,
) -> Response {
    // Try Authorization header first (constant-time comparison)
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
        && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
    {
        request.extensions_mut().insert(auth.principal.clone());
        return next.run(request).await;
    }

    // Fall back to query parameter for SSE EventSource (constant-time comparison)
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(token) = pair.strip_prefix("token=")
                && bool::from(token.as_bytes().ct_eq(auth.token.as_bytes()))
            {
                request.extensions_mut().insert(auth.principal.clone());
                return next.run(request).await;
            }
        }
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

/// Extractor for authenticated principal placed by [`auth_middleware`].
#[derive(Clone, Debug)]
pub struct RequestPrincipal(pub AuthPrincipal);

impl<S> FromRequestParts<S> for RequestPrincipal
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let principal = parts
            .extensions
            .get::<AuthPrincipal>()
            .cloned()
            .ok_or((StatusCode::UNAUTHORIZED, "Missing auth principal"))?;
        Ok(Self(principal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_state_clone() {
        let state = AuthState {
            token: "test-token".to_string(),
            principal: AuthPrincipal::new("default", UserRole::Admin),
        };
        let cloned = state.clone();
        assert_eq!(cloned.token, "test-token");
        assert_eq!(cloned.principal.user_id, "default");
        assert_eq!(cloned.principal.role, UserRole::Admin);
    }
}
