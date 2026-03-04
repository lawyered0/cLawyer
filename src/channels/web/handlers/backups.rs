//! Backup management handlers.

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
    routing::{get, post},
};
use chrono::Utc;
use uuid::Uuid;

use crate::channels::web::state::GatewayState;
use crate::channels::web::types::*;
use crate::db::AuditSeverity;

pub fn routes() -> Router<Arc<GatewayState>> {
    Router::new()
        .route("/api/backups/create", post(backups_create_handler))
        .route("/api/backups/verify", post(backups_verify_handler))
        .route(
            "/api/backups/restore",
            post(backups_restore_handler).layer(DefaultBodyLimit::max(
                crate::channels::web::server::BACKUP_RESTORE_SIZE_LIMIT,
            )),
        )
        .route("/api/backups/{id}/download", get(backups_download_handler))
}

fn parse_multipart_bool(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

async fn resolve_backup_master_key() -> Result<secrecy::SecretString, (StatusCode, String)> {
    let secrets = crate::config::SecretsConfig::resolve()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    secrets.master_key().cloned().ok_or((
        StatusCode::CONFLICT,
        "SECRETS_MASTER_KEY (or keychain-backed master key) is required".to_string(),
    ))
}

pub(crate) async fn backups_create_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<BackupCreateRequest>,
) -> Result<(StatusCode, Json<BackupCreateResponse>), (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let master_key = resolve_backup_master_key().await?;

    let backup_dir = crate::legal::backup::backups_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tokio::fs::create_dir_all(&backup_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let backup_id = format!(
        "backup-{}-{}",
        Utc::now().format("%Y%m%d-%H%M%S"),
        Uuid::new_v4().simple()
    );
    let output_path = backup_dir.join(format!("{backup_id}.clawyerbak"));
    let legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;

    let result = crate::legal::backup::create_backup_file(
        store.as_ref(),
        workspace.as_ref(),
        &state.user_id,
        &output_path,
        &master_key,
        &crate::legal::backup::BackupCreateOptions {
            include_ai_packets: req.include_ai_packets,
            matter_root: legal.matter_root.clone(),
        },
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "backup_created",
        state.user_id.as_str(),
        None,
        AuditSeverity::Info,
        serde_json::json!({
            "backup_id": result.artifact.id,
            "path": result.artifact.path,
            "size_bytes": result.artifact.size_bytes,
            "include_ai_packets": req.include_ai_packets,
        }),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(BackupCreateResponse {
            artifact: BackupArtifactInfo {
                id: result.artifact.id,
                path: result.artifact.path,
                created_at: result.artifact.created_at,
                size_bytes: result.artifact.size_bytes,
                encrypted: result.artifact.encrypted,
                plaintext_sha256: result.artifact.plaintext_sha256,
            },
            warnings: result.warnings,
            manifest: serde_json::to_value(result.manifest).unwrap_or_default(),
        }),
    ))
}

pub(crate) async fn backups_verify_handler(
    State(state): State<Arc<GatewayState>>,
    Json(req): Json<BackupVerifyRequest>,
) -> Result<Json<BackupVerifyResponse>, (StatusCode, String)> {
    let master_key = resolve_backup_master_key().await?;
    let input_path = crate::legal::backup::backup_path_for_id(&req.backup_id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if !input_path.exists() {
        return Err((StatusCode::NOT_FOUND, "Backup not found".to_string()));
    }

    let report = crate::legal::backup::verify_backup_file(&input_path, &master_key)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "backup_verified",
        state.user_id.as_str(),
        None,
        AuditSeverity::Info,
        serde_json::json!({
            "backup_id": req.backup_id,
            "path": input_path.to_string_lossy(),
            "valid": report.valid,
            "warning_count": report.warnings.len(),
        }),
    )
    .await;

    Ok(Json(BackupVerifyResponse {
        valid: report.valid,
        warnings: report.warnings,
        manifest: serde_json::to_value(report.manifest).unwrap_or_default(),
    }))
}

pub(crate) async fn backups_download_handler(
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let input_path = crate::legal::backup::backup_path_for_id(&id)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let bytes = tokio::fs::read(&input_path)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Backup not found".to_string()))?;

    let file_name = input_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("backup.clawyerbak")
        .to_string();

    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream".to_string()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", file_name),
            ),
        ],
        bytes,
    ))
}

pub(crate) async fn backups_restore_handler(
    State(state): State<Arc<GatewayState>>,
    mut multipart: Multipart,
) -> Result<Json<BackupRestoreResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let workspace = state.workspace.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))?;
    let master_key = resolve_backup_master_key().await?;
    let legal = crate::channels::web::server::legal_config_for_gateway_or_500(state.as_ref())?;

    let mut apply = false;
    let mut strict = false;
    let mut protect_identity_files = true;
    let mut file_bytes: Option<Vec<u8>> = None;
    let mut uploaded_name: Option<String> = None;

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Multipart read error: {}", e),
        )
    })? {
        let name = field.name().unwrap_or_default().to_string();
        if name == "apply" {
            let value = field.text().await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid apply flag: {}", e),
                )
            })?;
            apply = parse_multipart_bool(&value);
            continue;
        }
        if name == "strict" {
            let value = field.text().await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid strict flag: {}", e),
                )
            })?;
            strict = parse_multipart_bool(&value);
            continue;
        }
        if name == "protect_identity_files" {
            let value = field.text().await.map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid protect_identity_files flag: {}", e),
                )
            })?;
            protect_identity_files = parse_multipart_bool(&value);
            continue;
        }

        let filename_hint = field.file_name().map(|v| v.to_string());
        let bytes = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Invalid backup file: {}", e),
            )
        })?;
        file_bytes = Some(bytes.to_vec());
        uploaded_name = filename_hint;
    }

    let file_bytes = file_bytes.ok_or((
        StatusCode::BAD_REQUEST,
        "Missing backup file multipart field".to_string(),
    ))?;

    let backup_dir = crate::legal::backup::backups_dir()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tokio::fs::create_dir_all(&backup_dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let temp_path = backup_dir.join(format!(
        "restore-upload-{}-{}.clawyerbak",
        Utc::now().format("%Y%m%d-%H%M%S"),
        Uuid::new_v4().simple()
    ));
    tokio::fs::write(&temp_path, &file_bytes)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(err) = std::fs::set_permissions(&temp_path, perms) {
            tracing::warn!(
                "failed to tighten restore upload permissions for {}: {}",
                temp_path.to_string_lossy(),
                err
            );
        }
    }

    let restore_result = crate::legal::backup::restore_backup_file(
        store.as_ref(),
        workspace.as_ref(),
        &state.user_id,
        &temp_path,
        &master_key,
        &crate::legal::backup::BackupRestoreOptions {
            apply,
            strict,
            protect_identity_files,
            matter_root: legal.matter_root.clone(),
        },
    )
    .await;

    let _ = tokio::fs::remove_file(&temp_path).await;
    let report = restore_result.map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    crate::channels::web::server::record_legal_audit_event(
        state.as_ref(),
        "backup_restored",
        state.user_id.as_str(),
        None,
        if apply {
            AuditSeverity::Warn
        } else {
            AuditSeverity::Info
        },
        serde_json::json!({
            "uploaded_name": uploaded_name,
            "apply": apply,
            "strict": strict,
            "protect_identity_files": protect_identity_files,
            "restored_settings": report.restored_settings,
            "restored_workspace_files": report.restored_workspace_files,
            "skipped_workspace_files": report.skipped_workspace_files,
            "critical_failure_count": report.critical_failures.len(),
            "warning_count": report.warnings.len(),
        }),
    )
    .await;

    Ok(Json(BackupRestoreResponse {
        valid: report.valid,
        dry_run: report.dry_run,
        applied: report.applied,
        strict: report.strict,
        restored_settings: report.restored_settings,
        restored_workspace_files: report.restored_workspace_files,
        skipped_workspace_files: report.skipped_workspace_files,
        integrity: serde_json::to_value(report.integrity).unwrap_or_default(),
        critical_failures: report.critical_failures,
        warnings: report.warnings,
        manifest: serde_json::to_value(report.manifest).unwrap_or_default(),
    }))
}
