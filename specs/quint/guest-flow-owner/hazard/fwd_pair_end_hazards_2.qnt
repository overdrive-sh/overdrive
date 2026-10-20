// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): ends present at attach, the TCP reset after a FIN, and the early shutdown on the liveness instance (EVMOD-PAIR, EVMOD-END). Each is the design with ONE rule switched off
// (OFF), or the rejected alternative switched on, on the instance (CFG) that carries it. fwd_pair's hazards are
// spread over files of at most two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// EVMOD-END (TCP halves) off: a TCP half that read end-of-stream is taken as ended cleanly; a later sk_err is ignored
module pair_bugTcpEofIsClean { import fwd_pair(OFF = Set("tcpEofIsClean"), CFG = "ok", REC = false).* from "../fwd_pair" }
// ADR-0183 off, on the liveness instance: the units queued behind the early FIN are stranded
module pair_liveShutdownOnCallback { import fwd_pair(OFF = Set("shutdownOnCallback"), CFG = "live", REC = false).* from "../fwd_pair" }
