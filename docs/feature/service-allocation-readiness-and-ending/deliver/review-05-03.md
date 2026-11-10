# DELIVER review — step 05-03

## Metadata

| Item | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `05-03` — Guest-listener startup probe task writes rows |
| Iteration | 1 |
| Implementation commit | `c920d2e0f58d6c4a51aac0eda701c41f9e216c77` |
| COMMIT audit | `1bd822fc40d2f3806c8678d3faf183e080dad0b8` |
| Prior step | `05-02` approved at iteration 2 |
| Review scope | GuestListeners task, P1 startup threshold, DG-11 summaries, production board-sharing proof, affected fixtures and conformance-model/evidence changes |
| Working tree before artifact | Clean |

## Verdict

**APPROVED — iteration 1.** The implementation matches ADR-0269 and the approved 05-03 roadmap contract. The guest-listener task uses the shared reported-listener port and declared listener set, writes ordinary startup rows through the existing per-probe run state, preserves the designed failure retry cadence and counter semantics, and leaves the existing downstream lifecycle gates intact. The mechanic-specific threshold and both declaration-order summaries match P1 and DG-11. No production API or error variant was added.

No findings require remediation. One unproven cancellation hypothesis is recorded below without promoting it to a defect or design requirement.

## Contract and production-path review

### GuestListeners task and row semantics

`ProbeRunner::start_alloc` routes a `GuestListeners` descriptor to the event-driven task and passes that task `AllocationSpec.declared_listeners` (`crates/overdrive-worker/src/probe_runner/mod.rs:335-357`). The private task is registered under the allocation's Startup role and observes the existing `GuestListenerReports` port (`:465-489`). It writes `Pass` immediately when every declared `(protocol, port)` is reported, and returns after that write succeeds. While the set remains incomplete, it waits until `interval_seconds × max_attempts`, then writes the first `Fail` and schedules subsequent failures one interval apart (`:762-853`). The failure reason filters the declared set and renders only missing listeners in declaration order (`:855-872`).

Rows are built by the shared `record_probe_status` helper used by the existing dial task (`:717-759`). `run_seq` and `same_status_runs` are computed from the task's existing `ProbeRunState`; `record_written` runs only after `ObservationStore::write_probe_result` succeeds. Thus a failed write advances neither counter. A failed `Pass` is retried after the configured interval until it lands; a failed `Fail` leaves the state unchanged and the next interval attempts another `Fail`. No row field or counter representation changed.

### Startup stop threshold

`spec_facts_for_service` keeps `max_attempts` for `Tcp` and `Http` and derives `startup_attempts_required = 1` for `GuestListeners` (`crates/overdrive-reconcilers/src/service_lifecycle.rs:785-810`). The in-memory `ServiceAllocFact.max_attempts` rename to `startup_attempts_required` is the exact P1 contract; `startup_deadline` remains derived from the descriptor. `startup_probe_failed_action` still requires the latest Fail streak, the elapsed deadline, and no Pass (`:1192-1225`). The new threshold therefore lets the first deadline Fail stop a GuestListeners allocation while preserving the TCP/HTTP rule and every downstream action.

### Mechanic summaries and interface shape

Both formatters render `guest listeners tcp:8080 udp:5353` from the declared listeners, preserving declaration order (`crates/overdrive-reconcilers/src/service_lifecycle.rs:765-793`; `crates/overdrive-cli/src/render.rs:1399-1428`). The CLI receives `out.snapshot.listeners` through the existing `probes_section` call (`render.rs:289-300, 1471-1493`). No listener field was added to `ProbeMechanic::GuestListeners`, no wire field was added, and no public runner method, port, trait method, or error variant was invented.

### Typed-error mapping

The only new failure handling in this step is a probe-result observation write failure. `record_probe_status` retains the source in the existing `ProbeRunnerError::ObservationWriteFailed { alloc_id, probe_idx, source: ObservationStoreError }` representation and emits the existing `write_failed` diagnostic before the task applies the approved retry behavior (`crates/overdrive-worker/src/probe_runner/mod.rs:749-756`). The task does not create a string-only semantic error or return a new error surface. D14-E2 ingress errors are not introduced or modified by this step; their typed admission work remains assigned to the later roadmap step. No unresolved typed-error contract gap affects 05-03.

### Task ownership, cancellation, and stopping

The production start owner is `VmDriver::on_alloc_running`, which calls `ProbeRunner::start_alloc`; the terminal hook calls `stop_alloc` before closing the listener-board entry (`crates/overdrive-worker/src/vm_driver.rs:2102-2113`). In the real action-shim terminal path, cleanup is followed by `driver.on_alloc_terminal` before the lifecycle terminal-row write (`crates/overdrive-control-plane/src/action_shim/mod.rs:2194-2229`). The task checks its cancellation token at each loop entry and gives cancellation priority over the wait on either a listener change or deadline (`probe_runner/mod.rs:780-783,845-851`). The accepted `no_row_is_written_after_stop_alloc` test covers cancellation during that wait and asserts that a separate uncancelled control allocation still writes its Pass row (`crates/overdrive-worker/tests/acceptance/guest_listener_probe_rows.rs:263-293`).

**Unproven hypothesis, not a finding:** `stop_alloc` cooperatively cancels the supervisor and does not join task handles; if an observation write has already begun, this method does not cancel or drain that in-flight write (`probe_runner/supervisor.rs:33-39,60-68,184-193`; `probe_runner/mod.rs:363-375,805-839`). The assigned test demonstrates the required cancellation-from-wait behavior, but no bounded regression through the production owner path reproduced a row write started after `stop_alloc`. Under the repository reachability and reproduction gate, this timing possibility remains unproven and does not require remediation or a design change in this review.

## DISTILL support and model changes

The shared-board wiring scenario moved to the source-local `integration-tests` module so it can use the existing crate-private `SimGuestFlowIo` port while still calling `run_server`. The test submits the Service through the production HTTP handler, obtains the owner's leased CID/token, waits for the real `EXEC` beacon reply, and then sends `LISTEN` lines through `SimBeaconIo`. It never touches the board, calls `on_alloc_running`, or installs a new production effect. The undeclared and partial report complements and the exact single-Pass-row oracle remain (`crates/overdrive-control-plane/src/guest_listener_board_wiring_acceptance.rs:317-356`).

The restart-authority model correction changes the helper used by whole-workload ServiceLifecycle actions so it also projects Stable for every eligible Running allocation, matching `ServiceLifecycleReconciler::reconcile`'s allocation scan. The change is limited to the model's `ann` and `stable` projections; it does not remove or weaken row, claim, stop-stage, or author assertions (`specs/quint/service-allocation-readiness-and-ending/restart_authority.qnt:224-313`). The targeted seed `0xd18783b` exposed the production/model scope mismatch and passed after the correction. The scoped `inject_alloc_lifecycle_write_failure` addition is confined to `SimObservationStore` test scripting; it keeps the restart-authority `row_write` fault on the allocation-lifecycle write and leaves production ports and dispatch unchanged (`crates/overdrive-sim/src/adapters/observation_store.rs:184-189,717-731,830-853`).

The D9 runner controls now call the existing Stable hook sequence before expecting Liveness. Their original dial, response, timeout, run-sequence, streak, and not-sampled assertions remain; the eight-descriptor case now checks the two reachable phases and still requires all eight targets and rows (`crates/overdrive-worker/src/probe_runner/run_numbering_tests.rs:216-250,544-630`). The independent DISTILL remediation review approved these bounded test-support/model corrections.

## Acceptance coverage and test integrity

The step covers 11 distinct observable behaviors: full-set Pass; no pre-deadline row and recurring deadline Fail; missing-listener reason; Pass-write retry and counter preservation; Fail-write counter preservation; cancellation before a later write; undeclared/duplicate reports; producer run numbering; mechanic-specific stop threshold; declaration-order summaries at both render boundaries; and the shared production board path.

The 12 step acceptance bodies (8 guest-listener row cases, 2 stop-gate cases, and 2 summary cases) are within the `2 × 11 = 22` unit-test budget. The source-local `run_server` wiring test and Quint conformance scenarios are separate production-composition and model-conformance evidence, not additional unit tests for that budget.

All newly active tests carry per-test `CONTRACT_SHAPE` declarations. The source-local pure-function properties use the exact `/// CONTRACT_SHAPE: pure-function.` declaration. Test names do not match the banned `test_*` name pattern. The feature's prior 05-02 review records that no DISCUSS Elevator Pitch artifact exists for this feature; no outcome-anchor declaration is available to apply. The implementation commit activates the already authored 05-03 scenarios by removing their pending `#[ignore]` markers. The other acceptance edits are mechanical `max_attempts` fixture-field renames. The D9 controls add reachable Stable-phase setup and assertions; no existing behavioral assertion was removed or weakened. The moved wiring test adds the guest control-session setup and waits for `EXEC` before issuing `LISTEN`.

## Verification evidence

I reviewed the committed diff and evidence without rerunning the test suite. Retained logs and the crafter's final run record show:

| Evidence | Result | Source / limit |
|---|---:|---|
| Guest-listener row, P1 threshold, and summary focused tests | 18 focused tests passed | Crafter final-run record; not independently rerun |
| Production `run_server` board-sharing test | 1/1 passed | `.context/step-05-03-wiring-source-local-control-session.log` |
| D9 runner controls | 5/5 passed | `.context/step-05-03-d9-fixtures-after2.log` |
| Seeded restart-authority conformance | 1/1 passed for seed `0xd18783b` | `.context/step-05-03-restart-seed-after2.log` |
| Restart-authority conformance selection | 22/22 passed | `.context/step-05-03-restart-conformance-after.log` |
| Quint formal evidence | 84/84 expected outcomes matched; 13 current input hashes recorded | `.context/step-05-03-quint-formal-after.log`; `verify-evidence` reported passed by the crafter and independently reviewed in the DISTILL artifact |
| Roadmap, Metal placement, and formal evidence JSON | Valid JSON | Independently parsed |
| Formal input fingerprint | Current `restart_authority.qnt` SHA-256 matches its evidence entry | Independently checked; the DISTILL review checked all 13 current input hashes |
| Strict Clippy, all-target checks, changed-file formatting | Passed | Crafter final-run record; not independently rerun |
| Commit whitespace / working tree | `git diff --check` clean; worktree clean before this review artifact | Independently checked |

`execution-log.json` records the 05-03 RED failure, the documented GREEN failure during fixture/model remediation, the final GREEN pass, and COMMIT pass in that order. The COMMIT audit is the separate `1bd822f` commit. Both commits retain Marcus as Git author and contain exactly the required Codex co-author trailer. No per-step mutation testing was run; mutation testing remains the final DELIVER-wave gate.

## Review findings and dispositions

| ID | Finding | Severity | Disposition |
|---|---|---|---|
| — | No accepted finding | — | No remediation required |

The cancellation timing possibility above remains an unproven hypothesis only. It is not an accepted finding, remediation request, or DESIGN referral.

## Review dimensions

| Dimension | Result |
|---|---|
| Exact API and wire shape | **Pass** — implementation matches D14, P1, DG-11, and the roadmap; no new public surface |
| Typed-error contract | **Pass** — existing typed observation-write error is retained; no new semantic error path is underspecified |
| Production task lifecycle | **Pass** — real `start_alloc` path, board sharing, and cancellation-from-wait behavior are exercised |
| Acceptance coverage / external validity | **Pass** — row outcomes use the real ProbeRunner entry point; board-sharing evidence uses `run_server` and the real production composition |
| Test integrity | **Pass** — pending markers were activated; no assertions were weakened or deleted |
| Contract Shape | **Pass** — required declarations present, including exact pure-function declarations |
| Test budget | **Pass** — 12 step acceptance bodies against a 22-test budget |
| Formal model and evidence | **Pass** — corrected whole-workload projection, seeded conformance, and evidence refreshed without invariant weakening |
| Error handling / retries | **Pass** — row writes preserve counters on failure and follow the accepted retry cadence |
| Mutation gate | **Not run** — deferred to the final DELIVER-wave gate |
| RPP | L1 review found no step-specific production smell requiring action |

## Iteration 1 disposition

**APPROVED.** The approved implementation satisfies the 05-03 acceptance criteria and exact interface contract. No remediation is required, and the next roadmap step may begin.
