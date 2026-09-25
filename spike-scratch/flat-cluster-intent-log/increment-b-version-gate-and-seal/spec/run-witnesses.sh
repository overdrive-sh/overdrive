#!/usr/bin/env bash
# Non-vacuity: each witness is EXPECTED to be violated in the correct model (ok4) — the violation is
# Apalache's proof that the behaviour is reachable, so the safety passes are not vacuous.
set -u
D="$(dirname "${BASH_SOURCE[0]}")"
P=${1:-18922}
S=10
for W in WitnessGateNeverCommits WitnessNoLearnerHaltAtGate WitnessNoApplyAfterGate \
         WitnessNoSealAfterGate WitnessNoWireBumpedEpoch WitnessNoVoterChange \
         WitnessNoPartialSealQuorum WitnessNoUpgradedLearnerResumed; do
  "$D/verify-one.sh" "W-ok4-$W" ok4 invariant "$W" $S $P
done
