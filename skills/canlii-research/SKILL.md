---
name: canlii-research
version: 0.1.0
description: Canadian legal research via CanLII case law and legislation
activation:
  patterns:
    - "(?i)canlii|can ?lii"
    - "(?i)find.*case.*law|search.*case.*law"
    - "(?i)canadian.*jurisprudence|jurisprudence.*canada"
    - "(?i)(onca|onsc|scc|fca).*decision|decision.*(onca|onsc|scc|fca)"
    - "(?i)look up.*citation|citation.*look up"
  keywords:
    - canlii
    - case law
    - jurisprudence
    - precedent
  max_context_tokens: 2000
metadata:
  domain: legal
  requires_matter: true
  citation_mode: required
  clawyer:
    requires:
      bins: []
      env:
        - CANLII_API_KEY
---

# CanLII Research Skill

You are assisting with Canadian legal research.

Use `canlii_search` to search CanLII for case law and legislation. Search is jurisdiction-scoped, so run separate searches when the user needs more than one court or jurisdiction.

Common jurisdiction codes:

- `scc` — Supreme Court of Canada
- `fca` — Federal Court of Appeal
- `onca` — Ontario Court of Appeal
- `onsc` — Ontario Superior Court
- `ca` — Federal / Canada-wide statutory material when available

When interpreting returned citations, use the Canadian parsing helpers in `legal::citations` and always surface the source URL so the user can verify the authority directly.
