# Configuration

**Status:** placeholder — no application logic exists yet (Phase 1A is scaffold-only).

This document will cover, once configuration surfaces land:

- Environment variables (`AMX_READ_ONLY`, `AMX_MCP_HOST`, `AMX_MCP_AUTH_TOKEN`,
  `AMX_PROFILE`, and any others introduced in later phases) and their defaults.
- Where apple-mail-mcp expects to find Mail.app's data
  (`~/Library/Mail`) and what read permissions macOS requires (Full Disk
  Access, if needed).
- Where apple-mail-mcp's own config/cache/index files live on disk.
- Config file format and precedence (env var vs. file vs. CLI flag).
- SMTP credential storage via the system Keychain.
