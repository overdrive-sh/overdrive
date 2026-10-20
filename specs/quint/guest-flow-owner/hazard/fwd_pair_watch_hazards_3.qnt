// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the GuestStream's end and the watch's record order
// (EVMOD-STREAM-END: the resolution and the watch's record before the wakeup, F11 corrected), the CLEAN-FULL-END send-shut flag and EV-DGRAM D1. Each is the design
// with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// EV-DGRAM D1 off: a frame left incomplete at the peer's end is waited for
module pair_bugTruncatedFrameWaits { import fwd_pair(OFF = Set("truncatedFrameWaits"), CFG = "dgram", REC = false).* from "../fwd_pair" }
// the same, on the datagram liveness instance
module pair_dliveTruncatedFrameWaits { import fwd_pair(OFF = Set("truncatedFrameWaits"), CFG = "dlive", REC = false).* from "../fwd_pair" }
