# DELIVER Review — Step 02-03

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `02-03` — Failed startup stop retried once per probe row |
| Review iteration | 1 |
| Reviewer | Codex, fresh isolated DELIVER reviewer |
| Implementation commit | `31d37d41dfe0e94c9220414151b8a72056c663c1` |
| COMMIT audit commit | `92e1f13e05c8eb97093b5ed5f1506560a39e57ef` |
| Comparison base | `461438b7eca83daa6e15b84e83ab5c9016f16f17` |
| Authoritative contract | Feature delta D8, D8 item 3, B2 and D8 interface changes; ADR-0260, ADR-0266; `brief.md`; approved roadmap step 02-03 |
| Roadmap validation | Approved by the independent roadmap reviewer on 2026-11-09 |
| Review verdict | **APPROVED** |

## Executive summary

`ServiceLifecycleReconciler` now evaluates a `Running` allocation in `terminal_announced` against the startup-stop rule, and emits only when the latest written startup row has a `run_seq` greater than the previously acted run. It records that run when it emits the existing `Action::StopAllocation`. A later startup `Pass` reaches the existing `Stable` branch, ends the retry and removes the allocation from `terminal_announced`.

The persisted field has the exact approved shape, `BTreeMap<(AllocationId, ProbeIdx), ProbeRunSeq>` with `#[serde(default)]`. It was an existing scaffold; this step activates it without adding another public method, type, enum variant, action, port, wire field or error variant. The B2 hook ordering remains the accepted one: `Driver::on_alloc_terminal` fires after the requested terminal row is durable. The activated composed tests exercise the production reconciler runtime, action shim and `ProbeRunner` through `Sim*` driven ports.

## Contract and production-path review

### D8 — one startup stop emission per new written row

The changed loop retains the D5 terminal-row guard, but lets an announced allocation whose current state is `Running` continue through startup evaluation (`crates/overdrive-reconcilers/src/service_lifecycle.rs:568-590`). It reads the latest startup row at `(ProbeRole::Startup, ProbeIdx::new(0))`, checks `row.run_seq > startup_last_acted_run[(alloc, 0)]` (or no prior entry), and calls the existing startup-gate helper only for a newer row (`:661-681`). The helper still applies the accepted attempts, deadline, no-pass and non-terminal gates and returns the existing typed `StopAllocation { terminal: Some(Stopped { by: StartupProbe }) }` (`:1222-1258`). A missing startup row cannot emit a stop.

On emission, the view records the row's `run_seq` and keeps the allocation in `terminal_announced` (`:679-685`). The existing `Stable` branch precedes this gate; when a later `Pass` arrives, that branch announces `Stable`, removes the allocation from `terminal_announced`, and prevents another startup stop (`:635-658`). Liveness continues to skip `terminal_announced` members (`:1040-1052`), so the removal preserves the designed path for a late `Pass` to leave the set and reach liveness supervision.

The real owner ordering supports the emitted marker. `run_convergence_tick_inner` persists `next_view` through the runtime-owned `ViewStore` before action dispatch (`crates/overdrive-control-plane/src/reconciler_runtime.rs:1658-1673`), then awaits the action shim and re-enqueues after recoverable dispatch errors (`:1689-1759`, `:1781-1798`). `ServiceLifecycleReconciler` declares both `AllocStatus` and `ProbeResult` interests (`crates/overdrive-reconcilers/src/service_lifecycle.rs:541-548`), so each subsequent written probe row can wake the owner. The composed acceptance test also proves a failed stop is not repeated on an unchanged row and is attempted again after a new row (`crates/overdrive-control-plane/tests/acceptance/startup_stop_authority.rs:223-247`).

### B2 — preserve the probe supervisor until the stop row is durable

The existing `StopAllocation` arm treats `DriverError::NotFound` as an already-stopped outcome, propagates other driver and cleanup errors, and reaches the terminal-row write only after cleanup (`crates/overdrive-control-plane/src/action_shim/mod.rs:3252-3272`). A failed terminal-row write returns before `on_alloc_terminal`; the hook is called only when `terminal_row_durable` is true (`:3359-3375`). The S-30 row-write acceptance case observes that the **ProbeRunner probe supervisor** remains registered after a failed write, then falls to zero only after a later row triggers a successful retry (`startup_stop_authority.rs:296-325`).

The shim also calls `Driver::release_supervision` on the row-write error path (`action_shim/mod.rs:3366-3371`). This is distinct from `Driver::on_alloc_terminal`, which ends the ProbeRunner's probe supervision. The B2 guarantee concerns the latter, as the accepted ADR and the test's `supervised_allocations` projection specify; the existing launch/beacon supervision release does not contradict it.

### Designed failure conditions and typed representations

The D8/G-5 failure projection retains the existing `ShimError` owner-facing envelope. No new error taxonomy is required by this step. The concrete existing mappings preserve the typed cause through that envelope:

| Designed failure condition | Existing typed representation | Current production mapping |
|---|---|---|
| Non-`NotFound` driver stop failure | `DriverError` | `ShimError::Driver(DriverError)` (`action_shim/mod.rs:3252-3257`, `:3571-3582`); `NotFound` is treated as stopped |
| mTLS cleanup failure | `MtlsInterceptStopError` | `ShimError::MtlsStop(MtlsInterceptStopError)` (`:3263-3264`, `:3583-3586`) |
| Guest-network cleanup failure | `GuestNetworkError` | `ShimError::GuestNetwork(GuestNetworkError)` (`:3263-3272`, `:3574-3578`) |
| Terminal-row read/write failure | `ObservationStoreError` | `ShimError::Observation(ObservationStoreError)` through the existing `From` mapping (`:3289-3300`, `:3359-3371`, `:3587-3589`) |

The acceptance tests assert the retry and externally observable row/supervisor outcomes at the composed boundary; the implementation preserves the typed cause when propagating each dispatch error. There is no unresolved typed-error representation gap or `SPECIFICATION_AMBIGUITY` for this step.

## Test integrity and coverage

The five distinct scoped outcomes are: one stop per newer startup row with same-row deduplication; the successful stop and terminal hook preceding the successor; a late `Pass` ending the retry and reaching `Stable`; a failed terminal-row write preserving the ProbeRunner supervisor until a later successful retry; and persistence of the acted run across a control-plane runtime reload. The unit-test budget is ten (`5 behaviors × 2`); three unit-level/property and CBOR evolution test methods cover those contracts, within budget. The three composed acceptance cases and four named Quint conformance runs are separate behavior/composition evidence, not additional unit tests.

| Contract behavior | Evidence |
|---|---|
| New row retries, unchanged row does not, landed stop precedes successor | `startup_stop_authority::failed_startup_stop_is_retried_once_per_new_startup_row`; pure property `service_lifecycle_startup_stop::startup_stop_is_emitted_once_per_new_startup_row` |
| Late `Pass` stops retry and reaches `Stable` | `startup_stop_authority::late_startup_pass_after_a_failed_stop_ends_the_retry_and_reaches_stable`; `restart_authority_late_pass_after_failed_stop_reaches_stable` |
| Failed row write retains probe supervision and retries | `startup_stop_authority::failed_stop_row_write_keeps_the_supervisor_and_the_next_probe_row_retries`; `restart_authority_stop_row_write_failure_is_retried` |
| Control-plane reload and CBOR evolution preserve the acted run | `restart_authority_control_plane_restart_keeps_acted_run`; `startup_last_acted_run_evolution::{blob_without_startup_last_acted_run_decodes_with_an_empty_map, startup_last_acted_run_survives_persistence}` |

The control-plane conformance driver's `Idle` action rebuilds `ReconcilerRuntime`, registers both lifecycle reconcilers and reloads views from the redb-backed `ViewStore` (`crates/overdrive-control-plane/src/reconciler_runtime/quint_restart_authority.rs:351-368`). The CBOR tests independently cover the old blob's empty-map default and current-field round-trip (`crates/overdrive-reconcilers/tests/acceptance/startup_last_acted_run_evolution.rs:45-68`, `:95-109`). The composed Rust tests enter through the production convergence tick and do not spawn the built product binary or run expectation evidence.

The S-30 ordering assertion correction is separately recorded and independently approved in `distill/review-startup-stop-order-oracle-remediation.md`. The old assertion failed through the real composed path at seed `0`; its retained driver trace showed three startup-stop calls before the terminal hook and one exact-predecessor cleanup stop after the successor start. The corrected oracle asserts those three startup stops, terminal-hook-before-successor order, and the later cleanup separately (`startup_stop_authority.rs:248-269`). It strengthens the observable call-order check; it does not weaken a stop-row, retry, hook or successor assertion. No other test body was weakened, deleted or newly skipped. The activated tests retain their `CONTRACT_SHAPE` declarations.

## TDD and quality-gate review

`execution-log.json` records step 02-03 `RED / FAIL`, an initial `GREEN / FAIL`, a later `GREEN / PASS`, then `COMMIT / PASS` in timestamp order. The separate audit commit adds the COMMIT event. The original RED acceptance failure is semantically relevant: the same-row retry case failed because no retry occurred on a new probe row; the conformance RED run also found the expected model/production divergence. The later S-30 oracle failure was reproduced independently and corrected by DISTILL before the final GREEN pass.

| Gate | Result | Evidence |
|---|---|---|
| RED → GREEN → COMMIT sequence | PASS | `deliver/execution-log.json`; final RED failure, GREEN pass and COMMIT pass recorded in order |
| Roadmap criteria and named tests | PASS | Three composed cases, one pure property, four named D8 conformance runs and existing CBOR evolution checks are present and active where assigned |
| Test modification | PASS | No weakened or removed assertions; the only S-30 assertion change is the independently approved oracle correction described above |
| Contract Shape | PASS | All transitioned bodies retain their per-test declarations |
| Public API and persisted shape | PASS | Exact approved `startup_last_acted_run` field shape; no additional public action, method, type, variant, port, wire or error surface |
| Typed error contract | PASS | Each designed dispatch failure maps to the existing typed `ShimError` variant and preserves its cause |
| Focused Metal nextest | PASS, independently rerun | Four pure/composed acceptance bodies: 4/4 passed; run `7e3004ef64cf22ddf982a97d` |
| Named Quint conformance | PASS, independently rerun | Four D8 runs: 4/4 passed; run `d92a509c36f8d1589c7d34c1` |
| CBOR evolution | PASS, independently rerun | Two additive-default/round-trip checks: 2/2 passed; run `590a8a04a2b8421e62f7c7da` |
| Clippy | PASS, independently rerun | Metal Clippy across `overdrive-core`, `overdrive-control-plane`, and `overdrive-reconcilers`, all targets, integration-tests, `-D warnings`; run `c311d739b6f7867bd071e5da` |
| Formatting | PASS, independently rerun | Metal `cargo fmt -p overdrive-core -p overdrive-control-plane -p overdrive-reconcilers -- --check`; run `d1d2411666d5ca73ac4c2088` |
| Diff hygiene | PASS | `git diff --check 31d37d41dfe0e94c9220414151b8a72056c663c1^ 92e1f13e05c8eb97093b5ed5f1506560a39e57ef` |
| Per-step mutation test | Not run by rule | Mutation testing is a single final DELIVER-wave gate, after every step review |

The implementation and audit commits retain Marcus as Git author and each contains exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 02-03` and `Task-Id: service-allocation-readiness-and-ending`; neither contains Claude or Anthropic attribution. The implementation commit's seven-file scope is the lifecycle logic, assigned acceptance/conformance activation, execution log and independently reviewed DISTILL oracle artifacts. The audit commit modifies only `execution-log.json`. The workspace was clean before this review; no implementation, test, model, DES or commit changes were made by this reviewer.

## Findings and remediation disposition

No findings. D8's per-new-row retry, same-row deduplication, late-`Pass` transition and B2's row-durable ProbeRunner hook match the accepted contracts. The design's API and typed error contracts are complete and unchanged. The DISTILL correction is independently approved and preserves the startup-stop and exact-predecessor cleanup evidence. No DESIGN gap, `SPECIFICATION_AMBIGUITY`, unproven defect promoted to a finding, or remediation iteration is required.

## Final verdict

**APPROVED** — Step 02-03 implements the approved D8 retry rule and B2 startup-stop ordering with the exact approved view and action shapes, activates its assigned tests without weakening their assertions, and records the COMMIT audit event. The DELIVER orchestrator may proceed after its mechanical checks confirm this on-disk review artifact.
