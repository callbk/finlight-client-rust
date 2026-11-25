#!/usr/bin/env bash
# Copies the finlight API v3 contract kit into conformance/.
#
# Usage (one command):
#
#   scripts/sync-conformance.sh [KIT_DIR]
#
# KIT_DIR is the kit directory (shared/contracts/api-v3 in the finlight monorepo).
# Default: $FINLIGHT_CONTRACT_KIT, else ../../../contracts/api-v3 (where the kit
# sits when this repository is the monorepo submodule).
#
# The source is checked against its own MANIFEST first. Then conformance/ is
# replaced by exactly the files the MANIFEST lists, plus the MANIFEST itself,
# and the copy is checked again. The test suite repeats that check on every run.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
kit="${1:-${FINLIGHT_CONTRACT_KIT:-$root/../../../contracts/api-v3}}"
target="$root/conformance"

if [ ! -f "$kit/MANIFEST" ]; then
  echo "no contract kit at $kit (pass the kit directory as the first argument)" >&2
  exit 1
fi
kit="$(cd "$kit" && pwd)"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | cut -d' ' -f1
  else
    shasum -a 256 "$1" | cut -d' ' -f1
  fi
}

verify() {
  local dir="$1" problems=0 digest rel
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    digest="${line%%  *}"
    rel="${line#*  }"
    if [ ! -f "$dir/$rel" ]; then
      echo "missing: $rel" >&2
      problems=1
    elif [ "$(sha256 "$dir/$rel")" != "$digest" ]; then
      echo "checksum differs: $rel" >&2
      problems=1
    fi
  done <"$dir/MANIFEST"
  while IFS= read -r file; do
    rel="${file#"$dir"/}"
    [ "$rel" = "MANIFEST" ] && continue
    if ! grep -q "  $rel\$" "$dir/MANIFEST"; then
      echo "not in MANIFEST: $rel" >&2
      problems=1
    fi
  done < <(find "$dir" -type f)
  if [ "$problems" -ne 0 ]; then
    echo "$dir does not match its MANIFEST" >&2
    exit 1
  fi
}

verify "$kit"
rm -rf "$target"
mkdir -p "$target"
while IFS= read -r line; do
  [ -z "$line" ] && continue
  rel="${line#*  }"
  mkdir -p "$(dirname "$target/$rel")"
  cp "$kit/$rel" "$target/$rel"
done <"$kit/MANIFEST"
cp "$kit/MANIFEST" "$target/MANIFEST"
verify "$target"
echo "conformance/ synced: contract $(tr -d '[:space:]' <"$target/CONTRACT_VERSION"), $(grep -c . "$target/MANIFEST") files"
