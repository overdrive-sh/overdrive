# ADR-0051 — Wire-side `SubmitSpecInput` as third distinct type family (extends ADR-0050)

## Status

**Accepted (2026-05-15)**. User signed off on OQ-1 through OQ-8
verbatim as listed below; the recommendations have been converted to
binding decisions.

Decision-maker: Morgan (proposing); user sign-off recorded 2026-05-15.
Driven by: DELIVER wave of
`docs/feature/service-vip-allocator/` — currently stalled at step
**02-03b** on checkpoint commit `6b880e97` (no production code changes
— DES protocol state only). The crafter's PREPARE pass surfaced a
structural blocker: widening `SubmitWorkloadRequest.spec: JobSpecInput
→ WorkloadSpecInput` (the parser-side TOML-shape enum) silently
conflates the HTTP/JSON wire layer with the TOML parser layer, which
ADR-0050 separated from the persisted intent layer. The user
authorized **Option B**: introduce a NEW wire-shape enum
`SubmitSpecInput` rather than collapse the HTTP/JSON wire into
`WorkloadSpec`. This ADR is the SSOT for that decision.

**Open-question resolution (confirmed 2026-05-15)**:

- **OQ-1 (module placement)** — **confirmed**:
  `crates/overdrive-core/src/api/submit.rs` (new module). Dep-graph
  leaf so the CLI, tests, and any future SDK crate can depend on the
  wire shape without pulling in `overdrive-control-plane`; distinct
  from `aggregate::` so the parser-side and wire-side type families
  do not appear co-located (mirrors the `WorkloadSpec` vs
  `WorkloadIntent` separation ADR-0050 enforces).
- **OQ-2 (naming)** — **confirmed**: `SubmitSpecInput`. Pairs
  naturally with the existing `SubmitWorkloadRequest.spec` field and
  ends in `*Input` the same way today's `JobSpecInput` /
  `ServiceSpecInput` / `ScheduleSpecInput` do — readers fluent in
  the workspace pattern map the suffix instinctively. The `Wire*`
  prefix alternative is rejected as cosmetically ugly with no extra
  information.
- **OQ-3 (Job arm definition)** — **confirmed**: `JobSpecInput` is
  kept verbatim as the inner type of
  `SubmitSpecInput::Job(JobSpecInput)`. Additive, zero changes to
  today's Job wire shape — no cascade through existing Job submit
  tests, no regen of existing OpenAPI clients for the Job path.
  Aligns with single-cut greenfield discipline: the Service /
  Schedule arms are the only new wire surface in this slice.
- **OQ-4 (Service arm shape)** — **confirmed**: `ServiceSpecInput
  { id: String, replicas: u32, resources: ResourcesInput, driver:
  DriverInput, listeners: Vec<ListenerInput> }` where
  `ListenerInput { port: u16, protocol: String }` (NO `vip` per
  ADR-0049 § 5 and S-VIP-13 / S-VIP-14). Mirrors `JobSpecInput`
  field-for-field plus `listeners` and minus the run-to-completion
  semantics.
- **OQ-5 (Schedule arm in Phase 1 or deferred)** — **confirmed**:
  `ScheduleSpecInput` lands from day one. Keeps the enum
  exhaustive, avoids a second structural finding when Schedule
  lands, and pairs symmetrically with
  `WorkloadIntent::Schedule(ScheduleV1)` already Accepted in
  ADR-0050. The constructor lands as a `todo!("RED scaffold: ...")`
  stub per `.claude/rules/testing.md`; the cost of the third arm is
  ~20 lines of type definition + one OpenAPI variant + one
  validating constructor stub — negligible compared to the risk
  of repeating the 02-03b structural-finding cycle.
- **OQ-6 (validation boundary)** — **confirmed**: validation lives
  at the wire → intent boundary inside per-kind validating
  constructors (`JobV1::from_submit(&JobSpecInput)`,
  `ServiceV1::from_submit(&ServiceSpecInput)`,
  `ScheduleV1::from_submit(&ScheduleSpecInput)`). Mirrors
  `Job::from_spec` (ADR-0047) and `WorkloadIntent` per-kind
  validating constructors (ADR-0050 § 4). A central
  `SubmitSpecInput::validate` method is rejected — it would
  duplicate the per-kind logic the intent-side constructors
  already own.
- **OQ-7 (OpenAPI schema shape)** — **confirmed**:
  `oneOf`-discriminated JSON object with a `kind: "job" |
  "service" | "schedule"` tag and per-variant fields. Standard
  `utoipa` default rendering for tagged enums; aligns with the
  existing `WorkloadKind` discriminator semantics on
  `SubmitWorkloadRequest`. Existing `SubmitWorkloadRequest` schema
  in `api/openapi.yaml` regenerates in the same single-cut commit
  per ADR-0009.
- **OQ-8 (single-cut migration plan for 02-03b)** — **confirmed**:
  `JobSpecInput` stays at its current module + name;
  `SubmitWorkloadRequest.spec` field type changes from
  `JobSpecInput` to `SubmitSpecInput`; existing call sites that
  construct `JobSpecInput` directly are wrapped in
  `SubmitSpecInput::Job(jsi)` at the construction point; the now-
  redundant `workload_kind: Option<String>` field on
  `SubmitWorkloadRequest` is **deleted** in the same commit
  (superseded by the inner `kind` tag on `SubmitSpecInput`).
  Estimated cascade: ~10 files vs ~86 for the original
  widen-to-`WorkloadSpecInput` misread. The lower cascade is the
  load-bearing reason the wire layer is a NEW additive type rather
  than a refactor of `JobSpecInput`.

Tags: phase-1, application-arch, wire-shape, parser-vs-wire-vs-intent
separation, single-cut-migration.

**Relates to**: ADR-0047 § 1 (Job is run-to-completion, NO
`replicas` on the parser side — the source of the wire-vs-parser
asymmetry that motivated this ADR); ADR-0048 (rkyv versioned
envelope, codec-on-typed-value — referenced by ADR-0050 for the
intent layer; ADR-0051's wire layer does **not** use rkyv and does
**not** need an envelope, see § "Constraints" below); ADR-0049
(platform-issued `ServiceVipAllocator`; § 5 amendments rejecting
`Listener.vip` at the parser level — the wire layer carries the same
prohibition); ADR-0050 (intent-side `WorkloadIntent` enum,
parser-vs-intent separation — this ADR extends ADR-0050 by adding
the wire layer as the third corner).

**SSOT**: this ADR is the SSOT for the wire-side HTTP/JSON shape of
workload submissions. The parser-side `WorkloadSpec` aggregate at
`crates/overdrive-core/src/aggregate/workload_spec.rs` remains owned
by ADR-0047. The intent-side `WorkloadIntent` aggregate at
`crates/overdrive-core/src/aggregate/mod.rs` remains owned by
ADR-0050.

## Context

### What exists today

- **Wire-side** (`crates/overdrive-control-plane/src/api.rs:51-52`):
  `SubmitWorkloadRequest.spec: JobSpecInput`. The `JobSpecInput` type
  lives at `crates/overdrive-core/src/aggregate/mod.rs:579` and
  carries `{ id: String, replicas: u32, resources: ResourcesInput,
  #[serde(flatten)] driver: DriverInput }` where `DriverInput` is a
  tagged enum (`Exec(ExecInput)` today; `MicroVm(...)`,
  `Wasm(...)` additively per ADR-0031 § 2). This is the operator-
  visible HTTP/JSON shape used by both the CLI (TOML → JSON → HTTP)
  and any future SDK / gRPC bridge.
- **Parser-side** (`crates/overdrive-core/src/aggregate/workload_spec.rs`):
  the kind-discriminated `WorkloadSpec` aggregate from ADR-0047 §
  1, with `WorkloadSpec::Job(JobSpec)` whose `JobSpec` is `{ id,
  exec, resources }` — **no `replicas`** per ADR-0047 § 1 (Job is
  run-to-completion). TOML is the operator-typed encoding; the
  parser is custom (`WorkloadSpecInput::from_toml_str`) per
  ADR-0047 § 2.
- **Intent-side** (`crates/overdrive-core/src/aggregate/mod.rs`,
  per ADR-0050): the `WorkloadIntent` enum + `JobV1` / `ServiceV1`
  / `ScheduleV1` per-kind payloads. `JobV1` carries `replicas:
  NonZeroU32` (read by `alloc_status` as `replicas_desired` at
  `handlers.rs:744`).

The three layers are already two distinct type families (parser-side
vs intent-side, established by ADR-0050). The wire-side is currently
a Job-only type — `JobSpecInput` was named for the era when Job was
the only workload kind on the wire. The DELIVER wave for
`service-vip-allocator` needs to extend the wire layer to carry
Service (and Schedule) submissions; the question is **whether the
wire layer collapses into one of the existing type families or
becomes a third**.

### The structural finding (verbatim summary from checkpoint
commit `6b880e97`)

The roadmap directed widening `SubmitWorkloadRequest.spec:
JobSpecInput → WorkloadSpecInput` (the parser-side TOML-shape enum).
The crafter's PREPARE pass surfaced three findings that made this
unsafe:

1. **Migration cascade** — approximately 86 source/test paths
   construct or pattern-match `JobSpecInput` today, against the 5
   listed in the step's `files_to_modify` declaration. Single-cut
   on only the listed files leaves the workspace uncompilable. The
   crafter halted the slice before introducing the cascade.

2. **Wire-vs-parser semantic asymmetry on Job**:
   - `JobSpecInput` (wire today): `{ id, replicas: u32, resources,
     driver: DriverInput::Exec(ExecInput) }` — `replicas` is
     operator-visible; `driver` is a tagged enum
     (MicroVm/Wasm extensible per ADR-0031).
   - `WorkloadSpec::Job(JobSpec)` (parser-side, TOML-shape):
     `{ id, exec, resources }` — **NO `replicas`** (ADR-0047 § 1:
     Job is run-to-completion), **NO** tagged-driver enum (TOML
     section presence IS the dispatch).
   - `JobV1` (intent-side, preserved by ADR-0050 / 02-03a): still
     carries `replicas: NonZeroU32` (read by `alloc_status` as
     `replicas_desired` at `handlers.rs:744`).
   - Naïve widening of the wire to `WorkloadSpecInput` either
     **silently drops `replicas`** at the wire (regression — existing
     HTTP clients use the field) or **adds `replicas` to
     `WorkloadSpec::Job`** (contradicts ADR-0047 § 1 and re-opens the
     run-to-completion semantics).

3. **Pattern C violation** — ADR-0050 explicitly separated
   parser-side from intent-side (Pattern C: parsed-on-ingress,
   typed-on-disk). Collapsing the wire layer into the parser-side
   `WorkloadSpecInput` re-couples REST/JSON encoding to TOML-shape
   semantics — the inverse of what Pattern C established. Future
   evolution of the operator-facing TOML (a new optional field, a
   schema migration) would force every HTTP client to re-negotiate
   the wire — and vice versa.

The user reviewed the checkpoint and authorized **Option B**:
introduce a NEW wire-shape enum, additive over today's
`JobSpecInput`, rather than collapse the wire into either of the
existing type families.

### Constraints the decision must respect

- **Three distinct evolution surfaces** — HTTP/JSON (clients,
  OpenAPI consumers, future SDKs/gRPC), TOML (operators writing
  config files locally, CLI parsing), persisted rkyv (on-disk byte
  layout, reconciler reads). The three evolve on independent
  cadences: a new HTTP field need not change the on-disk format;
  an operator-facing TOML rename need not break HTTP clients; an
  on-disk envelope bump per ADR-0048 need not be operator-visible
  at all. **One type collapsing two of these surfaces freezes their
  cadences together** — exactly the coupling Pattern C avoids.

- **`replicas` on the wire is non-negotiable** — every existing HTTP
  client (CLI today, future SDKs) submits `replicas: u32` on Service
  and Job-replicated-mode bodies. The CLI's `submit_streaming_job`
  path constructs `JobSpecInput { replicas, ... }` today; removing
  the field at the wire layer is a regression. The wire-side Job
  arm therefore continues to carry `replicas: u32`; the parser-side
  Job arm continues to omit it per ADR-0047 § 1. The two are
  distinct types with distinct semantics.

- **rkyv does NOT apply at the wire layer** — `SubmitSpecInput` is
  JSON-over-HTTP, validated and projected onto `WorkloadIntent` at
  the admission boundary. The rkyv envelope discipline at ADR-0048
  governs on-disk bytes; HTTP/JSON evolves via serde + OpenAPI
  semver discipline (a separate concern). The wire-shape types are
  `serde::Serialize + serde::Deserialize + utoipa::ToSchema`; no
  `rkyv::Archive` derive, no envelope enum at this layer.

- **Listeners are wire-side too** — per ADR-0049 § 5 and
  S-VIP-13 / S-VIP-14, the operator does NOT supply a VIP. The
  wire-side `ListenerInput` carries `(port, protocol)` only, same
  as the parser-side `Listener` from
  `crates/overdrive-core/src/aggregate/workload_spec.rs:392-400`.
  Admission rejects any incoming JSON listener carrying a `vip`
  field via `#[serde(deny_unknown_fields)]` — same defense the
  parser layer applies for TOML.

- **Greenfield single-cut migrations**
  (`feedback_single_cut_greenfield_migrations.md`): no aliases, no
  shims, no grace periods. `SubmitWorkloadRequest.spec` flips from
  `JobSpecInput` to `SubmitSpecInput` in one commit; every direct
  `JobSpecInput` construction site in the workspace is wrapped in
  `SubmitSpecInput::Job(jsi)` at the construction point in the
  same commit. ~10-file cascade for this minimal-impact shape vs
  ~86 for the original misread.

## Decision

### 1. Three distinct type families for the same logical concept

The Rust universe now carries **three distinct type families** for
the same logical concept (a "workload"):

| Layer | Type | Stakeholder | Encoding | Module |
|---|---|---|---|---|
| TOML parser | `WorkloadSpec` / `WorkloadSpecInput` | operators writing config files | TOML | `overdrive-core::aggregate::workload_spec` (ADR-0047) |
| HTTP wire | `SubmitSpecInput` (NEW) | HTTP/JSON clients (CLI, future SDKs, future gRPC) | JSON | `overdrive-core::api::submit` (this ADR) |
| Persisted | `WorkloadIntent` | reconciler, `alloc_status`, recovery walk | rkyv envelope | `overdrive-core::aggregate` (ADR-0050) |

Mapping functions live at the boundaries:
- TOML → `WorkloadSpec` — CLI parses operator's TOML file.
- `WorkloadSpec` → `SubmitSpecInput` — CLI projects parsed TOML
  onto the wire shape before issuing HTTP; the projection adds
  `replicas` from the parsed body (Service / Job-replicated) and
  re-wraps `[exec]` as `DriverInput::Exec(ExecInput)`.
- `SubmitSpecInput` → `WorkloadIntent` — server-side admission,
  through per-kind validating constructors
  (`JobV1::from_submit`, `ServiceV1::from_submit`,
  `ScheduleV1::from_submit`).
- `WorkloadIntent` → `SubmitSpecInput` — `describe_workload`
  endpoint echoes the persisted intent back to the operator on
  GET; `JobV1 → JobSpecInput` reuses the existing `From<&Job>`
  impl at `aggregate/mod.rs:641`.

Each boundary is its own function; no single type spans more than
one layer.

### 2. Wire-shape enum — `SubmitSpecInput`

A new tagged enum lives in `overdrive-core::api::submit` (new
module). The Rust universe of three type families now has its
wire-side member:

```rust
// crates/overdrive-core/src/api/submit.rs

/// HTTP/JSON wire-shape for `POST /v1/workloads` (and the streaming
/// sibling). Per ADR-0051 this is the wire-side member of the
/// three-layer Rust universe — distinct from the parser-side
/// `WorkloadSpec` (TOML) and the persisted `WorkloadIntent` (rkyv).
///
/// Tagged JSON via `#[serde(tag = "kind", rename_all = "snake_case")]`
/// — the `kind` field discriminates `job` / `service` / `schedule`
/// and the per-variant fields populate the rest of the body.
///
/// `utoipa::ToSchema` renders this as a `oneOf`-discriminated
/// schema in the generated OpenAPI document per OQ-7.
///
/// Listener `vip` is structurally unrepresentable per ADR-0049 § 5;
/// `deny_unknown_fields` rejects any incoming JSON carrying it.
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, utoipa::ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case",
        deny_unknown_fields)]
pub enum SubmitSpecInput {
    Job(JobSpecInput),
    Service(ServiceSpecInput),
    Schedule(ScheduleSpecInput),
}
```

### 3. Per-kind wire payloads

#### `JobSpecInput` — unchanged

Per OQ-3, today's `JobSpecInput` at
`crates/overdrive-core/src/aggregate/mod.rs:579` is preserved
verbatim as the inner type of `SubmitSpecInput::Job(JobSpecInput)`.
Same field set (`id`, `replicas`, `resources`, `driver`), same
derives, same module location. Zero changes to the existing Job
wire shape and zero cascade through existing Job submit tests.

The relocation question (should `JobSpecInput` move from
`aggregate::` to `api::submit::` alongside the new types?) is
explicitly deferred to a future cleanup — the type's current
location is incoherent with the three-family model but moving it
in this slice would re-introduce the 86-file cascade the wire-
layer-as-third-family design is specifically intended to avoid.

#### `ServiceSpecInput` — new

Per OQ-4 (confirmed). Mirrors `JobSpecInput`'s shape (operator-
visible `replicas`, tagged `driver` enum) plus a `listeners` array.
No `vip` field; admission rejects any incoming JSON carrying it via
`deny_unknown_fields`.

```rust
/// HTTP/JSON wire-shape for a Service submission. Mirrors
/// `JobSpecInput` plus operator-declared listeners.
///
/// Per ADR-0049 § 5 and ADR-0051 § 2 listeners carry `(port,
/// protocol)` only — NO operator-supplied VIP. The platform issues
/// VIPs via `ServiceVipAllocator` keyed by `WorkloadIntent::
/// Service(_).spec_digest()` after admission; the operator never
/// names a VIP, on the wire or in TOML.
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceSpecInput {
    pub id: String,
    pub replicas: u32,
    pub resources: ResourcesInput,
    #[serde(flatten)]
    pub driver: DriverInput,
    /// Operator-declared listeners in declaration order. Validated
    /// at admission inside `ServiceV1::from_submit`: at least one
    /// element; no two share `(port, protocol)`; protocol is
    /// `tcp` / `udp` only.
    pub listeners: Vec<ListenerInput>,
}

/// HTTP/JSON wire-shape for a single listener entry.
///
/// Distinct from the parser-side `Listener` newtype at
/// `crate::aggregate::workload_spec::Listener` only in encoding:
/// the parser side carries `port: NonZeroU16` and `protocol: Proto`
/// after TOML decoding; the wire side carries `port: u16` and
/// `protocol: String` for JSON deserialise tolerance, with
/// validation deferred to `ServiceV1::from_submit`.
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListenerInput {
    #[schema(value_type = u16, minimum = 1, maximum = 65535)]
    pub port: u16,
    pub protocol: String,
}
```

**Why `port: u16` on the wire, not `NonZeroU16`**: JSON has no
non-zero integer type; `u16` is the natural deserialise target.
Validation (`port != 0`) lives in `ServiceV1::from_submit`
alongside duplicate detection and protocol-string parsing. The
parser-side `Listener` newtype's `port: NonZeroU16` is unchanged
— the constraints from S-08-06 still fire, just at the wire →
intent boundary instead of at the TOML → parser boundary.

#### `ScheduleSpecInput` — new

Per OQ-5 (confirmed). The Schedule arm lands from day one rather
than deferring; the validating constructor lands as a
`todo!("RED scaffold: ...")` stub. The cost is ~20 lines and the
benefit is an exhaustive enum that cannot surface another
structural finding when the Schedule streaming endpoint lands.

```rust
/// HTTP/JSON wire-shape for a Schedule submission. The per-fire
/// instance is a Job per ADR-0047 § 1 / ADR-0050 § 2; the schedule
/// adds the cron expression.
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSpecInput {
    pub id: String,
    /// Inner job specification fired on each cron tick. Same wire
    /// shape standalone Jobs use — operators write the schedule
    /// body and the inner job body in the same JSON document.
    pub job: JobSpecInput,
    /// Cron expression. String-shaped on the wire; validated and
    /// projected onto `CronExpr` inside `ScheduleV1::from_submit`.
    pub cron_expr: String,
}
```

### 4. Validation boundary — per-kind validating constructors

Per OQ-6, validation lives at the wire → intent boundary inside
per-kind validating constructors on `WorkloadIntent`:

```rust
impl JobV1 {
    /// Validate and project a wire-side `JobSpecInput` onto a
    /// persisted-shape `JobV1`. Fires the same validation rules
    /// `Job::from_spec` fires today; this method's existence
    /// renames the entry point (`from_spec` was the TOML era).
    pub fn from_submit(input: &JobSpecInput) -> Result<Self, ParseError> { ... }
}

impl ServiceV1 {
    /// Validate and project a wire-side `ServiceSpecInput` onto a
    /// persisted-shape `ServiceV1`. Fires:
    ///   * `id` non-empty after trim → `WorkloadId::from_str`.
    ///   * `replicas > 0` → `NonZeroU32`.
    ///   * `resources.memory_bytes != 0` (existing rule).
    ///   * driver validation (existing `WorkloadDriver` projection).
    ///   * `listeners.len() >= 1`.
    ///   * No two listeners share `(port, protocol)`.
    ///   * `port != 0` per listener.
    ///   * `protocol` parses to `Proto` (case-insensitive `tcp`/`udp`).
    pub fn from_submit(input: &ServiceSpecInput) -> Result<Self, ParseError> { ... }
}

impl ScheduleV1 {
    /// Validate and project a wire-side `ScheduleSpecInput` onto a
    /// persisted-shape `ScheduleV1`. Fires `JobV1::from_submit` on
    /// the inner job + `CronExpr` parse on `cron_expr`.
    pub fn from_submit(input: &ScheduleSpecInput) -> Result<Self, ParseError> { ... }
}
```

The handler dispatches on the enum variant:

```rust
let intent = match submit.spec {
    SubmitSpecInput::Job(jsi)      => WorkloadIntent::Job(JobV1::from_submit(&jsi)?),
    SubmitSpecInput::Service(ssi)  => WorkloadIntent::Service(ServiceV1::from_submit(&ssi)?),
    SubmitSpecInput::Schedule(sci) => WorkloadIntent::Schedule(ScheduleV1::from_submit(&sci)?),
};
```

A centralized `SubmitSpecInput::validate(&self) -> Result<(),
ParseError>` was considered (OQ-6 alt) and rejected — it would
duplicate the per-kind logic the intent-side constructors already
own, and re-introduce the parser/intent erosion ADR-0050 fenced
off.

### 5. OpenAPI shape

Per OQ-7, `SubmitSpecInput` renders in the generated OpenAPI
document as a `oneOf`-discriminated JSON object:

```yaml
SubmitSpecInput:
  oneOf:
    - $ref: '#/components/schemas/SubmitSpecInput_Job'
    - $ref: '#/components/schemas/SubmitSpecInput_Service'
    - $ref: '#/components/schemas/SubmitSpecInput_Schedule'
  discriminator:
    propertyName: kind
    mapping:
      job:      '#/components/schemas/SubmitSpecInput_Job'
      service:  '#/components/schemas/SubmitSpecInput_Service'
      schedule: '#/components/schemas/SubmitSpecInput_Schedule'
```

The existing `SubmitWorkloadRequest` schema regenerates in the
same single-cut commit per ADR-0009 / `cargo openapi-gen`.

### 6. Single-cut migration plan (per OQ-8)

In one atomic commit:

1. **New types land** at `crates/overdrive-core/src/api/submit.rs`:
   `SubmitSpecInput`, `ServiceSpecInput`, `ScheduleSpecInput`,
   `ListenerInput`. Module declared in
   `crates/overdrive-core/src/lib.rs`.

2. **`SubmitWorkloadRequest.spec` field type changes** from
   `JobSpecInput` to `SubmitSpecInput` at
   `crates/overdrive-control-plane/src/api.rs:52`. `utoipa` schema
   list updated to reference the new wire types.

3. **Per-kind validating constructors land** on `WorkloadIntent`
   payloads:
   - `JobV1::from_submit(&JobSpecInput)` — renames today's
     `Job::from_spec` entry point.
   - `ServiceV1::from_submit(&ServiceSpecInput)` — new.
   - `ScheduleV1::from_submit(&ScheduleSpecInput)` — new.

4. **`submit_workload` handler** dispatches on
   `SubmitSpecInput` variants and projects onto `WorkloadIntent`.
   The `workload_kind: Option<String>` field on
   `SubmitWorkloadRequest` is deleted in the same commit — the
   `kind` tag inside `SubmitSpecInput` supersedes it. Clients that
   relied on `workload_kind` were already on `kind = "job"` (the
   only realised value today); the new tag carries the same
   semantics with one less duplicate field.

5. **All `JobSpecInput` construction sites** in the workspace
   (CLI, tests, fixtures) wrap their construction in
   `SubmitSpecInput::Job(jsi)` at the construction point.
   Estimated cascade: ~10 files vs ~86 for the original
   widen-to-`WorkloadSpecInput` misread.

6. **CLI TOML → wire projection** updates: the CLI's
   `submit_streaming_job` path projects parsed `WorkloadSpec`
   onto `SubmitSpecInput::Job(jsi)`; `submit_streaming_service`
   projects onto `SubmitSpecInput::Service(ssi)`;
   `submit_streaming_schedule` (when it lands) onto
   `SubmitSpecInput::Schedule(sci)`.

7. **OpenAPI regenerate** via `cargo openapi-gen`; the existing
   `api/openapi.yaml` re-emits with the new schema.

## Consequences

### Positive

- **Three layers separated in types**. Reviewers can no longer
  accidentally couple HTTP/JSON evolution to TOML evolution or to
  rkyv evolution. Each surface evolves on its own cadence with the
  Rust type system as the structural defense.
- **`replicas` preserved at the wire**. Existing HTTP clients are
  not regressed; the wire-side Job arm continues to accept
  `replicas: u32` despite ADR-0047 § 1's parser-side prohibition.
  The asymmetry is now explicit, not accidental.
- **Smaller migration cascade**. ~10-file cascade vs ~86. The
  difference is the load-bearing reason for the additive shape:
  wrapping existing `JobSpecInput` construction sites in
  `SubmitSpecInput::Job(_)` is mechanical; widening every
  construction site to `WorkloadSpecInput` would have required
  per-site shape adjustments.
- **OpenAPI clients regenerate cleanly**. The `oneOf`-discriminated
  schema is idiomatic; existing Job-only clients see a `kind:
  "job"` discriminator added on their requests and otherwise
  identical payloads.
- **Listener `vip` rejection is structural at every layer**.
  Parser-level: `deny_unknown_fields` on TOML `Listener` (ADR-0049
  § 5). Wire-level: `deny_unknown_fields` on `ListenerInput`.
  Intent-level: `ServiceV1.listeners: Vec<Listener>` carries no
  `vip` field. Three independent rejections; an attacker or a
  buggy client cannot smuggle a VIP through any of the three
  encodings.
- **Schedule arm in from day one**. The enum is exhaustive across
  every workload kind the platform supports; no second structural
  finding when the Schedule streaming endpoint lands.

### Negative

- **A third type family adds cognitive load** for new
  contributors. The Rust universe of `WorkloadSpec` / `WorkloadIntent`
  / `SubmitSpecInput` is more types than a single-family design
  would carry. Mitigated by: (a) ADR-0047 / ADR-0050 / ADR-0051 as
  a coherent trio of SSOTs; (b) the table in § "Three distinct
  type families" above; (c) module placement signaling layer
  (`aggregate::workload_spec` = parser, `aggregate::*` = intent,
  `api::submit` = wire).
- **`JobSpecInput`'s current module location is now incoherent**
  with the three-family model. The type lives under `aggregate::`
  but it is a wire-shape type. Moving it to `api::submit::` was
  considered as part of this slice (OQ-3 alt) and rejected — the
  86-file cascade the wire-as-third-family design is specifically
  intended to avoid. Tracked as Future Work § "JobSpecInput
  relocation cleanup"; not a blocker for 02-03b.
- **Boundary functions multiply**. TOML → parser, parser → wire,
  wire → intent, intent → wire (describe), parser → intent (CLI
  one-shot for non-streaming submit, if it exists) — five
  projection functions where a one-type-family design would have
  zero or one. Each projection is mechanical and well-typed; the
  compiler ensures none of them silently mis-projects a field.

### Neutral

- **rkyv discipline is unchanged**. The wire layer is JSON-only;
  ADR-0048's envelope discipline continues to govern the intent
  layer only.
- **Single-cut greenfield migration** semantics are preserved —
  no aliases, no shims, no grace periods. The slice that lands
  this ADR's types replaces all `JobSpecInput` direct construction
  with `SubmitSpecInput::Job(_)` in one commit.
- **`spec_digest` keying is unchanged**. The
  `ServiceVipAllocator` continues to key by
  `WorkloadIntent::Service(_).spec_digest()` per ADR-0049 / ADR-0050.
  The wire layer never participates in content-addressing — it is
  validated and discarded after projection onto the intent.

## Alternatives Considered

### Alt-A — Widen `SubmitWorkloadRequest.spec` to `WorkloadSpecInput` (REJECTED)

The original roadmap step direction: change
`SubmitWorkloadRequest.spec: JobSpecInput → WorkloadSpecInput`
(the parser-side TOML-shape enum).

**Rejected**: this is the structural finding the crafter halted
on. Three independent reasons:

- **`replicas` regression on the wire**. `WorkloadSpec::Job(JobSpec)`
  has no `replicas` field per ADR-0047 § 1; widening the wire to
  `WorkloadSpecInput` silently drops the operator-visible
  `replicas` field for Job submissions. Adding `replicas` back to
  `JobSpec` contradicts ADR-0047 § 1 and re-opens the
  run-to-completion semantics.
- **Pattern C violation**. ADR-0050 explicitly separated parser
  from intent. Collapsing the wire into the parser re-couples
  REST/JSON to TOML — the inverse of Pattern C. Future evolution
  of either surface would force the other.
- **86-file cascade**. The roadmap's `files_to_modify` listed 5
  paths; the actual touched-site count is ~86 (every direct
  `JobSpecInput` construction in CLI, tests, fixtures). Single-cut
  on only the listed files leaves the workspace uncompilable.

The user explicitly rejected this direction at the `6b880e97`
checkpoint and authorized Option B (this ADR). Recording for
completeness.

### Alt-B — Collapse the wire into the intent type (`WorkloadIntent`) (REJECTED)

`SubmitWorkloadRequest.spec: WorkloadIntent` — the wire layer is
the same type the persisted layer uses.

**Rejected** for the same Pattern C reason as Alt-A but on the
intent side: HTTP/JSON evolution would freeze on-disk rkyv
evolution. Every additive HTTP field would force an envelope
version bump per ADR-0048; every envelope version bump would
become operator-observable through the HTTP/OpenAPI contract.
The two layers must evolve independently — Pattern C applies to
both boundaries of the wire layer (parser-side and intent-side),
not just one.

Additionally: `WorkloadIntent` carries `WorkloadDriver` (intent
shape after `From<DriverInput>`); wire clients send `DriverInput`
(operator shape). Forcing wire clients to send the intent shape
would re-import validation concerns that belong at admission.

### Alt-C — `SubmitSpecInput` placed at `api.rs` next to `SubmitWorkloadRequest` (REJECTED — OQ-1)

Co-locate the new wire enum with the existing request shape at
`crates/overdrive-control-plane/src/api.rs`.

**Rejected** at OQ-1: the CLI and (future) SDK / test crates need
to construct `SubmitSpecInput` without pulling in the entire
`overdrive-control-plane` dependency graph. The crate is a leaf
in the dep graph; the wire types belong in a leaf module of
`overdrive-core` so any submitter — CLI, tests, future SDK,
integration suites — depends only on the wire surface.

### Alt-D — `WireWorkloadInput` naming (REJECTED — OQ-2)

`WireWorkloadInput` or `WorkloadSubmitInput` instead of
`SubmitSpecInput`.

**Rejected** at OQ-2: `SubmitSpecInput` pairs naturally with
`SubmitWorkloadRequest.spec` and follows the existing
`*SpecInput` suffix family (`JobSpecInput`, `ServiceSpecInput`,
`ScheduleSpecInput`). `WireWorkloadInput` is cosmetically uglier
and carries no extra information — every input shape in the
workspace is implicitly "wire" by virtue of being a serde-
deserialised request body.

### Alt-E — Refactor `JobSpecInput` to align with `ServiceSpecInput` (REJECTED — OQ-3)

Rename or reshape `JobSpecInput` for symmetry with the new
`ServiceSpecInput` and `ScheduleSpecInput`.

**Rejected** at OQ-3: the additive shape is the load-bearing
reason for the wire-as-third-family design. Reshaping
`JobSpecInput` would re-introduce the 86-file cascade Alt-A was
rejected to avoid. Symmetry is a cosmetic goal; landing the
slice without a workspace-wide cascade is a structural one. The
relocation of `JobSpecInput` from `aggregate::` to `api::submit::`
is a worthwhile cleanup but belongs in a future dedicated slice
(see Future Work).

### Alt-F — Centralised `SubmitSpecInput::validate` (REJECTED — OQ-6)

A single validation method on the enum that fires before any
per-kind constructor.

**Rejected** at OQ-6: the per-kind validating constructors
(`JobV1::from_submit`, `ServiceV1::from_submit`,
`ScheduleV1::from_submit`) already own the per-kind validation
rules. A centralised wire-side `validate` would duplicate the
rules or split them across two sites, both of which violate the
"one validation site per invariant" rule implicit in ADR-0050 §
4. The boundary function pattern is the canonical shape.

### Alt-G — Defer `ScheduleSpecInput` to a future slice (REJECTED — OQ-5)

Ship the enum with `{ Job, Service }` only; add `Schedule` later
when the Schedule streaming endpoint lands.

**Rejected** at OQ-5: deferring Schedule risks a second
structural finding when the Schedule endpoint lands. The cost of
including the third arm now is ~20 lines of type definition + one
OpenAPI variant + one validating constructor stub (the
constructor can `todo!("RED scaffold: Schedule submit lands
in <feature>")` per `.claude/rules/testing.md`). The benefit is
an exhaustive enum the compiler can pin: any future handler
dispatching on `SubmitSpecInput` is forced by exhaustiveness to
handle all three kinds, and the boundary code at admission cannot
silently miss the Schedule case.

## Implementation plan (single-cut, per `feedback_single_cut_greenfield_migrations.md`)

The slice that lands this ADR's types (DELIVER step 02-03b
resumption) is a single atomic commit per the migration plan in §
6 above. No code is being written as part of this ADR — the
implementation plan exists here so the crafter resuming 02-03b has
the structural boundary pinned before they start.

## Future Work

- **`JobSpecInput` relocation cleanup**. The type currently lives
  at `crates/overdrive-core/src/aggregate/mod.rs:579` but is a
  wire-shape type by every other property (HTTP/JSON only, no
  rkyv, no parser concerns). Relocating to
  `crates/overdrive-core/src/api/submit.rs` alongside the new
  wire types would make the module structure coherent with the
  three-family model. **Deferred**: cost is the same ~10-file
  cascade the wire-as-third-family design avoids; benefit is
  module hygiene only. Land in a dedicated slice once 02-03b is
  closed.
- **gRPC bridge**. A future gRPC interface to the control plane
  would derive its protobuf shapes from `SubmitSpecInput` via a
  separate code-generator. Not in scope; surface here only so the
  next architect designing the gRPC bridge knows the wire-layer
  SSOT is `SubmitSpecInput`, not `WorkloadSpec` or
  `WorkloadIntent`.
- **SDK generation**. The `oneOf`-discriminated OpenAPI schema
  this ADR produces is the input contract for any future
  generated client SDK (TypeScript, Python, Go). The wire-layer
  evolution rules (additive JSON fields → OpenAPI semver minor;
  removed / renamed fields → semver major) become the contract
  for that ecosystem. Not in scope.

Neither future-work item has a GitHub issue per
`feedback_no_unilateral_gh_issues.md` — agents cannot create
issues. If the user decides either becomes near-term work, the
issue creation request is theirs.

## References

### ADRs

- ADR-0008 — Phase 1 REST endpoint table.
- ADR-0009 — OpenAPI schema generation via utoipa.
- ADR-0014 — Shared request/response types between CLI and server.
- ADR-0031 (and Amendment 1) — Driver tagged enum on Jobs
  (`DriverInput` wire shape, `WorkloadDriver` intent shape).
- ADR-0047 — Workload-kind discriminator (parser-side
  `WorkloadSpec`; the source of "Job is run-to-completion, no
  replicas" parser semantics).
- ADR-0048 — rkyv versioned envelope (codec-on-typed-value;
  governs the intent layer only — does NOT apply to this ADR's
  wire layer).
- ADR-0049 — Platform-issued `ServiceVipAllocator` (§ 5
  parser-level rejection of `Listener.vip`; this ADR carries the
  same rejection at the wire layer).
- ADR-0050 — Intent-side `WorkloadIntent` enum (parser-vs-intent
  separation; this ADR extends ADR-0050 with the wire as the
  third corner).

### Project rules

- `.claude/rules/development.md` § "Type-driven design" — sum
  types over sentinels; make invalid states unrepresentable.
- `.claude/rules/development.md` § "Persist inputs, not derived
  state" — VIP is platform-issued, structurally
  unrepresentable at every layer.
- `.claude/rules/development.md` § "Trait definitions specify
  behavior, not just signature" — per-kind validating
  constructors are the canonical boundary shape.

### Project memory

- `feedback_single_cut_greenfield_migrations.md` — no aliases,
  no shims, no grace periods; delete and replace in one commit.
- `feedback_no_unilateral_gh_issues.md` — agents cannot create
  GH issues; future-work items here have no issue numbers and
  await user direction.
- `feedback_research_scope_workflow_vs_orchestration.md` —
  Restate / Temporal excluded from evidence (this ADR is wire-
  shape only; the research doc does not apply).

### Checkpoint

- Commit `6b880e97` — DES protocol checkpoint capturing the
  crafter's PREPARE-pass structural finding. No production code
  changes; the body of the commit message contains the verbatim
  three-finding analysis summarised in § "Context" above.

### Sign-off

- User authorisation of Option B: 2026-05-15 (dispatch message).
- OQ-1 through OQ-8 recommendations in § "Status" above: **signed
  off 2026-05-15** (user accepted all eight recommendations
  verbatim). ADR Status flipped from Proposed to Accepted in the
  same edit.

## Amendment (2026-06-06) — describe direction superseded by ADR-0064

§ 1 ("Three distinct type families for the same logical concept") lists
the boundary mapping functions. One of them is, verbatim:

> - `WorkloadIntent` → `SubmitSpecInput` — `describe_workload`
>   endpoint echoes the persisted intent back to the operator on
>   GET; `JobV1 → JobSpecInput` reuses the existing `From<&Job>`
>   impl at `aggregate/mod.rs:641`.

**This describe-direction boundary is SUPERSEDED by ADR-0064**
(Accepted 2026-06-06). The `describe_workload` endpoint does NOT echo
the persisted intent back through `SubmitSpecInput`. It uses a
**distinct `DescribeSpecOutput`** type (a kind-discriminated `oneOf`
in `overdrive-core::api::describe`), for two reasons established at
ADR-0064 OQ-1:

1. **VIP surfacing.** The describe response must surface the
   platform-issued Service VIP (per #183). `SubmitSpecInput`
   structurally cannot carry it — its Service arm has no `vip` field
   and is `deny_unknown_fields` (per this ADR § 3 and ADR-0049 § 5,
   the operator never names a VIP). A distinct describe type is the
   only shape that surfaces the VIP without re-admitting an
   operator-supplied VIP on the submit wire.
2. **Cadence decoupling.** Echoing through `SubmitSpecInput` would
   re-couple describe ↔ submit evolution — a describe-only field would
   force a change on the submit wire and vice versa. That is the
   exact Pattern-C coupling this ADR fought to avoid (§ "Context" →
   "Constraints"). `DescribeSpecOutput` keeps the describe boundary
   independent.

**What remains valid:** the Job describe arm still reuses
`JobSpecInput` via the existing `From<&Job>` impl — but wrapped in
`DescribeSpecOutput::Job(JobSpecInput::from(&job))`, not handed back
as a bare `SubmitSpecInput`. The other three mapping functions in § 1
(TOML → `WorkloadSpec`, `WorkloadSpec` → `SubmitSpecInput`,
`SubmitSpecInput` → `WorkloadIntent`) are unchanged.

See **ADR-0064** for the full describe-side decision (OQ-1 / OQ-4 /
OQ-5 / OQ-7), the `DescribeSpecOutput` type sketch, the required-VIP
shape, the read-only allocator `get`, and the single-cut migration
plan.
