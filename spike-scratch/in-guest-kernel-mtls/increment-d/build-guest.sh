#!/usr/bin/env bash
# Spike B guest build (metal login user, NO sudo, NO system packages).
#
# Reuses Spike A3's user-space tools and clones under ~/igkm-spike-a/unikraft:
#   unikraft/           pristine clone at eb8fa236 (left untouched)
#   unikraft-igkmd/     git worktree of it at eb8fa236 + patches/0001 (this spike)
#   lib-lwip/           ec55ae17 (unpatched)
#   mbedtls/            Mbed TLS 3.6.7 LTS release tarball, sha256-pinned
#   lib-mtlsguard/      copy of ./lib-mtlsguard
#   app-igkmd/          copy of ./app
#   build-igkmd/        Unikraft O= build dir (from scratch every time)
#   out-igkmd/          the Firecracker image + debug image
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly W="$HOME/igkm-spike-a/unikraft"
readonly UK_PRISTINE="$W/unikraft"
readonly UK="$W/unikraft-igkmd"
readonly LWIP="$W/lib-lwip"
readonly GUARD="$W/lib-mtlsguard"
readonly APP="$W/app-igkmd"
readonly BUILD="$W/build-igkmd"
readonly OUT="$W/out-igkmd"
readonly UK_SHA=eb8fa2368618cea11c9bde196f79e6e6b9caeed5
readonly LWIP_SHA=ec55ae17618feeb57c8c10109bcf5c42723e8e95
readonly MBEDTLS_VER=3.6.7
readonly MBEDTLS_URL="https://github.com/Mbed-TLS/mbedtls/releases/download/mbedtls-$MBEDTLS_VER/mbedtls-$MBEDTLS_VER.tar.bz2"
# GitHub release asset digest for mbedtls-3.6.7.tar.bz2 (checked from the laptop too).
readonly MBEDTLS_SHA256=a7e8bcbec0e6f761b4af24f25677626b35f762f68eef79c08677a363212d11f6
readonly PATCH="$INCREMENT/patches/0001-posix-socket-connect-post-hook.patch"
readonly PATCH2="$INCREMENT/patches/0002-posix-socket-accept-post-hook.patch"

step() { printf '\n##### [build-guest] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }
export PATH="$W/tools/prefix/bin:$PATH"

step "pristine sources (Spike A3 clones)"
printf 'unikraft (pristine) HEAD=%s dirty=%s\n' "$(git -C "$UK_PRISTINE" rev-parse HEAD)" "$(git -C "$UK_PRISTINE" status --porcelain | wc -l)"
printf 'lib-lwip            HEAD=%s tree=%s dirty=%s\n' "$(git -C "$LWIP" rev-parse HEAD)" \
  "$(git -C "$LWIP" rev-parse 'HEAD^{tree}')" "$(git -C "$LWIP" status --porcelain | wc -l)"
[[ "$(git -C "$UK_PRISTINE" rev-parse HEAD)" == "$UK_SHA" ]]
[[ "$(git -C "$LWIP" rev-parse HEAD)" == "$LWIP_SHA" ]]

step "patched Unikraft worktree $UK"
if [[ ! -e "$UK/.git" ]]; then
  git -C "$UK_PRISTINE" worktree add --detach "$UK" "$UK_SHA"
fi
[[ "$(git -C "$UK" rev-parse HEAD)" == "$UK_SHA" ]] || { echo "FATAL: worktree HEAD"; exit 1; }
sha256sum "$PATCH" "$PATCH2"
# Restore ONLY the three files the patches own (this spike's own worktree),
# require clean, then apply 0001 and 0002 in order.
git -C "$UK" checkout -q -- lib/posix-socket/socket.c lib/posix-socket/include/uk/socket.h lib/posix-socket/exportsyms.uk
[[ -z "$(git -C "$UK" status --porcelain)" ]] || { echo "FATAL: worktree dirty with something else"; git -C "$UK" status --porcelain; exit 1; }
git -C "$UK" apply --check "$PATCH"
git -C "$UK" apply "$PATCH"
echo "patch 0001 applied"
git -C "$UK" apply --check "$PATCH2"
git -C "$UK" apply "$PATCH2"
echo "patch 0002 applied"
echo "--- git diff --stat (worktree vs $UK_SHA)"
git -C "$UK" diff --stat
echo "--- full diff"
git -C "$UK" diff

step "Mbed TLS $MBEDTLS_VER (sha256-pinned release tarball)"
mkdir -p "$W/mbedtls"
TB="$W/mbedtls/mbedtls-$MBEDTLS_VER.tar.bz2"
[[ -f "$TB" ]] || curl -fsSL -o "$TB" "$MBEDTLS_URL"
echo "$MBEDTLS_SHA256  $TB" | sha256sum -c -
MBEDTLS_DIR="$W/mbedtls/mbedtls-$MBEDTLS_VER"
[[ -d "$MBEDTLS_DIR" ]] || tar -xjf "$TB" -C "$W/mbedtls"
grep -m1 'MBEDTLS_VERSION_STRING ' "$MBEDTLS_DIR/include/mbedtls/build_info.h"

step "lib-mtlsguard + app sources"
mkdir -p "$GUARD" "$APP"
rsync -a --delete "$INCREMENT/lib-mtlsguard/" "$GUARD/"
rsync -a --delete --exclude=.config --exclude=.config.old "$INCREMENT/app/" "$APP/"
rm -f "$APP/.config" "$APP/.config.old"
( cd "$INCREMENT" && sha256sum lib-mtlsguard/guard.c lib-mtlsguard/Makefile.uk lib-mtlsguard/Config.uk \
    lib-mtlsguard/include/igkmd_mbedtls_config.h app/main.c app/defconfig app/Makefile.uk )
wc -l "$INCREMENT/lib-mtlsguard/guard.c" "$INCREMENT/app/main.c"

for t in gcc make ld flex bison m4; do
  printf '%-6s %s  %s\n' "$t" "$(command -v "$t")" "$("$t" --version 2>&1 | head -1)"
done

MK=(make -C "$UK" A="$APP" L="$LWIP:$GUARD" O="$BUILD" C="$APP/.config"
    LEX=flex YACC=bison UK_CFLAGS=-std=gnu17 MTLSGUARD_MBEDTLS_DIR="$MBEDTLS_DIR")

step "clean build dir"
rm -rf -- "$BUILD"

step "configure: make defconfig"
"${MK[@]}" UK_DEFCONFIG="$APP/defconfig" defconfig > "$W/igkmd-defconfig.log" 2>&1 || {
  echo "defconfig FAILED"; cat "$W/igkmd-defconfig.log"; exit 1; }
tail -n 3 "$W/igkmd-defconfig.log"

step "required symbols"
missing=0
for s in PLAT_KVM KVM_VMM_FIRECRACKER KVM_BOOT_PROTO_LXBOOT LIBVIRTIO_MMIO LIBVIRTIO_NET \
         LIBVIRTIO_VSOCK LIBUKVSOCKDEV LIBUKNETDEV LIBUKLIBPARAM LIBPOSIX_SOCKET LIBLWIP \
         LWIP_SOCKET LWIP_TCP LIBMTLSGUARD LIBPOSIX_TIME LIBUKPS2_SYSRESET; do
  if grep -qx "CONFIG_$s=y" "$APP/.config"; then echo "  CONFIG_$s=y"; else echo "  MISSING CONFIG_$s"; missing=1; fi
done
[[ "$missing" -eq 0 ]] || { echo "FATAL: required symbol missing"; exit 1; }

step "full resolved .config (sha256 $(sha256sum "$APP/.config" | cut -d' ' -f1))"
echo "----- BEGIN .config -----"
cat "$APP/.config"
echo "----- END .config -----"

step "build: make -j$(nproc)"
if ! "${MK[@]}" -j"$(nproc)" > "$W/igkmd-make.log" 2>&1; then
  echo "make FAILED; errors and last 80 lines:"
  grep -nE 'error:|Error [0-9]|undefined reference|No rule' "$W/igkmd-make.log" | head -60
  tail -n 80 "$W/igkmd-make.log"
  exit 1
fi
echo "compiler warnings: $(grep -cE 'warning:' "$W/igkmd-make.log" || true)"
grep -E 'warning:' "$W/igkmd-make.log" | sed "s#$HOME#~#g" | head -40 || true
tail -n 8 "$W/igkmd-make.log"

step "image identity"
IMG="$BUILD/igkmd_fc-x86_64"
mkdir -p "$OUT"
cp -f "$IMG" "$OUT/igkmd_fc-x86_64"
cp -f "$IMG.dbg" "$OUT/igkmd_fc-x86_64.dbg"
sha256sum "$OUT/igkmd_fc-x86_64" "$OUT/igkmd_fc-x86_64.dbg"
ls -l "$OUT"
readelf -h "$OUT/igkmd_fc-x86_64" | grep -E 'Class|Type|Entry'
echo "Xen notes: $(readelf -n "$OUT/igkmd_fc-x86_64" | grep -c Xen || true)"
echo "--- hook symbol (must be ONE strong 'T' definition, from lib-mtlsguard)"
nm "$OUT/igkmd_fc-x86_64.dbg" | grep -E ' uk_socket_connect_hook$' || true
echo "--- call site: the call in connect() must target the strong (lib-mtlsguard) definition"
HOOK_T="$(nm "$OUT/igkmd_fc-x86_64.dbg" | awk '$2=="T" && $3=="uk_socket_connect_hook"{print $1}')"
echo "strong hook address: 0x$HOOK_T"
objdump -d --no-show-raw-insn --disassemble=__uk_syscall_r_connect "$OUT/igkmd_fc-x86_64.dbg" | grep -E 'call.*uk_socket_connect_hook' || echo "NO call to the hook found in __uk_syscall_r_connect"
echo "--- accept hook: one strong T, called from uk_sys_accept"
nm "$OUT/igkmd_fc-x86_64.dbg" | grep -E ' uk_socket_accept_hook$' || true
objdump -d --no-show-raw-insn --disassemble=uk_sys_accept "$OUT/igkmd_fc-x86_64.dbg" | grep -E 'call.*uk_socket_accept_hook' || echo "NO call to the accept hook found in uk_sys_accept"
echo "--- guard + Mbed TLS symbols present"
nm "$OUT/igkmd_fc-x86_64.dbg" | grep -cE ' (mbedtls_gcm_auth_decrypt|mbedtls_gcm_crypt_and_tag|mbedtls_aesni_has_support|mbedtls_gcm_self_test)$' || true
echo "--- lib-mtlsguard object sizes"
size "$BUILD/libmtlsguard.o" 2>/dev/null || ls -l "$BUILD"/libmtlsguard* 2>/dev/null

step "done"
echo "IMAGE=$OUT/igkmd_fc-x86_64"
