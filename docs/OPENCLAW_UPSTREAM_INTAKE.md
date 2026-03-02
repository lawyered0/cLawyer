# OpenClaw / Upstream Intake Runbook

This runbook defines how cLawyer adopts updates from `openclaw/main` and `upstream/main` without losing cLawyer legal hardening guarantees.

## Goals

1. Pull high-value fixes quickly (especially security and correctness).
2. Avoid blind syncs that break legal defaults, matter isolation, or approval policy.
3. Keep changes auditable with explicit accept/reject/defer decisions.

## Cadence

1. Weekly intake pass against:
  - `openclaw/main`
  - `upstream/main`
2. Extra intake pass within 24 hours for confirmed critical security advisories.

## Priority Rules

1. Priority 1: security fixes
- auth/trust boundary fixes
- sandbox/tool execution hardening
- path traversal, injection, secret leakage, privilege escalation

2. Priority 2: data integrity and runtime correctness
- persistence correctness
- audit integrity
- deterministic policy enforcement
- migration safety

3. Priority 3: infra/build reliability
- CI stability
- dependency/build fixes
- reproducible local/production startup

4. Out of scope by default
- broad UX/theme churn
- unrelated feature additions
- behavior that weakens legal defaults or approval strictness

## Intake Procedure

1. Fetch remotes and collect candidate commits:
- `git fetch --all --prune --tags`
- Compare `origin/main` against `openclaw/main` and `upstream/main`.

2. Classify each candidate commit:
- `accept`: safe and valuable for cLawyer now
- `defer`: useful but blocked by dependencies/timing
- `reject`: incompatible with cLawyer legal model or priorities

3. Open scoped PRs (one patch family per PR):
- Branch naming: `codex/upstream-intake-<topic>-<date>`
- Prefer cherry-pick/backport over merge.
- Include compatibility note in PR body:
  - preserved legal defaults
  - preserved matter-scoped restrictions
  - preserved approval and audit behavior

4. Validate before merge:
- `cargo fmt --all -- --check`
- `cargo clippy --all-features --all-targets`
- `cargo test -- --test-threads=1`
- plus focused regression tests in touched subsystems

## Compatibility Guardrails (Must Hold)

1. Legal profile defaults remain strict (`max_lockdown` behavior).
2. Matter-context gating and matter-root correctness remain enforced.
3. Sensitive write/exec/network approvals remain required where mandated.
4. Audit append behavior and integrity semantics remain intact.
5. No regression in protected identity-file and traversal protections.

## Tracking and Auditability

1. Maintain one backlog issue: `Upstream Intake Tracker`.
2. For each candidate commit, record:
- source remote (`openclaw` or `upstream`)
- commit SHA and title
- decision (`accept` / `defer` / `reject`)
- rationale
- linked landing PR (if accepted)

3. Include intake summary in each release cycle notes:
- accepted items
- deferred items
- rejected categories (with rationale)

## Escalation Rules

1. Security-critical fixes may use expedited intake.
2. Do not relax branch protection/ruleset requirements for routine intake.
3. If a candidate requires policy changes, open a separate governance PR first.
