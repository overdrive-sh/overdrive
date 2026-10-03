#!/usr/bin/env bash
# Spike A prep (runs as the metal login user, NO sudo, NO system packages).
#
# Builds, entirely in user space under $HOME/igkm-spike-a (outside the rsynced
# ~/overdrive tree so `rsync --delete` cannot remove it):
#   - nasm from the upstream release tarball (host lacks nasm; no apt)
#   - Nanos kernel.img (PVH ELF), boot.img, mkfs at SHA aad473aa
#   - the static guest program
#   - the Nanos disk image (boot.img + bootfs(kernel) + rootfs(program))
# Each step is skipped if its output already exists, so re-runs are cheap.
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly SCRATCH="$HOME/igkm-spike-a"
readonly NANOS_SHA=aad473aad2f73a9893d5c32be333f2796725ab82
readonly NASM_VER=2.16.03
readonly NASM_URL="https://www.nasm.us/pub/nasm/releasebuilds/$NASM_VER/nasm-$NASM_VER.tar.xz"
readonly OUT="$SCRATCH/out"

step() { printf '\n##### [build] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

mkdir -p "$SCRATCH/src" "$SCRATCH/bin" "$OUT"
export PATH="$SCRATCH/bin:$PATH"

step "nasm $NASM_VER"
if [[ ! -x "$SCRATCH/bin/nasm" ]]; then
  cd "$SCRATCH/src"
  curl -fsSL -o "nasm-$NASM_VER.tar.xz" "$NASM_URL"
  sha256sum "nasm-$NASM_VER.tar.xz"
  rm -rf "nasm-$NASM_VER"
  tar -xf "nasm-$NASM_VER.tar.xz"
  cd "nasm-$NASM_VER"
  ./configure --prefix="$SCRATCH/nasm-prefix" >/dev/null
  make -j"$(nproc)" nasm >/dev/null
  install -m 0755 nasm "$SCRATCH/bin/nasm"
fi
sha256sum "$SCRATCH/src/nasm-$NASM_VER.tar.xz" 2>/dev/null || true
nasm -v

step "nanos source at $NANOS_SHA"
if [[ ! -d "$SCRATCH/nanos/.git" ]]; then
  git clone --quiet https://github.com/nanovms/nanos.git "$SCRATCH/nanos"
fi
git -C "$SCRATCH/nanos" -c advice.detachedHead=false checkout --quiet "$NANOS_SHA"
HEAD_SHA="$(git -C "$SCRATCH/nanos" rev-parse HEAD)"
echo "nanos HEAD=$HEAD_SHA"
[[ "$HEAD_SHA" == "$NANOS_SHA" ]] || { echo "FATAL: nanos HEAD mismatch"; exit 1; }
git -C "$SCRATCH/nanos" log -1 --format='nanos commit date=%cI subject=%s'
echo "nanos worktree dirty files: $(git -C "$SCRATCH/nanos" status --porcelain | wc -l)"

NANOS_OBJ="$SCRATCH/nanos/output/platform/pc"
KERNEL="$NANOS_OBJ/bin/kernel.img"
BOOTIMG="$NANOS_OBJ/boot/boot.img"
MKFS="$SCRATCH/nanos/output/tools/bin/mkfs"

step "nanos make kernel (boot.img + kernel.img)"
if [[ ! -f "$KERNEL" || ! -f "$BOOTIMG" ]]; then
  make -C "$SCRATCH/nanos" -j"$(nproc)" kernel > "$SCRATCH/nanos-make-kernel.log" 2>&1 || {
    echo "make kernel FAILED; last 60 lines:"; tail -n 60 "$SCRATCH/nanos-make-kernel.log"; exit 1; }
fi
step "nanos make mkfs"
if [[ ! -x "$MKFS" ]]; then
  make -C "$SCRATCH/nanos" mkfs > "$SCRATCH/nanos-make-mkfs.log" 2>&1 || {
    echo "make mkfs FAILED; last 60 lines:"; tail -n 60 "$SCRATCH/nanos-make-mkfs.log"; exit 1; }
fi
for v in acpica lwip mbedtls; do
  printf 'vendored %-8s %s\n' "$v" "$(git -C "$SCRATCH/nanos/vendor/$v" rev-parse HEAD 2>/dev/null || echo '?')"
done

step "artifact identity"
ls -l "$KERNEL" "$BOOTIMG" "$MKFS"
sha256sum "$KERNEL" "$BOOTIMG"
file "$KERNEL" "$BOOTIMG" || true
echo "--- readelf -h (kernel.img)"
readelf -h "$KERNEL" | sed -n '1,20p'
echo "--- readelf -n (kernel.img notes; PVH = Xen note type 0x12)"
readelf -n "$KERNEL"
echo "--- bzImage setup-header magic at 0x202 (expect NOT 'HdrS' for PVH route)"
dd if="$KERNEL" bs=1 skip=$((0x202)) count=4 2>/dev/null | od -An -c

step "guest program (static)"
gcc -static -O2 -Wall -Wextra -o "$OUT/vsock_probe" "$INCREMENT/guest/vsock_probe.c"
file "$OUT/vsock_probe"
sha256sum "$OUT/vsock_probe"

step "nanos disk image"
sed "s|@GUEST_BIN@|$OUT/vsock_probe|" "$INCREMENT/guest/vsock_probe.manifest.in" > "$OUT/vsock_probe.manifest"
cat "$OUT/vsock_probe.manifest"
rm -f "$OUT/nanos-vsock.img"
"$MKFS" -b "$BOOTIMG" -k "$KERNEL" "$OUT/nanos-vsock.img" < "$OUT/vsock_probe.manifest"
ls -l "$OUT/nanos-vsock.img"
sha256sum "$OUT/nanos-vsock.img"
echo "--- MBR partition table"
fdisk -l "$OUT/nanos-vsock.img" 2>&1 || true

step "done"
echo "KERNEL=$KERNEL"
echo "IMAGE=$OUT/nanos-vsock.img"
