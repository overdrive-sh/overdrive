// -*- mode: Bluespec; -*-
// Hazard / teeth / environment-fault instances of module D (../steering.qnt). Each instance
// switches ONE design rule off (BUG_*) or enables an environment fault (ENV_*). Expected verdicts
// are in ../checks.toml (expect = "violation" for teeth).

// FIX-1 off: a failed revoke closes the listener (the r1 design text; r1 finding 1)
module steer_bugRevokeFailCloses {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = true, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// FIX-1 fully off: revoke failure closes the listener AND release ignores elements
module steer_bugRevokeFailClosesNoElemGuard {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = true, BUG_RELEASE_IGNORES_ELEMENTS = true, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// the design (FIX-3 on) with a foreign flush of the shared table (K-D3 dropped)
module steer_envFlush {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = true, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// FIX-3 off under a foreign flush: converge_shared adds/keeps the route without the rules
module steer_envFlushBugConverge {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = true, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = true, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// D8a boot order off (route step before rules step), FIX-3 on, no crash: startup refuses forever
module steer_bugRouteFirst {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = false, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = true, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// D8a boot order off and FIX-3 off (revision-4 behaviour)
module steer_bugRouteFirstNoCheck {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = true, BUG_ROUTE_BEFORE_RULES = true, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// bring-up order off: element added before the listener listens
module steer_bugAdmitFirst {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = true, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// take-down order off: listener closed before its element is revoked
module steer_bugCloseFirst {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = true, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// quiescence leaves intake listeners bound (M-7 off)
module steer_bugQuiesceKeeps {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = true, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// activation returns Err before its rollback took down what it bound (two declared ports)
module steer_bugActErrEarly {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1, 2), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = true, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// activation returns Ok before / without every wanted listener (pre-fix U-6 reading)
module steer_bugActOkPartial {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = true, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// liveness teeth: no fairness on the owner's steps — the take-down never runs
module steer_liveTeeth {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true,
    ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false,
    BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false,
    BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false,
    BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false,
    BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
  temporal QuiesceCompletesNoFairness =
    weakFair(BootStep, (boot, up, rules, route)).implies(always(quiesced.implies(eventually(not(quiesced) or quiesceDone))))
}

// D8a-READMIT's re-admission off under a foreign flush: a resetting listener wanted again assumes its element
// (the r2 rule; r2 finding r2-2)
module steer_envFlushBugRewantKeeps {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = true, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = true, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
}

// D8a-READMIT's repair-in-quiescence off under a foreign flush: the rules are repaired while listeners keep serving
// (U-r2-4)
module steer_envFlushBugRepairOutsideQuiesce {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = true, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = true).* from "../steering"
}

// exposure-bound teeth: no fairness on the audit recovery — after a flush with serve up, nothing
// forces the repair, so the exposure can last forever
module steer_envFlushNoAuditFairness {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = true, BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false, BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false, BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false, BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false, BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false, BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "../steering"
  temporal ExposureEndsNoAuditFairness =
    and {
      weakFair(OwnerStep, ownerVars), weakFair(BootStep, (boot, up, rules, route)),
      weakFair(Restart, (up, boot)), eventually(always(up)),
    }.implies(always(exposed.implies(eventually(not(exposed)))))
}
