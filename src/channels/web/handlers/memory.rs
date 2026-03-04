//! Memory API handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Query, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/memory/tree", get(memory_tree_handler))
        .route("/api/memory/list", get(memory_list_handler))
        .route("/api/memory/read", get(memory_read_handler))
        .route("/api/memory/write", post(memory_write_handler))
        .route("/api/memory/search", post(memory_search_handler))
        .route(
            "/api/memory/upload",
            post(memory_upload_handler).layer(DefaultBodyLimit::max(
                crate::channels::web::server::UPLOAD_FILE_SIZE_LIMIT,
            )),
        )
}

pub(crate) async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    Query(_query): Query<crate::channels::web::server::TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in &all_paths {
        let parts: Vec<&str> = path.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(TreeEntry {
                    path: dir_path,
                    is_dir: true,
                });
            }
        }
        entries.push(TreeEntry {
            path: path.clone(),
            is_dir: false,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(MemoryTreeResponse { entries }))
}

pub(crate) async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<crate::channels::web::server::ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace
        .list(path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let list_entries: Vec<ListEntry> = entries
        .iter()
        .map(|e| ListEntry {
            name: e.path.rsplit('/').next().unwrap_or(&e.path).to_string(),
            path: e.path.clone(),
            is_dir: e.is_directory,
            updated_at: e.updated_at.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    Ok(Json(MemoryListResponse {
        path: path.to_string(),
        entries: list_entries,
    }))
}

pub(crate) async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    Query(query): Query<crate::channels::web::server::ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let doc = workspace
        .read(&query.path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(MemoryReadResponse {
        path: query.path,
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    }))
}

pub(crate) async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let resolved_path = crate::channels::web::server::resolve_memory_write_path_for_gateway(
        state.as_ref(),
        &req.path,
    )
    .await?;

    workspace
        .write(&resolved_path, &req.content)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if crate::legal::matter::is_workspace_conflicts_path(&resolved_path) {
        crate::legal::matter::invalidate_conflict_cache();
    }

    Ok(Json(MemoryWriteResponse {
        path: resolved_path,
        status: "written",
    }))
}

pub(crate) async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let limit = req.limit.unwrap_or(10);
    let results = workspace
        .search(&req.query, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let hits: Vec<SearchHit> = results
        .iter()
        .map(|r| SearchHit {
            path: r.document_id.to_string(),
            content: r.content.clone(),
            score: r.score as f64,
        })
        .collect();

    Ok(Json(MemorySearchResponse { results: hits }))
}

pub(crate) async fn memory_upload_handler(
    State(state): State<Arc<GatewayState>>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<MemoryUploadResponse>), (StatusCode, String)> {
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;

    let mut uploaded: Vec<UploadedFile> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Multipart read error: {e}"),
        )
    })? {
        let raw_name = field.file_name().unwrap_or("document.txt").to_string();
        let safe_name: String = raw_name
            .rsplit('/')
            .next()
            .unwrap_or("document.txt")
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ' '))
            .collect();
        let safe_name = if safe_name.trim().is_empty() {
            "document.txt".to_string()
        } else {
            safe_name.trim().to_string()
        };
        let dest_path = format!("uploads/{safe_name}");

        let data = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Failed to read upload body: {e}"),
            )
        })?;

        if data.len() > crate::channels::web::server::UPLOAD_FILE_SIZE_LIMIT {
            return Err((
                StatusCode::PAYLOAD_TOO_LARGE,
                format!(
                    "File '{}' exceeds the 10 MiB upload limit ({} bytes)",
                    raw_name,
                    data.len()
                ),
            ));
        }

        let content = String::from_utf8(data.to_vec()).map_err(|_| {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!(
                    "File '{}' contains non-UTF-8 bytes. Only plain-text files are supported.",
                    raw_name
                ),
            )
        })?;

        let byte_count = content.len();
        workspace
            .write(&dest_path, &content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        uploaded.push(UploadedFile {
            path: dest_path,
            bytes: byte_count,
            status: "written",
        });
    }

    Ok((
        StatusCode::CREATED,
        Json(MemoryUploadResponse { files: uploaded }),
    ))
}
