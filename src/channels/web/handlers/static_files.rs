//! Embedded static asset handlers.

use std::sync::Arc;

use axum::{
    Router,
    extract::Path,
    http::{StatusCode, header},
    response::IntoResponse,
    routing::get,
};

use crate::channels::web::state::GatewayState;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/", get(index_handler))
        .route("/style.css", get(css_handler))
        .route("/app.js", get(js_handler))
        .route("/app/{*path}", get(app_module_handler))
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

const APP_MODULES: &[(&str, &str)] = &[
    ("main.js", include_str!("../static/app/main.js")),
    ("core/dom.js", include_str!("../static/app/core/dom.js")),
    ("core/http.js", include_str!("../static/app/core/http.js")),
    ("core/state.js", include_str!("../static/app/core/state.js")),
    ("core/tabs.js", include_str!("../static/app/core/tabs.js")),
];

fn resolve_app_module(path: &str) -> Option<&'static str> {
    APP_MODULES
        .iter()
        .find_map(|(name, source)| (*name == path).then_some(*source))
}

async fn app_module_handler(Path(path): Path<String>) -> impl IntoResponse {
    let normalized = path.trim_start_matches('/');
    match resolve_app_module(normalized) {
        Some(source) => (
            [
                (header::CONTENT_TYPE, "application/javascript"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            source,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "Not Found").into_response(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn serves_app_main_module() {
        let state = crate::channels::web::test_support::minimal_test_gateway_state(None);
        let app = routes().with_state(state);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/app/main.js")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(res.status(), StatusCode::OK);
        let content_type = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default();
        assert!(content_type.contains("application/javascript"));
    }

    #[tokio::test]
    async fn serves_core_module_from_allowlist() {
        let state = crate::channels::web::test_support::minimal_test_gateway_state(None);
        let app = routes().with_state(state);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/app/core/dom.js")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(res.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_app_module_returns_not_found() {
        let state = crate::channels::web::test_support::minimal_test_gateway_state(None);
        let app = routes().with_state(state);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/app/does-not-exist.js")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn traversal_style_app_module_path_returns_not_found() {
        let state = crate::channels::web::test_support::minimal_test_gateway_state(None);
        let app = routes().with_state(state);

        let res = app
            .oneshot(
                Request::builder()
                    .uri("/app/../app.js")
                    .method("GET")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");

        assert_eq!(res.status(), StatusCode::NOT_FOUND);
    }
}
