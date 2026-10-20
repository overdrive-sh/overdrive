// -*- mode: Bluespec; -*-
// Hazard / teeth instance of fwd_pair (../fwd_pair.qnt) for the named run of U-R1 (EVMOD-CTL recovery step 1): the
// recovery leaves a held stream whose watch already ended as it was. On the run configuration (CFG "conf"), in a file
// of its own: the conf instance does not fit the Apalache server's fixed compile heap beside another instance.
// Expected verdicts are in ../checks.toml.

// U-R1 off, on the run configuration (the named run RecoveryEndsHeldStreamWhoseWatchEnded fails)
module pair_confRecoveryKeepsEndedStream { import fwd_pair(OFF = Set("recoveryKeepsEndedStream"), CFG = "conf", REC = false).* from "../fwd_pair" }
