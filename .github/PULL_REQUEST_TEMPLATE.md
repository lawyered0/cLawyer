## What does this PR do?

<!-- One or two sentences. What changed and why? -->

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor (no behavior change)
- [ ] Documentation
- [ ] Security / hardening
- [ ] Dependencies / CI

## Ship checklist

- [ ] `cargo fmt` — clean (no diff)
- [ ] `cargo clippy --all --benches --tests --examples --all-features` — zero warnings
- [ ] `cargo test --lib -- --test-threads=1` — all pass
- [ ] No `.unwrap()` or `.expect()` in production code paths
- [ ] `crate::` imports used (no `super::`)

## Both database backends compile (if persistence changed)

- [ ] `cargo check` (postgres, default)
- [ ] `cargo check --no-default-features --features libsql`

_Skip this section if no database code changed._

## Feature parity

- [ ] `FEATURE_PARITY.md` updated in this branch, **or** this change does not affect a tracked capability

## Test plan

<!-- How did you verify this works? Include curl examples, screenshots, or manual steps. -->

## Notes for reviewer

<!-- Anything the reviewer should know: tricky parts, alternatives considered, follow-up work. -->
