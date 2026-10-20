// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): forwarding and end classification (EVMOD forwarding, EVMOD-END, ADR-0183, the rejected sockmap redirect). Each is the design with ONE rule switched off
// (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// consume only what the target accepted, off: the run consumes what it read and holds the rest in its own memory
// (the sockmap redirect's shape)
module pair_bugConsumeBeforeAccept { import fwd_pair(OFF = Set("consumeBeforeAccept"), CFG = "ok", REC = false).* from "../fwd_pair" }
// consume only what the target accepted, off: a short send consumes the whole peek
module pair_bugConsumeOnShortSend { import fwd_pair(OFF = Set("consumeOnShortSend"), CFG = "ok", REC = false).* from "../fwd_pair" }
