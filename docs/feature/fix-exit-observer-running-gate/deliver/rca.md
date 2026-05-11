# RCA — exit observer races action-shim `obs.write(Running)` for sub-millisecond-exit workloads

**Status**: Approved fix direction (Solution 1' + Solution 4) — 2026-05-10
**Reporter**: rustdoc on `crates/overdrive-cli/tests/integration/coinflip_honesty_100_trials.rs:103-117` (and parallel `job_kind_streaming.rs:218-227`) flagging the `sleep 0.5` workaround as a load-bearing concealment of a Phase 1 observer race.
**Investigator**: nw-troubleshooter (Rex)
**Predecessor RCA**: `docs/feature/fix-exit-observer-write-retry/deliver/rca.md` (May-2 fixed the symmetric *write*-failure leg of the silent-drop tree; this RCA fixes the *read*-miss leg.)
**Full analysis**: `docs/analysis/root-cause-analysis-exit-observer-prior-row-race.md` — five-Whys investigation across four causal axes (producer ordering, store rendezvous, consumer one-shot read, observability), four solution proposals, back-chain validation, May-2 layering. **Read it first**; this file is the actionable scope summary.

---

## Defect

For a workload whose process lifetime from `Driver::start` return through `child.wait()` resolution is shorter than the wall-clock window between (a) the action shim returning from `driver.start(&spec).await` and (b) the action shim completing `obs.write(ObservationRow::AllocStatus(Running))`:

- The exit-watcher emits an `ExitEvent` before the action shim has written the `Running` row.
- `exit_observer::run_with_retry → handle_exit_event → find_prior_row(obs, &event.alloc)` returns `Ok(None)`.
- `RetryOutcome::NoPriorRow` is the empty arm at `exit_observer.rs:225-228` — no obs row, no `LifecycleEvent`, no log, no telemetry.
- The exit event is silently dropped. CLI `submit --watch` hangs or mis-renders the terminal verdict; KPI K1 (CLI exit-code honesty) degrades.

Today, every Job-kind acceptance fixture inserts `sleep 0.5` in the workload bash to widen the wall-clock window past the race. That is fixture-side concealment, not a fix.

The full WHY chains and evidence citations live in §2 of the analysis document. Four root causes, summarised:

| Root | Shape |
|---|---|
| **A. Producer-side ordering gap** | Action shim's `Running` write has no happens-before edge against the watcher's `ExitEvent` emission. Coincidence, not contract. |
| B. Store rendezvous gap | Obs store API is point-read; no per-key wait-for-row primitive backs the observer's reader-becomes-writer dependency. |
| C. Consumer one-shot read + silent verdict | `find_prior_row` is one-shot; `NoPriorRow` is treated as terminal/benign. |
| D. Silent failure mode + missing DST invariant | The May-2 "loud failure semantics" principle was not generalised to the read-miss branch; the DST invariant predecessor RCA flagged is still open. |

**B/C/D are downstream consequences of A.** Fix A and B/C/D's symptom paths become structurally unreachable; the underlying gaps remain latent.

---

## Approved fix — Solution 1' (oneshot-gated watcher emission) + Solution 4 (DST invariant)

### Solution 1' — Oneshot-gated watcher emission

Establish a structural happens-before edge between `obs.write(Running)` and the watcher's first `ExitEvent` emission via a `tokio::sync::oneshot` channel.

**Source-level shape**:

- `Driver::start` returns `(AllocationHandle, oneshot::Sender<()>)` (or stashes the sender in `LiveAllocation` and exposes a `release_for_exit_emission()` method that consumes/sends on it).
- The watcher does `child.wait().await` as today, then awaits a `oneshot::Receiver<()>` ("Running-confirmed" gate) **before** sending on `exit_tx`.
- The action shim, after `obs.write(Running)` resolves Ok, fires the corresponding `oneshot::Sender`.
- **Liveness rail (interaction with May-2)**: the May-2 `obs.write(Running)` retry path may exhaust retries and degrade to `LifecycleEvent`-only. In that path the gate **must still fire** (otherwise the watcher leaks forever waiting on a oneshot that nothing will ever send). Two firing sites: post-success and post-degraded-escalation. Either is fine for the watcher because the gate carries no payload.

The ordering edge becomes verifiable in source by reading three adjacent call sites: `obs.write(Running) commits → oneshot fires → watcher emits ExitEvent`.

**DST cleanliness**: `tokio::sync::oneshot` is not `Clock`-dependent; works under `SimClock`, turmoil, real tokio identically. The gate is a logical happens-before edge, not a wall-clock budget.

**Trade-off the user should weigh**: action-shim crash between `driver.start` resolving and `obs.write(Running)` succeeding leaves the watcher parked on the gate. Same orphan-process condition that exists today; reconciler convergence handles it on the next tick. Out of scope for this fix. Not a regression.

### Solution 4 — DST invariant: every consumed `ExitEvent` produces a visible outcome

Add `assert_eventually!("every ExitEvent produces an obs row write OR a terminal-failure LifecycleEvent OR a structured error log", …)` to the simulation harness. With Solution 1' landed, the invariant should never fire under the canonical flow; its load-bearing role is guarding latent B/C/D from re-emerging through any future emission path that bypasses the gate.

This is the gap predecessor RCA `fix-exit-observer-write-retry/deliver/rca.md:107-109` named and `docs/evolution/2026-05-02-fix-exit-observer-write-retry.md:64` left open. Closing it now closes a debt the predecessor RCA flagged.

---

## Files affected

| File | Change |
|---|---|
| `crates/overdrive-worker/src/driver.rs` | `Driver::start` returns `(AllocationHandle, oneshot::Sender<()>)` (or equivalent shape stashed in `LiveAllocation`). Update both `ExecDriver` and `SimDriver` (and any other impls) symmetrically. Watcher awaits the `oneshot::Receiver` before `exit_tx.send` at the existing emission site (~`driver.rs:638`). Trait docstring gains the post-condition: *"the returned `oneshot::Sender<()>` is fired exactly once after the corresponding `Running` row is committed (or after the May-2 retry path degrades to `LifecycleEvent`-only)."* |
| `crates/overdrive-control-plane/src/action_shim/mod.rs` | After `obs.write(Running)` resolves Ok at ~line 499, fire the corresponding `oneshot::Sender`. On the May-2 retry-exhaustion-degraded path (where the shim writes a degraded `LifecycleEvent` instead of the obs row), fire the sender anyway — required for liveness. |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/exit_observer.rs` | **Triage existing `NoPriorRow` tests**: split into (a) tests that defend the observer-receiver contract (still valid — observer correctly handles a delivered `ExitEvent` given a present prior row); (b) tests that defend the now-impossible producer-ordering path (delete with rationale comment naming this RCA). Add new test: *"watcher cannot emit `ExitEvent` before `Running` row is committed, including under DST schedules that would have raced."* |
| DST suite (location TBD by crafter — likely `crates/overdrive-sim/tests/...` or invariant catalogue) | Add Solution 4 invariant: every `ExitEvent` consumed by the observer produces (obs row write) ∨ (degraded `LifecycleEvent`) ∨ (structured error log). |
| `crates/overdrive-cli/tests/integration/coinflip_honesty_100_trials.rs` | After Solution 1' + 4 lands and DST invariant passes: remove `sleep 0.5` from the bash fixture body at line 128. Remove the rustdoc paragraphs disclaiming the workaround at lines 103-117. |
| `crates/overdrive-cli/tests/integration/job_kind_streaming.rs` | Same: remove `sleep 0.5` at lines 235 / 255 and the rustdoc paragraphs at lines 218-227. |

---

## Risk assessment

- **Driver trait surface change**: cross-crate, but mechanical — every `Driver::start` impl gains one return value or one stashed field. Caught at compile time. Sim and exec drivers update symmetrically.
- **May-2 retry interaction**: the gate-fire site on degraded escalation is the load-bearing liveness invariant. Crafter must verify both firing sites exist; missing the degraded-path fire would leak the watcher under transient ObservationStore failures.
- **Test triage**: some existing observer integration tests inject `ExitEvent` ahead of any `Running` row. Those tests exercised the now-unreachable `NoPriorRow` path. Distinguish "still defending the observer's receiver contract given a present prior row" (keep, possibly simplify) from "defending now-impossible producer-ordering" (delete with rationale).
- **K1 honesty test (`coinflip_honesty_100_trials.rs:294-302`) is the load-bearing observability KPI**. When 1' + 4 lands, K1 must pass at threshold ≥99/100 with `sleep 0.5` removed from the workload. Until that passes without the workaround, the fix is not delivered.
- **No `gh issue create`**: per CLAUDE.md, Solution 2 (defence-in-depth bounded retry) is a deferral candidate awaiting user decision; it is **not** to be created as a follow-up issue without explicit user approval.

---

## Project-rule guardrails

- **DST-clean**: oneshot is not `Clock`-dependent; no real-clock dependency in production or tests. Honors `.claude/rules/development.md` § "Production code is not shaped by simulation" — the gate is a structural ordering edge for a real production race, not a sim concession.
- **Trait definitions specify behavior, not just signature** (`development.md`): the `Driver::start` contract gains an explicit post-condition documented in the trait docstring; equivalence between `ExecDriver` and `SimDriver` is enforced by the existing DST harness plus the new Solution 4 invariant.
- **Persist inputs, not derived state** (`development.md`): no derived-cache field added; the gate is a one-shot synchronisation primitive, not persisted state.
- **No silent failure modes** (predecessor RCA's "loud failure semantics" principle generalised): Solution 4 invariant ensures every `ExitEvent` produces a visible outcome.
