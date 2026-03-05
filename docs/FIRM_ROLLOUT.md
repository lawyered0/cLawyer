# cLawyer Firm Rollout (Solo and Small-Firm)

This rollout is optimized for solo lawyers and 2-5 lawyer firms running one
firm deployment per host (single tenant).

## 1. Install and start

If you are using the release binary:

```bash
clawyer onboard --quickstart
clawyer run
```

If you are building from source:

```bash
cargo build --release
./target/release/clawyer onboard --quickstart
./target/release/clawyer run
```

During onboarding, keep these recommended defaults:

- legal profile enabled
- hardening: `max_lockdown`
- network: `deny_by_default`
- audit logging enabled

## 2. Matter operating baseline

Use:

- `matters/<matter_id>/` for all matter artifacts
- `matter.yaml` in each matter directory
- conflict review before matter create/activation

Review seeded legal docs:

- `AGENTS.md`
- `legal/CITATION_STYLE_GUIDE.md`
- `legal/CONFIDENTIALITY_NOTES.md`

## 3. Day-to-day policy checks

- Keep tool approvals enabled for sensitive actions.
- Only allowlist outbound domains with explicit business need.
- Require active matter context in legal workflows.

## 4. Operations checklist

Daily:

- confirm `logs/legal_audit.jsonl` is writable
- confirm blocked-action and redaction events are being recorded

Weekly:

- review domain allowlist entries
- review installed skills/extensions
- test one known block case (out-of-scope write or unallowlisted host)

Monthly:

- run backup create + verify drill
- test restore dry-run on a safe environment

## 5. Incident response (local deployment)

If suspicious behavior occurs:

1. Pause assistant usage.
2. Preserve `logs/legal_audit.jsonl` and recent backup artifacts.
3. Export current settings (`clawyer config list`).
4. Rotate secrets and remove untrusted installed skills/extensions.
5. Re-run with strict `--legal-profile max-lockdown`.
6. Document timeline and impact for internal review.
