# Crate API

**Status:** placeholder — no application logic exists yet (Phase 1A is scaffold-only).

This document will cover, once the `amx-*` library crates expose real types
and functions:

- Per-crate public API summaries for `amx-core`, `amx-store`, `amx-parse`,
  `amx-index`, `amx-compose`, `amx-send`, `amx-automation`, and `amx-mcp`.
- Which crates are portable (compile on any OS as documented no-op stubs) vs.
  Darwin-only (`#![cfg(target_os = "macos")]`), and what that means for a
  consumer depending on this workspace from outside.
- Stability guarantees, if any, ahead of a 1.0 release — today, `publish =
  false` on every crate means none of this is a public API contract yet.
- Links to `cargo doc`-generated reference once it's published somewhere.
