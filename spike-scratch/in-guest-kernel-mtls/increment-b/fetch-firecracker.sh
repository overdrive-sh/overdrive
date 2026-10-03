#!/usr/bin/env bash
# Spike A2 prep (metal login user, NO sudo, NO system packages).
#
#   fetch-firecracker.sh VERSION [VERSION...]     e.g. v1.17.0
#
# Downloads the official static Firecracker release tarball and its published
# .sha256.txt from GitHub releases into $HOME/igkm-spike-a/firecracker/<ver>/
# (outside the rsynced ~/overdrive tree), verifies the checksum, extracts it, and
# records the binary's SHA-256 and --version. The jailer shipped in the tarball
# is extracted but never used. Skips a version already fetched and verified.
set -euo pipefail

readonly FCROOT="$HOME/igkm-spike-a/firecracker"
[[ $# -gt 0 ]] || { echo "usage: fetch-firecracker.sh VERSION..." >&2; exit 2; }

for VER in "$@"; do
  printf '\n##### [fetch] firecracker %s  (%s)\n' "$VER" "$(date -u +%H:%M:%S)"
  DIR="$FCROOT/$VER"
  TGZ="firecracker-$VER-x86_64.tgz"
  BASE="https://github.com/firecracker-microvm/firecracker/releases/download/$VER"
  mkdir -p "$DIR"
  cd "$DIR"
  if [[ ! -f "$TGZ" || ! -f "$TGZ.sha256.txt" ]]; then
    curl -fsSL -o "$TGZ" "$BASE/$TGZ"
    curl -fsSL -o "$TGZ.sha256.txt" "$BASE/$TGZ.sha256.txt"
  fi
  echo "--- published checksum file"
  cat "$TGZ.sha256.txt"
  echo "--- verify"
  sha256sum -c "$TGZ.sha256.txt"
  rm -rf "release-$VER-x86_64"
  tar -xzf "$TGZ"
  ls -l "release-$VER-x86_64"
  BIN="$DIR/release-$VER-x86_64/firecracker-$VER-x86_64"
  echo "--- binary identity"
  sha256sum "$BIN"
  file "$BIN"
  "$BIN" --version
done
