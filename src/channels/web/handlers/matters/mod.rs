//! Matter-related web handlers.

pub mod conflicts;
pub mod core;
pub mod documents;
pub mod finance;
pub mod work;

use std::sync::Arc;

use axum::Router;

use crate::channels::web::state::GatewayState;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .merge(core::routes())
        .merge(documents::routes())
        .merge(finance::routes())
        .merge(work::routes())
        .merge(conflicts::routes())
}
