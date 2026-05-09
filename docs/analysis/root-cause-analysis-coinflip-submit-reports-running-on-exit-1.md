# Root Cause Analysis — `cargo overdrive job submit` reports "is running with 1/1 replicas (took live)" for a workload that exits with status 1

**Author:** Rex (RCA agent)
**Date:** 2026-05-09
**Spec under audit:** `examples/coinflip.toml` — `/bin/bash -c 'if (( RANDOM % 2 )); then echo SUCCESS; exit 0; else echo ERROR >&2; exit 1; fi'`
**Reproducer:** every `overdrive job submit examples/coinflip.toml` against a running `overdrive serve` prints `Job 'coinflipN' is running with 1/1 replicas (took live)` regardless of whether the workload exited 0 or 1, as confirmed by alternating SUCCESS/ERROR lines in the `serve` log.

---

## Problem statement

The streaming submit path emits `SubmitEvent::ConvergedRunning` and exits with code 0 the moment the action shim writes the first `AllocStatusRow { state: Running }`. That row is written **synchronously after `driver.start(&spec).await` returns `Ok(_handle)`** — i.e. as soon as the child process is `fork+exec`'d into its cgroup scope, with no liveness gate. A workload that exits with status 1 within milliseconds of `exec` produces the exit signal *after* the success row has already been broadcast, after the streaming subscriber has matched on `ConvergedRunning`, and after the CLI has already printed "is running" and returned exit code 0.

The user's framing is **confirmed and correct**. The CLI is reporting the alloc's first transition into `Running`, not its terminal outcome. The render line is unconditional on subsequent state transitions.

---

## Evidence map (file:line)

| Surface | Code citation | What it does |
|---|---|---|
| Submit-stream wire event | `crates/overdrive-control-plane/src/api.rs:599` | `SubmitEvent::ConvergedRunning { alloc_id, started_at }` — declared as a *terminal* event on the streaming bus |
| "Running" → terminal projection | `crates/overdrive-control-plane/src/streaming.rs:381-394` | A single `AllocStatusRow { state: Running }` row in the obs store triggers `ConvergedRunning` |
| TODO comment, same fn | `crates/overdrive-control-plane/src/streaming.rs:341-358` | Docstring: *"Phase 1 walking-skeleton workloads have `replicas == 1`, so any single `state == Running` row for the job triggers `ConvergedRunning`"* — and `TODO(#140): gate `ConvergedRunning` on `running_count >= replicas_desired` once a multi-replica workload lands` |
| Action shim writes Running on `Ok(_handle)` | `crates/overdrive-control-plane/src/action_shim/mod.rs:432-438` | `match driver.start(&spec).await { Ok(_handle) => (AllocState::Running, …) }` — `_handle` is dropped, no liveness check |
| Driver returns Ok the instant `Child::id()` exists | `crates/overdrive-worker/src/driver.rs:315-370` | `cmd.spawn() → Ok(child) → place_pid_in_scope → spawn_exit_watcher → return Ok(AllocationHandle { … })`. No "still alive after N ms" gate |
| Exit watcher is a separate task | `crates/overdrive-worker/src/driver.rs:510-518` | `tokio::spawn(async move { let status_result = child.wait().await; … })` — the `start` future returns `Ok` *before* this task ever runs |
| Exit observer writes Failed asynchronously | `crates/overdrive-control-plane/src/worker/exit_observer.rs:399-431, 446-457` | The `ExitEvent::Crashed { exit_code: Some(1), … }` becomes `AllocStatusRow { state: AllocState::Failed, … }` — but only after `child.wait()` resolves and the event is dispatched |
| CLI handler match arm | `crates/overdrive-cli/src/commands/job.rs:490-520` | On `SubmitEvent::ConvergedRunning`, immediately calls `format_running_summary(job_id, 1, 1, "live")` and `return Ok(SubmitStreamingOutput { exit_code: 0, … })` |
| Render template | `crates/overdrive-cli/src/render.rs:481-488` | `format!("Job '{job_name}' is running with {running}/{desired} replicas (took {took_human})\n")` — the `"live"` literal is hard-coded at the call site (`job.rs:504`) |
| Restart policy *exists* and would mask repeats | `crates/overdrive-core/src/reconciler.rs:1244, 1308-1321` | JobLifecycle restarts a `Failed` alloc up to `RESTART_BACKOFF_CEILING` (5 in Phase 1); each restart writes a fresh `Running` row via `Action::RestartAllocation` (action_shim mod.rs:493-499) |
| Streaming cap | `crates/overdrive-control-plane/src/lib.rs:177, 205` | `streaming_cap = 60s` default; the stream returns terminal-or-cap, whichever comes first |

---

## Toyota 5 Whys — multi-causal branching

```
PROBLEM: `overdrive job submit coinflip.toml` reports
         "is running with 1/1 replicas (took live)" exit 0,
         even when the workload's process exits status 1
         within milliseconds.

WHY 1A: The CLI prints the Running summary because it received
        SubmitEvent::ConvergedRunning from the streaming bus.
        [Evidence: cli/src/commands/job.rs:490-520; the match arm
         calls format_running_summary and returns exit_code: 0]

  WHY 2A: ConvergedRunning was emitted because the obs store has
          an AllocStatusRow with state == AllocState::Running.
          [Evidence: streaming.rs:381-394; check_terminal scans
           obs.alloc_status_rows() for any Running row matching job_id]

    WHY 3A: A Running row was written because driver.start(&spec)
            returned Ok(_handle) — the action shim writes
            (AllocState::Running, Started) the instant the future
            resolves Ok.
            [Evidence: action_shim/mod.rs:432-438; the row is built
             unconditionally on Ok(_handle) and written via
             obs.write(ObservationRow::AllocStatus(row))]

      WHY 4A: ExecDriver::start returns Ok the instant cmd.spawn()
              succeeds and place_pid_in_scope succeeds — i.e. as
              soon as fork+exec hands back a live child PID and that
              PID is placed into the workload's cgroup scope.
              [Evidence: driver.rs:315-370; no sleep, no wait, no
               liveness probe between spawn() and Ok(handle)]

        WHY 5A: ROOT CAUSE A — "Running" in the obs store is
                defined as "the kernel accepted exec and the PID is
                in its cgroup," NOT as "the process is still alive
                some non-trivial time after exec." There is no
                debounce / settle / liveness gate between the
                successful fork+exec and the Running write.
                [Evidence: action_shim/mod.rs:432, driver.rs:315-370,
                 ADR-0032 §5 + Amendment 2026-04-30 — the typed
                 cause-class classification fires on
                 DriverError::StartRejected, but a successful
                 fork+exec is the success criterion regardless of
                 what the child does next]

WHY 1B: The CLI treats ConvergedRunning as a *terminal* event on
        the stream — receiving it ends the loop and returns exit 0.
        [Evidence: cli/src/commands/job.rs:490-520; the arm
         `return`s out of the streaming loop. The error branch at
         job.rs:592-597 confirms the contract: the stream MUST
         close on one of {ConvergedRunning, ConvergedFailed,
         ConvergedStopped} or it's a protocol violation.]

  WHY 2B: The streaming protocol's *contract* is "single Running
          row meets the converged bar at replicas == 1."
          [Evidence: streaming.rs:341-358 (function docstring),
           api.rs:542 (SubmitEvent variants list), and
           architecture.md §10 referenced from job.rs:496-502]

    WHY 3B: The contract was sized for the Phase-1 walking-skeleton
            scope: replicas == 1 workloads where "the kernel said
            yes" was deemed a sufficient witness of convergence.
            [Evidence: streaming.rs:355 — `TODO(#140): gate
             ConvergedRunning on running_count >= replicas_desired
             once a multi-replica workload lands`. The TODO names
             the *replica-count* gap but not the *liveness-window*
             gap.]

      WHY 4B: The Phase-1 design assumed the workloads being
              streamed were long-lived (the canonical example is
              `/bin/sleep 3600` per the integration tests at
              cli/tests/integration/streaming_submit_happy_path.rs:9).
              A workload that exits during the streaming window —
              fast or slow — was not a designed-for case;
              ConvergedStopped was added later as a separate
              terminal arm (job.rs:553-579, render.rs:506-520, see
              the `fix-converged-stopped-cli-arm` RCA referenced at
              render.rs:505) but only fires once the exit observer
              has written a Terminated row.
              [Evidence: integration test fixtures use long-running
               binaries; the streaming_cap default is 60s
               (lib.rs:177); ConvergedStopped's existence was a
               post-hoc fix for clean-stop UX, not for fast-exit
               UX]

        WHY 5B: ROOT CAUSE B — The streaming submit's "converged"
                semantics conflate two distinct propositions:
                (i) "the desired replica count is met right now"
                (ii) "the workload has reached a stable state worth
                     reporting to the operator as success."
                Phase 1's contract treats a single Running row as
                proof of (ii); for short-lived workloads the
                propositions diverge and (ii) does not hold.
                [Evidence: same as 1B/2B/3B/4B — the design
                 contract has no notion of stability or
                 settle-time at all; first-Running IS converged.]

WHY 1C: The user submits the same job multiple times and observes
        "is running" every time, including for runs the script
        exited 1 for. The same exit-1 alloc would also subsequently
        be *restarted* by the JobLifecycle reconciler.
        [Evidence: reconciler.rs:1244 (restart loop), :1308-1321
         (Action::RestartAllocation construction), action_shim
         mod.rs:470-510 (each restart writes a fresh Running row
         on Ok(_handle))]

  WHY 2C: The restart on exit-1 would, in principle, eventually
          succeed (RANDOM is 50/50; on average <2 attempts to hit
          exit 0) — making the job genuinely reach a stable
          Running state for the SUCCESS branch within the 60s
          streaming window.
          [Evidence: RESTART_BACKOFF_CEILING == 5 (api.rs:410),
           backoff schedule via RETRY_BACKOFFS / backoff_for_attempt
           in reconciler.rs:1286]

    WHY 3C: BUT the streaming subscriber returns ConvergedRunning
            on the *first* Running row, not the *stable* one. So
            even when the user observed an exit-1 in the serve
            log, the CLI saw the pre-exit Running row from that
            same alloc (exit-1 sequence: Pending → Running → Failed
            → … → eventual restart). The CLI's render does NOT
            reflect the post-Running observations.
            [Evidence: streaming.rs:381-394 — the predicate is
             `state == Running`, no qualifier on durability; CLI
             match arm at job.rs:490 returns immediately]

      WHY 4C: The handler does not subscribe to *post-Running*
              events for the same alloc. There is no "watch the
              alloc for N seconds after Running and downgrade to
              ConvergedFailed if it crashes" path. ConvergedFailed
              fires only when the reconciler stamps a terminal
              `BackoffExhausted` claim onto a row (streaming.rs:
              369-371, 408-416), which by construction requires
              exhausting the entire restart budget — many seconds
              away even in pathological cases.
              [Evidence: streaming.rs:359-396 — the function
               returns Some(SubmitEvent) on the first matching
               row and the outer loop yields-and-returns; no
               "look-back" or "settle window" exists]

        WHY 5C: ROOT CAUSE C — The streaming protocol is
                edge-triggered on the *first* Running observation
                rather than level-triggered on the alloc's *stable*
                state. The contract collapses an unbounded sequence
                of (Running, Failed, Running, Failed, …, Terminal)
                transitions into the single first-Running event —
                losing all information about whether the workload
                is actually behaving correctly.
                [Evidence: same as 4C; reinforced by streaming.rs:
                 355's TODO(#140) which addresses replica counting
                 but not state stability]

WHY 1D: The render literal `"live"` is hard-coded at the call
        site, not derived from any actual liveness measurement.
        [Evidence: cli/src/commands/job.rs:504 — `"live"` is
         passed as the `took_human` argument to
         format_running_summary]

  WHY 2D: format_running_summary itself accepts whatever string
          the caller passes; the function is a pure formatter
          [Evidence: render.rs:481-488 — pure function, no
           validation of `took_human` shape]

    WHY 3D: The handler picks `"live"` because the ConvergedRunning
            event carries `started_at` (a logical timestamp string)
            but no elapsed-since-submit duration; the handler does
            not compute elapsed time itself either.
            [Evidence: api.rs:599 — variant is
             `ConvergedRunning { alloc_id, started_at }`; job.rs:
             490-505 — the arm destructures both as `_` and ignores
             them]

      WHY 4D: The Phase-1 design treats `"live"` as a sentinel
              meaning "the streaming witness fired" — which IS the
              authoritative timing for Phase 1's contract, because
              `(took {took})` is meant to convey "the streaming
              subscriber observed a Running event in real time"
              vs. a future replay/snapshot path that might present
              `"snapshot"` or `"recovered"` instead.
              [Evidence: render.rs:475-487 docstring; the
               recovery path at streaming.rs:441-480 builds the
               same ConvergedRunning shape from a snapshot]

        WHY 5D: ROOT CAUSE D — The "took live" literal is a
                category-error in the operator-facing render: it
                names the *delivery mechanism of the witness*
                (streaming vs snapshot), not the *outcome the
                operator cares about* (how long until the workload
                stabilised). To an operator reading the line, "took
                live" reads as "took [time] live" → "the workload
                came up live in [time]" — which it was not asked
                to assert.
                [Evidence: same as 4D; the docstring's framing
                 aligns with the developer mental model but does
                 not match the natural-language reading.]
```

---

## Cross-validation

**Backwards chain.** For each root cause, does it produce the observed symptom?

- **A** (no liveness gate after fork+exec): Yes — exec succeeds, `start` returns `Ok`, action_shim writes Running, streaming yields ConvergedRunning, CLI exits 0 — all before `child.wait()` resolves with status 1.
- **B** (first-Running == converged in the contract): Yes — the contract directly equates these; the symptom is the contract's correct execution.
- **C** (edge-triggered, not level-triggered): Yes — even if we fixed A and B in isolation, C would still let a Running→Failed→Running flapping alloc emit ConvergedRunning on the first leg. The user's "every submit reports Running" is consistent with this: the second-run output is the same as the first-run output.
- **D** (`"live"` is a delivery-mechanism marker, not a duration): Yes — independent symptom; would not by itself produce the false-positive but explains why the rendered line is *additionally* misleading even in cases where A/B/C correctly emit ConvergedRunning.

**Cross-cause consistency.** A, B, C, D do not contradict. They compose:
- A is a *worker-driver-side* design (`Driver::start` semantics).
- B is a *control-plane streaming-protocol* design (`SubmitEvent` semantics).
- C is a *temporal-coupling* design (edge vs level on the protocol).
- D is a *CLI-render* design (presentation literal).

Removing any one of {A, B, C} alone would close the false-positive on a *different* class of workload but not all of them. Removing all three closes it. D is independently fixable and improves UX regardless.

**Completeness check.** Are there missing branches?

- Considered and rejected: *spec.id reuse* — the fact that submits 1+2 produced `coinflip` and later `coinflip2/3/4` reflects the user pasting different `id =` lines into the spec (the toml shown carries `id = "coinflip4"`). It is unrelated to the false-positive. The intent-key idempotency surface (`SubmitEvent::Accepted.outcome`) governs replay semantics, not the stream's terminal classification.
- Considered and rejected: *RANDOM seeding in bash* — bash's `$RANDOM` is process-local and not seeded from time by default in non-interactive shells, but the user has confirmed alternating SUCCESS/ERROR in the serve log, so the workload is genuinely exiting with both outcomes; the false-positive is universal across both.
- Considered and rejected: *streaming_cap (60s) elapsed before terminal* — the cap arm at `streaming.rs:322-335` would emit ConvergedFailed{Timeout}, not ConvergedRunning. Not the failure mode here.

---

## Solutions (mapped to root causes)

The user has not asked for code changes. These are framing for a future fix; each solution names the root cause it addresses.

### Solution A → Root Cause A (no liveness gate)
**Permanent fix.** Add a *settle window* in `ExecDriver::start` (or in the action shim arm at `action_shim/mod.rs:432`) — wait `clock.sleep(START_SETTLE)` (e.g. 200 ms) after `place_pid_in_scope` and re-check `child.try_wait()` (or read from the exit-watcher channel non-blockingly). If the child has exited within the settle window, return `Err(DriverError::StartRejected { reason: "exited within settle window: exit_code=N" })` and the action shim writes `Failed`, not `Running`. The settle window is the simplest invariant that converts "kernel accepted the binary" into "the binary is alive long enough to bother reporting."
*Trade-off:* 200 ms latency added to the success path of every legitimate alloc. ADR-grade decision.

### Solution B → Root Cause B (first-Running == converged)
**Permanent fix.** Strengthen the ConvergedRunning predicate beyond `state == Running`. Two options, not mutually exclusive:
- (B1) Require the row to have been Running for ≥ N seconds (a *stability window* on the streaming side). The subscriber holds the candidate event and emits it on the timer; if a non-Running transition arrives in the window, it discards the candidate and continues subscribing.
- (B2) Adopt a richer ESR-style "stable" predicate per ADR-0033 — combine `state == Running` with `restart_count == 0` AND `oldest_running_row.age >= STABILITY_WINDOW`.

### Solution C → Root Cause C (edge-triggered protocol)
**Permanent fix.** Make the streaming protocol *level-triggered*. The subscriber tracks the alloc's full transition trajectory and only fires ConvergedRunning when the trajectory has stabilised. The `streaming_cap = 60s` default already gives the stream a bounded lifetime; within that lifetime the subscriber should be allowed to *retract* a tentative ConvergedRunning if the alloc moves out of Running. (This implies the wire protocol grows a `RunningCandidate { … }` non-terminal event before the eventual ConvergedRunning, or alternatively just suppresses ConvergedRunning until the stability window is met.) Resolves TODO(#140) by replacing the under-specified gate with a proper convergence predicate.

### Solution D → Root Cause D (`"live"` literal)
**Permanent fix (cosmetic but worthwhile).** Replace `format_running_summary("…", 1, 1, "live")` with an actual elapsed duration. The handler already has the `Accepted` event in scope and can record `Instant::now()` (via the injected `Clock`) at first send; the difference at ConvergedRunning time is the operator-meaningful "took 1.2s". Even better: drop the `(took …)` parenthetical entirely when the value is a literal sentinel; "took live" is not informative in any reading.

### Immediate mitigation (no code changes)
- Document in operator-facing material that "is running with 1/1 replicas" reflects the moment the kernel accepted the binary, not workload health. Direct operators to `overdrive alloc status --job <id>` for terminal classification.
- If the user wants a smoke-test that catches exit-1, recommend `overdrive job submit … && sleep N && overdrive alloc status --job …` as the current Phase-1 idiom.

---

## Meta-improvements surfaced

1. **The streaming protocol's terminal-event contract should be specified as a temporal-logic predicate**, not a row-shape predicate. The ADR-0033 ESR vocabulary (Eventual Stable Reachability — progress + stability) is the right tool; the current contract has progress without stability, which is exactly the failure mode this RCA names. An ADR amendment to make this explicit would prevent the same bug class from re-surfacing as the protocol grows new terminal arms.
2. **`Driver::start` returning `Ok(handle)` on bare-fork-success is a contract leak.** The `Driver` trait docstring at `crates/overdrive-core/src/traits/` (referenced indirectly via DriverError variants in this investigation) should pin what `Ok(handle)` *means* — specifically whether the contract is "exec succeeded" or "the workload is alive past a settle window." Per `.claude/rules/development.md` § "Trait definitions specify behavior, not just signature," this is a missing observable invariant on the trait.
3. **Render literals like `"live"` should not be hand-passed to operator-facing strings.** A `RenderTime` enum (`Live(Duration)`, `Snapshot`, `Recovered`) at the format-fn boundary would make the rendering decision typed and let `Display` produce something coherent in each case. The current shape is the kind of stringly-typed parameter the rest of the codebase rejects elsewhere.
4. **Phase-1 exit-fast workloads are a missing test fixture.** The streaming_submit happy-path integration tests (`cli/tests/integration/streaming_submit_happy_path.rs`) all use long-running binaries. A coinflip-style test — a workload that exits 1 within ms of exec, with the test asserting the CLI surface as `ConvergedFailed`, not `ConvergedRunning` — would have caught this and would lock in the fix for any of solutions A/B/C.
5. **The streaming.rs:355 TODO(#140) names only half the gap.** The TODO addresses replica-count under-specification but is silent on liveness/stability under-specification. That same TODO should be expanded (or split into #140 + #NEW) to cover the stability gate; otherwise a future implementer of #140 will close it without addressing this RCA's symptom.

---

## Validation summary

- All four root causes have direct file:line evidence in the live codebase.
- The four causes compose to fully explain the symptom; the cross-validation table above shows no contradictions.
- Each cause has a corresponding solution; A/B/C are independent fixes that each individually narrow the failure window but only collectively close the bug class.
- The investigation did not require a runtime reproduction — every causal step is verifiable from the current source on `marcus-sa/cgroup-eacces-debug` (HEAD `00dd9d7`).
