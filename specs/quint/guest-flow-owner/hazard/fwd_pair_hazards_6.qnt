// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): one per design decision (EVMOD-ADOPT forwarding, EVMOD-END,
// EVMOD-PAIR / EVMOD-ATTACH-END, EVMOD-CTL, EVMOD-REC, EVMOD-STREAM-END / -ABORT, WATCH-ABORT, EVMOD-DGRAM, ADR-0183).
// Each is the design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed
// compile heap. Expected verdicts are in ../checks.toml.

// EVMOD-STREAM-END off: the GuestStream reports the holder's read result (0 → clean), not the watch's
module pair_bugHolderTrustsRead { import fwd_pair(OFF = Set("holderTrustsRead"), CFG = "watch", REC = false).* from "../fwd_pair" }
