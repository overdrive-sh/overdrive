# ADR-0033 — `alloc status` snapshot enrichment: extend `AllocStatusResponse` in place; share `TransitionReason` with the streaming surface

## Status

Accepted. 2026-04-30. Decision-makers: Morgan (proposing), DISCUSS-wave
ratification of [D6] / [D7] (carried into DESIGN as constraints
[C6] / [C9]).

Tags: phase-1, cli-submit-vs-deploy-and-alloc-status, application-arch,
http-shape.

## Context

Today's `overdrive alloc status --job <id>` prints `Allocations: 1` and
nothing else. The journey-extended TUI mockup
(`docs/feature/cli-submit-vs-deploy-and-alloc-status/discuss/journey-submit-streams-default.yaml`
step 4) specifies the target snapshot:

```
$ overdrive alloc status --job payments-v2
Job:         payments-v2
Spec digest: sha256:7f3a9b12...
Replicas:    1 desired / 1 running

ALLOC ID   STATE      RESOURCES        STARTED               EXIT
a1b2c3     Running    2000mCPU/4 GiB   2026-04-30T10:15:32Z  -

Last transition: 2026-04-30T10:15:35Z
  Pending → Running   reason: driver started (pid 12345)
  source:  driver(process)
Restart budget: 0 / 5 used
```

US-05 makes the rendering AC concrete. US-06 + DISCUSS [D7] / [C6]
require the snapshot's `last_transition.reason` to equal the streaming
endpoint's `LifecycleTransition.reason` byte-for-byte for the same
allocation. This ADR records the **wire shape, the field source map,
the rendering contract, and the back-compat shape** for Slice 01.

DESIGN-wave open questions resolved here: extend `AllocStatusResponse`
in place vs. version it vs. split into per-allocation endpoint;
field set; render contract for Running / Failed / Pending /
no-allocations cases.

## Decision

### 1. Extend `AllocStatusResponse` in place (Call B → B1)

Single-cut migration per [C9]. No `AllocStatusResponseV2`, no parallel
endpoint.

```rust
// in overdrive-control-plane::api

/// Response for `GET /v1/allocs?job=<id>`.
///
/// Phase 1 single-node: one job per query, replicas=1 per slice
/// scope. The shape generalises to multi-replica without breaking
/// changes (the `rows` Vec already holds per-allocation details).
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct AllocStatusResponse {
    /// The job-id the snapshot describes. `None` for the
    /// no-job-filter case (Phase 1 always queries by job).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub job_id:           Option<String>,

    /// Canonical SHA-256 of the rkyv-archived `Job` bytes —
    /// byte-identical to the value `GET /v1/jobs/{id}` returns
    /// for the same job.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_digest:      Option<String>,

    /// Desired replica count from the `Job` aggregate.
    pub replicas_desired: u32,

    /// Allocations currently in `state == Running` (or projected
    /// equivalent). Phase 1: 0 or 1.
    pub replicas_running: u32,

    /// One row per surviving allocation. Empty when no allocation
    /// exists; for Pending/no-capacity cases, see §2.
    pub rows: Vec<AllocStatusRowBody>,

    /// Aggregate restart budget across all rows for this job. `None`
    /// when no allocations exist (the budget is per-allocation in
    /// Phase 1; aggregated at single-replica scale this collapses
    /// to the single allocation's budget).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub restart_budget:   Option<RestartBudget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema, PartialEq, Eq)]
pub struct AllocStatusRowBody {
    pub alloc_id: String,
    pub job_id:   String,
    pub node_id:  String,
    pub state:    String,                              // existing — lowercase per AllocState::Display

    /// Diagnostic on Pending rows resulting from a
    /// `PlacementError::NoCapacity` (existing — slice-01 inherited).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason:   Option<String>,

    // --- NEW per [D2] ---

    /// Resource envelope the allocation was scheduled with.
    pub resources: ResourcesBody,

    /// Wall-clock when the allocation entered the Running state.
    /// `None` for allocations that never reached Running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,                    // RFC 3339

    /// Process exit code if the allocation has terminated. `None`
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code:  Option<i32>,

    /// Last lifecycle transition observed for this allocation.
    /// `None` when no transitions have been recorded yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_transition: Option<TransitionRecord>,

    /// Verbatim driver text or NoCapacity diagnostic for a Failed /
    /// Pending allocation. Sourced from `AllocStatusRow.detail`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct TransitionRecord {
    pub from:   AllocStateWire,
    pub to:     AllocStateWire,
    pub reason: TransitionReason,                      // SHARED with SubmitEvent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub source: TransitionSource,
    pub at:     String,                                // RFC 3339
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct RestartBudget {
    pub used:      u32,
    pub max:       u32,
    pub exhausted: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ResourcesBody {
    pub cpu_milli:    u32,
    pub memory_bytes: u64,
}
```

`TransitionRecord` carries the **same `TransitionReason` enum** that
`SubmitEvent::LifecycleTransition` carries on the streaming surface
(per ADR-0032 §3). Drift is structurally impossible — the type is the
same.

### 2. Field source map (server-side hydration)

| Field | Sourced from | Trait surface |
|---|---|---|
| `job_id`, `spec_digest`, `replicas_desired` | `IntentStore::get(IntentKey::for_job)` → rkyv access of `Job` aggregate; `spec_digest = ContentHash::of(archived_bytes)` | existing |
| `replicas_running` | `obs.alloc_status_rows()` filtered by `job_id`, count where `state == Running` | existing |
| `rows[].alloc_id`, `job_id`, `node_id`, `state`, `reason` | `obs.alloc_status_rows()` projection of `AllocStatusRow` (existing fields) | existing |
| `rows[].resources` | `Job.driver`'s `Resources` (Phase 1: pulled from the Job aggregate; Phase 2+: per-alloc when the runtime tracks resize history) | EXTEND — handler reads Job and projects |
| `rows[].started_at` | First `LogicalTimestamp` where row state transitioned to `Running` | NEW — per-alloc tracking via the lifecycle reconciler view (libSQL `JobLifecycleView` extension) |
| `rows[].exit_code` | Phase 1 not tracked (the ExecDriver does not currently capture the child's exit status; it tracks lifecycle state only). Field is present-but-`None` until Phase 2 ExecDriver enhancement. | NEW — explicit `None` projection |
| `rows[].last_transition` | `obs.alloc_status_rows()`-derived row's `reason` + `detail` + the prior row's state for `from` (lifecycle reconciler view caches prior state per alloc) | EXTEND — handler computes from row + view |
| `rows[].error` | `AllocStatusRow.detail` (verbatim driver text or NoCapacity diagnostic) — populated by the action shim per ADR-0032 §4 | EXTEND — direct projection |
| `restart_budget.used` | `JobLifecycleView::restart_counts.values().sum::<u32>()` (single-replica Phase 1 collapses to one alloc's count) | REUSE — view exists |
| `restart_budget.max` | `RESTART_BUDGET_MAX = 5` constant | REUSE — Phase 1 hard-coded; Phase 2 makes it per-job-config |
| `restart_budget.exhausted` | `used >= max` | derived |

The handler reads the `JobLifecycleView` via the existing
`AppState::view_cache` surface (the same surface ADR-0023 §2 uses for
the action shim's restart-count probe). No new view-cache API; the
read is `view_cache.read(JobLifecycle, target=job_id)`.

### 3. Honest empty-state handling

Three empty-shape cases:

| Case | `rows` | `restart_budget` | Render |
|---|---|---|---|
| Job exists; no allocation yet (post-submit, pre-tick) | `[]` | `None` | "No allocations yet (next reconciler tick will schedule)" |
| Job exists; allocation Pending due to no node capacity | `[{ alloc_id, state: "pending", reason: Some("no capacity..."), error: Some("requested 10 GiB / free 3.2 GiB") }]` | `Some({0, 5, false})` | The single Pending row with the explicit reason — NOT a silent zero-allocations render |
| Job not found (404) | n/a | n/a | `ErrorBody { error: "not_found", message: "jobs/<id>", field: None }` per ADR-0015 |

The handler is exhaustive over the third case (ADR-0015's `NotFound`
maps to 404; CLI exits 2 per [C3] / ADR-0032 §9). The first two cases
both return 200 OK with the structured response; the CLI render
distinguishes them by inspecting `rows.is_empty()` and the per-row
`state`/`reason` fields.

### 4. CLI render contract

The CLI consumes the typed `AllocStatusResponse` and renders the
journey TUI mockup. Three render modes:

#### Running / partially-running

```
Job:         payments-v2
Spec digest: sha256:7f3a9b12...
Replicas:    1 desired / 1 running

ALLOC ID   STATE      RESOURCES        STARTED               EXIT
a1b2c3     Running    2000mCPU/4 GiB   2026-04-30T10:15:32Z  -

Last transition: 2026-04-30T10:15:35Z
  Pending → Running   reason: driver started (pid 12345)
  source:  driver(process)
Restart budget: 0 / 5 used
```

`reason: driver started` is the human-readable rendering of
`TransitionReason::Started`; the `(pid 12345)` is the `detail` field
when present. Mapping:

| `TransitionReason` variant | Human-readable rendering |
|---|---|
| `Scheduling` | `scheduling` |
| `Starting` | `starting` |
| `Started` | `driver started` |
| `DriverStartFailed` | `driver start failed` |
| `BackoffPending` | `backoff (attempt N)` (N from view) |
| `BackoffExhausted` | `backoff exhausted` |
| `Stopped` | `stopped` |
| `NoCapacity` | `no capacity` |

#### Failed (broken-binary regression target)

```
Job:         payments-v2
Spec digest: sha256:7f3a9b12...
Replicas:    1 desired / 0 running

ALLOC ID   STATE     RESOURCES        STARTED  EXIT
a1b2c3     Failed    500mCPU/128 MiB  -        -

Last transition: 2026-04-30T10:18:22Z
  Pending → Failed    reason: driver start failed
  source:  driver(exec)
  error:   stat /usr/local/bin/payments: no such file or directory
Restart budget: 5 / 5 used (backoff exhausted)
```

The verbatim driver error is on its own `error:` line, captured from
`row.error` (which is `AllocStatusRow.detail`). The `(backoff
exhausted)` annotation is unconditional when `restart_budget.exhausted
== true`.

#### Pending — no capacity

```
Job:         oversized-job
Spec digest: sha256:abcdef01...
Replicas:    1 desired / 0 running

ALLOC ID   STATE      RESOURCES         STARTED  EXIT
a1b2c3     Pending    10000mCPU/10 GiB  -        -

Last transition: 2026-04-30T10:20:01Z
  <new> → Pending   reason: no capacity
  source:  reconciler
  error:   requested 10000mCPU/10 GiB / free 4000mCPU/3.2 GiB
Restart budget: 0 / 5 used
```

Single-row Pending with the explicit no-capacity diagnostic, NOT a
silent zero-allocations render. (US-05 AC #3 / journey YAML failure
mode 3.)

### 5. Single source of truth ([C6])

`TransitionRecord.reason: TransitionReason` IS the same type as
`SubmitEvent::LifecycleTransition.reason: TransitionReason`. Both
surfaces serialise it identically via the same `Serialize` derive.
Both surfaces source it from the same `AllocStatusRow.reason: Option<TransitionReason>`
field — the row is the lineage; the streaming endpoint reads it
through the broadcast `LifecycleEvent`, the snapshot endpoint reads
it directly. **An integration test asserts byte-for-byte equality** in
the broken-binary regression case (US-06 AC #4, KPI-04).

## Considered alternatives

### Alternative A — Replace `AllocStatusResponse` with `AllocStatusResponseV2`

**Rejected.** Single-cut greenfield migration ([C9]) means there is
no `v1`-vs-`v2` story; existing fields are preserved
(`rows[].alloc_id`, `job_id`, `node_id`, `state`, `reason`); new
fields are pure additions. There is nothing to remove. A v2 path
would force a transition window the codebase does not need.

### Alternative B — Split into per-allocation endpoint `GET /v1/allocations/{id}`

**Rejected for Phase 1.** `replicas=1` is the slice-01 OUT-scope
constraint; the cardinality of `rows` is always 1. A per-allocation
endpoint duplicates handler code with no operator-facing benefit.
Phase 2+ multi-replica cases may revisit, at which point the
job-scoped snapshot becomes a list-summary and per-allocation reads
get their own path. Defer.

### Alternative C — Compute `last_transition.from` from successive AllocStatusRow snapshots

**Rejected.** Computing `from` by diffing successive snapshots means
the snapshot endpoint needs to hold per-call state (or read a history
table), and the per-tick state delta is not currently persisted.
Sourcing `from` from the lifecycle reconciler view (which DOES track
per-alloc prior state for the action shim's restart-count logic) is
mechanical and shares the same view ADR-0023 already established.
Single-source-of-truth wins.

### Alternative D — Surface the `TransitionReason` enum on the wire as a free-form string

**Rejected.** `String` invites silent drift between the streaming and
snapshot surfaces. The journey YAML's "told the truth" emotional
contract requires byte-for-byte equality (US-06 AC); a structured enum
is the only way to enforce that mechanically. Type system over
discipline.

### Alternative E — Render `restart_budget.exhausted` as a boolean on the wire AND derive in CLI

**Accepted in this form.** The boolean is redundant with `used >= max`,
but explicit on the wire so a CLI that wants to render the
`(backoff exhausted)` annotation does not have to compare two integers
each time. The redundancy is cheap; the explicit-state shape is
clearer for external consumers (a future Prometheus-style scraper, a
future TUI, etc.).

## Consequences

### Positive

- Snapshot field count rises from 1 (`Allocations: N`) to ≥ 6 actionable
  fields per the journey TUI mockup. KPI-03 met.
- `TransitionReason` enum reuse across streaming and snapshot makes
  KPI-04 (cross-surface coherence) a structural property, not a
  discipline.
- Honest empty-state handling — Pending-no-capacity shows the
  diagnostic; no silent zero-allocations render. (US-05 AC #3.)
- Verbatim driver error reaches the operator without polling
  `journalctl` / `systemctl status` / `cat /sys/fs/cgroup/...`.
  (KPI-02.)
- Snapshot is a typed Rust struct in `overdrive-control-plane::api` per
  ADR-0014; CLI and server can never drift.

### Negative

- `AllocStatusResponse` grows from 1 field to 6. Existing CLI
  consumers (just `overdrive alloc status` itself, in this workspace)
  re-render entirely. External SDK consumers (none in Phase 1) would
  see a backwards-compatible expansion (every new field is `Option`
  or has a default).
- `TransitionRecord.from` requires the lifecycle reconciler view to
  cache per-alloc prior state. Phase 1 view already tracks
  `restart_counts`; adding a `prior_state: BTreeMap<AllocationId,
  AllocState>` field is mechanical and additive (NextView shape per
  ADR-0013 absorbs it cleanly).
- The Phase 1 `exit_code` field is always `None` (the ExecDriver does
  not currently capture child exit status). The field is on the wire
  for forward-compatibility; a Phase 2 ExecDriver amendment populates
  it. Documented in §2 above.

### Quality-attribute impact

- **Maintainability — modifiability**: positive. Snapshot evolution
  is additive `Option` fields; CLI render handles new fields
  gracefully via the `serde(skip_serializing_if)` discipline.
- **Performance — time behaviour (snapshot read)**: positive. One
  IntentStore get + one ObservationStore alloc-rows read + one
  view-cache read; no new network hops.
- **Reliability — surface coherence**: positive. Same enum, same
  source field, byte-for-byte equality with streaming surface.
- **Usability — operability**: positive. Operator's second-day
  inspection workflow collapses from "submit → poll → journalctl →
  systemctl → re-derive state" to "alloc status".

### Migration

Single-cut per [C9]. The CLI's `alloc status` renderer is rewritten
in the same PR that lands the wire-shape change. No deprecation
window. The `AllocStatusRow` rkyv schema change (adds `reason` and
`detail` `Option` fields per ADR-0032 §4) is the only persistent-
artifact change; `Option<T>` archives are forward-compatible for
existing redb files.

### Enforcement

- `cargo xtask openapi-check` catches drift between Rust types and
  `api/openapi.yaml`.
- A unit test asserts `TransitionRecord.reason` and
  `SubmitEvent::LifecycleTransition.reason` resolve to the same
  enum type (compile-time equivalence).
- An integration test (Tier 3) submits a broken-binary spec via
  streaming, captures the `ConvergedFailed.error`, runs `alloc
  status`, captures the `last_transition.detail` (and `error`
  fields), asserts byte-for-byte equality. (KPI-04 / US-06 AC #4.)
- A CLI unit test against fixtures asserts the three render modes
  (Running, Failed, Pending-no-capacity) match the journey TUI
  mockups verbatim.

## Slice 01 back-prop

This ADR provides Slice 01 with:

- The exact `AllocStatusResponse` wire shape and per-row fields.
- The `TransitionReason` enum (shared with ADR-0032).
- The `RestartBudget`, `ResourcesBody`, `TransitionRecord` types.
- The field source map (which trait surface populates which field).
- The empty-state handling rules.
- The CLI render contract for Running / Failed / Pending-no-capacity.

Slice 01 AC #1 (Running render) and AC #2 (Failed render with verbatim
error) are structurally supported by the row schema's `detail` field
and the shared `TransitionReason`. AC #3 (Pending-no-capacity) inherits
from the existing `pending_with_reason` constructor, extended to carry
the structured `TransitionReason::NoCapacity` plus the diagnostic
`detail`.

## References

- DISCUSS-wave decisions [D6] / [D7] / [C6] / [C9] in
  `docs/feature/cli-submit-vs-deploy-and-alloc-status/discuss/wave-decisions.md`.
- DESIGN-wave decision D2 / D7 in
  `docs/feature/cli-submit-vs-deploy-and-alloc-status/design/wave-decisions.md`.
- ADR-0008 — REST + OpenAPI transport.
- ADR-0009 — OpenAPI schema derivation.
- ADR-0011 — `Job` aggregate; intent-side; `JobSpecInput` re-use.
- ADR-0013 — Reconciler primitive; `JobLifecycleView` is the
  restart-budget source.
- ADR-0014 — CLI HTTP client + shared types.
- ADR-0015 — HTTP error mapping; 404 NotFound flow for unknown job.
- ADR-0021 — Reconciler State shape; `JobLifecycleState.allocations`
  is the snapshot's primary input.
- ADR-0023 — Action shim; the writer of `AllocStatusRow.reason`
  / `detail`.
- ADR-0027 — Job-stop HTTP shape.
- ADR-0032 — NDJSON streaming submit (companion ADR; shares
  `TransitionReason` / `AllocStateWire` / `TransitionSource`).
- Feature artifacts:
  `docs/feature/cli-submit-vs-deploy-and-alloc-status/discuss/journey-submit-streams-default.yaml`
  step 4,
  `slices/slice-01-alloc-status-enrichment.md`,
  `discuss/user-stories.md` US-05 / US-06.

## Changelog

| Date | Change |
|---|---|
| 2026-04-30 | Initial ADR. Decision D2 / D7 from the DESIGN wave; constraints carried from DISCUSS wave-decisions. Slice 01 back-prop completed. Echo peer review pending. |
