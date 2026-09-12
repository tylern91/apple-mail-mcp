#!/usr/bin/env bash
# install.sh — content-aware install for amxcli
#
# cargo install --path skips reinstalling when the crate version hasn't changed.
# This script bypasses that version check: cargo build fingerprints source files,
# so a no-op build means no rebuild, and we copy whatever binary just came out
# into the cargo bin dir. No --force, no manual version bump required.
#
# Usage:
#   ./scripts/install.sh                       # dist profile (default)
#   AMX_PROFILE=release ./scripts/install.sh   # release profile
set -euo pipefail

PROFILE="${AMX_PROFILE:-dist}"
BIN_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"

# Build (cargo's fingerprint makes this a no-op when nothing changed)
cargo build --profile "$PROFILE" -p amx-cli "$@"

# Locate the output; 'dev' profile writes to target/debug, everything else to target/<profile>
if [[ "$PROFILE" == "dev" ]]; then
  OUT_DIR="target/debug"
else
  OUT_DIR="target/$PROFILE"
fi

# Atomic copy: install(1) writes to a temp file then renames, safe to replace a running binary
install -m 0755 "$OUT_DIR/amxcli" "$BIN_DIR/amxcli"

echo "installed amxcli → $BIN_DIR/amxcli ($("$BIN_DIR/amxcli" --version 2>/dev/null || echo 'version unknown'))"
