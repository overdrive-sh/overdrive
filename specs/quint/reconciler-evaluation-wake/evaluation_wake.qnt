// -*- mode: Bluespec; -*-
/**
 * Reconciler evaluation wake (reconciler-convergence-guarantees): D-1 / ADR-0296 (every reconciler declares its
 * managed targets; the runtime re-evaluates them every period) and D-3 (ServiceMapHydrator::next_evaluation_at).
 *
 * ABSTRACTION. Four evaluation keys (reconciler, target) of ONE shared workload (a Service with a listener):
 *   k1 = (r4W, t1) ~ (workload-lifecycle,       workload/<id>)
 *   k3 = (r1S, t1) ~ (service-lifecycle,        workload/<id>)
 *   k4 = (r3V, t1) ~ (svid-lifecycle,           workload/<id>)
 *   k2 = (r2M, t2) ~ (service-map-hydrator,     service/<id> of that workload's own listener)
 * The abstract reconciler names sort in the same order as the production names they stand for (r1S < r2M < r3V < r4W, as
 * service-lifecycle < service-map-hydrator < svid-lifecycle < workload-lifecycle), so the resync pass's registered
 * (BTreeMap) order is r1S, r2M, r3V, r4W. r4W, r1S and r3V are generic workload-shaped reconcilers sharing the target t1.
 * r2M is the hydrator-shaped reconciler (alone on its target): it paces retries through a View backoff window and implements
 * next_evaluation_at (D-3). r3V (svid-lifecycle) has a backoff window and next_evaluation_at too (D11), existing behaviour. k1, k3 and k4 share t1, so the loop's per-TARGET exclusion (`active_targets` is
 * a BTreeSet<TargetResource>, lib.rs:5165; drain_pending blocks on `key.1`, eval_broker.rs:129-170) is exercised. World per
 * key: `has` (the key has desired or actual state, built from intent AND observation, so `has` IS the managed set per
 * reconciler) and `need` (desired != actual).
 *
 * Managed-set coupling (SharedIntentImpliesBase; has(k1), has(k2), has(k3) only grow, dropping the intent is not modelled):
 * has(k3) and has(k4) imply has(k1), has(k4) implies has(k3), has(k2) implies has(k3), and an assigned VIP implies has(k2)
 * (D-21: the hydrator manages a Service whenever its intent has an assigned VIP, service_map_hydrator.rs managed_targets). A
 * workload INTENT alone makes only k1 managed. service-lifecycle manages a Service once it has an allocation row or an assigned
 * VIP (not on intent alone), so an allocation row makes it manage the target: has(k4) implies has(k3) (D7; the managed-set rule;
 * managed_targets is a DISTILL scaffold in the tree, conformance-driver.md "World construction"). has(k3) arises from a k3 world
 * change (VIP assigned), from a k4 world change (an allocation row), from a k2 world change (its published row implies a VIP),
 * or from a landed k1 start (an allocation exists). has(k2) arises from every step that makes vip true (a k3 or k2 world change)
 * and from a landed k3 publish.
 * has(k4) is NOT monotone (D-1a; svid_lifecycle.rs managed_targets): svid-lifecycle manages the target iff the workload has a
 * Running allocation or a held SVID, so has(k4) == (running or held) in every state (K4ManagedIffRunningOrHeld). It leaves the
 * managed set when a k1 world change takes the allocation out of Running while no SVID is held, and when a landed k4 drop
 * leaves neither. withK4 recomputes it after every step that changes need, held or has(k1).
 *
 * World facts beyond `has` / `need` (each a production fact, read at HEAD):
 *   vip    the Service's listener has an assigned VIP. service_dataplane_identities returns an empty map until the allocator
 *          memo has a VIP for the intent (service_lifecycle.rs:852-885), and service_backend_row_actions returns at once when
 *          that map is empty (:1096-1099). Set by a k3 or k2 world change (the submit edge assigns it); never cleared.
 *   held   an SVID is held by the node's IdentityMgr for some allocation of the workload. The k1 start path issues and holds
 *          the SVID itself before the allocation is released (ensure_intercept_identity, action_shim/mod.rs:187-213, called at
 *          :1870 and :2306); svid-lifecycle's own IssueSvid holds it too. It is volatile in a real process restart, but the
 *          conformance Crash rebuilds the runtime only (WK-5), so it is not reset.
 *   running  derived: a Running allocation exists, `has(k1) and not need(k1)` (workload-lifecycle emits nothing while a Running
 *          allocation exists and starts one when none does: workload_lifecycle.rs the `running_alloc` branch).
 * The needs of the four keys are then (production reconcile bodies):
 *   need(k1)  intent persisted and no Running allocation.
 *   need(k4)  a Running allocation with no held SVID (IssueSvid), or a held SVID of an allocation that is no longer Running
 *             (DropSvid, svid_lifecycle.rs `not running and held`). `running and held and not near-expiry` emits nothing.
 *   need(k3)  with a VIP: the observed backend row differs from the projection of the Running allocations (any membership change
 *             or the VIP/row itself); without a VIP: a Running allocation whose Stable is not committed (the startup opt-out
 *             commits Stable at the first Running row, service_lifecycle.rs branch (a')).
 *   need(k2)  the published backend set differs from the dataplane's programmed fingerprint (a non-empty set always differs after a
 *             k3 publish; an empty set differs only when a slot is programmed to purge: in_place, service_map_hydrator.rs:552-562).
 *   v3     service-lifecycle's next evaluation would change its View (D9): an allocation row it has not recorded yet (a Running
 *          allocation not in `observed`, a Failed one not in `terminal_announced`; service_lifecycle.rs reconcile, the two
 *          `insert`s at the head of the per-allocation loop; a committed-Stable allocation is skipped before them). Set by a k4
 *          world change (a fresh Running row), a k1 world change that replaces a Running allocation (a Failed row) and a landed
 *          k1 start (a new Running allocation); cleared by any k3 tick that persists. Its View has no other field a world change
 *          of the model touches (the probe fields are never written without probe rows).
 *   r4     svid-lifecycle's stored View holds an IssueRetry entry (D9, D11). The entry is bumped when an IssueSvid is EMITTED, not when
 *          it fails (svid_lifecycle.rs: "Record the attempt"), and removed by the next tick that sees the allocation held, or no
 *          longer Running (`retry.remove`, `retry.retain`). After any persisted k4 tick it holds iff that tick saw a Running
 *          unheld allocation (need(k4) and running).
 * So need(k1) and need(k4) cannot hold together EXCEPT through a held SVID to drop (D8): k4 needs a Running allocation and k1
 * needs none. World changes (stimulus): a k1 change replaces the Running allocation with a Failed row (need(k1); need(k3) :=
 * vip; need(k4) := held); a k4 change adds a fresh Running allocation with no SVID (need(k4), need(k3), need(k1) cleared); a k3
 * change assigns the VIP (need(k3)); a k2 change assigns the VIP and writes a backend row (need(k2), need(k3): the observed row
 * differs from the projection). A k3, k2 change on a workload with no intent persists it, so need(k1) (no allocation exists).
 *
 * Handoff wakes and landed-action consequences (existing behaviour, not decisions of this feature; read at HEAD):
 *   k1 acts      workload-lifecycle emitted an allocation-mutating action: it enqueues service-lifecycle (Service workload) then
 *                svid-lifecycle for the same workload/<id> (workload_lifecycle.rs:264-306; the shim submits them Immediate,
 *                action_shim/mod.rs:2899), pushed before the tick's own self-requeue. When the start lands (ok) the allocation
 *                exists and its SVID is held (D6): need(k3) (Stable to commit, and the projection changed when there is a VIP)
 *                and has(k3), has(k4). need(k4) is NOT set: the svid-lifecycle evaluation after the start is a Noop
 *                (`running and held`), so the k1 to k4 handoff wakes a key that is not needy (WK-7).
 *   k3 acts      service-lifecycle emitted its actions. Only with a VIP is there a WriteServiceBackendRow, and only beside it an
 *                EnqueueEvaluation for service-map-hydrator service/<id> (service_lifecycle.rs:1152-1166, push at :1161). With
 *                a VIP and ok the row lands: has(k2), need(k2) (D4). Without a VIP k3 acts (it commits Stable, so it requeues)
 *                but writes no row: no handoff, nothing lands for k2 (D5).
 *   k2, k4 act   no handoff.
 *   Noop evaluation   a handed-off (or submitted) key that is not needy evaluates to a Noop (svid-lifecycle emits Action::Noop,
 *                has_action is false, next_evaluation_at is None here): it is admitted and counted in `dispatched`, it does NOT
 *                requeue. A submit at an already pending key collapses and counts in `cancelled` whether or not the key is
 *                needy (EvaluationBroker::submit does not read the world; enqueue_evaluation.rs submits Immediate through
 *                ReconcilerRuntime::submit_evaluation).
 * Per outcome of Finish(k, o), "k acts" meaning needy and not inside its backoff window:
 *   ok           effect lands: the handoffs are submitted (in the order above) then the self-requeue; LAND consequences apply.
 *   dispatchErr  the mutating action failed but dispatch isolates actions (action_shim dispatch_with_workflow_intent doc:
 *                "per-action isolation"), so the EnqueueEvaluation actions still run: handoffs submitted, then the self-requeue;
 *                nothing lands, so no LAND consequence, EXCEPT for a failed k1 start: the start path issues and holds the SVID
 *                (ensure_intercept_identity, action_shim/mod.rs:1858) BEFORE driver.start and the Running-row write (:2044); a
 *                failed write tears the workload down (:2055-2096) but never releases the held SVID, so the failed start leaves
 *                an ORPHAN held SVID (no allocation row, no Running allocation): held, has(k4) (svid_lifecycle.rs managed_targets
 *                :317-323), need(k4) (DropSvid of a held SVID of a non-Running allocation). service-lifecycle manages no target
 *                (no allocation row: has(k3) unchanged). A later landed start mints a fresh allocation id (the View's
 *                restart_counts reserves the failed one, workload_lifecycle.rs next_allocation_attempt), so the orphan stays held
 *                and needy until a landed k4 drop.
 *   dpFail       (hydrator only) a result row, no handoff.
 * Backoff windows (D-3, D11). Two reconcilers have one: the hydrator (View RetryMemory) and svid-lifecycle (View IssueRetry). A
 * failed dataplane update enters the hydrator's window; a failed IssueSvid (dispatchErr on a Running, unheld allocation) enters
 * k4's window. An evaluation inside the window is a Noop; both reconcilers implement next_evaluation_at, so the in-window Noop
 * requeues with a NotBefore retention (svid_lifecycle.rs next_evaluation_at: the retry deadline of a Running, unheld,
 * never-issued allocation; the hydrator's is D-3, switched off by OFF "noRetention"; svid-lifecycle's is existing behaviour and
 * stays on in every instance). A k1 world change ends k4's window (the allocation is no longer Running; its entry is garbage
 * collected). Every backoff is the one constant RESTART_BACKOFF_DURATION (backoff_for_attempt ignores the attempt), so one
 * clock advance ends every open window (WindowEnds).
 *   hydrateErr, persistErr   end before dispatch: no handoff. Crash (between fsync and dispatch) loses the broker: none.
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
 *               window (hydrator, svid-lifecycle) the tick emits Noop and consults next_evaluation_at: Some -> a NotBefore
 *               requeue (D-3, switched off by OFF "noRetention"). Otherwise Noop, no requeue.
 *   dpFail      the action was dispatched, the dataplane executor recorded a Failed result (the tick itself is ok):
 *               the effect did not land, the View records the attempt (the key enters its backoff window), the
 *               has_action self-requeue is Immediate.
 *   hydrateErr  `?` on a hydrate call: no View write, no dispatch, no requeue.
 *   persistErr  `?` on persist_view: no dispatch, no requeue (View not written).
 *   dispatchErr the shim returned Err: the View was persisted, the effect did not land; the has_action self-requeue
 *               runs on all paths (reconciler_runtime.rs:1739), so it is Immediate.
 * Error outcomes are only offered for a key that was needy when the tick hydrates (an error on a key with nothing to do
 * changes nothing the model observes). persistErr is further offered only where production's tick CHANGES the View (D9):
 * persist_view returns before the write when next_view equals the stored View (reconciler_runtime.rs, `if current == view` in
 * every arm), so on an unchanged View the injected write failure is never reached and the tick dispatches and requeues like ok.
 *   workload-lifecycle  acting always changes the View (a new allocation id enters restart_counts)
 *   service-map-hydrator acting changes it (the attempt is bumped); an in-window Noop does not
 *   svid-lifecycle     an IssueSvid changes it (the IssueRetry entry is bumped); a DropSvid only when an entry is still stored (r4)
 *   service-lifecycle  only when the tick records an allocation it has not recorded (v3); a VIP and no allocation changes nothing
 *
 * Resync pass (D-1b): `due` is the period timer (Tick sets it; the first pass runs at boot, so init has due). A pass
 * starts only when due, clears it, and walks the registered reconcilers in BTreeMap order (r1S, r2M, r3V, r4W), one PassStep per
 * reconciler: managed_targets either returns the whole set (every `has` key of that reconciler, submitted
 * Immediate through the broker) or Err for the whole call (nothing submitted, the pass continues with the next
 * reconciler). The timer re-arms while a pass runs, so the pass is at most once per period and anchored (no
 * drift); see ASSUMPTION WK-3 for what the model does not carry.
 *
 * Environment faults (all budgeted by `faults < MAXF`, kinds enabled by FAULTS): hydrateErr, persistErr,
 * dispatchErr, dpFail, enumFail (managed_targets Err for one reconciler), crash (process restart between View
 * fsync and dispatch: the in-memory broker, the in-flight set and the broker's counters are lost, the View and the world
 * persist, the boot pass is due again). Dropped edge wake: DivergeDrop is a world change whose edge wake is lost (DROPS).
 * Drift is budgeted (MAXD). Spurious/edge wakes (EdgeWake) are unbudgeted.
 *
 * OFF names the decisions switched off (hazards):
 *   noPass       D-1  the resync pass is removed (PassStep disabled, so no boot pass either)
 *   noRetention  D-3  next_evaluation_at returns None (an in-window Noop is not requeued)
 *
 * Model-only (conformance): see README (pinned projection table); `conf_step` records the action that produced this state (model-only).
 * dispatched / cancelled count Drain admissions and same-key collapses only when REC (the conformance main);
 * the checking instances keep them 0 so TLC's state space is finite. They live in the in-memory broker
 * (BrokerCounters, reset only by constructing a new broker), so Crash resets both to 0 (WK-5).
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   WK-1  The broker's LWW collapse, eligibility merge, first-pending FIFO order and per-target blocking are as
 *         eval_broker.rs `submit` / `drain_pending` / `merge_eligibility` (read at HEAD fb71b400f).
 *         Discharged by: V-4 (existing eval_broker tests + run edge_wake_during_burst_is_admitted_after_burst and
 *         pass_submit_merges_retained_not_before_to_immediate under the quint-connect driver; the latter discharges the
 *         NotBefore to Immediate merge through admission: the following Drain admits the key and pending empties).
 *   WK-2  ReconcilerRuntime::submit_evaluation wakes the loop (ADR-0273), so a submitted key is drained without a
 *         separate sleep-until wake; the loop's cadence timer backstops it.
 *         Discharged by: the existing ADR-0273 submit-wakes-loop acceptance tests.
 *   WK-3  Time is abstract: the period is a timer event (Tick) with weak fairness, a backoff window ends by a
 *         WindowEnds event with weak fairness (one clock advance past the shared instant), which also makes a retained
 *         NotBefore eligible, exactly when its window ends (the deadline IS last_failure_seen_at + backoff_for_attempt, D-3). The 30 s value, "within one
 *         period plus queue time" as a duration, and anchored re-arming at previous + period on the injected clock
 *         are NOT checked here. Discharged by: a DISTILL obligation on the default-lane seeded overdrive-sim / SimClock
 *         harness covering the pass's boot run, anchoring at previous + period with no drift, at most one pass
 *         per period, and recovery within one period plus queue time. (V-3 was a native-metal cost measurement
 *         and does not discharge WK-3.)
 *   WK-4  managed_targets returns the complete set or Err for the whole call, from intent and from observation
 *         (D-1a postcondition); `has` is that set. Discharged by: the per-reconciler containment obligation, a DISTILL
 *         obligation (every target with a non-empty hydrate projection is in the returned managed_targets set;
 *         Err returns no partial set).
 *   WK-5  A crash loses exactly the in-memory broker (queue and its dispatched / cancelled counters) and in-flight evaluations; the View store, intent and
 *         observation rows survive (redb fsync, ADR-0035 step 7 -> 8). Discharged by: the existing WriteThrough
 *         ordering invariant and a DST crash scenario.
 *   WK-7  The handoff wakes, the landed-action consequences and the world needs above are as the cited production sources: which
 *         action emits which EnqueueEvaluation and in which order; that a k1 start holds its own SVID, so an svid-lifecycle
 *         evaluation after a k1 start is a Noop (it neither acts nor requeues; the earlier claim that it acts and requeues once
 *         was false, D6); that a k3 evaluation publishes a row and hands off to the hydrator only with a VIP (D5); that an
 *         allocation row makes service-lifecycle manage the target (D7); and that need(k1) and need(k4) exclude each other
 *         except through a held SVID to drop (D8). Discharged by: the fidelity probe (conformance-driver.md) and the quint-connect
 *         replay of the named runs against the real reconcilers. The k2/k3 partial-landing facts are part of WK-7: a k3 dispatchErr
 *         with a VIP and a Stable to commit still lands the row (the Stable commit is the first write to consume the armed
 *         seam, observation_store.rs write_alloc_lifecycle), a k2 dispatchErr leaves the dataplane programmed (ADR-0299), and a landed
 *         k3 publish recomputes need(k2) (cleared when the empty row is in place). Discharged by: the conformance replay with
 *         injected dispatchErr on k2 and k3 (quint_evaluation_wake.rs).
 *   WK-8  The View-change facts (D9, D11): which ticks change which View (persist_view's Eq-diff skip, service-lifecycle's
 *         observed / terminal_announced inserts, svid-lifecycle's IssueRetry bump on emit and its removal), that the IssueRetry is
 *         recorded when IssueSvid is emitted (so a failed issue leaves the key inside its window), and that svid-lifecycle's
 *         next_evaluation_at retains the in-window Noop. Discharged by: the quint-connect replay of the named runs and of the
 *         randomized traces against the real reconcilers (the fidelity probe, conformance-driver.md).
 *   WK-6  The only pacing View the model needs is the hydrator's and svid-lifecycle's backoff windows (WK-8); other reconcilers' View pacing
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
  val KEYS: List[str] = ["k1", "k2", "k3", "k4"]
  val KEYSET: Set[str] = Set("k1", "k2", "k3", "k4")
  val RECS: List[str] = ["r1S", "r2M", "r3V", "r4W"]   // registered (BTreeMap) order of the production names
  val NREC: int = 4
  val RECOF: str -> str = Map("k1" -> "r4W", "k2" -> "r2M", "k3" -> "r1S", "k4" -> "r3V")
  val TGT: str -> str = Map("k1" -> "t1", "k2" -> "t2", "k3" -> "t1", "k4" -> "t1")
  val ALLIMM: str -> str = Map("k1" -> "imm", "k2" -> "imm", "k3" -> "imm", "k4" -> "imm")
  val NO_PASS: bool = OFF.contains("noPass")
  val NO_RET: bool = OFF.contains("noRetention")

  pure def isH(k: str): bool = k == "k2"
  // managed-set coupling of the one shared workload: a service-lifecycle target (allocation or VIP) implies the workload intent;
  // an svid-lifecycle target (an allocation row) implies the intent and, on a Service, a service-lifecycle target (D7); a
  // hydrator target (a published backend row) implies a service-lifecycle target
  pure def withBase(k: str): Set[str] =
    if (k == "k3") Set("k3", "k1") else if (k == "k4") Set("k4", "k3", "k1") else if (k == "k2") Set("k2", "k3", "k1") else Set(k)
  // has(k4) follows "a Running allocation or a held SVID" (D-1a), where a Running allocation is has(k1) and not need(k1)
  pure def withK4(h: Set[str], n: Set[str], hd: bool): Set[str] =
    if ((h.contains("k1") and not(n.contains("k1"))) or hd) h.union(Set("k4")) else h.exclude(Set("k4"))
  // a hydrator target exists whenever the Service has an assigned VIP (D-21)
  pure def withVip(h: Set[str], v: bool): Set[str] = if (v) h.union(Set("k2")) else h
  pure def inP(p: List[str], k: str): bool = p.indices().exists(i => p[i] == k)
  pure def subP(p: List[str], k: str): List[str] = if (inP(p, k)) p else p.append(k)
  pure def subE(p: List[str], e: str -> str, k: str, kind: str): str -> str =
    if (inP(p, k)) (if (kind == "imm") e.set(k, "imm") else e) else e.set(k, kind)
  pure def subMany(p: List[str], e: str -> str, ks: List[str]): (List[str], str -> str) =
    ks.foldl((p, e), (acc, k) => (subP(acc._1, k), subE(acc._1, acc._2, k, "imm")))

  // world
  var has: Set[str]       // managed set (intent U observation): keys with desired or actual state
  var need: Set[str]      // desired != actual
  var inWin: Set[str]     // View: key is inside its backoff window (hydrator: after a failed dataplane result; svid-lifecycle: after a failed IssueSvid)
  var vip: bool           // the listener has an assigned VIP (D5: only then does service-lifecycle publish a backend row)
  var held: bool          // an SVID is held for some allocation of the workload (D6: a k1 start holds its own)
  var v3: bool            // service-lifecycle's next evaluation changes its View (an allocation it has not recorded yet: D9)
  var prog: bool          // the dataplane slot of the workload's service is programmed (set by a k2 tick that lands a non-empty set)
  var rowNE: bool         // the published service_backends row carries backends (written by k3's publish: allocations exist; by a k2 world change)
  var r4: bool            // svid-lifecycle's stored View holds an IssueRetry entry (D9, D11)
  var stU: bool           // a Running allocation whose Stable is not committed (service-lifecycle emits FinalizeFailed{Stable}; a lifecycle write)
  var rowD: bool          // the observed backend row differs from the projection (service-lifecycle emits WriteServiceBackendRow + EnqueueEvaluation(k2)); implies vip
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
  var conf_step: ConfStep // the action that produced this state (model-only)

  def canFault(f: str): bool = FAULTS.contains(f) and faults < MAXF
  // a world change of key k writes an allocation row service-lifecycle has not recorded: a fresh Running allocation (k4), or the
  // Failed row that replaces the Running one (k1, only when an allocation exists). D9
  def newAllocFor(k: str): bool = k == "k4" or (k == "k1" and has.contains("k1"))

  // a Running allocation exists (derived; see the header)
  val running: bool = has.contains("k1") and not(need.contains("k1"))
  // the needy set after a world change of key k (stimulus; see the header)
  def needAfter(k: str): Set[str] =
    if (k == "k1")
      need.exclude(Set("k3", "k4")).union(Set("k1")).union(if (vip) Set("k3") else Set()).union(if (held) Set("k4") else Set())
    else if (k == "k4") need.exclude(Set("k1")).union(Set("k3", "k4"))
    else need.union(if (k == "k2") Set("k2", "k3") else Set("k3")).union(if (has.contains("k1")) Set() else Set("k1"))

  // what a world change of key k does to the two sources of need(k3): a k4 change adds a fresh Running allocation (its Stable is
  // uncommitted and the projection changed); a k1 change replaces the Running allocation by a Failed row (no Stable to commit; the
  // projection lost a backend); a k3 or k2 change assigns the VIP and leaves a row that differs
  def stUAfter(k: str): bool = if (k == "k4") true else if (k == "k1") false else stU
  def rowDAfter(k: str): bool = if (k == "k1" or k == "k4") vip else true

  action init = all {
    has' = Set(), need' = Set(), inWin' = Set(), vip' = false, held' = false, v3' = false, r4' = false, prog' = false, rowNE' = false, stU' = false, rowD' = false, active' = Set(),
    pending' = List(), elig' = ALLIMM, due' = true, passPos' = 0, faults' = 0, drifts' = 0,
    dispatched' = 0, cancelled' = 0, conf_step' = variant("init", ()),
  }

  // the period timer fires
  action Tick = all {
    not(due),
    due' = true,
    has' = has, need' = need, inWin' = inWin, vip' = vip, held' = held, v3' = v3, r4' = r4, prog' = prog, rowNE' = rowNE, stU' = stU, rowD' = rowD, active' = active, pending' = pending, elig' = elig,
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
      has' = has, need' = need, inWin' = inWin, vip' = vip, held' = held, v3' = v3, r4' = r4, prog' = prog, rowNE' = rowNE, stU' = stU, rowD' = rowD, active' = active, drifts' = drifts,
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
      has' = has, need' = need, inWin' = inWin, vip' = vip, held' = held, v3' = v3, r4' = r4, prog' = prog, rowNE' = rowNE, stU' = stU, rowD' = rowD, due' = due, passPos' = passPos,
      faults' = faults, drifts' = drifts, cancelled' = cancelled,
      conf_step' = if (REC) variant("Drain", ()) else conf_step,
    }
  }

  // the end of one evaluation (run_convergence_tick)
  action Finish(k: str, o: str): bool = {
    val sn = need.contains(k)   // hydrated now: the world at the start of run_convergence_tick
    val win = inWin.contains(k)
    val acts = sn and not(win)
    // an in-window Noop consults next_evaluation_at: the hydrator's (D-3, switched off by noRetention) and svid-lifecycle's
    // (existing: svid_lifecycle.rs next_evaluation_at returns the retry deadline of a Running, unheld, never-issued allocation, D11)
    val retain = sn and win and ((isH(k) and not(NO_RET)) or k == "k4")
    // persist_view skips the write when next_view equals the stored View (reconciler_runtime.rs, `if current == view`), so an
    // injected View-write failure is reached only when the tick changes the View (D9). When the key acts: workload-lifecycle
    // always (a new allocation id enters its View), the hydrator always (attempts bump), svid-lifecycle for an IssueSvid (running:
    // the IssueRetry entry is bumped) and for a DropSvid only when an entry for the dropped allocation is still stored (r4:
    // the retain GC removes it), service-lifecycle only when it has an allocation to record (v3)
    val viewChanges =
      if (k == "k3") v3 else if (k == "k4") (running or r4) else true
    val kind =
      if (o == "ok") (if (acts) "imm" else if (retain) "nb" else "none")
      else if (o == "dpFail" or o == "dispatchErr") "imm"
      else "none"
    val allowed =
      if (o == "ok") true
      else if (o == "dpFail") (isH(k) and acts and canFault("dpFail"))
      else if (o == "dispatchErr") (acts and canFault("dispatchErr"))
      else if (o == "hydrateErr") (sn and canFault(o))
      else if (o == "persistErr") (acts and viewChanges and canFault(o))
      else false
    // THE EFFECTS OF A DISPATCH (per-action isolation: dispatch runs every action, each lands or fails on its own; a failed one leaves
    // the others landed). The driver's injected failure is one shot at the first write of the seam it arms (SimObservationStore
    // `write_alloc_lifecycle` consumes the lifecycle seam, then the generic seam; `write` only the generic one). The ordered effects
    // per key, and the one a dispatchErr hits:
    //   k1  StartAllocation: [issue + hold the SVID (generic audit write), driver.start, Running row (lifecycle write)]; the lifecycle
    //       seam hits the Running row (action_shim/mod.rs:2044): the workload is torn down, the SVID stays held (orphan), nothing lands
    //   k2  DataplaneUpdateService: [dataplane.update_service, result row (generic write)]; the generic seam hits the result row
    //       (dataplane_update_service.rs): the dataplane IS programmed, and the hydrator's `actual` is the programmed fingerprint
    //       (ADR-0299), so the set is in place: the world lands, the dispatch is an Err
    //   k3  [FinalizeFailed{Stable} (lifecycle write; only while Stable is uncommitted: stU), WriteServiceBackendRow (generic write) +
    //       EnqueueEvaluation(k2) (only with a VIP and a differing row: rowD)]; with a VIP the generic seam is armed and the Stable
    //       commit, the first write to consume it, takes the failure (observation_store.rs write_alloc_lifecycle): the row lands.
    //       With a VIP and no Stable to commit the row write takes it. Without a VIP the lifecycle seam is armed and the Stable
    //       commit takes it
    //   k4  IssueSvid: [CA issue + audit write (generic), hold]; the audit write takes the failure, nothing is held. DropSvid has no
    //       write (issue_svid.rs dispatch_drop): the injected failure is never consumed and the drop lands
    val e1 = k == "k3" and stU
    val e2 = k == "k3" and vip and rowD
    val landE1 = e1 and acts and o == "ok"
    val landE2 = e2 and acts and (o == "ok" or (o == "dispatchErr" and stU))
    val landK = k != "k3" and acts and (o == "ok" or (o == "dispatchErr" and (k == "k2" or (k == "k4" and not(running)))))
    val lands = landK
    // a failed k1 start leaves the SVID it issued held, for an allocation that has no row (see the header)
    val orphan = o == "dispatchErr" and acts and k == "k1"
    val persisted = o == "ok" or o == "dispatchErr" or o == "dpFail"
    // an emitted action is dispatched on ok and on dispatchErr (dispatch isolates actions, so the EnqueueEvaluation actions
    // run although the mutating action failed); hydrateErr, persistErr and Crash end before dispatch: no handoff. dpFail is a
    // result row, not a dispatch error, and only the hydrator (no handoff) produces it
    val emits = acts and (o == "ok" or o == "dispatchErr")
    // the handoffs of the actions k emits: k1 hands off to k3 then k4 (k4's evaluation is a Noop after a start: D6); k3 hands
    // off to the hydrator only beside its backend-row write, which needs a VIP and a differing row (D5); k2 and k4 hand off nothing
    val handOf: List[str] =
      if (k == "k1") ["k3", "k4"] else if (e2) ["k2"] else List()
    val hand = if (emits) handOf else List()
    val sub = subMany(pending, elig, hand)
    // what a landed action leaves in the world (D3 to D6): a start holds its SVID and leaves service-lifecycle needy (not
    // svid-lifecycle); a landed publish makes the hydrator managed and needy; an svid action leaves the SVID held for every
    // Running allocation (and drops the held SVID of an allocation that is not Running)
    // the row k3 publishes carries the allocations' backends: a non-empty set differs from what is programmed (k3 acts only on a
    // changed row); an empty set (no allocation, need(k1)) is in place unless a slot is programmed to purge (in_place: empty
    // survivors are in place only when nothing is programmed, service_map_hydrator.rs:552-562). On a landed publish need(k2) is
    // RECOMPUTED from the published row against the programmed state: a pending need(k2) is cleared when the new row is in place
    val stU2 = if (landE1) false else if (k == "k1" and lands) true else stU
    val rowD2 = if (landE2) false else if (k == "k1" and lands) vip else rowD
    val needL =
      if (k == "k1") need.exclude(Set("k1")).union(Set("k3"))
      else if (k == "k3")
        need.exclude(if (landE2) Set("k2", "k3") else Set("k3")).union(if (landE2 and (not(need.contains("k1")) or prog)) Set("k2") else Set())
             .union(if (stU2 or rowD2) Set("k3") else Set())
      else need.exclude(Set(k))
    val landed = lands or landE1 or landE2
    val hasL =
      if (k == "k1") has.union(Set("k3", "k4")) else if (landE2) has.union(Set("k2")) else has
    val heldL = if (k == "k1") true else if (k == "k4") running else held
    all {
      active.contains(k),
      allowed,
      need' = if (landed) needL else if (orphan) need.union(Set("k4")) else need,
      has' = if (landed) withK4(hasL, needL, heldL) else if (orphan) withK4(has, need.union(Set("k4")), true) else has,
      held' = if (lands) heldL else if (orphan) true else held,
      // a failed dataplane update (hydrator) or a failed IssueSvid (svid-lifecycle: IssueRetry recorded at emit, nothing held)
      // leaves the key inside its backoff window
      // a landed k3 publish carries a row that differs from the one the hydrator last attempted, and pacing_allows lets a changed
      // desired set dispatch at once (service_map_hydrator.rs pacing_allows): the hydrator's window ends
      inWin' = if (o == "dpFail" or (o == "dispatchErr" and k == "k4" and running)) inWin.union(Set(k))
               else if (landE2) inWin.exclude(Set("k2")) else inWin,
      // a persisted tick (ok, dispatchErr, dpFail) records every allocation service-lifecycle sees and rewrites svid-lifecycle's
      // retry map to the entries of Running unheld allocations; a k1 start adds an allocation service-lifecycle has not seen
      v3' = if (lands and k == "k1") true else if (k == "k3" and persisted) false else v3,
      r4' = if (k == "k4" and persisted) (sn and running) else r4,
      // a landed k2 tick programs what the row carries (empty: purge); a landed k3 publish writes a row with the allocations'
      // backends
      prog' = if (lands and k == "k2") rowNE else prog,
      rowNE' = if (landE2) not(need.contains("k1")) else rowNE,
      stU' = stU2, rowD' = rowD2,
      active' = active.exclude(Set(k)),
      pending' = if (kind == "none") sub._1 else subP(sub._1, k),
      elig' = if (kind == "none") sub._2 else subE(sub._1, sub._2, k, kind),
      cancelled' = if (REC) cancelled + hand.select(h => inP(pending, h)).length()
                           + (if (kind != "none" and inP(pending, k)) 1 else 0) else cancelled,
      faults' = if (o == "ok") faults else faults + 1,
      vip' = vip, due' = due, passPos' = passPos, drifts' = drifts, dispatched' = dispatched,
      conf_step' = if (REC) variant("Finish", { k: k, o: o }) else conf_step,
    }
  }

  // a world change with its edge wake (interests() / client request / EnqueueEvaluation)
  action DivergeWake(k: str): bool = all {
    drifts < MAXD,
    not(need.contains(k)),
    has' = withK4(withVip(has.union(withBase(k)), vip or k == "k2" or k == "k3"), needAfter(k), held),
    need' = needAfter(k),
    vip' = (vip or k == "k2" or k == "k3"),
    pending' = subP(pending, k),
    elig' = subE(pending, elig, k, "imm"),
    drifts' = drifts + 1,
    cancelled' = if (REC and inP(pending, k)) cancelled + 1 else cancelled,
    inWin' = if (k == "k1") inWin.exclude(Set("k4")) else inWin,
    v3' = (v3 or newAllocFor(k)), r4' = r4, prog' = prog, rowNE' = (rowNE or k == "k2"), stU' = stUAfter(k), rowD' = rowDAfter(k),
    held' = held, active' = active, due' = due, passPos' = passPos, faults' = faults,
    dispatched' = dispatched,
    conf_step' = if (REC) variant("DivergeWake", { k: k }) else conf_step,
  }

  // environment fault: the same world change whose edge wake is dropped (or never existed: a fresh deploy has
  // intent and no alloc_status row)
  action DivergeDrop(k: str): bool = all {
    DROPS,
    drifts < MAXD,
    not(need.contains(k)),
    has' = withK4(withVip(has.union(withBase(k)), vip or k == "k2" or k == "k3"), needAfter(k), held),
    need' = needAfter(k),
    vip' = (vip or k == "k2" or k == "k3"),
    drifts' = drifts + 1,
    inWin' = if (k == "k1") inWin.exclude(Set("k4")) else inWin,
    v3' = (v3 or newAllocFor(k)), r4' = r4, prog' = prog, rowNE' = (rowNE or k == "k2"), stU' = stUAfter(k), rowD' = rowDAfter(k),
    held' = held, active' = active, pending' = pending, elig' = elig, due' = due,
    passPos' = passPos, faults' = faults, dispatched' = dispatched, cancelled' = cancelled,
    conf_step' = if (REC) variant("DivergeDrop", { k: k }) else conf_step,
  }

  // an edge wake with no world change (accepted row, EnqueueEvaluation, spurious)
  action EdgeWake(k: str): bool = all {
    pending' = subP(pending, k),
    elig' = subE(pending, elig, k, "imm"),
    cancelled' = if (REC and inP(pending, k)) cancelled + 1 else cancelled,
    has' = has, need' = need, inWin' = inWin, vip' = vip, held' = held, v3' = v3, r4' = r4, prog' = prog, rowNE' = rowNE, stU' = stU, rowD' = rowD, active' = active, due' = due,
    passPos' = passPos, faults' = faults, drifts' = drifts, dispatched' = dispatched,
    conf_step' = if (REC) variant("EdgeWake", { k: k }) else conf_step,
  }

  // time passes: the injected clock advances by RESTART_BACKOFF_DURATION, the one backoff of every window (backoff_for_attempt
  // ignores the attempt), so every open window ends (the hydrator's and svid-lifecycle's) and every retained NotBefore (the
  // deadline IS last_failure_seen_at + backoff_for_attempt, D-3, D11) becomes eligible ("imm"). k names the window the step is
  // enabled for
  action WindowEnds(k: str): bool = all {
    inWin.contains(k),
    inWin' = Set(),
    elig' = KEYSET.mapBy(j => if (inP(pending, j) and elig.get(j) == "nb") "imm" else elig.get(j)),
    has' = has, need' = need, vip' = vip, held' = held, v3' = v3, r4' = r4, prog' = prog, rowNE' = rowNE, stU' = stU, rowD' = rowD, active' = active, pending' = pending, due' = due,
    passPos' = passPos, faults' = faults, drifts' = drifts, dispatched' = dispatched, cancelled' = cancelled,
    conf_step' = if (REC) variant("WindowEnds", { k: k }) else conf_step,
  }

  // environment fault: process restart between View fsync and dispatch; the boot pass is due again
  action Crash = all {
    canFault("crash"),
    pending' = List(), elig' = ALLIMM, active' = Set(),
    passPos' = 0, due' = true, faults' = faults + 1,
    dispatched' = 0, cancelled' = 0,
    has' = has, need' = need, inWin' = inWin, vip' = vip, held' = held, v3' = v3, r4' = r4, prog' = prog, rowNE' = rowNE, stU' = stU, rowD' = rowD, drifts' = drifts,
    conf_step' = if (REC) variant("Crash", ()) else conf_step,
  }

  action step = {
    nondet k = KEYSET.oneOf()
    nondet o = Set("ok", "dpFail", "hydrateErr", "persistErr", "dispatchErr").oneOf()
    nondet b = Set(true, false).oneOf()
    any { Tick, PassStep(b), Drain, Finish(k, o), DivergeWake(k), DivergeDrop(k), EdgeWake(k), WindowEnds(k), Crash }
  }

  val allVars = (has, need, inWin, vip, held, v3, r4, prog, rowNE, stU, rowD, active, pending, elig, due, passPos, faults, drifts, dispatched,
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
  // D-3, D11: a backoff window (the hydrator's, svid-lifecycle's) only exists for a needy key
  val WindowOnlyForNeedy: bool = inWin.subseteq(need)
  // world: the workload-target reconcilers derive from one intent (k3, k4 managed imply k1 managed), a Running allocation's row makes
  // service-lifecycle manage the Service (k4 and running imply k3: D7; an orphan held SVID has no row, so k4 alone does not), and a published backend row implies a service-lifecycle target
  val SharedIntentImpliesBase: bool = and { has.contains("k3").implies(has.contains("k1")), has.contains("k4").implies(has.contains("k1")),
                                            (has.contains("k4") and running).implies(has.contains("k3")), has.contains("k2").implies(has.contains("k3")) }
  // D-1a: svid-lifecycle manages the target iff the workload has a Running allocation or a held SVID (so has(k4) can shrink)
  val K4ManagedIffRunningOrHeld: bool = has.contains("k4") == (running or held)
  // D-21: an assigned VIP makes the hydrator manage the Service
  val VipImpliesHydratorManaged: bool = vip.implies(has.contains("k2"))
  // service-lifecycle needs work exactly when a Running allocation's Stable is uncommitted (stU, a lifecycle write) or the published
  // row differs (rowD, a backend-row write, only with a VIP); a Stable to commit needs a Running allocation
  val K3NeedIsStableOrRow: bool = and { need.contains("k3") == (stU or rowD), stU.implies(running), rowD.implies(vip) }
  val Safety: bool = and { ActiveWithinCap, OneActivePerTarget, PendingUnique, EligCanonical, NeedImpliesManaged,
                           WindowOnlyForNeedy, SharedIntentImpliesBase, K4ManagedIffRunningOrHeld, VipImpliesHydratorManaged, K3NeedIsStableOrRow }
  // D-3: a key inside its backoff window is always scheduled (in flight, or retained in the broker): the
  // re-evaluation does not depend on a row write or a pass. Holds when the only fault is the dataplane failing
  // (instance wake_retention); a hydrate/persist error or a crash legitimately ends the schedule (the pass retries)
  val RetentionSchedulesBackedOffKey: bool = inWin.forall(k => active.contains(k) or inP(pending, k))

  // ------------------------------------------------------------------ progress (temporal)
  temporal Fair = and {
    weakFair(Tick, allVars), weakFair(PassStep(true), allVars), weakFair(Drain, allVars),
    weakFair(Finish("k1", "ok"), allVars), weakFair(Finish("k2", "ok"), allVars), weakFair(Finish("k3", "ok"), allVars), weakFair(Finish("k4", "ok"), allVars),
    weakFair(WindowEnds("k1"), allVars), weakFair(WindowEnds("k2"), allVars), weakFair(WindowEnds("k3"), allVars), weakFair(WindowEnds("k4"), allVars),
  }
  // D-1: every needy key is eventually evaluated to convergence, from every error outcome, dropped wake and crash
  temporal ProgressByPass = Fair.implies(KEYSET.forall(k => always(need.contains(k).implies(eventually(not(need.contains(k)))))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // One shared workload (a Service with a listener): k1 workload-lifecycle, k3 service-lifecycle, k4 svid-lifecycle (all on
  // workload/<id>, t1), k2 service-map-hydrator on the workload's own listener (service/<id>, t2). A k1 action hands off to
  // k3 then k4; the landed start leaves k3 needy and the SVID held, so the k4 evaluation is a Noop (D6). A k3 action hands off
  // to k2 only with a VIP (D5), and then leaves k2 managed and needy (D4). Each run is written to replay against production
  // through the conformance driver: its world changes are the ones the driver constructs.

  // happy path: an edge wake converges k1; the landed start hands off to k3 and k4 (one target t1, so they run one at a time)
  // and holds its own SVID, so k3 commits Stable (no VIP: no row, no hydrator handoff, D5) and requeues, while the k4 evaluation
  // is a Noop that does not requeue (D6); the confirming requeues are Noops
  run healthy_edge_converges = init
    .then(DivergeWake("k1")).expect(need == Set("k1") and pending == ["k1"])
    .then(Drain).expect(active == Set("k1") and pending == List())
    .then(Finish("k1", "ok"))
    .expect(need == Set("k3") and has == Set("k1", "k3", "k4") and held and pending == ["k3", "k4", "k1"] and active == Set())
    .then(Drain).expect(active == Set("k3") and pending == ["k4", "k1"])
    .then(Finish("k3", "ok")).expect(need == Set() and pending == ["k4", "k1", "k3"])
    .then(Drain).expect(active == Set("k4") and pending == ["k1", "k3"])
    .then(Finish("k4", "ok")).expect(pending == ["k1", "k3"] and need == Set())
    .then(Drain).then(Finish("k1", "ok")).expect(pending == ["k3"])
    .then(Drain).then(Finish("k3", "ok"))
    .expect(need == Set() and pending == List() and active == Set() and dispatched == 5 and cancelled == 0 and Safety)

  // slice 01 fault 1: a hydrate error ends the evaluation with nothing scheduled and no handoff (it ends before dispatch);
  // the (boot) pass is the retry, and the retried start then hands off
  run hydrate_error_recovers_by_pass = init
    .then(DivergeWake("k1")).then(Drain)
    .then(Finish("k1", "hydrateErr")).expect(need == Set("k1") and pending == List() and active == Set() and faults == 1)
    .then(PassStep(true)).expect(pending == List() and due == false and passPos == 1)
    .then(PassStep(true)).then(PassStep(true)).expect(pending == List() and passPos == 3)
    .then(PassStep(true)).expect(pending == ["k1"] and passPos == 0)
    .then(Drain).then(Finish("k1", "ok"))
    .expect(need == Set("k3") and pending == ["k3", "k4", "k1"] and Safety)

  // slice 01 fault 2: a View persist error, same recovery (no dispatch, so no handoff)
  run persist_error_recovers_by_pass = init
    .then(DivergeWake("k1")).then(Drain)
    .then(Finish("k1", "persistErr")).expect(need == Set("k1") and pending == List() and active == Set())
    .then(PassStep(true)).then(PassStep(true)).then(PassStep(true)).then(PassStep(true))
    .expect(pending == ["k1"])
    .then(Drain).then(Finish("k1", "ok")).expect(need == Set("k3") and pending == ["k3", "k4", "k1"] and Safety)

  // environment fault: crash between View fsync and dispatch (no handoff was dispatched); the in-memory broker is lost,
  // the boot pass retries
  run crash_between_fsync_and_dispatch_recovers_by_boot_pass = init
    .then(DivergeWake("k1")).then(Drain)
    .then(Crash).expect(need == Set("k1") and pending == List() and active == Set() and due == true)
    .then(PassStep(true)).then(PassStep(true)).then(PassStep(true)).then(PassStep(true))
    .expect(pending == ["k1"])
    .then(Drain).then(Finish("k1", "ok")).expect(need == Set("k3") and pending == ["k3", "k4", "k1"] and Safety)

  // slice 01 fault 3 (D-3): the dataplane update fails; the confirming evaluation falls inside the backoff window,
  // emits Noop and its deadline is retained as NotBefore; the retry happens when the window ends, with no pass
  // (due stays true, passPos 0: the pass never ran) and no edge wake. The hydrator emits no handoff. The k2 world change also
  // assigns the VIP and persists the intent (k3 and k1 are needy too, never evaluated here)
  run failed_dataplane_inside_backoff_recovers_without_row_write = init
    .then(DivergeWake("k2")).then(Drain)
    .then(Finish("k2", "dpFail")).expect(inWin == Set("k2") and need.contains("k2") and pending == ["k2"] and elig.get("k2") == "imm")
    .then(Drain).then(Finish("k2", "ok"))
    .expect(need.contains("k2") and pending == ["k2"] and elig.get("k2") == "nb" and RetentionSchedulesBackedOffKey)
    .then(WindowEnds("k2")).expect(elig.get("k2") == "imm")
    .then(Drain).then(Finish("k2", "ok"))
    .expect(not(need.contains("k2")) and pending == ["k2"] and due == true and passPos == 0 and Safety)

  // slice 01 fresh deploy: intent exists, no row, the edge wake never happens; the pass evaluates it
  run fresh_deploy_without_row_is_evaluated = init
    .then(DivergeDrop("k1")).expect(need == Set("k1") and has == Set("k1") and pending == List())
    .then(PassStep(true)).expect(pending == List())
    .then(PassStep(true)).then(PassStep(true)).expect(pending == List())
    .then(PassStep(true)).expect(pending == ["k1"])
    .then(Drain).then(Finish("k1", "ok")).expect(need == Set("k3") and Safety)

  // enumeration failure of r4W (the last reconciler of the pass order r1S, r2M, r3V, r4W): nothing of r4W's set is submitted
  // (no partial set), r1S, r2M and r3V were still enumerated (the allocation row makes r1S manage the target: D7) and their keys
  // are evaluated; the next pass retries r4W. The k1 world change replaced the Running allocation (D8) while no SVID is held, so
  // svid-lifecycle no longer manages the target (D-1a: Running allocation or held SVID): k4 leaves the managed set and the pass
  // does not submit it. k3's evaluation is a Noop that does not requeue, and k1 stays needy until the next pass submits it
  run enumeration_failure_of_one_reconciler_does_not_starve_others = init
    .then(DivergeDrop("k4")).expect(need == Set("k3", "k4") and has == Set("k1", "k3", "k4"))
    .then(DivergeDrop("k1")).expect(need == Set("k1") and has == Set("k1", "k3"))
    .then(PassStep(true)).expect(pending == ["k3"] and passPos == 1)
    .then(PassStep(true)).expect(pending == ["k3"] and passPos == 2)
    .then(PassStep(true)).expect(pending == ["k3"] and passPos == 3)
    .then(PassStep(false)).expect(pending == ["k3"] and faults == 1 and passPos == 0)
    .then(Drain).expect(active == Set("k3") and pending == List())
    .then(Finish("k3", "ok")).expect(pending == List() and need == Set("k1") and cancelled == 0)
    .then(Tick).then(PassStep(true)).then(PassStep(true)).expect(pending == ["k3"])
    .then(PassStep(true)).expect(pending == ["k3"])
    .then(PassStep(true)).expect(pending == ["k3", "k1"])
    .then(Drain).expect(active == Set("k3") and pending == ["k1"])
    .then(Finish("k3", "ok")).then(Drain)
    .expect(active == Set("k1") and pending == List())
    .then(Finish("k1", "ok")).expect(need == Set("k3") and pending == ["k3", "k4", "k1"] and Safety)

  // a pass burst submits r1S's, r2M's and r4W's keys (order k3, k2, k1; the k3 world change assigned the VIP, so the hydrator
  // manages the Service, D-21); an edge wake for the hydrator's key (t2) submitted during the burst collapses onto its slot. Per-target exclusion: k1 waits while k3 holds t1, the edge wake is
  // admitted beside k3; k3's publish (the VIP is assigned) hands off to the (active) hydrator key, which queues again; k1 is
  // admitted when t1 frees, ahead of k3's confirming requeue, and its handoffs collapse onto the pending k3
  run edge_wake_during_burst_is_admitted_after_burst = init
    .then(DivergeDrop("k1")).then(DivergeDrop("k3"))
    .expect(has == Set("k1", "k2", "k3") and vip)
    .then(PassStep(true)).expect(pending == ["k3"])
    .then(EdgeWake("k2")).expect(pending == ["k3", "k2"])
    .then(PassStep(true)).then(PassStep(true)).then(PassStep(true)).expect(pending == ["k3", "k2", "k1"])
    .then(Drain).expect(active == Set("k3", "k2") and pending == ["k1"])
    .then(Finish("k3", "ok")).expect(need == Set("k1") and pending == ["k1", "k2", "k3"] and active == Set("k2"))
    .then(Drain).expect(active == Set("k2", "k1") and pending == ["k2", "k3"] and Safety)
    .then(Finish("k1", "ok")).then(Finish("k2", "ok"))
    .expect(need == Set("k3") and pending == ["k2", "k3", "k4", "k1"] and Safety)

  // V-4: a pass submit at a key holding a retained NotBefore merges it to Immediate, keeping the queue slot (the pass also
  // submits the workload's other managed keys, behind it)
  run pass_submit_merges_retained_not_before_to_immediate = init
    .then(DivergeWake("k2")).then(Drain).then(Finish("k2", "dpFail"))
    .then(Drain).then(Finish("k2", "ok")).expect(elig.get("k2") == "nb" and pending == ["k2"])
    .then(PassStep(true)).then(PassStep(true)).then(PassStep(true)).then(PassStep(true))
    .expect(elig.get("k2") == "imm" and pending == ["k2", "k3", "k1"] and inWin == Set("k2"))
    .then(Drain).expect(pending == ["k1"] and active == Set("k2", "k3"))

  // D-12 d: the world is read at Finish (run_convergence_tick hydrates at its start), not at Drain. k1 is admitted
  // while it does not need work; its divergence arrives (edge wake dropped) while it is in flight; Finish acts on
  // the world as read then (the start lands, holds its SVID and hands off to k3 and k4); the confirming evaluation queues
  // behind them, and every following evaluation is a Noop after k3's Stable commit
  run world_changes_between_drain_and_finish = init
    .then(EdgeWake("k1")).then(Drain)
    .expect(active == Set("k1") and need == Set() and pending == List())
    .then(DivergeDrop("k1")).expect(need == Set("k1") and active == Set("k1") and pending == List())
    .then(Finish("k1", "ok")).expect(need == Set("k3") and pending == ["k3", "k4", "k1"] and active == Set())
    .then(Drain).then(Finish("k3", "ok")).then(Drain)
    .then(Finish("k4", "ok")).then(Drain).then(Finish("k1", "ok"))
    .then(Drain).then(Finish("k3", "ok"))
    .expect(need == Set() and has == Set("k1", "k3", "k4") and pending == List() and Safety)
}

// Main: every decision on, every fault kind, dropped wakes. Safety and progress hold.
module wake_full {
  import evaluation_wake(OFF = Set(), FAULTS = Set("hydrateErr", "persistErr", "dispatchErr", "dpFail", "enumFail", "crash"), DROPS = true, REC = false).* from "./evaluation_wake"
}
// D-3 instance: the only faults are the dataplane failing (the hydrator's window) and a dispatch failing (svid-lifecycle's window,
// D11); the pass is not needed for the backoff schedule.
module wake_retention {
  import evaluation_wake(OFF = Set(), FAULTS = Set("dpFail", "dispatchErr"), DROPS = true, REC = false).* from "./evaluation_wake"
}
// Conformance main: records conf_step, the broker counters and every fault; only the named runs are checked on it.
module wake_conf {
  import evaluation_wake(OFF = Set(), FAULTS = Set("hydrateErr", "persistErr", "dispatchErr", "dpFail", "enumFail", "crash"), DROPS = true, REC = true).* from "./evaluation_wake"
}
