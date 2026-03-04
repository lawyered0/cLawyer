# cLawyer Backup and Recovery

This guide documents the encrypted backup workflow and restore behavior.

## Security defaults

- Backups are encrypted by default.
- A secrets master key is required (`SECRETS_MASTER_KEY` or keychain-backed key).
- Backup files are written with owner-only permissions (`0600` on Unix).
- Restore is dry-run by default; apply mode is explicit.

## CLI usage

Create backup:

```bash
clawyer backup create --output /path/to/backup.clawyerbak
```

Create backup with AI packet previews:

```bash
clawyer backup create --output /path/to/backup.clawyerbak --include-ai-packets
```

Verify backup:

```bash
clawyer backup verify --input /path/to/backup.clawyerbak
```

Dry-run restore:

```bash
clawyer backup restore --input /path/to/backup.clawyerbak --dry-run
```

Apply restore:

```bash
clawyer backup restore --input /path/to/backup.clawyerbak --apply
```

Apply restore in strict mode (fails when critical replay/integrity checks fail):

```bash
clawyer backup restore --input /path/to/backup.clawyerbak --apply --strict
```

Scan matter encryption health:

```bash
clawyer backup scan-matter-encryption --matter demo
```

Re-encrypt matter files in place (refresh envelope with current master key):

```bash
clawyer backup reencrypt-matter-files --matter demo
```

## Web API usage

- `POST /api/backups/create`
  - body: `{ "include_ai_packets": false }`
- `POST /api/backups/verify`
  - body: `{ "backup_id": "backup-..." }`
- `POST /api/backups/restore`
  - multipart fields:
    - `file` (backup file)
    - `apply` (`true|false`)
    - `strict` (`true|false`)
    - `protect_identity_files` (`true|false`)
- `GET /api/backups/{id}/download`

## Restore behavior

Restore apply replays:

- settings
- workspace files under legal restore scope:
  - configured `legal.matter_root/**`
  - `conflicts.json`
  - protected identity files only when identity-file protection is explicitly disabled
- idempotent legal DB entities:
  - clients
  - matters
  - templates
  - tasks
  - notes
  - deadlines
  - matter documents
  - document versions
  - time entries
  - expense entries
  - trust ledger entries
  - invoices
  - invoice line items
  - audit events

Restore apply returns:

- per-entity restored/skipped counters
- integrity summary (documents, document versions, invoice line items, trust balance checks)
- critical failure list

When strict mode is enabled, restore fails if critical failures are detected.

## Backup schema compatibility

- Current archive schema version: `2`.
- Schema `1` archives remain supported through in-process migration during verify/restore.
- Legacy fields are defaulted safely (`document_versions`, workspace `memory_document_id`) to keep older backups restorable.

## Rotation and storage

- Keep at least 3 recent backups per environment.
- Copy encrypted backups off-device.
- Run verify after backup creation and before restore drills.
- Test restore dry-run on a regular schedule.

## Known limitations

- Citation/source truth validation is out of scope for backup verification.
- Conflict graph data is backed up via conflict summary snapshots and workspace sources.
