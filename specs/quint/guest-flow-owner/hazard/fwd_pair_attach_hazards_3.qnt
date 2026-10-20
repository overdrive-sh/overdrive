// -*- mode: Bluespec; -*-
// Hazard / teeth instances of fwd_pair (../fwd_pair.qnt): the TCP error-report end and the ends present at attach
// (EV-END change 1, EV-PAIR corrections 1-2, EVMOD-ATTACH-END; items T-1 to T-4 of the third iteration). Each is the
// design with ONE rule switched off (OFF) on the instance (CFG) that carries it. fwd_pair's hazards are spread over files of at most
// two instances so that each file the Apalache server compiles for TLC fits its fixed compile heap.
// Expected verdicts are in ../checks.toml.

// T-4 off: on HalfEnded the owner closes the surviving TCP half without SO_LINGER{1,0} (a FIN, not a reset)
module pair_bugEndedAttachGracefulClose { import fwd_pair(OFF = Set("endedAttachGracefulClose"), CFG = "ok", REC = false).* from "../fwd_pair" }
// T-4 off: the owner treats HalfEnded as a retryable defect and keeps the flow
module pair_bugEndedAttachIgnored { import fwd_pair(OFF = Set("endedAttachIgnored"), CFG = "ok", REC = false).* from "../fwd_pair" }
