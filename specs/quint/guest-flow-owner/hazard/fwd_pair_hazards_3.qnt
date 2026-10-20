// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): forwarding and end classification (EVMOD forwarding, EVMOD-END, ADR-0183, the rejected sockmap redirect). Each is the design with ONE rule switched off
// (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// EVMOD-END off: the class is computed at end-of-stream from the accumulated peer_shutdown
module pair_bugClassifyAccumulated { import fwd_pair(OFF = Set("classifyAccumulated"), CFG = "ok", REC = false).* from "../fwd_pair" }
// ADR-0183 off: the target is shut in the callback, before the vsock queue is empty
module pair_bugShutdownOnCallback { import fwd_pair(OFF = Set("shutdownOnCallback"), CFG = "ok", REC = false).* from "../fwd_pair" }
