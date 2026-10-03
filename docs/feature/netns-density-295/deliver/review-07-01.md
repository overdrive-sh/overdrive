# DELIVER Review — Step 07-01

## Metadata

| Field | Value |
|---|---|
| Feature | `netns-density-295` |
| Step | `07-01` — awaited convergent element release and the five B-6 caller rules |
| Reviewer | `nw-software-crafter-reviewer` |
| Model | GPT-6 Luna, maximum reasoning |
| Iteration | 1 and 2 |
| Commits reviewed | `3e85d73f4cfe9820a0a0714a4bcc887e741d4d64`, `e5c5d562d9be976cf2ff3b64722b0f5ce59c11f8` |
| Verdict history | Iteration 1: **NEEDS REVISION — required verification gate unresolved**; Iteration 2: **APPROVED** |

The review applied `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`, and `nw-tdd-methodology`. The roadmap validation is approved (iteration 4, recorded 2026-10-03).

## Scope and contract sources

Reviewed the implementation and activated bodies against roadmap step 07-01, the accepted feature-delta sections for the R10 intercept-element release and B-6 caller contract, and DISTILL scenarios S-ND295-07, 07B, 21, and 54. The review covered the exact port and netlink shapes, convergent removal and read-back, the not-yet-observed guard value, the TCP listener projection, retirement retry ownership, shutdown sealing, typed stop errors and persisted detail, test activation/integrity, and the required verification commands.

The required control-plane integration command has one reproduced failure. Its test does not reach the intended exit-observer oracle because its fixture omits the shared mTLS owner startup required by the current allocation start path. This is a fixture gap owned by DISTILL; it is not evidence of a 07-01 production defect and does not authorize production recovery behavior or a changed assertion.

## Contract Shape Compliance

The changed interface shapes match the accepted contract. `MtlsIntercept::remove_allocation_elements(&self, Ipv4Addr, &[SocketAddrV4]) -> Result<InterceptState>` and `Client::local_route_present(&self, u32, &str) -> Result<bool, NetlinkError>` use the signatures pinned in the feature delta. `ServiceV1::listen_ports()` retains its existing signature. No public method, type, enum variant, trait member, parameter, wire field, persisted field, or error variant was added.

The worker changes (`element_effects`, retained stop generations, `completed_stops` under test/integration cfg, and the stop-attempt helpers) are private. `completed_stops` is test-only support behind an existing test query, not production state. The new private netlink element-I/O seam was already specified as module-private DISTILL support. The synchronous `MtlsIntercept` removal call is invoked in an awaited `spawn_blocking` task; the worker waits for the task result and maps a join failure into the existing typed `InterceptError`/`MtlsInterceptStopError` path. It does not detach the effect or return before it finishes.

**Source evidence:** feature-delta port and B-6 contract at `feature-delta.md:3218-3348` and `:3600-3670`; the exact removal semantics and read-back/rollback rules at `:3339-3370`; the worker implementation at `mtls_intercept_worker.rs:2475-2611`, `:2649-2750`, and `:2876-3025`; the host adapter at `mtls_intercept_port.rs:1210-1299`; and the netlink mutation/read-back at `nft.rs:4274-4320` and `:4380-4418`.

## Implementation review

`HostMtlsIntercept::remove_allocation_elements` rejects duplicate and zero-port destinations before I/O, requires a recorded program and matching observed identity, then deletes only the present requested subset in one atomic netlink transaction. `mutate_and_readback` verifies the exact post-state and complement; a failed or mismatched post-read performs one inverse transition and verification while retaining both causes. The host retires process-local tokens only after removal and policy-route read-back succeed. On error the worker keeps the drain, guards, and Retiring record for retry. Members already absent converge without a delete transaction.

`Client::local_route_present` performs a route dump and matches table, output interface, default destination, no gateway, local route type, and host scope. The 07-01 adapter reports the policy-route state from the existing fwmark-rule read and this route read. The host and Sim report `intercept_mark_guard = false`; the port rustdoc for all methods returning `InterceptState` and the field rustdoc state that, through 08-01, false means not observed present and exact.

The PORT-295-C projection filters to TCP and uses a seen set while traversing the declaration sequence. It therefore keeps the first TCP occurrence of each port in order and excludes UDP, matching `[8080, 8443]` for the pinned mixed listener list. Both `ServiceV1::listen_ports` and `project_service_listen_ports` document that contract; the activated pure-function body has the exact `/// CONTRACT_SHAPE: pure-function.` declaration.

The worker implements the B-6 caller rules as specified:

1. Failed enforced-handle teardowns retain the exact handles and drain. Joined callers await the same stored result, including pointer-equal typed sources.
2. The first stop after a completed error claims one retry while holding the lifecycle/read and allocation-state locks; simultaneous later callers join that retry and receive its result.
3. The owner shutdown synchronously seals the lifecycle before scheduling its drain, snapshots active and pending allocations under the lifecycle write fence, and places active shutdown stops in `stopping` before inline teardown. A caller after the seal starts no allocation attempt and waits for the owner result.
4. Successful stops remove their `stopping` entry only after completion, so a later Pending generation cannot join an earlier completed stop. Shutdown failure aggregation keeps at most one latest result per allocation.
5. `MtlsInterceptStopError::HandleTeardown` renders each connection and typed cause in teardown order; `ElementRemoval` renders its removal cause. `restart_abort_cleanup_detail` carries those Displays into the persisted Failed-row detail without stringifying away the underlying typed stop result before formatting.

The removal task runs in `start_capability_retirement`, the retry task, and the owner shutdown path through the same `run_capability_stop_attempt`. Each awaits the port call's blocking task before dropping element guards, completing the capability drain, or returning success. This preserves the release ordering while allowing other executor work, including stop callers, to run during a held synchronous adapter call.

## Test integrity, coverage, and budget

The commit activates only the assigned 07-01 bodies: S-ND295-07 and its seeded Sim body, S-ND295-07B, S-ND295-21, S-ND295-54, and the two DR-06 detail bodies. It removes their pending ignore markers. The only changed expectation-adjacent value is `intercept_mark_guard = false` in the Sim/test adapters, which aligns the fixtures with the accepted pre-08-01 meaning. No assertion, expected result, stimulus, or test was deleted, weakened, or skipped. All newly active bodies carry their applicable `CONTRACT_SHAPE` declaration and outcome anchor; source-local pure-function properties use the exact required rustdoc line.

The activated worker tests enter through `stop_alloc`/`shutdown_owner`; the worker Lima tests exercise the real host adapter and kernel; the control-plane bodies exercise the action owner and persisted row; and the seeded Sim body exercises the production action composition in-process. No Rust test starts the production binary or acts as an expectation runner.

The activated implementation/unit bodies comprise 11 tests across 11 distinct unit-level outcomes: one projection, one source-local adapter argument-refusal ordering body, four netlink batch/read-back outcomes, and five worker retirement outcomes. The budget is 22 (`11 × 2`), so the 11 bodies are within budget. The integration and seeded bodies remain separate evidence layers.

| Gate | Result | Evidence |
|---|---|---|
| G1 — activation scope | PASS | Only the bodies assigned to 07-01, including the seeded-retirement and two DR-06 detail bodies, were activated. |
| G2 — valid RED | PASS, recorded | `execution-log.json` records RED FAIL then RED PASS for 07-01. |
| G3 — meaningful assertions | PASS | Bodies check exact state, typed causes, member complements, shutdown outcomes, and lease/address ordering. |
| G4 — boundary discipline | PASS | Adapter effects use the specified module-private seam or real Lima kernel; control-plane tests remain in-process. |
| G5 — business/domain language | PASS | Activated tests retain their scenario IDs, outcome anchors, and declarations. |
| G6 — all required tests green | **BLOCKER** | Required control-plane integration command reproduced 25 passed / 1 failed. |
| G7 — green before commit | **BLOCKER** | The log records GREEN PASS and COMMIT PASS, but the required verification command is not green. The phase record cannot be accepted as truthful until the fixture gate is resolved and the original step crafter records the actual result. |
| G8 — test budget | PASS | 11 implementation/unit bodies for the step's distinct unit outcomes; within the `2 × behaviors` limit. |
| G9 — no test weakening | PASS | Diff inspection shows activation markers and the accepted guard-value correction; no test oracle was reduced. |
| External validity | PASS for 07-01 bodies | The assigned acceptance, worker, Sim, and Lima bodies reach their intended driving ports. The unrelated failing exit-observer body does not reach its own intended prior-Running oracle. |

## Required verification results

The following results were reported for the step. The control-plane integration command was independently rerun; the exact failing body was also rerun alone.

| Command/result | Outcome |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --lib -E 'test(mtls_intercept_worker) or test(mtls_intercept_port)' --no-fail-fast` | Reported pass: 41 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-worker --test integration --features integration-tests -E 'test(shared_intercept_members)' --no-fail-fast` | Reported pass: 2 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --lib -E 'test(service_projection_keeps_first_tcp_order_deduplicates_tcp_and_excludes_udp)' --no-fail-fast` | Reported pass: 1 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(netns_density_guest_network)' --no-fail-fast` | Reported pass: 3 |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(mtls_install_fail_closed) or test(server_lifecycle) or test(shared_element_cleanup_failure)' --no-fail-fast` | **Failed: 25 passed, 1 failed.** Failure: `integration::workload_lifecycle::exit_observer::exit_observer_lifecycle_from_reflects_prior_running_state`, panic at `exit_observer.rs:229`: `alloc must reach Running`. Independently reproduced. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(exit_observer_lifecycle_from_reflects_prior_running_state)' --no-fail-fast` | **Failed: 0 passed, 1 failed.** Same panic at `exit_observer.rs:229`. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --test acceptance -E 'test(netns_density_retiring_cleanup)' --no-fail-fast` | Reported pass: 1 |
| Netlink element rollback seam | Reported pass: 4 |
| Workspace check, clippy, formatting, and `git diff --check` | Reported pass |

### Reproduction and owner-path trace for the failing gate

The failing test's claimed prior-Running precondition is not established by its current fixture:

- `build_harness` constructs an `AppState` with a worker and owner at `exit_observer.rs:129-149`, but never calls `MtlsInterceptWorker::start_shared_owner` before returning. The only shared-owner startup found in the production composition is in `run_server_with_obs_and_driver`, before serving, at `lib.rs:7590-7605`.
- `drive_to_first_running` calls `run_convergence_tick_with_network_provisioner_for_test` at `exit_observer.rs:206-224`. That test dispatch chooses `dispatch_with_workflow_intent_and_network_provisioner_for_test` (`action_shim/mod.rs:1367-1399`), which supplies the legacy network-provisioner seam and no guest-network provisioner. Its test fallback assigns and injects a network assignment (`action_shim/mod.rs:1485-1506`, `:1514-1526`), so `spec.network` is present.
- The real worker's `start_alloc` takes the shared-allocation path for any present `spec.network` (`mtls_intercept_worker.rs:2369-2372`). `start_shared_allocation` returns `MtlsSharedOwnerError::NotStarted` when the shared owner was not started (`:2381-2389`). The action shim handles that install refusal through its fail-closed path after the Running write (`action_shim/mod.rs:2625-2639`), so the test's observation store never contains the Running row that `drive_to_first_running` requires.
- The failure occurs before `inject_exit_after` and before the `from == Running, to == Failed` assertion. The test does not demonstrate an exit-observer production defect. The fixture must compose the required owner startup/current owner path while preserving the existing oracle; test fixture and scenario semantics belong to DISTILL.

This is a bounded fixture-precondition gap, not authority for a blanket skip, assertion change, production recovery path, or 07-01 expansion. Until the required command passes, G6 and G7 remain blockers.

## DES, commit, and workspace evidence

The step log at `execution-log.json` records RED FAIL → RED PASS → GREEN PASS → COMMIT PASS. The independently reproduced required-command failure means the recorded GREEN PASS and downstream COMMIT PASS do not establish the required green gate. The original step crafter must preserve the failure evidence and make the phase record truthful after the fixture gate is resolved; this reviewer did not edit the execution log.

The implementation commit keeps Marcus as author, carries exactly `Co-Authored-By: Codex <codex@openai.com>`, and has `Step-Id: 07-01`. The execution-log commit has the same required attribution and step identifier. Neither commit contains Claude, Anthropic, or generated-by attribution. The only pre-existing dirty file observed before this review was the user's `AGENTS.md`; it was preserved. This review artifact is the only file written by the reviewer.

## Findings and remediation dispositions

### D1 — Required control-plane integration verification fails before its intended oracle

**Severity:** Blocker for step completion.  
**Location:** `crates/overdrive-control-plane/tests/integration/workload_lifecycle/exit_observer.rs:106-167,206-230`; production owner path at `crates/overdrive-worker/src/mtls_intercept_worker.rs:2369-2390`; production boot composition at `crates/overdrive-control-plane/src/lib.rs:7590-7605`.

**Evidence:** The exact verification command fails 25/26, and the isolated test reproduces the same failure. The complete fixture/dispatch/worker path above explains why: the fixture uses the legacy network-provisioner test seam, produces a network-bearing allocation, and does not start the shared mTLS owner before the worker's required `start_alloc` call. Fail-closed install handling prevents the Running row, so the test never injects an exit or reaches its intended lifecycle-event oracle.

**Disposition:** Return this exact fixture prerequisite to the acceptance designer (DISTILL) for a bounded, semantics-preserving fixture correction. Do not assign a production fix, skip, delete, or weaken the assertion under 07-01. No 07-01 remediation was dispatched. The required gate and GREEN/COMMIT evidence remain unresolved until the fixture is corrected, the exact command passes, and the original step crafter records the truthful DES outcome.

No other proven in-scope production defect, API divergence, test weakening, or test-boundary violation was found. No mutation testing was run during the individual step.

## Iteration 1 verdict

**NEEDS REVISION — BLOCKED ON REQUIRED VERIFICATION.** The 07-01 implementation matches the accepted R10, B-6, PORT-295-C, and DR-07 contract in the reviewed paths, and its assigned test bodies were activated without weakening. The exact required control-plane integration command still fails because an existing exit-observer fixture does not start the shared mTLS owner and therefore cannot establish its declared Running precondition. The review cannot approve this step or authorize the next roadmap step until that fixture gate passes and the execution record reflects the verified result.

## Iteration 2 — remediation re-review

### Scope and remediation

Re-reviewed D1 from iteration 1 against the DISTILL fixture evidence in `.context/distill-fixture-07-01.md` and the adopted commits `08d2c31113ec8571b401267920612d204015e315` and `9dcd4a6acc63e39fc5dcef595f2ee300c432d2b5`. The code change is limited to two test-support calls in the exact named body, `exit_observer_lifecycle_from_reflects_prior_running_state`:

- It awaits `h.state.mtls_worker.start_shared_owner()` before the existing convergence and allocation-start sequence.
- It awaits `h.state.mtls_worker.shutdown_owner()` after the existing `ev.from == Running` and `ev.to == Failed` assertions.

The existing owner, driver, observer, convergence path, exit injection, tick schedule, and both FromState assertions remain unchanged. No production code, public/API shape, test assertion, stimulus, timing, retry rule, or test marker changed. The startup is the omitted fixture prerequisite: `start_alloc` requires the published shared owner for a network-bearing allocation, while production startup already establishes that owner. The explicit shutdown awaits the same owner’s normal completion before the test exits.

### Finding disposition

**D1 — Resolved.** The focused test now establishes its declared Running precondition through the existing production convergence/action path, then sends the original crash through `SimDriver` and observes the original lifecycle event. It reaches both unchanged assertions. The full required selector also passes. This closes the gate with the exact fixture correction owned by DISTILL; it adds no production recovery behavior and does not weaken the acceptance oracle.

No new finding was introduced by the correction. The two support calls are confined to the actual test body named in D1; `server_lifecycle.rs`, shared fixture helpers, production sources, and unrelated tests are unchanged.

### Iteration 2 verification

| Check | Result | Evidence |
|---|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(exit_observer_lifecycle_from_reflects_prior_running_state)' --no-fail-fast` | **PASS, 1/1** | Independently rerun, Nextest run ID `78e0578f-8dd4-4ed9-9881-a03d909c04ea`; the body reaches the unchanged event and FromState assertions. |
| Required integration selector: `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test integration --features integration-tests -E 'test(mtls_install_fail_closed) or test(server_lifecycle) or test(shared_element_cleanup_failure)' --no-fail-fast` | **PASS, 26/26** | Independently rerun, Nextest run ID `dee3ddb1-e3bf-429c-8266-4cb3811c5e65`; 210 tests excluded by the existing selector. |
| `rustfmt --edition 2024 --check crates/overdrive-control-plane/tests/integration/workload_lifecycle/exit_observer.rs` | **PASS** | Independently rerun. |
| `git diff --check` and `git show --check` for both remediation commits | **PASS** | Independently rerun; no whitespace errors. |

The adopted remediation commit preserves Marcus as author, includes exactly `Co-Authored-By: Codex <codex@openai.com>`, and carries `Step-Id: 07-01`. The follow-up log commit has the same attribution and step identifier. The step log appends GREEN PASS and COMMIT PASS after the original entries; it does not claim a replacement RED or erase the first review’s failed gate. The original step crafter owns those phase events.

### Iteration 2 verdict

**APPROVED.** D1 is resolved by the precise DISTILL-owned fixture correction, the named test reaches its unchanged production-observer oracle, and the unchanged required integration command passes 26/26. The original 07-01 implementation remains compliant with the accepted R10, B-6, PORT-295-C, and DR-07 contracts, with no other proven blocker. No mutation testing was run during this individual step.
