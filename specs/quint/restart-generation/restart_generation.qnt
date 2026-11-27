/**
 * Restart generation (SD-6, ADR-0335, amends ADR-0073; launch outcomes ADR-0337, ADR-0338): a restart generation is handled
 * when its successor is OBSERVED (the allocation issued for it has a row), never when it is emitted. Models
 * `WorkloadLifecycle`'s restart rules over abstract launches; there is no volume, no claim and no VMM (those are
 * specs/quint/node-local-volume/).
 *
 * ONE node, ONE workload (`replicas = 1`), NA allocation ids (each minted once).
 *
 * Abstraction of the launch: a launch is ONE HELD DISPATCH in the order the action shim runs (ADR-0353, ADR-0355):
 * ADMISSION (`provision_and_inject_netns` hands out the lease), then the `Pending` row write, then IDENTITY issuance, then the
 * DRIVER. `WlStart(a)` issues allocation `a` (the evaluation's tick: the lane opens, the View records the issued id) and the
 * allocation is ISSUED: a launch holds the lane and NO row exists. Admission has two outcomes: `LaunchRefused` (capFull: no row,
 * no charge, the pool refused; ADR-0338) or `LaunchPending` (the lease is handed out and the `Pending` row is written; the
 * allocation is PENDING, which means the row is written, ADR-0353). Admission is passed for good: from PENDING the flag `capFull`
 * no longer matters. One later action resolves a PENDING launch, chosen by the environment flags at that instant: IDENTITY
 * issuance, then the DRIVER. Resolutions:
 *   LaunchRefused    ISSUED, capFull               no row, no charge (the guest-network pool refused; ADR-0338)
 *   LaunchPending    ISSUED, not capFull           the `Pending` row is written (ADR-0353); the allocation is PENDING
 *   LaunchPreFail    PENDING, idFault              a Failed row replaces the `Pending` row (identity issuance or a non-start driver error; ADR-0337)
 *   LaunchRunning    PENDING, no idFault           a Running row
 *   LaunchStartFail  PENDING, no idFault           a Failed row (any typed start failure, volume causes included)
 *   LaunchLost       environment event (OPS "lost") a PENDING launch ends ABANDONED, no charge: a launch superseded by a stop, a restart or
 *                                                  shutdown, or lost with the control plane (ADR-0355: the `Pending` row it
 *                                                  wrote before the VMM existed ends `Terminated`, `terminal: None`, reason
 *                                                  `LaunchSuperseded` or `LaunchLostWithControlPlane`). No lane signal is
 *                                                  modelled: for the generation rule a superseded launch and a lost one are
 *                                                  identical. CpCrash and NodeReboot take every PENDING allocation to ABANDONED
 *                                                  at the boot step (`converge`'s first step, RB-11). A control-plane loss before the
 *                                                  row is written (an ISSUED launch) ends the launch with NO row and no charge
 *                                                  (W_UNWRITTEN): there is no `Pending` row for boot to close.
 * ABANDONED is a terminal, restartable, uncharged class that is not a natural exit. The row is the newest row (`curA`), but a
 * launch that never ran does not count (RB-13): the count, the pending generation and the ceiling are derived from the newest row
 * that ran (`basisA`), so the relaunch carries what it would carry if the abandoned rows were deleted; after a stop nothing is
 * relaunched (`canStart` needs no stop intent).
 * `hasRow` is the model of "an allocation row exists": a PENDING allocation has its `Pending` row (ADR-0353) and an abandoned
 * launch its terminal row; an ISSUED launch, a refused launch (the guest-network pool refused admission before any row is
 * written) and a launch lost before its row was written (W_UNWRITTEN) have NONE.
 * `ig` is the View's issued-generation map, `dg` the
 * `workloads/<id>/generation` key, `og` a marker no production rule reads (the hazard's stamp on emit).
 *
 * Restart budget (user decision R-1 = option A, 2026-11-18): the rule is `restart_budget.qnt`, owned here and imported by
 * specs/quint/node-local-volume/; the model carries production's PER-ALLOCATION count
 * `cnt[a]` (the View's `restart_counts` at issue): a first placement (no current row) issues with 0, a crash restart (the
 * current row is Failed, no generation pending) with the failed predecessor's count + 1, a generation replacement with the
 * predecessor's count UNCHANGED (`workload_lifecycle.rs:1311-1318`: `predecessor_attempts` for a replacement, `+ 1`
 * otherwise). Production restarts a failed row iff its count is below the ceiling (`restart_decision`, `:51`, with
 * `generation_replacement` admitting regardless), so a chain fails CEILING + 1 times before BackoffExhausted; the model's
 * `charged(s)` counts failures INCLUDING the current failed row (`cnt[curA] + 1` when it is Failed), so
 * LIMIT = CEILING + 1 in these terms. `charged` and `exhausted` are DERIVED, not state; a lost or refused launch has no row,
 * so it changes nothing.
 *
 * Workload kind (H1, ADR-0335 item 3; RA11): the constant `JOB` (false in every main except `rg_conf_job`). A finished Job row
 * (`jobTerm`: a row with a terminal claim) never has a generation pending against it and a Job is placed once (`canStart`
 * needs no current row, so any row, a pre-driver `Failed` row included, ends the Job), so `workload restart` of a finished
 * Job bumps the generation and issues no id.
 *
 * Instances: rg_ok (exhaustive safety), rg_conf (named runs, REC = true), rg_conf_job (the two finished-Job runs),
 * rg_live_replacement and rg_live_prefail (liveness); the hazards under hazard/.
 *
 * Assumptions RA1, RA2, RA4..RA11: README.md.
 */
module restart_generation_conf {
  type ConfStep =
    | Init
    | Declare | OpStop | OpResume | IdFaultOn | IdFaultOff | CapFill | CapDrain
    | WorkloadRestart({ a: int }) | WlStart({ a: int }) | StopAlloc({ a: int }) | VmmCrash({ a: int }) | JobExit({ a: int })
    | LaunchRefused({ a: int }) | LaunchPending({ a: int }) | LaunchPreFail({ a: int }) | LaunchLost({ a: int })
    | LaunchRunning({ a: int }) | LaunchStartFail({ a: int })
    | CpCrash | NodeReboot
}

module restart_generation {
  import restart_generation_conf.ConfStep from "./restart_generation"
  import restart_budget.* from "./restart_budget"   // the restart budget rule, shared with specs/quint/node-local-volume/
  const OFF: Set[str]   // rules off: "stampOnEmit" (ADR-0335), "prefailNoRow" (ADR-0337), "refusalCharged" (ADR-0338), "abandonWritesNoRow" (ADR-0355), "abandonedRowCounts" (ADR-0355 item 2, RB-13)
  const REC: bool       // true on the runs' main: conf_step records the action taken
  const OPS: Set[str]   // enabled operator / environment actions: "stop" "restart" "lost" "idFault" (toggles IdFaultOn/Off) "idFaultHeld" (idFault true from init, never toggled) "capFull" (toggles CapFill/CapDrain)
  const NA: int         // allocation ids 1..NA (an id is minted once)
  const FB: int         // fault budget (VmmCrash, CpCrash, NodeReboot together)
  const LIMIT: int      // failures in a chain before BackoffExhausted (= production's ceiling + 1; abstracted)
  const DGMAX: int      // bound on the desired generation
  const JOB: bool       // the workload kind (H1, ADR-0335 item 3): true = a Job (run once; a finished row is never replaced), false = a service

  val AL = 1.to(NA)

  // allocation rows
  val UNMINTED = 0
  val PENDING = 1
  val RUNNING = 2
  val FAILED = 3
  val TERMINATED = 4
  val ABANDONED = 5     // a launch that ended before running: a Terminated row, `terminal: None`, uncharged (ADR-0355)
  val ISSUED = 6        // the launch holds the lane and has passed no admission: NO row yet (the Pending row is written after admission)
  // why an allocation ended (the numbering of the volume spec where the code exists there)
  val W_NONE = 0
  val W_CRASH = 1       // the VMM crashed, or was lost to a node reboot
  val W_STOP = 5
  val W_RESTART = 6
  val W_LOST = 7        // launch superseded or lost (control-plane crash, node reboot): an ABANDONED row, no charge
  val W_PREFAIL = 9     // a pre-driver failure (SVID issuance, non-start driver error): a Failed row
  val W_REFUSED = 10    // the guest-network pool refused admission: no row, no charge
  val W_STARTFAIL = 11  // one typed start failure (volume unavailable, held, driver rejection): a Failed row
  val W_DONE = 12       // a Job ran to completion (its natural exit): a Terminated row (JOB only)
  val W_UNWRITTEN = 13  // an ISSUED launch lost with the control plane before its Pending row was written: no row, no charge

  type S = {
    declared: bool, stopped: bool,
    as: int -> int, why: int -> int,
    faults: int,
    dg: int,              // committed desired generation (workloads/<id>/generation); survives CpCrash and NodeReboot
    ig: int -> int,       // the generation allocation a was issued for (the View's issued_generation); survives them too
    cnt: int -> int,      // the View's restart_counts at issue (R-1 = A); survives them too
    og: int,              // HAZARD-ONLY marker (stamp on emit): no production rule reads it
    idFault: bool,        // environment flag: SVID issuance fails / a non-start driver error
    capFull: bool         // environment flag: the guest-network pool is at its cap
  }

  var st: S
  var conf_step: ConfStep   // model-only: records the action taken for the conformance driver; never compared (L6)

  val cur: S = st

  action commit(n: S, t: ConfStep): bool = all {
    st' = n,
    conf_step' = if (REC) t else conf_step,
  }

  action init = all {
    st' = {
      declared: false, stopped: false,
      as: AL.mapBy(a => UNMINTED), why: AL.mapBy(a => W_NONE),
      faults: FB,
      dg: 0, ig: AL.mapBy(a => 0), cnt: AL.mapBy(a => 0), og: 0,
      idFault: OPS.contains("idFaultHeld"), capFull: false
    },
    conf_step' = variant("Init", ()),
  }

  // ---------------------------------------------------------------- definitions
  def nonTerminal(s: S): bool = AL.exists(a => s.as.get(a) == ISSUED or s.as.get(a) == PENDING or s.as.get(a) == RUNNING)
  // abandonWritesNoRow (ADR-0355): an abandoned launch is left `Pending` although nothing will ever resolve it
  def dead(s: S, a: int): bool = s.as.get(a) == PENDING and s.why.get(a) == W_LOST
  // a launch or an allocation is in flight: a live PENDING launch or a RUNNING allocation (a dead PENDING row blocks a
  // placement, like any non-terminal row, but nothing is in flight)
  def inFlight(s: S): bool =
    AL.exists(a => s.as.get(a) == ISSUED or (s.as.get(a) == PENDING and not(dead(s, a))) or s.as.get(a) == RUNNING)
  def s_issued(s: S, a: int): bool = s.as.get(a) != UNMINTED
  def mintedIds(s: S): int = AL.filter(a => s.as.get(a) != UNMINTED).size()
  // "an allocation row exists for a": a PENDING allocation has its `Pending` row (ADR-0353), an abandoned launch its terminal
  // row (ADR-0355) and a pre-launch failure (W_PREFAIL) its Failed row; an ISSUED launch (no admission yet), an admission
  // refusal (W_REFUSED) and a launch lost before its row (W_UNWRITTEN) have none.
  // Hazard "prefailNoRow" (ADR-0337): a pre-driver failure writes no row. Hazard "refusalCharged" (ADR-0338): an admission
  // refusal writes a (charged) row.
  def hasRow(s: S, a: int): bool = and {
    s.as.get(a) == PENDING or s.as.get(a) == RUNNING or s.as.get(a) == FAILED or s.as.get(a) == TERMINATED
      or s.as.get(a) == ABANDONED,
    OFF.contains("refusalCharged") or s.why.get(a) != W_REFUSED,
    s.why.get(a) != W_UNWRITTEN,
    not(OFF.contains("prefailNoRow")) or s.why.get(a) != W_PREFAIL,
  }
  // production `current_alloc` over the observed rows: the highest id with a row, 0 when none
  def curA(s: S): int = AL.fold(0, (m, a) => if (hasRow(s, a) and a > m) a else m)
  // RB-13 (ADR-0355 item 2): a launch that never ran does not count. The COUNT BASIS is the newest row that ran: `current_alloc`
  // over the rows that are not abandoned (`last_ran_alloc`). The abandoned rows stay in history (`curA` still returns the newest
  // row, and `canStart`'s "no row is in flight" reads every row); only the derivation of the count, of whether a generation is
  // pending and of the restart ceiling skips them. Hazard "abandonedRowCounts": the basis is the newest row, abandoned or not.
  def ranA(s: S, a: int): bool = hasRow(s, a) and s.as.get(a) != ABANDONED
  def basisA(s: S): int =
    if (OFF.contains("abandonedRowCounts")) curA(s) else AL.fold(0, (m, a) => if (ranA(s, a) and a > m) a else m)
  // H1 (ADR-0335 item 3): the row of `a` is a finished Job row (every ended Job row carries a terminal claim: Completed,
  // Failed from a natural exit, or Stopped{Operator} from a stop). The action shim's own immutability test.
  def jobTerm(s: S, a: int): bool = and { JOB, hasRow(s, a), s.as.get(a) == TERMINATED or s.as.get(a) == FAILED }
  // a generation is pending until the allocation issued for it has a row THAT RAN (an abandoned successor handles nothing, RB-13),
  // and never against a finished Job row (H1).
  // Hazard "stampOnEmit": the marker stamped on emit decides (the defect SD-6 closes).
  def pendingGen(s: S): bool =
    if (basisA(s) == 0) false
    else if (jobTerm(s, basisA(s))) false
    else if (OFF.contains("stampOnEmit")) s.og < s.dg
    else s.ig.get(basisA(s)) < s.dg
  // the operator-stop veto: the count basis was ended by an operator stop or restart and no generation is pending
  def vetoed(s: S): bool = and {
    basisA(s) != 0, s.as.get(basisA(s)) == TERMINATED,
    s.why.get(basisA(s)) == W_STOP or s.why.get(basisA(s)) == W_RESTART,
    not(pendingGen(s)),
  }
  // the failures counted against the budget, derived from the per-allocation count of the count basis: a Failed basis row
  // is one failure beyond its count at issue (R-1 = A); an abandoned row is never read (RB-13)
  def charged(s: S): int =
    budgetCharged(basisA(s), s.cnt, basisA(s) != 0 and s.as.get(basisA(s)) == FAILED)
  def exhausted(s: S): bool = budgetExhausted(charged(s), LIMIT, nonTerminal(s))
  // a placement is admitted below the ceiling and for a generation replacement; the abandoned rows between the basis and the
  // newest row change neither (the launch they ended was admitted by this same rule against this same basis)
  def mayRestart(s: S): bool = budgetMayRestart(charged(s), LIMIT, pendingGen(s))
  // the count the next issued allocation carries (the shared rule, `restart_budget.qnt`), taken from the count basis
  def startCnt(s: S): int = budgetStartCnt(basisA(s), s.cnt, pendingGen(s))

  // ---------------------------------------------------------------- operator and environment facts
  action Declare = all {
    not(cur.declared),
    commit({ ...cur, declared: true }, variant("Declare", ())),
  }
  action OpStop = all {
    OPS.contains("stop"), cur.declared, not(cur.stopped),
    commit({ ...cur, stopped: true }, variant("OpStop", ())),
  }
  // `workload restart` of a stopped workload: the generation bump and the sentinel clear are one commit. Enabled while the
  // allocation is still RUNNING (the stop has not been emitted yet).
  action OpResume = all {
    OPS.contains("stop"), cur.stopped, cur.dg < DGMAX,
    commit({ ...cur, stopped: false, dg: cur.dg + 1 }, variant("OpResume", ())),
  }
  action IdFaultOn = all {
    OPS.contains("idFault"), not(cur.idFault),
    commit({ ...cur, idFault: true }, variant("IdFaultOn", ())),
  }
  action IdFaultOff = all {
    OPS.contains("idFault"), cur.idFault,
    commit({ ...cur, idFault: false }, variant("IdFaultOff", ())),
  }
  action CapFill = all {
    OPS.contains("capFull"), not(cur.capFull),
    commit({ ...cur, capFull: true }, variant("CapFill", ())),
  }
  action CapDrain = all {
    OPS.contains("capFull"), cur.capFull,
    commit({ ...cur, capFull: false }, variant("CapDrain", ())),
  }
  // the loss a control-plane death inflicts on in-flight launches: boot's first step ends every PENDING row ABANDONED (reason
  // `LaunchLostWithControlPlane`, no charge, the count unchanged); an ISSUED launch has no row, so boot has none to close: it
  // ends with no row and no charge (W_UNWRITTEN); a RUNNING allocation stays RUNNING (the VMM survives the crash).
  // abandonWritesNoRow: the PENDING row is left PENDING (and nothing resolves it)
  def abandonedAs: int = if (OFF.contains("abandonWritesNoRow")) PENDING else ABANDONED
  def crashLoss(s: S): S =
    { ...s, as: AL.mapBy(a => if (s.as.get(a) == PENDING) abandonedAs else if (s.as.get(a) == ISSUED) FAILED else s.as.get(a)),
      why: AL.mapBy(a => if (s.as.get(a) == PENDING) W_LOST else if (s.as.get(a) == ISSUED) W_UNWRITTEN else s.why.get(a)) }

  // ---------------------------------------------------------------- workload-lifecycle
  def canStart(s: S, a: int): bool = and {
    s.declared, not(s.stopped), not(nonTerminal(s)), not(vetoed(s)),
    // a `workload restart` is a generation replacement, admitted whatever the attempt count (restart_decision(.., true)); so
    // is a relaunch after an abandoned launch (platform_reclaimed)
    mayRestart(s),
    // H1, RA11: a Job is placed once; any row ends it (its natural exit is finalised, never restarted, a pre-driver failure
    // included), so no crash or failure restarts it. A launch that never ran is not a finished Job: its abandoned row
    // (`terminal: None`, not a natural exit) is relaunched
    not(JOB) or basisA(s) == 0,
    s.as.get(a) == UNMINTED, AL.forall(x => s.as.get(x) != UNMINTED or a <= x),
  }
  // the evaluation's placement (the occupancy check at hydration: not while the pool is at its cap, ADR-0338)
  def canWlStart(s: S, a: int): bool = canStart(s, a) and not(s.capFull)
  action WlStart(a: int): bool = all {
    canWlStart(cur, a),
    commit({ ...cur, as: cur.as.set(a, ISSUED),
             ig: cur.ig.set(a, cur.dg), cnt: cur.cnt.set(a, startCnt(cur)),
             og: if (OFF.contains("stampOnEmit")) cur.dg else cur.og }, variant("WlStart", { a: a })),
  }
  // production emits StopAllocation only for a Running row; for the operator stop or a pending generation (R2)
  action StopAlloc(a: int): bool = all {
    cur.as.get(a) == RUNNING, cur.stopped or pendingGen(cur),
    commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, if (pendingGen(cur)) W_RESTART else W_STOP) },
      variant("StopAlloc", { a: a })),
  }
  // `workload restart`: the generation bump. A RUNNING allocation ends; an ISSUED or PENDING one changes nothing but the generation;
  // a FAILED current row (an exhausted workload included) stays as it is and the generation becomes pending
  action WorkloadRestart(a: int): bool = all {
    OPS.contains("restart"), cur.declared, not(cur.stopped), cur.dg < DGMAX,
    any {
      all {
        cur.as.get(a) == RUNNING,
        commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, W_RESTART), dg: cur.dg + 1 },
          variant("WorkloadRestart", { a: a })),
      },
      all {
        cur.as.get(a) == ISSUED or cur.as.get(a) == PENDING,
        commit({ ...cur, dg: cur.dg + 1 }, variant("WorkloadRestart", { a: a })),
      },
      all {
        cur.as.get(a) == FAILED, a == curA(cur),
        commit({ ...cur, dg: cur.dg + 1 }, variant("WorkloadRestart", { a: a })),
      },
      // H1: the operator's `workload restart` of a finished Job: the generation advances and nothing else changes
      all {
        JOB, cur.as.get(a) == TERMINATED, a == curA(cur),
        commit({ ...cur, dg: cur.dg + 1 }, variant("WorkloadRestart", { a: a })),
      },
    },
  }

  // ---------------------------------------------------------------- the launch resolutions (shim order: admission, the Pending row, identity, driver)
  // admission (`provision_and_inject_netns`): the pool refuses before the lease is taken and before any row is written (ADR-0338)
  action LaunchRefused(a: int): bool = all {
    cur.as.get(a) == ISSUED, cur.capFull,
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_REFUSED) },
      variant("LaunchRefused", { a: a })),
  }
  // admission passed: the lease is handed out and the `Pending` row is written with its address (ADR-0353). Provision failures
  // other than the refusal are not modelled (README)
  action LaunchPending(a: int): bool = all {
    cur.as.get(a) == ISSUED, not(cur.capFull),
    commit({ ...cur, as: cur.as.set(a, PENDING) }, variant("LaunchPending", { a: a })),
  }
  // identity issuance fails after the `Pending` write: a charged `Failed` row ends the `Pending` row (ADR-0337)
  action LaunchPreFail(a: int): bool = all {
    cur.as.get(a) == PENDING, not(dead(cur, a)), cur.idFault,
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_PREFAIL) },
      variant("LaunchPreFail", { a: a })),
  }
  // environment event (OPS "lost"): the launch is superseded by a stop, a restart or shutdown, or lost with the control plane:
  // its `Pending` row ends ABANDONED (ADR-0355), no charge. Only a PENDING launch (the row is written; supersession is observed
  // at the driver). Guarded in the shim's order (identity issuance, then the driver observes the signal), so it needs no
  // admission flag; CpCrash / NodeReboot lose a launch at any moment, an ISSUED one included (no row, W_UNWRITTEN). No fairness;
  // not part of `Progress`. abandonWritesNoRow: the row is left PENDING
  action LaunchLost(a: int): bool = all {
    OPS.contains("lost"), cur.as.get(a) == PENDING, not(dead(cur, a)), not(cur.idFault),
    commit({ ...cur, as: cur.as.set(a, abandonedAs), why: cur.why.set(a, W_LOST) }, variant("LaunchLost", { a: a })),
  }
  action LaunchRunning(a: int): bool = all {
    cur.as.get(a) == PENDING, not(dead(cur, a)), not(cur.idFault),
    commit({ ...cur, as: cur.as.set(a, RUNNING) }, variant("LaunchRunning", { a: a })),
  }
  action LaunchStartFail(a: int): bool = all {
    cur.as.get(a) == PENDING, not(dead(cur, a)), not(cur.idFault),
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_STARTFAIL) }, variant("LaunchStartFail", { a: a })),
  }

  // ---------------------------------------------------------------- environment faults (each draws on FB)
  // the VMM of a RUNNING allocation dies: a Failed row, one failure beyond its count
  action VmmCrash(a: int): bool = all {
    cur.as.get(a) == RUNNING, cur.faults > 0,
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_CRASH), faults: cur.faults - 1 },
      variant("VmmCrash", { a: a })),
  }
  // H1: a Job ran to completion (its natural exit): the row is Terminated with the Job's own reason. Environment event, no
  // fairness (none of the temporal properties runs with JOB).
  action JobExit(a: int): bool = all {
    JOB, cur.as.get(a) == RUNNING,
    commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, W_DONE) }, variant("JobExit", { a: a })),
  }
  // control-plane crash: a PENDING launch is lost (its row ends ABANDONED, no charge); a RUNNING allocation stays RUNNING (a RUNNING row
  // blocks a placement as in production; RA5)
  action CpCrash = all {
    cur.faults > 0,
    commit({ ...crashLoss(cur), faults: cur.faults - 1 }, variant("CpCrash", ())),
  }
  // node reboot: as CpCrash, and a RUNNING allocation ends Failed (W_CRASH)
  action NodeReboot = all {
    cur.faults > 0,
    commit({ ...crashLoss(cur), as: AL.mapBy(a => if (cur.as.get(a) == RUNNING) FAILED else crashLoss(cur).as.get(a)),
             why: AL.mapBy(a => if (cur.as.get(a) == RUNNING) W_CRASH else crashLoss(cur).why.get(a)),
             faults: cur.faults - 1 }, variant("NodeReboot", ())),
  }

  // ---------------------------------------------------------------- composition
  action Progress = {
    nondet a = AL.oneOf()
    any { WlStart(a), StopAlloc(a), LaunchRefused(a), LaunchPending(a), LaunchPreFail(a), LaunchRunning(a), LaunchStartFail(a) }
  }
  action step = {
    nondet a = AL.oneOf()
    any { Declare, OpStop, OpResume, WorkloadRestart(a), LaunchLost(a), IdFaultOn, IdFaultOff, CapFill, CapDrain, VmmCrash(a),
          JobExit(a), CpCrash, NodeReboot, Progress }
  }

  // ---------------------------------------------------------------- invariants (SD-6)
  // ADR-0338 / D-1: a node held at the guest-network cap never exhausts a healthy workload's budget: a workload that is
  // exhausted ended by a charged cause (a crash, a typed start failure or a pre-driver failure), never by a refusal or a lost launch
  val I_CAPACITY_WAIT_NEVER_EXHAUSTS = exhausted(st) implies and {
    basisA(st) != 0,
    st.why.get(basisA(st)) == W_CRASH or st.why.get(basisA(st)) == W_STARTFAIL or st.why.get(basisA(st)) == W_PREFAIL,
  }
  // H1 (ADR-0335 item 3): a Job is placed once and never replaced after it ends: at most one allocation id has a row, and
  // once the current row is finished no further id is minted (ids that end with no row, a supersession or a refusal, are not
  // counted: they leave no row). Vacuous for JOB = false.
  val I_FINISHED_JOB_ISSUES_NO_ID = not(JOB) or and {
    AL.filter(a => hasRow(st, a) and st.as.get(a) != ABANDONED).size() <= 1,
    jobTerm(st, basisA(st)) implies
      not(AL.exists(a => a > basisA(st) and (st.as.get(a) == ISSUED or st.as.get(a) == PENDING or hasRow(st, a)))),
  }
  // RB-13 (ADR-0355 item 2, deletion-equivalence): deleting the abandoned rows from the chain changes no later launch's count.
  // Every allocation's count at issue equals the count derived over the chain of the lower ids with the abandoned rows DELETED
  // (the rows below an issued id are final when it is issued: only an in-flight row changes). The basis is the newest surviving
  // row, a generation was pending iff its issued generation is older than this launch's, and then the count is unchanged, else + 1.
  def delBasis(s: S, a: int): int = AL.fold(0, (m, b) => if (b < a and ranA(s, b) and b > m) b else m)
  def delCnt(s: S, a: int): int =
    if (delBasis(s, a) == 0) 0
    else if (not(jobTerm(s, delBasis(s, a))) and s.ig.get(delBasis(s, a)) < s.ig.get(a)) s.cnt.get(delBasis(s, a))
    else s.cnt.get(delBasis(s, a)) + 1
  val I_ABANDONED_ROWS_DO_NOT_COUNT = AL.forall(a => s_issued(st, a) implies st.cnt.get(a) == delCnt(st, a))
  val Safety = and { I_FINISHED_JOB_ISSUES_NO_ID, I_CAPACITY_WAIT_NEVER_EXHAUSTS, I_ABANDONED_ROWS_DO_NOT_COUNT }

  // ---------------------------------------------------------------- liveness
  action fWlStart = { nondet a = AL.oneOf() WlStart(a) }
  action fStopAlloc = { nondet a = AL.oneOf() StopAlloc(a) }
  action fRefused = { nondet a = AL.oneOf() LaunchRefused(a) }
  action fPending = { nondet a = AL.oneOf() LaunchPending(a) }
  action fPreFail = { nondet a = AL.oneOf() LaunchPreFail(a) }
  action fRunning = { nondet a = AL.oneOf() LaunchRunning(a) }
  action fStartFail = { nondet a = AL.oneOf() LaunchStartFail(a) }
  // every progress action of the spec, the five launch resolutions among them; environment events (`LaunchLost` included) carry none
  temporal FairProgress = and {
    weakFair(fWlStart, (st, conf_step)), weakFair(fStopAlloc, (st, conf_step)), weakFair(fRefused, (st, conf_step)),
    weakFair(fPending, (st, conf_step)), weakFair(fPreFail, (st, conf_step)), weakFair(fRunning, (st, conf_step)), weakFair(fStartFail, (st, conf_step)),
  }
  // intent says run, the node is up and nothing blocks it, and no launch is in flight: a launch for the current generation
  // must eventually be issued
  val stranded = and {
    st.declared, not(st.stopped), not(st.capFull), not(st.idFault),
    mayRestart(st), not(inFlight(st)), AL.exists(a => st.as.get(a) == UNMINTED),
  }
  temporal RunIntentProgress = FairProgress.implies(always(stranded.implies(eventually(not(stranded)))))
  // under `idFault` held true forever a declared workload ends in BackoffExhausted, and the ids minted stay bounded: the
  // restart ceiling plus one uncharged id per operator commit (the generation replacement of the failed row; a restart of a
  // launch in flight changes nothing but the generation, and no launch is lost here)
  temporal PreFailExhausts = FairProgress.implies(and {
    always(st.declared.implies(eventually(exhausted(st)))),
    always(mintedIds(st) <= LIMIT + st.dg),
  })

  // ---------------------------------------------------------------- named runs (one per critical scenario)
  action running1 = init.then(Declare).then(WlStart(1)).then(LaunchPending(1)).then(LaunchRunning(1))
  // allocations 1 and 2 failed as pre-launch failures (the identity fault is still on), nothing in flight
  action prefail2 = init.then(Declare).then(IdFaultOn).then(WlStart(1)).then(LaunchPending(1)).then(LaunchPreFail(1))
    .then(WlStart(2)).then(LaunchPending(2)).then(LaunchPreFail(2))

  // SD-6, ADR-0355: a control-plane crash during the replacement launch loses the launch: boot ends its `Pending` row ABANDONED,
  // no charge; the abandoned row is restartable, so one fresh allocation is placed carrying its count
  run control_plane_crash_during_restart_launch_relaunches_once = running1
    .then(WorkloadRestart(1)).then(WlStart(2)).then(LaunchPending(2)).then(CpCrash)
    .expect(st.as.get(2) == ABANDONED and st.why.get(2) == W_LOST and hasRow(st, 2) and curA(st) == 2 and basisA(st) == 1
            and charged(st) == 0 and pendingGen(st) and canStart(st, 3) and Safety)
    .then(WlStart(3))
    .expect(st.ig.get(3) == 1 and st.cnt.get(3) == st.cnt.get(1) and Safety)
    .then(LaunchPending(3)).then(LaunchRunning(3))
    .expect(st.as.get(3) == RUNNING and curA(st) == 3 and not(pendingGen(st)) and charged(st) == 0 and Safety)

  // RB-13 (ADR-0355 item 2): a crash restart abandoned by an operator restart forgives the crash. #1 crashed (Failed, count 0), #2 is
  // its crash restart (count 1) and ends abandoned by the restart commit; the generation is still pending (an abandoned successor
  // handles nothing), so #3 is a generation replacement carrying the BASIS row's count 0, not #2's 1
  run crash_restart_abandoned_by_operator_restart_forgives_the_crash = running1
    .then(VmmCrash(1)).then(WlStart(2))
    .expect(st.cnt.get(2) == 1 and not(hasRow(st, 2)) and basisA(st) == 1 and charged(st) == 1 and Safety)
    .then(LaunchPending(2))
    .expect(st.cnt.get(2) == 1 and basisA(st) == 2 and charged(st) == 1 and Safety)
    .then(WorkloadRestart(2)).then(LaunchLost(2))
    .expect(st.as.get(2) == ABANDONED and curA(st) == 2 and basisA(st) == 1 and pendingGen(st) and charged(st) == 1 and Safety)
    .then(WlStart(3))
    .expect(st.cnt.get(3) == 0 and st.ig.get(3) == 1 and Safety)
    .then(LaunchPending(3)).then(LaunchRunning(3))
    .expect(st.as.get(3) == RUNNING and not(pendingGen(st)) and charged(st) == 0 and Safety)

  // RB-13: a lost launch with no operator action charges the crash once, however many consecutive launches were lost: #2, #3 abandoned,
  // #4 carries #1's count + 1 = 1 (not 2 or 3), and no generation is pending
  run consecutive_abandoned_launches_charge_the_crash_once = running1
    .then(VmmCrash(1)).then(WlStart(2)).then(LaunchPending(2)).then(LaunchLost(2))
    .then(WlStart(3)).then(LaunchPending(3)).then(LaunchLost(3))
    .expect(st.as.get(2) == ABANDONED and st.as.get(3) == ABANDONED and curA(st) == 3 and basisA(st) == 1
            and not(pendingGen(st)) and charged(st) == 1 and canStart(st, 4) and Safety)
    .then(WlStart(4))
    .expect(st.cnt.get(4) == 1 and Safety)

  // SD-6: stop, resume, a second restart that loses the launch (superseded; for the generation rule a superseded launch is a
  // lost one): an ABANDONED row, no charge, the generation stays pending, and exactly one launch is placed for the latest generation.
  // A restart commit enables the loss but does not force it.
  run second_restart_loses_launch_then_one_placement = running1
    .then(OpStop).then(StopAlloc(1)).then(OpResume).then(WlStart(2)).then(LaunchPending(2)).then(WorkloadRestart(2))
    .expect(st.dg == 2 and Safety)
    .then(LaunchLost(2))
    .expect(st.as.get(2) == ABANDONED and st.why.get(2) == W_LOST and hasRow(st, 2) and pendingGen(st) and charged(st) == 0 and Safety)
    .then(WlStart(3))
    .expect(st.ig.get(3) == 2 and Safety)
    .then(LaunchPending(3)).then(LaunchRunning(3))
    .expect(st.as.get(3) == RUNNING and not(pendingGen(st)) and charged(st) == 0
            and AL.filter(a => st.ig.get(a) == 2 and hasRow(st, a)).size() == 1 and Safety)

  // SD-6: a restart of a RUNNING workload whose stop is still pending (`OpResume` landed before `StopAlloc`): the allocation is
  // stopped for the restart (W_RESTART) and one replacement is placed, uncharged
  run resume_while_running_stops_and_replaces = running1
    .then(OpStop).then(OpResume)
    .expect(st.as.get(1) == RUNNING and curA(st) == 1 and st.dg == 1 and pendingGen(st) and not(vetoed(st)) and not(st.stopped)
            and Safety)
    .then(StopAlloc(1))
    .expect(st.as.get(1) == TERMINATED and st.why.get(1) == W_RESTART and not(vetoed(st)) and pendingGen(st) and Safety)
    .then(WlStart(2)).then(LaunchPending(2)).then(LaunchRunning(2))
    .expect(st.as.get(2) == RUNNING and curA(st) == 2 and not(pendingGen(st)) and charged(st) == 0
            and AL.filter(a => st.ig.get(a) == 1 and hasRow(st, a)).size() == 1 and Safety)

  // SD-6: `workload restart` of a workload whose budget is exhausted: the FAILED row stays, one relaunch for the new generation
  // starts uncharged (R-1 = A: it carries the predecessor's count, so the failure it replaced stops counting: `charged`
  // drops from LIMIT to LIMIT - 1 once the replacement has a row)
  run restart_of_exhausted_workload_relaunches_uncharged = init.then(Declare).then(IdFaultOn)
    .then(WlStart(1)).then(LaunchPending(1)).then(LaunchPreFail(1))
    .then(WlStart(2)).then(LaunchPending(2)).then(LaunchPreFail(2))
    .then(WlStart(3)).then(LaunchPending(3)).then(LaunchPreFail(3))
    .expect(exhausted(st) and charged(st) == LIMIT and AL.forall(a => not(canStart(st, a))) and Safety)
    .then(IdFaultOff).then(WorkloadRestart(3))
    .expect(st.dg == 1 and pendingGen(st) and st.as.get(3) == FAILED and st.why.get(3) == W_PREFAIL and curA(st) == 3
            and charged(st) == LIMIT and canStart(st, 4) and Safety)
    .then(WlStart(4))
    .expect(st.ig.get(4) == 1 and st.cnt.get(4) == st.cnt.get(3) and Safety)
    .then(LaunchPending(4)).then(LaunchRunning(4))
    .expect(st.as.get(4) == RUNNING and curA(st) == 4 and not(pendingGen(st)) and charged(st) == LIMIT - 1 and Safety)

  // R-1 = A, the divergence of the earlier running count: two pre-launch failures, `workload restart`, the replacement runs,
  // its VMM is killed. The replacement carried count 1, the crash is its second failure, and production's restart decision
  // admits another restart.
  run generation_replacement_forgives_predecessor_failure = prefail2
    .expect(charged(st) == 2 and not(exhausted(st)) and st.cnt.get(2) == 1 and Safety)
    .then(IdFaultOff).then(WorkloadRestart(2))
    .expect(st.dg == 1 and pendingGen(st) and charged(st) == 2 and Safety)
    .then(WlStart(3))
    .expect(st.ig.get(3) == 1 and st.cnt.get(3) == 1 and Safety)
    .then(LaunchPending(3)).then(LaunchRunning(3))
    .expect(st.as.get(3) == RUNNING and charged(st) == 1 and Safety)
    .then(VmmCrash(3))
    .expect(st.as.get(3) == FAILED and st.why.get(3) == W_CRASH and charged(st) == 2 and not(exhausted(st))
            and canStart(st, 4) and Safety)
    .then(WlStart(4))
    .expect(st.cnt.get(4) == 2 and Safety)
    .then(LaunchPending(4)).then(LaunchStartFail(4))
    .expect(st.why.get(4) == W_STARTFAIL and exhausted(st) and charged(st) == LIMIT and mintedIds(st) == 4
            and AL.forall(a => not(canStart(st, a))) and Safety)

  // SD-6 ruling B: a persistent pre-launch failure (identity issuance): every launch writes a charged Failed row with a reason,
  // then BackoffExhausted; the ids minted equal the restart ceiling and none is launched after
  run persistent_prelaunch_failure_ends_in_backoff_exhausted = init.then(Declare).then(IdFaultOn)
    .then(WlStart(1))
    .expect(st.as.get(1) == ISSUED and not(hasRow(st, 1)) and not(canWlStart(st, 2)) and Safety)
    .then(LaunchPending(1))
    .expect(st.as.get(1) == PENDING and hasRow(st, 1) and curA(st) == 1 and not(canWlStart(st, 2)) and Safety)
    .then(LaunchPreFail(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_PREFAIL and hasRow(st, 1) and charged(st) == 1 and Safety)
    .then(WlStart(2)).then(LaunchPending(2)).then(LaunchPreFail(2))
    .then(WlStart(3)).then(LaunchPending(3)).then(LaunchPreFail(3))
    .expect(charged(st) == LIMIT and exhausted(st) and AL.forall(a => not(canWlStart(st, a))) and mintedIds(st) == LIMIT
            and Safety)

  // SD-6 ruling B: a transient pre-launch failure: the failed launch is a charged Failed row, the next launch runs
  run transient_prelaunch_failure_recovers = init.then(Declare).then(IdFaultOn)
    .then(WlStart(1)).then(LaunchPending(1)).then(LaunchPreFail(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_PREFAIL and charged(st) == 1 and Safety)
    .then(IdFaultOff).then(WlStart(2)).then(LaunchPending(2)).then(LaunchRunning(2))
    .expect(st.as.get(2) == RUNNING and charged(st) == 1 and curA(st) == 2 and Safety)

  // SD-6 (D-1 = A): the pool filled between the occupancy check and the dispatch: admission refuses the ISSUED launch before the
  // `Pending` row exists (ADR-0353: the row is written after admission), so the refusal writes no row and charges nothing;
  // the occupancy gate (`WlStart` disabled while the pool is at its cap) paces it; when occupancy drops the next launch runs
  run admission_refusal_leaves_no_row_and_waits_for_capacity = init.then(Declare).then(WlStart(1))
    .expect(st.as.get(1) == ISSUED and not(hasRow(st, 1)) and curA(st) == 0 and Safety)
    .then(CapFill)
    .then(LaunchRefused(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_REFUSED and not(hasRow(st, 1)) and curA(st) == 0 and charged(st) == 0
            and canStart(st, 2) and not(canWlStart(st, 2)) and Safety)
    .then(CapDrain)
    .expect(canWlStart(st, 2))
    .then(WlStart(2)).then(LaunchPending(2)).then(LaunchRunning(2))
    .expect(st.as.get(2) == RUNNING and curA(st) == 2 and charged(st) == 0 and Safety)
  // H1: `workload restart` of a Job that completed: the generation advances, nothing is pending, no id is issued
  run restart_of_completed_job_does_nothing_and_issues_no_id = running1
    .then(JobExit(1))
    .expect(st.as.get(1) == TERMINATED and st.why.get(1) == W_DONE and jobTerm(st, 1) and curA(st) == 1 and Safety)
    .then(WorkloadRestart(1))
    .expect(st.dg == 1 and not(pendingGen(st)) and not(canStart(st, 2)) and mintedIds(st) == 1
            and AL.forall(a => not(canWlStart(st, a))) and Safety)

  // H1: stop, then `workload restart` of a Job: the operator-stopped Job stays down and no id is issued. Without the rule the
  // generation would be pending against the Terminated row and every evaluation would issue an id (the shim refuses each one)
  run resume_of_stopped_job_does_nothing_and_issues_no_id = running1
    .then(OpStop).then(StopAlloc(1))
    .expect(st.as.get(1) == TERMINATED and st.why.get(1) == W_STOP and jobTerm(st, 1) and Safety)
    .then(OpResume)
    .expect(st.dg == 1 and not(pendingGen(st)) and vetoed(st) and not(canStart(st, 2)) and mintedIds(st) == 1
            and AL.forall(a => not(canWlStart(st, a))) and Safety)
}

// exhaustive safety: every operator action and both environment flags enabled
module rg_ok {
  import restart_generation(OFF = Set(), REC = false,
    OPS = Set("stop", "restart", "lost", "idFault", "capFull"), NA = 4, FB = 2, LIMIT = 3, DGMAX = 2, JOB = false).*
    from "./restart_generation"
}
// the named runs; the #[quint_test] pins
module rg_conf {
  import restart_generation(OFF = Set(), REC = true,
    OPS = Set("stop", "restart", "lost", "idFault", "capFull"), NA = 4, FB = 2, LIMIT = 3, DGMAX = 2, JOB = false).*
    from "./restart_generation"
}
// liveness: intent says run, a launch is eventually issued (stop, restart and lost enabled; faults finite)
module rg_live_replacement {
  import restart_generation(OFF = Set(), REC = false, OPS = Set("stop", "restart", "lost"), NA = 4, FB = 1, LIMIT = 3,
    DGMAX = 2, JOB = false).* from "./restart_generation"
}
// liveness: an identity fault held true forever, restart enabled: BackoffExhausted with bounded ids. No other faults (FB = 0).
// No launch is lost here, so each launch resolves as a charged `Failed` row; the only uncharged id is the
// generation replacement of that row, one per operator commit: ids minted <= LIMIT + dg, hence NA = LIMIT + DGMAX.
module rg_live_prefail {
  import restart_generation(OFF = Set(), REC = false, OPS = Set("restart", "idFaultHeld"), NA = 4, FB = 0, LIMIT = 3,
    DGMAX = 1, JOB = false).* from "./restart_generation"
}
// H1: the finished Job runs (REC = true), the constants of rg_ok with the workload kind a Job
module rg_conf_job {
  import restart_generation(OFF = Set(), REC = true,
    OPS = Set("stop", "restart", "lost", "idFault", "capFull"), NA = 4, FB = 2, LIMIT = 3, DGMAX = 2, JOB = true).*
    from "./restart_generation"
}
