// -*- mode: Bluespec; -*-
// Teeth for ../relay_exchange.qnt: the approved rule set with ONE decision's rule switched off.
// Expected verdicts are in ../checks.toml.

// ADR-0239: the relay moves octets although the current message is complete (remaining-length guard dropped)
module relay_bugRelayPastBoundary { import relay_exchange(OFF = Set("relayPastBoundary"), REC = false).* from "../relay_exchange" }
