// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — steering concern 4: the shared nft table left partial by
 * Overdrive itself (K-D3): the mTLS worker's convergence stopping part-way (at boot step 8 or at
 * runtime) and a `serve` crash between its batches; the partial table persisting while `serve` is
 * down and converged again at boot step 8; the steering's independence from the table; the leg-C
 * bypass window; the table repair completing under the IpRules recovery's hold (D8a-HOLD).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" — § Composition (boot
 * steps 8, 9 and the ordering invariant: "The nft table is not a precondition of the route"),
 * § Earned Trust ("the shared nft table left partial at boot step 8 (boot keeps the route …)"),
 * Gate G-V5, § Required boundary scenarios R5-30, § Formal protocol model (concern 4, K-D3);
 * ADR-0171, ADR-0169, ADR-0152.
 *
 * Abstraction:
 *   - The table's constant rules in two parts: `divert` (prefix traffic to leg-C) and `reject`
 *     (output reject / prerouting drop of non-exempt prefix traffic). Each nft batch is atomic
 *     (K-D3); a partial table is any state other than both present.
 *   - Boot: steps 1–7 one step; step 8 (`start_shared_owner`: leg-C, then the rules) may stop
 *     part-way — boot continues with the table partial — or `serve` may crash between its batches;
 *     step 9 (concern 1) converges the pinned steering and the route `local` as one step.
 *   - One allocation, Active, its guest listening on declared port 1; port 2 is not declared. The
 *     owner's bring-up (bind, listen, steer) is one step (steer failures: concern 2).
 *   - Recoveries: IpRules (damage = the table partial) and FlowListeners (a listener-task exit,
 *     recurring at any time), with named holders as in concern 3.
 *
 * OFF names design rules switched off (hazard/shared_table_hazards.qnt):
 *   steeringInNft  D8a-LOOKUP: the steering decision lives in the shared table (a partial
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
  // the partial states of the table (each batch atomic, K-D3)
  val PARTIAL: Set[(bool, bool)] = Set((true, false), (false, true), (false, false))

  type Ghost = { partialUp: bool, crashPartial: bool, bypassed: bool, reopenEarly: bool }
  val g0: Ghost = { partialUp: false, crashPartial: false, bypassed: false, reopenEarly: false }

  var up: bool
  var phase: str            // start | rules | steer | open
  var divert: bool
  var reject: bool
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
  val complete: bool = divert and reject
  def damaged(c: str): bool = if (c == "IpRules") not(complete) else flDmg
  val net: Net = {
    marked: open, legC: up and phase != "start", divert: divert, reject: reject, route: route,
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

  action keepTable = all { divert' = divert, reject' = reject }
  action keepRec = all { holders' = holders, rec' = rec, att' = att }
  action keepNode = all { link' = link, route' = route }
  // process-scoped state lost at a serve exit
  action exitProcess = all {
    lis' = Set(), entries' = Set(), holders' = Set(), rec' = recIdle, att' = att0, flDmg' = false,
    link' = if (bug("unpinned")) "none" else link, route' = route,
  }

  action init = all {
    up' = true, phase' = "start", divert' = false, reject' = false, link' = "none", route' = "absent",
    lis' = Set(), entries' = Set(), holders' = Set(), rec' = recIdle, att' = att0, flDmg' = false,
    repLeft' = REPAIR_FAILS, g' = g0,
  }

  // ------------------------------------------------------------------ boot
  action BootStep = all {
    up, phase != "open",
    if (phase == "start") all { phase' = "rules", keepTable, keepNode }
    // step 8: leg-C, then the constant rules (converged complete)
    else if (phase == "rules") all { phase' = "steer", divert' = true, reject' = true, keepNode }
    // step 9: this boot's program loaded and swapped into the pinned link (concern 1); then the route `local`
    else all { phase' = "open", keepTable, link' = "pinned", route' = "local" },
    up' = up, lis' = lis, entries' = entries, keepRec, flDmg' = flDmg, repLeft' = repLeft, g' = g,
  }
  // step 8 stops part-way (the mTLS worker fails): boot keeps going with the table partial
  action RulesPartial = {
    nondet t = PARTIAL.oneOf()
    all { up, phase == "rules",
      phase' = "steer", divert' = t._1, reject' = t._2, keepNode,
      up' = up, lis' = lis, entries' = entries, keepRec, flDmg' = flDmg, repLeft' = repLeft,
      g' = { ...g, partialUp: true } }
  }
  // serve dies (crash, graceful shutdown); at step 8 it may die between two batches
  action Crash = {
    nondet t = PARTIAL.oneOf()
    all { ALLOW_CRASH, up,
      up' = false, phase' = "start", exitProcess, repLeft' = repLeft,
      divert' = if (phase == "rules") t._1 else divert, reject' = if (phase == "rules") t._2 else reject,
      g' = { ...g, crashPartial: g.crashPartial or (phase == "rules" and t != (divert, reject)) } }
  }
  action Restart = all { not(up),
    up' = true, phase' = "start", keepTable, keepNode, lis' = lis, entries' = entries, keepRec,
    flDmg' = flDmg, repLeft' = repLeft, g' = g }

  // ------------------------------------------------------------------ runtime faults (Overdrive's own)
  // the mTLS worker's convergence of the shared table stops part-way (steeringInNft: it can drop the
  // steering held in the table)
  action MtlsPartial = {
    nondet t = PARTIAL.oneOf()
    nondet dropSteer = Set(false, bug("steeringInNft")).oneOf()
    all { open,
      divert' = t._1, reject' = t._2, link' = if (dropSteer) "none" else link, route' = route,
      up' = up, phase' = phase, lis' = lis, entries' = entries, keepRec, flDmg' = flDmg,
      repLeft' = repLeft, g' = { ...g, partialUp: true } }
  }
  action ListenerTaskExit = all { open, not(flDmg),
    flDmg' = true, up' = up, phase' = phase, keepTable, keepNode, lis' = lis, entries' = entries,
    keepRec, repLeft' = repLeft, g' = g }

  // ------------------------------------------------------------------ owner
  // bring-up of a wanted port: bind, listen, steer (Ok)
  action BringUp(p: int): bool = all { serving(p), not(lis.contains(p)),
    lis' = lis.union(Set(p)), entries' = entries.union(Set(p)),
    up' = up, phase' = phase, keepTable, keepNode, keepRec, flDmg' = flDmg, repLeft' = repLeft,
    g' = { ...g, bypassed: g.bypassed or (not(divert) and not(reject)) } }

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
    divert' = (divert or c == "IpRules"), reject' = (reject or c == "IpRules"),
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
    any { BootStep, RulesPartial, Crash, Restart, MtlsPartial, ListenerTaskExit, BringUp(p),
          recStep(c), RepFail(c) }
  }

  // ------------------------------------------------------------------ invariants
  // (11, K-D3) with the shared table in any state — complete, partial or absent — no connection to the
  // prefix reaches a host service
  val NoWildcardReached: bool = not(exposedOn(net, PORTS))
  // (K-L5) remote traffic never reaches an intake directly
  val RemoteNeverDirect: bool = PORTS.forall(p => landsAt("remote", p) != "intake")
  // (K-L4) while serve is down — the partial table persisting — nothing is delivered to a host socket
  val NothingDeliveredWhileDown: bool = not(up) implies not(deliveredOn(net, PORTS))
  // (D8a-HOLD) forwarding never reopens while another recovery holds without having repaired
  val NoReopenBeforeRepair: bool = not(g.reopenEarly)
  // (M-7) quiescence closes every intake listener
  val QuiescedClosesListeners: bool = holders != Set() implies (lis == Set() and entries == Set())

  val Safety: bool = and {
    NoWildcardReached, RemoteNeverDirect, NothingDeliveredWhileDown, NoReopenBeforeRepair,
    QuiescedClosesListeners,
  }

  // ------------------------------------------------------------------ witnesses (expected violated)
  // the leg-C bypass window exists (documented: ADR-0124 bound, ended by the IpRules recovery)
  val WitnessNoBypass: bool = not(bypassed)
  val WitnessNoPartialWhileUp: bool = not(open and not(complete))
  val WitnessNoPartialWhileDown: bool = not(not(up) and not(complete) and route == "local")
  val WitnessNoCrashBetweenBatches: bool = not(g.crashPartial)

  // ------------------------------------------------------------------ liveness (TLC)
  val recVars = (rec, holders, divert, reject, flDmg)
  // fairness of the IpRules recovery's own steps and of boot only (none for the other recovery, the
  // owner, faults or restart)
  temporal IpRulesFair: bool = and {
    weakFair(RecQuiesce("IpRules"), recVars), weakFair(RecRepair("IpRules"), recVars),
    weakFair(RecRedo("IpRules"), recVars), weakFair(RecRestore("IpRules"), recVars),
    weakFair(FailStop("IpRules"), (recVars, up)), weakFair(BootStep, (up, phase)),
  }
  // (D8a-HOLD, concern 4) after the table is left partial, the IpRules recovery's repair completes
  // under its hold — the table is converged complete again — or serve stops; the repair is never
  // pre-empted forever. (The leg-C bypass itself ends at that recovery's first quiesce, which closes
  // every intake listener, so a property about the bypass ending holds trivially: not checked.)
  temporal TableRepaired = IpRulesFair.implies(always(not(complete).implies(eventually(complete or not(up)))))
}

// ------------------------------------------------------------------ design instances
// crash at any step (between step-8 batches included), the mTLS worker failing part-way at boot and at
// runtime (unbounded), listener-task exits (unbounded), one repair failure
module table_ok { import shared_table(OFF = Set(), CFG = "ok").* from "./shared_table" }
// no crash; a repair may fail forever
module table_live { import shared_table(OFF = Set(), CFG = "live").* from "./shared_table" }
