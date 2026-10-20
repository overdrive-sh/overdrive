// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — the CID lease (CID-NOCLAIM, RESET-EXIT, NO-SURVIVE).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: ADR-0156 (D16: CID = GUEST_CID_BASE + lease offset; next-fit, in-memory cursor),
 * ADR-0146 (the VMM opens its own /dev/vhost-vsock device and is the only holder of its fd), ADR-0182
 * (a guest reset ends the VMM), ADR-0159 as narrowed (VMs do not survive a `serve` restart: boot
 * reclamation kills every VM the new process does not supervise before any `assign`), the release-last
 * ordering of ADR-0133 (the lease is released only after the VMM has exited).
 *
 * There is no host-kernel CID claim: within one `serve` process, lease uniqueness is CID uniqueness. The
 * only other CID holders are VMMs of the previous process, until boot reclamation has killed them.
 *
 * Abstraction:
 *   - OFFS: lease offsets (CID = GUEST_CID_BASE + offset). ALLOCS: workloads placed on the node; a
 *     workload whose launch ended is retried by placement (`retry`), a stopped one is done and may be
 *     redeployed. `wait` / `done` are the node's persisted placement intent (the workload intent and its stop
 *     sentinel in the intent store), not pool state: `Stop` writes the stop sentinel and `Redeploy` is the restart
 *     verb's intent write that clears it (T-5 ruling, 2026-10-18).
 *   - `dev` is the kernel's holder table of the vhost-vsock device per CID: 0 free, a > 0 allocation a's
 *     VMM (the VMM is the only holder), -2 a VMM of the previous process (an orphan), still running until
 *     reclaimed. A VMM that finds its CID's device held exits before READY (the device open is refused).
 *   - `assign` walks the free offsets in next-fit order under the pool lock (one atomic step) and is
 *     enabled only after boot reclamation completed (`reclaimed`).
 *   - A `serve` crash empties the pool, its cursor and the in-memory placement (the persisted stop intent survives,
 *     so a stopped workload stays `done`: the T-5 ruling, 2026-10-18); every running VMM
 *     survives as an orphan (-2) until boot reclamation kills it. D-UP-19: boot reclamation is one
 *     `vm_reclamation_boot::converge` call, the one action `Reclaim`, which kills every orphan; the order in
 *     which converge kills them (ascending AllocationId) is not modelled, since no invariant depends on it.
 *     An orphan may also exit by itself before reclamation reaches it (a guest reset ends the VMM;
 *     `OrphanExit`), which also covers an orphan killed before a later crash.
 *   - RESET-EXIT: a guest reset (PID-1 crash, triple fault) ends the VMM process, so a CID's
 *     incarnation is one VMM process; the VMM's exit releases its device. Its lease stays Admitted (`ended`) until a
 *     dispatch retires it (feature-delta `cid_lease` item 7).
 *   - T-5 ruling (revised 2026-10-18, DR2-CLOSE): the production owner of placement is the WorkloadLifecycle
 *     reconciler; the conformance composition runs it through the production convergence tick and the real
 *     submit / stop / restart handlers, and the pool is touched only by the action shim dispatching its actions.
 *     `Assign` is a tick that places a `wait` workload (StartAllocation, `pool.assign`); `Launch` and `Release` are the
 *     held provisioner `provision` / `teardown` boundaries inside one dispatch; `wait` / `done` / `retry` are the
 *     reconciler's decision inputs (QPROJ-2W).
 *   - Crash-class restart granularity (feature-delta `cid_lease` item 7, DR5-CLOSE): the VMM's exit runs no tick (the
 *     lease stays Admitted, `ended`); the restart tick's dispatch leases the successor first, so the crashed
 *     allocation's lease is held as the predecessor (`pred` / `pst` = held); the successor's launch is followed by the
 *     predecessor's retirement (`pst` = retiring) and release. A pool refusal of the successor cleans the predecessor at
 *     once (`AssignRefused`). At the ceiling, FinalizeFailed (`Finalize`) retires the lease and writes the terminal after
 *     its release. A stop of an `ended` workload reclaims its lease.
 *   - Restart budget (RESTART-SSOT, GH #324; feature-delta § Per-module projection and action map, `cid_lease`
 *     items 2-6): `att` is WorkloadLifecycle's restart count of the workload's current allocation
 *     (`WorkloadLifecycleView::restart_counts`, persisted by the runtime: it survives a serve crash). It changes only at
 *     placement, as `restart_counts` does (workload_lifecycle.rs:1214-1221, :1339): `Assign` gives the new allocation
 *     `att + 1` when the predecessor is a crash-class row (`chg`), and `att` under the platform-reclaim or the
 *     generation exemption; a first placement gets 0. A failure does not change it. At a crash-class failure (a VMM
 *     crash, a guest reset that ends the VMM, a launch clash: neither exemption) the reconciler decides
 *     `retry = restart_decision(att, false, false)` = att < CEIL and `ex = not(retry)` (FinalizeFailed,
 *     BackoffExhausted; the workload is `failed`). A serve crash with a running VMM leaves an orphan that boot
 *     reclamation kills (StoppedBy::PlatformReclaimed): the workload waits whatever its count, with `chg` false.
 *     `Redeploy` (the restart verb's generation bump, from `done` or `failed`) is a generation replacement: the
 *     workload waits whatever its count, with `chg` false. `att` stops at the ceiling, so the state space is finite.
 *     CEIL is 5 on the conformance configuration (`lease_conf_ok`, `lease_five`, the production
 *     RESTART_BACKOFF_CEILING) and scaled to 2 on the model-checking instances (`lease_ok`, `lease_live`, the hazard
 *     `lease_bug*` instances); the predicate's shape is the same on every instance. With 5, the fifth crash-class
 *     failure (`att` = 4) is placed again and the sixth (`att` = 5) exhausts the budget.
 *   - A workload leased but not launched at a serve crash (feature-delta `cid_lease` item 3): its allocation has no
 *     row, so after the crash its current allocation is its predecessor; it goes to `wait` with the predecessor's
 *     count (`att - 1` when its placement charged, else `att`) and the same `chg`; a predecessor lease is lost with
 *     the pool. A generation replacement leased at the crash is placed again — the behaviour GH #325 is to deliver
 *     (RESTART-GEN-CRASH, user ruling 2026-10-18: production loses it today and #295 does not fix it; pending #325).
 *     Ghost `g.pendingGen` records the Redeploy's pre-state until the placed allocation's first row is observed at
 *     `Launch`; behaviourally inert in the design, it lets hazard `crashLosesPlacedGeneration` return the workload to
 *     that pre-state. Item 9: the recording conformance main `lease_conf_ok` does not enable `Crash` in that window;
 *     `lease_conf_325` keeps it and carries the pending run.
 *   - D-UP-20: `lease_live` is a model-checking instance (three abstract offsets), not a conformance target;
 *     the conformance mains `lease_conf_ok` and `lease_conf_325` (pending #325) use the five usable offsets of the
 *     /29 test pool.
 *   - D-UP-1 (conformance step record): on the conformance main `lease_conf_ok` (REC = true), `conf_step` records
 *     the action that produced the state, as a variant of `cid_lease_conf::ConfStep` whose tag is the action's name
 *     and whose value is the record of its parameters by parameter name (`init` and parameterless actions: the
 *     unit value). quint-connect reads the action and its picks from it (`Config::nondet`). Model-only and
 *     write-only; every other instance keeps it at `init`.
 *
 * OFF names the design rules switched off (teeth; instances in hazard/):
 *   releaseBeforeVmm    the lease is released while the VMM still holds the device
 *   assignBeforeReclaim assign runs before boot reclamation completed
 *   resetReboots        a guest reset reboots the guest inside the same VMM (no exit)
 *   reclaimNeverCompletes boot reclamation kills the orphans but never records that it completed, so `assign` stays disabled
 *   crashForgetsStop    a serve crash forgets the stop intent: a stopped workload waits to be placed again
 *   retryOffByOne       the full pre-fix `restart_status_for_alloc` as the restart decision: restart iff
 *                       attempts + 1 < CEILING, with no exemptions (reclaim and generation both dropped)
 *   reclaimNotExempt    only the platform-reclaim exemption removed: a reclaimed workload at the ceiling is finalised failed
 *   generationNotExempt only the generation exemption removed: `Redeploy` does not place a workload at the ceiling
 *   crashLosesPlacedGeneration  today's production (GH #325, pending): a generation replacement leased at a serve
 *                       crash is lost and the workload returns to its pre-Redeploy `done` / `failed`
 *   assignSplit         assign is not one step under the pool lock: the next-fit free offset is chosen in one step
 *                       and taken in a later one, without re-checking it is still free
 *   predecessorFirst    (item 7.2) the restart frees the crashed allocation's lease before it leases the successor
 *   predecessorCleanedAtAssign  (item 7.3) the predecessor is retired at the successor's assign, not after its launch
 *   refusalKeepsPredecessor     (item 7.4) a pool refusal leaves the predecessor held until the next launch
 *   terminalBeforeRelease       (item 7.5) FinalizeFailed writes the terminal before it releases the lease
 *   tickDuringPredecessorTeardown  (item 7.3 step boundary) a tick of the workload runs while its dispatch is held at a
 *                       predecessor teardown (the guard dropped from Assign, Stop and Finalize)
 *   vmmExitRetiresLease (item 7.1) the VMM's exit retires and releases its lease in the same step: the workload waits
 *                       with no lease (or, past the ceiling, is `ended` with none), so the restart keeps no predecessor
 *
 * ASSUMPTIONS:
 *   K-C1  A duplicate CID is refused at device creation and the first holder is unaffected; a CID is free
 *         right after the holder's last reference closes .............................. P-16
 *   K-C4  The VMM is the only holder of its vhost-vsock fd; no Overdrive process keeps a copy past VMM
 *         exit, so the device is released at VMM exit ................................ V-30 Q2c, Q2d
 *   K-C5  Boot reclamation kills every VMM the new process does not supervise before any `assign`, and a
 *         reclaimed VMM's device is released before its cgroup can be removed. Source: an exiting task drops
 *         its files (kernel/exit.c:971 @ v7.0) and runs its task work (kernel/exit.c:976 @ v7.0), where the
 *         final fput of a user task is deferred (fs/file_table.c:528-530 @ v7.0) and reaches
 *         vhost_vsock_dev_release; cgroup_rmdir waits in cgroup_drain_dying until every task has passed
 *         cgroup_task_dead (kernel/cgroup/cgroup.c:6309 @ v7.0). OPEN: the Tier-3 serve-restart
 *         reclamation scenario V-RECLAIM; not contract-shaping
 *   K-C6  A guest reset ends the VMM in the pinned fork ....................... ADR-0182 (fork validation item)
 */
// D-UP-1: the variants of `conf_step` (a module of its own: a constructor would clash with the action of the same
// name, so the actions build their variant with the builtin `variant`)
module cid_lease_conf {
  type ConfStep =
    | init
    | Assign({ a: int })
    | AssignRefused({ a: int })
    | AssignPick({ a: int })
    | AssignTake({ a: int })
    | Launch({ a: int })
    | Stop({ a: int })
    | VmmCrash({ a: int })
    | GuestReset({ a: int })
    | Finalize({ a: int })
    | Release({ a: int })
    | Redeploy({ a: int })
    | Crash
    | Restart
    | Reclaim
    | OrphanExit({ o: int })
}

module cid_lease {
  import cid_lease_conf.ConfStep from "./cid_lease"
  const OFF: Set[str]
  const CFG: str   // "ok" | "live" | "conf_ok" | "conf_325"
  // D-UP-1: true only on the module's conformance mains, where `conf_step` records the action taken; every
  // model-checking, liveness and hazard instance passes false and keeps `conf_step` at its initial value.
  const REC: bool

  // the conformance configuration (the five offsets of the /29 test pool, the production ceiling). `conf_325` is the
  // same configuration with the #325 window kept on its recording main (feature-delta `cid_lease` item 9)
  val CONF: bool = CFG.in(Set("conf_ok", "conf_325"))
  val LIVE: bool = CFG == "live"
  val OFFS: Set[int] = if (CONF) Set(0, 1, 2, 3, 4) else Set(0, 1, 2)
  val N: int = if (CONF) 5 else 3
  val ALLOCS: Set[int] = Set(1, 2)
  val BOOTS: int = if (LIVE) 1 else -1             // serve crash + restart (-1: unbounded)
  // RESTART_BACKOFF_CEILING (workload_lifecycle.rs:34): the production value on the conformance configuration,
  // scaled to 2 on the model-checking instances (the predicate's shape is unchanged)
  val CEIL: int = if (CONF) 5 else 2
  val BUG_RELEASE_BEFORE_VMM: bool = OFF.contains("releaseBeforeVmm")
  val BUG_ASSIGN_BEFORE_RECLAIM: bool = OFF.contains("assignBeforeReclaim")
  val BUG_RESET_REBOOTS: bool = OFF.contains("resetReboots")
  val BUG_ASSIGN_SPLIT: bool = OFF.contains("assignSplit")
  val BUG_RECLAIM_NEVER_COMPLETES: bool = OFF.contains("reclaimNeverCompletes")
  val BUG_CRASH_FORGETS_STOP: bool = OFF.contains("crashForgetsStop")
  val BUG_RETRY_OFF_BY_ONE: bool = OFF.contains("retryOffByOne")
  val BUG_RECLAIM_NOT_EXEMPT: bool = OFF.contains("reclaimNotExempt")
  val BUG_GENERATION_NOT_EXEMPT: bool = OFF.contains("generationNotExempt")
  val BUG_CRASH_LOSES_PLACED_GENERATION: bool = OFF.contains("crashLosesPlacedGeneration")
  // DR6-CLOSE: one hazard per item-7 rule
  val BUG_PREDECESSOR_FIRST: bool = OFF.contains("predecessorFirst")
  val BUG_PREDECESSOR_CLEANED_AT_ASSIGN: bool = OFF.contains("predecessorCleanedAtAssign")
  val BUG_REFUSAL_KEEPS_PREDECESSOR: bool = OFF.contains("refusalKeepsPredecessor")
  val BUG_TERMINAL_BEFORE_RELEASE: bool = OFF.contains("terminalBeforeRelease")
  val BUG_TICK_DURING_PREDECESSOR_TEARDOWN: bool = OFF.contains("tickDuringPredecessorTeardown")
  val BUG_VMM_EXIT_RETIRES_LEASE: bool = OFF.contains("vmmExitRetiresLease")

  // st (the placement lease's state, feature-delta `cid_lease` map): wait | picked (assignSplit only: an offset chosen,
  // not yet taken) | leased (lease, VMM not launched) | running (the VMM holds the device) | ended (the placement
  // allocation ended crash-class: the VMM has exited, its lease, if any (`off` >= 0), is still Admitted and no dispatch
  // has run for it yet) | retiring (lease Retiring, its dispatch held at the provisioner's teardown) | done (stopped) |
  // failed (restart budget exhausted, the terminal written)
  // retry: the allocation ended by a crash-class failure and the reconciler places the workload again; ex: it ended by a
  // crash-class failure and the reconciler finalises it failed (budget exhausted).
  // chg (model-only): in `wait` / `ended` / `retiring`, the next placement charges one (the predecessor is a crash-class
  // row); in `leased` / `picked`, the placement that took the lease charged one (so a serve crash before launch restores
  // the predecessor's count)
  // pred / pst (item 7): the predecessor lease of a crash-class restart — its offset (-1 none) and its state: held (still
  // Admitted, the restart dispatch has not cleaned it) | retiring (the dispatch's predecessor cleanup is held at its
  // teardown) | none. At most one per workload
  type A = { st: str, off: int, retry: bool, ex: bool, chg: bool, pred: int, pst: str }
  type Ghost = {
    clash: bool,       // a VMM found its CID's device held by another holder
    rebooted: bool,    // a guest reset left the VMM alive
    reclaimed: bool,   // an orphan was reclaimed (witness)
    stopped: Set[int], // workloads stopped and not redeployed since (the stop sentinel; model-only)
    // the workload's last ending: none | crash (crash-class failure) | reclaim (platform reclaim at a serve crash) |
    // redeploy (generation replacement) | stop
    lastEnd: int -> str,
    preEnd: int -> str,     // lastEnd before the latest Redeploy (crashLosesPlacedGeneration restores it)
    pendingGen: int -> str, // "" | the pre-state (done / failed) of a Redeploy whose placed allocation has no row yet
    fin: Set[int],          // workloads whose retiring lease is FinalizeFailed's (model-only, not projected: item 7)
  }

  var dev: int -> int
  var lease: int -> int          // offset -> holding allocation (0 = free); placement and predecessor leases alike
  var cursor: int
  var al: int -> A
  var up: bool                   // the serve process is up
  var reclaimed: bool            // boot reclamation of this process completed
  var bootLeft: int
  var g: Ghost
  var conf_step: ConfStep        // D-UP-1: the action that produced this state (model-only)
  var att: int -> int            // WorkloadLifecycleView::restart_counts of the workload's current allocation (persisted)

  val g0: Ghost = {
    clash: false, rebooted: false, reclaimed: false, stopped: Set(),
    lastEnd: ALLOCS.mapBy(a => "none"), preEnd: ALLOCS.mapBy(a => "none"), pendingGen: ALLOCS.mapBy(a => ""),
    fin: Set(),
  }
  // RESTART-SSOT: the reconciler's restart decision, `restart_decision(attempts, platform_reclaimed,
  // generation_replacement)` = attempts < RESTART_BACKOFF_CEILING || platform_reclaimed || generation_replacement
  // (workload_lifecycle.rs:1114-1119). retryOffByOne: the pre-fix accessor, attempts + 1 < CEILING, no exemptions;
  // reclaimNotExempt / generationNotExempt: one exemption removed
  def restartDecision(a: int, platformReclaimed: bool, generationReplacement: bool): bool =
    if (BUG_RETRY_OFF_BY_ONE) att.get(a) + 1 < CEIL
    else or {
      att.get(a) < CEIL,
      platformReclaimed and not(BUG_RECLAIM_NOT_EXEMPT),
      generationReplacement and not(BUG_GENERATION_NOT_EXEMPT),
    }
  // the placed allocation's restart count (workload_lifecycle.rs:1214-1221, :1339): + 1 after a crash-class row
  def placedAtt(a: int): int = if (al.get(a).chg) att.get(a) + 1 else att.get(a)
  // a launch clash (a crash-class failure, neither exemption) decided by the reconciler: the successor's start failed, and
  // `cleanup_restart_successor` retires its lease (action_shim/mod.rs:1734-1754); the predecessor lease is untouched
  def failedA(a: int): A = {
    val r = restartDecision(a, false, false)
    { ...al.get(a), st: "retiring", retry: r, ex: not(r), chg: r }
  }
  def endAs(a: int, e: str): int -> str = g.lastEnd.set(a, e)
  def isFree(o: int): bool = lease.get(o) == 0
  val freeOffs: Set[int] = OFFS.filter(o => isFree(o))
  def dist(o: int): int = (o - cursor + N) % N
  def nextFitIn(S: Set[int]): int = S.fold(-1, (best, o) =>
    if (best == -1) o else if (dist(o) < dist(best)) o else best)
  // the workload holds its placement lease (`off`) / a predecessor lease (`pred`)
  def hasOff(a: int): bool =
    al.get(a).st.in(Set("leased", "running", "retiring")) or (al.get(a).st == "ended" and al.get(a).off >= 0)
  def hasPred(a: int): bool = al.get(a).pst != "none"
  val orphans: Set[int] = OFFS.filter(o => dev.get(o) == -2)
  def devAt(o: int): int = if (OFFS.contains(o)) dev.get(o) else 0
  // the workload has a dispatch held at a predecessor teardown: no tick of its workload runs until that is released
  def predRetiring(a: int): bool = al.get(a).pst == "retiring"
  // the tick guard of a workload (Assign, Stop, Finalize): no tick runs while its dispatch is held at a predecessor
  // teardown. tickDuringPredecessorTeardown drops it
  def tickBlocked(a: int): bool = predRetiring(a) and not(BUG_TICK_DURING_PREDECESSOR_TEARDOWN)

  action keepAl = all { al' = al }

  action init = all {
    dev' = OFFS.mapBy(o => 0), lease' = OFFS.mapBy(o => 0), cursor' = 0,
    al' = ALLOCS.mapBy(a => { st: "wait", off: -1, retry: false, ex: false, chg: false, pred: -1, pst: "none" }),
    up' = true, reclaimed' = true, bootLeft' = BOOTS, g' = g0, att' = ALLOCS.mapBy(a => 0),
    conf_step' = variant("init", ()),
  }

  // ------------------------------------------------------------------ assign (pool lock)
  // A tick that places the workload: next-fit over the free offsets, after boot reclamation only. From `wait`
  // (StartAllocation, or RestartAllocation after a clash / a reclaim / a generation replacement), or from `ended` with
  // `retry` (RestartAllocation of the crashed row, item 7): its dispatch provisions the successor BEFORE it touches the
  // predecessor (action_shim/mod.rs:2707-2716, `pool.assign` at :1471), so the crashed allocation's lease stays
  // Admitted as the predecessor lease (`pst` = held). The placement sets the new allocation's restart count
  // (`placedAtt`: + 1 after a crash-class row, unchanged under an exemption, 0 for a first placement).
  // predecessorFirst: from `ended` the old lease is freed in the same step and next-fit runs over the free offsets with
  // it included; no predecessor lease is kept. predecessorCleanedAtAssign: from `ended` the predecessor is retired at
  // once (`pst` = retiring) instead of after the successor's launch
  action Assign(a: int): bool = all {
    not(BUG_ASSIGN_SPLIT),
    up, reclaimed or BUG_ASSIGN_BEFORE_RECLAIM, freeOffs != Set(), not(tickBlocked(a)),
    val x = al.get(a)
    val fromEnded = x.st == "ended" and x.retry and x.off >= 0
    val first = fromEnded and BUG_PREDECESSOR_FIRST
    val avail = if (first) freeOffs.union(Set(x.off)) else freeOffs
    all {
      x.st == "wait" or fromEnded,
      val o = nextFitIn(avail)
      all {
        lease' = (if (first) lease.set(x.off, 0) else lease).set(o, a), cursor' = (o + 1) % N,
        al' = al.set(a, {
          ...x, st: "leased", off: o, retry: false, ex: false,
          pred: if (fromEnded and not(first)) x.off else x.pred,
          pst:
            if (fromEnded and not(first)) (if (BUG_PREDECESSOR_CLEANED_AT_ASSIGN) "retiring" else "held")
            else x.pst,
        }),
      },
    },
    att' = att.set(a, placedAtt(a)),
    dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
    conf_step' = if (REC) variant("Assign", { a: a }) else conf_step,
  }
  // The same tick with no free offset: `pool.assign` returns PoolExhausted, an admission refusal
  // (action_shim/mod.rs:540-548), so the dispatch places nothing and runs the predecessor cleanup at once
  // (`finish_restart`, :2719-2731): the crashed allocation's lease is retired and the dispatch is held at its
  // teardown. The workload waits; its count is unchanged (the current row is still the predecessor's).
  // refusalKeepsPredecessor: the predecessor stays held, its cleanup deferred to the next launch
  action AssignRefused(a: int): bool = all {
    up, reclaimed, freeOffs == Set(),
    val x = al.get(a)
    all {
      x.st == "ended", x.retry, x.off >= 0, x.pst == "none",
      al' = al.set(a, { ...x, st: "wait", off: -1, retry: false, ex: false, chg: true, pred: x.off,
        pst: if (BUG_REFUSAL_KEEPS_PREDECESSOR) "held" else "retiring" }),
    },
    lease' = lease, cursor' = cursor, att' = att,
    dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
    conf_step' = if (REC) variant("AssignRefused", { a: a }) else conf_step,
  }

  // assignSplit: the walk picks the next-fit free offset in one step ...
  action AssignPick(a: int): bool = all {
    BUG_ASSIGN_SPLIT, up, reclaimed, al.get(a).st == "wait", freeOffs != Set(), not(predRetiring(a)),
    al' = al.set(a, { ...al.get(a), st: "picked", off: nextFitIn(freeOffs), retry: false, ex: false }),
    lease' = lease, cursor' = cursor, dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g, att' = att,
    conf_step' = if (REC) variant("AssignPick", { a: a }) else conf_step,
  }
  // ... and takes it in a later step, without re-checking that it is still free
  action AssignTake(a: int): bool = all {
    BUG_ASSIGN_SPLIT, up, al.get(a).st == "picked",
    val o = al.get(a).off
    all { lease' = lease.set(o, a), cursor' = (o + 1) % N, al' = al.set(a, { ...al.get(a), st: "leased" }) },
    att' = att.set(a, placedAtt(a)),
    dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
    conf_step' = if (REC) variant("AssignTake", { a: a }) else conf_step,
  }

  // ------------------------------------------------------------------ launch (VM driver, VMM)
  // The held `provision` returns and the dispatch starts the VMM, which opens /dev/vhost-vsock for the leased CID itself.
  // A free device: the VMM runs and the new allocation's row is written; for a restart the dispatch then runs the
  // predecessor cleanup (`finish_restart` -> `cleanup_restart_predecessor`, action_shim/mod.rs:1760-1789, :1808), which
  // retires the predecessor lease and is held at its teardown (`pst` held -> retiring). A held device refuses the open:
  // the VMM exits before READY, the start fails and `cleanup_restart_successor` retires the new lease (a crash-class
  // failure the reconciler decides on); the predecessor stays held until that teardown is released. Either way the
  // placed allocation now has a row, so a pending generation is observed
  action Launch(a: int): bool = all {
    up, al.get(a).st == "leased",
    val x = al.get(a)
    if (dev.get(x.off) == 0) all {
      dev' = dev.set(x.off, a),
      al' = al.set(a, { ...x, st: "running", chg: false, pst: if (x.pst == "held") "retiring" else x.pst }),
      g' = { ...g, pendingGen: g.pendingGen.set(a, "") },
    } else all {
      dev' = dev, al' = al.set(a, failedA(a)),
      g' = { ...g, clash: true, lastEnd: endAs(a, "crash"), pendingGen: g.pendingGen.set(a, "") },
    },
    att' = att, lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft,
    conf_step' = if (REC) variant("Launch", { a: a }) else conf_step,
  }
  // The VMM fails (a crash, or a guest reset: RESET-EXIT): the last close of its device frees the CID and the exit
  // observer writes the terminal row. No convergence tick runs in this step (item 7): below the attachment cap no
  // dispatch retires a crashed allocation's lease (the reclaim pass skips the pending restart owner,
  // workload_lifecycle.rs:399-478), so the lease stays Admitted (`ended`). `retry` / `ex` are the reconciler's restart
  // decision on that row (no exemption); the next tick is the workload's Assign, AssignRefused, Finalize or Stop.
  // vmmExitRetiresLease (item 7.1 off): the exit retires and releases the lease in the same step, so the workload waits
  // with no lease (`retry`) or is `ended` with none (`ex`), and its restart holds no predecessor
  action vmmExit(a: int): bool = all {
    val x = al.get(a)
    val r = restartDecision(a, false, false)
    all {
      dev' = if (dev.get(x.off) == a) dev.set(x.off, 0) else dev,
      if (BUG_VMM_EXIT_RETIRES_LEASE) all {
        lease' = lease.set(x.off, 0),
        al' = al.set(a, { ...x, st: if (r) "wait" else "ended", off: -1, retry: false, ex: not(r), chg: r }),
      } else all {
        lease' = lease,
        al' = al.set(a, { ...x, st: "ended", retry: r, ex: not(r), chg: r }),
      },
    },
    cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, att' = att,
    g' = { ...g, lastEnd: endAs(a, "crash") },
  }
  // the stop handler, then one tick. From `running`: the dispatch retires the lease, stops the VMM (its device is
  // released) and is held at the teardown. From `ended` with its lease: under a stop intent the reclaim pass emits
  // ReclaimAllocationNetwork for the ended allocation (workload_lifecycle.rs:405-407, :446-510), whose dispatch retires
  // the lease and is held at the teardown (action_shim/mod.rs:1609-1652)
  action Stop(a: int): bool = all {
    up, not(tickBlocked(a)),
    val x = al.get(a)
    all {
      x.st == "running" or (x.st == "ended" and x.off >= 0),
      dev' = if (x.st == "running" and dev.get(x.off) == a) dev.set(x.off, 0) else dev,
      al' = al.set(a, { ...x, st: "retiring", retry: false, ex: false, chg: false }),
    },
    lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, att' = att,
    g' = { ...g, stopped: g.stopped.union(Set(a)), lastEnd: endAs(a, "stop") },
    conf_step' = if (REC) variant("Stop", { a: a }) else conf_step,
  }
  // the VMM exits before READY or later by itself (a crash)
  action VmmCrash(a: int): bool = all {
    up, al.get(a).st == "running", vmmExit(a), conf_step' = if (REC) variant("VmmCrash", { a: a }) else conf_step,
  }
  // RESET-EXIT: a guest reset ends the VMM. resetReboots: the guest reboots inside the same VMM; nothing exits
  // and the restart budget is never charged
  action GuestReset(a: int): bool = all {
    up, al.get(a).st == "running",
    if (BUG_RESET_REBOOTS) all {
      g' = { ...g, rebooted: true }, att' = att,
      dev' = dev, al' = al, lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft,
    } else vmmExit(a),
    conf_step' = if (REC) variant("GuestReset", { a: a }) else conf_step,
  }
  // At the ceiling, the tick emits FinalizeFailed before any backoff check (workload_lifecycle.rs:1116-1179). With a
  // lease its dispatch retires it and is held at the teardown; the terminal BackoffExhausted is written only after the
  // teardown and the release (action_shim/mod.rs:2146-2176). Without one (after a serve crash) the terminal is written
  // at once. Like every tick it runs only after boot reclamation (run_server calls vm_reclamation_boot::converge,
  // lib.rs:3955, before it spawns the convergence loop, :4212); assignBeforeReclaim does not bypass this guard. The
  // guard (here and on Redeploy) keeps randomized conformance traces to production's boot order; it is not load-bearing
  // for Safety (no CID is touched: after a serve crash no workload holds a lease, and Assign still requires `reclaimed`)
  // terminalBeforeRelease: with a lease the terminal is written at once, the lease released by a later Release
  action Finalize(a: int): bool = all {
    up, reclaimed, not(tickBlocked(a)),
    val x = al.get(a)
    all {
      x.st == "ended", x.ex,
      if (x.off >= 0 and BUG_TERMINAL_BEFORE_RELEASE) all {
        al' = al.set(a, { ...x, st: "failed", retry: false, ex: false, chg: false }), g' = g,
      } else if (x.off >= 0) all {
        al' = al.set(a, { ...x, st: "retiring" }), g' = { ...g, fin: g.fin.union(Set(a)) },
      } else all {
        al' = al.set(a, { ...x, st: "failed", retry: false, ex: false, chg: false }), g' = g,
      },
    },
    lease' = lease, cursor' = cursor, dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, att' = att,
    conf_step' = if (REC) variant("Finalize", { a: a }) else conf_step,
  }
  // A held teardown returns and the dispatch runs `pool.release` (action_shim/mod.rs:1596-1600: release only after
  // teardown, which runs only after the VMM exited). The placement lease's teardown (`retiring`) is released first: the
  // workload then waits (a clash with `retry`; its next placement charges), is `ended` with no lease (a clash with `ex`:
  // FinalizeFailed has not run), is `failed` (FinalizeFailed's teardown: the terminal is written) or is done (a stop);
  // a predecessor still held (the clash path) is then retired, its cleanup held at its teardown. Otherwise the
  // predecessor's teardown (`pst` = retiring) is released: its offset is freed.
  // releaseBeforeVmm: the placement lease is released while the VMM still holds the device
  action Release(a: int): bool = all {
    up,
    val x = al.get(a)
    if (x.st == "retiring" or (BUG_RELEASE_BEFORE_VMM and x.st == "running" and x.pst != "retiring")) all {
      devAt(x.off) != a or BUG_RELEASE_BEFORE_VMM,
      lease' = lease.set(x.off, 0),
      al' = al.set(a, {
        ...x,
        st:
          if (x.st == "running" or x.retry) "wait"
          else if (x.ex) (if (g.fin.contains(a)) "failed" else "ended")
          else "done",
        off: -1, retry: false, ex: x.st == "retiring" and not(x.retry) and x.ex and not(g.fin.contains(a)),
        chg: x.st == "retiring" and x.retry,
        pst: if (x.pst == "held") "retiring" else x.pst,
      }),
      g' = { ...g, fin: g.fin.exclude(Set(a)) },
    } else if (BUG_TERMINAL_BEFORE_RELEASE and x.st == "failed" and x.off >= 0) all {
      // terminalBeforeRelease only: the lease of a workload whose terminal was written first is released afterwards
      lease' = lease.set(x.off, 0), al' = al.set(a, { ...x, off: -1 }), g' = g,
    } else all {
      x.pst == "retiring", devAt(x.pred) != a,
      lease' = lease.set(x.pred, 0),
      al' = al.set(a, { ...x, pred: -1, pst: "none" }),
      g' = g,
    },
    dev' = dev, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, att' = att,
    conf_step' = if (REC) variant("Release", { a: a }) else conf_step,
  }
  // the restart verb's intent write (the stop sentinel deleted, the generation bumped) for a stopped or a failed workload:
  // a generation replacement, so the reconciler places it again whatever its count, which it keeps (`chg` false).
  // generationNotExempt / retryOffByOne: the reconciler does not place a workload past the plain decision (no transition).
  // Like Finalize it runs only after boot reclamation: the HTTP listener that serves the restart verb binds
  // (lib.rs:4368) after run_server's vm_reclamation_boot::converge (:3955)
  action Redeploy(a: int): bool = all {
    up, reclaimed, al.get(a).st.in(Set("done", "failed")), restartDecision(a, false, true),
    al' = al.set(a, { ...al.get(a), st: "wait", off: -1, retry: false, ex: false, chg: false }), att' = att,
    dev' = dev, lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft,
    g' = {
      ...g, stopped: g.stopped.exclude(Set(a)), lastEnd: endAs(a, "redeploy"), preEnd: g.preEnd.set(a, g.lastEnd.get(a)),
      pendingGen: g.pendingGen.set(a, al.get(a).st),
    },
    conf_step' = if (REC) variant("Redeploy", { a: a }) else conf_step,
  }

  // ------------------------------------------------------------------ environment: serve crash, restart, reclamation
  // serve crashes: the pool, its cursor, every held dispatch and the in-memory placement are lost (predecessor leases
  // included); every running VMM survives as an orphan holding its device; allocations placed but not launched vanish.
  // The placement intent is persisted (the node's intent store survives, T-5 ruling 2026-10-18): a stopped workload —
  // done, or stopped and not yet released (retiring, neither retry nor exhausted) — keeps its stop sentinel and stays
  // done. A failed workload keeps its terminal row and its persisted restart count, and stays failed. An exhausted
  // workload whose terminal is not written yet (`ended` with `ex`, or FinalizeFailed held before the terminal write) is
  // `ended` with no lease: the new process's reconciler emits FinalizeFailed again. A crashed allocation the reconciler
  // places again (`ended` with `retry`) waits, its next placement charging. A running VMM survives as an orphan that boot
  // reclamation kills: its allocation is platform-reclaimed, so the reconciler places the workload again whatever its
  // count (the exemption; the count is kept, `chg` false). A leased, not launched workload goes back to its
  // predecessor's count and `chg` (feature-delta `cid_lease` item 3); a generation replacement among them is placed
  // again (the behaviour GH #325 delivers; production loses it today) unless crashLosesPlacedGeneration.
  // Every other workload waits to be placed again, with its `chg`
  def stopped(a: int): bool = al.get(a).st == "done" or (al.get(a).st == "retiring" and not(al.get(a).retry) and not(al.get(a).ex))
  def exhausted(a: int): bool =
    al.get(a).st == "failed" or (al.get(a).st.in(Set("retiring", "ended")) and al.get(a).ex)
  def lostGen(a: int): bool = BUG_CRASH_LOSES_PLACED_GENERATION and al.get(a).st == "leased" and g.pendingGen.get(a) != ""
  def crashA(a: int): A = {
    val x = al.get(a)
    val st2 =
      if (stopped(a) and not(BUG_CRASH_FORGETS_STOP)) "done"
      else if (x.st == "failed") "failed"
      else if (exhausted(a)) "ended"
      else if (x.st == "running") (if (restartDecision(a, true, false)) "wait" else "failed")
      else if (lostGen(a)) g.pendingGen.get(a)
      else "wait"
    {
      st: st2, off: -1, retry: false, ex: st2 == "ended",
      chg: st2 == "wait" and x.st != "running" and x.chg, pred: -1, pst: "none",
    }
  }
  def crashAtt(a: int): int = if (al.get(a).st == "leased" and al.get(a).chg) att.get(a) - 1 else att.get(a)
  def crashEnd(a: int): str =
    if (al.get(a).st == "running") "reclaim" else if (lostGen(a)) g.preEnd.get(a) else g.lastEnd.get(a)
  // feature-delta `cid_lease` item 9, pending GH #325: production loses a generation replacement leased at a serve
  // crash, the model places it again; so the recording conformance main `lease_conf_ok` does not enable Crash in that
  // window (no randomized conformance trace replays it). `lease_conf_325` keeps the window; #325 deletes this guard
  def inGenWindow(a: int): bool = al.get(a).st == "leased" and g.pendingGen.get(a) != ""
  action Crash = all {
    up, bootLeft != 0,
    not(CFG == "conf_ok" and REC) or ALLOCS.forall(a => not(inGenWindow(a))),
    up' = false, reclaimed' = false,
    dev' = OFFS.mapBy(o => if (dev.get(o) > 0) -2 else dev.get(o)),
    lease' = OFFS.mapBy(o => 0), cursor' = 0,
    al' = al.keys().mapBy(a => crashA(a)),
    att' = al.keys().mapBy(a => crashAtt(a)),
    bootLeft' = if (bootLeft < 0) bootLeft else bootLeft - 1,
    g' = {
      ...g, lastEnd: al.keys().mapBy(a => crashEnd(a)),
      pendingGen: al.keys().mapBy(a => if (lostGen(a)) "" else g.pendingGen.get(a)),
      fin: Set(),
    },
    conf_step' = if (REC) variant("Crash", ()) else conf_step,
  }
  // the new process starts; boot reclamation is still to run
  action Restart = all {
    not(up),
    up' = true, reclaimed' = (orphans == Set()),
    dev' = dev, lease' = lease, cursor' = cursor, al' = al, bootLeft' = bootLeft, g' = g, att' = att,
    conf_step' = if (REC) variant("Restart", ()) else conf_step,
  }
  // boot reclamation (one vm_reclamation_boot::converge call, D-UP-19) kills every orphan VMM it does not supervise;
  // their devices are released. reclaimNeverCompletes: reclamation never reports completion
  action Reclaim = all {
    up, not(reclaimed), orphans != Set(),
    dev' = OFFS.mapBy(o => if (dev.get(o) == -2) 0 else dev.get(o)), conf_step' = if (REC) variant("Reclaim", ()) else conf_step,
    reclaimed' = not(BUG_RECLAIM_NEVER_COMPLETES),
    lease' = lease, cursor' = cursor, al' = al, up' = up, bootLeft' = bootLeft, g' = { ...g, reclaimed: true }, att' = att,
  }
  // an orphan exits by itself before it is reclaimed (a guest reset ends the VMM)
  action OrphanExit(o: int): bool = all {
    dev.get(o) == -2,
    dev' = dev.set(o, 0), conf_step' = if (REC) variant("OrphanExit", { o: o }) else conf_step,
    reclaimed' = (reclaimed or (up and orphans.exclude(Set(o)) == Set())),
    lease' = lease, cursor' = cursor, al' = al, up' = up, bootLeft' = bootLeft, g' = g, att' = att,
  }

  action step = {
    nondet a = ALLOCS.oneOf()
    nondet o = OFFS.oneOf()
    any {
      Assign(a), AssignRefused(a), AssignPick(a), AssignTake(a), Launch(a), Stop(a), VmmCrash(a), GuestReset(a),
      Finalize(a), Release(a), Redeploy(a), Crash, Restart, Reclaim, OrphanExit(o),
    }
  }

  // ------------------------------------------------------------------ invariants
  // Held leases' CIDs are unique: every placement lease (`off`) and every predecessor lease (`pred`), within one
  // workload and across workloads (lease uniqueness is CID uniqueness).
  val CidUnique: bool = ALLOCS.forall(a => and {
    (hasOff(a) and hasPred(a)) implies al.get(a).off != al.get(a).pred,
    ALLOCS.forall(b => a != b implies and {
      (hasOff(a) and hasOff(b)) implies al.get(a).off != al.get(b).off,
      (hasOff(a) and hasPred(b)) implies al.get(a).off != al.get(b).pred,
      (hasPred(a) and hasPred(b)) implies al.get(a).pred != al.get(b).pred,
    }),
  })
  // no VMM launch fails on a CID clash — including across serve crash and restart
  val NoLaunchClash: bool = not(g.clash)
  // the lease outlives the VMM: a running VMM's offset stays leased until the VMM has exited (release-last)
  val LeaseOutlivesVmm: bool = OFFS.forall(o => dev.get(o) > 0 implies lease.get(o) == dev.get(o))
  // assign runs only after boot reclamation: no lease is on an offset a previous process's VMM still holds
  val AssignAfterReclaim: bool = OFFS.forall(o => lease.get(o) != 0 implies dev.get(o) != -2)
  // RESET-EXIT: a guest reset never leaves the VMM alive
  val GuestResetEndsVmm: bool = not(g.rebooted)
  // T-5 ruling: a stopped workload is never placed again until it is redeployed, across serve crashes included (the
  // stop intent is persisted)
  val StoppedWorkloadNotPlaced: bool = ALLOCS.forall(a => g.stopped.contains(a) implies al.get(a).st.in(Set("retiring", "done")))
  // RESTART-SSOT (the reconciler's restart decision): a workload is finalised failed only once its restart count reached
  // the ceiling
  val ExhaustsOnlyAtCeiling: bool = ALLOCS.forall(a => exhausted(a) implies att.get(a) >= CEIL)
  // RESTART-SSOT: an exhausted workload's last ending is a crash-class failure — never a platform reclamation and never a
  // Redeploy (both exemptions place the workload again whatever its count)
  val ExhaustsOnlyOnACrashClassFailure: bool = ALLOCS.forall(a => exhausted(a) implies g.lastEnd.get(a) == "crash")
  // DR6-CLOSE (item 7.5): a failed workload holds no lease — FinalizeFailed retires and releases the lease before it
  // writes the terminal
  val FailedHoldsNoLease: bool = ALLOCS.forall(a => al.get(a).st == "failed" implies OFFS.forall(o => lease.get(o) != a))
  // DR6-CLOSE: every leased offset is its holder's placement lease (`off`) or predecessor lease (`pred`); a lease no
  // workload tracks is a leaked CID
  val NoUntrackedLease: bool = OFFS.forall(o => lease.get(o) != 0 implies {
    val b = lease.get(o)
    (hasOff(b) and al.get(b).off == o) or (hasPred(b) and al.get(b).pred == o)
  })
  val Safety: bool = and {
    CidUnique, NoLaunchClash, LeaseOutlivesVmm, AssignAfterReclaim, GuestResetEndsVmm, StoppedWorkloadNotPlaced,
    ExhaustsOnlyAtCeiling, ExhaustsOnlyOnACrashClassFailure, FailedHoldsNoLease, NoUntrackedLease,
  }

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Deterministic scenarios, each ending in an `expect` of its outcome with `Safety` checked along the way; the main
  // each runs on is in checks.toml (`lease_conf_ok` unless stated).
  def st(a: int): str = al.get(a).st
  def pstOf(a: int): str = al.get(a).pst
  def offOf(a: int): int = al.get(a).off
  // happy path: next-fit assign, then the VMM opens its own device (the VMM is the only holder)
  run AssignThenVmmOpensItsOwnDevice = init
    .then(Assign(1)).expect(Safety and st(1) == "leased" and al.get(1).off == 0 and cursor == 1 and lease.get(0) == 1)
    .then(Launch(1)).expect(Safety and st(1) == "running" and dev.get(0) == 1)
  // the lease is released only after the VMM has exited: a release while the VMM runs is refused; a stop exits
  // the VMM (device released), then the release frees the offset
  run ReleaseWhileTheVmmRunsIsRefused = AssignThenVmmOpensItsOwnDevice
    .then(Release(1)).fail()
  run StopExitsTheVmmThenReleaseIsLast = AssignThenVmmOpensItsOwnDevice
    .then(Stop(1)).expect(Safety and st(1) == "retiring" and dev.get(0) == 0 and lease.get(0) == 1)
    .then(Release(1)).expect(Safety and st(1) == "done" and lease.get(0) == 0)
    .then(Redeploy(1)).expect(Safety and st(1) == "wait")
  // next-fit continues from the cursor and skips no free offset
  run NextFitContinuesFromTheCursor = init
    .then(Assign(1)).then(Assign(2)).expect(Safety and al.get(1).off == 0 and al.get(2).off == 1 and cursor == 2)
    .then(Launch(1)).then(Launch(2)).then(Stop(1)).then(Release(1)).then(Redeploy(1)).then(Assign(1))
    .expect(Safety and al.get(1).off == 2 and cursor == 3 % N)
  // RESET-EXIT: a guest reset ends the VMM and its device is released; the crashed allocation's lease stays Admitted
  // (no dispatch has run), the restart leases the successor first (the old lease is the predecessor, held), the
  // successor launches on a fresh VMM and the dispatch then retires the predecessor, whose release frees it
  run GuestResetEndsTheVmmAndPlacementRetries = init
    .then(Assign(1)).then(Launch(1)).then(GuestReset(1))
    .expect(Safety and st(1) == "ended" and al.get(1).retry and dev.get(0) == 0 and lease.get(0) == 1 and not(g.rebooted))
    .then(Assign(1))
    .expect(Safety and st(1) == "leased" and offOf(1) == 1 and al.get(1).pred == 0 and pstOf(1) == "held" and att.get(1) == 1)
    .then(Launch(1))
    .expect(Safety and st(1) == "running" and dev.get(1) == 1 and pstOf(1) == "retiring" and lease.get(0) == 1 and not(g.clash))
    .then(Release(1)).expect(Safety and st(1) == "running" and pstOf(1) == "none" and lease.get(0) == 0)
  // the VMM crashes: the device closes, the lease stays Admitted until the restart dispatch's predecessor cleanup
  run VmmCrashFreesItsCidAndPlacementRetries = init
    .then(Assign(1)).then(Launch(1)).then(VmmCrash(1))
    .expect(Safety and st(1) == "ended" and al.get(1).retry and dev.get(0) == 0 and lease.get(0) == 1)
    .then(Assign(1))
    .expect(Safety and st(1) == "leased" and offOf(1) == 1 and al.get(1).pred == 0 and pstOf(1) == "held" and att.get(1) == 1)
    .then(Launch(1))
    .expect(Safety and st(1) == "running" and dev.get(1) == 1 and pstOf(1) == "retiring" and lease.get(0) == 1)
    .then(Release(1)).expect(Safety and st(1) == "running" and pstOf(1) == "none" and lease.get(0) == 0)
  // NO-SURVIVE: serve crashes with a running VMM; the VMM survives as an orphan holding its device; the restarted
  // process reclaims it BEFORE any assign, and the next lease of that CID launches without a clash
  run ServeCrashOrphanIsReclaimedBeforeAnyAssign = init
    .then(Assign(1)).then(Launch(1)).then(Crash)
    .expect(Safety and not(up) and dev.get(0) == -2 and lease == OFFS.mapBy(o => 0) and cursor == 0 and st(1) == "wait")
    .then(Restart).expect(Safety and up and not(reclaimed) and orphans == Set(0))
    .then(Assign(1)).fail()
  run ReclamationReleasesTheOrphanDeviceThenAssignProceeds = init
    .then(Assign(1)).then(Launch(1)).then(Crash).then(Restart)
    .then(Reclaim).expect(Safety and reclaimed and dev.get(0) == 0 and g.reclaimed)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and al.get(1).off == 0)
    .then(Launch(1)).expect(Safety and st(1) == "running" and dev.get(0) == 1 and not(g.clash))
  // D-UP-19: two orphans; one boot reclamation (one converge call) kills both before any assign, then assign proceeds
  run ReclamationKillsEveryOrphanBeforeAssign = init
    .then(Assign(2)).then(Assign(1)).then(Launch(2)).then(Launch(1))
    .expect(Safety and dev.get(0) == 2 and dev.get(1) == 1)
    .then(Crash).then(Restart).expect(Safety and not(reclaimed) and orphans == Set(0, 1))
    .then(Reclaim).expect(Safety and reclaimed and orphans == Set() and dev == OFFS.mapBy(o => 0))
    .then(Assign(1)).expect(Safety and st(1) == "leased")
  // an orphan exits by itself (a guest reset) before reclamation reaches it; reclamation completes with nothing to kill
  run OrphanExitsByItselfBeforeReclamation = init
    .then(Assign(1)).then(Launch(1)).then(Crash).then(Restart).then(OrphanExit(0))
    .expect(Safety and reclaimed and dev.get(0) == 0)
    .then(Assign(2)).expect(Safety and st(2) == "leased")
  // T-5 ruling: serve crashes after a workload was stopped and before its lease was released; the stop is persisted
  // intent, so the restarted process never places it again until it is redeployed
  run StoppedWorkloadStaysStoppedAcrossServeCrash = init
    .then(Assign(1)).then(Launch(1)).then(Stop(1)).expect(Safety and st(1) == "retiring" and not(al.get(1).retry))
    .then(Crash).expect(Safety and st(1) == "done" and lease == OFFS.mapBy(o => 0))
    .then(Restart).expect(Safety and up and reclaimed)
    .then(Assign(1)).fail()
  run StoppedWorkloadIsPlacedAgainOnlyAfterRedeploy = init
    .then(Assign(1)).then(Launch(1)).then(Stop(1)).then(Crash).then(Restart).expect(Safety and st(1) == "done")
    .then(Redeploy(1)).expect(Safety and st(1) == "wait")
    .then(Assign(1)).expect(Safety and st(1) == "leased" and al.get(1).off == 0)
  // a crash with a lease that was never launched: nothing survives, the restarted pool is empty and assigns at once
  run ServeCrashWithUnlaunchedLeaseLeavesNothingToReclaim = init
    .then(Assign(1)).then(Crash)
    .expect(Safety and dev == OFFS.mapBy(o => 0) and lease == OFFS.mapBy(o => 0))
    .then(Restart).expect(Safety and up and reclaimed)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and al.get(1).off == 0)

  // RESTART-SSOT (GH #324), on the conformance main (CEIL = 5, the production RESTART_BACKOFF_CEILING). The count of the
  // n-th placement after n - 1 crash-class failures is n - 1; `att` changes at placement, not at the failure. Each
  // restart is item 7's sequence: VmmCrash (ended, lease Admitted), Assign (the successor, the old lease held as the
  // predecessor), Launch (the predecessor retiring), Release (the predecessor freed). After four restarts the workload
  // runs at att = CEIL - 1 = 4; the fifth failure there (where the pre-fix accessor reported no retry) is placed again,
  // and the successor leases with att = CEIL
  run FifthCrashAtCeilingBoundaryIsPlacedAgain = init
    .then(Assign(1)).then(Launch(1)).expect(Safety and st(1) == "running" and att.get(1) == 0)
    .then((CEIL - 1).reps(_ =>
      VmmCrash(1).expect(Safety and st(1) == "ended" and al.get(1).retry and not(al.get(1).ex) and lease.get(offOf(1)) == 1)
        .then(Assign(1)).expect(Safety and st(1) == "leased" and pstOf(1) == "held")
        .then(Launch(1)).expect(Safety and st(1) == "running" and pstOf(1) == "retiring")
        .then(Release(1)).expect(Safety and st(1) == "running" and pstOf(1) == "none")))
    .expect(Safety and st(1) == "running" and att.get(1) == CEIL - 1 and pstOf(1) == "none")
    .then(VmmCrash(1))
    .expect(Safety and st(1) == "ended" and al.get(1).retry and not(al.get(1).ex) and att.get(1) == CEIL - 1
      and lease.get(offOf(1)) == 1)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and att.get(1) == CEIL and pstOf(1) == "held")
  // continues: the sixth failure, at att = CEIL, exhausts the budget; FinalizeFailed retires the lease, its release is
  // followed by the terminal write, and the workload is never placed again
  run FailureAtCeilingExhaustsTheBudget = FifthCrashAtCeilingBoundaryIsPlacedAgain
    .then(Launch(1)).then(Release(1)).expect(Safety and st(1) == "running" and pstOf(1) == "none")
    .then(VmmCrash(1))
    .expect(Safety and st(1) == "ended" and not(al.get(1).retry) and al.get(1).ex and att.get(1) == CEIL)
    .then(Finalize(1)).expect(Safety and st(1) == "retiring" and al.get(1).ex)
    .then(Release(1)).expect(Safety and st(1) == "failed" and offOf(1) == -1 and lease == OFFS.mapBy(o => 0))
    .then(Assign(1)).fail()
  // continues (generation replacement): Redeploy places the exhausted workload again with its count, so its next
  // crash-class failure exhausts the budget at once
  run RedeployPlacesAnExhaustedWorkloadAgainWithItsCount = FifthCrashAtCeilingBoundaryIsPlacedAgain
    .then(Launch(1)).then(Release(1)).then(VmmCrash(1)).then(Finalize(1)).then(Release(1))
    .expect(Safety and st(1) == "failed")
    .then(Redeploy(1)).expect(Safety and st(1) == "wait" and not(al.get(1).chg) and att.get(1) == CEIL)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and att.get(1) == CEIL and pstOf(1) == "none")
    .then(Launch(1)).then(VmmCrash(1)).expect(Safety and st(1) == "ended" and al.get(1).ex and att.get(1) == CEIL)
    .then(Finalize(1)).then(Release(1)).expect(Safety and st(1) == "failed")
  // platform reclaim: serve crashes while a workload at the ceiling runs (its predecessor's cleanup still held); boot
  // reclamation kills its orphan, and the reconciler places it again whatever its count, which it keeps
  run ServeCrashPlacesAWorkloadAtTheCeilingAgain = FifthCrashAtCeilingBoundaryIsPlacedAgain
    .then(Launch(1)).expect(Safety and st(1) == "running" and att.get(1) == CEIL and pstOf(1) == "retiring")
    .then(Crash)
    .expect(Safety and st(1) == "wait" and not(al.get(1).chg) and att.get(1) == CEIL and pstOf(1) == "none"
      and lease == OFFS.mapBy(o => 0))
    .then(Restart).then(Reclaim).expect(Safety and reclaimed)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and att.get(1) == CEIL)
  // continues: a second reclamation past the ceiling charges nothing either; the next crash-class failure exhausts at once
  run ReclaimedWorkloadPastTheCeilingIsPlacedAgain = ServeCrashPlacesAWorkloadAtTheCeilingAgain
    .then(Launch(1)).then(Crash).then(Restart).then(Reclaim)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and att.get(1) == CEIL)
    .then(Launch(1)).then(VmmCrash(1)).expect(Safety and st(1) == "ended" and al.get(1).ex and att.get(1) == CEIL)
  // feature-delta `cid_lease` item 3: a successor leased but not launched at a serve crash has no row; after the restart
  // the workload's current allocation is its predecessor, and the next placement gives the successor the same count.
  // The predecessor lease is lost with the pool
  run ServeCrashBeforeLaunchPlacesTheSuccessorAgainWithItsCount = init
    .then(Assign(1)).then(Launch(1)).then(VmmCrash(1))
    .expect(Safety and st(1) == "ended" and al.get(1).retry and att.get(1) == 0)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and att.get(1) == 1 and pstOf(1) == "held")
    .then(Crash)
    .expect(Safety and st(1) == "wait" and al.get(1).chg and att.get(1) == 0 and pstOf(1) == "none"
      and lease == OFFS.mapBy(o => 0))
    .then(Restart).expect(Safety and up and reclaimed)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and att.get(1) == 1)
  // item 7: serve crashes while FinalizeFailed's dispatch is held before its terminal write; the restarted process's
  // reconciler emits FinalizeFailed again, which writes the terminal at once (no lease is left)
  run ServeCrashDuringFinalizeFinalizesAfterRestart = FifthCrashAtCeilingBoundaryIsPlacedAgain
    .then(Launch(1)).then(Release(1)).then(VmmCrash(1)).expect(Safety and st(1) == "ended" and al.get(1).ex)
    .then(Finalize(1)).expect(Safety and st(1) == "retiring" and al.get(1).ex)
    .then(Crash)
    .expect(Safety and st(1) == "ended" and al.get(1).ex and offOf(1) == -1 and lease == OFFS.mapBy(o => 0))
    .then(Restart).expect(Safety and up and reclaimed)
    .then(Finalize(1)).expect(Safety and st(1) == "failed" and offOf(1) == -1)
    .then(Assign(1)).fail()
  // item 7: a workload stopped after its VMM crashed and before the restart tick: the reclaim pass reclaims the ended
  // allocation's lease (no VMM to stop), and its release leaves the workload done with no lease
  run StopOfAnEndedWorkloadReclaimsItsLease = init
    .then(Assign(1)).then(Launch(1)).then(VmmCrash(1))
    .expect(Safety and st(1) == "ended" and lease.get(0) == 1 and dev.get(0) == 0)
    .then(Stop(1)).expect(Safety and st(1) == "retiring" and not(al.get(1).retry) and not(al.get(1).ex) and lease.get(0) == 1)
    .then(Release(1)).expect(Safety and st(1) == "done" and offOf(1) == -1 and lease == OFFS.mapBy(o => 0))
    .then(Assign(1)).fail()
  // PENDING GH #325 (checked on `lease_conf_325`; production loses this placement today): a Redeploy's placement leased
  // at a serve crash is not lost; the restarted process places it again
  run RedeployLeasedAtServeCrashIsPlacedAgain = init
    .then(Assign(1)).then(Launch(1)).then(Stop(1)).then(Release(1)).expect(Safety and st(1) == "done")
    .then(Redeploy(1)).then(Assign(1)).expect(Safety and st(1) == "leased" and g.pendingGen.get(1) == "done")
    .then(Crash).expect(Safety and st(1) == "wait")
    .then(Restart).then(Assign(1)).expect(Safety and st(1) == "leased")
  // item 9 (DR6-CLOSE), the #325-window guard's own check: a Redeploy's placement is leased and not launched
  // (`g.pendingGen` set), and a serve crash is not enabled. Holds on `lease_conf_ok` (the guard refuses Crash) and is
  // violated on `lease_conf_325` (no guard: Crash is enabled). The expect before Crash establishes every other
  // precondition of Crash (`up`, `bootLeft != 0`), so the only thing that can refuse it is the window guard; the positive
  // twin below pins the violation on `lease_conf_325` to Crash being enabled. Model-only, not a #[quint_test]; deleted
  // with the guard
  run CrashIsNotEnabledInThe325Window = init
    .then(Assign(1)).then(Launch(1)).then(Stop(1)).then(Release(1)).expect(Safety and st(1) == "done")
    .then(Redeploy(1)).then(Assign(1))
    .expect(Safety and st(1) == "leased" and g.pendingGen.get(1) != "" and up and bootLeft != 0 and inGenWindow(1))
    .then(Crash).fail()
  // the positive twin: the same prefix, then Crash is taken and the generation replacement waits to be placed again.
  // Holds on `lease_conf_325` (no guard) and is violated on `lease_conf_ok` (the guard refuses the Crash step)
  run CrashIsEnabledInThe325WindowWithoutTheGuard = init
    .then(Assign(1)).then(Launch(1)).then(Stop(1)).then(Release(1)).expect(Safety and st(1) == "done")
    .then(Redeploy(1)).then(Assign(1))
    .expect(Safety and st(1) == "leased" and g.pendingGen.get(1) != "" and up and bootLeft != 0 and inGenWindow(1))
    .then(Crash).expect(Safety and not(up) and st(1) == "wait")
  // item 7, on `lease_ok` and on its recording twin `lease_conf_three` (three offsets; unreachable on the five-offset
  // conformance pool; D-18: replayed by the driver's #[quint_test] on `lease_conf_three`): a restart that finds no free
  // offset is refused by the pool, so its dispatch cleans the predecessor at once; once that lease is released the
  // workload is placed with its charged count
  run PoolExhaustedRestartCleansThePredecessorThenPlaces = init
    .then(Assign(1)).then(Launch(1)).then(Assign(2)).then(Launch(2)).then(VmmCrash(1))
    .then(Assign(1)).expect(Safety and st(1) == "leased" and pstOf(1) == "held" and freeOffs == Set())
    .then(VmmCrash(2)).expect(Safety and st(2) == "ended" and al.get(2).retry and freeOffs == Set())
    .then(AssignRefused(2))
    .expect(Safety and st(2) == "wait" and pstOf(2) == "retiring" and att.get(2) == 0 and offOf(2) == -1
      and freeOffs == Set())
    .then(Release(2)).expect(Safety and st(2) == "wait" and pstOf(2) == "none" and freeOffs != Set())
    .then(Assign(2)).expect(Safety and st(2) == "leased" and att.get(2) == 1)

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoExhaustion: bool = ALLOCS.forall(a => al.get(a).st != "failed")
  val WitnessNoRunning: bool = ALLOCS.forall(a => al.get(a).st != "running")
  val WitnessNoOrphan: bool = orphans == Set()
  val WitnessNoReclaim: bool = not(g.reclaimed)

  // ------------------------------------------------------------------ liveness (TLC)
  // a workload waiting to be placed (`wait`, or `ended` with `retry`) is eventually placed: serve restarts are bounded
  // (BOOTS), boot reclamation progresses (weak fairness on Reclaim and Restart), a held teardown completes (weak fairness
  // on Release, which frees a predecessor lease that blocks the workload's next placement), and assign is strongly fair
  // (the pool lock is shared)
  def waiting(a: int): bool = al.get(a).st == "wait" or (al.get(a).st == "ended" and al.get(a).retry)
  temporal EventuallyPlaced =
    and {
      ALLOCS.forall(a => strongFair(Assign(a), (lease, al, reclaimed, up))),
      ALLOCS.forall(a => weakFair(Release(a), (lease, al, g))),
      weakFair(Restart, (up, reclaimed, dev)),
      weakFair(Reclaim, (dev, reclaimed, up)),
      OFFS.forall(o => weakFair(OrphanExit(o), dev)),
    }.implies(ALLOCS.forall(a => always((waiting(a) and (freeOffs != Set())).implies(
        eventually(not(waiting(a)) or freeOffs == Set())))))
}

// The design: two workloads, three offsets, serve crash + restart (VMMs survive each as orphans until reclaimed).
module lease_ok { import cid_lease(OFF = Set(), CFG = "ok", REC = false).* from "./cid_lease" }
// Conformance main (D-UP-1): `lease_ok` on the five offsets of the /29 test pool, recording `conf_step`; only the named
// runs are run on it. Crash is not enabled in the #325 window here (item 9)
module lease_conf_ok { import cid_lease(OFF = Set(), CFG = "conf_ok", REC = true).* from "./cid_lease" }
// Conformance main (D-18): `lease_ok`'s three offsets, recording `conf_step`; it carries only the run
// PoolExhaustedRestartCleansThePredecessorThenPlaces, which pool exhaustion makes unreachable on the five offsets
module lease_conf_three { import cid_lease(OFF = Set(), CFG = "ok", REC = true).* from "./cid_lease" }
// PENDING GH #325 (item 9): the conformance configuration with the #325 window kept, recording; it carries the run
// RedeployLeasedAtServeCrashIsPlacedAgain until #325 lands, whose change deletes this main and the guard
module lease_conf_325 { import cid_lease(OFF = Set(), CFG = "conf_325", REC = true).* from "./cid_lease" }
// The five offsets of the /29 test pool, not recording: the safety check of the conformance configuration (D-UP-1)
module lease_five { import cid_lease(OFF = Set(), CFG = "conf_ok", REC = false).* from "./cid_lease" }
// Liveness: one serve restart (three offsets: a model-checking instance, not a conformance target).
module lease_live { import cid_lease(OFF = Set(), CFG = "live", REC = false).* from "./cid_lease" }
