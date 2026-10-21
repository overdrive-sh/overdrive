// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): one per design decision (EVMOD-ADOPT forwarding, EVMOD-END,
// EVMOD-PAIR / EVMOD-ATTACH-END, EVMOD-CTL, EVMOD-REC, EVMOD-STREAM-END / -ABORT, WATCH-ABORT, EVMOD-DGRAM, ADR-0183).
// Each is the design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed
// compile heap. Expected verdicts are in ../checks.toml.

// EVMOD-DGRAM off: a datagram consumed when only part of its frame was accepted (needs three datagrams in one direction)
module pair_dhazConsumePartialFrame { import fwd_pair(OFF = Set("consumePartialFrame"), CFG = "dhaz", REC = false).* from "../fwd_pair" }
