# Security Policy

## Supported versions

apple-mail-mcp releases from `main` only; there are no maintained release
branches. Security fixes land in the latest release.

## Reporting a vulnerability

Please report suspected vulnerabilities privately, using
[GitHub's private vulnerability reporting](https://github.com/tylern91/apple-mail-mcp/security/advisories/new)
for this repository, rather than opening a public issue. This lets a fix land
before the details are public.

Include what you'd include in a bug report: repro steps, affected version
(`amxcli --version`), and impact. Response time isn't guaranteed — this is a
single-maintainer project — but reports will be triaged.

## Automated scanning

Every PR runs `cargo-audit`, `cargo-deny`, and [Trivy](https://github.com/aquasecurity/trivy)
against the dependency tree (`.github/workflows/security.yml`). CRITICAL-severity
vulnerabilities with a known fix block the merge; HIGH-severity findings are
recorded to the repository's Security tab for tracking but don't block.

## Security model

| Concern | Control |
|---|---|
| Mail store integrity | Envelope Index opened `mode=ro`, `SELECT`/`PRAGMA` only; all mutation goes through Mail.app, never direct writes to Apple's store |
| Data egress | Nothing leaves the machine except mail a client explicitly sends. No third-party relay, no telemetry |
| Credentials | SMTP/IMAP secrets live in the Keychain (service `apple-mail-mcp`, account = email address) via `keyring`; never written to config. `Credential`'s `Debug` impl is hand-written to print `Credential::OAuth2(<redacted>)` / `Credential::Password(<redacted>)` rather than deriving one — there is no `tracing` layer in this workspace to filter, so redaction is enforced at the type itself, not at a logging boundary |
| OAuth client secret | The Google OAuth Desktop-app client ID/secret (`AMX_GOOGLE_CLIENT_ID`/`AMX_GOOGLE_CLIENT_SECRET`) are read from the environment only, never committed; only the resulting refresh token is persisted, to the Keychain |
| Read-only mode | `AMX_READ_ONLY=1` removes every mutating tool from `tools/list` |
| Destructive actions | `trash_messages` moves to Mail's Trash — it never erases; bulk destructive plans take a separate, capped path with a review step |
| Attachment writes | Only under a configured temp directory; validated against path traversal (`..`, absolute paths, symlink escapes) |
| HTTP listener | Binds `127.0.0.1` by default (see below) |
| Prompt injection | Mail bodies are untrusted input. Returned content is treated as data, never as instructions; send tools require explicit invocation with explicit recipients |
| Supply chain | `cargo-audit` + `cargo-deny` + a weekly scheduled scan; `Cargo.lock` is committed |

## Known design boundary: the MCP HTTP listener requires a bearer token to bind beyond loopback

The listener binds to `127.0.0.1` by default. Unlike `rqmd`'s MCP listener —
which documents an intentionally unauthenticated boundary — this server
carries a **stricter** requirement, because a mutating tool set that can send
mail on your behalf is a materially higher-stakes surface than a read-only
search index: `amxcli mcp --http` (and `--daemon`) **refuses to start** if
`--host` (or `AMX_MCP_HOST`) resolves to anything other than `127.0.0.1`,
`localhost`, or `::1`, unless a bearer token is also configured (`--auth-token`
or `AMX_MCP_AUTH_TOKEN`). Passing it should be treated as exposing mail
read/mutate/send access to that network, not as a convenience flag — see
[docs/MCP.md](docs/MCP.md#binding-beyond-localhost) for the exact refusal
text and the full list of exposed tools, and
[docs/CONFIGURATION.md](docs/CONFIGURATION.md) for the env var.

If you need multi-tenant access to an apple-mail-mcp instance, that isn't
implemented today; a proposal is welcome as an issue.
