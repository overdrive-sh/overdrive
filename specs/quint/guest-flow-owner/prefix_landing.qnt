// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — where a connection to a workload address in the guest
 * prefix lands. This file holds two modules:
 *   `prefix_landing`  the pure definition (`lands` and helpers; no state), imported by three
 *                     steering modules (prefix_boot, intake, shared_table; quiescence abstracts the
 *                     landing to one wanted listener). Never a conformance target.
 *   `landing_machine` the state machine the QPROJ-8 real-kernel conformance driver replays
 *                     (quint-connect, instances `landing_ok`): the production-reachable
 *                     configuration transitions (boot, deploy, guest listens, serve killed /
 *                     stopped gracefully / restarted) and a probe-connect per client class x port.
 *
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" (D8a-LOAD-SWAP):
 * § Driven port — guest-prefix steering ("Decision (the program, constant)"), § Formal protocol
 * model (assumption table); ADR-0171 (D8a-LOOKUP), ADR-0152 (D8a), ADR-0178 (INTAKE-BIND). The
 * route has two forms, `absent` and `local` (the fence form was removed in revision 10).
 *
 * INTAKE-BIND (2026-10-09): the intake listener for a declared port p is bound to
 * workload_addr:(20480 + i), never to workload_addr:p, so no lookup finds it by its bound address:
 * it is reached only through its steering entry (K-L10). `lis` therefore matters only together
 * with an entry under the attached program.
 *
 * Client classes: `marked` = Overdrive's own marked clients (leg-S, probes; exempt from the nft
 * divert / reject); `unmarked` = any other host-local client; `remote` = a packet arriving on a
 * non-loopback device. A wildcard host service is assumed bound on EVERY port (worst case).
 * Outcomes: intake | legC | refused | wildcard (a host service: exposure) | elsewhere (routed off
 * the node).
 *
 * ASSUMPTIONS encoded here (discharged in README.md):
 *   K-D1  `local` route and no lookup decision → a connection to workload_addr:p reaches a
 *         wildcard host listener on p (an intake listener is never bound to workload_addr:p,
 *         INTAKE-BIND) ............................................................................. V-19
 *   K-D2  nft: non-exempt prefix traffic is diverted to leg-C, else rejected / dropped; a divert
 *         with no leg-C socket falls through to the reject ........................................ V-19
 *   K-D4  no tagged route → the prefix is not local: routed elsewhere unless nft diverts / rejects V-19
 *   K-L1  sk_lookup decides every loopback-ingress lookup to a `local` prefix address ......... V-26
 *   K-L2  a leg-C TPROXY-assigned packet bypasses sk_lookup ..................................... V-26
 *   K-L5  ingress is loopback for host-local connects, the receiving device otherwise ......... V-26
 *   K-L10 sk_lookup assigns the listener an entry names whatever address and port it is bound to V-26
 */
module prefix_landing {
  type Net = {
    marked: bool,       // Overdrive's marked clients exist (serve is serving)
    legC: bool,         // the leg-C socket exists
    divert: bool,       // nft: prefix traffic diverted to leg-C (output + prerouting)
    reject: bool,       // nft: output reject / prerouting drop of non-exempt prefix traffic
    route: str,         // absent | local  (the prefix's Overdrive-tagged route)
    steer: str,         // none | ours  (the pinned sk_lookup program, if attached; the shipped program is correct, V-26)
    entries: Set[int],  // ports with a steering entry
    lis: Set[int],      // declared ports with an open intake listener (bound to its block port)
    assignRemote: bool, // hazard hook: the program also assigns non-loopback ingress
  }

  val CLIENTS: Set[str] = Set("marked", "unmarked", "remote")

  // the socket lookup for a connection that reached local delivery
  def lookup(n: Net, p: int, loopback: bool): str =
    if (n.steer == "ours") {
      // K-L10: the entry for p assigns its listener whatever that listener is bound to
      if ((loopback or n.assignRemote) and n.entries.contains(p) and n.lis.contains(p)) "intake"
      else "refused"                                     // dropped; TCP answered with a reset
    } else "wildcard"                                    // no decision: K-D1 (INTAKE-BIND: no bind match)

  def lands(n: Net, c: str, p: int): str =
    if (c == "marked" and not(n.marked)) "refused"          // no such client
    else if (c != "marked" and n.divert and n.legC) "legC"   // K-D2 divert (K-L2: no lookup)
    else if (c != "marked" and n.reject) "refused"           // K-D2 reject / drop
    else if (n.route == "absent") "elsewhere"                // K-D4
    else lookup(n, p, c != "remote")

  def exposedOn(n: Net, ports: Set[int]): bool =
    CLIENTS.exists(c => ports.exists(p => lands(n, c, p) == "wildcard"))
  def elsewhereOn(n: Net, ports: Set[int]): bool =
    CLIENTS.exists(c => ports.exists(p => lands(n, c, p) == "elsewhere"))
  def deliveredOn(n: Net, ports: Set[int]): bool =
    CLIENTS.exists(c => ports.exists(p => lands(n, c, p).in(Set("intake", "wildcard"))))
}

/**
 * `landing_machine` — QPROJ-8 on the real kernel, as a state machine.
 *
 * Every action is a production-reachable transition the conformance driver makes through the
 * production composition (`run_server`, the HTTPS API, `ServerHandle::kill_for_test` /
 * `shutdown`); no route, rule, entry or listener is ever written by the driver. The `Net` inputs
 * are variables the driver READS by independent kernel queries (never writes) and compares after
 * every step; the observed landing of the last probe per (client class, port) is state.
 *
 * Abstraction (QPROJ-8): one declared-and-listening port (1), one declared-but-silent port (2) and
 * one undeclared port (3); the address does not influence `lands`, so the leased / unleased
 * targets are not distinguished. The shared nft table is only ever absent or complete (U-QP-1);
 * `absent` is outside the QPROJ-8 configurations (shared_table covers it). `assignRemote` is
 * false: not reachable through production, covered by shared_table's hazard instance.
 * Boot is one atomic step here; its internal ordering (steering before route) is prefix_boot's.
 *
 * Added assumptions (README): K-P1 (a killed / stopped serve closes leg-C and every intake
 * listener, and a listener's steering entry leaves with its socket: K-L3, K-L4).
 *
 * OFF names design rules switched off (hazard/prefix_landing_hazards.qnt):
 *   unpinned    D8a-LOOKUP / ADR-0159: the steering program is process-owned (gone with serve);
 *               not load-bearing HERE (the constant rules refuse a down node on their own): its
 *               teeth, with the table absent, are shared_table's `table_bugUnpinned`
 *   volatile    U-4: route, steering and constant rules are removed when serve goes down
 *   noRules     D8a / K-D2: boot installs no constant divert / reject rules
 *   routeFirst  D8a-LOAD-SWAP / D8a-ROUTE: boot writes the route without attached steering
 */
module landing_machine {
  import prefix_landing.* from "./prefix_landing"

  const OFF: Set[str]
  def bug(r: str): bool = OFF.contains(r)

  val PORTS: Set[int] = Set(1, 2, 3)   // 1 declared + listening, 2 declared + silent, 3 undeclared

  var marked: bool          // C: serve booted by the driver and neither killed nor stopped
  var legC: bool            // K: ss on leg-C's address
  var divert: bool          // K: nft list table
  var reject: bool          // K: nft list table
  var route: str            // K: ip route show proto GUEST_PREFIX_ROUTE_PROTOCOL
  var steer: str            // K: bpftool link on the pin path
  var entries: Set[int]     // K: bpftool map dump decoded with guest_steering_key
  var lis: Set[int]         // K: ss -ltn on the workload address
  var seen: (str, int) -> str   // observed landing of the last probe per cell; absent = unprobed
  var wl: Set[str]          // stimulus: workloads the driver deployed (listening | silent)
  var everBooted: bool      // ghost (model-only): serve has booted at least once

  val net: Net = {
    marked: marked, legC: legC, divert: divert, reject: reject, route: route, steer: steer,
    entries: entries, lis: lis, assignRemote: false,
  }

  action init = all {
    marked' = false, legC' = false, divert' = false, reject' = false, route' = "absent",
    steer' = "none", entries' = Set(), lis' = Set(), seen' = Map(), wl' = Set(), everBooted' = false,
  }

  // every configuration change invalidates the last probes (the driver re-probes)
  action cfg(m: bool, lg: bool, dv: bool, rj: bool, rt: str, st: str, en: Set[int], ls: Set[int], w: Set[str], eb: bool): bool = all {
    marked' = m, legC' = lg, divert' = dv, reject' = rj, route' = rt, steer' = st,
    entries' = en, lis' = ls, wl' = w, everBooted' = eb, seen' = Map(),
  }

  // `serve` starts (first boot or restart over the kept route / steering / rules): leg-C exists,
  // the constant rules and the route are in place, the shipped program is attached; the intake
  // listeners are opened by OpenIntake once the guest listens.
  action Boot: bool = all {
    not(marked),
    cfg(true, true, not(bug("noRules")), not(bug("noRules")), "local",
        if (bug("routeFirst")) "none" else "ours", entries, lis, wl, true),
  }

  // POST a workload: a guest listening on declared port 1, or a silent guest declaring port 2
  action Deploy(k: str): bool = all {
    marked, not(wl.contains(k)),
    cfg(marked, legC, divert, reject, route, steer, entries, lis, wl.union(Set(k)), everBooted),
  }

  // the listening guest's intake listener is open and steered (needs serve, the workload, and
  // the guest listening: a silent guest declares port 2 but opens nothing)
  action OpenIntake: bool = all {
    marked, wl.contains("listening"), not(lis.contains(1)),
    cfg(marked, legC, divert, reject, route, steer, entries.union(Set(1)), lis.union(Set(1)), wl, everBooted),
  }

  // what survives serve going down (K-P1, K-L3, K-L4, U-4)
  action Down(w: Set[str]): bool = all {
    marked,
    cfg(false, false,
        divert and not(bug("volatile")), reject and not(bug("volatile")),
        if (bug("volatile")) "absent" else route,
        if (bug("volatile") or bug("unpinned")) "none" else steer,
        Set(), Set(), w, everBooted),
  }
  action ServeKilled: bool = Down(wl)              // ServerHandle::kill_for_test (SIGKILL)
  action ServeStopped: bool = Down(Set())          // workloads stopped through the API, then ServerHandle::shutdown

  // connect by a client of class c to workload_addr:p; the first bytes decide the landing
  action Probe(c: str, p: int): bool = all {
    marked' = marked, legC' = legC, divert' = divert, reject' = reject, route' = route,
    steer' = steer, entries' = entries, lis' = lis, wl' = wl, everBooted' = everBooted,
    seen' = seen.put((c, p), lands(net, c, p)),
  }

  action step = {
    nondet c = CLIENTS.oneOf()
    nondet p = PORTS.oneOf()
    nondet k = Set("listening", "silent").oneOf()
    any { Boot, Deploy(k), OpenIntake, ServeKilled, ServeStopped, Probe(c, p) }
  }

  // ------------------------------------------------------------------ invariants
  val Cells: Set[(str, int)] = CLIENTS.map(c => PORTS.map(p => (c, p))).flatten()
  // D8a, K-D1, A-33: no connection to the prefix reaches a host service, in any configuration
  val NoWildcardReached: bool = Cells.forall(k => lands(net, k._1, k._2) != "wildcard")
  // QPROJ-8: an observed landing is the landing `lands` computes over the Net it was observed under
  val SeenIsLands: bool = seen.keys().forall(k => seen.get(k) == lands(net, k._1, k._2))
  // U-4, K-D3: while serve is down nothing is delivered (intake, leg-C or a host service)
  val NothingDeliveredWhileDown: bool =
    marked or Cells.forall(k => not(lands(net, k._1, k._2).in(Set("intake", "legC", "wildcard"))))
  // D8a / INTAKE-BIND / K-L10: an intake listener is reached only by Overdrive's marked client,
  // only through its steering entry, and by no remote client; every other class is diverted or refused
  val IntakeOnlyThroughEntryForMarked: bool = Cells.forall(k =>
    lands(net, k._1, k._2) == "intake" implies
      (k._1 == "marked" and entries.contains(k._2) and lis.contains(k._2)))
  // D8a, U-4: once serve has booted, no connection is routed off the node (fail closed)
  val ElsewhereOnlyBeforeFirstBoot: bool =
    everBooted implies Cells.forall(k => lands(net, k._1, k._2) != "elsewhere")
  // a silent or undeclared port never lands at an intake listener; it has no entry
  val EntriesAreOpenListeners: bool = entries.subseteq(lis)
  val Safety: bool = and {
    NoWildcardReached, SeenIsLands, NothingDeliveredWhileDown, IntakeOnlyThroughEntryForMarked,
    ElsewhereOnlyBeforeFirstBoot, EntriesAreOpenListeners,
  }

  // ------------------------------------------------------------------ named runs
  // never booted: marked client does not exist; the others are routed off the node
  run NeverBooted = init
    .then(Probe("marked", 1)).then(Probe("unmarked", 1)).then(Probe("remote", 1))
    .expect(Safety and seen.get(("marked", 1)) == "refused"
            and seen.get(("unmarked", 1)) == "elsewhere" and seen.get(("remote", 1)) == "elsewhere")

  // serving, guest listening on port 1: marked reaches the intake listener, the others leg-C
  run ServingGuestListening = init
    .then(Boot).then(Deploy("listening")).then(OpenIntake)
    .then(Probe("marked", 1)).then(Probe("unmarked", 1)).then(Probe("remote", 1))
    .expect(Safety and seen.get(("marked", 1)) == "intake"
            and seen.get(("unmarked", 1)) == "legC" and seen.get(("remote", 1)) == "legC")

  // serving, guest declared port 2 but not listening: no entry, marked client refused; port 3 too
  run ServingGuestNotListening = init
    .then(Boot).then(Deploy("silent"))
    .then(Probe("marked", 2)).then(Probe("unmarked", 2)).then(Probe("remote", 2)).then(Probe("marked", 3))
    .expect(Safety and entries == Set() and seen.get(("marked", 2)) == "refused"
            and seen.get(("unmarked", 2)) == "legC" and seen.get(("remote", 2)) == "legC"
            and seen.get(("marked", 3)) == "refused")

  // serve killed: leg-C and the listeners are gone, route / steering / rules persist: all refused
  run ServeKilledRefusesEverything = init
    .then(Boot).then(Deploy("listening")).then(OpenIntake).then(ServeKilled)
    .then(Probe("marked", 1)).then(Probe("unmarked", 1)).then(Probe("remote", 1))
    .expect(Safety and not(marked) and route == "local" and steer == "ours" and divert and reject
            and entries == Set() and seen.get(("marked", 1)) == "refused"
            and seen.get(("unmarked", 1)) == "refused" and seen.get(("remote", 1)) == "refused")

  // serve restarted after a kill: leg-C back at once, the guest's listener steered again after OpenIntake
  run ServeRestartedAfterKill = init
    .then(Boot).then(Deploy("listening")).then(OpenIntake).then(ServeKilled)
    .then(Boot).then(Probe("marked", 1))
    .expect(Safety and seen.get(("marked", 1)) == "refused" and legC)
    .then(OpenIntake).then(Probe("marked", 1)).then(Probe("unmarked", 1))
    .expect(Safety and seen.get(("marked", 1)) == "intake" and seen.get(("unmarked", 1)) == "legC")

  // serve stopped gracefully (workloads stopped first): as killed, nothing delivered
  run ServeStoppedGracefully = init
    .then(Boot).then(Deploy("listening")).then(OpenIntake).then(ServeStopped)
    .then(Probe("marked", 1)).then(Probe("unmarked", 1)).then(Probe("remote", 1))
    .expect(Safety and wl == Set() and route == "local" and steer == "ours"
            and seen.get(("marked", 1)) == "refused" and seen.get(("unmarked", 1)) == "refused"
            and seen.get(("remote", 1)) == "refused")
}

module landing_ok { import landing_machine(OFF = Set()).* from "./prefix_landing" }
