#!/usr/bin/env bash
# PROBE increment-k — ONE trial, or the API-discovery pass.
#
#   run.sh api-probe                 discover what vm.restore ACCEPTS
#   run.sh one <mode> <mem_mib> [label]
#
# <mode> is copy | ondemand | copy-prefault.
#
# The harness is the Rust binary; this script only assembles arguments and
# guarantees the box is in a known state first. Everything that has to be TIMED
# happens inside the harness, because a bash fork is worth more than the
# difference the probe is trying to resolve.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT=/var/tmp/spike-increment-k
SNAPROOT="${SNAPROOT:-/srv/vm/p13k}"
CMD="${1:-one}"
MODE="${2:-copy}"
MEM="${3:-2048}"
LABEL="${4:-${MODE}-${MEM}}"

# Touch 75% of guest RAM. A snapshot of NEVER-TOUCHED memory is essentially all
# zeros, and zeros are the case a demand-paging implementation serves most
# cheaply. Measuring `ondemand` against a zero snapshot would flatter it for a
# reason no real workload enjoys.
TOUCH="${TOUCH:-$(( MEM * 3 / 4 ))}"
WALK="${WALK:-2}"      # MiB per 25 ms tick == 80 MiB/s of demand faults

mkdir -p "$SNAPROOT"

# --- trap 6: the box is SHARED. Never pkill by name; a concurrent agent's
# --- sweep is what corrupted a previous probe. Only report, and let the
# --- harness kill exactly the pids it spawned.
FOREIGN="$(pgrep -x cloud-hyperviso 2>/dev/null | tr '\n' ' ')"
if [ -n "${FOREIGN// /}" ]; then
  echo "### WARNING: pre-existing cloud-hypervisor pids on this SHARED box: $FOREIGN"
  echo "### (not killing them — that is exactly what corrupted an earlier probe)"
fi

echo "##################################################################"
echo "### PROBE increment-k — memory_restore_mode=ondemand vs copy"
echo "### cloud-hypervisor : $(cloud-hypervisor --version 2>&1 | head -1)"
echo "### kernel/arch      : $(uname -r) $(uname -m)  virt=$(systemd-detect-virt || true)"
echo "### snapshot dir fs  : $(findmnt -no FSTYPE --target "$SNAPROOT" 2>/dev/null || echo '?')"
echo "### host mem         : $(awk '/MemTotal|MemAvailable/{printf "%s=%s ", $1, $2}' /proc/meminfo)"
echo "### date             : $(date -Is)"
echo "##################################################################"

ARGS=(
  --label "$LABEL" --mode "$MODE"
  --mem-mib "$MEM" --touch-mib "$TOUCH" --walk-mib "$WALK"
  --kernel "$OUT/kernel" --rootfs-src "$OUT/rootfs.ext4"
  --run-dir /run/spike-increment-k
  --snap-dir "$SNAPROOT/snap-$LABEL"
  --boot-ticks 80
)
[ "$CMD" = "api-probe" ] && ARGS+=(--api-probe)
[ "${WARM_CACHE:-0}" = "1" ] && ARGS+=(--warm-cache)

"$OUT/host-ondemand" "${ARGS[@]}"
