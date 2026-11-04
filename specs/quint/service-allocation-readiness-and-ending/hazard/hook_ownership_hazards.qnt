// -*- mode: Bluespec; -*-
// Teeth for ../hook_ownership.qnt: each instance is the design with ONE decision's rule switched off (OFF).
// Expected verdicts are in ../checks.toml.

// D1: the observer fires the hook only when the row write succeeded (the "terminal-row author owns the hook" reading)
module hooks_bugHookGatedOnRowWrite { import hook_ownership(OFF = Set("hookGatedOnRowWrite"), REC = false).* from "../hook_ownership" }
// D3: the shim releases the exit-event gate first and registers supervision afterwards (the pre-D3 order)
module hooks_bugGateBeforeRegistration { import hook_ownership(OFF = Set("gateBeforeRegistration"), REC = false).* from "../hook_ownership" }
