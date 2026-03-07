//! User administration handlers (role update, deactivation).
//!
//! All endpoints require Admin [`UserRole`]. They are intended for gateway
//! operators managing multi-user deployments and are always gated by the
//! standard bearer-token auth middleware.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};

use crate::channels::web::auth::RequestPrincipal;
use crate::channels::web::state::GatewayState;
use crate::channels::web::types::{UpdateUserRoleRequest, UserResponse};
use crate::db::UserRole;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route(
            "/api/users/{user_id}/role",
            axum::routing::put(update_user_role_handler),
        )
        .route(
            "/api/users/{user_id}/deactivate",
            post(deactivate_user_handler),
        )
}

/// `PUT /api/users/{user_id}/role` — change a user's system role (Admin only).
///
/// Request body: `{ "role": "attorney" }` where role is one of
/// `"admin"`, `"attorney"`, `"staff"`, `"viewer"`.
///
/// Returns 200 with the updated user record, or 403/404/400/503 on error.
async fn update_user_role_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(user_id): Path<String>,
    Json(body): Json<UpdateUserRoleRequest>,
) -> Result<Json<UserResponse>, StatusCode> {
    if principal.role != UserRole::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let new_role = UserRole::from_db_value(&body.role).ok_or(StatusCode::BAD_REQUEST)?;
    let store = state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let record = store
        .update_user_role(&user_id, new_role)
        .await
        .map_err(|e| {
            tracing::error!("update_user_role failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok(Json(UserResponse {
        user_id: record.id,
        display_name: record.display_name,
        role: record.role.as_str().to_string(),
        is_active: record.is_active,
        updated_at: record.updated_at.to_rfc3339(),
    }))
}

/// `POST /api/users/{user_id}/deactivate` — mark a user as inactive (Admin only).
///
/// Returns 204 on success. No-op if the user does not exist.
async fn deactivate_user_handler(
    State(state): State<Arc<GatewayState>>,
    RequestPrincipal(principal): RequestPrincipal,
    Path(user_id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    if principal.role != UserRole::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let store = state.store.as_ref().ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .deactivate_user(&user_id)
        .await
        .map_err(|e| {
            tracing::error!("deactivate_user failed: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(StatusCode::NO_CONTENT)
}
