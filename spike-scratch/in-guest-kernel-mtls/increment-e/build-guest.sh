#!/usr/bin/env bash
# Spike B2 guest build (GH #303, increment-e; metal login user, NO sudo, NO
# system packages).
#
# Reuses Spike A3/B's user-space tools and clones under ~/igkm-spike-a/unikraft
# and keeps Spike B's own trees untouched:
#   unikraft/            pristine clone at eb8fa236 (left untouched)
#   unikraft-igkmd/      Spike B's worktree (left untouched)
#   unikraft-igkme/      NEW worktree at eb8fa236 + increment-d patches 0001+0002
#   lib-lwip/            pristine clone at ec55ae17 (left untouched; Spike B uses it)
#   lib-lwip-igkme/      NEW worktree of lib-lwip at ec55ae17 + increment-e patch 0003
#   mbedtls/             Mbed TLS 3.6.7 LTS release tarball, sha256-pinned (shared)
#   lib-mtlsguard-igkme/ copy of ./lib-mtlsguard
#   app-igkme/           copy of ./app
#   build-igkme/         Unikraft O= build dir (from scratch every time)
#   out-igkme/           the Firecracker image + debug image
set -euo pipefail

readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly W="$HOME/igkm-spike-a/unikraft"
readonly UK_PRISTINE="$W/unikraft"
readonly UK="$W/unikraft-igkme"
readonly LWIP_PRISTINE="$W/lib-lwip"
readonly LWIP="$W/lib-lwip-igkme"
readonly GUARD="$W/lib-mtlsguard-igkme"
readonly APP="$W/app-igkme"
readonly BUILD="$W/build-igkme"
readonly OUT="$W/out-igkme"
readonly UK_SHA=eb8fa2368618cea11c9bde196f79e6e6b9caeed5
readonly LWIP_SHA=ec55ae17618feeb57c8c10109bcf5c42723e8e95
readonly MBEDTLS_VER=3.6.7
readonly MBEDTLS_SHA256=a7e8bcbec0e6f761b4af24f25677626b35f762f68eef79c08677a363212d11f6
# Spike B's core patches, reused read-only (never edited here).
readonly PATCH1="$INCREMENT/../increment-d/patches/0001-posix-socket-connect-post-hook.patch"
readonly PATCH2="$INCREMENT/../increment-d/patches/0002-posix-socket-accept-post-hook.patch"
readonly PATCH1_SHA=e3b35083
readonly PATCH3="$INCREMENT/patches/0003-lib-lwip-readiness-interposition.patch"

step() { printf '\n##### [build-guest] %s  (%s)\n' "$*" "$(date -u +%H:%M:%S)"; }
export PATH="$W/tools/prefix/bin:$PATH"

step "pristine sources (Spike A3 clones; must be clean at the pinned SHAs)"
printf 'unikraft (pristine) HEAD=%s dirty=%s\n' "$(git -C "$UK_PRISTINE" rev-parse HEAD)" "$(git -C "$UK_PRISTINE" status --porcelain | wc -l)"
printf 'lib-lwip (pristine) HEAD=%s dirty=%s\n' "$(git -C "$LWIP_PRISTINE" rev-parse HEAD)" "$(git -C "$LWIP_PRISTINE" status --porcelain | wc -l)"
[[ "$(git -C "$UK_PRISTINE" rev-parse HEAD)" == "$UK_SHA" ]]
[[ "$(git -C "$LWIP_PRISTINE" rev-parse HEAD)" == "$LWIP_SHA" ]]
[[ -z "$(git -C "$UK_PRISTINE" status --porcelain)" ]]
[[ -z "$(git -C "$LWIP_PRISTINE" status --porcelain)" ]]
echo "--- Spike B worktree (must still be exactly patches 0001+0002)"
git -C "$W/unikraft-igkmd" diff --stat

step "patches"
sha256sum "$PATCH1" "$PATCH2" "$PATCH3"
[[ "$(sha256sum "$PATCH1" | cut -c1-8)" == "$PATCH1_SHA" ]] || { echo "FATAL: increment-d patch 0001 changed"; exit 1; }

step "Unikraft worktree $UK = eb8fa236 + increment-d 0001 + 0002"
if [[ ! -e "$UK/.git" ]]; then
  git -C "$UK_PRISTINE" worktree add --detach "$UK" "$UK_SHA"
fi
[[ "$(git -C "$UK" rev-parse HEAD)" == "$UK_SHA" ]] || { echo "FATAL: worktree HEAD"; exit 1; }
git -C "$UK" checkout -q -- lib/posix-socket/socket.c lib/posix-socket/include/uk/socket.h lib/posix-socket/exportsyms.uk
[[ -z "$(git -C "$UK" status --porcelain)" ]] || { echo "FATAL: worktree dirty with something else"; git -C "$UK" status --porcelain; exit 1; }
git -C "$UK" apply --check "$PATCH1" && git -C "$UK" apply "$PATCH1"
git -C "$UK" apply --check "$PATCH2" && git -C "$UK" apply "$PATCH2"
git -C "$UK" diff --stat

step "lib-lwip worktree $LWIP = ec55ae17 + increment-e 0003"
if [[ ! -e "$LWIP/.git" ]]; then
  git -C "$LWIP_PRISTINE" worktree add --detach "$LWIP" "$LWIP_SHA"
fi
[[ "$(git -C "$LWIP" rev-parse HEAD)" == "$LWIP_SHA" ]] || { echo "FATAL: lib-lwip worktree HEAD"; exit 1; }
git -C "$LWIP" reset -q
git -C "$LWIP" checkout -q -- sockets.c
rm -f "$LWIP/include/uk/lwip_readiness.h"
rmdir "$LWIP/include/uk" 2>/dev/null || true
[[ -z "$(git -C "$LWIP" status --porcelain)" ]] || { echo "FATAL: lib-lwip worktree dirty"; git -C "$LWIP" status --porcelain; exit 1; }
git -C "$LWIP" apply --check "$PATCH3"
git -C "$LWIP" apply "$PATCH3"
echo "--- git diff --stat (lib-lwip worktree vs $LWIP_SHA, incl. the new header)"
git -C "$LWIP" add -N include/uk/lwip_readiness.h
git -C "$LWIP" diff --stat
echo "--- full diff"
git -C "$LWIP" diff

step "Mbed TLS $MBEDTLS_VER (sha256-pinned, shared with Spike B)"
TB="$W/mbedtls/mbedtls-$MBEDTLS_VER.tar.bz2"
echo "$MBEDTLS_SHA256  $TB" | sha256sum -c -
MBEDTLS_DIR="$W/mbedtls/mbedtls-$MBEDTLS_VER"
[[ -d "$MBEDTLS_DIR" ]]
grep -m1 'MBEDTLS_VERSION_STRING ' "$MBEDTLS_DIR/include/mbedtls/build_info.h"
sha256sum "$MBEDTLS_DIR"/library/{aes,aesni,gcm,block_cipher,constant_time,platform_util,sha256,md,hkdf}.c

step "lib-mtlsguard + app sources"
mkdir -p "$GUARD" "$APP"
rsync -a --delete "$INCREMENT/lib-mtlsguard/" "$GUARD/"
rsync -a --delete --exclude=.config --exclude=.config.old "$INCREMENT/app/" "$APP/"
rm -f "$APP/.config" "$APP/.config.old"
( cd "$INCREMENT" && sha256sum lib-mtlsguard/guard.c lib-mtlsguard/Makefile.uk lib-mtlsguard/Config.uk \
    lib-mtlsguard/include/igkme_mbedtls_config.h app/main.c app/defconfig app/Makefile.uk )
wc -l "$INCREMENT/lib-mtlsguard/guard.c" "$INCREMENT/app/main.c"

for t in gcc make ld flex bison m4; do
  printf '%-6s %s  %s\n' "$t" "$(command -v "$t")" "$("$t" --version 2>&1 | head -1)"
done

MK=(make -C "$UK" A="$APP" L="$LWIP:$GUARD" O="$BUILD" C="$APP/.config"
    LEX=flex YACC=bison UK_CFLAGS=-std=gnu17 MTLSGUARD_MBEDTLS_DIR="$MBEDTLS_DIR")

step "clean build dir"
rm -rf -- "$BUILD"

step "configure: make defconfig"
"${MK[@]}" UK_DEFCONFIG="$APP/defconfig" defconfig > "$W/igkme-defconfig.log" 2>&1 || {
  echo "defconfig FAILED"; cat "$W/igkme-defconfig.log"; exit 1; }
tail -n 3 "$W/igkme-defconfig.log"

step "required symbols"
missing=0
for s in PLAT_KVM KVM_VMM_FIRECRACKER KVM_BOOT_PROTO_LXBOOT LIBVIRTIO_MMIO LIBVIRTIO_NET \
         LIBVIRTIO_VSOCK LIBUKVSOCKDEV LIBUKNETDEV LIBUKLIBPARAM LIBPOSIX_SOCKET LIBLWIP \
         LWIP_SOCKET LWIP_TCP LIBMTLSGUARD LIBPOSIX_TIME LIBUKPS2_SYSRESET LIBPOSIX_POLL \
         LIBUKFILE_CHAINUPDATE LIBUKLOCK_MUTEX LIBUKSCHEDCOOP; do
  if grep -qx "CONFIG_$s=y" "$APP/.config"; then echo "  CONFIG_$s=y"; else echo "  MISSING CONFIG_$s"; missing=1; fi
done
[[ "$missing" -eq 0 ]] || { echo "FATAL: required symbol missing"; exit 1; }

step "full resolved .config (sha256 $(sha256sum "$APP/.config" | cut -d' ' -f1))"
echo "----- BEGIN .config -----"
cat "$APP/.config"
echo "----- END .config -----"

step "build: make -j$(nproc)"
if ! "${MK[@]}" -j"$(nproc)" > "$W/igkme-make.log" 2>&1; then
  echo "make FAILED; errors and last 80 lines:"
  grep -nE 'error:|Error [0-9]|undefined reference|No rule' "$W/igkme-make.log" | head -60
  tail -n 80 "$W/igkme-make.log"
  exit 1
fi
echo "compiler warnings: $(grep -cE 'warning:' "$W/igkme-make.log" || true)"
grep -E 'warning:' "$W/igkme-make.log" | sed "s#$HOME#~#g" | head -40 || true
tail -n 8 "$W/igkme-make.log"

step "image identity"
IMG="$BUILD/igkme_fc-x86_64"
mkdir -p "$OUT"
cp -f "$IMG" "$OUT/igkme_fc-x86_64"
cp -f "$IMG.dbg" "$OUT/igkme_fc-x86_64.dbg"
sha256sum "$OUT/igkme_fc-x86_64" "$OUT/igkme_fc-x86_64.dbg"
ls -l "$OUT"
readelf -h "$OUT/igkme_fc-x86_64" | grep -E 'Class|Type|Entry'
DBG="$OUT/igkme_fc-x86_64.dbg"
for h in uk_socket_connect_hook uk_socket_accept_hook lwip_posix_socket_events_hook; do
  echo "--- $h: definitions (must be ONE strong 'T', from lib-mtlsguard; a local 't' means a weak default was localized)"
  nm "$DBG" | grep -E " $h\$" || echo "  (none)"
done
echo "--- call sites must target the strong definitions"
for pair in "__uk_syscall_r_connect uk_socket_connect_hook" "uk_sys_accept uk_socket_accept_hook" \
            "lwip_posix_socket_event_callback lwip_posix_socket_events_hook" "lwip_posix_socket_poll_setup lwip_posix_socket_events_hook"; do
  set -- $pair
  strong="$(nm "$DBG" | awk -v s="$2" '$2=="T" && $3==s{print $1}')"
  echo "  $1 -> $2 (strong at 0x$strong):"
  objdump -d --no-show-raw-insn --disassemble="$1" "$DBG" | grep -E "call.*<$2>" || echo "    NO call to $2 found in $1 (inlined? check)"
done
echo "--- guard + Mbed TLS symbols present"
nm "$DBG" | grep -E ' (mbedtls_gcm_auth_decrypt|mbedtls_gcm_crypt_and_tag|mbedtls_aesni_has_support|mbedtls_hkdf_expand|mbedtls_sha256_self_test|uk_pollq_assign_n)$' || true
echo "--- object sizes"
size "$BUILD/libmtlsguard.o" "$BUILD/liblwip.o" 2>/dev/null || ls -l "$BUILD"/libmtlsguard* "$BUILD"/liblwip* 2>/dev/null

step "done"
echo "IMAGE=$OUT/igkme_fc-x86_64"
