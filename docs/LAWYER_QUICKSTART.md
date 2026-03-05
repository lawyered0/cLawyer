# cLawyer Lawyer Quickstart (Non-Dev)

This guide is for lawyers and legal operators who want a fast, practical first run.

## Goal

In about 15 minutes, complete this loop:

1. install cLawyer,
2. run quickstart onboarding,
3. create/activate a matter,
4. produce one cited draft.

## Hardware and runtime expectations

- **CPU-only laptop/desktop**: slower responses; usable for smaller drafts.
- **Modern workstation or cloud VM**: smoother chat and document workflows.
- **Local model runtime** (for example Ollama): strongest data locality, slower on weaker hardware.
- **Hosted model provider**: faster setup and responses, network egress applies to model calls.

Typical first-run times:

- installer + onboarding: 5-10 minutes
- first matter + first draft: 5-10 minutes

## Install commands

macOS, Linux, WSL:

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/lawyered0/cLawyer/releases/latest/download/clawyer-installer.sh | sh
clawyer onboard --quickstart
clawyer run
```

Windows (PowerShell):

```powershell
irm https://github.com/lawyered0/cLawyer/releases/latest/download/clawyer-installer.ps1 | iex
clawyer onboard --quickstart
clawyer run
```

If `clawyer` is not found, open a new terminal and retry.

## First matter walkthrough

1. Open the web UI (gateway URL shown in startup output, usually `http://127.0.0.1:3000`).
2. Go to **Matters** and create a matter with:
   - matter ID
   - client
   - confidentiality
   - retention
3. Run conflict review and record decision if prompted.
4. Set the matter active.
5. In **Chat**, send:
   - "Draft a first-pass issue memo for this matter with citations and uncertainty notes."
6. Confirm output includes:
   - citations,
   - key risks,
   - next-step checklist.
7. Open **Settings -> Logs & Audit** and verify events were recorded.

## Troubleshooting

| Symptom | Likely cause | Fix |
|---|---|---|
| Gateway won’t start | Missing setup or port conflict | Re-run `clawyer onboard --quickstart`; check if port 3000 is already used |
| Login/auth fails | Wrong or stale token | Re-run onboarding and use the refreshed token |
| Conflict block on matter activation | Cross-matter hit requires decision | Record `clear`, `waived`, or `declined` with a note where required |
| Responses are slow | CPU-only or remote provider latency | Reduce prompt size, use smaller model, or run on stronger hardware |
| Missing citations in output | Prompt too broad or missing source context | Ask for "structured citations + uncertainty notes" and attach relevant matter docs |

## Next references

- [README](../README.md)
- [Legal Profile](./LEGAL_PROFILE.md)
- [Firm Rollout](./FIRM_ROLLOUT.md)
- [Backup and Recovery](./BACKUP_AND_RECOVERY.md)
