# DISTILL review — STEP 13-02 legacy-field alignment

## Review metadata

| Field | Value |
|---|---|
| Review ID | `STEP13-02-DISTILL-LEGACY-FIELD-REVIEW-01` |
| Feature / roadmap step | `netns-density-295` / `13-02` |
| Review date | 2026-10-28 |
| Reviewer | Fresh independent DISTILL source reviewer |
| Iterations | 2 |
| Design authority | PRCI-1; `review-design-13-02-probe-input-shape.md` iteration 1 |
| Scope | Existing-fixture and compiler-fallout alignment for the deleted consumer streak fields |
| Latest verdict | **APPROVED** |
| Open findings | 0 critical; 0 high; 0 medium; 0 low |
| Historical findings | Iteration 1: 2 blockers; both closed in iteration 2 |

This review applied the current `AGENTS.md`, the repository's Rust, testing,
reconciler and review rules, and the installed nWave DISTILL,
acceptance-review, completeness, test-design and BDD criteria. The
repository-specific direct-Rust and native-Markdown conventions take
precedence over the installed review definition's generic `.feature` and YAML
examples.

## Review boundary and artifact identity

The review covers the DISTILL-owned fixture and assertion changes recorded in
`distill-13-02-legacy-field-alignment.md`, including the source-local
`cfg(test)` module in `overdrive-reconcilers::service_lifecycle`, the existing
ServiceLifecycle acceptance/integration bodies, the VM reclamation property,
the same-row/new-row liveness cases, the single-restart-authority trajectory,
and the existing probe-to-Stable and terminal-invariant fixtures.

The concurrent production implementation, DES log, roadmap state, marker
activation and commit remain the STEP 13-02 crafter's work. Production source
was read only far enough to resolve the approved carrier and the concrete
existing-test paths. This review does not approve the production
implementation. Only this review artifact was added; no source, test,
fixture, design, roadmap, execution log, DES event, model, lane, host state or
commit was changed.

| Artifact | SHA-256 at review |
|---|---|
| `deliver/design-13-02-probe-input-shape.md` | `77e121a1d4eb6df177b88b32a0415ce246badc1ea2f3ad03e086a752ec5d42e2` |
| `deliver/review-design-13-02-probe-input-shape.md` | `712a9bff6c66364d35dc152781a1ba9b79705b6b7a84c93b0c04da832e72f657` |
| `deliver/distill-13-02-legacy-field-alignment.md` | `5955ec8d510a3cf6d2a0890cce5b0b0c5740fdbc4d2480cf0dc74465cee0df1b` |
| `feature-delta.md` | `66786bbbd36e5fbcf2e21258d5625b761513433793fa80ddcc4887be9ca85b09` |
| `adr-0101-service-backend-health-observed-convergence.md` | `300747e7f10269f5b819702498d628cb80246a39b1d4ef99905c85cb00397d70` |

## Exact contract assessed

PRCI-1 authorizes exactly one new carrier:

```rust
#[doc(hidden)]
pub probe_rows: BTreeMap<(ProbeRole, ProbeIdx), ProbeResultRow>,
```

on each `ServiceAllocFact`. The enclosing `ServiceLifecycleState` retains
exactly `allocs`, `service_dataplane`, and `observed_backend_rows`. Every raw
row's allocation, role and index must agree with the enclosing fact and map
key. An absent stored row remains an absent entry. The three retained
`latest_*_probe` projections and `latest_startup_probe_observed_at` must come
from the same hydrated rows, so a fact cannot mix snapshots or carry a
projected probe result without its raw-row input.

The four consumer streak/timestamp fields and `update_startup_attempts` are
deleted. Startup failure reads the startup row's `same_status_runs`; readiness
reads the readiness row's `status` and `same_status_runs`; liveness additionally
reads `run_seq` and records that sequence only when emitting the existing stop.
The helper signatures and every other public/cross-crate type remain as pinned
by ADR-0101 and PRCI-1.

The source package satisfies several important parts of that contract:

- `ServiceLifecycleState` has the exact three fields; no parallel raw-row map
  survives.
- The fact carrier has the exact documentation-hidden map and no second
  accessor, wrapper, action, port, enum variant or test seam.
- The current View contains `liveness_last_acted_run` and none of the four
  deleted fields. Repository search finds the old names only in the deliberate
  `PersistedViewBeforeRunCount` compatibility fixture and explanatory
  Markdown.
- The startup counter/timestamp helper property and the counter-reset-only
  liveness/readiness properties were deleted with their production subject.
  Functional threshold, pass/fail, Stable, terminal-dedup, same-row act-once,
  newer-row retry, non-Running suppression, restart-authority, View
  write-through and reclamation coverage remain present.
- The rkyv V1 probe-row fixture bytes are unchanged; the only diff in that
  schema-evolution file is activation of the already-authored test.
- Existing and transitioned live properties retain their Contract Shape
  classifications. The two source-local properties retain the exact
  `/// CONTRACT_SHAPE: pure-function.` line.

Those strengths do not close the two source blockers below.

## Findings

### F-13-02-DISTILL-01 — behavioral facts still mix retained projections with missing or different raw rows

**Severity: BLOCKER. Kind: `AT_GAP_IN_DELIVERY_SCOPE`.**

PRCI-1 requires the retained status/time projections and `probe_rows` to be
projections of the same allocation-scoped row snapshot. Several migrated
acceptance fixtures instead preserve a projected Startup result while adding
only the role currently under test, or preserve a startup observation time
that differs from the inserted row:

- `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_readiness.rs:88-111`
  constructs every readiness fact with `latest_startup_probe = Some(Pass)` and
  an empty raw-row map. `state_with` at `:152-158` inserts only a Readiness row.
  The resulting production-driving fact therefore claims a Startup Pass for
  which no Startup row exists. The no-readiness and first-readiness cases use
  the same helper.
- `crates/overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs:1067-1098`
  has the same Startup Pass projection, while `readiness_state` at
  `:1121-1127` inserts only the Readiness row.
- `crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs:488-495`
  asks the real reconciler to announce Stable from `running_fact(...,
  ProbeStatus::Pass, ...)` before any Startup row is recorded. The readiness
  journey beginning at `:315` likewise adds a Readiness row to a fact whose
  projected Startup Pass has no raw Startup row.
- `crates/overdrive-control-plane/tests/acceptance/service_lifecycle_stable.rs:311-325`
  retains `latest_startup_probe_observed_at = 1200ms`, while the inserted run-3
  row's `last_observed_at_unix_ms` is `3` because `put_startup_row` at
  `:135-156` derives observation time from `run_seq` and does not update the
  retained projections. The status agrees, but the fact still combines two
  snapshots contrary to PRCI-1.

This is directly reproducible from the current fixture constructors: after
`state_with(readiness_fact(...))`, lookup of `(ProbeRole::Startup,
ProbeIdx::new(0))` is `None` while `latest_startup_probe` is `Some(Pass)`; in
the startup-failure fixture the retained timestamp is `1200` while the row
contains `3`. No hypothetical schedule or private production state is
involved. These facts are passed directly into the production
`ServiceLifecycleReconciler::reconcile` driving boundary.

**Required disposition:** align the private fixtures so every retained
`Some(status)` and startup observation stamp is sourced from the exact raw row
the fact carries. Add the missing Startup rows in the readiness and VM
lifecycle fixtures, and make row helpers update all retained projections from
the row (or construct both from one input). Keep the existing scenarios and
assertions; add no public helper, model, scenario or lane.

### F-13-02-DISTILL-02 — the live hydration characterization golden cannot match the accepted carrier

**Severity: BLOCKER. Kind: `AT_GAP_IN_DELIVERY_SCOPE`.**

`ServiceAllocFact` derives `Debug` and now contains `probe_rows`
(`crates/overdrive-reconcilers/src/service_lifecycle.rs:61,207-212`). The
non-ignored characterization test
`pre_move_hydrated_anystate_golden_covers_all_seven_reconcilers` renders the
real hydrated `AnyState` with `{actual:#?}` and compares it byte-for-byte with
the committed fixture
(`crates/overdrive-control-plane/tests/acceptance/hydration_characterization_golden.rs:290-317,345-371,378-384`).
Its fixed Service input writes an actual Startup Pass `ProbeResultRow` before
hydrating ServiceLifecycle (`:412-432`), so the accepted actual output must now
contain the fact-local Startup row.

The committed
`crates/overdrive-control-plane/tests/acceptance/fixtures/hydration_golden/service_lifecycle.txt:22-57`
ends the fact after `liveness_failure_threshold: 3` and contains no
`probe_rows` field. This is a deterministic existing-test failure: the current
derived Debug value necessarily contains a field that the exact expected
string omits, and the hydrated value additionally contains the seeded Startup
row. Compilation does not exercise this equality, which explains why both
reported Metal `cargo check` commands passed.

**Required disposition:** update only the existing ServiceLifecycle hydration
characterization fixture to the exact approved hydrated value, and add it to
the alignment handoff's coverage/file inventory. Do not alter the assertion,
regeneration gate, production carrier, or rkyv V1 golden bytes, and do not add
a scenario or lane.

## Retained functional coverage

| Required behavior | Existing source evidence | Review result |
|---|---|---|
| Startup threshold, deadline, exact reason and attempts | `service_lifecycle_reconcile_branches.rs` Category C; `service_lifecycle_stable.rs` startup-failure case | Retained; fixture snapshot consistency requires F-01 remediation. |
| Stable on Startup Pass and terminal dedup | `service_lifecycle_stable.rs`; `service_kind_vm_workloads.rs` | Retained; the VM journey's initial Startup row is missing. |
| Readiness eligibility and live threshold | Core and control-plane readiness properties plus the VM readiness journey | Retained; Startup precondition rows are missing in the two readiness suites and VM journey. |
| Liveness threshold | Core liveness property, control-plane liveness suite, VM liveness journey | Retained with producer streak inputs. |
| Same row acts once | `liveness_terminate_records_run_and_same_row_does_not_reemit`; control-plane same-row case | Retained. |
| Newer crossed row retries | `liveness_terminate_is_once_per_row_and_retries_on_the_next_crossed_row` | Retained. |
| Non-Running suppression | Core predicate property and `liveness_predicate_false_once_row_leaves_running` | Retained. |
| Single restart authority | `single_restart_authority_liveness_trajectory.rs` and `workload_lifecycle_alone_decides_restart_after_liveness_stop` | Retained with exact raw liveness input. |
| View persistence equality/write-through | `reconciler_runtime_view_store.rs` changes run 3 to run 4 | Retained. |
| Reclamation complement / unchanged universe | `vm_reclamation_plan_purity.rs` maps positive attempts to a raw row and zero to absence | Retained and honest. |
| Production hydration snapshot | Existing seven-reconciler characterization golden | Present but deterministically red until F-02 is fixed. |

The deletion of the missing-row StartupProbeFailed case is correct: under the
approved contract, no row supplies no attempts and cannot satisfy the startup
failure threshold. The removed counter increment/reset properties likewise
had no surviving subject. No sad path, threshold boundary, act-once/retry
case, restart-authority trajectory, View persistence complement or reclamation
complement was otherwise erased.

## Acceptance-review dimensions

| Dimension | Score | Assessment |
|---|---:|---|
| Happy-path bias | 9/10 | Startup, readiness and liveness failures, threshold boundaries, retry and non-Running cases remain. |
| Given/When/Then form | 8/10 | Existing focused Rust scenarios retain one state transition and observable oracle; several Given facts are not production-reachable PRCI-1 snapshots. |
| Domain language | 9/10 | Startup, readiness, liveness, allocation state and restart authority remain precise. |
| Coverage completeness | 4/10 | The load-bearing hydration golden is stale and several behavioral fixtures omit required raw Startup inputs. |
| Walking-skeleton user-centricity | 10/10 N/A | No walking-skeleton change is in this bounded alignment. |
| Priority validation | 10/10 | The migration removes only the explicitly deleted streak mechanism and keeps the accepted raw inputs. |
| Observable behavior | 6/10 | Action/View assertions are observable, but some pass against impossible mixed-snapshot facts. |
| Traceability | 9/10 | The handoff maps the retained behaviors to PRCI-1 and ADR-0191, but omits the live hydration golden from its file inventory. |
| Walking-skeleton boundary | 10/10 N/A | No boundary strategy changes; composed Sim/host coverage remains in the existing lanes. |

The approval threshold is not met because coverage completeness is below 5
and two source blockers remain.

## Test-design mandates and completeness

| Gate | Result | Evidence |
|---|---|---|
| CM-A — production/hexagonal boundary | PASS | Behavioral bodies still call the production `Reconciler::reconcile` port or production runtime/hydration composition. |
| CM-B — domain-language abstraction | PASS | Assertions remain on actions, View persistence and backend eligibility. |
| CM-C — complete focused journey | FAIL | Several focused journeys start from a fact the production hydrator cannot produce, and the composed hydration characterization is stale. |
| CM-D — pure logic / fixture isolation | FAIL | Some fixtures retain the derived status projection without the raw input that PRCI-1 makes authoritative. |
| CM-E — observable universe | PASS | Liveness and persistence cases retain their changed/unchanged complements; integration layers may use direct assertions. |
| CM-F — layer-dependent PBT | PASS | Pure/in-memory properties remain in the fast layer; no PBT was added to a real-I/O lane. |
| CM-G — two-tier rich journey | PASS / N/A | No new journey or tier is introduced. |
| CM-H — explicit integration sad paths | PASS | Existing startup failure, readiness withdrawal, liveness retry and reclamation cases remain explicit. |

The canonical C1-C7 checklist scores **14/15** for this bounded migration.
Boundary, cardinality, idempotency/act-once, mode, typed-error and interruption
items are retained or delta-inapplicable. C2a fails because the migrated
readiness/VM transitions do not consistently construct a legal hydrated
state. The numeric taxonomy threshold does not override the binary mandate
failures or runnable-suite blocker.

## Mechanical verification

| Check | Result |
|---|---|
| Exact carrier declaration | PASS — one documentation-hidden fact-local map. |
| `ServiceLifecycleState` shape | PASS — exactly three fields. |
| Helper type/order shape | PASS — approved argument/return types retained. |
| Deleted current-View fields | PASS — absent outside the intentional legacy-CBOR fixture and explanatory docs. |
| Deleted private helper | PASS — `update_startup_attempts` absent from live Rust. |
| New test/scenario/lane/model/API | PASS — none added by the alignment. |
| Functional test-body deletion | PASS — only bodies whose complete subject was the deleted consumer streak/timestamp mechanism were removed. |
| Contract Shape | PASS — retained/transitioned bodies keep their classifications; source-local pure properties use the exact line. |
| rkyv V1 fixture bytes | PASS — untouched. |
| Hydration characterization fixture | **FAIL — F-02.** |
| Fact/raw-row snapshot consistency | **FAIL — F-01.** |
| Scoped `git diff --check` | PASS. |
| Reported Metal compile checks | PASS per source handoff; not rerun. |
| Runtime replay | NOT RUN — source review precedes replay; current golden is deterministically stale. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |
| Commit | NOT CREATED. |

## Iteration 1 verdict

**CHANGES_REQUESTED.** The migration correctly removes the deleted consumer
streak/timestamp mechanism and preserves the requested startup, Stable,
readiness, liveness, act-once, retry, non-Running, restart-authority,
persistence and reclamation behaviors. It does not yet provide an honest,
runnable acceptance package for PRCI-1: several production-driving facts mix a
retained Startup projection with no matching raw row or a different observation
stamp, and the live ServiceLifecycle hydration golden necessarily disagrees
with the additive fact field and its seeded Startup row.

Remediation is bounded to private fixture construction, the existing
ServiceLifecycle hydration text fixture, and the alignment handoff inventory.
No production mechanism, public API, model, scenario, lane, persisted schema
or rkyv V1 bytes need change. Return the same source package for iteration 2
before runtime replay.

# CHANGES_REQUESTED

---

# Review iteration 2 — raw-row honesty, hydration golden, and composed-owner fixture

## Iteration metadata

| Field | Value |
|---|---|
| Iteration | 2 |
| Review date | 2026-10-28 |
| Reviewer | Same fresh independent DISTILL source reviewer |
| Scope | F-01/F-02 remediation plus the existing probe-count composition's boot-precondition correction |
| Verdict | **APPROVED** |
| Open findings | 0 critical; 0 high; 0 medium; 0 low |

Iteration 2 preserves the complete iteration-1 record, corrects its evidence
status with the required native receipts, and reviews only the bounded private
fixture/text-golden remediation. It changes no accepted outcome, production
API, owner, lifecycle, action, typed error, persisted schema, rkyv fixture,
model, scenario or lane.

## Iteration-1 reproduction receipt and classification correction

The original crafter ran exactly the six existing bodies selected by the
reviewer through durable Metal. The collector-resolvable run ID is
`d5669ff988ee0ca65fdefd5a` (the source handoff omitted one `e`); nextest run
`4bc0843c-cfda-426e-a1b3-e51f15dc7d5f` executed six tests across 28 binaries.
Five passed:

- `readiness_flips_backend_healthy_within_one_tick`;
- `readiness_pass_threshold_one_is_healthy_fail_is_not`;
- `given_startup_probe_exhausts_attempts_when_reconcile_then_emits_failed_startup_probe_failed`;
- `vm_liveness_threshold_emits_only_the_existing_liveness_stop`; and
- `vm_readiness_flaps_only_backend_eligibility_and_recovers`.

The sole failure was the existing non-ignored
`pre_move_hydrated_anystate_golden_covers_all_seven_reconcilers`. At
`hydration_characterization_golden.rs:309` the exact assertion reported
`HYDRATION CHARACTERIZATION GOLDEN DRIFT for service_lifecycle.txt`. Its
actual left-hand value contained:

```text
probe_rows: {
    (Startup, ProbeIdx(0)): ProbeResultRowV2 {
        alloc_id: AllocationId("svc-app-0"),
        role: Startup,
        status: Pass,
        last_observed_at_unix_ms: 1700000000000,
        run_seq: ProbeRunSeq(1700000000000),
        same_status_runs: SameStatusRuns(1),
    },
}
```

The expected right-hand fixture ended after `liveness_failure_threshold: 3`.
The run summary was `5 passed, 1 failed, 2093 skipped`. This is the concrete
existing-test reproduction for F-02.

F-01 is classified more precisely after that run: it was **not** a runtime
assertion failure. All five affected behavioral selectors passed. It was a
source-honesty violation of PRCI-1's approved raw-input invariant: each retained
`latest_*_probe` and `latest_startup_probe_observed_at` is a projection of the
same fact-local raw row, and an absent stored row is an absent entry. The
current test paths passed projected Startup status directly to production
`ServiceLifecycleReconciler::reconcile` while omitting the raw Startup row, or
paired the projection with a different timestamp. Those states cannot be
produced by `hydrate_service_alloc_facts`, even though the pure reconciler
accepted them. Remediation was therefore required to make the existing Given
facts honest, while every behavioral assertion remained unchanged.

## F-01 remediation review — exact fact-local inputs

F-01 is closed.

- `service_lifecycle_readiness.rs:152-162` now records the retained Startup
  projection before the Readiness row. `put_startup_row` at `:193-215` writes
  the matching role/index/status, first run/streak and observation stamp.
  `put_readiness_row` at `:167-190` now updates the retained Readiness
  projection from the same status it places in the raw row.
- `service_lifecycle_reconcile_branches.rs:1121-1131` now records both the
  Startup and Readiness rows. Its existing generic helper at `:116-149`
  updates the matching retained projection and Startup observation time from
  the same row input.
- `service_kind_vm_workloads.rs:83-128` now makes `state_for` carry the exact
  Startup row before any Stable, readiness or liveness transition. The same
  private `record_probe_run` helper updates each retained role projection and
  the raw row together. The existing scenario bodies and action assertions
  are unchanged.
- `service_lifecycle_stable.rs:135-161` now accepts observation time as its own
  raw input. The startup-exhaustion case at `:313-331` supplies run/streak 3
  and `last_observed_at_unix_ms = 1200`, so the retained timestamp and raw row
  describe the same stored observation while the existing deadline/reason/
  attempts oracle remains exact.
- The intentionally invalid `Running + started_at == None` fixture in
  `service_lifecycle_hydrate.rs:648-700` now violates only the invariant named
  by that test. Its retained Startup Pass has a matching Startup row, so an
  unrelated PRCI-1 inconsistency can no longer trigger or obscure the expected
  hydrate-invariant panic.

No helper is public or cross-crate. No row is synthesized for a retained
`None`; missing row remains absence. Allocation, role and index identities
match every enclosing fact/map key.

## F-02 remediation review — exact text golden

F-02 is closed. The existing
`tests/acceptance/fixtures/hydration_golden/service_lifecycle.txt` adds only
the exact Startup row printed as the failed run's left-hand value: allocation
`svc-app-0`, Startup/index 0, Pass, observation/run sequence
`1700000000000`, and streak one. The assertion and regeneration gate are
unchanged. The rkyv V1 byte constant remains byte-for-byte unchanged; its only
source diff is removal of the existing pending marker.

Durable Metal run `cc6ad1f224e778bb26ea692f`, nextest run
`0ea2aa6a-d281-45f2-bda1-5ac6317067db`, replayed the same six selectors after
remediation. All six passed, including the byte-for-byte seven-reconciler
hydration characterization. Summary: `6 passed, 2093 skipped`.

## Probe-count composition receipt and causal proof

The activated existing
`resubmitted_workload_counts_its_probe_runs_afresh` cell then supplied a
second, distinct concrete receipt. Durable Metal run
`e3dedf287783b22c6b736ff7`, nextest run
`f2432452-cd19-4872-a95a-baa6ad6c471d`, reached its original assertion at
`probe_run_count.rs:350` and reported:

```text
assertion `left == right` failed: one Fail row of its own stops nothing
  left: 1
 right: 0
```

The retained preconditions immediately before that assertion passed: the
fresh allocation ID differed from the predecessor, and its Liveness row was
exactly `(run_seq = 1, status = Fail, same_status_runs = 1)`
(`probe_run_count.rs:333-350`). The stop therefore needed classification; the
assertion was not weakened.

The complete current owner/action path proves the count was fixture boot
cleanup rather than an early liveness stop:

1. The old fixture constructed `shared_network_test_ports::mtls_worker()`,
   whose constructor is explicitly an unstarted worker
   (`shared_network_test_ports.rs:668-690`), and passed it to `AppState`.
2. The production StartAllocation dispatch calls
   `mtls_lifecycle.start_alloc(&spec)` after the Sim driver has reached
   Running (`action_shim/mod.rs:2591-2609`).
3. A transported allocation on an unstarted worker reaches
   `start_shared_allocation`; absent `shared_owner.owner` returns
   `MtlsInterceptInstallError::SharedOwner { source: NotStarted }`
   (`mtls_intercept_worker.rs:2713-2744,2748-2760`).
4. That exact error enters the accepted fail-closed path, which calls
   `driver.stop(handle)` before writing the failed-start disposition
   (`action_shim/mod.rs:656-684`).
5. `ProbeWorld::stop_calls_for` counts every `SimDriver::stop` call for the
   allocation (`probe_count_fixture.rs:591-594`; `SimDriver::stop_calls` at
   `overdrive-sim/src/adapters/driver.rs:168-172`). Thus the observed one stop
   is exactly the cleanup call the unstarted owner caused.

The private fixture now establishes only production boot preconditions before
constructing `AppState`: it starts the same mTLS worker, converges the same
shared guest-network owner, opens one EXEC wiring after both owners are ready,
and passes those same instances/capability into `AppState`
(`probe_count_fixture.rs:317-350`). This matches the accepted boot order in
`feature-delta.md`: `start_shared_owner`, then `converge_shared`, then
`open_after_boot`; the test owner reports its flow ports only after
`converge_shared` (`shared_network_test_ports.rs:350-364`), and the fixture
constructor copies those ports into the action-shim state only when the same
owner is supplied (`lib.rs:700-708`). The EXEC gate shares its state by `Arc`,
so passing the gate retains the Open state after the local wiring value drops.

This change does not bypass the production owner path, pre-author a probe row,
or change the cell's oracle. It removes only unrelated fail-closed cleanup from
the fixture baseline. The private module remains gated by
`#[cfg(all(test, feature = "integration-tests"))]`; no production API or seam
is added.

The same existing resubmission cell then passed five consecutive times in
durable Metal run `b40b994c12d7bddb5aa22eb3` (nextest runs
`0a5b0f95-2f13-4de3-83b8-afdd06594f7f`,
`dc15c80a-c047-4d9d-a4db-63eb9cbbda5f`,
`ed34270c-2bfe-4442-aa4b-13841f796ea7`,
`9e519883-28ec-4d92-a94e-2c193909c9a1`, and
`573c64c0-56f4-4786-83bf-a11c9ce56ae1`). Every run reported `1 passed`.
The repeated receipt rejects a one-off scheduling explanation without adding a
new test or assertion.

Finally, durable Metal run `0b4285e2edbd13d32d311496`, nextest run
`d162e43e-87a9-4f46-b563-65768bc85701`, executed the complete activated
probe-run-count and Quint selector set: 12 composed probe-count cells, nine
named conformance runs, and the 100-trace randomized conformance run. Summary:
`22 passed, 1101 skipped`. The randomized run used seed `0x1713745b`. This
confirms the fixture precondition correction preserves no-sample, coalescing,
same-row act-once, newer-row retry, readiness, status-flip, backward-clock,
fresh-allocation and guest-dial coverage.

## Iteration-2 gates

| Gate | Result |
|---|---|
| F-01 raw-row/projection identity | PASS — every reviewed retained projection is backed by the same fact-local row. |
| F-02 hydration text golden | PASS — exact failed actual value folded; existing byte equality now passes. |
| Startup failure behavior | PASS — exact reason, attempts and deadline selector passes. |
| Readiness behavior | PASS — both focused properties and the VM flap journey pass. |
| Liveness behavior | PASS — VM threshold journey and complete composed set pass. |
| Same-row act-once / newer-row retry | PASS — composed and Quint cases pass. |
| Fresh-allocation first row / zero premature stop | PASS — original oracle unchanged; five repeats plus complete suite pass. |
| Single restart authority | PASS — fixture cleanup is separated from the liveness decision; no restart API or owner changed. |
| View persistence / reclamation coverage | PASS — source bodies and their exact carrier inputs remain unchanged from the reviewed migration. |
| Contract Shape | PASS — no test declaration changed in remediation. |
| Public API / typed errors / persisted format | PASS — no change; raw-row map remains transient State only. |
| Scenario / model / lane | PASS — no addition or weakening. |
| rkyv V1 bytes | PASS — untouched. |
| `git diff --check` | PASS. |
| Mutation testing | NOT RUN — correctly reserved for the final DELIVER gate. |
| Reviewer-authored source changes | Review artifact only. |

## Iteration 2 verdict

**APPROVED.** F-01 is closed as a source-honesty correction: all retained
probe projections and timestamps now derive from their exact fact-local raw
rows while missing row remains absence, and the original behavioral selectors
still pass. F-02 is closed by folding the reproduced actual hydrated Startup
row into the existing text golden without touching the rkyv V1 bytes.

The additional resubmission receipt was classified through the complete
production action path. The stop came from accepted fail-closed cleanup after
an unstarted shared mTLS owner, not from the one-row liveness predicate. The
private fixture now follows the accepted owner/EXEC boot preconditions with
the same instances passed into `AppState`; the original run-1/streak-1 and
zero-premature-stop assertions remain exact. Focused, repeated and complete
probe-count/Quint replay is green.

There is no remaining DISTILL blocker and no specification ambiguity. The
STEP 13-02 crafter may continue with the normal DELIVER review gate.

# APPROVED
