//! Bearer token authentication middleware for the web gateway.

use std::sync::Arc;

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::db::{Database, UserRole};

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
    pub fallback_principal: AuthPrincipal,
    pub store: Option<Arc<dyn Database>>,
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
    if let Some(token) = extract_header_token(&headers)
        && let Some(principal) = resolve_principal_for_token(&auth, &token).await
    {
        request.extensions_mut().insert(principal);
        return next.run(request).await;
    }

    // Fall back to query token for EventSource and compatibility with mixed proxy headers.
    if let Some(token) = extract_query_token(request.uri().query())
        && let Some(principal) = resolve_principal_for_token(&auth, &token).await
    {
        request.extensions_mut().insert(principal);
        return next.run(request).await;
    }

    (StatusCode::UNAUTHORIZED, "Invalid or missing auth token").into_response()
}

async fn resolve_principal_for_token(auth: &AuthState, token: &str) -> Option<AuthPrincipal> {
    // Resolve principal from persisted token hash first.
    if let Some(store) = auth.store.as_ref() {
        let token_hash = hash_auth_token(token);
        match store.get_user_by_token_hash(&token_hash).await {
            Ok(Some(user)) => return Some(AuthPrincipal::new(user.id, user.role)),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!("Failed to resolve auth principal from token hash: {}", err);
            }
        }
    }

    // Backward-compatible fallback for shared gateway token mode.
    if bool::from(token.as_bytes().ct_eq(auth.token.as_bytes())) {
        return Some(auth.fallback_principal.clone());
    }

    None
}

fn extract_header_token(headers: &HeaderMap) -> Option<String> {
    if let Some(auth_header) = headers.get("authorization")
        && let Ok(value) = auth_header.to_str()
        && let Some(token) = value.strip_prefix("Bearer ")
    {
        return Some(token.to_string());
    }
    None
}

fn extract_query_token(query: Option<&str>) -> Option<String> {
    // SSE EventSource path: token is passed via query parameter.
    let query = query?;
    for pair in query.split('&') {
        if let Some(token) = pair.strip_prefix("token=") {
            return Some(token.to_string());
        }
    }
    None
}

pub fn hash_auth_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
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
    use axum::Router;
    use axum::http::{Request, StatusCode};
    use axum::middleware;
    use axum::response::IntoResponse;
    use axum::routing::get;
    use tower::ServiceExt;

    #[test]
    fn test_auth_state_clone() {
        let state = AuthState {
            token: "test-token".to_string(),
            fallback_principal: AuthPrincipal::new("default", UserRole::Admin),
            store: None,
        };
        let cloned = state.clone();
        assert_eq!(cloned.token, "test-token");
        assert_eq!(cloned.fallback_principal.user_id, "default");
        assert_eq!(cloned.fallback_principal.role, UserRole::Admin);
        assert!(cloned.store.is_none());
    }

    #[test]
    fn test_hash_auth_token_deterministic() {
        let first = hash_auth_token("abc123");
        let second = hash_auth_token("abc123");
        let third = hash_auth_token("abc124");
        assert_eq!(first, second);
        assert_ne!(first, third);
    }

    #[tokio::test]
    async fn test_auth_middleware_falls_back_to_query_token_when_header_invalid() {
        let auth_state = AuthState {
            token: "good-token".to_string(),
            fallback_principal: AuthPrincipal::new("default", UserRole::Admin),
            store: None,
        };

        async fn ok_handler() -> impl IntoResponse {
            StatusCode::OK
        }

        let app = Router::new()
            .route("/", get(ok_handler))
            .route_layer(middleware::from_fn_with_state(auth_state, auth_middleware));

        let req = Request::builder()
            .uri("/?token=good-token")
            .header("authorization", "Bearer bad-token")
            .body(axum::body::Body::empty())
            .expect("valid request");

        let response = app.oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }
}
