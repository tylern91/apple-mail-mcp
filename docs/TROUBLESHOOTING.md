# Troubleshooting

**Status:** placeholder — no application logic exists yet (Phase 1A is scaffold-only).

This document will cover, once there's a running binary to troubleshoot:

- Mail.app Automation permission not granted — the exact macOS prompt/System
  Settings path to fix it.
- Envelope Index not found or unreadable (wrong macOS account, iCloud Mail
  sync still indexing, Full Disk Access missing).
- MCP listener refusing to bind beyond loopback without a token — how to
  read that error and what to configure.
- SMTP send failures and how to distinguish a Keychain credential problem
  from a mail-server-side rejection.
- Where to find logs, and how to raise verbosity.
