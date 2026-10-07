// -*- mode: Bluespec; -*-
// Hazard / teeth instances of module C (../cid_lease.qnt). Each switches ONE rule of D16-CLAIM off.
// Expected verdicts are in ../checks.toml.

// Check-then-claim (ADR-0170 rejected alternative): assign only checks the CID is free and the VMM
// claims it at device creation; a foreign user can take it in between.
module lease_bugCheckThenClaim {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN0 = Set(0),
    FOREIGN_UNBOUNDED = true, FOREIGN_CHANGES = 0, ALLOW_BOOT = true, PRE_READY_EXITS = 1,
    CLAIM_ERRORS = 0, BUG_CHECK_THEN_CLAIM = true, BUG_RELEASE_BEFORE_VMM = false,
    BUG_REMEMBER_INUSE = false).* from "../cid_lease"
}

// The claim is released before the VMM holds it (serve closes its reference at take_claim).
module lease_bugReleaseBeforeVmm {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN0 = Set(0),
    FOREIGN_UNBOUNDED = true, FOREIGN_CHANGES = 0, ALLOW_BOOT = true, PRE_READY_EXITS = 1,
    CLAIM_ERRORS = 0, BUG_CHECK_THEN_CLAIM = false, BUG_RELEASE_BEFORE_VMM = true,
    BUG_REMEMBER_INUSE = false).* from "../cid_lease"
}

// InUse offsets are remembered per workload and skipped later without asking the kernel.
module lease_bugRemember {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN0 = Set(0),
    FOREIGN_UNBOUNDED = true, FOREIGN_CHANGES = 0, ALLOW_BOOT = true, PRE_READY_EXITS = 1,
    CLAIM_ERRORS = 0, BUG_CHECK_THEN_CLAIM = false, BUG_RELEASE_BEFORE_VMM = false,
    BUG_REMEMBER_INUSE = true).* from "../cid_lease"
}
// ... the liveness consequence: a workload refuses forever while a free offset's CID stays unheld
module lease_live_bugRemember {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN0 = Set(0),
    FOREIGN_UNBOUNDED = false, FOREIGN_CHANGES = 2, ALLOW_BOOT = false, PRE_READY_EXITS = 2,
    CLAIM_ERRORS = 1, BUG_CHECK_THEN_CLAIM = false, BUG_RELEASE_BEFORE_VMM = false,
    BUG_REMEMBER_INUSE = true).* from "../cid_lease"
}
