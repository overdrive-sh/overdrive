#!/usr/bin/env bash
# PROBE increment-l — the four-arm × two-write-mode durability matrix.
#
#   arms.sh [repeats]
#
# Order is INTERLEAVED by write mode within each arm, and the arms are run in
# the order a<->b first, because A-buffered vs B-buffered is THE control: those
# two runs are identical up to one step (restore `memory-ranges` vs delete it),
# so any difference in surviving records is attributable to the memory discard
# and to nothing else in this harness. If BOTH lose records the harness is
# broken, not the model — and that has to be visible before anything else is
# believed.
#
# C vs D is the load-bearing comparison and is never blurred: same crash, the
# single difference being a guest-side syncfs(2)+FIFREEZE first.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-l
EV="${EV:-/var/tmp/spike-increment-l-evidence}"     # OUTSIDE the rsync'd tree
REPEATS="${1:-1}"
MEM="${MEM:-2048}"
TOUCH="${TOUCH:-512}"
CUT="${CUT:-300}"
STAMP="$(date +%Y%m%dT%H%M%S)"
mkdir -p "$EV"
LOG="$EV/arms-${STAMP}.txt"

{
echo "##################################################################"
echo "### increment-l ARMS   mem=${MEM}MiB touch=${TOUCH}MiB cut_at=${CUT} repeats=${REPEATS}"
echo "### cloud-hypervisor : $(cloud-hypervisor --version 2>&1 | head -1)"
echo "### kernel/arch      : $(uname -r) $(uname -m)  virt=$(systemd-detect-virt || true)"
echo "### volume fs        : ext4, mkfs defaults (has_journal, data=ordered)"
echo "### started          : $(date -Is)"
echo "##################################################################"

for r in $(seq 1 "$REPEATS"); do
  for arm in a b c d; do
    for sync in 1 0; do
      echo
      echo "=================================================================="
      echo "=== REPEAT $r  ARM=$arm  SYNC=$sync   $(date -Is)"
      echo "=================================================================="
      "$OUT/host-cache" --cmd arm --arm "$arm" --sync "$sync" \
        --label "r${r}-${arm}-s${sync}" \
        --mem-mib "$MEM" --touch-mib "$TOUCH" --cut-at "$CUT" \
        --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4" \
        --vol-src "$OUT/vol-blank.ext4"
    done
  done

  # One extra C variant with the host-side cache bypassed. The guest's fsync
  # lands in the HOST page cache unless CH opened the backing file O_DIRECT, so
  # SIGKILLing the VMM tests losing the VM, NOT losing the host. `direct=on` is
  # the knob that closes that gap; whether it changes the durability picture
  # says which layer the loss actually lives in.
  echo
  echo "=================================================================="
  echo "=== REPEAT $r  ARM=c  SYNC=1  direct=on   $(date -Is)"
  echo "=================================================================="
  "$OUT/host-cache" --cmd arm --arm c --sync 1 --direct 1 \
    --label "r${r}-c-s1-direct" \
    --mem-mib "$MEM" --touch-mib "$TOUCH" --cut-at "$CUT" \
    --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4" \
    --vol-src "$OUT/vol-blank.ext4"
done

# ---------------------------------------------------------------------------
# LONG-RUN buffered arms.
#
# The short arms write for ~6 s. ext4's jbd2 commits metadata every 5 s, but
# with delayed allocation there is no metadata transaction pending for data that
# has not been written back, and writeback itself only chases pages older than
# `dirty_expire_centisecs` (30 s by default). So a 6 s buffered run can lose
# EVERYTHING, and "everything" is a number that says nothing about where the
# boundary is. These runs write for ~40 s, crossing both thresholds, which turns
# the answer from "all of it" into "everything younger than the writeback
# window" — the form a driver can actually act on.
# ---------------------------------------------------------------------------
echo
echo "### kernel writeback knobs in force for the LONG-RUN arms:"
for k in dirty_expire_centisecs dirty_writeback_centisecs dirty_ratio dirty_background_ratio; do
  echo "###   vm.$k = $(cat /proc/sys/vm/$k)"
done
echo "###   ext4 commit= interval: mount default (5 s)"
for arm in b c d; do
  echo
  echo "=================================================================="
  echo "=== LONGRUN  ARM=$arm  SYNC=0  cut_at=2000 (~40 s of writes)  $(date -Is)"
  echo "=================================================================="
  "$OUT/host-cache" --cmd arm --arm "$arm" --sync 0 \
    --label "long-${arm}-s0" \
    --mem-mib "$MEM" --touch-mib "$TOUCH" --cut-at 2000 \
    --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4" \
    --vol-src "$OUT/vol-blank.ext4"
done

echo
echo "##################################################################"
echo "### SUMMARY — every L-RESULT line, nothing averaged away"
echo "##################################################################"
} 2>&1 | tee "$LOG"

echo
echo "=== L-RESULT lines:"
grep '^L-RESULT' "$LOG" || true
echo
echo "log: $LOG"
