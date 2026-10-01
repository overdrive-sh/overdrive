#!/usr/bin/env bash
# Spike C (GH #303, increment-h): build ONE out-of-tree kernel module against
# the stock host kernel's headers. The guest runs the same kernel (the host
# bzImage), so the vermagic matches and the .ko insmods in the guest.
#   build-module.sh <module-basename>   e.g. igkmg_probe | igkmg_mtls
# Writes out/<module>.ko. Builds on the metal host (needs the kernel headers).
# Does NOT install anything or touch the host kernel.
set -euo pipefail
MOD="${1:?usage: build-module.sh <module-basename>}"
INC="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KREL="$(uname -r)"
KDIR="/lib/modules/$KREL/build"
SRC="$INC/module/$MOD.c"
# Outputs go to the UNSYNCED scratch tree (never the rsync'd source tree), so
# root-owned build artifacts can't break the next `cargo xtask metal` rsync.
OUT="$HOME/igkm-spike-a/igkmh/out/module"
BD="$OUT/build-$MOD"

[[ -f "$SRC" ]] || { echo "FATAL: no source $SRC"; exit 2; }
[[ -e "$KDIR/Makefile" ]] || { echo "FATAL: no kernel headers at $KDIR"; exit 2; }

rm -rf "$BD"; mkdir -p "$BD"
cp "$SRC" "$BD/"
printf 'obj-m := %s.o\nccflags-y += -Wall\n' "$MOD" > "$BD/Kbuild"

echo "=== building $MOD.ko against $KDIR (gcc $(gcc -dumpversion)) ==="
make -C "$KDIR" M="$BD" modules 2>&1
cp "$BD/$MOD.ko" "$OUT/$MOD.ko"
echo "=== built ==="
ls -l "$OUT/$MOD.ko"
modinfo "$OUT/$MOD.ko" 2>&1 | grep -E '^(filename|vermagic|license|depends)'
echo "host kernel vermagic: $(cat "$KDIR/include/config/kernel.release")"
