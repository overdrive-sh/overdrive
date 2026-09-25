#!/usr/bin/env bash
# Exhaustive explicit-state checks with TLC (via `quint verify --backend tlc`). The model is finite
# (bounded ops, epochs, versions), so TLC explores the COMPLETE reachable state graph — no step bound.
# Evidence: evidence/tlc-<label>.txt (raw) + evidence/tlc-summary.txt.
set -u
source "$(dirname "${BASH_SOURCE[0]}")/env.sh"
cd "$SPEC_DIR"
sum="$EVIDENCE_DIR/tlc-summary.txt"
echo "# TLC (quint verify --backend tlc) — $(date -u +%FT%TZ) — uname -r $(uname -r) — quint $(quint --version) — spec sha256 $(sha256sum gate_seal.qnt | cut -d' ' -f1)" >>"$sum"
# Optional (added on resume): TLC_ENDPOINT = private Apalache port for the Quint->TLA+ compile step
# (default: Quint's 8822, which a sibling increment may also use); TLC_TMPDIR = where Quint puts the
# TLA file and TLC's -metadir (default os.tmpdir() = the VM's 7.8 GB /tmp tmpfs, too small for ok5);
# TLC_CONFIG = Quint --tlc-config JSON (e.g. {"workers": 6}) to leave cores for concurrent agents.
tlc() { # label main kind(invariant|temporal) prop
  local label=$1 main=$2 kind=$3 prop=$4
  local out="$EVIDENCE_DIR/tlc-$label.txt" s; s=$(date +%s)
  local ep=() tmp="${TLC_TMPDIR:-}"
  [ -n "${TLC_ENDPOINT:-}" ] && ep=(--server-endpoint "$TLC_ENDPOINT")
  [ -n "${TLC_CONFIG:-}" ] && ep+=(--tlc-config "$TLC_CONFIG")
  [ -n "$tmp" ] && mkdir -p "$tmp"
  echo "\$ ${tmp:+TMPDIR=$tmp }quint verify --backend tlc ${ep[*]} --main $main --$kind $prop gate_seal.qnt" >"$out"
  echo "# started $(date -u +%FT%TZ) — uname -r $(uname -r) — spec sha256 $(sha256sum gate_seal.qnt | cut -d' ' -f1)" >>"$out"
  TMPDIR="${tmp:-${TMPDIR:-/tmp}}" quint verify --backend tlc "${ep[@]}" --main "$main" "--$kind" "$prop" gate_seal.qnt 2>&1 | grep -v makeExtensionsImmutable >>"$out"
  local rc=${PIPESTATUS[0]}
  echo "# exit $rc, wall-clock $(( $(date +%s) - s )) s" >>"$out"
  printf '%-52s rc=%s %s | %s\n' "$label" "$rc" \
    "$(grep -E 'distinct states found, 0 states left|depth of the complete' "$out" | tr '\n' ' ')" \
    "$(grep -E '^\[(ok|violation)\]|Error: |is violated|Temporal properties were violated' "$out" | head -2 | tr '\n' ' ')" | tee -a "$sum"
}
case "${1:-all}" in
  safety)
    tlc ok3-AllSafety ok3 invariant AllSafety
    tlc ok4-AllSafety ok4 invariant AllSafety
    tlc ok5-AllSafety ok5 invariant AllSafety
    tlc B4prime-unverifiedFetchFenced-AllSafety bugUnverifiedFetchFenced invariant AllSafety
    tlc B6prime-resealFenced-AllSafety bugResealFenced invariant AllSafety ;;
  broken)
    tlc B1-majority-GateOnlyWithAllVoters bugMajority invariant GateOnlyWithAllVoters
    tlc B2-proposetime-GateOnlyWithAllVoters bugProposeTime invariant GateOnlyWithAllVoters
    tlc B3-nofence-NoAckAfterSeal bugNoFence invariant NoAckAfterSeal
    tlc B3-nofence-CommittedPrefixPreserved bugNoFence invariant CommittedPrefixPreserved
    tlc B3-nofence-NoTwoCommittingPrimaries bugNoFence invariant NoTwoCommittingPrimaries
    tlc B4-unverifiedfetch-LearnerNoDivergence bugUnverifiedFetch invariant LearnerNoDivergence
    tlc B5-admitunsupported-VotersSupportClusterVersion bugAdmitUnsupported invariant VotersSupportClusterVersion
    tlc B6-resealnofence-UniqueSuccessor bugResealNoFence invariant UniqueSuccessor
    tlc B7-applyunsupported-NoApplyAboveSupported bugApplyUnsupported invariant NoApplyAboveSupported ;;
  ok5)
    tlc ok5-AllSafety-attempt2-vartmp ok5 invariant AllSafety ;;
  liveness3)
    tlc ok3-GateEventuallyCommits ok3 temporal GateEventuallyCommits ;;
  liveness)
    tlc ok3-GateEventuallyCommits ok3 temporal GateEventuallyCommits
    tlc ok4-GateEventuallyCommits ok4 temporal GateEventuallyCommits
    tlc ok5-GateEventuallyCommits ok5 temporal GateEventuallyCommits ;;
esac
