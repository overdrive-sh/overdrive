#!/usr/bin/env bash
# The deliberately broken variants: each must produce an Apalache counterexample for the invariant it
# targets. Plus the two "defence in depth" variants (B4', B6') that must NOT violate AllSafety.
set -u
D="$(dirname "${BASH_SOURCE[0]}")"
P=${1:-18922}
S=8
"$D/verify-one.sh" B1-majority-GateOnlyWithAllVoters            bugMajority          invariant GateOnlyWithAllVoters       $S $P
"$D/verify-one.sh" B2-proposetime-GateOnlyWithAllVoters         bugProposeTime       invariant GateOnlyWithAllVoters       $S $P
"$D/verify-one.sh" B3-nofence-NoAckAfterSeal                    bugNoFence           invariant NoAckAfterSeal              $S $P
"$D/verify-one.sh" B3-nofence-CommittedPrefixPreserved          bugNoFence           invariant CommittedPrefixPreserved    $S $P
"$D/verify-one.sh" B3-nofence-NoTwoCommittingPrimaries          bugNoFence           invariant NoTwoCommittingPrimaries    $S $P
"$D/verify-one.sh" B4-unverifiedfetch-LearnerNoDivergence       bugUnverifiedFetch   invariant LearnerNoDivergence         $S $P
"$D/verify-one.sh" B5-admitunsupported-VotersSupportClusterVersion bugAdmitUnsupported invariant VotersSupportClusterVersion $S $P
"$D/verify-one.sh" B6-resealnofence-UniqueSuccessor             bugResealNoFence     invariant UniqueSuccessor             $S $P
"$D/verify-one.sh" B7-applyunsupported-NoApplyAboveSupported    bugApplyUnsupported  invariant NoApplyAboveSupported       $S $P
