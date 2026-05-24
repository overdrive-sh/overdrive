# ADR-0056 — `ServiceSubmitEvent` gains `Stable { settled_in, witness }` and `Failed { reason: ServiceFailureReason }`; rkyv-envelope V1→V2 bump; `ServiceFailureReason` evolution discipline

## Status

Accepted. 2026-05-24. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/service-health-check-probes/`.

Tags: phase-1, service-kind, wire-shape, rkyv-envelope, streaming.

**Amends**: ADR-0032 (NDJSON streaming + per-kind SubmitEvent
envelope), ADR-0047 (§3 wire shape for ServiceSubmitEvent), ADR-0048
(rkyv versioned envelope discipline). **Companion ADRs**: ADR-0054
(ProbeRunner), ADR-0055 (ServiceLifecycleReconciler), ADR-0057
(TOML spec).

## Context

ADR-0047 §3 defined `ServiceSubmitEvent` with variants `Accepted`,
`Pending`, `Running`, `ConvergedRunning`, `ConvergedFailed`,
`ConvergedStopped`. Per RCA-A and the
`service-health-check-probes` feature, `ConvergedRunning` is the
exact misleading variant — it fires when the kernel accepts
`fork+exec`, before the workload is operator-meaningfully serving.

The feature replaces the structural shape: a new `Stable` variant
fires only after `ServiceLifecycleReconciler`'s startup gate
(ADR-0055) emits the deciding `TerminalCondition::Stable`; a new
`Failed { reason }` variant fires for `StartupProbeFailed` and
`EarlyExit`. The wire boundary lands at the streaming subscriber per
ADR-0032 §"dispatcher walks the broadcast bus".

Open questions resolved here (P1-Q3 part 2):

- How does `ServiceFailureReason` evolve SemVer-wise? Single per-kind
  reason enum or per-condition reason enums?
- How does the rkyv envelope version bump per ADR-0048?
- What happens to `ConvergedRunning` — kept as alias, deleted, or
  superseded?

## Decision

### 1. `ServiceSubmitEvent` shape — V2

```rust
// crates/overdrive-control-plane/src/api.rs (amended)

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceSubmitEvent {
    Accepted { spec_digest: String, intent_key: String, outcome: IdempotencyOutcome },
    Pending  { reason: Option<String> },
    Running  { since: String },                    // informational, NOT terminal

    /// NEW per ADR-0056. Reconciler-confirmed operator-meaningful liveness.
    /// Replaces ADR-0047 §3's `ConvergedRunning` in wire semantics.
    Stable {
        alloc_id: AllocationId,
        settled_in_ms: u64,                        // Duration projected to wire
        witness: ProbeWitnessWire,                 // probe_idx + role + mechanic summary + inferred?
    },

    /// NEW per ADR-0056. Service did not reach Stable within startup_deadline,
    /// OR exited within startup_deadline before any startup probe could pass.
    Failed {
        reason: ServiceFailureReasonWire,
    },

    ConvergedStopped { alloc_id: AllocationId, by: StopInitiator },

    // `ConvergedRunning` and `ConvergedFailed` from ADR-0047 §3 are
    // DELETED in this slice's same-PR migration. Per
    // `feedback_single_cut_greenfield_migrations.md` no compat shim,
    // no deprecation path; Phase 1 Service-kind wire surface is
    // greenfield prior to this feature.
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ProbeWitnessWire {
    pub probe_idx: u32,
    pub role: &'static str,           // "startup" | "readiness" | "liveness"
    pub mechanic_summary: String,
    pub inferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceFailureReasonWire {
    StartupProbeFailed {
        probe_idx: u32,
        attempts: u32,
        last_fail: String,             // ProbeFailure Display projection
        elapsed_ms: u64,
        startup_deadline_ms: u64,
    },
    EarlyExit {
        exit_code: i32,
        elapsed_ms: u64,
        startup_deadline_ms: u64,
        stderr_tail: Vec<String>,
    },
    // ConvergedFailed from ADR-0047 is folded into this via the
    // `BackoffExhausted` variant added by the liveness slice (Slice 05).
    BackoffExhausted {
        attempts: u32,
        last_exit_code: Option<i32>,
        stderr_tail: Vec<String>,
    },
    // Stopped is a separate event (ConvergedStopped); not a Failed reason.
}
```

The new variants and the supporting Wire types are reachable from
the outer envelope:

```rust
pub enum SubmitEvent {
    Service(ServiceSubmitEvent),  // V2 shape per this ADR
    Job(JobSubmitEvent),          // unchanged
    Schedule(ScheduleSubmitEvent),// unchanged
}
```

### 2. rkyv envelope bump — `ServiceSubmitEventEnvelope::V1 → V2`

Per ADR-0048's "Version-bump procedure" single-commit discipline.
The wire-side `ServiceSubmitEvent` is serde/JSON on the NDJSON wire
(not rkyv-archived) — the envelope discipline applies to the
**persisted observation rows** that carry the wire-projected payload
on the snapshot surface.

Concretely:

- `AllocStatusRow.terminal: Option<TerminalCondition>` (already rkyv
  per ADR-0037 §3 + ADR-0048) gains additive `Stable` and `Failed`
  variants inside `TerminalCondition`. Per ADR-0037 §5 ("New
  variants are additive minor") this is NOT an envelope bump on its
  own — only an inner-enum tail extension. Existing
  `AllocStatusRowEnvelope::V1` continues to decode.
- A NEW rkyv-archived row, `ProbeResultRow` (ADR-0054 §5), ships
  with its own envelope `ProbeResultRowEnvelope::V1(ProbeResultRowV1)`.
  Per ADR-0048's "Version-bump procedure" the new row gets its own
  fixture file `crates/overdrive-core/tests/schema_evolution/
  probe_result_row.rs` with `FIXTURE_V1` pinning V1 archived bytes.

There is no `ServiceSubmitEventEnvelope` because the wire is JSON.
JSON additive evolution is governed by serde's default
"ignore-unknown-fields" tolerance + `#[serde(tag = "kind")]`
dispatch on the variant tag. **A V0 (pre-this-feature) consumer
receiving a V1 (this-feature) `Stable` event would receive a
`kind: "stable"` envelope it does not recognise.** Per
`feedback_single_cut_greenfield_migrations.md` Phase 1 Service-kind
streaming is greenfield prior to this feature; no pre-existing V0
consumer exists.

### 3. `ServiceFailureReason` SemVer surface (P1-Q3 resolution)

**Decision: single `ServiceFailureReason` enum (not per-condition
sub-enums).** Same SemVer convention as `TerminalCondition` per
ADR-0037 §5:

- Well-known variants (`StartupProbeFailed`, `EarlyExit`,
  `BackoffExhausted`) are stable contract; renames are major.
- New variants are additive minor (e.g. a future
  `ReadinessExhausted` for `[health_check].readiness.failure_threshold`
  exhaustion if liveness/readiness shapes converge in Phase 2+).
- `#[non_exhaustive]` is required on both the typed enum
  (`overdrive-core::transition_reason::ServiceFailureReason`) and
  the wire projection (`ServiceFailureReasonWire`); consumers match
  with a wildcard arm.

Rationale for single-enum over per-condition:

- Operators reason about "this Service failed; why?" — a single
  surface. Splitting `StartupReason` / `LivenessReason` /
  `EarlyExitReason` forces consumers to match on three orthogonal
  enums for one question.
- The wire JSON has one `reason` field; one enum maps cleanly.
- Future "general Service failure" categories (out-of-memory,
  cgroup-kill, image-pull-fail) land as additive variants without
  schema topology change.

The typed enum lives at
`crates/overdrive-core/src/transition_reason.rs` next to
`TerminalCondition`. The wire projection lives at
`crates/overdrive-control-plane/src/api.rs` next to the wire
envelope. The two are kept in lockstep by a property test
(`every_typed_reason_has_wire_projection`) that walks every
typed variant and asserts a wire projection exists.

### 4. Action-shim integration

The action shim's existing typed write (per ADR-0037 §4) gains
mapping logic from `TerminalCondition::Stable` / `Failed` to the
wire projection. The mapping site is the **single source** for both:

```rust
// crates/overdrive-control-plane/src/streaming.rs (extension)

fn project_terminal_to_service_event(
    terminal: &TerminalCondition,
    alloc_id: &AllocationId,
    started_at: UnixInstant,
    now: UnixInstant,
) -> Option<ServiceSubmitEvent> {
    match terminal {
        TerminalCondition::Stable { settled_in, witness } =>
            Some(ServiceSubmitEvent::Stable { ... }),
        TerminalCondition::Failed { reason } =>
            Some(ServiceSubmitEvent::Failed { reason: reason.to_wire() }),
        TerminalCondition::Stopped { by } =>
            Some(ServiceSubmitEvent::ConvergedStopped { ... }),
        _ => None,  // Job-kind variants ignored on Service path
    }
}
```

The byte-equality contract from ADR-0037 §4 ("AllocStatusRow.terminal
and LifecycleEvent.terminal carry byte-identical values") is
preserved unchanged: the row write and broadcast write are both
sourced from the same `Action::SetTerminalCondition(cond)` payload.

### 5. Streaming-cap interplay (P2-Q5 resolution)

Per `feature-delta.md` C10 (streaming cap 60s default is
**unchanged**) and the slow-warming Services edge case (>60s startup
budget):

- The 60s streaming cap is the operator-facing wait budget. If
  `startup_deadline` (computed per ADR-0058) exceeds 60s, the
  streaming client receives `ServiceSubmitEvent::Running` until cap;
  cap elapses; client exits with the existing `Timeout` wire signal.
- The `ServiceLifecycleReconciler` continues to drive probes after
  the streaming client disconnects; eventual `Stable` lands on
  `AllocStatusRow`; operator inspects via `overdrive alloc status`
  (which shows the Probes section per ADR-0033 / US-06).
- **No new operator knob is introduced in Phase 1.** Operators who
  need to bypass the cap can re-run `alloc status` post-hoc. The
  `--wait-cap` flag and per-spec `startup_deadline_seconds` knob
  are deferred to a future operator-UX iteration; no architecture
  decision is forced today.

This is a deliberate non-decision: the architecture allows the cap
to remain at 60s; operators with slow-warming Services adopt the
"submit → cap → inspect" pattern. If operator feedback demands the
knob, a new ADR adds it (additive: per-spec
`[service.streaming].timeout_seconds`).

### 6. `--json` Probes shape (P2-Q6 resolution — DEVOPS / US-06)

The JSON-mode `alloc status` shape is governed by ADR-0033's
enrichment convention. The Probes section per US-06 renders to JSON
as:

```json
{
  "probes": [
    {
      "role": "startup",
      "probe_idx": 0,
      "mechanic": { "kind": "tcp", "addr": "0.0.0.0:8080" },
      "inferred": true,
      "last_status": "pass",
      "last_observed_at": "2026-05-24T18:42:11Z",
      "last_fail_reason": null,
      "attempts": 3
    },
    /* ... */
  ]
}
```

Schema lives in `crates/overdrive-control-plane/src/api.rs` as
`ProbeResultRowJson` (derived via `utoipa::ToSchema` per ADR-0009).
Per US-06 AC the human-readable text shape and the JSON shape carry
the same per-probe fields.

## Considered alternatives

### Alternative A — Keep `ConvergedRunning` as alias for `Stable`

Map `ConvergedRunning` to `Stable` for backward compat. Rejected:
the ConvergedRunning shape DOES NOT carry `settled_in` or `witness`
fields, so a structural alias is impossible. Per
`feedback_single_cut_greenfield_migrations.md` no deprecation path
is the project convention.

### Alternative B — Per-condition reason enums

Split `ServiceFailureReason` into `StartupFailureReason` and
`LivenessFailureReason`. Rejected for §3 above — operator-facing
single enum is cleaner; future categories land additively.

### Alternative C — rkyv envelope bump for ServiceSubmitEvent

Treat the wire as rkyv-archived and bump
`ServiceSubmitEventEnvelope::V1 → V2`. Rejected: the wire is JSON
per ADR-0032. The bump applies only to persisted rows; new variants
on `TerminalCondition` and the new `ProbeResultRow` are both governed
by ADR-0048 already.

### Alternative D — Make `Stable` carry full ProbeResult history

Embed `Vec<ProbeResultRow>` in `Stable` so the streaming consumer
sees the full set of probes that contributed. Rejected: violates
`development.md` § "Persist inputs, not derived state" by elevating
observation rows onto the wire boundary; the operator's view of
probe history belongs to `alloc status` per US-06, not to the
streaming submit signal.

## Consequences

### Positive

- **Wire shape closes RCA-A structurally for Service kind**:
  `Stable` cannot fire from a kernel-accepted exec; it fires only
  from a reconciler-confirmed startup-gate pass.
- **Single `ServiceFailureReason` enum** matches operator mental
  model; additive new variants are non-breaking.
- **Byte-equality contract preserved** from ADR-0037 §4.
- **JSON-mode `alloc status` Probes section** has a stable schema
  pinned by `utoipa::ToSchema`.

### Negative

- **`ConvergedRunning` / `ConvergedFailed` deletion** (single-cut
  migration). Any DELIVER-wave code touching these constants is
  rewritten in the same PR train.
- **`ServiceFailureReasonWire` is a separate type from the typed
  `ServiceFailureReason`**; lockstep is enforced by property test.
- **Streaming cap interplay (P2-Q5) is a non-decision** today;
  operators with slow-warming Services adopt a workaround. Future
  ADR may add per-spec knob.

### Quality-attribute impact

| Attribute | Impact |
|---|---|
| Functional correctness | RCA-A closed structurally for Service kind |
| Compatibility — evolvability | Additive variant convention for ServiceFailureReason; non-breaking |
| Maintainability — modifiability | One enum, one wire projection, one mapping site |
| Reliability — surface coherence | Byte-equality preserved |

## Cross-references

- ADR-0032 — NDJSON streaming; this ADR amends Service wire shape
- ADR-0037 — TerminalCondition + byte-equality; this ADR adds
  variants
- ADR-0047 — per-kind streaming protocols; this ADR amends Service
- ADR-0048 — rkyv envelope discipline; ProbeResultRow ships V1
- ADR-0054 — ProbeRunner produces inputs
- ADR-0055 — ServiceLifecycleReconciler decides the variant
- `feature-delta.md` P1-Q3, P2-Q5, P2-Q6

## Changelog

- 2026-05-24 — Initial accepted version. Resolves P1-Q3 (in part),
  P2-Q5 (deferred-non-decision), P2-Q6.
