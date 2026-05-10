# RCA — `exit_observer.find_prior_row` race against action-shim `obs.write(Running)` for sub-millisecond-exit workloads

**Status**: Analysis (RCA only; no implementation)
**Revised**: 2026-05-10 — recommendation corrected per user pushback that Solution 2 was a tolerator, not a fixer. Solution 1' (oneshot-gated watcher emission) is now the recommended path; Solution 2 retained as defence-in-depth fallback only.
**Investigator**: Rex (nw-troubleshooter)
**Date**: 2026-05-10
**Triggering artifact**: rustdoc on `crates/overdrive-cli/tests/integration/coinflip_honesty_100_trials.rs:103-117` and `crates/overdrive-cli/tests/integration/job_kind_streaming.rs:218-227`
**Predecessor RCA**: `docs/feature/fix-exit-observer-write-retry/deliver/rca.md` (2026-05-02, Option A — bounded retry on the *write*; this report covers a different branch — the *read* of the prior row)

---

## 1. Problem definition (scope and evidence)

### 1.1 Symptom

For a workload whose process lifetime from `Driver::start` return through `child.wait()` resolution is shorter than the wall-clock window between (a) the action shim returning from `driver.start(&spec).await` and (b) the action shim completing `obs.write(ObservationRow::AllocStatus(Running))`:

- The exit-watcher's `ExitEvent` lands on the `mpsc::Receiver<ExitEvent>` inside `spawn_with_runtime` (`exit_observer.rs:185`) before the action shim has written the `Running` row.
- `run_with_retry` calls `handle_exit_event` (`exit_observer.rs:298`) → `find_prior_row(obs, &event.alloc)` (`exit_observer.rs:411`) → `obs.alloc_status_row(alloc)` returns `Ok(None)`.
- `handle_exit_event` returns `Ok(None)` (`exit_observer.rs:413`).
- `run_with_retry` returns `RetryOutcome::NoPriorRow` (`exit_observer.rs:303`).
- The branch arm at `exit_observer.rs:225-228` is empty: *no obs row written, no LifecycleEvent broadcast, no terminal escalation, no log line at any level*. The exit event is silently dropped.
- Downstream: the alloc's only obs row remains the `Running` row that the action shim writes immediately after (line 499). The reconciler reads `Running`, emits zero actions (`reconciler.rs:1066-1069` per predecessor RCA). The CLI's streaming session waits for a terminal verdict that never arrives, or hangs until a higher-level timeout / transport close — and per `coinflip_honesty_100_trials.rs:295-302` the failure mode degrades the CLI's exit code honesty (KPI K1).

### 1.2 Concealment in current tests

Every Job-kind acceptance fixture inserts `sleep 0.5` in the workload bash. Cited:

- `coinflip_honesty_100_trials.rs:128` — `"sleep 0.5; if (( RANDOM % 2 )); …"`
- `job_kind_streaming.rs:235` — `"sleep 0.5; exit 0"`
- `job_kind_streaming.rs:255` — `"sleep 0.5; echo 'workload stderr line' >&2; exit 1"`

The rustdoc at `coinflip_honesty_100_trials.rs:103-117` and `job_kind_streaming.rs:218-227` calls this concealment out by name and labels it a known Phase 1 race.

### 1.3 Scope boundaries

In scope: the producer/transport/consumer triangle — action shim's `Running` write, the in-process `ObservationStore` ordering, and `exit_observer::find_prior_row` consumption strategy.

Out of scope: the CLI's streaming-wire close-vs-process-exit ordering (separately enumerated at `coinflip_honesty_100_trials.rs:298-301`); reconciler's `hydrate_actual` Driver-status probing (rejected by predecessor RCA Option C); the May-2 obs-write-retry path itself, *except* where it overlaps the consumption-side race (§7).

---

## 2. Five-Whys investigation

> **Per skill methodology**: each branch is independent through level 5; evidence cites file:line.

### 2.1 Producer-side branch (A) — action shim ordering

**WHY 1A — symptom**: For a sub-ms-exit workload, the `ExitEvent` mpsc message arrives at the observer's receiver before the action shim has called `obs.write(Running)`.
- Evidence: `action_shim/mod.rs:472` `driver.start(&spec).await` returns *with* the watcher already spawned and the child already running. The `obs.write(...)` is at line 499 — after the await, after the build of the row, after the `match` against driver result. For a `/bin/true`-shaped workload, `child.wait()` resolves before the awaiting task is scheduled past line 472.

**WHY 2A — context**: `driver.start` returns as soon as the child is `mkdir`'d into its cgroup scope and `spawn_exit_watcher` has been spawned. It does not await any post-start barrier with the action shim.
- Evidence: `crates/overdrive-worker/src/driver.rs:391-402` — `spawn_exit_watcher` is `tokio::spawn`'d, then `LiveAllocation` is inserted into `self.live`, then `Ok(AllocationHandle { … })` is returned. No coordination with any `Running`-row publication.
- The watcher itself goes straight to `child.wait().await` at `driver.rs:571` and emits the event with `let _ = exit_tx.send(event).await;` at `driver.rs:638`.

**WHY 3A — system**: The action shim's contract treats the `Running` row as a *post-condition* of `driver.start`'s success, not as a *pre-condition* the driver must observe before resolving its exit watcher. The watcher's `mpsc::send` and the shim's `obs.write` are concurrent emitters racing onto the same logical observation surface (the alloc's row stream) from two different control-plane writers, with no happens-before edge between them.
- Evidence: `action_shim/mod.rs:448-501` — the entire arm is straight-line; the `Running` write happens-after `driver.start` returns but happens-after-NOTHING with respect to the watcher's `child.wait()` resolution. The two are siblings, not ordered.

**WHY 4A — design**: The architecture (per `architecture.md §10` referenced at `action_shim/mod.rs:311`) defines "every successful action emits an obs row + lifecycle event" as the shim's contract. It does not require that contract to hold against an independent emitter (the watcher) racing it. The implicit assumption: workloads run "long enough" that the shim's own write lands first.
- Evidence: the design assumption is invisible in code but visible in the sleep-0.5 fixture concealment (`coinflip_honesty_100_trials.rs:107-117` rustdoc names "sub-millisecond-exit workloads race the obs write" as the pathological case). A wall-clock budget IS the design — undocumented and unverified.

**WHY 5A — root cause**: **The action shim establishes no happens-before edge between `obs.write(Running)` and the exit-watcher's first `ExitEvent` emission. The "Running before exit" ordering is a wall-clock coincidence, not a structural guarantee.**
- → **ROOT CAUSE A**: Producer-side ordering gap — the `Running` row publication is concurrent with, not prior to, the watcher's exit-event emission.

### 2.2 Transport / store branch (B) — single-writer, two-emitter rendezvous

**WHY 1B — symptom**: When the watcher's `ExitEvent` arrives at the observer first, the observer queries `obs.alloc_status_row(alloc)` and gets `None` — there is no prior row at all, even though the shim is microseconds away from writing one.
- Evidence: `exit_observer.rs:411` `find_prior_row(obs, &event.alloc)` → `exit_observer.rs:491` `obs.alloc_status_row(alloc).await?`. The function is a one-shot point read: if no row exists right now, return `Ok(None)`.

**WHY 2B — context**: The `ObservationStore::alloc_status_row` API gives no ordering guarantee against in-flight writes from other writers. It snapshots whatever the store contains *now*.
- Evidence: trait method behaviour as exercised by the `SimObservationStore` `single_peer` test path (`crates/overdrive-control-plane/tests/integration/job_lifecycle/exit_observer.rs:88-89`) and the redb-backed production path. The store does not know that "an exit row for alloc X is being processed" or "a Running row for alloc X is imminent."

**WHY 3B — system**: There is no per-alloc happens-before edge in the obs store between the action shim (writer 1) and the exit observer (writer 2). LWW counter dominance solves *which row wins on retrieval*; it does not solve *waiting for any row to exist before classifying an exit*.
- Evidence: `exit_observer.rs:418-421` — the observer reuses `prior.updated_at.counter.saturating_add(1)` and `prior.node_id.clone()` to build the new row's `LogicalTimestamp`. Both fields are read from `prior`; without `prior` there is no way to construct a well-formed successor row at all.
- This dependency is named in the rustdoc at `exit_observer.rs:485-486` ("Find the LWW-winner row for this alloc — used to recover the (`job_id`, `node_id`) tuple and the prior `LogicalTimestamp` counter").

**WHY 4B — design**: The Phase 1 obs-as-truth design (per predecessor RCA `fix-exec-driver-exit-watcher`, cited at `fix-exit-observer-write-retry/deliver/rca.md:42-46`) deliberately consolidates "what is" into the obs store as a single rendezvous. That consolidation is correct for read consumers (reconciler, gateway) but creates a transitive dependency for write-only producers (the exit observer) that need to read another producer's row before they can write their own.
- Evidence: `exit_observer.rs:411-446` — the observer's whole write path is structurally dependent on a successful `find_prior_row`. The shape "exit observer is a producer" is true at the surface; underneath, it is "exit observer is a producer that must first be a reader of another producer's output."

**WHY 5B — root cause**: **The obs store is the rendezvous medium for two producers (action shim, exit observer) but provides no rendezvous semantics — no waitable subscription, no per-key happens-before, no "barrier on first write for this alloc." The consumer-becomes-producer pattern in the observer is a structural debt, not an implementation choice.**
- → **ROOT CAUSE B**: Transport/store gap — the observer's row-construction depends on the action shim's prior row, but the store API is point-read, not happens-before / wait-for.

### 2.3 Consumer-side branch (C) — `find_prior_row` is one-shot; `NoPriorRow` is silent

**WHY 1C — symptom**: When `find_prior_row` returns `Ok(None)`, the entire exit-event lifecycle is dropped: no row written, no event broadcast, no error log, no telemetry counter incremented, no retry. The observer just goes back to `rx.recv()`.
- Evidence: `exit_observer.rs:225-228` (full text):
  ```
  RetryOutcome::NoPriorRow => {
      // No prior row — event dropped (alloc never
      // reached Running per the observer's vantage point).
  }
  ```
  The variant is consumed; nothing is emitted; the loop iterates.

**WHY 2C — context**: `handle_exit_event` returns `Ok(None)` at `exit_observer.rs:413` for the no-prior-row case, which `run_with_retry` (`exit_observer.rs:302-304`) maps to `RetryOutcome::NoPriorRow` directly — not to a retryable error. The retry loop introduced May 2 is keyed on `Err(HandleError::Observation)` from a *write* failure (`exit_observer.rs:305`); a *read* miss (no prior row) does not enter that loop.
- Evidence: `exit_observer.rs:298-308` — the `match` distinguishes three outcomes: `Ok(Some(...))` writes; `Ok(None)` early-returns `NoPriorRow`; `Err(HandleError::Observation(err))` enters retry. The May-2 retry mechanism explicitly does not cover the read-miss branch.

**WHY 3C — system**: `find_prior_row` is a single point-read with no retry, no subscription, no deferral, and no error escalation when it returns `None`. `NoPriorRow` is treated as a terminal "alloc never reached Running per the observer's vantage point" — the rustdoc at `exit_observer.rs:404-406` says this explicitly:
  > "Returns `Ok(None)` when no prior row exists for the alloc (the alloc never reached Running per the observer's vantage point — only possible under racy injection in tests; production drivers always emit Running through the action shim before any exit event can fire)."

  The rustdoc itself encodes the assumption that defines the bug — "production drivers always emit Running through the action shim before any exit event can fire" — and labels the counter-case "only possible under racy injection in tests." The sub-ms-exit case is the production counter-case the rustdoc disclaims.
- Evidence: `exit_observer.rs:399-406` (rustdoc) + `exit_observer.rs:487-492` (function body). The function body has zero defensive logic.

**WHY 4C — design**: The observer's design assumption — "by the time an `ExitEvent` arrives, a `Running` row exists" — is unverified in production and is *concealed* in the test suite by `sleep 0.5` rather than *enforced* in the code. The May-2 fix (`fix-exit-observer-write-retry`) recognised that the *write* path can fail transiently and added bounded retry; it did not recognise that the *read* path of `find_prior_row` can fail transiently for the symmetric reason (the row hasn't arrived yet) and add the symmetric retry.
- Evidence: predecessor RCA `fix-exit-observer-write-retry/deliver/rca.md` covers writes (lines 9-50, 68-83). The word "read" appears once (line 104) only in the context of "re-read on retry to keep LWW counter monotonic" — i.e., re-reading on the write-side retry, not retrying a missed read. The branch was not investigated.
- The `NoPriorRow` variant at `exit_observer.rs:270` carries no diagnostic data (no alloc-id capture for telemetry, no log call). The branch arm at `exit_observer.rs:225-228` is intentionally empty.

**WHY 5C — root cause**: **`NoPriorRow` is a typed terminal verdict treated as benign ("only possible under racy injection in tests"), but production drivers CAN deliver an `ExitEvent` before the action shim's `Running` row has landed. The variant should be a transient signal that triggers retry/wait/subscribe, OR the observer's contract should require the action shim's `Running` write to complete before the watcher's `ExitEvent` can be delivered.**
- → **ROOT CAUSE C**: Consumer-side gap — `find_prior_row` is one-shot; `NoPriorRow` is silently absorbed; no telemetry, no retry, no log.

### 2.4 Idempotency / observability branch (D) — silent drop is invisible

**WHY 1D — symptom**: An operator looking at logs, metrics, or `submit --watch` output for a sub-ms-exit Job sees no error — just an alloc stuck in `Running` forever (or until reconciler-driven restart on a different code path) and a CLI that never reports a verdict.
- Evidence: `exit_observer.rs:225-228` — no `tracing::error!`, no `tracing::warn!`, no `tracing::info!`, no counter increment. Compare to the symmetric `RetryOutcome::Failed` arm at lines 229-247 which does emit `tracing::error!` AND a degraded `LifecycleEvent`.

**WHY 2D — context**: The May-2 RCA explicitly added the "louder failure semantics than `warn!`" branch for write-exhaustion, on the rationale that the Phase 1 obs-as-truth design makes silent drops a control-plane invariant violation (predecessor RCA, contributing factor #4). The `NoPriorRow` arm did not get the same treatment because the rustdoc disclaims production-reachability ("only possible under racy injection in tests").
- Evidence: predecessor RCA `fix-exit-observer-write-retry/deliver/rca.md:118` ("Single observation surface design ... means a write failure is a control-plane invariant violation with no fallback channel. Acceptable for Phase 1 scope but warrants louder failure semantics than `warn!`."). The same logic applies one branch over but was not generalised.

**WHY 3D — system**: There is no DST invariant of the form "every `ExitEvent` produces either an obs-row write OR a degraded `LifecycleEvent` OR a structured error log." Such an invariant would have failed under any DST seed that ran the watcher's emission ahead of the shim's write — and that ordering is reachable under any non-trivial scheduler permutation.
- Evidence: predecessor RCA `fix-exit-observer-write-retry/deliver/rca.md:107-109` named this gap explicitly as "DST invariant currently absent: 'every running alloc whose exit-event fired eventually shows non-Running in obs OR a terminal-failure event is broadcast.'" The `2026-05-02-fix-exit-observer-write-retry.md` evolution doc at line 64 lists it as still open.
- Confirmation: the gap was *named* in May 2 and *not closed*. The `NoPriorRow` branch is the second silent-drop the same gap permits.

**WHY 4D — design**: The Phase 1 design treats the obs store as the durable surface but treats the lifecycle bus as best-effort/transient. There is no "the observer must emit *something* per `ExitEvent` consumed" structural contract — neither in the trait surface, in a DST invariant, nor in a runtime assertion.
- Evidence: `exit_observer.rs:165-251` — the `tokio::spawn(async move { … })` body has three outcomes (`Wrote`, `NoPriorRow`, `Failed`) and only two of them emit on the bus. The third is silent by construction. No type-level mechanism ensures the third never happens, and no observability mechanism surfaces when it does.

**WHY 5D — root cause**: **`NoPriorRow` is a structurally silent failure mode. The May-2 fix recognised that *write* failures are control-plane invariant violations and gave them loud semantics; it did not generalise the principle to "every `ExitEvent` must produce a visible outcome." The same gap noted in the May-2 RCA contributing factors (#3, "no DST invariant for exit event → eventual obs convergence") is the load-bearing missing rail this report's race exploits.**
- → **ROOT CAUSE D**: Observability gap — the `NoPriorRow` branch emits no signal, no DST invariant catches it, and the rustdoc's "only possible under racy injection in tests" disclaimer became prophecy: it is reachable in production but suppressed by fixture wall-clock workarounds rather than fixed.

---

## 3. Map of branches → root causes

| Branch | Root cause | One-line shape |
|---|---|---|
| A — Producer ordering | **A. Producer-side ordering gap** | Action shim's `Running` write has no happens-before edge against the watcher's `ExitEvent` emission. |
| B — Transport/store | **B. Store rendezvous gap** | The obs store's read API is point-read; no per-key happens-before / wait-for-row primitive backs the observer's reader-becomes-writer dependency. |
| C — Consumer | **C. Consumer one-shot read + silent verdict** | `find_prior_row` is one-shot; `NoPriorRow` is treated as terminal/benign. |
| D — Observability | **D. Silent failure mode + missing DST invariant** | The May-2 "loud failure semantics" principle was not generalised to the read-miss branch; the DST invariant the predecessor RCA flagged is still open. |

### 3.1 Cross-validation

- A + B: consistent. A is the producer asymmetry; B is why the store's API surface cannot fix it transparently. They compose.
- B + C: consistent. B says "the store has no happens-before"; C says "the consumer assumes one." Together they explain why the bug manifests as a missed read rather than (e.g.) a deadlock or a stale row.
- C + D: consistent. C is the typed-verdict choice; D is the observability consequence. C says "we treat `None` as benign"; D says "and we emit nothing about it."
- A + C: A is sufficient *if* C's assumption (Running always lands first) is held; C's assumption fails under A, so the bug requires both. Removing either alone closes the symptom; the May-2 RCA's "all root causes addressed" discipline argues for closing both, since the second one bites again on the next variant of A (a new driver, a new shim arm, a new emission path).

### 3.2 Symptoms collectively explained?

Yes:

| Observed symptom | Root cause(s) |
|---|---|
| Sub-ms-exit workload's exit event silently dropped | A + C |
| `sleep 0.5` is load-bearing in test fixtures | A (the wall-clock window the sleep widens) |
| Observer emits no log line for the dropped event | D |
| Reconciler never reclassifies the alloc; CLI hangs / mis-renders | B (no fallback channel) + downstream of A+C |
| The May-2 fix did not catch this branch despite same author + same file | D (gap was named but the principle was not generalised across both the read-miss and write-fail branches) |

---

## 4. Solution proposals

Each solution names which root causes it addresses. At least one is DST-clean (no real-clock dependency).

### Solution 1' — Oneshot-gated watcher emission (RECOMMENDED)

**Sketch**: Establish a structural happens-before edge between `obs.write(Running)` and the watcher's first `ExitEvent` emission via a `tokio::sync::oneshot` channel.

- The watcher does `child.wait().await` as today, but before sending on `exit_tx` it awaits a `tokio::sync::oneshot::Receiver<()>` ("Running-confirmed" gate).
- The driver returns the corresponding `oneshot::Sender<()>` alongside the existing `AllocationHandle` (or stashes it in `LiveAllocation` and exposes a `release_for_exit_emission()` method that consumes/sends on the sender).
- The action shim, after `obs.write(Running)` resolves successfully, fires the corresponding `oneshot::Sender`.
- The ordering edge becomes structural and verifiable in source: `obs.write(Running) commits → oneshot fires → watcher emits ExitEvent`.

**Covers**: Root cause A (closed by structural happens-before). Roots B/C/D's symptom paths become structurally unreachable in the canonical emission path (the watcher cannot emit before the gate opens, and the gate doesn't open until `obs.write(Running)` commits, so `find_prior_row` cannot fire before the row is visible). B/C/D's underlying gaps remain latent — call this out honestly: they are no longer reachable through the action-shim/exec-driver path, but a future emitter (a different driver shape, a sim-injection path that bypasses the action shim) could re-expose them.

**Trade-offs**:
- (+) **DST-clean by construction**: `tokio::sync::oneshot` is not `Clock`-dependent. Works under `SimClock`, turmoil, and real tokio identically — the gate is a logical happens-before edge, not a wall-clock budget.
- (+) **Closes Root A by structural happens-before, not wall-clock budget**. The fix lives in source, is verifiable by reading the action shim and the watcher in two adjacent files, and does not depend on any timing assumption.
- (+) **Drives B/C/D's symptom paths to unreachable**: `find_prior_row` cannot fire before the shim's `Running` write has committed (because the watcher cannot emit `ExitEvent` before the gate opens, and the gate doesn't open until the write commits, and the observer's `find_prior_row` only runs in response to a delivered `ExitEvent`).
- (+) **Smaller blast radius than the original Solution 1**: no two-stage `Driver::start` split, no replacement-row semantics on `StartRejected`, no driver-trait surface inversion. The driver returns one extra value (or stashes one extra field); the action shim fires the gate after the existing `obs.write` call; the watcher awaits one extra thing before its existing `exit_tx.send`. Three small changes, three adjacent call sites.
- (+) Preserves the Phase 1 obs-as-truth contract; the `Running` row is still the only durable signal that the alloc reached Running.
- (–) **Action-shim crash between `driver.start` resolving and `obs.write(Running)` succeeding leaves the watcher parked on the gate.** This produces an orphan process / parked watcher condition that today's failure mode already exhibits (the alloc has no Running row and no terminal row; reconciler convergence handles it on the next tick). Same blast radius as today; out of scope for this fix to fix harder. Calling this out so the user can weigh it: it is not a regression, it is a no-change.
- (–) **The May-2 `obs.write(Running)` retry path interacts**: the gate must fire after the FINAL successful retry, not after the first attempt. If the write exhausts retries and degrades to `LifecycleEvent`-only (per the May-2 RCA's escalation path), **the gate must still fire** — otherwise the watcher leaks forever waiting on a oneshot that nothing will ever send. Fire-on-degraded-escalation is required for liveness. The action-shim wiring is: fire the gate after `obs.write(Running)` resolves Ok, AND fire the gate after the May-2 retry exhausts and degrades (just before the degraded `LifecycleEvent` emission). Two firing sites, both structurally necessary; either is fine for the watcher because the gate carries no payload.
- (–) **Existing exit-observer integration tests need updating**: the tests at `tests/integration/job_lifecycle/exit_observer.rs` that exercise the `NoPriorRow` branch via racy injection (driving `ExitEvent` ahead of any `Running` row) are exercising a path that becomes structurally unreachable in the canonical action-shim flow. Some of those tests are testing the observer-receiver contract (still valid: prove the observer handles a delivered ExitEvent correctly given a present prior row); some are testing the now-impossible producer-ordering path (they must be re-shaped or deleted). §7 distinguishes the two cases.

### Solution 2 — `find_prior_row` waits for the row instead of one-shot reading (DEFENCE-IN-DEPTH FALLBACK; not recommended as primary fix)

**Sketch**: Change `find_prior_row` to a bounded wait: on miss, subscribe to the `ObservationStore`'s row-write stream (or poll on a `Clock::sleep` cadence), retry up to N attempts within a budget, and then either succeed (row arrived) or escalate via the same degraded-`LifecycleEvent` path the May-2 fix added. Mirror the May-2 retry loop's shape (`run_with_retry` at `exit_observer.rs:290-325`): bounded, `Clock`-injected, escalating-on-exhaustion.

**Reframed role**: Solution 2 is a **tolerator**, not a fixer. It absorbs the producer-side ordering gap (Root A) on the consumer side instead of closing it; it pays for that absorption with a wall-clock retry budget on the exit-event handling path. **The user's correction is exactly right: this just moves the wall-clock concealment from `sleep 0.5` in the workload bash into a `Clock::sleep` retry loop in the observer. Same shape, different layer.** Landing it instead of Solution 1' would be the higher-quality bandaid the user pushed back on.

It stays in this document as a **defence-in-depth fallback** for the case where Solution 1' cannot fire the gate — for example, a future driver shape that genuinely cannot expose a oneshot, or a sim-injection path that deliberately bypasses the action shim. In those (currently hypothetical) cases, Solution 2's bounded retry guards against the symptom re-emerging. It is not a substitute for Solution 1'.

**Covers**: Root causes B (the store read API gains a wait-for primitive at the call site that needs it), C (`NoPriorRow` is no longer terminal/benign — it triggers retry/escalation), and D (escalation path emits a degraded `LifecycleEvent` and `tracing::error!`). Does **not** close Root A — the underlying ordering gap remains.

**Trade-offs**:
- (+) DST-clean — uses the existing `Clock` injection that the May-2 fix already threads through `spawn_with_runtime`.
- (+) Symmetric with the May-2 fix; one mental model for both branches of the observer's I/O.
- (+) Does not change the action-shim contract or the driver trait; localised to `exit_observer.rs`.
- (–) **Does not address Root cause A** — the underlying ordering gap remains, and any future emitter that races the shim faces the same problem. The retry just moves wall-clock concealment from `sleep 0.5` in the workload bash into a `Clock::sleep` retry budget in the observer.
- (–) Adds wall-clock budget to the exit-event handling path. Backoff should match May-2's 50/100/200ms shape; total worst-case is ~350ms before escalation. Production workloads exceed this; pathological sub-ms-exit + slow shim case is the only one paying the latency.
- (–) Requires a new `ObservationStore::subscribe_alloc(alloc)` or polling primitive. Polling via `Clock::sleep` is the smaller change; subscription is the structurally cleaner one but expands the trait.

### Solution 3 — Treat `ExitEvent` carrying enough state for the observer to write WITHOUT a prior row read

**Sketch**: Extend `ExitEvent` so the watcher emits `(alloc, job_id, node_id, kind, intentional_stop, stderr_tail, prior_counter_or_zero)` and the observer no longer needs to read the prior row to recover `(job_id, node_id, counter)`. The driver knows `(alloc, job_id, node_id)` because the action shim told it on `Driver::start(&spec)` — `spec.alloc` already exists, and the driver can be passed `(job_id, node_id)` at start. The counter starts at 0 if no prior row exists; LWW dominance then sorts itself out when the shim's `Running` write (counter 1) arrives later.

**Covers**: Root cause B (the observer no longer has a reader-becomes-writer dependency); partial C (no `NoPriorRow` branch needed at all since the read is gone).

**Trade-offs**:
- (+) DST-clean. Pure data-flow rearrangement.
- (+) Eliminates the `find_prior_row` call entirely from the exit-event path — turns the consumer-becomes-producer into pure-producer.
- (–) LWW dominance under "exit row arrives before Running row" needs careful thinking: the Failed/Terminated row's counter is 0; the Running row's counter is whatever the shim picks (today via `find_prior_alloc_row` at `action_shim/mod.rs:457-459` defaulting to Pending=0+1=1). Result: Running (counter 1) dominates Failed (counter 0) on the snapshot — wrong terminal verdict on read.
- (–) Fixing the dominance requires the observer to always pick a counter HIGHER than any plausible Running row, OR the action shim to recognise "an exit row already exists; my Running write must be skipped or downgraded." Either is a structural change beyond a simple data-flow rearrangement.
- (–) `LifecycleEvent.terminal: None` and `from` field both depend on knowing the prior state; without the read, `from` defaults to `Pending` even when `Running` was the actual prior — breaks the regression test at `tests/integration/job_lifecycle/exit_observer.rs:493-544` ("exit_observer_lifecycle_from_reflects_prior_running_state").

Solution 3 looks attractive structurally but the LWW interaction makes it costlier than Solution 2 in practice.

### Solution 4 — DST invariant: every consumed `ExitEvent` produces a visible outcome

**Sketch**: Add a DST invariant — `assert_eventually!("every ExitEvent produces an obs row write OR a terminal-failure LifecycleEvent OR a structured error log", …)` — to the simulation harness. Whichever solution above lands, this invariant guards the regression. This is the gap predecessor RCA `fix-exit-observer-write-retry/deliver/rca.md:107-109` named and `2026-05-02-fix-exit-observer-write-retry.md:64` left open.

**Covers**: Root cause D (the observability/structural-guard gap that turned a single fix into a fix-then-need-second-fix sequence).

**Trade-offs**:
- (+) DST-clean by construction; the invariant runs against `SimClock` + `SimObservationStore`.
- (+) Closes the gap once for all current and future emission paths through the observer.
- (–) Does not fix the symptom alone — it catches the symptom on the next regression. Pair with one of solutions 1/2/3.
- (–) Requires DST-side instrumentation: counting `ExitEvent`s consumed by the observer, counting writes/escalations emitted, asserting equality. Modest plumbing.

---

## 5. Back-chain validation

For each solution, walk back up each WHY chain and assert closure.

### Solution 1' (oneshot-gated watcher emission)

| Level | A — producer | B — store | C — consumer | D — observability |
|---|---|---|---|---|
| 5 (root) | **CLOSES** — structural happens-before via `tokio::sync::oneshot`: `obs.write(Running) commits → oneshot fires → watcher emits ExitEvent`. The ordering is sourced in code, not in a wall-clock budget. | symptom path **UNREACHABLE** — `find_prior_row` cannot fire before the row is visible because the watcher cannot emit `ExitEvent` before the gate opens, and the gate doesn't open until `obs.write(Running)` commits. The store-rendezvous gap remains latent: a future emitter that bypasses the gate (different driver, sim-injection path) could re-expose it. | symptom path **UNREACHABLE** — `Ok(None)` cannot be returned from `find_prior_row` in the canonical action-shim flow because the prior row is committed before the event is delivered. The one-shot read + silent verdict gap remains latent for any non-canonical emission path. | symptom path **UNREACHABLE** in the canonical flow (no silent drop because `NoPriorRow` is unreachable); the underlying observability gap remains latent. Solution 4 closes this latent gap structurally. |
| 4 (design) | **CLOSES** — the wall-clock assumption ("workloads run long enough") is replaced by an explicit ordering edge in the source. | unchanged at the design layer; symptom path closed by A. | unchanged at the design layer; symptom path closed by A. | unchanged at the design layer; symptom path closed by A. Solution 4 closes the design-level gap. |
| 3 (system) | **CLOSES** — happens-before edge between shim write and watcher emission, expressed as a oneshot the verifier/reader can trace by reading three adjacent files. | unchanged at the system layer; symptom path closed by A. | unchanged at the system layer; symptom path closed by A. | unchanged at the system layer; symptom path closed by A. |
| 2 (context) | **CLOSES** — the action shim's contract gains a structural post-condition: "after `obs.write(Running)` resolves Ok (or after May-2 retry exhausts and degrades), fire the gate." The watcher's contract gains a structural pre-condition: "await the gate before emitting `ExitEvent`." | symptom path closed by A. | symptom path closed by A. | symptom path closed by A. |
| 1 (symptom) | **CLOSES**. | **CLOSES** in the canonical flow. | **CLOSES** in the canonical flow. | **CLOSES** in the canonical flow; latent gap remains for non-canonical emission paths. |

**Verdict**: Solution 1' closes Root A by structural happens-before and drives Roots B/C/D's *symptom paths* to unreachable in the canonical action-shim/exec-driver flow. The underlying gaps in B/C/D remain latent — a future emitter that bypasses the gate (a different driver shape, a sim-injection path, a hypothetical retry path that re-emits `ExitEvent`) could re-expose them. Solution 4 (DST invariant) is the structural guard that ensures any such regression fails loudly under DST rather than silently in production. **1' alone closes the symptom; 1' + 4 closes the symptom AND prevents recurrence through any future emission path.**

### Solution 2 (consumer-side wait-with-escalation)

| Level | A — producer | B — store | C — consumer | D — observability |
|---|---|---|---|---|
| 5 (root) | not addressed; ordering gap remains, but the consumer now tolerates it. | **CLOSES** — observer has a wait-for-row primitive at the call site that needs it. | **CLOSES** — `NoPriorRow` becomes transient; retry-then-escalate. | **CLOSES** — escalation emits degraded `LifecycleEvent` + `tracing::error!`, mirroring May-2. |
| 4 (design) | unchanged. | **CLOSES**. | **CLOSES**. | **CLOSES** in this code path; the principle "every `ExitEvent` produces a visible outcome" is now structural for this branch. |
| 3 (system) | unchanged. | **CLOSES**. | **CLOSES**. | **CLOSES**. |
| 2 (context) | unchanged — the race window still exists but is bounded. | **CLOSES** — bounded wait. | **CLOSES**. | **CLOSES**. |
| 1 (symptom) | symptom-path closed via the consumer-side. | **CLOSES**. | **CLOSES**. | **CLOSES**. |

**Verdict**: Solution 2 alone closes the symptom AND addresses Roots B, C, D. Root A (the producer-side ordering gap) remains; the wait absorbs it. Acceptable iff the wait budget is structurally sufficient.

### Solution 3 (event carries enough state)

| Level | A | B | C | D |
|---|---|---|---|---|
| 5 | not addressed | **CLOSES** for this code path | partial; the read goes away so the verdict goes away | not addressed |

**Verdict**: Solution 3 has a real LWW-dominance regression (Running row dominates Failed on later arrival) that the analysis surfaces. Combined with the regression-test breakage on `from`-field semantics, it is structurally inferior to Solution 2 absent a counter-allocation strategy that solves the dominance problem. **Not recommended on its own.**

### Solution 4 (DST invariant)

| Level | A | B | C | D |
|---|---|---|---|---|
| 5 | not addressed | not addressed | not addressed | **CLOSES** the gap that the May-2 RCA left open. |

**Verdict**: Solution 4 is a regression-class guard, not a symptom fix. Its role is to ensure the next bug in this class fails loudly under DST rather than silently in production. **Always combine with one of 1/2/3.**

### Combination matrix

Legend: ✓ = root cause closed structurally; ⊘ = symptom path made unreachable but underlying gap remains latent; – = not addressed.

| Combination | A | B | C | D | Symptom? | DST-clean? |
|---|---|---|---|---|---|---|
| 1' alone | ✓ | ⊘ | ⊘ | ⊘ | YES | ✓ |
| **1' + 4 (RECOMMENDED)** | ✓ | ⊘ | ⊘ | ✓ | YES | ✓ |
| 2 alone (defence-in-depth only) | – | ✓ | ✓ | ✓ | YES | ✓ |
| 2 + 4 (tolerator combination — not recommended; A remains open) | – | ✓ | ✓ | ✓ | YES | ✓ |
| 1' + 2 + 4 (1' as fixer, 2 as defence-in-depth, 4 as guard) | ✓ | ✓ | ✓ | ✓ | YES | ✓ |

The recommended combination is **1' + 4**: 1' closes Root A by structural happens-before; 4 closes Root D's design-level gap and ensures any future emission path that bypasses the gate fails loudly under DST. Roots B and C have their symptom paths made unreachable by 1'; their underlying gaps are latent but guarded by 4. Solution 2 is a tolerator and is not recommended as a primary fix; it stays in this document only as a defence-in-depth fallback for hypothetical future emission paths that genuinely cannot expose a oneshot.

---

## 6. Relationship to the May-2 `fix-exit-observer-write-retry` RCA

### 6.1 What overlaps

- Both RCAs pivot on the same architectural seam: the exit observer is the single durable channel from process exit to reconciler-visible state, and the May-2 RCA's contributing factor #3 ("no DST invariant for exit event → eventual obs convergence", `fix-exit-observer-write-retry/deliver/rca.md:117`) is the same gap this RCA's Root D names.
- Both find that "log and forget" / "drop and forget" silent absorption is the failure mode (May-2: write-failure absorbed at `warn!`; this RCA: read-miss absorbed silently).
- The May-2 fix's structural shape — bounded retry + `Clock` injection + escalation via degraded `LifecycleEvent` — is directly applicable to this RCA's Solution 2.

### 6.2 What's new

- **The race surface**: May-2 dealt with ObservationStore-side write rejection (a writer-internal failure mode). This RCA deals with cross-producer ordering between two writers (action shim, exit observer) — a *system-level* failure mode the predecessor RCA did not consider.
- **The producer side**: May-2 did not investigate `action_shim/mod.rs:472-499` ordering. The producer-side root cause (A) is genuinely new.
- **The transport/store rendezvous gap (B)**: May-2 treated the obs store as the rendezvous and the failure mode as "the rendezvous itself rejected a write." This RCA shows the rendezvous lacks happens-before semantics across producers — a different layer of the same surface.

### 6.3 What May-2 should have caught but didn't

- **Generalise the principle**. May-2's RCA correctly identified that a silent drop on the only durable signal channel is a Phase 1 invariant violation warranting loud semantics. It applied the principle to the *write* branch and noted in its own contributing-factors section that the missing DST invariant was a contributing cause. It did not survey *all* outcome variants of `handle_exit_event`/`run_with_retry` — specifically, the `Ok(None) → NoPriorRow` branch — for the same silent-drop pattern. A "completeness check at every level" (per the 5-Whys methodology) on the May-2 investigation would have flagged the read-miss branch as the symmetric case to the write-fail branch.
- **Close the DST gap then, not now**. May-2's RCA listed the missing DST invariant as a contributing factor (#3) and the May-2 evolution doc lists it as still open (`2026-05-02-fix-exit-observer-write-retry.md:64`). Had it landed, the K1 honesty test (`coinflip_honesty_100_trials.rs`) would have failed under any DST seed that reordered watcher emission ahead of shim write — and the sub-ms-exit production path would have been flagged before the `sleep 0.5` workaround was naturalised across the test suite.
- **Not surveyed**: the rustdoc disclaimer at `exit_observer.rs:399-406` ("only possible under racy injection in tests") was already present in May-2's diff; it warranted scrutiny then. A reviewer applying "interrogate what the design assumed that the trigger violated" (the project's own RCA discipline per `.claude/rules/debugging.md` § 1) would have asked whether the assumption was empirically true under all production driver shapes — and `ExecDriver`'s `spawn_exit_watcher` (`crates/overdrive-worker/src/driver.rs:391-397`, present at May-2 commit time) emits without coordination with the action shim.

### 6.4 Layering claim

This RCA does not contradict or supersede May-2. It is the next branch on the same tree: May-2 closed the write-failure leg; this RCA exposes the read-miss leg. The recommended fix (§7 below) does *not* reuse May-2's bounded-retry-then-escalate machinery on the read path — that shape was the original Solution 2, reframed in §4 as a tolerator after user pushback. Instead, the recommended fix (Solution 1') closes the producer-side ordering gap by structural happens-before, which makes the read-miss path unreachable in the canonical flow. The May-2 retry machinery is *interacted with* by the fix (the gate must fire after the May-2 retry exhausts and degrades, for liveness — see §7) but is not extended to the read path.

---

## 7. Recommendation

**Land Solution 1' + Solution 4 in a single fix.**

Reasoning chain:

1. **Root A is THE producer-side ordering gap.** The action shim establishes no happens-before edge between `obs.write(Running)` and the watcher's first `ExitEvent` emission. Today's "Running before exit" ordering is a wall-clock coincidence, not a structural guarantee. Every other root cause in this report is a downstream consequence of A.

2. **Roots B, C, and D are downstream consequences of A.** B (store rendezvous gap) only manifests because A allows the read to fire before the write commits. C (one-shot read + silent `NoPriorRow`) only manifests because A allows `find_prior_row` to return `Ok(None)` for a real production trajectory. D (silent failure mode) only manifests because A makes the silent branch reachable. **Fix A and the symptom paths for B/C/D become structurally unreachable.** Their underlying gaps remain latent — that is what Solution 4 is for.

3. **Solution 1' closes Root A by structural happens-before.** A `tokio::sync::oneshot` channel between the action shim (sender, fired after `obs.write(Running)` resolves Ok or after May-2 retry exhausts and degrades) and the watcher (receiver, awaited before `exit_tx.send`) makes the ordering edge sourced in code, not in a wall-clock budget. **The fix lives in three adjacent call sites and is verifiable by reading them in sequence.** No driver-trait surface inversion; no two-stage `Driver::start` split; no replacement-row semantics on `StartRejected`.

4. **Solution 4 (DST invariant) guards against the latent B/C/D gaps re-emerging.** If a future emission path bypasses the gate (a different driver shape, a sim-injection path, a hypothetical retry path that re-emits `ExitEvent`), the symptom would re-emerge. The DST invariant — "every `ExitEvent` consumed by the observer produces an obs-row write OR a degraded `LifecycleEvent` OR a structured error log" — fails loudly the moment any such regression lands. This is the gap predecessor RCA `fix-exit-observer-write-retry/deliver/rca.md:107-109` named and `2026-05-02-fix-exit-observer-write-retry.md:64` left open; landing it now closes a debt the May-2 RCA flagged.

5. **Solution 2 is explicitly a tolerator and is not recommended.** Quoting the user's correction directly: *"the retry just moves wall-clock concealment from the workload bash into the observer's retry loop. Same shape, different layer."* Solution 2 absorbs Root A on the consumer side instead of closing it; it pays for that absorption with a wall-clock retry budget on the exit-event handling path. It stays in this document as a defence-in-depth fallback for hypothetical future emission paths that cannot expose a oneshot — not as a substitute for closing A.

6. **Solution 3 stays rejected** absent a counter-allocation strategy that prevents the Running-dominates-Failed LWW inversion (see §4 Solution 3 trade-offs). Not viable as proposed.

**Concrete delivery shape**:

| File | Change |
|---|---|
| `crates/overdrive-worker/src/driver.rs` | `Driver::start` returns `(AllocationHandle, oneshot::Receiver<()>)` — or, equivalently, the receiver is stashed in `LiveAllocation` and exposed via `release_for_exit_emission()` consumed by the action shim. The watcher (`spawn_exit_watcher`, lines 555-638) awaits the receiver after `child.wait().await` resolves and before `exit_tx.send(event).await` (line 638). On receiver-dropped (action shim crashed before firing), the watcher logs `tracing::error!` and exits without emitting — the orphan-process condition is identical to today's failure mode and is handled by reconciler convergence on the next tick. |
| `crates/overdrive-control-plane/src/action_shim/mod.rs` | After `obs.write(Running)` at line 499 resolves Ok, fire the corresponding `oneshot::Sender`. **Liveness requirement**: on May-2 write-retry exhaustion that degrades to `LifecycleEvent`-only, fire the sender anyway (just before the degraded `LifecycleEvent` emission) — otherwise the watcher leaks forever waiting on a oneshot that nothing will ever send. Two firing sites (Ok path, degraded path); both structurally necessary. The sender carries no payload; firing twice is impossible because the sender is consumed on send. |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/exit_observer.rs` | **New tests**: assert that the watcher cannot emit `ExitEvent` before the `Running` row is committed, including under DST schedules that would have raced. At minimum: (a) `watcher_emission_blocks_until_running_row_commits` — drive a sub-ms-exit workload through the action shim, assert observer-side `find_prior_row` always sees a present row when an `ExitEvent` is delivered. (b) `watcher_emits_after_degraded_running_write_path` — force May-2 retry exhaustion, assert the watcher still emits and the observer escalates correctly. **Update existing tests**: tests in this file that exercised `NoPriorRow` via racy injection (driving `ExitEvent` ahead of any `Running` row) must be triaged: tests defending the observer-receiver contract (does the observer correctly handle a delivered `ExitEvent` given a present prior row?) stay and are reshaped to drive a present row first; tests defending the now-impossible producer-ordering path (no prior row at all in the canonical flow) are deleted with an explicit rationale comment naming this RCA. If Solution 2 lands later as defence-in-depth, the deleted tests can be reincarnated to defend the bounded-retry path. |
| DST suite | **Solution 4 invariant** — `assert_eventually!("every ExitEvent produces a visible outcome", …)`: every `ExitEvent` consumed by the observer produces (obs row write) ∨ (degraded `LifecycleEvent`) ∨ (structured error log). With Solution 1' landed, the invariant should never fire under the canonical flow; its load-bearing role is guarding latent B/C/D from re-emerging through a future emission path that bypasses the gate. Closes the predecessor RCA's open gap. |
| `crates/overdrive-cli/tests/integration/coinflip_honesty_100_trials.rs` and `crates/overdrive-cli/tests/integration/job_kind_streaming.rs` | After Solution 1' + 4 lands and the DST invariant passes: remove `sleep 0.5` from the bash fixture bodies (lines 128 / 235 / 255). Remove the rustdoc paragraphs disclaiming the workaround (lines 103-117 / 218-227) at the same time. |

**Project-policy guardrails honoured** (per CLAUDE.md, `.claude/rules/development.md`, `.claude/rules/testing.md`):

- **DST cleanliness**: `tokio::sync::oneshot` is not `Clock`-dependent. The gate is a logical happens-before edge that works under `SimClock`, turmoil, and real tokio identically. No `tokio::time::sleep` on the production hot path; no real-clock dependency in tests.
- **"Production code is not shaped by simulation"** (`development.md`): the oneshot gate is structural ordering for a real production race, not a sim-shape concession. The watcher would await the gate identically under any runtime.
- **"Persist inputs, not derived state"** (`development.md`): no derived-cache field added; the gate is a transient per-allocation synchronisation primitive that lives only for the duration of `LiveAllocation`.
- **"Trait definitions specify behavior, not just signature"** (`development.md`): the `Driver::start` contract gains an explicit post-condition — "the returned `oneshot::Receiver<()>` is fired exactly once after the corresponding `Running` row is committed (or after the May-2 retry path degrades to `LifecycleEvent`-only)." This goes in the trait docstring; the equivalence between `ExecDriver` and `SimDriver` is enforced by the existing DST harness plus the new Solution 4 invariant.
- **Deferrals require GitHub issues and user approval** (CLAUDE.md): per project rules, **this report does not create issues**. Deferral candidates surfaced for user decision:
  1. **Solution 2 (defence-in-depth bounded retry)** — recommend tracking as a follow-up issue *only if* a future emission path is identified that genuinely cannot expose a oneshot. Awaiting user approval before any `gh issue create`. Until such a path exists, no issue is needed.
  2. **Solution 3 (event-carries-state)** — explicitly rejected on technical grounds; no issue needed unless the LWW-dominance problem is independently resolved later.

**The K1 honesty test (`coinflip_honesty_100_trials.rs:294-302`) is the load-bearing observability KPI for this fix.** When Solution 1' + 4 lands, the K1 trial should pass at threshold ≥99/100 with `sleep 0.5` removed from the workload. Until that test passes without the workaround, the fix is not delivered.

---

## 8. Files cited

- `crates/overdrive-control-plane/src/worker/exit_observer.rs` (lines 117-251, 270, 290-325, 399-406, 411-446, 487-492)
- `crates/overdrive-control-plane/src/action_shim/mod.rs` (lines 311, 363-371, 444, 448-501, 499)
- `crates/overdrive-worker/src/driver.rs` (lines 161-167, 199-206, 376-402, 405-444, 529-530, 555-638)
- `crates/overdrive-cli/tests/integration/coinflip_honesty_100_trials.rs` (lines 103-117, 128, 294-302)
- `crates/overdrive-cli/tests/integration/job_kind_streaming.rs` (lines 218-227, 235, 255)
- `crates/overdrive-control-plane/tests/integration/job_lifecycle/exit_observer.rs` (lines 79-154, 198-286, 491-544)
- `crates/overdrive-control-plane/tests/integration/exit_observer_stderr_tail.rs` (lines 80-99, 122-184)
- `docs/feature/fix-exit-observer-write-retry/deliver/rca.md` (lines 9-50, 68-83, 85-94, 107-109, 114-119)
- `docs/evolution/2026-05-02-fix-exit-observer-write-retry.md` (lines 17, 21-27, 64)
