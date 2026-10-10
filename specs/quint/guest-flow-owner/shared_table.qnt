// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — steering concern 4: the shared nft table left absent by
 * Overdrive itself (K-D3, U-QP-1): the mTLS worker's single-transaction write at boot step 8 failing
 * on a node with no table, or a later replacement failing (the table stays as it was); the absent
 * table persisting while `serve` is down and converged again at boot step 8; the steering's
 * independence from the table; the leg-C bypass window; the table repair completing under the
 * IpRules recovery's hold (D8a-HOLD).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" — § Composition (boot
 * steps 8, 9 and the ordering invariant: "The nft table is not a precondition of the route"),
 * § Earned Trust ("the shared nft table left absent at boot step 8 (boot keeps the route …)"),
 * Gate G-V5, § Required boundary scenarios R5-30, § Formal protocol model (concern 4, K-D3),
 * § Conformance observation surface and action map (QPROJ-7 `shared_table_state`) and U-QP-1
 * (spec change S-5); ADR-0171, ADR-0169, ADR-0152.
 *
 * Abstraction:
 *   - The table is written and replaced only in single nft transactions (D26-PINS F-2, boot step 8),
 *     so it has exactly two states, `absent` and `complete` (U-QP-1): the divert rule (prefix
 *     traffic to leg-C) and the reject / drop rule (non-exempt prefix traffic) are present together
 *     or not at all. A failed first write leaves it absent; a failed replacement and a crash leave
 *     it as it was (the replaced-over table has the same constant rules and is `complete`).
 *   - Boot: steps 1–7 one step; step 8 (`start_shared_owner`: leg-C, then the table's transaction)
 *     succeeds (the table is complete) or its transaction fails (the table stays as it was; boot
 *     continues); `serve` may crash at any step; step 9 (concern 1) converges the pinned steering
 *     and the route `local` as one step.
 *   - One allocation, Active, its guest listening on declared port 1; port 2 is not declared. The
 *     owner's bring-up (bind, listen, steer) is one step (steer failures: concern 2).
 *   - Recoveries: IpRules (damage = the table absent) and FlowListeners (a listener-task exit,
 *     recurring at any time), with named holders as in concern 3.
 *
 * OFF names design rules switched off (hazard/shared_table_hazards.qnt):
 *   steeringInNft  D8a-LOOKUP: the steering decision lives in the shared table (a failed
 *                  convergence can drop it)
 *   unpinned       D8a-LOOKUP / ADR-0159: the steering is process-owned (gone while serve is down)
 *   nonLoopback    D8a-LOOKUP: the program also assigns non-loopback ingress
 *   anyRestore     D8a-HOLD: any recovery's restore reopens forwarding and ends every hold
 */
module shared_table {
  import prefix_landing.* from "./prefix_landing"

  const OFF: Set[str]
  const CFG: str   // "ok" | "live"

  def bug(r: str): bool = OFF.contains(r)
  val PORTS: Set[int] = Set(1, 2)
  val WANTED: Set[int] = Set(1)
  val COMPONENTS: Set[str] = Set("IpRules", "FlowListeners")
  val MAXATT: int = 2
  val ALLOW_CRASH: bool = CFG == "ok"
  val REPAIR_FAILS: int = if (CFG == "ok") 1 else -1

  type Ghost = { absentUp: bool, bypassed: bool, reopenEarly: bool }
  val g0: Ghost = { absentUp: false, bypassed: false, reopenEarly: false }

  var up: bool
  var phase: str            // start | rules | steer | open
  var table: str            // absent | complete  (single-transaction writes only, U-QP-1)
  var link: str             // none | pinned  (the pinned steering link running this boot's program)
  var route: str            // absent | local
  var lis: Set[int]
  var entries: Set[int]
  var holders: Set[str]
  var rec: str -> str       // idle | held | repaired
  var att: str -> int
  var flDmg: bool           // FlowListeners damaged (a listener-task exit)
  var repLeft: int
  var g: Ghost

  val open: bool = up and phase == "open"
  val complete: bool = table == "complete"
  def damaged(c: str): bool = if (c == "IpRules") table == "absent" else flDmg
  val net: Net = {
    marked: open, legC: up and phase != "start", divert: complete, reject: complete, route: route,
    steer: if (link == "pinned") "ours" else "none", entries: entries, lis: lis,
    assignRemote: bug("nonLoopback"),
  }
  def landsAt(c: str, p: int): str = lands(net, c, p)
  // the leg-C bypass: an unmarked host-local client reaches a serving intake directly
  val bypassed: bool = PORTS.exists(p => landsAt("unmarked", p) == "intake")
  def serving(p: int): bool = open and holders == Set() and WANTED.contains(p)
  val recIdle: str -> str = COMPONENTS.mapBy(c => "idle")
  val att0: str -> int = COMPONENTS.mapBy(c => 0)
  def dec(x: int): int = if (x < 0) x else x - 1

  action keepTable = all { table' = table }
  action keepRec = all { holders' = holders, rec' = rec, att' = att }
  action keepNode = all { link' = link, route' = route }
  // process-scoped state lost at a serve exit
  action exitProcess = all {
    lis' = Set(), entries' = Set(), holders' = Set(), rec' = recIdle, att' = att0, flDmg' = false,
    link' = if (bug("unpinned")) "none" else link, route' = route,
  }

  action init = all {
    up' = true, phase' = "start", table' = "absent", link' = "none", route' = "absent",
    lis' = Set(), entries' = Set(), holders' = Set(), rec' = recIdle, att' = att0, flDmg' = false,
    repLeft' = REPAIR_FAILS, g' = g0,
  }

  // ------------------------------------------------------------------ boot
  action BootStep = all {
    up, phase != "open",
    if (phase == "start") all { phase' = "rules", keepTable, keepNode }
    // step 8: leg-C, then the table's single transaction (succeeds: the table is complete)
    else if (phase == "rules") all { phase' = "steer", table' = "complete", keepNode }
    // step 9: this boot's program loaded and swapped into the pinned link (concern 1); then the route `local`
    else all { phase' = "open", keepTable, link' = "pinned", route' = "local" },
    up' = up, lis' = lis, entries' = entries, keepRec, flDmg' = flDmg, repLeft' = repLeft, g' = g,
  }
  // step 8's transaction fails (the mTLS worker): the table stays as it was — absent on a node that
  // never had one, complete after an earlier boot — and boot keeps going
  action RulesFail = all { up, phase == "rules",
    phase' = "steer", keepTable, keepNode,
    up' = up, lis' = lis, entries' = entries, keepRec, flDmg' = flDmg, repLeft' = repLeft,
    g' = { ...g, absentUp: g.absentUp or table == "absent" } }
  // serve dies (crash, graceful shutdown) at any step: a transaction in progress leaves the table as
  // it was
  action Crash = all { ALLOW_CRASH, up,
    up' = false, phase' = "start", exitProcess, keepTable, repLeft' = repLeft, g' = g }
  action Restart = all { not(up),
    up' = true, phase' = "start", keepTable, keepNode, lis' = lis, entries' = entries, keepRec,
    flDmg' = flDmg, repLeft' = repLeft, g' = g }

  // ------------------------------------------------------------------ runtime faults (Overdrive's own)
  // the mTLS worker's replacement transaction of the shared table fails: the table stays as it was
  // (a stutter in the design; no Overdrive write leaves it absent while serving). steeringInNft: the
  // steering held in the table is dropped by the replacement transaction
  action MtlsReplaceFail = {
    nondet dropSteer = Set(false, bug("steeringInNft")).oneOf()
    all { open,
      keepTable, link' = if (dropSteer) "none" else link, route' = route,
      up' = up, phase' = phase, lis' = lis, entries' = entries, keepRec, flDmg' = flDmg,
      repLeft' = repLeft, g' = g }
  }
  action ListenerTaskExit = all { open, not(flDmg),
    flDmg' = true, up' = up, phase' = phase, keepTable, keepNode, lis' = lis, entries' = entries,
    keepRec, repLeft' = repLeft, g' = g }

  // ------------------------------------------------------------------ owner
  // bring-up of a wanted port: bind, listen, steer (Ok)
  action BringUp(p: int): bool = all { serving(p), not(lis.contains(p)),
    lis' = lis.union(Set(p)), entries' = entries.union(Set(p)),
    up' = up, phase' = phase, keepTable, keepNode, keepRec, flDmg' = flDmg, repLeft' = repLeft,
    g' = { ...g, bypassed: g.bypassed or table == "absent" } }

  // ------------------------------------------------------------------ recoveries (D8a-HOLD)
  action common = all { up' = up, phase' = phase, repLeft' = repLeft }
  action RecQuiesce(c: str): bool = all {
    open, rec.get(c) == "idle", damaged(c), not(holders.contains(c)),
    holders' = holders.union(Set(c)), rec' = rec.set(c, "held"), att' = att,
    lis' = Set(), entries' = Set(), keepTable, keepNode, flDmg' = flDmg, common, g' = g,
  }
  // the repair converges component c only: IpRules → the table complete; FlowListeners → the task
  action RecRepair(c: str): bool = all {
    open, rec.get(c) == "held", att.get(c) < MAXATT,
    table' = (if (c == "IpRules") "complete" else table),
    flDmg' = (flDmg and c != "FlowListeners"),
    rec' = rec.set(c, "repaired"), holders' = holders, att' = att, lis' = lis, entries' = entries,
    keepNode, common, g' = g,
  }
  action RepFail(c: str): bool = all {
    open, rec.get(c) == "held", att.get(c) < MAXATT, repLeft != 0,
    att' = att.set(c, att.get(c) + 1), repLeft' = dec(repLeft), holders' = holders, rec' = rec,
    lis' = lis, entries' = entries, keepTable, keepNode, flDmg' = flDmg, up' = up, phase' = phase, g' = g,
  }
  action RecRedo(c: str): bool = all {
    open, rec.get(c) == "repaired", damaged(c),
    rec' = rec.set(c, "held"), att' = att.set(c, att.get(c) + 1), holders' = holders,
    lis' = lis, entries' = entries, keepTable, keepNode, flDmg' = flDmg, common, g' = g,
  }
  action RecRestore(c: str): bool = all {
    open, rec.get(c) == "repaired", not(damaged(c)),
    val others = holders.exclude(Set(c))
    val reopens = bug("anyRestore") or others == Set()
    all {
      holders' = if (bug("anyRestore")) Set() else others,
      rec' = if (bug("anyRestore")) recIdle else rec.set(c, "idle"),
      att' = if (bug("anyRestore")) att0 else att.set(c, 0),
      g' = { ...g, reopenEarly: g.reopenEarly or (reopens and COMPONENTS.exists(d => d != c and rec.get(d) == "held")) },
    },
    lis' = lis, entries' = entries, keepTable, keepNode, flDmg' = flDmg, common,
  }
  action FailStop(c: str): bool = all {
    open, rec.get(c) == "held", att.get(c) >= MAXATT,
    up' = false, phase' = "start", exitProcess, keepTable, repLeft' = repLeft, g' = g,
  }
  action recStep(c: str): bool = any { RecQuiesce(c), RecRepair(c), RecRedo(c), RecRestore(c), FailStop(c) }

  action step = {
    nondet c = COMPONENTS.oneOf()
    nondet p = PORTS.oneOf()
    any { BootStep, RulesFail, Crash, Restart, MtlsReplaceFail, ListenerTaskExit, BringUp(p),
          recStep(c), RepFail(c) }
  }

  // ------------------------------------------------------------------ invariants
  // (11, K-D3) with the shared table in either state — complete or absent — no connection to the
  // prefix reaches a host service
  val NoWildcardReached: bool = not(exposedOn(net, PORTS))
  // (K-L5) remote traffic never reaches an intake directly
  val RemoteNeverDirect: bool = PORTS.forall(p => landsAt("remote", p) != "intake")
  // (K-L4) while serve is down — an absent table persisting — nothing is delivered to a host socket
  val NothingDeliveredWhileDown: bool = not(up) implies not(deliveredOn(net, PORTS))
  // (D8a-HOLD) forwarding never reopens while another recovery holds without having repaired
  val NoReopenBeforeRepair: bool = not(g.reopenEarly)
  // (M-7) quiescence closes every intake listener
  val QuiescedClosesListeners: bool = holders != Set() implies (lis == Set() and entries == Set())

  val Safety: bool = and {
    NoWildcardReached, RemoteNeverDirect, NothingDeliveredWhileDown, NoReopenBeforeRepair,
    QuiescedClosesListeners,
  }

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // Each run ends in an `expect` of its scenario's outcome and checks `Safety` along the way. Runs
  // that need the default instance run on table_ok; the repair-forever run on table_live.
  // happy path: boot steps 1-9, the table complete, the steering pinned, the route local, the
  // owner brings up the wanted port
  run HappyPathBootAndBringUp = init
    .then(BootStep).expect(Safety and phase == "rules")
    .then(BootStep).expect(Safety and complete)
    .then(BootStep).expect(Safety and open and link == "pinned" and route == "local")
    .then(BringUp(1)).expect(Safety and lis == Set(1) and entries == Set(1) and not(g.bypassed))
  // K-D3 / U-QP-1: step 8's transaction fails on a node with no table; boot keeps the route and the
  // steering; no wildcard reachable with the table absent; the leg-C bypass window opens
  run TableAbsentAtBootKeepsRouteAndSteering = init
    .then(BootStep).then(RulesFail).expect(Safety and not(complete))
    .then(BootStep)
    .expect(Safety and open and not(complete) and route == "local" and link == "pinned")
    .then(BringUp(1)).expect(Safety and bypassed and g.bypassed)
  // D8a-HOLD: the IpRules recovery quiesces (closing every intake listener, M-7), repairs the table
  // in one transaction and restores; the owner brings the port up again behind the complete table
  run IpRulesRecoveryRepairsAbsentTable = init
    .then(BootStep).then(RulesFail).then(BootStep).then(BringUp(1))
    .then(RecQuiesce("IpRules")).expect(Safety and lis == Set() and entries == Set() and holders == Set("IpRules"))
    .then(RecRepair("IpRules")).expect(Safety and complete)
    .then(RecRestore("IpRules")).expect(Safety and holders == Set() and rec.get("IpRules") == "idle")
    .then(BringUp(1)).expect(Safety and lis == Set(1) and not(bypassed))
  // K-L4: the absent table persists while serve is down (crash after boot) and the route stays; nothing
  // is delivered to a host socket; the restart's boot step 8 converges the table
  run AbsentTablePersistsWhileDownThenBootConverges = init
    .then(BootStep).then(RulesFail).then(BootStep)
    .then(Crash).expect(Safety and not(up) and not(complete) and route == "local" and link == "pinned" and not(deliveredOn(net, PORTS)))
    .then(Restart).then(BootStep).then(BootStep).expect(Safety and complete and phase == "steer")
    .then(BootStep).expect(Safety and open and complete and not(bypassed))
  // crash at each boot step: the table stays as it was (a transaction in progress leaves it so),
  // restart re-runs boot from step 1
  run CrashAtEachBootStepThenRestart = init
    .then(Crash).expect(Safety and not(up) and not(complete))
    .then(Restart).then(BootStep)
    .then(Crash).expect(Safety and not(up) and not(complete) and phase == "start")
    .then(Restart).then(BootStep).then(BootStep)
    .then(Crash).expect(Safety and not(up) and complete and link == "none")
    .then(Restart).then(BootStep).then(BootStep).then(BootStep)
    .expect(Safety and open and complete and link == "pinned" and route == "local")
    .then(Crash).expect(Safety and not(up) and complete and link == "pinned" and not(deliveredOn(net, PORTS)))
    .then(Restart).then(BootStep).then(BootStep).then(BootStep).expect(Safety and open)
  // a runtime replacement transaction fails: the table stays as it was and the steering is unchanged
  run RuntimeReplacementFailureLeavesTableAsWas = init
    .then(BootStep).then(BootStep).then(BootStep).then(BringUp(1))
    .then(MtlsReplaceFail).expect(Safety and complete and link == "pinned" and lis == Set(1))
  // FlowListeners recovery: a listener-task exit, quiesce closes the intake listeners, repair, restore
  run FlowListenersRecoveryAfterTaskExit = init
    .then(BootStep).then(BootStep).then(BootStep).then(BringUp(1))
    .then(ListenerTaskExit).expect(Safety and flDmg)
    .then(RecQuiesce("FlowListeners")).expect(Safety and lis == Set() and holders == Set("FlowListeners"))
    .then(RecRepair("FlowListeners")).expect(Safety and not(flDmg))
    .then(RecRestore("FlowListeners")).expect(Safety and holders == Set())
    .then(BringUp(1)).expect(Safety and lis == Set(1))
  // D8a-HOLD: two recoveries hold at once; restoring one does not reopen forwarding while the other
  // has not repaired
  run SecondHolderKeepsForwardingClosed = init
    .then(BootStep).then(RulesFail).then(BootStep).then(ListenerTaskExit)
    .then(RecQuiesce("IpRules")).then(RecQuiesce("FlowListeners"))
    .expect(Safety and holders == Set("IpRules", "FlowListeners"))
    .then(RecRepair("IpRules")).then(RecRestore("IpRules"))
    .expect(Safety and holders == Set("FlowListeners") and not(serving(1)) and not(g.reopenEarly))
    .then(RecRepair("FlowListeners")).then(RecRestore("FlowListeners"))
    .expect(Safety and holders == Set() and serving(1))
  // one repair failure (table_ok): the recovery retries under its hold and completes
  run RepairFailureRetriesUnderHold = init
    .then(BootStep).then(RulesFail).then(BootStep)
    .then(RecQuiesce("IpRules")).then(RepFail("IpRules"))
    .expect(Safety and att.get("IpRules") == 1 and holders == Set("IpRules") and not(complete))
    .then(RecRepair("IpRules")).then(RecRestore("IpRules")).expect(Safety and complete and holders == Set())
  // a repair that fails forever (table_live): after MAXATT attempts the recovery fail-stops serve; the
  // hold is released with the process and the next boot converges the table
  run RepairFailsForeverFailStopsThenBootConverges = init
    .then(BootStep).then(RulesFail).then(BootStep)
    .then(RecQuiesce("IpRules")).then(RepFail("IpRules")).then(RepFail("IpRules"))
    .expect(Safety and att.get("IpRules") == MAXATT)
    .then(FailStop("IpRules")).expect(Safety and not(up) and holders == Set() and not(complete))
    .then(Restart).then(BootStep).then(BootStep).then(BootStep).expect(Safety and open and complete)

  // ------------------------------------------------------------------ witnesses (expected violated)
  // the leg-C bypass window exists (documented: ADR-0124 bound, ended by the IpRules recovery)
  val WitnessNoBypass: bool = not(bypassed)
  val WitnessNoAbsentWhileUp: bool = not(open and not(complete))
  val WitnessNoAbsentWhileDown: bool = not(not(up) and not(complete) and route == "local")

  // ------------------------------------------------------------------ liveness (TLC)
  val recVars = (rec, holders, table, flDmg)
  // fairness of the IpRules recovery's own steps and of boot only (none for the other recovery, the
  // owner, faults or restart)
  temporal IpRulesFair: bool = and {
    weakFair(RecQuiesce("IpRules"), recVars), weakFair(RecRepair("IpRules"), recVars),
    weakFair(RecRedo("IpRules"), recVars), weakFair(RecRestore("IpRules"), recVars),
    weakFair(FailStop("IpRules"), (recVars, up)), weakFair(BootStep, (up, phase)),
  }
  // (D8a-HOLD, concern 4) after the table is left absent, the IpRules recovery's repair completes
  // under its hold — the table is converged complete again — or serve stops; the repair is never
  // pre-empted forever. (The leg-C bypass itself ends at that recovery's first quiesce, which closes
  // every intake listener, so a property about the bypass ending holds trivially: not checked.)
  temporal TableRepaired = IpRulesFair.implies(always(not(complete).implies(eventually(complete or not(up)))))
}

// ------------------------------------------------------------------ design instances
// crash at any step, the mTLS worker's transaction failing at boot step 8 and at runtime (unbounded;
// a failed replacement leaves the table as it was), listener-task exits (unbounded), one repair failure
module table_ok { import shared_table(OFF = Set(), CFG = "ok").* from "./shared_table" }
// no crash; a repair may fail forever
module table_live { import shared_table(OFF = Set(), CFG = "live").* from "./shared_table" }
