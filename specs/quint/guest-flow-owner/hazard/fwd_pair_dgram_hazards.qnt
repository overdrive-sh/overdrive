// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): datagram mode (EVMOD-DGRAM). Each is the design with ONE rule switched off
// (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// a datagram is consumed when the vsock accepted part of its frame
module pair_bugConsumePartialFrame { import fwd_pair(OFF = Set("consumePartialFrame"), CFG = "dgram", REC = false).* from "../fwd_pair" }
// a frame whose UDP target send would block is dropped by the forwarder instead of waiting
module pair_bugDropWhenTargetBlocked { import fwd_pair(OFF = Set("dropWhenTargetBlocked"), CFG = "dgram", REC = false).* from "../fwd_pair" }
