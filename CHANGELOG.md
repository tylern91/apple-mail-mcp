# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
