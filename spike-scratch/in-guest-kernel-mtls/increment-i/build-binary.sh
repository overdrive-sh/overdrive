#!/usr/bin/env bash
# Spike D (GH #303, increment-i): build the STATIC guest app (no libc in the
# initramfs). Output to the unsynced scratch tree.
set -euo pipefail
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HOME/igkm-spike-a/igkmd/out/bin"
mkdir -p "$OUT"
echo "=== gcc -static guest app ==="
gcc -static -O2 -Wall -o "$OUT/igkmd_app" "$INC/app/igkmd_app.c"
file "$OUT/igkmd_app"
file "$OUT/igkmd_app" | grep -q 'statically linked' || { echo "FATAL: app not static"; exit 2; }
ls -l "$OUT/igkmd_app"
