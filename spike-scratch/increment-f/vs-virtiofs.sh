#!/usr/bin/env bash
# virtiofs (increment-e) vs virtio-blk (increment-f), same payload, same box,
# same XFS filesystem underneath, interleaved so host-side drift cannot load
# onto one arm.
#
# The single-sample run showed block FASTER on the streaming write and SLOWER
# per small file, which is counterintuitive enough that it needs more than one
# sample before it goes in a findings doc. Every sample is printed; nothing is
# averaged away.
#
# Usage: vs-virtiofs.sh [trials]     (default 3)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
E="$(cd "$HERE/../increment-e" && pwd)"
OUT=/var/tmp/spike-increment-f
RESULTS="$OUT/vs-virtiofs.txt"
TRIALS="${1:-3}"
mkdir -p "$OUT"
: >"$RESULTS"

echo "=================================================================="
echo "virtiofs vs virtio-blk — $TRIALS interleaved trials each"
echo "both on XFS(reflink=1) /dev/nvme1n1; payload 256 MiB + 1000 files"
echo "=================================================================="

for i in $(seq 1 "$TRIALS"); do
  printf 'trial %s virtiofs ... ' "$i"
  log="$OUT/vs-fs-$i.log"
  VOLROOT=/srv/vm/p6 "$E/run.sh" full >"$log" 2>&1
  if [ $? = 0 ]; then
    echo ok
    { echo "trial=$i mech=virtiofs"
      grep -ho 'FS-THROUGHPUT.*' "$log" | head -1 | sed 's/^/    /'
      grep -ho 'FS-LATENCY.*'    "$log" | head -1 | sed 's/^/    /'
      grep -h  'host 256 MiB'    "$log" | head -1
      grep -h  'host 1000 small' "$log" | head -1; } >>"$RESULTS"
  else
    echo "INCOMPLETE"; echo "trial=$i mech=virtiofs INCOMPLETE" >>"$RESULTS"
  fi

  printf 'trial %s virtio-blk ... ' "$i"
  log="$OUT/vs-blk-$i.log"
  "$HERE/run.sh" blk >"$log" 2>&1
  if [ $? = 0 ]; then
    echo ok
    { echo "trial=$i mech=virtio-blk"
      grep -ho 'FS-THROUGHPUT.*' "$log" | head -1 | sed 's/^/    /'
      grep -ho 'FS-LATENCY.*'    "$log" | head -1 | sed 's/^/    /'
      grep -h  'host 256 MiB'    "$log" | head -1
      grep -h  'host 1000 small' "$log" | head -1
      grep -h  'reflink-cloned'  "$log" | head -1 | sed 's/^/    /'; } >>"$RESULTS"
  else
    echo "INCOMPLETE"; echo "trial=$i mech=virtio-blk INCOMPLETE" >>"$RESULTS"
  fi
done

echo
echo "=================================================================="
echo "ALL SAMPLES"
echo "=================================================================="
cat "$RESULTS"

echo
echo "--- 256 MiB streaming write, MiB/s (write only):"
for m in virtiofs virtio-blk; do
  printf '    %-12s ' "$m"
  grep -A1 "mech=$m\$" "$RESULTS" | grep -o '[0-9.]* MiB/s (write only)' | tr '\n' ' '; echo
done
echo "--- 1000 files, mean ms/file:"
for m in virtiofs virtio-blk; do
  printf '    %-12s ' "$m"
  grep -A2 "mech=$m\$" "$RESULTS" | grep -o 'mean [0-9.]* ms/file' | tr '\n' ' '; echo
done
echo
echo "results: $RESULTS"
