// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): forwarding and end classification (EVMOD forwarding, EVMOD-END, ADR-0183, the rejected sockmap redirect). Each is the design with ONE rule switched off
// (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// rejected alternative (the BPF sockmap redirect): the vsock credit is returned at read and the target's egress
// queue is unbounded
module pair_altSockmapRedirect { import fwd_pair(OFF = Set("altSockmapRedirect"), CFG = "ok", REC = false).* from "../fwd_pair" }
// R2-END off: every vsock end is forwarded as a FIN
module pair_bugEofAsFin { import fwd_pair(OFF = Set("eofAsFin"), CFG = "ok", REC = false).* from "../fwd_pair" }
