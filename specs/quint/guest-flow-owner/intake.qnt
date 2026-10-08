// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — steering concern 2: intake listeners and their steering
 * entries (D8a-LOOKUP, D8a-CAP, D23, U-6, U-3, M-7), and where each client class lands.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" — § Owner and provisioner
 * (`activate`, `teardown`, "One writer per allocation (M-7)", "Intake listener mirroring (D23)"),
 * § Driven port — guest-prefix steering (`steer`, "Capacity (D8a-CAP)", "Entry lifetime = listener
 * lifetime"), § Composition (teardown, reclaim and stop), error taxonomy, Gates G-V0 / G-V8,
 * § User decisions — 2026-10-08 (D8a-CAP); ADR-0171 (D8a-LOOKUP), ADR-0163 (D23, U-6, M-7),
 * ADR-0176 (D8a-CAP).
 *
 * The rules modelled:
 *   - admission (D8a-CAP): `GuestAddressPool::assign` reserves the allocation's declared TCP ports
 *     with its lease against the steering capacity CAP (the map's `max_entries`) and refuses
 *     `IntakeCapacityReached` (no state change) when the node's reservations would exceed it; a
 *     Retiring lease keeps its reservation until `pool.release`.
 *   - serving(p) ⇔ Active ∧ forwarding not quiesced ∧ session live ∧ last ListenState(p) =
 *     listening ∧ p declared. Bring-up: bind → listen (one step), then `steer` (Ok = entry present).
 *   - `steer` failures (D8a-CAP): `KernelMemory` (transient, A-MEM) closes the listener and the
 *     bring-up is retried; a defect variant closes the listener, is reported and not retried — the
 *     port comes back only when wanted anew (it stops serving: next ListenState change, session
 *     loss / reopen, quiescence / restore). A bind / listen failure closes the listener and is
 *     retried. The port is refused meanwhile. With admission, `steer` never fails for capacity; the
 *     kernel's capacity refusal is modelled so the hazards can reach it.
 *   - every per-allocation event (provision, session open / loss, ListenState, teardown) is one step
 *     of the per-CID order and closes, in the same step, every listener whose port stops serving;
 *     closing removes its steering entry in the same kernel step, freeing its slot (K-L3). No removal
 *     can fail.
 *   - activate is all or nothing (U-6): Ok = Active with a steered listener for every
 *     reported-listening declared port; a failure closes every listener of the CID → Provisioned.
 *   - teardown closes every listener and the session; `pool.release` only after it returns Ok (U-3).
 *   - quiescence (the holder detail is concern 3) closes every intake listener; activation waits.
 *
 * Abstraction:
 *   - ONE workload address, reused by successive allocations `gen`. Boot is one step (concern 1):
 *     this boot's program runs in the pinned link, the route is `local`, the nft rules complete
 *     (concern 4 models a partial table). A crash closes every listener; the owner's CID table and
 *     the pool's reservations are lost (every lease is gone in this module's abstraction).
 *   - ONE other allocation on the node, abstracted to its reservation `oRes` (0 or OD ports) and its
 *     steered listeners `oEnt` (each holds one map slot): it is admitted, steers, closes and is
 *     released by the environment, against the same capacity.
 *
 * ASSUMPTIONS: K-L1–K-L5 (V-26), K-D1, K-D2 (V-19), K-D5 (V-13), A-MEM — see prefix_landing.qnt /
 * README. A-MEM (a `KernelMemory` refusal is not permanent) is the fairness on a successful steer in
 * `ServingReachable`; each failed attempt closes the listener, so the steer is enabled only
 * intermittently and the fairness is strong (SF) on that action. `ServingReachableNoMemRecovery`
 * drops it.
 *
 * OFF names design rules switched off (hazard/intake_hazards.qnt):
 *   entryOutlives       K-L3: an entry survives its listener's close
 *   takedownBeforeClose D8a-LOOKUP: revision-8 order — a separate entry removal precedes the close
 *                       and can fail, keeping the listener bound
 *   rollbackPartial     U-6: the activation rollback closes only the failing port
 *   actOkPartial        U-6: activation returns Ok without every wanted port steered
 *   teardownKeeps       U-3: teardown returns Ok with listeners open
 *   quiesceKeeps        M-7: quiescence leaves intake listeners open
 *   noIntakeAdmission   D8a-CAP: admission does not reserve declared ports against the capacity
 *   capRetry            revision 10: a capacity steer failure is retried (not a reported defect)
 */
module intake {
  import prefix_landing.* from "./prefix_landing"

  const OFF: Set[str]
  const CFG: str   // "ok" | "two" | "live"

  def bug(r: str): bool = OFF.contains(r)
  val PORTS: Set[int] = Set(1, 2)
  val DECLARED: Set[int] = if (CFG == "two") Set(1, 2) else Set(1)
  val MAXGEN: int = 2
  val BIND_FAILS: int = 1
  val DEFECTS: bool = CFG != "live"   // a steer defect variant can occur (not retried)
  val CAP: int = 2                    // GUEST_INTAKE_STEERING_MAX (the steering map's max_entries)
  val OD: int = 2                     // declared TCP ports of the other allocation

  type Ghost = {
    actPartial: bool,   // activation returned Ok with a wanted port not steered
    actResidue: bool,   // activation returned Err with a listener of the CID left
    actErr: bool,       // an activation returned Err (witness)
    relListener: bool,  // pool.release ran while a listener of the allocation was open
    steerFailed: bool,  // a steer failed (witness)
    resteered: bool,    // a port was steered after a steer failure (witness)
    revokeKept: bool,   // takedownBeforeClose: a failed removal kept a non-serving listener bound
    capFail: bool,      // a steer (ours or the other allocation's) failed for map capacity
    capRefused: bool,   // an admission was refused IntakeCapacityReached (witness)
    retiringRefused: bool, // ... while our lease was Retiring (witness: its reservation is kept)
    defected: bool,     // a steer failure was reported and not retried (witness)
  }
  val g0: Ghost = { actPartial: false, actResidue: false, actErr: false, relListener: false,
                    steerFailed: false, resteered: false, revokeKept: false, capFail: false,
                    capRefused: false, retiringRefused: false, defected: false }

  var up: bool
  var lis: Set[int]       // ports with an open (bound, listening) intake listener
  var entries: Set[int]   // ports with a steering entry
  var alloc: str          // Absent | Provisioned | Activating | Active | Retiring
  var gen: int
  var sess: bool
  var lastLS: Set[int]    // ports whose last ListenState says listening
  var quiesced: bool
  var parked: Set[int]    // serving ports whose bring-up a reported steer defect stopped
  var oRes: int           // the other allocation's reserved ports (0 or OD)
  var oEnt: int           // the other allocation's steered listeners (one map slot each)
  var bindLeft: int
  var g: Ghost

  def servingWith(a: str, q: bool, s: bool, ls: Set[int], p: int): bool = and {
    up, not(q), s, ls.contains(p), DECLARED.contains(p), a.in(Set("Active", "Activating")),
  }
  // the design's serving set (for the properties)
  def servingTrue(p: int): bool = servingWith(alloc, quiesced, sess, lastLS, p)
  // the owner's serving set (quiesceKeeps: quiescence ignored)
  def serving(p: int): bool = servingWith(alloc, quiesced and not(bug("quiesceKeeps")), sess, lastLS, p)
  // (D8a-CAP) our lease's reservation: its declared ports while the lease is held (Retiring included)
  def resOf(a: str): int = if (a.in(Set("Provisioned", "Activating", "Active", "Retiring"))) DECLARED.size() else 0
  val res: int = resOf(alloc)
  def admits(mine: int, other: int, want: int): bool = bug("noIntakeAdmission") or mine + other + want <= CAP
  // the steering map has a free slot
  val room: bool = entries.size() + oEnt < CAP

  val net: Net = {
    marked: up, legC: up, divert: true, reject: true, route: "local", steer: "ours",
    entries: entries, lis: lis, assignRemote: false,
  }
  def landsAt(c: str, p: int): str = lands(net, c, p)
  def dec(x: int): int = if (x < 0) x else x - 1

  action keepAlloc = all { alloc' = alloc, gen' = gen, sess' = sess, lastLS' = lastLS }
  action keepOther = all { oRes' = oRes, oEnt' = oEnt }

  action init = all {
    up' = true, lis' = Set(), entries' = Set(), alloc' = "Absent", gen' = 0, sess' = false,
    lastLS' = Set(), quiesced' = false, parked' = Set(), oRes' = 0, oEnt' = 0,
    bindLeft' = BIND_FAILS, g' = g0,
  }

  // ------------------------------------------------------------------ per-allocation events (M-7)
  // The event and the close of every listener whose port stops serving are one step; the close
  // removes the entry in the same kernel step (K-L3). takedownBeforeClose: nothing is closed here.
  // A parked port is released when it stops serving (it is wanted anew when it serves again).
  action applyEvent(a: str, n: int, s: bool, ls: Set[int], q: bool, keepAll: bool): bool = all {
    val keep = lis.filter(p => keepAll or bug("takedownBeforeClose") or
                               servingWith(a, q and not(bug("quiesceKeeps")), s, ls, p))
    all {
      lis' = keep,
      entries' = if (bug("entryOutlives") or bug("takedownBeforeClose")) entries
                 else entries.filter(p => keep.contains(p)),
      parked' = parked.filter(p => servingWith(a, q, s, ls, p)),
      alloc' = a, gen' = n, sess' = s, lastLS' = ls, quiesced' = q,
    },
  }
  val evGuard: bool = up and alloc != "Activating"
  // admission (pool.assign, D8a-CAP): reserves the declared ports with the lease ...
  action Provision = all { evGuard, alloc == "Absent", gen < MAXGEN, admits(0, oRes, DECLARED.size()),
    applyEvent("Provisioned", gen + 1, false, Set(), quiesced, false), up' = up, keepOther,
    bindLeft' = bindLeft, g' = g }
  // ... or refuses IntakeCapacityReached with no state change (non-terminal; placement retries)
  action ProvisionRefused = all { evGuard, alloc == "Absent", gen < MAXGEN, not(admits(0, oRes, DECLARED.size())),
    up' = up, lis' = lis, entries' = entries, keepAlloc, quiesced' = quiesced, parked' = parked,
    keepOther, bindLeft' = bindLeft, g' = { ...g, capRefused: true } }
  // a session opens with every port not listening; the guest re-sends full state
  action SessOpen = all { evGuard, alloc.in(Set("Provisioned", "Active")), not(sess),
    applyEvent(alloc, gen, true, Set(), quiesced, false), up' = up, keepOther, bindLeft' = bindLeft, g' = g }
  // session loss closes every intake listener of the CID
  action SessLoss = all { evGuard, sess,
    applyEvent(alloc, gen, false, lastLS, quiesced, false), up' = up, keepOther, bindLeft' = bindLeft, g' = g }
  action ListenState(p: int, v: bool): bool = all {
    evGuard, sess, alloc.in(Set("Provisioned", "Active")), lastLS.contains(p) != v,
    applyEvent(alloc, gen, sess, if (v) lastLS.union(Set(p)) else lastLS.exclude(Set(p)), quiesced, false),
    up' = up, keepOther, bindLeft' = bindLeft, g' = g,
  }
  // teardown: Retiring (the reservation kept); listeners (and their entries) and the session closed; Ok
  action Teardown = all { evGuard, alloc.in(Set("Provisioned", "Active")),
    applyEvent("Retiring", gen, false, lastLS, quiesced, bug("teardownKeeps")), up' = up, keepOther,
    bindLeft' = bindLeft, g' = g }
  // pool.release: only after teardown returned Ok; frees the reservation
  action Release = all { evGuard, alloc == "Retiring",
    alloc' = "Absent", gen' = gen, sess' = false, lastLS' = Set(), lis' = lis, entries' = entries,
    quiesced' = quiesced, parked' = Set(), up' = up, keepOther, bindLeft' = bindLeft,
    g' = { ...g, relListener: g.relListener or lis != Set() } }
  // quiescence (first holder): closes every intake listener of every allocation; serialized with activation
  action Quiesce = all { evGuard, not(quiesced),
    applyEvent(alloc, gen, sess, lastLS, true, false), up' = up, oRes' = oRes,
    oEnt' = if (bug("quiesceKeeps")) oEnt else 0, bindLeft' = bindLeft, g' = g }
  // restore (last holder): reopens; listeners come back through the owner's bring-up
  action Restore = all { evGuard, quiesced,
    applyEvent(alloc, gen, sess, lastLS, false, false), up' = up, keepOther, bindLeft' = bindLeft, g' = g }

  // ------------------------------------------------------------------ the other allocation (environment)
  action keepMine = all { up' = up, lis' = lis, entries' = entries, keepAlloc, quiesced' = quiesced,
                          parked' = parked, bindLeft' = bindLeft }
  // its admission against the same capacity (our lease's reservation counts while Retiring)
  action OtherAdmit = all { up, oRes == 0,
    if (admits(res, 0, OD)) all { oRes' = OD, oEnt' = 0, g' = g }
    else all { keepOther, g' = { ...g, capRefused: true, retiringRefused: g.retiringRefused or alloc == "Retiring" } },
    keepMine }
  // one of its listeners is steered (the kernel refuses an insert into a full map)
  action OtherSteer = all { up, not(quiesced), oEnt < oRes,
    if (room) all { oEnt' = oEnt + 1, oRes' = oRes, g' = g }
    else all { keepOther, g' = { ...g, capFail: true } },
    keepMine }
  // one of its listeners closes (its entry and slot go with it, K-L3)
  action OtherClose = all { up, oEnt > 0, oEnt' = oEnt - 1, oRes' = oRes, keepMine, g' = g }
  // its teardown closes every listener; release frees its reservation
  action OtherRelease = all { up, oRes > 0, oRes' = 0, oEnt' = 0, keepMine, g' = g }

  // ------------------------------------------------------------------ activation (all or nothing)
  action ActivateBegin = all { evGuard, alloc == "Provisioned", sess, not(quiesced),
    alloc' = "Activating", gen' = gen, sess' = sess, lastLS' = lastLS, lis' = lis, entries' = entries,
    quiesced' = quiesced, parked' = parked, up' = up, keepOther, bindLeft' = bindLeft, g' = g }
  action ActivateOk = all {
    up, alloc == "Activating",
    bug("actOkPartial") or PORTS.forall(p => serving(p) implies (lis.contains(p) and entries.contains(p))),
    alloc' = "Active", gen' = gen, sess' = sess, lastLS' = lastLS, lis' = lis, entries' = entries,
    quiesced' = quiesced, parked' = parked, up' = up, keepOther, bindLeft' = bindLeft,
    g' = { ...g, actPartial: g.actPartial or PORTS.exists(p => serving(p) and not(entries.contains(p))) },
  }
  // a bind / listen / steer failure: during activation the rollback closes every listener of the CID
  // (cannot fail) and returns Err with the CID Provisioned; otherwise the listener is closed and the
  // port refused meanwhile — retried later, or (park: a reported defect) parked until wanted anew
  action bringUpFails(p: int, steerFail: bool, park: bool): bool = all {
    val act = alloc == "Activating"
    val left = if (act) (if (bug("rollbackPartial")) lis.exclude(Set(p)) else Set()) else lis.exclude(Set(p))
    all {
      lis' = left,
      entries' = if (bug("entryOutlives")) entries else entries.filter(q => left.contains(q)),
      alloc' = if (act) "Provisioned" else alloc,
      parked' = if (park and not(act)) parked.union(Set(p)) else parked,
      g' = { ...g, actErr: g.actErr or act, actResidue: g.actResidue or (act and left != Set()),
             steerFailed: g.steerFailed or steerFail, defected: g.defected or (park and not(act)) },
    },
    gen' = gen, sess' = sess, lastLS' = lastLS, quiesced' = quiesced, up' = up, keepOther,
  }

  // ------------------------------------------------------------------ owner listener steps
  action BindListen(p: int): bool = all { up, serving(p), not(lis.contains(p)), not(parked.contains(p)),
    lis' = lis.union(Set(p)), entries' = entries, keepAlloc, quiesced' = quiesced, parked' = parked,
    up' = up, keepOther, bindLeft' = bindLeft, g' = g }
  action BindFail(p: int): bool = all { up, serving(p), not(lis.contains(p)), not(parked.contains(p)),
    bindLeft != 0, bringUpFails(p, false, false), bindLeft' = dec(bindLeft) }
  def steerable(p: int): bool = and { up, serving(p), lis.contains(p), not(entries.contains(p)) }
  // GuestPrefixSteering::steer after listen: Ok = the entry is present (a free slot exists)
  action Steer(p: int): bool = all { steerable(p), room,
    entries' = entries.union(Set(p)), lis' = lis, keepAlloc, quiesced' = quiesced, parked' = parked,
    up' = up, keepOther, bindLeft' = bindLeft, g' = { ...g, resteered: g.resteered or g.steerFailed } }
  // steer → KernelMemory (transient, A-MEM): the listener is closed and the bring-up retried
  action SteerMemFail(p: int): bool = all { steerable(p),
    bringUpFails(p, true, false), bindLeft' = bindLeft }
  // steer → a defect variant: reported, the listener closed, not retried (the port is parked)
  action SteerDefect(p: int): bool = all { DEFECTS, steerable(p),
    bringUpFails(p, true, true), bindLeft' = bindLeft }
  // the kernel refuses the insert into a full map (unreachable with admission): a defect variant,
  // not retried — capRetry (revision 10): retried like KernelMemory
  action SteerCapFail(p: int): bool = all { steerable(p), not(room),
    bringUpFails(p, true, not(bug("capRetry"))), bindLeft' = bindLeft }
  // takedownBeforeClose: remove the entry, then close the listener (one step when the removal works) ...
  action TakeDown(p: int): bool = all { bug("takedownBeforeClose"), up, lis.contains(p), not(serving(p)),
    lis' = lis.exclude(Set(p)), entries' = entries.exclude(Set(p)), keepAlloc, quiesced' = quiesced,
    parked' = parked, up' = up, keepOther, bindLeft' = bindLeft, g' = g }
  // ... the removal fails: the listener stays bound (revision 8)
  action RevokeFail(p: int): bool = all { bug("takedownBeforeClose"), up, lis.contains(p),
    entries.contains(p), not(serving(p)),
    lis' = lis, entries' = entries, keepAlloc, quiesced' = quiesced, parked' = parked, up' = up,
    keepOther, bindLeft' = bindLeft, g' = { ...g, revokeKept: true } }

  // ------------------------------------------------------------------ crash / restart
  action Crash = all { up,
    up' = false, lis' = Set(), entries' = if (bug("entryOutlives")) entries else Set(),
    alloc' = "Absent", gen' = gen, sess' = false, lastLS' = Set(), quiesced' = false, parked' = Set(),
    oRes' = 0, oEnt' = 0, bindLeft' = bindLeft, g' = g }
  action Restart = all { not(up),
    up' = true, lis' = lis, entries' = entries, keepAlloc, quiesced' = quiesced, parked' = parked,
    keepOther, bindLeft' = bindLeft, g' = g }

  action OwnerStep = {
    nondet p = PORTS.oneOf()
    // the owner's steer call: Ok, or (map full) the kernel's capacity refusal
    any { BindListen(p), Steer(p), SteerCapFail(p), TakeDown(p), ActivateOk }
  }
  action Env = {
    nondet p = PORTS.oneOf()
    nondet v = Set(true, false).oneOf()
    any {
      BindFail(p), SteerMemFail(p), SteerDefect(p), RevokeFail(p), ListenState(p, v),
      Provision, ProvisionRefused, SessOpen, SessLoss, ActivateBegin, Teardown, Release, Quiesce,
      Restore, Crash, Restart, OtherAdmit, OtherSteer, OtherClose, OtherRelease,
    }
  }
  action step = any { OwnerStep, Env }

  // ------------------------------------------------------------------ invariants
  // (7, D8a-LOOKUP) traffic to a workload address never reaches a wildcard host service
  val NoWildcardReached: bool = not(exposedOn(net, PORTS))
  // (7, D8a-LOOKUP, G-V8) a port that is not serving is never reachable (no connect completes on it)
  val ReachOnlyServing: bool = CLIENTS.forall(c => PORTS.forall(p => landsAt(c, p) == "intake" implies servingTrue(p)))
  // (7, D8a-LOOKUP) every steering entry names an open intake listener of a serving port
  val EntryNamesServingListener: bool = entries.forall(p => lis.contains(p) and servingTrue(p))
  // (K-L4) while serve is down nothing to the prefix is delivered to a host socket
  val NothingDeliveredWhileDown: bool = not(up) implies not(deliveredOn(net, PORTS))
  // (K-L5) remote traffic never reaches an intake directly
  val RemoteNeverDirect: bool = PORTS.forall(p => landsAt("remote", p) != "intake")
  // (ADR-0152 point 2) unmarked host-local traffic stays on leg-C (complete nft rules)
  val UnmarkedOnLegC: bool = PORTS.forall(p => landsAt("unmarked", p) != "intake")
  // (8, U-6) activation is all or nothing
  val ActivationAllOrNothing: bool = not(g.actPartial) and not(g.actResidue)
  // (8, U-6) a Provisioned CID has no listener
  val ProvisionedHasNoListener: bool = alloc == "Provisioned" implies lis == Set()
  // (7, U-3) no lease is released while a listener of its allocation is open
  val NoReleaseWithListener: bool = not(g.relListener)
  // (M-7, D8a-HOLD) while forwarding is quiesced no intake listener or entry exists
  val QuiescedClosesListeners: bool = quiesced implies (lis == Set() and entries == Set() and oEnt == 0)
  // (12, D8a-CAP) the declared ports reserved by held leases (Retiring included) never exceed the
  // steering capacity
  val ReservedWithinCapacity: bool = res + oRes <= CAP
  // (12, D8a-CAP) open intake listeners never exceed the reserved ports
  val ListenersWithinReserved: bool = lis.size() + oEnt <= res + oRes
  // (12, D8a-CAP) steering entries never exceed open intake listeners
  val EntriesWithinListeners: bool = entries.size() <= lis.size()
  // (12, D8a-CAP) no steer fails for capacity
  val NoCapacitySteerFailure: bool = not(g.capFail)
  // a parked port is a serving one (it is released when the port stops serving)
  val ParkedOnlyWhileServing: bool = parked.forall(p => servingTrue(p) and not(lis.contains(p)))

  val Safety: bool = and {
    NoWildcardReached, ReachOnlyServing, EntryNamesServingListener, NothingDeliveredWhileDown,
    RemoteNeverDirect, UnmarkedOnLegC, ActivationAllOrNothing, ProvisionedHasNoListener,
    NoReleaseWithListener, QuiescedClosesListeners, ReservedWithinCapacity, ListenersWithinReserved,
    EntriesWithinListeners, NoCapacitySteerFailure, ParkedOnlyWhileServing,
  }

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoSteered: bool = not(alloc == "Active" and entries != Set())
  val WitnessNoSecondAllocation: bool = gen < 2 or alloc != "Active"
  val WitnessNoActivationErr: bool = not(g.actErr)
  val WitnessNoResteer: bool = not(g.resteered)
  // unmarked host-local traffic to a served, steered port lands on leg-C (not on the intake)
  val WitnessNoLegCWhileServed: bool = not(PORTS.exists(p => and {
    servingTrue(p), entries.contains(p), landsAt("marked", p) == "intake", landsAt("unmarked", p) == "legC",
  }))
  // a serving port refused to a marked client (its steering being retried)
  val WitnessNoServingRefused: bool = not(PORTS.exists(p => servingTrue(p) and landsAt("marked", p) == "refused"))
  // an admission refused IntakeCapacityReached
  val WitnessNoCapacityRefusal: bool = not(g.capRefused)
  // ... while our lease was Retiring (its reservation kept until release)
  val WitnessNoRetiringRefusal: bool = not(g.retiringRefused)
  // the map full with our listener steered and the other allocation's (the capacity reached exactly)
  val WitnessNoFullMap: bool = not(entries != Set() and entries.size() + oEnt == CAP)
  // a steer defect reported (the port parked)
  val WitnessNoDefect: bool = not(g.defected)

  // ------------------------------------------------------------------ liveness (TLC)
  val ownerVars = (lis, entries, alloc)
  // (D8a-CAP, A-MEM) a serving declared port eventually becomes reachable (steered) — or stops
  // serving, or a reported defect parks it — under weak fairness of the owner and A-MEM (a retried
  // steer eventually succeeds: strong fairness on Steer(1), enabled intermittently between retries)
  temporal ServingReachable =
    and { weakFair(OwnerStep, ownerVars), strongFair(Steer(1), ownerVars) }
      .implies(always((servingTrue(1) and not(parked.contains(1))).implies(eventually(
        landsAt("marked", 1) == "intake" or not(servingTrue(1)) or parked.contains(1)))))
  // assumption off: kernel memory never recovers (no fairness on the steer's success)
  temporal ServingReachableNoMemRecovery =
    weakFair(OwnerStep, ownerVars)
      .implies(always((servingTrue(1) and not(parked.contains(1))).implies(eventually(
        landsAt("marked", 1) == "intake" or not(servingTrue(1)) or parked.contains(1)))))
  // an activation eventually returns
  temporal ActivationCompletes =
    weakFair(OwnerStep, ownerVars)
      .implies(always((alloc == "Activating").implies(eventually(alloc != "Activating"))))
}

// ------------------------------------------------------------------ design instances
// one declared port, crash at any step, one bind failure, KernelMemory steer failures unbounded,
// steer defects, the other allocation competing for the capacity
module intake_ok { import intake(OFF = Set(), CFG = "ok").* from "./intake" }
// two declared ports: an activation can fail on one port after steering the other (rollback)
module intake_two { import intake(OFF = Set(), CFG = "two").* from "./intake" }
// liveness: as intake_ok without steer defects (no port is ever parked)
module intake_live { import intake(OFF = Set(), CFG = "live").* from "./intake" }
