// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the TCP error-report end and the ends present at attach
// (EV-END change 1, EV-PAIR corrections 1-2, EVMOD-ATTACH-END; items T-1 to T-4 of the third iteration). Each is the
// design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are spread over files of at most
// two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// T-2 off (ATTACH-SEED): the callbacks are installed and the attach-time state is not classified (the module's h_noseed)
module pair_bugNoSeed { import fwd_pair(OFF = Set("noSeed"), CFG = "ok", REC = false).* from "../fwd_pair" }
// T-3 off (ATTACH-CLOSE-WAIT): attach refuses a TCP half in CLOSE_WAIT (the module's h_strict); the owner aborts the flow
module pair_bugEstablishedOnly { import fwd_pair(OFF = Set("establishedOnly"), CFG = "ok", REC = false).* from "../fwd_pair" }
