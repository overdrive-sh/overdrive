// -*- mode: Bluespec; -*-
/**
 * Guest-flow owner (netns-density-295) — steering concern 3: named quiescence holders and the
 * per-component recoveries of the runtime supervisor (D8a-HOLD, ADR-0124 bounded recovery, UP-7,
 * UP-12).
 *
 * Design artifact and DISTILL oracle. Models the design text, not an implementation.
 * Source of truth: feature-delta.md § "vsock Attachment Replacement DESIGN" — § Owner and provisioner
 * (`quiesce_forwarding`, `restore_forwarding`, `QuiescenceHolder`, `QuiescenceHold`,
 * `converge_shared` as a runtime repair, `audit_shared`), Gate G-V5, § Revision 9 decisions item 7
 * (UP-7, UP-12 kept); ADR-0169 (D8a-HOLD), ADR-0124 (1 s audit, bounded attempts, fail-stop).
 *
 * The rules modelled:
 *   - quiesce_forwarding(Recovery(c)) adds c to the holder set; the first holder latches admission
 *     and closes every intake listener (each close removes its entry). restore_forwarding(hold)
 *     removes only that holder and reopens (latch cleared, listeners brought back) only when no
 *     holder remains.
 *   - each recovery: the audit finds c damaged → quiesce under its own holder → repair c
 *     (`converge_shared` for c only: never the steering program or the route) → audit → restore with
 *     its hold. A failed repair or a failed post-repair audit is a failed attempt (UP-7); the budget
 *     is per component recovery; when spent, serve fail-stops. Node-global objects (the route,
 *     which has the single form `local`, and the pinned steering boot converged) stay as they are
 *     (UP-12).
 *
 * Abstraction:
 *   - Components IpRules (damage = the shared nft table left partial by the mTLS worker; concern 4
 *     models the table itself) and FlowListeners (a listener-task exit). Damage recurs at any time
 *     (unbounded): an Overdrive component can fail again. The audit's finding and the recovery's
 *     quiesce are one step.
 *   - ONE wanted intake listener (`lis`) stands for the owner's intake listeners (concern 2).
 *   - Crash / restart: process-scoped state is lost; the partial table persists and boot step 8
 *     converges it again (possibly partially).
 *
 * OFF names design rules switched off (hazard/quiescence_hazards.qnt):
 *   anyRestore         D8a-HOLD: one latch — any recovery's restore reopens; every hold ends
 *   repairNoHold       D8a-HOLD: a recovery repairs without holding quiescence
 *   redoNoCount        UP-7: a failed post-repair audit is not a failed attempt
 *   otherRepairsPrefix the repair also performs a runtime write of the route or the steering (any
 *                      such write; a failed repair leaves it interrupted)
 */
module quiescence {
  const OFF: Set[str]
  const CFG: str   // "ok" | "live"

  def bug(r: str): bool = OFF.contains(r)
  val COMPONENTS: Set[str] = Set("IpRules", "FlowListeners")
  val MAXATT: int = 2
  val ALLOW_CRASH: bool = CFG == "ok"
  val REPAIR_FAILS: int = if (CFG == "ok") 1 else -1   // -1: a repair may fail forever

  type Ghost = {
    reopenEarly: bool,   // forwarding reopened while another recovery held without having repaired
    repairNoHold: bool,  // a repair ran without its recovery's hold
    prefixWritten: bool, // a recovery's repair wrote the steering program or the route
    overlapped: bool,    // two holders held at once (witness)
    stillQuiesced: bool, // a restore returned StillQuiesced (witness)
    failStopped: bool,   // a recovery fail-stopped (witness)
  }
  val g0: Ghost = { reopenEarly: false, repairNoHold: false, prefixWritten: false, overlapped: false,
                    stillQuiesced: false, failStopped: false }

  var up: bool
  var stop: str              // none | crash | failstop
  var holders: Set[str]
  var latch: bool            // admission quiesced
  var rec: str -> str        // per component recovery: idle | held | repaired
  var dmg: str -> bool       // per component damage
  var att: str -> int        // failed attempts of the current episode (process-scoped)
  var repLeft: int
  var lis: bool              // the wanted intake listener is open (and steered)
  var prefix: str            // booted | written: the route (`local`) and the pinned steering are as
                             // boot converged them, or a runtime step wrote one of them
  var g: Ghost

  val recIdle: str -> str = COMPONENTS.mapBy(c => "idle")
  val att0: str -> int = COMPONENTS.mapBy(c => 0)
  def dec(x: int): int = if (x < 0) x else x - 1

  action keepState = all { holders' = holders, latch' = latch, rec' = rec, att' = att, lis' = lis }
  action keepEnv = all { up' = up, stop' = stop, dmg' = dmg, repLeft' = repLeft, prefix' = prefix }

  action init = all {
    up' = true, stop' = "none", holders' = Set(), latch' = false, rec' = recIdle,
    dmg' = COMPONENTS.mapBy(c => false), att' = att0, repLeft' = REPAIR_FAILS, lis' = true,
    prefix' = "booted", g' = g0,
  }

  // ------------------------------------------------------------------ environment
  // Overdrive's own failure damages c (the mTLS worker leaves the table partial; a listener task exits)
  action Damage(c: str): bool = all { up, not(dmg.get(c)),
    dmg' = dmg.set(c, true), up' = up, stop' = stop, repLeft' = repLeft, prefix' = prefix, keepState, g' = g }
  action Crash = all { ALLOW_CRASH, up,
    up' = false, stop' = "crash", holders' = Set(), latch' = false, rec' = recIdle, att' = att0,
    lis' = false, dmg' = dmg.set("FlowListeners", false), repLeft' = repLeft, prefix' = prefix, g' = g }
  // boot: step 8 converges the table again (it may be left partial again); the route and the pinned
  // steering are converged at step 9 (concern 1) — as booted here
  action Restart = {
    nondet partial = Set(false, dmg.get("IpRules")).oneOf()
    all { not(up),
      up' = true, stop' = "none", dmg' = dmg.set("IpRules", partial), holders' = holders,
      latch' = latch, rec' = rec, att' = att, lis' = lis, repLeft' = repLeft, prefix' = "booted", g' = g }
  }

  // ------------------------------------------------------------------ owner
  // restore / boot: the owner brings the wanted listener back (bind, listen, steer)
  action BringUp = all { up, not(latch), not(lis),
    lis' = true, holders' = holders, latch' = latch, rec' = rec, att' = att, keepEnv, g' = g }

  // ------------------------------------------------------------------ recovery of component c
  // the audit finds c damaged → quiesce_forwarding(Recovery(c))
  action RecQuiesce(c: str): bool = all {
    up, rec.get(c) == "idle", dmg.get(c), not(bug("repairNoHold")), not(holders.contains(c)),
    holders' = holders.union(Set(c)), latch' = true, rec' = rec.set(c, "held"), att' = att,
    lis' = if (holders == Set()) false else lis, keepEnv,
    g' = { ...g, overlapped: g.overlapped or holders != Set() },
  }
  // the repair: converge_shared for c only (never the steering program or the route)
  action RecRepair(c: str): bool = all {
    up, rec.get(c) == "held", holders.contains(c), att.get(c) < MAXATT,
    dmg' = dmg.set(c, false), rec' = rec.set(c, "repaired"),
    prefix' = if (bug("otherRepairsPrefix")) "written" else prefix, holders' = holders, latch' = latch, att' = att, lis' = lis,
    up' = up, stop' = stop, repLeft' = repLeft,
    g' = { ...g, prefixWritten: g.prefixWritten or bug("otherRepairsPrefix") },
  }
  // the repair returns Err: a failed attempt (otherRepairsPrefix: its runtime write of the route or the
  // steering is left interrupted)
  action RepFail(c: str): bool = all {
    up, rec.get(c) == "held", holders.contains(c), att.get(c) < MAXATT, repLeft != 0,
    att' = att.set(c, att.get(c) + 1), repLeft' = dec(repLeft),
    prefix' = if (bug("otherRepairsPrefix")) "written" else prefix,
    holders' = holders, latch' = latch, rec' = rec, lis' = lis, up' = up, stop' = stop, dmg' = dmg,
    g' = { ...g, prefixWritten: g.prefixWritten or bug("otherRepairsPrefix") },
  }
  // the post-repair audit finds c damaged again: a failed attempt (UP-7); repair again under the hold
  action RecRedo(c: str): bool = all {
    up, rec.get(c) == "repaired", dmg.get(c),
    rec' = rec.set(c, "held"),
    att' = if (bug("redoNoCount")) att else att.set(c, att.get(c) + 1),
    holders' = holders, latch' = latch, lis' = lis, keepEnv, g' = g,
  }
  // clean audit → restore_forwarding(hold): removes only c; reopens only if no holder remains.
  // anyRestore: one latch — the restore reopens and every other recovery loses its hold.
  action RecRestore(c: str): bool = all {
    up, rec.get(c) == "repaired", not(dmg.get(c)),
    val others = holders.exclude(Set(c))
    val reopens = bug("anyRestore") or others == Set()
    val early = reopens and COMPONENTS.exists(d => d != c and rec.get(d) == "held")
    all {
      holders' = if (bug("anyRestore")) Set() else others,
      latch' = not(reopens),
      rec' = if (bug("anyRestore")) recIdle else rec.set(c, "idle"),
      att' = if (bug("anyRestore")) att0 else att.set(c, 0),
      g' = { ...g, reopenEarly: g.reopenEarly or early, stillQuiesced: g.stillQuiesced or not(reopens) },
    },
    lis' = lis, keepEnv,
  }
  // ADR-0124: the attempt budget is spent — serve fail-stops; node-global objects stay as they are
  action FailStop(c: str): bool = all {
    up, rec.get(c) == "held", att.get(c) >= MAXATT,
    up' = false, stop' = "failstop", holders' = Set(), latch' = false, rec' = recIdle, att' = att0,
    lis' = false, dmg' = dmg.set("FlowListeners", false), repLeft' = repLeft, prefix' = prefix,
    g' = { ...g, failStopped: true },
  }
  // repairNoHold: the recovery repairs without quiescing
  action RecRepairNoHold(c: str): bool = all {
    bug("repairNoHold"), up, rec.get(c) == "idle", dmg.get(c),
    dmg' = dmg.set(c, false), up' = up, stop' = stop, repLeft' = repLeft, prefix' = prefix, keepState,
    g' = { ...g, repairNoHold: true },
  }

  action recStep(c: str): bool = any {
    RecQuiesce(c), RecRepair(c), RecRedo(c), RecRestore(c), FailStop(c), RecRepairNoHold(c),
  }
  action step = {
    nondet c = COMPONENTS.oneOf()
    any { recStep(c), RepFail(c), Damage(c), Crash, Restart, BringUp }
  }

  // ------------------------------------------------------------------ invariants
  // (10, D8a-HOLD) forwarding is quiesced exactly while some holder holds
  val LatchIffHolders: bool = latch == (holders != Set())
  // (10, D8a-HOLD) a recovery that holds is in the holder set: no restore ended another's hold
  val HeldIsHolder: bool = COMPONENTS.forall(c => rec.get(c) != "idle" implies holders.contains(c))
  // (10, D8a-HOLD) forwarding never reopens while another recovery holds without having repaired
  val NoReopenBeforeRepair: bool = not(g.reopenEarly)
  // (10, D8a-HOLD) no repair runs without that recovery's hold
  val NoRepairWithoutHold: bool = not(g.repairNoHold)
  // (10, M-7) quiescence closes every intake listener
  val QuiescedClosesListeners: bool = latch implies not(lis)
  // (6, G-V5) no recovery's repair writes the steering program or the route
  val RepairNeverWritesPrefix: bool = not(g.prefixWritten)
  // (UP-12, 6) another component's fail-stop leaves the route `local` and the pinned steering as boot
  // converged them: no runtime step wrote either
  val FailStopLeavesPrefixAsBooted: bool = (not(up) and stop == "failstop") implies prefix == "booted"

  val Safety: bool = and {
    LatchIffHolders, HeldIsHolder, NoReopenBeforeRepair, NoRepairWithoutHold, QuiescedClosesListeners,
    RepairNeverWritesPrefix, FailStopLeavesPrefixAsBooted,
  }

  // ------------------------------------------------------------------ witnesses (expected violated)
  val WitnessNoOverlap: bool = not(g.overlapped)
  val WitnessNoStillQuiesced: bool = not(g.stillQuiesced)
  val WitnessNoFailStop: bool = not(g.failStopped)

  // ------------------------------------------------------------------ liveness (TLC)
  // fairness of component c's own recovery steps only (no fairness for any other recovery, for
  // damage, crash, restart or the owner)
  temporal OwnFair(c: str): bool = and {
    weakFair(RecQuiesce(c), (rec, holders, dmg)), weakFair(RecRepair(c), (rec, holders, dmg)),
    weakFair(RecRedo(c), (rec, holders, dmg)), weakFair(RecRestore(c), (rec, holders, dmg)),
    weakFair(FailStop(c), (rec, holders, dmg, up)),
  }
  // (D8a-HOLD, ADR-0124, UP-7) a recovery episode ends — restored, or serve stopped — under the
  // recovery's own fairness, whatever the other recovery and recurring damage do
  temporal IpRulesEpisodeEnds =
    OwnFair("IpRules").implies(always((rec.get("IpRules") != "idle").implies(
      eventually(rec.get("IpRules") == "idle" or not(up)))))
  temporal FlowListenersEpisodeEnds =
    OwnFair("FlowListeners").implies(always((rec.get("FlowListeners") != "idle").implies(
      eventually(rec.get("FlowListeners") == "idle" or not(up)))))
  // (D8a-HOLD) damage of IpRules is repaired (or serve stops) under its own recovery's fairness only:
  // no other recovery can pre-empt the repair forever
  temporal IpRulesRepaired =
    OwnFair("IpRules").implies(always(dmg.get("IpRules").implies(
      eventually(not(dmg.get("IpRules")) or not(up)))))
}

// ------------------------------------------------------------------ design instances
// crash + restart (boot may leave the table partial again), one repair failure, recurring damage
module quiesce_ok { import quiescence(OFF = Set(), CFG = "ok").* from "./quiescence" }
// no crash; a repair may fail forever; recurring damage
module quiesce_live { import quiescence(OFF = Set(), CFG = "live").* from "./quiescence" }
