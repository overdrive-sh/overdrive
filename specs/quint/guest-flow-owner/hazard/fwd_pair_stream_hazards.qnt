// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the owner's abort of a watched stream and the holder's drop
// (WATCH-ABORT, EVMOD-STREAM-ABORT, EV-CTL 4.4; items T-5 and T-6 of the third iteration). Each is the design with ONE
// rule switched off (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's
// hazards are spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile
// heap. Expected verdicts are in ../checks.toml.

// the rejected EVMOD-WATCH-SHUT: the owner's abort of a watch shuts V_h
module pair_bugWatchAbortShutsVh { import fwd_pair(OFF = Set("watchAbortShutsVh"), CFG = "watch", REC = false).* from "../fwd_pair" }
// EVMOD-STREAM-ABORT off: the owner sends no Abort { flow } for an aborted stream (and the holder keeps the stream)
module pair_bugNoGuestAbort { import fwd_pair(OFF = Set("noGuestAbort"), CFG = "watch", REC = false).* from "../fwd_pair" }
