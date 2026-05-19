# ADR-0050 — Intent-side workload aggregate hierarchy: kind-agnostic `WorkloadIntent` enum

## Status

**Accepted (2026-05-14)**. Decision-makers: Hera (proposing); user
sign-off 2026-05-14 locked in the five open questions surfaced at
proposal time. Unblocks DELIVER step 02-03a of
`docs/feature/service-vip-allocator/` (currently checkpointed at
commit `eec64ed0` after the crafter's structural finding surfaced the
parser/intent-side conflation).

**Open-question resolution (2026-05-14)**:

- **OQ-1 (Phase 1 vs Phase 2 cut)**: **Option β (incremental)**. Phase
  1 lands the `WorkloadIntent` enum + per-kind aggregates (`Job`,
  `Service`, `Schedule`) + persistence codec **without** revision
  lineage. Revision lineage (`generation`, `observed_generation`,
  sibling revision rows, retention) is deferred to issue
  [#180](https://github.com/overdrive-sh/overdrive/issues/180).
- **OQ-2 (Retention default)**: deferred to #180 (a consequence of
  Option β, not a separate decision).
- **OQ-3 (`ServiceV1` listener fields)**: Phase 1 mirrors the
  parser-side `Listener` shape only — `{ port, protocol }`. No
  health-check policy, no TLS-termination config, no backend weights.
  Those are additive later via envelope evolution per ADR-0048.
- **OQ-4 (`ScheduleV1` inner-job model)**: **embedded** `job: JobV1`.
  The per-fire instance is a Job. Matches the existing parser-side
  `ScheduleSpec { job_inner: JobSpec, cron_expr }`. The deferred-bytes
  (`job_spec_bytes: AlignedVec`) alternative is rejected.
- **OQ-5 (`IntentKey` relocation)**: **single-cut**. Delete
  `IntentKey::for_job(&id)` in the same commit that lands
  `WorkloadIntent`. Replace entirely with the workload-keyed chain
  (`workloads/<id>`-prefixed keys). No parallel `jobs/<id>` path. Per
  `feedback_single_cut_greenfield_migrations.md`.

Tags: phase-1, application-arch, aggregate-design, persistence-boundary,
intent-vs-parser-separation.

**Relates to**: ADR-0011 (intent vs observation aggregate split —
this ADR extends that boundary on the intent side); ADR-0035 / ADR-0036
(Reconciler I/O — typed `View`, runtime owns persistence, pure
`reconcile` over typed `(desired, actual, view, tick)`); ADR-0047
(workload-kind discriminator byte at `workloads/<id>/kind` — Slice 01's
parser-side `WorkloadSpec` enum that this ADR refuses to extend to
the intent side); ADR-0048 (rkyv versioned envelope; codec-on-typed-
value — this ADR applies the same shape to the new outer
`WorkloadIntent` type); ADR-0049 (platform-issued `ServiceVipAllocator`
— the feature whose DELIVER surfaced this gap; VIPs live in the
allocator's persisted state, NOT on the aggregate's spec).

**SSOT**: this ADR is the SSOT for the intent-side workload aggregate
shape. The parser-side `WorkloadSpec` enum at
`crates/overdrive-core/src/aggregate/workload_spec.rs:504-508` is and
remains a separate concern owned by ADR-0047.

**Research input**:
[`docs/research/aggregates/workload-spec-intent-separation-patterns.md`](../../research/aggregates/workload-spec-intent-separation-patterns.md)
— Kubernetes, Nomad, and Fly Machines survey. Restate and Temporal
sections of the research doc are deliberately excluded from this
ADR's evidence base per project memory
`feedback_research_scope_workflow_vs_orchestration.md` (workflow /
durable-execution platforms, not orchestration).

## Context

### What exists today

- **Parser-side** (`crates/overdrive-core/src/aggregate/workload_spec.rs`):
  the kind-discriminated `WorkloadSpec` aggregate landed in Slice 01 of
  `workload-kind-discriminator` per ADR-0047. Three variants:
  `Service(ServiceSpec) | Job(JobSpec) | Schedule(ScheduleSpec)`. The
  wire shape is `WorkloadSpecInput` (TOML deserialise with
  `#[serde(tag = "kind")]`), produced by `WorkloadSpecInput::from_toml_str`
  at the admission boundary. This is the operator-facing decoded shape.

- **Intent-side** (`crates/overdrive-core/src/aggregate/mod.rs:113`):
  only the `JobV1` aggregate exists, aliased as `pub type Job = JobV1`
  per ADR-0048's UI-02 alias-to-payload amendment. Carries
  `id: WorkloadId, replicas: NonZeroU32, resources: Resources,
  driver: WorkloadDriver`. The persistence-boundary codec lives on the
  typed value per ADR-0048 § 4b: `Job::archive_for_store` (write),
  `Job::from_store_bytes` (read), `Job::spec_digest` (content hash).
  The envelope is `JobEnvelope::V1(JobV1)`.

- **Discriminator byte** (`workloads/<id>/kind`): per ADR-0047 Slice 02
  a separate intent record holds the kind discriminator as a single
  ASCII byte (`s` / `j` / `c`). The streaming endpoint and reconciler
  runtime read this key to dispatch on kind without touching the
  aggregate body.

### What's missing

There is no kind-agnostic intent-side aggregate. The 02-03a step
description for `service-vip-allocator` ("migrate every
`Job::archive_for_store` reader site to `WorkloadSpec::*`") conflated
the parser-side enum (`WorkloadSpec`) with the intent-side aggregate
hierarchy. The user's pushback on commit `eec64ed0` was structural:
parser-side and intent-side are two different layers. Using
`WorkloadSpec` directly as the intent type would erode the
parser/intent boundary — exactly the Anti-pattern Z ("one type for
both wire format and persistence") that every orchestration platform
surveyed in the research doc avoids.

### Constraints the decision must respect

- **Pattern C (parsed-on-ingress, typed-on-disk)**: every orchestration
  platform surveyed (K8s, Nomad, Fly Machines) parses operator input
  into a *single canonical typed in-memory type* before persistence.
  Parser shape and persisted shape are explicitly different. Overdrive
  already does this for Jobs (`JobSpecInput` → `Job::from_spec` →
  `JobV1`); this ADR extends the pattern.

- **Reconciler purity** (ADR-0035/0036): `reconcile` is a pure
  synchronous function over `(desired, actual, view, tick)`. The
  reconciler reads typed aggregates; it does NOT orchestrate
  transitions between revisions. Rolling updates (when they arrive
  per #180) are workflows.

- **Persist inputs, not derived state** (`.claude/rules/development.md`):
  the aggregate carries the operator-submitted intent. Allocator
  outputs (VIP, BackendId, schedule next-fire time) are derived and
  live elsewhere (allocator state, observation rows, recomputed on
  every tick).

- **rkyv envelope evolution** (ADR-0048): every rkyv-persisted type at
  the redb boundary goes through a versioned envelope. The
  intent-side type gets one envelope at the outer-enum level; embedded
  per-kind payloads (`JobV1`, `ServiceV1`, `ScheduleV1`) are NOT
  separately wrapped per ADR-0048 § 4 (one envelope per persistence
  boundary type; embedded type changes bump the outer version).

- **Greenfield single-cut migrations**
  (`feedback_single_cut_greenfield_migrations.md`): no aliases, no
  shims, no grace periods. `Job::archive_for_store` /
  `Job::from_store_bytes` / `Job::spec_digest` are deleted in the same
  commit that lands the new outer codec. `IntentKey::for_job(&id)` is
  deleted in the same commit; replaced by the workload-keyed chain.
  On-disk bytes change format; pre-commit redb files are unreadable;
  operator deletes the redb file per the greenfield rule.

## Decision

### 1. Intent-side aggregate enum — `WorkloadIntent`

A new kind-agnostic outer enum lives at the intent layer alongside the
parser-side `WorkloadSpec`. The two are structurally distinct types
that happen to share variant names:

```rust
// crates/overdrive-core/src/aggregate/mod.rs

/// Kind-agnostic intent-side workload aggregate.
///
/// Per Pattern C (parsed-on-ingress, typed-on-disk) — distinct from
/// the parser-side `WorkloadSpec` at
/// `crate::aggregate::workload_spec::WorkloadSpec`. `WorkloadSpec` is
/// the operator-decoded TOML shape; `WorkloadIntent` is the
/// persisted-after-validation shape. The two are NOT aliases; they
/// are independent type families with different evolution
/// constraints (parser shape evolves with operator-facing fields;
/// intent shape evolves with on-disk bytes via the
/// `WorkloadIntentEnvelope`).
pub type WorkloadIntent = WorkloadIntentV1;

#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum WorkloadIntentEnvelope {
    V1(WorkloadIntentV1),
}

#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum WorkloadIntentV1 {
    Job(JobV1),
    Service(ServiceV1),
    Schedule(ScheduleV1),
}
```

**Name rationale**: `WorkloadIntent`, not `WorkloadAggregate` /
`Intent` / `Workload`. Three reasons:

- `Aggregate` is a generic OO term that doesn't disambiguate from
  observation aggregates (`AllocStatusRow`, `ServiceBackendRow`,
  etc.). Reviewers reading `WorkloadAggregate` would have to ask
  "intent or observation?"
- Bare `Intent` is too short; collides with `IntentStore`,
  `IntentKey`, and every other type with `intent` in its name.
  `git grep Intent` would have noise.
- `Workload` alone is the *concept*, not the *typed value*.
  `Workload::Service(_)` reads ambiguously — is it the wire-side or
  the persisted side? `WorkloadIntent::Service(_)` is unambiguous.

The pairing `WorkloadSpec` (parser) ↔ `WorkloadIntent` (persisted)
mirrors the existing `JobSpecInput` (wire) ↔ `Job` / `JobV1`
(persisted) shape — readers fluent in the workspace pattern map the
suffix instinctively.

### 2. Per-kind inner payloads

#### `JobV1` (existing — no change)

Carries `id: WorkloadId, replicas: NonZeroU32, resources: Resources,
driver: WorkloadDriver`. The `from_spec` validating constructor
remains; only the persistence-boundary surface relocates from
`Job::archive_for_store` to `WorkloadIntent::archive_for_store` (see
§ 4 below). The `Job` alias (`pub type Job = JobV1`) is retained so
existing struct-literal `Job { id, replicas, resources, driver }`
construction across the workspace stays unchanged.

#### `ServiceV1` (new — Phase 1 minimal shape)

Phase 1 scope per **OQ-3**: mirror the parser-side `Listener` shape
only — `{ port, protocol }`. No health-check policy, no
TLS-termination config, no backend weights. Those land later via
additive envelope evolution per ADR-0048.

```rust
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ServiceV1 {
    /// Workload identity. Same newtype Jobs use.
    pub id: WorkloadId,
    /// Replica count — long-running supervised workload per
    /// ADR-0047 § 1.
    pub replicas: NonZeroU32,
    /// Per-replica resource envelope. Same shape Jobs use.
    pub resources: Resources,
    /// Driver-class declaration carrying the operator's invocation
    /// shape. Same tagged enum Jobs use per ADR-0031 Amendment 1.
    pub driver: WorkloadDriver,
    /// Operator-declared listeners in declaration order.
    ///
    /// Per ADR-0049 § 5 the listener carries `(port, protocol)`
    /// ONLY — no operator-supplied VIP. VIPs are platform-issued
    /// via `ServiceVipAllocator` keyed by `spec_digest` and live in
    /// the allocator's persisted state, NOT on the aggregate. The
    /// platform projects the allocated VIP back onto listener rows
    /// when rendering the dataplane view; the intent-side
    /// aggregate never carries it.
    ///
    /// Phase 1 scope per OQ-3 (sign-off 2026-05-14): listener
    /// carries `(port, protocol)` ONLY. Health-check policy,
    /// TLS-termination config, and backend weights are NOT
    /// represented at the aggregate level — they land later via
    /// additive envelope evolution per ADR-0048.
    pub listeners: Vec<Listener>,
}
```

**Why `replicas` + `resources` + `driver` mirror `JobV1`**: per Slice 01
of `workload-kind-discriminator` (ADR-0047) the operator-facing
Service body declares the same fields as a Job (the kind discriminator
is what changes the supervision semantics — service is long-running
and restart-budget-driven, job is run-to-completion). Mirroring the
shape avoids spurious schema-evolution divergence; the two persist
identical bytes for shared fields.

**Listener field type**: the existing `Listener` newtype from
`crate::aggregate::workload_spec` (re-exported at
`crates/overdrive-core/src/aggregate/mod.rs:38`). The parser-side
`Listener` already carries `(port: NonZeroU16, protocol: Proto)` after
the `service-vip-allocator` step 02-01 amendment removed the
operator-supplied `vip` field. The intent side reuses this newtype —
it's a value-typed property of the spec, not an aggregate-crossing
concern.

**What `ServiceV1` does NOT carry**:

- **VIP**: platform-issued via `ServiceVipAllocator` per ADR-0049 § 1.
  Lives in the allocator's persisted state under the dataplane crate.
  Cross-reference: ADR-0049 § 5 explicitly removed the operator-facing
  `vip` field at the parser layer.
- **Backend list / replica endpoints**: observation, not intent.
  Lives in `service_backends` and `alloc_status` observation rows.
- **Health state**: observation, not intent.
- **Health-check policy / TLS config / backend weights**: deferred per
  OQ-3 (additive evolution, not Phase 1 scope).

#### `ScheduleV1` (new — embedded-job shape per OQ-4)

Per **OQ-4** (sign-off 2026-05-14): the embedded-`job: JobV1` shape
is adopted. The deferred-bytes (`job_spec_bytes: AlignedVec`)
alternative is rejected — it would force every `ScheduleV1` reader
to perform a second envelope decode to inspect the inner job, and
the schedule's per-fire instance IS a Job at the type system level.

The parser-side `ScheduleSpec` carries `job_inner: JobSpec, cron_expr:
CronExpr`. The intent-side projection follows the same shape: the
schedule's inner workload IS a Job (run-to-completion when fired); the
schedule adds the cron expression.

```rust
#[derive(Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ScheduleV1 {
    /// Workload identity for the schedule itself.
    pub id: WorkloadId,
    /// Inner job specification fired on each cron tick. Per Slice 05
    /// of `workload-kind-discriminator`: the per-fire instance is a
    /// run-to-completion Job, the schedule provides the timing.
    ///
    /// Embedded-job shape adopted per OQ-4 (sign-off 2026-05-14).
    /// The deferred-bytes alternative is rejected — every reader
    /// would otherwise pay a second envelope decode to inspect the
    /// inner job, with no compensating benefit.
    pub job: JobV1,
    /// Cron expression. Phase-1 String-shaped newtype validating
    /// non-empty-after-trim; richer semantic parsing tracked under
    /// GH #166 (Slice 05 of `workload-kind-discriminator`).
    pub cron_expr: CronExpr,
}
```

**What `ScheduleV1` does NOT carry**:

- **`next_fire_at` / `last_fired_at`**: derived from `cron_expr` +
  wall-clock per the "persist inputs, not derived state" rule.
  Recomputed every reconcile tick from `tick.now` + the persisted
  inputs.
- **Per-fire allocation history**: observation, not intent.

### 3. Persistence keying — single-cut to `workloads/<id>` (per OQ-5)

Per **OQ-5** (sign-off 2026-05-14): `IntentKey::for_job(&id)` is
**deleted** in the same commit that lands `WorkloadIntent`. The
replacement is the workload-keyed chain (`workloads/<id>`-prefixed
keys). There is no parallel `jobs/<id>` path during or after the
transition — per `feedback_single_cut_greenfield_migrations.md`, in
greenfield, removed is removed.

The Phase 1 key derivations:

- `IntentKey::for_workload(&WorkloadId)` → `workloads/<id>` — the
  aggregate body row carrying `WorkloadIntentEnvelope` bytes.
- `IntentKey::for_workload_stop(&WorkloadId)` → `workloads/<id>/stop`
  — the existing stop-sentinel marker, relocated from `jobs/<id>/stop`
  for consistency.
- `IntentKey::for_workload_kind(&WorkloadId)` → `workloads/<id>/kind`
  — the kind discriminator byte from ADR-0047 Slice 02. Already
  workload-keyed; no relocation needed.

The revision-lineage chain (`workloads/<id>/current`,
`workloads/<id>/revisions/<RevisionId>`) is deferred to #180. Phase 1
persists the aggregate body directly at `workloads/<id>`; #180 lands
the pointer-and-revision-rows refactor when the rolling-update
workflow needs them.

### 4. Persistence boundary — codec on `WorkloadIntent`

Per ADR-0048 § 4b (typed codec on the value, not the trait), the
persistence-boundary codec lives on `WorkloadIntent` itself:

```rust
impl WorkloadIntentV1 {
    /// Archive a `WorkloadIntent` for persistence through the
    /// `IntentStore`. Returns canonical rkyv bytes (envelope-wrapped).
    pub fn archive_for_store(&self) -> Result<AlignedVec, EnvelopeError> {
        let envelope = WorkloadIntentEnvelope::latest(self.clone());
        rkyv::to_bytes::<rkyv::rancor::Error>(&envelope)
            .map_err(|source| EnvelopeError::Malformed { source })
    }

    /// Decode persisted bytes back into a `WorkloadIntent`. On error,
    /// fires `health.startup.refused` per ADR-0048 § 6 and returns
    /// `IntentStoreError::Envelope`.
    pub fn from_store_bytes(
        bytes: &[u8],
        redb_path: &Path,
        key: Option<&str>,
    ) -> Result<Self, IntentStoreError> {
        match decode_envelope_bytes::<WorkloadIntentEnvelope>(bytes) {
            Ok(intent) => Ok(intent),
            Err(envelope_error) => {
                tracing::error!(
                    name: "health.startup.refused",
                    redb_path = %redb_path.display(),
                    key = key.unwrap_or("<unknown>"),
                    envelope_error = ?envelope_error,
                    "intent envelope decode failed; control-plane refusing to start",
                );
                Err(IntentStoreError::Envelope {
                    redb_path: redb_path.to_path_buf(),
                    source: envelope_error,
                })
            }
        }
    }

    /// Content-addressed identity. SHA-256 over the rkyv-archived
    /// raw payload (NOT the envelope). Stable across envelope
    /// version bumps. Same digest the `ServiceVipAllocator` already
    /// keys by per ADR-0049 § 1.
    pub fn spec_digest(&self) -> Result<ContentHash, EnvelopeError> {
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(self)
            .map_err(|source| EnvelopeError::Malformed { source })?;
        Ok(ContentHash::of(bytes.as_ref()))
    }
}
```

### 5. Reconciler vs workflow boundary

Per ADR-0035 / ADR-0036 the reconciler is a pure synchronous function
`reconcile(desired, actual, view, tick) → (actions, next_view)`. The
reconciler reads the current aggregate at `workloads/<id>` and
converges actual → desired. The reconciler does NOT orchestrate
transitions between revisions; rolling updates land later when
revision lineage lands per #180.

**Phase 1 update semantics**: a new submission overwrites
`workloads/<id>` and the reconciler converges immediately
(drop-and-replace). This is the same semantics today's `JobV1` path
provides; nothing regresses. Once #180 lands the revision-lineage
storage, a `RollingUpdate` workflow can coordinate gradual transitions
without changing the reconciler contract.

## Consequences

### Positive

- **Parser/intent separation made explicit in types**. Reviewers can
  no longer accidentally extend the parser-side `WorkloadSpec` to
  serve as the persisted shape (Anti-pattern Z). The two type families
  evolve independently.
- **Smaller Phase 1 scope** — the service-vip-allocator feature
  unblocks against the minimal `WorkloadIntent` enum + per-kind
  payloads + persistence codec. Revision lineage moves to its own
  design surface under #180.
- **`spec_digest` shape preserved**. The `ServiceVipAllocator` memo
  continues to key by `WorkloadIntent::Service(_).spec_digest()` —
  the same hash value `Job::spec_digest` produces today, just relocated
  to the outer enum's codec. No identifier drift.
- **On-disk format is correct enough for current scope**.
  `workloads/<id>` carrying `WorkloadIntentEnvelope` bytes is the
  final byte shape for the aggregate body. When revision lineage
  lands per #180, the aggregate body bytes stay byte-identical; the
  refactor adds *new* row families (pointer + revision rows), it
  doesn't change the existing aggregate's archived bytes.
- **Kind-agnostic outer envelope from day one**. Adding a fourth kind
  (`Function` for FaaS, `MicroVm`, `Wasm` per ADR-0031 / ADR-0047) is
  additive: append a variant to `WorkloadIntentV1`, bump to
  `WorkloadIntentV2` only if the layout shifts (it shouldn't for
  appended variants per ADR-0048 § "Why a per-type rkyv enum is
  forward-compatible across variant additions").
- **`IntentKey` keyspace becomes coherent**. Every workload-scoped key
  shares the `workloads/<id>` prefix — discriminator byte, stop
  sentinel, aggregate body. No more `jobs/<id>` outlier.

### Negative

- **On-disk single-cut migration**. Every operator with a non-empty
  redb file loses it on upgrade. Acceptable under the greenfield rule;
  surfaces in the upgrade notes.
- **A future single-cut migration is required when revision lineage
  lands** (tracked in #180). Every existing `workloads/<id>` row gets
  rewritten as a `workloads/<id>/current` pointer plus a
  `workloads/<id>/revisions/<RevisionId>` row (synthetic
  `revision_id = spec_digest`, `generation = 1`). Cheap under the
  greenfield single-cut rule (delete the redb file, resubmit), but
  operator-visible — surfaces a second time in the upgrade notes
  when #180 ships.
- **No revision history in Phase 1**. Audit-trail of prior submissions
  is not preserved; a resubmission overwrites the previous aggregate
  bytes at `workloads/<id>`. Acceptable for Phase 1 single-node scope;
  the audit-trail story is part of #180.

### Neutral

- **`Job::from_spec` validating constructor is preserved** —
  unchanged signature, just relocated callers (`submit_workload`
  produces `JobV1` then wraps in `WorkloadIntent::Job(_)`). No
  validation rules change.
- **`ServiceVipAllocator` keying is unchanged** — already keyed by
  `spec_digest` per ADR-0049. The allocator continues to consume the
  same hash value, now produced by `WorkloadIntent::spec_digest`.
- **Workflow primitive is unaffected** in Phase 1 — the rolling-update
  workflow doesn't yet exist; the storage it needs lands later under
  #180.

## Alternatives Considered

### Alt-A — One type for both parser and intent (REJECTED)

Use the parser-side `WorkloadSpec` directly as the persisted
intent-side type. `WorkloadSpec::archive_for_store` on the same enum
that powers TOML parsing.

**Rejected**: violates Pattern C (parsed-on-ingress, typed-on-disk).
Every orchestration platform surveyed avoids this — K8s typed Go
structs distinct from YAML, Nomad `*structs.Job` distinct from HCL,
Fly Machines server-side JSON distinct from `fly.toml`. The user's
pushback on commit `eec64ed0` was structural: parser shape and
persisted shape have different evolution constraints. The parser
needs to accept new operator-facing fields (e.g., schema migrations
on `WorkloadSpecInput`); the persistence layer needs byte-stable
rkyv archives. Coupling them through one type guarantees every
operator-facing field change triggers an on-disk migration — which
violates the rkyv envelope evolution discipline at ADR-0048.

This is the option the user explicitly rejected at the
`eec64ed0` checkpoint. Recording it for completeness.

### Alt-B — Widen `WorkloadSpec::Job` to carry `JobV1` directly (REJECTED)

Modify the parser-side enum to embed the validated intent types:
`WorkloadSpec::Job(JobV1)` instead of `WorkloadSpec::Job(JobSpec)`.

**Rejected**: this was the crafter's "option A" from the 02-03a
finding. Same flaw as Alt-A but lower-impact at the surface level —
the parser-side enum becomes a thin discriminator wrapper around the
intent-side types, erasing the parser/intent boundary anyway. The
parser is no longer free to evolve (e.g., adding an optional
operator-facing field on `JobSpec` would require an intent-side
schema bump because the persisted bytes would change shape). The
two layers must remain independently evolvable.

### Alt-C — Option α (land full revision lineage in Phase 1) (REJECTED)

Land `WorkloadIntent` + per-kind payloads + revision row family +
pointer row + `generation` / `observed_generation` indices all in
Phase 1, as a single greenfield-clean cut.

| Axis | Option α (greenfield-clean) | Option β (incremental — DECIDED) |
|---|---|---|
| **Phase 1 scope** | Full `WorkloadIntent` enum + revision lineage storage + `generation` / `observed_generation` indices | Kind-agnostic envelope + per-kind payloads + persistence codec only; revision lineage deferred to #180 |
| **`Job::archive_for_store` migration** | Single-cut delete; readers move to `WorkloadIntent` codec same commit | Single-cut delete; readers move to `WorkloadIntent` codec same commit |
| **`IntentKey::for_job` migration** | Single-cut delete; replaced by full workload-revision-pointer chain | Single-cut delete; replaced by flat `workloads/<id>` keying. Pointer/revision chain lands under #180 |
| **On-disk format** | Final shape for revision lineage from day one | Aggregate-body bytes are final; pointer/revision row families land under #180 (a second single-cut) |
| **02-03a unblock cost** | Larger (~30–40h: `ServiceV1` + `ScheduleV1` + revision pointer + sibling rows + recovery walk + golden fixtures + 02-03b on top) | Smaller (~12–18h: `ServiceV1` + outer codec + 02-03b on top) |
| **Future migration cost** | Zero | One single-cut migration of every persisted aggregate when #180 ships (cheap under greenfield, operator-visible) |

**Rejected in favor of Option β** (user sign-off 2026-05-14): scope
discipline preferred over format finality for this feature. Revision
lineage is a separate concern with its own design surface — it
involves operator-facing identity ergonomics (CLI `rollout undo
--to-revision=N`), retention policy (per-tenant or global,
`commercial.md`-bound or `[control_plane]`-bound), audit-trail
semantics, and the `RollingUpdate` workflow that exploits the
storage. Landing all of it speculatively as part of an in-flight
feature would block the service-vip-allocator on architectural
decisions that have no immediate consumer. Issue #180 owns the
follow-up.

The on-disk single-cut that lands when #180 ships is genuinely cheap
under the greenfield rule (delete the redb file, resubmit) — the same
rule that governs Phase 1's own single-cut. The cost being paid
twice instead of once is operator-visible upgrade notes; the cost
being saved is ~20h of speculative design baked into an unrelated
feature.

### Alt-D — `ScheduleV1` carrying deferred-bytes inner job (REJECTED — OQ-4)

`ScheduleV1 { id, job_spec_bytes: AlignedVec, cron_expr }` — store the
inner job as opaque archived bytes; decode on demand.

**Rejected** (user sign-off 2026-05-14 on OQ-4): every reader of a
schedule's inner job would pay a second envelope decode for no
compensating benefit. The schedule's per-fire instance IS a Job at
the type-system level; modelling it as opaque bytes erodes that
relationship without any storage or evolution win (the inner `JobV1`
participates in the outer `WorkloadIntentEnvelope`'s evolution
already; nothing about deferred bytes simplifies that). The embedded
`job: JobV1` shape is the clean Pattern C projection of the existing
parser-side `ScheduleSpec { job_inner: JobSpec, cron_expr }`.

## Implementation plan (single-cut, per `feedback_single_cut_greenfield_migrations.md`)

In one atomic commit:

1. **New types land** at `crates/overdrive-core/src/aggregate/mod.rs`:
   - `WorkloadIntentEnvelope`, `WorkloadIntentV1`, `WorkloadIntent` alias.
   - `ServiceV1`, `ScheduleV1` structs.
   - `WorkloadIntent::archive_for_store` / `from_store_bytes` /
     `spec_digest`.
   - Golden-bytes test fixture at
     `crates/overdrive-core/tests/schema_evolution/workload_intent.rs`
     with `FIXTURE_V1`.

2. **`IntentKey` derivations updated** (OQ-5 single-cut):
   - `IntentKey::for_workload(&WorkloadId)` → `workloads/<id>` —
     new; the aggregate-body row.
   - `IntentKey::for_workload_stop(&WorkloadId)` →
     `workloads/<id>/stop` — relocated from the existing
     `jobs/<id>/stop` derivation.
   - `IntentKey::for_workload_kind(&WorkloadId)` → `workloads/<id>/kind`
     — already exists per ADR-0047 Slice 02; unchanged.
   - **Deleted**: `IntentKey::for_job(&id)` → `jobs/<id>` and any
     `for_job_stop` derivation. No parallel `jobs/<id>` path.

3. **Old types deleted** (greenfield single-cut):
   - `Job::archive_for_store` — deleted.
   - `Job::from_store_bytes` — deleted.
   - `Job::spec_digest` — deleted (replaced by
     `WorkloadIntent::spec_digest`; the `Job` alias to `JobV1`
     remains for struct-literal construction inside
     `WorkloadIntent::Job(_)`).
   - `JobEnvelope` — deleted.

4. **All readers / writers migrate same commit**:
   - `submit_workload` handler: parses `WorkloadSpec` → projects to
     `WorkloadIntent` via per-kind validating constructors → writes
     `workloads/<id>` aggregate body + `workloads/<id>/kind`
     discriminator byte + (if stopping) `workloads/<id>/stop` sentinel.
   - `describe_workload` handler: reads `workloads/<id>` and projects
     back to wire shape.
   - Reconciler runtime `hydrate_desired`: reads `workloads/<id>`
     directly.
   - Recovery walk in `LocalIntentStore::open`: iterates
     `workloads/<id>` rows; each decode failure fires
     `health.startup.refused` per ADR-0048 § 6.
   - Streaming `submit-stream` endpoint: same as `submit_workload` +
     emits per-kind streaming events; the kind discriminator at
     `workloads/<id>/kind` (ADR-0047 Slice 02) is rewritten alongside
     the aggregate body.
   - `ServiceVipAllocator` keying: continues to key by
     `WorkloadIntent::Service(_).spec_digest()` — same hash value
     `Job::spec_digest` produced previously.

5. **On-disk single-cut**: pre-commit redb files unreadable. Operator
   follows the greenfield rule: delete `<data_dir>/intent.redb` and
   resubmit workloads.

## Future work

Revision lineage (Pattern A sibling-row family, Pattern B
`generation` / `observed_generation` indices, `RollingUpdate`
workflow, retention policy, operator-facing CLI ergonomics) is
tracked under issue
[#180](https://github.com/overdrive-sh/overdrive/issues/180). When
that work ships, the persisted-aggregate-body layout extends with two
new row families (`workloads/<id>/current` pointer +
`workloads/<id>/revisions/<RevisionId>` siblings) and a second
single-cut migration; the aggregate body bytes themselves stay
byte-identical to today's Phase 1 layout.

## References

### ADRs

- ADR-0011 — Intent vs observation aggregate split.
- ADR-0031 (and Amendment 1) — Driver tagged enum on `Job`.
- ADR-0035 / ADR-0036 — Reconciler I/O contract (typed `View`,
  runtime owns persistence, pure `reconcile`).
- ADR-0047 — Workload-kind discriminator (Slice 01 parser-side
  `WorkloadSpec`).
- ADR-0048 — rkyv versioned envelope (codec-on-typed-value at the
  persistence boundary).
- ADR-0049 — Platform-issued `ServiceVipAllocator` (the feature
  whose DELIVER surfaced this ADR; VIPs are platform-issued and
  unrepresentable in the spec).

### GitHub issues

- [#180](https://github.com/overdrive-sh/overdrive/issues/180) —
  Revision lineage on `WorkloadIntent` (Pattern A sibling rows,
  Pattern B `generation` / `observed_generation`, retention policy,
  `RollingUpdate` workflow). Deferred from this ADR per OQ-1 / OQ-2
  sign-off 2026-05-14.

### Project rules

- `.claude/rules/development.md` § "Reconciler I/O" — typed View,
  pure synchronous `reconcile`, runtime owns persistence.
- `.claude/rules/development.md` § "rkyv schema evolution" —
  per-type versioned envelope; alias-to-payload; codec on the typed
  value; golden-bytes fixtures.
- `.claude/rules/development.md` § "Persist inputs, not derived state"
  — `next_fire_at` / VIP / backend list are derived, not persisted on
  the spec.
- `.claude/rules/development.md` § "Type-driven design" — sum types
  over sentinels; make invalid states unrepresentable.

### Research

- [`docs/research/aggregates/workload-spec-intent-separation-patterns.md`](../../research/aggregates/workload-spec-intent-separation-patterns.md)
  — orchestration platform survey (Kubernetes, Nomad, Fly Machines).
  Pattern C (parsed-on-ingress, typed-on-disk) is the load-bearing
  pattern adopted here. Pattern A (sibling aggregate per revision)
  and Pattern B (two monotonic indices) are referenced in #180 for
  the future revision-lineage work. Restate and Temporal sections of
  the research doc are excluded from this ADR's evidence base.

### Project memory

- `feedback_single_cut_greenfield_migrations.md` — no aliases, no
  shims, no grace periods; delete and replace in one commit.
- `feedback_no_unilateral_gh_issues.md` — agents cannot create GH
  issues; #180 was created with user approval prior to this ADR
  amendment.
- `feedback_research_scope_workflow_vs_orchestration.md` — Restate
  and Temporal are workflow / durable-execution platforms, not
  orchestration; excluded from this ADR's evidence base.

### Sign-off

- User sign-off on OQ-1 through OQ-5: 2026-05-14.
