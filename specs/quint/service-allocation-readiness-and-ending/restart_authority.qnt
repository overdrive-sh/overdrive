// -*- mode: Bluespec; -*-
/**
 * One authority for a Service allocation's ending (service-allocation-readiness-and-ending): D5 (ServiceLifecycle announces a
 * startup-probe failure only for a live allocation), D6 (WorkloadLifecycle alone authors a crashed allocation's ending and
 * names a pre-startup crash loop at give-up), D7 (a startup-probe failure stops the allocation; WorkloadLifecycle restarts
 * it and names the failure at give-up), D8 (a startup stop that does not land is retried once per new startup probe row).
 * D9 (liveness begins at Stable) is its own module (liveness_gate.qnt); the deploy stream (D10, D11) is deploy_stream.qnt.
 *
 * One workload of kind Service; attempts 0..CEIL, each a fresh allocation id whose row persists (ADR-0105). The restart
 * ceiling is the module parameter CEIL (restarts allowed before the give-up). `decl` is set by the first action, Deploy:
 * the Service declares a startup probe (decl) or opts out (no startup probe; its first Running is its Stable, ADR-0058).
 * DG-2: Deploy and WLRestart take the start outcome `ok` and apply it atomically (no pending row, no separate start action).
 * The backoff between a terminal row and its restart is abstracted away (any time later).
 *
 * Per attempt (record Att):
 *   row      none | running | failed_crash (Failed, WorkloadCrashedImmediately | VmOutOfMemory: the two reasons the exit
 *            observer writes for a guest that ended on its own, B3 / ADR-0258) | failed_other (Failed, any other reason: a
 *            start rejection, a fail-closed supersession of a Running row such as MtlsInterceptInstallFailed,
 *            WorkloadNetnsProvisionFailed or DriverInternalError; these are not guest crashes and are outside the crash class) | t_startup (Terminated, Stopped StartupProbe)
 *            | t_liveness (Terminated, Stopped LivenessProbe) | t_operator (Terminated, Stopped Operator: the
 *            intentional-stop set) | t_clean (Terminated, Stopped Process: a clean exit)
 *   sup      the probe supervisor is present (the D1 hook ran at the crash / stop, so probes stop and rows stop growing)
 *   srow     latest startup probe row: absent | fail | pass;  sseq: startup rows written;  sstreak: the producer-counted
 *            consecutive failures, capped at MAXA (the startup max_attempts)
 *   dl       the startup deadline has passed;  stable: Stable announced;  ann: terminal_announced membership
 *   acted    ServiceLifecycleView.startup_last_acted_run for (alloc, 0): the sseq of the row a stop last acted on (0: none)
 *   stopped  outcome of the last startup stop: none | landed | failed;  vm: the VM slot is live
 *   Stop stages (B2, ADR-0266): a stop (startup or liveness) is the shim's StopAllocation arm in the order effects
 *            (VmDriver::stop, mTLS stop, network teardown), terminal row write, on_alloc_terminal hook. The stage parameter is
 *            landed (all three ran), pre_hook (an effect failed: nothing changed, the row stays Running) or row_write (the
 *            effects ran, the VM is stopped, the row write failed: the row stays Running and, in the design, the supervisor
 *            stays registered because the hook has not run)
 *   claim    the failure claim on the row's terminal: "" | early_exit | startup_probe_failed | liveness_probe_failed |
 *            backoff_exhausted;  author: the reconciler that authored the claim
 *   crashed  ghost: the row was once failed_crash
 * Workload flag gl (set at Deploy, only with decl): the Service's startup descriptor is GuestListeners (D14 item 3). It carries two
 * rules: no Fail row before the deadline (the producer), and the stop gate threshold sstreak >= 1 instead of MAXA.
 * Workload: cur (current attempt), used (restart budget consumed), done (given up).
 *
 * Actors. Environment: Deploy, Crash, CleanExit, OperatorStop, StartupRow, Deadline.
 * ServiceLifecycle: SLAnnounceTerminal (DG-12: any Failed / Terminated row joins ann), SLStable, SLStartupStop (the dispatch of the stop is awaited inside the reconcile tick, so its
 * stage (landed | pre_hook | row_write) is decided in the same step, as in probe-count PC-4), SLLivenessStop. WorkloadLifecycle: WLRestart,
 * WLGiveUp (the only author of a failure claim).
 *
 * Not modelled (stated by the design, not reached by an invariant here): E-2 (a crash racing the stop between evaluation
 * and dispatch overwrites the Failed row with the stop row; DISTILL S-28 pins it); the reclaimed row (restarted at any
 * count, consumes no budget, never the row a give-up is decided on: feature-delta Pinned readings R-1); backend membership and readiness; the Job kind.
 *
 * E-2 exclusion (R-3, a documented bounded exposure, not modelled): a crash between the stop's evaluation and its dispatch would
 * overwrite the Failed row with the stop row. The stop is a single awaited step (RA-2 = V28), so that crash is not generated;
 * feature-delta D7 "Exposures" E-2 and DISTILL S-28 carry it.
 *
 * OFF names the rules switched off (hazards):
 *   guardAcceptsFailed (D5)       the startup stop is also emitted for a Failed crash row
 *   slFinalisesCrash (D6)         a ServiceLifecycle action writes the early_exit claim on a Failed crash row while
 *                                 WorkloadLifecycle keeps restarting it
 *   slFinalisesStartup (D7)       ServiceLifecycle writes startup_probe_failed on a live row (the HEAD finalisation)
 *                                 instead of stopping it
 *   hookBeforeRowWrite (B2)       a row-write failure of a stop has already removed the supervisor (the hook ran before the
 *                                 row write), so nothing converges the stopped-but-Running allocation
 *   stampAtEmission (D8)          the dedup is terminal_announced membership: a startup stop that did not land is never
 *                                 re-emitted
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   RA-1  The D1 hook ran at a crash or a stop: the supervisor is gone and the startup streak stops growing (hook_ownership
 *         module). Discharged by: S-1, S-5, V2.
 *   RA-2  The shim's StopAllocation dispatch is awaited inside the ServiceLifecycle reconcile tick, so a stop's outcome is
 *         known before the next evaluation (`landed` writes the row Terminated and stops the VM; `failed` leaves the row
 *         Running), and the arm's cleanup effects are idempotent when it runs again. Discharged by: V28 (which reads
 *         mtls_lifecycle.stop_alloc and the other effects), S-30 (scripted SimDriver::stop failure), as probe-count PC-4.
 *   RA-3  A row written Terminated / Failed is never deleted or rewritten by a successor (fresh ids, V9). Discharged by: V9.
 *   RA-4  A dead guest cannot produce a startup Pass (V18, Q3): the model writes startup rows only while the supervisor is
 *         present and the row is Running. Discharged by: V18, S-30.
 *   RA-5  A probe row of a Running allocation arrives every interval after the deadline (V17), so a retry keyed by row
 *         sequence is paced. Discharged by: V17.
 *
 * D-UP-1 (conformance step record): `conf_step`, as in probe-count. Model-only, write-only. Conformance projection:
 * see the feature-delta "Pinned conformance projection" (the startup stop is the StopAllocation action of
 * ServiceLifecycleReconciler::reconcile, the restart and give-up are WorkloadLifecycle::reconcile's RestartAllocation and
 * FinalizeFailed; `acted` is ServiceLifecycleView.startup_last_acted_run); row, srow, sseq, sstreak, dl, stable, stopped,
 * crashed, sup, vm are inputs the driver supplies or projects through ObservationStore::alloc_status_row.
 */
module restart_authority_conf {
  type ConfStep =
    | init
    | Deploy({ decl: bool, gl: bool, ok: bool })
    | Crash({ i: int })
    | CleanExit({ i: int })
    | OperatorStop({ i: int })
    | StartupRow({ i: int, res: str })
    | Deadline({ i: int })
    | SLStable({ i: int })
    | SLStartupStop({ i: int, stage: str })
    | SLLivenessStop({ i: int, stage: str })
    | SLFinalizeCrash({ i: int })
    | SLFinalizeStartup({ i: int })
    | SLAnnounceTerminal({ i: int })
    | WLRestart({ ok: bool })
    | WLGiveUp
    | Idle
}

module restart_authority {
  import restart_authority_conf.ConfStep from "./restart_authority"
  const CEIL: int
  const OFF: Set[str]
  const REC: bool
  const GLFREE: bool    // true: the guest-listener flag gl is free at Deploy (ra_conf only); false: gl is pinned false

  val MAXA: int = 2     // startup max_attempts
  val SMAX: int = 3     // bound on startup rows written per attempt
  val IDS: Set[int] = 0.to(CEIL)
  val BUG_GUARD: bool = OFF.contains("guardAcceptsFailed")
  val BUG_SLCRASH: bool = OFF.contains("slFinalisesCrash")
  val BUG_SLSTART: bool = OFF.contains("slFinalisesStartup")
  val BUG_STAMP: bool = OFF.contains("stampAtEmission")
  val BUG_HOOKFIRST: bool = OFF.contains("hookBeforeRowWrite")

  type Att = {
    row: str, sup: bool, srow: str, sseq: int, sstreak: int, dl: bool, stable: bool, ann: bool,
    acted: int, stopped: str, vm: bool, claim: str, author: str, crashed: bool,
  }
  type Ghost = {
    startupOverTerm: bool,   // a startup stop was emitted for a row that was not Running
    emitCount: int,          // startup stops emitted (the named runs pin retries by it)
    stableOverAnn: bool,     // Stable announced for an announced id whose latest startup row is not Pass
  }

  var at: int -> Att
  var decl: bool
  var gl: bool           // the workload's startup descriptor is GuestListeners (D14 item 3): set at Deploy, only with decl
  var cur: int
  var used: int
  var done: bool
  var deployed: bool
  var g: Ghost
  var conf_step: ConfStep

  val blank: Att = {
    row: "none", sup: false, srow: "absent", sseq: 0, sstreak: 0, dl: false, stable: false, ann: false,
    acted: 0, stopped: "none", vm: false, claim: "", author: "", crashed: false,
  }

  action init = all {
    at' = IDS.mapBy(i => blank), decl' = false, gl' = false, cur' = 0, used' = 0, done' = false, deployed' = false,
    g' = { startupOverTerm: false, emitCount: 0, stableOverAnn: false },
    conf_step' = variant("init", ()),
  }

  pure def restartable(row: str): bool =
    Set("failed_crash", "failed_other", "t_startup", "t_liveness", "t_clean").contains(row)

  // D6 item 3 and D7 item 3: the give-up terminal from the final row, in the pinned order liveness, startup, pre-startup
  // crash, otherwise backoff_exhausted
  def classify(a: Att): str =
    if (a.row == "t_liveness") "liveness_probe_failed"
    else if (a.row == "t_startup") "startup_probe_failed"
    else if (decl and a.row == "failed_crash" and a.srow != "pass") "early_exit"
    else "backoff_exhausted"

  // ---------------------------------------------------------------- environment
  action Deploy(d: bool, gd: bool, ok: bool): bool = all {
    not(deployed), gd implies d, GLFREE or not(gd),
    deployed' = true, decl' = d, gl' = gd,
    at' = at.set(0, started(at.get(0), ok)),
    cur' = cur, used' = used, done' = done, g' = g,
    conf_step' = if (REC) variant("Deploy", { decl: d, gl: gd, ok: ok }) else conf_step,
  }

  // DG-2: the start outcome is applied atomically by the start-producing action (the shim's start is awaited inside the
  // tick): ok writes the Running row with the supervisor registered and the VM live; rejected (DriverError::StartRejected, a
  // fail-closed supersession) writes a Failed row with a non-crash reason that never reached Running (started_at: None)
  pure def started(a: Att, ok: bool): Att =
    if (ok) { ...a, row: "running", sup: true, vm: true } else { ...a, row: "failed_other" }

  // the guest crashes (the exit observer writes the Failed row; the D1 hook has stopped the probes)
  action Crash(i: int): bool = all {
    at.get(i).row == "running", at.get(i).vm,
    at' = at.set(i, { ...at.get(i), row: "failed_crash", sup: false, vm: false, crashed: true }),
    decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, deployed' = deployed, g' = g,
    conf_step' = if (REC) variant("Crash", { i: i }) else conf_step,
  }

  // a clean exit: Terminated, Stopped Process
  action CleanExit(i: int): bool = all {
    at.get(i).row == "running", at.get(i).vm,
    at' = at.set(i, { ...at.get(i), row: "t_clean", sup: false, vm: false }),
    decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, deployed' = deployed, g' = g,
    conf_step' = if (REC) variant("CleanExit", { i: i }) else conf_step,
  }

  // an operator stop: the intentional-stop set, never restarted
  action OperatorStop(i: int): bool = all {
    at.get(i).row == "running",
    at' = at.set(i, { ...at.get(i), row: "t_operator", sup: false, vm: false }),
    decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, deployed' = deployed, g' = g,
    conf_step' = if (REC) variant("OperatorStop", { i: i }) else conf_step,
  }

  // the startup probe task writes a row (only while the supervisor is present, the allocation Running and the startup role
  // not yet retired by Stable)
  action StartupRow(i: int, res: str): bool = {
    val a = at.get(i)
    all {
      startupRowEnabled(i, res),
      at' = at.set(i, { ...a, sseq: a.sseq + 1, srow: res,
                        // the producer-counted streak, honest for both kinds; MAXA is the model's cap
                        sstreak: if (res == "fail") (if (a.sstreak < MAXA) a.sstreak + 1 else MAXA) else 0 }),
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, g' = g,
      conf_step' = if (REC) variant("StartupRow", { i: i, res: res }) else conf_step,
    }
  }

  // the startup deadline passes (a started allocation)
  action Deadline(i: int): bool = {
    val a = at.get(i)
    all {
      Set("running", "failed_crash").contains(a.row), not(a.dl),
      at' = at.set(i, { ...a, dl: true }),
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, g' = g,
      conf_step' = if (REC) variant("Deadline", { i: i }) else conf_step,
    }
  }

  // ---------------------------------------------------------------- ServiceLifecycle
  // Each ServiceLifecycle tick scans this workload's full allocation set. D6/DG-12
  // acknowledges every Failed or Terminated fact it sees, even when this tick
  // also announces Stable or a stop for another attempt.
  pure def acknowledgeTerminalFacts(rows: int -> Att): int -> Att =
    IDS.fold(rows, (acc, i) => {
      val a = acc.get(i)
      if (a.row != "none" and a.row != "running") acc.set(i, { ...a, ann: true }) else acc
    })

  // Stable: Running, and (a startup probe passed, or the opt-out first Running). D8 item 3: an announced id whose latest
  // startup row is Pass is evaluated by the Stable branch and leaves terminal_announced.
  action SLStable(i: int): bool = {
    val a = at.get(i)
    all {
      a.row == "running", not(a.stable),
      if (decl) a.srow == "pass" else true,
      (a.ann implies (a.srow == "pass")),
      at' = acknowledgeTerminalFacts(at).set(i, { ...a, stable: true, ann: false }),
      g' = { ...g, stableOverAnn: g.stableOverAnn or (a.ann and a.srow != "pass") },
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done,
      conf_step' = if (REC) variant("SLStable", { i: i }) else conf_step,
    }
  }

  // The three gates (attempts at the maximum, deadline passed, no pass) and the D5 guard
  def startupGates(a: Att): bool =
    decl and a.srow == "fail" and a.sstreak >= (if (gl) 1 else MAXA) and a.dl and not(a.stable)
  def startupGuard(a: Att): bool =
    if (BUG_GUARD) (a.row == "running" or a.row == "failed_crash") else a.row == "running"
  // whether the startup row action is enabled for an attempt (the producer rule for gl, PR-3)
  def startupRowEnabled(i: int, res: str): bool = {
    val a = at.get(i)
    decl and a.row == "running" and a.sup and not(a.stable) and a.sseq < SMAX and (res == "pass" or not(gl) or a.dl) and (res == "fail" or a.vm)   // PR-3: a gl workload writes no Fail before the deadline; RA-4: a dead guest cannot produce a Pass
  }
  def startupStopEnabled(a: Att): bool =
    startupGates(a) and startupGuard(a) and
    (if (BUG_STAMP) not(a.ann) else a.sseq > a.acted) and not(BUG_SLSTART)

  // The post-stop slice of an attempt for a stop stage (B2): landed = Terminated row (t), VM stopped, supervisor removed;
  // pre_hook = unchanged; row_write = VM stopped, row stays Running, supervisor stays (removed in the hazard)
  pure def afterStop(a: Att, stage: str, t: str, hookFirst: bool): Att =
    if (stage == "landed") { ...a, row: t, vm: false, sup: false }
    else if (stage == "row_write") { ...a, vm: false, sup: if (hookFirst) false else a.sup }
    else a

  // D7 item 2 and D8: the startup stop. Emission records the row it acted on and joins terminal_announced; the stop lands
  // (the row becomes Terminated / Stopped StartupProbe, the VM stops, the hook runs after the row write) or fails at a stage
  // (the row stays Running).
  action SLStartupStop(i: int, stage: str): bool = {
    val a = at.get(i)
    all {
      startupStopEnabled(a),
      at' = acknowledgeTerminalFacts(at).set(i, { ...afterStop(a, stage, "t_startup", BUG_HOOKFIRST), acted: a.sseq, ann: true,
                        stopped: if (stage == "landed") "landed" else "failed" }),
      g' = { ...g, startupOverTerm: g.startupOverTerm or a.row != "running",
             emitCount: g.emitCount + 1 },
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done,
      conf_step' = if (REC) variant("SLStartupStop", { i: i, stage: stage }) else conf_step,
    }
  }

  // The liveness stop of a Stable, Running allocation (D9: liveness exists only after Stable; its streak is liveness_gate).
  // Retried by the next liveness row: the row streak is liveness_gate's, so the model re-enables the stop while the row is
  // Running (B2).
  action SLLivenessStop(i: int, stage: str): bool = {
    val a = at.get(i)
    all {
      a.row == "running", a.stable,
      at' = acknowledgeTerminalFacts(at).set(i, afterStop(a, stage, "t_liveness", BUG_HOOKFIRST)),
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, g' = g,
      conf_step' = if (REC) variant("SLLivenessStop", { i: i, stage: stage }) else conf_step,
    }
  }

  // DG-12: every allocation ServiceLifecycle observes in Failed or Terminated joins terminal_announced, whatever its
  // started_at (a started-then-crashed allocation as well as a pre-Running failure)
  action SLAnnounceTerminal(i: int): bool = {
    val a = at.get(i)
    all {
      a.row != "none", a.row != "running", not(a.ann),
      at' = acknowledgeTerminalFacts(at),
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, g' = g,
      conf_step' = if (REC) variant("SLAnnounceTerminal", { i: i }) else conf_step,
    }
  }

  // HAZARD D6: the removed early-exit branch
  action SLFinalizeCrash(i: int): bool = {
    val a = at.get(i)
    all {
      BUG_SLCRASH, decl, a.row == "failed_crash", a.srow != "pass", not(a.dl), a.claim == "",
      at' = at.set(i, { ...a, claim: "early_exit", author: "service_lifecycle" }),
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, g' = g,
      conf_step' = if (REC) variant("SLFinalizeCrash", { i: i }) else conf_step,
    }
  }

  // HAZARD D7: the HEAD live-row finalisation (writes the claim, the VM keeps running)
  action SLFinalizeStartup(i: int): bool = {
    val a = at.get(i)
    all {
      BUG_SLSTART, startupGates(a), a.row == "running", not(a.ann),
      at' = at.set(i, { ...a, row: "failed_other", sup: false, ann: true,
                        claim: "startup_probe_failed", author: "service_lifecycle" }),
      deployed' = deployed, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, g' = g,
      conf_step' = if (REC) variant("SLFinalizeStartup", { i: i }) else conf_step,
    }
  }

  // ---------------------------------------------------------------- WorkloadLifecycle
  val restartEnabled: bool =
    deployed and not(done) and restartable(at.get(cur).row) and used < CEIL and cur < CEIL
  val giveUpEnabled: bool =
    deployed and not(done) and restartable(at.get(cur).row) and used >= CEIL

  // restart every terminal row except the intentional-stop set, below the ceiling, consuming budget; the successor is a
  // fresh id and its predecessor's row is untouched
  action WLRestart(ok: bool): bool = all {
    restartEnabled,
    cur' = cur + 1, used' = used + 1,
    at' = at.set(cur + 1, started(at.get(cur + 1), ok)),
    done' = done, deployed' = deployed, decl' = decl, gl' = gl, g' = g,
    conf_step' = if (REC) variant("WLRestart", { ok: ok }) else conf_step,
  }

  // The give-up at the ceiling: the only author of a failure claim. The
  // FinalizeFailed shim writes a Failed row. It forwards the prior typed
  // reason: a crash stays failed_crash, while a startup/liveness stop or
  // other restartable cause becomes failed_other. `claim` separately records
  // the exact typed terminal, so this row-state projection loses no verdict.
  action WLGiveUp = all {
    giveUpEnabled,
    at' = at.set(cur, { ...at.get(cur),
                       row: if (at.get(cur).row == "failed_crash") "failed_crash" else "failed_other",
                       claim: classify(at.get(cur)), author: "workload_lifecycle" }),
    done' = true,
    cur' = cur, used' = used, deployed' = deployed, decl' = decl, gl' = gl, g' = g,
    conf_step' = if (REC) variant("WLGiveUp", ()) else conf_step,
  }

  action Idle = all {
    at' = at, decl' = decl, gl' = gl, cur' = cur, used' = used, done' = done, deployed' = deployed, g' = g,
    conf_step' = if (REC) variant("Idle", ()) else conf_step,
  }

  action step = {
    nondet i = IDS.oneOf()
    nondet d = Set(true, false).oneOf()
    nondet gd = Set(true, false).oneOf()
    nondet ok = Set(true, false).oneOf()
    nondet res = Set("fail", "pass").oneOf()
    nondet stage = Set("landed", "pre_hook", "row_write").oneOf()
    any {
      Deploy(d, gd, ok), Crash(i), CleanExit(i), OperatorStop(i), StartupRow(i, res), Deadline(i),
      SLStable(i), SLStartupStop(i, stage), SLLivenessStop(i, stage), SLFinalizeCrash(i), SLFinalizeStartup(i), SLAnnounceTerminal(i),
      WLRestart(ok), WLGiveUp, Idle,
    }
  }

  // ------------------------------------------------------------------ invariants
  def claimRows: Set[int] = IDS.filter(i => at.get(i).claim != "")
  // D5 NoStartupVerdictOverCrash: no startup stop (the startup-failure verdict) is decided for an allocation whose row is
  // not Running
  val NoStartupVerdictOverCrash: bool = not(g.startupOverTerm)
  // D6 OneAuthorPerEnding: every failure claim on any row was authored by WorkloadLifecycle
  val OneAuthorPerEnding: bool = IDS.forall(i => at.get(i).claim != "" implies at.get(i).author == "workload_lifecycle")
  // D7 StartupFailureNamedOnlyAtGiveUp: a startup_probe_failed claim exists only as the give-up of the final attempt
  val StartupFailureNamedOnlyAtGiveUp: bool = IDS.forall(i =>
    (at.get(i).claim == "startup_probe_failed") implies
      (at.get(i).author == "workload_lifecycle" and i == cur and done and used >= CEIL))
  // D8 NoStableOverPendingStop: Stable is never announced for an id in terminal_announced unless its latest startup row is
  // Pass, and the two sets stay disjoint
  val NoStableOverPendingStop: bool = and {
    not(g.stableOverAnn),
    IDS.forall(i => (at.get(i).row == "running") implies not(at.get(i).ann and at.get(i).stable)),
  }

  // B2 NoStrandedStop (ADR-0266): a Running row whose VM slot is not live has a supervisor present, so its next probe row
  // re-emits the stop (nothing else converges it)
  val NoStrandedStop: bool = IDS.forall(i => (at.get(i).row == "running" and not(at.get(i).vm)) implies at.get(i).sup)

  val Safety: bool = and {
    NoStartupVerdictOverCrash, OneAuthorPerEnding, StartupFailureNamedOnlyAtGiveUp, NoStableOverPendingStop, NoStrandedStop,
  }

  // ------------------------------------------------------------------ progress (TLC, under fairness)
  temporal fairWL = and { weakFair(WLRestart(true), (at, cur, used, done)), weakFair(WLGiveUp, (at, cur, used, done)) }
  action stopAny(i: int): bool = any { SLStartupStop(i, "landed"), SLStartupStop(i, "pre_hook"), SLStartupStop(i, "row_write") }

  // D5 CrashedAllocEventuallyDecidedByRestartAuthority: a failed allocation is eventually restarted or given up on by
  // WorkloadLifecycle, never decided by the startup verdict
  temporal CrashedAllocEventuallyDecidedByRestartAuthority =
    fairWL.implies(IDS.forall(i => always((Set("failed_crash", "failed_other").contains(at.get(i).row)).implies(
      eventually(cur > i or done)))))

  // D8 FailedStartupStopIsRetried: an id that stays Running with the gates holding, whose last stop failed and that has a
  // newer startup row than the one acted on, is stopped again (or leaves Running)
  def retryDue(i: int): bool = {
    val a = at.get(i)
    a.row == "running" and a.stopped == "failed" and startupGates(a) and a.sseq > a.acted
  }
  temporal FailedStartupStopIsRetried =
    IDS.forall(i => weakFair(stopAny(i), (at, g))).implies(
      IDS.forall(i => always(retryDue(i).implies(eventually(not(retryDue(i)))))))

  // D8 LatePassUnstrandsTheInstance: a Pass recorded after a stop that did not land lets the id reach Stable
  def stranded(i: int): bool = {
    val a = at.get(i)
    a.row == "running" and a.ann and a.srow == "pass" and not(a.stable)
  }
  temporal LatePassUnstrandsTheInstance =
    IDS.forall(i => weakFair(SLStable(i), (at, g))).implies(
      IDS.forall(i => always(stranded(i).implies(eventually(not(stranded(i)))))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // D5: the streak reached the maximum, the allocation crashed, then the deadline passed: no startup verdict;
  // WorkloadLifecycle restarts
  run crash_after_attempts_exhausted_then_deadline_passes = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail"))
    .expect(at.get(0).sstreak == MAXA and at.get(0).sup)
    .then(Crash(0)).then(Deadline(0))
    .expect(at.get(0).sstreak == MAXA and at.get(0).dl and at.get(0).row == "failed_crash"
            and not(startupStopEnabled(at.get(0))) and Safety)
    .then(WLRestart(true)).expect(cur == 1 and used == 1 and at.get(0).row == "failed_crash" and at.get(0).claim == "" and Safety)

  // D5: the live path still produces the startup verdict (now the stop, D7)
  run running_alloc_with_exhausted_probes_past_deadline_is_stopped = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0))
    .expect(startupStopEnabled(at.get(0)))
    .then(SLStartupStop(0, "landed"))
    .expect(at.get(0).row == "t_startup" and not(at.get(0).vm) and not(at.get(0).sup) and at.get(0).claim == "" and Safety)

  // D6: a declared probe, no pass, every attempt crashes: restarts to the ceiling, then give-up early_exit; no
  // ServiceLifecycle claim on any Failed row
  run pre_startup_crash_loop_gives_up_early_exit = init
    .then(Deploy(true, false, true))
    .then(Crash(0)).then(WLRestart(true)).then(Crash(1)).then(WLRestart(true)).then(Crash(2)).then(WLRestart(true)).then(Crash(3)).then(WLRestart(true)).then(Crash(4)).then(WLRestart(true))
    .then(Crash(5))
    .expect(cur == 5 and used == 5 and not(done) and giveUpEnabled and not(restartEnabled) and Safety)
    .then(WLGiveUp)
    .expect(at.get(5).claim == "early_exit" and at.get(5).author == "workload_lifecycle" and done
            and at.get(0).claim == "" and at.get(4).claim == "" and at.get(0).row == "failed_crash" and Safety)

  // D6: the first attempt crashes before a pass, the successor passes and is Stable; the predecessor row stays Failed with
  // its crash and no terminal is written
  run pre_startup_crash_then_successor_passes = init
    .then(Deploy(true, false, true)).then(Crash(0)).then(WLRestart(true))
    .then(StartupRow(1, "pass")).then(SLStable(1))
    .expect(at.get(1).stable and at.get(0).row == "failed_crash" and at.get(0).crashed and at.get(0).claim == ""
            and at.get(1).claim == "" and not(done) and Safety)

  // D6: a crash after a startup pass gives up backoff_exhausted
  run crash_after_startup_pass_gives_up_backoff_exhausted = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "pass")).then(Crash(0)).then(WLRestart(true)).then(StartupRow(1, "pass")).then(Crash(1)).then(WLRestart(true)).then(StartupRow(2, "pass")).then(Crash(2)).then(WLRestart(true)).then(StartupRow(3, "pass")).then(Crash(3)).then(WLRestart(true)).then(StartupRow(4, "pass")).then(Crash(4)).then(WLRestart(true))
    .then(StartupRow(5, "pass")).then(Crash(5)).then(WLGiveUp)
    .expect(at.get(5).claim == "backoff_exhausted" and used == 5 and Safety)

  // D6: an opt-out Service (no startup probe) crash loop gives up backoff_exhausted
  run opt_out_service_crash_loop_gives_up_backoff_exhausted = init
    .then(Deploy(false, false, true)).then(SLStable(0))
    .then(Crash(0)).then(WLRestart(true)).then(Crash(1)).then(WLRestart(true)).then(Crash(2)).then(WLRestart(true)).then(Crash(3)).then(WLRestart(true)).then(Crash(4)).then(WLRestart(true))
    .then(Crash(5)).then(WLGiveUp)
    .expect(at.get(5).claim == "backoff_exhausted" and at.get(0).stable and Safety)

  // D6: a Failed row from a start rejection or fail-closed supersession is never classified
  run non_crash_failed_row_gives_up_backoff_exhausted = init
    .then(Deploy(true, false, true))
    .then(Crash(0)).then(WLRestart(true)).then(Crash(1)).then(WLRestart(true)).then(Crash(2)).then(WLRestart(true)).then(Crash(3)).then(WLRestart(true))
    .then(Crash(4)).then(WLRestart(false))
    .expect(cur == 5 and used == 5 and at.get(5).row == "failed_other" and not(at.get(5).vm) and not(at.get(5).sup) and giveUpEnabled)
    .then(WLGiveUp)
    .expect(at.get(5).row == "failed_other" and at.get(5).claim == "backoff_exhausted" and Safety)

  // D6: a liveness-kill loop gives up liveness_probe_failed (precedence unchanged)
  run liveness_kill_loop_gives_up_liveness_probe_failed = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "pass")).then(SLStable(0)).then(SLLivenessStop(0, "landed")).then(WLRestart(true)).then(StartupRow(1, "pass")).then(SLStable(1)).then(SLLivenessStop(1, "landed")).then(WLRestart(true)).then(StartupRow(2, "pass")).then(SLStable(2)).then(SLLivenessStop(2, "landed")).then(WLRestart(true)).then(StartupRow(3, "pass")).then(SLStable(3)).then(SLLivenessStop(3, "landed")).then(WLRestart(true)).then(StartupRow(4, "pass")).then(SLStable(4)).then(SLLivenessStop(4, "landed")).then(WLRestart(true))
    .then(StartupRow(5, "pass")).then(SLStable(5)).then(SLLivenessStop(5, "landed")).then(WLGiveUp)
    .expect(at.get(5).claim == "liveness_probe_failed" and Safety)

  // D7: the instance stays alive and never passes: a startup stop on every attempt, restarts to the ceiling, then give-up
  // startup_probe_failed; no failure claim on any row before it
  run startup_never_passes_loop_gives_up_startup_probe_failed = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0)).then(SLStartupStop(0, "landed"))
    .expect(at.get(0).claim == "" and at.get(0).row == "t_startup" and Safety)
    .then(WLRestart(true)).then(StartupRow(1, "fail")).then(StartupRow(1, "fail")).then(Deadline(1)).then(SLStartupStop(1, "landed"))
    .expect(at.get(1).claim == "" and not(done) and Safety)
    .then(WLRestart(true)).then(StartupRow(2, "fail")).then(StartupRow(2, "fail")).then(Deadline(2)).then(SLStartupStop(2, "landed"))
    .expect(at.get(2).claim == "" and not(done) and Safety)
    .then(WLRestart(true)).then(StartupRow(3, "fail")).then(StartupRow(3, "fail")).then(Deadline(3)).then(SLStartupStop(3, "landed"))
    .expect(at.get(3).claim == "" and not(done) and Safety)
    .then(WLRestart(true)).then(StartupRow(4, "fail")).then(StartupRow(4, "fail")).then(Deadline(4)).then(SLStartupStop(4, "landed"))
    .expect(at.get(4).claim == "" and not(done) and Safety)
    .then(WLRestart(true)).then(StartupRow(5, "fail")).then(StartupRow(5, "fail")).then(Deadline(5)).then(SLStartupStop(5, "landed"))
    .expect(at.get(5).claim == "" and not(done) and giveUpEnabled and Safety)
    .then(WLGiveUp)
    .expect(at.get(5).claim == "startup_probe_failed" and at.get(0).claim == "" and at.get(4).claim == "" and done and Safety)

  // R-2: the gate reads the LATEST startup row: pass, fail, fail, then the deadline enables the stop although a Pass was
  // recorded earlier
  run pass_then_fails_before_stable_is_stopped = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "pass")).then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0))
    .then(SLStartupStop(0, "landed"))
    .expect(at.get(0).row == "t_startup" and not(at.get(0).stable) and Safety)

  // D7: the first attempt is startup-stopped, the successor passes and is Stable; no startup_probe_failed claim is ever
  // written
  run startup_stop_then_successor_passes_is_stable = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0)).then(SLStartupStop(0, "landed")).then(WLRestart(true))
    .then(StartupRow(1, "pass")).then(SLStable(1))
    .expect(at.get(1).stable and at.get(0).row == "t_startup" and at.get(0).claim == "" and at.get(1).claim == ""
            and not(done) and Safety)

  // D7: last cause wins at the ceiling: attempt one startup-stopped then a final pre-startup crash gives early_exit; the
  // reverse order gives startup_probe_failed
  run startup_stop_then_crash_names_last_cause = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0)).then(SLStartupStop(0, "landed")).then(WLRestart(true))
    .then(Crash(1)).then(WLRestart(true)).then(Crash(2)).then(WLRestart(true)).then(Crash(3)).then(WLRestart(true)).then(Crash(4)).then(WLRestart(true))
    .then(Crash(5)).then(WLGiveUp)
    .expect(at.get(5).claim == "early_exit" and Safety)
  run crash_then_startup_stop_names_last_cause = init
    .then(Deploy(true, false, true))
    .then(Crash(0)).then(WLRestart(true)).then(Crash(1)).then(WLRestart(true)).then(Crash(2)).then(WLRestart(true)).then(Crash(3)).then(WLRestart(true)).then(Crash(4)).then(WLRestart(true))
    .then(StartupRow(5, "fail")).then(StartupRow(5, "fail")).then(Deadline(5)).then(SLStartupStop(5, "landed")).then(WLGiveUp)
    .expect(at.get(5).claim == "startup_probe_failed" and Safety)

  // DG-12: every allocation ServiceLifecycle observes in Failed or Terminated joins terminal_announced, whatever its
  // started_at: a started-then-crashed allocation (it was Stable) and a start-rejected one (never Running)
  run crashed_and_rejected_allocs_join_terminal_announced = init
    .then(Deploy(true, false, true)).then(StartupRow(0, "pass")).then(SLStable(0))
    .expect(not(at.get(0).ann))
    .then(Crash(0)).then(SLAnnounceTerminal(0))
    .expect(at.get(0).ann and at.get(0).crashed and at.get(0).stable and NoStableOverPendingStop and Safety)
    .then(WLRestart(false)).then(SLAnnounceTerminal(1))
    .expect(at.get(1).row == "failed_other" and at.get(1).ann and not(at.get(1).stable) and NoStableOverPendingStop and Safety)

  // D7: a startup-stopped row is restarted; an operator-stopped row beside it is not
  run startup_stop_is_not_intentional_stop = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0)).then(SLStartupStop(0, "landed"))
    .expect(restartEnabled and Safety)
    .then(WLRestart(true)).expect(cur == 1 and used == 1)
    .then(OperatorStop(1))
    .expect(at.get(1).row == "t_operator" and not(restartEnabled) and not(giveUpEnabled) and Safety)

  // D14 item 3 (PR-3): the guest-listener producer (gl) writes no Fail before the deadline: its first Fail row is written at
  // the deadline with no earlier row, is the first row of the attempt (sstreak 1), and alone enables the stop (the gate
  // threshold is 1 for gl). D8 for gl: that stop fails before the hook; a second Fail (sstreak 2, sseq 2) is a newer row and
  // re-emits the stop, which lands (sseq 1 then 2). The successor's Pass is the first row of its attempt (no earlier Fail)
  run first_fail_at_deadline_with_no_earlier_row_stops = init
    .then(Deploy(true, true, true))
    .expect(not(startupRowEnabled(0, "fail")))
    .then(Deadline(0))
    .expect(at.get(0).srow == "absent" and at.get(0).sseq == 0 and not(startupStopEnabled(at.get(0))) and startupRowEnabled(0, "fail"))
    .then(StartupRow(0, "fail"))
    .expect(at.get(0).sseq == 1 and at.get(0).sstreak == 1 and startupStopEnabled(at.get(0)) and Safety)
    .then(SLStartupStop(0, "pre_hook"))
    .expect(at.get(0).row == "running" and at.get(0).acted == 1 and not(startupStopEnabled(at.get(0))))
    .then(StartupRow(0, "fail"))
    .expect(at.get(0).sseq == 2 and at.get(0).sstreak == 2 and startupStopEnabled(at.get(0)) and retryDue(0))
    .then(SLStartupStop(0, "landed"))
    .expect(at.get(0).row == "t_startup" and at.get(0).acted == 2 and g.emitCount == 2 and Safety)
    .then(WLRestart(true))
    .then(StartupRow(1, "pass"))
    .expect(at.get(1).sseq == 1 and at.get(1).srow == "pass" and Safety)
    .then(SLStable(1))
    .expect(at.get(1).stable and at.get(0).claim == "" and not(done) and Safety)

  // D14 item 3 / D15 / G-10 (6): an explicit [[health_check.startup]] dial probe (gl unset) keeps the counted streak: the
  // deadline passes with no earlier row, the first Fail carries sstreak 1 and does not enable the stop, the second carries
  // MAXA and does
  run explicit_probe_fail_after_deadline_keeps_counted_streak = init
    .then(Deploy(true, false, true)).then(Deadline(0))
    .then(StartupRow(0, "fail"))
    .expect(at.get(0).sseq == 1 and at.get(0).sstreak == 1 and not(startupStopEnabled(at.get(0))) and Safety)
    .then(StartupRow(0, "fail"))
    .expect(at.get(0).sseq == 2 and at.get(0).sstreak == MAXA and startupStopEnabled(at.get(0)) and Safety)

  // D8: the stop fails, a new startup row arrives, the second stop is emitted and lands
  run failed_startup_stop_is_retried_on_next_row = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0))
    .then(SLStartupStop(0, "pre_hook"))
    .expect(at.get(0).row == "running" and at.get(0).acted == 2 and at.get(0).stopped == "failed" and at.get(0).ann
            and not(startupStopEnabled(at.get(0))) and Safety)
    .then(StartupRow(0, "fail"))
    .expect(startupStopEnabled(at.get(0)) and retryDue(0))
    .then(SLStartupStop(0, "landed"))
    .expect(at.get(0).row == "t_startup" and at.get(0).acted == 3 and g.emitCount == 2 and Safety)

  // D8: the stop fails, a Pass row arrives: no further stop; the id reaches Stable and leaves terminal_announced
  run late_pass_after_failed_stop_reaches_stable = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0))
    .then(SLStartupStop(0, "pre_hook"))
    .then(StartupRow(0, "pass"))
    .expect(not(startupStopEnabled(at.get(0))) and stranded(0))
    .then(SLStable(0))
    .expect(at.get(0).stable and not(at.get(0).ann) and at.get(0).row == "running" and g.emitCount == 1 and Safety)

  // B2 (ADR-0266): the stop's row write fails after the effects ran: the VM is stopped, the row stays Running, the supervisor
  // stays registered (the hook has not run), the next startup row re-emits the stop and the retry lands. The same for a
  // liveness stop.
  run stop_row_write_failure_is_retried = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0))
    .then(SLStartupStop(0, "row_write"))
    .expect(at.get(0).row == "running" and not(at.get(0).vm) and at.get(0).sup and at.get(0).stopped == "failed"
            and at.get(0).acted == 2 and not(startupStopEnabled(at.get(0))) and NoStrandedStop and Safety)
    .then(StartupRow(0, "fail"))
    .expect(startupStopEnabled(at.get(0)) and retryDue(0))
    .then(SLStartupStop(0, "landed"))
    .expect(at.get(0).row == "t_startup" and not(at.get(0).vm) and not(at.get(0).sup) and g.emitCount == 2 and Safety)
    .then(WLRestart(true)).then(StartupRow(1, "pass")).then(SLStable(1))
    .then(SLLivenessStop(1, "row_write"))
    .expect(at.get(1).row == "running" and not(at.get(1).vm) and at.get(1).sup and at.get(1).stable and NoStrandedStop and Safety)
    .then(SLLivenessStop(1, "landed"))
    .expect(at.get(1).row == "t_liveness" and not(at.get(1).vm) and not(at.get(1).sup) and Safety)

  // D8: the recorded acted run lives in the persisted View, so a control-plane restart (a View reload) keeps it and the
  // same row is not re-stopped. The model's `acted` is the View field and is not reset by Idle (a reload is a no-op on
  // persisted state); the run pins that nothing but a newer row re-enables the stop.
  run control_plane_restart_keeps_acted_run = init
    .then(Deploy(true, false, true))
    .then(StartupRow(0, "fail")).then(StartupRow(0, "fail")).then(Deadline(0))
    .then(SLStartupStop(0, "pre_hook"))
    .then(Idle)
    .expect(at.get(0).acted == 2 and not(startupStopEnabled(at.get(0))))
    .then(StartupRow(0, "fail"))
    .expect(at.get(0).acted == 2 and startupStopEnabled(at.get(0)) and g.emitCount == 1)
}

// Main instance for the checks: two attempts (CEIL = 1): the restart, the give-up and a successor are all reached.
module ra_ok { import restart_authority(CEIL = 1, OFF = Set(), REC = false, GLFREE = false).* from "./restart_authority" }
// Conformance main: the production ceiling (RESTART_BACKOFF_CEILING = 5: restart iff attempts < 5, so 5 restarts and six
// attempts, ids 0..5). Records conf_step; only the named runs are checked on it.
module ra_conf { import restart_authority(CEIL = 5, OFF = Set(), REC = true, GLFREE = true).* from "./restart_authority" }
