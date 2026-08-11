#!/usr/bin/env bash
# PROBE increment-i / P11 side-probe — is the `shared=on` write-phase collapse
# recoverable with a host setting?
#
# The four-arm bench found `--memory shared=on` costs plain `--disk` 55% of its
# durable streaming throughput (986 -> 447 MiB/s, ranges disjoint). The /proc
# capture then showed WHY, and it is not subtle:
#
#     plain         AnonHugePages: 264192 kB   ShmemPmdMapped: 0 kB
#     plain-shared  AnonHugePages:      0 kB   ShmemPmdMapped: 0 kB
#     host policy   anon : always [madvise] never
#                   shmem: always within_size advise [never] deny force
#
# `shared=on` backs guest RAM with a memfd instead of anonymous memory. CH
# madvise's its guest RAM, so the anonymous case gets 2 MiB transparent huge
# pages; the shmem case cannot, because the distro default for
# /sys/kernel/mm/transparent_hugepage/shmem_enabled is `never`. Every guest page
# fault in the write path then runs at 4 KiB granularity.
#
# That is a HOST TUNABLE, which makes the difference between "vhost-user costs
# 55% of streaming throughput" and "the distro default costs it, and one setting
# recovers it". This probe measures which.
#
# Restores the original setting on exit, including on failure.
#
# Usage: thp-probe.sh [trials]   (default 3)
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-i
RESULTS="$OUT/thp-probe.txt"
TRIALS="${1:-3}"
KNOB=/sys/kernel/mm/transparent_hugepage/shmem_enabled
mkdir -p "$OUT"; : >"$RESULTS"

ORIG="$(sed -n 's/.*\[\(.*\)\].*/\1/p' "$KNOB")"
restore() { echo "$ORIG" >"$KNOB" 2>/dev/null; echo "--- restored shmem_enabled=$ORIG"; }
trap restore EXIT

echo "=================================================================="
echo "shmem THP probe — does it recover the shared=on write phase?"
echo "knob     : $KNOB"
echo "original : $ORIG"
echo "trials   : $TRIALS per (setting x arm)"
echo "=================================================================="

sample() {
  local setting="$1" arm="$2" i="$3"
  local log="$OUT/thp-$setting-$arm-$i.log"
  printf '  shmem_enabled=%-8s %-13s trial %s ... ' "$setting" "$arm" "$i"
  "$HERE/run.sh" "$arm" >"$log" 2>&1
  if [ $? = 0 ]; then
    echo ok
    { echo "setting=$setting arm=$arm trial=$i"
      grep -ho 'FS-THROUGHPUT.*' "$log" | head -1 | sed 's/^/    /'
      grep -ho 'FS-LATENCY.*'    "$log" | head -1 | sed 's/^/    /'
      grep -A4 'huge pages backing' "$log" | grep -E 'AnonHugePages|ShmemPmdMapped' | sed 's/^/    /'
    } >>"$RESULTS"
  else
    echo INCOMPLETE
    echo "setting=$setting arm=$arm trial=$i INCOMPLETE" >>"$RESULTS"
  fi
}

for setting in "$ORIG" advise force; do
  # `advise` only grants huge pages to mappings that ask via madvise; `force`
  # grants them regardless. Running both distinguishes "CH does not madvise its
  # memfd" from "the policy forbids it outright".
  echo "$setting" >"$KNOB" || { echo "!!! cannot set $KNOB=$setting"; continue; }
  echo
  echo "### shmem_enabled now: $(cat "$KNOB")"
  for i in $(seq 1 "$TRIALS"); do
    for arm in plain-shared vublk; do sample "$setting" "$arm" "$i"; done
  done
done

echo
echo "=================================================================="
echo "ALL SAMPLES"
echo "=================================================================="
cat "$RESULTS"
echo
echo "=== durable MiB/s (incl. fsync), by setting x arm"
for setting in "$ORIG" advise force; do
  for arm in plain-shared vublk; do
    printf '    shmem=%-8s %-13s ' "$setting" "$arm"
    grep -A1 "setting=$setting arm=$arm " "$RESULTS" \
      | grep -o '[0-9.]* MiB/s (incl. fsync)' | sed 's/ MiB.*//' | tr '\n' ' '; echo
  done
done
echo
echo "=== ShmemPmdMapped kB at the beacon (0 = no huge pages behind guest RAM)"
for setting in "$ORIG" advise force; do
  printf '    shmem=%-8s ' "$setting"
  grep -A4 "setting=$setting arm=plain-shared " "$RESULTS" \
    | grep -o 'ShmemPmdMapped: *[0-9]*' | grep -o '[0-9]*$' | tr '\n' ' '; echo
done
echo
echo "results: $RESULTS"
