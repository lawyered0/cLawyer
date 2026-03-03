use std::collections::{BTreeMap, HashMap};
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Archive, Builder, Header};
use uuid::Uuid;

use crate::db::{
    AppendAuditEventParams, AuditEventQuery, AuditEventRecord, AuditSeverity, ClientRecord,
    CreateClientParams, Database, DocumentTemplateRecord, ExpenseEntryRecord,
    InvoiceLineItemRecord, InvoiceRecord, MatterDeadlineRecord, MatterDocumentRecord,
    MatterNoteRecord, MatterRecord, MatterTaskRecord, MatterTimeSummary, TimeEntryRecord,
    TrustLedgerEntryRecord, UpsertMatterParams,
};
use crate::error::WorkspaceError;
use crate::legal::matter::read_matter_metadata_for_root;
use crate::legal::policy::sanitize_matter_id;
use crate::safety::LeakDetector;
use crate::workspace::{Workspace, paths};

const BACKUP_FORMAT: &str = "clawyer-backup";
const BACKUP_FORMAT_VERSION: u32 = 1;
const BACKUP_SCHEMA_VERSION: u32 = 1;
const BACKUP_MAX_FILE_BYTES: usize = 512 * 1024;
const BACKUP_MAX_TOTAL_FILE_BYTES: usize = 64 * 1024 * 1024;
const BACKUP_AUDIT_MAX_ROWS: usize = 10_000;

const PROTECTED_IDENTITY_FILES: &[&str] =
    &[paths::IDENTITY, paths::SOUL, paths::AGENTS, paths::USER];
const ALLOWED_GLOBAL_RESTORE_FILES: &[&str] = &["conflicts.json"];

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("backup config error: {0}")]
    Config(String),
    #[error("backup serialization error: {0}")]
    Serialization(String),
    #[error("backup IO error: {0}")]
    Io(String),
    #[error("backup crypto error: {0}")]
    Crypto(String),
    #[error("backup policy error: {0}")]
    Policy(String),
    #[error("backup validation failed: {0}")]
    Validation(String),
}

impl From<WorkspaceError> for BackupError {
    fn from(value: WorkspaceError) -> Self {
        Self::Io(value.to_string())
    }
}

#[derive(Debug, Clone)]
pub struct BackupCreateOptions {
    pub include_ai_packets: bool,
    pub matter_root: String,
}

impl Default for BackupCreateOptions {
    fn default() -> Self {
        Self {
            include_ai_packets: false,
            matter_root: "matters".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackupRestoreOptions {
    pub apply: bool,
    pub protect_identity_files: bool,
    pub matter_root: String,
}

impl Default for BackupRestoreOptions {
    fn default() -> Self {
        Self {
            apply: false,
            protect_identity_files: true,
            matter_root: "matters".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct MatterRetrievalExportOptions {
    pub redacted: bool,
    pub matter_root: String,
}

impl Default for MatterRetrievalExportOptions {
    fn default() -> Self {
        Self {
            redacted: true,
            matter_root: "matters".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupArtifactInfo {
    pub id: String,
    pub path: String,
    pub created_at: String,
    pub size_bytes: usize,
    pub encrypted: bool,
    pub plaintext_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupCreateResult {
    pub artifact: BackupArtifactInfo,
    pub warnings: Vec<String>,
    pub manifest: BackupManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupVerifyResult {
    pub valid: bool,
    pub warnings: Vec<String>,
    pub manifest: BackupManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRestoreResult {
    pub valid: bool,
    pub dry_run: bool,
    pub applied: bool,
    pub restored_settings: usize,
    pub restored_workspace_files: usize,
    pub skipped_workspace_files: usize,
    pub entity_counts: BackupRestoreEntityCounts,
    pub warnings: Vec<String>,
    pub manifest: BackupManifest,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupRestoreEntityCounts {
    pub clients: usize,
    pub matters: usize,
    pub templates: usize,
    pub tasks: usize,
    pub notes: usize,
    pub deadlines: usize,
    pub documents: usize,
    pub document_versions: usize,
    pub time_entries: usize,
    pub expense_entries: usize,
    pub invoices: usize,
    pub invoice_line_items: usize,
    pub trust_ledger: usize,
    pub audit_events: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatterRetrievalExportResult {
    pub matter_id: String,
    pub output_dir: String,
    pub redacted: bool,
    pub files: Vec<String>,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupManifest {
    pub format_version: u32,
    pub schema_version: u32,
    pub app_version: String,
    pub created_at: String,
    pub user_id: String,
    pub encrypted: bool,
    pub hash_algorithm: String,
    pub section_checksums: BTreeMap<String, String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupEnvelope {
    format: String,
    format_version: u32,
    created_at: String,
    manifest: BackupManifest,
    encryption: BackupEncryption,
    ciphertext_b64: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupEncryption {
    algorithm: String,
    salt_b64: String,
    plaintext_sha256: String,
    ciphertext_sha256: String,
    compressed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BackupSnapshot {
    created_at: String,
    user_id: String,
    settings: HashMap<String, serde_json::Value>,
    legal: LegalBackupSnapshot,
    workspace: WorkspaceBackupSnapshot,
    ai_packets: Vec<AiPacketPreview>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegalBackupSnapshot {
    clients: Vec<ClientRecord>,
    matters: Vec<MatterRecord>,
    tasks: Vec<MatterTaskRecord>,
    notes: Vec<MatterNoteRecord>,
    deadlines: Vec<MatterDeadlineRecord>,
    documents: Vec<MatterDocumentRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    document_versions: Vec<crate::db::DocumentVersionRecord>,
    templates: Vec<DocumentTemplateRecord>,
    time_entries: Vec<TimeEntryRecord>,
    expense_entries: Vec<ExpenseEntryRecord>,
    time_summaries: Vec<MatterTimeSummaryRecord>,
    trust_ledger: Vec<TrustLedgerEntryRecord>,
    invoices: Vec<InvoiceRecord>,
    invoice_line_items: Vec<InvoiceLineItemRecord>,
    audit_events: Vec<AuditEventRecord>,
    conflict_graph_summary: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MatterTimeSummaryRecord {
    matter_id: String,
    summary: MatterTimeSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceBackupSnapshot {
    files: Vec<WorkspaceFileSnapshot>,
    total_original_bytes: usize,
    total_stored_bytes: usize,
    truncated_files: usize,
    skipped_files: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WorkspaceFileSnapshot {
    path: String,
    memory_document_id: Option<String>,
    content: String,
    original_bytes: usize,
    stored_bytes: usize,
    truncated: bool,
    skipped: bool,
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AiPacketPreview {
    matter_id: String,
    brief_markdown: String,
}

pub async fn create_backup_file(
    db: &dyn Database,
    workspace: &Workspace,
    user_id: &str,
    output_path: &Path,
    master_key: &SecretString,
    options: &BackupCreateOptions,
) -> Result<BackupCreateResult, BackupError> {
    let mut warnings = Vec::new();
    let snapshot = collect_snapshot(db, workspace, user_id, options, &mut warnings).await?;
    let manifest = build_manifest(&snapshot, true, &warnings)?;
    let archive = build_archive_bytes(&manifest, &snapshot)?;
    let plaintext_sha256 = sha256_hex(&archive);
    let archive_b64 = BASE64.encode(&archive);

    let crypto = crate::secrets::SecretsCrypto::new(master_key.clone())
        .map_err(|e| BackupError::Crypto(e.to_string()))?;
    let (ciphertext, salt) = crypto
        .encrypt(archive_b64.as_bytes())
        .map_err(|e| BackupError::Crypto(e.to_string()))?;

    let envelope = BackupEnvelope {
        format: BACKUP_FORMAT.to_string(),
        format_version: BACKUP_FORMAT_VERSION,
        created_at: manifest.created_at.clone(),
        manifest: manifest.clone(),
        encryption: BackupEncryption {
            algorithm: "aes-256-gcm-hkdf-sha256".to_string(),
            salt_b64: BASE64.encode(salt),
            plaintext_sha256: plaintext_sha256.clone(),
            ciphertext_sha256: sha256_hex(&ciphertext),
            compressed: true,
        },
        ciphertext_b64: BASE64.encode(ciphertext),
    };

    let serialized = serde_json::to_vec_pretty(&envelope)
        .map_err(|e| BackupError::Serialization(e.to_string()))?;

    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
    }
    tokio::fs::write(output_path, &serialized)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;

    set_owner_only_permissions(output_path)?;

    Ok(BackupCreateResult {
        artifact: BackupArtifactInfo {
            id: output_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("backup")
                .to_string(),
            path: output_path.to_string_lossy().to_string(),
            created_at: manifest.created_at.clone(),
            size_bytes: serialized.len(),
            encrypted: true,
            plaintext_sha256,
        },
        warnings,
        manifest,
    })
}

pub async fn verify_backup_file(
    input_path: &Path,
    master_key: &SecretString,
) -> Result<BackupVerifyResult, BackupError> {
    let envelope_bytes = tokio::fs::read(input_path)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let (manifest, snapshot, warnings) = decode_and_validate_backup(&envelope_bytes, master_key)?;

    let mut out_warnings = warnings;
    if snapshot.user_id.trim().is_empty() {
        out_warnings.push("snapshot user_id is empty".to_string());
    }

    Ok(BackupVerifyResult {
        valid: true,
        warnings: out_warnings,
        manifest,
    })
}

pub async fn restore_backup_file(
    db: &dyn Database,
    workspace: &Workspace,
    user_id: &str,
    input_path: &Path,
    master_key: &SecretString,
    options: &BackupRestoreOptions,
) -> Result<BackupRestoreResult, BackupError> {
    let envelope_bytes = tokio::fs::read(input_path)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let (manifest, snapshot, mut warnings) =
        decode_and_validate_backup(&envelope_bytes, master_key)?;

    let mut restored_settings = 0usize;
    let mut restored_workspace_files = 0usize;
    let mut skipped_workspace_files = 0usize;
    let mut entity_counts = BackupRestoreEntityCounts::default();

    if options.apply {
        db.set_all_settings(user_id, &snapshot.settings)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        restored_settings = snapshot.settings.len();
        let mut memory_doc_id_map: HashMap<uuid::Uuid, uuid::Uuid> = HashMap::new();

        for file in &snapshot.workspace.files {
            if file.skipped {
                skipped_workspace_files += 1;
                continue;
            }

            let Some(normalized_path) = normalize_restore_path(&file.path) else {
                skipped_workspace_files += 1;
                warnings.push(format!("skipped unsafe path '{}'", file.path));
                continue;
            };

            if options.protect_identity_files && is_protected_identity_path(&normalized_path) {
                skipped_workspace_files += 1;
                warnings.push(format!(
                    "skipped protected identity file '{}'",
                    normalized_path
                ));
                continue;
            }

            if !is_allowed_restore_path(&normalized_path, &options.matter_root) {
                skipped_workspace_files += 1;
                warnings.push(format!(
                    "skipped path '{}' outside restore scope (matter_root='{}')",
                    normalized_path, options.matter_root
                ));
                continue;
            }

            let restored_doc =
                if crate::legal::workspace_crypto::is_encrypted_payload(&file.content) {
                    workspace
                        .write_stored(&normalized_path, &file.content)
                        .await
                        .map_err(|e| BackupError::Io(e.to_string()))?
                } else {
                    workspace
                        .write(&normalized_path, &file.content)
                        .await
                        .map_err(|e| BackupError::Io(e.to_string()))?
                };
            if let Some(raw_old_id) = file.memory_document_id.as_deref()
                && let Ok(old_id) = uuid::Uuid::parse_str(raw_old_id)
            {
                memory_doc_id_map.insert(old_id, restored_doc.id);
            }
            restored_workspace_files += 1;
        }

        let mut client_id_map: HashMap<uuid::Uuid, uuid::Uuid> = HashMap::new();
        for client in &snapshot.legal.clients {
            let restored = db
                .upsert_client_by_normalized_name(
                    user_id,
                    &CreateClientParams {
                        name: client.name.clone(),
                        client_type: client.client_type,
                        email: client.email.clone(),
                        phone: client.phone.clone(),
                        address: client.address.clone(),
                        notes: client.notes.clone(),
                    },
                )
                .await
                .map_err(|e| BackupError::Io(e.to_string()))?;
            client_id_map.insert(client.id, restored.id);
            entity_counts.clients += 1;
        }

        for matter in &snapshot.legal.matters {
            let Some(restored_client_id) = client_id_map.get(&matter.client_id).copied() else {
                warnings.push(format!(
                    "skipped matter '{}' due to missing client mapping",
                    matter.matter_id
                ));
                continue;
            };
            db.upsert_matter(
                user_id,
                &UpsertMatterParams {
                    matter_id: matter.matter_id.clone(),
                    client_id: restored_client_id,
                    status: matter.status,
                    stage: matter.stage.clone(),
                    practice_area: matter.practice_area.clone(),
                    jurisdiction: matter.jurisdiction.clone(),
                    opened_at: matter.opened_at,
                    closed_at: matter.closed_at,
                    assigned_to: matter.assigned_to.clone(),
                    custom_fields: matter.custom_fields.clone(),
                },
            )
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
            entity_counts.matters += 1;
        }

        for template in &snapshot.legal.templates {
            db.upsert_document_template(
                user_id,
                &crate::db::UpsertDocumentTemplateParams {
                    matter_id: template.matter_id.clone(),
                    name: template.name.clone(),
                    body: template.body.clone(),
                    variables_json: template.variables_json.clone(),
                },
            )
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
            entity_counts.templates += 1;
        }

        for row in &snapshot.legal.tasks {
            match db.upsert_matter_task_record(row).await {
                Ok(_) => entity_counts.tasks += 1,
                Err(e) => warnings.push(format!(
                    "failed to restore task {} (matter={}): {}",
                    row.id, row.matter_id, e
                )),
            }
        }

        for row in &snapshot.legal.notes {
            match db.upsert_matter_note_record(row).await {
                Ok(_) => entity_counts.notes += 1,
                Err(e) => warnings.push(format!(
                    "failed to restore note {} (matter={}): {}",
                    row.id, row.matter_id, e
                )),
            }
        }

        for row in &snapshot.legal.deadlines {
            match db.upsert_matter_deadline_record(row).await {
                Ok(_) => entity_counts.deadlines += 1,
                Err(e) => warnings.push(format!(
                    "failed to restore deadline {} (matter={}): {}",
                    row.id, row.matter_id, e
                )),
            }
        }

        for row in &snapshot.legal.documents {
            let mut adjusted = row.clone();
            if let Some(mapped) = memory_doc_id_map.get(&row.memory_document_id).copied() {
                adjusted.memory_document_id = mapped;
            }
            match db.upsert_matter_document_record(&adjusted).await {
                Ok(_) => entity_counts.documents += 1,
                Err(e) => warnings.push(format!(
                    "failed to restore document {} (matter={} path='{}'): {}",
                    row.id, row.matter_id, row.path, e
                )),
            }
        }

        for row in &snapshot.legal.document_versions {
            let mut adjusted = row.clone();
            if let Some(mapped) = memory_doc_id_map.get(&row.memory_document_id).copied() {
                adjusted.memory_document_id = mapped;
            }
            match db.upsert_document_version_record(&adjusted).await {
                Ok(_) => entity_counts.document_versions += 1,
                Err(e) => warnings.push(format!(
                    "failed to restore document version {}: {}",
                    row.id, e
                )),
            }
        }

        for row in &snapshot.legal.time_entries {
            match db.upsert_time_entry_record(row).await {
                Ok(_) => entity_counts.time_entries += 1,
                Err(e) => warnings.push(format!("failed to restore time entry {}: {}", row.id, e)),
            }
        }

        for row in &snapshot.legal.expense_entries {
            match db.upsert_expense_entry_record(row).await {
                Ok(_) => entity_counts.expense_entries += 1,
                Err(e) => {
                    warnings.push(format!("failed to restore expense entry {}: {}", row.id, e))
                }
            }
        }

        for row in &snapshot.legal.invoices {
            match db.upsert_invoice_record(row).await {
                Ok(_) => entity_counts.invoices += 1,
                Err(e) => warnings.push(format!("failed to restore invoice {}: {}", row.id, e)),
            }
        }

        for row in &snapshot.legal.invoice_line_items {
            match db.upsert_invoice_line_item_record(row).await {
                Ok(_) => entity_counts.invoice_line_items += 1,
                Err(e) => warnings.push(format!(
                    "failed to restore invoice line item {}: {}",
                    row.id, e
                )),
            }
        }

        for row in &snapshot.legal.trust_ledger {
            match db.upsert_trust_ledger_entry_record(row).await {
                Ok(_) => entity_counts.trust_ledger += 1,
                Err(e) => warnings.push(format!(
                    "failed to restore trust ledger entry {}: {}",
                    row.id, e
                )),
            }
        }

        for row in &snapshot.legal.audit_events {
            match db.upsert_audit_event_record(row).await {
                Ok(_) => entity_counts.audit_events += 1,
                Err(e) => warnings.push(format!("failed to restore audit event {}: {}", row.id, e)),
            }
        }

        let _ = db
            .append_audit_event(
                user_id,
                &AppendAuditEventParams {
                    event_type: "backup_restore_applied".to_string(),
                    actor: user_id.to_string(),
                    matter_id: None,
                    severity: AuditSeverity::Info,
                    details: serde_json::json!({
                        "input": input_path.to_string_lossy(),
                        "restored_settings": restored_settings,
                        "restored_workspace_files": restored_workspace_files,
                        "skipped_workspace_files": skipped_workspace_files,
                        "entity_counts": entity_counts.clone(),
                    }),
                },
            )
            .await;
    }

    Ok(BackupRestoreResult {
        valid: true,
        dry_run: !options.apply,
        applied: options.apply,
        restored_settings,
        restored_workspace_files,
        skipped_workspace_files,
        entity_counts,
        warnings,
        manifest,
    })
}

pub async fn export_matter_retrieval_packet(
    db: &dyn Database,
    workspace: &Workspace,
    user_id: &str,
    matter_id: &str,
    options: &MatterRetrievalExportOptions,
    redaction_cfg: Option<&crate::config::LegalRedactionConfig>,
) -> Result<MatterRetrievalExportResult, BackupError> {
    let matter_id = sanitize_matter_id(matter_id);
    if matter_id.is_empty() {
        return Err(BackupError::Validation(
            "matter_id is empty after sanitization".to_string(),
        ));
    }

    let matter = db
        .get_matter_db(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?
        .ok_or_else(|| BackupError::Validation(format!("matter '{}' not found", matter_id)))?;

    let metadata = read_optional_matter_metadata(workspace, &options.matter_root, &matter_id).await;
    let tasks = db
        .list_matter_tasks(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let notes = db
        .list_matter_notes(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let deadlines = db
        .list_matter_deadlines(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let documents = db
        .list_matter_documents_db(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let time_entries = db
        .list_time_entries(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let expenses = db
        .list_expense_entries(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let trust = db
        .list_trust_ledger_entries(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let invoices = db
        .list_invoices(user_id, Some(&matter_id))
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let audits = db
        .list_audit_events(
            user_id,
            &AuditEventQuery {
                matter_id: Some(matter_id.clone()),
                ..AuditEventQuery::default()
            },
            500,
            0,
        )
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    let summary = db
        .matter_time_summary(user_id, &matter_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;

    let ts = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let export_suffix = Uuid::new_v4().simple().to_string();
    let out_dir = format!(
        "{}/{}/exports/retrieval/{}-{}",
        options.matter_root.trim_matches('/'),
        matter_id,
        ts,
        &export_suffix[..8]
    );

    let detector = LeakDetector::new_with_legal_redaction(redaction_cfg);
    let mut files = Vec::new();
    let mut sources_rows: Vec<Vec<String>> = Vec::new();

    let overview_csv = build_csv(
        &[
            "matter_id",
            "client",
            "status",
            "practice_area",
            "jurisdiction",
            "opened_at",
            "closed_at",
        ],
        vec![vec![
            matter.matter_id.clone(),
            metadata
                .as_ref()
                .map(|m| m.client.clone())
                .unwrap_or_else(|| "".to_string()),
            matter.status.as_str().to_string(),
            matter.practice_area.clone().unwrap_or_default(),
            matter.jurisdiction.clone().unwrap_or_default(),
            matter
                .opened_at
                .map(|d| d.date_naive().to_string())
                .unwrap_or_default(),
            matter
                .closed_at
                .map(|d| d.date_naive().to_string())
                .unwrap_or_default(),
        ]],
    );
    let overview_csv = redact_if_needed(&overview_csv, options.redacted, &detector)?;
    let overview_path = format!("{}/matter_overview.csv", out_dir);
    workspace
        .write(&overview_path, &overview_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(overview_path.clone());
    sources_rows.push(vec![
        "matter_overview".to_string(),
        matter.matter_id.clone(),
        "db.matters".to_string(),
    ]);

    let parties_csv = build_csv(&["role", "name"], {
        let mut rows = Vec::new();
        if let Some(meta) = metadata.as_ref() {
            rows.push(vec!["client".to_string(), meta.client.clone()]);
            for adv in &meta.adversaries {
                rows.push(vec!["adversary".to_string(), adv.clone()]);
            }
            for member in &meta.team {
                rows.push(vec!["team".to_string(), member.clone()]);
            }
        }
        rows
    });
    let parties_csv = redact_if_needed(&parties_csv, options.redacted, &detector)?;
    let parties_path = format!("{}/parties.csv", out_dir);
    workspace
        .write(&parties_path, &parties_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(parties_path.clone());

    let deadlines_csv = build_csv(
        &["id", "title", "type", "due_at", "completed_at", "rule_ref"],
        deadlines
            .iter()
            .map(|d| {
                vec![
                    d.id.to_string(),
                    d.title.clone(),
                    d.deadline_type.as_str().to_string(),
                    d.due_at.to_rfc3339(),
                    d.completed_at.map(|v| v.to_rfc3339()).unwrap_or_default(),
                    d.rule_ref.clone().unwrap_or_default(),
                ]
            })
            .collect(),
    );
    let deadlines_csv = redact_if_needed(&deadlines_csv, options.redacted, &detector)?;
    let deadlines_path = format!("{}/deadlines.csv", out_dir);
    workspace
        .write(&deadlines_path, &deadlines_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(deadlines_path.clone());

    let tasks_csv = build_csv(
        &["id", "title", "status", "assignee", "due_at"],
        tasks
            .iter()
            .map(|t| {
                vec![
                    t.id.to_string(),
                    t.title.clone(),
                    t.status.as_str().to_string(),
                    t.assignee.clone().unwrap_or_default(),
                    t.due_at.map(|v| v.to_rfc3339()).unwrap_or_default(),
                ]
            })
            .collect(),
    );
    let tasks_csv = redact_if_needed(&tasks_csv, options.redacted, &detector)?;
    let tasks_path = format!("{}/tasks.csv", out_dir);
    workspace
        .write(&tasks_path, &tasks_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(tasks_path.clone());

    let notes_csv = build_csv(
        &["id", "author", "pinned", "created_at", "body"],
        notes
            .iter()
            .map(|n| {
                vec![
                    n.id.to_string(),
                    n.author.clone(),
                    n.pinned.to_string(),
                    n.created_at.to_rfc3339(),
                    n.body.clone(),
                ]
            })
            .collect(),
    );
    let notes_csv = redact_if_needed(&notes_csv, options.redacted, &detector)?;
    let notes_path = format!("{}/notes.csv", out_dir);
    workspace
        .write(&notes_path, &notes_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(notes_path.clone());

    let documents_csv = build_csv(
        &["id", "name", "path", "category", "updated_at"],
        documents
            .iter()
            .map(|d| {
                vec![
                    d.id.to_string(),
                    d.display_name.clone(),
                    d.path.clone(),
                    d.category.as_str().to_string(),
                    d.updated_at.to_rfc3339(),
                ]
            })
            .collect(),
    );
    let documents_csv = redact_if_needed(&documents_csv, options.redacted, &detector)?;
    let documents_path = format!("{}/documents.csv", out_dir);
    workspace
        .write(&documents_path, &documents_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(documents_path.clone());

    let time_csv = build_csv(
        &[
            "id",
            "timekeeper",
            "description",
            "hours",
            "entry_date",
            "billable",
        ],
        time_entries
            .iter()
            .map(|e| {
                vec![
                    e.id.to_string(),
                    e.timekeeper.clone(),
                    e.description.clone(),
                    e.hours.to_string(),
                    e.entry_date.to_string(),
                    e.billable.to_string(),
                ]
            })
            .collect(),
    );
    let time_csv = redact_if_needed(&time_csv, options.redacted, &detector)?;
    let time_path = format!("{}/time_entries.csv", out_dir);
    workspace
        .write(&time_path, &time_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(time_path.clone());

    let expenses_csv = build_csv(
        &[
            "id",
            "submitted_by",
            "description",
            "amount",
            "category",
            "entry_date",
            "billable",
        ],
        expenses
            .iter()
            .map(|e| {
                vec![
                    e.id.to_string(),
                    e.submitted_by.clone(),
                    e.description.clone(),
                    e.amount.to_string(),
                    e.category.as_str().to_string(),
                    e.entry_date.to_string(),
                    e.billable.to_string(),
                ]
            })
            .collect(),
    );
    let expenses_csv = redact_if_needed(&expenses_csv, options.redacted, &detector)?;
    let expenses_path = format!("{}/expenses.csv", out_dir);
    workspace
        .write(&expenses_path, &expenses_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(expenses_path.clone());

    let trust_csv = build_csv(
        &[
            "id",
            "entry_type",
            "amount",
            "balance_after",
            "description",
            "recorded_by",
            "created_at",
        ],
        trust
            .iter()
            .map(|e| {
                vec![
                    e.id.to_string(),
                    e.entry_type.as_str().to_string(),
                    e.amount.to_string(),
                    e.balance_after.to_string(),
                    e.description.clone(),
                    e.recorded_by.clone(),
                    e.created_at.to_rfc3339(),
                ]
            })
            .collect(),
    );
    let trust_csv = redact_if_needed(&trust_csv, options.redacted, &detector)?;
    let trust_path = format!("{}/trust_ledger.csv", out_dir);
    workspace
        .write(&trust_path, &trust_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(trust_path.clone());

    let audit_csv = build_csv(
        &["id", "event_type", "actor", "severity", "created_at"],
        audits
            .iter()
            .map(|a| {
                vec![
                    a.id.to_string(),
                    a.event_type.clone(),
                    a.actor.clone(),
                    a.severity.as_str().to_string(),
                    a.created_at.to_rfc3339(),
                ]
            })
            .collect(),
    );
    let audit_csv = redact_if_needed(&audit_csv, options.redacted, &detector)?;
    let audit_path = format!("{}/audit_events.csv", out_dir);
    workspace
        .write(&audit_path, &audit_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(audit_path.clone());

    let brief = build_matter_brief(
        &matter,
        metadata.as_ref(),
        &summary,
        &deadlines,
        &tasks,
        invoices.len(),
        trust.last().map(|row| row.balance_after.to_string()),
        options.redacted,
    );
    let brief = redact_if_needed(&brief, options.redacted, &detector)?;
    let brief_path = format!("{}/matter_brief.md", out_dir);
    workspace
        .write(&brief_path, &brief)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(brief_path.clone());

    let sources_csv = build_csv(&["artifact", "record_id", "source"], {
        if sources_rows.is_empty() {
            vec![vec![
                "matter_brief".to_string(),
                matter_id.clone(),
                "derived".to_string(),
            ]]
        } else {
            sources_rows
        }
    });
    let sources_path = format!("{}/sources_index.csv", out_dir);
    workspace
        .write(&sources_path, &sources_csv)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    files.push(sources_path);

    let warning = if options.redacted {
        None
    } else {
        Some(
            "Unredacted retrieval export generated; review before any external sharing."
                .to_string(),
        )
    };

    Ok(MatterRetrievalExportResult {
        matter_id,
        output_dir: out_dir,
        redacted: options.redacted,
        files,
        warning,
    })
}

pub fn default_backup_filename() -> String {
    let ts = Utc::now().format("%Y%m%d-%H%M%S").to_string();
    format!("clawyer-backup-{ts}.clawyerbak")
}

pub fn backups_dir() -> Result<PathBuf, BackupError> {
    let home = dirs::home_dir()
        .ok_or_else(|| BackupError::Config("could not determine home directory".to_string()))?;
    Ok(home.join(".clawyer").join("backups"))
}

pub fn backup_path_for_id(id: &str) -> Result<PathBuf, BackupError> {
    let sanitized: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    if sanitized.is_empty() {
        return Err(BackupError::Validation("invalid backup id".to_string()));
    }
    Ok(backups_dir()?.join(format!("{sanitized}.clawyerbak")))
}

async fn read_optional_matter_metadata(
    workspace: &Workspace,
    matter_root: &str,
    matter_id: &str,
) -> Option<crate::legal::matter::MatterMetadata> {
    read_matter_metadata_for_root(workspace, matter_root, matter_id)
        .await
        .ok()
}

fn normalize_restore_path(path: &str) -> Option<String> {
    let mut parts = Vec::new();
    let p = Path::new(path.trim());
    if p.as_os_str().is_empty() || p.is_absolute() {
        return None;
    }

    for comp in p.components() {
        match comp {
            Component::Normal(seg) => parts.push(seg.to_string_lossy().to_string()),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn is_protected_identity_path(path: &str) -> bool {
    let normalized = path.trim_matches('/');
    PROTECTED_IDENTITY_FILES
        .iter()
        .any(|p| normalized.eq_ignore_ascii_case(p.trim_matches('/')))
}

fn normalize_matter_root_path(path: &str) -> Option<String> {
    normalize_restore_path(path)
}

fn is_path_within_root(path: &str, root: &str) -> bool {
    path == root || path.starts_with(&format!("{root}/"))
}

fn is_allowed_restore_path(path: &str, matter_root: &str) -> bool {
    if is_protected_identity_path(path) {
        return true;
    }
    if ALLOWED_GLOBAL_RESTORE_FILES
        .iter()
        .any(|allowed| path.eq_ignore_ascii_case(allowed))
    {
        return true;
    }
    let Some(root) = normalize_matter_root_path(matter_root) else {
        return false;
    };
    is_path_within_root(path, &root)
}

async fn collect_snapshot(
    db: &dyn Database,
    workspace: &Workspace,
    user_id: &str,
    options: &BackupCreateOptions,
    warnings: &mut Vec<String>,
) -> Result<BackupSnapshot, BackupError> {
    let settings = db
        .get_all_settings(user_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;

    let mut clients = db
        .list_clients(user_id, None)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    clients.sort_by(|a, b| a.id.cmp(&b.id));

    let mut matters = db
        .list_matters_db(user_id)
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    matters.sort_by(|a, b| a.matter_id.cmp(&b.matter_id));

    let mut tasks = Vec::new();
    let mut notes = Vec::new();
    let mut deadlines = Vec::new();
    let mut documents = Vec::new();
    let mut document_versions = Vec::new();
    let mut templates = Vec::new();
    let mut time_entries = Vec::new();
    let mut expense_entries = Vec::new();
    let mut time_summaries = Vec::new();
    let mut trust_ledger = Vec::new();
    let mut invoices = Vec::new();
    let mut invoice_line_items = Vec::new();

    for matter in &matters {
        let matter_id = &matter.matter_id;

        let mut matter_tasks = db
            .list_matter_tasks(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_tasks.sort_by(|a, b| a.id.cmp(&b.id));
        tasks.extend(matter_tasks);

        let mut matter_notes = db
            .list_matter_notes(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_notes.sort_by(|a, b| a.id.cmp(&b.id));
        notes.extend(matter_notes);

        let mut matter_deadlines = db
            .list_matter_deadlines(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_deadlines.sort_by(|a, b| a.id.cmp(&b.id));
        deadlines.extend(matter_deadlines);

        let mut matter_docs = db
            .list_matter_documents_db(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_docs.sort_by(|a, b| a.id.cmp(&b.id));
        for doc in &matter_docs {
            let mut versions = db
                .list_document_versions(user_id, doc.id)
                .await
                .map_err(|e| BackupError::Io(e.to_string()))?;
            versions.sort_by(|a, b| a.id.cmp(&b.id));
            document_versions.extend(versions);
        }
        documents.extend(matter_docs);

        let mut matter_templates = db
            .list_document_templates(user_id, Some(matter_id))
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_templates.sort_by(|a, b| a.id.cmp(&b.id));
        templates.extend(matter_templates);

        let mut matter_time_entries = db
            .list_time_entries(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_time_entries.sort_by(|a, b| a.id.cmp(&b.id));
        time_entries.extend(matter_time_entries);

        let mut matter_expenses = db
            .list_expense_entries(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_expenses.sort_by(|a, b| a.id.cmp(&b.id));
        expense_entries.extend(matter_expenses);

        let summary = db
            .matter_time_summary(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        time_summaries.push(MatterTimeSummaryRecord {
            matter_id: matter_id.clone(),
            summary,
        });

        let mut ledger = db
            .list_trust_ledger_entries(user_id, matter_id)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        ledger.sort_by(|a, b| a.id.cmp(&b.id));
        trust_ledger.extend(ledger);

        let mut matter_invoices = db
            .list_invoices(user_id, Some(matter_id))
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        matter_invoices.sort_by(|a, b| a.id.cmp(&b.id));
        for invoice in &matter_invoices {
            let mut line_items = db
                .list_invoice_line_items(user_id, invoice.id)
                .await
                .map_err(|e| BackupError::Io(e.to_string()))?;
            line_items.sort_by(|a, b| a.id.cmp(&b.id));
            invoice_line_items.extend(line_items);
        }
        invoices.extend(matter_invoices);
    }

    let mut audit_events = Vec::new();
    let audit_query = AuditEventQuery::default();
    let mut offset = 0usize;
    while audit_events.len() < BACKUP_AUDIT_MAX_ROWS {
        let batch = db
            .list_audit_events(user_id, &audit_query, 200, offset)
            .await
            .map_err(|e| BackupError::Io(e.to_string()))?;
        if batch.is_empty() {
            break;
        }
        offset += batch.len();
        audit_events.extend(batch);
    }
    if audit_events.len() >= BACKUP_AUDIT_MAX_ROWS {
        warnings.push(format!(
            "audit events capped at {} rows for backup size safety",
            BACKUP_AUDIT_MAX_ROWS
        ));
    }

    let conflict_graph_summary = match workspace.read("conflicts.json").await {
        Ok(doc) => serde_json::from_str::<serde_json::Value>(&doc.content).unwrap_or_else(|_| {
            warnings.push(
                "conflicts.json exists but is not valid JSON; stored as raw text summary"
                    .to_string(),
            );
            serde_json::json!({"raw": doc.content})
        }),
        Err(WorkspaceError::DocumentNotFound { .. }) => serde_json::json!({"present": false}),
        Err(err) => {
            warnings.push(format!("failed to read conflicts.json: {}", err));
            serde_json::json!({"present": false, "error": err.to_string()})
        }
    };

    let mut all_paths = workspace
        .list_all()
        .await
        .map_err(|e| BackupError::Io(e.to_string()))?;
    all_paths.sort();

    let mut files = Vec::new();
    let mut total_original_bytes = 0usize;
    let mut total_stored_bytes = 0usize;
    let mut truncated_files = 0usize;
    let mut skipped_files = 0usize;

    for path in all_paths {
        let doc = match workspace.read_stored(&path).await {
            Ok(doc) => doc,
            Err(WorkspaceError::DocumentNotFound { .. }) => continue,
            Err(err) => {
                warnings.push(format!("failed to read workspace file '{}': {}", path, err));
                continue;
            }
        };

        let original = doc.content.as_bytes();
        let original_bytes = original.len();
        total_original_bytes = total_original_bytes.saturating_add(original_bytes);

        if total_stored_bytes >= BACKUP_MAX_TOTAL_FILE_BYTES {
            skipped_files += 1;
            files.push(WorkspaceFileSnapshot {
                path: path.clone(),
                memory_document_id: Some(doc.id.to_string()),
                content: String::new(),
                original_bytes,
                stored_bytes: 0,
                truncated: true,
                skipped: true,
                sha256: String::new(),
            });
            continue;
        }

        let remaining = BACKUP_MAX_TOTAL_FILE_BYTES - total_stored_bytes;
        let cap = BACKUP_MAX_FILE_BYTES.min(remaining);

        let (stored_content, truncated, stored_bytes) = if original_bytes > cap {
            truncated_files += 1;
            let mut end = cap;
            while end > 0 && !doc.content.is_char_boundary(end) {
                end -= 1;
            }
            let safe = if end == 0 {
                String::new()
            } else {
                doc.content[..end].to_string()
            };
            let bytes = safe.len();
            (safe, true, bytes)
        } else {
            (doc.content.clone(), false, original_bytes)
        };

        total_stored_bytes = total_stored_bytes.saturating_add(stored_bytes);

        files.push(WorkspaceFileSnapshot {
            path,
            memory_document_id: Some(doc.id.to_string()),
            content: stored_content.clone(),
            original_bytes,
            stored_bytes,
            truncated,
            skipped: false,
            sha256: sha256_hex(stored_content.as_bytes()),
        });
    }

    if truncated_files > 0 {
        warnings.push(format!(
            "{} workspace files were truncated due to size limits",
            truncated_files
        ));
    }
    if skipped_files > 0 {
        warnings.push(format!(
            "{} workspace files were skipped due to total backup size limit",
            skipped_files
        ));
    }

    let mut ai_packets = Vec::new();
    if options.include_ai_packets {
        for matter in &matters {
            let metadata =
                read_optional_matter_metadata(workspace, &options.matter_root, &matter.matter_id)
                    .await;
            let brief = build_matter_brief(
                matter,
                metadata.as_ref(),
                &time_summaries
                    .iter()
                    .find(|s| s.matter_id == matter.matter_id)
                    .map(|s| s.summary.clone())
                    .unwrap_or(MatterTimeSummary {
                        total_hours: rust_decimal::Decimal::ZERO,
                        billable_hours: rust_decimal::Decimal::ZERO,
                        unbilled_hours: rust_decimal::Decimal::ZERO,
                        total_expenses: rust_decimal::Decimal::ZERO,
                        billable_expenses: rust_decimal::Decimal::ZERO,
                        unbilled_expenses: rust_decimal::Decimal::ZERO,
                    }),
                &[],
                &[],
                0,
                None,
                true,
            );
            ai_packets.push(AiPacketPreview {
                matter_id: matter.matter_id.clone(),
                brief_markdown: brief,
            });
        }
    }

    Ok(BackupSnapshot {
        created_at: Utc::now().to_rfc3339(),
        user_id: user_id.to_string(),
        settings,
        legal: LegalBackupSnapshot {
            clients,
            matters,
            tasks,
            notes,
            deadlines,
            documents,
            document_versions,
            templates,
            time_entries,
            expense_entries,
            time_summaries,
            trust_ledger,
            invoices,
            invoice_line_items,
            audit_events,
            conflict_graph_summary,
        },
        workspace: WorkspaceBackupSnapshot {
            files,
            total_original_bytes,
            total_stored_bytes,
            truncated_files,
            skipped_files,
        },
        ai_packets,
    })
}

fn build_manifest(
    snapshot: &BackupSnapshot,
    encrypted: bool,
    warnings: &[String],
) -> Result<BackupManifest, BackupError> {
    let mut checksums = BTreeMap::new();
    checksums.insert("settings".to_string(), checksum_json(&snapshot.settings)?);
    checksums.insert("legal".to_string(), checksum_json(&snapshot.legal)?);
    checksums.insert("workspace".to_string(), checksum_json(&snapshot.workspace)?);
    checksums.insert(
        "ai_packets".to_string(),
        checksum_json(&snapshot.ai_packets)?,
    );

    Ok(BackupManifest {
        format_version: BACKUP_FORMAT_VERSION,
        schema_version: BACKUP_SCHEMA_VERSION,
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        created_at: snapshot.created_at.clone(),
        user_id: snapshot.user_id.clone(),
        encrypted,
        hash_algorithm: "sha256".to_string(),
        section_checksums: checksums,
        notes: warnings.to_vec(),
    })
}

fn build_archive_bytes(
    manifest: &BackupManifest,
    snapshot: &BackupSnapshot,
) -> Result<Vec<u8>, BackupError> {
    let mut buf = Vec::new();
    {
        let encoder = GzEncoder::new(&mut buf, Compression::default());
        let mut tar = Builder::new(encoder);

        let manifest_json = serde_json::to_vec_pretty(manifest)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;
        append_tar_entry(&mut tar, "manifest.json", &manifest_json)?;

        let snapshot_json = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| BackupError::Serialization(e.to_string()))?;
        append_tar_entry(&mut tar, "snapshot.json", &snapshot_json)?;

        tar.finish().map_err(|e| BackupError::Io(e.to_string()))?;
    }
    Ok(buf)
}

fn append_tar_entry<W: std::io::Write>(
    tar: &mut Builder<W>,
    path: &str,
    bytes: &[u8],
) -> Result<(), BackupError> {
    let mut header = Header::new_gnu();
    header
        .set_path(path)
        .map_err(|e| BackupError::Io(e.to_string()))?;
    header.set_size(bytes.len() as u64);
    header.set_mode(0o600);
    header.set_cksum();
    tar.append(&header, bytes)
        .map_err(|e| BackupError::Io(e.to_string()))
}

fn decode_and_validate_backup(
    envelope_bytes: &[u8],
    master_key: &SecretString,
) -> Result<(BackupManifest, BackupSnapshot, Vec<String>), BackupError> {
    let envelope: BackupEnvelope = serde_json::from_slice(envelope_bytes)
        .map_err(|e| BackupError::Serialization(e.to_string()))?;

    if envelope.format != BACKUP_FORMAT {
        return Err(BackupError::Validation(
            "unsupported backup format".to_string(),
        ));
    }
    if envelope.format_version != BACKUP_FORMAT_VERSION {
        return Err(BackupError::Validation(format!(
            "unsupported backup format_version {}",
            envelope.format_version
        )));
    }

    let ciphertext = BASE64
        .decode(&envelope.ciphertext_b64)
        .map_err(|e| BackupError::Serialization(e.to_string()))?;
    if sha256_hex(&ciphertext) != envelope.encryption.ciphertext_sha256 {
        return Err(BackupError::Validation(
            "ciphertext checksum mismatch".to_string(),
        ));
    }

    let salt = BASE64
        .decode(&envelope.encryption.salt_b64)
        .map_err(|e| BackupError::Serialization(e.to_string()))?;

    let crypto = crate::secrets::SecretsCrypto::new(master_key.clone())
        .map_err(|e| BackupError::Crypto(e.to_string()))?;
    let plaintext = crypto
        .decrypt(&ciphertext, &salt)
        .map_err(|e| BackupError::Crypto(e.to_string()))?;
    let plaintext_b64 = plaintext.expose();
    let plaintext_bytes = BASE64
        .decode(plaintext_b64)
        .map_err(|e| BackupError::Serialization(e.to_string()))?;

    if sha256_hex(&plaintext_bytes) != envelope.encryption.plaintext_sha256 {
        return Err(BackupError::Validation(
            "plaintext checksum mismatch".to_string(),
        ));
    }

    let (manifest, snapshot) = parse_archive(&plaintext_bytes)?;

    let mut warnings = Vec::new();
    validate_manifest_checksums(&manifest, &snapshot, &mut warnings)?;

    Ok((manifest, snapshot, warnings))
}

fn parse_archive(archive_bytes: &[u8]) -> Result<(BackupManifest, BackupSnapshot), BackupError> {
    let decoder = GzDecoder::new(Cursor::new(archive_bytes));
    let mut archive = Archive::new(decoder);

    let mut manifest: Option<BackupManifest> = None;
    let mut snapshot: Option<BackupSnapshot> = None;

    let entries = archive
        .entries()
        .map_err(|e| BackupError::Io(e.to_string()))?;

    for entry in entries {
        let mut entry = entry.map_err(|e| BackupError::Io(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| BackupError::Io(e.to_string()))?
            .to_string_lossy()
            .to_string();

        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| BackupError::Io(e.to_string()))?;

        match path.as_str() {
            "manifest.json" => {
                manifest = Some(
                    serde_json::from_slice(&buf)
                        .map_err(|e| BackupError::Serialization(e.to_string()))?,
                );
            }
            "snapshot.json" => {
                snapshot = Some(
                    serde_json::from_slice(&buf)
                        .map_err(|e| BackupError::Serialization(e.to_string()))?,
                );
            }
            _ => {}
        }
    }

    let manifest =
        manifest.ok_or_else(|| BackupError::Validation("manifest.json missing".to_string()))?;
    let snapshot =
        snapshot.ok_or_else(|| BackupError::Validation("snapshot.json missing".to_string()))?;
    Ok((manifest, snapshot))
}

fn validate_manifest_checksums(
    manifest: &BackupManifest,
    snapshot: &BackupSnapshot,
    warnings: &mut Vec<String>,
) -> Result<(), BackupError> {
    let actual = [
        ("settings", checksum_json(&snapshot.settings)?),
        ("legal", checksum_json(&snapshot.legal)?),
        ("workspace", checksum_json(&snapshot.workspace)?),
        ("ai_packets", checksum_json(&snapshot.ai_packets)?),
    ];

    for (key, value) in actual {
        match manifest.section_checksums.get(key) {
            Some(expected) if expected == &value => {}
            Some(_) => {
                return Err(BackupError::Validation(format!(
                    "section checksum mismatch for '{}'",
                    key
                )));
            }
            None => warnings.push(format!("manifest missing section checksum '{}'", key)),
        }
    }

    Ok(())
}

fn checksum_json<T: Serialize>(value: &T) -> Result<String, BackupError> {
    let bytes = serde_json::to_vec(value).map_err(|e| BackupError::Serialization(e.to_string()))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn set_owner_only_permissions(path: &Path) -> Result<(), BackupError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms).map_err(|e| BackupError::Io(e.to_string()))?;
    }
    Ok(())
}

fn build_csv(headers: &[&str], rows: Vec<Vec<String>>) -> String {
    let mut out = String::new();
    out.push_str(
        &headers
            .iter()
            .map(|h| csv_escape(h))
            .collect::<Vec<_>>()
            .join(","),
    );
    out.push('\n');
    for row in rows {
        out.push_str(
            &row.iter()
                .map(|v| csv_escape(v))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out
}

fn csv_escape(value: &str) -> String {
    let mut normalized = value.to_string();
    if matches!(
        normalized.chars().next(),
        Some('=' | '+' | '-' | '@' | '\t' | '\r')
    ) {
        normalized.insert(0, '\'');
    }
    let needs_quotes = normalized.contains(',')
        || normalized.contains('"')
        || normalized.contains('\n')
        || normalized.contains('\r');
    if needs_quotes {
        let escaped = normalized.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        normalized
    }
}

fn redact_if_needed(
    content: &str,
    redacted: bool,
    detector: &LeakDetector,
) -> Result<String, BackupError> {
    if !redacted {
        return Ok(content.to_string());
    }

    let scan = detector.scan(content);
    if scan.should_block {
        return Err(BackupError::Policy(
            "export content contains blocked-sensitive patterns".to_string(),
        ));
    }
    Ok(scan.redacted_content.unwrap_or_else(|| content.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn build_matter_brief(
    matter: &MatterRecord,
    metadata: Option<&crate::legal::matter::MatterMetadata>,
    summary: &MatterTimeSummary,
    deadlines: &[MatterDeadlineRecord],
    tasks: &[MatterTaskRecord],
    invoice_count: usize,
    trust_balance: Option<String>,
    redacted: bool,
) -> String {
    let mut out = String::new();
    out.push_str("# Matter Retrieval Brief\n\n");
    out.push_str("## Matter Snapshot\n\n");
    out.push_str(&format!("- Matter ID: `{}`\n", matter.matter_id));
    out.push_str(&format!("- Status: `{}`\n", matter.status.as_str()));
    if let Some(meta) = metadata {
        out.push_str(&format!("- Client: {}\n", meta.client));
        out.push_str(&format!("- Confidentiality: {}\n", meta.confidentiality));
        out.push_str(&format!("- Retention: {}\n", meta.retention));
        if !meta.adversaries.is_empty() {
            out.push_str(&format!("- Adversaries: {}\n", meta.adversaries.join(", ")));
        }
    }

    out.push_str("\n## Upcoming Deadlines\n\n");
    if deadlines.is_empty() {
        out.push_str("- None recorded.\n");
    } else {
        for deadline in deadlines.iter().take(10) {
            out.push_str(&format!(
                "- {} ({})\n",
                deadline.title,
                deadline.due_at.date_naive()
            ));
        }
    }

    out.push_str("\n## Open Tasks\n\n");
    let open_tasks: Vec<&MatterTaskRecord> = tasks
        .iter()
        .filter(|t| t.status != crate::db::MatterTaskStatus::Done)
        .collect();
    if open_tasks.is_empty() {
        out.push_str("- No open tasks.\n");
    } else {
        for task in open_tasks.iter().take(10) {
            out.push_str(&format!("- {} [{}]\n", task.title, task.status.as_str()));
        }
    }

    out.push_str("\n## Financial Snapshot\n\n");
    out.push_str(&format!("- Invoices: {}\n", invoice_count));
    out.push_str(&format!("- Total hours: {}\n", summary.total_hours));
    out.push_str(&format!("- Billable hours: {}\n", summary.billable_hours));
    out.push_str(&format!("- Total expenses: {}\n", summary.total_expenses));
    if let Some(balance) = trust_balance {
        out.push_str(&format!("- Trust balance: {}\n", balance));
    }

    out.push_str("\n## Risk / Uncertainty\n\n");
    out.push_str("- Citation and factual truth are not auto-verified by this export.\n");
    out.push_str(
        "- Validate deadlines and filing constraints before relying on generated summaries.\n",
    );

    out.push_str("\n## Provenance\n\n");
    out.push_str("- Sources: matter DB rows + matter workspace files under exports/retrieval.\n");
    out.push_str("- If evidence is missing for a claim, treat as insufficient evidence.\n");

    if redacted {
        out.push_str("\n_Redaction mode: enabled by default._\n");
    } else {
        out.push_str("\n_Redaction mode: disabled (unredacted export)._\n");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[test]
    fn backup_manifest_checksum_is_deterministic() {
        let snapshot = BackupSnapshot {
            created_at: "2026-03-01T00:00:00Z".to_string(),
            user_id: "u".to_string(),
            settings: HashMap::from([(String::from("a"), serde_json::json!(1))]),
            legal: LegalBackupSnapshot {
                clients: vec![],
                matters: vec![],
                tasks: vec![],
                notes: vec![],
                deadlines: vec![],
                documents: vec![],
                document_versions: vec![],
                templates: vec![],
                time_entries: vec![],
                expense_entries: vec![],
                time_summaries: vec![],
                trust_ledger: vec![],
                invoices: vec![],
                invoice_line_items: vec![],
                audit_events: vec![],
                conflict_graph_summary: serde_json::json!({"present": false}),
            },
            workspace: WorkspaceBackupSnapshot {
                files: vec![],
                total_original_bytes: 0,
                total_stored_bytes: 0,
                truncated_files: 0,
                skipped_files: 0,
            },
            ai_packets: vec![],
        };

        let m1 = build_manifest(&snapshot, true, &[]).expect("manifest");
        let m2 = build_manifest(&snapshot, true, &[]).expect("manifest");
        assert_eq!(m1.section_checksums, m2.section_checksums);
    }

    #[test]
    fn backup_encrypt_decrypt_roundtrip() {
        let key = SecretString::new("0123456789abcdef0123456789abcdef".to_string().into());
        let snapshot = BackupSnapshot {
            created_at: "2026-03-01T00:00:00Z".to_string(),
            user_id: "u".to_string(),
            settings: HashMap::new(),
            legal: LegalBackupSnapshot {
                clients: vec![],
                matters: vec![],
                tasks: vec![],
                notes: vec![],
                deadlines: vec![],
                documents: vec![],
                document_versions: vec![],
                templates: vec![],
                time_entries: vec![],
                expense_entries: vec![],
                time_summaries: vec![],
                trust_ledger: vec![],
                invoices: vec![],
                invoice_line_items: vec![],
                audit_events: vec![],
                conflict_graph_summary: serde_json::json!({}),
            },
            workspace: WorkspaceBackupSnapshot {
                files: vec![],
                total_original_bytes: 0,
                total_stored_bytes: 0,
                truncated_files: 0,
                skipped_files: 0,
            },
            ai_packets: vec![],
        };
        let manifest = build_manifest(&snapshot, true, &[]).expect("manifest");
        let archive = build_archive_bytes(&manifest, &snapshot).expect("archive");
        let archive_b64 = BASE64.encode(&archive);
        let crypto = crate::secrets::SecretsCrypto::new(key.clone()).expect("crypto");
        let (ciphertext, salt) = crypto.encrypt(archive_b64.as_bytes()).expect("encrypt");
        let plaintext = crypto.decrypt(&ciphertext, &salt).expect("decrypt");
        let decoded = BASE64.decode(plaintext.expose()).expect("b64 decode");
        assert_eq!(decoded, archive);
    }

    #[test]
    fn legacy_snapshot_without_document_versions_is_still_valid() {
        #[derive(Serialize)]
        struct LegacyLegalBackupSnapshot {
            clients: Vec<ClientRecord>,
            matters: Vec<MatterRecord>,
            tasks: Vec<MatterTaskRecord>,
            notes: Vec<MatterNoteRecord>,
            deadlines: Vec<MatterDeadlineRecord>,
            documents: Vec<MatterDocumentRecord>,
            templates: Vec<DocumentTemplateRecord>,
            time_entries: Vec<TimeEntryRecord>,
            expense_entries: Vec<ExpenseEntryRecord>,
            time_summaries: Vec<MatterTimeSummaryRecord>,
            trust_ledger: Vec<TrustLedgerEntryRecord>,
            invoices: Vec<InvoiceRecord>,
            invoice_line_items: Vec<InvoiceLineItemRecord>,
            audit_events: Vec<AuditEventRecord>,
            conflict_graph_summary: serde_json::Value,
        }

        #[derive(Serialize)]
        struct LegacyBackupSnapshot {
            created_at: String,
            user_id: String,
            settings: HashMap<String, serde_json::Value>,
            legal: LegacyLegalBackupSnapshot,
            workspace: WorkspaceBackupSnapshot,
            ai_packets: Vec<AiPacketPreview>,
        }

        let legacy = LegacyBackupSnapshot {
            created_at: "2026-03-01T00:00:00Z".to_string(),
            user_id: "u".to_string(),
            settings: HashMap::from([(String::from("a"), serde_json::json!(1))]),
            legal: LegacyLegalBackupSnapshot {
                clients: vec![],
                matters: vec![],
                tasks: vec![],
                notes: vec![],
                deadlines: vec![],
                documents: vec![],
                templates: vec![],
                time_entries: vec![],
                expense_entries: vec![],
                time_summaries: vec![],
                trust_ledger: vec![],
                invoices: vec![],
                invoice_line_items: vec![],
                audit_events: vec![],
                conflict_graph_summary: serde_json::json!({"present": false}),
            },
            workspace: WorkspaceBackupSnapshot {
                files: vec![],
                total_original_bytes: 0,
                total_stored_bytes: 0,
                truncated_files: 0,
                skipped_files: 0,
            },
            ai_packets: vec![],
        };

        let legacy_json = serde_json::to_vec(&legacy).expect("serialize legacy snapshot");
        let snapshot: BackupSnapshot =
            serde_json::from_slice(&legacy_json).expect("deserialize legacy snapshot");
        assert!(
            snapshot.legal.document_versions.is_empty(),
            "legacy snapshots should default missing document_versions to empty"
        );

        let mut checksums = BTreeMap::new();
        checksums.insert(
            "settings".to_string(),
            checksum_json(&legacy.settings).expect("settings checksum"),
        );
        checksums.insert(
            "legal".to_string(),
            checksum_json(&legacy.legal).expect("legacy legal checksum"),
        );
        checksums.insert(
            "workspace".to_string(),
            checksum_json(&legacy.workspace).expect("workspace checksum"),
        );
        checksums.insert(
            "ai_packets".to_string(),
            checksum_json(&legacy.ai_packets).expect("ai packets checksum"),
        );

        let manifest = BackupManifest {
            format_version: BACKUP_FORMAT_VERSION,
            schema_version: BACKUP_SCHEMA_VERSION,
            app_version: "test".to_string(),
            created_at: legacy.created_at,
            user_id: legacy.user_id,
            encrypted: true,
            hash_algorithm: "sha256".to_string(),
            section_checksums: checksums,
            notes: vec![],
        };

        let mut warnings = Vec::new();
        validate_manifest_checksums(&manifest, &snapshot, &mut warnings)
            .expect("legacy checksum should validate with defaulted document_versions");
    }

    #[test]
    fn csv_escape_quotes_and_commas() {
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn csv_escape_prevents_formula_injection() {
        assert_eq!(csv_escape("=1+1"), "'=1+1");
        assert_eq!(csv_escape("+SUM(A1:A2)"), "'+SUM(A1:A2)");
        assert_eq!(csv_escape("-10"), "'-10");
        assert_eq!(csv_escape("@cmd"), "'@cmd");
    }

    #[test]
    fn restore_scope_enforces_matter_root() {
        assert!(is_allowed_restore_path("matters/demo/note.md", "matters"));
        assert!(is_allowed_restore_path("conflicts.json", "matters"));
        assert!(is_allowed_restore_path(paths::IDENTITY, "matters"));
        assert!(!is_allowed_restore_path("notes/todo.md", "matters"));
        assert!(!is_allowed_restore_path(
            "matters/demo/note.md",
            "client/matters"
        ));
        assert!(is_allowed_restore_path(
            "client/matters/demo/note.md",
            "client/matters"
        ));
    }
}
