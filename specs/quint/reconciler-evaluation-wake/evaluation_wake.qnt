// -*- mode: Bluespec; -*-
/**
 * Reconciler evaluation wake (reconciler-convergence-guarantees): D-1 / ADR-0282 (every reconciler declares its
 * managed targets; the runtime re-evaluates them every period) and D-3 (ServiceMapHydrator::next_evaluation_at).
 *
 * ABSTRACTION. Three evaluation keys (reconciler, target):
 *   k1 = (rA, t1)   k2 = (rH, t1)   k3 = (rA, t2)
 * rA is a generic reconciler. rH is the hydrator-shaped reconciler: the only one that paces retries through a
 * View backoff window and implements next_evaluation_at (D-3). k1 and k2 share the target t1, so the loop's
 * per-TARGET exclusion (`active_targets` is a BTreeSet<TargetResource>, lib.rs:5165; drain_pending blocks on
 * `key.1`, eval_broker.rs:129-170) is exercised. World per key: `has` (the key has desired or actual state, built
 * from intent AND observation, so `has` IS the managed set per reconciler) and `need` (desired != actual).
 *
 * Broker (eval_broker.rs): `pending` is a FIFO list of unique keys (first-pending order is retained on a same-key
 * resubmit); `elig` is "imm" | "nb" (NotBefore) per key. Submit at a pending key collapses (cancelled++); an
 * Immediate submit merges a pending NotBefore to Immediate (merge_eligibility); a NotBefore submit leaves it.
 * `Drain` is the loop's admission: oldest first, at most CAP - |active| keys, skipping keys whose TARGET is already
 * active (or taken earlier in the same drain) and keys that are not Immediate-eligible; the whole batch is atomic
 * (the drain holds the broker lock). CAP = 2 stands for CONVERGENCE_MAX_IN_FLIGHT = 8.
 *
 * Evaluation = Drain (admission only: drain_pending, no hydration) then Finish(k, outcome) = run_convergence_tick
 * (reconciler_runtime.rs:1580-1750), which hydrates at its start, so the world is read at Finish, not at Drain
 * (D-12 d: a world change between Drain and Finish is seen by the tick):
 *   ok          reconcile emitted an action iff the key is needy now and the key is not inside its backoff window: the effect
 *               lands (need cleared) and the runtime self-requeues ONE Immediate evaluation (has_action). Inside the
 *               window (hydrator only) the tick emits Noop and consults next_evaluation_at: Some -> a NotBefore
 *               requeue (D-3, switched off by OFF "noRetention"). Otherwise Noop, no requeue.
 *   dpFail      the action was dispatched, the dataplane executor recorded a Failed result (the tick itself is ok):
 *               the effect did not land, the View records the attempt (the key enters its backoff window), the
 *               has_action self-requeue is Immediate.
 *   hydrateErr  `?` on a hydrate call: no View write, no dispatch, no requeue.
 *   persistErr  `?` on persist_view: no dispatch, no requeue (View not written).
 *   dispatchErr the shim returned Err: the View was persisted, the effect did not land; the has_action self-requeue
 *               runs on all paths (reconciler_runtime.rs:1739), so it is Immediate.
 * Error outcomes are only offered for a key that was needy when the tick hydrates (an error on a key with nothing to do
 * changes nothing the model observes).
 *
 * Resync pass (D-1b): `due` is the period timer (Tick sets it; the first pass runs at boot, so init has due). A pass
 * starts only when due, clears it, and walks the registered reconcilers in BTreeMap order (rA, rH), one PassStep per
 * reconciler: managed_targets either returns the whole set (every `has` key of that reconciler, submitted
 * Immediate through the broker) or Err for the whole call (nothing submitted, the pass continues with the next
 * reconciler). The timer re-arms while a pass runs, so the pass is at most once per period and anchored (no
 * drift); see ASSUMPTION WK-3 for what the model does not carry.
 *
 * Environment faults (all budgeted by `faults < MAXF`, kinds enabled by FAULTS): hydrateErr, persistErr,
 * dispatchErr, dpFail, enumFail (managed_targets Err for one reconciler), crash (process restart between View
 * fsync and dispatch: the in-memory broker and the in-flight set are lost, the View and the world persist, the
 * boot pass is due again). Dropped edge wake: DivergeDrop is a world change whose edge wake is lost (DROPS).
 * Drift is budgeted (MAXD). Spurious/edge wakes (EdgeWake) are unbudgeted.
 *
 * OFF names the decisions switched off (hazards):
 *   noPass       D-1  the resync pass is removed (PassStep disabled, so no boot pass either)
 *   noRetention  D-3  next_evaluation_at returns None (an in-window Noop is not requeued)
 *
 * Model-only (conformance): see README (pinned projection table); `conf_step` records the action (D-UP-1).
 * dispatched / cancelled count Drain admissions and same-key collapses only when REC (the conformance main);
 * the checking instances keep them 0 so TLC's state space is finite.
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   WK-1  The broker's LWW collapse, eligibility merge, first-pending FIFO order and per-target blocking are as
 *         eval_broker.rs `submit` / `drain_pending` / `merge_eligibility` (read at HEAD fb71b400f).
 *         Discharged by: V-4 (existing eval_broker tests + run edge_wake_during_burst_is_admitted_after_burst and
 *         pass_submit_merges_retained_not_before_to_immediate under the quint-connect driver).
 *   WK-2  ReconcilerRuntime::submit_evaluation wakes the loop (ADR-0273), so a submitted key is drained without a
 *         separate sleep-until wake; the loop's cadence timer backstops it.
 *         Discharged by: the existing ADR-0273 submit-wakes-loop acceptance tests.
 *   WK-3  Time is abstract: the period is a timer event (Tick) with weak fairness, a backoff window ends by a
 *         WindowEnds event with weak fairness, and a retained NotBefore becomes eligible exactly when its window
 *         ends (the deadline IS last_failure_seen_at + backoff_for_attempt, D-3). The 30 s value, "within one
 *         period plus queue time" as a duration, and anchored re-arming at previous + period on the injected clock
 *         are NOT checked here; they are checked by the seeded overdrive-sim scenario on SimClock (V-3).
 *   WK-4  managed_targets returns the complete set or Err for the whole call, from intent and from observation
 *         (D-1a postcondition); `has` is that set. Discharged by: the managed_targets contract tests per
 *         reconciler (DISTILL) and V-5.
 *   WK-5  A crash loses exactly the in-memory broker queue and in-flight evaluations; the View store, intent and
 *         observation rows survive (redb fsync, ADR-0035 step 7 -> 8). Discharged by: the existing WriteThrough
 *         ordering invariant and a DST crash scenario.
 *   WK-6  The only pacing View the model needs is the hydrator's backoff window; other reconcilers' View pacing
 *         (restart budgets, attempts) never stops a due key from being re-evaluated by a pass (they only turn an
 *         evaluation into a Noop). Discharged by: slice 04 property (actual == desired within N passes).
 */
module reconciler_wake_conf {
  type ConfStep =
    | init
    | Tick
    | PassStep({ enumOk: bool })
    | Drain
    | Finish({ k: str, o: str })
    | DivergeWake({ k: str })
    | DivergeDrop({ k: str })
    | EdgeWake({ k: str })
    | WindowEnds({ k: str })
    | Due({ k: str })
    | Crash
}

module evaluation_wake {
  import reconciler_wake_conf.ConfStep from "./evaluation_wake"
  const OFF: Set[str]
  const FAULTS: Set[str]
  const DROPS: bool
  const REC: bool

  val CAP: int = 2
  val MAXF: int = 2
  val MAXD: int = 2
  val KEYS: List[str] = ["k1", "k2", "k3"]
  val KEYSET: Set[str] = Set("k1", "k2", "k3")
  val RECS: List[str] = ["rA", "rH"]
  val NREC: int = 2
  val RECOF: str -> str = Map("k1" -> "rA", "k2" -> "rH", "k3" -> "rA")
  val TGT: str -> str = Map("k1" -> "t1", "k2" -> "t1", "k3" -> "t2")
  val ALLIMM: str -> str = Map("k1" -> "imm", "k2" -> "imm", "k3" -> "imm")
  val NO_PASS: bool = OFF.contains("noPass")
  val NO_RET: bool = OFF.contains("noRetention")

  pure def isH(k: str): bool = k == "k2"
  pure def inP(p: List[str], k: str): bool = p.indices().exists(i => p[i] == k)
  pure def subP(p: List[str], k: str): List[str] = if (inP(p, k)) p else p.append(k)
  pure def subE(p: List[str], e: str -> str, k: str, kind: str): str -> str =
    if (inP(p, k)) (if (kind == "imm") e.set(k, "imm") else e) else e.set(k, kind)
  pure def subMany(p: List[str], e: str -> str, ks: List[str]): (List[str], str -> str) =
    ks.foldl((p, e), (acc, k) => (subP(acc._1, k), subE(acc._1, acc._2, k, "imm")))

  // world
  var has: Set[str]       // managed set (intent U observation): keys with desired or actual state
  var need: Set[str]      // desired != actual
  var inWin: Set[str]     // hydrator View: key is inside its backoff window after a failed dataplane result
  // loop
  var active: Set[str]    // in-flight evaluations
  // broker
  var pending: List[str]  // FIFO, unique
  var elig: str -> str    // "imm" | "nb"; "imm" for a key that is not pending
  // resync driver
  var due: bool           // period timer fired and no pass has started for it
  var passPos: int        // reconcilers already walked by the pass in progress (0: idle)
  // budgets / counters
  var faults: int
  var drifts: int
  var dispatched: int     // REC only
  var cancelled: int      // REC only
  var conf_step: ConfStep // D-UP-1: the action that produced this state (model-only)

  def canFault(f: str): bool = FAULTS.contains(f) and faults < MAXF

  action init = all {
    has' = Set(), need' = Set(), inWin' = Set(), active' = Set(),
    pending' = List(), elig' = ALLIMM, due' = true, passPos' = 0, faults' = 0, drifts' = 0,
    dispatched' = 0, cancelled' = 0, conf_step' = variant("init", ()),
  }

  // the period timer fires
  action Tick = all {
    not(due),
    due' = true,
    has' = has, need' = need, inWin' = inWin, active' = active, pending' = pending, elig' = elig,
    passPos' = passPos, faults' = faults, drifts' = drifts, dispatched' = dispatched, cancelled' = cancelled,
    conf_step' = if (REC) variant("Tick", ()) else conf_step,
  }

  // D-1: one reconciler's step of the resync pass
  action PassStep(enumOk: bool): bool = {
    val r = RECS[passPos]
    val ks = KEYS.select(k => RECOF.get(k) == r and has.contains(k))
    val sub = subMany(pending, elig, ks)
    all {
      not(NO_PASS),
      passPos < NREC,
      passPos > 0 or due,
      enumOk or canFault("enumFail"),
      pending' = if (enumOk) sub._1 else pending,
      elig' = if (enumOk) sub._2 else elig,
      cancelled' = if (REC and enumOk) cancelled + ks.select(k => inP(pending, k)).length() else cancelled,
      faults' = if (enumOk) faults else faults + 1,
      passPos' = if (passPos + 1 == NREC) 0 else passPos + 1,
      due' = if (passPos == 0) false else due,
      has' = has, need' = need, inWin' = inWin, active' = active, drifts' = drifts,
      dispatched' = dispatched,
      conf_step' = if (REC) variant("PassStep", { enumOk: enumOk }) else conf_step,
    }
  }

  // the loop's admission (drain_pending): oldest first, capacity, per-target exclusion, Immediate only
  val drainSel: List[str] = {
    val limit = CAP - active.size()
    pending.foldl({ adm: List(), tg: active.map(a => TGT.get(a)) }, (acc, k) =>
      if (acc.adm.length() >= limit or acc.tg.contains(TGT.get(k)) or elig.get(k) != "imm") acc
      else { adm: acc.adm.append(k), tg: acc.tg.union(Set(TGT.get(k))) }).adm
  }

  action Drain = {
    val sel = drainSel
    val s = sel.foldl(Set(), (a, k) => a.union(Set(k)))
    all {
      sel.length() > 0,
      pending' = pending.select(k => not(s.contains(k))),
      elig' = KEYSET.mapBy(k => if (s.contains(k)) "imm" else elig.get(k)),
      active' = active.union(s),
      dispatched' = if (REC) dispatched + sel.length() else dispatched,
      has' = has, need' = need, inWin' = inWin, due' = due, passPos' = passPos, faults' = faults,
      drifts' = drifts, cancelled' = cancelled,
      conf_step' = if (REC) variant("Drain", ()) else conf_step,
    }
  }

  // the end of one evaluation (run_convergence_tick)
  action Finish(k: str, o: str): bool = {
    val sn = need.contains(k)   // hydrated now: the world at the start of run_convergence_tick
    val win = inWin.contains(k)
    val acts = sn and not(win)
    val retain = sn and win and isH(k) and not(NO_RET)
    val kind =
      if (o == "ok") (if (acts) "imm" else if (retain) "nb" else "none")
      else if (o == "dpFail" or o == "dispatchErr") "imm"
      else "none"
    val allowed =
      if (o == "ok") true
      else if (o == "dpFail") (isH(k) and acts and canFault("dpFail"))
      else if (o == "dispatchErr") (acts and canFault("dispatchErr"))
      else if (o == "hydrateErr" or o == "persistErr") (sn and canFault(o))
      else false
    all {
      active.contains(k),
      allowed,
      need' = if (o == "ok" and acts) need.exclude(Set(k)) else need,
      inWin' = if (o == "dpFail") inWin.union(Set(k)) else inWin,
      active' = active.exclude(Set(k)),
      pending' = if (kind == "none") pending else subP(pending, k),
      elig' = if (kind == "none") elig else subE(pending, elig, k, kind),
      cancelled' = if (REC and kind != "none" and inP(pending, k)) cancelled + 1 else cancelled,
      faults' = if (o == "ok") faults else faults + 1,
      has' = has, due' = due, passPos' = passPos, drifts' = drifts, dispatched' = dispatched,
      conf_step' = if (REC) variant("Finish", { k: k, o: o }) else conf_step,
    }
  }

  // a world change with its edge wake (interests() / client request / EnqueueEvaluation)
  action DivergeWake(k: str): bool = all {
    drifts < MAXD,
    not(need.contains(k)),
    has' = has.union(Set(k)),
    need' = need.union(Set(k)),
    pending' = subP(pending, k),
    elig' = subE(pending, elig, k, "imm"),
    drifts' = drifts + 1,
    cancelled' = if (REC and inP(pending, k)) cancelled + 1 else cancelled,
    inWin' = inWin, active' = active, due' = due, passPos' = passPos, faults' = faults,
    dispatched' = dispatched,
    conf_step' = if (REC) variant("DivergeWake", { k: k }) else conf_step,
  }

  // environment fault: the same world change whose edge wake is dropped (or never existed: a fresh deploy has
  // intent and no alloc_status row)
  action DivergeDrop(k: str): bool = all {
    DROPS,
    drifts < MAXD,
    not(need.contains(k)),
    has' = has.union(Set(k)),
    need' = need.union(Set(k)),
    drifts' = drifts + 1,
    inWin' = inWin, active' = active, pending' = pending, elig' = elig, due' = due,
    passPos' = passPos, faults' = faults, dispatched' = dispatched, cancelled' = cancelled,
    conf_step' = if (REC) variant("DivergeDrop", { k: k }) else conf_step,
  }

  // an edge wake with no world change (accepted row, EnqueueEvaluation, spurious)
  action EdgeWake(k: str): bool = all {
    pending' = subP(pending, k),
    elig' = subE(pending, elig, k, "imm"),
    cancelled' = if (REC and inP(pending, k)) cancelled + 1 else cancelled,
    has' = has, need' = need, inWin' = inWin, active' = active, due' = due, passPos' = passPos,
    faults' = faults, drifts' = drifts, dispatched' = dispatched,
    conf_step' = if (REC) variant("EdgeWake", { k: k }) else conf_step,
  }

  // time passes: the hydrator's backoff window of k ends
  action WindowEnds(k: str): bool = all {
    inWin.contains(k),
    inWin' = inWin.exclude(Set(k)),
    has' = has, need' = need, active' = active, pending' = pending, elig' = elig, due' = due,
    passPos' = passPos, faults' = faults, drifts' = drifts, dispatched' = dispatched, cancelled' = cancelled,
    conf_step' = if (REC) variant("WindowEnds", { k: k }) else conf_step,
  }

  // time passes: a retained NotBefore evaluation reaches its deadline (= the end of its window, WK-3)
  action Due(k: str): bool = all {
    inP(pending, k),
    elig.get(k) == "nb",
    not(inWin.contains(k)),
    elig' = elig.set(k, "imm"),
    has' = has, need' = need, inWin' = inWin, active' = active, pending' = pending, due' = due,
    passPos' = passPos, faults' = faults, drifts' = drifts, dispatched' = dispatched, cancelled' = cancelled,
    conf_step' = if (REC) variant("Due", { k: k }) else conf_step,
  }

  // environment fault: process restart between View fsync and dispatch; the boot pass is due again
  action Crash = all {
    canFault("crash"),
    pending' = List(), elig' = ALLIMM, active' = Set(),
    passPos' = 0, due' = true, faults' = faults + 1,
    has' = has, need' = need, inWin' = inWin, drifts' = drifts, dispatched' = dispatched, cancelled' = cancelled,
    conf_step' = if (REC) variant("Crash", ()) else conf_step,
  }

  action step = {
    nondet k = KEYSET.oneOf()
    nondet o = Set("ok", "dpFail", "hydrateErr", "persistErr", "dispatchErr").oneOf()
    nondet b = Set(true, false).oneOf()
    any { Tick, PassStep(b), Drain, Finish(k, o), DivergeWake(k), DivergeDrop(k), EdgeWake(k), WindowEnds(k),
          Due(k), Crash }
  }

  val allVars = (has, need, inWin, active, pending, elig, due, passPos, faults, drifts, dispatched,
                 cancelled, conf_step)

  // ------------------------------------------------------------------ safety
  // D-1 / bounded in-flight capacity
  val ActiveWithinCap: bool = active.size() <= CAP
  // D-1 / one active evaluation per TARGET (not per reconciler): two reconcilers on one target never overlap
  val OneActivePerTarget: bool = active.map(a => TGT.get(a)).size() == active.size()
  // broker: a pending key is unique (same-key collapse)
  val PendingUnique: bool = pending.indices().forall(i => pending.indices().forall(j => pending[i] == pending[j] implies i == j))
  // broker: eligibility is "imm" for any key that is not pending
  val EligCanonical: bool = KEYSET.forall(k => not(inP(pending, k)) implies elig.get(k) == "imm")
  // world: a needy key is managed (the enumeration postcondition has something to find)
  val NeedImpliesManaged: bool = need.subseteq(has)
  // D-3: the hydrator window only exists for a needy key
  val WindowOnlyForNeedy: bool = inWin.subseteq(need)
  val Safety: bool = and { ActiveWithinCap, OneActivePerTarget, PendingUnique, EligCanonical, NeedImpliesManaged,
                           WindowOnlyForNeedy }
  // D-3: a key inside its backoff window is always scheduled (in flight, or retained in the broker): the
  // re-evaluation does not depend on a row write or a pass. Holds when the only fault is the dataplane failing
  // (instance wake_retention); a hydrate/persist error or a crash legitimately ends the schedule (the pass retries)
  val RetentionSchedulesBackedOffKey: bool = inWin.forall(k => active.contains(k) or inP(pending, k))

  // ------------------------------------------------------------------ progress (temporal)
  temporal Fair = and {
    weakFair(Tick, allVars), weakFair(PassStep(true), allVars), weakFair(Drain, allVars),
    weakFair(Finish("k1", "ok"), allVars), weakFair(Finish("k2", "ok"), allVars), weakFair(Finish("k3", "ok"), allVars),
    weakFair(WindowEnds("k1"), allVars), weakFair(WindowEnds("k2"), allVars), weakFair(WindowEnds("k3"), allVars),
    weakFair(Due("k1"), allVars), weakFair(Due("k2"), allVars), weakFair(Due("k3"), allVars),
  }
  // D-1: every needy key is eventually evaluated to convergence, from every error outcome, dropped wake and crash
  temporal ProgressByPass = Fair.implies(KEYSET.forall(k => always(need.contains(k).implies(eventually(not(need.contains(k)))))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // happy path: an edge wake alone converges the key; the confirming requeue is a Noop
  run healthy_edge_converges = init
    .then(DivergeWake("k1")).expect(need == Set("k1") and pending == ["k1"])
    .then(Drain).expect(active == Set("k1") and pending == List())
    .then(Finish("k1", "ok")).expect(need == Set() and pending == ["k1"] and active == Set())
    .then(Drain).then(Finish("k1", "ok"))
    .expect(need == Set() and pending == List() and active == Set() and Safety)

  // slice 01 fault 1: a hydrate error ends the evaluation with nothing scheduled; the (boot) pass is the retry
  run hydrate_error_recovers_by_pass = init
    .then(DivergeWake("k1")).then(Drain)
    .then(Finish("k1", "hydrateErr")).expect(need == Set("k1") and pending == List() and active == Set() and faults == 1)
    .then(PassStep(true)).expect(pending == ["k1"] and due == false and passPos == 1)
    .then(PassStep(true)).expect(passPos == 0)
    .then(Drain).then(Finish("k1", "ok")).expect(need == Set() and Safety)

  // slice 01 fault 2: a View persist error, same recovery
  run persist_error_recovers_by_pass = init
    .then(DivergeWake("k1")).then(Drain)
    .then(Finish("k1", "persistErr")).expect(need == Set("k1") and pending == List() and active == Set())
    .then(PassStep(true)).then(PassStep(true))
    .then(Drain).then(Finish("k1", "ok")).expect(need == Set() and Safety)

  // environment fault: crash between View fsync and dispatch; the in-memory broker is lost, the boot pass retries
  run crash_between_fsync_and_dispatch_recovers_by_boot_pass = init
    .then(DivergeWake("k1")).then(Drain)
    .then(Crash).expect(need == Set("k1") and pending == List() and active == Set() and due == true)
    .then(PassStep(true)).then(PassStep(true))
    .expect(pending == ["k1"])
    .then(Drain).then(Finish("k1", "ok")).expect(need == Set() and Safety)

  // slice 01 fault 3 (D-3): the dataplane update fails; the confirming evaluation falls inside the backoff window,
  // emits Noop and its deadline is retained as NotBefore; the retry happens when the window ends, with no pass
  // (due stays true, passPos 0: the pass never ran) and no edge wake
  run failed_dataplane_inside_backoff_recovers_without_row_write = init
    .then(DivergeWake("k2")).then(Drain)
    .then(Finish("k2", "dpFail")).expect(inWin == Set("k2") and need == Set("k2") and pending == ["k2"] and elig.get("k2") == "imm")
    .then(Drain).then(Finish("k2", "ok"))
    .expect(need == Set("k2") and pending == ["k2"] and elig.get("k2") == "nb" and RetentionSchedulesBackedOffKey)
    .then(WindowEnds("k2")).then(Due("k2")).expect(elig.get("k2") == "imm")
    .then(Drain).then(Finish("k2", "ok"))
    .expect(need == Set() and due == true and passPos == 0 and Safety)

  // slice 01 fresh deploy: intent exists, no row, the edge wake never happens; the pass evaluates it
  run fresh_deploy_without_row_is_evaluated = init
    .then(DivergeDrop("k1")).expect(need == Set("k1") and has == Set("k1") and pending == List())
    .then(PassStep(true)).expect(pending == ["k1"])
    .then(PassStep(true))
    .then(Drain).then(Finish("k1", "ok")).expect(need == Set() and Safety)

  // enumeration failure of rA: nothing of rA's set is submitted (no partial set), rH is still enumerated and
  // evaluated; the next pass retries rA
  run enumeration_failure_of_one_reconciler_does_not_starve_others = init
    .then(DivergeDrop("k1")).then(DivergeDrop("k2"))
    .then(PassStep(false)).expect(pending == List() and faults == 1 and passPos == 1)
    .then(PassStep(true)).expect(pending == ["k2"] and passPos == 0)
    .then(Drain).then(Finish("k2", "ok")).expect(need == Set("k1"))
    .then(Drain).then(Finish("k2", "ok")).expect(pending == List() and need == Set("k1"))
    .then(Tick).then(PassStep(true)).expect(pending == ["k1"])
    .then(PassStep(true)).expect(pending == ["k1", "k2"])
    .then(Drain).expect(active == Set("k1") and pending == ["k2"])
    .then(Finish("k1", "ok")).expect(need == Set() and Safety)

  // a pass burst fills the capacity; an edge wake submitted during the burst queues behind it and is admitted
  // after it, before the confirming requeues the burst's own evaluations produce
  run edge_wake_during_burst_is_admitted_after_burst = init
    .then(DivergeDrop("k1")).then(DivergeDrop("k3"))
    .then(PassStep(true)).expect(pending == ["k1", "k3"])
    .then(EdgeWake("k2")).expect(pending == ["k1", "k3", "k2"])
    .then(Drain).expect(active == Set("k1", "k3") and pending == ["k2"])
    .then(Finish("k1", "ok")).expect(need == Set("k3") and pending == ["k2", "k1"])
    .then(Drain).expect(active == Set("k3", "k2") and pending == ["k1"] and Safety)
    .then(Finish("k3", "ok")).then(Finish("k2", "ok")).expect(need == Set() and Safety)

  // V-4: a pass submit at a key holding a retained NotBefore merges it to Immediate, keeping the queue slot
  run pass_submit_merges_retained_not_before_to_immediate = init
    .then(DivergeWake("k2")).then(Drain).then(Finish("k2", "dpFail"))
    .then(Drain).then(Finish("k2", "ok")).expect(elig.get("k2") == "nb" and pending == ["k2"])
    .then(PassStep(true)).then(PassStep(true))
    .expect(elig.get("k2") == "imm" and pending == ["k2"] and inWin == Set("k2"))

  // D-12 d: the world is read at Finish (run_convergence_tick hydrates at its start), not at Drain. k1 is admitted
  // while it does not need work; its divergence arrives (edge wake dropped) while it is in flight; Finish acts on
  // the world as read then (the effect lands, one confirming requeue), and that confirming evaluation is a Noop
  run world_changes_between_drain_and_finish = init
    .then(EdgeWake("k1")).then(Drain)
    .expect(active == Set("k1") and need == Set() and pending == List())
    .then(DivergeDrop("k1")).expect(need == Set("k1") and active == Set("k1") and pending == List())
    .then(Finish("k1", "ok")).expect(need == Set() and pending == ["k1"] and active == Set())
    .then(Drain).then(Finish("k1", "ok"))
    .expect(need == Set() and pending == List() and active == Set() and Safety)
}

// Main: every decision on, every fault kind, dropped wakes. Safety and progress hold.
module wake_full {
  import evaluation_wake(OFF = Set(), FAULTS = Set("hydrateErr", "persistErr", "dispatchErr", "dpFail", "enumFail", "crash"), DROPS = true, REC = false).* from "./evaluation_wake"
}
// D-3 instance: the only fault is the dataplane failing; the pass is not needed for the backoff schedule.
module wake_retention {
  import evaluation_wake(OFF = Set(), FAULTS = Set("dpFail"), DROPS = true, REC = false).* from "./evaluation_wake"
}
// Conformance main: records conf_step, the broker counters and every fault; only the named runs are checked on it.
module wake_conf {
  import evaluation_wake(OFF = Set(), FAULTS = Set("hydrateErr", "persistErr", "dispatchErr", "dpFail", "enumFail", "crash"), DROPS = true, REC = true).* from "./evaluation_wake"
}
