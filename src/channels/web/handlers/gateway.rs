//! Gateway health and status handlers.

use std::sync::Arc;

use axum::{Json, Router, extract::State, routing::get};

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::HealthResponse;

pub fn public_routes() -> Router<Arc<GatewayState>> {
    Router::new().route("/api/health", get(health_handler))
}

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new().route("/api/gateway/status", get(gateway_status_handler))
}

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy",
        channel: "gateway",
    })
}

// --- Chat handlers ---

async fn gateway_status_handler(
    State(state): State<Arc<GatewayState>>,
) -> Json<GatewayStatusResponse> {
    let sse_connections = state.sse.connection_count();
    let ws_connections = state
        .ws_tracker
        .as_ref()
        .map(|t| t.connection_count())
        .unwrap_or(0);

    let uptime_secs = state.startup_time.elapsed().as_secs();

    let (daily_cost, actions_this_hour, model_usage) = if let Some(ref cg) = state.cost_guard {
        let cost = cg.daily_spend().await;
        let actions = cg.actions_this_hour().await;
        let usage = cg.model_usage().await;
        let models: Vec<ModelUsageEntry> = usage
            .into_iter()
            .map(|(model, tokens)| ModelUsageEntry {
                model,
                input_tokens: tokens.input_tokens,
                output_tokens: tokens.output_tokens,
                cost: format!("{:.6}", tokens.cost),
            })
            .collect();
        (Some(format!("{:.4}", cost)), Some(actions), Some(models))
    } else {
        (None, None, None)
    };

    Json(GatewayStatusResponse {
        sse_connections,
        ws_connections,
        total_connections: sse_connections + ws_connections,
        uptime_secs,
        daily_cost,
        actions_this_hour,
        model_usage,
    })
}

#[derive(serde::Serialize)]
struct ModelUsageEntry {
    model: String,
    input_tokens: u64,
    output_tokens: u64,
    cost: String,
}

#[derive(serde::Serialize)]
struct GatewayStatusResponse {
    sse_connections: u64,
    ws_connections: u64,
    total_connections: u64,
    uptime_secs: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    daily_cost: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions_this_hour: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_usage: Option<Vec<ModelUsageEntry>>,
}
