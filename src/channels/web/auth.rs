//! Bearer token authentication middleware for the web gateway.

use std::sync::Arc;

use axum::{
    extract::{FromRequestParts, Request, State},
    http::{HeaderMap, StatusCode, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::db::{Database, UserRole};

type HmacSha256 = Hmac<Sha256>;

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
    /// HMAC-SHA256 key used to hash tokens before DB storage/lookup.
    ///
    /// When `None`, falls back to plain SHA-256 (backward-compatible with
    /// existing DB rows). Always set by `start_server` via
    /// `derive_token_hmac_key`.
    pub hmac_key: Option<Vec<u8>>,
}

/// Auth middleware that validates bearer token from header or query param.
///
/// SSE connections can't set headers from `EventSource`, so we also accept
/// `?token=xxx` as a query parameter. The query value is percent-decoded before
/// validation so that tokens with URL-reserved characters (e.g. `+`, `=`, `/`)
/// are handled correctly.
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
        let token_hash = compute_token_hash(token, auth.hmac_key.as_deref());
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
    // Percent-decode the raw value so tokens with URL-reserved characters
    // (e.g. `+`, `=`, `/` from base64url) are accepted correctly.
    let query = query?;
    for pair in query.split('&') {
        if let Some(raw) = pair.strip_prefix("token=") {
            let token = urlencoding::decode(raw)
                .map(|cow| cow.into_owned())
                .unwrap_or_else(|_| raw.to_string());
            return Some(token);
        }
    }
    None
}

/// Hash a token using plain SHA-256, returning lowercase hex.
///
/// Retained for backward-compatibility with existing DB rows and for use in
/// tests. Production code should prefer [`compute_token_hash`] which uses
/// HMAC-SHA256 when a key is available.
pub fn hash_auth_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Derive a stable 32-byte HMAC key from the gateway auth token using
/// HKDF-SHA256 with a fixed info label.
///
/// The derived key is deterministic: the same `auth_token` always yields the
/// same key. Rotating `auth_token` (and restarting the server, which
/// re-upserts the token hash) automatically invalidates previously stored
/// hashes, which is the desired behaviour.
pub fn derive_token_hmac_key(auth_token: &str) -> Vec<u8> {
    let hk = Hkdf::<Sha256>::new(None, auth_token.as_bytes());
    let mut okm = vec![0u8; 32];
    // HKDF-SHA256 can produce up to 255 * 32 = 8 160 bytes of output.
    // Requesting 32 bytes is always within this limit; the error branch is
    // unreachable in practice. Degrade to raw SHA-256 bytes if it ever fires.
    if hk.expand(b"ironclaw-token-hash-v1", &mut okm).is_err() {
        let mut hasher = Sha256::new();
        Digest::update(&mut hasher, auth_token.as_bytes());
        return hasher.finalize().to_vec();
    }
    okm
}

/// Compute an HMAC-SHA256 of `data` keyed with `key`, returning lowercase hex.
fn hmac_sha256_hex(key: &[u8], data: &str) -> String {
    match HmacSha256::new_from_slice(key) {
        Ok(mut mac) => {
            mac.update(data.as_bytes());
            // Collect GenericArray bytes into hex string.
            mac.finalize()
                .into_bytes()
                .iter()
                .map(|b| format!("{:02x}", b))
                .collect()
        }
        Err(_) => {
            // HMAC accepts any key size; `InvalidLength` is unreachable here.
            // Degrade gracefully to plain SHA-256.
            hash_auth_token(data)
        }
    }
}

/// Hash a token using HMAC-SHA256 when a key is available, or plain SHA-256
/// when no key is provided.
///
/// Both the DB-write path (server startup `upsert_user_token_hash`) and the
/// DB-read path (request-time `get_user_by_token_hash`) must use this function
/// with the same key to keep hashes consistent.
pub fn compute_token_hash(token: &str, hmac_key: Option<&[u8]>) -> String {
    match hmac_key {
        Some(key) => hmac_sha256_hex(key, token),
        None => hash_auth_token(token),
    }
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
            hmac_key: None,
        };
        let cloned = state.clone();
        assert_eq!(cloned.token, "test-token");
        assert_eq!(cloned.fallback_principal.user_id, "default");
        assert_eq!(cloned.fallback_principal.role, UserRole::Admin);
        assert!(cloned.store.is_none());
        assert!(cloned.hmac_key.is_none());
    }

    #[test]
    fn test_hash_auth_token_deterministic() {
        let first = hash_auth_token("abc123");
        let second = hash_auth_token("abc123");
        let third = hash_auth_token("abc124");
        assert_eq!(first, second);
        assert_ne!(first, third);
    }

    #[test]
    fn test_compute_token_hash_hmac_differs_from_sha256() {
        let token = "my-secret-token";
        let key = derive_token_hmac_key("gateway-secret");
        let sha256_hash = hash_auth_token(token);
        let hmac_hash = compute_token_hash(token, Some(&key));
        // HMAC-keyed hash must differ from plain SHA-256.
        assert_ne!(sha256_hash, hmac_hash);
        // HMAC is deterministic: same key + token → same hash.
        assert_eq!(hmac_hash, compute_token_hash(token, Some(&key)));
        // Different keys → different hashes.
        let other_key = derive_token_hmac_key("different-gateway-secret");
        assert_ne!(hmac_hash, compute_token_hash(token, Some(&other_key)));
    }

    #[test]
    fn test_derive_token_hmac_key_is_deterministic() {
        let k1 = derive_token_hmac_key("stable-token");
        let k2 = derive_token_hmac_key("stable-token");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
        let k3 = derive_token_hmac_key("other-token");
        assert_ne!(k1, k3);
    }

    #[test]
    fn test_extract_query_token_url_decodes() {
        // Plain token (no encoding needed).
        assert_eq!(
            extract_query_token(Some("token=abc123")),
            Some("abc123".to_string())
        );
        // Token with URL-reserved characters that need decoding.
        assert_eq!(
            extract_query_token(Some("token=abc%2B123%3D%3D")),
            Some("abc+123==".to_string())
        );
        // token= is not the first param.
        assert_eq!(
            extract_query_token(Some("foo=bar&token=hello%20world")),
            Some("hello world".to_string())
        );
        // No token param.
        assert_eq!(extract_query_token(Some("foo=bar")), None);
        // No query string.
        assert_eq!(extract_query_token(None), None);
    }

    #[tokio::test]
    async fn test_auth_middleware_falls_back_to_query_token_when_header_invalid() {
        let auth_state = AuthState {
            token: "good-token".to_string(),
            fallback_principal: AuthPrincipal::new("default", UserRole::Admin),
            store: None,
            hmac_key: None,
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

    #[tokio::test]
    async fn test_auth_middleware_query_token_url_encoded() {
        // Token contains `+` which becomes `%2B` when URL-encoded.
        let raw_token = "tok+en==value";
        let auth_state = AuthState {
            token: raw_token.to_string(),
            fallback_principal: AuthPrincipal::new("default", UserRole::Admin),
            store: None,
            hmac_key: None,
        };

        async fn ok_handler() -> impl IntoResponse {
            StatusCode::OK
        }

        let app = Router::new()
            .route("/", get(ok_handler))
            .route_layer(middleware::from_fn_with_state(auth_state, auth_middleware));

        let req = Request::builder()
            .uri("/?token=tok%2Ben%3D%3Dvalue")
            .body(axum::body::Body::empty())
            .expect("valid request");

        let response = app.oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_auth_middleware_db_token_lookup_sha256_path() {
        // Exercises the multi-user DB lookup path with no HMAC key (SHA-256).
        let (db, _tmp) = crate::testing::test_db().await;
        let db: std::sync::Arc<dyn crate::db::Database> = db;

        // The token-hash FK references the users table — create the user first.
        db.ensure_user_account("db-user", "DB User", UserRole::Staff)
            .await
            .expect("create user");

        let token = "db-user-token-sha256";
        let token_hash = hash_auth_token(token);
        db.upsert_user_token_hash("db-user", &token_hash)
            .await
            .expect("upsert token hash");

        let auth_state = AuthState {
            token: "shared-fallback-token".to_string(),
            fallback_principal: AuthPrincipal::new("fallback-user", UserRole::Admin),
            store: Some(db),
            hmac_key: None,
        };

        async fn ok_handler() -> impl IntoResponse {
            StatusCode::OK
        }

        let app = Router::new()
            .route("/", get(ok_handler))
            .route_layer(middleware::from_fn_with_state(auth_state, auth_middleware));

        // Request with the DB-stored token → should authenticate via DB lookup.
        let req = Request::builder()
            .header("authorization", format!("Bearer {}", token))
            .body(axum::body::Body::empty())
            .expect("valid request");
        let response = app.clone().oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        // Request with the wrong token → should be rejected.
        let req = Request::builder()
            .header("authorization", "Bearer wrong-token")
            .body(axum::body::Body::empty())
            .expect("valid request");
        let response = app.oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_auth_middleware_db_token_lookup_hmac_path() {
        // Exercises the multi-user DB lookup path with HMAC-SHA256.
        let (db, _tmp) = crate::testing::test_db().await;
        let db: std::sync::Arc<dyn crate::db::Database> = db;

        // The token-hash FK references the users table — create the user first.
        db.ensure_user_account("hmac-user", "HMAC User", UserRole::Staff)
            .await
            .expect("create user");

        let token = "db-user-token-hmac";
        let hmac_key = derive_token_hmac_key("gateway-secret-for-test");
        let token_hash = compute_token_hash(token, Some(&hmac_key));
        db.upsert_user_token_hash("hmac-user", &token_hash)
            .await
            .expect("upsert token hash");

        let auth_state = AuthState {
            token: "shared-fallback-token".to_string(),
            fallback_principal: AuthPrincipal::new("fallback-user", UserRole::Admin),
            store: Some(db),
            hmac_key: Some(hmac_key),
        };

        async fn ok_handler() -> impl IntoResponse {
            StatusCode::OK
        }

        let app = Router::new()
            .route("/", get(ok_handler))
            .route_layer(middleware::from_fn_with_state(auth_state, auth_middleware));

        // Correct token → authenticated via HMAC DB lookup.
        let req = Request::builder()
            .header("authorization", format!("Bearer {}", token))
            .body(axum::body::Body::empty())
            .expect("valid request");
        let response = app.clone().oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);

        // Wrong token → rejected (HMAC hash won't match).
        let req = Request::builder()
            .header("authorization", "Bearer wrong-token")
            .body(axum::body::Body::empty())
            .expect("valid request");
        let response = app.oneshot(req).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
