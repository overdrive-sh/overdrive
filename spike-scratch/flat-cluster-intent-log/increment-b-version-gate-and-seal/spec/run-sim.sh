#!/usr/bin/env bash
# Independent cross-check with the Quint SIMULATOR (random traces, deeper than the Apalache bound):
# the correct models must show no violation; every broken variant must show one. Evidence:
# evidence/sim-summary.txt (verdict lines) + evidence/sim-<label>.txt (raw output incl. traces).
set -u
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
cd "$SPEC_DIR"
sum="$EVIDENCE_DIR/sim-summary.txt"
echo "# quint run cross-check — $(date -u +%FT%TZ) — uname -r $(uname -r) — quint $(quint --version) — spec sha256 $(sha256sum gate_seal.qnt | cut -d' ' -f1)" >"$sum"
sim() { # label main invariant steps samples
  local label=$1 main=$2 inv=$3 steps=$4 samples=$5
  local out="$EVIDENCE_DIR/sim-$label.txt" s; s=$(date +%s)
  echo "\$ quint run --main $main --invariant $inv --max-steps $steps --max-samples $samples --seed 20260925 gate_seal.qnt" >"$out"
  quint run --main "$main" --invariant "$inv" --max-steps "$steps" --max-samples "$samples" --seed 20260925 gate_seal.qnt >>"$out" 2>&1
  local rc=$?
  echo "# exit $rc, wall-clock $(( $(date +%s) - s )) s" >>"$out"
  printf '%-58s rc=%s  %s\n' "$label" "$rc" "$(grep -E '^\[(ok|violation)\]' "$out" | head -1)" | tee -a "$sum"
}
sim ok4-AllSafety                      ok4                 AllSafety                   40 40000
sim ok5-AllSafety                      ok5                 AllSafety                   40 40000
sim ok3-AllSafety                      ok3                 AllSafety                   40 40000
sim B4prime-unverifiedFetchFenced      bugUnverifiedFetchFenced AllSafety              40 40000
sim B6prime-resealFenced               bugResealFenced     AllSafety                   40 40000
sim B1-majority                        bugMajority         GateOnlyWithAllVoters       40 40000
sim B2-proposetime                     bugProposeTime      GateOnlyWithAllVoters       40 40000
sim B3-nofence-NoAckAfterSeal          bugNoFence          NoAckAfterSeal              40 40000
sim B3-nofence-CommittedPrefix         bugNoFence          CommittedPrefixPreserved    40 40000
sim B3-nofence-NoTwoCommitting         bugNoFence          NoTwoCommittingPrimaries    40 40000
sim B4-unverifiedfetch                 bugUnverifiedFetch  LearnerNoDivergence         40 40000
sim B5-admitunsupported                bugAdmitUnsupported VotersSupportClusterVersion 40 40000
sim B6-resealnofence                   bugResealNoFence    UniqueSuccessor             40 40000
sim B7-applyunsupported                bugApplyUnsupported NoApplyAboveSupported       40 40000
