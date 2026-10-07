// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — module C: CID lease with the atomic host-kernel claim
 * (D16-CLAIM), next-fit assignment, the claim's handoff to the VMM, release, foreign vhost users,
 * `serve` crash with surviving VMMs, and restart.
 *
 * Design artifact and DISTILL oracle. Models the design text (revision 9), not an implementation.
 *
 * Source of truth: ADR-0170 (D16-CLAIM), ADR-0156 (D16: CID derived from the lease offset; next-fit
 * M-9, in-memory cursor), feature-delta.md § "vsock Attachment Replacement DESIGN": § Core vocabulary
 * (`GuestCidClaim`, `ClaimedGuestCid`, `GuestTransportAssignment::take_claim`,
 * `GuestAddressPool::assign` / `release` with the claim, `GuestCidsHeldElsewhere`, `GuestCidClaim`
 * error), § VMM backend contract (`Vmm::create(config, vsock: ClaimedGuestCid)`: Err drops the
 * claim; Ok = the VMM holds the device, the adapter holds no copy), § Composition.
 *
 *   D16-CLAIM  `assign` walks the free offsets in next-fit order under the pool lock; for each it
 *          claims GUEST_CID_BASE+offset on the host kernel (the kernel's CID holder table is the
 *          arbiter). Ok → the lease is Admitted with that offset together with its claim (the lease
 *          exists only with its claim). InUse → the next free offset; nothing is remembered. Any
 *          other claim error → a typed error, no state change, no further offset tried. Every free
 *          offset InUse, each at the moment its own claim was tried (not necessarily all at once) →
 *          GuestCidsHeldElsewhere (no state change). No free offset → PoolExhausted.
 *          The move-only claim travels to the VMM (`take_claim` once; `create` Ok: the VMM holds
 *          it, `serve` holds no copy; `create` Err: dropped = released). An untaken claim is
 *          released at `pool.release`; a handed-off claim is released at the VMM's exit.
 *
 * Abstraction:
 *   - OFFS: lease offsets (CID = GUEST_CID_BASE + offset). ALLOCS: workloads placed on the node; a
 *     workload whose launch ended before READY is retried by placement (`retry`), a stopped one is
 *     done and may be redeployed.
 *   - `kern` is the host kernel's CID holder table: 0 free, -1 a foreign vhost user, -2 a VMM that
 *     survived a `serve` crash (its process still holds the handed-off device), a > 0 the claim of
 *     allocation a (in the lease's take-once slot, in transit to `create`, or held by a's VMM).
 *   - The walk of `assign` is stepwise (one claim ioctl per step): foreign vhost users act between
 *     the steps; other pool operations do not (the pool lock).
 *   - A `serve` crash closes every claim `serve` holds (slot or in transit) — the kernel frees those
 *     CIDs — while VMMs that hold their device survive (-2) until reclaimed or dead (`OrphanExit`).
 *     The pool, its cursor and the walk are in memory: after the restart every placed workload is
 *     placed again.
 *   - BUG_* constants each switch ONE design rule off (teeth; instances in hazard/).
 *
 * ASSUMPTIONS:
 *   K-C1  A duplicate CID is refused at device creation and the first holder is unaffected; a CID
 *         is free right after the holder's last reference closes ........................ P-16
 *   K-C2  Foreign vhost users take and release CIDs at any time (modelled: ForeignTake /
 *         ForeignRelease) ............................................. environment; nothing to discharge
 *   K-C3  A claim on an unowned instance is exclusive while any reference to its file is open,
 *         survives the handoff to the VMM, and is released at the last close ................ V-25
 */
module cid_lease {
  const OFFS: Set[int]           // 0 .. N-1
  const N: int
  const ALLOCS: Set[int]
  const FOREIGN0: Set[int]       // offsets whose CID a foreign vhost user holds at start
  const FOREIGN_UNBOUNDED: bool  // foreign users take / release CIDs forever
  const FOREIGN_CHANGES: int     // otherwise: bound on foreign take / release steps
  const ALLOW_BOOT: bool         // serve crash + restart
  const PRE_READY_EXITS: int     // bound on VMM exits before READY / create failures (any cause)
  const CLAIM_ERRORS: int        // bound on non-InUse claim errors
  const BUG_CHECK_THEN_CLAIM: bool   // assign only checks the CID is free; the VMM claims it at create
  const BUG_RELEASE_BEFORE_VMM: bool // serve closes its claim when the driver takes it, before the VMM holds it
  const BUG_REMEMBER_INUSE: bool     // assign remembers offsets that returned InUse and skips them later

  type A = { st: str, off: int, loc: str, retry: bool, remembered: Set[int] }
  type Ghost = {
    clash: bool,            // a VMM create found its CID held by another holder
    skipFree: bool,         // an offset was skipped while its CID was free in the kernel
    skipped: bool,          // an InUse skip happened (witness)
    skipOrphan: bool,       // an InUse skip on a CID held by a VMM that survived a serve crash (witness)
    refusedHeld: bool,      // assign refused GuestCidsHeldElsewhere (witness)
    refusedClaimable: bool, // ... at a moment when some free offset's CID was unheld (stepwise walk)
    claimErr: bool,         // a non-InUse claim error ended an assign (witness)
    refusedUnasked: bool,   // GuestCidsHeldElsewhere returned with a free offset not claimed in this call
  }

  var kern: int -> int
  var lease: int -> int          // offset -> holding allocation (0 = free)
  var cursor: int
  var walker: int                // allocation whose assign walk holds the pool lock (0 = none)
  var todo: Set[int]             // free offsets the walk has not tried yet
  // st: wait | admitted (lease + claim in the slot) | launching (claim taken, create running) |
  //     running (VMM holds the device) | retiring (lease Retiring) | done
  // loc: none | slot | taken | vmm — where the claim's file reference is
  var al: int -> A
  var foreignLeft: int
  var exitsLeft: int
  var errsLeft: int
  var g: Ghost

  val g0: Ghost = { clash: false, skipFree: false, skipped: false, skipOrphan: false,
                    refusedHeld: false, refusedClaimable: false, claimErr: false,
                    refusedUnasked: false }
  def isFree(o: int): bool = lease.get(o) == 0
  val freeOffs: Set[int] = OFFS.filter(o => isFree(o))
  val claimableFree: bool = OFFS.exists(o => isFree(o) and kern.get(o) == 0)
  def dist(o: int): int = (o - cursor + N) % N
  def nextFitIn(S: Set[int]): int = S.fold(-1, (best, o) =>
    if (best == -1) o else if (dist(o) < dist(best)) o else best)
  def holdsClaim(st: str): bool = st.in(Set("admitted", "launching", "running"))

  action keepCounters = all { foreignLeft' = foreignLeft, exitsLeft' = exitsLeft, errsLeft' = errsLeft }

  action init = all {
    kern' = OFFS.mapBy(o => if (FOREIGN0.contains(o)) -1 else 0),
    lease' = OFFS.mapBy(o => 0),
    cursor' = 0, walker' = 0, todo' = Set(),
    al' = ALLOCS.mapBy(a => { st: "wait", off: -1, loc: "none", retry: false, remembered: Set() }),
    foreignLeft' = FOREIGN_CHANGES, exitsLeft' = PRE_READY_EXITS, errsLeft' = CLAIM_ERRORS,
    g' = g0,
  }

  // ------------------------------------------------------------------ assign (pool lock)
  // Start the walk over the free offsets (PoolExhausted when none is free: disabled, no change).
  action AssignBegin(a: int): bool = all {
    walker == 0, al.get(a).st == "wait", freeOffs != Set(),
    val rem = al.get(a).remembered
    val cand = if (BUG_REMEMBER_INUSE) freeOffs.exclude(rem) else freeOffs
    val skipF = BUG_REMEMBER_INUSE and freeOffs.exists(o => rem.contains(o) and kern.get(o) == 0)
    if (cand == Set()) all {
      // bug only: every free offset remembered — refused without asking the kernel
      walker' = 0, todo' = Set(),
      g' = { ...g, skipFree: g.skipFree or skipF, refusedHeld: true,
             refusedClaimable: g.refusedClaimable or claimableFree, refusedUnasked: true },
    } else all {
      walker' = a, todo' = cand, g' = { ...g, skipFree: g.skipFree or skipF },
    },
    kern' = kern, lease' = lease, cursor' = cursor, al' = al, keepCounters,
  }
  // One claim ioctl on the next free offset in next-fit order.
  action AssignTry = all {
    walker != 0, todo != Set(),
    val a = walker
    val o = nextFitIn(todo)
    val x = al.get(a)
    if (kern.get(o) == 0) all {
      // claimed (BUG_CHECK_THEN_CLAIM: only checked; nothing held)
      kern' = if (BUG_CHECK_THEN_CLAIM) kern else kern.set(o, a),
      lease' = lease.set(o, a), cursor' = (o + 1) % N,
      al' = al.set(a, { ...x, st: "admitted", off: o, loc: if (BUG_CHECK_THEN_CLAIM) "none" else "slot" }),
      walker' = 0, todo' = Set(), g' = g,
    } else all {
      // InUse: skip this offset for this call only (BUG_REMEMBER_INUSE: remember it)
      val rest = todo.exclude(Set(o))
      all {
        kern' = kern, lease' = lease, cursor' = cursor,
        al' = if (BUG_REMEMBER_INUSE) al.set(a, { ...x, remembered: x.remembered.union(Set(o)) }) else al,
        walker' = if (rest == Set()) 0 else walker,
        todo' = rest,
        g' = { ...g, skipped: true, skipOrphan: g.skipOrphan or kern.get(o) == -2,
               refusedHeld: g.refusedHeld or rest == Set(),
               // the refusal is decided per ioctl: an offset tried earlier may be unheld by now
               refusedClaimable: g.refusedClaimable or (rest == Set() and claimableFree) },
      }
    },
    keepCounters,
  }
  // A claim error other than InUse: typed error, no state change, no further offset tried.
  action AssignClaimErr = all {
    walker != 0, errsLeft > 0,
    walker' = 0, todo' = Set(), errsLeft' = errsLeft - 1, g' = { ...g, claimErr: true },
    kern' = kern, lease' = lease, cursor' = cursor, al' = al,
    foreignLeft' = foreignLeft, exitsLeft' = exitsLeft,
  }

  // ------------------------------------------------------------------ launch (VM driver, VMM)
  // spec.transport.take_claim(): the claim leaves the slot (once).
  action LaunchBegin(a: int): bool = all {
    al.get(a).st == "admitted",
    val x = al.get(a)
    all {
      al' = al.set(a, { ...x, st: "launching", loc: "taken" }),
      // BUG_RELEASE_BEFORE_VMM: serve closes its reference before the VMM holds the file
      kern' = if (BUG_RELEASE_BEFORE_VMM and kern.get(x.off) == a) kern.set(x.off, 0) else kern,
    },
    lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, keepCounters, g' = g,
  }
  // Vmm::create Ok: the VMM holds the claimed device; serve holds no copy. With either bug the VMM
  // has no claim of its own to inherit and claims the CID itself at device creation (ADR-0146).
  action CreateOk(a: int): bool = all {
    al.get(a).st == "launching",
    val x = al.get(a)
    val vmmClaims = BUG_CHECK_THEN_CLAIM or BUG_RELEASE_BEFORE_VMM
    if (not(vmmClaims) or kern.get(x.off) == 0) all {
      kern' = kern.set(x.off, a),
      al' = al.set(a, { ...x, st: "running", loc: "vmm" }),
      g' = g,
    } else all {
      // the CID is held elsewhere: the VMM exits before READY on the clash
      kern' = kern,
      al' = al.set(a, { ...x, st: "retiring", loc: "none", retry: true }),
      g' = { ...g, clash: true },
    },
    lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, keepCounters,
  }
  def dropClaim(k: int -> int, o: int, a: int): int -> int = if (k.get(o) == a) k.set(o, 0) else k
  // Vmm::create Err (any cause): the claim is dropped (released); the lease retires.
  action CreateErr(a: int): bool = all {
    al.get(a).st == "launching", exitsLeft > 0,
    val x = al.get(a)
    all {
      kern' = dropClaim(kern, x.off, a),
      al' = al.set(a, { ...x, st: "retiring", loc: "none", retry: true }),
    },
    exitsLeft' = exitsLeft - 1, foreignLeft' = foreignLeft, errsLeft' = errsLeft,
    lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, g' = g,
  }
  // the VMM exits before READY (any cause): its device closes, the kernel frees the CID
  action VmmFail(a: int): bool = all {
    al.get(a).st == "running", exitsLeft > 0,
    val x = al.get(a)
    all {
      kern' = dropClaim(kern, x.off, a),
      al' = al.set(a, { ...x, st: "retiring", loc: "none", retry: true }),
    },
    exitsLeft' = exitsLeft - 1, foreignLeft' = foreignLeft, errsLeft' = errsLeft,
    lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, g' = g,
  }
  // stop: a running VMM exits; an admitted (unlaunched) allocation keeps its claim in the slot
  // until pool.release
  action Stop(a: int): bool = all {
    al.get(a).st.in(Set("running", "admitted")),
    val x = al.get(a)
    all {
      kern' = if (x.st == "running") dropClaim(kern, x.off, a) else kern,
      al' = al.set(a, { ...x, st: "retiring", loc: if (x.st == "running") "none" else x.loc, retry: false }),
    },
    lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, keepCounters, g' = g,
  }
  // pool.release (after teardown; pool lock): an untaken claim is dropped here; the offset is freed
  action Release(a: int): bool = all {
    walker == 0, al.get(a).st == "retiring",
    val x = al.get(a)
    all {
      kern' = if (x.loc == "slot") dropClaim(kern, x.off, a) else kern,
      lease' = lease.set(x.off, 0),
      al' = al.set(a, { st: if (x.retry) "wait" else "done", off: -1, loc: "none", retry: false,
                        remembered: if (x.retry) x.remembered else Set() }),
    },
    cursor' = cursor, walker' = walker, todo' = todo, keepCounters, g' = g,
  }
  action Redeploy(a: int): bool = all {
    al.get(a).st == "done",
    al' = al.set(a, { st: "wait", off: -1, loc: "none", retry: false, remembered: Set() }),
    kern' = kern, lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, keepCounters, g' = g,
  }

  // ------------------------------------------------------------------ environment
  action foreignStep(o: int, v: int): bool = all {
    FOREIGN_UNBOUNDED or foreignLeft > 0,
    kern' = kern.set(o, v),
    foreignLeft' = if (FOREIGN_UNBOUNDED) foreignLeft else foreignLeft - 1,
    exitsLeft' = exitsLeft, errsLeft' = errsLeft,
    lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, al' = al, g' = g,
  }
  action ForeignTake(o: int): bool = all { kern.get(o) == 0, foreignStep(o, -1) }
  action ForeignRelease(o: int): bool = all { kern.get(o) == -1, foreignStep(o, 0) }
  // a VMM that survived a serve crash is reclaimed or dies: its device closes
  action OrphanExit(o: int): bool = all {
    kern.get(o) == -2,
    kern' = kern.set(o, 0),
    lease' = lease, cursor' = cursor, walker' = walker, todo' = todo, al' = al, keepCounters, g' = g,
  }
  // serve crash + restart: serve's claims (slot, in transit) close; VMM-held devices survive
  action Boot = all {
    ALLOW_BOOT,
    kern' = OFFS.mapBy(o =>
      val k = kern.get(o)
      if (k > 0) (if (al.get(k).loc == "vmm") -2 else 0) else k),
    lease' = OFFS.mapBy(o => 0), cursor' = 0, walker' = 0, todo' = Set(),
    al' = al.keys().mapBy(a =>
      val x = al.get(a)
      if (x.st == "done") x else { st: "wait", off: -1, loc: "none", retry: false, remembered: Set() }),
    keepCounters, g' = g,
  }

  action OwnerStep = {
    nondet a = ALLOCS.oneOf()
    any { AssignBegin(a), AssignTry, LaunchBegin(a), CreateOk(a), Release(a) }
  }
  action step = {
    nondet a = ALLOCS.oneOf()
    nondet o = OFFS.oneOf()
    any {
      AssignBegin(a), AssignTry, AssignClaimErr, LaunchBegin(a), CreateOk(a), CreateErr(a),
      VmmFail(a), Stop(a), Release(a), Redeploy(a),
      ForeignTake(o), ForeignRelease(o), OrphanExit(o), Boot,
    }
  }

  // ------------------------------------------------------------------ invariants
  // Leased CIDs are unique among allocations holding a lease.
  def leased(a: int): bool = al.get(a).st.in(Set("admitted", "launching", "running", "retiring"))
  val CidUnique: bool = ALLOCS.forall(a => ALLOCS.forall(b =>
    (a != b and leased(a) and leased(b)) implies al.get(a).off != al.get(b).off))
  // (5, D16-CLAIM) every Admitted lease's CID is held by the kernel for that lease's claim (in the
  // slot, in transit, or in its VMM) — never free, never another holder
  val ClaimHeld: bool = ALLOCS.forall(a => holdsClaim(al.get(a).st) implies kern.get(al.get(a).off) == a)
  // (5) no launch fails on a CID clash — including across serve crash and restart
  val NoLaunchClash: bool = not(g.clash)
  // (5) InUse is the only skip, and it is the kernel's answer at that step: no offset is skipped
  // while its CID is unheld
  val NoSkipWhileClaimable: bool = not(g.skipFree)
  // (5) a kernel claim of Overdrive's exists only for its lease: no leaked claim
  val NoLeakedClaim: bool = OFFS.forall(o => kern.get(o) > 0 implies (
    val x = al.get(kern.get(o))
    x.off == o and (holdsClaim(x.st) or (x.st == "retiring" and x.loc == "slot"))))
  // (5, D16-CLAIM wording of revision 9 item 5) GuestCidsHeldElsewhere means every free offset's claim
  // returned InUse during this call — each CID held by another holder at the moment its own claim was
  // tried (NoSkipWhileClaimable), not necessarily all at once (RefusalSnapshotConsistent is violated)
  val RefusalEachClaimInUse: bool = not(g.refusedUnasked)
  val Safety: bool = and {
    CidUnique, ClaimHeld, NoLaunchClash, NoSkipWhileClaimable, NoLeakedClaim, RefusalEachClaimInUse,
  }

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoSkip: bool = not(g.skipped)
  val WitnessNoOrphanSkip: bool = not(g.skipOrphan)
  val WitnessNoHeldElsewhere: bool = not(g.refusedHeld)
  val WitnessNoClaimErr: bool = not(g.claimErr)
  val WitnessNoRunning: bool = ALLOCS.forall(a => al.get(a).st != "running")
  // The refusal is decided per claim ioctl (ADR-0170 "at that moment"): with foreign users acting
  // between the ioctls of one walk, GuestCidsHeldElsewhere can be returned while an offset tried
  // earlier has just become unheld. Expected violated (documented; the refusal is non-terminal).
  val RefusalSnapshotConsistent: bool = not(g.refusedClaimable)

  // ------------------------------------------------------------------ liveness (TLC)
  // (D16-CLAIM) a waiting workload is eventually placed whenever a free offset's CID stays unheld:
  // foreign churn is bounded (FOREIGN_CHANGES), surviving VMMs are reclaimed (weak fairness on
  // OrphanExit), the walk progresses, and AssignBegin is strongly fair (the pool lock is shared).
  temporal EventuallyPlaced =
    and {
      ALLOCS.forall(a => strongFair(AssignBegin(a), (walker, al, todo))),
      weakFair(AssignTry, (walker, todo, al, kern)),
      OFFS.forall(o => weakFair(OrphanExit(o), kern)),
    }.implies(ALLOCS.forall(a => always((al.get(a).st == "wait").implies(
        eventually(al.get(a).st != "wait" or not(claimableFree))))))
}

// The design: two workloads, three offsets, one foreign CID at start, foreign users take and
// release CIDs forever, serve crash + restart, one pre-READY exit, one claim error.
module lease_ok {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN0 = Set(0),
    FOREIGN_UNBOUNDED = true, FOREIGN_CHANGES = 0, ALLOW_BOOT = true, PRE_READY_EXITS = 1,
    CLAIM_ERRORS = 1, BUG_CHECK_THEN_CLAIM = false, BUG_RELEASE_BEFORE_VMM = false,
    BUG_REMEMBER_INUSE = false).* from "./cid_lease"
}
// Three workloads on three offsets (the pool fills; GuestCidsHeldElsewhere vs PoolExhausted).
module lease_three {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2, 3), FOREIGN0 = Set(0),
    FOREIGN_UNBOUNDED = true, FOREIGN_CHANGES = 0, ALLOW_BOOT = true, PRE_READY_EXITS = 1,
    CLAIM_ERRORS = 0, BUG_CHECK_THEN_CLAIM = false, BUG_RELEASE_BEFORE_VMM = false,
    BUG_REMEMBER_INUSE = false).* from "./cid_lease"
}
// Liveness: bounded foreign churn (two changes), no restart, two pre-READY exits, one claim error.
module lease_live {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN0 = Set(0),
    FOREIGN_UNBOUNDED = false, FOREIGN_CHANGES = 2, ALLOW_BOOT = false, PRE_READY_EXITS = 2,
    CLAIM_ERRORS = 1, BUG_CHECK_THEN_CLAIM = false, BUG_RELEASE_BEFORE_VMM = false,
    BUG_REMEMBER_INUSE = false).* from "./cid_lease"
}
