# ADR-0064 — Describe-side `DescribeSpecOutput` as a distinct kind-discriminated `oneOf` (mirror of ADR-0051; surfaces the platform-issued VIP)

## Status

**Accepted (2026-06-06)**. User signed off on OQ-1, OQ-4, OQ-5, OQ-7
verbatim as listed below; the recommendations are binding decisions.

Decision-maker: Morgan (proposing); user sign-off recorded 2026-06-06.
Driven by: [overdrive-sh/overdrive#183](https://github.com/overdrive-sh/overdrive/issues/183)
— "WorkloadDescription Service-arm wire-shape widening (oneOf
discriminator for `describe_workload`)". This is the **DESCRIBE-side
mirror** of ADR-0051 (the SUBMIT-side `SubmitSpecInput` discriminator).

**Open-question resolution (confirmed 2026-06-06)**:

- **OQ-1 (wire shape)** — **DISTINCT `DescribeSpecOutput`**. A new
  tagged enum `DescribeSpecOutput { Job(...), Service(...),
  Schedule(...) }`, `#[serde(tag = "kind", rename_all = "snake_case")]`,
  `utoipa::ToSchema` → `oneOf` with `discriminator: propertyName: kind`.
  **NOT** a reuse of `SubmitSpecInput`. Two load-bearing reasons:
  1. **Describe must surface the platform-issued VIP**, which
     `SubmitSpecInput` structurally *cannot* carry — it has no `vip`
     field and is `deny_unknown_fields` (ADR-0051 § 3, ADR-0049 § 5:
     the operator never names a VIP, so the submit shape forbids it).
  2. **Reusing `SubmitSpecInput` would re-couple describe ↔ submit
     evolution cadence** — the exact Pattern-C coupling ADR-0051 was
     created to avoid (§ "Context" → "Constraints"). A describe-only
     field (the VIP) would force a change on the submit wire shape, and
     vice versa.
  `WorkloadDescription.spec` changes type from `JobSpecInput` to
  `DescribeSpecOutput`; `spec_digest` stays top-level on
  `WorkloadDescription`.
- **OQ-4 (Service VIP field)** — **REQUIRED, not `Option`**. The
  Service describe arm carries a non-optional VIP (`ServiceVip`
  newtype wrapping `Ipv4Addr`, serialised as a dotted-quad string on
  the wire). A persisted-and-describable Service ALWAYS has an
  allocated VIP per ADR-0049 (submit-time admission allocates before
  the intent is written). Absence is made unrepresentable per
  `.claude/rules/development.md` § "Type-driven design". **Consequence
  (pinned):** if `allocator.get(&spec_digest)` returns `None` for a
  `WorkloadIntent::Service`, that is an internal-invariant violation →
  HTTP 500 via a typed `ControlPlaneError` variant, NOT a silent
  `Option` / empty string. Job and Schedule describe arms carry no
  VIP.
- **OQ-7 (VIP retrieval)** — **READ-ONLY `allocator.get(&spec_digest)`**.
  Describe MUST NOT mutate allocator state. The read-only method
  **already exists** —
  `PersistentServiceVipAllocator::get(&ServiceSpecDigest) ->
  Option<ServiceVip>` at
  `crates/overdrive-dataplane/src/allocators/persistent_service_vip.rs:251`
  (sync, `&self`, delegates to the in-memory memo). Describe **reuses
  it**; it does NOT call the mutating `allocate(&mut self)` /
  `release(&mut self)`. (Mechanical note: `AppState.allocator` is an
  `Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>`, so describe
  takes the mutex briefly — `let vip = state.allocator.lock().await.get(&digest)` —
  but the *operation* on the allocator is non-mutating. No new allocator
  method is needed.)
- **OQ-5 (Schedule arm)** — **LAND ALL THREE ARMS NOW**. Exhaustive
  enum. The Schedule render path (`ScheduleV1::to_describe()`) lands as
  a `todo!("RED scaffold: ...")` per
  `.claude/rules/testing.md` § "Production-side scaffolds" because the
  Schedule submit path is itself a RED scaffold today
  (`ScheduleV1::from_submit` at `aggregate/mod.rs:646`). The describe
  handler returns a structured rejection on `WorkloadIntent::Schedule`
  so the `to_describe()` body is unreachable from any existing test.
  Mirrors ADR-0051 OQ-5.

### Amendment (2026-06-07) — Service describe wire mirrors probe vecs

The original § 2 Service-arm spec listed the "classic five" operator
fields (`id`, `replicas`, `resources`, `driver`, `listeners`) plus the
`vip`, and omitted the three probe vectors (`startup_probes`,
`readiness_probes`, `liveness_probes: Vec<ProbeDescriptor>`) that
ADR-0057/0058 added to the **submit** shape (`ServiceSpecInput`) and
that `ServiceV1` persists. That left a round-trip gap: an operator who
declares `[[health_check.startup]]` in TOML has it accepted, persisted,
and acted on by the reconciler, but `GET` describe did not reflect it.

**Decision:** the describe wire MUST mirror the probe vectors for an
honest round-trip. `ServiceSpecOutput` now carries all three
`Vec<ProbeDescriptor>` fields (projected read-only from the persisted
`ServiceV1`) in addition to the prior field set and the `vip`. All
three probe vectors (`startup`, `readiness`, `liveness`) are
operator-declared in TOML, parsed, persisted on `ServiceV1`, and
evaluated by the reconciler runtime — readiness flips
`Backend.healthy`, liveness emits a restart (ADR-0055). They are fully
implemented and shipped (the `service-health-check-probes` feature,
finalized 2026-05-30). The describe wire was the only surface dropping
them; surfacing all three closes the round-trip gap. There is no
deferral and no future slice — every probe vector reflects state the
operator can observe today.

Cross-references: issue
[#183](https://github.com/overdrive-sh/overdrive/issues/183) (the
describe-side work this ADR governs) and ADR-0057/0058 (the
service-health-check-probes additions to the submit shape). `ProbeDescriptor`
lives at `overdrive_core::aggregate::probe_descriptor::ProbeDescriptor`
and already derives `utoipa::ToSchema` (it is in the OpenAPI components
graph via `ServiceSpecInput`), so the describe-side surfacing adds no
new schema-registration burden beyond `ServiceSpecOutput` itself.

Tags: phase-1, application-arch, wire-shape, describe-side, oneOf
discriminator, parser-vs-wire-vs-intent separation,
single-cut-migration.

**Relates to**:

- **ADR-0051** (wire-side `SubmitSpecInput`) — the symmetric
  submit-side precedent this ADR mirrors structurally. **This ADR
  amends ADR-0051 § 1**: its boundary note "`WorkloadIntent →
  SubmitSpecInput` — `describe_workload` endpoint echoes the persisted
  intent back to the operator on GET" is SUPERSEDED for the describe
  direction (see ADR-0051 § "Amendment (2026-06-06)"). Describe now
  uses a distinct `DescribeSpecOutput`, not `SubmitSpecInput`.
- **ADR-0050** (intent-side `WorkloadIntent` enum + `JobV1` / `ServiceV1`
  / `ScheduleV1`) — the persisted payloads the `to_describe` render
  paths read from.
- **ADR-0049** (platform-issued Service VIP allocator) — the VIP is
  platform-issued, keyed by `spec_digest`; the operator never names it.
  Describe SURFACES it (read-only) via § 5a's "the allocator's memo IS
  the source of truth".
- **ADR-0047** (workload-kind discriminator — parser-side
  `WorkloadSpec`); **ADR-0048** (rkyv versioned envelope — governs the
  intent layer only; does NOT apply to this ADR's wire layer);
  **ADR-0014** (shared CLI/server request/response types); **ADR-0009**
  (OpenAPI schema generation via utoipa); **ADR-0020** (describe
  response shape `{spec, spec_digest}`).

**SSOT**: this ADR is the SSOT for the wire-side HTTP/JSON shape of the
`GET /v1/jobs/{id}` describe **response**. The submit-side wire shape
(`SubmitSpecInput`) remains owned by ADR-0051; the persisted intent
shape (`WorkloadIntent`) remains owned by ADR-0050.

## Context

### What exists today

- **Describe response wire shape**
  (`crates/overdrive-control-plane/src/api.rs:153-157`):
  `WorkloadDescription { spec: JobSpecInput, spec_digest: String }`.
  The `spec` field is typed `JobSpecInput` — Job-only. This dates from
  the era when Job was the only describable workload kind.
- **Describe handler**
  (`crates/overdrive-control-plane/src/handlers.rs:581-638`,
  `describe_workload`): reads the persisted `WorkloadIntent` from the
  `IntentStore`, computes `spec_digest`, then **hard-rejects** any
  non-Job intent with HTTP 400 at lines 628-635:

  ```rust
  let WorkloadIntent::Job(job) = intent else {
      return Err(ControlPlaneError::Validation {
          field: Some("id".to_owned()),
          message: "describe is only available for Job workloads in Phase 1 (Service/Schedule describe wire shape is GH #183)".to_owned(),
      });
  };
  Ok(Json(api::WorkloadDescription { spec: JobSpecInput::from(&job), spec_digest }))
  ```

  The comment at lines 623-627 explicitly defers Service/Schedule
  describe to this issue (#183).
- **Submit-side wire layer** (`overdrive-core::api::submit`, ADR-0051):
  the kind-discriminated `SubmitSpecInput { Job, Service, Schedule }`
  already exists, with per-kind validating constructors
  (`JobV1::from_submit`, `ServiceV1::from_submit`,
  `ScheduleV1::from_submit`). The `api::` module
  (`crates/overdrive-core/src/api/mod.rs`) declares `pub mod submit;` —
  adding `pub mod describe;` is a one-line additive sibling.
- **Intent payloads** (`overdrive-core::aggregate`, ADR-0050):
  `WorkloadIntent::Job(JobV1)` / `Service(ServiceV1)` /
  `Schedule(ScheduleV1)`. `pub type Job = JobV1`. `JobV1` carries
  `{ id, replicas: NonZeroU32, resources, driver: WorkloadDriver }`;
  `ServiceV1` adds `listeners: Vec<Listener>` plus the three probe
  vectors `startup_probes` / `readiness_probes` / `liveness_probes:
  Vec<ProbeDescriptor>` (ADR-0057/0058) — all three persisted on the
  intent and projected onto the describe wire (§ 2);
  `ScheduleV1` carries `{ id, job: JobV1, cron_expr: CronExpr }`.
- **VIP retrieval** (ADR-0049):
  `PersistentServiceVipAllocator::get(&ServiceSpecDigest) ->
  Option<ServiceVip>` already exists and is read-only. The submit path
  allocates via the mutating `allocate(&mut self)` keyed on
  `*spec_digest_hash.as_bytes()` (a `[u8; 32]` = `ServiceSpecDigest`)
  at `handlers.rs:323-331`. Describe reuses the same digest-bytes key.
- **Existing Job-describe reuse**: `impl From<&Job> for JobSpecInput`
  at `aggregate/mod.rs:896` is what the current handler uses
  (`JobSpecInput::from(&job)`). `Job = JobV1`, so this is
  `From<&JobV1>`.

### The four type families for "a workload"

ADR-0051 established three distinct type families (TOML parser, HTTP
submit wire, persisted intent). This ADR adds a **fourth, narrow corner**
— the HTTP describe wire — distinct from the submit wire because it
carries a derived, platform-owned field (the VIP) that the submit shape
structurally forbids:

| Layer | Type | Direction | Stakeholder | Encoding | Module |
|---|---|---|---|---|---|
| TOML parser | `WorkloadSpec` / `WorkloadSpecInput` | operator → parser | operators writing config | TOML | `overdrive-core::aggregate::workload_spec` (ADR-0047) |
| HTTP submit wire | `SubmitSpecInput` | client → server (request) | HTTP/JSON clients (CLI, SDK) | JSON | `overdrive-core::api::submit` (ADR-0051) |
| Persisted | `WorkloadIntent` | server-internal | reconciler, recovery walk | rkyv envelope | `overdrive-core::aggregate` (ADR-0050) |
| **HTTP describe wire** | **`DescribeSpecOutput` (NEW)** | **server → client (response)** | **HTTP/JSON clients reading GET** | **JSON** | **`overdrive-core::api::describe` (this ADR)** |

The describe wire is a **read-only output projection** of the persisted
intent + the allocator-owned VIP. It is the inverse-direction sibling
of `SubmitSpecInput`: where submit projects `client JSON →
WorkloadIntent` (validation), describe projects `WorkloadIntent (+ VIP)
→ client JSON` (rendering).

### Constraints the decision must respect

- **The VIP is describe-only.** Surfacing the platform-issued VIP on
  the describe response is the core requirement of #183 (the operator
  needs to learn the address the platform assigned). `SubmitSpecInput`
  cannot carry it (no `vip` field, `deny_unknown_fields`). A distinct
  describe type is the only shape that surfaces it without corrupting
  the submit contract.
- **The VIP is read at describe time, never persisted on the response
  shape.** Per `.claude/rules/development.md` § "Persist inputs, not
  derived state": the VIP is an allocator-owned fact derived from
  `(spec_digest, pool policy)` per ADR-0049 § 5a. Describe recomputes
  it on every GET via `allocator.get(&spec_digest)`; the
  `DescribeSpecOutput` value is constructed per-request and discarded
  after serialisation. There is no new persisted field anywhere.
- **rkyv does NOT apply at the wire layer** — same as ADR-0051 § 3.
  `DescribeSpecOutput` is `serde::Serialize + serde::Deserialize +
  utoipa::ToSchema`; no `rkyv::Archive`, no envelope enum. The
  describe wire evolves via serde + OpenAPI semver discipline,
  independent of the intent layer's ADR-0048 envelope cadence.
- **Single-cut greenfield migration**
  (`feedback_single_cut_greenfield_migrations.md`): no aliases, no
  shims, no grace periods. `WorkloadDescription.spec` flips from
  `JobSpecInput` to `DescribeSpecOutput`, every consumer updates, the
  HTTP 400 rejection is removed, and OpenAPI regenerates — all in one
  commit.
- **Phase 1, single-node.** No multi-region / HA concerns. The
  allocator memo is local; `get` is an in-memory `BTreeMap` lookup.

## Decision

### 1. New describe-wire enum — `DescribeSpecOutput`

A new tagged enum lives in `overdrive-core::api::describe` (new module,
sibling of `api::submit`). Dep-graph leaf so the CLI, tests, and any
future SDK crate depend on the describe wire shape without pulling in
`overdrive-control-plane`:

```rust
// crates/overdrive-core/src/api/describe.rs

/// HTTP/JSON wire-shape for the `GET /v1/jobs/{id}` describe RESPONSE.
/// Per ADR-0064 this is the describe-side member of the type-family
/// universe — the read-only output projection distinct from the
/// submit-side `SubmitSpecInput` (ADR-0051) because it surfaces the
/// platform-issued Service VIP, which the submit shape structurally
/// forbids (ADR-0049 § 5).
///
/// Tagged JSON via `#[serde(tag = "kind", rename_all = "snake_case")]`
/// — the `kind` field discriminates `job` / `service` / `schedule`.
///
/// `utoipa::ToSchema` renders this as a `oneOf`-discriminated schema
/// in the generated OpenAPI document per OQ-1.
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DescribeSpecOutput {
    Job(JobSpecInput),
    Service(ServiceSpecOutput),
    Schedule(ScheduleSpecOutput),
}
```

**Why `JobSpecInput` is reused verbatim as the Job describe arm**
(mirrors ADR-0051 OQ-3): the Job describe path has no derived
platform-owned field to surface (no VIP). Today's handler already
renders `JobSpecInput::from(&job)`; wrapping that in
`DescribeSpecOutput::Job(JobSpecInput::from(&job))` is the minimal
change — zero behavioural change to the Job describe path beyond the
enum wrapper. The existing `From<&Job> for JobSpecInput` impl
(`aggregate/mod.rs:896`) IS the Job render path. No new Job describe
type is introduced.

**Why `deny_unknown_fields` is NOT applied** (unlike `SubmitSpecInput`):
this is a server → client response shape, not a client → server
request. `deny_unknown_fields` is a defense against clients smuggling
fields *in*; on a response, the server is the sole author. Omitting it
keeps the describe shape forward-tolerant for clients deserialising a
newer server's response (additive fields are ignored, not rejected) —
the correct posture for a response wire per OpenAPI semver discipline.

### 2. Per-kind describe payloads

#### Job arm — `JobSpecInput`, reused verbatim

Per OQ-1 / the Job-reuse rationale above. No new type. The describe Job
arm is `DescribeSpecOutput::Job(JobSpecInput)` where the inner value is
produced by the existing `From<&Job>` impl.

#### Service arm — `ServiceSpecOutput` (new)

Mirrors the **full** `ServiceSpecInput` operator-input field set —
`id`, `replicas`, `resources`, `driver`, `listeners`, and the three
probe vectors `startup_probes` / `readiness_probes` / `liveness_probes`
added to the submit shape by ADR-0057/0058 — PLUS a **required** `vip:
ServiceVip` field. The VIP is the platform-issued address surfaced
read-only:

```rust
/// HTTP/JSON wire-shape for a Service describe RESPONSE arm.
///
/// Mirrors the full Service submit shape (`id`, `replicas`,
/// `resources`, `driver`, `listeners`, and the `startup_probes` /
/// `readiness_probes` / `liveness_probes` vectors added by
/// ADR-0057/0058) PLUS the platform-issued `vip` — the field
/// `ServiceSpecInput` structurally cannot carry (ADR-0049 § 5). The
/// `vip` is REQUIRED per OQ-4: a persisted-and-describable Service
/// always has an allocated VIP (submit-time admission allocates before
/// the intent is written — ADR-0049 § 4). Absence is unrepresentable;
/// a missing allocator entry is an internal-invariant violation
/// surfaced as HTTP 500, never an `Option`.
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, utoipa::ToSchema)]
pub struct ServiceSpecOutput {
    pub id: String,
    pub replicas: u32,
    pub resources: ResourcesInput,
    #[serde(flatten)]
    pub driver: DriverInput,
    /// Operator-declared listeners, `(port, protocol)` only — no VIP
    /// per listener (ADR-0049 § 5a: one VIP per Service, surfaced once
    /// at the Service level, not per-listener).
    pub listeners: Vec<ListenerInput>,
    /// Operator-declared startup probes, projected from the persisted
    /// `ServiceV1` (ADR-0057/0058). Operator-observable today. Read-only
    /// on the describe wire: the operator declares these on submit via
    /// `[[health_check.startup]]`; describe reflects them back verbatim.
    pub startup_probes: Vec<ProbeDescriptor>,
    /// Operator-declared readiness probes, projected read-only from the
    /// persisted `ServiceV1` (ADR-0057/0058). The operator declares
    /// these on submit via `[[health_check.readiness]]`; describe
    /// reflects them back verbatim so the Service's probes round-trip.
    pub readiness_probes: Vec<ProbeDescriptor>,
    /// Operator-declared liveness probes, projected read-only from the
    /// persisted `ServiceV1` (ADR-0057/0058). The operator declares
    /// these on submit via `[[health_check.liveness]]`; describe
    /// reflects them back verbatim so the Service's probes round-trip.
    pub liveness_probes: Vec<ProbeDescriptor>,
    /// The platform-issued Service VIP. REQUIRED — serialised as a
    /// dotted-quad string (the `ServiceVip` newtype's `Display`).
    /// Read-only: the operator never sets this on submit; the platform
    /// assigns it via `ServiceVipAllocator` (ADR-0049).
    pub vip: ServiceVip,
}
```

`ServiceVip` is the canonical newtype at `overdrive-core::id::ServiceVip`
(wrapping `Ipv4Addr` per ADR-0049 § 2), which already implements
`Display` / `FromStr` / `Serialize` / `Deserialize` (newtype
completeness). On the wire it renders as a dotted-quad string
(`"10.96.0.7"`). `ListenerInput` and `ResourcesInput` / `DriverInput`
are reused from `api::submit` / the existing wire surface — no
duplication.

#### Schedule arm — `ScheduleSpecOutput` (new, RED scaffold render path)

Per OQ-5. Lands from day one to keep the enum exhaustive; the
`ScheduleV1::to_describe()` render constructor is a `todo!("RED
scaffold: ...")` because Schedule submit is itself unrealised
(`ScheduleV1::from_submit` is a RED scaffold). A Schedule cannot be
persisted yet, so `WorkloadIntent::Schedule` is unreachable at the
describe handler in Phase 1 — the handler returns a structured
rejection on that variant (parallel to the submit handler's Schedule
rejection), and the `to_describe()` body is never exercised:

```rust
/// HTTP/JSON wire-shape for a Schedule describe RESPONSE arm. The
/// per-fire instance is a Job; the schedule adds the cron expression.
/// No VIP (a Schedule is not a Service).
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, utoipa::ToSchema)]
pub struct ScheduleSpecOutput {
    pub id: String,
    pub job: JobSpecInput,
    pub cron_expr: String,
}
```

### 3. Render constructors — `to_describe` family on the intent payloads

Validation lives on the submit side (`from_submit`); rendering lives
here as the inverse. Per-kind constructors on the `WorkloadIntent`
payloads, mirroring the `from_submit` family:

```rust
impl JobV1 {
    /// Project a persisted `JobV1` onto its describe-wire shape. The
    /// Job arm carries no platform-derived field, so this delegates to
    /// the existing `From<&Job>` impl. (`Job = JobV1`.)
    pub fn to_describe(&self) -> JobSpecInput { JobSpecInput::from(self) }
}

impl ServiceV1 {
    /// Project a persisted `ServiceV1` plus its platform-issued VIP
    /// onto the describe-wire shape. The VIP is passed in by the
    /// handler after a read-only `allocator.get(&spec_digest)` — the
    /// allocator memo is the source of truth (ADR-0049 § 5a), so the
    /// VIP is NOT read from the spec (the spec carries no VIP).
    pub fn to_describe(&self, vip: ServiceVip) -> ServiceSpecOutput { ... }
}

impl ScheduleV1 {
    /// RED scaffold per `.claude/rules/testing.md` §
    /// "Production-side scaffolds": Schedule describe is unreachable in
    /// Phase 1 (no Schedule can be persisted — `from_submit` is itself
    /// a scaffold). Lands GREEN when the Schedule submit path ships.
    #[expect(clippy::todo, reason = "RED scaffold — lands with Schedule submit per OQ-5")]
    pub fn to_describe(&self) -> ScheduleSpecOutput {
        todo!("RED scaffold: ScheduleV1::to_describe lands with the Schedule submit path per ADR-0064 OQ-5")
    }
}
```

**Why the VIP is a parameter on `ServiceV1::to_describe(vip)`, not read
inside it**: the allocator handle lives in `AppState`
(`overdrive-control-plane`); `ServiceV1` lives in `overdrive-core` and
must not depend on the control-plane crate. The handler performs the
read-only `get` and passes the resolved `ServiceVip` in. This keeps the
render constructor pure and the dependency direction correct (core does
not reach into the control plane).

### 4. Handler dispatch — read-only VIP retrieval + exhaustive match

The `describe_workload` handler replaces the HTTP 400 `let-else`
rejection (lines 628-635) with an exhaustive match over the intent
variant:

```rust
let spec = match intent {
    WorkloadIntent::Job(job) => DescribeSpecOutput::Job(job.to_describe()),
    WorkloadIntent::Service(svc) => {
        // OQ-7: READ-ONLY get; never allocate/release at describe.
        // OQ-4: a persisted Service ALWAYS has a VIP; None is an
        //       internal-invariant violation → HTTP 500.
        let digest_bytes: [u8; 32] = *spec_digest_hash.as_bytes();
        let vip = state
            .allocator
            .lock()
            .await
            .get(&digest_bytes)
            .ok_or(ControlPlaneError::ServiceVipMissing { /* spec_digest */ })?;
        DescribeSpecOutput::Service(svc.to_describe(vip))
    }
    WorkloadIntent::Schedule(_) => {
        // Phase 1: Schedule cannot be persisted (submit is a scaffold).
        // Structured rejection mirrors the submit handler.
        return Err(ControlPlaneError::Validation {
            field: Some("id".to_owned()),
            message: "describe is not available for Schedule workloads in Phase 1 (Schedule submit is unrealised)".to_owned(),
        });
    }
};
Ok(Json(api::WorkloadDescription { spec, spec_digest }))
```

Notes:
- `spec_digest_hash` is already computed in the handler (it produces
  the top-level `spec_digest` string). The Service arm reuses
  `*spec_digest_hash.as_bytes()` as the `ServiceSpecDigest` ([u8; 32])
  allocator key — the same digest-bytes the submit path keys on
  (`handlers.rs:324`). No second digest computation.
- `state.allocator` is `Arc<tokio::sync::Mutex<PersistentServiceVipAllocator>>`;
  describe takes the lock briefly and calls the non-mutating `get`. The
  lock is dropped before `to_describe` runs (clone the `ServiceVip` out
  — it is `Copy`).
- **New typed error variant** `ControlPlaneError::ServiceVipMissing`
  per OQ-4. Maps to HTTP 500 (it is an internal-invariant violation —
  a persisted Service with no allocator entry means the
  submit-time-allocate / boot-rebuild invariant was broken). Carries
  the `spec_digest` for diagnostics. Per
  `.claude/rules/development.md` § "Errors → Never flatten a typed
  error to `Internal(String)`", this is a dedicated variant, not
  `ControlPlaneError::internal(...)`.

### 5. OpenAPI shape

`DescribeSpecOutput` renders as a `oneOf`-discriminated JSON object,
identical in form to ADR-0051 § 5's `SubmitSpecInput` rendering:

```yaml
DescribeSpecOutput:
  oneOf:
    - $ref: '#/components/schemas/DescribeSpecOutput_Job'
    - $ref: '#/components/schemas/DescribeSpecOutput_Service'
    - $ref: '#/components/schemas/DescribeSpecOutput_Schedule'
  discriminator:
    propertyName: kind
    mapping:
      job:      '#/components/schemas/DescribeSpecOutput_Job'
      service:  '#/components/schemas/DescribeSpecOutput_Service'
      schedule: '#/components/schemas/DescribeSpecOutput_Schedule'
```

The `WorkloadDescription` schema regenerates with `spec:
DescribeSpecOutput` in the same single-cut commit per ADR-0009 /
`cargo openapi-gen`. The new per-variant schemas
(`DescribeSpecOutput_Service` carrying `vip`, `DescribeSpecOutput_Schedule`)
plus `ServiceSpecOutput` / `ScheduleSpecOutput` are registered
explicitly in the `components(schemas(...))` list at
`api.rs:356-372` alongside the existing `WorkloadDescription` entry.

### 6. Single-cut migration plan (per `feedback_single_cut_greenfield_migrations.md`)

In one atomic commit:

1. **New types land** at `crates/overdrive-core/src/api/describe.rs`:
   `DescribeSpecOutput`, `ServiceSpecOutput`, `ScheduleSpecOutput`.
   Module declared in `crates/overdrive-core/src/api/mod.rs`
   (`pub mod describe;` + re-export).
2. **`WorkloadDescription.spec` field type changes** from
   `JobSpecInput` to `DescribeSpecOutput` at `api.rs:155`. The utoipa
   `components(schemas(...))` list (`api.rs:356-372`) gains the new
   describe wire types.
3. **Render constructors land** on the intent payloads:
   `JobV1::to_describe()` (delegates to existing `From<&Job>`),
   `ServiceV1::to_describe(vip)` (new), `ScheduleV1::to_describe()`
   (RED scaffold).
4. **`describe_workload` handler** (handlers.rs:581-638) replaces the
   `let WorkloadIntent::Job(job) else { HTTP 400 }` rejection
   (lines 628-635) and the `#183` pointer comment (lines 623-627) with
   the exhaustive match in § 4. The read-only `allocator.get` is added
   for the Service arm.
5. **New `ControlPlaneError::ServiceVipMissing` variant** lands with an
   HTTP-500 mapping in the error-mapping module (ADR-0015).
6. **Every `describe_workload` consumer updates** — the CLI describe
   command and all tests/fixtures that unwrap `description.spec` as
   `JobSpecInput` are rewrapped to match on / construct
   `DescribeSpecOutput::Job(...)`. Estimated cascade: see § Consequences
   → "Consumer cascade".
7. **OpenAPI regenerate** via `cargo openapi-gen`; the golden
   `api/openapi.yaml` re-emits with the new schema; the OpenAPI gate
   acceptance test (`tests/integration/openapi_gate.rs`) is updated to
   the new golden.

## Consequences

### Positive

- **The platform-issued VIP is finally surfaced to operators** —
  closing #183's core requirement. An operator running `overdrive`
  describe on a Service learns the address the platform assigned, which
  was previously unreachable (HTTP 400).
- **Submit ↔ describe cadences stay decoupled.** The describe-only VIP
  field lives on `DescribeSpecOutput`, never on `SubmitSpecInput`. A
  future change to either wire surface does not force the other —
  Pattern C preserved on the describe boundary (the same property
  ADR-0051 bought on the submit boundary).
- **Absence of a VIP is unrepresentable.** `ServiceSpecOutput.vip:
  ServiceVip` (required) means a Service describe response cannot exist
  without a VIP; the only failure mode is an explicit HTTP 500 naming
  the broken invariant, never a silent empty string. Type-driven design
  per `.claude/rules/development.md`.
- **No allocator state mutation at describe.** Read-only `get` reuse;
  describe is idempotent and side-effect-free. The existing read-only
  method means OQ-7 adds zero allocator surface.
- **VIP is recomputed on read, never persisted on the response.**
  Persist-inputs-not-derived-state preserved: the allocator memo is the
  source of truth (ADR-0049 § 5a); describe projects it per-request.
- **Exhaustive enum from day one.** The `DescribeSpecOutput` match in
  the handler is compiler-enforced exhaustive; when the Schedule submit
  path lands, the describe Schedule arm cannot be silently forgotten.
- **Minimal Job-path change.** The Job describe arm reuses
  `JobSpecInput` and the existing `From<&Job>` impl — no regression to
  the realised Job describe path beyond the enum wrapper.

### Negative

- **A fourth type family adds cognitive load.** The Rust universe now
  carries `WorkloadSpec` / `SubmitSpecInput` / `WorkloadIntent` /
  `DescribeSpecOutput`. Mitigated by: (a) ADR-0047 / 0050 / 0051 / 0064
  as a coherent quartet of SSOTs; (b) the four-family table in
  § "Context"; (c) module placement signalling direction
  (`api::submit` = request, `api::describe` = response).
- **`JobSpecInput`'s module location remains incoherent** with the
  family model (it lives under `aggregate::` but is a wire type used by
  both submit and describe). This is the same pre-existing debt ADR-0051
  § "Future Work" tracks; this ADR does not worsen it (it reuses the
  type in place) and does not fix it (the relocation cascade is out of
  scope). See § "Deferred / surfaced for user decision".
- **Boundary functions multiply.** The `to_describe` family is the
  inverse of `from_submit`; six projection functions now exist where a
  one-family design would have one. Each is mechanical and well-typed.

### Neutral

- **rkyv discipline is unchanged.** The describe wire is JSON-only;
  ADR-0048's envelope discipline continues to govern the intent layer
  only.
- **`spec_digest` keying is unchanged.** Describe reuses the same
  `[u8; 32]` digest-bytes the submit path keys the allocator on. The
  wire layer never participates in content-addressing.

### Consumer cascade

The migration touches `WorkloadDescription` / `description.spec`
consumers. Estimated cascade (to be confirmed by the crafter's PREPARE
grep at DELIVER time): **~6–10 files** — materially smaller than
ADR-0051's ~10-file submit cascade because describe has far fewer
construction/consumption sites than submit:

- `crates/overdrive-control-plane/src/api.rs` — `WorkloadDescription`
  struct + utoipa schema list (1).
- `crates/overdrive-control-plane/src/handlers.rs` — `describe_workload`
  handler (1).
- `crates/overdrive-core/src/api/{mod,describe}.rs` — new module +
  re-export (2, mostly new files).
- `crates/overdrive-core/src/aggregate/mod.rs` — three `to_describe`
  impls (1).
- `crates/overdrive-control-plane/src/error.rs` (or the error-mapping
  site) — `ServiceVipMissing` variant + HTTP-500 mapping (1).
- The CLI describe command + its response-parsing site (1–2).
- Describe-path tests / fixtures that unwrap `description.spec` as
  `JobSpecInput` (1–3, depending on how many integration tests assert
  on the describe response shape).
- OpenAPI golden `api/openapi.yaml` + the OpenAPI gate test (2).

The precise count is a DELIVER PREPARE-pass concern; the load-bearing
property is that the cascade is bounded and single-cut.

## Alternatives Considered

### Alt-A — Reuse `SubmitSpecInput` for the describe response (REJECTED — OQ-1 loser)

Type `WorkloadDescription.spec: SubmitSpecInput` and echo the persisted
intent back through the submit wire shape (the original ADR-0051 § 1
boundary note: "`WorkloadIntent → SubmitSpecInput` — describe echoes
back").

**Rejected.** Two independent reasons:

1. **`SubmitSpecInput` structurally cannot carry the VIP.** Its Service
   arm (`ServiceSpecInput`) has no `vip` field and is
   `deny_unknown_fields` — surfacing the platform-issued VIP (the core
   #183 requirement) is impossible without modifying the *submit* shape,
   which would re-admit operator-supplied VIPs at the type level (the
   exact thing ADR-0049 § 5 made unrepresentable).
2. **It re-couples describe ↔ submit evolution cadence.** A
   describe-only field would force a change on the submit wire, and any
   future submit-side change would ripple into describe — the Pattern-C
   coupling ADR-0051 was created to break. A distinct `DescribeSpecOutput`
   keeps the two boundaries independent.

The user explicitly chose OQ-1 = distinct `DescribeSpecOutput`. This
ADR amends ADR-0051 § 1's boundary note accordingly (see ADR-0051
§ "Amendment (2026-06-06)").

### Alt-B — `Option<ServiceVip>` on the Service describe arm (REJECTED — OQ-4 loser)

Type the Service arm's VIP as `vip: Option<ServiceVip>`, rendering
`None` when the allocator has no entry.

**Rejected.** A persisted-and-describable Service ALWAYS has an
allocated VIP — submit-time admission allocates the VIP before the
intent is written (ADR-0049 § 4), and the boot rebuild re-seeds the
allocator memo from the intent SSOT (ADR-0049 § 8). An `Option` would
make a structurally-impossible state representable, and every consumer
would have to handle a `None` that cannot legitimately occur — the
`if vip.is_none()` branch is dead code that invites silent
mis-rendering ("vip: " with an empty value). Per
`.claude/rules/development.md` § "Type-driven design → make invalid
states unrepresentable", the field is required; a genuinely-missing
allocator entry is an internal-invariant violation surfaced as HTTP 500
(`ServiceVipMissing`), not a `None` the client must defensively handle.
The user chose OQ-4 = REQUIRED.

### Alt-C — Idempotent `allocate` at describe time (REJECTED — OQ-7 loser)

If the allocator has no entry for the Service's digest, call the
mutating `allocate(&mut self)` at describe time (idempotent — memo-hit
returns the existing VIP; memo-miss allocates a fresh one).

**Rejected.** Describe is a read (GET); it must not mutate platform
state. Two harms:
1. **A describe of a Service whose allocator entry was lost would
   silently allocate a new VIP** — masking the broken invariant
   (ADR-0049 § 8's boot rebuild should have re-seeded it) instead of
   surfacing it. The HTTP 500 (`ServiceVipMissing`) is the honest
   signal; a silent re-allocate is a cover-up.
2. **A GET that writes to the `IntentStore`** (the allocator's
   write-through fsync) violates the read/write separation and the
   HTTP-method contract; it would also make describe non-idempotent and
   add an fsync to a hot read path.
The read-only `get` already exists; there is no reason to reach for the
mutating path. The user chose OQ-7 = read-only `get`.

### Alt-D — Defer the Schedule describe arm (REJECTED — OQ-5 loser)

Ship `DescribeSpecOutput` with `{ Job, Service }` only; add `Schedule`
when the Schedule submit path lands.

**Rejected.** Same reasoning as ADR-0051 OQ-5: deferring Schedule risks
a second structural finding when the Schedule path lands, and a
non-exhaustive enum lets a future handler silently miss the Schedule
case. The cost of the third arm now is one type definition + one
`oneOf` variant + one `todo!` render scaffold — negligible. The
exhaustive enum is the compiler's structural guard. The user chose
OQ-5 = land all three arms.

## References

### ADRs

- ADR-0051 — Wire-side `SubmitSpecInput` (the submit-side mirror; this
  ADR amends its § 1 describe-echo boundary note).
- ADR-0050 — Intent-side `WorkloadIntent` enum (the payloads
  `to_describe` reads from).
- ADR-0049 — Platform-issued Service VIP allocator (§ 5a the allocator
  memo is the VIP source of truth; the read-only `get`).
- ADR-0048 — rkyv versioned envelope (governs the intent layer only;
  does NOT apply to this ADR's wire layer).
- ADR-0047 — Workload-kind discriminator (parser-side `WorkloadSpec`).
- ADR-0020 — Describe response shape `{spec, spec_digest}`.
- ADR-0015 — HTTP error mapping (`ServiceVipMissing` → 500).
- ADR-0014 — Shared CLI/server request/response types.
- ADR-0009 — OpenAPI schema generation via utoipa.

### Project rules

- `.claude/rules/development.md` § "Type-driven design" — make invalid
  states unrepresentable (the required `vip`).
- `.claude/rules/development.md` § "Persist inputs, not derived state" —
  the VIP is read at describe time from the allocator memo, never
  persisted on the response shape.
- `.claude/rules/development.md` § "Errors → Never flatten a typed
  error to `Internal(String)`" — `ServiceVipMissing` is a dedicated
  variant.
- `.claude/rules/testing.md` § "Production-side scaffolds" — the
  Schedule `to_describe` RED scaffold shape.
- `feedback_single_cut_greenfield_migrations.md` — one atomic commit;
  no aliases, no shims.

### Issue

- [overdrive-sh/overdrive#183](https://github.com/overdrive-sh/overdrive/issues/183)
  — WorkloadDescription Service-arm wire-shape widening.

### Sign-off

- User authorisation of OQ-1 / OQ-4 / OQ-5 / OQ-7: **2026-06-06**
  (dispatch message, this session). ADR Status set to Accepted in the
  same edit.
