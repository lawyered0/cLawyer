use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Subcommand;

use crate::config::Config;
use crate::db::Database;
use crate::legal::backup::{
    BackupCreateOptions, BackupRestoreOptions, MatterRetrievalExportOptions, create_backup_file,
    export_matter_retrieval_packet, reencrypt_matter_files, scan_matter_encryption,
    verify_backup_file,
};
use crate::workspace::{LegalContentPolicy, Workspace};

#[derive(Subcommand, Debug, Clone)]
pub enum BackupCommand {
    /// Create an encrypted full-system backup bundle.
    Create {
        /// Output backup file path (.clawyerbak).
        #[arg(long)]
        output: PathBuf,
        /// Include AI packet previews in snapshot metadata.
        #[arg(long, default_value_t = false)]
        include_ai_packets: bool,
    },

    /// Verify backup integrity/decryptability and checksum consistency.
    Verify {
        /// Input backup file path.
        #[arg(long)]
        input: PathBuf,
    },

    /// Restore a backup (dry-run by default; use --apply to mutate state).
    Restore {
        /// Input backup file path.
        #[arg(long)]
        input: PathBuf,
        /// Explicit dry-run mode (default behavior when --apply is omitted).
        #[arg(long, default_value_t = false)]
        dry_run: bool,
        /// Apply restore changes.
        #[arg(long, default_value_t = false)]
        apply: bool,
        /// Strict mode: fail restore when critical replay/integrity checks fail.
        #[arg(long, default_value_t = false)]
        strict: bool,
    },

    /// Export one matter as CSV + brief for downstream AI workflows.
    ExportMatter {
        /// Matter ID to export.
        #[arg(long)]
        matter: String,
        /// Output directory where generated files will be copied.
        #[arg(long)]
        output_dir: PathBuf,
        /// Disable default redaction (requires careful handling).
        #[arg(long, default_value_t = false)]
        unredacted: bool,
    },

    /// Scan matter-root files for encryption scope/integrity status.
    ScanMatterEncryption {
        /// Optional matter ID filter.
        #[arg(long)]
        matter: Option<String>,
    },

    /// Re-encrypt matter files in place with current master key material.
    ReencryptMatterFiles {
        /// Optional matter ID filter.
        #[arg(long)]
        matter: Option<String>,
        /// Preview only (no writes).
        #[arg(long, default_value_t = false)]
        dry_run: bool,
    },
}

pub async fn run_backup_command(cmd: BackupCommand) -> anyhow::Result<()> {
    let config = Config::from_env()
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let master_key = config
        .secrets
        .master_key()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("SECRETS_MASTER_KEY (or keychain-backed master key) is required for backup encryption/decryption"))?;

    let db: Arc<dyn Database> = crate::db::connect_from_config(&config.database)
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))?;

    let mut workspace = Workspace::new_with_db("default", Arc::clone(&db));
    if config.legal.enabled && config.legal.encryption.enabled {
        let crypto = Arc::new(
            crate::secrets::SecretsCrypto::new(master_key.clone())
                .map_err(|e| anyhow::anyhow!(e.to_string()))?,
        );
        workspace = workspace.with_legal_content_policy(LegalContentPolicy::new(
            config.legal.matter_root.clone(),
            config.legal.encryption.exclude_from_search,
            crypto,
        ));
    }
    let user_id = "default";

    match cmd {
        BackupCommand::Create {
            output,
            include_ai_packets,
        } => {
            let result = create_backup_file(
                db.as_ref(),
                &workspace,
                user_id,
                &output,
                &master_key,
                &BackupCreateOptions {
                    include_ai_packets,
                    matter_root: config.legal.matter_root.clone(),
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            println!("Backup created:");
            println!("  id: {}", result.artifact.id);
            println!("  path: {}", result.artifact.path);
            println!("  size: {} bytes", result.artifact.size_bytes);
            println!("  sha256: {}", result.artifact.plaintext_sha256);
            if !result.warnings.is_empty() {
                println!("Warnings:");
                for warning in result.warnings {
                    println!("  - {}", warning);
                }
            }
        }
        BackupCommand::Verify { input } => {
            let report = verify_backup_file(&input, &master_key)
                .await
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            println!(
                "Backup verification: {}",
                if report.valid { "PASS" } else { "FAIL" }
            );
            println!("  created_at: {}", report.manifest.created_at);
            println!("  user_id: {}", report.manifest.user_id);
            println!("  app_version: {}", report.manifest.app_version);
            if !report.warnings.is_empty() {
                println!("Warnings:");
                for warning in report.warnings {
                    println!("  - {}", warning);
                }
            }
        }
        BackupCommand::Restore {
            input,
            dry_run,
            apply,
            strict,
        } => {
            let effective_apply = apply && !dry_run;
            let report = crate::legal::backup::restore_backup_file(
                db.as_ref(),
                &workspace,
                user_id,
                &input,
                &master_key,
                &BackupRestoreOptions {
                    apply: effective_apply,
                    strict,
                    protect_identity_files: true,
                    matter_root: config.legal.matter_root.clone(),
                },
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            println!(
                "Backup restore: {}",
                if report.applied { "APPLIED" } else { "DRY-RUN" }
            );
            println!("  strict: {}", report.strict);
            println!("  restored_settings: {}", report.restored_settings);
            println!(
                "  restored_workspace_files: {}",
                report.restored_workspace_files
            );
            println!(
                "  skipped_workspace_files: {}",
                report.skipped_workspace_files
            );
            if !report.critical_failures.is_empty() {
                println!("Critical failures:");
                for item in &report.critical_failures {
                    println!("  - {}", item);
                }
            }
            println!(
                "Integrity: docs {}/{} missing, versions {}/{} missing, invoice items {}/{} missing, trust mismatches {}/{}",
                report
                    .integrity
                    .checked_documents
                    .saturating_sub(report.integrity.missing_documents),
                report.integrity.checked_documents,
                report
                    .integrity
                    .checked_document_versions
                    .saturating_sub(report.integrity.missing_document_versions),
                report.integrity.checked_document_versions,
                report
                    .integrity
                    .checked_invoice_line_items
                    .saturating_sub(report.integrity.missing_invoice_line_items),
                report.integrity.checked_invoice_line_items,
                report.integrity.mismatched_trust_balances,
                report.integrity.checked_trust_balances,
            );
            if !report.warnings.is_empty() {
                println!("Warnings:");
                for warning in report.warnings {
                    println!("  - {}", warning);
                }
            }
        }
        BackupCommand::ExportMatter {
            matter,
            output_dir,
            unredacted,
        } => {
            let result = export_matter_retrieval_packet(
                db.as_ref(),
                &workspace,
                user_id,
                &matter,
                &MatterRetrievalExportOptions {
                    redacted: !unredacted,
                    matter_root: config.legal.matter_root.clone(),
                },
                Some(&config.legal.redaction),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            tokio::fs::create_dir_all(&output_dir)
                .await
                .with_context(|| format!("failed to create output dir {}", output_dir.display()))?;

            for ws_path in &result.files {
                let doc = workspace
                    .read(ws_path)
                    .await
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let file_name = PathBuf::from(ws_path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("export.txt")
                    .to_string();
                let dest = output_dir.join(file_name);
                tokio::fs::write(&dest, doc.content)
                    .await
                    .with_context(|| format!("failed to write {}", dest.display()))?;
            }

            println!("Matter retrieval export created:");
            println!("  matter_id: {}", result.matter_id);
            println!("  workspace_dir: {}", result.output_dir);
            println!("  copied_to: {}", output_dir.display());
            println!("  redacted: {}", result.redacted);
            if let Some(warn) = result.warning {
                println!("  warning: {}", warn);
            }
        }
        BackupCommand::ScanMatterEncryption { matter } => {
            let report = scan_matter_encryption(
                &workspace,
                &config.legal.matter_root,
                &master_key,
                matter.as_deref(),
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            println!("Matter encryption scan:");
            println!("  scanned_files: {}", report.scanned_files);
            println!("  encrypted_files: {}", report.encrypted_files);
            println!("  plaintext_files: {}", report.plaintext_files);
            println!("  invalid_envelopes: {}", report.invalid_envelopes);
            if !report.warnings.is_empty() {
                println!("Warnings:");
                for warning in report.warnings {
                    println!("  - {}", warning);
                }
            }
        }
        BackupCommand::ReencryptMatterFiles { matter, dry_run } => {
            let report = reencrypt_matter_files(
                &workspace,
                &config.legal.matter_root,
                &master_key,
                matter.as_deref(),
                !dry_run,
            )
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;

            println!(
                "Matter file re-encryption: {}",
                if dry_run { "DRY-RUN" } else { "APPLIED" }
            );
            println!("  scanned_files: {}", report.scanned_files);
            println!("  encrypted_files: {}", report.encrypted_files);
            println!("  plaintext_files: {}", report.plaintext_files);
            println!("  invalid_envelopes: {}", report.invalid_envelopes);
            println!("  reencrypted_files: {}", report.reencrypted_files);
            if !report.warnings.is_empty() {
                println!("Warnings:");
                for warning in report.warnings {
                    println!("  - {}", warning);
                }
            }
        }
    }

    Ok(())
}
