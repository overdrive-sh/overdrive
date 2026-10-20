// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the GuestStream's end and the watch's record order
// (EVMOD-STREAM-END: the resolution and the watch's record before the wakeup, F11 corrected), the CLEAN-FULL-END send-shut flag and EV-DGRAM D1. Each is the design
// with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// EVMOD-STREAM-END off: the GuestStream reports the holder's read result (0 → clean), not the watch's
module pair_bugHolderTrustsRead { import fwd_pair(OFF = Set("holderTrustsRead"), CFG = "watch", REC = false).* from "../fwd_pair" }
// EVMOD-STREAM-END (the watch's record before the original callback; EV-END change 3) off: the watch's sk_error_report wakes the holder before it queues the record
module pair_bugOriginalBeforeRecord { import fwd_pair(OFF = Set("originalBeforeRecord"), CFG = "watch", REC = false).* from "../fwd_pair" }
