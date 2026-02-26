# Security Policy

## Scope

cLawyer is experimental software designed for local, single-tenant legal workflows. Its security model assumes a trusted local network and a single authenticated user. It is **not** designed for multi-tenant or public internet deployment.

Security issues in scope include but are not limited to:

- Prompt injection that bypasses matter isolation, audit logging, or privilege boundaries
- Path traversal in the workspace or file tools
- Secret / credential leakage through tool output, LLM responses, or log streams
- Authentication bypass in the web gateway
- Sandbox escape from Docker or WASM execution environments
- Audit log integrity violations (hash chain bypass, permission downgrade)
- Unsafe deserialization or parsing in any input path

## Supported Versions

Only the latest commit on the `main` branch receives security fixes. There are no versioned releases with long-term support at this time.

## Reporting a Vulnerability

**Do not open a public GitHub issue for security vulnerabilities.**

To report a vulnerability privately, use one of these channels:

1. **GitHub private security advisory** (preferred) — [open an advisory](https://github.com/lawyered0/cLawyer/security/advisories/new) directly on this repository. Only you and the maintainers can see it until disclosure.
2. **Email** — contact the maintainer at the address listed on their GitHub profile.

### What to include

- A description of the vulnerability and its potential impact
- The affected component(s) and file paths
- Steps to reproduce (proof-of-concept code or curl commands are helpful)
- Your assessment of severity (CVSS score if you have one)
- Any suggested mitigations or patches

### Response timeline

| Milestone | Target |
|-----------|--------|
| Initial acknowledgement | 72 hours |
| Severity assessment | 7 days |
| Fix or mitigation | Depends on severity — critical issues targeted within 14 days |
| Coordinated disclosure | Agreed with reporter before any public announcement |

We will credit you in the fix commit and changelog unless you request anonymity.

## Security Design Notes

Understanding the threat model helps identify what is and is not in scope.

### What cLawyer protects against

- **Prompt injection from tool output** — All external data passes through `SafetyLayer` (sanitizer → validator → policy → leak detector) before reaching the LLM.
- **Secret leakage** — API keys and tokens are AES-256-GCM encrypted at rest; the shell tool scrubs sensitive env vars before execution; the leak detector scans LLM responses before they reach the user.
- **Filesystem escape from tools** — `MemoryWriteTool` enforces matter-scoped path prefixes and rejects `ParentDir` components (path traversal).
- **Audit log tampering** — The legal audit log is append-only JSONL with a SHA-256 hash chain. Log files are created `0o600` (owner read/write only).
- **Container network access** — Sandbox containers route HTTP/HTTPS through a host-side proxy with a domain allowlist. Credentials are injected at the proxy layer; containers never receive raw secret values.

### Known limitations

- **Raw TCP/UDP bypass** — The network proxy controls HTTP/HTTPS only. Sandbox containers using raw TCP or UDP can reach the host network. Mitigate in production with a per-sandbox user-defined Docker network.
- **No encryption at rest for libSQL** — The local SQLite database stores content in plaintext (only secrets are encrypted). Use full-disk encryption (FileVault, LUKS, BitLocker) if this is a concern.
- **Single-tenant only** — The bearer token auth model assumes one user. Do not expose the gateway to untrusted networks.

## Dependency Vulnerabilities

To audit third-party dependencies:

```bash
cargo install cargo-audit
cargo audit
```

Please report dependency vulnerabilities through the same private channel described above so we can coordinate with upstream maintainers before public disclosure.
