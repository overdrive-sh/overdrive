#!/usr/bin/env bash
# PROBE increment-g — S-1: does snapshot/restore work at all on CH v53 with the
# block-only shape we are recommending? Plus S-6 (does CPU hotplug still work on
# a RESTORED VM) and S-7 (snapshot size and write time vs guest RAM).
#
# Everything downstream of the I-6 decision rests on S-1, and it has never been
# run here. The research doc reasons about snapshot/restore extensively; nobody
# has executed it once.
#
# Lifecycle under test, driven over CH's HTTP-over-unix API (`ch-remote` is NOT
# installed on this box, so curl --unix-socket drives it directly):
#
#   boot -> tick a while -> vm.pause -> vm.snapshot -> KILL the VMM
#        -> start a NEW VMM with --restore -> tick again
#
# The verdict is NOT "the restored VM is alive". A rebooted VM is also alive.
# The verdict is whether the RAM-only BOOT_NONCE survived and the tick counter
# continued — see the guest source for why that distinction is the whole probe.
#
# Usage: run.sh [mode]
#   basic         rootfs only                             (S-1)
#   hotplug       as basic, then add a vCPU AFTER restore  (S-6)
#   blk           + a virtio-blk volume                    (S-2)
#   fs            + a virtiofs share, and virtiofsd is KILLED with the VMM and
#                 a FRESH one started before restore       (S-2, the real
#                 checkpoint case: a temporal gap in which no daemon exists)
#   fs-keepalive  + a virtiofs share, but the SAME virtiofsd survives across the
#                 checkpoint                               (S-2, the contrast)
#
# The fs / fs-keepalive split is the point. virtiofsd supports LIVE MIGRATION,
# which hands off between two simultaneously-live daemons; a CHECKPOINT is a gap
# where no source daemon exists. Conflating those two is the trap the research
# doc named, so the probe measures them separately instead of arguing.
#
# Run as root on the bare-metal box.
set -uo pipefail

MODE="${1:-basic}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-g
ARCH="$(uname -m)"
RUN=/run/spike-increment-g
# Snapshots go on the XFS volume: they are large, and putting them on tmpfs
# would both distort the size/time numbers and eat RAM we are measuring.
SNAPROOT="${SNAPROOT:-/srv/vm/p6g}"
KERNEL="$OUT/kernel"
MEM_SIZE="${MEM_SIZE:-512M}"
BOOT_VCPUS="${BOOT_VCPUS:-1}"
MAX_VCPUS="${MAX_VCPUS:-4}"
case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0  ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
esac
case "$MODE" in
  blk)             VOL_KIND=blk;  WANT_FSD=0 ;;
  fs|fs-keepalive) VOL_KIND=fs;   WANT_FSD=1 ;;
  *)               VOL_KIND=none; WANT_FSD=0 ;;
esac
VIRTIOFSD=/usr/libexec/virtiofsd
CMDLINE="root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1 loglevel=4 spike.vol=${VOL_KIND}"

# pkill traps, both of which bit during this probe:
#   * `-x cloud-hypervisor` NEVER matches. The kernel truncates comm to 15
#     chars, so the process is `cloud-hyperviso`. Every such pkill was a silent
#     no-op and stale VMMs accumulated holding the API socket, which surfaced
#     much later as a baffling `ApiSocketInUse`.
#   * `-f "cloud-hypervisor --api-socket"` matches THIS SCRIPT's own command
#     line (and any ssh command containing the string), killing the invoking
#     shell. Match the truncated comm, not the cmdline.
cleanup() { pkill -9 -x cloud-hyperviso 2>/dev/null; }
trap cleanup EXIT

echo "##################################################################"
echo "### PROBE increment-g — snapshot/restore   mode=$MODE"
echo "### cloud-hypervisor : $(cloud-hypervisor --version 2>&1 | head -1)"
echo "### kernel/arch      : $(uname -r) $ARCH   virt=$(systemd-detect-virt || true)"
echo "### guest RAM        : $MEM_SIZE   vcpus boot=$BOOT_VCPUS max=$MAX_VCPUS"
echo "### snapshot dir fs  : $(findmnt -no FSTYPE --target "$(dirname "$SNAPROOT")" 2>/dev/null || echo '?')"
echo "##################################################################"
echo

pkill -9 -x cloud-hyperviso 2>/dev/null; pkill -9 -x virtiofsd 2>/dev/null; sleep 0.5
rm -rf "$RUN" "$SNAPROOT"; mkdir -p "$RUN" "$SNAPROOT/snap"
cp "$OUT/rootfs.ext4" "$RUN/rootfs.ext4"

##########################################################################
# S-2 volume preparation.
VOL_IMG="$SNAPROOT/vol.ext4"
FSD_DIR="$SNAPROOT/fsdshare"
FSD_SOCK="$RUN/virtiofs.sock"
start_fsd() {   # a FRESH daemon each time it is called
  $VIRTIOFSD --socket-path="$FSD_SOCK" --shared-dir="$FSD_DIR" --tag=volrw \
    --cache=never --sandbox=namespace --seccomp kill --log-level=info \
    >>"$RUN/fsd.log" 2>&1 &
  for _ in $(seq 1 100); do [ -S "$FSD_SOCK" ] && return 0; sleep 0.1; done
  return 1
}
case "$VOL_KIND" in
  blk)
    truncate -s 256M "$VOL_IMG"
    mkfs.ext4 -F -L s2vol "$VOL_IMG" >/dev/null 2>&1
    echo "=== [0] S-2 block volume prepared: $(ls -la "$VOL_IMG" | awk '{print $5}') bytes"
    ;;
  fs)
    mkdir -p "$FSD_DIR"; chmod 0777 "$FSD_DIR"
    start_fsd || { echo "!!! virtiofsd socket never appeared"; exit 1; }
    echo "=== [0] S-2 virtiofsd up, pid $(pgrep -x virtiofsd | tail -1)"
    ;;
esac

API="$RUN/ch-api.sock"
# ONE console file, not two. A restored VM re-opens the serial path recorded in
# the SNAPSHOT's config.json — the CLI `--serial` on the restore command is
# ignored. So the restored guest appends to the original file, and the probe
# compares tick counts across the snapshot rather than diffing two files.
# Driver consequence: the serial path in the snapshot must still exist on
# whatever host performs the restore.
CONSOLE="$RUN/console.log"
CHLOG_A="$RUN/ch-before.log"
CHLOG_B="$RUN/ch-after.log"

# curl against CH's HTTP-over-unix API. CH answers 204 No Content on success for
# most PUTs, so an empty body is the SUCCESS case, not a failure.
api() {  # <method> <path> [json-body]
  local method="$1" path="$2" body="${3:-}"
  if [ -n "$body" ]; then
    curl -s -o /dev/null -w '%{http_code}' --unix-socket "$API" \
      -X "$method" -H 'Content-Type: application/json' -d "$body" \
      "http://localhost/api/v1/$path"
  else
    curl -s -o /dev/null -w '%{http_code}' --unix-socket "$API" \
      -X "$method" "http://localhost/api/v1/$path"
  fi
}
api_get() { curl -s --unix-socket "$API" "http://localhost/api/v1/$1"; }

##########################################################################
echo "=== [1] boot the VM"
BOOT_ARGV=(
  cloud-hypervisor
  --api-socket "path=$API"
  --cpus "boot=$BOOT_VCPUS,max=$MAX_VCPUS"
  --kernel "$KERNEL" --cmdline "$CMDLINE"
  --serial "file=$CONSOLE" --console off
)
# virtiofs REQUIRES shared=on; block does not. That difference is exactly what
# makes the two volume kinds different subjects here, not just different paths.
if [ "$VOL_KIND" = fs ]; then
  BOOT_ARGV+=(--memory "size=$MEM_SIZE,shared=on")
  BOOT_ARGV+=(--disk "path=$RUN/rootfs.ext4,image_type=raw")
  BOOT_ARGV+=(--fs "tag=volrw,socket=$FSD_SOCK")
else
  BOOT_ARGV+=(--memory "size=$MEM_SIZE")
  BOOT_ARGV+=(--disk "path=$RUN/rootfs.ext4,image_type=raw")
  [ "$VOL_KIND" = blk ] && BOOT_ARGV+=(--disk "path=$VOL_IMG,image_type=raw")
fi
"${BOOT_ARGV[@]}" >"$CHLOG_A" 2>&1 &
CH_PID=$!

for _ in $(seq 1 100); do [ -S "$API" ] && break; sleep 0.1; done
# Let it tick enough that "the counter continued" is unambiguous.
sleep 6
kill -0 "$CH_PID" 2>/dev/null || { echo "!!! VMM died during boot"; head -20 "$CHLOG_A"; exit 1; }

# The restored VM re-opens the serial path from the SNAPSHOT's config.json and
# TRUNCATES it, destroying the pre-snapshot transcript. Copy it aside first or
# the "before" half of the comparison silently vanishes — which is exactly what
# happened on the first attempt and read as "restore produced no output".
cp "$CONSOLE" "$RUN/console-before-snapshot.log" 2>/dev/null
echo "--- pre-snapshot console tail:"
grep -E '^(init:|TICK)' "$CONSOLE" | tail -5 | sed 's/^/    /'
NONCE_BEFORE="$(grep -oE 'nonce=[0-9a-f]+' "$RUN/console-before-snapshot.log" | head -1 | cut -d= -f2)"
LAST_TICK_BEFORE="$(grep -oE '^TICK n=[0-9]+' "$RUN/console-before-snapshot.log" | tail -1 | grep -oE '[0-9]+')"
echo "--- BOOT_NONCE before = ${NONCE_BEFORE:-<none>}"
echo "--- last tick before  = ${LAST_TICK_BEFORE:-<none>}"
echo

##########################################################################
echo "=== [2] vm.pause"
echo "    HTTP $(api PUT vm.pause)"

echo "=== [3] vm.snapshot -> $SNAPROOT/snap"
T0=$(date +%s.%N)
SNAP_CODE="$(api PUT vm.snapshot "{\"destination_url\":\"file://$SNAPROOT/snap\"}")"
T1=$(date +%s.%N)
echo "    HTTP $SNAP_CODE   elapsed $(echo "$T1 - $T0" | bc)s"
if [ "$SNAP_CODE" != "204" ] && [ "$SNAP_CODE" != "200" ]; then
  echo "!!! snapshot REFUSED (HTTP $SNAP_CODE)"
  tail -20 "$CHLOG_A"
  echo "--- S-1 VERDICT: SNAPSHOT UNSUPPORTED/FAILED on this shape"
  exit 1
fi
echo "--- snapshot contents (S-7: size vs $MEM_SIZE guest RAM):"
ls -la "$SNAPROOT/snap" | sed 's/^/    /'
du -sh --apparent-size "$SNAPROOT/snap" | sed 's/^/    apparent: /'
du -sh "$SNAPROOT/snap" | sed 's/^/    on disk : /'
echo

echo "=== [4] kill the original VMM — the checkpoint's temporal gap"
kill -9 "$CH_PID" 2>/dev/null; wait "$CH_PID" 2>/dev/null
# v53 keeps a LOCK FILE beside the API socket. A SIGKILLed VMM leaves it behind,
# and the next VMM on that path refuses to start:
#   Fatal error: StartVmmThread(ApiSocketInUse("..."))
# Removing only the socket is NOT enough. A driver that checkpoints by killing
# the VMM must clean both, or use a fresh socket path per incarnation.
rm -f "$API" "$API.lock"
echo "    original VMM gone; API socket AND .lock removed"
# THE S-2 DISTINCTION. In `fs` the daemon dies with the VM and a brand-new one
# takes its place — a real checkpoint, with a gap in which no daemon holds the
# FUSE session. In `fs-keepalive` the same daemon survives, which is the
# live-migration-shaped case and NOT what a checkpoint is.
if [ "$WANT_FSD" = 1 ]; then
  if [ "$MODE" = fs ]; then
    echo "    killing virtiofsd (the checkpoint's temporal gap applies to it too)"
    pkill -9 -x virtiofsd 2>/dev/null; sleep 0.5
    rm -f "$FSD_SOCK"
    echo "    starting a FRESH virtiofsd on the same socket path"
    start_fsd || echo "    !!! fresh virtiofsd did not come up"
  else
    echo "    leaving the ORIGINAL virtiofsd alive (pid $(pgrep -x virtiofsd | tail -1))"
  fi
fi
echo

##########################################################################
echo "=== [5] restore into a NEW VMM — via the API, NOT the CLI"
# Both exist in v53. Only one works.
#
#   CLI  `cloud-hypervisor --restore source_url=...`
#        - demands --kernel/--firmware at clap level even though the snapshot's
#          own config.json already names the payload, and then
#        - does NOTHING: exits with no error, no log line, no guest. A silent
#          no-op is the worst failure mode available here, because "the VMM is
#          running" and "the VM was restored" are indistinguishable from
#          outside unless something like this probe's RAM-only nonce is checked.
#
#   API  start a VMM with ONLY --api-socket (no VM configured), then
#        PUT /api/v1/vm.restore, then PUT /api/v1/vm.resume.
#
# The API form is what CH documents and what the driver must implement.
cloud-hypervisor --api-socket "path=$API" >"$CHLOG_B" 2>&1 &
CH_PID2=$!
for _ in $(seq 1 100); do [ -S "$API" ] && break; sleep 0.1; done

RESTORE_CODE="$(api PUT vm.restore "{\"source_url\":\"file://$SNAPROOT/snap\"}")"
echo "    PUT vm.restore -> HTTP $RESTORE_CODE"
if [ "$RESTORE_CODE" != "204" ] && [ "$RESTORE_CODE" != "200" ]; then
  echo "!!! restore REFUSED (HTTP $RESTORE_CODE)"
  head -20 "$CHLOG_B" | sed 's/^/    /'
  echo "--- S-1 VERDICT: RESTORE FAILED"
  exit 1
fi
echo "=== [6] vm.resume -> HTTP $(api PUT vm.resume)"
sleep 5

##########################################################################
if [ "$MODE" = hotplug ]; then
  echo
  echo "=== [7] S-6: hot-plug a vCPU on the RESTORED VM"
  echo "    (Firecracker forbids this outright; CH's docs are silent, and CPU"
  echo "     hotplug is the entire reason this feature chose CH over Firecracker)"
  HP="$(api PUT vm.resize "{\"desired_vcpus\":$((BOOT_VCPUS + 1))}")"
  echo "    vm.resize desired_vcpus=$((BOOT_VCPUS + 1)) -> HTTP $HP"
  sleep 4
fi

##########################################################################
echo
echo "=========================== POST-RESTORE CONSOLE ==========================="
grep -E '^(init:|TICK)' "$CONSOLE" 2>/dev/null | head -12 | sed 's/^/    /'
echo "    ..."
grep -E '^(init:|TICK)' "$CONSOLE" 2>/dev/null | tail -4 | sed 's/^/    /'
echo "=========================== END POST-RESTORE ==============================="
echo

NONCE_AFTER="$(grep -oE 'nonce=[0-9a-f]+' "$CONSOLE" 2>/dev/null | tail -1 | cut -d= -f2)"
LAST_TICK_AFTER="$(grep -oE '^TICK n=[0-9]+' "$CONSOLE" 2>/dev/null | tail -1 | grep -oE '[0-9]+')"
FIRST_TICK_AFTER="$LAST_TICK_BEFORE"
# `grep -c` exits 1 with a "0" on stdout when there is no match, so `|| echo 0`
# emits a SECOND zero and the variable becomes "0\n0". Force a clean integer.
REBOOTED_BANNER="$(grep -c 'SNAP PROBE up' "$CONSOLE" 2>/dev/null | head -1)"
: "${REBOOTED_BANNER:=0}"

echo "=========================== S-1 VERDICT ===================================="
printf '  %-26s %s\n' "BOOT_NONCE before"  "${NONCE_BEFORE:-<none>}"
printf '  %-26s %s\n' "BOOT_NONCE after"   "${NONCE_AFTER:-<none>}"
printf '  %-26s %s\n' "last tick before"   "${LAST_TICK_BEFORE:-<none>}"
printf '  %-26s %s\n' "first tick after"   "${FIRST_TICK_AFTER:-<none>}"
printf '  %-26s %s\n' "last tick after"    "${LAST_TICK_AFTER:-<none>}"
printf '  %-26s %s\n' "boot banners after" "$REBOOTED_BANNER  (must be 0 — the restored VM truncates the console, so a banner here means it re-ran main() i.e. REBOOTED)"
echo
if [ -n "$NONCE_AFTER" ] && [ "$NONCE_BEFORE" = "$NONCE_AFTER" ] \
   && [ "$REBOOTED_BANNER" = "0" ] \
   && [ -n "$LAST_TICK_AFTER" ] && [ -n "$LAST_TICK_BEFORE" ] \
   && [ "$LAST_TICK_AFTER" -gt "$LAST_TICK_BEFORE" ]; then
  echo "  +++ RESTORED FROM MEMORY: nonce identical, no re-boot banner, counter continued."
else
  echo "  !!! NOT a memory restore — see the fields above."
  echo "      nonce differing or a boot banner present means it REBOOTED, which"
  echo "      would have read as 'restore works' without this check."
fi
[ "$MODE" = hotplug ] && {
  echo
  echo "  S-6 last tick: $(grep -oE 'vcpu_online=[0-9]+ vcpu_present=[0-9]+' "$CONSOLE" | tail -1)"
  echo "      onlining events seen: $(grep -c 'ONLINED' "$CONSOLE" | head -1)"
  echo "     (boot was $BOOT_VCPUS; an increase means hotplug works on a restored VM)"
}
if [ "$VOL_KIND" != none ]; then
  echo
  echo "  --- S-2: the volume fd held OPEN across the checkpoint (kind=$VOL_KIND)"
  echo "      pre-snapshot  : $(grep -oE 'vol=[A-Za-z_:0-9]+' "$RUN/console-before-snapshot.log" 2>/dev/null | tail -1)"
  echo "      post-restore  : $(grep -oE 'vol=[A-Za-z_:0-9]+' "$CONSOLE" 2>/dev/null | tail -1)"
  echo "      distinct post-restore statuses: $(grep -oE 'vol=[A-Za-z_:0-9]+' "$CONSOLE" 2>/dev/null | sort -u | tr '\n' ' ')"
  if [ "$VOL_KIND" = fs ]; then
    echo "      virtiofsd log:"; tail -4 "$RUN/fsd.log" 2>/dev/null | sed 's/^/        /'
  fi
  echo "      host-side file content now: $(
      if [ "$VOL_KIND" = fs ]; then cat "$FSD_DIR/persist.bin" 2>/dev/null | tr -d '\n'
      else echo '<inside the block image; not readable while attached>'; fi)"
fi
echo "=========================== END VERDICT ===================================="

kill -9 "$CH_PID2" 2>/dev/null
pkill -9 -x virtiofsd 2>/dev/null
