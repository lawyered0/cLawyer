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
3. When active matter is set and metadata is valid, structured `matter.yaml` fields are injected into legal prompt context (`matter_id`, `client`, `confidentiality`, `retention`, `team`, `adversaries`, optional `jurisdiction`, optional `practice_area`, optional `opened_at`) as untrusted data.
4. Sensitive tool calls are approval-gated in `max_lockdown`.
5. Memory/file writes are scoped to `matters/<matter_id>/...` when matter context is required.
6. Output is scanned for leakage and structured citation-format markers.
7. Audit events are appended to JSONL with hash-chain links.

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
- `opened_at` (`YYYY-MM-DD`)

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
  - parsed deadline rows from `deadlines/calendar.md` with overdue flags.
- `POST /api/matters/{id}/filing-package`
  - writes a matter-local filing package index to `matters/<id>/exports/`.
- `POST /api/matters/conflict-check`
  - runs intake-time conflict review against the DB-backed party graph and returns structured `ConflictHit` rows.
- `POST /api/matters`
  - server-hard-gated on conflict hits: `clear`/`waived` can proceed, `declined` blocks creation and records clearance.
- `POST /api/matters/conflicts/reindex`
  - rebuilds DB conflict graph from `matters/*/matter.yaml` plus workspace `conflicts.json`.

## Conflict Check Limits

- Intake conflict checks use a DB-backed party graph (`parties`, `party_aliases`, `matter_parties`) with exact+alias+fuzzy matching.
- Chat conflict checks are DB-first. Fallback to workspace-global `conflicts.json` is controlled by `legal.conflict_file_fallback_enabled`.
- Startup can auto-reindex DB conflict graph from workspace (`legal.conflict_reindex_on_startup = true`).
- Existing `POST /api/matters/conflicts/check` remains for compatibility and now uses the same DB-first path plus fallback.
- Matching remains normalized/boundary-aware and heuristic; short aliases are intentionally ignored to reduce false positives.

## Citation Check Limits

- Citation enforcement checks for structured citation formats in generated text.
- This check does not verify the truth, existence, or legal validity of cited sources.

## Deferred Architecture Items

- Self-repair stuck-job handling is still attempt-count based; time-threshold stuck detection is not implemented yet.
- Conflict checks do not yet traverse `party_relationships` recursively for affiliate/corporate-family logic.

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

Counters tracked in audit state:

- `blocked_actions`
- `approval_required`
- `redaction_events`
