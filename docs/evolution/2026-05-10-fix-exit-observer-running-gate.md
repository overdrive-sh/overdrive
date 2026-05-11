# fix-exit-observer-running-gate — Feature Evolution

**Feature ID**: fix-exit-observer-running-gate
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver` → `/nw-finalize`)
**Branch**: `marcus-sa/nw-discuss`
**Date**: 2026-05-10
**Commits**:
- `6f14cd0` — `test(control-plane): RED scaffold — exit_observer running-gate regression`
- `db7d5ce` — `feat(driver): expose Running-confirmed gate to action shim`
- `5ea9200` — `feat(action-shim): fire Running-confirmed gate at success and degraded paths`
- `97b5136` — `test(control-plane): triage exit_observer NoPriorRow scenarios — all KEEP`
- `d2d7b29` — `feat(sim): DST invariant — every consumed ExitEvent produces visible outcome`
- `5215e59` — `test(cli): remove sleep 0.5 fixture concealment; K1 100/100 holds`
- `6b66c6f` — `chore(fix-exit-observer-running-gate): final integration sweep — all gates green`

**Status**: Delivered. Closes the named-and-open DST invariant gap from the May-2 predecessor RCA (`docs/evolution/2026-05-02-fix-exit-observer-write-retry.md:64`).

---

## Symptom

For a workload whose process lifetime from `Driver::start` return through `child.wait()` resolution is shorter than the wall-clock window between (a) the action shim returning from `driver.start(&spec).await` and (b) the action shim completing `obs.write(ObservationRow::AllocStatus(Running))`, the exit watcher's `ExitEvent` arrived at the observer's mpsc receiver *before* the action shim had written the `Running` row. `exit_observer::run_with_retry → handle_exit_event → find_prior_row(obs, &event.alloc)` returned `Ok(None)`; `RetryOutcome::NoPriorRow` was the empty arm at `exit_observer.rs:225-228` — no obs row, no `LifecycleEvent`, no log line at any level, no telemetry counter. The exit event was silently dropped. Downstream the alloc's only obs row remained the `Running` row that the action shim wrote microseconds later, the reconciler read `Running` and emitted zero actions, and the CLI's `submit --watch` session waited for a terminal verdict that never arrived. KPI K1 (CLI exit-code honesty over 100 trials in `coinflip_honesty_100_trials.rs`) degraded.

The test suite concealed the race by inserting `sleep 0.5` into every Job-kind acceptance fixture's bash body. The rustdoc on `coinflip_honesty_100_trials.rs:103-117` and `job_kind_streaming.rs:218-227` named the workaround a known Phase 1 race; the May-2 predecessor RCA flagged the missing DST invariant for this exact symptom class as a contributing factor and left it open. This fix closes the symmetric *read*-miss leg of the silent-drop tree (May-2 closed the *write*-failure leg).

## Root cause

Multi-causal: four root causes mapped across producer/transport/consumer/observability axes (full evidence at `docs/analysis/root-cause-analysis-exit-observer-prior-row-race.md` § 2 — five-Whys per branch, file:line citations throughout).

| Root | Shape |
|---|---|
| **A. Producer-side ordering gap** | The action shim establishes no happens-before edge between `obs.write(Running)` (`action_shim/mod.rs:499`) and the watcher's first `ExitEvent` emission (`driver.rs:638`). The two are concurrent emitters racing onto the same logical observation surface from two different control-plane writers. "Running before exit" was a wall-clock coincidence, not a structural guarantee. |
| B. Store rendezvous gap | `ObservationStore::alloc_status_row` is point-read; no per-key wait-for-row primitive backs the observer's reader-becomes-writer dependency. The Phase 1 obs-as-truth design correctly consolidates "what is" but creates a transitive dependency for write-only producers (the exit observer) that need to read another producer's row before they can write their own. |
| C. Consumer one-shot read + silent verdict | `find_prior_row` (`exit_observer.rs:487-492`) is one-shot; `Ok(None)` mapped to `RetryOutcome::NoPriorRow` and the branch arm at lines 225-228 emitted nothing. The rustdoc at lines 399-406 disclaimed production-reachability ("only possible under racy injection in tests") — that disclaimer became prophecy. |
| D. Silent failure mode + missing DST invariant | The May-2 "loud failure semantics" principle for exit-row writes was not generalised to the read-miss branch; the DST invariant predecessor RCA flagged at `fix-exit-observer-write-retry/deliver/rca.md:107-109` was still open. |

**B/C/D are downstream consequences of A.** Fix A by structural happens-before and B/C/D's symptom paths become structurally unreachable; their underlying gaps remain latent (a future emitter that bypasses the gate could re-expose them, which is what Solution 4 guards).

## Fix

**Approved fix shape**: **Solution 1' (oneshot-gated watcher emission) + Solution 4 (DST invariant)** — landed together. Rejected alternatives are catalogued in the analysis doc § 4: **Solution 1** (heavier two-stage `Driver::start` split) — rejected as larger blast radius than necessary; **Solution 2** (consumer-side bounded-retry on `NoPriorRow`) — rejected after user pushback as a *tolerator* not a *fixer* (it absorbs Root A on the consumer side instead of closing it, paying for that absorption with a wall-clock retry budget — same shape as `sleep 0.5`, different layer); **Solution 3** (event-carries-state to eliminate the read) — rejected on LWW-dominance grounds.

**Solution 1'** establishes a structural happens-before edge between `obs.write(Running)` and the watcher's first `ExitEvent` emission via a `tokio::sync::oneshot` channel:

- The driver stashes a `oneshot::Sender<()>` in `LiveAllocation` and exposes a paired method on the trait — `release_for_exit_emission(&self, alloc: &AllocationId)` — that consumes/sends on it. **The trait surface gains a paired method, not a return-shape extension** (the original analysis sketched returning `(AllocationHandle, oneshot::Sender)` from `Driver::start`; the implementation chose the paired-method shape after weighing call-site ergonomics — the `LiveAllocation` already owns per-alloc state and the action shim already addresses the driver by `&AllocationId` in adjacent paths). Both `ExecDriver` and `SimDriver` implement the method symmetrically.
- The watcher (`spawn_exit_watcher` in `driver.rs`) does `child.wait().await` as today, then awaits the `oneshot::Receiver<()>` ("Running-confirmed" gate) **before** sending on `exit_tx`. The gate is a logical happens-before edge with no payload.
- The action shim, after `obs.write(Running)` resolves Ok at the success path, calls `driver.release_for_exit_emission(alloc)`. **Liveness rail (interaction with May-2)**: on the May-2 retry-exhaustion-degraded path (where the shim emits a degraded `LifecycleEvent` with `DriverInternalError` instead of an obs row), the gate **must still fire** — otherwise the watcher leaks forever waiting on a oneshot that nothing will ever send. Two firing sites are structurally necessary: post-success and post-degraded-escalation. Either is fine for the watcher because the gate carries no payload; the firing is idempotent at the watcher's side (a single `recv()` consumes the gate).

The ordering edge becomes verifiable in source by reading three adjacent call sites: `obs.write(Running) commits → release_for_exit_emission fires the oneshot → watcher emits ExitEvent`.

**Solution 4** adds a DST invariant — *every `ExitEvent` consumed by the observer produces a visible outcome*: (a) a terminal obs row write OR (b) a degraded `LifecycleEvent` (with `DriverInternalError` from the May-2 escalation path) OR (c) a structured `tracing::error!` log. With Solution 1' landed, the invariant should never fire under the canonical action-shim/exec-driver flow; its load-bearing role is guarding latent B/C/D from re-emerging through a future emission path that bypasses the gate. **This is the gap the May-2 predecessor RCA named at `fix-exit-observer-write-retry/deliver/rca.md:107-109` and the predecessor evolution doc left open at `docs/evolution/2026-05-02-fix-exit-observer-write-retry.md:64`**.

**Project-policy guardrails honoured**:

- **DST cleanliness** (`.claude/rules/testing.md`): `tokio::sync::oneshot` is not `Clock`-dependent; works under `SimClock`, turmoil, real tokio identically. The gate is a logical happens-before edge, not a wall-clock budget.
- **"Production code is not shaped by simulation"** (`development.md`): the oneshot gate is structural ordering for a real production race, not a sim-shape concession. The watcher would await the gate identically under any runtime.
- **"Trait definitions specify behavior, not just signature"** (`development.md`): `Driver`'s `release_for_exit_emission` carries an explicit post-condition in its rustdoc — *"fires the Running-confirmed gate exactly once after the corresponding `Running` row is committed (or after the May-2 retry path degrades to `LifecycleEvent`-only)."* Equivalence between `ExecDriver` and `SimDriver` is enforced by the existing DST harness plus the new Solution 4 invariant.
- **"Persist inputs, not derived state"** (`development.md`): no derived-cache field added; the gate is a transient per-allocation synchronisation primitive that lives only for the duration of `LiveAllocation`.

## Acceptance signal

**KPI K1 — CLI exit-code honesty over 100 trials**: `crates/overdrive-cli/tests/integration/coinflip_honesty_100_trials.rs`.

- **Threshold**: ≥99/100 trials pass.
- **Pre-fix (with `sleep 0.5` workaround in the workload bash)**: 100/100, wall-clock ~62.8s. The workaround widened the race window past the obs-write hot path; passing was a fixture-side artifact, not a property of the production code.
- **Post-fix (Solution 1' + 4 landed; `sleep 0.5` removed at step 01-06; commit `5215e59`)**: **100/100, wall-clock 23.4s.** Passing is now a structural property: the watcher cannot emit `ExitEvent` before `obs.write(Running)` commits, so `find_prior_row` cannot observe `Ok(None)` in the canonical action-shim/exec-driver flow.
- **Telemetry observation**: K1 wall-clock dropped 62.8s → 23.4s post-fix. The 39.4s delta confirms Solution 1' replaces *concealment* (the sleep widened the window past the race) with *genuine ordering* (the race is closed at the source); the timing shift is a side-effect of the structural fix, not the fix itself.

**Final integration sweep** (step 01-07; commit `6b66c6f`): full workspace nextest under Lima — **1182/1182 passing**. Workspace clippy (`-D warnings`) — clean. Workspace doctests (`cargo test --doc --workspace`) — clean. No regressions in the broader exit-observer / lifecycle / streaming surfaces.

## Architectural delta

| Surface | Change |
|---|---|
| `Driver` trait (`crates/overdrive-core` / `crates/overdrive-worker`) | New paired method `release_for_exit_emission(&self, alloc: &AllocationId)` with a documented post-condition in the trait rustdoc. The `start` return shape is unchanged; the `oneshot::Sender<()>` is stashed in `LiveAllocation` and consumed via the paired method. |
| `ExecDriver` (`crates/overdrive-worker/src/driver.rs`) | `LiveAllocation` gains an `Option<oneshot::Sender<()>>` field populated at `start`. `spawn_exit_watcher` (~line 555-638) awaits the corresponding `Receiver<()>` after `child.wait().await` resolves and before `exit_tx.send`. On receiver-dropped (action shim crashed before firing), the watcher logs `tracing::error!` and exits without emitting — the orphan-process condition is identical to today's failure mode and is handled by reconciler convergence on the next tick (no regression). |
| `SimDriver` (`crates/overdrive-sim/src/adapters/driver.rs`) | Symmetric implementation — same `LiveAllocation` field, same gate-await before emission. Sim equivalence preserved; required for the new Solution 4 DST invariant to assert against either adapter under the same harness. |
| Action shim (`crates/overdrive-control-plane/src/action_shim/mod.rs`) | After `obs.write(Running)` resolves Ok at ~line 499, calls `driver.release_for_exit_emission(alloc)`. **Liveness rail**: on the May-2 retry-exhaustion path (the `obs.write(Running)` retry exhausts and degrades to `LifecycleEvent`-only with `DriverInternalError`), the shim fires the gate just before the degraded `LifecycleEvent` emission. Two firing sites; the second is structurally necessary for liveness (a watcher parked on a never-firing oneshot would leak). |
| DST invariant catalogue (`crates/overdrive-sim/...`) | New `assert_eventually!` invariant: every `ExitEvent` consumed by the observer produces (terminal obs row write) ∨ (degraded `LifecycleEvent` with `DriverInternalError`) ∨ (structured `tracing::error!`). Closes the predecessor RCA's open gap. |
| Existing exit-observer integration tests | Triaged at step 01-04 (commit `97b5136`). All scenarios survived as **KEEP**: tests that injected `ExitEvent` ahead of any `Running` row were defending the observer-receiver contract (does the observer correctly handle a delivered `ExitEvent`?), not the now-impossible producer-ordering path. They were reshaped to drive a present prior row first; the module docstring was updated to name this RCA. **0 deletions** (the planning doc anticipated some deletions; on inspection, every test was load-bearing under the receiver-contract framing). |
| CLI fixtures (`coinflip_honesty_100_trials.rs`, `job_kind_streaming.rs`) | `sleep 0.5` removed at step 01-06; rustdoc paragraphs disclaiming the workaround removed at the same time. K1 honesty test holds at 100/100 without the workaround. |

## Files changed

`git diff --stat 6f14cd0^..6b66c6f`:

- 24 files changed, 2529 insertions, 73 deletions across 7 commits.
- Production code: `crates/overdrive-worker/src/driver.rs`, `crates/overdrive-core/src/traits/driver.rs`, `crates/overdrive-sim/src/adapters/driver.rs`, `crates/overdrive-control-plane/src/action_shim/mod.rs`.
- Test surface: `crates/overdrive-control-plane/tests/integration/job_lifecycle/exit_observer.rs` (triage; KEEP all + module docstring update), new RED scaffold + DST invariant tests in `overdrive-sim`, fixture cleanup in `crates/overdrive-cli/tests/integration/coinflip_honesty_100_trials.rs` and `job_kind_streaming.rs`.
- Process artifacts: deliver workspace (roadmap, execution log, RCA summary, develop-progress) — kept in place per predecessor pattern; the durable multi-causal analysis lives at `docs/analysis/root-cause-analysis-exit-observer-prior-row-race.md`.

## Tests added

- **NEW (RED scaffold, step 01-01, commit `6f14cd0`)**: regression test exercising the producer-ordering race without the `sleep 0.5` concealment. Lands as `#[should_panic(expected = "RED scaffold")]` per `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing commits"; flips to GREEN as the fix wires through 01-02 → 01-03.
- **NEW (DST invariant, step 01-05, commit `d2d7b29`)**: simulation-harness assertion that every `ExitEvent` consumed by the observer produces a visible outcome. Closes the predecessor RCA's open gap.
- **PRESERVED (step 01-04, commit `97b5136`)**: every scenario in `tests/integration/job_lifecycle/exit_observer.rs` that injected `ExitEvent` ahead of any `Running` row stayed as **KEEP** — they defend the observer-receiver contract, not the now-impossible producer-ordering path. Module docstring updated to name this RCA. 0 deletions.
- **CLEANUP (step 01-06, commit `5215e59`)**: `sleep 0.5` removed from the bash fixture bodies in `coinflip_honesty_100_trials.rs:128`, `job_kind_streaming.rs:235`, `job_kind_streaming.rs:255`. Rustdoc paragraphs at `coinflip_honesty_100_trials.rs:103-117` and `job_kind_streaming.rs:218-227` removed at the same time.

## Quality gates

- **DES integrity** — all 7 steps have complete 5-phase traces in `docs/feature/fix-exit-observer-running-gate/deliver/execution-log.json` with `status: EXECUTED, decision: PASS` on every COMMIT phase. SKIPPED phases carry NOT_APPLICABLE rationale strings.
- **Workspace nextest** on Linux via Lima (`cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`) — **1182/1182 passing** at step 01-07.
- **K1 honesty test** — 100/100 trials pass with `sleep 0.5` workaround removed (vs ≥99/100 threshold).
- **Workspace clippy** (`-D warnings`) — clean.
- **Workspace doctests** (`cargo test --doc --workspace`) — clean.
- **Refactor pass** (step 03-refactor, APPROVED_SKIP) — refactor-pass evaluated all 18 modified files against L1-L4 (naming, duplication, structural, cohesion); no improvements warranted on this focused 6-commit fix. The closest candidate (L2 — gate-fire duplication across `StartAllocation`/`RestartAllocation` arms in action_shim) was rejected because the 4-line × 2-site shape carries a load-bearing AC2 comment block that consolidation would defeat (would shuffle code, not net-positive). Naming intention-revealing throughout; sim/host split per `development.md` preserved; user-signed-off paired-method gate shape kept intact.
- **Mutation gate** — out of scope for this finalize per project pattern (predecessor finalize also skipped mutation per user direction).

## Out of scope (deferral candidates — awaiting user decision; NO GitHub issues created)

Per CLAUDE.md § "Deferrals require GitHub issues — AND user approval BEFORE creation," the following candidates are surfaced for user decision. **No `gh issue create` was invoked**.

- **Solution 2 (defence-in-depth bounded retry on `find_prior_row`)** — only relevant if a future emission path emerges that genuinely cannot expose a oneshot (e.g., a different driver shape, a sim-injection path that deliberately bypasses the action shim, a hypothetical retry path that re-emits `ExitEvent`). Until such a path is identified, no follow-up is required: Solution 1' makes the symptom path structurally unreachable in the canonical flow, and Solution 4's DST invariant guards against regression. **Recommend tracking only if/when a concrete second emission path is proposed.**
- **Solution 1 (heavier two-stage `Driver::start` split)** — defence-in-depth on Root A; the analysis doc judged Solution 1' sufficient as the primary fix (smaller blast radius, no driver-trait surface inversion, no replacement-row semantics on `StartRejected`). Solution 1 stays rejected unless a future regression demonstrates that Solution 1's structural shape is needed.
- **Solution 3 (event-carries-state)** — explicitly rejected on technical grounds in the analysis doc § 4 (LWW-dominance regression: Running row counter dominates Failed row counter on later arrival; breaks the regression test at `tests/integration/job_lifecycle/exit_observer.rs:493-544` for `LifecycleEvent.from`-field semantics). Stays rejected unless a counter-allocation strategy is independently developed.

## References

- **Multi-causal analysis (durable; SSOT for this fix)**: `docs/analysis/root-cause-analysis-exit-observer-prior-row-race.md` — five-Whys investigation across four causal axes, four solution proposals, back-chain validation, May-2 layering. The lasting analysis lives at `docs/analysis/`; this finalize does NOT migrate it.
- **Focused-scope summary**: `docs/feature/fix-exit-observer-running-gate/deliver/rca.md` — the deliver-scoped actionable summary; preserved in the deliver workspace per predecessor pattern.
- **Predecessor RCA (write-failure leg)**: `docs/feature/fix-exit-observer-write-retry/deliver/rca.md` (May 2). The two RCAs are siblings on the same architectural seam: this fix closes the *read*-miss leg of the silent-drop tree the predecessor opened with the *write*-failure leg.
- **Predecessor evolution doc**: `docs/evolution/2026-05-02-fix-exit-observer-write-retry.md`. Line 64 of that doc lists the DST invariant as still open; **this fix closes that gap** (Solution 4 lands the invariant the predecessor flagged).
- **Prior obs-as-truth fix**: `docs/evolution/2026-05-01-fix-exec-driver-exit-watcher.md` — same architectural seam (single observation surface; exit-event → obs convergence). The May-1 fix established the design assumption ("the observer assumes a `Running` row exists by the time an `ExitEvent` arrives"); this fix makes that assumption a structural guarantee instead of a wall-clock coincidence.
- **Test discipline**: `.claude/rules/testing.md` § "RED scaffolds and intentionally-failing commits", § "Running tests — Lima VM".
- **Reconciler purity contract**: `.claude/rules/development.md` § "Reconciler I/O" — the `reconcile` function reads observation as input state, which is why a missed `ExitEvent` would have stranded the alloc indefinitely (the reconciler reads `Running`, sees no terminal row, emits zero actions).
- **Trait contract discipline**: `.claude/rules/development.md` § "Trait definitions specify behavior, not just signature" — the new `release_for_exit_emission` paired method carries an explicit post-condition in its rustdoc; `ExecDriver` and `SimDriver` are both bound by it, and the DST equivalence harness (extended by Solution 4) is the structural guard against drift.
- **Commits**: `6f14cd0`, `db7d5ce`, `5ea9200`, `97b5136`, `d2d7b29`, `5215e59`, `6b66c6f`.
