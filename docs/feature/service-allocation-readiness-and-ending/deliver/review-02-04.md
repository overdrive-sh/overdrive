# DELIVER Review — Step 02-04

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `02-04` — Committed Stable gates backend eligibility |
| Review iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Implementation commit | `4652c403e8aff6bd1cbe7d146608e48bab97df5d` |
| COMMIT audit commit | `bb18901a4ec31104c881c908ae2d4b82e5318589` |
| Comparison base | `9eddb57434c34e78409be3c55d6529d2df426e41` |
| Authoritative contract | Approved roadmap step 02-04; feature-delta B1, R2-1, R2-2, D12 and G-9; ADR-0265, ADR-0267 and the dated ADR-0096 amendment; `docs/product/architecture/brief.md` |
| Roadmap validation | Approved by the independent roadmap review (`deliver/review-roadmap.md`, iteration 3) |
| Review verdict | **APPROVED** |

## Executive summary

The step uses each allocation's own committed `Stable` row as the input for both Stable re-announcement and startup-probed backend eligibility. `ServiceLifecycle` hydrates `stable_committed` from that allocation row, skips Stable only when that claim exists, and does not persist an emission marker. A rejected Stable-row write therefore leaves the next evaluation able to re-emit the existing `FinalizeFailed { Stable }` action. Backend eligibility requires the committed claim for startup-probed allocations before applying the unchanged readiness threshold; the explicit startup opt-out keeps its existing behavior.

The obsolete `ServiceLifecycleView.stable_announced` field and View-only `has_alloc_mid_startup_window` method are removed. The old-CBOR-key case remains active and proves the removed key is ignored. The prior per-allocation startup veto and its backend-row splice are gone. No new public or cross-crate method, type, enum variant, action, port, persisted field, wire shape, or error variant was added.

## Contract and production-path review

### B1 — committed row controls Stable emission

`ServiceAllocFact::stable_committed` has the approved field shape (`crates/overdrive-reconcilers/src/service_lifecycle.rs:288–292`). `hydrate_service_alloc_facts` derives it from that same allocation's `row.terminal` being `TerminalCondition::Stable` (`:907–941`). In `reconcile`, a terminal allocation is handled first, and a live allocation is skipped for Stable only when its hydrated `stable_committed` claim is true (`:550–562`). Both the opt-out Stable branch and the startup-Pass Stable branch emit the existing `FinalizeFailed { terminal: Some(Stable { .. }) }` action without stamping View state (`:583–635`).

The existing production owner order supports the retry contract. `run_convergence_tick_inner` computes retry eligibility from emitted actions, persists the returned View before dispatch, awaits dispatch, self-re-enqueues after the action path, and returns the dispatch outcome only after that re-enqueue (`crates/overdrive-control-plane/src/reconciler_runtime.rs:1634–1673,1689–1798`). The `FinalizeFailed(Stable)` shim arm writes the still-Running allocation row before calling `on_alloc_stable` and publishing the lifecycle occurrence (`crates/overdrive-control-plane/src/action_shim/mod.rs:2001–2019,2145–2154`). A failed row write returns before the hook; after the failed dispatch the row still lacks Stable, so rehydration permits the same Stable action again. The shim did not need a step change.

The two activated S-43 composed tests exercise this owner path and assert that a failed Stable write leaves the row without Stable, that the next evaluation commits it, and that a control-plane restart still permits the re-announcement (`crates/overdrive-control-plane/src/reconciler_runtime/liveness_gate_composed.rs:136–177`). The runtime reloads its persisted View in `restart_control_plane` before the follow-up evaluation (`liveness_gate_fixture.rs:621–634`).

### D12 and R2-1 — own Stable first, then the readiness rule

The backend predicate is the designed two-stage rule: a startup-probed allocation without its own committed Stable is unhealthy; otherwise the unchanged readiness test checks a current readiness Pass and producer streak against the threshold (`crates/overdrive-reconcilers/src/service_lifecycle.rs:1156–1172`). The predicate does not consult the latest startup row or `terminal_announced`; the opt-out skips the startup gate. Backend projection still includes only Running allocations and derives `healthy` from this predicate (`:1090–1110`).

The pre-authored S-46 property now drives `ServiceLifecycleReconciler::reconcile` and checks the written backend row across opt-out, committed Stable, own and unrelated terminal membership, startup rows, readiness outcomes, and explicit at/below/above-threshold cases (`crates/overdrive-core/tests/acceptance/service_lifecycle_startup_stop.rs:370–439`). The S-47 composed cases exercise the production lifecycle/runtime/shim path and the resolver's `BackendIndex` for the first allocation, a startup-stop successor, a crash successor, readiness-before-Stable, a later startup Fail, a control-plane restart, and opt-out (`crates/overdrive-control-plane/tests/acceptance/startup_gated_backend_eligibility.rs:38–141`). The independent seeded terminal invariant remains active and verifies a dead allocation never becomes eligible again (`crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs:239–342`).

The old `terminal_startup_veto` parameter, `startup_failed_this_tick` state, and veto-before-stop backend splice have been deleted. The normal backend projection follows the startup-stop action and reports an uncommitted startup allocation unhealthy; it does not add a second veto path. The updated test preserves the exact existing `StopAllocation { Stopped { by: StartupProbe } }` action and checks the unhealthy backend projection (`crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs:167–278`). This matches the approved R2-1 subtraction and does not alter D7/D8 ownership or retry behavior.

### R2-2 — remove the View marker and preserve the declared CBOR behavior

`ServiceLifecycleView` no longer contains `stable_announced`, and the View-only `has_alloc_mid_startup_window` method is deleted (`crates/overdrive-reconcilers/src/service_lifecycle.rs:359–429`). The scheduling hook uses `observed`, `terminal_announced`, and hydrated `stable_committed` to decide whether a startup deadline remains (`:703–730`). The three tests that exercised only the deleted predicate were removed; the replacements assert `next_evaluation_at` returns no boundary for empty or completed state and the exact still-waiting allocation deadline for a mixed population. The S-56 terminal grid, Running control, startup-stop scheduling, and composed runtime re-enqueue/drain coverage remain (`crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs:579–886`).

The removed-key persisted-format criterion remains active. Its distinct legacy payload contains `stable_announced`, and the decoded current View is asserted to equal a view with that key absent while retained fields survive (`crates/overdrive-reconcilers/tests/acceptance/startup_last_acted_run_evolution.rs:44–99`). The D8 missing-field and current-field round-trip cases remain separate. The D16 greenfield historical-byte waiver did not remove this explicitly approved R2-2 case.

### Public interface and typed error contract

The API change is limited to the approved in-memory `ServiceAllocFact.stable_committed` field and deletion of the approved View field and predicate. The private helper signatures changed only internally. No caller-visible error representation changed.

| Semantic failure | Approved and implemented representation | Evidence |
|---|---|---|
| Allocation/probe observation read fails during ServiceLifecycle hydration | Existing `HydrateError::ObservationRead(String)`, forwarded by the runtime as `ConvergenceError::Hydrate(HydrateError)` | `service_lifecycle.rs:907–918`; `reconciler_runtime.rs:1877–1903` |
| The Stable allocation-row write is rejected | Existing `ShimError::Observation(ObservationStoreError)`, forwarded as `ConvergenceError::Shim(ShimError)` | `action_shim/mod.rs:2145–2154,3571–3589`; `reconciler_runtime.rs:1725–1754` |

For the injected S-43 write failure, the concrete source is `ObservationStoreError::Io`; the shim's `?` preserves it in `ShimError::Observation`, and the runtime preserves that typed shim error in `ConvergenceError::Shim`. The existing retry is the persisted-row re-evaluation described above. This step adds no semantic error distinction and no string-only substitute for one; no `SPECIFICATION_AMBIGUITY` is present.

## Acceptance coverage and test integrity

| Step outcome | Evidence and disposition |
|---|---|
| Startup-probed allocation is ineligible before its own Stable, then readiness narrows eligibility | S-46 covers the predicate including threshold boundaries; S-47 observes the written backend row and resolver outcome. Post-Stable readiness fixtures now carry the accepted committed-row precondition. Thresholds, Pass/Fail outcomes, backend-count assertions, and readiness recovery assertions remain unchanged. |
| Lost Stable write is retried and remains recoverable after control-plane restart | Both S-43 composed tests run through the production reconciler runtime and action shim. The no-Stable row and successful later commit are asserted. |
| Stable View marker is removed and old CBOR key is ignored | The old key is serialized in a dedicated legacy fixture and asserted absent after decode; D8 additive-field and round-trip checks remain. |
| Startup-stop, terminal, liveness, and startup opt-out behavior remain intact | Existing typed stop, terminal, liveness threshold, readiness flap/recovery, S-56 terminal, and opt-out controls remain active in the focused selection. |

The DISTILL fixture revision is independently approved through three iterations in `distill/review-committed-stable-fixture-remediation.md`. Its deletions target tests of the removed View-only helper; replacement tests drive the production scheduling hook and preserve exact deadlines and terminal outcomes. Its fixture edits model post-Stable facts only where the test sequence already required a committed Stable. The runtime source-local readiness test obtains the typed Stable action from the production reconciler, writes it through `ObservationStore`, and rehydrates before asserting readiness recovery. No behavior assertion was weakened, no new `#[ignore]` was added, and no production behavior was encoded in a test double.

### Contract Shape Compliance

All newly activated or transitioned Rust bodies in the step diff carry per-test Contract Shape declarations. The source-local backend-policy property carries the exact required `/// CONTRACT_SHAPE: pure-function.` declaration. Other modified behavior tests retain their existing per-test declarations. No module-level tag is used as a substitute for a transitioned test's declaration.

### TDD and quantitative checks

The step has three observable outcome groups: committed-row Stable retry, per-allocation backend eligibility with readiness and opt-out behavior, and removal/decoding of the obsolete View field. The source-local predicate property is one unit/property test through the public reconciler boundary; the remaining 91-test focused selection consists of the pre-authored acceptance/conformance bodies and existing controls, not 91 newly added unit tests. No test-budget excess is identified.

`execution-log.json` records the canonical sequence: `RED / FAIL` at `2026-11-10T02:55:11Z`, `GREEN / PASS` at `04:30:01Z` and `04:31:04Z`, then `COMMIT / PASS` at `04:34:57Z`. The RED was meaningful: the removed-key decoder retained the obsolete field, the old backend policy allowed an uncommitted startup allocation to be healthy, and both lost-write cases failed to commit Stable. The multiple GREEN entries record the final focused selections after the independently reviewed fixture remediation; they do not conceal a failed final selection.

The changed production path is limited to B1/R2-1/R2-2/D12. The added property exercises `reconcile` and its emitted backend row; composed cases enter through the production runtime and shim. Sim adapters are at driven ports. Rust tests do not spawn the built product binary or act as expectation runners. I found no testing theater, implementation-only assertion as the primary oracle, new test double boundary, or new L1/L2 RPP issue in the reviewed diff.

## Verification and limits

| Check | Result |
|---|---|
| Focused step acceptance/conformance selection | **91/91 passed**, 2,283 skipped; retained log `.context/step-02-04-green-focused-final.log`, Metal run `515a1a85b16e378017c2bc17` |
| Final readiness and source-local hydration selection | **8/8 passed**, 1,331 skipped; `.context/step-02-04-green-readiness-final.log`, Metal run `793d153228d63485bf6724e8` |
| Seeded terminal invariant | **1/1 passed**; `.context/step-02-04-green-sim-control.log`, Metal run `58e2a085d9aa0480a2c83d89` |
| Scoped Clippy and formatting | Crafter logs record strict Clippy and final formatting checks passing (`.context/step-02-04-green-clippy.log`, `step-02-04-green-cp-clippy-final.log`, `step-02-04-green-fmt-final.log`) |
| Diff hygiene | My read-only `git diff --check 4652c403e8aff6bd1cbe7d146608e48bab97df5d^ bb18901a4ec31104c881c908ae2d4b82e5318589` passed |
| Broad three-crate selection | The earlier 2,288-test run recorded 2,207 passed, 81 failed and 86 skipped. It preceded the final D12 fixture correction; the four affected readiness cases were individually reproduced before correction and are included in the later 8/8 pass. The remaining broad-run failures were not established as baseline or attributed here, including the recorded host FD-budget failure. |
| Mutation testing | Not run; the repository requires one final DELIVER-wave mutation gate after all step reviews. |

The implementation commit retains Marcus as Git author and contains exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 02-04`, and `Task-Id: service-allocation-readiness-and-ending`; it contains no Claude or Anthropic attribution. The audit commit changes only `execution-log.json` and has the same required trailers. The workspace was clean before this review artifact was created. No tests, production code, DES events, or commits were created by this reviewer.

## Findings and remediation disposition

No findings. B1's row-based retry, D12's own-committed-Stable gate and unchanged readiness rule, R2-1's removal of the redundant veto, and R2-2's View/CBOR contract match the approved design. The production entry path, ownership, dispatch ordering, and typed error mappings are consistent with the contract. Test oracle changes were bounded DISTILL revisions independently approved after reproduction; their retained outcomes are covered at the declared production boundaries. No DESIGN gap, `SPECIFICATION_AMBIGUITY`, unproven hypothesis promoted to a finding, or remediation iteration remains.

## Final verdict

**APPROVED** — Step 02-04 implements the approved committed-Stable and backend-eligibility contract with the exact interface changes, preserves readiness, opt-out and terminal behavior, activates its assigned evidence without weakening outcomes, and records the COMMIT audit event. The DELIVER orchestrator may proceed after its mechanical checks confirm this on-disk review artifact.
