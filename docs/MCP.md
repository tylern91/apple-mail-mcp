# MCP Server

**Status:** placeholder — no application logic exists yet (Phase 1A is scaffold-only).

This document will cover, once `amx-mcp` implements the protocol:

- The full tool list exposed via `tools/list` (search, read, send, move,
  flag, trash, and any others), with input/output schemas.
- Daemon lifecycle: starting via `amxcli mcp --http` / `--daemon`, stopping,
  and log locations.
- **Binding beyond localhost** — the exact refusal behavior and message when
  `--host`/`AMX_MCP_HOST` resolves to a non-loopback address without
  `--auth-token`/`AMX_MCP_AUTH_TOKEN` also configured (see
  [SECURITY.md](../SECURITY.md) for the rationale).
- Client integration examples (Claude Desktop, Claude Code, other MCP
  clients).
- `AMX_READ_ONLY=1` behavior — which tools disappear from `tools/list`.
