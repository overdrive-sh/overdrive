// -*- mode: Bluespec; -*-
/**
 * Probe-run counting (netns-density-295, round 2026-10-13): PROBE-RUN-COUNT (ADR-0191, approved by the user
 * 2026-10-13 including the producer-counted `same_status_runs` rule and the act-once consumer rule),
 * PROBE-NOSAMPLE (ADR-0187 decisions 1-3, approved). No PROPOSED decision remains.
 *
 * ONE producer (the probe task of one key (alloc, role, probe_idx)) and ONE consumer (ServiceLifecycle).
 *
 * Producer: ProbeRun(outcome, reason), outcome in Pass | Fail | NoSample | WriteFail. Pass / Fail write a row
 * {run_seq = last written + 1, status, same_status_runs}; same_status_runs counts the consecutive written runs,
 * ending with this one, with this status, counted by the producer (first row 1; same status as the previous
 * written row => previous + 1; otherwise 1). NoSample / WriteFail write nothing, consume no number, and neither
 * break nor extend the streak. Environment action: ClockBack only. A same-key loss of the producer's streak
 * memory is not modelled: the streak counter and run_seq are task-private to one probe task, a task is never
 * re-spawned for a live allocation, and a restart mints a new key (PC-1).
 * Store: one latest row, replaced iff the written run_seq is strictly greater (merge by run_seq).
 * Consumer: ConsumerLook reads the latest row (N, st, s); it keeps NO streak and does no arithmetic over runs
 * since its last look. Liveness crossed iff st == Fail and s >= LT. Readiness eligible iff st == Pass and
 * s >= RT (a level, re-derived at every look; no row => not eligible). Act once per ROW: the view keeps A, the
 * run_seq at which it last emitted (0: never); it emits a liveness terminate iff crossed AND the allocation is
 * Running AND N > A, and only when it emits sets A := N. A re-read of the same row therefore does nothing; a new
 * crossed row while the allocation is still Running (the previous stop failed) emits again: a retry once per
 * new written row.
 * Dispatch outcome: the action-shim dispatch of StopAllocation is awaited inside the reconcile tick (PC-4), so
 * the outcome is decided in the SAME ConsumerLook step: the parameter `stopOk` is chosen nondeterministically by
 * `step`. If the look emits and stopOk, the shim wrote the terminal row inline: `running := false`; if it emits
 * and not stopOk (Driver::stop or a cleanup step failed) the allocation stays Running. `A` is persisted before
 * dispatch, so it is updated either way. When running is false ConsumerLook emits nothing (the Running guard).
 * Producer runs and looks continue after the stop succeeded (allowed, not restricted); they cannot terminate.
 *
 * OFF names the rules switched off (hazards):
 *   reconstruct   the previous rule: the consumer keeps its own streak fs advanced by N - M (M = last counted
 *                 run_seq) for the row's status, reset on the other status, terminates when fs >= LT
 *   noActOnce     terminate whenever crossed and Running, no A check
 *   oncePerStreak the previous act condition: emit iff crossed and N - s >= A (once per Fail streak)
 *   onceEver      emit only if A == 0 (at most once per key, ever)
 *   noSampleFail  a NoSample run is written as a Fail row
 *   lwwObservedAt the store merges by observed-at (strictly later) instead of run_seq, with ClockBack steps
 * Main instance probe_streak: OFF = {}.
 *
 * Ghost / model-only state: `hist` (true sequence of written outcomes), `terms` / `tseqs` (terminates emitted and
 * the set of run_seqs they acted on), `g`. `fs` and `M` exist only for the `reconstruct` hazard and stay 0
 * otherwise. Conformance projection: rseq, rst, rsame (the stored row), A (consumer view) and running (the
 * allocation's AllocStatusRow is not terminal); the ghost terms are model-only.
 *
 * ASSUMPTIONS (each discharged by the item after it):
 *   PC-1  The producer is the single writer of its key, and a probe task is never re-spawned for a live allocation
 *         (`start_alloc` returns once `started`, crates/overdrive-worker/src/probe_runner/mod.rs:297-300; `stop_role`
 *         is final, :364-369); a restart or `serve` restart mints a new AllocationId (vm_reclamation_boot.rs:110-140,
 *         workload_lifecycle.rs:1207,1355-1359). So run_seq never goes backwards within a key. A crash-restart's
 *         predecessor probe tasks keep writing under their own (old) AllocationId: a different key, so not a second
 *         writer of this key; that leak is GH #318. A probe-runner restart is therefore never a same-key action: it
 *         coincides with a fresh key (run_seq 1, same_status_runs 1, no act-once entry), so it can only delay.
 *         Discharged by: R5-PROBE-COUNT (DISTILL seeded-sim scenario of the production ServiceLifecycle + probe
 *         runner, defined in the netns-density-295 feature delta)
 *   PC-2  Hydration yields the latest stored row per key (overdrive-core eval_broker coalescing; a look sees one row).
 *         Discharged by: R5-PROBE-COUNT
 *   PC-3  A run that writes no row (no sample, adapter error, failed write) leaves the store unchanged (the write
 *         surface reports failure; no partial row).
 *         Discharged by: R5-PROBE-COUNT, and for the host store by R5-PROBE-MERGE (LocalObservationStore
 *         failed-write case)
 *   PC-4  The action-shim dispatch of StopAllocation is awaited inside the reconcile tick
 *         (reconciler_runtime.rs:1604-1640 run_convergence_tick), so after a successful stop the allocation row is
 *         terminal at the next look (the existing Running guard blocks any re-terminate), and a failed stop
 *         (Driver::stop Err, action_shim/mod.rs:3229-3233, or a later cleanup step, :3240-3248) leaves it Running.
 *         The view (with A) is persisted before dispatch.
 *         Discharged by: R5-PROBE-COUNT fault (h): a scripted Driver::stop failure
 */
module probe_count {
  const OFF: Set[str]

  val LT: int = 3          // liveness failure threshold
  val RT: int = 2          // readiness success threshold
  val MAXRUNS: int = 7     // ProbeRun actions of any outcome (Fail x3, Pass, Fail x3 fits); looks are unbounded (state is finite)
  val BUG_RECON: bool = OFF.contains("reconstruct")
  val BUG_NOACT: bool = OFF.contains("noActOnce")
  val BUG_NSF: bool = OFF.contains("noSampleFail")
  val BUG_LWW: bool = OFF.contains("lwwObservedAt")
  val BUG_ONCEPER: bool = OFF.contains("oncePerStreak")
  val BUG_ONCEEVER: bool = OFF.contains("onceEver")

  type Ghost = {
    missTouched: bool,   // a NoSample / WriteFail run wrote or changed a row
    staleChange: bool,   // a look that saw no new row changed consumer state or terminated
    early: bool,         // a terminate at a look where the true trailing Fail rows < LT
    pt: int,             // true trailing Pass rows at the latest look
    ready: bool,         // readiness eligible at the latest look
    seen: int,           // run_seq of the row at the latest look
    stoppedTerm: bool,   // a terminate emitted while the allocation was not Running
    missed: bool,        // a look on a Running allocation, latest row crossed with N > A, emitted no terminate
  }

  // producer
  var runs: int            // ProbeRun actions so far (bound)
  var pseq: int            // last written run_seq (0: none)
  var pst: str             // status of the last written row ("": none)
  var psame: int           // same_status_runs of the last written row
  // store (latest row only)
  var rseq: int            // N; 0: no row
  var rst: str             // st
  var rsame: int           // s
  var rat: int             // observed-at (operator display; decides the merge only under lwwObservedAt)
  var clock: int
  // consumer view
  var A: int               // run_seq at which the consumer last emitted (0: never)
  var running: bool        // the allocation is still Running (no successful stop yet)
  var fs: int              // `reconstruct` hazard only
  var M: int               // `reconstruct` hazard only
  // ghost
  var hist: List[str]      // true sequence of written outcomes
  var terms: int           // terminates emitted
  var tseqs: Set[int]      // run_seqs of the rows terminates acted on
  var g: Ghost

  pure def trail(h: List[str], s: str): int = h.foldl(0, (c, x) => if (x == s) c + 1 else 0)

  action init = all {
    runs' = 0, pseq' = 0, pst' = "", psame' = 0,
    rseq' = 0, rst' = "", rsame' = 0, rat' = 0, clock' = 0,
    A' = 0, running' = true, fs' = 0, M' = 0,
    hist' = List(), terms' = 0, tseqs' = Set(),
    g' = { missTouched: false, staleChange: false, early: false, pt: 0, ready: false, seen: 0,
           stoppedTerm: false, missed: false },
  }

  action ProbeRun(out: str, reason: str): bool = {
    val isWrite = or { out == "Pass", out == "Fail", BUG_NSF and out == "NoSample" }
    val isMiss = or { out == "NoSample", out == "WriteFail" }
    val wst = if (out == "Pass") "Pass" else "Fail"
    val nsame = if (pst == wst) psame + 1 else 1
    val accept = if (BUG_LWW) (rseq == 0 or clock > rat) else (pseq + 1 > rseq)
    all {
      runs < MAXRUNS,
      runs' = runs + 1,
      pseq' = if (isWrite) pseq + 1 else pseq,
      pst' = if (isWrite) wst else pst,
      psame' = if (isWrite) nsame else psame,
      clock' = if (isWrite) clock + 1 else clock,
      rseq' = if (isWrite and accept) pseq + 1 else rseq,
      rst' = if (isWrite and accept) wst else rst,
      rsame' = if (isWrite and accept) nsame else rsame,
      rat' = if (isWrite and accept) clock else rat,
      hist' = if (out == "Pass" or out == "Fail") hist.append(out) else hist,
      g' = { ...g, missTouched: g.missTouched or (isMiss and isWrite and accept) },
      A' = A, running' = running, fs' = fs, M' = M, terms' = terms, tseqs' = tseqs,
    }
  }

  // environment fault: a backward step of the wall clock
  action ClockBack = all {
    clock >= 2,
    clock' = clock - 2,
    runs' = runs, pseq' = pseq, pst' = pst, psame' = psame,
    rseq' = rseq, rst' = rst, rsame' = rsame, rat' = rat,
    A' = A, running' = running, fs' = fs, M' = M, hist' = hist, terms' = terms, tseqs' = tseqs, g' = g,
  }

  action ConsumerLook(stopOk: bool): bool = {
    val hasRow = rseq > 0
    val newRow = hasRow and rseq > g.seen
    val crossed = hasRow and rst == "Fail" and rsame >= LT
    val ready = hasRow and rst == "Pass" and rsame >= RT
    // `reconstruct` hazard: the consumer's own streak, advanced by N - M
    val rNew = hasRow and rseq > M
    val rfs = if (not(rNew)) fs else if (rst == "Fail") fs + (rseq - M) else 0
    val rterm = rNew and rfs >= LT
    val canAct = running and crossed
    val emit =
      if (BUG_RECON) (running and rterm)
      else if (BUG_NOACT) canAct
      else if (BUG_ONCEPER) (canAct and rseq - rsame >= A)
      else if (BUG_ONCEEVER) (canAct and A == 0)
      else (canAct and rseq > A)
    val tf = trail(hist, "Fail")
    val tp = trail(hist, "Pass")
    val nA = if (emit and not(BUG_RECON)) rseq else A
    val nfs = if (not(BUG_RECON)) fs else if (rterm) 0 else rfs
    val nM = if (BUG_RECON and rNew) rseq else M
    all {
      fs' = nfs, M' = nM, A' = nA,
      terms' = if (emit) terms + 1 else terms,
      running' = if (emit and stopOk) false else running,
      tseqs' = if (emit) tseqs.union(Set(rseq)) else tseqs,
      g' = {
        ...g,
        staleChange: g.staleChange or (not(newRow) and (nA != A or emit)),
        early: g.early or (emit and tf < LT),
        pt: tp, ready: ready, seen: rseq,
        stoppedTerm: g.stoppedTerm or (emit and not(running)),
        missed: g.missed or (running and crossed and rseq > A and not(emit)),
      },
      runs' = runs, pseq' = pseq, pst' = pst, psame' = psame,
      rseq' = rseq, rst' = rst, rsame' = rsame, rat' = rat, clock' = clock, hist' = hist,
    }
  }

  action Idle = all {
    runs' = runs, pseq' = pseq, pst' = pst, psame' = psame,
    rseq' = rseq, rst' = rst, rsame' = rsame, rat' = rat, clock' = clock,
    A' = A, running' = running, fs' = fs, M' = M, hist' = hist, terms' = terms, tseqs' = tseqs, g' = g,
  }

  action step = {
    nondet out = Set("Pass", "Fail", "NoSample", "WriteFail").oneOf()
    nondet reason = Set("node_capacity", "quiesced").oneOf()
    nondet stopOk = Set(true, false).oneOf()
    any { ProbeRun(out, reason), ClockBack, ConsumerLook(stopOk), Idle }
  }

  // ------------------------------------------------------------------ invariants
  // PROBE-RUN-COUNT/NoEarlyTerminate: every terminate happens at a look where the true trailing Fail rows >= LT
  val NoEarlyTerminate: bool = not(g.early)
  // PROBE-RUN-COUNT/NoReactOnSameRow: no two terminates on the same row (same run_seq): a re-read after a failed
  // stop does not re-terminate
  val NoReactOnSameRow: bool = terms == tseqs.size()
  // PROBE-RUN-COUNT/NoTerminateWhenStopped: no terminate after a stop succeeded (allocation no longer Running)
  val NoTerminateWhenStopped: bool = not(g.stoppedTerm)
  // PROBE-RUN-COUNT/NoMissedTerminate (progress, as a per-look safety obligation): at every look on a Running
  // allocation whose latest row is crossed with N > A, a terminate is emitted at that look
  val NoMissedTerminate: bool = not(g.missed)
  // PROBE-RUN-COUNT/NoEarlyReady: eligible only if the true trailing Pass rows at that look >= RT
  val NoEarlyReady: bool = g.ready implies g.pt >= RT
  // PROBE-RUN-COUNT/StreakExact: the stored same_status_runs equals the true trailing count of its status
  val StreakExact: bool = rseq > 0 implies rsame == trail(hist, rst)
  // PROBE-NOSAMPLE/MissingSampleInert: NoSample and WriteFail runs write or change no row
  val MissingSampleInert: bool = not(g.missTouched)
  // PROBE-RUN-COUNT/StoreHoldsLatest: the stored row is the last written row (merge by run_seq)
  val StoreHoldsLatest: bool = rseq == pseq
  // PROBE-RUN-COUNT/NoStaleChange: a look that sees no new row changes no consumer state and emits no terminate
  val NoStaleChange: bool = not(g.staleChange)
  val Safety: bool = and { NoEarlyTerminate, NoReactOnSameRow, NoTerminateWhenStopped, NoMissedTerminate,
                           NoEarlyReady, StreakExact, MissingSampleInert, StoreHoldsLatest, NoStaleChange }

  // non-vacuity witnesses (expected violations)
  val WitnessNeverTerminates: bool = terms == 0
  val WitnessNeverReady: bool = not(g.ready)

  // ------------------------------------------------------------------ named runs (critical scenarios)
  // happy path: three Fails, a look after each; terminate at the third; the stop succeeds
  run HappyPathThreeFailsTerminateAtThreshold = init
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true)).expect(terms == 0 and rsame == 1)
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true)).expect(terms == 0 and rsame == 2)
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true))
    .expect(terms == 1 and A == 3 and rsame == 3 and not(running) and Safety)

  // three Fails, one look: terminate on the first look, no undercount
  run CoalescedFailsTerminateOnFirstLook = init
    .then(ProbeRun("Fail", "")).then(ProbeRun("Fail", "")).then(ProbeRun("Fail", ""))
    .then(ConsumerLook(true)).expect(terms == 1 and A == 3 and Safety)

  // NoSample / WriteFail between Fails are inert; the run number is reused
  run MissingSampleChangesNothing = init
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true)).expect(rseq == 1 and rsame == 1)
    .then(ProbeRun("NoSample", "node_capacity")).then(ProbeRun("WriteFail", "")).then(ProbeRun("NoSample", "quiesced"))
    .expect(rseq == 1 and pseq == 1 and rsame == 1 and Safety)
    .then(ConsumerLook(true)).expect(terms == 0 and A == 0 and Safety)
    .then(ProbeRun("Fail", "")).expect(rseq == 2 and pseq == 2 and rsame == 2)
    .then(ConsumerLook(true)).expect(terms == 0 and Safety)

  // cross, terminate, the stop FAILS (allocation stays Running), look again with no new row: still one terminate
  run ReReadAfterFailedStopDoesNotReTerminate = init
    .then(ProbeRun("Fail", "")).then(ProbeRun("Fail", "")).then(ProbeRun("Fail", ""))
    .then(ConsumerLook(false)).expect(terms == 1 and A == 3 and running)
    .then(ConsumerLook(false)).expect(terms == 1 and A == 3 and running and Safety)

  // cross, terminate, the stop fails, one more Fail, look: a second terminate (retry once per new row); the
  // stop then succeeds; further looks and rows emit nothing
  run FailedStopRetriedOnNextRow = init
    .then(ProbeRun("Fail", "")).then(ProbeRun("Fail", "")).then(ProbeRun("Fail", ""))
    .then(ConsumerLook(false)).expect(terms == 1 and A == 3 and running)
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true)).expect(terms == 2 and A == 4 and not(running) and Safety)
    .then(ConsumerLook(true)).expect(terms == 2 and Safety)
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true)).expect(terms == 2 and A == 4 and Safety)

  // a failed stop, then Pass, then a new Fail streak crossing again while still Running: terminate again
  run NewStreakAfterPassTerminatesAgain = init
    .then(ProbeRun("Fail", "")).then(ProbeRun("Fail", "")).then(ProbeRun("Fail", ""))
    .then(ConsumerLook(false)).expect(terms == 1 and A == 3 and running)
    .then(ProbeRun("Pass", "")).then(ConsumerLook(false)).expect(terms == 1 and A == 3 and Safety)
    .then(ProbeRun("Fail", "")).then(ProbeRun("Fail", "")).then(ProbeRun("Fail", ""))
    .then(ConsumerLook(false)).expect(terms == 2 and A == 7 and rsame == 3 and running and Safety)

  // Fail, look, Pass, Fail, look: the streak is 1, no early terminate
  run FlipBetweenLooksNoEarlyTerminate = init
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true))
    .then(ProbeRun("Pass", "")).then(ProbeRun("Fail", ""))
    .then(ConsumerLook(true)).expect(terms == 0 and rsame == 1 and Safety)

  // readiness is a level re-derived at every look
  run ReadinessReachesThreshold = init
    .then(ProbeRun("Pass", "")).then(ConsumerLook(true)).expect(not(g.ready) and Safety)
    .then(ProbeRun("Pass", "")).then(ConsumerLook(true)).expect(g.ready and Safety)
    .then(ProbeRun("Fail", "")).then(ConsumerLook(true)).expect(not(g.ready) and Safety)

  // a backward clock step does not lose a row: the merge is by run_seq, not observed-at
  run ClockStepBackDoesNotLoseRow = init
    .then(ProbeRun("Fail", "")).then(ProbeRun("Fail", "")).then(ClockBack).then(ProbeRun("Fail", ""))
    .expect(rseq == 3 and pseq == 3 and Safety)
    .then(ConsumerLook(true)).expect(terms == 1 and Safety)
}

// Main instance: the approved rule. All invariants hold.
module probe_streak { import probe_count(OFF = Set()).* from "./probe_count" }
