#!/usr/bin/env bash
# Spike B3 guest build (GH #303, increment-f; metal login user, NO sudo, NO
# system packages).
#
#   build-guest.sh [VARIANT]      VARIANT: main (default) | strace | go | go-strace
#
# main/strace: config/igkmf.defconfig (the checklist image; strace adds one
# line). go/go-strace (stretch only): the same plus config/igkmf-go.fragment
# (posix-futex, posix-eventfd, posix-pipe) for the Go runtime.
#
# The guest is Unikraft's app-elfloader (unmodified, pinned) plus Spike B2's
# mechanism: the core posix-socket connect/accept post-hooks (increment-d
# patches 0001+0002, read-only), the lib-lwip readiness hook (increment-e patch
# 0003, read-only) and lib-mtlsguard (increment-f copy = B2's library + an
# initrd hash for evidence). Spike B/B2 trees are left untouched:
#   unikraft/            pristine clone at eb8fa236 (untouched)
#   unikraft-igkmd/      Spike B worktree (untouched)
#   unikraft-igkme/      Spike B2 worktree (untouched)
#   unikraft-igkmf/      NEW worktree at eb8fa236 + increment-d 0001 + 0002
#   lib-lwip/            pristine clone at ec55ae17 (untouched)
#   lib-lwip-igkme/      Spike B2 worktree (untouched)
#   lib-lwip-igkmf/      NEW worktree at ec55ae17 + increment-e 0003
#   app-elfloader/       NEW clone, pinned a6c9dc2b (used read-only as A=)
#   lib-libelf/          NEW clone (HEAD recorded; its make fetches elftoolchain)
#   mbedtls/             Mbed TLS 3.6.7, sha256-pinned (shared)
#   lib-mtlsguard-igkmf/ copy of ./lib-mtlsguard
#   config-igkmf-<v>/    the resolved .config (C=)
#   build-igkmf-<v>/     Unikraft O= build dir (from scratch every time)
#   out-igkmf/           the Firecracker image(s) + debug image(s)
set -euo pipefail

readonly VARIANT="${1:-main}"
case "$VARIANT" in main|strace|go|go-strace) ;; *) echo "unknown variant $VARIANT"; exit 2 ;; esac
readonly INCREMENT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly W="$HOME/igkm-spike-a/unikraft"
readonly UK_PRISTINE="$W/unikraft"
readonly UK="$W/unikraft-igkmf"
readonly LWIP_PRISTINE="$W/lib-lwip"
readonly LWIP="$W/lib-lwip-igkmf"
readonly ELFL="$W/app-elfloader"
readonly LIBELF="$W/lib-libelf"
readonly GUARD="$W/lib-mtlsguard-igkmf"
readonly CFGDIR="$W/config-igkmf-$VARIANT"
readonly BUILD="$W/build-igkmf-$VARIANT"
readonly OUT="$W/out-igkmf"
readonly UK_SHA=eb8fa2368618cea11c9bde196f79e6e6b9caeed5
readonly LWIP_SHA=ec55ae17618feeb57c8c10109bcf5c42723e8e95
readonly ELFL_SHA=a6c9dc2b655572bee1f9657cbcc2d1eca8f6cb73
readonly ELFL_URL=https://github.com/unikraft/app-elfloader.git
readonly LIBELF_URL=https://github.com/unikraft/lib-libelf.git
readonly MBEDTLS_VER=3.6.7
readonly MBEDTLS_SHA256=a7e8bcbec0e6f761b4af24f25677626b35f762f68eef79c08677a363212d11f6
readonly PATCH1="$INCREMENT/../increment-d/patches/0001-posix-socket-connect-post-hook.patch"
readonly PATCH2="$INCREMENT/../increment-d/patches/0002-posix-socket-accept-post-hook.patch"
readonly PATCH3="$INCREMENT/../increment-e/patches/0003-lib-lwip-readiness-interposition.patch"
readonly PATCH1_SHA=e3b35083
readonly PATCH2_SHA=9c798cce
readonly PATCH3_SHA=a4857fce

step() { printf '\n##### [build-guest %s] %s  (%s)\n' "$VARIANT" "$*" "$(date -u +%H:%M:%S)"; }
export PATH="$W/tools/prefix/bin:$PATH"

step "pristine sources (must be clean at the pinned SHAs)"
printf 'unikraft (pristine) HEAD=%s dirty=%s\n' "$(git -C "$UK_PRISTINE" rev-parse HEAD)" "$(git -C "$UK_PRISTINE" status --porcelain | wc -l)"
printf 'lib-lwip (pristine) HEAD=%s dirty=%s\n' "$(git -C "$LWIP_PRISTINE" rev-parse HEAD)" "$(git -C "$LWIP_PRISTINE" status --porcelain | wc -l)"
[[ "$(git -C "$UK_PRISTINE" rev-parse HEAD)" == "$UK_SHA" ]]
[[ "$(git -C "$LWIP_PRISTINE" rev-parse HEAD)" == "$LWIP_SHA" ]]
[[ -z "$(git -C "$UK_PRISTINE" status --porcelain)" ]]
[[ -z "$(git -C "$LWIP_PRISTINE" status --porcelain)" ]]
echo "--- Spike B / B2 worktrees (must still be exactly their patches)"
git -C "$W/unikraft-igkmd" diff --stat
git -C "$W/unikraft-igkme" diff --stat
git -C "$W/lib-lwip-igkme" diff --stat

step "patches (reused read-only from increments d and e)"
sha256sum "$PATCH1" "$PATCH2" "$PATCH3"
[[ "$(sha256sum "$PATCH1" | cut -c1-8)" == "$PATCH1_SHA" ]] || { echo "FATAL: increment-d patch 0001 changed"; exit 1; }
[[ "$(sha256sum "$PATCH2" | cut -c1-8)" == "$PATCH2_SHA" ]] || { echo "FATAL: increment-d patch 0002 changed"; exit 1; }
[[ "$(sha256sum "$PATCH3" | cut -c1-8)" == "$PATCH3_SHA" ]] || { echo "FATAL: increment-e patch 0003 changed"; exit 1; }

step "app-elfloader clone $ELFL (pinned $ELFL_SHA; used read-only as A=)"
if [[ ! -e "$ELFL/.git" ]]; then
  git clone --quiet "$ELFL_URL" "$ELFL"
fi
git -C "$ELFL" checkout -q --detach "$ELFL_SHA"
[[ "$(git -C "$ELFL" rev-parse HEAD)" == "$ELFL_SHA" ]] || { echo "FATAL: app-elfloader HEAD"; exit 1; }
[[ -z "$(git -C "$ELFL" status --porcelain)" ]] || { echo "FATAL: app-elfloader dirty"; git -C "$ELFL" status --porcelain; exit 1; }
git -C "$ELFL" log -1 --format='app-elfloader %H %ci %s'
git -C "$ELFL" remote get-url origin

step "lib-libelf clone $LIBELF (HEAD recorded; first clone pins it)"
if [[ ! -e "$LIBELF/.git" ]]; then
  git clone --quiet "$LIBELF_URL" "$LIBELF"
fi
[[ -z "$(git -C "$LIBELF" status --porcelain)" ]] || { echo "FATAL: lib-libelf dirty"; git -C "$LIBELF" status --porcelain; exit 1; }
git -C "$LIBELF" log -1 --format='lib-libelf %H %ci %s'
git -C "$LIBELF" remote get-url origin
echo "--- lib-libelf fetch/checksum lines"
grep -nE 'URL|SHA|fetch|VERSION' "$LIBELF/Makefile.uk" | head -20 || true

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
git -C "$LWIP" add -N include/uk/lwip_readiness.h
git -C "$LWIP" diff --stat

step "Mbed TLS $MBEDTLS_VER (sha256-pinned, shared)"
TB="$W/mbedtls/mbedtls-$MBEDTLS_VER.tar.bz2"
echo "$MBEDTLS_SHA256  $TB" | sha256sum -c -
MBEDTLS_DIR="$W/mbedtls/mbedtls-$MBEDTLS_VER"
[[ -d "$MBEDTLS_DIR" ]]

step "lib-mtlsguard (increment-f copy) + config fragment"
mkdir -p "$GUARD" "$CFGDIR"
rsync -a --delete "$INCREMENT/lib-mtlsguard/" "$GUARD/"
( cd "$INCREMENT" && sha256sum lib-mtlsguard/guard.c lib-mtlsguard/Makefile.uk lib-mtlsguard/Config.uk \
    lib-mtlsguard/include/igkme_mbedtls_config.h config/igkmf.defconfig )
echo "--- guard.c vs Spike B2's (increment-e): only the evidence addition may differ"
{ diff "$INCREMENT/../increment-e/lib-mtlsguard/guard.c" "$INCREMENT/lib-mtlsguard/guard.c" || true; } | { grep -E '^[<>]' || true; } | wc -l | sed 's/^/  changed lines: /'
{ diff "$INCREMENT/../increment-e/lib-mtlsguard/guard.c" "$INCREMENT/lib-mtlsguard/guard.c" || true; } | { grep -cE '^<' || true; } | sed 's/^/  removed lines (must be 0): /'
FRAG="$CFGDIR/defconfig"
cp "$INCREMENT/config/igkmf.defconfig" "$FRAG"
if [[ "$VARIANT" == go || "$VARIANT" == go-strace ]]; then
  printf '\n' >> "$FRAG"
  cat "$INCREMENT/config/igkmf-go.fragment" >> "$FRAG"
  sha256sum "$INCREMENT/config/igkmf-go.fragment"
fi
if [[ "$VARIANT" == strace || "$VARIANT" == go-strace ]]; then
  printf '\n# strace variant: strace-like line per binary system call\nCONFIG_LIBSYSCALL_SHIM_STRACE=y\n' >> "$FRAG"
fi
rm -f "$CFGDIR/.config" "$CFGDIR/.config.old"

for t in gcc make ld flex bison m4 wget; do
  printf '%-6s %s  %s\n' "$t" "$(command -v "$t")" "$("$t" --version 2>&1 | head -1)"
done

MK=(make -C "$UK" A="$ELFL" L="$LIBELF:$LWIP:$GUARD" O="$BUILD" C="$CFGDIR/.config"
    LEX=flex YACC=bison UK_CFLAGS=-std=gnu17 MTLSGUARD_MBEDTLS_DIR="$MBEDTLS_DIR")

step "clean build dir"
rm -rf -- "$BUILD"

step "configure: make defconfig"
"${MK[@]}" UK_DEFCONFIG="$FRAG" defconfig > "$W/igkmf-$VARIANT-defconfig.log" 2>&1 || {
  echo "defconfig FAILED"; cat "$W/igkmf-$VARIANT-defconfig.log"; exit 1; }
tail -n 3 "$W/igkmf-$VARIANT-defconfig.log"

step "required symbols"
missing=0
req=(PLAT_KVM KVM_VMM_FIRECRACKER KVM_BOOT_PROTO_LXBOOT LIBVIRTIO_MMIO LIBVIRTIO_NET
     LIBVIRTIO_VSOCK LIBUKVSOCKDEV LIBUKNETDEV LIBUKLIBPARAM LIBPOSIX_SOCKET LIBLWIP
     LWIP_SOCKET LWIP_TCP LIBMTLSGUARD LIBPOSIX_TIME LIBUKPS2_SYSRESET LIBPOSIX_POLL
     LIBUKSCHEDCOOP LIBUKBOOT_MAINTHREAD
     APPELFLOADER_INITRDEXEC APPELFLOADER_CUSTOMAPPNAME LIBELF LIBSYSCALL_SHIM
     LIBSYSCALL_SHIM_HANDLER LIBSYSCALL_SHIM_HANDLER_ULTLS LIBPOSIX_PROCESS
     LIBPOSIX_PROCESS_MULTITHREADING LIBPOSIX_PROCESS_ARCH_PRCTL LIBPOSIX_PROCESS_BRK
     LIBUKRANDOM_GETRANDOM LIBPOSIX_MMAP)
[[ "$VARIANT" == strace || "$VARIANT" == go-strace ]] && req+=(LIBSYSCALL_SHIM_STRACE)
[[ "$VARIANT" == go || "$VARIANT" == go-strace ]] && req+=(LIBPOSIX_FUTEX LIBPOSIX_EVENTFD LIBPOSIX_PIPE LIBPOSIX_VFS LIBPOSIX_VFS_MULTICTX)
for s in "${req[@]}"; do
  if grep -qx "CONFIG_$s=y" "$CFGDIR/.config"; then echo "  CONFIG_$s=y"; else echo "  MISSING CONFIG_$s"; missing=1; fi
done
offs=(LIBVFSCORE LIBPOSIX_VFS)
[[ "$VARIANT" == go || "$VARIANT" == go-strace ]] && offs=(LIBVFSCORE APPELFLOADER_AUTOGEN)
for s in "${offs[@]}"; do
  if grep -qx "CONFIG_$s=y" "$CFGDIR/.config"; then echo "  UNEXPECTED CONFIG_$s=y"; missing=1; else echo "  CONFIG_$s off"; fi
done
[[ "$missing" -eq 0 ]] || { echo "FATAL: required symbol missing / unexpected symbol set"; exit 1; }

step "full resolved .config (sha256 $(sha256sum "$CFGDIR/.config" | cut -d' ' -f1))"
echo "----- BEGIN .config -----"
cat "$CFGDIR/.config"
echo "----- END .config -----"

step "build: make -j$(nproc)"
if ! "${MK[@]}" -j"$(nproc)" > "$W/igkmf-$VARIANT-make.log" 2>&1; then
  echo "make FAILED; errors and last 80 lines:"
  grep -nE 'error:|Error [0-9]|undefined reference|No rule' "$W/igkmf-$VARIANT-make.log" | head -60 || true
  tail -n 80 "$W/igkmf-$VARIANT-make.log"
  exit 1
fi
echo "compiler warnings: $(grep -cE 'warning:' "$W/igkmf-$VARIANT-make.log" || true)"
grep -E 'warning:' "$W/igkmf-$VARIANT-make.log" | sed "s#$HOME#~#g" | sort | uniq -c | sort -rn | head -40 || true
echo "--- fetched archives (lib-libelf's elftoolchain)"
find "$BUILD" -maxdepth 2 -type f \( -name '*.tar*' -o -name '*.tgz' -o -name '*.zip' \) -exec sha256sum {} \; 2>/dev/null
tail -n 8 "$W/igkmf-$VARIANT-make.log"

step "image identity"
IMG="$BUILD/igkmf_fc-x86_64"
mkdir -p "$OUT"
cp -f "$IMG" "$OUT/igkmf-${VARIANT}_fc-x86_64"
cp -f "$IMG.dbg" "$OUT/igkmf-${VARIANT}_fc-x86_64.dbg"
sha256sum "$OUT/igkmf-${VARIANT}_fc-x86_64" "$OUT/igkmf-${VARIANT}_fc-x86_64.dbg"
ls -l "$OUT"
readelf -h "$OUT/igkmf-${VARIANT}_fc-x86_64" | grep -E 'Class|Type|Entry'
DBG="$OUT/igkmf-${VARIANT}_fc-x86_64.dbg"
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
echo "--- binary-syscall path: the Linux-ABI handler and the socket/poll handlers it dispatches to"
nm "$DBG" | grep -E ' (ukplat_syscall_handler|uk_syscall6_do_e|uk_syscall_r_connect|uk_syscall_r_accept|uk_syscall_r_accept4|uk_syscall_r_getsockopt|uk_syscall_r_epoll_wait|uk_syscall_r_epoll_pwait|uk_syscall_r_poll|uk_syscall_r_ppoll|uk_syscall_r_read|uk_syscall_r_write|uk_syscall_r_sendto|uk_syscall_r_recvfrom|uk_syscall_r_clock_nanosleep|uk_syscall_r_exit_group)$' || true
echo "--- uk_syscall_r_connect / _accept4 reach the hooked paths"
objdump -d --no-show-raw-insn --disassemble=uk_syscall_r_connect "$DBG" | grep -E 'call|jmp.*<' | head -5 || true
objdump -d --no-show-raw-insn --disassemble=do_accept4 "$DBG" | grep -E 'call.*<uk_sys_accept>' || echo "  (do_accept4 -> uk_sys_accept inlined? check)"
echo "--- elfloader + evidence symbols"
nm "$DBG" | grep -E ' (elf_load_img|elf_ctx_init|initrd_evidence|mbedtls_sha256)$' || true
echo "--- object sizes"
size "$BUILD/libmtlsguard.o" "$BUILD/liblwip.o" "$BUILD/appelfloader.o" "$BUILD/libelf.o" 2>/dev/null || ls -l "$BUILD"/*.o 2>/dev/null | head

step "done"
echo "IMAGE=$OUT/igkmf-${VARIANT}_fc-x86_64"
