#!/usr/bin/env bash
# PROBE increment-j — S-3: what happens to VSOCK across a snapshot/restore?
#
# WHY THIS MATTERS. The driver's workload-Running gate rides on the vsock
# channel: the guest connects out to a host-side listener to signal readiness
# (P2). If vsock breaks across a checkpoint, the persistent-microVM lifecycle
# breaks with it. P8 proved snapshot/restore works on v53 — but deliberately
# used the SERIAL CONSOLE so "did restore work" was not entangled with "did the
# vsock peer reconnect". This probe is the entanglement, run on purpose.
#
# Four questions, in priority order:
#   1. Does a VM WITH a --vsock device snapshot and restore at all?
#      (P8's control is the same probe WITHOUT --vsock, already green.)
#   2. What happens to an ESTABLISHED connection across the checkpoint? Errno.
#      cloud-hypervisor#7958 ("reset on snapshot restore to avoid stale
#      half-open connections") reportedly landed in v52.0 and we run v53, so the
#      EXPECTED behaviour is a clean reset rather than a silently-stale socket.
#      That is a claim to confirm or refute, not to assume.
#   3. Does restore FAIL if the host-side socket path is gone? The snapshot's
#      config.json records ABSOLUTE paths.
#   4. Can a FRESH listener re-attach after restore, and can the guest open a
#      NEW connection? This is the one that decides whether the Running gate is
#      recoverable — an established connection dying is survivable if a new one
#      can be made.
#
# MODES — the four differ ONLY in what happens to the host side during the
# checkpoint gap, so any result they share is attributable to the checkpoint
# rather than to listener lifecycle:
#
#   drop       the driver's expected flow. Host listeners are killed with the
#              VMM and their `_N` sockets removed; the stale ch.vsock is cleaned;
#              FRESH listeners are bound only AFTER vm.resume. The deliberate
#              gap between resume and re-bind is what exposes the errno a guest
#              sees when nothing is listening (Q4's negative half).
#   keep       the SAME listener processes survive the checkpoint. The contrast
#              arm: if `drop` and `keep` agree, the listener lifecycle is not the
#              cause.
#   stalesock  as `drop`, but the stale ch.vsock UDS left behind by the SIGKILLed
#              VMM is NOT removed before restore. (Q3, and the direct analogue of
#              P8's `<api-socket>.lock` trap.)
#   nosockdir  the ENTIRE directory holding ch.vsock is deleted before restore.
#              (Q3 proper — an absolute path in config.json that no longer
#              resolves.)
#
# Run as root on the bare-metal box.
set -uo pipefail

MODE="${1:-drop}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-j
ARCH="$(uname -m)"
RUN=/run/spike-increment-j
SNAPROOT="${SNAPROOT:-/srv/vm/p12j}"
KERNEL="$OUT/kernel"
LISTENER="$OUT/vsock-listener"
MEM_SIZE="${MEM_SIZE:-512M}"
BOOT_VCPUS="${BOOT_VCPUS:-1}"
GUEST_CID="${GUEST_CID:-3}"
PORT_HELD=1234
PORT_NEW=1235
case "$ARCH" in
  x86_64)  CONSOLE_DEV=ttyS0  ;;
  aarch64) CONSOLE_DEV=ttyAMA0 ;;
esac
CMDLINE="root=/dev/vda rw console=${CONSOLE_DEV} init=/init panic=1 loglevel=4"

case "$MODE" in
  drop|keep|stalesock|nosockdir) ;;
  *) echo "!!! unknown mode '$MODE' (drop|keep|stalesock|nosockdir)" >&2; exit 2 ;;
esac

# pkill traps from P8, both of which cost real time there:
#   * `-x cloud-hypervisor` NEVER matches — the kernel truncates comm to 15
#     chars, so the process is `cloud-hyperviso`. Every such pkill is a silent
#     no-op and stale VMMs accumulate, surfacing later as `ApiSocketInUse`.
#   * `-f "cloud-hypervisor --api-socket"` matches THIS SCRIPT's own command line
#     (and any ssh command containing the string), killing the invoking shell.
#
# AND A THIRD, FOUND HERE (increment-j, first `drop` run): `pkill -9 -x
# cloud-hyperviso` is not scoped to this probe. The box is shared, and when
# increments e and i started concurrently, THEIR broad pkill SIGKILLed THIS
# probe's restoring VMM mid-`vm.restore`. The result was `HTTP 000` with an empty
# CH log — which reads exactly like "restore refused" and would have been written
# up as a Q3 finding. It was collateral damage from another probe.
#
# So this probe kills ONLY PIDs it recorded itself, and never sweeps by name.
CH_PID=""; CH_PID2=""; LP_HELD=""; LP_NEW=""; LP_HELD2=""; LP_NEW2=""
cleanup() {
  for p in "${CH_PID:-}" "${CH_PID2:-}" "${LP_HELD:-}" "${LP_NEW:-}" "${LP_HELD2:-}" "${LP_NEW2:-}"; do
    [ -n "$p" ] && kill -9 "$p" 2>/dev/null
  done
  return 0
}
trap cleanup EXIT

# The other half of the same defence: refuse to start while a FOREIGN probe is
# live, because its cleanup will kill this one's VMM. Poll rather than fail so an
# unattended run just waits its turn.
foreign_procs() {
  pgrep -af 'cloud-hyperviso|spike-scratch/increment-' 2>/dev/null \
    | grep -v "increment-j" | grep -v "spike-increment-j" | grep -v "^$$ " || true
}
WAIT_QUIET_SECS="${WAIT_QUIET_SECS:-1800}"
waited=0
while [ -n "$(foreign_procs)" ]; do
  if [ "$waited" = 0 ]; then
    echo "### another probe is live on this shared box; waiting for a quiet box"
    foreign_procs | cut -c1-120 | sed 's/^/###   /'
  fi
  [ "$waited" -ge "$WAIT_QUIET_SECS" ] && {
    echo "!!! box still busy after ${WAIT_QUIET_SECS}s; refusing to run (a foreign pkill would kill this VMM mid-restore)" >&2
    exit 3
  }
  sleep 10; waited=$((waited + 10))
done
[ "$waited" -gt 0 ] && echo "### box quiet after ${waited}s; proceeding"

echo "##################################################################"
echo "### PROBE increment-j — S-3 vsock across snapshot/restore  mode=$MODE"
echo "### cloud-hypervisor : $(cloud-hypervisor --version 2>&1 | head -1)"
echo "### kernel/arch      : $(uname -r) $ARCH   virt=$(systemd-detect-virt || true)"
echo "### guest RAM/vcpus  : $MEM_SIZE / $BOOT_VCPUS      guest CID=$GUEST_CID"
echo "### snapshot dir fs  : $(findmnt -no FSTYPE --target "$(dirname "$SNAPROOT")" 2>/dev/null || echo '?')"
echo "##################################################################"
echo

# No name-scoped pkill here either — see the cleanup() comment. Nothing of this
# probe's should be alive at this point anyway, since the previous run's own
# EXIT trap killed its recorded PIDs.
rm -rf "$RUN" "$SNAPROOT"; mkdir -p "$RUN" "$SNAPROOT/snap"
cp "$OUT/rootfs.ext4" "$RUN/rootfs.ext4"

# The vsock socket lives in its OWN directory so `nosockdir` can delete it
# without taking the API socket with it.
VSOCK_DIR="$RUN/vsock"
mkdir -p "$VSOCK_DIR"
CHVSOCK="$VSOCK_DIR/ch.vsock"

API="$RUN/ch-api.sock"
# ONE console file (P8 trap 2): a restored VM re-opens the serial path recorded
# in the SNAPSHOT's config.json and TRUNCATES it — the CLI `--serial` on the
# restore command is ignored. So the pre-snapshot transcript must be copied
# aside, and the post-restore file conveniently contains only post-restore ticks.
CONSOLE="$RUN/console.log"
CHLOG_A="$RUN/ch-before.log"
CHLOG_B="$RUN/ch-after.log"

api() {  # <method> <path> [json-body]  — CH answers 204 on success, so an empty
         # body is the SUCCESS case, not a failure.
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
##########################################################################
echo "=== [0] bind the host-side vsock listeners"
echo "    CH's vsock host end is a UNIX socket; a guest connect to port N lands"
echo "    on a listener the HOST binds at <socket>_N. Both are bound BEFORE boot."
"$LISTENER" "${CHVSOCK}_${PORT_HELD}" HELD  >"$RUN/held.log" 2>&1 &
LP_HELD=$!
"$LISTENER" "${CHVSOCK}_${PORT_NEW}"  RECON >"$RUN/recon.log" 2>&1 &
LP_NEW=$!
sleep 0.7
echo "    listener pids: held=$LP_HELD recon=$LP_NEW"
ls -la "$VSOCK_DIR" | sed 's/^/    /'
echo

##########################################################################
echo "=== [1] boot the VM (WITH --vsock — that is the variable vs P8)"
cloud-hypervisor \
  --api-socket "path=$API" \
  --cpus "boot=$BOOT_VCPUS,max=4" \
  --memory "size=$MEM_SIZE" \
  --kernel "$KERNEL" --cmdline "$CMDLINE" \
  --disk "path=$RUN/rootfs.ext4,image_type=raw" \
  --vsock "cid=$GUEST_CID,socket=$CHVSOCK" \
  --serial "file=$CONSOLE" --console off \
  >"$CHLOG_A" 2>&1 &
CH_PID=$!
for _ in $(seq 1 100); do [ -S "$API" ] && break; sleep 0.1; done
sleep 6
kill -0 "$CH_PID" 2>/dev/null || { echo "!!! VMM died during boot"; head -20 "$CHLOG_A"; exit 1; }

cp "$CONSOLE" "$RUN/console-before-snapshot.log" 2>/dev/null
cp "$RUN/held.log"  "$RUN/held-before.log"  2>/dev/null
cp "$RUN/recon.log" "$RUN/recon-before.log" 2>/dev/null

echo "--- guest prerequisite evidence (see the guest source for why this gate exists):"
grep -E 'insmod|/dev/vsock|PREREQ|HELD connect' "$RUN/console-before-snapshot.log" | sed 's/^/    /'
echo "--- pre-snapshot console tail:"
grep -E '^TICK' "$RUN/console-before-snapshot.log" | tail -3 | sed 's/^/    /'
echo "--- pre-snapshot HOST transcript (held listener), tail:"
tail -4 "$RUN/held-before.log" | sed 's/^/    /'
echo "--- pre-snapshot HOST transcript (reconnect listener), tail:"
tail -4 "$RUN/recon-before.log" | sed 's/^/    /'

NONCE_BEFORE="$(grep -oE 'nonce=[0-9a-f]+' "$RUN/console-before-snapshot.log" | head -1 | cut -d= -f2)"
LAST_TICK_BEFORE="$(grep -oE '^TICK n=[0-9]+' "$RUN/console-before-snapshot.log" | tail -1 | grep -oE '[0-9]+')"
HELD_LINES_BEFORE="$(grep -c 'HELD n=' "$RUN/held-before.log" 2>/dev/null | head -1)"
RECON_CONNS_BEFORE="$(grep -c 'ACCEPT conn#' "$RUN/recon-before.log" 2>/dev/null | head -1)"
: "${HELD_LINES_BEFORE:=0}"; : "${RECON_CONNS_BEFORE:=0}"
echo "--- BOOT_NONCE before        = ${NONCE_BEFORE:-<none>}"
echo "--- last tick before         = ${LAST_TICK_BEFORE:-<none>}"
echo "--- HELD lines host-side     = $HELD_LINES_BEFORE"
echo "--- reconnects host-side     = $RECON_CONNS_BEFORE"

# THE HONESTY GATE. Two prior probes in this spike produced confident WRONG
# NEGATIVES because a guest-side prerequisite was missing. If vsock was not
# working BEFORE the snapshot, nothing measured after it means anything, and the
# right answer is "harness defect", not "vsock does not survive restore".
if [ "$HELD_LINES_BEFORE" -lt 2 ] || [ "$RECON_CONNS_BEFORE" -lt 2 ]; then
  echo
  echo "!!! HARNESS DEFECT: vsock was not working BEFORE the snapshot."
  echo "!!! Any post-restore result would be a false negative. Aborting."
  echo "--- guest console:"; tail -30 "$RUN/console-before-snapshot.log" | sed 's/^/    /'
  echo "--- ch log:";        tail -20 "$CHLOG_A" | sed 's/^/    /'
  exit 1
fi
echo "    +++ PREREQ GATE PASSED: vsock demonstrably worked pre-snapshot."
echo

##########################################################################
echo "=== [2] vm.pause -> HTTP $(api PUT vm.pause)"

echo "=== [3] vm.snapshot -> $SNAPROOT/snap"
T0=$(date +%s.%N)
SNAP_CODE="$(api PUT vm.snapshot "{\"destination_url\":\"file://$SNAPROOT/snap\"}")"
T1=$(date +%s.%N)
echo "    HTTP $SNAP_CODE   elapsed $(echo "$T1 - $T0" | bc)s"
if [ "$SNAP_CODE" != "204" ] && [ "$SNAP_CODE" != "200" ]; then
  echo "!!! snapshot REFUSED (HTTP $SNAP_CODE) — Q1 ANSWER: a VM with --vsock cannot be snapshotted"
  tail -20 "$CHLOG_A" | sed 's/^/    /'
  exit 1
fi
ls -la "$SNAPROOT/snap" | sed 's/^/    /'
echo "--- Q3 evidence: what the snapshot's config.json records for vsock (ABSOLUTE path):"
python3 -c "
import json,sys
c=json.load(open('$SNAPROOT/snap/config.json'))
print('   ', json.dumps(c.get('vsock'), indent=2).replace('\n','\n    '))
" 2>/dev/null || grep -o '"vsock":[^}]*}' "$SNAPROOT/snap/config.json" | sed 's/^/    /'
echo

##########################################################################
echo "=== [4] kill the original VMM — the checkpoint's temporal gap"
kill -9 "$CH_PID" 2>/dev/null; wait "$CH_PID" 2>/dev/null
# P8 trap 1: v53 keeps a LOCK FILE beside the API socket. A SIGKILLed VMM leaves
# it and the next VMM refuses to start with StartVmmThread(ApiSocketInUse(...)).
rm -f "$API" "$API.lock"
echo "    original VMM gone; API socket AND .lock removed"
echo "    vsock dir as the SIGKILL left it:"
ls -la "$VSOCK_DIR" 2>/dev/null | sed 's/^/      /'

# Order matters and is deliberate: the VMM is killed BEFORE the listeners, so a
# listener death can never propagate into the guest's saved socket state. The
# snapshot was taken with the peer alive.
case "$MODE" in
  drop|stalesock|nosockdir)
    echo "    killing BOTH host listeners (pids $LP_HELD $LP_NEW) — the gap applies to them too"
    kill -9 "$LP_HELD" "$LP_NEW" 2>/dev/null; sleep 0.3
    rm -f "${CHVSOCK}_${PORT_HELD}" "${CHVSOCK}_${PORT_NEW}"
    LP_HELD=""; LP_NEW=""
    ;;
  keep)
    echo "    leaving BOTH host listeners ALIVE (pids $LP_HELD $LP_NEW) — the contrast arm"
    ;;
esac

case "$MODE" in
  drop|keep)
    rm -f "$CHVSOCK"
    echo "    removed the stale ch.vsock UDS left behind by the SIGKILLed VMM"
    ;;
  stalesock)
    echo "    LEAVING the stale ch.vsock in place ON PURPOSE (Q3: does bind() collide?)"
    ;;
  nosockdir)
    rm -rf "$VSOCK_DIR"
    echo "    DELETED the entire vsock socket directory $VSOCK_DIR (Q3 proper)"
    ;;
esac
ls -la "$VSOCK_DIR" 2>/dev/null | sed 's/^/      /' || echo "      (directory does not exist)"
echo

##########################################################################
echo "=== [5] restore into a NEW VMM — via the API, NOT the CLI (P8: the CLI is a silent no-op)"
cloud-hypervisor --api-socket "path=$API" >"$CHLOG_B" 2>&1 &
CH_PID2=$!
for _ in $(seq 1 100); do [ -S "$API" ] && break; sleep 0.1; done
# ONE call, not two: `vm.restore` is not idempotent, so asking twice (once for
# the body, once for the status) would make the second attempt fail against an
# already-restored VM and report a bogus refusal.
RESTORE_RAW="$(curl -s -w '\n%{http_code}' --unix-socket "$API" \
  -X PUT -H 'Content-Type: application/json' \
  -d "{\"source_url\":\"file://$SNAPROOT/snap\"}" \
  "http://localhost/api/v1/vm.restore")"
RESTORE_CODE="$(printf '%s' "$RESTORE_RAW" | tail -1)"
RESTORE_BODY="$(printf '%s' "$RESTORE_RAW" | head -n -1)"
echo "    PUT vm.restore -> HTTP $RESTORE_CODE"
[ -n "$RESTORE_BODY" ] && echo "    response body: $RESTORE_BODY"
if [ "$RESTORE_CODE" != "204" ] && [ "$RESTORE_CODE" != "200" ]; then
  # DISTINGUISH THE TWO SHAPES. `HTTP 000` means curl never got a response —
  # which is what happens both when CH refuses at the transport level AND when
  # the VMM process was killed out from under it (the first `drop` run: a
  # concurrent probe's `pkill -x cloud-hyperviso`). Reporting the second as a
  # restore refusal would be a false Q3 finding, so check liveness explicitly.
  if ! kill -0 "$CH_PID2" 2>/dev/null; then
    echo "!!! HARNESS DEFECT: the restoring VMM (pid $CH_PID2) is GONE — it did not"
    echo "!!! refuse the restore, it died. Almost always a foreign pkill on this"
    echo "!!! shared box. This is NOT a Q3 result. Foreign processes now:"
    foreign_procs | cut -c1-120 | sed 's/^/    /'
    echo "--- CH log from the restoring VMM ($(stat -c %s "$CHLOG_B" 2>/dev/null) bytes):"
    head -30 "$CHLOG_B" | sed 's/^/    /'
    exit 4
  fi
  echo "!!! RESTORE REFUSED (HTTP $RESTORE_CODE) — VMM pid $CH_PID2 is still alive, so this is CH's own answer"
  echo "--- CH log from the restoring VMM:"
  head -30 "$CHLOG_B" | sed 's/^/    /'
  echo
  # Is a refused restore RECOVERABLE on the SAME VMM, or must the driver respawn
  # it? The answer decides whether cleanup-and-retry is a cheap in-place fix or a
  # full re-spawn, so measure it rather than assume.
  echo "--- Q3 follow-up: remediate the path and retry vm.restore on the SAME VMM (pid $CH_PID2)"
  mkdir -p "$VSOCK_DIR"; rm -f "$CHVSOCK"
  echo "    (mkdir -p the directory; unlink the stale ch.vsock)"
  RETRY_RAW="$(curl -s -w '\n%{http_code}' --unix-socket "$API" \
    -X PUT -H 'Content-Type: application/json' \
    -d "{\"source_url\":\"file://$SNAPROOT/snap\"}" \
    "http://localhost/api/v1/vm.restore")"
  RETRY_CODE="$(printf '%s' "$RETRY_RAW" | tail -1)"
  echo "    retry PUT vm.restore -> HTTP $RETRY_CODE"
  [ -n "$(printf '%s' "$RETRY_RAW" | head -n -1)" ] && \
    echo "    response body: $(printf '%s' "$RETRY_RAW" | head -n -1)"
  echo
  echo "=========================== VERDICT (mode=$MODE) ==========================="
  echo "  Q3 ANSWER for this mode: RESTORE FAILS. HTTP $RESTORE_CODE."
  echo "  Q3 follow-up: after remediating the path, retry on the SAME VMM -> HTTP $RETRY_CODE"
  echo "=========================== END VERDICT ===================================="
  kill -9 "$CH_PID2" 2>/dev/null
  exit 0
fi
echo "=== [6] vm.resume -> HTTP $(api PUT vm.resume)"

# The deliberate gap: in `drop`/`stalesock` no listener exists yet, so the next
# few guest ticks record exactly what a guest sees when nothing is bound on the
# host — the negative half of Q4, which is otherwise invisible.
GAP_SECS=3
echo "=== [7] ticking for ${GAP_SECS}s with the post-restore host side as-is"
sleep "$GAP_SECS"
cp "$CONSOLE" "$RUN/console-gap.log" 2>/dev/null
echo "--- guest ticks during the gap:"
grep -E '^TICK' "$RUN/console-gap.log" 2>/dev/null | tail -4 | sed 's/^/    /'
echo

case "$MODE" in
  drop|stalesock)
    echo "=== [8] Q4: bind FRESH host listeners on the SAME paths, AFTER resume"
    "$LISTENER" "${CHVSOCK}_${PORT_HELD}" HELD2  >"$RUN/held-after.log" 2>&1 &
    LP_HELD2=$!
    "$LISTENER" "${CHVSOCK}_${PORT_NEW}"  RECON2 >"$RUN/recon-after.log" 2>&1 &
    LP_NEW2=$!
    sleep 0.7
    echo "    fresh listener pids: held2=$LP_HELD2 recon2=$LP_NEW2"
    head -2 "$RUN/held-after.log" "$RUN/recon-after.log" 2>/dev/null | sed 's/^/    /'
    ;;
  keep)
    echo "=== [8] listeners were never killed; nothing to re-bind"
    ;;
  nosockdir)
    echo "=== [8] socket directory is gone; no listener can be bound"
    ;;
esac
sleep 5

##########################################################################
echo
echo "=========================== POST-RESTORE CONSOLE ==========================="
grep -E '^(init:|TICK)' "$CONSOLE" 2>/dev/null | head -3 | sed 's/^/    /'
echo "    ..."
grep -E '^(init:|TICK)' "$CONSOLE" 2>/dev/null | tail -6 | sed 's/^/    /'
echo "=========================== END POST-RESTORE ==============================="
echo

NONCE_AFTER="$(grep -oE 'nonce=[0-9a-f]+' "$CONSOLE" 2>/dev/null | tail -1 | cut -d= -f2)"
LAST_TICK_AFTER="$(grep -oE '^TICK n=[0-9]+' "$CONSOLE" 2>/dev/null | tail -1 | grep -oE '[0-9]+')"
FIRST_TICK_AFTER="$(grep -oE '^TICK n=[0-9]+' "$CONSOLE" 2>/dev/null | head -1 | grep -oE '[0-9]+')"
# `grep -c` exits 1 with "0" on stdout when there is no match, so `|| echo 0`
# emits a SECOND zero and the variable becomes "0\n0". Force a clean integer.
REBOOTED_BANNER="$(grep -c 'VSOCK-SNAP PROBE up' "$CONSOLE" 2>/dev/null | head -1)"
: "${REBOOTED_BANNER:=0}"

echo "=========================== MEMORY-RESTORE CHECK ==========================="
printf '  %-26s %s\n' "BOOT_NONCE before"  "${NONCE_BEFORE:-<none>}"
printf '  %-26s %s\n' "BOOT_NONCE after"   "${NONCE_AFTER:-<none>}"
printf '  %-26s %s\n' "last tick before"   "${LAST_TICK_BEFORE:-<none>}"
printf '  %-26s %s\n' "first tick after"   "${FIRST_TICK_AFTER:-<none>}"
printf '  %-26s %s\n' "last tick after"    "${LAST_TICK_AFTER:-<none>}"
printf '  %-26s %s\n' "boot banners after" "$REBOOTED_BANNER  (must be 0 — a banner means it re-ran main(), i.e. REBOOTED)"
MEMOK=no
if [ -n "$NONCE_AFTER" ] && [ "$NONCE_BEFORE" = "$NONCE_AFTER" ] \
   && [ "$REBOOTED_BANNER" = "0" ] \
   && [ -n "$LAST_TICK_AFTER" ] && [ -n "$LAST_TICK_BEFORE" ] \
   && [ "$LAST_TICK_AFTER" -gt "$LAST_TICK_BEFORE" ]; then
  MEMOK=yes
  echo "  +++ RESTORED FROM MEMORY: nonce identical, no boot banner, counter continued."
else
  echo "  !!! NOT a memory restore — every vsock claim below would describe a FRESH BOOT."
fi
echo

##########################################################################
echo "=========================== S-3 VERDICT (mode=$MODE) ======================="
echo "  Q1 — VM with --vsock snapshots and restores:"
echo "       snapshot HTTP $SNAP_CODE   restore HTTP $RESTORE_CODE   memory-restore=$MEMOK"
echo
echo "  Q2 — the ESTABLISHED connection, held open across the checkpoint"
echo "       guest-side, pre-snapshot  : $(grep -oE 'held_w=[^ ]+ held_r=[^ ]+' "$RUN/console-before-snapshot.log" 2>/dev/null | tail -1)"
echo "       guest-side, FIRST post-restore tick:"
grep -E '^TICK' "$CONSOLE" 2>/dev/null | head -1 | grep -oE 'held_w=[^ ]+ held_r=[^ ]+' | sed 's/^/         /'
echo "       guest-side, distinct post-restore statuses:"
grep -oE 'held_w=[^ ]+ held_r=[^ ]+' "$CONSOLE" 2>/dev/null | sort -u | sed 's/^/         /'
HELD_AFTER_HOST=0
for f in "$RUN/held.log" "$RUN/held-after.log"; do
  [ -f "$f" ] || continue
  c="$(awk -v t="${LAST_TICK_BEFORE:-0}" '
      match($0, /HELD n=[0-9]+/) {
        s=substr($0, RSTART+7, RLENGTH-7); if (s+0 > t+0) c++
      } END { print c+0 }' "$f")"
  HELD_AFTER_HOST=$((HELD_AFTER_HOST + c))
done
echo "       HOST-SIDE: bytes received on the OLD connection with tick > $LAST_TICK_BEFORE : $HELD_AFTER_HOST"
echo "         (guest send() returning success while this stays 0 == silently stale half-open;"
echo "          an errno on the guest side with this 0 == a clean reset)"
echo
echo "  Q3 — the host-side socket path"
echo "       mode=$MODE   restore HTTP $RESTORE_CODE"
echo "       vsock dir now: $(ls -la "$VSOCK_DIR" 2>/dev/null | tail -n +2 | awk '{print $9}' | tr '\n' ' ' || echo '<gone>')"
echo
echo "  Q4 — a FRESH connection after restore"
echo "       guest-side, pre-snapshot  : $(grep -oE 'new=[^ ]+' "$RUN/console-before-snapshot.log" 2>/dev/null | tail -1)"
echo "       guest-side, during the gap (no listener bound in drop/stalesock):"
grep -oE 'new=[^ ]+' "$RUN/console-gap.log" 2>/dev/null | sort -u | sed 's/^/         /'
echo "       guest-side, distinct post-restore statuses overall:"
grep -oE 'new=[^ ]+' "$CONSOLE" 2>/dev/null | sort -u | sed 's/^/         /'
echo "       guest-side new_ok_total, last tick: $(grep -oE 'new_ok_total=[0-9]+' "$CONSOLE" 2>/dev/null | tail -1)"
RECON_AFTER_HOST=0
for f in "$RUN/recon.log" "$RUN/recon-after.log"; do
  [ -f "$f" ] || continue
  c="$(awk -v t="${LAST_TICK_BEFORE:-0}" '
      match($0, /NEW n=[0-9]+/) {
        s=substr($0, RSTART+6, RLENGTH-6); if (s+0 > t+0) c++
      } END { print c+0 }' "$f")"
  RECON_AFTER_HOST=$((RECON_AFTER_HOST + c))
done
echo "       HOST-SIDE: NEW-connection payloads received with tick > $LAST_TICK_BEFORE : $RECON_AFTER_HOST"
echo
echo "  --- host listener transcripts, tails ---"
for f in held.log held-after.log recon.log recon-after.log; do
  [ -f "$RUN/$f" ] || continue
  echo "      == $f =="
  tail -5 "$RUN/$f" | sed 's/^/         /'
done
echo "=========================== END VERDICT ===================================="

kill -9 "$CH_PID2" 2>/dev/null
