#!/usr/bin/env bash
# PROBE increment-i / P11 side-probe — is the vublk durable number REAL?
#
# WHY THIS EXISTS, and it is the whole reason this probe is trustworthy. With
# shmem THP enabled, the vublk arm measured 2958 MiB/s "durable" (incl. fsync)
# against a host `dd bs=1M count=256 conv=fsync` baseline of ~1163 MiB/s. A
# guest cannot durably write 2.5x faster than the host it runs on. Either
#
#   (a) the FLUSH is being dropped, and the number is not durable at all, or
#   (b) qemu-storage-daemon's aio=threads backend genuinely extracts more from
#       the NVMe than single-threaded dd does, and the dd baseline — not the
#       vublk number — is the misleading one.
#
# Publishing (a) as a throughput win would be a confident-but-wrong positive of
# exactly the shape this spike has already been burned by twice. So:
#
#   1. `noflush` is the CONTROL. cache.no-flush=on tells qemu to DISCARD flush
#      requests. If `noflush` measures the same as `writeback`, then flushes
#      were never doing anything and the writeback number is not durable.
#   2. `direct` is the FLOOR. cache.direct=on is O_DIRECT — nothing lands in the
#      host page cache, so the number cannot be inflated by it.
#   3. A parallel-dd ceiling establishes what the DEVICE can actually do, which
#      is what decides between (a) and (b). Single-threaded dd is a latency
#      measurement wearing a throughput costume.
#
# Runs in the shmem_enabled=advise regime, since that is where the surprising
# number appeared. Restores the knob on exit.
#
# Usage: durability-probe.sh [trials]   (default 3)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-i
RESULTS="$OUT/durability-probe.txt"
TRIALS="${1:-3}"
KNOB=/sys/kernel/mm/transparent_hugepage/shmem_enabled
mkdir -p "$OUT"; : >"$RESULTS"

ORIG="$(sed -n 's/.*\[\(.*\)\].*/\1/p' "$KNOB")"
restore() { echo "$ORIG" >"$KNOB" 2>/dev/null; echo "--- restored shmem_enabled=$ORIG"; }
trap restore EXIT
echo advise >"$KNOB"

echo "=================================================================="
echo "vublk durability probe — shmem_enabled=$(cat $KNOB)"
echo "=================================================================="
echo
echo "### [0] what can the DEVICE actually do? (the dd baseline is single-threaded)"
CEIL=/srv/vm/p6f/ceiling; mkdir -p "$CEIL"
echo "--- single-threaded, buffered + fsync (the baseline every arm is quoted against):"
S=$(date +%s.%N); dd if=/dev/zero of="$CEIL/s.bin" bs=1M count=256 conv=fsync 2>&1 | tail -1; E=$(date +%s.%N)
echo "    -> $(echo "256 / ($E - $S)" | bc -l | cut -c1-7) MiB/s"
echo "--- 4 concurrent writers, buffered + fsync (what a threaded backend can reach):"
S=$(date +%s.%N)
for j in 0 1 2 3; do dd if=/dev/zero of="$CEIL/p$j.bin" bs=1M count=64 conv=fsync 2>/dev/null & done; wait
E=$(date +%s.%N)
echo "    -> $(echo "256 / ($E - $S)" | bc -l | cut -c1-7) MiB/s aggregate"
rm -rf "$CEIL"
echo

sample() {
  local cache="$1" i="$2"
  local log="$OUT/dur-$cache-$i.log"
  printf '  VUB_CACHE=%-10s trial %s ... ' "$cache" "$i"
  VUB_CACHE="$cache" "$HERE/run.sh" vublk >"$log" 2>&1
  if [ $? = 0 ]; then
    echo ok
    { echo "cache=$cache trial=$i"
      grep -ho 'FS-THROUGHPUT.*' "$log" | head -1 | sed 's/^/    /'
      grep -ho 'FS-LATENCY.*'    "$log" | head -1 | sed 's/^/    /'
      grep -h 'payload.bin  :'   "$log" | head -1
      grep -h 'small files  :'   "$log" | head -1
      grep -h 'BYTE-IDENTICAL\|DIFFERS\|MISSING' "$log" | head -1
    } >>"$RESULTS"
  else
    echo INCOMPLETE; echo "cache=$cache trial=$i INCOMPLETE" >>"$RESULTS"
  fi
}

echo "### [1] the three caching contracts"
for i in $(seq 1 "$TRIALS"); do
  for c in writeback noflush direct; do sample "$c" "$i"; done
done

echo
echo "=================================================================="
echo "ALL SAMPLES"
echo "=================================================================="
cat "$RESULTS"
echo
echo "=== durable MiB/s (incl. fsync)"
for c in writeback noflush direct; do
  printf '    %-10s ' "$c"
  grep -A1 "cache=$c " "$RESULTS" | grep -o '[0-9.]* MiB/s (incl. fsync)' | sed 's/ MiB.*//' | tr '\n' ' '; echo
done
echo "=== write/fsync split, seconds"
for c in writeback noflush direct; do
  printf '    %-10s ' "$c"
  grep -A1 "cache=$c " "$RESULTS" | grep -o 'write=[0-9.]*s fsync=[0-9.]*s' \
    | sed 's/write=//;s/s fsync=/\//;s/s$//' | tr '\n' ' '; echo
done
echo "=== per-file ms"
for c in writeback noflush direct; do
  printf '    %-10s ' "$c"
  grep -A2 "cache=$c " "$RESULTS" | grep -o 'mean [0-9.]* ms/file' | sed 's/mean //;s/ ms.*//' | tr '\n' ' '; echo
done
echo
echo "READ THIS: if writeback == noflush, the FLUSH is a no-op and the"
echo "writeback 'durable' number is NOT durable. If writeback > noflush is"
echo "false but direct is much lower, the host page cache is doing the work."
echo
echo "results: $RESULTS"
