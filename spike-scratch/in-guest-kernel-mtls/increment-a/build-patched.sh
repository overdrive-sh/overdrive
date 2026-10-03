#!/usr/bin/env bash
# Spike A prep, second pass (login user, NO sudo). Builds two probe kernels in a
# separate git worktree so the unmodified aad473aa build stays pristine:
#   kernel-pvhfix.img           = aad473aa + patches/0001 (stack before first call)
#   kernel-pvhfix-sockdebug.img = the above + patches/0002 (VIRTIO_SOCK_DEBUG, diagnostic)
# Vendored third-party sources are copied from the first build so they are
# byte-identical across all three kernels.
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRATCH="$HOME/igkm-spike-a"
readonly NANOS_SHA=aad473aad2f73a9893d5c32be333f2796725ab82
readonly WT="$SCRATCH/nanos-pvhfix"
readonly OUT="$SCRATCH/out"
export PATH="$SCRATCH/bin:$PATH"

step() { printf '\n##### [build-patched] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

step "separate local clone $WT at $NANOS_SHA"
# A git worktree does not work: Nanos's make depends on $(ROOTDIR)/.git/index
# (gitversion.c), and a worktree's .git is a file (run 0004 failure).
if [[ -f "$WT/.git" ]]; then
  git -C "$SCRATCH/nanos" worktree remove --force "$WT"
fi
if [[ ! -d "$WT/.git" ]]; then
  git clone --quiet --no-checkout "$SCRATCH/nanos" "$WT"
  git -C "$WT" -c advice.detachedHead=false checkout --quiet "$NANOS_SHA"
  cp -a "$SCRATCH/nanos/vendor" "$WT/vendor"
fi
git -C "$WT" rev-parse HEAD
for v in acpica lwip mbedtls; do
  printf 'vendored %-8s %s\n' "$v" "$(git -C "$WT/vendor/$v" rev-parse HEAD)"
done
git -C "$WT" checkout --quiet -- src

step "kernel-pvhfix (patch 0001 only)"
git -C "$WT" apply --verbose "$INCREMENT/patches/0001-pvh-set-stack-before-first-call.patch"
git -C "$WT" diff --stat
make -C "$WT" -j"$(nproc)" kernel > "$SCRATCH/nanos-pvhfix-make.log" 2>&1 || {
  echo "make FAILED"; tail -n 60 "$SCRATCH/nanos-pvhfix-make.log"; exit 1; }
cp "$WT/output/platform/pc/bin/kernel.img" "$OUT/kernel-pvhfix.img"

step "kernel-pvhfix-sockdebug (patch 0001 + diagnostic 0002)"
git -C "$WT" apply --verbose "$INCREMENT/patches/0002-diagnostic-enable-virtio-sock-debug.patch"
git -C "$WT" diff
make -C "$WT" -j"$(nproc)" kernel > "$SCRATCH/nanos-pvhfix-sockdebug-make.log" 2>&1 || {
  echo "make FAILED"; tail -n 60 "$SCRATCH/nanos-pvhfix-sockdebug-make.log"; exit 1; }
cp "$WT/output/platform/pc/bin/kernel.img" "$OUT/kernel-pvhfix-sockdebug.img"

step "artifact identity"
sha256sum "$SCRATCH/nanos/output/platform/pc/bin/kernel.img" "$OUT/kernel-pvhfix.img" "$OUT/kernel-pvhfix-sockdebug.img"
set +e
for k in "$SCRATCH/nanos/output/platform/pc/bin/kernel.img" "$OUT/kernel-pvhfix.img" "$OUT/kernel-pvhfix-sockdebug.img"; do
  echo "--- $k"
  readelf -n "$k" | grep -A1 Xen
  echo "32-bit disassembly at the PVH entry 0x2000dc (patched: expect mov esp,0xa000 first):"
  objdump -D -M i386 --start-address=0x2000dc --stop-address=0x2000f0 "$k" | tail -n 6
done
set -e
step "done"
