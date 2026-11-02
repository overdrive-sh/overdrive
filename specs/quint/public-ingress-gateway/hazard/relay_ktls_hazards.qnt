// -*- mode: Bluespec; -*-
// Teeth for ../relay_ktls.qnt: the approved rule set with ONE decision's rule switched off.
// Expected verdicts are in ../checks.toml.

// ADR-0244: the kTLS rekey sequence does not require the tx lock free (it races a writer)
module ktls_bugRekeyDuringWrite { import relay_ktls(OFF = Set("rekeyDuringWrite")).* from "../relay_ktls" }
