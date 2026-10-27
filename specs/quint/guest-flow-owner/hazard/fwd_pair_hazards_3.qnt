// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): one per design decision (EVMOD-ADOPT forwarding, EVMOD-END,
// EVMOD-PAIR / EVMOD-ATTACH-END, EVMOD-CTL, EVMOD-REC, EVMOD-STREAM-END / -ABORT, WATCH-ABORT, EVMOD-DGRAM, ADR-0183).
// Each is the design with ONE rule switched off (OFF) on the instance (CFG) that carries it. This wrapper imports one
// fwd_pair instance so that the Apalache server can compile it within its fixed heap. Expected verdicts are in
// ../checks.toml.

// EVMOD-CTL off: the context's release leaves its pairs forwarding
module pair_bugReleaseKeepsPairs { import fwd_pair(OFF = Set("releaseKeepsPairs"), CFG = "ok", REC = false).* from "../fwd_pair" }
