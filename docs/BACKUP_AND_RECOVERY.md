# cLawyer Backup and Recovery

This guide documents the v1 encrypted backup workflow and restore behavior.

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

## Web API usage

- `POST /api/backups/create`
  - body: `{ "include_ai_packets": false }`
- `POST /api/backups/verify`
  - body: `{ "backup_id": "backup-..." }`
- `POST /api/backups/restore`
  - multipart fields:
    - `file` (backup file)
    - `apply` (`true|false`)
    - `protect_identity_files` (`true|false`)
- `GET /api/backups/{id}/download`

## Restore behavior (v1)

Restore apply replays:

- settings
- workspace files (path safety and protected identity-file guards apply)
- idempotent legal DB entities: clients, matters, templates

Other legal DB entities are preserved in the backup and currently require manual migration/replay.

## Rotation and storage

- Keep at least 3 recent backups per environment.
- Copy encrypted backups off-device.
- Run verify after backup creation and before restore drills.
- Test restore dry-run on a regular schedule.

## Known limitations

- Full automatic restore for every legal table is not implemented in v1.
- Citation/source truth validation is out of scope for backup verification.
- Conflict graph data is backed up via conflict summary snapshots and workspace sources.
