// -*- mode: Bluespec; -*-
// Teeth for ../gateway_application.qnt: the approved rule set with ONE decision's rule switched off.
// Expected verdicts are in ../checks.toml.

// ADR-0222: a Draining generation is retired while connections still hold leases on it
module app_bugRetireWhileLeased { import gateway_application(OFF = Set("retireWhileLeased"), REC = false).* from "../gateway_application" }

// R-65: `stage` ignores the recorded failed Route generation and re-stages a candidate that failed the gate
module app_bugRestageFailed { import gateway_application(OFF = Set("restageFailed"), REC = false).* from "../gateway_application" }
