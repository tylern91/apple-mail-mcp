# Architecture

**Status:** placeholder — no application logic exists yet (Phase 1A is scaffold-only).

This document will cover, once Phase 1B+ lands real code:

- Workspace layout and why each `amx-*` crate exists (`amx-core`, `amx-store`,
  `amx-parse`, `amx-index`, `amx-compose`, `amx-send`, `amx-automation`,
  `amx-mcp`, `amx-cli`).
- The portable/Darwin-only crate split and why four crates compile as empty
  stubs on Linux CI while five are gated `#![cfg(target_os = "macos")]`.
- Data flow: Envelope Index (read path) vs. Mail.app Automation (mutate/send
  path), and why those two paths never cross.
- Indexing strategy and how it's kept in sync with Mail.app's own state.
- The MCP server's request lifecycle, from `tools/list` through tool
  invocation.
- Key design decisions and the trade-offs behind them.
