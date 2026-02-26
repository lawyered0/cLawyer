# Contributing to cLawyer

Thank you for helping make cLawyer better. This guide covers everything you need to go from idea to merged PR.

> **cLawyer is experimental software** used in legal contexts. Changes that affect correctness, security, or auditability are held to a high bar. When in doubt, open an issue first.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [The Ship Checklist](#the-ship-checklist)
- [Opening Issues](#opening-issues)
- [Opening Pull Requests](#opening-pull-requests)
- [Commit Conventions](#commit-conventions)
- [Code Style Rules](#code-style-rules)
- [Feature Parity](#feature-parity)
- [Architecture Quick Reference](#architecture-quick-reference)
- [Automated Labels](#automated-labels)
- [Reporting Security Vulnerabilities](#reporting-security-vulnerabilities)

---

## Code of Conduct

Be direct, respectful, and charitable. Disagree with ideas, not people. Keep feedback actionable. Legal tooling affects real people's work — hold correctness and honesty to a high standard.

---

## Getting Started

1. **Search existing issues** before opening a new one — your question or idea may already have a thread.
2. **Open an issue first** for significant changes (new features, architectural shifts, security-related work) so the direction can be agreed on before you write code.
3. **Fork → branch → PR** for all code changes. Do not push directly to `main`.

---

## Development Setup

### Prerequisites

| Tool | Minimum version | Notes |
|------|----------------|-------|
| Rust | stable (1.85+) | Install via [rustup](https://rustup.rs) |
| PostgreSQL | 15+ | Needs [pgvector](https://github.com/pgvector/pgvector) extension |
| Docker | 24+ | Required only for sandbox execution (`SANDBOX_ENABLED=true`) |
| libSQL (optional) | — | Zero-dependency alternative to PostgreSQL; see `DATABASE_BACKEND=libsql` |

### First-time setup

```bash
git clone https://github.com/lawyered0/cLawyer.git
cd cLawyer

# Copy and populate the environment file
cp .env.example .env
# Edit .env — at minimum set DATABASE_URL and your LLM provider credentials

# Build (PostgreSQL backend, default)
cargo build

# Build with libSQL instead
cargo build --no-default-features --features libsql

# Build with both backends available
cargo build --features "postgres,libsql"
```

### Running locally

```bash
# With verbose logging
RUST_LOG=ironclaw=debug cargo run

# Module-scoped logging
RUST_LOG=ironclaw::agent=debug cargo run
```

The web gateway starts at `http://127.0.0.1:3001` by default (`GATEWAY_PORT`).

---

## The Ship Checklist

**Run these three commands before every commit. All must be clean.**

```bash
# 1. Format — must produce no changes
cargo fmt

# 2. Lint — zero warnings, zero errors
cargo clippy --all --benches --tests --examples --all-features

# 3. Test — all must pass (run single-threaded to avoid a pre-existing SIGABRT flake in parallel mode)
cargo test --lib -- --test-threads=1
```

Expected output for a clean ship:
- `cargo fmt` → silent (no diff)
- `cargo clippy` → `Finished` with no `warning:` lines
- `cargo test --lib` → `test result: ok. N passed; 0 failed`

> **Both database backends must compile.** If your change touches persistence:
> ```bash
> cargo check --no-default-features --features libsql   # libsql-only
> cargo check                                            # postgres-only (default)
> cargo check --all-features                             # both
> ```

> **Integration tests require PostgreSQL** and are not part of the local gate. Only `--lib` failures are blocking.

### Mechanical pre-commit checks

Run these on changed files before pushing:

```bash
# No panics in production code
grep -rnE '\.unwrap\(|\.expect\(' src/

# No super:: imports (use crate:: instead)
grep -rn 'super::' src/

# If you fixed a pattern bug, check for other instances
grep -rn '<the-pattern>' src/
```

---

## Opening Issues

### Bug reports

Include:
- cLawyer version / commit hash
- Steps to reproduce (minimal)
- Expected vs. actual behavior
- Relevant log output (`RUST_LOG=ironclaw=debug`)
- Database backend in use (`postgres` / `libsql`)

### Feature requests

Include:
- The problem you're solving (not just the solution)
- How it fits the legal-first mission
- Any alternatives you considered

### Security issues

**Do not open a public issue for security vulnerabilities.** See [Reporting Security Vulnerabilities](#reporting-security-vulnerabilities).

---

## Opening Pull Requests

### Branch naming

```
feat/<short-slug>          # new capability
fix/<short-slug>           # bug fix
chore/<short-slug>         # maintenance, deps, CI
docs/<short-slug>          # documentation only
refactor/<short-slug>      # no behavior change
```

### PR requirements

Every PR must:

- [ ] Pass the [Ship Checklist](#the-ship-checklist) (fmt, clippy, tests)
- [ ] Have a clear title following [Commit Conventions](#commit-conventions)
- [ ] Update `FEATURE_PARITY.md` if the change affects a tracked capability (see [Feature Parity](#feature-parity))
- [ ] Include tests for new logic (unit tests in `mod tests {}` at the bottom of the file)
- [ ] Not introduce `.unwrap()` or `.expect()` in production code paths
- [ ] Use `crate::` imports, not `super::`

### PR size guidelines

Prefer small, reviewable PRs. The CI labeler will tag each PR automatically:

| Label | Changed lines |
|-------|--------------|
| `size: XS` | < 10 |
| `size: S` | < 50 |
| `size: M` | < 200 |
| `size: L` | < 500 |
| `size: XL` | 500+ |

XL PRs will receive extra scrutiny. If your PR is XL, consider splitting it.

### What makes a great PR description

- **What** changed and **why** (not just a restatement of the title)
- A test plan: what you tested and how to verify it
- Any alternatives you considered and ruled out
- Screenshots or `curl` examples for API / UI changes

---

## Commit Conventions

We use [Conventional Commits](https://www.conventionalcommits.org/). The format is:

```
<type>(<scope>): <short description>

[optional body]

[optional footer]
```

### Types

| Type | When to use |
|------|-------------|
| `feat` | New user-visible feature |
| `fix` | Bug fix |
| `chore` | Maintenance, dependency updates, CI |
| `docs` | Documentation only |
| `refactor` | Code change that adds no feature and fixes no bug |
| `test` | Adding or correcting tests |
| `perf` | Performance improvement |
| `security` | Security hardening or vulnerability fix |

### Common scopes

`agent`, `channel/web`, `channel/cli`, `tool/builtin`, `tool/wasm`, `db`, `db/postgres`, `db/libsql`, `workspace`, `sandbox`, `safety`, `llm`, `legal`, `ci`, `docs`

### Examples

```
feat(channel/web): add document upload to Memory tab
fix(safety): reject path traversal sequences in MemoryWriteTool
chore(deps): bump axum to 0.8.3
security(audit): enforce 0o600 permissions on legal audit log
docs(contributing): expand contributing guide
```

---

## Code Style Rules

These rules are enforced by CI and code review.

### Error handling

```rust
// ✅ Good — thiserror with context
use thiserror::Error;
#[derive(Error, Debug)]
pub enum MyError {
    #[error("thing failed: {reason}")]
    Failed { reason: String },
}
fn do_thing() -> Result<(), MyError> {
    something().map_err(|e| MyError::Failed { reason: e.to_string() })?;
    Ok(())
}

// ❌ Bad — panics in production
let value = map.get("key").unwrap();
let conn = db.connect().expect("db must be up");
```

`unwrap()` and `expect()` are allowed **only** in test code (`#[cfg(test)]` blocks or test binaries).

### Imports

```rust
// ✅ Good
use crate::agent::scheduler::Scheduler;

// ❌ Bad
use super::scheduler::Scheduler;
```

Always use `crate::` for paths within the crate. `super::` is disallowed.

### Async and shared state

```rust
// Shared read-write state
let state: Arc<RwLock<MyState>> = Arc::new(RwLock::new(MyState::default()));

// Shared read-only state
let config: Arc<MyConfig> = Arc::new(MyConfig::load()?);
```

All I/O is async (tokio). Use `Arc<T>` for shared state across tasks, `RwLock` when concurrent mutation is needed.

### Types over strings

```rust
// ✅ Good — strong types
pub enum SandboxPolicy { ReadOnly, WorkspaceWrite, FullAccess }

// ❌ Bad — stringly typed
pub fn set_policy(policy: &str) { ... }
```

### Persistence: both backends required

Any change that adds or modifies persistence **must** be implemented in both:
- `src/db/postgres.rs` (delegate to Store/Repository)
- `src/db/libsql_backend.rs` (native SQLite-dialect SQL)

And must be added as a method on the `Database` trait in `src/db/mod.rs`. See the [Database section of CLAUDE.md](CLAUDE.md) for the full schema translation guide.

### Comments

Write comments for non-obvious logic only. Prefer self-documenting names over comments that restate what the code does.

```rust
// ✅ Good — explains *why*
// SECURITY: reject path traversal sequences regardless of how the path
// was constructed above; canonicalization alone is not sufficient.
if path.components().any(|c| c == Component::ParentDir) { ... }

// ❌ Bad — restates the code
// Check if path contains parent dir
if path.components().any(|c| c == Component::ParentDir) { ... }
```

---

## Feature Parity

`FEATURE_PARITY.md` tracks what cLawyer has vs. the upstream OpenClaw reference implementation. Keeping it accurate is a shared responsibility.

### When you must update it

Update `FEATURE_PARITY.md` **in the same commit** as your code change when:

- You implement a feature marked ❌ or 🔮 → change to ✅ or 🚧
- You partially complete a feature → update notes
- You intentionally scope something out → mark 🚫 with a rationale note

### How to update it

1. Find the relevant section in `FEATURE_PARITY.md`
2. Update the IronClaw column status
3. Update the Notes column with any caveats or implementation details
4. Commit the matrix change alongside the code

---

## Architecture Quick Reference

### Adding a new tool (built-in Rust)

1. Create `src/tools/builtin/<name>.rs` and implement the `Tool` trait
2. Register it in `src/tools/builtin/mod.rs`
3. Add to the registry in `main.rs`

See `src/tools/README.md` for full details and the WASM vs. built-in decision guide.

### Adding a new channel

1. Create `src/channels/<name>.rs` and implement the `Channel` trait
2. Add config fields in `src/config.rs`
3. Wire up in `main.rs`

### Adding a new API endpoint

1. Add the handler function in `src/channels/web/server.rs`
2. Add request/response types in `src/channels/web/types.rs`
3. Register the route in the `Router` builder in `server.rs`

### Adding a new database method

1. Declare it on the `Database` trait in `src/db/mod.rs`
2. Implement in `src/db/postgres.rs`
3. Implement in `src/db/libsql_backend.rs`
4. Verify both backends compile (`cargo check --no-default-features --features libsql && cargo check`)

### Key module specs

Some modules have a `README.md` that is the authoritative specification. **Read the spec before changing the module.**

| Module | Spec |
|--------|------|
| `src/setup/` | `src/setup/README.md` |
| `src/workspace/` | `src/workspace/README.md` |
| `src/tools/` | `src/tools/README.md` |

---

## Automated Labels

CI automatically applies labels to every PR. You do not need to set these yourself.

### Scope labels (`scope: *`)

Applied by `.github/labeler.yml` based on which files changed. Examples: `scope: channel/web`, `scope: tool/builtin`, `scope: safety`.

### Size labels (`size: *`)

Applied by `.github/scripts/pr-labeler.sh` based on changed lines (excluding docs):

| Label | Changed lines |
|-------|--------------|
| `size: XS` | < 10 |
| `size: S` | < 50 |
| `size: M` | < 200 |
| `size: L` | < 500 |
| `size: XL` | 500+ |

### Risk labels (`risk: *`)

Applied automatically based on files touched:

- `risk: high` — security-sensitive paths (`src/safety/`, `src/sandbox/`, `src/secrets/`, `src/legal/`)
- `risk: medium` — core agent or DB changes
- `risk: low` — UI, docs, or test-only changes

### Contributor labels

First-time contributors receive a `first-time contributor` label. Welcome!

---

## Reporting Security Vulnerabilities

**Do not open a public GitHub issue for security vulnerabilities.**

To report a vulnerability, email the maintainer directly (see the repository's GitHub profile for contact details) or open a [GitHub private security advisory](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing/privately-reporting-a-security-vulnerability).

Please include:
- A description of the vulnerability and its impact
- Steps to reproduce
- Any suggested mitigations

You will receive a response within 72 hours. We will coordinate a disclosure timeline with you before any public announcement.

---

*This guide is a living document. If something is missing or unclear, open a PR to improve it.*
