// One hazard per decision (ADR-0326, ADR-0328, ADR-0329).

// SD-6 (ADR-0326): the restart generation is recorded handled when the launch is EMITTED, not when its allocation is
// observed: `og = dg` at WlStart and `pendingGen` reads `og`. Counterexample: restart of a Running workload, the replacement
// launch is issued (`og = dg`), the launch is lost (no row), no generation is pending and the operator-stop veto is set:
// the workload is stranded forever. The smallest bounds that still violate (three ids, one restart commit, no fault).
module rg_bugStampOnEmit {
  import restart_generation(OFF = Set("stampOnEmit"), REC = false, OPS = Set("restart", "lost"), NA = 3, FB = 0, LIMIT = 3,
    DGMAX = 1, JOB = false).* from "../restart_generation"
}

// ADR-0328: a pre-driver failure writes a charged `Failed` row. Hazard: it writes no row, so nothing is charged, the current
// row stays as it was and an id is issued per tick: the budget never exhausts. The constants of `rg_live_prefail`.
module rg_bugPrefailNoRow {
  import restart_generation(OFF = Set("prefailNoRow"), REC = false, OPS = Set("restart", "idFaultHeld"), NA = 4, FB = 0,
    LIMIT = 3, DGMAX = 1, JOB = false).* from "../restart_generation"
}

// ADR-0329: an admission refusal at the guest-network cap writes no row and charges nothing. Hazard: the refusal is a charged
// row, so a node held at the cap exhausts a healthy workload's budget. Three refusals suffice (LIMIT = 3).
module rg_bugRefusalCharged {
  import restart_generation(OFF = Set("refusalCharged"), REC = false, OPS = Set("capFull"), NA = 4, FB = 0, LIMIT = 3,
    DGMAX = 1, JOB = false).* from "../restart_generation"
}
