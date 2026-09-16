# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.1.0] - 2026-09-16

### Added

- `amxcli credentials set <email>` / `amxcli credentials status <email>`:
  interactive (hidden-input) password storage and presence-only status
  reporting against the `amx-send` `CredentialBroker` Keychain namespace —
  never exposes a stored secret.
- `amxcli auth google <email>`: drives the existing Google OAuth2 PKCE +
  loopback-redirect consent flow and persists the resulting refresh token.
- `CredentialBroker::has_password` / `has_refresh_token`: presence-only
  checks backing the new `status` subcommand.

## [1.0.0] - 2026-09-14

The wire contract (tool names, schemas, and lane assignments) freezes as of
this release.

### Added

- `amx-compose`: portable MIME construction via `mail-builder`
  (`Draft`/`EmailAddress`/`compose`) — multipart/alternative bodies, RFC 2047
  header encoding, generated `Message-ID`, `References`/`In-Reply-To`
  threading. `derive_reply`/`derive_forward` compute the reply-all recipient
  set and quoted-history forward body from a source message; a missing
  `Message-ID` or an unreadable/unindexed source message is a hard
  `AmxError::ReplyDerivationFailed`, never a silent fallback to a plain send.
- `amx-send` (macOS only): `CredentialBroker` (Keychain-backed, service
  `apple-mail-mcp`), `Provider::{Google, AppSpecificPassword}`, OAuth2/XOAUTH2
  for Google accounts and app-specific-password auth for iCloud/generic IMAP;
  `submit` (SMTP, STARTTLS) and `append_to_special_use` (IMAP `APPEND` to the
  `\Sent`/`\Drafts` special-use mailbox).
- `amx-mcp`: the send lane — `send_message`, `create_draft`, `reply_message`,
  `forward_message` — gated behind `ToolLane::Send`, hidden under
  `AMX_READ_ONLY`, `openWorldHint: true`. Sent-mailbox filing follows IMAP
  `APPEND` for iCloud/IMAP/Exchange accounts, skipped for Gmail (server-side
  Sent filing) — the Gmail/non-Gmail distinction is derived from the
  account's SMTP hostname suffix, not `AccountKind`.
- `crates/amx-compose/tests/subject_non_ascii.rs`: regression fixture
  (UseJunior #198) asserting a non-ASCII `Subject:` round-trips through a
  single `=?utf-8?` encoded-word pass with no double-encoding.

### Known gaps

- Sent-mailbox filing has no path for POP or On-My-Mac accounts — neither
  exposes a remote protocol to `APPEND` over. This is a documented limitation,
  not a bug.
- Live-send verification (non-ASCII subject arriving correctly in Outlook and
  Gmail, reply `References`/reply-all correctness against a real thread) is
  **deferred** — it requires a Google Cloud OAuth Desktop-app client and an
  iCloud app-specific password loaded into the Keychain, and `amxcli` has no
  `credentials`/`auth` subcommand yet to drive that setup. Until this is done,
  `send_message`/`reply_message`/`forward_message`/`create_draft` are
  code-reviewed, unit-tested, and clippy/deny-clean but have not exchanged
  mail with a real SMTP/IMAP server.
- There is no `tracing` layer anywhere in this workspace, so credential
  redaction is enforced structurally instead (a manual `Debug` impl on
  `Credential` that never prints the secret) rather than via a log-layer
  filter; see `SECURITY.md`.

## [0.3.0] - 2026-09-13

### Added

- `amx-index::search`: portable query builder (boosted subject/body/sender/
  recipients/attachment-text, mailbox/account/date-range filters) returning
  ranked hits alongside a `CoverageEnvelope` from a single collector pass.
- `amx-mcp`: the read lane's 14-tool MCP surface — `search_messages`,
  `get_message`, `list_attachments`, `get_attachment`,
  `extract_attachment_text`, `get_thread`, `get_message_links`,
  `count_messages`, `recent_messages`, `resolve_address`, `list_accounts`,
  `list_mailboxes`, `doctor`, `status` — behind a `ToolLane` registry that
  filters mutating/send tools out of `tools/list` under `AMX_READ_ONLY`.
- `amx-mcp::health_monitor`: `HealthMonitor` tracks store-access state via
  `TccProbe` re-probing, stamping `health.store_access` on every response
  while degraded and naming the resolved responsible process on remediation.
- `amx-mcp::transport` + `amxcli serve`: stdio and streamable-HTTP transports.
  Binding a non-loopback address without `--token` is refused at startup
  (umbrella §8); the HTTP path also gates every request behind the bearer
  token when one is configured.
- `amxcli`: `Serve { --transport stdio|http, --bind, --token }` subcommand,
  registered locally with Claude Code and verified end-to-end — ranked body
  search with a correct coverage envelope, and `amxcli sync` running
  concurrently with a live `amxcli serve` with no restart (imdinu #122).

### Fixed

- `amx-mcp`'s `list_tools` response now carries `resultType`/`ttlMs`/
  `cacheScope` (SEP-2322/SEP-2549) — required once a connecting client
  negotiates protocol revision `2026-07-28`, previously rejected as missing.

## [0.2.0] - 2026-09-12

### Added

- `amx-core`: error taxonomy (`AmxError`), the two-dimensional coverage model
  (`Availability`/`BodyState`/`AttachmentState`/`UnavailableReason`), core traits
  (`MailStore`/`Automation`/`Transport`), and layered config loading.
- `amx-store`: read-only Envelope Index access (`RoConnection`), Full Disk Access
  probing (`TccProbe`), account resolution against `Accounts4.sqlite`
  (`AccountResolver`), mailbox lookup with percent-decode/NFC/casefold
  normalization (`MailboxRegistry`), the `.emlx` on-disk shard path algorithm
  (`EmlxPathResolver`), and filtered message queries (`MessageQuery`). The crate
  is now portable (Linux-buildable) apart from its macOS-only `tcc`/`watch`
  modules.
- `amx-parse`: `.emlx` container parsing via `mail-parser`, the
  attachment-completeness oracle (declared vs. on-disk size), and attachment
  content extraction for HTML, PDF, and Office Open XML (DOCX/PPTX/XLSX).
- `amxcli doctor`: a read-only readiness screen reporting Full Disk Access
  state, store version, account/mailbox counts, and raw `.emlx` coverage
  (clean-parse / quarantine / partial).

### Fixed

- `.emlx` test fixtures are now marked binary in `.gitattributes` so
  `core.autocrlf` can no longer silently corrupt them on checkout.

## [0.1.0] - 2026-09-12

### Added

- Initial workspace scaffold: nine-crate layout, CI/CD workflows, release
  scripting, and repo metadata. No application logic yet.
