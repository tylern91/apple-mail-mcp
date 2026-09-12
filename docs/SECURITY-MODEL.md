# Security Model

**Status:** placeholder — no application logic exists yet (Phase 1A is scaffold-only).
For the current, authoritative summary see [SECURITY.md](../SECURITY.md) at
the repo root; this document will expand it once real code exists, covering:

- Full threat model: what an attacker with local access, network access to
  the MCP listener, or control over inbound mail content can and can't do.
- Why the Envelope Index is opened read-only, and the exact boundary between
  "read directly" and "mutate only via Mail.app Automation."
- The prompt-injection boundary in detail — how mail body content is kept
  from being interpreted as tool-call instructions, with examples.
- The bearer-token requirement for binding the MCP listener beyond loopback,
  including the exact refusal path and how the token is verified.
- Attachment-write path traversal defenses, with the specific validation
  rules.
- Supply-chain posture: `cargo-audit`/`cargo-deny` policy and what triggers a
  release block vs. a tracked-but-non-blocking finding.
