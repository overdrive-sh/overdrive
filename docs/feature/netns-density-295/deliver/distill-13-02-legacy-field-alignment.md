# STEP 13-02 DISTILL legacy-field alignment

## Status

**REMEDIATION SOURCE COMPLETE — ready for fresh independent DISTILL review
iteration 2 before final test replay.**

This alignment consumes the independently approved PRCI-1 contract in
[`design-13-02-probe-input-shape.md`](design-13-02-probe-input-shape.md) and
[`review-design-13-02-probe-input-shape.md`](review-design-13-02-probe-input-shape.md).
The only probe-row carrier used by the fixtures is:

```rust
#[doc(hidden)]
pub probe_rows: BTreeMap<(ProbeRole, ProbeIdx), ProbeResultRow>,
```

on each `ServiceAllocFact`. `ServiceLifecycleState` retains exactly its three
approved fields. No test adds a method, wrapper, State field, adapter seam,
scenario, lane, model, or production mechanism.

## Compiler evidence

The initial native compile identified the legacy-field fallout:

```text
cargo xtask metal run -- cargo check -p overdrive-reconcilers --tests
```

It reported fourteen diagnostics: six reads of deleted
`ServiceLifecycleView` fields in `service_kind_vm_workloads.rs`, three
deleted-field uses plus one outdated readiness-helper call in the source-local
tests, and four calls to the deleted `update_startup_attempts` helper.

After PRCI-1 implementation and the bounded fixture alignment:

- `cargo xtask metal run -- cargo check -p overdrive-reconcilers --tests`
  passed;
- `cargo xtask metal run -- cargo check --workspace --all-targets --features
  integration-tests` passed.

The original crafter then replayed the reviewer-selected existing bodies on
Metal run `d5669ff988e0ca65fdefd5a`, nextest run
`4bc0843c-cfda-426e-a1b3-e51f15dc7d5f`: five passed and the existing
ServiceLifecycle hydration characterization golden failed because its expected
Debug value omitted the additive fact-local `probe_rows` map and its seeded
Startup row. The remediation updates only that text fixture.

The activated probe-count suite's latest reproduction is distinct from its
earlier `flow_ports == None` failure. Metal run
`e3dedf287783b22c6b736ff7`, nextest run
`f2432452-cd19-4872-a95a-baa6ad6c471d`, reached the retained fresh-allocation
oracle at `probe_run_count.rs:350`: expected zero driver stops after the fresh
allocation's first Fail row (`run_seq = 1`, `same_status_runs = 1`), observed
one. Source tracing proves that stop was fixture boot cleanup, not a premature
liveness decision: the fixture supplied an unstarted `MtlsInterceptWorker`;
`MtlsInterceptWorker::start_alloc` refuses a transported allocation with
`SharedOwner(NotStarted)`, and the StartAllocation action shim's accepted
fail-closed path calls `driver.stop` before recording the failed start. The
fixture now follows the existing same-owner composition pattern: start the
shared mTLS owner, converge the same shared guest-network owner passed to
`AppState`, open its EXEC gate, and then construct `AppState`. The original
zero-stop assertion and fresh-run invariant are unchanged.

No post-remediation `cargo nextest` replay has run. The same-step DISTILL
reviewer receives this source package first.

## Before-to-after coverage mapping

| Path | Legacy input or assertion | Approved replacement | Retained observable contract |
| --- | --- | --- | --- |
| `crates/overdrive-reconcilers/src/service_lifecycle.rs` (`cfg(test)` only) | Seeded `readiness_consecutive_successes`; directly tested `update_startup_attempts`. | Inserts an exact readiness `ProbeResultRow` into `ServiceAllocFact::probe_rows`; compares its `same_status_runs` with the live threshold. Deletes only the private-helper property whose complete subject was the removed startup counter/timestamp mechanism. | Startup veto dominates readiness; Pass/Fail/absence, threshold, no-readiness default, and unrelated View inputs remain asserted. |
| `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs` | Repeated reconcile calls reconstructed readiness/liveness streaks; assertions inspected the removed View maps. | Fixture records Startup, Readiness, and Liveness rows with exact role/index, `run_seq`, and `same_status_runs`; liveness emission asserts `liveness_last_acted_run`. | Stable, StartupProbeFailed, backend eligibility withdrawal/recovery, liveness StopAllocation, and sole restart authority remain asserted. |
| `crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs` | Category C and liveness/readiness properties seeded deleted counters; Category F observed the removed startup counter directly; the missing-row terminal case depended on a View counter with no row. | Retained branch properties consume fact-local rows. Category F and the missing-row terminal case are deleted under the explicit deletion criterion. Same-row liveness re-read is inert; a newer crossed row retries. | Startup threshold and elapsed boundaries, Pass prevention, terminal dedup, readiness threshold truth table, Running guard, act-once, and D-1 retry remain asserted. |
| `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs` | Startup failure was reached by seeding `startup_attempts_per_alloc` and `startup_last_fail_seen_at`. | Startup Fail row carries streak/run 3. | Existing StartupProbeFailed terminal, exact reason, attempt count, and deadline behavior remain asserted. |
| `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_readiness.rs` | Readiness thresholds were reconstructed from a View counter. | Generated producer rows carry the generated streak. | Multi-backend Pass/Fail eligibility, live success threshold, no-readiness default, and no-restart assertions remain. |
| `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_liveness.rs` | Failure counters advanced per reconcile and reset on Pass/emit. | Each run supplies its producer streak. A same-row re-read emits nothing; the next newer crossed row retries if still Running. | Threshold StopAllocation, Fail/Fail/Pass safety, act-once, D-1 retry, non-Running suppression, and fresh-allocation first streak remain. |
| `crates/overdrive-control-plane/tests/acceptance/single_restart_authority_liveness_trajectory.rs` | Threshold-one fact carried status only. | Fact carries the matching first-run liveness row. | Full ServiceLifecycle stop → WorkloadLifecycle restart-budget exhaustion trajectory remains. |
| `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs` | Startup/readiness facts carried status only. | Fact-local Startup and Readiness rows mirror those status projections. | Terminal allocation never returns to backend eligibility; a distinct successor can become healthy. |
| `crates/overdrive-core/tests/acceptance/vm_reclamation_plan_purity.rs` | Generated startup attempts lived in the removed View map. | Zero attempts means no row; positive attempts produce a Fail row whose streak equals the generated input. | A reclaimed allocation never receives a fabricated StartupProbeFailed terminal. |
| `crates/overdrive-control-plane/tests/integration/reconciler_runtime_view_store.rs` | Eq-diff test changed `startup_attempts_per_alloc`. | Changes `liveness_last_acted_run` from run 3 to run 4. | Equal View skips write-through; changed View writes exactly once and updates memory. |
| Existing neutral `ServiceAllocFact` fixtures | Struct literals lacked the PRCI-1 field. | Add `BTreeMap::new()` only. | Existing scenario meaning is unchanged. |
| `service_lifecycle_probe_to_stable.rs` | Projected the ProbeRunner row only into the legacy status field. | Also carries that exact stored row in the fact-local map. | ProbeRunner Pass row still drives Stable through the real reconcile port. |
| `aggregate_validation.rs` | Historical comment named `startup_attempts_per_alloc`. | States the durable rule: no row consumes no startup attempt. | No executable coverage change. |
| `crates/overdrive-control-plane/tests/acceptance/fixtures/hydration_golden/service_lifecycle.txt` | Expected the pre-PRCI-1 fact Debug shape. | Adds the exact seeded Startup row already written by the characterization input. | The existing byte-for-byte hydration characterization remains unchanged and source-honest. |
| `crates/overdrive-control-plane/src/reconciler_runtime/probe_count_fixture.rs` | Constructed `AppState` before either shared owner was ready and with a closed EXEC gate, so a fresh StartAllocation could execute fail-closed driver cleanup before the liveness oracle. | Uses the repository's existing boot composition order: start shared mTLS owner, converge the same guest-network owner, open EXEC, pass those instances into `AppState`. | Fresh allocation still must start at run/streak 1 and incur zero liveness stops; no assertion or scenario changed. |

## Deletion disposition

The removed current-View fields are:

- `startup_attempts_per_alloc`;
- `liveness_consecutive_failures`;
- `readiness_consecutive_successes`;
- `startup_last_fail_seen_at`.

Tests whose only subject was their arithmetic, reset, timestamp de-duplication,
or the deleted private helper were removed. Functional paths formerly using
those fields were migrated to exact producer rows rather than deleted.

`crates/overdrive-reconcilers/tests/acceptance/probe_run_count.rs` intentionally
retains a test-only `PersistedViewBeforeRunCount` CBOR fixture naming the four
old keys. It proves serde ignores unknown legacy fields, preserves the retained
announcement sets, and defaults `liveness_last_acted_run`; it does not restore
those fields to the current View.

## Schema-fixture boundary

The rkyv V1 golden bytes in
`crates/overdrive-core/tests/schema_evolution/probe_result_row.rs` are
untouched by this alignment. No code generation or durable schema fixture was
regenerated. The fact-local PRCI-1 map is transient hydrated State.

## Review and replay handoff

The fresh DISTILL reviewer should check:

1. every behavioral fixture keeps its retained status projection and raw row
   mutually consistent;
2. missing row remains absence, with no invented default row;
3. `ServiceLifecycleState` has no fourth field and private helper signatures
   remain the PRCI-1 forms;
4. no deleted current-View field survives outside the intentional legacy CBOR
   fixture;
5. startup, readiness, liveness, same-row act-once, newer-row retry,
   non-Running suppression, single restart authority, View persistence, and
   reclamation assertions remain at least as strong as before.
6. every retained Startup projection and observation timestamp comes from the
   same fact-local row, including readiness preconditions and invalid-state
   invariant fixtures;
7. the resubmission fixture's fresh driver-stop baseline is no longer polluted
   by an unstarted shared mTLS owner or closed EXEC gate, while the original
   no-premature-liveness-stop oracle remains exact.

After approval, replay the affected tests through `cargo xtask metal run --`.
Do not use Lima and do not change physical-host or kernel state.
