---
name: Bug report
about: Something isn't working correctly
labels: bug
---

## Describe the bug

<!-- A clear, concise description of what's wrong. -->

## Steps to reproduce

1.
2.
3.

## Expected behavior

<!-- What should have happened? -->

## Actual behavior

<!-- What actually happened? Include any error output. -->

## Environment

| Field | Value |
|-------|-------|
| cLawyer version / commit | <!-- run `git rev-parse --short HEAD` --> |
| OS | |
| Rust version | <!-- `rustc --version` --> |
| Database backend | <!-- postgres / libsql --> |
| LLM backend | <!-- nearai / openai / anthropic / ollama --> |

## Logs

<!-- Run with `RUST_LOG=ironclaw=debug cargo run` and paste the relevant section. -->

<details>
<summary>Log output</summary>

```
paste logs here
```

</details>

## Additional context

<!-- Anything else that might help: config snippet (redact secrets), screenshots, etc. -->
