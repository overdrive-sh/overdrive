#!/usr/bin/env bash
# A/B evidence for the Apalache behaviour-loss hazard: the SAME spec, differing only in the gate
# proposal's `nondet` range form, checked by both backends (Quint simulator and Apalache).
set -u
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
P=${1:-18922}
out="$EVIDENCE_DIR/apalache-range-hazard.txt"
cd "$SPEC_DIR"
run() { # label, cmd...
  local label=$1; shift
  echo "## $label"
  echo "\$ $*"
  local s; s=$(date +%s)
  "$@" 2>&1 | grep -E 'violation|No violation|\[ok\]|outcome|QNT|rror' | grep -v makeExtensionsImmutable
  echo "(wall-clock $(( $(date +%s) - s )) s)"
  echo
}
{
  echo "# Apalache range-hazard A/B — $(date -u +%FT%TZ) — uname -r $(uname -r) — quint $(quint --version) — apalache 0.56.1"
  echo "# A = spec/hazard/gate_seal_state_bound_range.qnt : nondet v = (clusterVersion + 1).to(MAXV).oneOf()"
  echo "# B = spec/gate_seal.qnt                           : nondet v = 1.to(MAXV).oneOf() + guard v > clusterVersion"
  echo "# Property: bugApplyUnsupported / NoApplyAboveSupported. Expected (faithful semantics): VIOLATED."
  echo
  for f in hazard/gate_seal_state_bound_range.qnt gate_seal.qnt; do
    run "simulator  $f  init=debugPreProposeInit" quint run --main bugApplyUnsupported --init debugPreProposeInit --step step --invariant NoApplyAboveSupported --max-steps 4 --max-samples 2000 --seed 1 --verbosity 1 "$f"
    run "apalache   $f  init=debugPreProposeInit" quint verify --main bugApplyUnsupported --init debugPreProposeInit --step step --invariant NoApplyAboveSupported --max-steps 4 --server-endpoint "localhost:$P" "$f"
    run "apalache   $f  init=init (max-steps 7)" quint verify --main bugApplyUnsupported --invariant NoApplyAboveSupported --max-steps 7 --server-endpoint "localhost:$P" "$f"
  done
  echo "# Context-free minimal reproductions (NOT triggering the hazard — kept as falsified hypotheses):"
  for m in reproBugTrue reproValBugTrue reproNondetBugTrue; do
    run "apalache   repro_if_const.qnt  $m" quint verify --main "$m" --invariant Inv --max-steps 3 --server-endpoint "localhost:$P" repro_if_const.qnt
  done
  for m in rangeState rangeConst; do
    run "apalache   repro_range.qnt  $m" quint verify --main "$m" --invariant Inv --max-steps 3 --server-endpoint "localhost:$P" repro_range.qnt
  done
} >"$out" 2>&1
cat "$out"
