// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): one per design decision (EVMOD-ADOPT forwarding, EVMOD-END,
// EVMOD-PAIR / EVMOD-ATTACH-END, EVMOD-CTL, EVMOD-REC, EVMOD-STREAM-END / -ABORT, WATCH-ABORT, EVMOD-DGRAM, ADR-0183).
// Each is the design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed
// compile heap. Expected verdicts are in ../checks.toml.

// WATCH-ABORT off (the rejected EVMOD-WATCH-SHUT): the owner's abort of a watch shuts V_h
module pair_bugWatchAbortShutsVh { import fwd_pair(OFF = Set("watchAbortShutsVh"), CFG = "watch", REC = false).* from "../fwd_pair" }
// EVMOD-STREAM-ABORT (T-6) off: the holder's drop only closes V_h; the forwarder's reference keeps the socket open (P-70 § 4.4)
module pair_bugDropWithoutAbort { import fwd_pair(OFF = Set("dropWithoutAbort"), CFG = "watch", REC = false).* from "../fwd_pair" }
