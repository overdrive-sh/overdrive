// -*- mode: Bluespec; -*-
// Hazard / teeth instances of module C (../cid_lease.qnt). Expected verdicts are in ../checks.toml.

// D16-EXCL off (the r1 design): no per-allocation clash exclusion — r1 finding 2, concurrent case;
// also the teeth for D16-PREF's preference (an excluded offset returned while an unexcluded one is free)
module lease_bugNoExclusion {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2, 3), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = true, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "../cid_lease"
}
// D16-EXCL off, one allocation, serve restarts — r1 finding 2, restart case
module lease_single_boot_bugNoExclusion {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1), FOREIGN = Set(0),
    ALLOW_BOOT = true, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = true, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "../cid_lease"
}
// D16-EXCL off and next-fit off (first-fit): a single allocation re-hits the foreign CID at once
module lease_single_bugFirstFitNoExclusion {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = true, BUG_NO_EXCLUSION = true, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "../cid_lease"
}
// next-fit off but D16-EXCL on: hypothesis — the exclusion set alone carries the retry guarantee
module lease_firstFitWithExclusion {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2, 3), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = true, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "../cid_lease"
}
// D16-PREF off: the r2 rule — refuse when every free offset is excluded (r2 finding r2-1)
module lease_transient_bugRefuse {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 2, BUG_REFUSE_WHEN_ALL_EXCLUDED = true).* from "../cid_lease"
}
