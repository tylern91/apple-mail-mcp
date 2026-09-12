#!/usr/bin/env bash
# update-homebrew-formula.sh <version>
#
# Fetches the release SHA-256 checksum from GitHub, renders the Homebrew
# formula template at packaging/homebrew/amxcli.rb.template, and writes the
# result to packaging/homebrew/amxcli.rb (the template itself is never
# modified, so re-running this script always starts from the same
# placeholders).
#
# When HOMEBREW_TAP_TOKEN is set, also clones github.com/tylern91/homebrew-apple-mail-mcp
# and pushes the updated Formula/amxcli.rb.
#
# apple-mail-mcp only ships a macOS arm64 binary (it reads Mail.app's on-disk
# store and drives it via AppleScript — there is no Linux build), so unlike
# rqmd's template this fetches and substitutes a single checksum.
#
# Usage:
#   ./scripts/update-homebrew-formula.sh v0.3.0      # from the repo root
#   VERSION=v0.3.0 ./scripts/update-homebrew-formula.sh

set -euo pipefail

VERSION="${1:-${VERSION:-}}"
if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version>  (e.g. v0.3.0)" >&2
  exit 1
fi

# Normalize: always have the 'v' prefix for URLs / git tags,
# and the bare version (without 'v') for the formula version field.
VERSION="${VERSION#v}"          # strip leading v if present
TAG="v${VERSION}"               # canonical tag: v0.3.0
BARE="${VERSION}"               # bare: 0.3.0

REPO="tylern91/apple-mail-mcp"
BASE_URL="https://github.com/${REPO}/releases/download/${TAG}"
HOMEBREW_DIR="$(cd "$(dirname "$0")/.." && pwd)/packaging/homebrew"
TEMPLATE="${HOMEBREW_DIR}/amxcli.rb.template"
RENDERED="${HOMEBREW_DIR}/amxcli.rb"

if [[ ! -f "$TEMPLATE" ]]; then
  echo "Template not found: $TEMPLATE" >&2
  exit 1
fi

fetch_sha256() {
  local name_suffix="$1"
  local tarball="amx-cli-${TAG}-${name_suffix}.tar.gz"
  local sha_file="${tarball}.sha256"
  local url="${BASE_URL}/${sha_file}"
  echo "Fetching ${sha_file} …" >&2
  # The .sha256 file contains "<hash>  <filename>" — extract just the hash.
  curl -fsSL "$url" | awk '{print $1}'
}

SHA256_MACOS_ARM64="$(fetch_sha256 macos-arm64)"

echo "  macOS arm64 : ${SHA256_MACOS_ARM64}" >&2

# Substitute placeholders in the template.
UPDATED="$(sed \
  -e "s|AMXCLI_VERSION|${TAG}|g" \
  -e "s|AMXCLI_BARE_VERSION|${BARE}|g" \
  -e "s|AMXCLI_SHA256_MACOS_ARM64|${SHA256_MACOS_ARM64}|g" \
  "$TEMPLATE")"

# Write the rendered formula to its own path — never back over the template,
# or a second run would find no placeholders left to substitute and silently
# re-ship the previous run's (now stale) version/SHA values.
printf '%s\n' "$UPDATED" > "$RENDERED"
echo "Updated ${RENDERED}" >&2

# Optionally push to the Homebrew tap repo.
if [[ -n "${HOMEBREW_TAP_TOKEN:-}" ]]; then
  TAP_REPO="tylern91/homebrew-apple-mail-mcp"
  TAP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TAP_DIR"' EXIT

  echo "Cloning tap repo ${TAP_REPO} …" >&2
  git clone --depth 1 \
    "https://x-access-token:${HOMEBREW_TAP_TOKEN}@github.com/${TAP_REPO}.git" \
    "$TAP_DIR"

  mkdir -p "${TAP_DIR}/Formula"
  cp "$RENDERED" "${TAP_DIR}/Formula/amxcli.rb"

  git -C "$TAP_DIR" config user.name  "github-actions[bot]"
  git -C "$TAP_DIR" config user.email "tylern91@users.noreply.github.com"

  git -C "$TAP_DIR" add Formula/amxcli.rb
  if git -C "$TAP_DIR" diff --cached --quiet; then
    echo "No formula changes to push." >&2
  else
    git -C "$TAP_DIR" commit -m "chore: update amxcli formula to ${TAG}"
    git -C "$TAP_DIR" push
    echo "Pushed Formula/amxcli.rb to ${TAP_REPO}" >&2
  fi
fi
