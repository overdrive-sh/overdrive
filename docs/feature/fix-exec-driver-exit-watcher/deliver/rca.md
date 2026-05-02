# RCA — ExecDriver crash detection gap

## Defect

Crashed workloads are silently stranded at `AllocState::Running` in the
ObservationStore. The `ExecDriver` stores `Child` handles in
`LiveAllocation::Running { child, scope }` (`crates/overdrive-worker/src/
driver.rs:47-53,270`) but never observes natural child exit. The
convergence loop reads actual state exclusively from
`obs.alloc_status_rows()`
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:499-516`);
`driver.status()` exists on the trait
(`crates/overdrive-core/src/traits/driver.rs:159`) but has zero callers.
The reconciler emits no actions, the crash recovery path never fires.

## Root cause

There is no worker-owned subsystem that observes process exits and writes
them as `AllocState::Terminated`/`Failed` to the ObservationStore. The
§4 owner-writer model expects every node to write its own observation
rows; the driver is the natural owner of exit events but lacks an
observation seam.

## Contributing factors

- `Driver` trait has unused `status()` surface — design admitted the
  observation gap but never wired the materialisation path.
- `crates/overdrive-control-plane/tests/integration/job_lifecycle/
  crash_recovery.rs:112-150` masks the gap with a synthetic obs write
  ("Phase 1 has no real crash detector wired yet — the direct-write
  models the post-detection state the reconciler would observe").
  Test design accepted the gap as a Phase-1 deferral comment instead
  of a tracked open scenario.
- No DST invariant asserts "every running alloc whose driver has
  exited eventually shows non-Running in obs."

## Proposed fix (Phase 1, single-node, single-cut)

1. **`Driver` trait — new `ExitEvent` type.** Carries `alloc`,
   `exit_code`, `signalled`, `intentional_stop` discriminator.

2. **`ExecDriver::start`** — after spawn, transfer `Child` ownership
   into a `tokio::spawn`'d per-allocation watcher task that calls
   `child.wait().await`. On exit, the task emits an `ExitEvent` to
   a worker-owned `mpsc::Sender<ExitEvent>` and drops the `Child`.
   Replace `LiveAllocation::Running { child, scope }` with a
   shape that owns a `JoinHandle<ExitEvent>` (or drops the `Child`
   entirely from `LiveAllocation`).

3. **`ExecDriver::stop`** — set `intentional_stop = true` on the
   shared `LiveAllocation` state under a `Mutex` BEFORE issuing
   SIGTERM. The watcher reads this flag when classifying the exit
   so operator-stop is not misclassified as crash.

4. **New worker subsystem `worker::exit_observer`** — `tokio::task`
   consuming `ExitEvent`s. For each event:
   - Map `(exit_code, signalled, intentional_stop)` to
     `AllocState::Terminated` or `AllocState::Failed { reason }`.
   - Write `AllocStatusRow` to `obs` with a `LogicalTimestamp` whose
     counter is sourced from the same `Clock` instance the
     action-shim uses, ensuring strict dominance over the prior
     `Running` row.
   - Submit an `Evaluation { reconciler: job_lifecycle, target:
     job/<id> }` to the broker
     (`reconciler_runtime.rs:317-328`) so the reconciler
     re-evaluates promptly.

5. **`run_server_with_obs_and_driver` wiring** — spawn the
   exit-observer task alongside the convergence loop; shutdown
   token must drain the observer BEFORE the convergence task to
   ensure final exit events land in obs.

6. **`SimDriver::inject_exit_after(alloc, ticks, code, signalled)`**
   — DST counterpart driven by `SimClock`. Exit events feed the
   same trait abstraction so the same harness asserts "obs
   reflects `Failed` within N ticks of injected exit."

7. **`crash_recovery.rs` rewrite** — replace the synthetic obs
   write at lines 112-150 with a real SIGKILL on the spawned PID,
   assert the watcher writes `Failed`, the reconciler re-enqueues,
   and a fresh allocation comes up. Remove the `tick_n =
   crashed_counter` skip workaround at lines 119-127, 152-159.

## Files affected

| Path | Change |
|---|---|
| `crates/overdrive-core/src/traits/driver.rs:140-166` | New `ExitEvent` type |
| `crates/overdrive-worker/src/driver.rs:47-53,184-273,275-344` | Watcher spawn; `intentional_stop` flag; `LiveAllocation` reshape |
| `crates/overdrive-worker/src/exit_observer.rs` (new) | Subsystem |
| `crates/overdrive-control-plane/src/lib.rs` | Wire exit-observer task with shutdown ordering |
| `crates/overdrive-control-plane/src/action_shim.rs:357,424` | `stop()` marks `intentional_stop` |
| `crates/overdrive-sim/src/adapters/driver.rs:55-81` | DST exit injection |
| `crates/overdrive-control-plane/tests/integration/job_lifecycle/crash_recovery.rs:112-150` | Real SIGKILL; remove synthetic-write workaround |

## Risk assessment

- **stop() vs natural exit race** — mitigated by `intentional_stop`
  flag under `Mutex`, checked by watcher before classifying exit
  as crash. Single transition serialises the race.
- **PID reuse** — non-issue: watcher owns `Child` for its full
  lifetime (kernel-guaranteed unique handle while parent holds it).
- **Double-stop** — `stop()` already handles `Terminated` arm
  idempotently (`driver.rs:284-288`). Watcher must update
  `LiveAllocation` to `Terminated` BEFORE writing obs so a racing
  `stop()` finds the terminal state.
- **LWW counter discipline** — watcher MUST use the same `Clock`
  source as `action_shim::timestamp_for(tick)`
  (`action_shim.rs:198`); otherwise watcher writes lose to stale
  shim writes.
- **DST coverage gap** — must add `assert_eventually!("crashed
  alloc reaches non-Running", …)` invariant and proptest for
  `SimDriver::inject_exit_after`.
- **Mutation gate** — exit-classification logic
  (`(exit_code, signalled, intentional_stop) → AllocState`) must
  reach ≥80% kill rate per `.claude/rules/testing.md`.

## Phase 1 scope adherence

Single-node only: watcher writes obs rows for allocs it owns
(matches §4 owner-writer model). No node-registration, no
multi-region. Compatible with `--allow-no-cgroups` (ADR-0028) — the
watcher only reads `Child` exit, not cgroup state.

## Regression test

The regression test for this defect is a real-process crash test
(replacing the synthetic-write `crash_recovery.rs`). It must:

1. Submit a job with a workload that exits non-zero shortly after
   start (e.g. `sh -c 'exit 1'`).
2. Wait for the exit-observer to write `AllocState::Failed` to obs.
3. Assert the reconciler re-enqueues and a fresh allocation reaches
   `Running`.
4. Fail without the fix (current code leaves the alloc at `Running`
   forever, the test times out).
