#!/usr/bin/env bash
# Spike B3 stretch (GH #303, increment-f): build a real-world, unmodified
# program, BusyBox wget, from the upstream release tarball with the host's
# stock gcc + glibc as a static-pie ELF (the form app-elfloader loads). Metal
# login user, NO sudo, NO system packages. The tarball is verified against the
# sha256 file BusyBox publishes next to it.
#
# Output: ./out/bin/busybox (gitignored). Scratch: ~/igkm-spike-a/stretch/.
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly S="$HOME/igkm-spike-a/stretch"
readonly VER=1.37.0
readonly TB="busybox-$VER.tar.bz2"
readonly URL="https://busybox.net/downloads/$TB"
readonly OUTD="$INCREMENT/out/bin"

step() { printf '\n##### [build-busybox] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

step "fetch $URL (+ .sha256)"
mkdir -p "$S"
cd "$S"
[[ -f "$TB" ]] || curl -fsSL -o "$TB" "$URL"
curl -fsSL -o "$TB.sha256" "$URL.sha256"
cat "$TB.sha256"
sha256sum -c "$TB.sha256"
ls -l "$TB"

step "unpack + configure (allnoconfig + the busybox and wget applets only)"
rm -rf "busybox-$VER"
tar xjf "$TB"
cd "busybox-$VER"
make -s allnoconfig > /dev/null
set_cfg() { # name value(y|"string"|n)
  sed -i -E "/^(# )?CONFIG_$1[= ]/d" .config
  if [[ "$2" == n ]]; then echo "# CONFIG_$1 is not set" >> .config; else echo "CONFIG_$1=$2" >> .config; fi
}
set_cfg BUSYBOX y
set_cfg WGET y
# Run 0015: CONFIG_EXTRA_LDFLAGS also reaches BusyBox's partial `ld -r`
# links ("-r and -pie may not be used together"). Use BusyBox's own PIE knob
# (-fpie on every object) and give ONLY the final link -static-pie, through
# CFLAGS_busybox (the variable BusyBox reserves for the final link) on the
# make command line.
set_cfg STATIC n
set_cfg PIE y
set_cfg EXTRA_CFLAGS '"-std=gnu17"'
set_cfg EXTRA_LDFLAGS '""'
yes "" | make -s oldconfig > /dev/null 2>&1 || true
grep -E '^CONFIG_(BUSYBOX|WGET|FEATURE_WGET[A-Z_]*|STATIC|PIE|EXTRA_CFLAGS|EXTRA_LDFLAGS)[= ]|^# CONFIG_(STATIC|PIE|FEATURE_WGET[A-Z_]*) ' .config
sha256sum .config

step "build (host gcc $(gcc -dumpfullversion), glibc $(getconf GNU_LIBC_VERSION | cut -d' ' -f2))"
make -j"$(nproc)" CFLAGS_busybox=-static-pie > "$S/busybox-make.log" 2>&1 || { tail -n 60 "$S/busybox-make.log"; exit 1; }
grep -E 'trylink|static-pie' "$S/busybox-make.log" | head -5 || true
echo "warnings: $(grep -c 'warning:' "$S/busybox-make.log" || true)"
tail -n 5 "$S/busybox-make.log"

step "identity"
mkdir -p "$OUTD"
cp -f busybox "$OUTD/busybox"
file "$OUTD/busybox"
sha256sum "$OUTD/busybox"
ls -l "$OUTD/busybox"
readelf -h "$OUTD/busybox" | grep -E 'Type|Entry'
readelf -lW "$OUTD/busybox" | grep -E '^\s+(INTERP|DYNAMIC|TLS)' || true
readelf -dW "$OUTD/busybox" | grep NEEDED || echo "  NEEDED: (none)"
"$OUTD/busybox" --list

step "native smoke test on the host Linux"
"$OUTD/busybox" 2>&1 | head -3 || true
"$OUTD/busybox" wget --help 2>&1 | head -6 || true
