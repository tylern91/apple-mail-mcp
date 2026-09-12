# Installation

**Status:** placeholder — no application logic exists yet (Phase 1A is scaffold-only,
so there is nothing to install yet). This document will cover, once a first
release ships:

- Homebrew: `brew tap tylern91/apple-mail-mcp && brew install amxcli`.
- Prebuilt binary: downloading and verifying the macOS arm64 release asset
  (SHA-256 checksum, GPG-signed tag).
- From source: `cargo install` / building from a checkout.
- Minimum macOS version and Apple Silicon requirement (no Intel build is
  published).
- First-run setup: granting Mail.app Automation permission, and Full Disk
  Access if required.
