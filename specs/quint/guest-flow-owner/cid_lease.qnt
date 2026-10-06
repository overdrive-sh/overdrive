// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — module C: CID lease, next-fit assignment, CID-clash
 * retry with a per-allocation exclusion set.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: ADR-0156 (D16, next-fit M-9: first free offset at or after an in-memory cursor
 * that advances past each assigned offset and wraps; the cursor is not persisted and starts at the
 * first offset on each boot); feature-delta.md § Composition (deploy step 1, teardown →
 * pool.release), G-V4, R5-19; and the user decision of 2026-10-06 that followed the first model
 * check (spike/quint-owner-findings.md finding 2):
 *   D16-EXCL  Each workload's placement keeps an in-memory set of lease offsets on which its launch
 *          already ended before READY (the only projection of a CID clash: the driver cannot tell
 *          a clash from any other pre-READY VMM exit). The set is cleared when a launch of that
 *          workload reaches READY or the workload is stopped; it is never persisted (empty after a
 *          `serve` restart).
 * and the user decision of 2026-10-06 that followed the second model check
 * (spike/quint-owner-findings-r2.md finding r2-1):
 *   D16-PREF  The exclusion is a PREFERENCE: `assign` picks next-fit among free offsets the
 *          workload has not excluded; if none is free it falls back to next-fit among the free
 *          excluded offsets rather than refusing. `assign` refuses (pool exhaustion) only when no
 *          offset is free.
 *
 * Abstraction:
 *   - OFFS: lease offsets (CID = GUEST_CID_BASE + offset). ALLOCS: workloads placed on the node
 *     (each placement retry is a new allocation of the same workload; the set is per workload).
 *   - FOREIGN: offsets whose CID another host vhost user holds; launching on one fails before
 *     READY (P-16: CH exits with EADDRINUSE) — the "CID clash".
 *   - A failed launch retires then releases the lease (ADR-0133), then placement retries `assign`.
 *   - OTHER_FAILURES bounds pre-READY VMM exits that are NOT a CID clash (any offset); they are
 *     excluded the same way.
 *   - The exclusion set is in memory (like the cursor): `Boot` (a `serve` restart) clears it.
 *     `Redeploy` models a new placement of a stopped workload (the set was cleared at stop).
 *   - The exhaustion refusal is modelled as `Assign` being disabled; the workload keeps waiting.
 *   - BUG_REFUSE_WHEN_ALL_EXCLUDED restores the round-2 rule (refuse when every free offset is
 *     excluded) — the teeth for D16-PREF.
 *
 * ASSUMPTIONS:
 *   K-C1  A duplicate CID is refused at device creation and the first holder is unaffected; a CID
 *         is free again immediately after VMM exit .......................... P-16 (proven, bounded)
 *   K-C2  The foreign vhost user's CIDs do not change while the model runs (worst case: fixed)
 *         ......................................................................... none (environment)
 */
module cid_lease {
  const OFFS: Set[int]           // 0 .. N-1
  const N: int
  const ALLOCS: Set[int]
  const FOREIGN: Set[int]
  const ALLOW_BOOT: bool
  const BUG_FIRST_FIT: bool      // lowest free offset (the pre-M-9 assignment)
  const BUG_NO_EXCLUSION: bool   // D16-EXCL off: no per-workload exclusion (r1 design)
  const OTHER_FAILURES: int      // bound on pre-READY VMM exits that are not a CID clash
  const BUG_REFUSE_WHEN_ALL_EXCLUDED: bool // D16-PREF off: the r2 rule (refuse instead of fallback)

  // st: wait (needs placement) | held (lease assigned, not launched) | running | failed (launch
  //     failed; lease Retiring) | done
  type A = { st: str, off: int, failedOff: int, clashed: Set[int] }

  var lease: int -> int          // offset -> holding allocation (0 = free)
  var otherLeft: int             // remaining non-clash pre-READY failures
  var cursor: int
  var al: int -> A
  // ghosts
  var everClashed: int -> Set[int] // per allocation, every offset it ever clashed on (never cleared)
  var retrySame: bool            // a retry received the offset whose launch just failed
  var retrySameUnforced: bool    // ... while some other offset was free (contradicts ADR-0156)
  var retrySameForeign: bool     // ... and that offset is a foreign CID it already clashed on
  var reclashInBoot: bool        // an allocation clashed twice on one offset within one serve lifetime
  var reclashEver: bool          // an allocation clashed twice on one offset (across restarts)
  var exclWhileUnexcl: bool      // assign returned an excluded offset while an unexcluded one was free

  def isFree(o: int): bool = lease.get(o) == 0
  def eligible(a: int, o: int): bool = isFree(o) and (BUG_NO_EXCLUSION or not(al.get(a).clashed.contains(o)))
  // next-fit: first offset of S at or after `cursor`, wrapping; -1 when S is empty
  def dist(o: int): int = (o - cursor + N) % N
  def nextFitIn(S: Set[int]): int = S.fold(-1, (best, o) =>
    if (best == -1) o else if (dist(o) < dist(best)) o else best)
  def firstFitIn(S: Set[int]): int = S.fold(-1, (best, o) =>
    if (best == -1) o else if (o < best) o else best)
  def fitIn(S: Set[int]): int = if (BUG_FIRST_FIT) firstFitIn(S) else nextFitIn(S)
  // D16-PREF: prefer unexcluded free offsets; fall back to the free excluded ones; -1 (the
  // exhaustion refusal) only when no offset is free
  def pick(a: int): int =
    val preferred = OFFS.filter(o => eligible(a, o))
    if (preferred != Set()) fitIn(preferred)
    else if (BUG_REFUSE_WHEN_ALL_EXCLUDED) -1
    else fitIn(OFFS.filter(o => isFree(o)))

  action keepGhosts = all {
    everClashed' = everClashed, retrySame' = retrySame, retrySameUnforced' = retrySameUnforced,
    retrySameForeign' = retrySameForeign, reclashInBoot' = reclashInBoot, reclashEver' = reclashEver,
    exclWhileUnexcl' = exclWhileUnexcl,
  }

  action init = all {
    lease' = OFFS.mapBy(o => 0),
    otherLeft' = OTHER_FAILURES,
    cursor' = 0,
    al' = ALLOCS.mapBy(a => { st: "wait", off: -1, failedOff: -1, clashed: Set() }),
    everClashed' = ALLOCS.mapBy(a => Set()),
    retrySame' = false, retrySameUnforced' = false, retrySameForeign' = false,
    reclashInBoot' = false, reclashEver' = false, exclWhileUnexcl' = false,
  }

  // deploy step 1: pool.assign — the CID is derived from the offset in exactly this place
  action Assign(a: int): bool = all {
    al.get(a).st == "wait", pick(a) != -1,
    val p = pick(a)
    val f = al.get(a).failedOff
    val othersFree = OFFS.exists(o => o != f and isFree(o))
    val excl = al.get(a).clashed
    all {
      exclWhileUnexcl' = (exclWhileUnexcl or
        (excl.contains(p) and OFFS.exists(o => isFree(o) and not(excl.contains(o))))),
      retrySame' = (retrySame or (f != -1 and p == f)),
      retrySameUnforced' = (retrySameUnforced or (f != -1 and p == f and othersFree)),
      retrySameForeign' = (retrySameForeign or (f != -1 and p == f and FOREIGN.contains(f))),
      lease' = lease.set(p, a),
      cursor' = (p + 1) % N,
      al' = al.set(a, { ...al.get(a), st: "held", off: p }),
    },
    everClashed' = everClashed, reclashInBoot' = reclashInBoot, reclashEver' = reclashEver,
    otherLeft' = otherLeft,
  }
  // driver.start: CH creates the vsock device with the leased CID. A foreign CID clashes; any
  // other launch may also end before READY (bounded by OTHER_FAILURES). Either is recorded in the
  // workload's exclusion set; reaching READY clears it.
  action launchEnds(a: int, preReady: bool, otherLeft2: int): bool = all {
    val x = al.get(a)
    all {
      al' = al.set(a, { ...x, st: if (preReady) "failed" else "running",
                              clashed: if (preReady) x.clashed.union(Set(x.off)) else Set() }),
      reclashInBoot' = (reclashInBoot or (preReady and x.clashed.contains(x.off))),
      reclashEver' = (reclashEver or (preReady and everClashed.get(a).contains(x.off))),
      everClashed' = if (preReady) everClashed.set(a, everClashed.get(a).union(Set(x.off))) else everClashed,
    },
    otherLeft' = otherLeft2, lease' = lease, cursor' = cursor, exclWhileUnexcl' = exclWhileUnexcl,
    retrySame' = retrySame, retrySameUnforced' = retrySameUnforced, retrySameForeign' = retrySameForeign,
  }
  action Launch(a: int): bool = all {
    al.get(a).st == "held",
    launchEnds(a, FOREIGN.contains(al.get(a).off), otherLeft),
  }
  action LaunchFailsOther(a: int): bool = all {
    al.get(a).st == "held", otherLeft > 0, not(FOREIGN.contains(al.get(a).off)),
    launchEnds(a, true, otherLeft - 1),
  }
  // pre-READY VMM exit: retire, then release the lease; placement will retry (exclusion kept)
  action RetireRelease(a: int): bool = all {
    al.get(a).st == "failed",
    lease' = lease.set(al.get(a).off, 0),
    al' = al.set(a, { ...al.get(a), st: "wait", off: -1, failedOff: al.get(a).off }),
    cursor' = cursor, keepGhosts,
    otherLeft' = otherLeft,
  }
  // a running allocation is stopped (or fails later, for an unrelated reason) and its lease released
  action StopRunning(a: int): bool = all {
    al.get(a).st == "running",
    lease' = lease.set(al.get(a).off, 0),
    al' = al.set(a, { st: "done", off: -1, failedOff: -1, clashed: Set() }),
    cursor' = cursor, keepGhosts,
    otherLeft' = otherLeft,
  }
  // a stopped workload is deployed again: a new allocation with no failure history
  action Redeploy(a: int): bool = all {
    al.get(a).st == "done",
    al' = al.set(a, { st: "wait", off: -1, failedOff: -1, clashed: Set() }),
    lease' = lease, cursor' = cursor,
    everClashed' = everClashed.set(a, Set()),
    retrySame' = retrySame, retrySameUnforced' = retrySameUnforced, retrySameForeign' = retrySameForeign,
    reclashInBoot' = reclashInBoot, reclashEver' = reclashEver, exclWhileUnexcl' = exclWhileUnexcl,
    otherLeft' = otherLeft,
  }
  // serve restart: in-memory pool, cursor and exclusion sets reset; VMs reclaimed; placement
  // intent survives (the same allocations are placed again)
  action Boot: bool = all {
    ALLOW_BOOT,
    lease' = OFFS.mapBy(o => 0),
    cursor' = 0,
    al' = al.keys().mapBy(a =>
      val x = al.get(a)
      if (x.st == "done") x
      else if (x.st == "failed") { st: "wait", off: -1, failedOff: x.off, clashed: Set() }
      else if (x.st == "wait") { ...x, clashed: Set() }
      else { st: "wait", off: -1, failedOff: -1, clashed: Set() }),
    keepGhosts,
    otherLeft' = otherLeft,
  }

  action step = {
    nondet a = ALLOCS.oneOf()
    any { Assign(a), Launch(a), LaunchFailsOther(a), RetireRelease(a), StopRunning(a), Redeploy(a), Boot }
  }

  // ------------------------------------------------------------------ invariants
  // Leased CIDs are unique (one lease per offset, every held/running allocation on its own offset).
  val CidUnique: bool = ALLOCS.forall(a => ALLOCS.forall(b =>
    (a != b and al.get(a).st.in(Set("held", "running", "failed")) and al.get(b).st.in(Set("held", "running", "failed")))
      implies al.get(a).off != al.get(b).off))
  // (5) a CID-clash retry never receives the same CID (r1 claim; with D16-PREF it fails when
  //     only excluded offsets are free — a documented consequence).
  val RetryNeverSameCid: bool = not(retrySame)
  // (5') ADR-0156: different "unless every other offset is held".
  val RetryDifferentUnlessAllHeld: bool = not(retrySameUnforced)
  // (5'') ADR-0156 consequence: "the retry does not hit the same foreign CID again".
  val RetryNeverSameForeignCid: bool = not(retrySameForeign)
  // r2 D16-EXCL: between two READY / stop events of a workload, within one serve process, its launch
  // never ends before READY twice on the same offset. With D16-PREF this holds only while an
  // unexcluded offset is free at each retry.
  val NoReclashWithinBoot: bool = not(reclashInBoot)
  // across serve restarts (expected to FAIL with ALLOW_BOOT: the exclusion set is in memory).
  val NoReclashEver: bool = not(reclashEver)
  // D16-PREF (the within-process guarantee): assign never returns an offset the workload
  // excluded while an offset it has not excluded is free.
  val NoExcludedWhileUnexcludedFree: bool = not(exclWhileUnexcl)
  // D16-PREF: a waiting workload is never refused while any offset is free.
  val NoRefusalWhileFree: bool = not(ALLOCS.exists(a => and {
    al.get(a).st == "wait", OFFS.exists(o => isFree(o)), pick(a) == -1 }))
  val Safety: bool = CidUnique and NoExcludedWhileUnexcludedFree and NoRefusalWhileFree

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoClash: bool = ALLOCS.forall(a => al.get(a).failedOff == -1)
  // a waiting workload is assigned an excluded offset (the fallback is reachable)
  val WitnessNoFallback: bool = not(ALLOCS.exists(a => and {
    al.get(a).st == "wait", OFFS.exists(o => isFree(o)),
    OFFS.forall(o => isFree(o) implies al.get(a).clashed.contains(o)) }))
  // ... the r2-1 shape: refused although a free offset is NOT a foreign CID (only non-clash
  // pre-READY exits excluded it). Holds under D16-PREF; violated by the r2 rule.
  val NoStarvedOnUsableOffset: bool = not(ALLOCS.exists(a => and {
    al.get(a).st == "wait", OFFS.exists(o => isFree(o) and not(FOREIGN.contains(o))), pick(a) == -1 }))

  // ------------------------------------------------------------------ liveness (TLC)
  // a waiting workload is eventually placed while a usable (non-foreign) offset stays free
  def usableFree: bool = OFFS.exists(o => isFree(o) and not(FOREIGN.contains(o)))
  temporal EventuallyPlaced =
    ALLOCS.forall(a => weakFair(Assign(a), (lease, al)))
      .implies(ALLOCS.forall(a => always((al.get(a).st == "wait" and usableFree).implies(
        eventually(al.get(a).st != "wait" or not(usableFree))))))
}

// 3 offsets, 3 allocations, offset 0 is a foreign CID; no restarts
module lease_ok {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2, 3), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "./cid_lease"
}
// two foreign offsets out of three, two allocations
module lease_twoForeign {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN = Set(0, 1),
    ALLOW_BOOT = false, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "./cid_lease"
}
// a single allocation on the node
module lease_single {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "./cid_lease"
}
// with serve restarts: what is guaranteed after a restart
module lease_boot {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN = Set(0),
    ALLOW_BOOT = true, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "./cid_lease"
}
module lease_single_boot {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1), FOREIGN = Set(0),
    ALLOW_BOOT = true, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 0, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "./cid_lease"
}
// one workload, transient (non-clash) pre-READY exits possible: what the exclusion costs
module lease_transient {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 2, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "./cid_lease"
}
// two workloads, transient pre-READY exits possible: the preference and the fallback under
// concurrent placement and release
module lease_transientTwo {
  import cid_lease(OFFS = Set(0, 1, 2), N = 3, ALLOCS = Set(1, 2), FOREIGN = Set(0),
    ALLOW_BOOT = false, BUG_FIRST_FIT = false, BUG_NO_EXCLUSION = false, OTHER_FAILURES = 2, BUG_REFUSE_WHEN_ALL_EXCLUDED = false).* from "./cid_lease"
}
