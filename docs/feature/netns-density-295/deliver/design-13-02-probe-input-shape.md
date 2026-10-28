# STEP 13-02 PROBE-RUN-COUNT input-shape reconciliation

## Status

**APPROVED — fresh independent DESIGN contract review iteration 1 returned
APPROVED on 2026-10-28 with no blocking finding.** See
[`review-design-13-02-probe-input-shape.md`](review-design-13-02-probe-input-shape.md).
STEP 13-02 may resume only against PRCI-1's exact shape; implementation and
DISTILL alignment retain their separate review gates.

This is a bounded interface-contract completion for the already-approved
PROBE-RUN-COUNT behavior in ADR-0191. It changes no outcome, policy, owner,
lifecycle transition, action, port, persistence or wire format, observation
store schema, CLI surface, model, scenario, lane, or typed error set.

## Finding

ADR-0191 requires `ServiceLifecycle` to read the latest probe row's
`run_seq` and `same_status_runs`. The accepted feature delta states that
hydration carries those inputs but leaves their cross-crate representation to
DELIVER. That is a genuine interface gap: `Reconciler::reconcile` is pure and
receives only `ServiceLifecycleState`, while the accepted public state carries
only `ServiceAllocFact` values whose three latest-probe fields contain
`ProbeStatus` alone. `ServiceLifecycleView::liveness_last_acted_run` supplies
the previously acted sequence `A`; no accepted input supplies the current
row's sequence `N` or producer streak `s`.

The partial STEP 13-02 implementation added a fourth public
`ServiceLifecycleState::probe_rows` field and changed private helper
signatures. That shape conflicts with ADR-0101's exact three-field state and
its unchanged helper signatures. Green behavior would therefore still be an
API-contract divergence.

## Production path and necessity

The existing production path is:

1. One probe task owns a key `(AllocationId, ProbeRole, ProbeIdx)` and writes
   `ProbeResultRowV2 { run_seq, same_status_runs, ... }` through
   `ObservationStore::write_probe_result`.
2. `LocalObservationStore` and `SimObservationStore` retain the row with the
   greatest `run_seq` for that key.
3. `ServiceLifecycleReconciler::hydrate_actual` calls
   `ObservationStore::list_probe_results_for_alloc` while constructing each
   `ServiceAllocFact`.
4. The pure `reconcile` receives only the hydrated `ServiceLifecycleState`
   and its persisted `ServiceLifecycleView`.
5. Startup and readiness compare the producer streak with live thresholds;
   liveness additionally compares the current `run_seq` with
   `liveness_last_acted_run` before emitting `StopAllocation`.

The existing `latest_startup_probe`, `latest_readiness_probe`, and
`latest_liveness_probe` fields cannot implement that contract: they retain
status but discard both raw inputs. `latest_startup_probe_observed_at` is a
wall-clock stamp, not a monotone probe-run identity, and cannot substitute for
either input. No bounded arithmetic over reconcile looks can reconstruct the
producer streak; ADR-0191 and the existing `streak_bugReconstruct` Quint
hazard already reject that alternative.

## Decision PRCI-1 — one raw-row collection on each allocation fact

`ServiceAllocFact` gains exactly one public, documentation-hidden field:

```rust
#[doc(hidden)]
pub probe_rows: BTreeMap<(ProbeRole, ProbeIdx), ProbeResultRow>,
```

`ProbeResultRow` is the public latest-payload alias for
`ProbeResultRowV2`. The enclosing `ServiceLifecycleState::allocs` key and
`ServiceAllocFact::alloc_id` supply the allocation identity, so the nested map
key contains only role and probe index.

The exact hydration invariants are:

- each entry is the latest stored row for the enclosing allocation and map
  key after the store's `run_seq` merge;
- `row.alloc_id == fact.alloc_id`, `row.role == key.0`, and
  `row.probe_idx == key.1`;
- the three existing `latest_*_probe` status fields and
  `latest_startup_probe_observed_at` are projections of the same hydrated rows,
  so one fact never mixes two store snapshots;
- an absent stored row is an absent map entry; hydration invents no status,
  sequence, streak, or default row;
- hydration carries every row returned for the allocation, while the current
  policy continues to consume only `ProbeIdx::new(0)` for Startup, Readiness,
  and Liveness. GH #319 remains the sole home of policy for other indices.

All other `ServiceAllocFact` fields, names, types, and meanings remain
unchanged. `ServiceLifecycleState` remains exactly the ADR-0101 three-field
shape: `allocs`, `service_dataplane`, and `observed_backend_rows`. Its
`Default` implementation and `Reconciler` associated-type surface do not
change.

The existing private helper signatures recorded by ADR-0101 remain unchanged,
including `hydrate_service_alloc_facts -> Result<BTreeMap<AllocationId,
ServiceAllocFact>, HydrateError>` and the four-argument
`compute_backend_healthy`. Private local threading inside those bodies remains
the crafter's responsibility. No second public method, tuple return, state
field, wrapper type, accessor, action, or observation API is authorized.

## Consumer contract

For `role` and `ProbeIdx::new(0)`, the reconciler reads the raw row from the
enclosing fact and applies ADR-0191 unchanged:

| Role | Raw inputs consumed | Existing effect |
|---|---|---|
| Startup | `status`, `same_status_runs` | `StartupProbeFailed` only when the latest status is `Fail`, the producer streak reaches `max_attempts`, the existing deadline has elapsed, and no Pass has established Stable |
| Readiness | `status`, `same_status_runs` | backend eligibility is true only for latest `Pass` at or above the live `readiness_success_threshold`, subject to the existing startup terminal veto and no-readiness rule |
| Liveness | `status`, `same_status_runs`, `run_seq` | emit the existing liveness `StopAllocation` only for a Running allocation at or above the live failure threshold with `run_seq` newer than `liveness_last_acted_run`; record that sequence only on emit |

No new row changes nothing. Missing-row semantics, failed-stop retry on the
next newer crossed row, read-time threshold changes, all-workload-kind scope,
and the single restart authority remain exactly as approved.

## Typed error mapping

No new semantic failure is introduced.

| Condition | Exact representation |
|---|---|
| `list_probe_results_for_alloc` read fails | existing `HydrateError::ObservationRead(String)` mapping |
| the allocation has no stored row for a key | absent `probe_rows` entry; not an error |
| a stored row is below threshold or has the other status | ordinary no-action or ineligible result; not an error |

No generic carrier, diagnostic string, new error variant, or error payload is
needed. The existing outer error mapping remains complete for this boundary.

## Alternatives

| Alternative | Disposition |
|---|---|
| Use the three status fields and startup observation time | Rejected: neither `run_seq` nor `same_status_runs` is present, and wall clock is not the run identity. |
| Add `probe_rows` as a fourth `ServiceLifecycleState` field | Rejected: it contradicts ADR-0101's exact state shape, separates one allocation's raw rows from its pre-joined fact, and pressures hydration toward a second map return or a second store read. |
| Change all three `latest_*_probe` fields from `Option<ProbeStatus>` to full rows | Rejected: three established public field types and all their callers change when one additive raw-input collection suffices. |
| Add separate run-sequence and streak scalars for each role | Rejected: at least six public fields duplicate one existing typed row and make status/sequence/streak disagreement easier to construct. |
| Persist crossed booleans or consumer counters | Rejected: they are policy outputs, make threshold changes stale, and contradict ADR-0191's producer streak and read-time threshold. |
| Read the ObservationStore from `reconcile` | Rejected: it breaks the pure synchronous reconciler boundary and adds a port dependency where hydration already owns the read. |

PRCI-1 is the smallest shape that preserves the accepted pre-joined
per-allocation fact boundary, the exact three-field state, the exact helper
signatures, and the approved producer-to-consumer facts.

## Caller fallout

The bounded compiler fallout is limited to:

- `ServiceAllocFact` construction in production hydration and existing
  source-local, acceptance, integration, and simulation fixtures;
- the existing startup, readiness, and liveness decision sites reading the
  nested row for role/index 0;
- removal of the partial, unapproved `ServiceLifecycleState::probe_rows`
  field and tuple-return helper shape.

Neutral fixtures use `BTreeMap::new()`. Fixtures that exercise
PROBE-RUN-COUNT insert the exact raw row they claim. This is compiler-required
fallout of the one field, not a new scenario or test-support API. Existing
DISTILL bodies may be aligned to the approved shape; no new test, lane, model,
or production seam is authorized.

## Ownership and quality attributes

- **Ownership:** the probe task remains the single producer; ObservationStore
  remains the durable arbiter; `ServiceLifecycle` remains the threshold
  consumer; `WorkloadLifecycle` remains the single restart authority.
- **Correctness:** the decider receives the exact producer facts and cannot
  infer a streak from reconcile cadence.
- **Persistence:** the added field is transient hydrated State. No View, row,
  schema, or migration changes.
- **Determinism:** `BTreeMap` preserves the repository's ordered-collection
  rule; the stored row winner remains `run_seq`-ordered.
- **Performance:** no new store call, task, retry, queue, or persisted value is
  required. The transient map contains only rows hydration already reads.
- **Compatibility:** this greenfield step updates bounded Rust struct literals
  in one cut; there is no compatibility wrapper or dual shape.

## Lifecycle Gate Ownership

Not applicable. PRCI-1 transports raw inputs to the three already-approved
Startup, Readiness, and Liveness decisions. It adds, removes, moves, or
reinterprets no gate or lifecycle state.

## Formal protocol model

No new model is required. PRCI-1 changes only the in-process carrier for the
already-modelled PROBE-RUN-COUNT variables `rseq`, `rst`, and `rsame`.
`specs/quint/probe-count` and its existing hazards, runs, and conformance
projection remain unchanged.

## Standing authority and handoff

The user's standing ruling is: “Stop only for a gap that changes the outcome;
for anything else, take the design's evident intent and report it after the
step.” PRCI-1 preserves every approved outcome and pins only the necessary raw
input representation. It therefore needs no separate outcome decision.

The fresh independent DESIGN review approved this contract with no blocking
finding, and the canonical feature delta and ADR-0101 agree with it. Roadmap
validation is restored only for this exact carrier. The original STEP 13-02
crafter owns implementation/remediation and the existing DISTILL agent owns
test-body alignment; neither may invent another public shape.
