// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — steering concern 2: intake listeners and their steering
 * entries (D8a-LOOKUP, D23, U-6, U-3, M-7), and where each client class lands.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" — § Owner and provisioner
 * (`activate`, `teardown`, "One writer per allocation (M-7)", "Intake listener mirroring (D23)"),
 * § Driven port — guest-prefix steering (`steer`, "Entry lifetime = listener lifetime"),
 * § Composition (teardown, reclaim and stop), Gate G-V8, § Revision 9 decisions items 1, 4, 6;
 * ADR-0171 (D8a-LOOKUP), ADR-0163 (D23, U-6, M-7).
 *
 * The rules modelled:
 *   - serving(p) ⇔ Active ∧ forwarding not quiesced ∧ session live ∧ last ListenState(p) =
 *     listening ∧ p declared. Bring-up: bind → listen (one step), then `steer` (Ok = entry present).
 *     A bind / listen / steer failure closes the listener; the port is refused meanwhile and the
 *     bring-up retried (steer may fail forever).
 *   - every per-allocation event (provision, session open / loss, ListenState, teardown) is one step
 *     of the per-CID order and closes, in the same step, every listener whose port stops serving;
 *     closing removes its steering entry in the same kernel step (K-L3). No removal can fail.
 *   - activate is all or nothing (U-6): Ok = Active with a steered listener for every
 *     reported-listening declared port; a failure closes every listener of the CID → Provisioned.
 *   - teardown closes every listener and the session; `pool.release` only after it returns Ok (U-3).
 *   - quiescence (the holder detail is concern 3) closes every intake listener; activation waits.
 *
 * Abstraction:
 *   - ONE workload address, reused by successive allocations `gen`. Boot is one step (concern 1):
 *     the steering is verified and pinned, the route `local`, the nft rules complete (concern 4
 *     models a partial table). A crash closes every listener; the owner's CID table is lost.
 *
 * ASSUMPTIONS: K-L1–K-L5 (V-26), K-D1, K-D2 (V-19), K-D5 (V-13) — see prefix_landing.qnt / README.
 *
 * OFF names design rules switched off (hazard/intake_hazards.qnt):
 *   entryOutlives       K-L3: an entry survives its listener's close
 *   takedownBeforeClose D8a-LOOKUP: revision-8 order — a separate entry removal precedes the close
 *                       and can fail, keeping the listener bound
 *   rollbackPartial     U-6: the activation rollback closes only the failing port
 *   actOkPartial        U-6: activation returns Ok without every wanted port steered
 *   teardownKeeps       U-3: teardown returns Ok with listeners open
 *   quiesceKeeps        M-7: quiescence leaves intake listeners open
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
  val STEER_FAILS: int = if (CFG == "ok") -1 else 1   // -1: may fail forever

  type Ghost = {
    actPartial: bool,   // activation returned Ok with a wanted port not steered
    actResidue: bool,   // activation returned Err with a listener of the CID left
    actErr: bool,       // an activation returned Err (witness)
    relListener: bool,  // pool.release ran while a listener of the allocation was open
    steerFailed: bool,  // a steer failed (witness)
    resteered: bool,    // a port was steered after a steer failure (witness)
    revokeKept: bool,   // takedownBeforeClose: a failed removal kept a non-serving listener bound
  }
  val g0: Ghost = { actPartial: false, actResidue: false, actErr: false, relListener: false,
                    steerFailed: false, resteered: false, revokeKept: false }

  var up: bool
  var lis: Set[int]       // ports with an open (bound, listening) intake listener
  var entries: Set[int]   // ports with a steering entry
  var alloc: str          // Absent | Provisioned | Activating | Active | Retiring
  var gen: int
  var sess: bool
  var lastLS: Set[int]    // ports whose last ListenState says listening
  var quiesced: bool
  var bindLeft: int
  var steerLeft: int
  var g: Ghost

  def servingWith(a: str, q: bool, s: bool, ls: Set[int], p: int): bool = and {
    up, not(q), s, ls.contains(p), DECLARED.contains(p), a.in(Set("Active", "Activating")),
  }
  // the design's serving set (for the properties)
  def servingTrue(p: int): bool = servingWith(alloc, quiesced, sess, lastLS, p)
  // the owner's serving set (quiesceKeeps: quiescence ignored)
  def serving(p: int): bool = servingWith(alloc, quiesced and not(bug("quiesceKeeps")), sess, lastLS, p)

  val net: Net = {
    marked: up, legC: up, divert: true, reject: true, route: "local", steer: "ours",
    entries: entries, lis: lis, assignRemote: false,
  }
  def landsAt(c: str, p: int): str = lands(net, c, p)
  def dec(x: int): int = if (x < 0) x else x - 1

  action keepAlloc = all { alloc' = alloc, gen' = gen, sess' = sess, lastLS' = lastLS }
  action keepCounters = all { bindLeft' = bindLeft, steerLeft' = steerLeft }

  action init = all {
    up' = true, lis' = Set(), entries' = Set(), alloc' = "Absent", gen' = 0, sess' = false,
    lastLS' = Set(), quiesced' = false, bindLeft' = BIND_FAILS, steerLeft' = STEER_FAILS, g' = g0,
  }

  // ------------------------------------------------------------------ per-allocation events (M-7)
  // The event and the close of every listener whose port stops serving are one step; the close
  // removes the entry in the same kernel step (K-L3). takedownBeforeClose: nothing is closed here.
  action applyEvent(a: str, n: int, s: bool, ls: Set[int], q: bool, keepAll: bool): bool = all {
    val keep = lis.filter(p => keepAll or bug("takedownBeforeClose") or
                               servingWith(a, q and not(bug("quiesceKeeps")), s, ls, p))
    all {
      lis' = keep,
      entries' = if (bug("entryOutlives") or bug("takedownBeforeClose")) entries
                 else entries.filter(p => keep.contains(p)),
      alloc' = a, gen' = n, sess' = s, lastLS' = ls, quiesced' = q,
    },
  }
  val evGuard: bool = up and alloc != "Activating"
  action Provision = all { evGuard, alloc == "Absent", gen < MAXGEN,
    applyEvent("Provisioned", gen + 1, false, Set(), quiesced, false), up' = up, keepCounters, g' = g }
  // a session opens with every port not listening; the guest re-sends full state
  action SessOpen = all { evGuard, alloc.in(Set("Provisioned", "Active")), not(sess),
    applyEvent(alloc, gen, true, Set(), quiesced, false), up' = up, keepCounters, g' = g }
  // session loss closes every intake listener of the CID
  action SessLoss = all { evGuard, sess,
    applyEvent(alloc, gen, false, lastLS, quiesced, false), up' = up, keepCounters, g' = g }
  action ListenState(p: int, v: bool): bool = all {
    evGuard, sess, alloc.in(Set("Provisioned", "Active")), lastLS.contains(p) != v,
    applyEvent(alloc, gen, sess, if (v) lastLS.union(Set(p)) else lastLS.exclude(Set(p)), quiesced, false),
    up' = up, keepCounters, g' = g,
  }
  // teardown: Retiring; listeners (and their entries) and the session closed; Ok
  action Teardown = all { evGuard, alloc.in(Set("Provisioned", "Active")),
    applyEvent("Retiring", gen, false, lastLS, quiesced, bug("teardownKeeps")), up' = up, keepCounters, g' = g }
  // pool.release: only after teardown returned Ok
  action Release = all { evGuard, alloc == "Retiring",
    alloc' = "Absent", gen' = gen, sess' = false, lastLS' = Set(), lis' = lis, entries' = entries,
    quiesced' = quiesced, up' = up, keepCounters, g' = { ...g, relListener: g.relListener or lis != Set() } }
  // quiescence (first holder): closes every intake listener; serialized with activation
  action Quiesce = all { evGuard, not(quiesced),
    applyEvent(alloc, gen, sess, lastLS, true, false), up' = up, keepCounters, g' = g }
  // restore (last holder): reopens; listeners come back through the owner's bring-up
  action Restore = all { evGuard, quiesced,
    applyEvent(alloc, gen, sess, lastLS, false, false), up' = up, keepCounters, g' = g }

  // ------------------------------------------------------------------ activation (all or nothing)
  action ActivateBegin = all { evGuard, alloc == "Provisioned", sess, not(quiesced),
    alloc' = "Activating", gen' = gen, sess' = sess, lastLS' = lastLS, lis' = lis, entries' = entries,
    quiesced' = quiesced, up' = up, keepCounters, g' = g }
  action ActivateOk = all {
    up, alloc == "Activating",
    bug("actOkPartial") or PORTS.forall(p => serving(p) implies (lis.contains(p) and entries.contains(p))),
    alloc' = "Active", gen' = gen, sess' = sess, lastLS' = lastLS, lis' = lis, entries' = entries,
    quiesced' = quiesced, up' = up, keepCounters,
    g' = { ...g, actPartial: g.actPartial or PORTS.exists(p => serving(p) and not(entries.contains(p))) },
  }
  // a bind / listen / steer failure: during activation the rollback closes every listener of the CID
  // (cannot fail) and returns Err with the CID Provisioned; otherwise the listener is closed, the
  // port refused meanwhile, the bring-up retried later
  action bringUpFails(p: int, steerFail: bool): bool = all {
    val act = alloc == "Activating"
    val left = if (act) (if (bug("rollbackPartial")) lis.exclude(Set(p)) else Set()) else lis.exclude(Set(p))
    all {
      lis' = left,
      entries' = if (bug("entryOutlives")) entries else entries.filter(q => left.contains(q)),
      alloc' = if (act) "Provisioned" else alloc,
      g' = { ...g, actErr: g.actErr or act, actResidue: g.actResidue or (act and left != Set()),
             steerFailed: g.steerFailed or steerFail },
    },
    gen' = gen, sess' = sess, lastLS' = lastLS, quiesced' = quiesced, up' = up,
  }

  // ------------------------------------------------------------------ owner listener steps
  action BindListen(p: int): bool = all { up, serving(p), not(lis.contains(p)),
    lis' = lis.union(Set(p)), entries' = entries, keepAlloc, quiesced' = quiesced, up' = up,
    keepCounters, g' = g }
  action BindFail(p: int): bool = all { up, serving(p), not(lis.contains(p)), bindLeft != 0,
    bringUpFails(p, false), bindLeft' = dec(bindLeft), steerLeft' = steerLeft }
  // GuestPrefixSteering::steer after listen: Ok = the entry is present
  action Steer(p: int): bool = all { up, serving(p), lis.contains(p), not(entries.contains(p)),
    entries' = entries.union(Set(p)), lis' = lis, keepAlloc, quiesced' = quiesced, up' = up,
    keepCounters, g' = { ...g, resteered: g.resteered or g.steerFailed } }
  action SteerFail(p: int): bool = all { up, serving(p), lis.contains(p), not(entries.contains(p)),
    steerLeft != 0,
    bringUpFails(p, true), steerLeft' = dec(steerLeft), bindLeft' = bindLeft }
  // takedownBeforeClose: remove the entry, then close the listener (one step when the removal works) ...
  action TakeDown(p: int): bool = all { bug("takedownBeforeClose"), up, lis.contains(p), not(serving(p)),
    lis' = lis.exclude(Set(p)), entries' = entries.exclude(Set(p)), keepAlloc, quiesced' = quiesced,
    up' = up, keepCounters, g' = g }
  // ... the removal fails: the listener stays bound (revision 8)
  action RevokeFail(p: int): bool = all { bug("takedownBeforeClose"), up, lis.contains(p),
    entries.contains(p), not(serving(p)),
    lis' = lis, entries' = entries, keepAlloc, quiesced' = quiesced, up' = up, keepCounters,
    g' = { ...g, revokeKept: true } }

  // ------------------------------------------------------------------ crash / restart
  action Crash = all { up,
    up' = false, lis' = Set(), entries' = if (bug("entryOutlives")) entries else Set(),
    alloc' = "Absent", gen' = gen, sess' = false, lastLS' = Set(), quiesced' = false, keepCounters, g' = g }
  action Restart = all { not(up),
    up' = true, lis' = lis, entries' = entries, keepAlloc, quiesced' = quiesced, keepCounters, g' = g }

  action OwnerStep = {
    nondet p = PORTS.oneOf()
    any { BindListen(p), Steer(p), TakeDown(p), ActivateOk }
  }
  action Env = {
    nondet p = PORTS.oneOf()
    nondet v = Set(true, false).oneOf()
    any {
      BindFail(p), SteerFail(p), RevokeFail(p), ListenState(p, v), Provision, SessOpen, SessLoss,
      ActivateBegin, Teardown, Release, Quiesce, Restore, Crash, Restart,
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
  val QuiescedClosesListeners: bool = quiesced implies (lis == Set() and entries == Set())

  val Safety: bool = and {
    NoWildcardReached, ReachOnlyServing, EntryNamesServingListener, NothingDeliveredWhileDown,
    RemoteNeverDirect, UnmarkedOnLegC, ActivationAllOrNothing, ProvisionedHasNoListener,
    NoReleaseWithListener, QuiescedClosesListeners,
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

  // ------------------------------------------------------------------ liveness (TLC)
  val ownerVars = (lis, entries, alloc)
  // a serving declared port eventually becomes reachable (steered), once steer failures stop
  temporal ServingReachable =
    weakFair(OwnerStep, ownerVars)
      .implies(always(servingTrue(1).implies(eventually(landsAt("marked", 1) == "intake" or not(servingTrue(1))))))
  // an activation eventually returns
  temporal ActivationCompletes =
    weakFair(OwnerStep, ownerVars)
      .implies(always((alloc == "Activating").implies(eventually(alloc != "Activating"))))
}

// ------------------------------------------------------------------ design instances
// one declared port, crash at any step, one bind failure, steer failing forever (R5-22)
module intake_ok { import intake(OFF = Set(), CFG = "ok").* from "./intake" }
// two declared ports: an activation can fail on one port after steering the other (rollback)
module intake_two { import intake(OFF = Set(), CFG = "two").* from "./intake" }
// liveness: steer fails once
module intake_live { import intake(OFF = Set(), CFG = "live").* from "./intake" }
