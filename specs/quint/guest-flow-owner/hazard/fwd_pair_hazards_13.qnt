// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): one per design decision (EVMOD-ADOPT forwarding, EVMOD-END,
// EVMOD-PAIR / EVMOD-ATTACH-END, EVMOD-CTL, EVMOD-REC, EVMOD-STREAM-END / -ABORT, WATCH-ABORT, EVMOD-DGRAM, ADR-0183).
// Each is the design with ONE rule switched off (OFF) on the instance (CFG) that carries it. This wrapper imports one
// fwd_pair instance so that the Apalache server can compile it within its fixed heap. Expected verdicts are in
// ../checks.toml.

// EVMOD-STREAM-ABORT (T-6) off: the holder's drop only closes V_h; the forwarder's reference keeps the socket open (P-70 § 4.4)
module pair_bugDropWithoutAbort { import fwd_pair(OFF = Set("dropWithoutAbort"), CFG = "watch", REC = false).* from "../fwd_pair" }
