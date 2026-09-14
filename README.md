# apple-mail-mcp

An MCP (Model Context Protocol) server that gives LLM clients read, search, and
send access to Apple Mail — reading Mail.app's on-disk store directly for fast
local search, and driving Mail.app via Automation for mutating operations
(move, flag, delete, send). macOS only.

**Status:** `1.0.0` — read, mutate, and send lanes are all implemented; the
wire contract (tool names, schemas, lane assignments) is frozen as of this
release. See `docs/` for the design as it lands.

## Sending mail

`send_message`/`reply_message`/`forward_message`/`create_draft` build MIME
in-process (`amx-compose`) and submit over the account's own SMTP endpoint
(`amx-send`) — no AppleScript compose path. Credentials (OAuth2 for Google,
an app-specific password for iCloud/generic IMAP) live in the macOS Keychain
under service `apple-mail-mcp`, one entry per account email address;
Mail.app's own Keychain items are never touched. A sent copy is filed to the
Sent mailbox via IMAP `APPEND` for iCloud/IMAP/Exchange accounts — skipped for
Gmail, which files server-side already. POP and On-My-Mac accounts have no
outbound protocol to append a Sent copy over, so that gap is undocumented in
software and stays a known limitation.

These four tools are hidden entirely from `tools/list` under
`AMX_READ_ONLY=1`.

## Documentation

This README covers the essentials. Everything else lives in `docs/`:

| Doc | Covers |
|-----|--------|
| [docs/INSTALL.md](docs/INSTALL.md) | Install matrix — Homebrew, cargo, prebuilt binaries, source |
| [docs/CLI.md](docs/CLI.md) | `amxcli` command reference and global flags |
| [docs/MCP.md](docs/MCP.md) | MCP server tools, daemon lifecycle, binding beyond localhost |
| [docs/CONFIGURATION.md](docs/CONFIGURATION.md) | Environment variables, where data lives, Mail.app permissions |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | How it works, design decisions, workspace layout |
| [docs/CRATE-API.md](docs/CRATE-API.md) | Rust API for the `amx-*` crates |
| [docs/SECURITY-MODEL.md](docs/SECURITY-MODEL.md) | Threat model, local-only guarantees, listener auth |
| [docs/MIGRATING.md](docs/MIGRATING.md) | Breaking-change migration notes across releases |
| [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md) | Common setup and permission issues |

Also: [CONTRIBUTING.md](CONTRIBUTING.md) · [SECURITY.md](SECURITY.md) · [DISCLAIMER.md](DISCLAIMER.md)

## License

MIT — see [LICENSE](LICENSE).
