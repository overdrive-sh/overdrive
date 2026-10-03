#!/usr/bin/env bash
# Spike A3 build (metal login user, NO sudo, NO system packages).
#
# Builds the native Unikraft app in ./app with Unikraft's own make/Kconfig
# system (kraft is not used) against the clones build-tools.sh made:
#
#   $HOME/igkm-spike-a/unikraft/app-igkmc     copy of ./app (outside the rsynced tree)
#   $HOME/igkm-spike-a/unikraft/build-igkmc   Unikraft O= build dir
#   $HOME/igkm-spike-a/unikraft/out/          the Firecracker image + debug image
#
# Prints the full resolved .config and the image's ELF identity (entry, notes,
# program headers) so the capture carries the configuration and boot contract.
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly W="$HOME/igkm-spike-a/unikraft"
readonly UK="$W/unikraft"
readonly LWIP="$W/lib-lwip"
readonly APP="$W/app-igkmc"
readonly BUILD="$W/build-igkmc"
readonly OUT="$W/out"

step() { printf '\n##### [build] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }

export PATH="$W/tools/prefix/bin:$PATH"
mkdir -p "$OUT"

step "sources"
printf 'unikraft HEAD=%s tree=%s dirty=%s\n' "$(git -C "$UK" rev-parse HEAD)" \
  "$(git -C "$UK" rev-parse 'HEAD^{tree}')" "$(git -C "$UK" status --porcelain | wc -l)"
printf 'lib-lwip HEAD=%s tree=%s dirty=%s\n' "$(git -C "$LWIP" rev-parse HEAD)" \
  "$(git -C "$LWIP" rev-parse 'HEAD^{tree}')" "$(git -C "$LWIP" status --porcelain | wc -l)"
echo "fork-lwip branch head at build time (lib-lwip default LWIP_UNIKRAFT21X downloads this branch zip):"
git ls-remote https://github.com/unikraft/fork-lwip.git refs/heads/UNIKRAFT-2_1_x
for t in gcc make ld flex bison m4 python3; do
  printf '%-8s %s  %s\n' "$t" "$(command -v "$t")" "$("$t" --version 2>&1 | head -1)"
done

step "app sources -> $APP"
mkdir -p "$APP"
rsync -a --delete --exclude=.config --exclude=.config.old "$INCREMENT/app/" "$APP/"
rm -f "$APP/.config" "$APP/.config.old"
sha256sum "$APP"/main.c "$APP"/defconfig "$APP"/Makefile.uk "$APP"/Config.uk

# LEX/YACC on the command line: before a .config exists the top-level Makefile
# has not yet set `LEX := flex` / `YACC := bison` (they sit inside the
# UK_HAVE_DOT_CONFIG block, unikraft:Makefile:589,677-678), so the Kconfig
# sub-make gets make's built-in defaults `lex`/`yacc` (run 0003: `lex: not found`).
# UK_CFLAGS=-std=gnu17: gcc 15 defaults to C23, where `void (*)()` means
# `void (*)(void)`; Unikraft's `typedef void (*uk_ctor_func_t)();`
# (unikraft:include/uk/ctors.h:45) then fails at lib/ukboot/boot.c:489
# (`(*ctorfn)(argc, argv)`, run 0004). UK_CFLAGS is Unikraft's documented hook
# for extra C flags (unikraft:Makefile:716,1239).
MK=(make -C "$UK" A="$APP" L="$LWIP" O="$BUILD" C="$APP/.config" LEX=flex YACC=bison UK_CFLAGS=-std=gnu17)

step "clean build dir (every build is from scratch)"
rm -rf -- "$BUILD"

step "configure: make defconfig UK_DEFCONFIG=$APP/defconfig"
"${MK[@]}" UK_DEFCONFIG="$APP/defconfig" defconfig > "$W/igkmc-defconfig.log" 2>&1 || {
  echo "defconfig FAILED"; cat "$W/igkmc-defconfig.log"; exit 1; }
tail -n 5 "$W/igkmc-defconfig.log"

step "required symbols in the resolved .config"
missing=0
for s in PLAT_KVM KVM_VMM_FIRECRACKER KVM_BOOT_PROTO_LXBOOT VIRTIO_MMIO_LINUX_COMPAT_CMDLINE \
         LIBVIRTIO_MMIO LIBVIRTIO_NET LIBVIRTIO_VSOCK LIBUKVSOCKDEV LIBUKNETDEV \
         LIBUKNETDEV_EINFO_LIBPARAM LIBUKLIBPARAM LIBPOSIX_SOCKET LIBLWIP LWIP_UKNETDEV \
         LWIP_SOCKET LWIP_TCP LIBNS16550 LIBNS16550_EARLY_CONSOLE LIBUKPM LIBUKPS2_SYSRESET \
         LIBUKPRINT_KLVL_INFO; do
  if grep -qx "CONFIG_$s=y" "$APP/.config"; then echo "  CONFIG_$s=y"; else echo "  MISSING CONFIG_$s"; missing=1; fi
done
for s in KVM_BOOT_PROTO_MULTIBOOT KVM_BOOT_PROTO_EFI_STUB LIBVIRTIO_PCI LIBUKACPI OPTIMIZE_PIE; do
  printf '  %-34s %s\n' "CONFIG_$s" "$(grep -E "^(# )?CONFIG_$s[= ]" "$APP/.config" || echo '(absent)')"
done
[[ "$missing" -eq 0 ]] || { echo "FATAL: required symbol missing"; exit 1; }

step "full resolved .config (sha256 $(sha256sum "$APP/.config" | cut -d' ' -f1))"
echo "----- BEGIN .config -----"
cat "$APP/.config"
echo "----- END .config -----"

step "build: make -j$(nproc)"
if ! "${MK[@]}" -j"$(nproc)" > "$W/igkmc-make.log" 2>&1; then
  echo "make FAILED; errors and last 80 lines:"
  grep -nE 'error:|Error [0-9]|undefined reference|No rule' "$W/igkmc-make.log" | head -40
  tail -n 80 "$W/igkmc-make.log"
  exit 1
fi
echo "compiler warnings: $(grep -cE 'warning:' "$W/igkmc-make.log" || true)"
tail -n 15 "$W/igkmc-make.log"

step "fetched lwIP archive"
find "$BUILD/liblwip" -maxdepth 1 -type f -name '*.zip' -exec sha256sum {} \;

step "image identity"
IMG="$BUILD/igkmc_fc-x86_64"
ls -l "$IMG" "$IMG.dbg"
cp -f "$IMG" "$OUT/igkmc_fc-x86_64"
cp -f "$IMG.dbg" "$OUT/igkmc_fc-x86_64.dbg"
sha256sum "$OUT/igkmc_fc-x86_64" "$OUT/igkmc_fc-x86_64.dbg"
file "$OUT/igkmc_fc-x86_64"
echo "--- readelf -h"
readelf -h "$OUT/igkmc_fc-x86_64" | sed -n '1,20p'
echo "--- readelf -n (a PVH note would be Xen type 0x12; expect none)"
readelf -n "$OUT/igkmc_fc-x86_64" || true
echo "Xen notes: $(readelf -n "$OUT/igkmc_fc-x86_64" | grep -c Xen)"
echo "--- readelf -l"
readelf -lW "$OUT/igkmc_fc-x86_64"
echo "--- entry symbol (debug image)"
nm "$OUT/igkmc_fc-x86_64.dbg" | grep -E ' (_lxboot_entry|lxboot_entry|_multiboot_entry|uk_efi_entry64|main)$' || true
echo "--- bzImage setup-header magic at 0x202 (expect NOT 'HdrS': this is a plain ELF)"
dd if="$OUT/igkmc_fc-x86_64" bs=1 skip=$((0x202)) count=4 2>/dev/null | od -An -c

step "done"
echo "IMAGE=$OUT/igkmc_fc-x86_64"
