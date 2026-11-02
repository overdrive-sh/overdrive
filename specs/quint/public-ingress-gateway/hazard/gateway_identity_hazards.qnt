// -*- mode: Bluespec; -*-
// Teeth for ../gateway_identity.qnt: the approved rule set with ONE decision's rule switched off.
// Expected verdicts are in ../checks.toml.

// ADR-0225: hold_audited / drop_after_drain ignore the checked epoch: a stale Issue holds after disable
module identity_bugNoEpochCheck { import gateway_identity(OFF = Set("noEpochCheck"), REC = false).* from "../gateway_identity" }
