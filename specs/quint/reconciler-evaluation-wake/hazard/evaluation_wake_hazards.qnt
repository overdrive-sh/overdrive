// -*- mode: Bluespec; -*-
// Teeth for ../evaluation_wake.qnt: each instance is the approved rule with ONE decision switched off (OFF).
// Expected verdicts are in ../checks.toml.

// D-1: the resync pass is removed. Dropped wakes are disabled so the wedge is the one ADR-0282 names: a hydrate
// error, a persist error, a dispatch error or a crash ends an evaluation and nothing else re-submits the key.
module wake_noPass {
  import evaluation_wake(OFF = Set("noPass"), FAULTS = Set("hydrateErr", "persistErr", "dispatchErr"), DROPS = false, REC = false).* from "../evaluation_wake"
}
// D-3: next_evaluation_at returns None, so an in-window Noop is not requeued (the pre-D-3 hydrator).
module wake_noRetention {
  import evaluation_wake(OFF = Set("noRetention"), FAULTS = Set("dpFail"), DROPS = true, REC = false).* from "../evaluation_wake"
}
