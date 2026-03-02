# cLawyer AI Retrieval Exports

cLawyer can generate matter-scoped retrieval packets for downstream AI workflows.

## Default behavior

- Output is matter-local: `matters/<matter_id>/exports/retrieval/<timestamp>/`
- Redaction is enabled by default.
- Export includes CSV artifacts, a plain-English brief, and source index.

## Generated files

- `matter_overview.csv`
- `parties.csv`
- `deadlines.csv`
- `tasks.csv`
- `notes.csv`
- `documents.csv`
- `time_entries.csv`
- `expenses.csv`
- `trust_ledger.csv`
- `audit_events.csv`
- `matter_brief.md`
- `sources_index.csv`

## CLI

```bash
clawyer backup export-matter --matter <matter_id> --output-dir /tmp/retrieval
```

Unredacted export:

```bash
clawyer backup export-matter --matter <matter_id> --output-dir /tmp/retrieval --unredacted
```

## Web API

`POST /api/matters/{id}/exports/retrieval-packet`

Request body:

```json
{
  "unredacted": false
}
```

Response includes:

- `matter_id`
- `output_dir`
- `redacted`
- `files[]`
- optional `warning`

## Governance guidance

- Prefer redacted packets for model ingestion.
- Treat unredacted exports as sensitive and short-lived.
- Record approval context when choosing unredacted mode.
- Validate high-impact facts and legal authorities before use.
