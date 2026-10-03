#!/usr/bin/env bash
# Spike B3 (GH #303, increment-f): build the UNMODIFIED Linux test binary with
# the host's stock gcc + glibc (metal login user, NO sudo, NO packages), record
# its identity, and run it natively on the host Linux as a smoke test.
#
# Output: ./out/bin/igkmf-app (gitignored; the boot runner hands this exact
# file to Firecracker as the initrd).
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SRC="$INCREMENT/app-linux/igkmf_app.c"
readonly OUTD="$INCREMENT/out/bin"
readonly BIN="$OUTD/igkmf-app"

step() { printf '\n##### [build-binary] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

step "toolchain (stock Ubuntu packages, unmodified)"
gcc --version | head -1
ld --version | head -1
ldd --version | head -1
dpkg-query -W -f='${Package} ${Version}\n' gcc-15 libc6 libc6-dev binutils 2>&1
sha256sum "$SRC"
echo "--- the source includes only standard Linux/POSIX/glibc headers:"
grep -E '^#include' "$SRC"
if grep -nE 'uk/|ukplat|unikraft' "$SRC" | grep -v '^\s*[0-9]*: \*'; then
  echo "NOTE: the matches above are comments only if every line starts with ' *'"
fi

step "compile: gcc -static-pie (the form app-elfloader loads: ET_DYN, no PT_INTERP)"
mkdir -p "$OUTD"
CMD=(gcc -O2 -g0 -Wall -Wextra -fPIE -static-pie -o "$BIN" "$SRC")
echo "${CMD[*]}"
"${CMD[@]}" 2>&1
echo "exit=$?"

step "identity of the produced ELF"
file "$BIN"
sha256sum "$BIN"
ls -l "$BIN"
readelf -h "$BIN" | grep -E 'Class|OS/ABI|Type|Machine|Entry'
echo "--- program headers (no INTERP expected; DYNAMIC present for self-relocation)"
readelf -lW "$BIN" | grep -E '^\s+(LOAD|INTERP|DYNAMIC|TLS|GNU_STACK|GNU_RELRO)'
echo "--- dynamic section NEEDED entries (none expected):"
readelf -dW "$BIN" | grep -E 'NEEDED' || echo "  (none)"
echo "--- strings containing 'unikraft' / 'ukplat' (0 expected):"
{ strings -a "$BIN" | grep -ciE 'unikraft|ukplat' || true; }
echo "--- glibc version string embedded:"
{ strings -a "$BIN" | grep -m1 -E 'GNU C Library' || true; }

step "native smoke test on the host Linux: ./igkmf-app id"
"$BIN" id
echo "exit=$?"
