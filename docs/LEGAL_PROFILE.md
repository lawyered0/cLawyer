# cLawyer Legal Profile

`cLawyer` ships with legal-mode enabled by default for U.S.-general workflows.

## Defaults

- `legal.enabled = true`
- `legal.jurisdiction = "us-general"`
- `legal.hardening = "max_lockdown"`
- `legal.require_matter_context = true`
- `legal.citation_required = true`
- `legal.matter_root = "matters"`
- `legal.conflict_file_fallback_enabled = true`
- `legal.conflict_reindex_on_startup = true`
- `legal.network.deny_by_default = true`
- `legal.audit.enabled = true`
- `legal.audit.path = "logs/legal_audit.jsonl"`
- `legal.audit.hash_chain = true`

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
7. Generated legal text is still scanned for leakage and citation-format markers, but filing readiness now depends on persisted citation verification results.
8. Audit events are appended to JSONL with hash-chain links.

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
  - hard-blocks export when any DB-backed `pleading` or `filing` document for the matter is not `ready_to_file`.
- `GET /api/matters/{id}/documents`
  - DB-backed matter document index linked to `memory_documents` (workspace backfill for legacy files).
- `GET /api/matters/{id}/templates`
  - DB-backed template list (workspace templates are backfilled for compatibility).
- `POST /api/documents/generate`
  - renders a DB template with matter/client context, writes a draft document, links it to the matter, and records a version row.
  - generated matter documents start in `draft` readiness.
- `POST /api/matters/{id}/citations/verify`
  - extracts reporter-style citations from a matter document, verifies them through the provider abstraction, persists the verification run/results, and updates document readiness.
  - CourtListener is the first provider in this phase; attorney waivers are persisted with actor, reason, and timestamp.
- `GET /api/documents/{id}/citations`
  - returns extracted citations, the latest verification run, stored results, and the current document readiness state.
- `POST /api/documents/{id}/ready`
  - marks a document `ready_to_file` only when the latest verification run matches the current document hash and every extracted citation is verified or waived.
- `POST /api/matters/conflict-check`
  - runs intake-time conflict review against the DB-backed party graph and returns structured `ConflictHit` rows.
- `POST /api/matters`
  - server-hard-gated on conflict hits: `clear`/`waived` can proceed, `declined` blocks creation and records clearance.
- `POST /api/matters/active`
  - conflict hard gate for activation: when unresolved cross-matter hits are present, setting active matter is blocked until a clearance decision is recorded.
  - request supports optional `conflict_decision` (`clear|waived|declined`) + `conflict_note` (required for `waived`/`declined`) to persist review decisions.
- `POST /api/matters/conflicts/reindex`
  - rebuilds DB conflict graph from `matters/*/matter.yaml` plus workspace `conflicts.json`.
- `GET /api/matters/{id}/parties`
  - lists structured DB-backed matter parties, bootstrapping from `matter.yaml` when the matter has not been reviewed yet.
- `POST /api/matters/{id}/parties`
  - records manually reviewed parties with role, aliases, notes, and optional open/close timestamps.
- `POST /api/matters/{id}/parties/relationships`
  - records affiliate/principal/opposing-counsel style party relationships used during conflict traversal.
- `GET /api/matters/{id}/conflicts/report`
  - returns a structured conflict report with checked parties, relationship rows, detailed hits, and the latest clearance record.
- `POST /api/matters/{id}/conflicts/clearance`
  - records signed attorney review decisions with reviewer identity, report hash, note, and hit snapshot.
- `GET /api/trust/account`
  - returns the primary deployment trust account and current account-level book balance.
- `PUT /api/trust/account`
  - configures or updates the primary trust account (Phase 1 assumes one primary IOLTA/trust account per deployment).
- `POST /api/trust/statements/import`
  - imports canonical CSV bank statements with columns `date`, `description`, `debit`, `credit`, `balance`, and optional `reference`.
- `POST /api/trust/reconciliations/compute`
  - computes and persists three-way reconciliation between the imported statement balance, account book balance, and summed client/matter trust balances.
- `POST /api/trust/reconciliations/{id}/signoff`
  - signs off a computed reconciliation and produces examiner-readable report content.
- `GET /api/matters/{id}/trust/ledger`
  - returns the matter ledger, account summary, and latest reconciliation status for the primary trust account.
- `GET /api/billing/rates`
  - lists effective-dated billing rate schedules.
- `POST /api/billing/rates`
  - creates a billing rate schedule. Resolution precedence is matter override, then timekeeper default, then explicit per-entry manual rate.
- `PATCH /api/billing/rates/{id}`
  - updates an existing billing rate schedule without rewriting historical invoice line-item snapshots.
- `GET /api/invoices/{id}/ledes?format=98b`
  - exports the invoice as LEDES98B with UTBMS task/activity codes and snapshot billing-rate data.
- `POST /api/matters/{id}/exports/retrieval-packet`
  - generates matter-local CSV + plain-English retrieval artifacts for AI workflows under `matters/<id>/exports/retrieval/<timestamp>/`.

## Backup and Recovery APIs

- `POST /api/backups/create`
  - creates encrypted full-system backup bundles.
- `POST /api/backups/verify`
  - validates backup integrity and decryptability.
- `POST /api/backups/restore`
  - multipart upload restore endpoint; dry-run by default, apply mode explicit.
- `GET /api/backups/{id}/download`
  - downloads a stored backup artifact by ID.

CLI parity:

- `clawyer backup create`
- `clawyer backup verify`
- `clawyer backup restore`
- `clawyer backup export-matter`

## Conflict Check Limits

- Intake conflict checks use a DB-backed party graph (`parties`, `party_aliases`, `matter_parties`) with exact+alias+fuzzy matching.
- Chat conflict checks are DB-first. Fallback to workspace-global `conflicts.json` is controlled by `legal.conflict_file_fallback_enabled`.
- Startup can auto-reindex DB conflict graph from workspace (`legal.conflict_reindex_on_startup = true`).
- Existing `POST /api/matters/conflicts/check` remains for compatibility and now uses the same DB-first path plus fallback.
- Structured matter conflict review now persists manual parties, aliases, relationships, and signed clearance records; workspace data is bootstrap input, not the authoritative review record.
- Matching remains normalized/boundary-aware and heuristic; short aliases are intentionally ignored to reduce false positives.

## Deadline Reminder Notes

- Deadline reminders are stored as one-shot routines named `deadline-reminder-{matter_id}-{deadline_id}-{days}`.
- Reminder routines are auto-disabled after a successful/attention run.
- Updating, completing, or deleting a deadline disables obsolete reminder routines and re-syncs current ones.

## Citation Check Limits

- Generated text is still scanned for structured citation formats, but filing readiness is gated on stored citation verification results rather than regex markers alone.
- Phase 1 ships a provider abstraction with CourtListener as the only concrete adapter.
- Attorney waivers are supported and fully audited, but they do not make a document `ready_to_file` until the explicit ready transition is recorded.
- Verification quality still depends on provider coverage and API availability; this phase does not integrate Westlaw or Lexis.

## Trust Accounting Limits

- Phase 1 assumes one primary trust account per deployment.
- Statement import is canonical CSV only; OFX/QFX/PDF parsing is out of scope for this phase.
- Matter/client trust ledgers are filtered views over the account ledger rather than separate ledgers with independent balances.

## Deferred Architecture Items

- Self-repair stuck-job handling is still attempt-count based; time-threshold stuck detection is not implemented yet.
- Per-matter encryption-at-rest for workspace files remains a follow-up phase.

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
- trust reconciliation events (`trust_statement_imported`, `trust_reconciliation_computed`, `trust_reconciliation_signed_off`)
- conflict system events (`conflict_detected`, `conflict_graph_reindexed`)
- citation workflow events (`document_citations_verified`, `document_marked_ready`)

Counters tracked in audit state:

- `blocked_actions`
- `approval_required`
- `redaction_events`
