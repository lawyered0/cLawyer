//! Embedded static asset handlers.

use std::sync::Arc;

use axum::{Router, http::header, response::IntoResponse, routing::get};

use crate::channels::web::state::GatewayState;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/", get(index_handler))
        .route("/style.css", get(css_handler))
        .route("/app.js", get(js_handler))
        .route("/favicon.ico", get(favicon_handler))
}

async fn index_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/index.html"),
    )
}

async fn css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/style.css"),
    )
}

async fn js_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "application/javascript"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        include_str!("../static/app.js"),
    )
}

async fn favicon_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/x-icon"),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        include_bytes!("../static/favicon.ico").as_slice(),
    )
}

// --- Health ---
