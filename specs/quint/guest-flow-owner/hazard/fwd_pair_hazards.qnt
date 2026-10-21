// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): one per design decision (EVMOD-ADOPT forwarding, EVMOD-END,
// EVMOD-PAIR / EVMOD-ATTACH-END, EVMOD-CTL, EVMOD-REC, EVMOD-STREAM-END / -ABORT, WATCH-ABORT, EVMOD-DGRAM, ADR-0183).
// Each is the design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed
// compile heap. Expected verdicts are in ../checks.toml.

// EVMOD-ADOPT (consume only what the target accepted) off: a short send consumes the whole peek
// (on the instance with more units than the pipeline holds: a short send needs a full target)
module pair_bugConsumeOnShortSend { import fwd_pair(OFF = Set("consumeOnShortSend"), CFG = "stall", REC = false).* from "../fwd_pair" }
// R2-END off: every vsock end is forwarded as a FIN
module pair_bugEofAsFin { import fwd_pair(OFF = Set("eofAsFin"), CFG = "ok", REC = false).* from "../fwd_pair" }
