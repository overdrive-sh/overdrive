// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the TCP error-report end and the ends present at attach
// (EV-END change 1, EV-PAIR corrections 1-2, EVMOD-ATTACH-END; items T-1 to T-4 of the third iteration). Each is the
// design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are spread over files of at most
// two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// T-1 off (TCP-ERR-REPORT): a TCP reset is seen only by a later recvmsg of the half or a sendmsg to it (hazard 16)
module pair_bugTcpNoErrorReport { import fwd_pair(OFF = Set("tcpNoErrorReport"), CFG = "ok", REC = false).* from "../fwd_pair" }
// the same, on the liveness instance: a reset half the forwarder no longer reads strands the pair
module pair_liveTcpNoErrorReport { import fwd_pair(OFF = Set("tcpNoErrorReport"), CFG = "live", REC = false).* from "../fwd_pair" }
