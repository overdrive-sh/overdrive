// -*- mode: Bluespec; -*-
/**
 * The Service deploy stream is bounded by a progress deadline derived from the startup deadline
 * (service-allocation-readiness-and-ending D10, ADR-0262). Stream termination protocol: a timer racing the lifecycle terminals.
 *
 * Time is the injected Clock in 10 s ticks; `now` advances by Advance(t). FLOOR = 9 (the 90 s streaming cap), MARGIN = 3
 * (PROGRESS_DEADLINE_MARGIN, 30 s). The workload's startup deadline `sd` (interval x max_attempts) is chosen once by Deploy:
 * 1 (10 s, a short deadline) or 12 (120 s, above the floor). deadline(sd) = max(FLOOR, sd + MARGIN).
 *
 * Stream slots SLOTS (a module parameter): one in `pd_ok` and the hazard (the one remaining checked invariant needs one stream),
 * two in `pd_conf` (a redeploy of an unchanged spec opens a second stream with its own clock). Per stream (record Strm):
 *   st      unopened | open | closed;  kind: stable | giveup | timeout | operator_stop (when closed)
 *   armed   the tick at which the stream armed its timer (stream open);  dl: the deadline it armed
 *   last    the latest attempt-ending observation this stream saw after it opened: none | startup_stop | crash
 *   closes  how many times the stream closed;  verdict: the closure carries the restart authority's verdict (giveup only)
 * Environment / lifecycle: Advance, Open, AttemptEnd(obs) (an attempt-ending lifecycle event; a restartable probe stop
 * projects to no terminal, D7 item 5), StableHappens (the workload becomes Stable at now <= sd), GiveUp (the restart
 * authority's give-up reaches the bus), OperatorStop, Fire (the timer). The stream writes no row.
 *
 * Streams open only before the workload has a terminal: an Open after Stable or a give-up is the redeploy of a settled
 * workload, which is D11's protocol (stream_snapshot.qnt), not this one.
 *
 * OFF names the rules switched off (hazards): fixedCap (D10: the fixed 90 s cap regardless of the startup deadline).
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   DL-1  (dropped) Stable may happen at any tick: the model does not assume it happens by the startup deadline (a late
 *         Pass after a failed stop reaches Stable, D8 item 3). The invariant only constrains a Stable at or before sd.
 *   DL-2  The restart authority's give-up reaches the bus no earlier than 6 x sd + 1 tick (RESTART_BACKOFF_CEILING + 1
 *         attempts of one startup deadline each, plus backoffs). Discharged by: V15.
 *   DL-3  The stream receives every attempt-ending lifecycle event of its workload published after it opened (a lagged
 *         subscriber misses events: bounded, G-7). Discharged by: V21.
 *   DL-4  The timer is the injected Clock (SimClock under test), armed at stream open. Discharged by: S-35.
 *   DL-6  Boot plus placement plus one evaluation is at most the margin (PROGRESS_DEADLINE_MARGIN = VM_BOOT_DEADLINE = 30 s):
 *         the stream clock starts at open and the startup deadline at the Running row, so the progress deadline exceeds the
 *         startup window only under DL-6. HealthySlowStartNeverTimesOut is scoped by DL-6; where DL-6 fails the stream's
 *         timeout is honest and retracts nothing (exposure E-4, accepted). The model has no boot time: it starts the stream
 *         clock and the startup clock together. Discharged by: V30 (settled by source), S-35 (f).
 *   DL-5  A CLI disconnect ends the stream and persists nothing; a redeploy opens a new stream with a new clock.
 *         Discharged by: S-35 (d).
 *
 * D-UP-1: `conf_step`. Conformance projection: the first terminal line yielded by build_service_stream under SimClock
 * (Failed { Timeout }, Stable, a give-up Failed, Stopped), the value returned by progress_deadline, and the last_attempt
 * field of that Failed line; Advance / the lifecycle events are the driver's SimClock advance and bus sends.
 */
module progress_deadline_conf {
  type ConfStep =
    | init
    | Deploy({ sd: int })
    | Open({ s: int })
    | Advance({ t: int })
    | AttemptEnd({ obs: str })
    | StableHappens
    | GiveUp
    | OperatorStop
    | Fire({ s: int })
    | Idle
}

module progress_deadline {
  import progress_deadline_conf.ConfStep from "./progress_deadline"
  const OFF: Set[str]
  const REC: bool
  const SLOTS: Set[int]

  val FLOOR: int = 9
  val MARGIN: int = 3
  val MAXT: int = 16
  val SIDS: Set[int] = SLOTS
  val BUG_CAP: bool = OFF.contains("fixedCap")

  type Strm = { st: str, kind: str, armed: int, dl: int, last: str, closes: int, verdict: bool }
  type World = { sd: int, now: int, stableAt: int, gaveUp: bool, opStopped: bool, authors: Set[str], deployed: bool }

  var w: World
  var sm: int -> Strm
  var conf_step: ConfStep

  pure def max2(a: int, b: int): int = if (a > b) a else b
  def deadlineFor(sd: int): int = if (BUG_CAP) FLOOR else max2(FLOOR, sd + MARGIN)
  val blankS: Strm = { st: "unopened", kind: "", armed: 0, dl: 0, last: "none", closes: 0, verdict: false }

  action init = all {
    w' = { sd: 0, now: 0, stableAt: -1, gaveUp: false, opStopped: false, authors: Set(), deployed: false },
    sm' = SIDS.mapBy(s => blankS),
    conf_step' = variant("init", ()),
  }

  // the workload's startup probe fixes its startup deadline sd
  action Deploy(sd: int): bool = all {
    not(w.deployed),
    w' = { ...w, sd: sd, deployed: true },
    sm' = sm,
    conf_step' = if (REC) variant("Deploy", { sd: sd }) else conf_step,
  }

  // a deploy stream opens (after Accepted): arms its timer with the derived deadline; only before the workload is settled
  action Open(s: int): bool = all {
    w.deployed, sm.get(s).st == "unopened",
    w.stableAt < 0, not(w.gaveUp), not(w.opStopped),
    sm' = sm.set(s, { ...sm.get(s), st: "open", armed: w.now, dl: deadlineFor(w.sd) }),
    w' = w,
    conf_step' = if (REC) variant("Open", { s: s }) else conf_step,
  }

  action Advance(t: int): bool = all {
    t > w.now, t <= MAXT,
    w' = { ...w, now: t },
    sm' = sm,
    conf_step' = if (REC) variant("Advance", { t: t }) else conf_step,
  }

  def closeOpen(kind: str, verdict: bool): int -> Strm =
    SIDS.mapBy(s => {
      val x = sm.get(s)
      if (x.st == "open") { ...x, st: "closed", kind: kind, closes: x.closes + 1, verdict: verdict } else x
    })

  // an attempt-ending lifecycle event (a startup stop, a crash): kept as the latest observation by each open stream; it
  // closes nothing (a restartable stop projects to no terminal, D7 item 5)
  action AttemptEnd(obs: str): bool = all {
    w.deployed, w.stableAt < 0, not(w.gaveUp),
    sm' = SIDS.mapBy(s => if (sm.get(s).st == "open") { ...sm.get(s), last: obs } else sm.get(s)),
    w' = w,
    conf_step' = if (REC) variant("AttemptEnd", { obs: obs }) else conf_step,
  }

  // ServiceLifecycle commits Stable, the shim publishes it: every open stream closes as stable. Stable can happen at any
  // tick, whether or not a stream is still open.
  action StableHappens = all {
    w.deployed, w.stableAt < 0, not(w.gaveUp), not(w.opStopped),
    w' = { ...w, stableAt: w.now },
    sm' = closeOpen("stable", false),
    conf_step' = if (REC) variant("StableHappens", ()) else conf_step,
  }

  // the restart authority's give-up (the only authored failure verdict): reachable no earlier than 6 x sd + 1 (DL-2)
  action GiveUp = all {
    w.deployed, w.stableAt < 0, not(w.gaveUp), w.now >= 6 * w.sd + 1,
    w' = { ...w, gaveUp: true, authors: w.authors.union(Set("workload_lifecycle")) },
    sm' = closeOpen("giveup", true),
    conf_step' = if (REC) variant("GiveUp", ()) else conf_step,
  }

  action OperatorStop = all {
    w.deployed, w.stableAt < 0, not(w.gaveUp), not(w.opStopped),
    w' = { ...w, opStopped: true },
    sm' = closeOpen("operator_stop", false),
    conf_step' = if (REC) variant("OperatorStop", ()) else conf_step,
  }

  // the timer: fires when the stream's own deadline has elapsed; it closes with the labelled last observation and authors
  // nothing
  def canFire(s: int): bool = sm.get(s).st == "open" and w.now - sm.get(s).armed >= sm.get(s).dl
  action Fire(s: int): bool = all {
    canFire(s),
    sm' = sm.set(s, { ...sm.get(s), st: "closed", kind: "timeout", closes: sm.get(s).closes + 1, verdict: false }),
    w' = w,
    conf_step' = if (REC) variant("Fire", { s: s }) else conf_step,
  }

  action Idle = all { w' = w, sm' = sm, conf_step' = if (REC) variant("Idle", ()) else conf_step }

  action step = {
    nondet s = SIDS.oneOf()
    nondet t = 1.to(MAXT).oneOf()
    nondet sd = Set(1, 12).oneOf()
    nondet obs = Set("startup_stop", "crash").oneOf()
    any { Deploy(sd), Open(s), Advance(t), AttemptEnd(obs), StableHappens, GiveUp, OperatorStop, Fire(s), Idle }
  }

  // ------------------------------------------------------------------ invariants
  def opened(s: int): bool = sm.get(s).st != "unopened"
  // D10 HealthySlowStartNeverTimesOut, scoped by assumption DL-6 (the model starts the stream clock and the startup clock
  // together; with real boot time the progress deadline can fire first, exposure E-4): a workload that reaches Stable at or before its startup deadline never sees a stream
  // close as timeout
  val HealthySlowStartNeverTimesOut: bool =
    SIDS.forall(s => (sm.get(s).kind == "timeout") implies (w.stableAt < 0 or w.stableAt > w.sd))
  val Safety: bool = HealthySlowStartNeverTimesOut

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // startup deadline 120 s (12 ticks), floor 90 s (9 ticks): Stable at 100 s (tick 10) reaches Stable, not Timeout. The
  // stream's timer has not fired at the floor.
  run slow_start_within_startup_deadline_reaches_stable_not_timeout = init
    .then(Deploy(12)).then(Open(1)).then(Advance(9))
    .expect(not(canFire(1)) and sm.get(1).dl > w.sd and Safety)
    .then(Advance(10)).then(StableHappens)
    .expect(sm.get(1).st == "closed" and sm.get(1).kind == "stable" and w.stableAt == 10 and Safety)

  // never ready: an attempt is startup-stopped, then the deadline elapses: the stream closes as timeout carrying the last
  // attempt's observation as detail, and authors no verdict
  run never_ready_times_out_with_last_attempt_cause = init
    .then(Deploy(12)).then(Open(1)).then(Advance(12)).then(AttemptEnd("startup_stop"))
    .then(Advance(15)).then(Fire(1))
    .expect(sm.get(1).st == "closed" and sm.get(1).kind == "timeout" and sm.get(1).last == "startup_stop"
            and not(sm.get(1).verdict) and w.authors == Set() and Safety)

  // a short startup deadline: the give-up lands inside the progress deadline and wins over the timeout
  run giveup_inside_deadline_wins_over_timeout = init
    .then(Deploy(1)).then(Open(1)).then(AttemptEnd("startup_stop")).then(Advance(7)).then(GiveUp)
    .expect(sm.get(1).kind == "giveup" and sm.get(1).verdict and Safety)
    .then(Advance(16))
    .expect(not(canFire(1)) and sm.get(1).closes == 1 and Safety)

  // a redeploy of an unchanged, stuck spec: a second stream with a new clock; the first stays closed
  run redeploy_unchanged_opens_new_clock = init
    .then(Deploy(12)).then(Open(1)).then(Advance(15)).then(Fire(1))
    .expect(sm.get(1).kind == "timeout")
    .then(Open(2))
    .expect(sm.get(2).st == "open" and sm.get(2).armed == 15 and not(canFire(2)) and sm.get(1).closes == 1 and Safety)
    .then(Advance(16))
    .expect(not(canFire(2)) and sm.get(1).st == "closed" and Safety)
}

module pd_ok { import progress_deadline(OFF = Set(), REC = false, SLOTS = Set(1)).* from "./progress_deadline" }
module pd_conf { import progress_deadline(OFF = Set(), REC = true, SLOTS = Set(1, 2)).* from "./progress_deadline" }
