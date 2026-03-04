//! Route composition for web gateway handlers.

use std::sync::Arc;

use axum::{Router, middleware};

use crate::channels::web::auth::{AuthState, auth_middleware};
use crate::channels::web::state::GatewayState;

pub fn public_routes() -> Router<Arc<GatewayState>> {
    super::gateway::public_routes()
}

pub fn static_routes() -> Router<Arc<GatewayState>> {
    super::static_files::routes()
}

pub fn project_routes(auth_state: AuthState) -> Router<Arc<GatewayState>> {
    super::projects::routes()
        .route_layer(middleware::from_fn_with_state(auth_state, auth_middleware))
}

pub fn protected_feature_routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .merge(super::chat::routes())
        .merge(super::memory::routes())
        .merge(super::matters::routes())
        .merge(super::legal::routes())
        .merge(super::jobs::routes())
        .merge(super::logs::routes())
        .merge(super::extensions::routes())
        .merge(super::pairing::routes())
        .merge(super::routines::routes())
        .merge(super::settings::routes())
        .merge(super::backups::routes())
        .merge(super::gateway::routes())
        .merge(super::skills::routes())
}
