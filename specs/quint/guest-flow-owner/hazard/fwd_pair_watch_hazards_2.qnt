// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the GuestStream's end and the watch's record order
// (EVMOD-STREAM-END: the resolution and the watch's record before the wakeup, F11 corrected), the CLEAN-FULL-END send-shut flag and EV-DGRAM D1. Each is the design
// with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// EVMOD-STREAM-END (3) off: a stream that finds no record and no end of its owner reports a clean end instead of waiting
module pair_bugNoRecordIsClean { import fwd_pair(OFF = Set("noRecordIsClean"), CFG = "watch", REC = false).* from "../fwd_pair" }
// CLEAN-FULL-END off: this side's send-shut flag is set in a step after the SHUTDOWN it sends
module pair_bugLateSendShut { import fwd_pair(OFF = Set("lateSendShut"), CFG = "ok", REC = false).* from "../fwd_pair" }
