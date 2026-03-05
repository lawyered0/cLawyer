<p align="center">
  <img src="clawyer.png?v=2" alt="cLawyer" width="200"/>
</p>

<h1 align="center">cLawyer</h1>

<p align="center">
  <strong>Local-first legal AI assistant for law firms: matter-scoped, citation-required, and auditable by default.</strong>
  <br />
  <strong>Experimental software: not safe for production use. Use at your own risk.</strong>
</p>

<p align="center">
  <a href="#license"><img src="https://img.shields.io/badge/license-MIT%20OR%20Apache%202.0-blue.svg" alt="License: MIT OR Apache-2.0" /></a>
</p>

<p align="center">
  <a href="#about">About</a> •
  <a href="#for-lawyers-non-dev-quickstart">Lawyer Quickstart</a> •
  <a href="#practical-use-cases">Use Cases</a> •
  <a href="#features">Features</a> •
  <a href="#installation">Installation</a> •
  <a href="#for-risk-partners-and-it">Risk/IT</a> •
  <a href="#configuration">Configuration</a> •
  <a href="#security">Security</a> •
  <a href="#architecture">Architecture</a> •
  <a href="#roadmap-next-36-months">Roadmap</a>
</p>

---

## About

cLawyer is a hardened, local-first fork of IronClaw/OpenClaw built for legal teams.

Version 1 defaults to a U.S.-general legal profile with max-lockdown guardrails so law firm staff can run secure workflows for intake, chronology, contract review, litigation support, and research synthesis.

### Why cLawyer for lawyers

- **No cross-matter leakage by design** - Legal conversations are matter-bound and blocked from cross-matter reuse.
- **Citations and uncertainty are first-class** - Legal outputs are expected to include source references plus risk/uncertainty sections.
- **Runs on your infrastructure** - Local-first architecture, firm-controlled secrets, and audit logging without SaaS telemetry.

- **Matter-first by default** - Work is scoped to `matters/<matter_id>/...` with matter metadata and conflict-check hooks
- **Citation discipline** - Legal outputs require source references and uncertainty/risk sections
- **Max-lockdown hardening** - Deny-by-default network, approval-gated sensitive actions, and strict write boundaries
- **Auditability built in** - Append-only legal audit log with hash-chain integrity and security counters

cLawyer is designed for confidential, defensible legal work product on a single-tenant local deployment.

## For Lawyers (Non-Dev) Quickstart

If you are a lawyer or legal ops lead and want a fast proof of value, use this path.

### 15-minute outcome target

In one session, you should be able to:

1. install cLawyer,
2. run `onboard --quickstart`,
3. create a matter,
4. generate a first cited draft.

### Install (copy/paste)

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

Next: open [docs/LAWYER_QUICKSTART.md](docs/LAWYER_QUICKSTART.md) for expected hardware, timing, and troubleshooting.

## Practical Use Cases

### 1) NDA first-pass review

- Prompt: "Review this NDA for one-way indemnity, broad IP assignment, and unusual termination exposure."
- Expected output: issue table with clause citations, severity, fallback language suggestions, and uncertainty notes.

### 2) Issue-spotting memo

- Prompt: "Draft an issue-spotting memo for this commercial dispute. Separate facts, open legal questions, and immediate risks."
- Expected output: structured memo with cited support, missing facts section, and next research actions.

### 3) Client interview outline

- Prompt: "Build a 30-minute intake interview outline from matter facts and known adversaries."
- Expected output: sectioned interview checklist with follow-up questions, document requests, and conflict-sensitive notes.

## Features

### Security First

- **WASM Sandbox** - Untrusted tools run in isolated WebAssembly containers with capability-based permissions
- **Credential Protection** - Secrets are never exposed to tools; injected at the host boundary with leak detection
- **Prompt Injection Defense** - Pattern detection, content sanitization, and policy enforcement
- **Endpoint Allowlisting** - HTTP requests only to explicitly approved hosts and paths

### Always Available

- **Multi-channel** - REPL, HTTP webhooks, WASM channels (Telegram, Slack), and web gateway
- **Docker Sandbox** - Isolated container execution with per-job tokens and orchestrator/worker pattern
- **Web Gateway** - Browser UI with real-time SSE/WebSocket streaming
- **Routines** - Cron schedules, event triggers, webhook handlers for background automation
- **Heartbeat System** - Proactive background execution for monitoring and maintenance tasks
- **Parallel Jobs** - Handle multiple requests concurrently with isolated contexts
- **Self-repair** - Automatic detection and recovery of stuck operations

### Self-Expanding

- **Dynamic Tool Building** - Describe what you need, and cLawyer builds it as a WASM tool
- **MCP Protocol** - Connect to Model Context Protocol servers for additional capabilities
- **Plugin Architecture** - Drop in new WASM tools and channels without restarting

### Persistent Memory

- **Hybrid Search** - Full-text + vector search using Reciprocal Rank Fusion
- **Workspace Filesystem** - Flexible path-based storage for notes, logs, and context
- **Identity Files** - Maintain consistent personality and preferences across sessions

## Installation

### Prerequisites

- Rust 1.85+
- PostgreSQL 15+ with [pgvector](https://github.com/pgvector/pgvector) extension
- NEAR AI account (authentication handled via setup wizard)

If you use the installer scripts above, you can skip source compilation and start directly with `clawyer onboard --quickstart`.

## Download or Build

Visit [Releases page](https://github.com/lawyered0/cLawyer/releases/) to see the latest updates.

<details>
  <summary>Install via Windows Installer (Windows)</summary>

Download the [Windows Installer](https://github.com/lawyered0/cLawyer/releases/latest/download/clawyer-x86_64-pc-windows-msvc.msi) and run it.

</details>

<details>
  <summary>Install via powershell script (Windows)</summary>

```sh
irm https://github.com/lawyered0/cLawyer/releases/latest/download/clawyer-installer.ps1 | iex
```

</details>

<details>
  <summary>Install via shell script (macOS, Linux, Windows/WSL)</summary>

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/lawyered0/cLawyer/releases/latest/download/clawyer-installer.sh | sh
```
</details>

<details>
  <summary>Install via Homebrew (macOS/Linux)</summary>

```sh
brew install clawyer
```

</details>

<details>
  <summary>Compile the source code (Cargo on Windows, Linux, macOS)</summary>

Install it with `cargo`, just make sure you have [Rust](https://rustup.rs) installed on your computer.

```bash
# Clone the repository
git clone https://github.com/lawyered0/cLawyer.git
cd clawyer

# Build
cargo build --release

# Run tests
cargo test
```

For **full release** (after modifying channel sources), run `./scripts/build-all.sh` to rebuild channels first.

</details>

### Database Setup

```bash
# Create database
createdb clawyer

# Enable pgvector
psql clawyer -c "CREATE EXTENSION IF NOT EXISTS vector;"
```

## Configuration

Run the setup wizard to configure cLawyer:

```bash
clawyer onboard
```

The wizard now supports two modes:

- `clawyer onboard --quickstart` (recommended): lawyer-focused defaults
- `clawyer onboard --advanced`: full technical setup flow

If no mode flag is passed, `clawyer onboard` prompts once and defaults to quickstart.
Settings are persisted in the connected database; bootstrap variables
(e.g. `DATABASE_URL`, `LLM_BACKEND`) are written to `~/.clawyer/.env` so they are
available before the database connects.

### Alternative LLM Providers

cLawyer defaults to NEAR AI but works with any OpenAI-compatible endpoint.
Popular options include **OpenRouter** (300+ models), **Together AI**, **Fireworks AI**,
**Ollama** (local), and self-hosted servers like **vLLM** or **LiteLLM**.

Select *"OpenAI-compatible"* in the wizard, or set environment variables directly:

```env
LLM_BACKEND=openai_compatible
LLM_BASE_URL=https://openrouter.ai/api/v1
LLM_API_KEY=sk-or-...
LLM_MODEL=anthropic/claude-sonnet-4
```

See [docs/LLM_PROVIDERS.md](docs/LLM_PROVIDERS.md) for a full provider guide.

### Legal Mode Quick Start

`cLawyer` defaults to a legal-first profile (`us-general`, `max_lockdown`).

Run with an active matter:

```bash
clawyer --matter acme-v-foo
```

Key references:

- [Legal Profile](docs/LEGAL_PROFILE.md)
- [Migration Guide](docs/MIGRATION_TO_CLAWYER.md)
- [Firm Rollout Guide](docs/FIRM_ROLLOUT.md)

## For Risk Partners and IT

### Data residency

- Matter data, audit artifacts, and workspace files stay on the machine or server you control.
- cLawyer is local-first and single-tenant by deployment design.

### Network egress behavior

- Legal max-lockdown mode defaults to deny-by-default outbound network rules.
- You explicitly allow domains required for your workflow.
- Model calls go only to your configured provider or local runtime.

### Audit and deletion model

- Legal and tool events are written to auditable logs with hash-chain support.
- Matter artifacts are file-backed and can be archived/exported/deleted through controlled workflows.
- Backup and retrieval exports support firm retention/offboarding policies.

## Security

cLawyer implements defense in depth to protect your data and prevent misuse.

### WASM Sandbox

All untrusted tools run in isolated WebAssembly containers:

- **Capability-based permissions** - Explicit opt-in for HTTP, secrets, tool invocation
- **Endpoint allowlisting** - HTTP requests only to approved hosts/paths
- **Credential injection** - Secrets injected at host boundary, never exposed to WASM code
- **Leak detection** - Scans requests and responses for secret exfiltration attempts
- **Rate limiting** - Per-tool request limits to prevent abuse
- **Resource limits** - Memory, CPU, and execution time constraints

```
WASM ──► Allowlist ──► Leak Scan ──► Credential ──► Execute ──► Leak Scan ──► WASM
         Validator     (request)     Injector       Request     (response)
```

### Prompt Injection Defense

External content passes through multiple security layers:

- Pattern-based detection of injection attempts
- Content sanitization and escaping
- Policy rules with severity levels (Block/Warn/Review/Sanitize)
- Tool output wrapping for safe LLM context injection

### Data Protection

- All data stored locally in your PostgreSQL database
- Secrets encrypted with AES-256-GCM
- No telemetry, analytics, or data sharing
- Full audit log of all tool executions

## Architecture

### Simple mental model

```text
Browser UI
   ->
Local web gateway (auth + policy + legal checks)
   ->
LLM provider (local or configured endpoint) + legal tools
   ->
Local DB + matter files (matters/<id>/...)
```

```
┌────────────────────────────────────────────────────────────────┐
│                          Channels                              │
│  ┌──────┐  ┌──────┐   ┌─────────────┐  ┌─────────────┐         │
│  │ REPL │  │ HTTP │   │WASM Channels│  │ Web Gateway │         │
│  └──┬───┘  └──┬───┘   └──────┬──────┘  │ (SSE + WS)  │         │
│     │         │              │         └──────┬──────┘         │
│     └─────────┴──────────────┴────────────────┘                │
│                              │                                 │
│                    ┌─────────▼─────────┐                       │
│                    │    Agent Loop     │  Intent routing       │
│                    └────┬──────────┬───┘                       │
│                         │          │                           │
│              ┌──────────▼────┐  ┌──▼───────────────┐           │
│              │  Scheduler    │  │ Routines Engine  │           │
│              │(parallel jobs)│  │(cron, event, wh) │           │
│              └──────┬────────┘  └────────┬─────────┘           │
│                     │                    │                     │
│       ┌─────────────┼────────────────────┘                     │
│       │             │                                          │
│   ┌───▼─────┐  ┌────▼────────────────┐                         │
│   │ Local   │  │    Orchestrator     │                         │
│   │Workers  │  │  ┌───────────────┐  │                         │
│   │(in-proc)│  │  │ Docker Sandbox│  │                         │
│   └───┬─────┘  │  │   Containers  │  │                         │
│       │        │  │ ┌───────────┐ │  │                         │
│       │        │  │ │Worker / CC│ │  │                         │
│       │        │  │ └───────────┘ │  │                         │
│       │        │  └───────────────┘  │                         │
│       │        └─────────┬───────────┘                         │
│       └──────────────────┤                                     │
│                          │                                     │
│              ┌───────────▼──────────┐                          │
│              │    Tool Registry     │                          │
│              │  Built-in, MCP, WASM │                          │
│              └──────────────────────┘                          │
└────────────────────────────────────────────────────────────────┘
```

### Core Components

| Component | Purpose |
|-----------|---------|
| **Agent Loop** | Main message handling and job coordination |
| **Router** | Classifies user intent (command, query, task) |
| **Scheduler** | Manages parallel job execution with priorities |
| **Worker** | Executes jobs with LLM reasoning and tool calls |
| **Orchestrator** | Container lifecycle, LLM proxying, per-job auth |
| **Web Gateway** | Browser UI with chat, memory, jobs, logs, extensions, routines |
| **Routines Engine** | Scheduled (cron) and reactive (event, webhook) background tasks |
| **Workspace** | Persistent memory with hybrid search |
| **Safety Layer** | Prompt injection defense and content sanitization |

## Usage

```bash
# First-time setup (recommended quickstart mode)
clawyer onboard --quickstart

# Full technical setup flow
clawyer onboard --advanced

# Prompt for mode (defaults to quickstart)
clawyer onboard

# Start interactive REPL
cargo run

# With debug logging
RUST_LOG=clawyer=debug cargo run
```

## Development

```bash
# Format code
cargo fmt

# Lint
cargo clippy --all --benches --tests --examples --all-features

# Run tests
createdb clawyer_test
cargo test

# Run specific test
cargo test test_name
```

- **Telegram channel**: See [docs/TELEGRAM_SETUP.md](docs/TELEGRAM_SETUP.md) for setup and DM pairing.
- **Changing channel sources**: Run `./channels-src/telegram/build.sh` before `cargo build` so the updated WASM is bundled.

## cLawyer Heritage

cLawyer is a Rust reimplementation inspired by [OpenClaw](https://github.com/openclaw/openclaw). See [FEATURE_PARITY.md](FEATURE_PARITY.md) for the complete tracking matrix.

Key differences:

- **Rust vs TypeScript** - Native performance, memory safety, single binary
- **WASM sandbox vs Docker** - Lightweight, capability-based security
- **PostgreSQL vs SQLite** - Production-ready persistence
- **Security-first design** - Multiple defense layers, credential protection

## Deployment Patterns for Firms

- **Solo / 1-2 lawyers** - single high-spec workstation with local model or one hosted provider.
- **Small firm / 2-5 lawyers** - one shared host with centralized audit operations and staged RBAC/membership rollout.
- **Pilot / 10-50 lawyers** - firm-managed server deployment, staged onboarding by practice group, and controlled provider policy.

## Who Built cLawyer

cLawyer is built by **Lawyered** ([x.com/BitGrateful](https://x.com/BitGrateful/)), with a legal-first focus on confidentiality, matter isolation, and practical daily workflows for lawyers.

## Roadmap (next 3-6 months)

Current focus is lawyer adoption for solo and small firms:

1. Lawyer quickstart clarity and onboarding polish.
2. Matter-level collaboration and access enforcement (RBAC).
3. Mobile usability for chat + matters workflows.
4. Maintainability upgrades in high-change client modules.

Track progress in the [issue tracker](https://github.com/lawyered0/cLawyer/issues).

## License

Licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT License ([LICENSE-MIT](LICENSE-MIT))

at your option.
