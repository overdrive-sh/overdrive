#!/usr/bin/env bash
# PROBE increment-k — the interleaved bench.
#
#   bench.sh <mem_mib> <trials> [walk_mib]
#
# Modes are INTERLEAVED (copy, ondemand, copy-prefault, copy, ondemand, ...)
# rather than run in blocks, so a drift in box state over the run — thermal,
# another tenant, page-cache shape — lands on every arm instead of on whichever
# arm happened to run last.
#
# Every trial is COLD-CACHE. The harness sync(2)s and then drops caches, and
# PRINTS Cached before/after so a drop that did not drop is visible. That
# matters more than it sounds: the first cut of this probe dropped caches
# without syncing, `vm.snapshot`'s 2 GiB were still DIRTY and therefore
# undroppable, and `copy` "read" them back at 8.8 GB/s against a device that
# does 2.6 GB/s. See the comment in host_ondemand.rs.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MEM="${1:-2048}"
TRIALS="${2:-5}"
WALK_MIB="${3:-2}"
STAMP="$(date +%Y%m%dT%H%M%S)"
LOG="${LOG:-$HERE/evidence/bench-${MEM}m-walk${WALK_MIB}-${STAMP}.txt}"
mkdir -p "$(dirname "$LOG")"

MODES=(copy ondemand copy-prefault)

{
echo "##################################################################"
echo "### increment-k BENCH   mem=${MEM}MiB trials=$TRIALS walk=${WALK_MIB}MiB/tick"
echo "### modes (interleaved): ${MODES[*]}"
echo "### cloud-hypervisor : $(cloud-hypervisor --version 2>&1 | head -1)"
echo "### kernel/arch      : $(uname -r) $(uname -m)  virt=$(systemd-detect-virt || true)"
echo "### started          : $(date -Is)"
echo "##################################################################"

echo
echo "### DEVICE BASELINE — cold sequential read of one 2 GiB memory-ranges."
echo "### Any restore that beats this is reading from RAM, not the device."
sync; echo 3 > /proc/sys/vm/drop_caches
BASEFILE="$(find /srv/vm/p13k -name memory-ranges 2>/dev/null | head -1)"
if [ -n "$BASEFILE" ]; then
  dd if="$BASEFILE" of=/dev/null bs=1M iflag=direct 2>&1 | tail -1 | sed 's/^/###   O_DIRECT : /'
  sync; echo 3 > /proc/sys/vm/drop_caches
  dd if="$BASEFILE" of=/dev/null bs=1M 2>&1 | tail -1 | sed 's/^/###   buffered : /'
else
  echo "###   (no prior snapshot to read; baseline skipped)"
fi

for t in $(seq 1 "$TRIALS"); do
  for m in "${MODES[@]}"; do
    echo
    echo "=================================================================="
    echo "=== TRIAL $t / $TRIALS   mode=$m   mem=${MEM}MiB   $(date -Is)"
    echo "=================================================================="
    WALK="$WALK_MIB" "$HERE/run.sh" one "$m" "$MEM" "t${t}-${m}-${MEM}"
  done
done

echo
echo "##################################################################"
echo "### SUMMARY — every sample printed, nothing averaged away"
echo "##################################################################"
} 2>&1 | tee "$LOG"

echo
echo "=== K-RESULT lines from $LOG:"
grep '^K-RESULT' "$LOG"
echo
echo "=== per-mode ranges (restore_s | resume_to_tick_ms | user_visible_ms | rss_at_restore_kb)"
for m in "${MODES[@]}"; do
  echo "--- $m"
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | \
    sed -E 's/.*restore_s=([0-9.]+).*resume_to_tick_ms=([0-9.-]+) user_visible_ms=([0-9.-]+) rss_at_restore_kb=([0-9]+).*genuine=/\1 \2 \3 \4 /' \
    >/dev/null 2>&1
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep '^restore_s=' | cut -d= -f2 | tr '\n' ' ' | sed 's/^/    restore_s          : /'; echo
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep '^resume_to_tick_ms=' | cut -d= -f2 | tr '\n' ' ' | sed 's/^/    resume_to_tick_ms  : /'; echo
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep '^user_visible_ms=' | cut -d= -f2 | tr '\n' ' ' | sed 's/^/    user_visible_ms    : /'; echo
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep '^rss_at_restore_kb=' | cut -d= -f2 | tr '\n' ' ' | sed 's/^/    rss_at_restore_kb  : /'; echo
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep '^rss_t1000_kb=' | cut -d= -f2 | tr '\n' ' ' | sed 's/^/    rss_t1000_kb       : /'; echo
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep '^genuine=' | cut -d= -f2 | tr '\n' ' ' | sed 's/^/    genuine (must be all true) : /'; echo
  grep "^K-RESULT" "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep '^status=' | cut -d= -f2 | tr '\n' ' ' | sed 's/^/    status                     : /'; echo
done
echo
echo "log: $LOG"
