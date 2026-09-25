// -*- mode: Bluespec; -*-
// Teeth check for GateEventuallyCommits (added on resume): the same ok3 instance of gate_seal.qnt
// (imported unchanged, so gate_seal.qnt's sha256 is untouched), with the property's fairness
// assumptions weakened. Each weakened form is EXPECTED to be violated; a violation proves the
// ok3 liveness PASS is not vacuous (TLC can find the lasso the fairness assumption rules out).
module liveness_teeth {
  import gate_seal(
    NODES = Set("n1", "n2", "n3"), INIT_VOTERS = Set("n1", "n2", "n3"), VOTER_COUNT = 3,
    MAXV = 2, MAXE = 2, MAXOPS = 2,
    BUG_GATE_MAJORITY = false, BUG_GATE_PROPOSE_TIME_ONLY = false, BUG_NO_SEAL_FENCE = false,
    BUG_UNVERIFIED_FETCH = false, BUG_ADMIT_UNSUPPORTED_VOTER = false, BUG_RESEAL = false,
    BUG_APPLY_UNSUPPORTED = false
  ).* from "./gate_seal"

  // (T1) no fairness on CommitGate: a proposal may sit pending forever.
  temporal NoCommitFairness =
    and {
      weakFair(ProposeGate, (pending, clusterVersion)),
      weakFair(DropStaleGate, pending),
    }.implies(always(allUpgraded.implies(eventually(clusterVersion == MAXV))))

  // (T2) no fairness at all.
  temporal NoFairness = always(allUpgraded.implies(eventually(clusterVersion == MAXV)))
}
