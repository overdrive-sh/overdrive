#!/usr/bin/env bash
# Does the virtio-blk VOLUME path boot under NESTED virt (Lima on Apple
# Silicon), where the virtiofs + `--memory shared=on` path does not?
#
# This exists because the claim "a block volume path runs in Lima" was an
# INFERENCE — extrapolated from increment-a booting a block *rootfs* — and was
# challenged. increment-f had only ever run on bare metal. Inference is not
# measurement, so here is the measurement.
#
# Hypothesis:        the nested stall is a property of `shared=on` guest memory,
#                    so the block-volume path stalls at the SAME rate as the
#                    plain rootfs boot, while the virtiofs path stalls at 100%.
# Predicted outcome: arm A ~= arm F (both partial, per increment-a's ~2/3), and
#                    arm E = 0/N.
# Falsification:     arm F stalls markedly worse than arm A -> adding block
#                    volumes itself costs boot reliability under nesting, and
#                    "block runs in Lima" is wrong for a different reason than
#                    shared=on. OR arm F is clean 100% while arm A is not ->
#                    the earlier increment-a stall rate is stale.
#
# Three arms, interleaved so any drift in the host VM hits all three equally:
#   A  increment-a  rootfs only, no volumes, no shared=on   (the baseline)
#   F  increment-f  rootfs + 2 block volumes, no shared=on
#   E  increment-e  rootfs + 2 virtiofs shares + shared=on
#
# Run inside the Lima VM as root.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
A="$(cd "$HERE/../increment-a" && pwd)"
E="$(cd "$HERE/../increment-e" && pwd)"
N="${N:-6}"
OUT=/var/tmp/spike-increment-f
mkdir -p "$OUT"
LOG="$OUT/nested-check.txt"
: >"$LOG"

echo "=================================================================="
echo "NESTED-VIRT CHECK — does the block VOLUME path boot under nesting?"
echo "host kernel : $(uname -r)  $(uname -m)"
echo "virt        : $(systemd-detect-virt)"
echo "ch          : $(cloud-hypervisor --version 2>&1)"
echo "trials      : $N per arm, interleaved"
echo "=================================================================="

declare -A PASS FAIL
for arm in A F E; do PASS[$arm]=0; FAIL[$arm]=0; done

run_arm() {   # <arm> <trial>
  local arm="$1" i="$2" rc out
  case "$arm" in
    A) out=$("$A/run.sh" host 2>&1); rc=$? ;;
    F) out=$(VOLROOT=/var/tmp/inc-f-vols "$HERE/run.sh" blk 2>&1); rc=$? ;;
    E) out=$(VOLROOT=/var/tmp/inc-e-vols "$E/run.sh" full 2>&1); rc=$? ;;
  esac
  if [ "$rc" = 0 ]; then
    PASS[$arm]=$(( ${PASS[$arm]} + 1 ))
    printf '  arm %s trial %s: PASS\n' "$arm" "$i"
  else
    FAIL[$arm]=$(( ${FAIL[$arm]} + 1 ))
    # Where it died matters: a stall never reaches /init, a config rejection
    # never starts, and a completed-but-failed run is a different animal.
    local last
    last=$(grep -E '^init: |^\[HOST|Error booting' <<<"$out" | tail -1)
    printf '  arm %s trial %s: FAIL(rc=%s)  last: %s\n' "$arm" "$i" "$rc" "${last:-<no output at all — stalled before /init>}"
  fi
  printf '=== arm=%s trial=%s rc=%s\n' "$arm" "$i" "$rc" >>"$LOG"
  grep -E '^init: |^\[HOST|Error booting' <<<"$out" | tail -3 >>"$LOG"
}

for i in $(seq 1 "$N"); do
  for arm in A F E; do run_arm "$arm" "$i"; done
done

echo
echo "=================================================================="
echo "RESULT"
echo "=================================================================="
printf '  %-4s %-46s %s\n' "arm" "what" "boots"
printf '  %-4s %-46s %s/%s\n' "A" "rootfs only (no volumes, no shared=on)"      "${PASS[A]}" "$N"
printf '  %-4s %-46s %s/%s\n' "F" "+ 2 virtio-blk volumes (no shared=on)"       "${PASS[F]}" "$N"
printf '  %-4s %-46s %s/%s\n' "E" "+ 2 virtiofs shares (shared=on)"             "${PASS[E]}" "$N"
echo
echo "full log: $LOG"
