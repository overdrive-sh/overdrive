# DELIVER review — 01-03 VmDriver shared supervision

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Step | `01-03` — VmDriver shared supervision |
| Reviewed commit | `9267b25fe1e91219a26e4a60f8c24a6380155189` |
| Base commit | `9267b25f^` (`058344a9bb271911655fac9ba6d270ad05942843`) |
| Iterations | 1 |
| Reviewer | Fresh independent implementation reviewer |
| Verdict | **APPROVED** |

## Scope and accepted contract

This review is limited to step 01-03 and ADR-0090 / DESIGN ACD-1. `VmDriver::new` must be exactly `(vmm, clock, fs, cgroup_accounting, probe_runner, layout)`, with the trusted runner a required fifth parameter and no alternate builder. The existing hooks must register all roles on Running, remove Startup only on Stable, and stop all probe supervision on a terminal allocation. The one boot-probed runner must be shared by the Exec and optional VM drivers without changing VM capability behavior or the `Driver` API.

The worktree already had unrelated modified and untracked feature/design/roadmap files. They were not treated as part of commit `9267b25f` and were not changed by this review.

## Iteration 1 evidence

### Design and API exactness

- `VmDriver::new` has the exact six-parameter order required by ADR-0090 at `crates/overdrive-worker/src/vm_driver.rs:989-997`. The runner is stored as `Arc<ProbeRunner>` at `:969-978`; no VM-specific runner, `Driver` trait method, optional constructor argument, or builder was added.
- `VmDriver` implements only the three existing lifecycle hooks: Running calls `start_alloc(spec)`, terminal calls `stop_alloc(alloc_id)`, and Stable calls `stop_role(alloc_id, ProbeRole::Startup)` at `vm_driver.rs:1874-1884`. This preserves Startup/Readiness/Liveness ownership exactly as the design specifies.
- Every pre-existing `VmDriver::new` construction in the commit receives an explicit runner. The neutral changes in the existing VM acceptance fixtures are compiler-required fallout from the new mandatory dependency and do not add VM behavior.

### Production composition and ownership path

- `run_server` creates the trusted runner once through `compose_production_driver` at `crates/overdrive-control-plane/src/lib.rs:1689-1699`, retains it rather than discarding the tuple value, gives the Exec driver its clone inside `compose_production_driver` (`:2067-2081`), and passes `Arc::clone(&probe_runner)` into the existing private `compose_vm_driver` (`:1738-1746`). `compose_vm_driver` forwards that same argument directly to `VmDriver::new` (`:1861-1869`, `:1997-2004`). VM discovery/probe and its `NotAvailable` versus `Refused` branches remain in place before construction, so the capability gate is unchanged.
- On the real start path, the action shim first writes the successful Running observation and then calls `driver.on_alloc_running(&spec)` at `crates/overdrive-control-plane/src/action_shim/mod.rs:2150-2203`. On a Stable transition it commits the non-terminal row and calls `on_alloc_stable` at `:1727-1736`; genuine terminals call `on_alloc_terminal` only for the indexed owning driver at `:1745-1754`. The restart success path calls `on_alloc_running(&spec)` with the action's current, newly provisioned spec at `:2625-2671`.
- Restart does not leave an old task set live: its production precondition is a prior terminal allocation (`action_shim/mod.rs:2218-2234`), and the terminal paths above cancel that allocation's runner before the subsequent Running handoff. `ProbeRunner::start_alloc` then preserves the established one-supervisor idempotence guard at `crates/overdrive-worker/src/probe_runner/mod.rs:298-367`, so the current `AllocationSpec` becomes the one task set for the new allocation generation without duplicate tasks.

### Tests, scope, and verification

- S-SVM-15 is transitioned to behavioral lifecycle assertions at `crates/overdrive-worker/tests/acceptance/service_kind_vm_workloads.rs:300-330`: all three roles start, Stable cancels Startup only, and terminal removes the allocation supervisor.
- S-SVM-16 models the terminal-to-restart handoff and verifies re-registration from the current spec leaves exactly one supervisor at `:332-362`. S-SVM-22 verifies one earned-trust probe, shared supervision across the two driver implementations, and terminal cleanup at `crates/overdrive-control-plane/tests/acceptance/service_kind_vm_workloads.rs:73-149`.
- The tests use real public driver/runner APIs and assert state changes that would fail if the forwarding hooks or terminal cancellation were removed. They contain the required `CONTRACT_SHAPE: bounded-change.` declarations and do not construct a `Vm + None` case.
- The execution log contains successful RED, GREEN, and COMMIT events for `01-03` in `docs/feature/service-kind-vm-workloads/deliver/execution-log.json`.
- Required scoped verification passed: `cargo xtask lima run -- cargo nextest run -p overdrive-worker -p overdrive-control-plane --test acceptance -E 'test(vm_driver_delegates_existing_probe_lifecycle_hooks_to_shared_runner) or test(vm_restart_reregistration_is_idempotent_and_does_not_duplicate_probe_tasks) or test(one_server_boot_shares_exactly_one_trusted_probe_runner_with_both_drivers)'` — 3 passed.
- `git diff --check 9267b25f^ 9267b25f` completed cleanly. Mutation testing was not run, per the DELIVER final-wave-only rule.

## Findings

No findings. The initially inspected restart path is reachable only after a terminal prior allocation; the action shim's terminal hook cancels the old supervisor before the new Running hook receives the current spec. Consequently there is no reachable stale-supervisor or duplicate-task failure to remediate.

## Remediation disposition

No remediation is required. No design or API expansion is proposed.

## Final verdict

**APPROVED.** Step 01-03 matches the accepted constructor and lifecycle-hook contract, wires the single trusted runner through the real `serve` composition path, preserves the existing VM capability behavior and Driver API, and supplies focused behavioral evidence without testing theater or scope expansion.
