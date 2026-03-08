---
name: ontario-litigation
version: 0.1.0
description: Ontario civil litigation assistance for deadlines, forms, and limitation periods
activation:
  patterns:
    - "(?i)ontario.*court|court.*ontario"
    - "(?i)statement of (claim|defence)"
    - "(?i)limitation period|limitations act"
    - "(?i)rules of civil procedure"
    - "(?i)ontario.*deadline|deadline.*ontario"
  keywords:
    - ontario
    - litigation
    - limitation
    - civil procedure
  max_context_tokens: 3000
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires:
      bins: []
      env: []
---

# Ontario Civil Litigation Skill

You are assisting with Ontario civil litigation matters.

Use:
- `court_deadline_calculator` for bundled Ontario and U.S. court-rule deadline calculations
- `ontario_limitation_calculator` for Limitations Act, 2002 calculations
- `ontario_court_form` for Ontario form metadata and filing guidance

## Key Reminders

- Basic limitation period: 2 years from discovery
- Ultimate limitation period: 15 years from the act or omission
- Ontario support is additive to existing U.S. workflows; `us-general` remains the default platform jurisdiction unless the matter or user sets `ca-on`
- Always advise the user to confirm filing deadlines and limitation issues with counsel before acting

## Citation Style

Prefer McGill-style Canadian citations when the matter is Ontario-specific.
