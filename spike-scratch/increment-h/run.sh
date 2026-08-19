#!/usr/bin/env bash
# PROBE increment-h — S-8: `vhost-user-blk`.
#
# WHY THIS EXISTS. #97 proposes `overdrive-fs` — a content-addressed chunk store
# on Garage with libSQL metadata — served to the guest over `vhost-user-fs`.
# Fly Sprites converged on the same STORAGE MODEL (JuiceFS-shaped chunks on
# object storage, NVMe as pure cache) but a different GUEST SEAM: the guest sees
# ext4 on a block device, with no virtiofs anywhere. `vhost-user-blk` is the
# userspace-backend block path that would let a custom store serve chunks while
# the guest still sees a plain block device. It has never been measured here.
#
# The decision-relevant questions, in order:
#   1. Does `--disk vhost_user=on,socket=` work at all on CH v53?
#   2. Does it REQUIRE `--memory shared=on`? This matters a lot: `shared=on` is
#      one of the main costs charged against virtiofs (memfd, the RLIMIT_FSIZE
#      trap, no nested-virt boot). If vhost-user-blk needs it too, that argument
#      does NOT transfer, and the honest comparison narrows.
#   3. Does it survive snapshot/restore?
#   4. What is the backend daemon's lifecycle? virtiofsd shuts down the moment
#      its client disconnects, which forced an ordered "restart the daemon
#      before vm.restore" step. Does a block backend do the same?
#
# Backend is `qemu-storage-daemon` (QEMU 10.2.1), the standard vhost-user-blk
# export. It stands in for what `overdrive-fs` would eventually be; the point is
# the TRANSPORT, not this particular daemon.
#
# Reuses increment-g's kernel, rootfs and guest binary unchanged, so the only
# deliberate variable is how the volume is attached. The guest already mounts
# /dev/vdb as ext4 and round-trips a held-open fd when given `spike.vol=blk`.
#
# Usage: run.sh <mode>
#   shared     vhost-user-blk WITH    --memory shared=on
#   noshare    vhost-user-blk WITHOUT --memory shared=on   (question 2)
#   plain      ordinary --disk, no vhost-user               (the control)
#
# Run as root on the bare-metal box.
set -uo pipefail

MODE="${1:-shared}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
G_OUT=/var/tmp/spike-increment-g        # kernel + rootfs come from increment-g
OUT=/var/tmp/spike-increment-h
RUN=/run/spike-increment-h
SNAPROOT="${SNAPROOT:-/srv/vm/p6h}"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0 ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
esac
MEM_SIZE="${MEM_SIZE:-512M}"
CMDLINE="root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1 loglevel=4 spike.vol=blk"

cleanup() {
  pkill -9 -x cloud-hyperviso 2>/dev/null
  pkill -9 -x qemu-storage-da 2>/dev/null   # comm is truncated to 15 chars
}
trap cleanup EXIT

echo "##################################################################"
echo "### PROBE increment-h — S-8 vhost-user-blk   mode=$MODE"
echo "### cloud-hypervisor    : $(cloud-hypervisor --version 2>&1 | head -1)"
echo "### qemu-storage-daemon : $(qemu-storage-daemon --version 2>&1 | head -1)"
echo "### host                : $(uname -r) $ARCH  virt=$(systemd-detect-virt || true)"
echo "##################################################################"
echo

[ -f "$G_OUT/kernel" ] && [ -f "$G_OUT/rootfs.ext4" ] || {
  echo "!!! need increment-g's kernel+rootfs — run increment-g/build.sh first" >&2; exit 1; }

pkill -9 -x cloud-hyperviso 2>/dev/null; pkill -9 -x qemu-storage-da 2>/dev/null; sleep 0.5
rm -rf "$RUN" "$SNAPROOT" "$OUT"; mkdir -p "$RUN" "$SNAPROOT/snap" "$OUT"
cp "$G_OUT/rootfs.ext4" "$RUN/rootfs.ext4"

VOL_IMG="$SNAPROOT/vol.ext4"
truncate -s 256M "$VOL_IMG"
mkfs.ext4 -F -L s8vol "$VOL_IMG" >/dev/null 2>&1

API="$RUN/ch-api.sock"
VUB_SOCK="$RUN/vublk.sock"
CONSOLE="$RUN/console.log"

api() {
  local method="$1" path="$2" body="${3:-}"
  if [ -n "$body" ]; then
    curl -s -o /dev/null -w '%{http_code}' --unix-socket "$API" -X "$method" \
      -H 'Content-Type: application/json' -d "$body" "http://localhost/api/v1/$path"
  else
    curl -s -o /dev/null -w '%{http_code}' --unix-socket "$API" -X "$method" \
      "http://localhost/api/v1/$path"
  fi
}

start_backend() {
  qemu-storage-daemon \
    --blockdev "driver=file,node-name=f0,filename=$VOL_IMG" \
    --blockdev "driver=raw,node-name=r0,file=f0" \
    --export "type=vhost-user-blk,id=e0,node-name=r0,addr.type=unix,addr.path=$VUB_SOCK,writable=on" \
    >>"$RUN/vublk.log" 2>&1 &
  for _ in $(seq 1 100); do [ -S "$VUB_SOCK" ] && return 0; sleep 0.1; done
  return 1
}

##########################################################################
if [ "$MODE" != plain ]; then
  echo "=== [0] start the vhost-user-blk backend"
  start_backend || { echo "!!! backend socket never appeared"; tail -10 "$RUN/vublk.log"; exit 1; }
  echo "    qemu-storage-daemon pid=$(pgrep -x qemu-storage-da | tail -1)  socket=$VUB_SOCK"
  echo
fi

echo "=== [1] boot"
ARGV=(
  cloud-hypervisor
  --api-socket "path=$API"
  --cpus "boot=1"
  --kernel "$G_OUT/kernel" --cmdline "$CMDLINE"
  --serial "file=$CONSOLE" --console off
)
case "$MODE" in
  shared)  ARGV+=(--memory "size=$MEM_SIZE,shared=on") ;;
  *)       ARGV+=(--memory "size=$MEM_SIZE") ;;
esac
ARGV+=(--disk "path=$RUN/rootfs.ext4,image_type=raw")
case "$MODE" in
  plain) ARGV+=(--disk "path=$VOL_IMG,image_type=raw") ;;
  *)     ARGV+=(--disk "vhost_user=on,socket=$VUB_SOCK") ;;
esac
printf '    argv: %q' "${ARGV[@]}"; echo
"${ARGV[@]}" >"$RUN/ch.log" 2>&1 &
CH_PID=$!
for _ in $(seq 1 100); do [ -S "$API" ] && break; sleep 0.1; done
sleep 6

if ! kill -0 "$CH_PID" 2>/dev/null; then
  echo "!!! VMM DIED"
  echo "--- ch.log:"; head -15 "$RUN/ch.log" | sed 's/^/    /'
  echo
  echo "--- S-8 mode=$MODE VERDICT: DID NOT BOOT"
  exit 1
fi

echo "--- guest, pre-snapshot:"
grep -E '^(init:|TICK)' "$CONSOLE" 2>/dev/null | tail -4 | sed 's/^/    /'
VOL_BEFORE="$(grep -oE 'vol=[A-Za-z_:0-9]+' "$CONSOLE" 2>/dev/null | tail -1)"
NONCE_BEFORE="$(grep -oE 'nonce=[0-9a-f]+' "$CONSOLE" 2>/dev/null | head -1 | cut -d= -f2)"
LAST_BEFORE="$(grep -oE '^TICK n=[0-9]+' "$CONSOLE" 2>/dev/null | tail -1 | grep -oE '[0-9]+')"
cp "$CONSOLE" "$RUN/console-before-snapshot.log" 2>/dev/null
echo "    volume status before = ${VOL_BEFORE:-<none>}"
echo

##########################################################################
echo "=== [2] vm.pause -> HTTP $(api PUT vm.pause)"
SNAP="$(api PUT vm.snapshot "{\"destination_url\":\"file://$SNAPROOT/snap\"}")"
echo "=== [3] vm.snapshot -> HTTP $SNAP"
if [ "$SNAP" != "204" ] && [ "$SNAP" != "200" ]; then
  echo "    !!! snapshot refused with a vhost-user-blk device attached"
  tail -8 "$RUN/ch.log" | sed 's/^/    /'
  echo "--- S-8 mode=$MODE VERDICT: BOOTS, SNAPSHOT REFUSED"
  exit 1
fi
ls -la "$SNAPROOT/snap" | sed 's/^/    /'

echo "=== [4] kill the VMM"
kill -9 "$CH_PID" 2>/dev/null; wait "$CH_PID" 2>/dev/null
rm -f "$API" "$API.lock"
# Question 4: does the block backend die with its client, the way virtiofsd does?
if [ "$MODE" != plain ]; then
  sleep 1
  BE_ALIVE="$(pgrep -x qemu-storage-da | wc -l)"
  echo "    backend daemon still alive after the VMM died: $BE_ALIVE  (virtiofsd would be 0)"
  if [ "$BE_ALIVE" = 0 ]; then
    echo "    -> restarting the backend before restore (same ordered step virtiofs needs)"
    rm -f "$VUB_SOCK"; start_backend || echo "    !!! backend did not restart"
  fi
fi
echo

echo "=== [5] restore via the API"
cloud-hypervisor --api-socket "path=$API" >"$RUN/ch-after.log" 2>&1 &
CH2=$!
for _ in $(seq 1 100); do [ -S "$API" ] && break; sleep 0.1; done
RC="$(api PUT vm.restore "{\"source_url\":\"file://$SNAPROOT/snap\"}")"
echo "    PUT vm.restore -> HTTP $RC"
if [ "$RC" != "204" ] && [ "$RC" != "200" ]; then
  echo "    --- ch-after.log:"; head -12 "$RUN/ch-after.log" | sed 's/^/      /'
  echo "--- S-8 mode=$MODE VERDICT: SNAPSHOT OK, RESTORE FAILED (HTTP $RC)"
  exit 1
fi
echo "    PUT vm.resume  -> HTTP $(api PUT vm.resume)"
sleep 5

##########################################################################
NONCE_AFTER="$(grep -oE 'nonce=[0-9a-f]+' "$CONSOLE" 2>/dev/null | tail -1 | cut -d= -f2)"
LAST_AFTER="$(grep -oE '^TICK n=[0-9]+' "$CONSOLE" 2>/dev/null | tail -1 | grep -oE '[0-9]+')"
VOL_AFTER="$(grep -oE 'vol=[A-Za-z_:0-9]+' "$CONSOLE" 2>/dev/null | tail -1)"
BANNERS="$(grep -c 'SNAP PROBE up' "$CONSOLE" 2>/dev/null | head -1)"; : "${BANNERS:=0}"

echo
echo "--- guest, post-restore:"
grep -E '^TICK' "$CONSOLE" 2>/dev/null | tail -4 | sed 's/^/    /'
echo
echo "=========================== S-8 VERDICT (mode=$MODE) ======================="
printf '  %-28s %s\n' "boots"                "yes"
printf '  %-28s %s\n' "memory argv"          "$([ "$MODE" = shared ] && echo "size=$MEM_SIZE,shared=on" || echo "size=$MEM_SIZE  (NO shared=on)")"
printf '  %-28s %s\n' "volume before"        "${VOL_BEFORE:-<none>}"
printf '  %-28s %s\n' "volume after restore" "${VOL_AFTER:-<none>}"
printf '  %-28s %s\n' "nonce before/after"   "${NONCE_BEFORE:-?} / ${NONCE_AFTER:-?}"
printf '  %-28s %s\n' "tick before/after"    "${LAST_BEFORE:-?} / ${LAST_AFTER:-?}"
printf '  %-28s %s\n' "reboot banners"       "$BANNERS  (0 = resumed, not rebooted)"
if [ -n "$NONCE_AFTER" ] && [ "$NONCE_BEFORE" = "$NONCE_AFTER" ] && [ "$BANNERS" = "0" ] \
   && [ -n "$LAST_AFTER" ] && [ -n "$LAST_BEFORE" ] && [ "$LAST_AFTER" -gt "$LAST_BEFORE" ]; then
  echo "  +++ RESTORED FROM MEMORY with a $([ "$MODE" = plain ] && echo "plain --disk" || echo "vhost-user-blk") volume attached"
else
  echo "  !!! NOT a clean memory restore — read the fields above"
fi
echo "=========================== END VERDICT ==================================="
kill -9 "$CH2" 2>/dev/null
