# cLawyer Legal Profile

`cLawyer` ships with legal-mode enabled by default for U.S.-general workflows.

## Defaults

- `legal.enabled = true`
- `legal.jurisdiction = "us-general"`
- `legal.hardening = "max_lockdown"`
- `legal.require_matter_context = true`
- `legal.citation_required = true`
- `legal.matter_root = "matters"`
- `legal.conflict_file_fallback_enabled = false`
- `legal.conflict_reindex_on_startup = true`
- `legal.network.deny_by_default = true`
- `legal.audit.enabled = true`
- `legal.audit.path = "logs/legal_audit.jsonl"`
- `legal.audit.hash_chain = true`
- `legal.encryption.enabled = true`
- `legal.encryption.matter_scope_only = true`
- `legal.encryption.exclude_from_search = true`
- `legal.encryption.require_master_key_in_max_lockdown = true`

## CLI Controls

- `--matter <matter_id>`
- `--jurisdiction <code>`
- `--legal-profile <max-lockdown|standard>`
- `--allow-domain <domain>` (repeatable)

## Runtime Flow

1. Request enters preflight.
2. cLawyer checks: active matter (for non-trivial legal requests), conflict list, tool approval policy, and domain allowlist.
3. When active matter is set and metadata is valid, structured `matter.yaml` fields are injected into legal prompt context (`matter_id`, `client`, `confidentiality`, `retention`, `team`, `adversaries`, optional `jurisdiction`, optional `practice_area`, optional `opened_date`) as untrusted data.
4. cLawyer also injects curated matter memory files when present (`facts.md`, `parties.md`, `strategy.md`, `documents.md`) with strict sanitization/truncation.
5. Sensitive tool calls are approval-gated in `max_lockdown`.
6. Memory/file writes are scoped to `matters/<matter_id>/...` when matter context is required.
7. Output is scanned for leakage and structured citation-format markers.
8. Audit events are appended to JSONL with hash-chain links.

## Matter Encryption at Rest

- Files under `legal.matter_root/**` are transparently encrypted at rest when legal encryption is enabled.
- Non-matter workspace files keep normal plaintext behavior.
- Encrypted matter files are excluded from plaintext chunk/embedding indexing by design.
- In `max_lockdown`, startup fails fast when encryption is enabled but no master key is available.
- Legacy plaintext matter files are read-compatible and migrate lazily on next write.

## Matter-Bound Conversations

- Legal conversations are now bound to one `matter_id` in persistence (`conversations.matter_id`).
- If a thread is already bound to matter A, using it under matter B is blocked before LLM/tool execution.
- Legacy conversations with `NULL matter_id` are lazily bound on first legal use with an active matter.
- New threads created from the web gateway are pre-bound when `legal.active_matter` is set.
- Compaction for bound conversations writes to `matters/<matter_id>/sessions/YYYY-MM-DD.md` (falls back to `daily/` when unbound).

This prevents cross-matter context bleed by construction.

## Matter Model

Use:

```text
matters/<matter_id>/matter.yaml
```

Required metadata fields:

- `matter_id`
- `client`
- `confidentiality`
- `retention`

Optional metadata fields:

- `jurisdiction`
- `practice_area`
- `opened_date` (`YYYY-MM-DD`)

Backward compatibility:
- legacy `opened_at` values are accepted and normalized.

If metadata is missing or invalid, legal task execution is blocked with guidance.

## Matter Workflow Scaffold

New matters now include a practical workflow scaffold for day-to-day legal work:

- `workflows/intake_checklist.md`
- `workflows/review_and_filing_checklist.md`
- `deadlines/calendar.md`
- `facts/key_facts.md`
- `research/authority_table.md`
- `discovery/request_tracker.md`
- `communications/contact_log.md`
- expanded templates under `templates/` (memo, chronology, discovery, contract issues, research synthesis)

## Matter Workflow APIs

For web-first firm workflows, matter detail now includes:

- `GET /api/matters/{id}/dashboard`
  - scorecard totals for documents, drafts, templates, checklist completion, and deadline risk.
- `GET /api/matters/{id}/deadlines`
  - DB-backed deadline list (falls back to `deadlines/calendar.md` when DB rows are absent).
- `POST /api/matters/{id}/deadlines`
  - create deadline records with type, due date, optional completion/rule/task linkage, and reminder offsets.
- `PATCH /api/matters/{id}/deadlines/{deadline_id}`
  - update deadline fields and reminder settings.
- `DELETE /api/matters/{id}/deadlines/{deadline_id}`
  - delete a deadline and disable any scheduled reminder routines for it.
- `POST /api/matters/{id}/deadlines/compute`
  - computes deadline previews from bundled court rules without persisting.
- `GET /api/legal/court-rules`
- `GET /api/legal/audit`
  - returns DB-backed, user-scoped legal audit events with filters:
    - `event_type`, `matter_id`, `severity`, `since`, `until`, `limit`, `offset`
  - file audit log remains authoritative; DB records are best-effort mirrors.
  - lists bundled rule metadata (citation, deadline type, offset, court-day behavior).
- `POST /api/matters/{id}/filing-package`
  - writes a matter-local filing package index to `matters/<id>/exports/`.
- `GET /api/matters/{id}/documents`
  - DB-backed matter document index linked to `memory_documents` (workspace backfill for legacy files).
- `GET /api/matters/{id}/templates`
  - DB-backed template list (workspace templates are backfilled for compatibility).
- `POST /api/documents/generate`
  - renders a DB template with matter/client context, writes a draft document, links it to the matter, and records a version row.
- `POST /api/matters/conflict-check`
  - runs intake-time conflict review against the DB-backed party graph and returns structured `ConflictHit` rows.
- `POST /api/matters`
  - server-hard-gated on conflict hits: `clear`/`waived` can proceed, `declined` blocks creation and records clearance.
- `POST /api/matters/conflicts/reindex`
  - rebuilds DB conflict graph from `matters/*/matter.yaml` plus workspace `conflicts.json`.
- `POST /api/matters/{id}/exports/retrieval-packet`
  - generates matter-local CSV + plain-English retrieval artifacts for AI workflows under `matters/<id>/exports/retrieval/<timestamp>/`.

## Backup and Recovery APIs

- `POST /api/backups/create`
  - creates encrypted full-system backup bundles.
- `POST /api/backups/verify`
  - validates backup integrity and decryptability.
- `POST /api/backups/restore`
  - multipart upload restore endpoint; dry-run by default, apply mode explicit.
  - apply mode replays full legal DB entities with idempotent upsert semantics.
- `GET /api/backups/{id}/download`
  - downloads a stored backup artifact by ID.

CLI parity:

- `clawyer backup create`
- `clawyer backup verify`
- `clawyer backup restore`
- `clawyer backup export-matter`

## Conflict Check Limits

- Intake conflict checks use a DB-backed party graph (`parties`, `party_aliases`, `matter_parties`) with exact+alias+fuzzy matching.
- Chat conflict checks are DB-authoritative by default.
- Workspace-global `conflicts.json` fallback is available only when `legal.conflict_file_fallback_enabled = true`.
- Startup can auto-reindex DB conflict graph from workspace (`legal.conflict_reindex_on_startup = true`).
- Existing `POST /api/matters/conflicts/check` remains for compatibility and now uses the same DB-first path plus fallback.
- Matching remains normalized/boundary-aware and heuristic; short aliases are intentionally ignored to reduce false positives.

## Deadline Reminder Notes

- Deadline reminders are stored as one-shot routines named `deadline-reminder-{matter_id}-{deadline_id}-{days}`.
- Reminder routines are auto-disabled after a successful/attention run.
- Updating, completing, or deleting a deadline disables obsolete reminder routines and re-syncs current ones.

## Citation Check Limits

- Citation enforcement checks for structured citation formats in generated text.
- This check does not verify the truth, existence, or legal validity of cited sources.

## Deferred Architecture Items

- Self-repair stuck-job handling is still attempt-count based; time-threshold stuck detection is not implemented yet.
- Conflict checks do not yet traverse `party_relationships` recursively for affiliate/corporate-family logic.
- Per-matter key rotation / re-key workflows are not implemented yet.

## Bundled Legal Skills

Trusted bundled skills:

- `legal-intake`
- `legal-chronology`
- `legal-gmail-inbox-triage`
- `legal-contract-review`
- `legal-litigation-support`
- `legal-research-synthesis`

Each bundled legal skill is expected to include:

- `domain: legal`
- `requires_matter: true`
- `citation_mode: required`

## Audit and Metrics

Audit log events include:

- prompts
- approvals
- blocked operations
- redaction events
- skill activations
- LLM lifecycle events (`llm_call_started`, `llm_call_completed`, `llm_call_failed`)
- tool lifecycle events (`tool_call_started`, `tool_call_completed`)
- explicit approval decision events (`approval_decision`)
- matter lifecycle events (`matter_created`, `matter_closed`)
- billing/trust events (`invoice_finalized`, `payment_recorded`, `trust_deposit`, `trust_withdrawal_rejected`)
- conflict system events (`conflict_detected`, `conflict_graph_reindexed`)

Counters tracked in audit state:

- `blocked_actions`
- `approval_required`
- `redaction_events`
