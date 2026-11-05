// -*- mode: Bluespec; -*-
/**
 * The Service deploy stream first reports the verdict the workload's current attempt already carries, then waits
 * (service-allocation-readiness-and-ending D11, ADR-0263). An ordering protocol between the stream's subscription, its read of the
 * recorded verdict, the shim's row commit and its event publication (List-then-Watch). The stream is an observer: it writes
 * no row. Time is not modelled here (D10's timer is progress_deadline.qnt); `armed` records that the stream reached the wait
 * and armed D10's deadline.
 *
 * The workload (environment, the production shim and reconcilers): attempts 0 and 1 (the current attempt `cur`; a fresh-id
 * restart supersedes attempt 0). `com(a)` is the terminal the shim COMMITTED to attempt a's row (none | stable |
 * startup_stop | liveness_stop | giveup | operator_stop; one terminal per row), `pub(a)` whether the shim PUBLISHED the
 * lifecycle event for that write (commit precedes publish, V23). `proj` is the model of service_event_from_terminal: stable,
 * giveup and operator_stop close the stream; the two restartable probe stops project to none (D7 item 5).
 *
 * The stream (record Strm): st open | closed; phase: unsub | sub | rd (Subscribe then Read, each once); chan: published
 * attempts delivered to the subscription and not yet consumed; lagged: the receiver lagged, a relist is due; armed;
 * kind (stable | giveup | operator_stop | error), src (live | read | err), closeTerm / closeAtt / readCur (the terminal and
 * the attempt that closed it, and the attempt current at the read); closes: verdict lines yielded; readFail.
 * Steps of the stream (D11 item 1): Subscribe, Read (each once, in that order), then wait: Deliver (a channel event), Relist
 * (after Lag; Deliver is disabled while lagged and takes only the oldest event). Environment: Commit, Publish, Lag, Restart.
 *
 * OFF names the rules switched off (hazards): readBeforeSubscribe (D11: the stream reads the recorded verdict and then
 * subscribes).
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   SS-1  The shim commits a row before it publishes the event for it, on every terminal path (V23). Discharged by: S-41.
 *   SS-2  The stream receives every event published after it subscribed, except those a lagged receiver drops (V21).
 *         Discharged by: V21, S-40.
 *   SS-3  current_alloc selects the workload's current attempt: the numeric-maximum attempt suffix, and every restart mints a
 *         higher suffix (V25). Discharged by: V25.
 *   SS-4  The recorded verdict's terminal is the terminal the row's event carries (ADR-0037 section 4, S-41's byte-equality
 *         check). Discharged by: S-41.
 *   SS-5  The read and the relist are single atomic store scans (one `alloc_status_rows` call). Discharged by: S-40.
 *   SS-7  The bus channel is FIFO per receiver and Lag is abstracted as drop-all-unseen (see Lag). Discharged by: V21, S-40.
 *   SS-6  The event for an attempt's terminal is published before the attempt's successor is minted (a fresh-id restart),
 *         so the live path never sees a superseded attempt's publication. Discharged by: V29 (feature-delta R-4).
 *
 * D-UP-1: `conf_step`. Conformance projection: the first terminal line yielded by build_service_stream under SimClock (the
 * closing kind); Commit is a write through SimObservationStore, Publish a LifecycleEvent sent on the bus, Subscribe / Read
 * the stream's own steps (the `Accepted` line marks the subscribe point); no new accessor.
 */
module stream_snapshot_conf {
  type ConfStep =
    | init
    | Subscribe
    | Read({ ok: bool })
    | Commit({ a: int, t: str })
    | Publish({ a: int })
    | Deliver({ a: int })
    | Lag
    | Relist({ ok: bool })
    | Restart
    | Idle
}

module stream_snapshot {
  import stream_snapshot_conf.ConfStep from "./stream_snapshot"
  const OFF: Set[str]
  const REC: bool

  val ATTS: Set[int] = Set(0, 1)
  val BUG_READFIRST: bool = OFF.contains("readBeforeSubscribe")

  type Strm = {
    st: str, phase: str, chan: List[int], lagged: bool, armed: bool, kind: str, src: str,
    closeTerm: str, closeAtt: int, readCur: int, closes: int, readFail: bool, lags: int,
  }
  type World = { cur: int, com: int -> str, pub: int -> int }  // pub: -1 unpublished, else the attempt current (w.cur) when the event was published

  var w: World
  var s: Strm
  var conf_step: ConfStep

  // the model of service_event_from_terminal
  pure def proj(t: str): str =
    if (t == "stable") "stable" else if (t == "giveup") "giveup" else if (t == "operator_stop") "operator_stop" else ""

  action init = all {
    w' = { cur: 0, com: ATTS.mapBy(a => "none"), pub: ATTS.mapBy(a => -1) },
    s' = { st: "open", phase: "unsub", chan: List(), lagged: false, armed: false, kind: "", src: "",
           closeTerm: "", closeAtt: -1, readCur: -1, closes: 0, readFail: false, lags: 0 },
    conf_step' = variant("init", ()),
  }

  // Subscribe to the bus: events published from now on reach the channel
  action Subscribe = all {
    s.st == "open", s.phase == (if (BUG_READFIRST) "readfirst" else "unsub"),
    s' = { ...s, phase: if (BUG_READFIRST) "rd" else "sub" },
    w' = w,
    conf_step' = if (REC) variant("Subscribe", ()) else conf_step,
  }

  // The recorded-verdict read: the current attempt's committed terminal projected through service_event_from_terminal. A
  // verdict closes the stream (no timer); none arms the deadline once; a failed read closes with an error.
  def doRead(x: Strm, ok: bool): Strm = {
    val t = w.com.get(w.cur)
    val v = proj(t)
    if (not(ok)) { ...x, st: "closed", kind: "error", src: "err", closes: x.closes + 1, readFail: true }
    else if (v != "") { ...x, st: "closed", kind: v, src: "read", closeTerm: t, closeAtt: w.cur, readCur: w.cur,
                        closes: x.closes + 1 }
    else { ...x, lagged: false, armed: true }
  }
  action Read(ok: bool): bool = all {
    s.st == "open", s.phase == (if (BUG_READFIRST) "unsub" else "sub"),
    s' = { ...doRead(s, ok), phase: if (BUG_READFIRST) "readfirst" else "rd" },
    w' = w,
    conf_step' = if (REC) variant("Read", { ok: ok }) else conf_step,
  }

  // the shim commits a terminal to attempt a's row (once)
  action Commit(a: int, t: str): bool = all {
    a <= w.cur, w.com.get(a) == "none",
    w' = { ...w, com: w.com.set(a, t) },
    s' = s,
    conf_step' = if (REC) variant("Commit", { a: a, t: t }) else conf_step,
  }

  // the shim publishes the event for the committed write; a subscribed open stream receives it
  action Publish(a: int): bool = all {
    w.com.get(a) != "none", w.pub.get(a) < 0,
    w' = { ...w, pub: w.pub.set(a, w.cur) },
    s' = if (s.st == "open" and (s.phase == "sub" or s.phase == "rd")) { ...s, chan: s.chan.append(a) } else s,
    conf_step' = if (REC) variant("Publish", { a: a }) else conf_step,
  }

  // the stream consumes the OLDEST channel event (after its read; the broadcast channel is FIFO per receiver; disabled while
  // lagged: the relist comes first): a projecting terminal closes it, a restartable stop closes nothing
  action Deliver(a: int): bool = {
    val t = w.com.get(a)
    val v = proj(t)
    all {
      s.st == "open", s.phase == "rd", not(s.lagged), s.chan.length() > 0, s.chan[0] == a,
      s' = if (v != "") { ...s, chan: s.chan.tail(), st: "closed", kind: v, src: "live", closeTerm: t,
                          closeAtt: a, closes: s.closes + 1 }
           else { ...s, chan: s.chan.tail() },
      w' = w,
      conf_step' = if (REC) variant("Deliver", { a: a }) else conf_step,
    }
  }

  // the receiver lags: the model drops ALL unseen events and a relist is due (bounded: two lags). Abstraction (SS-7): tokio's
  // broadcast receiver, after RecvError::Lagged, resumes at the oldest event still retained; the model drops every unseen
  // one. The difference is harmless: the relist reads the store, commit precedes publish (SS-1), so any retained
  // event's terminal is already in the relisted verdict, and a retained event delivered later only re-reads a terminal that
  // the store already showed. Publish keeps appending while lagged.
  action Lag = all {
    s.st == "open", s.phase == "rd", s.chan.length() > 0, s.lags < 2,
    s' = { ...s, chan: List(), lagged: true, lags: s.lags + 1 },
    w' = w,
    conf_step' = if (REC) variant("Lag", ()) else conf_step,
  }

  // D11 item 4: on Lagged the stream repeats the read, then continues to wait
  action Relist(ok: bool): bool = all {
    s.st == "open", s.phase == "rd", s.lagged,
    s' = doRead(s, ok),
    w' = w,
    conf_step' = if (REC) variant("Relist", { ok: ok }) else conf_step,
  }

  // a fresh-id restart supersedes attempt 0 (it needs a terminal row on attempt 0, and SS-6: that attempt's event is
  // published before the successor is minted)
  action Restart = all {
    w.cur == 0, w.com.get(0) != "none", w.pub.get(0) >= 0,
    w' = { ...w, cur: 1 },
    s' = s,
    conf_step' = if (REC) variant("Restart", ()) else conf_step,
  }

  action Idle = all { w' = w, s' = s, conf_step' = if (REC) variant("Idle", ()) else conf_step }

  action step = {
    nondet a = ATTS.oneOf()
    nondet term = Set("stable", "startup_stop", "liveness_stop", "giveup", "operator_stop").oneOf()
    nondet ok = Set(true, false).oneOf()
    any { Subscribe, Read(ok), Commit(a, term), Publish(a), Deliver(a), Lag, Relist(ok), Restart, Idle }
  }

  // ------------------------------------------------------------------ invariants
  // D11 StreamEndsOnceHonestly: the stream yields at most one verdict line, and never stable unless a Stable was committed
  val StreamEndsOnceHonestly: bool = and {
    s.closes <= 1,
    (s.kind == "stable") implies ATTS.exists(a => w.com.get(a) == "stable"),
  }
  // D7 item 5 / D11 RestartableStopNeverEndsStream: no closure from a probe stop, live or read
  val RestartableStopNeverEndsStream: bool =
    (s.src == "live" or s.src == "read") implies (s.closeTerm != "startup_stop" and s.closeTerm != "liveness_stop")
  // D11 FailedReadNeverWaits: a failed read (at open or on a relist) closes the stream as an error at once: it neither waits
  // nor is treated as "no verdict" (the run failed_read_ends_with_error pins that the open-time failure arms no deadline)
  val FailedReadNeverWaits: bool = s.readFail implies (s.kind == "error" and s.st == "closed")
  val Safety: bool = and { StreamEndsOnceHonestly, RestartableStopNeverEndsStream, FailedReadNeverWaits }

  // D11 NoMissedTerminal (progress under fairness): an open stream that has subscribed and read, whose current attempt has a
  // committed projecting terminal, eventually closes (or its attempt is superseded)
  def missed: bool = s.st == "open" and s.phase == "rd" and proj(w.com.get(w.cur)) != ""
  action deliverAny = any { Deliver(0), Deliver(1) }
  action relistAny = any { Relist(true), Relist(false) }
  temporal NoMissedTerminal =
    and {
      ATTS.forall(a => weakFair(Publish(a), (w, s, conf_step))),
      weakFair(deliverAny, (w, s, conf_step)),
      weakFair(relistAny, (w, s, conf_step)),
    }.implies(always(missed.implies(eventually(not(missed)))))

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // redeploy of a Stable service: Stable was committed and published before the stream subscribed; the read finds it, the
  // stream closes as stable at once, with no deadline armed
  run redeploy_unchanged_of_stable_ends_stable = init
    .then(Commit(0, "stable")).then(Publish(0)).then(Subscribe)
    .expect(s.chan == List())
    .then(Read(true))
    .expect(s.kind == "stable" and s.src == "read" and not(s.armed) and s.closes == 1 and Safety)

  // a terminal committed (and published) between the subscribe and the read is not lost: the read finds it; the event is
  // in the channel and is not delivered to a closed stream; the stream closed exactly once
  run terminal_between_subscribe_and_read_is_not_lost = init
    .then(Subscribe).then(Commit(0, "giveup")).then(Publish(0))
    .expect(s.chan == List(0))
    .then(Read(true))
    .expect(s.kind == "giveup" and s.src == "read" and s.closes == 1 and Safety)
    .then(Idle).expect(s.closes == 1 and not(s.armed) and Safety)

  // a restartable stop recorded on the current attempt is not a verdict: the stream stays open and waits
  run restartable_stop_recorded_is_not_a_verdict = init
    .then(Commit(0, "startup_stop")).then(Publish(0)).then(Subscribe).then(Read(true))
    .expect(s.st == "open" and s.armed and s.closes == 0 and Safety)
    .then(Restart).then(Commit(1, "liveness_stop")).then(Publish(1)).then(Deliver(1))
    .expect(s.st == "open" and s.closes == 0 and s.lagged == false and Safety)

  // a give-up on attempt 0 and a fresh-id restart making attempt 1 current with no terminal: the stream stays open
  run superseded_attempt_terminal_is_not_reported = init
    .then(Commit(0, "giveup")).then(Publish(0)).then(Restart).then(Subscribe).then(Read(true))
    .expect(s.st == "open" and s.closes == 0 and s.armed and w.cur == 1 and Safety)

  // Lagged then a missed commit then a relist: the stream closes
  run lagged_relist_recovers_terminal = init
    .then(Subscribe).then(Read(true))
    .then(Commit(0, "stable")).then(Publish(0)).then(Lag)
    .expect(s.lagged and s.chan == List() and s.st == "open")
    .then(Relist(true))
    .expect(s.kind == "stable" and s.src == "read" and s.closes == 1 and Safety)

  // an event published after a relist that found no verdict reaches the channel and is delivered (FIFO)
  run publish_after_relist_reaches_channel = init
    .then(Subscribe).then(Read(true))
    .then(Commit(0, "startup_stop")).then(Publish(0)).then(Lag)
    .expect(s.lagged and s.chan == List())
    .then(Restart).then(Commit(1, "liveness_stop")).then(Publish(1))
    .expect(s.chan == List(1) and s.lagged)
    .then(Relist(true))
    .expect(s.st == "open" and not(s.lagged) and s.chan == List(1))
    .then(Deliver(1))
    .expect(s.st == "open" and s.chan == List() and s.closes == 0 and Safety)

  // a failed read ends the stream with an error, neither waiting nor timing out
  run failed_read_ends_with_error = init
    .then(Subscribe).then(Read(false))
    .expect(s.kind == "error" and not(s.armed) and s.readFail and Safety)
}

module ss_ok { import stream_snapshot(OFF = Set(), REC = false).* from "./stream_snapshot" }
module ss_conf { import stream_snapshot(OFF = Set(), REC = true).* from "./stream_snapshot" }
