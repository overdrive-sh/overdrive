# RCA: Crash-restart never stops the predecessor allocation's probe tasks (GH #318)

Investigated at HEAD `02d0787b7` (branch `marcus-sa/nw-bugfix-crash-restart-tasks`). Source reading only; no tests were run.

## 1. Problem statement

When a VM allocation crashes, the exit observer writes a `Failed` row. `WorkloadLifecycle` then emits `Action::RestartAllocation { alloc_id: <predecessor>, spec: <successor, fresh alloc id> }`. The shim starts the successor and calls `on_alloc_running(&spec)`, which starts a second probe supervisor. Nothing ever stops the predecessor's supervisor. Its per-probe tasks loop until process exit, still writing `ProbeResultRow`s for a dead allocation id, once per probe interval, on every crash-restart.

Scope: the crash-restart path (`RestartAllocation`) with probe-bearing VM allocations (`VmDriver` is the only `Driver` that overrides `on_alloc_terminal`).

Unaffected (verified): liveness kill (`StopAllocation`, `crates/overdrive-control-plane/src/action_shim/mod.rs:3258`) and restart-budget exhaustion / other genuine terminals (`FinalizeFailed`, `mod.rs:2162`).

## 2. Verification of the issue's claims against HEAD

| Issue claim | Verdict | Current location |
|---|---|---|
| Probe tasks stop only via `Driver::on_alloc_terminal` -> `probe_runner.stop_alloc` | Confirmed | `crates/overdrive-worker/src/vm_driver.rs:2061-2063` (was ~1946); `probe_runner/mod.rs:374-379` removes the supervisor and cancels it |
| `cleanup_restart_predecessor` never calls `on_alloc_terminal` | Confirmed | `action_shim/mod.rs:1763-1792`: retire lease, `driver.stop` loop (1774-1780), `mtls_lifecycle.stop_alloc` (1781), `teardown_for_dispatch` (1782-1789), `alloc_drivers.lock().remove` (1790) |
| After the index removal nothing resolves the old id to a driver | Confirmed | `resolve_drivers_for_alloc` (`mod.rs:970-980`) falls back to *every* composed driver on an index miss, so it is not literally unreachable. But no code path calls the hook, and the terminal arms use the index directly (`mod.rs:2159-2160`, `3255-3256`) |
| `on_alloc_terminal` called only from FinalizeFailed terminal arm and StopAllocation arm | Confirmed | Only production call sites: `mod.rs:2162` and `mod.rs:3258` (grep over `crates/`, non-test). `exit_observer.rs` has no probe/hook reference |
| Successor gets a fresh alloc id; `on_alloc_running` starts a second supervisor | Confirmed | `workload_lifecycle.rs:1583` `restart_allocation_action(..., successor_alloc_id)`; `SpiffeId::for_allocation(&job.id, &successor_alloc_id)`; shim `mod.rs:3188` `driver.on_alloc_running(&spec)` |
| Probe loop exits only on cancellation | Confirmed | `probe_runner/mod.rs:776-781` (`supervised_probe_loop`, `biased; child_token.cancelled() => return`), no other exit |
| S-SVM-16 masks the gap | Confirmed | `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:489-492` hand-calls `driver.on_alloc_terminal(&initial.alloc)` before re-registering |

## 3. Five Whys

PROBLEM: after each crash-restart, the predecessor's probe tasks keep running forever. `ProbeRunner::active_alloc_count()` grows by one per restart.

### Branch A (primary): the hook is never called on the restart path

- **WHY 1** The predecessor's supervisor stays in `ProbeRunner.supervisors` and its tasks keep ticking. [Evidence: `stop_alloc` is the only remover, `probe_runner/mod.rs:374-379`; the loop exits only on cancel, `mod.rs:776-781`.]
- **WHY 2** `stop_alloc` is never invoked for the predecessor. [Evidence: its only caller is `VmDriver::on_alloc_terminal`, `vm_driver.rs:2061`. The only production callers of `on_alloc_terminal` are `action_shim/mod.rs:2162` and `:3258`. `RestartAllocation` (`mod.rs:2684-3206`) reaches the predecessor only through `finish_restart` -> `cleanup_restart_predecessor`, which does not call it.]
- **WHY 3** The `RestartAllocation` predecessor cleanup was written as a third, hand-rolled copy of the terminal-cleanup sequence, and it omits one step. [Evidence: the three sites share the sequence "retire lease; driver.stop; mtls stop_alloc; teardown_for_dispatch". `FinalizeFailed` (`2149-2163`) and `StopAllocation` (`3245-3259`) both end with `on_alloc_terminal`, resolved via the alloc->driver index. `cleanup_restart_predecessor` (`1773-1790`) has no such step. Commit `c830b5545` ("complete successor before exact-old cleanup") introduced the helper.]
- **WHY 4** The design treats the hook as part of "writing a terminal row" rather than part of "ending an allocation". The `Driver` contract says the hook fires "immediately after the action shim writes the terminal `AllocStatusRow`" (`overdrive-core/src/traits/driver.rs:739-751`). The restart path never writes a terminal row for the predecessor. The exit observer wrote it earlier, and the observer has no hook. The row is already `Failed` when `RestartAllocation` arrives (the arm requires `Failed | Terminated`, `mod.rs:2694`), so the "after the terminal write" trigger point does not exist on this path. In effect the hook has two owners (the terminal arms of the shim, and nobody on exit-observer-originated terminals) and the contract does not say what the restart path owes.
- **WHY 5 (ROOT CAUSE A)** There is no single owner of "an allocation's per-alloc resources are released". Each effect (lease, driver stop, mTLS, netns, index entry, probe supervisor) is released by a sequence copied into each terminal path. Nothing structural (shared helper, exhaustive test) forces a new path to cover the set. The 'successor-first, exact-old cleanup' restructuring (ADR-0105/0106 work, `c830b5545`) copied the sequence and dropped the probe-supervisor step. [Evidence: three divergent copies; the diff of `cleanup_restart_predecessor` vs. `mod.rs:2149-2163` is exactly the `on_alloc_terminal` call; a detail not in the comment at `mod.rs:1759-1761`, which lists only "capability captured before successor work".]

### Branch B (test masking): no test exercises the shim path

- **WHY 1** The leak shipped undetected. [Evidence: issue filed from reading, no failing test.]
- **WHY 2** The only test that crosses "terminal -> restart" for probes is S-SVM-16, at the *driver* level, and it hand-calls the hook (`service_kind_vm_workloads.rs:489-492`). The shim-level `RestartAllocation` tests (`action_shim/replacement_acceptance.rs`, `action_shim_restart_uses_spec_from_action.rs`, `admission_refusal_acceptance` in `mod.rs`) use drivers that do not override `on_alloc_terminal` (`RecordingDriver` at `replacement_acceptance.rs:178-232`, `mod.rs:4030-4078`) and `SimDriver` (which keeps the default, `overdrive-sim/src/adapters/driver.rs:392+`).
- **WHY 3** `TraceEvent` in `replacement_acceptance.rs:77-84` has no terminal-hook variant, so ordering/precedence assertions cover Start/Stop/Mtls/Provision/Teardown only. The hook is a defaulted no-op, so omission is invisible to the compiler and to every test double.
- **WHY 4** The test author modelled the driver-level re-registration as "the shim called terminal, then running" (S-SVM-16's docstring: "registering the same allocation again after restart"). That models the lifecycle the contract describes, not the sequence the shim performs. It also uses the *same* alloc id for both registrations, which the production path never does (successor id is fresh).
- **WHY 5 (ROOT CAUSE B)** The lifecycle-hook contract is verified only at the driver unit boundary, not at the action-shim composition boundary, so a mismatch between "what the shim calls" and "what the driver assumes was called" is untestable. This is the `.claude/rules/testing.md` "DISTILL: prove composed system behavior" gap (cross-component criterion covered only by isolated tests).

### Contributing factors

1. **Fresh successor id.** Because the successor has a new id (`workload_lifecycle.rs:1583`), `start_alloc`'s idempotence (`probe_runner/mod.rs:296-300`, `is_started`) cannot absorb the duplicate; it was designed for same-id re-registration. The leak is a growing set, not a replaced one.
2. **Hook is a defaulted no-op** (`driver.rs:751`). Omitting it compiles everywhere; only `VmDriver` has an effect, so the sim and recording drivers hide the omission.
3. **Exit observer has no hook** and is not the owner (`worker/exit_observer.rs` contains no probe/`on_alloc_*` reference). A crashed-and-not-yet-restarted alloc (restart backoff window) also keeps probing for the entire backoff, which is a related, smaller leak: probes against a dead VM write failing `ProbeResultRow`s. Not addressed by the minimal fix; tracked as [#334](https://github.com/overdrive-sh/overdrive/issues/334).
4. **Resource effects of the leak.** Each leaked supervisor holds `tokio` tasks, a `Clock` sleep per interval, TCP/HTTP probe attempts to a guest address that is no longer valid (the guest network lease was retired at `mod.rs:1773`), and keeps writing rows keyed by the dead alloc id. These compete for the guest-dial capacity (`GuestDialError::Capacity`, `probe_runner/mod.rs:~590`), so the leak couples to the 16,384-lease admission cap work.
5. **Index removal immediately after cleanup** (`mod.rs:1790`) means any post-hoc fix keyed on the index must run before it.

### Cross-validation

- A (hook omitted) + B (no composed test): consistent. A explains the symptom; B explains why it survived review and the mutation gate (`cleanup_restart_predecessor` has no assertion on the hook).
- Backward chain: if the predecessor cleanup omits `on_alloc_terminal` (A) and the successor registers a different id (factor 1), then `active_alloc_count()` rises by one per crash-restart and the old supervisor never exits (WHY 1). Matches. Liveness-kill and exhaustion paths go through the terminal arms that *do* call the hook, matching the "unaffected" list. All symptoms explained.

## 4. Answers to the specific questions

**Other drivers.** `VmDriver` (`vm_driver.rs:2061`) is the only production `Driver` overriding `on_alloc_terminal`. `SimDriver` (`overdrive-sim/src/adapters/driver.rs:392`) and every other production `impl Driver` use the trait default no-op, so no other production driver leaks probe tasks via this gap. Test-only wrappers (`netns_density_activation_order.rs:439`, `netns_density_guest_network.rs:336`, `allocation_restart_write_spike.rs:124`) forward to the inner driver but are not production. Other per-alloc cleanup in the restart predecessor path (lease, mTLS, netns, index) is covered. The only missing step is the probe supervisor.

**Trait contract (`overdrive-core/src/traits/driver.rs:739-751`).** "Fired by the action shim when an allocation transitions to a terminal state (Terminated / Failed) -- immediately after the action shim writes the terminal `AllocStatusRow`." Implementations "call `probe_runner.stop_alloc(alloc_id)`... shutting down every per-probe task spawned under the allocation's supervisor". The doc says nothing about restart or exit-observer-originated terminals, which is the contract gap behind Branch A WHY 4. `on_alloc_stable` (770) explicitly says routing Stable through terminal was a category error, so the two hooks are distinct and `on_alloc_terminal` is the correct (full-teardown) hook for an allocation that is being replaced.

**Observable for a shim-level test.** `ProbeRunner::active_alloc_count()` (`probe_runner/mod.rs:414`) and `is_role_live(alloc, role)` (`:402`) are existing public inspection surfaces (used by S-SVM-15/16 and the shared-runner test at `crates/overdrive-control-plane/tests/acceptance/service_kind_vm_workloads.rs:151-153`). `SimDriver` has no probe runner; the existing shim-level test drivers record calls only for `stop`/`start`/`on_alloc_running` (`mod.rs:4013`, `4075`). A recording `on_alloc_terminal` override on the test-local driver is the natural observable for the shim ordering contract.

**Shim-level harnesses driving `RestartAllocation`.**
- `crates/overdrive-control-plane/src/action_shim/replacement_acceptance.rs`: `dispatch_one` (413) wraps `dispatch_with_network_provisioner_and_guest`; `RecordingDriver` (`:178` impl), `TraceEvent` ordered trace, `seeded_predecessor` (481). Tests `successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver` (503) and `successor_and_cleanup_outcomes_follow_the_ratified_precedence_table` (612).
- `action_shim/mod.rs` inline `admission_refusal_acceptance` (4638; `a_refused_restart_successor_still_cleans_up_its_predecessor_once`), and `fail_closed_mtls_tests` with its own `RecordingDriver` (4030).
- `crates/overdrive-control-plane/tests/acceptance/action_shim_restart_uses_spec_from_action.rs` and `tests/integration/mtls_install_fail_closed.rs`.

**Ordering: before or after `driver.stop`?** The terminal arms call the hook after `driver.stop`, mtls stop, and teardown, resolved through the still-present index entry (`mod.rs:2149-2163`, `3245-3259`). The predecessor cleanup should call it after the `driver.stop` loop and before the index removal. Placing it after the stop loop matches both existing arms (stop precedes the hook; probes of a stopped VM no longer have a live target). I recommend placing it immediately after the stop loop, before `mtls_lifecycle.stop_alloc` / `teardown_for_dispatch`, rather than after teardown, for one reason that departs from strict parity: `stop_alloc` on the probe runner is in-memory and infallible, whereas `mtls stop_alloc` and `teardown_for_dispatch` can return `Err` (`?`), short-circuiting the function. If the hook sat after them, a transient mTLS/teardown failure would leave the dead alloc's probe supervisor running even though the driver is already stopped. The call is idempotent, so the retried cleanup calling it again is harmless. If strict parity with the terminal arms is preferred, place it between `teardown_for_dispatch` and the index removal; the regression test below passes either way for the success path. Surface this choice to the user; it is within the existing hook, not new API.

## 5. Proposed minimal fix (no new API)

In `crates/overdrive-control-plane/src/action_shim/mod.rs`, `cleanup_restart_predecessor`:

```rust
    retire_guest_network_lease(&handle.alloc, guest_network);
    for driver in prior_drivers {
        if let Err(error) = driver.stop(handle).await
            && !matches!(error, DriverError::NotFound { .. })
        {
            return Err(error.into());
        }
    }
    // The predecessor is terminal and stopped: release its probe supervisor
    // while `prior_drivers` (captured before successor work) still names its
    // owner. Idempotent; the index entry removed below is not consulted.
    for driver in prior_drivers {
        driver.on_alloc_terminal(&handle.alloc);
    }
    mtls_lifecycle.stop_alloc(&handle.alloc).await?;
    ...
```

Notes:
- Uses `prior_drivers`, already captured at `mod.rs:2701` before successor work, so the hook cannot be redirected through a newer allocation (the same invariant the function docstring states, `1759-1761`). No index lookup, no race with the removal at 1790.
- On an index miss `prior_drivers` is every composed driver (`resolve_drivers_for_alloc`, 975-979). `on_alloc_terminal` is a no-op for drivers that never supervised the id (`stop_alloc` on an unknown id is documented idempotent, `probe_runner/mod.rs:370-373`), and only `VmDriver` overrides it today. `driver.stop` already uses the same fan-out, so this does not widen blast radius. (The `FinalizeFailed` arm's comment at `mod.rs:2124-2127` prefers an active-owner-only hook; here the existing `driver.stop` already addresses all `prior_drivers`, so keeping them consistent is simplest. Flag to reviewer.)
- `finish_restart` calls cleanup on every successor outcome path (success, provision failure, admission refusal), so the predecessor's probes are released on all of them.
- Also update the `on_alloc_terminal` rustdoc in `driver.rs:739-751` to state that the shim fires it for every terminal allocation it ends, including a replaced restart predecessor whose terminal row was authored by the exit observer. This documents the contract (no signature change). Per the DESIGN/rules discipline this is a doc-only edit to a port trait; confirm the user is fine with touching it.

**No new public API is needed.** No blocker.

## 6. Files affected

- `crates/overdrive-control-plane/src/action_shim/mod.rs` (`cleanup_restart_predecessor`, ~1773-1792): the fix.
- `crates/overdrive-core/src/traits/driver.rs` (rustdoc at 739-751 only): contract clarification.
- Tests (see below): `crates/overdrive-control-plane/src/action_shim/replacement_acceptance.rs`.
- Optional follow-up for the user to decide (do not add unilaterally): S-SVM-16 (`crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:489-492`) still hand-calls the hook; its docstring should stop implying the shim performs "terminal then running" with the same id.

## 7. Proposed regression test

**Location:** `crates/overdrive-control-plane/src/action_shim/replacement_acceptance.rs`, beside `successor_outcome_precedes_blocked_predecessor_cleanup_for_every_driver`. Same harness: `RecordingDriver`, `dispatch_one`, `seeded_predecessor`, `RecordingMtlsLifecycle`, `RecordingNetwork`, `SimObservationStore`, `AllocDriverIndex`.

**Test-support change (test-local, DISTILL-owned per the rules; no production surface):** add `fn on_alloc_terminal(&self, alloc: &AllocationId)` to `impl Driver for RecordingDriver` that records into a new `terminals: Mutex<Vec<AllocationId>>` field and pushes a new `TraceEvent::Terminal(AllocationId)` onto the shared trace.

**Test:** `crash_restart_releases_the_predecessors_probe_supervision_exactly_once`
1. `seeded_predecessor(DriverType::Vm)` (Failed predecessor, fresh successor id), `RecordingDriver::new(..)` (not blocked), seed `alloc_drivers[predecessor] = Vm`, start mTLS for the predecessor as in the neighbouring test, clear the trace.
2. `dispatch_one(Action::RestartAllocation { alloc_id: predecessor, spec: successor_spec(..), kind: Service }, ...)`; `.expect("restart succeeds")`.
3. Assertions (exact):
   - `driver.terminals.lock().as_slice() == std::slice::from_ref(&predecessor)`: the hook fired exactly once and only for the predecessor, never the successor.
   - In `driver.trace`, `position(Stop(predecessor)) < position(Terminal(predecessor))`, and `position(Start(successor)) < position(Terminal(predecessor))` (successor-first precedence preserved, ADR-0105/0106).
   - `!alloc_drivers.lock().contains_key(&predecessor)` (existing invariant retained).
4. Companion case, same file, reusing `with_failed_predecessor_stop()`: when `driver.stop` fails, assert the hook is NOT called and the index entry remains (so a retry still owns it). Pins the ordering decision above.

**Why it fails on current code:** `cleanup_restart_predecessor` never invokes `on_alloc_terminal`, so `terminals` is empty and the first assertion fails (`[] != [predecessor]`) while the pre-existing assertions pass. Verify RED by running the new test before applying the fix: `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests -E 'test(crash_restart_releases_the_predecessors_probe_supervision_exactly_once)'` (module is `#![cfg(feature = "integration-tests")]`, `replacement_acceptance.rs:8`).

**Optional second layer (real runner):** drive the same dispatch with `VmDriver` backed by the shared `ProbeRunner` (the fixture in `tests/acceptance/service_kind_vm_workloads.rs:100-153` shows how) and assert `runner.active_alloc_count() == 1` (successor only) after the restart. This is the issue's literal symptom, but it is heavier; propose only if the user wants it (agents propose, the user adds).

**Mutation:** the new assertions kill the mutant "delete the `on_alloc_terminal` loop" in `cleanup_restart_predecessor`, which is a mandatory-target, diff-scoped file.

## 8. Risk assessment

- **Low code risk.** One idempotent, infallible, in-memory call added to an existing cleanup function; same hook already used by the two sibling terminal arms.
- **Ordering risk:** if placed before `mtls stop`/teardown, it runs even when those later fail; harmless because the call is idempotent and the VM is already stopped. If the retried `RestartAllocation` re-runs cleanup, `stop_alloc` on an absent id is a no-op.
- **Fan-out on index miss:** `prior_drivers` can be all composed drivers; only `VmDriver` has a non-default body and ignores unknown ids. Future drivers overriding `on_alloc_terminal` must keep it idempotent for unknown ids (add to the trait rustdoc).
- **No behaviour change on success path** for successor supervision: the successor id differs, so `stop_alloc(predecessor)` cannot touch the successor's supervisor. The `BTreeMap` is keyed by alloc id (`probe_runner/mod.rs` `supervisors`).
- **Residual (not fixed):** probes of an exited-but-not-yet-restarted allocation keep running during restart backoff, because the exit observer does not fire the hook. Fixing that is a design change to who owns the hook for observer-authored terminals; tracked as [#334](https://github.com/overdrive-sh/overdrive/issues/334).
- **Process-loss case:** probe tasks are process-local, so a control-plane restart already drops them; this leak is a live-process phenomenon only.
