#!/usr/bin/env bash
# PROBE increment-l — cold-boot-to-ready vs restore-to-ready.
#
#   bench.sh <mem_mib> <trials>
#
# The economic question: if cold boot is fast enough, keeping a memory snapshot
# at all may not be worth the disk. So all three modes are timed to the SAME
# observable event — the guest's first post-start work-counter write — and the
# headline is `spawn_ready_ms`, measured from the spawn of the incarnation that
# will serve. P13 reported the `vm.restore` CALL; a pool driver also has to
# spawn a VMM and wait for its socket, and cold boot has no other number to
# give, so only spawn-to-ready is apples to apples. Both are printed.
#
# INTERLEAVED (cold, restore-copy, restore-ondemand, cold, ...) rather than in
# blocks, so drift in box state over the run lands on every arm instead of on
# whichever arm happened to run last.
#
# Every trial is COLD CACHE, and the harness sync(2)s BEFORE drop_caches —
# drop_caches cannot evict dirty pages, and increment-k's first cut "read" 2 GiB
# at 8.8 GB/s off a 2.6 GB/s device because of exactly that omission.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-l
EV="${EV:-/var/tmp/spike-increment-l-evidence}"
MEM="${1:-2048}"
TRIALS="${2:-5}"
TOUCH="${TOUCH:-$(( MEM * 3 / 4 ))}"
STAMP="$(date +%Y%m%dT%H%M%S)"
mkdir -p "$EV"
LOG="$EV/bench-${MEM}m-${STAMP}.txt"
MODES=(cold restore-copy restore-ondemand)

{
echo "##################################################################"
echo "### increment-l BENCH  mem=${MEM}MiB touch=${TOUCH}MiB trials=${TRIALS}"
echo "### modes (interleaved): ${MODES[*]}"
echo "### observable         : the guest's FIRST post-start WORK record"
echo "### cloud-hypervisor   : $(cloud-hypervisor --version 2>&1 | head -1)"
echo "### kernel/arch        : $(uname -r) $(uname -m)"
echo "### started            : $(date -Is)"
echo "##################################################################"

echo
echo "### DEVICE BASELINE — cold sequential read of one ${MEM} MiB memory-ranges."
echo "### Any restore that beats this is reading from RAM, not the device."
sync; echo 3 > /proc/sys/vm/drop_caches
BASEFILE="$(find /srv/vm/p14l -name memory-ranges 2>/dev/null | head -1)"
if [ -n "$BASEFILE" ]; then
  dd if="$BASEFILE" of=/dev/null bs=1M iflag=direct 2>&1 | tail -1 | sed 's/^/###   O_DIRECT : /'
  sync; echo 3 > /proc/sys/vm/drop_caches
  dd if="$BASEFILE" of=/dev/null bs=1M 2>&1 | tail -1 | sed 's/^/###   buffered : /'
else
  echo "###   (no prior snapshot yet; baseline re-printed at the end)"
fi

for t in $(seq 1 "$TRIALS"); do
  for m in "${MODES[@]}"; do
    echo
    echo "=================================================================="
    echo "=== TRIAL $t / $TRIALS   mode=$m   mem=${MEM}MiB   $(date -Is)"
    echo "=================================================================="
    "$OUT/host-cache" --cmd bench --bench-mode "$m" \
      --label "b${t}-${m}-${MEM}" \
      --mem-mib "$MEM" --touch-mib "$TOUCH" --boot-ticks 40 \
      --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4" \
      --vol-src "$OUT/vol-seeded.ext4"
  done
done

echo
echo "### DEVICE BASELINE (again, at the END of the run)"
sync; echo 3 > /proc/sys/vm/drop_caches
BASEFILE="$(find /srv/vm/p14l -name memory-ranges 2>/dev/null | head -1)"
[ -n "$BASEFILE" ] && dd if="$BASEFILE" of=/dev/null bs=1M iflag=direct 2>&1 | tail -1 | sed 's/^/###   O_DIRECT : /'
} 2>&1 | tee "$LOG"

echo
echo "=== L-BENCH lines from $LOG:"
grep '^L-BENCH' "$LOG" || true
echo
echo "=== per-mode ranges (EVERY sample, nothing averaged)"
for m in "${MODES[@]}"; do
  echo "--- $m"
  for k in spawn_ready_ms call_ms resume_ms status genuine; do
    printf '    %-18s : ' "$k"
    grep '^L-BENCH' "$LOG" | grep " mode=$m " | tr ' ' '\n' | grep "^${k}=" | cut -d= -f2 | tr '\n' ' '
    echo
  done
done
echo
echo "log: $LOG"
