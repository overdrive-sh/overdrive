# Evolution: fix-exec-driver-exit-watcher

**Date**: 2026-05-01
**Branch**: `marcus-sa/phase1-first-workload`
**Wave shape**: bugfix (RCA → /nw-deliver, single-phase 2-step roadmap: 01-01 RED, 01-02 GREEN)
**Status**: shipped, both DES steps EXECUTED with PASS verdict

---

## Defect

Crashed `ExecDriver`-backed allocations were silently stranded at
`AllocState::Running` in the ObservationStore. The driver stored
`tokio::process::Child` handles in `LiveAllocation::Running { child, scope }`
(`crates/overdrive-worker/src/driver.rs:47-53,270`) but never observed
natural child exit. The convergence loop reads actual state exclusively
from `obs.alloc_status_rows()`
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:499-516`);
`Driver::status()` existed on the trait
(`crates/overdrive-core/src/traits/driver.rs:159`) but had zero callers.
The reconciler emitted no actions; the crash recovery path never fired.

For the test surface, `crash_recovery.rs:112-150` previously masked the
gap with a synthetic `obs.write(AllocStatus(Terminated))` workaround
labelled "Phase 1 has no real crash detector wired yet — the direct-write
models the post-detection state the reconciler would observe." Test
design accepted the gap as a Phase-1 deferral comment instead of a
tracked open scenario.

## Root cause

There was no worker-owned subsystem that observed process exits and
wrote them as `AllocState::Terminated` / `AllocState::Failed` to the
ObservationStore. The whitepaper §4 owner-writer model expects every
node to write its own observation rows; the driver is the natural owner
of exit events but lacked an observation seam.

Contributing factors:

- The `Driver` trait carried an unused `status()` surface — design
  admitted the observation gap but never wired the materialisation
  path.
- The `crash_recovery.rs` test masked the gap with a synthetic obs
  write rather than a real SIGKILL on the workload PID, so the
  integration suite went green against a fiction.
- No DST invariant asserted "every running alloc whose driver has
  exited eventually shows non-Running in obs."
- The reconciler had no operator-stop terminal-state guard, so even
  with exit detection wired the reconciler would have re-scheduled
  workloads the operator had explicitly stopped (latent race).

## Decision

**Per-alloc watcher tasks emit `ExitEvent`s on a worker-owned mpsc
channel; a `worker::exit_observer` subsystem consumes events and
materialises `AllocStatusRow` writes.** The watcher is the natural
owner of exit observation per the §4 owner-writer model — single-writer
per allocation, no multi-source reconciliation needed. The observer
subsystem is spawned alongside the convergence loop with explicit
shutdown ordering: convergence drains first, then the observer drains
in-flight `ExitEvent`s, ensuring final exits land in obs before the
process exits.

`ExitKind` is a sum type (`CleanExit` / `Crashed { exit_code, signal }`)
matching the §17 newtype discipline. Operator-stop intent is pinned
against the SIGTERM-driven natural-exit race via an
`Arc<AtomicBool>::intentional_stop` flag on `LiveAllocation`, set under
SeqCst BEFORE `stop()` issues SIGTERM and read under SeqCst by the
watcher at exit-classification time. The watcher's `LogicalTimestamp`
counter is sourced from the same `Clock` instance the action-shim uses,
ensuring strict LWW dominance over the prior `Running` row.

## Scope landed

Per RCA `docs/feature/fix-exec-driver-exit-watcher/deliver/rca.md`
§Approved fix items 1-7:

1. **`Driver` trait** — new `ExitEvent` and `ExitKind` types; new
   `take_exit_receiver() -> Option<mpsc::Receiver<ExitEvent>>` returning
   the per-driver receiver (consumer-once semantics).
2. **`ExecDriver::start`** — spawns a per-allocation watcher task that
   owns the `Child`, awaits `child.wait()`, classifies the exit via
   `classify_exit(ExitStatus, intentional_stop)`, and emits an
   `ExitEvent` on the driver's `mpsc::Sender<ExitEvent>`.
3. **`ExecDriver::stop`** — sets the per-alloc `intentional_stop` flag
   to `true` BEFORE delivering SIGTERM; the watcher's SeqCst load on
   the flag at classification time pins operator stop intent against
   the SIGTERM-driven natural-exit race.
4. **`worker::exit_observer` subsystem** — new
   `crates/overdrive-control-plane/src/worker/exit_observer.rs` consuming
   events. For each event: maps `(exit_code, signal, intentional_stop)`
   to `AllocState::Terminated` or `AllocState::Failed { reason }`,
   writes the `AllocStatusRow` (counter+1 against the prior row under
   LWW), broadcasts a `LifecycleEvent` on `state.lifecycle_events`,
   and re-enqueues the `job_lifecycle` reconciler on the broker.
5. **`run_server_with_obs_and_driver` wiring** — exit-observer task
   spawned alongside the convergence loop. Shutdown ordering: convergence
   drains first, then observer drains in-flight events before exit.
6. **`SimDriver::inject_exit_after(alloc, after, ExitKind)`** — DST
   counterpart driven by `SimClock`, exercising the same trait
   abstraction so the observer subsystem is end-to-end testable under
   simulation without a real kernel.
7. **`crash_recovery.rs` rewrite** — replaces the synthetic obs write
   at lines 112-150 with a real `libc::kill(pid, SIGKILL)` on the
   workload PID read from `cgroup.procs`. Removes the
   `tick_n = crashed_counter` skip workaround at lines 119-127, 152-159.
   Phase-3 assertions now pin (i) the watcher writes `Failed` (NOT
   `Terminated` — kill-by-signal without `intentional_stop = true` is
   a crash), (ii) a fresh `Running` row whose counter strictly dominates
   the `Failed` row.

Three structural fixes layered on top of the §1-7 RCA items emerged
during GREEN delivery and are part of this feature's permanent
contribution:

- **Clock injection on `SimDriver`** via `with_clock(DriverType,
  Arc<dyn Clock>)` so the observer's logical-timestamp counter and
  the driver's emit-delay derive from the same logical-time source the
  harness drives. Prior to this fix the sim driver had its own
  internal monotonic counter that skewed against the observer's,
  producing latent ordering bugs that would have surfaced under
  multi-allocation DST scenarios.
- **Reconciler operator-stop terminal-state gate** — `JobLifecycle::reconcile`
  now extracts `is_operator_stopped` and `is_restartable` helpers; an
  alloc whose obs row carries `Stopped { by: Operator }` is terminal
  and the reconciler MUST NOT schedule a fresh replacement. Closes the
  operator-stop / reconciler-restart race that would otherwise mask
  intentional stops as "crashed, restart it." This was an existing
  latent bug independent of the exit-detection gap; both surfaced
  together because both paths converge on the same obs row.
- **Observer LifecycleEvent emission** — the observer broadcasts a
  `LifecycleEvent::Crashed` / `LifecycleEvent::Terminated` on
  `state.lifecycle_events` alongside the obs write. The lifecycle event
  bus is the durable record other subsystems (xt-replay, the
  investigation agent, future telemetry exporters) read from; the obs
  row is the eventually-consistent state. Both are written in the same
  task to ensure consumers cannot observe one without the other.

## Quality gate results

| Gate | Result |
|---|---|
| Default-lane workspace nextest | PASS — 610 / 610 |
| Integration suite (Lima, `--features integration-tests`) | PASS — 763 / 763, 0 timeouts (was 4 timeouts pre-fix) |
| 5 target tests (`exit_observer::*` + `crash_recovery::*`) | PASS — 5 / 5 |
| `cargo clippy --all-targets --features integration-tests -- -D warnings` | PASS — clean |
| `cargo xtask dst-lint` | PASS — clean |
| `cargo fmt --all -- --check` | PASS — clean |
| Adversarial review | APPROVED — 0 blockers, 0 majors, 1 minor (non-blocking) |
| Mutation testing — `overdrive-core` | PASS — 100% kill rate |
| Mutation testing — `overdrive-worker` | PASS — 100% kill rate |
| Mutation testing — `overdrive-control-plane` | PASS — 100% kill rate (6 / 6 mutants caught) |
| DES integrity verification | PASS — all 2 steps complete trace |

## Files modified

| File | Change |
|---|---|
| `crates/overdrive-core/src/traits/driver.rs` | New `ExitEvent`, `ExitKind`; `take_exit_receiver()` on `Driver` trait |
| `crates/overdrive-core/src/reconciler.rs` | `is_operator_stopped` / `is_restartable` helpers; reconciler operator-stop gate |
| `crates/overdrive-worker/src/driver.rs` | Watcher spawn; `intentional_stop` SeqCst flag; `LiveAllocation` reshape; `classify_exit(ExitStatus, intentional_stop) → ExitKind` |
| `crates/overdrive-control-plane/src/worker/exit_observer.rs` (new) | Observer subsystem: consume `ExitEvent`s, write `AllocStatusRow`, broadcast `LifecycleEvent`, re-enqueue reconciler |
| `crates/overdrive-control-plane/src/worker/mod.rs` (new) | Module registration |
| `crates/overdrive-control-plane/src/lib.rs` | Wire observer task; shutdown ordering (convergence drains first, then observer); `spawn_convergence_loop` extracted to satisfy 100-LOC clippy ceiling |
| `crates/overdrive-control-plane/src/reconciler_runtime.rs` | Reconciler observes terminal `Stopped { by: Operator }` rows |
| `crates/overdrive-sim/src/adapters/clock.rs` | `Arc<dyn Clock>` shape for cross-component injection |
| `crates/overdrive-sim/src/adapters/driver.rs` | `inject_exit_after(alloc, after, ExitKind)` DST exit injection; `with_clock(DriverType, Arc<dyn Clock>)` injection seam |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery.rs` | Real SIGKILL on workload PID; remove synthetic-write workaround; Phase-3 assertions pinned |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/exit_observer.rs` (new) | Four `#[tokio::test]` scenarios pinning `ExitEvent` / `ExitKind` / `inject_exit_after` / `worker::exit_observer::spawn` |
| `crates/overdrive-control-plane/tests/integration.rs` | One-line `mod exit_observer;` registration |
| `.config/nextest.toml` | Per-test budget overrides for DST canary tests (360s, `dst-canary` test-group) and store-local snapshot proptest (240s); default 120s unchanged |

## Lessons learned

- **LWW snapshot is the wrong observation surface for transient
  state; the lifecycle event bus is the durable record.** When the
  observer writes the `AllocStatusRow` it ALSO broadcasts a
  `LifecycleEvent::Crashed` / `Terminated`. The obs row is
  eventually-consistent CRDT state for the convergence loop to react
  to; the lifecycle event is the durable, ordered audit record any
  downstream subsystem (investigation agent, telemetry exporter,
  xt-replay) reads. Future observation work should reach for both
  surfaces from the same write site, not pick one and hope downstream
  catches up.
- **Operator-stop / reconciler-restart races and exit-detection gaps
  share a single observation surface.** The exit-detection bug masked
  the operator-stop terminal-state gap: with no exit detection, the
  reconciler never had occasion to misclassify operator stop as crash.
  When the exit-detection path was wired, the operator-stop race
  surfaced immediately on the same code path. The structural fix
  (terminal-state guard in `JobLifecycle::reconcile`) belongs in the
  reconciler's pure compute phase, not in the observer — the observer's
  job is "write what happened," the reconciler's job is "decide what
  to do next."
- **Synthetic test fixtures hide the bugs they were meant to model.**
  The `crash_recovery.rs` synthetic obs write made the test pass while
  the production crash detector did not exist — the test was
  measuring the reconciler's reaction to a fictional input. Real-PID
  SIGKILL is the only honest signal for a crash recovery test, and
  the cost (Linux-only, requires `unsafe { libc::kill }` with a
  `// SAFETY:` comment in test code) is paid once for a permanent
  guarantee.

## Cross-references

- Whitepaper §4 *Control Plane / Owner-writer model* — the structural
  reason the watcher writes its own obs rows rather than going through
  the reconciler.
- Whitepaper §17 *Storage Architecture / ObservationStore (Corrosion)*
  — the LWW counter discipline the watcher follows.
- `.claude/rules/testing.md` § *RED scaffolds and intentionally-failing
  commits* — the convention the 01-01 commit followed (test-only,
  intentionally non-compiling, `--no-verify` justified).
- `.claude/rules/development.md` § *Reconciler I/O* — the pure-compute
  contract the operator-stop gate fix preserves (helpers are sync,
  read-only against `actual: &State`, no `.await` inside `reconcile`).

## Commits (chronological)

| SHA | Step | Title |
|---|---|---|
| `08ac89b` | 01-01 | `test(exit-observer): pin Driver exit-event abstraction + crash_recovery rewrite (RED scaffold — see Step-ID: 01-01)` |
| `afe3f1b` | 01-02 | `fix(exit-observer): wire ExecDriver exit watcher + worker exit_observer subsystem` |
