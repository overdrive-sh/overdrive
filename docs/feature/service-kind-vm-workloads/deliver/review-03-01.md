# DELIVER implementation review — step 03-01 E11 readiness recovery

## Metadata

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Roadmap step | `03-01` — E11 readiness recovery |
| Reviewed commit | `a71db44e45cfc1b3e8419790f59bd2f3cda07673` |
| Reviewed parent | `485c7ca3fdb15ffe4e449420049de32910212b16` |
| Reviewer | Fresh isolated implementation reviewer, Codex (`nw-sc-review-dimensions`) |
| Review iteration | 1 |
| Review date | 2026-09-09 |
| Final verdict | **NEEDS_REVISION** |

This review is limited to the as-landed step-03-01 commit and its parent. The
only file written by this reviewer is this Markdown artifact. The pre-existing
dirty worktree changes (`AGENTS.md`, the feature/design and ADR documents, and
the untracked E11 design-review documents) were read as authority where
applicable and were not modified, staged, or committed.

## Scope and authority

The reviewed contract is the approved `03-01` entry in
`docs/feature/service-kind-vm-workloads/deliver/roadmap.json`, together with:

- `design/amendment-e11-readiness-wake.md` and
  `design/review-amendment-e11-readiness-wake.md`;
- the revised ADR-0101 and ADR-0084, feature delta, and wave decisions;
- the upstream S-SVM-27A/B/C scenarios and acceptance documentation; and
- the `03-01` RED/GREEN/COMMIT history in
  `deliver/execution-log.json`.

The closed implementation surface is exactly the event-only
`ObservationRow::ProbeResult(ProbeResultRow)` and
`ObservationRowKind::ProbeResult` additions, the existing
`ServiceLifecycle` interest declaration changing to the exact
`[AllocStatus, ProbeResult]` pair, accepted-write event projection, and the
private asynchronous router point-read needed to derive the existing Service
workload target. The review does not authorize a new store, public method,
broker capability, target kind, cadence, consumer protocol, lifecycle owner,
or retry/recovery architecture.

## Accepted contract audit

| Contract obligation | Evidence | Result |
|---|---|---|
| Append only `ObservationRow::ProbeResult(ProbeResultRow)` after `Signal`; append only `ObservationRowKind::ProbeResult`; preserve existing order | `crates/overdrive-core/src/traits/observation_store.rs:807-811, 866-888` | **PASS** |
| Total kind projection and exact label `probe-result` | `observation_store.rs:890-932`; `crates/overdrive-core/tests/acceptance/interest_row_kind.rs:213-294` | **PASS** |
| Probe results remain event-only and are not `ObservationWrite`, generic history, a new envelope/table, or gossip payload | `observation_store.rs:807-811`; local adapter `observation_backend.rs:637-642,971-991`; Sim adapter `observation_store.rs:293-298,869-888` | **PASS** |
| Existing `write_probe_result`, subscription, allocation point-read, and probe-list signatures remain unchanged | `docs/feature/service-kind-vm-workloads/design/amendment-e11-readiness-wake.md:160-183`; trait and both adapters | **PASS** |
| Accepted strict-LWW winner emits one existing `SubscriptionEvent::Row(ObservationRow::ProbeResult(_))` after commit; stale/equal writes are silent | local `observation_backend.rs:974-991`; Sim `observation_store.rs:878-888`; local/Sim integration tests | **PASS in code and covered for winner/losers** |
| `ServiceLifecycleReconciler::interests()` returns exactly `[AllocStatus, ProbeResult]`; all other declarations stay unchanged | `crates/overdrive-reconcilers/src/service_lifecycle.rs:464-477`; all reconciler declarations | **PASS** |
| Probe-event routing uses existing `alloc_status_row`, routes only a current Service to `workload/<workload_id>`, and ignores orphan/non-Service rows | `crates/overdrive-control-plane/src/lib.rs:3476-3523`; `tests/acceptance/interest_router.rs:503-559` | **PASS for covered outcomes; typed point-read failure evidence is missing (F-02)** |
| Existing allocation List, Lagged relist, periodic relist, hydration, broker collapse, and consumer boundaries remain unchanged | `lib.rs:3526-3557,3580-3682`; unchanged hydration/consumer code; full router acceptance suite | **PASS** |
| Readiness changes only `ServiceBackendRow.backends[*].healthy`; Running, Stable, terminal history, liveness, restart, and cleanup owners remain existing owners | `service_lifecycle.rs:475-477` and unchanged reconcile branches; Sim owner-path test and native transcript | **PASS** |
| Seed `25717` terminal dominance and readiness convergence through production composition | `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs:427-768` | **PASS** |
| Native E11 two transitions, peer traffic, same allocation, lifecycle, and zero cleanup | retained native evidence and runner audit below | **Mechanically PASS; catalogue remains pending independent evidence audit** |

## Production entry-point and owner-path review

The accepted event path is implemented without moving ownership:

1. `ProbeRunner::probe_once_and_record` remains the existing producer boundary
   and awaits `ObservationStore::write_probe_result`.
2. The local adapter completes the redb transaction and only then calls its
   existing subscription emitter (`crates/overdrive-store-local/src/observation_backend.rs:971-991`).
   The Sim adapter applies the dedicated LWW map and only then sends the
   event on its existing broadcast (`crates/overdrive-sim/src/adapters/observation_store.rs:869-888`).
3. The existing interest-router task receives that same `Row` stream. Its
   `ProbeResult` arm point-reads `alloc_status_row(&probe_row.alloc_id)`,
   rejects `Ok(None)` and non-`Service` rows, and derives only the existing
   `workload/<current.workload_id>` target (`crates/overdrive-control-plane/src/lib.rs:3495-3515`).
4. The existing broker submits the existing `Evaluation`; the existing
   `ServiceLifecycle` hydration reads the latest probe snapshot through
   `list_probe_results_for_alloc`, computes readiness, and writes the existing
   complete `ServiceBackendRow`. No WorkloadLifecycle evaluation is created by
   the probe event.

The router helper is now `async` only because the already-approved point read
must be awaited. The public router signature, broker capability, subscription
shape, List path, 30-second relist, Lagged handling, and 100 ms convergence
cadence remain unchanged. There is no detached event task, forced abort, or
second retry queue. The point-read error branch logs and returns at
`lib.rs:3499-3507`; the absence of a test exercising that branch is recorded
as F-02 rather than treated as an imagined production failure.

The existing lifecycle branches do not change. In particular, the readiness
transition acceptance test asserts a false then true backend row while the
allocation remains `Running`, its `Stable` view remains announced, and no
`RestartAllocation`, `StopAllocation`, or `FinalizeFailed` action is emitted
(`crates/overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs:238-321`).
The terminal path remains a ServiceLifecycle terminal veto, while
WorkloadLifecycle remains the sole restart/finalization owner.

## Design/API-shape audit

The commit matches the closed amendment surface exactly:

- The two public enum variants are appended in the required positions.
- `ObservationRow::kind` gains exactly one arm and
  `ObservationRowKind::as_str` gains exactly the `"probe-result"` label.
- `ObservationWrite`, `ObservationStore::write`, the durable probe payload,
  its `(alloc_id, role, probe_idx)` key, its existing rkyv envelope/table, and
  generic Sim gossip are unchanged for probe persistence.
- `write_probe_result` still returns `Result<(), ObservationStoreError>`; no
  public accepted boolean, callback, channel, target, cache, or configuration
  value was added.
- `ServiceLifecycle::interests` retains its signature and returns exactly the
  approved two-element static slice. No other reconciler declares
  `ProbeResult`.
- `route_observation_row` and `list_and_route` are private and have the exact
  approved asynchronous shapes. The only compiler/development fallout is
  passing the existing observation store and awaiting the private helper.

No invented public method, type, enum variant beyond the two approved
variants, field, trait parameter, action, broker API, persistence mechanism,
consumer protocol, or lifecycle authority was found.

## LWW, event ordering, routing, and shutdown audit

The local implementation retains the existing transaction boundary:
`apply_probe_result_lww` returns a private accepted boolean, the redb write is
committed, and the async method then emits the event. A strict greater
`last_observed_at_unix_ms` wins; equal and older rows do not call `emit`. A
transaction or commit error returns before `emit`. The Sim implementation
uses the same strict-greater rule in its `(AllocationId, ProbeRole, ProbeIdx)`
map and sends only for an accepted merge. Neither path puts ProbeResult into
generic row history or gossip.

The local and Sim event tests assert the complete typed row, one event per
accepted winner, durable/latest-row agreement, and no event for equal or stale
rows. The live router test asserts one Service evaluation with target
`workload/svc`, and no evaluation for a Job or orphan allocation. The full
16-test `interest_router` acceptance target also passed after the private
helper became asynchronous, preserving subscribe-first boot, allocation List,
Lagged relist, periodic relist, LWW loser suppression, broker collapse, and
cooperative shutdown coverage.

The router remains cooperatively cancellable at its existing `select!` loop;
the implementation does not spawn or abort a detached point-read task. The
production Local and Sim point reads are finite existing adapter operations,
and no reachable production shutdown defect was reproduced. A potential
theoretical cancellation while an arbitrary store future is pending is not a
finding under the repository reachability rule.

## Contract Shape and test-integrity audit

All changed or transitioned tests carry the required declaration. The four
source-local pure-function properties in
`crates/overdrive-core/tests/acceptance/interest_row_kind.rs:231,248,267,286`
use the exact rustdoc line:

```rust
/// CONTRACT_SHAPE: pure-function.
```

The changed router, reconciler, Sim, and local integration properties carry
`/// CONTRACT_SHAPE: bounded-change.`; the retained terminal property remains
`unbounded-preservation`. No `#[should_panic]` RED marker remains on the
activated readiness property, and its assertions cover the full allowed
state/action delta. The Sim test does not inject a synthetic subscription
event or call a private reconciler helper: it uses the production
`ProbeRunner`, Sim observation store, exact interest-table builder, live
router, runtime broker, and `run_convergence_tick` composition. It explicitly
checks the durable probe row against the event, equal/stale silence, backend
false/true transitions, unchanged allocation/Stable/history/restart count,
and terminal late-Pass ineligibility.

The native expectation remains black-box. `runner.sh` drives the checked-in
`examples/service-kind-vm-workloads/run-example.sh` through the built
default-feature binary and the external peer VM Job. It imports no
`overdrive-*` crate, runs no Rust test binary, and recreates no workload or
probe specification inline. Rust tests remain in-process.

The runner's steady-state parser checks `Running` and restart count `0`; the
raw native transcript independently records the same allocation identifier at
all three describes (`product-run.out:37,73,109`) and records the initial
Stable event (`product-run.out:30`). The public Service describe surface has
no later Stable field, and the production terminal/Stable owner path is also
asserted by the Sim composition. Because the captured output proves the
claimed identity and the production path supplies a stable allocation slot
with the durable restart counter, the parser's lack of a separate ID column
is noted but is not promoted to a reachable defect or an unauthorized API
expansion.

## Seeded Sim invariant

The fixed `SEED: u64 = 25_717` is printed on both the terminal and readiness
tests (`service_kind_vm_terminal_invariant.rs:69,172,436`). The readiness test
starts one Service through the registered WorkloadLifecycle/ServiceLifecycle
runtime, the existing allocation driver hook, and the production ProbeRunner
path. It observes:

1. Running plus Stable and a healthy backend baseline;
2. an accepted readiness Fail through `ProbeRunner` and the Sim store;
3. one typed `SubscriptionEvent::Row(ObservationRow::ProbeResult(_))` and
   equality with the durable LWW row;
4. a live ServiceLifecycle broker evaluation and false backend row;
5. unchanged Running, Stable, allocation ID, restart count, and terminal
   history;
6. equal and stale rows with no event or durable change;
7. an accepted readiness Pass, a true backend row, and the same unchanged VM;
8. a real operator-stop terminal row followed by a late accepted Pass that
   remains ineligible and does not create a replacement allocation.

The production owner path is therefore exercised rather than replaced by a
hand-built `ServiceAllocFact` for the E11 wake property. The existing direct
terminal property remains as a separate preservation control.

## Native E11 evidence actuality

The retained manifest at
`verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/verification.yaml`
records:

- `expectation_id: E11`, seed `25717`, `execution_substrate: native-metal`,
  `executed_in_lima: false`, and runner exit `0`;
- the exact harness invocation
  `verification/harness/run-expectation.sh E11`;
- the product and harness SHA as the parent commit plus
  `working_tree_dirty: true`; the accompanying `dirty-status.txt` and full
  `dirty-diff.patch` preserve the source state that was synced to metal.

The retained `product-run.meta` records one native-metal journey with a
1200-second remote owner, 90-second cleanup grace, and 1350-second transport
bound. The product transcript shows one PTY Service deployment with
`Accepted` followed by `Service ... is stable`, one Running allocation, and
the same allocation identifier at each readiness phase. The extracted ledger
is:

| Phase | Readiness | Observed → detected | Latency | Lifecycle | Restarts | Peer result |
|---|---|---:|---:|---|---:|---|
| before | Pass | `1788951794412 → 1788951794742` | 330 ms | Running | 0 | exact reply |
| during | Fail | `1788951803435 → 1788951803632` | 197 ms | Running | 0 | no exact reply |
| after | Pass | `1788951813461 → 1788951813628` | 167 ms | Running | 0 | exact reply |

The raw output also contains three successful public peer Job verdicts and
`E11 teardown deltas: vm=0 probe=0 network=0 cgroup=0 run-directory=0
mount=0 loop=0 preparation=0` (`product-run.out:62,98,134,147-151`). The
runner's checks at `runner.sh:52-85` reject a missing phase, wrong readiness
polarity, a transition above 2000 ms, a non-Running lifecycle, non-zero
restarts, wrong peer outcome, missing successful peer verdicts, or non-zero
cleanup.

The E11 README and INDEX intentionally remain `pending` pending the separate
human/different-fox evidence audit. This implementation review records the
capture's mechanical and transcript facts but does not rewrite that catalogue
status or claim an independent native rerun.

## DES phase and commit audit

The `03-01` execution-log entries retain the failed first attempt and the
post-amendment retry; no failed native result was silently converted to a
pass:

| Sequence | Phase | Status/result | Assessment |
|---:|---|---|---|
| 1 | RED | `EXECUTED` / focused RED pass | Initial bounded RED activation recorded. |
| 2 | GREEN | `EXECUTED` / focused GREEN pass | Initial implementation tests passed. |
| 3 | GREEN | `EXECUTED` / native/Sim failure | Reachable E11 wake defect retained; no green claim. |
| 4 | COMMIT | `SKIPPED` / blocked by dependency | Commit correctly withheld for the failed contract. |
| 5 | RED | `EXECUTED` / `FAIL` | Fresh post-amendment RED re-entry is recorded independently. |
| 6 | GREEN | `EXECUTED` / focused native/Sim pass | Corrected implementation and evidence passed. |
| 7 | COMMIT | `EXECUTED` / `PASS` | Commit `a71db44e` recorded with `Step-Id: 03-01`. |

`PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity
--roadmap-only docs/feature/service-kind-vm-workloads/deliver` passes. The
unscoped full integrity command reports only the expected future-step gap:
step `03-02` has no execution-log entries; that is outside this review and is
not attributed to step 03-01.

Commit metadata is compliant:

```text
Author: Marcus Schack Abildskov <work@marcus-sa.dev>
Co-Authored-By: Codex <codex@openai.com>
Step-Id: 03-01
```

The author was not changed, and no Claude, Anthropic, or generated-by trailer
is present. The 25 changed paths are tightly related to the approved enum,
adapter, router, reconciler, in-process proof, checked-in example, E11
expectation, native evidence, and DES bookkeeping. `Cargo.lock` and the Sim
dev dependency are compiler/test fallout for the bounded in-process test.
There is no unrelated production subsystem or adjacent lifecycle hardening in
the commit.

`git diff --check parent..a71db44e` reports whitespace only in the retained
verbatim PTY output and its full dirty patch (including terminal carriage
returns/alignment and a blank final line); no source-format defect was found.

## Findings — iteration 1

### F-01 — Active observation-kind documentation still declares AllocStatus-only interests

**Severity:** Medium — blocking contract/documentation consistency.

The commit updates the public `ObservationRowKind` documentation to say “All
nine variants are listed” at
`crates/overdrive-core/src/traits/observation_store.rs:863`, but leaves the
next active sentence saying that at Phase 1 a reconciler's interest is
`AllocStatus` alone (`:864-865`). The same as-landed public implementation
changes `ServiceLifecycleReconciler::interests()` to the approved two-kind
slice at `crates/overdrive-reconcilers/src/service_lifecycle.rs:475-477`.
This is a concrete current-contract contradiction in the touched public
rustdoc, not a hypothetical scheduler state: a maintainer or downstream API
reader following the documentation can omit the required ProbeResult wake.

Related active comments still repeat the superseded statement: the core
acceptance module says “all 8 row families” at
`crates/overdrive-core/tests/acceptance.rs:37-44`, and the exit-observer source
and integration commentary still describe ServiceLifecycle as an
AllocStatus-only consumer at
`crates/overdrive-control-plane/src/worker/exit_observer.rs:125-134,222-233`
and `crates/overdrive-control-plane/tests/integration/workload_lifecycle/exit_observer.rs:594-600`.
Historical ADR/design and execution-log records must remain historical; this
finding concerns active explanatory documentation only.

**Smallest remediation:** update the active public rustdoc and directly
related active comments to state that `ServiceLifecycle` declares exactly
`[AllocStatus, ProbeResult]` while the other allocation consumers retain
`AllocStatus`. Update the row-family count to nine. Do not change any API,
behavior, historical record, or architecture.

**Disposition:** Accepted for bounded documentation remediation. No production
behavior change or design amendment is required.

### F-02 — Required typed point-read failure evidence is absent

**Severity:** Medium — blocking proof-obligation completeness.

The approved amendment explicitly requires adapter/router evidence for a
“typed point-read failure” alongside target derivation, orphan/non-Service
rejection, List, Lagged relist, and periodic relist
(`design/amendment-e11-readiness-wake.md:431-450`, especially `:443-448`,
and `design/review-amendment-e11-readiness-wake.md:389-396`). The production
router has a concrete typed-error branch at
`crates/overdrive-control-plane/src/lib.rs:3496-3507`: an existing
`alloc_status_row` error is logged and the ProbeResult edge is ignored.

The changed router test at
`crates/overdrive-control-plane/tests/acceptance/interest_router.rs:509-559`
proves only the Service, Job, and orphan outcomes. The existing
`inject_alloc_status_rows_failure` tests exercise the List method, not the
point-read method, and a repository search found no router test or test seam
that returns an `ObservationStoreError` from `alloc_status_row` while routing
a live ProbeResult. Therefore the required typed-failure behavior is
currently static-only and the router's post-error liveness is unproven.

This is an evidence-completeness finding, not a claim that the error branch is
wrong or that a new production failure state must be invented. The branch is
reachable whenever the existing store point read returns its typed `Result`
error. The original crafter should add a bounded in-process router test using
a test-owned `ObservationStore` delegate or an already-approved test seam:
return one existing typed error from `alloc_status_row`, assert that no
evaluation or synthetic target is submitted, then prove a subsequent accepted
ProbeResult still routes. Do not add a public fault-injection API, retry queue,
broker capability, or architecture mechanism.

**Disposition:** Accepted for bounded test-only remediation. No production
behavior change or design amendment is required.

## Findings and non-findings disposition

| Item | Disposition |
|---|---|
| F-01 active stale interest documentation | Open; return to the original step-03-01 crafter for documentation-only remediation. |
| F-02 typed point-read failure proof obligation | Open; return to the original step-03-01 crafter for a bounded in-process router test. |
| Native runner's separate allocation-ID assertion | Not promoted. The raw transcript records the same ID at all three phases, the durable restart counter is zero, and no production path/reproducible failure shows a different-ID/zero-restart state for this journey. |
| Later Stable visibility in native `describe` | Not promoted. The initial built-product stream records Stable, the unchanged allocation remains Running with zero restart, the internal production composition checks the Stable terminal, and no new public Stable field is authorized. |
| Hypothetical cancellation while an arbitrary point-read future never resolves | Not promoted. No reachable Local/Sim production path or reproducible owner shutdown trace hangs; the router remains cooperative and does not detach/abort work. |
| Sim late Pass after terminal | Not promoted. The test uses the existing public ProbeRunner one-shot producer to model a completed in-flight result after cooperative cancellation, and the existing terminal veto remains authoritative. |
| Mutation testing | Not run, as explicitly forbidden during an individual roadmap step. |

## Verification performed

| Command or evidence | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim -p overdrive-reconcilers --test acceptance -E 'test(terminal_state_wins_and_dead_vm_backend_never_returns_to_eligibility) or test(vm_readiness_flaps_only_backend_eligibility_and_recovers)'` | **PASS** — 2 tests. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(interest_router)' --no-fail-fast` | **PASS** — 16 tests, 325 skipped. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance -E 'test(kind_maps_each_observation_row_variant_exhaustively) or test(kind_projects_each_variant_to_a_distinct_kind) or test(kind_table_covers_every_variant) or test(observation_row_kind_as_str_is_canonical_kebab_label)'` | **PASS** — 4 tests. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --test acceptance -E 'test(accepted_probe_result_emits_once_and_lww_losers_are_silent) or test(seeded_probe_result_wake_converges_vm_readiness_without_restart)'` | **PASS** — 2 tests. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-store-local --test integration --features integration-tests -E 'test(accepted_probe_result_emits_once_after_commit_and_lww_losers_are_silent)'` | **PASS** — 1 test. A full `probe_result_roundtrip` target run also passed 6 tests. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --test acceptance -E 'test(service_kind_vm_workloads)' --no-fail-fast` | **PASS** — 6 tests, 1 skipped. |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | **PASS**. |
| `cargo xtask lima run -- cargo fmt --all -- --check` | **PASS**. |
| `bash -n examples/service-kind-vm-workloads/run-example.sh verification/expectations/E11-vm-service-readiness-traffic-recovery/runner.sh && examples/service-kind-vm-workloads/run-example.sh check-source` | **PASS**. |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver` | **PASS**. Full integrity reports only future step `03-02` missing its entries. |
| Retained native E11 `verification.yaml`, `product-run.meta`, `product-run.out`, `readiness-recovery.tsv`, runner predicates, dirty status/patch | **Mechanically PASS** — native-metal runner exit 0, both transition latencies ≤2000 ms, correct peer outcomes, Running/0 restarts, and zero cleanup delta. Independent evidence status remains pending. |
| `git diff --check 485c7ca3fdb15ffe4e449420049de32910212b16..a71db44e45cfc1b3e8419790f59bd2f3cda07673` | **PASS for source; expected whitespace-only findings in verbatim native evidence and its retained dirty patch.** |

No native run, mutation test, source edit, DES event, staging operation, or
commit was performed by this reviewer. The native evidence was independently
read and parsed; the separate catalogue evidence audit still owns any
`pending` → `satisfied` status transition.

## Remediation disposition and verdict

Both findings are bounded and can be remediated by the original step-03-01
crafter without changing the approved architecture or public API:

1. correct the active interest/count documentation; and
2. add the required in-process typed point-read failure routing test.

The original crafter must then return this same step to this step-specific
reviewer for re-review. The no-iteration-cap DELIVER rule remains in force;
step `03-02` must not begin until a later on-disk iteration records
**APPROVED**.

**NEEDS_REVISION.** The as-landed production wake path, LWW ordering,
terminal/restart ownership, seeded Sim invariant, and retained native E11
journey are otherwise within the approved contract. Approval is blocked by
the active documentation contradiction and the amendment-mandated missing
typed point-read failure evidence, not by a proposed new architecture or an
unproven production failure.

## Review iteration 2 — remediation re-review

### Metadata and review boundary

| Field | Value |
|---|---|
| Step | 03-01 (E11 readiness recovery) |
| Reviewed range | a71db44e45cfc1b3e8419790f59bd2f3cda07673..a94347e1f7334c7de344b869b0e8764f08f206c2 |
| Remediation commit | a94347e1f7334c7de344b869b0e8764f08f206c2 |
| Reviewer | Fresh replacement step-specific reviewer; iteration 2 |
| Review basis | Step 03-01 roadmap contract, ADR-0101/amendment E11, and findings F-01/F-02 above |

This iteration reviews only the remediation range and the two previously
accepted findings. The implementation-review dimensions were applied for
contract shape, reachable behavior, test integrity, scope, and evidence
completeness. The iteration-1 history above is retained unchanged.

### Remediation verification

#### F-01 — active interest/count documentation

The remediation corrected several active statements. The public
ObservationRowKind rustdoc now records all nine variants and the exact
current declarations (ServiceLifecycle uses [AllocStatus, ProbeResult];
WorkloadLifecycle and SvidLifecycle retain [AllocStatus]) at
crates/overdrive-core/src/traits/observation_store.rs:863-866. The related
active comments now use the same three-consumer and two-kind terminology in
crates/overdrive-control-plane/src/worker/exit_observer.rs:130-135,227-232,
crates/overdrive-control-plane/tests/integration/workload_lifecycle/exit_observer.rs:596-602,
and crates/overdrive-core/tests/acceptance.rs:37-44.

The correction is not complete. Two directly related active comments still
contradict the current code:

- crates/overdrive-control-plane/src/lib.rs:3082-3084 still says there are
  four alloc_status consumers and that the production interest table is
  empty until step 02-03. The current registration immediately builds the
  table from declared interests at :3087, and the current consumers are
  WorkloadLifecycle, ServiceLifecycle, and SvidLifecycle.
- crates/overdrive-core/tests/acceptance/reconciler_trait_surface.rs:682-684
  still describes WorkloadLifecycle as one of four consumers. This is an
  active acceptance-test explanation, not a historical record, and the test
  below it asserts the current [AllocStatus] declaration.

These are documentation-only contradictions; no production behavior or API
defect is being asserted. F-01 therefore remains open. The smallest
remediation is to update only these two active comments to the current
three-consumer/non-empty-table and exact interest declarations. Historical
ADR, design, review, and execution-log records must remain unchanged.

#### F-02 — typed alloc_status_row point-read failure

F-02 is closed by the bounded in-process test added at
crates/overdrive-control-plane/tests/acceptance/interest_router.rs:90-161
and :713-769. The private PointReadFailureStore delegates writes and the
live subscription to the real SimObservationStore; it changes only the
first existing alloc_status_row point read to return the existing typed
ObservationStoreError::Unreachable variant. It adds no production seam,
public fault-injection API, retry queue, or synthetic event.

The test writes the allocation before subscribing, uses a ProbeResult-only
interest table, and then sends an accepted live ProbeResult. The router
exercises the existing point-read error branch exactly once, the broker
remains empty, and the drain is empty, proving that the failed read submits
neither an evaluation nor a synthetic target. A subsequent accepted, newer
ProbeResult on the same Service allocation routes exactly one evaluation to
service-lifecycle at workload/svc. The test carries the required
CONTRACT_SHAPE: bounded-change. declaration and uses the real production
router composition in-process.

### API, architecture, and scope audit

The remediation commit changes six files: three active comments, one core
rustdoc block, one acceptance-test comment, the bounded router acceptance
test, and the step execution log. The production-source changes are
documentation only; the test delegate and test helper are private. No public
method, type, enum variant, trait, parameter, persistence boundary, or
architecture mechanism was added or changed. git diff --check passes for the
remediation range.

The following pre-existing dirty paths were preserved and were not changed by
this reviewer: AGENTS.md,
docs/feature/service-kind-vm-workloads/design/wave-decisions.md,
docs/feature/service-kind-vm-workloads/feature-delta.md,
docs/product/architecture/adr-0084-reconciler-cadence-and-interest-declarations.md,
docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md,
docs/feature/service-kind-vm-workloads/design/amendment-e11-readiness-wake.md,
and docs/feature/service-kind-vm-workloads/design/review-amendment-e11-readiness-wake.md.
Only this review artifact was edited during this re-review.

### DES, commit, and command evidence

The remediation commit preserves the original Git author and contains
exactly one Co-Authored-By: Codex <codex@openai.com> line, exactly one
Step-Id: 03-01 line, and no Claude, Anthropic, or generated-by attribution.
The appended DES events are ordered and all pass:

| Phase | Status | Decision | Timestamp |
|---|---|---|---|
| RED | EXECUTED | PASS | 2026-09-09T13:14:38Z |
| GREEN | EXECUTED | PASS | 2026-09-09T13:15:47Z |
| COMMIT | EXECUTED | PASS | 2026-09-09T13:16:55Z |

Earlier failed attempts remain retained as history; the final remediation
RED → GREEN → COMMIT sequence is valid. Roadmap-only DES integrity passes.
Full DES integrity reports only the expected future-step gap (03-02 has no
entries yet); it reports no 03-01 violation.

| Command | Result |
|---|---|
| cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(probe_result_point_read_error_drops_only_one_wake)' --no-fail-fast | PASS — 1 test, 341 skipped |
| cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(interest_router)' --no-fail-fast | PASS — 17 tests, 325 skipped |
| cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests | PASS |
| cargo xtask lima run -- cargo fmt --all -- --check | PASS |
| cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings | PASS |
| PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver | PASS |
| PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity docs/feature/service-kind-vm-workloads/deliver | Expected step-boundary result — only future 03-02 is missing entries |
| git diff --check a71db44e45cfc1b3e8419790f59bd2f3cda07673..a94347e1f7334c7de344b869b0e8764f08f206c2 | PASS |

Mutation testing was not run, as required for an individual roadmap step.
The retained native-metal E11 evidence was not rerun by this reviewer; its
independent catalogue audit remains the owner of any pending evidence-status
transition.

### Iteration-2 findings and dispositions

| Finding | Status | Disposition |
|---|---|---|
| F-01 active stale interest/count documentation | OPEN | The remediation corrected several active statements but left the two concrete contradictions at lib.rs:3082-3084 and reconciler_trait_surface.rs:682-684; return to the original step-03-01 crafter for documentation-only correction. |
| F-02 typed point-read failure proof obligation | CLOSED | The live accepted ProbeResult test proves the existing typed failure drops only that wake and the next accepted ProbeResult reaches workload/svc, with no evaluation or synthetic target on the failure. |

### Remediation disposition and final verdict

F-02 is fully remediated within the approved contract. F-01 remains a
bounded active-documentation defect, with a two-comment correction sufficient;
no design amendment, public API, production behavior change, or architecture
expansion is required. The original step-03-01 crafter must correct those
comments and return this same step to a step-specific reviewer before step
03-02 begins.

**NEEDS_REVISION.** The E11 production wake path, LWW ordering,
terminal/restart ownership, seeded Sim invariant, and retained native journey
remain within the approved contract. Approval is blocked solely by the two
remaining active documentation contradictions; the typed point-read failure
evidence is now complete.

## Review iteration 3 — F-01 remediation re-review

### Review metadata and boundary

| Field | Value |
|---|---|
| Roadmap step | 03-01 |
| Remediation commit | `d516c021241059f63bce03b9864301d92968ce6e` |
| Predecessor | `a94347e1f7334c7de344b869b0e8764f08f206c2` |
| Reviewed scope | The remediation commit, its DES records, and F-01 from this artifact's iteration 2 |
| Reviewer | Fresh step-specific iteration-3 reviewer; original reviewers unavailable |
| Review date | 2026-09-09 |

This iteration is limited to the two stale active comments named by F-01.
The review did not modify production code, tests, DES files, design artifacts,
or any foreign dirty path. The only permitted write is this appended review
section; all pre-existing dirty work remains preserved.

### F-01 correction verification

The remediation contains exactly the two requested comment corrections.

* `crates/overdrive-control-plane/src/lib.rs:3082-3086` now states that the
  three current `alloc_status` consumers have non-empty interests. It names
  `WorkloadLifecycle` and `SvidLifecycle` with the exact
  `&[ObservationRowKind::AllocStatus]` declaration and
  `ServiceLifecycle` with the exact
  `&[ObservationRowKind::AllocStatus, ObservationRowKind::ProbeResult]`
  declaration.
* `crates/overdrive-core/tests/acceptance/reconciler_trait_surface.rs:682-688`
  now states the same current three-consumer contract and the exact
  `ServiceLifecycle` pair. The assertion and forwarding behavior immediately
  below the comment are unchanged.

The production declarations that these comments describe are unchanged from
the predecessor: `WorkloadLifecycle` returns the single `AllocStatus`
interest at `crates/overdrive-reconcilers/src/workload_lifecycle.rs:170-172`,
`SvidLifecycle` returns the same single interest at
`crates/overdrive-reconcilers/src/svid_lifecycle.rs:279-281`, and
`ServiceLifecycle` returns the exact two-element slice at
`crates/overdrive-reconcilers/src/service_lifecycle.rs:475-477`.
`AnyReconciler::interests` continues to forward those declarations at
`crates/overdrive-reconcilers/src/lib.rs:217-226`. Thus the remediation is
documentation-only and does not alter behavior, a public API, or the approved
architecture.

### F-02 preservation check

`git diff --quiet a94347e1f7334c7de344b869b0e8764f08f206c2..d516c021241059f63bce03b9864301d92968ce6e -- crates/overdrive-control-plane/tests/acceptance/interest_router.rs`
passes: the F-02 test and its supporting `PointReadFailureStore` remain
untouched by this remediation. F-02 therefore remains CLOSED from iteration 2;
no reachable production failure or new evidence reopens it.

### Stale-string audit

A targeted `rg` audit over the current control-plane, core, and reconciler
source/test paths found no stale F-01 wording at either corrected location:
the old four-consumer `alloc_status` claim, the empty production interest-table
claim, and the alloc-status-only claim are absent from those locations. The
current three-consumer wording and the exact ServiceLifecycle pair are present
at the corrected comments and at the corresponding current production
declarations.

The audit also finds a pre-existing migration/provenance comment at
`crates/overdrive-control-plane/tests/integration/workload_lifecycle/exit_observer.rs:784-787`
that refers to the historical “four consumers.” That comment was not part of
the two concrete F-01 contradictions identified in iteration 2 and is outside
this exact remediation boundary; historical ADR, design, evolution, and DES
records likewise remain unchanged as required. It is recorded here for audit
transparency, but is not promoted to a new finding: it does not change the
current production contract, public API, or reachable behavior under review.

### Diff, scope, and architecture audit

The name-status comparison of the exact review range is:

| Path | Change |
|---|---|
| `crates/overdrive-control-plane/src/lib.rs` | Comment text only |
| `crates/overdrive-core/tests/acceptance/reconciler_trait_surface.rs` | Comment text only |
| `docs/feature/service-kind-vm-workloads/deliver/execution-log.json` | Three DES events for this remediation |

`git diff --check a94347e1f7334c7de344b869b0e8764f08f206c2..d516c021241059f63bce03b9864301d92968ce6`
passes. No production behavior, public API, architecture, F-02 file, or
foreign file changed. The execution-log additions are limited to the required
RED, GREEN, and COMMIT records.

### DES and commit evidence

The remediation has the required completed DES phases:

| Phase | Status | Timestamp |
|---|---|---|
| RED | EXECUTED / PASS | `2026-09-09T14:17:42Z` |
| GREEN | EXECUTED / PASS | `2026-09-09T14:18:40Z` |
| COMMIT | EXECUTED / PASS | `2026-09-09T14:18:50Z` |

`des-verify-integrity --roadmap-only` passes for the deliver directory. Full
DES integrity reports only the expected future-step boundary: 03-02 has no
entries yet; there is no 03-01 violation. Commit metadata preserves the
existing Marcus author and committer, contains exactly one
`Co-Authored-By: Codex <codex@openai.com>` trailer, contains no Claude or
Anthropic attribution, and contains exactly one `Step-Id: 03-01` trailer.

Mutation testing was not run because it is prohibited during an individual
roadmap step. The retained native-metal E11 evidence was not rerun by this
documentation-only reviewer; this remediation does not alter that evidence or
the independent catalogue audit.

### Relevant verification

| Check | Result |
|---|---|
| `cargo xtask lima run -- cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` | PASS |
| `cargo xtask lima run -- cargo nextest run -p overdrive-core --test acceptance -E 'test(any_reconciler_interests_forwards_to_inner_reconciler)'` | PASS — 1 passed, 556 skipped |
| `cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --test acceptance -E 'test(interest_router)' --no-fail-fast` | PASS — 17 passed, 325 skipped |
| `PYTHONPATH=/Users/marcus/.claude/lib/python des-verify-integrity --roadmap-only docs/feature/service-kind-vm-workloads/deliver` | PASS |
| `git diff --check a94347e1f7334c7de344b869b0e8764f08f206c2..d516c021241059f63bce03b9864301d92968ce6` | PASS |

These checks are relevant to the comment-only remediation and confirm that
the current contract remains buildable, lint-clean, and covered by the
existing forwarding and interest-router checks.

### Iteration-3 findings and dispositions

| Finding | Status | Disposition |
|---|---|---|
| F-01 active stale interest/count documentation | CLOSED | Both named active comments now state the current three-consumer contract and the exact ServiceLifecycle `[AllocStatus, ProbeResult]` interest; no production or API change was introduced. |
| F-02 typed point-read failure proof obligation | CLOSED / unchanged | The F-02 test and support remain byte-for-byte outside the remediation range; no reachable defect or regression is present in this commit. |
| New findings | NONE | The exact remediation range contains no behavior, API, architecture, or foreign-file change requiring a finding. |

### Final verdict

**APPROVED.** Commit `d516c021241059f63bce03b9864301d92968ce6e` closes F-01
with exactly the two requested stale-comment corrections. F-02 remains closed,
the DES and commit metadata are valid, the stale-string audit is documented,
and no production behavior, public API, architecture, or unrelated dirty work
changed. This iteration-3 review is complete.
