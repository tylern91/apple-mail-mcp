# Contributing to apple-mail-mcp

Thanks for considering a contribution. This document covers everything a new
contributor needs to land a correct PR without having to ask.

## ⚠️ Before your first commit: GPG signing is mandatory

`main` is protected by a repository ruleset that requires every commit to be
GPG-signed (`required_signatures`, `bypass_actors: []`) — an unsigned commit
is rejected at push, not just at merge. Set up commit signing before you push
anything:

```sh
git config commit.gpgsign true
git config user.signingkey <your-key-id>
```

The same ruleset also enforces:

- **Squash-merge only** — the target branch never sees your individual
  commits, only one squashed commit per PR.
- **Linear history** — no merge commits.
- No force-push, no branch deletion on `main`.
- 0 required approvals (the maintainer merges solo today, but every other
  gate above still applies).

## Design principles

These aren't aspirational — they're enforced by CI or by the shape of the
code. Know them before proposing a change that cuts against one:

- **Local-first.** apple-mail-mcp sends no telemetry, and the Mail data path
  never leaves the machine. The only outbound network access is SMTP
  delivery, to the server your existing Mail.app account is already
  configured against.
- **Read-only by default against Apple's own store.** `amx-store` opens the
  Envelope Index `mode=ro`; every mutation (move, flag, delete) goes through
  Mail.app's own Automation interface, never a direct write to Apple's SQLite
  file. Don't add a code path that writes to the Envelope Index directly.
- **Mail bodies are untrusted input.** Returned message content is data, not
  instructions — a tool result must never be treated as a directive from the
  user. Send tools require explicit invocation with explicit recipients; see
  [SECURITY.md](SECURITY.md).
- **Never block a read on Mail.app being unreachable.** Search and read tools
  work directly against the on-disk store; only mutate/send tools need
  Mail.app running and the Automation permission granted.
- **macOS only, on purpose.** This isn't a portability gap to fix — the whole
  design assumes Mail.app's on-disk format and Apple Events. A "portable"
  crate here means "compiles on Linux CI as an empty stub," not "has a real
  Linux implementation."

## Ways to contribute

| Type | How |
|---|---|
| Report a bug | Open an issue with repro steps, `amxcli doctor` output, and macOS version |
| Fix a bug | See the local gate below, then open a PR |
| Add a feature | Consider opening an issue first for anything touching the mutate or send lanes |
| Review a PR | Check it against the design principles above, not just style |
| Improve docs | README, `docs/`, and this file all welcome fixes |

## Commit convention

[Conventional Commits](https://www.conventionalcommits.org/), using the types
and scopes actually in use in this repo:

- **Types:** `feat`, `fix`, `chore`, `ci`, `docs`, `perf`
- **Scopes:** `cli`, `mcp`, `store`, `parse`, `index`, `compose`, `send`,
  `automation`, `core`, `release`, `security`, `packaging`
- **Multiple scopes:** comma-join them — `fix(store,index): ...`
- Bare `docs:` / `chore:` with no scope are fine.

**Because merges are squash-only, your PR title becomes the commit message.**
Release automation regexes the title for breaking-change escalation (see
below), so title it as you'd want it to read in `CHANGELOG.md`.

## Branch naming

`<type>/<kebab-case-slug>` — `fix/`, `feat/`, `docs/`, `ci/`, and `chore/` are
all in use. This is a convention, not an enforced gate.

## Local gate before pushing

Run the exact commands CI runs, before you push:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo build --workspace
cargo test --workspace --lib
```

**CI only triggers on code changes.** `rust.yml` is path-filtered to
`crates/**`, `Cargo.toml`, `Cargo.lock`, `.cargo/**`, `CHANGELOG.md`,
`scripts/check-version-sync.sh`, and its own file — a docs-only PR (like this
one) runs no Rust CI at all. `security.yml` (cargo-audit, cargo-deny, Trivy)
runs on every PR regardless; only a **CRITICAL** vulnerability with a known
fix blocks the merge, HIGH severity is recorded to the Security tab but
doesn't block.

## MSRV policy

`rust-version = "1.88"` in the root `Cargo.toml` (`[workspace.package]`,
inherited by every crate) mirrors the floor established in the sibling
`rqmd` repo's resolved dependency graph. Don't raise it casually to pick up a
new API; if a dependency bump pushes this project's own floor higher, that's
when it moves.

- Raising the MSRV is a **minor** version bump — it can break users on older
  toolchains.
- The edition is 2024, which needs Rust ≥1.85 — below the 1.88 floor, so it
  costs nothing in compatibility and isn't itself an MSRV constraint.

## Version + CHANGELOG convention

This is the part that isn't written down anywhere else, so read it carefully:

A PR that should ship in a release **finalizes its own release section** in
`CHANGELOG.md`:

1. Add `## [X.Y.Z] - YYYY-MM-DD` above the previous release heading.
2. Bump `version` under `[workspace.package]` in the root `Cargo.toml` to
   match `X.Y.Z` exactly. `scripts/check-version-sync.sh` fails CI if the two
   disagree.
3. Leave `## [Unreleased]` in the file — **empty**, as a permanent
   placeholder. Never delete it: the release workflow's empty-notes guard
   silently no-ops a release if `## [Unreleased]` is missing entirely.
4. Use the existing buckets — `### Added` / `Changed` / `Fixed` /
   `Documentation` — followed by a `---`. Write bullets as prose explaining
   *why* the change matters.

## Semver labels

Apply exactly one on your PR: `patch`, `minor`, or `skip-release`.
**A PR with no label produces no release** — the release workflow simply
doesn't run.

Docs-only changes use `skip-release`, **not** `patch` — `patch` fails the
changelog gate on a PR that adds no `CHANGELOG.md` entry.

## Breaking changes

Escalate a `major`/`minor` label to a major version bump with any of:

- A PR title matching `^[a-z]+(\([^)]+\))?!:` (e.g. `feat(mcp)!: ...`)
- A `BREAKING CHANGE:` line in the PR body
- The `breaking-change` label

## No CLA, no DCO, no sign-off

Your PR is not gated on signing a Contributor License Agreement or adding a
`Signed-off-by` trailer. Submitting a PR is enough.

## Don't touch — looks usable, isn't

- **`.github/workflows/nix.yml`** — triggers only on a `release` branch that
  doesn't exist in this repo (same as in `rqmd`), so it never runs today. If
  you want a real Nix build gate, that's a welcome PR on its own — it needs
  either a `release` branch or a retargeted trigger, not an assumption that
  it's currently exercised.

`scripts/pre-push` and `scripts/crosscheck.sh` are **not** carried forward
as-is from `rqmd`: `pre-push` here checks the root `[workspace.package]`
version rather than grepping each crate's manifest (the pattern `rqmd`'s
version uses never matches `version.workspace = true` and would silently
no-op), and `crosscheck.sh` — which compares Rust `rqmd` against a
TypeScript `qmd` sibling — has no equivalent here and isn't present.
