#!/usr/bin/env bash
# PROBE increment-j — characterise the STALE-WRITE WINDOW across repetitions.
#
# WHY. The first `keep` run showed something the first `drop` run did not: on the
# FIRST post-restore tick the guest's send() on the pre-snapshot connection
# returned SUCCESS (49 bytes) while the host received nothing, and only from the
# next tick did it report EPIPE. That is a *silently stale half-open* write — the
# exact shape cloud-hypervisor#7958 claims to have removed in v52.0 — and one
# observation cannot tell a deterministic window from a race.
#
# So: run each arm N times and count, per run, how many post-restore ticks the
# guest wrote successfully into a connection the host had already lost. A driver
# whose Running gate writes-and-assumes-delivery is exposed for exactly that
# many ticks.
#
# Usage: repeat.sh [N] [mode ...]
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN=/run/spike-increment-j
N="${1:-5}"; shift || true
MODES=("${@:-drop keep}")
[ $# -eq 0 ] && MODES=(drop keep)

printf '%-10s %-4s %-6s %-6s %-14s %-13s %-11s %s\n' \
  MODE ITER lastB firstA first_held_w stale_writes host_after verdict
for mode in "${MODES[@]}"; do
  for i in $(seq 1 "$N"); do
    "$HERE/run.sh" "$mode" >"$RUN/../repeat-$mode-$i.txt" 2>&1
    BEFORE="$RUN/console-before-snapshot.log"; AFTER="$RUN/console.log"
    lastB="$(grep -oE '^TICK n=[0-9]+' "$BEFORE" 2>/dev/null | tail -1 | grep -oE '[0-9]+')"
    firstA="$(grep -oE '^TICK n=[0-9]+' "$AFTER" 2>/dev/null | head -1 | grep -oE '[0-9]+')"
    firstw="$(grep -E '^TICK' "$AFTER" 2>/dev/null | head -1 | grep -oE 'held_w=[^ ]+')"
    # Post-restore ticks whose held_w reports a SUCCESSFUL byte count.
    stale="$(grep -E '^TICK' "$AFTER" 2>/dev/null | grep -c 'held_w=w=' | head -1)"
    # HELD payloads the host actually received carrying a post-snapshot tick.
    host=0
    for f in "$RUN/held.log" "$RUN/held-after.log"; do
      [ -f "$f" ] || continue
      c="$(awk -v t="${lastB:-0}" 'match($0,/HELD n=[0-9]+/){s=substr($0,RSTART+7,RLENGTH-7); if (s+0>t+0) c++} END{print c+0}' "$f")"
      host=$((host + c))
    done
    memok="$(grep -c 'RESTORED FROM MEMORY' "$RUN/../repeat-$mode-$i.txt" | head -1)"
    v="mem_ok=$memok"
    [ "${stale:-0}" -gt 0 ] && [ "$host" = 0 ] && v="$v STALE-WRITE"
    # A row is only evidence if THIS probe's VMM survived. On a shared box a
    # foreign `pkill -x cloud-hyperviso` kills it mid-flight; that is a harness
    # event, not a measurement, and must not be averaged in silently.
    if grep -qE 'VMM died during boot|HARNESS DEFECT' "$RUN/../repeat-$mode-$i.txt"; then
      v="CONTAMINATED (foreign kill) — DISCARD"
    fi
    printf '%-10s %-4s %-6s %-6s %-14s %-13s %-11s %s\n' \
      "$mode" "$i" "${lastB:-?}" "${firstA:-?}" "${firstw:-?}" "${stale:-?}" "$host" "$v"
  done
done
