#!/usr/bin/env bash
# PROBE increment-l — single-shot entry points.
#
#   run.sh seed                 build vol-seeded.ext4 (the bench's starting volume)
#   run.sh arm <a|b|c|d> <sync> one durability arm
#   run.sh bench <mode> <mem>   one timing trial
#
# The bench's volume is SEEDED rather than blank so that cold boot is charged
# for the journal recovery scan a real cold boot has to do, and so all three
# bench modes start from one identical on-disk state.
set -uo pipefail

OUT=/var/tmp/spike-increment-l
CMD="${1:-seed}"

# trap 6: the box is SHARED. Report foreign VMMs; never sweep them. A concurrent
# agent's `pkill -9 -x cloud-hyperviso` is what corrupted an earlier probe.
FOREIGN="$(pgrep -x cloud-hyperviso 2>/dev/null | tr '\n' ' ')"
if [ -n "${FOREIGN// /}" ]; then
  echo "### WARNING: pre-existing cloud-hypervisor pids on this SHARED box: $FOREIGN"
  echo "### (not killing them — that is exactly what corrupted an earlier probe)"
fi

case "$CMD" in
  seed)
    cp --reflink=auto "$OUT/vol-blank.ext4" "$OUT/vol-seeded.ext4"
    "$OUT/host-cache" --cmd seed --cut-at "${CUT:-300}" \
      --mem-mib 2048 --touch-mib 64 \
      --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4" \
      --vol-src "$OUT/vol-seeded.ext4"
    ;;
  arm)
    "$OUT/host-cache" --cmd arm --arm "${2:-a}" --sync "${3:-1}" \
      --label "one-${2:-a}-s${3:-1}" \
      --mem-mib "${MEM:-2048}" --touch-mib "${TOUCH:-512}" --cut-at "${CUT:-300}" \
      --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4" \
      --vol-src "$OUT/vol-blank.ext4"
    ;;
  bench)
    "$OUT/host-cache" --cmd bench --bench-mode "${2:-cold}" \
      --label "one-${2:-cold}" \
      --mem-mib "${3:-2048}" --touch-mib "${TOUCH:-1536}" --boot-ticks 40 \
      --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4" \
      --vol-src "$OUT/vol-seeded.ext4"
    ;;
  *) echo "usage: run.sh seed | arm <a|b|c|d> <0|1> | bench <mode> <mem>" >&2; exit 1 ;;
esac
