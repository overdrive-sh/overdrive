// -*- mode: Bluespec; -*-
// Teeth for the counted hold and the boot write (../quiescence.qnt): each instance is the design with ONE rule
// switched off (OFF). Expected verdicts are in ../checks.toml.

// QHOLD: a boolean latch — the first recovery's restore reopens while the second is still repairing
module quiesce_bugBoolLatch { import quiescence(OFF = Set("boolLatch"), CFG = "ok", REC = false).* from "../quiescence" }
// QHOLD (D-UP-3): a dropped hold releases the count (the guard's Drop decrements, besides counting the leak)
module quiesce_bugDropDecrements { import quiescence(OFF = Set("dropDecrements"), CFG = "ok", REC = false).* from "../quiescence" }
// ADR-0184: boot continues with serve running after a failed write of the shared table
module quiesce_bugBootContinues { import quiescence(OFF = Set("bootContinues"), CFG = "ok", REC = false).* from "../quiescence" }
// ADR-0124 (D-UP-3): the supervisor does not consult the leaked-hold count — a leaked hold keeps forwarding quiesced
// for good
module quiesce_live_bugLeakIgnored { import quiescence(OFF = Set("leakIgnored"), CFG = "live", REC = false).* from "../quiescence" }
// QHOLD: the first hold latches admission but does not abort the open flows
module quiesce_bugQuiesceSkipsFlows { import quiescence(OFF = Set("quiesceSkipsFlows"), CFG = "ok", REC = false).* from "../quiescence" }
// D-UP-4 (PD-1): a restore with a hold another ledger issued is redeemed as the owner's own (decrements the count;
// admission reopens at zero)
module quiesce_bugForeignDecrements { import quiescence(OFF = Set("foreignDecrements"), CFG = "ok", REC = false).* from "../quiescence" }
// D-UP-4 (PD-1): the supervisor does not fail-stop on ForeignHold — serve keeps running with a composition defect
module quiesce_live_bugForeignIgnored { import quiescence(OFF = Set("foreignIgnored"), CFG = "live", REC = false).* from "../quiescence" }
