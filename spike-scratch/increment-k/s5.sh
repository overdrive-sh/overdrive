#!/usr/bin/env bash
# PROBE increment-k — S-5: restore the SAME snapshot into N simultaneous VMs.
#
#   s5.sh <n> <mem_mib> [snapshot_dir]
#
# This is the warm-pool economics question, and it is separate from the
# latency measurement — reported separately so it cannot dilute it.
#
# Two phases, because the naive form does not survive contact:
#
#   PHASE A — restore the SAME snapshot directory into 2 VMMs, unmodified.
#             The snapshot's config.json records ABSOLUTE paths for the rootfs
#             disk and the serial file, so N VMMs would open the same image.
#             v53 locks block devices ("Cannot lock images of all block
#             devices"), so this is expected to fail — and a warm pool has to
#             know that before it designs around it.
#
#   PHASE B — one snapshot COPY per VM, reflinked (P4: reflink on XFS is ~260x
#             faster than a copy and costs no space), with config.json rewritten
#             to a per-VM rootfs and console path. Then measure whether the host
#             pays N x guest RAM or something less.
#
# The memory question is answered three ways, because one way is not enough:
#   * sum of per-VMM VmRSS,
#   * host MemAvailable delta against a pre-launch baseline, and
#   * Pss from smaps_rollup — if Pss ~= Rss then NOTHING is shared, whatever
#     the reflinked file on disk suggests.
set -uo pipefail

N="${1:-4}"
MEM="${2:-2048}"
SRC_SNAP="${3:-}"
RUN=/run/spike-increment-k-s5
POOL=/srv/vm/p13k-s5
OUT=/var/tmp/spike-increment-k

if [ -z "$SRC_SNAP" ]; then
  SRC_SNAP="$(ls -d /srv/vm/p13k/snap-*${MEM} 2>/dev/null | head -1)"
fi
[ -d "$SRC_SNAP" ] || { echo "!!! no source snapshot; run bench.sh first" >&2; exit 1; }

PIDS=()
cleanup() { for p in "${PIDS[@]:-}"; do [ -n "$p" ] && kill -9 "$p" 2>/dev/null; done; }
trap cleanup EXIT

api() { curl -s -o /dev/null -w '%{http_code}' --unix-socket "$1" -X PUT \
          -H 'Content-Type: application/json' -d "$3" "http://localhost/api/v1/$2"; }
api_np() { curl -s -o /dev/null -w '%{http_code}' --unix-socket "$1" -X PUT \
          "http://localhost/api/v1/$2"; }
avail() { awk '/MemAvailable/{print $2}' /proc/meminfo; }

echo "##################################################################"
echo "### increment-k S-5 — N=$N simultaneous restores of ONE snapshot"
echo "### source snapshot : $SRC_SNAP"
echo "### guest RAM       : ${MEM} MiB   =>  naive expectation N x RAM = $(( N * MEM )) MiB"
echo "### host            : $(uname -r) $(uname -m)  MemAvailable=$(avail) kB"
echo "### date            : $(date -Is)"
echo "##################################################################"
echo
echo "### snapshot config.json — note the ABSOLUTE paths that make phase A fail:"
python3 - "$SRC_SNAP/config.json" <<'PY'
import json,sys
c=json.load(open(sys.argv[1]))
print("###   disks  :", [d.get("path") for d in (c.get("disks") or [])])
print("###   serial :", (c.get("serial") or {}).get("file"))
print("###   memory :", c.get("memory",{}).get("size"))
PY

rm -rf "$RUN" "$POOL"; mkdir -p "$RUN" "$POOL"

##########################################################################
echo
echo "=================================================================="
echo "=== PHASE A — 2 VMMs, the SAME snapshot dir, nothing rewritten"
echo "=================================================================="
# HARNESS DEFECT, first cut: the bench deletes its per-run rootfs on exit, so
# the absolute disk path baked into config.json pointed at a MISSING file and
# BOTH VMs failed with ENOENT before either could contend for the lock. The
# lock hypothesis was never actually tested, and reporting that first run as
# "N-way restore is refused" would have been a confident wrong negative with
# the wrong mechanism attached. Recreate the file at the recorded path so the
# phase tests what it claims to.
DISKPATH="$(python3 -c 'import json,sys;print((json.load(open(sys.argv[1])).get("disks") or [{}])[0].get("path",""))' "$SRC_SNAP/config.json")"
if [ -n "$DISKPATH" ] && [ ! -f "$DISKPATH" ]; then
  mkdir -p "$(dirname "$DISKPATH")"
  cp --reflink=auto "$OUT/rootfs.ext4" "$DISKPATH"
  echo "### recreated the snapshot's recorded rootfs at $DISKPATH"
  echo "### (without this both VMs die on ENOENT and the lock is never reached)"
fi
for i in 0 1; do
  S="$RUN/a$i.sock"
  cloud-hypervisor --api-socket "path=$S" >"$RUN/a$i.log" 2>&1 &
  PIDS+=($!)
  for _ in $(seq 1 200); do [ -S "$S" ] && break; sleep 0.05; done
  C="$(api "$S" vm.restore "{\"source_url\":\"file://$SRC_SNAP\",\"memory_restore_mode\":\"Copy\"}")"
  R="$(api_np "$S" vm.resume)"
  echo "  VM$i  restore -> HTTP $C   resume -> HTTP $R"
  grep -iE "error|lock" "$RUN/a$i.log" | head -3 | sed 's/^/        log: /'
done
for p in "${PIDS[@]}"; do kill -9 "$p" 2>/dev/null; done
PIDS=(); sleep 1; rm -f "$RUN"/a*.sock*

##########################################################################
echo
echo "=================================================================="
echo "=== PHASE B — one reflinked snapshot copy per VM, paths rewritten"
echo "=================================================================="
for MODE in Copy OnDemand; do
  rm -rf "$POOL"; mkdir -p "$POOL"
  rm -f "$RUN"/b*.sock*
  # Per-VM snapshot copy. --reflink=auto so memory-ranges costs no space on
  # XFS; config.json is rewritten to a per-VM rootfs + console.
  for i in $(seq 0 $((N-1))); do
    cp -a --reflink=auto "$SRC_SNAP" "$POOL/vm$i"
    cp --reflink=auto "$OUT/rootfs.ext4" "$POOL/rootfs$i.ext4"
    python3 - "$POOL/vm$i/config.json" "$POOL/rootfs$i.ext4" "$POOL/console$i.log" <<'PY'
import json,sys
p,disk,ser=sys.argv[1],sys.argv[2],sys.argv[3]
c=json.load(open(p))
for d in (c.get("disks") or []): d["path"]=disk
if c.get("serial"): c["serial"]["file"]=ser
json.dump(c,open(p,"w"))
PY
  done
  sync; echo 3 > /proc/sys/vm/drop_caches; sleep 0.5
  BASE="$(avail)"
  echo
  echo "--- mode=$MODE   baseline MemAvailable=$BASE kB"
  T0=$(date +%s.%N)
  for i in $(seq 0 $((N-1))); do
    S="$RUN/b$i.sock"
    cloud-hypervisor --api-socket "path=$S" >"$RUN/b$i.log" 2>&1 &
    PIDS+=($!)
    for _ in $(seq 1 200); do [ -S "$S" ] && break; sleep 0.05; done
    C="$(api "$S" vm.restore "{\"source_url\":\"file://$POOL/vm$i\",\"memory_restore_mode\":\"$MODE\"}")"
    R="$(api_np "$S" vm.resume)"
    echo "    VM$i restore=$C resume=$R pid=${PIDS[-1]}"
    grep -iE "error" "$RUN/b$i.log" | head -2 | sed 's/^/          log: /'
  done
  T1=$(date +%s.%N)
  echo "    all $N launched in $(echo "$T1 - $T0" | bc)s"
  # Let OnDemand's background uffd fill finish. Sampling before it completes
  # would report a small number that is about TIMING, not about SHARING.
  for W in 1 5 15; do
    sleep_for=$W; [ "$W" = 5 ] && sleep_for=4; [ "$W" = 15 ] && sleep_for=10
    sleep "$sleep_for"
    SUM=0; ALIVE=0
    for p in "${PIDS[@]}"; do
      if kill -0 "$p" 2>/dev/null; then
        ALIVE=$((ALIVE+1))
        R=$(awk '/^VmRSS:/{print $2}' "/proc/$p/status" 2>/dev/null || echo 0)
        SUM=$((SUM + ${R:-0}))
      fi
    done
    NOW="$(avail)"
    echo "    t+${W}s  alive=$ALIVE/$N  sum(VmRSS)=$SUM kB  MemAvailable=$NOW kB  delta=$((BASE - NOW)) kB"
  done
  echo "    --- per-VMM detail (Pss ~= Rss means NOTHING is shared):"
  for p in "${PIDS[@]}"; do
    kill -0 "$p" 2>/dev/null || continue
    RSS=$(awk '/^VmRSS:/{print $2}' "/proc/$p/status")
    PSS=$(awk '/^Pss:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null)
    SHC=$(awk '/^Shared_Clean:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null)
    SHD=$(awk '/^Shared_Dirty:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null)
    PRD=$(awk '/^Private_Dirty:/{print $2}' "/proc/$p/smaps_rollup" 2>/dev/null)
    echo "        pid=$p Rss=${RSS} Pss=${PSS} Shared_Clean=${SHC} Shared_Dirty=${SHD} Private_Dirty=${PRD} kB"
  done
  echo "    --- on-disk cost of $N reflinked snapshot copies:"
  du -sh --apparent-size "$POOL" | sed 's/^/        apparent: /'
  du -sh "$POOL" | sed 's/^/        on disk : /'
  for p in "${PIDS[@]}"; do kill -9 "$p" 2>/dev/null; done
  PIDS=(); sleep 1
done

rm -rf "$POOL" "$RUN"
echo
echo "=== S-5 DONE"
