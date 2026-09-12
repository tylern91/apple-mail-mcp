# apple-mail-mcp

An MCP (Model Context Protocol) server that gives LLM clients read, search, and
send access to Apple Mail — reading Mail.app's on-disk store directly for fast
local search, and driving Mail.app via Automation for mutating operations
(move, flag, delete, send). macOS only.

**Status:** early scaffold — no application logic yet. See `docs/` for the
design as it lands.

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
