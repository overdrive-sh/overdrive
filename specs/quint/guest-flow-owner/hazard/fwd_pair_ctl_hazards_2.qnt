// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the context, its end records, and its recovery (EVMOD-CTL, EVMOD-REC). Each is the design with ONE rule switched off
// (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// the same, on the liveness instance: the owner never reads the dropped record
module pair_liveLossyEndRecord { import fwd_pair(OFF = Set("lossyEndRecord"), CFG = "live", REC = false).* from "../fwd_pair" }
// EVMOD-CTL (recovery order) off: the new context is opened while the old one still holds its pairs
module pair_bugReopenBeforeRelease { import fwd_pair(OFF = Set("reopenBeforeRelease"), CFG = "ok", REC = false).* from "../fwd_pair" }
