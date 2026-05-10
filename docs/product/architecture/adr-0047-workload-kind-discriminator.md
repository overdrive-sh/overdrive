# ADR-0047 — Workload kind discriminator: `WorkloadKind` enum at the spec-parser boundary; `Service` / `Job` / `Schedule` as separate aggregate variants; per-kind streaming protocols

## Status

Accepted. 2026-05-10. Decision-makers: Morgan (proposing); DESIGN-wave
output of `docs/feature/workload-kind-discriminator/`.

Tags: phase-1, application-arch, type-shape, operator-surface, bug-fix.

**Partial supersession of ADR-0011.** The ADR-0011 ruling that "the
intent-side `Job` aggregate stays as the single workload aggregate
named `Job`" is amended (not totally superseded): the intent-side
aggregate becomes a tagged enum `WorkloadSpec` with three variants
(`Service`, `Job`, `Schedule`); the existing `Job` struct is moved
to `WorkloadSpec::Job(JobInner)` with `JobInner` carrying the prior
fields. ADR-0011's load-bearing claim — that intent-side aggregates
and observation-side row shapes stay in different modules and never
merge — is preserved verbatim. The whitepaper's ubiquitous-language
claim that the platform's central aggregate is named "Job" is
demoted to "Job is one of three workload kinds"; the whitepaper is
amended in the same wave (out of scope for this ADR).

## Context

`overdrive job submit examples/coinflip.toml` against an exit-1
workload reports `Job 'coinflip' is running with 1/1 replicas
(took live)` followed by CLI exit 0. Industry research
(`docs/research/platform/workload-type-taxonomy-research.md`)
cross-validates a three-aggregate model (Service / Job / Schedule)
against 13 of 15 vendor primaries. The DISCUSS wave for
`workload-kind-discriminator` converged on:

1. **Section-as-discriminator** in TOML — `[service]` / `[job]` /
   `[job] + [schedule]` — over an internally-tagged `kind = "..."`
   field. Section presence IS the kind; mixed sections are a parse
   error with named guidance.
2. **Per-kind streaming protocol** — separate `ServiceSubmitEvent` /
   `JobSubmitEvent` / `ScheduleSubmitEvent` enums, each closed.
   The Job enum has NO `ConvergedRunning` variant; the structural
   fix for RCA root causes B+C is that the call site that today
   emits `ConvergedRunning { ... }` for an exit-1 Job cannot exist
   on the Job code path because the variant does not exist there.
3. **Kind denormalised at write time** onto observation rows so
   `alloc status` can branch on kind without re-deriving from intent.

The DESIGN-wave open questions resolved here: aggregate
decomposition (single tagged enum vs three independent types vs
extend existing `Job`); streaming-protocol per-kind split shape
(three sibling enums vs one enum with kind-tagged variants);
denormalisation shape on `AllocStatusRow`; placement of the
`Listener` field set introduced by Slice 06 / GH #164.

## Decision

### 1. `WorkloadSpec` aggregate — tagged enum at the parser boundary

```rust
// in overdrive-core::aggregate

/// Validated intent-side workload aggregate. The kind discriminator
/// is structural: each variant carries the per-kind validated fields
/// and nothing else. Mixing kinds is a compile-time error in
/// downstream code.
///
/// rkyv-archived form is the canonical byte sequence used for
/// content-addressed identity per ADR-0002. serde + JSON is the wire
/// lane for CLI-to-server.
#[derive(
    Debug, Clone, PartialEq, Eq,
    Serialize, Deserialize,
    rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum WorkloadSpec {
    Service(ServiceSpec),
    Job(JobSpec),
    Schedule(ScheduleSpec),
}

#[derive(...)]
pub struct ServiceSpec {
    pub id: JobId,                   // identifier reused; see §1a
    pub replicas: NonZeroU32,
    pub resources: Resources,
    pub driver: WorkloadDriver,
    pub listeners: NonEmptyVec<Listener>,  // Slice 06 / GH #164
}

#[derive(...)]
pub struct JobSpec {
    pub id: JobId,
    pub resources: Resources,
    pub driver: WorkloadDriver,
    pub backoff_limit: NonZeroU32,   // Phase 1 default: 3
    // No `replicas` — Job kind is run-to-completion on one alloc per
    // Phase 1; per research R1 a future `parallelism` field appends
    // additively.
}

#[derive(...)]
pub struct ScheduleSpec {
    pub job_inner: JobSpec,          // composition, not inheritance
    pub cron: CronExpr,              // newtype; Slice 05 ships
                                     // string-only validation; see §6
}
```

The `Job` struct from ADR-0011 / ADR-0031 Amendment 1 is
**renamed in place** to `JobSpec` and re-shaped to drop
`replicas` (Job kind is run-to-completion). Its fields move into
the variant body. No compat shim per
`feedback_single_cut_greenfield_migrations.md`.

#### 1a. Identifier reuse — `JobId` stays

Per ADR-0011, intent-side identifiers live in
`overdrive-core::id`. The `JobId` newtype is reused verbatim across
`Service`, `Job`, and `Schedule` variants — operators type
`overdrive job submit` and `overdrive alloc status --job <id>`
regardless of kind, and the existing `JobId` validation surface
is correct for all three. A `WorkloadId` rename was considered and
rejected: the CLI verb stays `job` for this feature
(per Slice 04, the kind-agnostic `overdrive submit` verb is a
follow-up not in scope), and proliferating identifier newtypes
would complicate every downstream call site for no validation
benefit.

#### 1b. `WorkloadKind` enum — derivable, not stored

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadKind {
    Service,
    Job,
    Schedule,
}

impl WorkloadSpec {
    pub fn kind(&self) -> WorkloadKind {
        match self {
            Self::Service(_) => WorkloadKind::Service,
            Self::Job(_)     => WorkloadKind::Job,
            Self::Schedule(_) => WorkloadKind::Schedule,
        }
    }
}
```

`WorkloadKind` is the projection used by render layers and the
denormalised `AllocStatusRow.kind` column. Storing it on the
aggregate would violate "persist inputs, not derived state"
(`development.md`) — the variant IS the input; the kind is the
projection.

### 2. Wire shape — `WorkloadSpecInput` mirrors the aggregate

```rust
// in overdrive-control-plane::api

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields, untagged)]
pub enum WorkloadSpecInput {
    Service(ServiceSpecInput),
    Job(JobSpecInput),
    Schedule(ScheduleSpecInput),
}
```

The TOML parser lands a custom `Deserialize` impl (NOT bare
`#[serde(untagged)]` — the section-as-discriminator + named-error
requirement defeats untagged's blanket "no variant matched" message).
The custom impl walks the TOML `Value::Table`, branches on the
presence of `[service]` / `[job]` / `[schedule]` top-level tables,
and produces the correct variant or a typed `ParseError` naming
the offending sections.

Validation rules (parser-side, at the ingress boundary):

| Rule | Outcome |
|---|---|
| Both `[service]` and `[job]` present | `ParseError::MixedKinds { service_line, job_line }` |
| `[schedule]` without `[job]` | `ParseError::ScheduleRequiresJob { schedule_line }` |
| `[schedule]` with `[service]` | `ParseError::ScheduleWithService { schedule_line, service_line }` |
| Neither `[service]` nor `[job]` | `ParseError::NoKindSection` |
| Missing `[exec]` (any kind) | `ParseError::MissingExecBlock { kind }` |
| Missing `[resources]` (any kind) | `ParseError::MissingResourcesBlock { kind }` |
| `[schedule].cron` absent or empty | `ParseError::MissingCron` |
| Service with zero `[[listener]]` | `ParseError::NoListeners` (Slice 06) |
| Duplicate listener `(vip, port, protocol)` triple | `ParseError::DuplicateListenerTriple` (Slice 06) |

Every variant carries enough context for the CLI to render the
operator-visible message named in user-stories.md UAT scenarios.

**Universal flattening principle.** Only the kind discriminator
(`[service]` / `[job]` / `[schedule]`) is a "container" table at
the TOML top level, and even then it carries only the kind tag plus
kind-specific small fields (e.g. `id`, `replicas` for Service;
`backoff_limit` for Job; `cron` for Schedule). Every other
workload-level concern is a sibling top-level table — `[exec]`,
`[resources]`, `[[listener]]`, `[microvm]`, `[[sidecars]]`,
`[[policies]]`, `[security]`. The pre-existing nested form
(`[job.microvm]`, `[[job.sidecars]]`, `[[job.policies]]`,
`[job.security]`) is replaced wholesale; flattening is universal,
not per-section. The custom `Deserialize` impl reads each top-level
table independently and assembles the kind-tagged aggregate; the
parser does NOT walk a nested address space rooted at the kind tag.
This rule is the section-as-discriminator convention applied
consistently — supersedes any prior whitepaper TOML examples that
showed nested form.

### 3. Streaming protocol — three sibling enums (per-kind types)

```rust
// in overdrive-control-plane::api::streaming

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ServiceSubmitEvent {
    Accepted { spec_digest: String, intent_key: String, outcome: IdempotencyOutcome },
    Pending { reason: Option<String> },
    Running { since: String },
    ConvergedRunning { alloc_id: AllocationId, started_at: String },
    ConvergedFailed { reason: TerminalReason },         // existing shape, preserved
    ConvergedStopped { alloc_id: AllocationId, by: StopInitiator },
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobSubmitEvent {
    Accepted { spec_digest: String, intent_key: String, outcome: IdempotencyOutcome },
    Pending { reason: Option<String> },
    Running { since: String },                          // informational, NOT terminal
    AttemptFailed {
        attempt_index: u32,
        exit_code: i32,
        duration_ms: u64,
        will_restart: bool,
        next_attempt_delay_ms: Option<u64>,
    },
    Succeeded { exit_code: i32, duration_ms: u64, attempts: u32 },
    Failed {
        exit_code: i32,
        duration_ms: u64,
        attempts: u32,
        max_attempts: u32,
        stderr_tail: Vec<String>,
    },
    // NO `ConvergedRunning` — structural fix for RCA root causes B+C.
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScheduleSubmitEvent {
    Accepted { spec_digest: String, intent_key: String, outcome: IdempotencyOutcome },
    Registered {
        cron: String,
        deferral_url: String,                           // sourced from CLI constant; see ADR §6
    },
}
```

Each enum is closed; consumers exhaustively match. The wire
discriminator on the NDJSON stream is the kind tag in the
`SubmitEvent` envelope:

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubmitEvent {
    Service(ServiceSubmitEvent),
    Job(JobSubmitEvent),
    Schedule(ScheduleSubmitEvent),
}
```

The streaming subscriber selects which inner enum to use based on
the kind read off the persisted `WorkloadSpec` at the start of the
stream. The kind is NEVER inferred from the alloc-status row — the
intent-side spec is the authority.

### 4. `AllocStatusRow.kind` — denormalised at write time

The observation-side row gains one column:

```rust
// in overdrive-core::traits::observation_store

pub struct AllocStatusRow {
    pub alloc_id: AllocationId,
    pub job_id: JobId,
    pub node_id: NodeId,
    pub state: AllocState,
    pub updated_at: LogicalTimestamp,
    // ... existing fields ...

    /// NEW per ADR-0047. Denormalised from the originally-submitted
    /// `WorkloadSpec.kind()` at the moment the row is first written.
    /// Never re-derived. Phase 1 is greenfield — no backfill.
    pub kind: WorkloadKind,
}
```

The kind flows: parser → `WorkloadSpec` variant → submit-handler
writes intent + writes the first `alloc_status` row (or it is
written by the JobLifecycle reconciler on the first reconcile
tick, whichever happens first per ADR-0033). Both call sites have
the spec in scope and copy `spec.kind()` onto the row.

#### 4a. Listener denormalisation — embed `Vec<ListenerRow>` on the row

For Service-kind allocs, the listener triples must surface in
`alloc status` (KPI K6 byte-equality). Two shapes were considered:

- **(a) Embed `listeners: Vec<ListenerRow>` on `AllocStatusRow`.**
  Single read returns everything `alloc status` needs to render.
  Single write adopts the spec's listener slice verbatim. Cross-alloc
  listener queries (e.g. "what VIPs are in use?") require a full
  table scan — not a Phase 1 concern.
- **(b) Separate `service_listener` ObservationStore table, keyed
  `(alloc_id, listener_idx)`.** Cross-alloc queries become an
  index lookup; render path requires a join. Adds a second observation
  table, second writer-ownership boundary, second migration on
  schema evolution.

**Decision: (a) embed `listeners: Vec<ListenerRow>` on
`AllocStatusRow`** — present for Service kind, empty for Job /
Schedule. Rationale:

1. The render path is the only Phase 1 consumer; one query is
   simpler than join.
2. `ObservationStore` is full-row write (whitepaper §4 guardrail);
   listener triples are part of the row's logical state.
3. Cross-alloc listener queries belong to the runtime VIP allocator
   primitive (#167). When that primitive lands, the allocator owns
   its own state; the `service_listener` table can be added then
   without breaking shape (a) — option (a) does not foreclose
   option (b).
4. CR-SQLite LWW (Phase 2+) will replicate the embedded vec as a
   single field; no per-listener row-id contention.

```rust
pub struct ListenerRow {
    pub port: NonZeroU16,
    pub protocol: Proto,                  // overdrive-core::Proto reused
    pub vip: Option<ServiceVip>,          // newtype over Ipv4Addr
}
```

### 5. Render layer — kind-aware branches at the CLI surface

The CLI's existing `format_running_summary` is preserved on the
**Service code path only**, with vocabulary changed from "Job" to
"Service" and the literal `"live"` removed (RCA root cause D).
New render functions land for Job:

- `format_job_succeeded_summary(name, exit_code, duration, attempts)`
- `format_job_failed_summary(name, exit_code, duration, attempts, max_attempts, stderr_tail)`
- `format_job_attempt_failed(name, attempt, exit_code, duration, retry_in)`
- `format_job_alloc_status_header(name, kind, spec_digest, verdict)`
- `format_job_alloc_status_attempts_table(rows)`

For Schedule:

- `format_schedule_registered(name, cron, deferral_url)`
- `format_schedule_alloc_status(name, cron, deferral_url)`

Render is a pure dispatch on `kind`. The `alloc status` command
loads the row, reads `row.kind`, and branches once at the top of
the render function.

### 6. Deferral URL constant — single SSOT

```rust
// in overdrive-cli::render::deferrals
pub const SCHEDULE_EXECUTION_TRACKING_URL: &str =
    "https://github.com/overdrive-sh/overdrive/issues/166";
pub const SERVICE_VIP_ALLOCATOR_TRACKING_URL: &str =
    "https://github.com/overdrive-sh/overdrive/issues/167";
```

Both submit-echo and `alloc status` render layers read from these
constants. KPI K5 (Schedule URL byte-equality) and the K6
pending-VIP marker reference these. A grep gate (Slice 01) rejects
the literal `"live"` in render code.

### 7. Anti-pattern grep gate

The DST-lint `xtask` gains a new scanner that walks every
`.rs` file under `crates/overdrive-cli/src/render*` and
`crates/overdrive-cli/src/commands/*` and rejects any string
literal `"live"` passed as a positional argument to
`format_running_summary` (or any function whose name starts with
`format_`). Comments and docstrings are exempt. The gate runs in
CI alongside the existing `dst-lint` rule.

## Considered alternatives

### Alternative A — Three independent aggregate types (no `WorkloadSpec` enum)

```rust
pub struct ServiceSpec { ... }
pub struct JobSpec { ... }
pub struct ScheduleSpec { ... }
// no enum — three sibling types
```

**Rejected.** Every downstream consumer (intent-store key derivation,
streaming dispatcher, alloc-status writer) would need to be
parametrised three times — once per type — or accept a generic that
the trait system cannot bound usefully. The enum is the natural Rust
shape for a closed three-way discriminator; iterating the variants is
a `match`, not a runtime type-check.

### Alternative B — Extend the existing `Job` struct with a `kind: WorkloadKind` field

```rust
pub struct Job {
    pub id: JobId,
    pub kind: WorkloadKind,
    pub replicas: Option<NonZeroU32>,    // valid only for Service
    pub backoff_limit: Option<NonZeroU32>, // valid only for Job
    pub cron: Option<CronExpr>,           // valid only for Schedule
    pub listeners: Vec<Listener>,         // valid only for Service
    // ...
}
```

**Rejected.** Makes invalid states representable
(`kind=Service, cron=Some(...)` would compile). Forces every
consumer to pattern-match on `kind` AND then `expect(...)` on the
optional fields. Violates "make invalid states unrepresentable"
(`development.md` § "Sum types over sentinels"). The whole point
of the kind discriminator is to push validation into the type
system; weakening it back to runtime predicates defeats the bug
fix.

### Alternative C — Internally-tagged TOML `kind = "service"` field

```toml
kind = "service"
id = "payments"
[exec] ...
```

**Rejected** in DISCUSS (transcript). Section presence is more
operator-readable; it eliminates the "kind says service but I have
a `[schedule]` block" ambiguity case (which becomes a parser
contradiction); and it matches the existing `[exec]` /
`[resources]` table-as-discriminator convention from ADR-0031.

### Alternative D — One `SubmitEvent` enum with all variants flat (no per-kind sub-enums)

**Rejected.** This was the bug shape: every CLI consumer had to
defensively check `if event matches ConvergedRunning { ... } && spec.kind == Job` —
and the bug under audit is exactly the case where that check was
forgotten. Per-kind sibling enums make the impossible-for-Job
variants structurally absent; the bug becomes unrepresentable.

### Alternative E — Separate `service_listener` ObservationStore table

**Rejected** for §4a above. Defers cross-alloc query support to
#167 without foreclosing it. The embedded-vec shape is the simpler
write side and the simpler render side.

## Consequences

### Positive

1. **RCA root causes B+C+D are structurally closed for Job kind.**
   `JobSubmitEvent` has no `ConvergedRunning`; the literal `"live"`
   is removed; the streaming subscriber for Job kind waits for
   ExitObserver's terminal observation row before emitting
   Succeeded / Failed.
2. **Honesty rate (K1) moves from 0% to ≥99%** by construction —
   not by convention. The bug shape is unrepresentable, not
   forbidden by review discipline.
3. **Forward-compatible with future workload kinds.** A future
   `WorkloadSpec::Function(FunctionSpec)` (FaaS) variant appends
   without breaking existing variants; per-kind streaming protocols
   gain a fourth sibling enum.
4. **Spec layer carries listener triples (Slice 06 / GH #164)
   without committing to the runtime allocator (#167).** The
   `Option<ServiceVip>` field is forward-compatible with both
   "allocate at runtime" and "reject at admission" outcomes.
5. **Schedule kind ships the syntactic surface today**; execution
   semantics are honestly named as deferred (#166) and ship later
   without spec-shape rework.

### Negative

1. **Repository-wide rename.** Every `Job` consumer in
   `overdrive-control-plane`, `overdrive-cli`,
   `overdrive-store-local` must update to `WorkloadSpec` /
   `JobSpec`. Single-cut migration per
   `feedback_single_cut_greenfield_migrations.md`. The size is
   bounded — Phase 1 is small — and the cleanup is purely
   mechanical.
2. **Three sibling streaming-event enums replace one.** Adds wire
   surface but the surface is internal (single-binary Phase 1).
   The OpenAPI schema gains three `*SubmitEvent` types; existing
   consumers that don't care about kind dispatch on the outer
   `SubmitEvent` envelope.
3. **`AllocStatusRow.kind` is non-nullable** — Phase 1 has no
   surviving rows pre-feature; greenfield. Phase 2+ migration to
   real Corrosion will write rows fresh with the kind column;
   no backfill required.
4. **Section-as-discriminator parser is not a stock serde derive.**
   A custom `Deserialize` impl (or a hand-rolled `from_toml`
   constructor) is needed for the named-error contract. Adds ~150
   LoC in the parser module; bounded; well-tested.

### Quality attribute trade-offs

| Attribute | Impact | Direction |
|---|---|---|
| Functional correctness | Bug fix is structural — invalid states unrepresentable | + |
| Maintainability | Closed enums + exhaustive match catches every kind-add at compile time | + |
| Testability | Per-kind streaming protocols testable in isolation; anti-scenario tests assert variant non-existence | + |
| Operator usability | Kind-aware vocabulary matches mental model; deferral URLs honest | + |
| Performance | Negligible — three enum variants vs one, same alloc shape | 0 |
| Reliability | Honesty rate measurement (K1) is now achievable | + |
| Backward compatibility | Single-cut migration; Phase 1 greenfield | − (bounded) |

## Implementation note

Slice ordering is locked by the DISCUSS slice carve:

1. **Slice 01** — parser kind discriminator + grep gate +
   `examples/coinflip.toml` migration.
2. **Slice 02** — `JobSubmitEvent` + Job streaming subscriber +
   CLI Job render. Closes the bug.
3. **Slice 03** — `AllocStatusRow.kind` denormalisation + kind-aware
   alloc status render.
4. **Slice 04** — Service vocabulary preservation + `"live"` removal.
5. **Slice 05** — Schedule parsing + deferral URL.
6. **Slice 06** — Service `[[listener]]` spec shape + alloc status
   Listeners section. **Recommendation: keep as one slice, do NOT
   split** — see DESIGN-wave decision in
   `docs/feature/workload-kind-discriminator/design/wave-decisions.md`
   for the effort-asymmetry analysis.

The crafter dispatches at most one slice per PR train. Each slice's
artifacts (renamed types, new event enums, render branches) cross
the wire-vs-intent boundary explicitly via `WorkloadSpec` ↔
`WorkloadSpecInput` projections in the parser.

## Cross-references

- ADR-0011 — partially superseded (intent-vs-observation split
  preserved; "Job" demoted to one of three kinds)
- ADR-0019 — amended (`[[listener]]` array-of-tables placement)
- ADR-0031 — amended (per-kind shapes; listener fields)
- ADR-0032 — amended (per-kind `SubmitEvent` enums)
- ADR-0033 — amended (`AllocStatusRow.kind` + listener fields)
- ADR-0037 — amended (Job emits `Completed{exit_code:0}` /
  `Failed{exit_code:N}` typed terminal conditions)
- GH #166 — Schedule execution semantics (deferred)
- GH #167 — VIP allocator primitive (deferred)
- `docs/feature/workload-kind-discriminator/discuss/` — DISCUSS
  artifacts
- `docs/research/platform/workload-type-taxonomy-research.md` —
  industry validation (13/15 vendor primaries)
