// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — module D: boot convergence of the guest-prefix steering
 * rules and the shared `local` route; intake listeners mirroring guest listen state (with
 * all-or-nothing activation and revoke retry); where a connection to a workload address lands.
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "vsock Attachment Replacement
 * DESIGN": § Composition (boot steps 7-9, D8a ordering invariant), § Owner and provisioner
 * (activate "all or nothing", converge_shared, quiesce_forwarding / restore_forwarding, M-7
 * per-allocation order, `bound(port) ⇔ Active ∧ ¬Quiesced ∧ session Live ∧ ListenState ∧ declared`),
 * § Intake listener mirroring (D23: bind → listen → register → admit; revoke → unregister → close),
 * § mTLS forwarded-outbound port (admit_intake_listener / IntakeAdmission::revoke), § Lifecycle,
 * ADR-0152 (D8a), ADR-0163 (D23); and the user decisions of 2026-10-06 that followed the first
 * model check (docs/feature/netns-density-295/spike/quint-owner-findings{,-r2}.md):
 *   D8a-REVOKE  A failed element removal leaves the intake listener bound and RESETTING what it
 *          accepts (`rst`), reported and retried at the audit cadence; the listener closes only
 *          once its element is gone; a lease is never released while an element exists (teardown
 *          returns Err until then). A resetting listener whose port is wanted again serves normally.
 *   D8a-ROUTE  converge_shared keeps or adds the shared route only when the steering rules are
 *          present and verified; otherwise it removes the Overdrive-tagged route and startup
 *          refuses (fail closed).
 *   U-4    The shared route is never removed at shutdown, graceful or not.
 *   U-6    Activation is all or nothing: Ok = Active with an admitted listener for every wanted
 *          port; a bring-up failure returns Err with the CID Provisioned after taking down what it
 *          bound — a rollback revoke that fails leaves that listener resetting (D8a-REVOKE).
 * and the user decisions of 2026-10-06 that followed the second model check
 * (spike/quint-owner-findings-r2.md findings r2-2, U-r2-4):
 *   D8a-READMIT  On restore_forwarding, every kept or re-established intake listener has its
 *          element (re-)admitted before it serves: a resetting listener whose port is wanted again
 *          is re-admitted (`admit_intake_listener`, an already-present element counts as present),
 *          not assumed to still hold its element. Firewall (IpRules) repair runs only inside
 *          quiescence: quiesce → repair → audit → restore.
 *   D8a-PROBE / D8a-FLUSH  Accepted, bounded design behaviour (not defects): a resetting listener
 *          refuses marked probes while its removal is retried; a foreign flush exposes host services
 *          for a bounded window that ends at the audit's repair (serve up) or at the next boot's
 *          start_shared_owner.
 *
 * Abstraction:
 *   - ONE workload address (one lease, reused by successive allocations `gen`). PORTS: guest ports;
 *     DECLARED ⊆ PORTS are the allocation's declared TCP listen ports.
 *   - Node-global kernel objects survive a `serve` crash or exit: `rules` (D8a steering rules incl.
 *     the marked-reset rule, in the shared nft table), `route` (the tagged `local <prefix>` route)
 *     and `elems` (the `intake_listeners` set). Process-scoped objects (intake listeners, control
 *     sessions, the owner CID table, the quiescence latch) die with the process. A graceful
 *     shutdown is a subset of `Crash` here (the route is never removed either way, FIX-5).
 *   - Boot steps: 0 started → 1 sweep_stale → 2 start_shared_owner (rules; intake_listeners
 *     converged to empty) → 3 converge_shared (route, FIX-3) → 4 open_after_boot. A crash may
 *     happen between any two steps; converge_shared may refuse (serve exits).
 *   - Activation is a multi-step owner item: `ActivateBegin` (CID → Activating), listener
 *     bring-up for every wanted port, then `ActivateOk` (→ Active) or, after a bring-up failure,
 *     a rollback take-down and `ActivateErr` (→ Provisioned). Per-allocation events (M-7) are
 *     not applied while an item is in progress (`rest`).
 *   - Connection landing is a STATE predicate evaluated for two client classes: marked (leg-S /
 *     marked probes — exist only while serve is open) and other (unmarked host-local, remote). A
 *     host service is assumed bound to 0.0.0.0 on EVERY port (worst case, R5-1).
 *
 * ASSUMPTIONS (kernel / environment):
 *   K-D1  With `local <prefix> dev lo` present and no steering rules, a connection to
 *         workload_addr:p reaches a wildcard host listener on p unless a listener bound to
 *         workload_addr:p exists (more-specific bind wins) ............................ V-19
 *   K-D2  With the rules present: a marked connection whose (addr, port) is not in
 *         intake_listeners is reset; every other non-diverted packet is dropped / rejected; a
 *         leg-C divert whose socket is absent (serve down) falls through to that drop ...... V-19
 *   K-D3  The shared nft table, route and set elements persist across a serve crash; nft batches
 *         apply atomically ................................................... V-19 (persistence)
 *         "Not removed by other software" is NOT assumed: ENV_FOREIGN_FLUSH models a foreign
 *         flush of the table (rules and elements) as an environment action ....... no V-item
 *   K-D4  Without the local route no guest-prefix address is locally delivered ............ V-19
 *   K-D5  Closing a listening socket resets the children still in its accept queue (ADR-0160) V-13
 *
 * BUG_* constants each switch ONE design rule off (teeth; instances in hazard/). ENV_* constants
 * enable an environment fault the design does not control.
 */
module steering {
  const PORTS: Set[int]
  const DECLARED: Set[int]
  const MAXGEN: int
  const ALLOW_CRASH: bool
  const ADMIT_FAILURES: int               // bound on bind / admit_intake_listener failures
  const REVOKE_FAILURES: int              // bound on IntakeAdmission::revoke failures
  const ENV_FOREIGN_FLUSH: bool           // other software flushes the shared nft table (K-D3)
  const BUG_REVOKE_FAIL_CLOSES: bool      // D8a-REVOKE off: revoke failure closes the listener (r1 text)
  const BUG_RELEASE_IGNORES_ELEMENTS: bool // D8a-REVOKE off: pool.release guard does not name elements
  const BUG_CONVERGE_IGNORES_RULES: bool  // D8a-ROUTE off: converge_shared adds/keeps the route regardless
  const BUG_ROUTE_BEFORE_RULES: bool      // D8a boot order off: converge_shared before start_shared_owner
  const BUG_ADMIT_BEFORE_LISTEN: bool     // element added before the listener listens
  const BUG_CLOSE_BEFORE_REVOKE: bool     // listener closed before its element is revoked
  const BUG_QUIESCE_KEEPS_LISTENERS: bool // quiescence leaves intake listeners bound
  const BUG_ACT_ERR_EARLY: bool           // activation returns Err before its rollback completed
  const BUG_ACT_OK_PARTIAL: bool          // activation returns Ok with a wanted port not admitted
  const BUG_REWANT_KEEPS_ELEMENT: bool    // D8a-READMIT off (re-admit): the r2 rule (assume the element is still there)
  const BUG_REPAIR_OUTSIDE_QUIESCE: bool  // D8a-READMIT off (repair in quiescence): rules repaired without quiescing

  // ghosts (never read by the protocol; evaluated by invariants)
  type Ghost = {
    relElem: bool,      // pool.release ran while an intake_listeners element existed
    actPartial: bool,   // activation returned Ok while a wanted port had no admitted listener
    actResidue: bool,   // activation returned Err while a serving listener or a non-resetting element remained
    routeNoRules: bool, // converge_shared added or kept the route while the rules were absent
    flushed: bool,      // a foreign flush happened and the rules have not been re-installed since
    refused: bool,      // converge_shared refused startup (witness)
    revokeRetried: bool,// a revoke failed and the listener stayed bound, resetting (witness)
    actErrDone: bool,   // an activation completed with Err (witness)
  }

  var up: bool
  var boot: int
  var rules: bool
  var route: bool
  var elems: Set[int]
  var alloc: str              // Absent | Provisioned | Activating | Active | Retiring
  var gen: int
  var sess: bool
  var quiesced: bool
  var guestL: int -> bool     // the guest application is listening on p (guest truth)
  var lastLS: int -> bool     // the owner's last ListenState for p
  // none | bound (bound + listening) | reg (registered with the forwarder) | adm (element present)
  // | rst (element removal failed: bound, element present, resets what it accepts; retried)
  // | rev (element revoked, still listening) | unreg (unregistered, still bound)
  // | pre (bug: element added, not yet bound) | gone (bug: closed, element not yet revoked)
  var lst: int -> str
  var admitLeft: int
  var revokeLeft: int
  var actFailed: bool         // the activation in progress hit a bring-up failure (rolling back)
  var g: Ghost

  val open: bool = up and boot == 4
  def listening(p: int): bool = lst.get(p).in(Set("bound", "reg", "adm", "rst", "rev", "unreg"))
  // a listener that can hand a connection to a flow (bound and not resetting)
  def serving(p: int): bool = lst.get(p).in(Set("bound", "reg", "adm"))
  // every element names a resetting listener (what a failed removal may leave behind)
  val elemsOnlyResetting: bool = elems.forall(p => lst.get(p) == "rst")
  val activeLike: bool = alloc == "Active" or (alloc == "Activating" and not(actFailed))
  def want(p: int): bool = and {
    open, activeLike, BUG_QUIESCE_KEEPS_LISTENERS or not(quiesced), sess, lastLS.get(p),
    DECLARED.contains(p),
  }
  val busy: bool = PORTS.exists(p => lst.get(p).in(Set("bound", "reg", "rev", "unreg", "pre", "gone")))
  // A take-down that the last event made necessary belongs to that event's serialized item (M-7).
  val pendingTakeDown: bool = PORTS.exists(p => lst.get(p) == "adm" and not(want(p)))
  // between per-allocation items (activation is one item: M-7 + "serialized with quiescence")
  val rest: bool = not(busy) and not(pendingTakeDown) and alloc != "Activating"
  val allNone: int -> str = PORTS.mapBy(p => "none")
  val allFalse: int -> bool = PORTS.mapBy(p => false)
  val g0: Ghost = { relElem: false, actPartial: false, actResidue: false, routeNoRules: false,
                    flushed: false, refused: false, revokeRetried: false, actErrDone: false }

  // ------------------------------------------------------------------ frame helpers
  action keepKernel = all { rules' = rules, route' = route, elems' = elems }
  action keepProc = all { up' = up, boot' = boot }
  action keepAlloc = all {
    alloc' = alloc, gen' = gen, sess' = sess, quiesced' = quiesced, guestL' = guestL,
    lastLS' = lastLS, actFailed' = actFailed,
  }
  action keepOwner = all { lst' = lst, admitLeft' = admitLeft, revokeLeft' = revokeLeft }

  action init = all {
    up' = true, boot' = 0, rules' = false, route' = false, elems' = Set(),
    alloc' = "Absent", gen' = 0, sess' = false, quiesced' = false,
    guestL' = allFalse, lastLS' = allFalse, lst' = allNone, actFailed' = false,
    admitLeft' = ADMIT_FAILURES, revokeLeft' = REVOKE_FAILURES, g' = g0,
  }

  // ------------------------------------------------------------------ boot, crash, restart
  // Which boot step installs the rules and which converges the route (BUG_ROUTE_BEFORE_RULES swaps).
  val rulesStep: int = if (BUG_ROUTE_BEFORE_RULES) 2 else 1
  val routeStep: int = if (BUG_ROUTE_BEFORE_RULES) 1 else 2
  action BootStep = all {
    up, boot < 4,
    if (boot == rulesStep) all {
      // start_shared_owner: constant rules installed, intake_listeners converged to empty
      boot' = boot + 1, up' = up, rules' = true, elems' = Set(), route' = route,
      g' = { ...g, flushed: false },
    } else if (boot == routeStep) {
      if (rules or BUG_CONVERGE_IGNORES_RULES) all {
        // converge_shared: keep / replace / add the tagged route
        boot' = boot + 1, up' = up, route' = true, rules' = rules, elems' = elems,
        g' = { ...g, routeNoRules: g.routeNoRules or not(rules) },
      } else all {
        // D8a-ROUTE: rules not verified — remove the Overdrive-tagged route; startup refuses
        up' = false, boot' = 0, route' = false, rules' = rules, elems' = elems,
        g' = { ...g, refused: true },
      }
    } else all {
      boot' = boot + 1, up' = up, keepKernel, g' = g,
    },
    keepAlloc, keepOwner,
  }
  // serve dies (or shuts down): process-scoped objects vanish; VMs are reclaimed at the next boot
  // (D19). Node-global kernel objects persist (K-D3); the route is never removed (FIX-5).
  action Crash = all {
    ALLOW_CRASH, up,
    up' = false, boot' = 0, lst' = allNone, sess' = false, alloc' = "Absent", quiesced' = false,
    guestL' = allFalse, lastLS' = allFalse, actFailed' = false,
    keepKernel, gen' = gen, admitLeft' = admitLeft, revokeLeft' = revokeLeft, g' = g,
  }
  // a refused or crashed serve that has released its state can start again
  action Restart = all {
    not(up), PORTS.forall(p => lst.get(p) == "none"), alloc == "Absent",
    up' = true, boot' = 0, keepKernel, keepAlloc, keepOwner, g' = g,
  }
  // K-D3 dropped: other software flushes the shared nft table (rules and set elements)
  action ForeignFlush = all {
    ENV_FOREIGN_FLUSH, rules,
    rules' = false, elems' = Set(), route' = route, g' = { ...g, flushed: true },
    keepProc, keepAlloc, keepOwner,
  }
  // the shared audit finds the IpRules component damaged; recovery (G-V5, D8a-READMIT)
  // quiesces — closing every serving intake listener — then repairs the constant rules (ADR-0124);
  // restore later re-binds listeners and (re-)admits their elements
  action RulesRepair = all {
    open, not(rules), BUG_REPAIR_OUTSIDE_QUIESCE or quiesced, rest,
    rules' = true, route' = route, elems' = elems, g' = { ...g, flushed: false },
    keepProc, keepAlloc, keepOwner,
  }

  // ------------------------------------------------------------------ per-allocation events (M-7 order)
  action ev(alloc2: str, gen2: int, sess2: bool, quiesced2: bool, lastLS2: int -> bool, guestL2: int -> bool): bool = all {
    alloc' = alloc2, gen' = gen2, sess' = sess2, quiesced' = quiesced2, lastLS' = lastLS2,
    guestL' = guestL2, actFailed' = actFailed,
    keepProc, keepKernel, keepOwner, g' = g,
  }
  action Provision = all { open, rest, alloc == "Absent", gen < MAXGEN,
    ev("Provisioned", gen + 1, false, quiesced, allFalse, allFalse) }
  action SessOpen = all { open, rest, alloc.in(Set("Provisioned", "Active")), not(sess),
    ev(alloc, gen, true, quiesced, allFalse, guestL) }   // a session opens with every port not listening
  action SessLoss = all { open, rest, sess,
    ev(alloc, gen, false, quiesced, lastLS, guestL) }
  action RecvListenState(p: int): bool = all { open, rest, sess, lastLS.get(p) != guestL.get(p),
    ev(alloc, gen, sess, quiesced, lastLS.set(p, guestL.get(p)), guestL) }
  action Quiesce = all { open, rest, not(quiesced),
    ev(alloc, gen, sess, true, lastLS, guestL) }
  action Restore = all { open, rest, quiesced,
    ev(alloc, gen, sess, false, lastLS, guestL) }
  action Teardown = all { open, rest, alloc.in(Set("Provisioned", "Active")),
    ev("Retiring", gen, false, quiesced, lastLS, guestL) }
  // pool.release: only after teardown returned Ok — no listener and (D8a-REVOKE) no element
  action Release = all {
    open, rest, alloc == "Retiring", PORTS.forall(p => lst.get(p) == "none"),
    BUG_RELEASE_IGNORES_ELEMENTS or elems == Set(),
    alloc' = "Absent", gen' = gen, sess' = false, quiesced' = quiesced, lastLS' = allFalse,
    guestL' = allFalse, actFailed' = false,
    keepProc, keepKernel, keepOwner, g' = { ...g, relElem: g.relElem or elems != Set() },
  }
  // the guest application listens / stops listening (guest side; not in the host's order)
  action GuestApp(p: int): bool = all { alloc.in(Set("Provisioned", "Activating", "Active")), up,
    alloc' = alloc, gen' = gen, sess' = sess, quiesced' = quiesced, lastLS' = lastLS,
    guestL' = guestL.set(p, not(guestL.get(p))), actFailed' = actFailed,
    keepProc, keepKernel, keepOwner, g' = g }

  // ------------------------------------------------------------------ activation (all or nothing)
  // the only Provisioned → Active edge starts: requires a live session and no quiescence
  action ActivateBegin = all { open, rest, alloc == "Provisioned", sess, not(quiesced),
    alloc' = "Activating", actFailed' = false, gen' = gen, sess' = sess, quiesced' = quiesced,
    lastLS' = lastLS, guestL' = guestL, keepProc, keepKernel, keepOwner, g' = g }
  // Ok: every wanted port has its listener and element
  action ActivateOk = all {
    up, alloc == "Activating", not(busy),
    not(actFailed) or BUG_ACT_OK_PARTIAL,
    BUG_ACT_OK_PARTIAL or PORTS.forall(p => want(p) implies lst.get(p) == "adm"),
    alloc' = "Active", actFailed' = false, gen' = gen, sess' = sess, quiesced' = quiesced,
    lastLS' = lastLS, guestL' = guestL, keepProc, keepKernel, keepOwner,
    g' = { ...g, actPartial: g.actPartial or PORTS.exists(p =>
             DECLARED.contains(p) and lastLS.get(p) and sess and not(quiesced) and lst.get(p) != "adm") },
  }
  // Err: after a bring-up failure the rollback took down what it bound; a listener whose
  // rollback revoke failed stays resetting (D8a-REVOKE) — the CID is Provisioned regardless
  action ActivateErr = all {
    up, alloc == "Activating", actFailed,
    BUG_ACT_ERR_EARLY or (not(busy) and PORTS.forall(p => lst.get(p).in(Set("none", "rst"))) and elemsOnlyResetting),
    alloc' = "Provisioned", actFailed' = false, gen' = gen, sess' = sess, quiesced' = quiesced,
    lastLS' = lastLS, guestL' = guestL, keepProc, keepKernel, keepOwner,
    g' = { ...g, actErrDone: true,
           actResidue: g.actResidue or PORTS.exists(p => not(lst.get(p).in(Set("none", "rst"))))
                       or not(elemsOnlyResetting) },
  }

  // ------------------------------------------------------------------ owner listener steps
  action ls(p: int, s: str, e: Set[int]): bool = all {
    lst' = lst.set(p, s), elems' = e, rules' = rules, route' = route, keepProc,
  }
  // bring-up: bind + listen
  action Bind(p: int): bool = all { up, want(p), lst.get(p) == "none", not(BUG_ADMIT_BEFORE_LISTEN),
    ls(p, "bound", elems), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  action Register(p: int): bool = all { up, lst.get(p) == "bound",
    ls(p, "reg", elems), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  action Admit(p: int): bool = all { up, lst.get(p) == "reg",
    ls(p, "adm", elems.union(Set(p))), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  // bind / admit fails: the listener (no element) is unregistered and closed. During activation
  // this starts the rollback; after activation the bind is retried at the audit cadence.
  action AdmitFail(p: int): bool = all { up, lst.get(p) == "reg", admitLeft > 0,
    ls(p, "none", elems), admitLeft' = admitLeft - 1, revokeLeft' = revokeLeft,
    alloc' = alloc, gen' = gen, sess' = sess, quiesced' = quiesced, guestL' = guestL,
    lastLS' = lastLS, actFailed' = (actFailed or alloc == "Activating"), g' = g }
  // BUG_ADMIT_BEFORE_LISTEN: element first, listener afterwards
  action PreAdmit(p: int): bool = all { up, want(p), lst.get(p) == "none", BUG_ADMIT_BEFORE_LISTEN,
    ls(p, "pre", elems.union(Set(p))), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  action LateBind(p: int): bool = all { up, lst.get(p) == "pre",
    ls(p, "adm", elems), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  // take-down: revoke (awaited) → unregister → close
  action Revoke(p: int): bool = all { up, not(want(p)), lst.get(p).in(Set("adm", "rst")), not(BUG_CLOSE_BEFORE_REVOKE),
    ls(p, "rev", elems.exclude(Set(p))), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  // revoke fails. D8a-REVOKE: the listener stays bound, resetting what it accepts; the removal is
  // retried at the audit cadence and the current item completes (unconfirmed / Err).
  // BUG_REVOKE_FAIL_CLOSES: the r1 design text — "handled like a bind failure: the listener is closed".
  action RevokeFail(p: int): bool = all { up, not(want(p)), lst.get(p).in(Set("adm", "rst")), revokeLeft > 0,
    not(BUG_CLOSE_BEFORE_REVOKE),
    ls(p, if (BUG_REVOKE_FAIL_CLOSES) "none" else "rst", elems),
    admitLeft' = admitLeft, revokeLeft' = revokeLeft - 1, keepAlloc,
    g' = { ...g, revokeRetried: g.revokeRetried or not(BUG_REVOKE_FAIL_CLOSES) } }
  // a resetting listener whose port is wanted again serves normally; D8a-READMIT: its element
  // is re-admitted (a foreign flush may have removed it). BUG_REWANT_KEEPS_ELEMENT: the r2 rule.
  action Rewant(p: int): bool = all { up, want(p), lst.get(p) == "rst",
    ls(p, "adm", if (BUG_REWANT_KEEPS_ELEMENT) elems else elems.union(Set(p))),
    admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  action Unregister(p: int): bool = all { up, lst.get(p) == "rev",
    ls(p, "unreg", elems), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  action Close(p: int): bool = all { up, lst.get(p) == "unreg",
    ls(p, "none", elems), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  // BUG_CLOSE_BEFORE_REVOKE: close first, revoke afterwards
  action EarlyClose(p: int): bool = all { up, not(want(p)), lst.get(p) == "adm", BUG_CLOSE_BEFORE_REVOKE,
    ls(p, "gone", elems), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }
  action LateRevoke(p: int): bool = all { up, lst.get(p) == "gone",
    ls(p, "none", elems.exclude(Set(p))), admitLeft' = admitLeft, revokeLeft' = revokeLeft, keepAlloc, g' = g }

  action OwnerStep = {
    nondet p = PORTS.oneOf()
    any { Bind(p), Register(p), Admit(p), PreAdmit(p), LateBind(p), Revoke(p), Rewant(p), Unregister(p),
          Close(p), EarlyClose(p), LateRevoke(p), ActivateOk, ActivateErr }
  }
  action Env = {
    nondet p = PORTS.oneOf()
    any { AdmitFail(p), RevokeFail(p), GuestApp(p), RecvListenState(p), Provision, SessOpen,
          SessLoss, ActivateBegin, Quiesce, Restore, Teardown, Release, Crash, Restart,
          ForeignFlush, RulesRepair }
  }
  action step = any { BootStep, OwnerStep, Env }

  // ------------------------------------------------------------------ where a connection lands
  // A connection to workload_addr:p reaches the wildcard HOST service (never allowed).
  def landsOnHostServiceOther(p: int): bool = route and not(rules) and not(listening(p))
  def landsOnHostServiceMarked(p: int): bool =
    route and not(listening(p)) and (not(rules) or (open and elems.contains(p)))

  // ------------------------------------------------------------------ invariants
  // (6) After a crash at any boot step, the shared route is never present without the rules.
  val RouteImpliesRules: bool = route implies rules
  // (7) Traffic to a workload address reaches only a bound intake listener or leg-C — never
  //     another host service.
  val NoHostServiceReached: bool = PORTS.forall(p =>
    not(landsOnHostServiceOther(p)) and not(open and landsOnHostServiceMarked(p)))
  // pinned seeded-sim invariant (§ Architecture enforcement): every element names a listening
  // intake listener — while serve is open (stale elements while serve is down are inert, K-D2).
  val ElementNamesListener: bool = open implies elems.forall(p => listening(p))
  // G-V8 / D8a at rest: an element exists only while its port's condition holds, or for a
  // resetting listener whose removal is being retried (D8a-REVOKE).
  val ElementsMirrorState: bool = (open and rest) implies elems.forall(p => want(p) or lst.get(p) == "rst")
  // G-V5 / M-7 at rest: while forwarding is quiesced no SERVING intake listener exists, and an
  // element survives only for a resetting listener (probes are refused either way).
  val NoListenerWhileQuiesced: bool = (open and quiesced and rest) implies
    (PORTS.forall(p => not(serving(p))) and elemsOnlyResetting)
  // FIX-1: a lease is never freed while an intake_listeners element exists.
  val NoReleaseWithElement: bool = not(g.relElem)
  // FIX-5 / G-V3: activation is all or nothing.
  val ActivationAllOrNothing: bool = not(g.actPartial) and not(g.actResidue)
  // at rest a Provisioned CID has no serving intake listener (probes cannot pass early); an element
  // survives only for a resetting listener
  val ProvisionedHasNoIntake: bool = (open and rest and alloc == "Provisioned") implies
    (PORTS.forall(p => not(serving(p))) and elemsOnlyResetting)
  // FIX-3: converge_shared never adds or keeps the route while the rules are absent.
  val ConvergeNeedsRules: bool = not(g.routeNoRules)
  // K-D3 outcome: a host service is reached only inside a foreign-flush window (flush until the
  // rules are re-installed by the audit repair or by the next boot's start_shared_owner).
  val LeakOnlyWhileFlushed: bool = NoHostServiceReached or g.flushed
  // after a repaired flush, every admitted listener still has its element (marked clients reach it)
  val AdmittedHasElement: bool = (open and rules) implies
    PORTS.forall(p => lst.get(p) == "adm" implies elems.contains(p))
  val Safety: bool = and {
    RouteImpliesRules, NoHostServiceReached, ElementNamesListener, ElementsMirrorState,
    NoListenerWhileQuiesced, NoReleaseWithElement, ActivationAllOrNothing, ProvisionedHasNoIntake,
    ConvergeNeedsRules,
  }
  // the safety set that must still hold when a foreign flush is possible
  val SafetyUnderFlush: bool = and {
    LeakOnlyWhileFlushed, ElementNamesListener, NoListenerWhileQuiesced, NoReleaseWithElement,
    ActivationAllOrNothing, ConvergeNeedsRules,
  }

  // ------------------------------------------------------------------ liveness (TLC)
  val quiesceDone: bool = PORTS.forall(p => lst.get(p) == "none") and elems == Set()
  val ownerVars = (lst, elems, alloc, actFailed)
  // quiesce eventually completes: every intake listener closed and every element removed
  temporal QuiesceCompletes =
    and { weakFair(OwnerStep, ownerVars), weakFair(BootStep, (boot, up, rules, route)) }
      .implies(always(quiesced.implies(eventually(not(quiesced) or quiesceDone))))
  // the listener follows the guest: a wanted declared port is eventually admitted (port 1 is the
  // declared port in every instance)
  temporal ListenerFollowsGuest =
    and { weakFair(OwnerStep, ownerVars), weakFair(BootStep, (boot, up, rules, route)) }
      .implies(always(want(1).implies(eventually(lst.get(1) == "adm" or not(want(1))))))
  // a resetting listener is eventually closed or serves again (retried removal; bounded failures)
  temporal ResettingResolves =
    and { weakFair(OwnerStep, ownerVars), weakFair(BootStep, (boot, up, rules, route)) }
      .implies(always((lst.get(1) == "rst").implies(eventually(lst.get(1) != "rst"))))
  // an activation eventually returns (Ok or Err)
  temporal ActivationCompletes =
    and { weakFair(OwnerStep, ownerVars), weakFair(BootStep, (boot, up, rules, route)) }
      .implies(always((alloc == "Activating").implies(eventually(alloc != "Activating"))))
  // K-D3 accepted exposure is bounded: a host service reached after a foreign flush stops being
  // reachable — at the audit's repair (serve up: quiesce, then repair) or at the next boot's
  // start_shared_owner. Assumes serve eventually stays up (finitely many crashes / refusals) and
  // each step of the audit recovery is strongly fair on its own: the quiesce it issues on damage,
  // and the repair once quiesced. (Fairness on the two as ONE action is too weak: a Quiesce /
  // Restore loop satisfies it without ever repairing — r3 first pass.)
  val exposed: bool = not(NoHostServiceReached)
  action QuiesceOnDamage = all { not(rules), Quiesce }
  temporal ExposureEnds =
    and {
      weakFair(OwnerStep, ownerVars), weakFair(BootStep, (boot, up, rules, route)),
      weakFair(Restart, (up, boot)), strongFair(QuiesceOnDamage, (quiesced, rules)),
      strongFair(RulesRepair, (quiesced, rules)), eventually(always(up)),
    }.implies(always(exposed.implies(eventually(not(exposed)))))
  // serve eventually opens (no crash, no flush: instance steer_live)
  temporal BootOpens =
    and { weakFair(BootStep, (boot, up, rules, route)), weakFair(Restart, (up, boot)) }
      .implies(eventually(open))

  // ------------------------------------------------------------------ non-vacuity witnesses
  // Each is EXPECTED to be violated: the violation proves the behaviour is reachable.
  val WitnessNoAdmitted: bool = PORTS.forall(p => lst.get(p) != "adm")
  val WitnessNoCrashAfterRoute: bool = not(not(up) and route)
  val WitnessNoQuiesceWithListener: bool = not(quiesced and PORTS.exists(p => lst.get(p) == "rev"))
  val WitnessNoSecondAllocation: bool = gen < 2 or alloc != "Active"
  val WitnessNoRevokeRetry: bool = not(g.revokeRetried and open and quiesced and rest and PORTS.exists(p => lst.get(p) == "rst"))
  // an activation returned Err while a rollback listener was still resetting
  val WitnessNoErrWithResetting: bool = not(alloc == "Provisioned" and g.actErrDone and PORTS.exists(p => lst.get(p) == "rst"))
  val WitnessNoActivationErr: bool = not(g.actErrDone)
  val WitnessNoBootRefusal: bool = not(g.refused)
}

// ------------------------------------------------------------------ design instances
// The design: crash at any step, one bind/admit failure and one revoke failure possible.
module steer_ok {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true,
    ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false,
    BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false,
    BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false,
    BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false,
    BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false,
    BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "./steering"
}
// The design with no crash: for BootOpens (a crash loop would trivially prevent opening).
module steer_live {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = false,
    ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false,
    BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false,
    BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false,
    BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false,
    BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false,
    BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "./steering"
}
// The design with two declared ports: an activation can fail on one port after binding the other,
// so the rollback (and a failing rollback revoke) is reachable.
module steer_two {
  import steering(PORTS = Set(1, 2), DECLARED = Set(1, 2), MAXGEN = 2, ALLOW_CRASH = true,
    ADMIT_FAILURES = 1, REVOKE_FAILURES = 1, ENV_FOREIGN_FLUSH = false,
    BUG_REVOKE_FAIL_CLOSES = false, BUG_RELEASE_IGNORES_ELEMENTS = false,
    BUG_CONVERGE_IGNORES_RULES = false, BUG_ROUTE_BEFORE_RULES = false,
    BUG_ADMIT_BEFORE_LISTEN = false, BUG_CLOSE_BEFORE_REVOKE = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ACT_ERR_EARLY = false,
    BUG_ACT_OK_PARTIAL = false, BUG_REWANT_KEEPS_ELEMENT = false,
    BUG_REPAIR_OUTSIDE_QUIESCE = false).* from "./steering"
}
