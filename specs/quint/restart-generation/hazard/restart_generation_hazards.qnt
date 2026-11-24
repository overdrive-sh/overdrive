// One hazard per decision (ADR-0335, ADR-0337, ADR-0338).

// SD-6 (ADR-0335): the restart generation is recorded handled when the launch is EMITTED, not when its allocation is
// observed: `og = dg` at WlStart and `pendingGen` reads `og`. Counterexample: restart of a Running workload, the replacement
// launch is issued (`og = dg`), the pool fills and the launch is refused at admission (no row: ADR-0338), no generation is
// pending and the operator-stop veto is set: the workload is stranded forever. (A launch lost or superseded no longer
// qualifies: ADR-0355 ends its `Pending` row with an abandoned row, which is restartable, so the rule's defence is the refused
// launch, the one launch that still ends with no row.) The smallest bounds that still violate (three ids, one restart commit,
// no fault).
module rg_bugStampOnEmit {
  import restart_generation(OFF = Set("stampOnEmit"), REC = false, OPS = Set("restart", "capFull"), NA = 3, FB = 0, LIMIT = 3,
    DGMAX = 1, JOB = false).* from "../restart_generation"
}

// ADR-0337: a pre-driver failure writes a charged `Failed` row. Hazard: it writes no row, so nothing is charged, the current
// row stays as it was and an id is issued per tick: the budget never exhausts. The constants of `rg_live_prefail`.
module rg_bugPrefailNoRow {
  import restart_generation(OFF = Set("prefailNoRow"), REC = false, OPS = Set("restart", "idFaultHeld"), NA = 4, FB = 0,
    LIMIT = 3, DGMAX = 1, JOB = false).* from "../restart_generation"
}

// ADR-0338: an admission refusal at the guest-network cap writes no row and charges nothing. Hazard: the refusal is a charged
// row, so a node held at the cap exhausts a healthy workload's budget. Three refusals suffice (LIMIT = 3).
module rg_bugRefusalCharged {
  import restart_generation(OFF = Set("refusalCharged"), REC = false, OPS = Set("capFull"), NA = 4, FB = 0, LIMIT = 3,
    DGMAX = 1, JOB = false).* from "../restart_generation"
}

// ADR-0355: a launch that ends before running ends its `Pending` row with an abandoned terminal row. Hazard: the row is left
// `Pending` (boot, a supersession and a loss write nothing): the dead row blocks every placement as any non-terminal row does and
// nothing resolves it, so the workload is never relaunched. The smallest bounds that still violate (one fault, two ids).
module rg_bugAbandonWritesNoRow {
  import restart_generation(OFF = Set("abandonWritesNoRow"), REC = false, OPS = Set("lost"), NA = 2, FB = 1, LIMIT = 3,
    DGMAX = 1, JOB = false).* from "../restart_generation"
}

// ADR-0355 item 2 (RB-13): a launch that never ran does not count; the count basis is the newest row that ran. Hazard: the basis is
// the newest row, abandoned or not, so each abandoned launch is one more crash restart of the one before it. Counterexample: #1
// crashes, #2 is its crash restart (count 1) and is lost, #3 is a crash restart of the abandoned #2 (count 2, expected 1); with
// several consecutive abandoned rows the count grows by one for each (NA = 4: #2 and #3 lost, #4 carries 3, expected 1).
module rg_bugAbandonedRowCounts {
  import restart_generation(OFF = Set("abandonedRowCounts"), REC = false, OPS = Set("lost"), NA = 4, FB = 1, LIMIT = 4,
    DGMAX = 1, JOB = false).* from "../restart_generation"
}
