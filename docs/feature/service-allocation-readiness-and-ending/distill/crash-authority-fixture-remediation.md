# D6 crash-authority acceptance fixture remediation

Status: bounded DISTILL test/support fallout from the approved D6 deletion of `ServiceAllocFact.exit_code`; pending independent DISTILL review and the original step 02-01 crafter's continuation. This work does not implement the D6 production behavior, create DES events, or commit the active step.

## Contract and reproduced failure

The authoritative feature delta § D6 item 1 and its interface table, with ADR-0258, delete `ServiceAllocFact.exit_code` and its ServiceLifecycle hydration. `WorkloadLifecycle` reads the final attempt's durable `AllocStatusRow.reason` and the current attempt's startup probe result at give-up. A terminal `Failed` or `Terminated` fact makes `ServiceLifecycle` emit **no action** and join `terminal_announced`, regardless of `started_at` (DG-12). The typed `HydrateError::ObservationRead` contract remains unchanged.

Before this remediation, the Metal command `cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(service_lifecycle_hydrate)'` failed compilation with 13 E0560/E0609 diagnostics: 11 obsolete `ServiceAllocFact { exit_code: ... }` fields and two reads of `fact.exit_code`. The same accepted field deletion also affected test fixtures in other crates and the ServiceLifecycle hydration characterization golden. The reproduced diagnostics are in `.context/step-02-01-distill-compile-before.log`.

## Exact test changes

| Old test assumption | Approved replacement or disposition |
|---|---|
| `service_lifecycle_hydrate.rs` GAP-1-AT-07 said `started_at: None` merely skips the old EarlyExit and startup-failure branches | The existing test now requires **zero ServiceLifecycle actions** and membership in `terminal_announced` for the terminal Failed fact. This is D6/DG-12's stronger observable result. |
| GAP-11 asserted `fact.exit_code` equals the crash row's extracted code or `None` | The same three typed reason cases remain. The renamed test asserts ServiceLifecycle hydration still projects `Failed` and `started_at`, while the durable observation row retains the exact crash or non-crash reason and payload for `WorkloadLifecycle` to read. There is no fact-level copy. The Metal placement identity was updated. |
| `service_lifecycle_stable.rs` constructed a Failed fact with an exit-code field and asserted only no EarlyExit after Stable | Its fixture no longer takes an exit code. The existing test still requires no action and now also requires terminal membership. Stable happy-path, dedup and startup-stop complements remain unchanged. |
| `service_lifecycle_reconcile_branches.rs` had two mutation tests for the deleted EarlyExit `no_pass`/deadline branch | Those obsolete tests and their Metal placement entries were removed. S-56 and the retained terminal-window tests exercise the approved no-action/membership rule. The private `fact` fixture's unused `_exit_code` parameter and 20 call-site arguments were removed; the remaining state-delta and unaffected branch tests are unchanged. Its focused suite passes 33/33 after the subtraction. |
| Other `ServiceAllocFact` literals carried neutral `exit_code` values | Removed only that obsolete member from the affected control-plane, core, reconcilers and sim test fixtures. `TerminalCondition` and `TransitionReason` exit-code payloads remain. |
| `fixtures/hydration_golden/service_lifecycle.txt` included `exit_code: None` in the debug snapshot | Removed exactly that one line. Before/after golden runs proved this was the only drift; the other six reconciler snapshots were unchanged. |

The test/support files changed by DISTILL are `crates/overdrive-control-plane/tests/acceptance/{service_lifecycle_hydrate.rs,service_lifecycle_liveness.rs,service_lifecycle_probe_to_stable.rs,service_lifecycle_readiness.rs,service_lifecycle_stable.rs,service_submit_event_taxonomy.rs,single_restart_authority_liveness_trajectory.rs,fixtures/hydration_golden/service_lifecycle.txt}`, `crates/overdrive-core/tests/{acceptance/service_lifecycle_reconcile_branches.rs,acceptance/service_lifecycle_startup_stop.rs,acceptance/vm_reclamation_plan_purity.rs,capture_advertise_port_set_equality.rs}`, `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs`, `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs`, and `infra/metal/test-placement.json`. The original crafter's production edits, activated pending markers and execution log were preserved.

## Verification and handoff

| Command or check | Result |
|---|---|
| Focused `service_lifecycle_hydrate` and `service_lifecycle_stable` acceptance run on Metal | 18/18 passed, including the rewritten tests. |
| Hydration characterization golden, before/after the one-line fixture update | Failed only on `exit_code: None`; then 1/1 passed. |
| Core/reconcilers/sim fixture-focused Metal run | 18/18 passed across 39 binaries; no compile errors from the deleted field. |
| Other affected control-plane acceptance modules | 24/24 passed. |
| Core `service_lifecycle_reconcile_branches` after removing the two obsolete branch tests | 33/33 passed. The first Metal attempt timed out acquiring another workspace's lease before execution; the retry ran to completion. |
| Activated step 02-01 D6/B3/DG-12 selection after compile repair | 15/17 passed. `pre_startup_crash_loop_ends_the_stream_with_early_exit_at_give_up` still fails its stderr-tail assertion; `restart_authority_pre_startup_crash_then_successor_passes` still diverges under seed `0x3fbbc5ad`. These are remaining original-crafter implementation/conformance work, not fixture compile failures. Log: `.context/step-02-01-distill-activated.log`. |
| Metal Clippy across affected crates with warnings denied | Blocked by the original crafter's `clippy::unnested_or_patterns` in production `workload_lifecycle.rs:1165`. A final scoped rerun allowing only that lint passed across all targets after the test subtraction. No production file was edited by DISTILL. |
| Changed test-file `rustfmt --check`, JSON parse and `git diff --check` | Passed. |

The original crafter should resume its own step 02-01 GREEN work and resolve the two remaining semantic failures and its production lint. The independent DISTILL reviewer should review this bounded accepted-contract test revision first. No acceptance marker was activated or deactivated by DISTILL; every transitioned test retains its per-test Contract Shape declaration, including the rewritten hydration test.

## Iteration 2 — retained hook and whole-workload model fidelity

The original step 02-01 crafter subsequently fixed the in-scope stderr projection and its production Clippy lint. Two bounded test-contract failures remained. DISTILL reproduced both against the current production composition before changing test support. The iteration-1 15/17 and Clippy observations above are historical; the evidence below supersedes their handoff status.

### D4/D2 terminal-hook count over a crash give-up

`cargo xtask metal run -- cargo nextest run -p overdrive-control-plane --features integration-tests --test acceptance -E 'test(pre_startup_crash_loop_ends_the_stream_with_early_exit_at_give_up)'` reproduced the failure: the acceptance body expected six `on_alloc_terminal` calls but observed seven (`.context/step-02-01-distill-hooks-before.log`). Its composition uses the real exit observer with the `HookedDriver` forwarding hooks to a real `ProbeRunner`. The observer calls the hook once for each of the six consumed natural-exit events (D1). The final `WorkloadLifecycle` `FinalizeFailed` is dispatched through `action_shim/mod.rs` and **retains its own hook** under approved D4; D2 makes that repeated final-id call idempotent. `ServiceLifecycle` adds no hook or teardown on the crashed rows under D6.

The existing body now proves the approved sequence and effect: before give-up it requires exactly six observer calls and zero active probe supervisors; after give-up it requires one call for each predecessor, **two** for the final id, seven in total, and still zero active supervisors. Its stream verdict, one terminal row, predecessor crash rows and restart-budget assertions remain. No hook was removed, and no production behavior changed. The focused test passes (`.context/step-02-01-distill-hooks-after.log`).

### D6/DG-12 membership in a whole-workload ServiceLifecycle tick

With `QUINT_SEED=0x3fbbc5ad` temporarily supplied through Cargo's environment configuration, the original named conformance run reproduced a precise difference at `SLStable(1)`: the model's predecessor attempt 0 had `ann=false`, while the driver's approved projection read `ann=true` from `ServiceLifecycleView.terminal_announced` (`.context/step-02-01-distill-quint-before.log`). The driver calls the real `run_convergence_tick_with_guest_network_provisioner_for_test` for the **workload target**, which hydrates all its allocation facts. `ServiceLifecycleReconciler::reconcile` iterates all those facts; its D6/DG-12 head branch adds every observed `Failed` or `Terminated` allocation to `terminal_announced` before it announces the successor's Stable. The existing Quint `SLStable(i)` transition had changed only attempt `i`; `SLAnnounceTerminal(i)` was a separate model action, although the real tick performs both effects together. The row and view projections were correct and remain unchanged.

The `restart_authority.qnt` abstraction now applies the already approved terminal-membership rule to the full attempt map on each ServiceLifecycle tick (`SLStable`, `SLStartupStop`, `SLLivenessStop`, `SLAnnounceTerminal`) before the target attempt's action-specific result. This adds no policy, actor, state field, guard or production seam. The conformance driver's action-map comment names the whole-workload scope; its actual driving call and honest projection are unchanged. No terminal is newly finalised by ServiceLifecycle. The exact seed now passes (`.context/step-02-01-distill-quint-after2.log`).

| Verification | Result |
|---|---|
| Step 02-01 activated acceptance and named conformance selection, same 17 tests as the crafter's failing selection | **17/17 passed** (`.context/step-02-01-distill-activated-after2.log`). |
| Full feature Quint check after the model-fidelity correction | **84/84 expected results passed**, including the restart-authority named run, safety/liveness checks and hazards (`.context/step-02-01-distill-quint-full.log`). The full run was repeated with `--record`; 84/84 passed and the generated evidence was pulled into this checkout. |
| `cargo xtask quint verify-evidence --subsystem service-allocation-readiness-and-ending` through Metal | Passed after re-recording; all current input hashes and expected outcomes match. Before re-recording it correctly reported the changed `restart_authority.qnt` as stale. |
| `cargo clippy -p overdrive-control-plane --all-targets --features integration-tests -- -D warnings` through Metal | Passed (`.context/step-02-01-distill-clippy-iteration2.log`). |
| Changed-file Rust formatting, evidence JSON parse and `git diff --check` | Passed. Workspace Metal `cargo fmt --all -- --check` still reports only two unrelated clean files: `crates/overdrive-testing/bin/fwd_guest_suite.rs` and `xtask/tests/acceptance.rs`; no file changed in this iteration appears in its diff. |

The temporary seed/verbosity settings in `.cargo/config.toml` were restored byte-for-byte; it is clean. DISTILL changed only the existing acceptance assertion, the existing Quint conformance driver's documentation, the Quint model's already approved D6/DG-12 abstraction, and the generated full-suite Quint evidence. The original crafter's production code, activated markers and DES events remain untouched. Independent acceptance review is required before the original crafter resumes step 02-01 and commits.
