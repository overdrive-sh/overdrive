# VM lifecycle latency — L1–L6 refactoring log

## Scope and baseline

This refactor was applied after the approved DELIVER steps and the approved
Landlock amendment. The scope is the feature-touched production code only; the
pre-existing dirty `AGENTS.md`, tests, examples, expectation evidence, design
artifacts, DES log, and mutation artifacts were not changed.

The baseline was measured at `HEAD` before this refactor. The selected source
files were the twelve production Rust files changed by the feature (including
the simulation harness source used by the feature's adapter composition).
The batch was planned in RPP cascade order and applied once before any
terminating test run.

## Cascade assessment and transformations

### L1 — Readability

Applied to the feature-owned paths:

- `EvaluationBroker` now names the pending replacement, age-ordered
  candidates, unavailable target set, admitted evaluations, and reaped count
  directly (`crates/overdrive-core/src/eval_broker.rs`). These names preserve
  the ADR-0102 distinction between pending identity, target blocking, and
  admission.
- `VmConfig::landlock_rules` names the capacity calculation
  `explicit_rule_count` (`crates/overdrive-core/src/vm/config.rs`), making the
  one run-directory rule plus the optional selected-TAP rule explicit without
  changing cardinality or order.
- The convergence-loop startup comment now describes bounded concurrent
  ownership and shutdown draining instead of the superseded one-at-a-time
  dispatch description (`crates/overdrive-control-plane/src/lib.rs`).

No public or accepted private signature changed. No state, wire, persistence,
Landlock rule, or lifecycle ordering changed.

### L2 — Complexity

Applied to two concrete feature-local control-flow smells:

- `network_launch_prefix` uses a guard clause for the no-network case, leaving
  the networked `ip netns exec` construction as the single remaining path
  (`crates/overdrive-host/src/vmm.rs`). The wrapper order and returned argv are
  unchanged.
- `supervise_command` routes all parsed control lines through the private
  `handle_control_line` helper (`crates/overdrive-init/src/main.rs`). The
  helper retains the existing Shutdown flag/diagnostic behavior, typed
  unexpected-message and parse errors, and one `begin_group_termination` call;
  EOF handling remains in the surrounding byte-read branch. The existing
  polling, signal, group-reap, deadline, and direct-status ordering is
  unchanged.
- The convergence owner now uses one named duration-to-milliseconds conversion
  and one admission guard, removing repeated conversion expressions and one
  nested branch without changing the biased `select!`, capacity, or shutdown
  semantics (`crates/overdrive-control-plane/src/lib.rs`).

### L3 — Responsibilities

The existing `ClaimGuard` remains the owner of originating-session claim
checking. Its duplicated private predicate was placed in
`ClaimGuard::is_originating_live`; both `try_begin_ending` and `Drop` still
perform their checks under the same `LiveMap` lock. No ownership moved between
the watcher, operator stop path, reclamation, or `EndingInFlight` state.

### L4 — Abstractions

One feature-local duplication was removed by reusing the `ClaimGuard` private
predicate for both the successful handoff and failed-handoff drop path. The
accepted `cleanup_driver_artifacts` helper remains the only shared cleanup
extraction and its four-call order is untouched. No speculative interface,
pattern, subsystem, task, retry, timeout, or persistence abstraction was
introduced.

### L5 — Patterns

No justified pattern transformation. The existing owner and adapter shapes
already express the approved architecture; introducing a strategy, command,
or state-pattern layer would add ceremony and risk obscuring the required
ordering.

### L6 — SOLID++

No justified SOLID transformation. The touched code already keeps the
broker, convergence owner, guest supervisor, VMM adapter, and `ClaimGuard`
within their approved ownership boundaries. Dependency inversion or interface
segmentation changes would alter accepted API/architecture rather than remove
a concrete smell.

## Behavior-preservation boundary

The batch changes only local names, comments, a guard-clause layout, a pure
duration conversion helper, a private control-line extraction, and reuse of a
private claim predicate. It preserves the exact ADR-0102 broker signatures and
FIFO/target exclusion/timestamp semantics; ADR-0103 stop, writer/VMM overlap,
guest supervision, direct-status and cleanup order; and the approved Landlock
producer, rule order, access modes, and argv placement. Tests were immutable.

## Verification disposition

The full feature-crate Lima suite was executed with `--no-fail-fast` and ran
2,251 tests: 2,245 passed, 20 were skipped, and 6 failed. A clean detached
pre-refactor worktree at `79ae96d2` reproduced the exact same six failures (see
the comparison section below), so no test or adjacent behavior was changed.

The focused terminating Lima suite then covered the six refactored production
owners while excluding only those six baseline failures: it passed with
2,245 tests and 20 skips. The final workspace check and clippy `-D warnings`
pass, and `cargo fmt --all --check` plus `git diff --check` pass.

The isolated pinned-artifact restart test passed on the clean baseline in
18.350 s and on the current refactor tree in 5.545 s after the host was
cleaned, so the first isolated timeout was not refactor-caused. A subsequent
pinned-artifact bounded CLI run with `--no-fail-fast` ran all 16 discovered
tests: 15 passed and the same restart test timed out at 120.008 s while it ran
after the preceding 13 VM cases. A minimized current-tree pair reproduced the
timeout twice after `operator_stop_is_terminated_and_consumes_no_restart_budget`;
the same pair passed once with a state monitor. The clean pre-refactor pair at
`79ae96d2` passed both tests (15.671 s and 5.540 s), while a later current-tree
restart-only run also reproduced the timeout. This makes the signal an
in-process restart/cleanup race, not a deterministic consequence of any local
L1–L4 transformation.

The durable current-tree strace (`/tmp/vll-restart-current.strace`, captured by
`/tmp/vll-metal-restart-strace.log`, nextest run
`c8099058-1a0e-47ca-9f98-aac6dc15605c`) identifies the exact wait and stale
prerequisite. `poll_until_restarted` remains in its 120-second HTTPS describe
loop after boot #2; the boot-epoch reclaimer kills the old
`cloud-hypervisor` (PID 1749578) through
`alloc-vm-clean-rootfs-0.scope/cgroup.kill` at trace lines 208884–208898.
The replacement driver then creates the same fixed allocation run directory
at lines 209965–209966 and begins recreating the same fixed scope at lines
210039–210046. The still-live boot #1 driver's detached exit-watcher cleanup
concurrently removes that replacement run directory at lines 210075–210085
and the replacement's kernel copy consequently fails with `ENOENT` at line
210249 (the surrounding cleanup removes the clone and scope). No second VMM
is spawned; the poller sees no `Running` row with `restart_count >= 1` and
waits until the nextest 120-second timeout. This is the fixed
`alloc-vm-clean-rootfs-0`/`ovd-ns-0000`/`ovd-tp-0000` identity collision between
the same-process boot #1 reaper and boot #2 replacement; cgroup, run-dir,
netns, clone, allocator, and persistent-store surfaces were empty after the
timeout, so no external leftover was found. The in-process server restart
fixture has no accepted synchronization boundary for joining the old driver's
detached reaper before replacement start, and adding a new production owner,
generation, lock, retry, or timeout would exceed the approved API/design.
The pinned host-equivalence gate passed all 9 tests, but the required 16-test
CLI gate remains a genuine blocker and no commit is authorized. Mutation
testing was not run.

## Clean pre-refactor comparison

To separate refactor regressions from the retained full-suite limitation, a
clean detached worktree at the pre-refactor `HEAD` `79ae96d2` was created at
`/Users/marcus/conductor/workspaces/helios/vll-refactor-baseline`. Its focused
run used the same Lima command and selected exactly the six failed nextest
items. The first baseline attempt correctly stopped because the clean
worktree had no generated BPF object; `cargo xtask bpf-build` supplied that
required generated artifact. The focused baseline then reproduced all six
failures with the following exact differences:

1. `overdrive-control-plane::compile_fail::compile_fail_cases`, fixture
   `lifecycle_event_does_not_carry_alloc_status_row.rs`: expected the complete
   rustc source snippet and `found AllocStatusRow`; actual rustc elided the
   snippet (`from: row, ...`) and reported `found AllocStatusRowV3`.
2. `overdrive-control-plane::integration::workload_lifecycle::exit_observer::exit_observer_writes_failed_and_does_not_name_consumers_on_observed_exit`:
   line 796 reports `left: 2`, `right: 0`; the untouched assertion expects no
   observer broker submissions. The path is
   `spawn_with_runtime(Some(runtime)) → run_with_retry → ObservationStore /
   LifecycleEvent` in `worker/exit_observer.rs`, with the runtime broker and
   interest-router composition around it.
3. `overdrive-core::compile_fail::compile_fail_cases`, fixture
   `observation_write_rejects_intent.rs`: expected the full
   `Result<(), ObservationStoreError>` method note; actual rustc elided it as
   `Result<(), Observa...`. The boundary is the unchanged
   `ObservationStore::write` trait method in
   `core/src/traits/observation_store.rs:2070`.
4. `overdrive-sim::integration::service_backend_projection::liveness_restart_budget_and_finalization_keep_projection_handoffs`:
   line 535 reports only a pending `svid-lifecycle` evaluation at `61.003s`,
   while the assertion requires the service-lifecycle handoff. The path is
   the in-process `run_convergence_tick → action_shim → EvaluationBroker`
   service/SVID projection composition.
5. `overdrive-sim::integration::service_backend_projection::workload_start_and_stop_wake_the_service_projection_owner`:
   line 535 reports only a pending `service-lifecycle` evaluation at `0ns`,
   while the same assertion requires both service and SVID handoffs. It uses
   the same `World → run_convergence_tick → action_shim → broker` path.
6. `overdrive-worker::compile_fail::compile_fail_fixtures`, fixture
   `exec_driver_missing_fs.rs`: expected the complete `ExecDriver::new`
   signature note containing `Arc<(dyn CgroupFs + 'static)>`; actual rustc
   elided that note as `Arc<(dyn CgroupFs + 'static)>...`. The unchanged
   production boundary is `overdrive-worker/src/driver.rs:285`.

The clean baseline run was `6 tests run: 0 passed, 6 failed, 2,207 skipped`
(`nextest` run `2b83c322-5a1d-4024-9840-806239062711`). The refactored
focused run reproduced the same three rustc/trybuild mismatches and the same
three assertion failures. The refactor touched neither the failing test
files nor the three compile-fail production boundaries, and its production
changes are semantic-preserving local renames, extraction, and control-flow
layout. The six failures are therefore classified as pre-existing baseline
failures, not refactor regressions; no test or adjacent behavior is changed.
