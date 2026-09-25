#!/usr/bin/env bash
# Long correct-model checks (expected: NoError), run detached. Each lane uses its own Apalache port.
# Usage: run-correct.sh <lane>   — writes evidence/verify-*.txt and evidence/lane-<lane>.done
set -u
D="$(dirname "${BASH_SOURCE[0]}")"
source "$D/env.sh"
lane=$1
case "$lane" in
  a) "$D/verify-one.sh" C-ok4-AllSafety-10       ok4 invariant AllSafety 10 18923 ;;
  b) "$D/verify-one.sh" C-ok5-AllSafety-8        ok5 invariant AllSafety 8 18924 ;;
  c) "$D/verify-one.sh" C-ok3-AllSafety-10       ok3 invariant AllSafety 10 18925 ;;
  d) "$D/verify-one.sh" C-B4prime-unverifiedFetchFenced-AllSafety-8 bugUnverifiedFetchFenced invariant AllSafety 8 18926
     "$D/verify-one.sh" C-B6prime-resealFenced-AllSafety-8          bugResealFenced          invariant AllSafety 8 18926 ;;
  e) "$D/verify-one.sh" L-ok4-GateEventuallyCommits-10 ok4 temporal GateEventuallyCommits 10 18927 ;;
esac
echo "$?" >"$EVIDENCE_DIR/lane-$lane.done"
