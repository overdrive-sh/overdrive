// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — module D: guest-prefix local delivery decided by the
 * pinned socket-lookup steering (D8a-LOOKUP), the prefix's tagged route and its fence (D8a-ROUTE,
 * D8a-FENCE), intake listeners mirroring guest listen state (D23, M-7), all-or-nothing activation
 * (U-6), teardown / release (U-3), named quiescence holders with per-component recovery (D8a-HOLD),
 * and where a connection to a workload address lands.
 *
 * Design artifact and DISTILL oracle. Models the design text (revision 9, § Revision 9 decisions
 * items 1–8 as of 2026-10-07: D8a-LOOKUP, D8a-ROUTE, D8a-FENCE with item 7's answers to round 4c —
 * one owner and one runtime repair of the guest prefix, the serialized guest-prefix order, every
 * quiesce call and every intake listener close verifies first, UP-5, UP-7, UP-11, UP-12), not an
 * implementation.
 *
 * Source of truth: docs/feature/netns-density-295/feature-delta.md § "vsock Attachment Replacement
 * DESIGN": § Revision 9 decisions (items 3, 7, 8), § Driven port — guest-prefix steering ("Owner"
 * bullet), § Owner and provisioner (converge_shared, repair_guest_prefix, audit_shared,
 * quiesce_forwarding, QuiescenceHolderMismatch, "Intake listener close (D8a-FENCE, every cause)",
 * restore_forwarding, M-7); ADR-0152 point 1, ADR-0169 (D8a-HOLD), ADR-0171 (D8a-LOOKUP),
 * ADR-0163 (D23, U-6, M-7), ADR-0124 (bounded recovery, then fail-stop; D-295-R13 attempt).
 *
 *   D8a-LOOKUP  A pinned sk_lookup program on the host root netns decides local delivery to the
 *          guest prefix (gateway excluded): a host-local (loopback-ingress) TCP connection to
 *          (addr, port) is assigned the open intake listener its map entry names; no entry, UDP, or
 *          non-loopback ingress → dropped (TCP sees a reset). `steer` inserts the entry after the
 *          listener listens (may fail, possibly forever; Ok = present; idempotent). Closing the
 *          listener removes its entry in the same kernel step. Program, link and map are pinned and
 *          outlive serve; after a serve exit every listener is closed, so every lookup drops.
 *   D8a-ROUTE / D8a-FENCE  The prefix's Overdrive-tagged route has exactly two forms: `local` and
 *          the fence `prohibit <prefix>`, replaced atomically (K-L6). A fenced prefix refuses every
 *          connection. The steering and the route are ONE shared component (GuestPrefixSteering)
 *          with one owner and one runtime repair:
 *            - boot converge_shared: converge the steering → verify → probe → only then `local`;
 *              any failure fences and startup refuses;
 *            - repair_guest_prefix(&hold) (requires the Recovery(GuestPrefixSteering) hold): fence
 *              → converge → verify → probe → `local`; a failure leaves the fence, the attempt fails;
 *            - at runtime converge_shared (the repair of every OTHER recovery) never writes the
 *              steering or the route; only boot and repair_guest_prefix lift a fence.
 *          Every fence, every lift and the verify that precedes a lift are serialized in the owner's
 *          guest-prefix order: no Overdrive fence falls between a lift's verify and its route
 *          write (root may still act there: trust boundary).
 *   Item 7 (every close verifies)  EVERY quiesce_forwarding call (first or joining) and EVERY
 *          intake listener close of any cause (teardown, quiesce, session loss, ListenState off,
 *          activation rollback, failed bring-up) first runs verify (read-only) and fences the
 *          prefix if it fails, in the guest-prefix order, before closing. A fence failure there is
 *          reported; the quiesce goes on (design text); a close goes on (modelled the same way —
 *          the design does not say; finding UP-15).
 *   Guest-prefix recovery (ADR-0124, ADR-0169): the audit finds damage (verify fails, or the
 *          route is not `local` while no Recovery(GuestPrefixSteering) hold stands) → quiesce
 *          (verifies and fences first) → repair_guest_prefix → audit → restore. A failed
 *          post-repair audit is a failed attempt (UP-7); the next attempt is again
 *          repair_guest_prefix, whose first step is the fence (UP-6). Budget per episode, then
 *          fail-stop.
 *   Other recoveries (IpRules, FlowListeners): finding → quiesce (verify + fence) → repair of the
 *          component only (never the steering or the route) → clean audit → restore; damaged again
 *          → failed attempt; budget spent → fail-stop (route left as it is: `local` behind verified
 *          pinned steering, or the fence, UP-12).
 *   D8a-HOLD  quiesce_forwarding(holder) adds the holder (the first one quiesces: latch, abort
 *          flows, close every intake listener); restore_forwarding(hold) removes only its holder and
 *          reopens only when no holder remains; each recovery repairs only while it holds.
 *   U-6 / U-3 / constant nft rules / UP-1: as round 4b.
 *
 * Abstraction:
 *   - ONE workload address (one lease reused by successive allocations `gen`). PORTS: guest ports;
 *     DECLARED ⊆ PORTS are the allocation's declared TCP listen ports.
 *   - Node-global kernel state persisting across a serve exit: `rules`, `route` ∈ {absent, local,
 *     fence} (`absent` also stands for a different route: traffic leaves the node), `link` (the
 *     pinned sk_lookup link attached as ours: verify + probe pass), `entries`.
 *     History variables (not kernel state, properties only): `estab`, `verSince` (a lift's verify
 *     returned Ok after the last fence), `tamper` (root detached the link or set `local` after the
 *     last fence or the last lift-verify), `rmv` (root removed the route after Overdrive's last
 *     route write), `rfn` (root fenced the route after Overdrive's last lift), `win` / `winE` /
 *     `winF` (guest-prefix-recovery steps taken while exposed / while traffic leaves the node /
 *     while root-fenced; they measure the windows).
 *   - repair_guest_prefix's steps (fence, converge, verify + probe, route) are separate steps; a
 *     crash or root action may fall between any two. The serialized order is a lock held from the
 *     lift's verify to its route write: an Overdrive step that would fence waits for it.
 *   - A close's verify, fence and close are one step (ENV_DETACH_IN_CLOSE_GAP lets root detach the
 *     link between the passing verify and the close).
 *   - Per-allocation serialization (M-7): each event is one atomic step that also closes every
 *     listener whose port stops serving. Activation is one item; the sequencer orders it with every
 *     quiesce_forwarding call (the call's verify + fence included: finding UP-13).
 *   - Landing: three client classes (`marked` = leg-S / probes, present whenever serve is up;
 *     `unmarked` host-local; `remote`) × every port; a wildcard host service on EVERY port (worst
 *     case). Outcomes: intake | legC | refused | wildcard (exposure) | elsewhere (leaves the node).
 *
 * ASSUMPTIONS (kernel / environment):
 *   K-L1 … K-L6, K-D1 … K-D5 as round 4b (discharged by V-26, V-19, V-23, V-13).
 *   K-L7  `verify` (read-only link query) observes a root detach that happened before it ........ V-26
 *   Root tampering (ENV_ROOT_DETACH, ENV_ROOT_ENTRY_DELETE, ENV_ROOT_ROUTE, ENV_DETACH_IN_CLOSE_GAP)
 *   is the host's trust boundary: modelled as faults, the resulting windows are measured, not
 *   assumed away. A fence (route replace) can fail (FENCE_FAILURES: netlink error), a fault.
 *
 * BUG_* constants each switch ONE design rule off (teeth; instances in hazard/). ENV_* constants
 * enable an environment fault.
 */
module steering {
  const PORTS: Set[int]
  const DECLARED: Set[int]
  const MAXGEN: int
  const ALLOW_CRASH: bool
  const BIND_FAILURES: int            // bound on bind / listen failures
  const STEER_FAILURES: int           // bound on steer failures (when not forever)
  const STEER_FAIL_FOREVER: bool      // steer may fail forever
  const CONVERGE_FAILURES: int        // bound on boot converge / verify+probe failures
  const REVOKE_FAILURES: int          // BUG_ELEMENT_SET only: bound on element-removal failures
  const REPAIR_FAILURES: int          // bound on runtime repair failures (any recovery)
  const REPAIR_FAIL_FOREVER: bool     // a runtime repair may fail forever
  const MAXATT: int                   // ADR-0124 attempt budget of each recovery episode
  const FENCE_FAILURES: int           // bound on fence (route replace) failures
  const ENV_FOREIGN_FLUSH: bool       // other software deletes the shared nft table (K-D3)
  const ENV_ROOT_ENTRY_DELETE: bool   // root deletes a steering entry
  const ENV_ROOT_DETACH: bool         // root detaches the pinned link
  const ENV_ROOT_ROUTE: bool          // root removes / replaces / fences / sets `local` on the tagged route
  const ENV_LISTENER_EXIT: bool       // a flow-listener task exits (FlowListeners damage)
  const ENV_DETACH_IN_CLOSE_GAP: bool // root detaches the link between a close's passing verify and the close
  const BUG_ELEMENT_SET: bool         // D8a-LOOKUP off: revision-8 shape (firewall element, fallible removal)
  const BUG_ELEMENT_CLOSE_FIRST: bool // ... with the listener closed before its element is removed
  const BUG_STEERING_IN_NFT: bool     // D8a-LOOKUP off: steering lives in the flushable nft table
  const BUG_UNPINNED: bool            // D8a-LOOKUP off: steering owned by the serve process (ADR-0159)
  const BUG_NONLOOPBACK_ASSIGN: bool  // D8a-LOOKUP off: the program also assigns non-loopback ingress
  const BUG_ENTRY_OUTLIVES_CLOSE: bool// K-L3 off: an entry survives its listener's close
  const BUG_ROUTE_BEFORE_STEERING: bool // D8a-ROUTE off: route step before steering converge / verify
  const BUG_NO_VERIFY: bool           // D8a-ROUTE off: converge / verify failure ignored at boot
  const BUG_ROLLBACK_LEAVES_LISTENER: bool // U-6 off: rollback closes only the failing port
  const BUG_ACT_OK_PARTIAL: bool      // U-6 off: Ok without every wanted port steered
  const BUG_TEARDOWN_KEEPS_LISTENERS: bool // U-3 off: teardown returns Ok with listeners open
  const BUG_QUIESCE_KEEPS_LISTENERS: bool  // M-7 / D8a-HOLD off: quiescence leaves listeners open
  const BUG_ANY_RESTORE: bool         // D8a-HOLD off: one latch, any recovery's restore reopens
  const BUG_REPAIR_WITHOUT_HOLD: bool // D8a-HOLD off: a recovery repairs without holding
  const BUG_REMOVE_NOT_FENCE: bool    // D8a-FENCE off: boot refusal / the fence REMOVE the route
  const BUG_NO_FRESH_VERIFY: bool     // D8a-FENCE off: repair_guest_prefix lifts without verify + probe
  const BUG_RESTORE_LIFTS_FENCE: bool // D8a-FENCE off: restore_forwarding (reopen) lifts the fence
  const BUG_QUIESCE_NO_FENCE: bool    // item 7 off: no quiesce call verifies / fences before closing
  const BUG_JOIN_NO_VERIFY: bool      // item 7 off: only the FIRST quiesce call verifies (a joining call does not)
  const BUG_CLOSE_NO_VERIFY: bool     // item 7 off: a non-quiesce intake listener close does not verify
  const BUG_OTHER_REPAIRS_PREFIX: bool // one owner off: every recovery's repair converges the steering and
                                      // lifts the route (round 4c's total converge_shared; a failure fences)
  const BUG_UNSERIALIZED_LIFT: bool   // serialized order off: a fence may fall between a lift's verify and its route write
  const BUG_AUDIT_IGNORES_ROUTE: bool // UP-5 off: the audit does not observe the tagged route's form
  const BUG_REDO_NO_FENCE: bool       // UP-6 off: a re-attempt after a failed audit does not begin with the fence
  const BUG_REDO_NO_COUNT: bool       // UP-7 off: a failed post-repair audit is not a failed attempt

  type Ghost = {
    routeUnverified: bool, // `local` was set while this caller's verify had not returned Ok
    refused: bool,         // startup refused (witness)
    actPartial: bool,      // activation returned Ok with a wanted port not steered
    actResidue: bool,      // activation returned Err with a listener of the CID left
    actErrDone: bool,      // an activation returned Err (witness)
    relListener: bool,     // pool.release ran while a listener of the allocation was open
    reopenEarly: bool,     // forwarding reopened while another recovery had not repaired
    repairNoHold: bool,    // a repair ran without its recovery's hold
    resteered: bool,       // a serving listener's missing entry was re-inserted (witness)
    stillQuiesced: bool,   // a restore returned StillQuiesced (witness)
    overlapped: bool,      // two holders held at once (witness)
    revokeKept: bool,      // BUG_ELEMENT_SET: a failed removal kept a non-serving listener bound
    quiesceWidened: bool,  // a first-holder quiesce closed open intake listeners while, after its
                           // verify + fence, the prefix was `local` with the link detached
    gpsWidened: bool,      // ... and that quiesce was the guest-prefix recovery's own
    quiesceFenced: bool,   // a non-guest-prefix holder's quiesce found the steering unverified and fenced (witness)
    joinFenced: bool,      // a joining quiesce call (not the first holder) fenced (witness)
    closeFenced: bool,     // a per-allocation listener close fenced (witness)
    widenClose: bool,      // a per-allocation close (ListenState / session loss / teardown / rollback /
                           // bring-up failure) closed an open listener while, after its verify + fence,
                           // the prefix was `local` with the link detached
    widenRoute: bool,      // a route step lifted a fence / absent route to `local` while the link was
                           // detached (root detach between its verify and its route step)
    otherLift: bool,       // a lift (fence / absent → `local`) by anything but boot or repair_guest_prefix
  }

  val GPS: str = "GuestPrefixSteering"
  val COMPONENTS: Set[str] = Set("IpRules", "GuestPrefixSteering", "FlowListeners")
  val OTHERS: Set[str] = Set("IpRules", "FlowListeners")
  val CLIENTS: Set[str] = Set("marked", "unmarked", "remote")
  val OPEN: int = 5
  // recovery states: GPS idle → found → held → fenced → conv → ver → repaired; others idle → held →
  // repaired (BUG_OTHER_REPAIRS_PREFIX: held → conv → ver → repaired)
  val HOLDING: Set[str] = Set("held", "fenced", "conv", "ver", "repaired")
  val UNREPAIRED: Set[str] = Set("held", "fenced", "conv", "ver")

  var up: bool
  var boot: int
  var stop: str                // why serve is down: none | crash | refused | failstop | failstopGps
  var rules: bool
  var route: str               // absent | local | fence
  var link: bool
  var verified: Set[str]       // callers whose current lift-verify + probe returned Ok ("boot" or a component)
  var estab: bool              // history: Overdrive has set the tagged route at least once
  var verSince: bool           // history: a lift's verify returned Ok after the last fence
  var tamper: bool             // history: root detached the link / set `local` after the last fence or lift-verify
  var rmv: bool                // history: root removed the route after Overdrive's last route write
  var rfn: bool                // history: root fenced the route after Overdrive's last lift
  var entries: Set[int]
  var lis: Set[int]            // ports with an open (bound, listening) intake listener
  var alloc: str               // Absent | Provisioned | Activating | Active | Retiring
  var gen: int
  var sess: bool
  var lastLS: Set[int]         // ports whose last ListenState says listening
  var holders: Set[str]
  var latch: bool              // forwarding quiesced
  var rec: str -> str          // per component recovery state
  var flowL: bool              // FlowListeners healthy
  var bindLeft: int
  var steerLeft: int
  var convLeft: int
  var revokeLeft: int
  var repLeft: int
  var fenceLeft: int
  var att: str -> int          // failed attempts of each component's current recovery episode (process-scoped)
  var win: int                 // guest-prefix-recovery steps taken while exposed (detach window)
  var winE: int                // guest-prefix-recovery steps taken while prefix traffic leaves the node
  var winF: int                // guest-prefix-recovery steps taken while the prefix is root-fenced
  var g: Ghost

  val g0: Ghost = { routeUnverified: false, refused: false, actPartial: false, actResidue: false,
                    actErrDone: false, relListener: false, reopenEarly: false, repairNoHold: false,
                    resteered: false, stillQuiesced: false, overlapped: false, revokeKept: false,
                    quiesceWidened: false, gpsWidened: false, quiesceFenced: false, joinFenced: false,
                    closeFenced: false, widenClose: false, widenRoute: false, otherLift: false }
  val open: bool = up and boot == OPEN
  val legC: bool = up and boot >= 2
  val recIdle: str -> str = COMPONENTS.mapBy(c => "idle")
  val att0: str -> int = COMPONENTS.mapBy(c => 0)
  // audit_shared (UP-5 restated): the guest prefix is damaged when verify fails, or when the tagged
  // route is not `local` while no Recovery(GuestPrefixSteering) hold stands (absent, different, fenced)
  def damaged(c: str): bool =
    if (c == "IpRules") not(rules)
    else if (c == GPS) or {
      not(link),
      and { not(BUG_AUDIT_IGNORES_ROUTE), route != "local", not(holders.contains(GPS)) },
    }
    else not(flowL)
  // the serialized guest-prefix order: a lift is in flight between its verify and its route write;
  // an Overdrive fence waits for it (BUG_UNSERIALIZED_LIFT: it does not)
  def liftInFlightBy(c: str): bool = COMPONENTS.exists(d => d != c and rec.get(d) == "ver")
  val lockFree: bool = BUG_UNSERIALIZED_LIFT or not(COMPONENTS.exists(d => rec.get(d) == "ver"))
  def servingWith(alloc2: str, holders2: Set[str], sess2: bool, ls2: Set[int], p: int): bool = and {
    open, holders2 == Set(), sess2, ls2.contains(p), DECLARED.contains(p),
    alloc2.in(Set("Active", "Activating")),
  }
  def serving(p: int): bool = servingWith(alloc, if (BUG_QUIESCE_KEEPS_LISTENERS) Set() else holders, sess, lastLS, p)
  // the design's serving set (independent of the quiescence hazard), for the properties
  def servingTrue(p: int): bool = servingWith(alloc, holders, sess, lastLS, p)

  // ------------------------------------------------------------------ where a connection lands
  // The socket lookup for a connection that reached local delivery (`local` route, not diverted).
  def lookupLands(p: int, loopback: bool): str =
    if (BUG_ELEMENT_SET) {
      // revision 8: the marked-reset rule in the nft table decides; delivery is the normal lookup
      if (rules and loopback) {
        if (entries.contains(p)) (if (lis.contains(p)) "intake" else "wildcard") else "refused"
      } else if (lis.contains(p)) "intake" else "wildcard"
    } else if (link) {
      // D8a-LOOKUP: assign the listener the entry names, else drop (reset)
      if ((loopback or BUG_NONLOOPBACK_ASSIGN) and entries.contains(p) and lis.contains(p)) "intake"
      else "refused"
    } else if (lis.contains(p)) "intake" else "wildcard"   // no lookup decision: K-D1
  def present(c: str): bool = c != "marked" or up
  def lands(c: str, p: int): str =
    if (not(present(c))) "refused"
    else if (route == "fence") "refused"                                  // K-L6: the fence refuses all
    else if (c != "marked" and rules) (if (legC) "legC" else "refused")   // divert or reject / drop
    else if (route == "absent") "elsewhere"                               // K-D4: routed off the node
    else lookupLands(p, c != "remote")

  val exposed: bool = CLIENTS.exists(c => PORTS.exists(p => lands(c, p) == "wildcard"))
  val elsewhere: bool = CLIENTS.exists(c => PORTS.exists(p => lands(c, p) == "elsewhere"))
  val leaves: bool = estab and elsewhere
  val rootFenced: bool = rfn and route == "fence"
  val stopped: bool = not(up) and stop.in(Set("refused", "failstop", "failstopGps"))
  def capInc(w: int, cond: bool, stepped: bool, cap: int): int =
    if (cond) (if (stepped and w < cap) w + 1 else w) else 0
  // nondeterministic choices of a close: the fence fails (bounded) / root detaches in the close gap
  val fkSet: Set[bool] = if (fenceLeft > 0) Set(false, true) else Set(false)
  val gapSet: Set[bool] = if (ENV_DETACH_IN_CLOSE_GAP and link) Set(false, true) else Set(false)
  def fenceAfter(fk: bool): int = if (fk) fenceLeft - 1 else fenceLeft

  // ------------------------------------------------------------------ frame helpers
  action keepBoot = all { up' = up, boot' = boot, stop' = stop }
  action keepRoute = all {
    route' = route, estab' = estab, verSince' = verSince, tamper' = tamper, rmv' = rmv, rfn' = rfn,
  }
  action keepKernel = all { rules' = rules, link' = link, verified' = verified, keepRoute }
  action keepListen = all { lis' = lis, entries' = entries }
  action keepAlloc = all { alloc' = alloc, gen' = gen, sess' = sess, lastLS' = lastLS }
  action keepHold = all { holders' = holders, latch' = latch, rec' = rec, flowL' = flowL }
  action keepRec = all { repLeft' = repLeft, att' = att, fenceLeft' = fenceLeft }
  action keepLeft = all {
    bindLeft' = bindLeft, steerLeft' = steerLeft, convLeft' = convLeft, revokeLeft' = revokeLeft,
  }
  // window counters: `gps` = this is a step of the guest-prefix recovery
  action tick(gps: bool): bool = all {
    win' = capInc(win, exposed, gps, 3), winE' = capInc(winE, leaves, gps, 4),
    winF' = capInc(winF, rootFenced, gps, 7),
  }
  action keepCount = all { keepLeft, keepRec, tick(false) }
  // counters of a non-recovery step that may have consumed a fence failure
  action countF(fk: bool): bool = all {
    keepLeft, repLeft' = repLeft, att' = att, fenceLeft' = fenceAfter(fk), tick(false),
  }
  // counters of a step of component c's recovery
  action recCount(c: str, att2: int, rep2: int, fence2: int): bool = all {
    keepLeft, att' = att.set(c, att2), repLeft' = rep2, fenceLeft' = fence2, tick(c == GPS),
  }
  // the fence (D8a-FENCE): the tagged route becomes `prohibit <prefix>` in one replace
  // (BUG_REMOVE_NOT_FENCE: the route is removed instead). Callers set rules', link', verified'.
  action fenceRoute = all {
    route' = if (BUG_REMOVE_NOT_FENCE) "absent" else "fence",
    estab' = true, verSince' = false, tamper' = false, rmv' = false, rfn' = rfn,
  }
  // a lift (boot converge_shared or repair_guest_prefix): `local` replaces the fence / a missing route
  action localRoute = all {
    route' = "local", estab' = true, verSince' = verSince, tamper' = tamper, rmv' = false, rfn' = false,
  }
  // the kernel effect of a verify-then-close: the fence if the verify failed and the fence did not
  // fail; `gap`: root detached the link between the passing verify and the close
  action closeKernel(fences: bool, gap: bool): bool = all {
    link' = if (gap) false else link,
    if (fences) fenceRoute
    else all { route' = route, estab' = estab, verSince' = verSince, tamper' = (tamper or gap), rmv' = rmv, rfn' = rfn },
  }

  action init = all {
    up' = true, boot' = 0, stop' = "none", rules' = false, route' = "absent", link' = false,
    verified' = Set(), estab' = false, verSince' = false, tamper' = false, rmv' = false, rfn' = false,
    entries' = Set(), lis' = Set(), alloc' = "Absent", gen' = 0, sess' = false, lastLS' = Set(),
    holders' = Set(), latch' = false, rec' = recIdle, flowL' = true,
    bindLeft' = BIND_FAILURES, steerLeft' = STEER_FAILURES, convLeft' = CONVERGE_FAILURES,
    revokeLeft' = REVOKE_FAILURES, repLeft' = REPAIR_FAILURES, fenceLeft' = FENCE_FAILURES,
    att' = att0, win' = 0, winE' = 0, winF' = 0, g' = g0,
  }

  // ------------------------------------------------------------------ boot, crash, restart
  val convStep: int = if (BUG_ROUTE_BEFORE_STEERING) 3 else 2
  val verifyStep: int = if (BUG_ROUTE_BEFORE_STEERING) 4 else 3
  val routeStep: int = if (BUG_ROUTE_BEFORE_STEERING) 2 else 4
  // converge_shared could not verify the steering at boot: fence the prefix, startup refuses, serve exits
  action refuse = all {
    up' = false, boot' = 0, stop' = "refused", fenceRoute, rules' = rules, link' = link, verified' = Set(),
    g' = { ...g, refused: true },
  }
  action BootStep = all {
    up, boot < OPEN,
    if (boot == 1) all {
      // start_shared_owner: constant rules + leg-C; (BUG_ELEMENT_SET: intake_listeners converged empty)
      boot' = boot + 1, up' = up, stop' = stop, rules' = true, link' = link, verified' = verified,
      keepRoute, g' = g, entries' = if (BUG_ELEMENT_SET) Set() else entries, lis' = lis,
    } else if (boot == convStep) all {
      // GuestPrefixSteering::converge: adopt / update / attach and pin; never detaches
      boot' = boot + 1, up' = up, stop' = stop, link' = true, verified' = Set(), rules' = rules,
      keepRoute, g' = g, keepListen,
    } else if (boot == verifyStep) {
      // verify + probe: Ok iff the link is attached as ours (BUG_NO_VERIFY: the result is not acted on)
      if (link or BUG_NO_VERIFY) all {
        boot' = boot + 1, up' = up, stop' = stop, rules' = rules, link' = link,
        verified' = if (link) Set("boot") else Set(), route' = route, estab' = estab,
        verSince' = (verSince or link), tamper' = (tamper and not(link)), rmv' = rmv, rfn' = rfn,
        g' = g, keepListen,
      } else all { refuse, keepListen }
    } else if (boot == routeStep) all {
      // keep / replace (the fence included) / add the tagged `local` route
      boot' = boot + 1, up' = up, stop' = stop, localRoute, rules' = rules, link' = link,
      verified' = verified, keepListen,
      g' = { ...g, routeUnverified: g.routeUnverified or not(verified.contains("boot")),
             widenRoute: g.widenRoute or (route != "local" and not(link)) },
    } else all {
      boot' = boot + 1, up' = up, stop' = stop, keepKernel, g' = g, keepListen,
    },
    keepAlloc, keepHold, keepCount,
  }
  // converge fails (e.g. PinnedObjectForeign) or verify / probe fails with the link attached:
  // fence + refuse (BUG_NO_VERIFY: continue)
  action ConvergeFail = all {
    up, or { boot == convStep, boot == verifyStep }, convLeft > 0,
    if (BUG_NO_VERIFY) all { boot' = boot + 1, up' = up, stop' = stop, keepKernel, g' = g }
    else refuse,
    convLeft' = convLeft - 1, bindLeft' = bindLeft, steerLeft' = steerLeft, revokeLeft' = revokeLeft,
    keepRec, tick(false), keepListen, keepAlloc, keepHold,
  }
  // serve dies (or shuts down). Process-scoped objects vanish: every listener closes, so (K-L4) the
  // map holds no live socket. Node-global objects persist; the route is never removed (U-4).
  action Crash = all {
    ALLOW_CRASH, up,
    up' = false, boot' = 0, stop' = "crash", lis' = Set(), verified' = Set(),
    entries' = if (BUG_ELEMENT_SET or BUG_ENTRY_OUTLIVES_CLOSE) entries else Set(),
    link' = if (BUG_UNPINNED) false else link, rules' = rules, keepRoute,
    alloc' = "Absent", gen' = gen, sess' = false, lastLS' = Set(),
    holders' = Set(), latch' = false, rec' = recIdle, flowL' = true,
    keepLeft, repLeft' = repLeft, fenceLeft' = fenceLeft, att' = att0, tick(false), g' = g,
  }
  action Restart = all {
    not(up),
    up' = true, boot' = 0, stop' = "none", keepKernel, keepListen, keepAlloc, keepHold, keepCount, g' = g,
  }

  // ------------------------------------------------------------------ environment faults
  // other software deletes the shared nft table: rules and divert gone; steering untouched
  action ForeignFlush = all {
    ENV_FOREIGN_FLUSH, rules,
    rules' = false, link' = if (BUG_STEERING_IN_NFT) false else link, verified' = verified, keepRoute,
    entries' = if (BUG_ELEMENT_SET or BUG_STEERING_IN_NFT) Set() else entries, lis' = lis,
    keepBoot, keepAlloc, keepHold, keepCount, g' = g,
  }
  // root detaches the pinned link (trust boundary)
  action RootDetach = all {
    ENV_ROOT_DETACH, link,
    link' = false, tamper' = true, rules' = rules, route' = route, verified' = verified,
    estab' = estab, verSince' = verSince, rmv' = rmv, rfn' = rfn,
    keepListen, keepBoot, keepAlloc, keepHold, keepCount, g' = g,
  }
  // root removes (or replaces by a different route: same landing), sets `local`, or fences the tagged
  // route (trust boundary)
  action RootRoute(rv: str): bool = all {
    ENV_ROOT_ROUTE, route != rv,
    route' = rv, tamper' = (tamper or rv == "local"), rmv' = (rv == "absent"), rfn' = (rv == "fence"),
    rules' = rules, link' = link, verified' = verified, estab' = estab, verSince' = verSince,
    keepListen, keepBoot, keepAlloc, keepHold, keepCount, g' = g,
  }
  // root deletes a steering entry (trust boundary)
  action RootEntryDelete(p: int): bool = all {
    ENV_ROOT_ENTRY_DELETE, entries.contains(p),
    entries' = entries.exclude(Set(p)), lis' = lis, keepKernel, keepBoot, keepAlloc, keepHold, keepCount, g' = g,
  }
  action ListenerTaskExit = all {
    ENV_LISTENER_EXIT, up, flowL,
    flowL' = false, holders' = holders, latch' = latch, rec' = rec,
    keepKernel, keepListen, keepBoot, keepAlloc, keepCount, g' = g,
  }

  // ------------------------------------------------------------------ per-allocation events (M-7)
  // The event and the closing of every listener whose port stops serving are one step; closing
  // removes the steering entry in the same kernel step (K-L3). Item 7: every close first runs verify
  // and, if it fails, fences (in the guest-prefix order: it waits while a lift is in flight) before
  // closing. BUG_ELEMENT_SET: listeners are taken down afterwards by TakeDown.
  action applyEvent(alloc2: str, gen2: int, sess2: bool, ls2: Set[int], holders2: Set[str], keepAll: bool,
                    fk: bool, gap: bool): bool = all {
    val keep = lis.filter(p => keepAll or servingWith(alloc2, holders2, sess2, ls2, p))
    val closes = and { not(BUG_ELEMENT_SET), lis.exists(p => not(keep.contains(p))) }
    val verifies = closes and not(BUG_CLOSE_NO_VERIFY)
    val wouldFence = verifies and not(link)
    val fences = wouldFence and not(fk)
    val routeAfter = if (fences) (if (BUG_REMOVE_NOT_FENCE) "absent" else "fence") else route
    val closesExposing = and { closes, up, routeAfter == "local", not(link) or gap }
    all {
      fk implies wouldFence, gap implies (verifies and link), wouldFence implies lockFree,
      lis' = if (BUG_ELEMENT_SET) lis else keep,
      entries' = if (BUG_ELEMENT_SET or BUG_ENTRY_OUTLIVES_CLOSE) entries else entries.filter(p => keep.contains(p)),
      closeKernel(fences, gap),
      g' = { ...g, widenClose: g.widenClose or closesExposing, closeFenced: g.closeFenced or fences },
    },
    alloc' = alloc2, gen' = gen2, sess' = sess2, lastLS' = ls2,
  }
  val evGuard: bool = open and alloc != "Activating"
  action Provision = all { evGuard, alloc == "Absent", gen < MAXGEN,
    applyEvent("Provisioned", gen + 1, false, Set(), holders, false, false, false),
    keepBoot, rules' = rules, verified' = verified, keepHold, keepCount }
  // a session opens with every port not listening; the guest re-sends full state
  action SessOpen = all { evGuard, alloc.in(Set("Provisioned", "Active")), not(sess),
    applyEvent(alloc, gen, true, Set(), holders, false, false, false),
    keepBoot, rules' = rules, verified' = verified, keepHold, keepCount }
  // session loss closes every intake listener of the CID
  action SessLoss = {
    nondet fk = fkSet.oneOf()
    nondet gap = gapSet.oneOf()
    all { evGuard, sess,
      applyEvent(alloc, gen, false, lastLS, holders, false, fk, gap),
      keepBoot, rules' = rules, verified' = verified, keepHold, countF(fk) }
  }
  action ListenState(p: int, v: bool): bool = {
    nondet fk = fkSet.oneOf()
    nondet gap = gapSet.oneOf()
    all { evGuard, sess, alloc.in(Set("Provisioned", "Active")), lastLS.contains(p) != v,
      applyEvent(alloc, gen, sess, if (v) lastLS.union(Set(p)) else lastLS.exclude(Set(p)), holders, false, fk, gap),
      keepBoot, rules' = rules, verified' = verified, keepHold, countF(fk) }
  }
  // teardown: Retiring; listeners (and their entries) and the session closed; Ok
  action Teardown = {
    nondet fk = fkSet.oneOf()
    nondet gap = gapSet.oneOf()
    all { evGuard, alloc.in(Set("Provisioned", "Active")),
      applyEvent("Retiring", gen, false, lastLS, holders, BUG_TEARDOWN_KEEPS_LISTENERS, fk, gap),
      keepBoot, rules' = rules, verified' = verified, keepHold, countF(fk) }
  }
  // pool.release: only after teardown returned Ok
  action Release = all { evGuard, alloc == "Retiring",
    alloc' = "Absent", gen' = gen, sess' = false, lastLS' = Set(), keepListen,
    keepBoot, keepKernel, keepHold, keepCount,
    g' = { ...g, relListener: g.relListener or lis != Set() } }

  // ------------------------------------------------------------------ activation (all or nothing)
  action ActivateBegin = all { evGuard, alloc == "Provisioned", sess, holders == Set(), not(latch),
    alloc' = "Activating", gen' = gen, sess' = sess, lastLS' = lastLS,
    keepListen, keepBoot, keepKernel, keepHold, keepCount, g' = g }
  action ActivateOk = all {
    up, alloc == "Activating",
    BUG_ACT_OK_PARTIAL or PORTS.forall(p => serving(p) implies (lis.contains(p) and entries.contains(p))),
    alloc' = "Active", gen' = gen, sess' = sess, lastLS' = lastLS,
    keepListen, keepBoot, keepKernel, keepHold, keepCount,
    g' = { ...g, actPartial: g.actPartial or PORTS.exists(p => serving(p) and not(entries.contains(p))) },
  }
  // a bring-up failure during activation: rollback closes every listener of the CID (cannot fail)
  // and returns Err with the CID Provisioned. Outside activation the port is just refused and the
  // bring-up retried at the audit cadence. Every close verifies first (item 7).
  action bringUpFails(p: int, fk: bool, gap: bool): bool = all {
    val left = if (alloc == "Activating") (if (BUG_ROLLBACK_LEAVES_LISTENER) lis.exclude(Set(p)) else Set())
               else lis.exclude(Set(p))
    val closes = lis.exists(q => not(left.contains(q)))
    val verifies = closes and not(BUG_CLOSE_NO_VERIFY)
    val wouldFence = verifies and not(link)
    val fences = wouldFence and not(fk)
    val routeAfter = if (fences) (if (BUG_REMOVE_NOT_FENCE) "absent" else "fence") else route
    val closesExposing = and { closes, up, routeAfter == "local", not(link) or gap }
    all {
      fk implies wouldFence, gap implies (verifies and link), wouldFence implies lockFree,
      lis' = left,
      entries' = if (BUG_ENTRY_OUTLIVES_CLOSE) entries else entries.filter(q => left.contains(q)),
      alloc' = if (alloc == "Activating") "Provisioned" else alloc,
      closeKernel(fences, gap),
      g' = if (alloc == "Activating")
             { ...g, actErrDone: true, actResidue: g.actResidue or left != Set(),
               widenClose: g.widenClose or closesExposing, closeFenced: g.closeFenced or fences }
           else { ...g, widenClose: g.widenClose or closesExposing, closeFenced: g.closeFenced or fences },
    },
    gen' = gen, sess' = sess, lastLS' = lastLS, keepBoot, rules' = rules, verified' = verified, keepHold,
  }

  // ------------------------------------------------------------------ owner listener steps
  // bring-up: bind + listen
  action BindListen(p: int): bool = all { up, serving(p), not(lis.contains(p)),
    lis' = lis.union(Set(p)), entries' = entries,
    keepAlloc, keepBoot, keepKernel, keepHold, keepCount, g' = g }
  action BindFail(p: int): bool = {
    nondet fk = fkSet.oneOf()
    nondet gap = gapSet.oneOf()
    all { up, serving(p), not(lis.contains(p)), bindLeft > 0,
      bringUpFails(p, fk, gap), bindLeft' = bindLeft - 1, steerLeft' = steerLeft, convLeft' = convLeft,
      revokeLeft' = revokeLeft, repLeft' = repLeft, att' = att, fenceLeft' = fenceAfter(fk), tick(false),
    }
  }
  // GuestPrefixSteering::steer after listen (also the audit's re-steer of a missing entry).
  // BUG_ELEMENT_SET: admit the firewall element (needs the nft table).
  action Steer(p: int): bool = all { up, serving(p), lis.contains(p), not(entries.contains(p)),
    not(BUG_ELEMENT_SET) or rules,
    entries' = entries.union(Set(p)), lis' = lis,
    keepAlloc, keepBoot, keepKernel, keepHold, keepCount,
    g' = { ...g, resteered: g.resteered or alloc == "Active" } }
  // steer fails: the listener is closed, the port refused meanwhile, bring-up retried later
  action SteerFail(p: int): bool = {
    nondet fk = fkSet.oneOf()
    nondet gap = gapSet.oneOf()
    all { up, serving(p), lis.contains(p), not(entries.contains(p)),
      STEER_FAIL_FOREVER or steerLeft > 0,
      bringUpFails(p, fk, gap), steerLeft' = if (STEER_FAIL_FOREVER) steerLeft else steerLeft - 1,
      bindLeft' = bindLeft, convLeft' = convLeft, revokeLeft' = revokeLeft, repLeft' = repLeft, att' = att,
      fenceLeft' = fenceAfter(fk), tick(false),
    }
  }
  // BUG_ELEMENT_SET take-down (revision 8): remove the element, then close; a failed removal keeps
  // the listener bound. BUG_ELEMENT_CLOSE_FIRST: close first, remove the element afterwards.
  action TakeDown(p: int): bool = all { BUG_ELEMENT_SET, up, lis.contains(p), not(serving(p)),
    if (BUG_ELEMENT_CLOSE_FIRST) all { lis' = lis.exclude(Set(p)), entries' = entries }
    else all { lis' = lis.exclude(Set(p)), entries' = entries.exclude(Set(p)) },
    keepAlloc, keepBoot, keepKernel, keepHold, keepCount, g' = g }
  action RevokeFail(p: int): bool = all { BUG_ELEMENT_SET, not(BUG_ELEMENT_CLOSE_FIRST), up, lis.contains(p),
    not(serving(p)), entries.contains(p), revokeLeft > 0,
    keepListen, keepAlloc, keepBoot, keepKernel, keepHold,
    bindLeft' = bindLeft, steerLeft' = steerLeft, convLeft' = convLeft, revokeLeft' = revokeLeft - 1,
    keepRec, tick(false), g' = { ...g, revokeKept: true } }
  action LateRevoke(p: int): bool = all { BUG_ELEMENT_SET, up, not(lis.contains(p)), entries.contains(p),
    not(serving(p)),
    entries' = entries.exclude(Set(p)), lis' = lis,
    keepAlloc, keepBoot, keepKernel, keepHold, keepCount, g' = g }

  // ------------------------------------------------------------------ quiescence (D8a-HOLD, item 7)
  // EVERY call first runs verify (read-only) and, if it fails, fences the prefix in the guest-prefix
  // order (it waits while a lift is in flight); a fence failure (fk) does not stop the quiesce. Then
  // the holder is added; the first holder latches and closes every intake listener (each close
  // removes its entry). Serialized with activation by the sequencer (callers check the guard).
  action quiesceTo(c: str, r: str, fk: bool, gap: bool): bool = all {
    val first = holders == Set()
    val keepL = BUG_QUIESCE_KEEPS_LISTENERS or BUG_ELEMENT_SET
    val verifies = not(BUG_QUIESCE_NO_FENCE) and (first or not(BUG_JOIN_NO_VERIFY))
    val wouldFence = verifies and not(link)
    val fences = wouldFence and not(fk)
    val closes = and { first, not(keepL), lis != Set() }
    val routeAfter = if (fences) (if (BUG_REMOVE_NOT_FENCE) "absent" else "fence") else route
    val widens = and { closes, up, routeAfter == "local", not(link) or gap }
    all {
      fk implies wouldFence, gap implies (verifies and link and closes), wouldFence implies lockFree,
      holders' = holders.union(Set(c)), latch' = true, rec' = rec.set(c, r), flowL' = flowL,
      lis' = if (keepL or not(first)) lis else Set(),
      entries' = if (keepL or not(first) or BUG_ENTRY_OUTLIVES_CLOSE) entries else Set(),
      closeKernel(fences, gap),
      g' = { ...g, overlapped: g.overlapped or not(first),
             quiesceWidened: g.quiesceWidened or widens,
             gpsWidened: g.gpsWidened or (widens and c == GPS),
             quiesceFenced: g.quiesceFenced or (fences and c != GPS),
             joinFenced: g.joinFenced or (fences and not(first)) },
    },
    keepAlloc, keepBoot, rules' = rules, verified' = verified,
  }
  // clean audit: restore_forwarding(hold) — removes only this holder; reopens only if none remains
  // (listeners are brought back by the owner's bring-up steps). BUG_ANY_RESTORE: one latch — the
  // restore reopens and every other recovery loses its hold. BUG_RESTORE_LIFTS_FENCE: reopening
  // also puts the `local` route back. The recovery episode ends: its attempt budget resets.
  action RecRestore(c: str): bool = all {
    open, rec.get(c) == "repaired", not(damaged(c)),
    val others = holders.exclude(Set(c))
    val reopens = BUG_ANY_RESTORE or others == Set()
    val lifts = and { BUG_RESTORE_LIFTS_FENCE, reopens, route == "fence" }
    all {
      if (BUG_ANY_RESTORE) all {
        holders' = Set(), latch' = false, rec' = recIdle, att' = att0,
        g' = { ...g, otherLift: g.otherLift or lifts,
               reopenEarly: g.reopenEarly or COMPONENTS.exists(d => d != c and rec.get(d).in(UNREPAIRED)) },
      } else all {
        holders' = others, latch' = others != Set(), rec' = rec.set(c, "idle"), att' = att.set(c, 0),
        g' = { ...g, stillQuiesced: g.stillQuiesced or others != Set(), otherLift: g.otherLift or lifts,
               reopenEarly: g.reopenEarly or (others == Set() and COMPONENTS.exists(d => d != c and rec.get(d).in(UNREPAIRED))) },
      },
      if (lifts) localRoute else keepRoute,
    },
    flowL' = flowL, rules' = rules, link' = link, verified' = verified.exclude(Set(c)),
    keepListen, keepAlloc, keepBoot, keepLeft, repLeft' = repLeft, fenceLeft' = fenceLeft,
    tick(c == GPS),
  }
  // BUG_REPAIR_WITHOUT_HOLD: the recovery repairs without quiescing (and, for the guest prefix,
  // without fencing or verifying)
  action RecRepairNoHold(c: str): bool = all {
    BUG_REPAIR_WITHOUT_HOLD, open, rec.get(c) == "idle", damaged(c),
    rules' = if (c == "IpRules") true else rules, link' = if (c == GPS) true else link,
    flowL' = if (c == "FlowListeners") true else flowL,
    verified' = verified, keepRoute,
    rec' = rec, holders' = holders, latch' = latch,
    keepListen, keepAlloc, keepBoot, keepLeft, keepRec, tick(c == GPS), g' = { ...g, repairNoHold: true },
  }

  // ------------------------------------------------------------------ IpRules / FlowListeners recovery
  // the audit finds damage of c; the recovery quiesces under its own hold (the call verifies + fences)
  action RecQuiesce(c: str): bool = {
    nondet fk = fkSet.oneOf()
    nondet gap = gapSet.oneOf()
    all {
      OTHERS.contains(c), open, rec.get(c) == "idle", damaged(c), alloc != "Activating",
      not(BUG_REPAIR_WITHOUT_HOLD),
      quiesceTo(c, "held", fk, gap), recCount(c, att.get(c), repLeft, fenceAfter(fk)),
    }
  }
  // damaged again after the repair (a failed post-repair audit: a failed attempt, UP-7); repair again
  // under the same hold
  action RecRedo(c: str): bool = all {
    OTHERS.contains(c), open, rec.get(c) == "repaired", damaged(c),
    rec' = rec.set(c, "held"), holders' = holders, latch' = latch, flowL' = flowL,
    keepKernel, keepListen, keepAlloc, keepBoot,
    recCount(c, if (BUG_REDO_NO_COUNT) att.get(c) else att.get(c) + 1, repLeft, fenceLeft), g' = g,
  }

  // ------------------------------------------------------------------ repairs
  // repair_guest_prefix(&hold), first step: the fence (every attempt begins with it, UP-6). A fence
  // failure fails the attempt.
  action RepFence = all {
    open, rec.get(GPS) == "held", holders.contains(GPS), att.get(GPS) < MAXATT, lockFree,
    any {
      all {
        fenceRoute, rules' = rules, link' = link, verified' = verified.exclude(Set(GPS)),
        rec' = rec.set(GPS, "fenced"), recCount(GPS, att.get(GPS), repLeft, fenceLeft),
      },
      all {
        fenceLeft > 0, keepKernel, rec' = rec,
        recCount(GPS, att.get(GPS) + 1, repLeft, fenceLeft - 1),
      },
    },
    holders' = holders, latch' = latch, flowL' = flowL, keepListen, keepAlloc, keepBoot, g' = g,
  }
  // the repair's converge step. Guest prefix: GuestPrefixSteering::converge (re-attaches; never
  // detaches), after its fence. Others: the component's own repair only (IpRules: the rules;
  // FlowListeners: the task) — the steering and the route untouched. BUG_OTHER_REPAIRS_PREFIX: every
  // other repair also converges the steering and goes on to verify + lift. BUG_NO_FRESH_VERIFY: the
  // guest-prefix repair skips verify + probe.
  action RepConverge(c: str): bool = all {
    open, holders.contains(c),
    if (c == GPS) rec.get(c) == "fenced" else and { rec.get(c) == "held", att.get(c) < MAXATT },
    val prefix = c == GPS or BUG_OTHER_REPAIRS_PREFIX
    all {
      rules' = if (c == "IpRules") true else rules,
      flowL' = if (c == "FlowListeners") true else flowL,
      link' = if (prefix) true else link,
      rec' = rec.set(c, if (not(prefix)) "repaired" else if (BUG_NO_FRESH_VERIFY and c == GPS) "ver" else "conv"),
    },
    verified' = verified.exclude(Set(c)), keepRoute, holders' = holders, latch' = latch,
    keepListen, keepAlloc, keepBoot, recCount(c, att.get(c), repLeft, fenceLeft), g' = g,
  }
  // the repair returns Err (possibly forever): the attempt fails. Guest prefix (converge / verify /
  // probe failed): the fence stays, nothing is written. Others: their component repair failed,
  // nothing of the prefix is written. BUG_OTHER_REPAIRS_PREFIX: round 4c's failing total
  // converge_shared fences.
  action RepFail(c: str): bool = all {
    open, holders.contains(c), REPAIR_FAIL_FOREVER or repLeft > 0,
    val fencesPrefix = c != GPS and BUG_OTHER_REPAIRS_PREFIX
    all {
      if (c == GPS) rec.get(c).in(Set("fenced", "conv"))
      else or { and { rec.get(c) == "held", att.get(c) < MAXATT }, and { BUG_OTHER_REPAIRS_PREFIX, rec.get(c) == "conv" } },
      fencesPrefix implies lockFree,
      if (fencesPrefix) fenceRoute else keepRoute,
    },
    rules' = rules, link' = link, verified' = verified.exclude(Set(c)),
    rec' = rec.set(c, "held"), holders' = holders, latch' = latch, flowL' = flowL,
    keepListen, keepAlloc, keepBoot,
    recCount(c, att.get(c) + 1, if (REPAIR_FAIL_FOREVER) repLeft else repLeft - 1, fenceLeft), g' = g,
  }
  // verify + probe of the lift — Ok iff the link is attached as ours: the lift is now in flight (the
  // guest-prefix order is held until its route write); else the attempt fails (the fence stays)
  action RepVerify(c: str): bool = all {
    open, rec.get(c) == "conv", BUG_UNSERIALIZED_LIFT or not(liftInFlightBy(c)),
    val fencesPrefix = c != GPS and BUG_OTHER_REPAIRS_PREFIX
    if (link) all {
      verified' = verified.union(Set(c)), verSince' = true, tamper' = false, route' = route,
      estab' = estab, rmv' = rmv, rfn' = rfn, rec' = rec.set(c, "ver"),
      recCount(c, att.get(c), repLeft, fenceLeft),
    } else all {
      fencesPrefix implies lockFree,
      if (fencesPrefix) fenceRoute else keepRoute,
      verified' = verified.exclude(Set(c)), rec' = rec.set(c, "held"),
      recCount(c, att.get(c) + 1, repLeft, fenceLeft),
    },
    rules' = rules, link' = link, holders' = holders, latch' = latch, flowL' = flowL,
    keepListen, keepAlloc, keepBoot, g' = g,
  }
  // the lift — `local` replaces the fence (or a missing / different route)
  action RepRoute(c: str): bool = all {
    open, rec.get(c) == "ver",
    localRoute, rules' = rules, link' = link, verified' = verified,
    rec' = rec.set(c, "repaired"), holders' = holders, latch' = latch, flowL' = flowL,
    keepListen, keepAlloc, keepBoot, recCount(c, att.get(c), repLeft, fenceLeft),
    g' = { ...g, routeUnverified: g.routeUnverified or not(verified.contains(c)),
           widenRoute: g.widenRoute or (route != "local" and not(link)),
           otherLift: g.otherLift or (c != GPS and route != "local") },
  }
  // ADR-0124: the recovery's attempt budget is spent — serve fail-stops (process-scoped state gone;
  // node-global objects, the route among them, stay as they are)
  action FailStop(c: str): bool = all {
    open, rec.get(c) == "held", att.get(c) >= MAXATT,
    up' = false, boot' = 0, stop' = if (c == GPS) "failstopGps" else "failstop", lis' = Set(), verified' = Set(),
    entries' = if (BUG_ELEMENT_SET or BUG_ENTRY_OUTLIVES_CLOSE) entries else Set(),
    link' = if (BUG_UNPINNED) false else link, rules' = rules, keepRoute,
    alloc' = "Absent", gen' = gen, sess' = false, lastLS' = Set(),
    holders' = Set(), latch' = false, rec' = recIdle, flowL' = true,
    keepLeft, att' = att0, repLeft' = repLeft, fenceLeft' = fenceLeft, tick(c == GPS), g' = g,
  }

  // ------------------------------------------------------------------ guest-prefix recovery
  // the audit finds the guest prefix damaged (verify fails, or the route is not `local` while no
  // Recovery(GuestPrefixSteering) hold stands)
  action AuditFind = all {
    open, rec.get(GPS) == "idle", damaged(GPS),
    rec' = rec.set(GPS, "found"), holders' = holders, latch' = latch, flowL' = flowL,
    keepKernel, keepListen, keepAlloc, keepBoot, recCount(GPS, att.get(GPS), repLeft, fenceLeft), g' = g,
  }
  // quiesce_forwarding(Recovery(GuestPrefixSteering)): verifies and fences first (every call)
  action RecQuiesceGps = {
    nondet fk = fkSet.oneOf()
    nondet gap = gapSet.oneOf()
    all {
      open, rec.get(GPS) == "found", alloc != "Activating", not(BUG_REPAIR_WITHOUT_HOLD),
      quiesceTo(GPS, "held", fk, gap), recCount(GPS, att.get(GPS), repLeft, fenceAfter(fk)),
    }
  }
  // the post-repair audit finds the steering unverified: a failed attempt (UP-7); the next attempt
  // is repair_guest_prefix again, whose first step is the fence (UP-6). BUG_REDO_NO_FENCE: the
  // re-attempt goes straight to converge.
  action RecRedoGps = all {
    open, rec.get(GPS) == "repaired", damaged(GPS),
    rec' = rec.set(GPS, if (BUG_REDO_NO_FENCE) "fenced" else "held"), holders' = holders, latch' = latch,
    flowL' = flowL, keepKernel, keepListen, keepAlloc, keepBoot,
    recCount(GPS, if (BUG_REDO_NO_COUNT) att.get(GPS) else att.get(GPS) + 1, repLeft, fenceLeft), g' = g,
  }

  action OwnerStep = {
    nondet p = PORTS.oneOf()
    any { BindListen(p), Steer(p), TakeDown(p), LateRevoke(p), ActivateOk }
  }
  action Env = {
    nondet p = PORTS.oneOf()
    nondet v = Set(true, false).oneOf()
    nondet rv = Set("absent", "local", "fence").oneOf()
    any {
      BindFail(p), SteerFail(p), RevokeFail(p), ListenState(p, v), Provision, SessOpen, SessLoss,
      ActivateBegin, Teardown, Release, Crash, Restart, ConvergeFail, ForeignFlush, RootDetach,
      RootRoute(rv), RootEntryDelete(p), ListenerTaskExit,
    }
  }
  action GpsStep = any {
    AuditFind, RecQuiesceGps, RepFence, RepConverge(GPS), RepFail(GPS), RepVerify(GPS), RepRoute(GPS),
    RecRedoGps, FailStop(GPS), RecRestore(GPS), RecRepairNoHold(GPS),
  }
  action recStep(c: str): bool = any {
    RecQuiesce(c), RepConverge(c), RepFail(c), RepVerify(c), RepRoute(c), RecRedo(c), FailStop(c),
    RecRestore(c), RecRepairNoHold(c),
  }
  action OtherRec = {
    nondet c = OTHERS.oneOf()
    recStep(c)
  }
  // an in-flight lift completes its route write (releases the guest-prefix order)
  action LiftReturns = {
    nondet c = COMPONENTS.oneOf()
    RepRoute(c)
  }
  action step = any { BootStep, OwnerStep, Env, GpsStep, OtherRec }

  // ------------------------------------------------------------------ invariants
  // (D8a-LOOKUP) traffic to a workload address never reaches a wildcard host service
  val NoWildcardReached: bool = not(exposed)
  // (D8a-LOOKUP) a port that is not serving is never reachable (no connect completes on it)
  val ReachOnlyServing: bool = CLIENTS.forall(c => PORTS.forall(p => lands(c, p) == "intake" implies servingTrue(p)))
  // (D8a-LOOKUP) every steering entry names an open intake listener of a serving port
  val EntryNamesServingListener: bool = entries.forall(p => lis.contains(p) and servingTrue(p))
  // (D8a-LOOKUP) while serve is down nothing in the prefix is delivered to a host socket
  val NothingDeliveredWhileDown: bool = not(up) implies
    CLIENTS.forall(c => PORTS.forall(p => not(lands(c, p).in(Set("intake", "wildcard")))))
  // (D8a-LOOKUP, K-L5) remote traffic reaches an intake only through leg-C
  val RemoteNeverDirect: bool = PORTS.forall(p => lands("remote", p) != "intake")
  // (ADR-0152 point 2) unmarked host-local traffic stays on leg-C (needs the nft rules)
  val UnmarkedOnLegC: bool = PORTS.forall(p => lands("unmarked", p) != "intake")
  // (D8a-ROUTE) the `local` route is never present without the steering attached
  val RouteImpliesSteering: bool = route == "local" implies link
  // (D8a-ROUTE, D8a-FENCE) no lift (boot or repair_guest_prefix) sets `local` unless its own verify +
  // probe returned Ok
  val ConvergeNeedsSteering: bool = not(g.routeUnverified)
  // (D8a-ROUTE / D8a-FENCE) the route is `local` only if a lift's verify returned Ok after the last fence
  val RouteLocalOnlyVerified: bool = route == "local" implies verSince
  // (D8a-FENCE) once the route exists, prefix traffic is never routed elsewhere (fence, never removal)
  val NoElsewhere: bool = not(leaves)
  // (D8a-FENCE) a wildcard host service is reachable only in the trust-boundary window: after a root
  // detach (or root `local` route) and before the next fence (or the next lift-verify)
  val ExposureOnlyInTamperWindow: bool = exposed implies tamper
  // (D8a-FENCE, item 8) the detach window lasts at most two steps of the guest-prefix recovery (the
  // audit's finding and its quiesce's verify + fence; or the failed audit and the next attempt's fence)
  val ExposureWithinTwoSteeringSteps: bool = exposed implies win <= 1
  // ... counted while serve is open (after a fail-stop the exposure lasts until the next boot's converge)
  val OpenExposureWithinTwoSteeringSteps: bool = (open and exposed) implies win <= 1
  // (D8a-FENCE) from the repair's fence until its lift, the prefix is fenced
  val FencedWhileSteeringRecovers: bool = rec.get(GPS).in(Set("fenced", "conv", "ver")) implies route == "fence"
  // (D8a-FENCE: one owner, one runtime repair) nothing but boot and repair_guest_prefix lifts a fence
  val OnlyGuestPrefixRecoveryLifts: bool = not(g.otherLift)
  // (D8a-FENCE) while serve is down after a boot refusal or a fail-stop, with no root tampering since
  // the last fence / lift-verify, the prefix is fenced or `local` behind attached steering (UP-12) ...
  val FenceHoldsWhileStopped: bool = (stopped and not(tamper)) implies (route == "fence" or (route == "local" and link))
  // ... and every connection to it is refused
  val NothingWhileStopped: bool = (stopped and not(tamper)) implies CLIENTS.forall(c => PORTS.forall(p => lands(c, p) == "refused"))
  // (D8a-FENCE, UP-12) the guest-prefix recovery's fail-stop leaves the prefix fenced
  val GuestPrefixFailStopFenced: bool = (not(up) and stop == "failstopGps") implies route == "fence"
  // (item 7) no quiesce — of any holder — closes open intake listeners while, after its verify +
  // fence, the prefix is `local` with the steering unverified
  val QuiescenceNeverWidens: bool = not(g.quiesceWidened)
  // (item 7) the guest-prefix recovery's own quiesce never widens the exposure
  val SteeringQuiesceNeverWidens: bool = not(g.gpsWidened)
  // (item 7) no per-allocation close (teardown, session loss, ListenState off, activation rollback,
  // failed bring-up) widens the exposure
  val CloseNeverWidens: bool = not(g.widenClose)
  // (item 7, item 8: "no owner step widens it meanwhile") no owner close of any cause widens the exposure
  val NoOwnerStepWidens: bool = QuiescenceNeverWidens and CloseNeverWidens
  // (item 8) no lift widens the exposure (violated only by a root detach between a lift's verify and
  // its route write: the trust boundary)
  val RouteNeverWidens: bool = not(g.widenRoute)
  // (UP-5) prefix traffic leaves the node only between a root route removal and the next Overdrive
  // route write (the repair's fence, or a lift) ...
  val ElsewhereOnlyInRemovalWindow: bool = leaves implies rmv
  // ... within at most four steps of the guest-prefix recovery
  val ElsewhereWithinGpsRepair: bool = leaves implies winE <= 3
  // (UP-5) a root fence is lifted within at most seven steps of the guest-prefix recovery
  val RootFenceWithinGpsRepair: bool = rootFenced implies winF <= 6
  // (U-6) activation is all or nothing
  val ActivationAllOrNothing: bool = not(g.actPartial) and not(g.actResidue)
  // (U-6) at every point a Provisioned CID has no listener
  val ProvisionedHasNoListener: bool = alloc == "Provisioned" implies lis == Set()
  // (U-3) no lease is released while a listener of its allocation is open
  val NoReleaseWithListener: bool = not(g.relListener)
  // (D8a-HOLD, M-7) while forwarding is quiesced no intake listener or entry exists
  val QuiescedClosesListeners: bool = latch implies (lis == Set() and entries == Set())
  // (12, D8a-HOLD) forwarding is quiesced exactly while some holder holds
  val LatchIffHolders: bool = latch == (holders != Set())
  // (12, D8a-HOLD) a holding recovery is in the holder set (no restore ended another's hold)
  val HeldIsHolder: bool = COMPONENTS.forall(c => rec.get(c).in(HOLDING) implies holders.contains(c))
  // (12, D8a-HOLD) forwarding never reopened while another recovery had not repaired
  val NoReopenBeforeRepair: bool = not(g.reopenEarly)
  // (12, D8a-HOLD) no repair runs without that recovery's hold
  val NoRepairWithoutHold: bool = not(g.repairNoHold)

  val Safety: bool = and {
    NoWildcardReached, ReachOnlyServing, EntryNamesServingListener, NothingDeliveredWhileDown,
    RemoteNeverDirect, RouteImpliesSteering, ConvergeNeedsSteering, RouteLocalOnlyVerified,
    NoElsewhere, FenceHoldsWhileStopped, NothingWhileStopped, GuestPrefixFailStopFenced,
    FencedWhileSteeringRecovers, OnlyGuestPrefixRecoveryLifts, NoOwnerStepWidens, RouteNeverWidens,
    ActivationAllOrNothing, ProvisionedHasNoListener, NoReleaseWithListener, QuiescedClosesListeners,
    LatchIffHolders, HeldIsHolder, NoReopenBeforeRepair, NoRepairWithoutHold,
  }
  // what must still hold when root detaches the pinned link (trust boundary)
  val SafetyUnderDetach: bool = and {
    ReachOnlyServing, EntryNamesServingListener, ConvergeNeedsSteering, RouteLocalOnlyVerified,
    NoElsewhere, ExposureOnlyInTamperWindow, OpenExposureWithinTwoSteeringSteps,
    FencedWhileSteeringRecovers, OnlyGuestPrefixRecoveryLifts, NoOwnerStepWidens,
    SteeringQuiesceNeverWidens, FenceHoldsWhileStopped, NothingWhileStopped, ActivationAllOrNothing,
    ProvisionedHasNoListener, NoReleaseWithListener, QuiescedClosesListeners, LatchIffHolders,
    HeldIsHolder, NoReopenBeforeRepair, NoRepairWithoutHold,
  }
  // what must still hold when root removes, replaces or fences the tagged route (trust boundary)
  val SafetyUnderRouteTamper: bool = and {
    ReachOnlyServing, EntryNamesServingListener, ConvergeNeedsSteering, ExposureOnlyInTamperWindow,
    ElsewhereOnlyInRemovalWindow, ElsewhereWithinGpsRepair, RootFenceWithinGpsRepair,
    NoOwnerStepWidens, OnlyGuestPrefixRecoveryLifts, ActivationAllOrNothing, ProvisionedHasNoListener,
    NoReleaseWithListener, QuiescedClosesListeners, LatchIffHolders, HeldIsHolder,
    NoReopenBeforeRepair, NoRepairWithoutHold,
  }

  // ------------------------------------------------------------------ liveness (TLC)
  val bypassed: bool = not(UnmarkedOnLegC)
  val ownerVars = (lis, entries, alloc)
  val bootVars = (up, boot, link, route)
  val gpsVars = (rec, route, link, holders)
  // after the guest-prefix recovery has begun (the audit found damage), it ends with the prefix
  // `local` behind attached steering and the recovery idle, or with serve stopped and the prefix
  // fenced or `local` behind attached steering
  val resolved: bool = or {
    up and route == "local" and link and rec.get(GPS) == "idle",
    stopped and (route == "fence" or (route == "local" and link)),
  }
  // a serving declared port is eventually reachable (steered), if steer failures eventually stop;
  // also after a foreign entry deletion (the audit re-steers it). Port 1 is declared in every instance.
  temporal ServingReachable =
    weakFair(OwnerStep, ownerVars)
      .implies(always(servingTrue(1).implies(eventually(lands("marked", 1) == "intake" or not(servingTrue(1))))))
  // an activation eventually returns
  temporal ActivationCompletes =
    weakFair(OwnerStep, ownerVars)
      .implies(always((alloc == "Activating").implies(eventually(alloc != "Activating"))))
  // (D8a-FENCE, item 7, UP-6) after a root link detach the exposure ends under fairness of the
  // guest-prefix recovery's finding, quiesce (verify + fence), repair fence, failed-audit step and
  // fail-stop, the owner's activation steps, boot and restart — not of any repair (converge, verify,
  // lift) and not of any other recovery
  temporal DetachExposureEndsFindFence =
    and {
      weakFair(AuditFind, gpsVars), strongFair(RecQuiesceGps, gpsVars), weakFair(RepFence, gpsVars),
      weakFair(RecRedoGps, gpsVars), weakFair(FailStop(GPS), gpsVars), weakFair(OwnerStep, ownerVars),
      weakFair(BootStep, bootVars), weakFair(Restart, bootVars),
    }.implies(always(exposed.implies(eventually(not(exposed)))))
  // (D8a-FENCE, item 8) ... under fairness of the guest-prefix recovery's own steps only (strong: its
  // quiesce waits behind an activation item), boot and restart; no fairness of the owner or of any
  // other recovery
  temporal DetachExposureEnds =
    and {
      strongFair(GpsStep, gpsVars), weakFair(BootStep, bootVars), weakFair(Restart, bootVars),
    }.implies(always(exposed.implies(eventually(not(exposed)))))
  // ... plus fairness of the owner's activation steps (the sequencer orders the recovery's quiesce,
  // and so its verify + fence, behind an in-flight activation: UP-13)
  temporal DetachExposureEndsWithOwner =
    and {
      strongFair(GpsStep, gpsVars), weakFair(OwnerStep, ownerVars),
      weakFair(BootStep, bootVars), weakFair(Restart, bootVars),
    }.implies(always(exposed.implies(eventually(not(exposed)))))
  // (D8a-FENCE, D8a-HOLD, ADR-0124) the guest-prefix recovery resolves: the prefix becomes `local`
  // again behind verified steering, or serve stops — under fairness of that recovery's own steps, of
  // the owner's activation steps, boot and restart; serve eventually stays up
  temporal SteeringRecoveryResolves =
    and {
      strongFair(GpsStep, gpsVars), weakFair(OwnerStep, ownerVars),
      weakFair(BootStep, bootVars), weakFair(Restart, bootVars), eventually(always(up)),
    }.implies(always((rec.get(GPS) == "found").implies(eventually(resolved))))
  // ... and also when every other recovery is fair (with one owner, no recovery undoes another's
  // progress, UP-11)
  temporal SteeringRecoveryResolvesAllRecoveries =
    and {
      strongFair(GpsStep, gpsVars), strongFair(recStep("IpRules"), gpsVars),
      strongFair(recStep("FlowListeners"), gpsVars), weakFair(OwnerStep, ownerVars),
      weakFair(BootStep, bootVars), weakFair(Restart, bootVars), eventually(always(up)),
    }.implies(always((rec.get(GPS) == "found").implies(eventually(resolved))))
  // after a foreign nft flush, unmarked host-local traffic returns to leg-C — under fairness of the
  // IpRules recovery's own steps only (plus an in-flight lift returning: its quiesce may wait for the
  // guest-prefix order)
  temporal LegCBypassEnds =
    and {
      weakFair(OwnerStep, ownerVars),
      strongFair(RecQuiesce("IpRules"), (rec, holders, rules)),
      strongFair(RepConverge("IpRules"), (rec, holders, rules)),
      weakFair(RecRedo("IpRules"), (rec, holders, rules)),
      weakFair(FailStop("IpRules"), (rec, holders, rules)),
      weakFair(LiftReturns, gpsVars),
      weakFair(BootStep, bootVars), weakFair(Restart, bootVars), eventually(always(up)),
    }.implies(always(bypassed.implies(eventually(not(bypassed)))))
  // (UP-5) after root removes (or replaces) the tagged route, prefix traffic stops leaving the node —
  // under fairness of the guest-prefix recovery's own steps, the owner's steps, boot and restart;
  // serve eventually stays up
  temporal RouteTamperEnds =
    and {
      weakFair(OwnerStep, ownerVars), strongFair(GpsStep, gpsVars),
      weakFair(BootStep, bootVars), weakFair(Restart, bootVars), eventually(always(up)),
    }.implies(always(leaves.implies(eventually(not(leaves)))))
  // (UP-5) after root fences the prefix, the fence is lifted — same fairness
  temporal RootFenceEnds =
    and {
      weakFair(OwnerStep, ownerVars), strongFair(GpsStep, gpsVars),
      weakFair(BootStep, bootVars), weakFair(Restart, bootVars), eventually(always(up)),
    }.implies(always(rootFenced.implies(eventually(not(rootFenced)))))
  // serve eventually opens (no crash: instance steer_boot)
  temporal BootOpens =
    and { weakFair(BootStep, bootVars), weakFair(Restart, bootVars) }.implies(eventually(open))

  // ------------------------------------------------------------------ non-vacuity witnesses
  // Each is EXPECTED to be violated: the violation proves the behaviour is reachable.
  val WitnessNoSteered: bool = not(alloc == "Active" and entries != Set())
  val WitnessNoSecondAllocation: bool = gen < 2 or alloc != "Active"
  val WitnessNoActivationErr: bool = not(g.actErrDone)
  val WitnessNoBootRefusal: bool = not(g.refused)
  val WitnessNoResteer: bool = not(g.resteered)
  val WitnessNoOverlap: bool = not(g.overlapped)
  val WitnessNoStillQuiesced: bool = not(g.stillQuiesced)
  val WitnessNoCrashWithRoute: bool = not(not(up) and route == "local" and link)
  val WitnessNoLegC: bool = CLIENTS.forall(c => PORTS.forall(p => lands(c, p) != "legC"))
  // the guest-prefix repair fenced (state `fenced` reached)
  val WitnessNoRecoveryFence: bool = rec.get(GPS) != "fenced"
  val WitnessNoFailStop: bool = not(stopped and stop == "failstopGps")
  val WitnessNoRefusedWhileDown: bool = not(stopped and route == "fence")
  // the window spans the audit's finding: an exposure survives one guest-prefix-recovery step
  val WitnessNoOneStepExposure: bool = not(exposed and win >= 1)
  // item 7: a non-guest-prefix holder's quiesce found the steering unverified and fenced
  val WitnessNoQuiesceFence: bool = not(g.quiesceFenced)
  // item 7: a joining quiesce call fenced
  val WitnessNoJoinFence: bool = not(g.joinFenced)
  // item 7: a per-allocation listener close fenced
  val WitnessNoCloseFence: bool = not(g.closeFenced)
  // UP-5: the guest-prefix recovery repaired after a route finding
  val WitnessNoRouteRepair: bool = not(rec.get(GPS) == "repaired" and link)
  // UP-5: traffic still leaves the node after three guest-prefix-recovery steps
  val WitnessNoThreeStepElsewhere: bool = not(leaves and winE >= 3)
  // UP-5: the root fence still stands after six guest-prefix-recovery steps
  val WitnessNoSixStepRootFence: bool = not(rootFenced and winF >= 6)
  // While the guest-prefix recovery holds, Overdrive's marked clients to a port the guest is
  // listening on reach a wildcard host service.
  val WitnessNoServableWildcardWhileHeld: bool = not(and {
    holders.contains(GPS), alloc == "Active", sess,
    DECLARED.exists(p => lastLS.contains(p) and lands("marked", p) == "wildcard"),
  })
}

// ------------------------------------------------------------------ design instances

// The design under every environment fault except root tampering of the link / route: crash at any
// step, foreign nft flush, root entry deletion, listener-task exit (overlapping recoveries), one bind
// failure, steer failing forever, one boot converge / verify failure.
module steer_ok {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = true, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "./steering"
}

// The design without a foreign flush: unmarked host-local traffic always stays on leg-C.
module steer_noflush {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = true, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = true, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = true, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "./steering"
}

// Two declared ports: an activation can fail on one port after steering the other (rollback).
module steer_two {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1, 2), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 1, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = true, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "./steering"
}

// Liveness: steer failures stop (one), entry deletion and crashes possible.
module steer_live {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = true, BIND_FAILURES = 1,
    STEER_FAILURES = 1, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 0, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = true, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "./steering"
}

// Boot with no crash: serve opens.
module steer_boot {
  import steering(
    PORTS = Set(1, 2), DECLARED = Set(1), MAXGEN = 2, ALLOW_CRASH = false, BIND_FAILURES = 0,
    STEER_FAILURES = 0, STEER_FAIL_FOREVER = false, CONVERGE_FAILURES = 1, REVOKE_FAILURES = 0,
    REPAIR_FAILURES = 0, REPAIR_FAIL_FOREVER = false, MAXATT = 2, FENCE_FAILURES = 0,
    ENV_FOREIGN_FLUSH = false, ENV_ROOT_ENTRY_DELETE = false, ENV_ROOT_DETACH = false,
    ENV_ROOT_ROUTE = false, ENV_LISTENER_EXIT = false, ENV_DETACH_IN_CLOSE_GAP = false,
    BUG_ELEMENT_SET = false, BUG_ELEMENT_CLOSE_FIRST = false, BUG_STEERING_IN_NFT = false,
    BUG_UNPINNED = false, BUG_NONLOOPBACK_ASSIGN = false, BUG_ENTRY_OUTLIVES_CLOSE = false,
    BUG_ROUTE_BEFORE_STEERING = false, BUG_NO_VERIFY = false, BUG_ROLLBACK_LEAVES_LISTENER = false,
    BUG_ACT_OK_PARTIAL = false, BUG_TEARDOWN_KEEPS_LISTENERS = false,
    BUG_QUIESCE_KEEPS_LISTENERS = false, BUG_ANY_RESTORE = false, BUG_REPAIR_WITHOUT_HOLD = false,
    BUG_REMOVE_NOT_FENCE = false, BUG_NO_FRESH_VERIFY = false, BUG_RESTORE_LIFTS_FENCE = false,
    BUG_QUIESCE_NO_FENCE = false, BUG_JOIN_NO_VERIFY = false, BUG_CLOSE_NO_VERIFY = false,
    BUG_OTHER_REPAIRS_PREFIX = false, BUG_UNSERIALIZED_LIFT = false, BUG_AUDIT_IGNORES_ROUTE = false,
    BUG_REDO_NO_FENCE = false, BUG_REDO_NO_COUNT = false).* from "./steering"
}
