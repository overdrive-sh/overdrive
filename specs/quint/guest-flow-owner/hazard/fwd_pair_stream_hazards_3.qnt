// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the owner's abort of a watched stream and the holder's drop
// (WATCH-ABORT, EVMOD-STREAM-ABORT, EV-CTL 4.4; items T-5 and T-6 of the third iteration). Each is the design with ONE
// rule switched off (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's
// hazards are spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile
// heap. Expected verdicts are in ../checks.toml.

// the same, on the watch liveness instance
module pair_wliveDropWithoutAbort { import fwd_pair(OFF = Set("dropWithoutAbort"), CFG = "wlive", REC = false).* from "../fwd_pair" }
// U-R1 off (EVMOD-CTL recovery step 1): the recovery leaves a held stream whose watch already ended as it was
module pair_bugRecoveryKeepsEndedStream { import fwd_pair(OFF = Set("recoveryKeepsEndedStream"), CFG = "watch", REC = false).* from "../fwd_pair" }
