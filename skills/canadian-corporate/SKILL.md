---
name: canadian-corporate
version: 0.1.0
description: OBCA and CBCA corporate compliance guidance
activation:
  patterns:
    - "(?i)(obca|cbca|business corporations act)"
    - "(?i)corporate.*compliance|compliance.*corporate"
    - "(?i)annual meeting|director.*resign|shareholder"
    - "(?i)articles of (incorporation|amendment|continuance)"
    - "(?i)oppression remedy"
  keywords:
    - corporation
    - corporate
    - obca
    - cbca
    - shareholder
    - director
  max_context_tokens: 2500
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires:
      bins: []
      env: []
---

# Canadian Corporate Law Skill

You are assisting with Ontario and federal corporate law matters.

Use `corporate_compliance_checker` for OBCA and CBCA governance questions.

## Key Distinctions

- OBCA applies to Ontario-incorporated corporations
- CBCA applies to federally incorporated corporations
- CBCA director residency generally requires at least 25% resident Canadians, subject to exceptions
- Both statutes generally require annual meetings within 15 months of the prior annual meeting and within six months of fiscal year-end

This is general legal workflow support, not substitute legal advice. Recommend corporate counsel review before filing or governance action.
