#!/usr/bin/env bash
# Spike C (GH #303, increment-h): build the STATIC guest app (no libc in the
# initramfs). Output to the unsynced scratch tree.
set -euo pipefail
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT="$HOME/igkm-spike-a/igkmh/out/bin"
mkdir -p "$OUT"
echo "=== gcc -static guest app ==="
gcc -static -O2 -Wall -o "$OUT/igkmh_app" "$INC/app/igkmh_app.c"
file "$OUT/igkmh_app"
file "$OUT/igkmh_app" | grep -q 'statically linked' || { echo "FATAL: app not static"; exit 2; }
ls -l "$OUT/igkmh_app"
