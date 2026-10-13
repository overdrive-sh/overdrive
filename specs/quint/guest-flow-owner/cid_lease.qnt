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
 *     redeployed.
 *   - `dev` is the kernel's holder table of the vhost-vsock device per CID: 0 free, a > 0 allocation a's
 *     VMM (the VMM is the only holder), -2 a VMM of the previous process (an orphan), still running until
 *     reclaimed. A VMM that finds its CID's device held exits before READY (the device open is refused).
 *   - `assign` walks the free offsets in next-fit order under the pool lock (one atomic step) and is
 *     enabled only after boot reclamation completed (`reclaimed`).
 *   - A `serve` crash empties the pool, its cursor and the in-memory placement; every running VMM
 *     survives as an orphan (-2) until boot reclamation kills it, one orphan per step. An orphan may
 *     also exit by itself before reclamation reaches it (a guest reset ends the VMM).
 *   - RESET-EXIT: a guest reset (PID-1 crash, triple fault) ends the VMM process, so a CID's
 *     incarnation is one VMM process; the VMM's exit releases its device and the lease retires.
 *
 * OFF names the design rules switched off (teeth; instances in hazard/):
 *   releaseBeforeVmm    the lease is released while the VMM still holds the device
 *   assignBeforeReclaim assign runs before boot reclamation completed
 *   resetReboots        a guest reset reboots the guest inside the same VMM (no exit)
 *   reclaimNeverCompletes boot reclamation kills the orphans but never records that it completed, so `assign` stays disabled
 *   assignSplit         assign is not one step under the pool lock: the next-fit free offset is chosen in one step
 *                       and taken in a later one, without re-checking it is still free
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
module cid_lease {
  const OFF: Set[str]
  const CFG: str   // "ok" | "live" | "conf_ok"

  val CONF: bool = CFG == "conf_ok"
  val LIVE: bool = CFG == "live"
  val OFFS: Set[int] = if (CONF) Set(0, 1, 2, 3, 4) else Set(0, 1, 2)
  val N: int = if (CONF) 5 else 3
  val ALLOCS: Set[int] = Set(1, 2)
  val BOOTS: int = if (LIVE) 1 else -1             // serve crash + restart (-1: unbounded)
  val BUG_RELEASE_BEFORE_VMM: bool = OFF.contains("releaseBeforeVmm")
  val BUG_ASSIGN_BEFORE_RECLAIM: bool = OFF.contains("assignBeforeReclaim")
  val BUG_RESET_REBOOTS: bool = OFF.contains("resetReboots")
  val BUG_ASSIGN_SPLIT: bool = OFF.contains("assignSplit")
  val BUG_RECLAIM_NEVER_COMPLETES: bool = OFF.contains("reclaimNeverCompletes")

  // st: wait | picked (assignSplit only: an offset chosen, not yet taken) | leased (lease, VMM not launched) | running (the VMM holds the device) | retiring (lease Retiring) | done
  type A = { st: str, off: int, retry: bool }
  type Ghost = {
    clash: bool,       // a VMM found its CID's device held by another holder
    rebooted: bool,    // a guest reset left the VMM alive
    reclaimed: bool,   // an orphan was reclaimed (witness)
  }

  var dev: int -> int
  var lease: int -> int          // offset -> holding allocation (0 = free)
  var cursor: int
  var al: int -> A
  var up: bool                   // the serve process is up
  var reclaimed: bool            // boot reclamation of this process completed
  var bootLeft: int
  var g: Ghost

  val g0: Ghost = { clash: false, rebooted: false, reclaimed: false }
  def isFree(o: int): bool = lease.get(o) == 0
  val freeOffs: Set[int] = OFFS.filter(o => isFree(o))
  def dist(o: int): int = (o - cursor + N) % N
  def nextFitIn(S: Set[int]): int = S.fold(-1, (best, o) =>
    if (best == -1) o else if (dist(o) < dist(best)) o else best)
  def leased(a: int): bool = al.get(a).st.in(Set("leased", "running", "retiring"))
  val orphans: Set[int] = OFFS.filter(o => dev.get(o) == -2)
  def devAt(o: int): int = if (OFFS.contains(o)) dev.get(o) else 0

  action keepAl = all { al' = al }

  action init = all {
    dev' = OFFS.mapBy(o => 0), lease' = OFFS.mapBy(o => 0), cursor' = 0,
    al' = ALLOCS.mapBy(a => { st: "wait", off: -1, retry: false }),
    up' = true, reclaimed' = true, bootLeft' = BOOTS, g' = g0,
  }

  // ------------------------------------------------------------------ assign (pool lock)
  // next-fit over the free offsets; after boot reclamation only
  action Assign(a: int): bool = all {
    not(BUG_ASSIGN_SPLIT),
    up, reclaimed or BUG_ASSIGN_BEFORE_RECLAIM, al.get(a).st == "wait", freeOffs != Set(),
    val o = nextFitIn(freeOffs)
    all {
      lease' = lease.set(o, a), cursor' = (o + 1) % N,
      al' = al.set(a, { st: "leased", off: o, retry: false }),
    },
    dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
  }

  // assignSplit: the walk picks the next-fit free offset in one step ...
  action AssignPick(a: int): bool = all {
    BUG_ASSIGN_SPLIT, up, reclaimed, al.get(a).st == "wait", freeOffs != Set(),
    al' = al.set(a, { st: "picked", off: nextFitIn(freeOffs), retry: false }),
    lease' = lease, cursor' = cursor, dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
  }
  // ... and takes it in a later step, without re-checking that it is still free
  action AssignTake(a: int): bool = all {
    BUG_ASSIGN_SPLIT, up, al.get(a).st == "picked",
    val o = al.get(a).off
    all { lease' = lease.set(o, a), cursor' = (o + 1) % N, al' = al.set(a, { ...al.get(a), st: "leased" }) },
    dev' = dev, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
  }

  // ------------------------------------------------------------------ launch (VM driver, VMM)
  // the VMM opens /dev/vhost-vsock for the leased CID itself; a held device refuses the open and the VMM exits
  // before READY (the lease retires and placement retries)
  action Launch(a: int): bool = all {
    up, al.get(a).st == "leased",
    val x = al.get(a)
    if (dev.get(x.off) == 0) all {
      dev' = dev.set(x.off, a), al' = al.set(a, { ...x, st: "running" }), g' = g,
    } else all {
      dev' = dev, al' = al.set(a, { ...x, st: "retiring", retry: true }), g' = { ...g, clash: true },
    },
    lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft,
  }
  // the VMM exits (stop, a crash, or a guest reset: RESET-EXIT): the last close of its device frees the CID
  action vmmExit(a: int, retry: bool): bool = all {
    val x = al.get(a)
    all {
      dev' = if (dev.get(x.off) == a) dev.set(x.off, 0) else dev,
      al' = al.set(a, { ...x, st: "retiring", retry: retry }),
    },
    lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
  }
  action Stop(a: int): bool = all { up, al.get(a).st == "running", vmmExit(a, false) }
  // the VMM exits before READY or later by itself (a crash)
  action VmmCrash(a: int): bool = all { up, al.get(a).st == "running", vmmExit(a, true) }
  // RESET-EXIT: a guest reset ends the VMM. resetReboots: the guest reboots inside the same VMM; nothing exits
  // and the restart budget is never charged
  action GuestReset(a: int): bool = all {
    up, al.get(a).st == "running",
    if (BUG_RESET_REBOOTS) all {
      g' = { ...g, rebooted: true },
      dev' = dev, al' = al, lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft,
    } else vmmExit(a, true),
  }
  // pool.release (after teardown; pool lock): only after the VMM exited; the offset is freed.
  // releaseBeforeVmm: the lease is released while the VMM still holds the device
  action Release(a: int): bool = all {
    up, al.get(a).st == "retiring" or (BUG_RELEASE_BEFORE_VMM and al.get(a).st == "running"),
    devAt(al.get(a).off) != a or BUG_RELEASE_BEFORE_VMM,
    val x = al.get(a)
    all {
      lease' = lease.set(x.off, 0),
      al' = al.set(a, { st: if (x.st == "running" or x.retry) "wait" else "done", off: -1, retry: false }),
    },
    dev' = dev, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
  }
  action Redeploy(a: int): bool = all {
    up, al.get(a).st == "done",
    al' = al.set(a, { st: "wait", off: -1, retry: false }),
    dev' = dev, lease' = lease, cursor' = cursor, up' = up, reclaimed' = reclaimed, bootLeft' = bootLeft, g' = g,
  }

  // ------------------------------------------------------------------ environment: serve crash, restart, reclamation
  // serve crashes: the pool, its cursor and the placement are lost; every running VMM survives as an orphan
  // holding its device; allocations placed but not launched vanish
  action Crash = all {
    up, bootLeft != 0,
    up' = false, reclaimed' = false,
    dev' = OFFS.mapBy(o => if (dev.get(o) > 0) -2 else dev.get(o)),
    lease' = OFFS.mapBy(o => 0), cursor' = 0,
    al' = al.keys().mapBy(a => if (al.get(a).st == "done") al.get(a) else { st: "wait", off: -1, retry: false }),
    bootLeft' = if (bootLeft < 0) bootLeft else bootLeft - 1, g' = g,
  }
  // the new process starts; boot reclamation is still to run
  action Restart = all {
    not(up),
    up' = true, reclaimed' = (orphans == Set()),
    dev' = dev, lease' = lease, cursor' = cursor, al' = al, bootLeft' = bootLeft, g' = g,
  }
  // boot reclamation kills one orphan VMM it does not supervise; its device is released
  action Reclaim(o: int): bool = all {
    up, not(reclaimed), dev.get(o) == -2,
    dev' = dev.set(o, 0),
    reclaimed' = ((orphans.exclude(Set(o)) == Set()) and not(BUG_RECLAIM_NEVER_COMPLETES)),
    lease' = lease, cursor' = cursor, al' = al, up' = up, bootLeft' = bootLeft, g' = { ...g, reclaimed: true },
  }
  // an orphan exits by itself before it is reclaimed (a guest reset ends the VMM)
  action OrphanExit(o: int): bool = all {
    dev.get(o) == -2,
    dev' = dev.set(o, 0),
    reclaimed' = (reclaimed or (up and orphans.exclude(Set(o)) == Set())),
    lease' = lease, cursor' = cursor, al' = al, up' = up, bootLeft' = bootLeft, g' = g,
  }

  action step = {
    nondet a = ALLOCS.oneOf()
    nondet o = OFFS.oneOf()
    any {
      Assign(a), AssignPick(a), AssignTake(a), Launch(a), Stop(a), VmmCrash(a), GuestReset(a), Release(a), Redeploy(a),
      Crash, Restart, Reclaim(o), OrphanExit(o),
    }
  }

  // ------------------------------------------------------------------ invariants
  // Leased CIDs are unique among allocations holding a lease (lease uniqueness is CID uniqueness).
  val CidUnique: bool = ALLOCS.forall(a => ALLOCS.forall(b =>
    (a != b and leased(a) and leased(b)) implies al.get(a).off != al.get(b).off))
  // no VMM launch fails on a CID clash — including across serve crash and restart
  val NoLaunchClash: bool = not(g.clash)
  // the lease outlives the VMM: a running VMM's offset stays leased until the VMM has exited (release-last)
  val LeaseOutlivesVmm: bool = OFFS.forall(o => dev.get(o) > 0 implies lease.get(o) == dev.get(o))
  // assign runs only after boot reclamation: no lease is on an offset a previous process's VMM still holds
  val AssignAfterReclaim: bool = OFFS.forall(o => lease.get(o) != 0 implies dev.get(o) != -2)
  // RESET-EXIT: a guest reset never leaves the VMM alive
  val GuestResetEndsVmm: bool = not(g.rebooted)
  val Safety: bool = and { CidUnique, NoLaunchClash, LeaseOutlivesVmm, AssignAfterReclaim, GuestResetEndsVmm }

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Deterministic scenarios on the design instance `lease_ok` (offsets 0-2, workloads 1 and 2), each ending
  // in an `expect` of its outcome with `Safety` checked along the way.
  def st(a: int): str = al.get(a).st
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
  // RESET-EXIT: a guest reset ends the VMM; its device is released, the lease retires and placement retries on a
  // fresh VMM (one incarnation per VMM process)
  run GuestResetEndsTheVmmAndPlacementRetries = init
    .then(Assign(1)).then(Launch(1)).then(GuestReset(1))
    .expect(Safety and st(1) == "retiring" and al.get(1).retry and dev.get(0) == 0 and lease.get(0) == 1 and not(g.rebooted))
    .then(Release(1)).expect(Safety and st(1) == "wait")
    .then(Assign(1)).then(Launch(1)).expect(Safety and st(1) == "running" and not(g.clash))
  // the VMM crashes before READY: the device closes, the lease retires, placement retries
  run VmmCrashFreesItsCidAndPlacementRetries = init
    .then(Assign(1)).then(Launch(1)).then(VmmCrash(1))
    .expect(Safety and st(1) == "retiring" and dev.get(0) == 0 and al.get(1).retry)
    .then(Release(1)).expect(Safety and st(1) == "wait" and lease.get(0) == 0)
  // NO-SURVIVE: serve crashes with a running VMM; the VMM survives as an orphan holding its device; the restarted
  // process reclaims it BEFORE any assign, and the next lease of that CID launches without a clash
  run ServeCrashOrphanIsReclaimedBeforeAnyAssign = init
    .then(Assign(1)).then(Launch(1)).then(Crash)
    .expect(Safety and not(up) and dev.get(0) == -2 and lease == OFFS.mapBy(o => 0) and cursor == 0 and st(1) == "wait")
    .then(Restart).expect(Safety and up and not(reclaimed) and orphans == Set(0))
    .then(Assign(1)).fail()
  run ReclamationReleasesTheOrphanDeviceThenAssignProceeds = init
    .then(Assign(1)).then(Launch(1)).then(Crash).then(Restart)
    .then(Reclaim(0)).expect(Safety and reclaimed and dev.get(0) == 0 and g.reclaimed)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and al.get(1).off == 0)
    .then(Launch(1)).expect(Safety and st(1) == "running" and dev.get(0) == 1 and not(g.clash))
  // an orphan exits by itself (a guest reset) before reclamation reaches it; reclamation completes with nothing to kill
  run OrphanExitsByItselfBeforeReclamation = init
    .then(Assign(1)).then(Launch(1)).then(Crash).then(Restart).then(OrphanExit(0))
    .expect(Safety and reclaimed and dev.get(0) == 0)
    .then(Assign(2)).expect(Safety and st(2) == "leased")
  // a crash with a lease that was never launched: nothing survives, the restarted pool is empty and assigns at once
  run ServeCrashWithUnlaunchedLeaseLeavesNothingToReclaim = init
    .then(Assign(1)).then(Crash)
    .expect(Safety and dev == OFFS.mapBy(o => 0) and lease == OFFS.mapBy(o => 0))
    .then(Restart).expect(Safety and up and reclaimed)
    .then(Assign(1)).expect(Safety and st(1) == "leased" and al.get(1).off == 0)

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoRunning: bool = ALLOCS.forall(a => al.get(a).st != "running")
  val WitnessNoOrphan: bool = orphans == Set()
  val WitnessNoReclaim: bool = not(g.reclaimed)

  // ------------------------------------------------------------------ liveness (TLC)
  // a waiting workload is eventually placed: serve restarts are bounded (BOOTS), boot reclamation progresses
  // (weak fairness on Reclaim and Restart), and assign is strongly fair (the pool lock is shared)
  temporal EventuallyPlaced =
    and {
      ALLOCS.forall(a => strongFair(Assign(a), (lease, al, reclaimed, up))),
      weakFair(Restart, (up, reclaimed, dev)),
      OFFS.forall(o => weakFair(Reclaim(o), (dev, reclaimed, up))),
      OFFS.forall(o => weakFair(OrphanExit(o), dev)),
    }.implies(ALLOCS.forall(a => always((al.get(a).st == "wait" and (freeOffs != Set())).implies(
        eventually(al.get(a).st != "wait" or freeOffs == Set())))))
}

// The design: two workloads, three offsets, serve crash + restart (VMMs survive each as orphans until reclaimed).
module lease_ok { import cid_lease(OFF = Set(), CFG = "ok").* from "./cid_lease" }
// Conformance: `lease_ok` on the five offsets of the /29 test pool.
module lease_conf_ok { import cid_lease(OFF = Set(), CFG = "conf_ok").* from "./cid_lease" }
// Liveness: one serve restart.
module lease_live { import cid_lease(OFF = Set(), CFG = "live").* from "./cid_lease" }
