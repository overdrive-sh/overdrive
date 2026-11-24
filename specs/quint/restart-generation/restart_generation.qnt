/**
 * Restart generation (SD-6, ADR-0335, amends ADR-0073; launch outcomes ADR-0337, ADR-0338): a restart generation is handled
 * when its successor is OBSERVED (the allocation issued for it has a row), never when it is emitted. Models
 * `WorkloadLifecycle`'s restart rules over abstract launches; there is no volume, no claim and no VMM (those are
 * specs/quint/node-local-volume/).
 *
 * ONE node, ONE workload (`replicas = 1`), NA allocation ids (each minted once).
 *
 * Abstraction of the launch: a launch is ONE HELD DISPATCH. `WlStart(a)` issues allocation `a` (the evaluation's tick: the
 * lane opens, the View records the issued id) and the allocation is PENDING, which stands for "a launch holds the lane"
 * (production writes no row before the dispatch resolves). One later action resolves it, chosen by the environment flags
 * at that instant, in the order the shim runs: ADMISSION, then IDENTITY issuance, then the DRIVER. Resolutions:
 *   LaunchRefused    capFull                       no row, no charge (the guest-network pool refused; ADR-0338)
 *   LaunchPreFail    not capFull, idFault          a Failed row (SVID issuance or a non-start driver error; ADR-0337)
 *   LaunchRunning    no flag                       a Running row
 *   LaunchStartFail  no flag                       a Failed row (any typed start failure, volume causes included)
 *   LaunchLost       environment event (OPS "lost") an ABANDONED row, no charge: a launch superseded by a stop, a restart or
 *                                                  shutdown, or lost with the control plane (ADR-0355: the `Pending` row it
 *                                                  wrote before the VMM existed ends `Terminated`, `terminal: None`, reason
 *                                                  `LaunchSuperseded` or `LaunchLostWithControlPlane`). No lane signal is
 *                                                  modelled: for the generation rule a superseded launch and a lost one are
 *                                                  identical. CpCrash and NodeReboot take every PENDING allocation to ABANDONED
 *                                                  at the boot step (`converge`'s first step, RB-11).
 * ABANDONED is a terminal, restartable, uncharged class that is not a natural exit: the row is the current row, the relaunch
 * carries its `cnt` unchanged, and it is exempt from the restart ceiling (the platform ended the launch, the workload's intent
 * stands); after a stop nothing is relaunched (`canStart` needs no stop intent).
 * `hasRow` is the model of "an allocation row exists": a PENDING allocation has its `Pending` row (ADR-0353) and an abandoned
 * launch its terminal row; only a refused launch (the guest-network pool refused admission before any row is written) has NONE.
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
    | LaunchRefused({ a: int }) | LaunchPreFail({ a: int }) | LaunchLost({ a: int })
    | LaunchRunning({ a: int }) | LaunchStartFail({ a: int })
    | CpCrash | NodeReboot
}

module restart_generation {
  import restart_generation_conf.ConfStep from "./restart_generation"
  import restart_budget.* from "./restart_budget"   // the restart budget rule, shared with specs/quint/node-local-volume/
  const OFF: Set[str]   // rules off: "stampOnEmit" (ADR-0335), "prefailNoRow" (ADR-0337), "refusalCharged" (ADR-0338), "abandonWritesNoRow" (ADR-0355)
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
  def nonTerminal(s: S): bool = AL.exists(a => s.as.get(a) == PENDING or s.as.get(a) == RUNNING)
  // abandonWritesNoRow (ADR-0355): an abandoned launch is left `Pending` although nothing will ever resolve it
  def dead(s: S, a: int): bool = s.as.get(a) == PENDING and s.why.get(a) == W_LOST
  // a launch or an allocation is in flight: a live PENDING launch or a RUNNING allocation (a dead PENDING row blocks a
  // placement, like any non-terminal row, but nothing is in flight)
  def inFlight(s: S): bool = AL.exists(a => (s.as.get(a) == PENDING and not(dead(s, a))) or s.as.get(a) == RUNNING)
  // the current row is an abandoned launch (the platform ended it; the workload's intent stands)
  def abandonedCur(s: S): bool = curA(s) != 0 and s.as.get(curA(s)) == ABANDONED
  def mintedIds(s: S): int = AL.filter(a => s.as.get(a) != UNMINTED).size()
  // "an allocation row exists for a": a PENDING allocation has its `Pending` row (ADR-0353), an abandoned launch its terminal
  // row (ADR-0355) and a pre-launch failure (W_PREFAIL) its Failed row; an admission refusal (W_REFUSED) has none.
  // Hazard "prefailNoRow" (ADR-0337): a pre-driver failure writes no row. Hazard "refusalCharged" (ADR-0338): an admission
  // refusal writes a (charged) row.
  def hasRow(s: S, a: int): bool = and {
    s.as.get(a) == PENDING or s.as.get(a) == RUNNING or s.as.get(a) == FAILED or s.as.get(a) == TERMINATED
      or s.as.get(a) == ABANDONED,
    OFF.contains("refusalCharged") or s.why.get(a) != W_REFUSED,
    not(OFF.contains("prefailNoRow")) or s.why.get(a) != W_PREFAIL,
  }
  // production `current_alloc` over the observed rows: the highest id with a row, 0 when none
  def curA(s: S): int = AL.fold(0, (m, a) => if (hasRow(s, a) and a > m) a else m)
  // H1 (ADR-0335 item 3): the row of `a` is a finished Job row (every ended Job row carries a terminal claim: Completed,
  // Failed from a natural exit, or Stopped{Operator} from a stop). The action shim's own immutability test.
  def jobTerm(s: S, a: int): bool = and { JOB, hasRow(s, a), s.as.get(a) == TERMINATED or s.as.get(a) == FAILED }
  // a generation is pending until the allocation issued for it has a row, and never against a finished Job row (H1).
  // Hazard "stampOnEmit": the marker stamped on emit decides (the defect SD-6 closes).
  def pendingGen(s: S): bool =
    if (curA(s) == 0) false
    else if (jobTerm(s, curA(s))) false
    else if (OFF.contains("stampOnEmit")) s.og < s.dg
    else s.ig.get(curA(s)) < s.dg
  // the operator-stop veto: the current row was ended by an operator stop or restart and no generation is pending
  def vetoed(s: S): bool = and {
    curA(s) != 0, s.as.get(curA(s)) == TERMINATED,
    s.why.get(curA(s)) == W_STOP or s.why.get(curA(s)) == W_RESTART,
    not(pendingGen(s)),
  }
  // the failures counted against the budget, derived from the per-allocation count of the current row: a Failed current row
  // is one failure beyond its count at issue (R-1 = A)
  def charged(s: S): int =
    budgetCharged(curA(s), s.cnt, curA(s) != 0 and s.as.get(curA(s)) == FAILED)
  // an abandoned current row is exempt from the ceiling (`is_platform_reclaimed`), so it is never BackoffExhausted
  def exhausted(s: S): bool = budgetExhausted(charged(s), LIMIT, nonTerminal(s)) and not(abandonedCur(s))
  // a placement is admitted below the ceiling, for a generation replacement, and after an abandoned launch (ADR-0355 item 2)
  def mayRestart(s: S): bool = budgetMayRestart(charged(s), LIMIT, pendingGen(s) or abandonedCur(s))
  // the count the next issued allocation carries (the shared rule, `restart_budget.qnt`); an abandoned row's own count is
  // carried unchanged (it charges nothing: the abandoned id's entry already holds what its issue assigned)
  def startCnt(s: S): int = if (abandonedCur(s)) s.cnt.get(curA(s)) else budgetStartCnt(curA(s), s.cnt, pendingGen(s))

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
  // `LaunchLostWithControlPlane`, no charge, the count unchanged); a RUNNING allocation stays RUNNING (the VMM survives the
  // crash). abandonWritesNoRow: the row is left PENDING (and nothing resolves it)
  def abandonedAs: int = if (OFF.contains("abandonWritesNoRow")) PENDING else ABANDONED
  def crashLoss(s: S): S =
    { ...s, as: AL.mapBy(a => if (s.as.get(a) == PENDING) abandonedAs else s.as.get(a)),
      why: AL.mapBy(a => if (s.as.get(a) == PENDING) W_LOST else s.why.get(a)) }

  // ---------------------------------------------------------------- workload-lifecycle
  def canStart(s: S, a: int): bool = and {
    s.declared, not(s.stopped), not(nonTerminal(s)), not(vetoed(s)),
    // a `workload restart` is a generation replacement, admitted whatever the attempt count (restart_decision(.., true)); so
    // is a relaunch after an abandoned launch (platform_reclaimed)
    mayRestart(s),
    // H1, RA11: a Job is placed once; any row ends it (its natural exit is finalised, never restarted, a pre-driver failure
    // included), so no crash or failure restarts it. A launch that never ran is not a finished Job: its abandoned row
    // (`terminal: None`, not a natural exit) is relaunched
    not(JOB) or curA(s) == 0 or abandonedCur(s),
    s.as.get(a) == UNMINTED, AL.forall(x => s.as.get(x) != UNMINTED or a <= x),
  }
  // the evaluation's placement (the occupancy check at hydration: not while the pool is at its cap, ADR-0338)
  def canWlStart(s: S, a: int): bool = canStart(s, a) and not(s.capFull)
  action WlStart(a: int): bool = all {
    canWlStart(cur, a),
    commit({ ...cur, as: cur.as.set(a, PENDING),
             ig: cur.ig.set(a, cur.dg), cnt: cur.cnt.set(a, startCnt(cur)),
             og: if (OFF.contains("stampOnEmit")) cur.dg else cur.og }, variant("WlStart", { a: a })),
  }
  // production emits StopAllocation only for a Running row; for the operator stop or a pending generation (R2)
  action StopAlloc(a: int): bool = all {
    cur.as.get(a) == RUNNING, cur.stopped or pendingGen(cur),
    commit({ ...cur, as: cur.as.set(a, TERMINATED), why: cur.why.set(a, if (pendingGen(cur)) W_RESTART else W_STOP) },
      variant("StopAlloc", { a: a })),
  }
  // `workload restart`: the generation bump. A RUNNING allocation ends; a PENDING one changes nothing but the generation;
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
        cur.as.get(a) == PENDING,
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

  // ---------------------------------------------------------------- the launch resolutions (shim order: admission, identity, driver)
  action LaunchRefused(a: int): bool = all {
    cur.as.get(a) == PENDING, not(dead(cur, a)), cur.capFull,
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_REFUSED) },
      variant("LaunchRefused", { a: a })),
  }
  action LaunchPreFail(a: int): bool = all {
    cur.as.get(a) == PENDING, not(dead(cur, a)), not(cur.capFull), cur.idFault,
    commit({ ...cur, as: cur.as.set(a, FAILED), why: cur.why.set(a, W_PREFAIL) },
      variant("LaunchPreFail", { a: a })),
  }
  // environment event (OPS "lost"): the launch is superseded by a stop, a restart or shutdown, or lost with the control plane:
  // its `Pending` row ends ABANDONED (ADR-0355), no charge. Guarded in the shim's order (admission, identity issuance, then the
  // driver observes the signal), so it needs no flag; CpCrash / NodeReboot lose a launch at any moment. No fairness; not part
  // of `Progress`. abandonWritesNoRow: the row is left PENDING
  action LaunchLost(a: int): bool = all {
    OPS.contains("lost"), cur.as.get(a) == PENDING, not(dead(cur, a)), not(cur.capFull), not(cur.idFault),
    commit({ ...cur, as: cur.as.set(a, abandonedAs), why: cur.why.set(a, W_LOST) }, variant("LaunchLost", { a: a })),
  }
  action LaunchRunning(a: int): bool = all {
    cur.as.get(a) == PENDING, not(dead(cur, a)), not(cur.capFull), not(cur.idFault),
    commit({ ...cur, as: cur.as.set(a, RUNNING) }, variant("LaunchRunning", { a: a })),
  }
  action LaunchStartFail(a: int): bool = all {
    cur.as.get(a) == PENDING, not(dead(cur, a)), not(cur.capFull), not(cur.idFault),
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
    any { WlStart(a), StopAlloc(a), LaunchRefused(a), LaunchPreFail(a), LaunchRunning(a), LaunchStartFail(a) }
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
    curA(st) != 0,
    st.why.get(curA(st)) == W_CRASH or st.why.get(curA(st)) == W_STARTFAIL or st.why.get(curA(st)) == W_PREFAIL,
  }
  // H1 (ADR-0335 item 3): a Job is placed once and never replaced after it ends: at most one allocation id has a row, and
  // once the current row is finished no further id is minted (ids that end with no row, a supersession or a refusal, are not
  // counted: they leave no row). Vacuous for JOB = false.
  val I_FINISHED_JOB_ISSUES_NO_ID = not(JOB) or and {
    AL.filter(a => hasRow(st, a) and st.as.get(a) != ABANDONED).size() <= 1,
    jobTerm(st, curA(st)) implies not(AL.exists(a => a > curA(st) and (st.as.get(a) == PENDING or hasRow(st, a)))),
  }
  val Safety = and { I_FINISHED_JOB_ISSUES_NO_ID, I_CAPACITY_WAIT_NEVER_EXHAUSTS }

  // ---------------------------------------------------------------- liveness
  action fWlStart = { nondet a = AL.oneOf() WlStart(a) }
  action fStopAlloc = { nondet a = AL.oneOf() StopAlloc(a) }
  action fRefused = { nondet a = AL.oneOf() LaunchRefused(a) }
  action fPreFail = { nondet a = AL.oneOf() LaunchPreFail(a) }
  action fRunning = { nondet a = AL.oneOf() LaunchRunning(a) }
  action fStartFail = { nondet a = AL.oneOf() LaunchStartFail(a) }
  // every progress action of the spec, the four launch resolutions among them; environment events (`LaunchLost` included) carry none
  temporal FairProgress = and {
    weakFair(fWlStart, (st, conf_step)), weakFair(fStopAlloc, (st, conf_step)), weakFair(fRefused, (st, conf_step)),
    weakFair(fPreFail, (st, conf_step)), weakFair(fRunning, (st, conf_step)), weakFair(fStartFail, (st, conf_step)),
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
  action running1 = init.then(Declare).then(WlStart(1)).then(LaunchRunning(1))
  // allocations 1 and 2 failed as pre-launch failures (the identity fault is still on), nothing in flight
  action prefail2 = init.then(Declare).then(IdFaultOn).then(WlStart(1)).then(LaunchPreFail(1))
    .then(WlStart(2)).then(LaunchPreFail(2))

  // SD-6, ADR-0355: a control-plane crash during the replacement launch loses the launch: boot ends its `Pending` row ABANDONED,
  // no charge; the abandoned row is restartable, so one fresh allocation is placed carrying its count
  run control_plane_crash_during_restart_launch_relaunches_once = running1
    .then(WorkloadRestart(1)).then(WlStart(2)).then(CpCrash)
    .expect(st.as.get(2) == ABANDONED and st.why.get(2) == W_LOST and hasRow(st, 2) and curA(st) == 2 and charged(st) == 0
            and not(pendingGen(st)) and canStart(st, 3) and Safety)
    .then(WlStart(3))
    .expect(st.ig.get(3) == 1 and st.cnt.get(3) == st.cnt.get(2) and Safety)
    .then(LaunchRunning(3))
    .expect(st.as.get(3) == RUNNING and curA(st) == 3 and not(pendingGen(st)) and charged(st) == 0 and Safety)

  // SD-6: stop, resume, a second restart that loses the launch (superseded; for the generation rule a superseded launch is a
  // lost one): an ABANDONED row, no charge, the generation stays pending, and exactly one launch is placed for the latest generation.
  // A restart commit enables the loss but does not force it.
  run second_restart_loses_launch_then_one_placement = running1
    .then(OpStop).then(StopAlloc(1)).then(OpResume).then(WlStart(2)).then(WorkloadRestart(2))
    .expect(st.dg == 2 and Safety)
    .then(LaunchLost(2))
    .expect(st.as.get(2) == ABANDONED and st.why.get(2) == W_LOST and hasRow(st, 2) and pendingGen(st) and charged(st) == 0 and Safety)
    .then(WlStart(3))
    .expect(st.ig.get(3) == 2 and Safety)
    .then(LaunchRunning(3))
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
    .then(WlStart(2)).then(LaunchRunning(2))
    .expect(st.as.get(2) == RUNNING and curA(st) == 2 and not(pendingGen(st)) and charged(st) == 0
            and AL.filter(a => st.ig.get(a) == 1 and hasRow(st, a)).size() == 1 and Safety)

  // SD-6: `workload restart` of a workload whose budget is exhausted: the FAILED row stays, one relaunch for the new generation
  // starts uncharged (R-1 = A: it carries the predecessor's count, so the failure it replaced stops counting: `charged`
  // drops from LIMIT to LIMIT - 1 once the replacement has a row)
  run restart_of_exhausted_workload_relaunches_uncharged = init.then(Declare).then(IdFaultOn)
    .then(WlStart(1)).then(LaunchPreFail(1)).then(WlStart(2)).then(LaunchPreFail(2)).then(WlStart(3)).then(LaunchPreFail(3))
    .expect(exhausted(st) and charged(st) == LIMIT and AL.forall(a => not(canStart(st, a))) and Safety)
    .then(IdFaultOff).then(WorkloadRestart(3))
    .expect(st.dg == 1 and pendingGen(st) and st.as.get(3) == FAILED and st.why.get(3) == W_PREFAIL and curA(st) == 3
            and charged(st) == LIMIT and canStart(st, 4) and Safety)
    .then(WlStart(4))
    .expect(st.ig.get(4) == 1 and st.cnt.get(4) == st.cnt.get(3) and Safety)
    .then(LaunchRunning(4))
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
    .then(LaunchRunning(3))
    .expect(st.as.get(3) == RUNNING and charged(st) == 1 and Safety)
    .then(VmmCrash(3))
    .expect(st.as.get(3) == FAILED and st.why.get(3) == W_CRASH and charged(st) == 2 and not(exhausted(st))
            and canStart(st, 4) and Safety)
    .then(WlStart(4))
    .expect(st.cnt.get(4) == 2 and Safety)
    .then(LaunchStartFail(4))
    .expect(st.why.get(4) == W_STARTFAIL and exhausted(st) and charged(st) == LIMIT and mintedIds(st) == 4
            and AL.forall(a => not(canStart(st, a))) and Safety)

  // SD-6 ruling B: a persistent pre-launch failure (identity issuance): every launch writes a charged Failed row with a reason,
  // then BackoffExhausted; the ids minted equal the restart ceiling and none is launched after
  run persistent_prelaunch_failure_ends_in_backoff_exhausted = init.then(Declare).then(IdFaultOn)
    .then(WlStart(1))
    .expect(st.as.get(1) == PENDING and not(canWlStart(st, 2)) and Safety)
    .then(LaunchPreFail(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_PREFAIL and hasRow(st, 1) and charged(st) == 1 and Safety)
    .then(WlStart(2)).then(LaunchPreFail(2)).then(WlStart(3)).then(LaunchPreFail(3))
    .expect(charged(st) == LIMIT and exhausted(st) and AL.forall(a => not(canWlStart(st, a))) and mintedIds(st) == LIMIT
            and Safety)

  // SD-6 ruling B: a transient pre-launch failure: the failed launch is a charged Failed row, the next launch runs
  run transient_prelaunch_failure_recovers = init.then(Declare).then(IdFaultOn)
    .then(WlStart(1)).then(LaunchPreFail(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_PREFAIL and charged(st) == 1 and Safety)
    .then(IdFaultOff).then(WlStart(2)).then(LaunchRunning(2))
    .expect(st.as.get(2) == RUNNING and charged(st) == 1 and curA(st) == 2 and Safety)

  // SD-6 (D-1 = A): the pool filled between the occupancy check and the dispatch: the refusal writes no row and charges nothing;
  // the occupancy gate (`WlStart` disabled while the pool is at its cap) paces it; when occupancy drops the next launch runs
  run admission_refusal_leaves_no_row_and_waits_for_capacity = init.then(Declare).then(WlStart(1)).then(CapFill)
    .then(LaunchRefused(1))
    .expect(st.as.get(1) == FAILED and st.why.get(1) == W_REFUSED and not(hasRow(st, 1)) and curA(st) == 0 and charged(st) == 0
            and canStart(st, 2) and not(canWlStart(st, 2)) and Safety)
    .then(CapDrain)
    .expect(canWlStart(st, 2))
    .then(WlStart(2)).then(LaunchRunning(2))
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
