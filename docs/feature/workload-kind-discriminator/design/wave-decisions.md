# DESIGN Decisions — workload-kind-discriminator

**Wave**: DESIGN
**Feature**: workload-kind-discriminator
**Date**: 2026-05-10
**Author**: Morgan (nw-solution-architect)
**Mode**: propose

## Configuration captured at dispatch

| Field | Value |
|---|---|
| feature_id | `workload-kind-discriminator` |
| interaction_mode | propose |
| paradigm | OOP (established by project CLAUDE.md) |
| diagram_format | Mermaid (C4) |
| review_enabled | true (orchestrator runs `nw-solution-architect-reviewer` after handoff) |
| output_directory | `docs/feature/workload-kind-discriminator/design/` (feature delta) + `docs/product/architecture/` (SSOT) |

---

## Key Decisions

- **[D1] `WorkloadSpec` is a tagged enum with three variants
  (`Service`/`Job`/`Schedule`)**, NOT three independent types and
  NOT an extended `Job` struct with a `kind` field. The enum is the
  natural Rust shape for a closed three-way discriminator; downstream
  consumers (intent-store key derivation, streaming dispatcher,
  alloc-status writer) match exhaustively. Extending `Job` with a
  kind field would make invalid states representable (e.g.
  `kind=Service, cron=Some(...)`); three sibling structs with no
  enum would force every consumer to be parametrised three times.
  Source: ADR-0047 §1 / Alternatives A+B.

- **[D2] `JobSubmitEvent` has NO `ConvergedRunning` variant.** This
  is the structural fix for RCA root causes B + C. The bug under
  audit was a Job submit emitting `ConvergedRunning` after seeing
  the first `state=Running` row, then exiting before the workload
  reached terminal. By removing the variant from the enum, the
  exact-1-of-3 emit sites that produced the bug literally cannot
  exist on the Job code path; the bug becomes unrepresentable, not
  forbidden by review. Source: ADR-0047 §3 / ADR-0032 Amendment
  2026-05-10.

- **[D3] Section-as-discriminator parsing via custom `Deserialize`,
  NOT bare `#[serde(untagged)]`.** Untagged enums collapse to "no
  variant matched" on failure; the named-error contract (UAT
  scenarios in user-stories.md require errors that name the
  offending sections by line) requires per-field walking of the
  TOML `Value::Table`. Source: ADR-0047 §2.

- **[D4] `AllocStatusRow.kind` is denormalised at write time;
  greenfield, no backfill.** Phase 1 has no surviving rows
  pre-feature. Cost = one column added to the row shape; benefit =
  render layer branches once on `kind` without re-fetching intent.
  Source: ADR-0047 §4 / ADR-0033 Amendment 2026-05-10.

- **[D5] Listener slice embedded as `Vec<ListenerRow>` on the row,
  NOT a separate `service_listener` table.** Render-path simplicity
  outweighs the cross-alloc-query ergonomic. When the runtime VIP
  allocator (#167) lands, it owns its own state; the embedded vec
  shape does not foreclose adding a separate listener-index table
  later. Source: ADR-0047 §4a.

- **[D6] `JobId` reused across all three kinds.** No `WorkloadId`
  rename. The CLI verb stays `overdrive job submit` /
  `overdrive alloc status --job <id>` (a kind-agnostic verb is a
  follow-up not in scope per Slice 04). The existing `JobId`
  validation surface is correct for all three kinds; proliferating
  identifier newtypes adds zero validation benefit. Source: ADR-0047
  §1a.

- **[D7] Three sibling streaming-event enums, NOT one flat enum
  with kind-tagged variants.** Per-kind closed enums make the
  impossible-for-Job variants structurally absent; a single flat
  enum was the bug shape (every consumer had to defensively check
  kind). Source: ADR-0047 §3 / Alternative D.

- **[D8] Slice 06 ships as a single slice; do NOT split.** See
  "Reviewer-flagged decisions" §1 below for the effort-asymmetry
  analysis and decision rationale.

- **[D9] K3 measurement cadence = pre-release manual gate (one-shot
  at first release).** See "Reviewer-flagged decisions" §2 below.

- **[D10] No new ADR for the `Listener` aggregate** — folded into
  ADR-0047 §1 as a "Service listener fields" sub-section. The
  Listener type is not load-bearing enough at the architecture
  layer to warrant an ADR of its own; its placement and shape
  decisions are well-captured inside ADR-0047.

---

## Architecture Summary

- **Pattern**: Hexagonal (ports and adapters), single-process. Same
  pattern as the rest of Phase 1 — this feature does NOT introduce
  a new architectural style. The `WorkloadSpec` aggregate sits on
  the intent side of the ports/state-layer boundary already
  established in brief.md §4 (state-layer discipline).
- **Paradigm**: OOP (Rust trait-based). Established by project
  CLAUDE.md; not re-litigated.
- **Key components affected**:
  - `overdrive-core::aggregate::*` — `Job` struct renamed to
    `JobSpec`; new `WorkloadSpec` enum; new `ServiceSpec`,
    `ScheduleSpec`, `Listener`, `ServiceVip`, `CronExpr`. Existing
    `Proto` newtype reused (NOT duplicated).
  - `overdrive-core::traits::observation_store::AllocStatusRow` —
    gains `kind: WorkloadKind` and `listeners: Vec<ListenerRow>`.
  - `overdrive-control-plane::api::streaming` — flat `SubmitEvent`
    becomes a kind-discriminating outer envelope wrapping three
    sibling enums.
  - `overdrive-cli::commands::job` — submit handler updated to
    branch on kind for streaming dispatch; line 504 `"live"`
    literal removed.
  - `overdrive-cli::render` — kind-aware render branches; new
    deferral URL constants; `format_running_summary` retained but
    reachable only on Service code path.
  - `xtask::dst_lint` — gains the `"live"` grep gate.

---

## Reuse Analysis

| Existing Component | File | Overlap | Decision | Justification |
|--------------------|------|---------|----------|---------------|
| `Job` aggregate (intent-side) | `crates/overdrive-core/src/aggregate/mod.rs:92` | Carries the workload's authoritative declaration. `WorkloadSpec` must carry the same data plus per-kind variation. | **EXTEND (rename + restructure)** | Single-cut rename to `JobSpec`; the struct becomes the inner body of `WorkloadSpec::Job`. `replicas` field removed (Job kind is run-to-completion). Greenfield Phase 1; no compat shim. |
| `JobSpecInput` (wire shape) | `crates/overdrive-core/src/aggregate/mod.rs:230` | Wire-side twin of the aggregate; same single-driver shape. | **EXTEND (rename + wrap in enum)** | Renamed to `JobSpecInput` (preserved name semantics for the inner) and wrapped in `WorkloadSpecInput` outer enum. The custom `Deserialize` impl branches on TOML section presence. |
| `DriverInput` / `WorkloadDriver` enums | `crates/overdrive-core/src/aggregate/mod.rs:124,265` | Carries `[exec]` table; future `MicroVm`/`Wasm` variants. Reused verbatim across all three kinds. | **EXTEND (no change)** | Each kind variant carries the existing `WorkloadDriver` / `DriverInput` shape verbatim. ADR-0031's tagged-driver decision is preserved. |
| `Proto` newtype | `crates/overdrive-core/src/dataplane/backend_key.rs:65` | Already a `Tcp`/`Udp` enum used by the dataplane. The Listener spec needs the same. | **EXTEND (re-export, do not duplicate)** | Per Slice 06 / GH #164 explicit decision: the spec layer imports `overdrive-core::Proto` directly. No second copy. |
| `Backend` type (dataplane) | `crates/overdrive-core/src/traits/dataplane.rs:54-60` | Existing destination-address type used by `Dataplane::update_service`. | **CREATE NEW (`Listener`)** under different name | Section name `[[listener]]` and type name `Listener` chosen specifically to avoid collision per #164 converged decision. The `Listener` is a *spec-layer* listener-attachment intent; `Backend` is a *dataplane-layer* destination. Different bounded contexts; structural collision avoided by naming. |
| `JobSubmitEvent` (current) | `crates/overdrive-control-plane/src/streaming.rs` (single flat `SubmitEvent` enum) | Existing flat enum carries all variants. | **EXTEND (split per-kind)** | Refactored into three sibling enums `Service*` / `Job*` / `Schedule*` wrapped in a kind-tagged outer envelope. Existing variants redistributed; `ConvergedRunning` moves to `ServiceSubmitEvent` only. |
| `AllocStatusRow` shape | `overdrive-core::traits::observation_store` (per ADR-0011) | Existing observation-side row. | **EXTEND (additive)** | Adds two fields: `kind: WorkloadKind` and `listeners: Vec<ListenerRow>`. Greenfield — no backfill. |
| `format_running_summary` render fn | `crates/overdrive-cli/src/render.rs:481` | Currently emits `Job '...' is running with N/M (took D)` for every kind. | **EXTEND (Service-only call site)** | Vocabulary changed to "Service"; literal `"live"` removed. Function survives, but its only call site is the Service code path. |
| `format_stopped_summary` render fn | `crates/overdrive-cli/src/render.rs` | Emits `Job '...' was stopped by ...`. | **EXTEND (kind-aware)** | Per slice-04 risk note, vocabulary becomes kind-aware (`Service '...' was stopped` / `Job '...' was stopped`). Architect bundles into Slice 04. |
| `xtask::dst_lint` scanner | `xtask/src/dst_lint.rs` | Existing banned-API scanner. | **EXTEND (new rule)** | Adds the `"live"` grep gate (US-06). Same pattern as existing rules; <100 LoC addition. |
| `examples/coinflip.toml` | `examples/coinflip.toml` | Bug reproduction file. | **EXTEND (single-cut migration)** | Rewritten to `[job]` shape per US-07; no compat shim. Phase 1 greenfield. |

**Verdict**: 10 EXTEND, 1 CREATE NEW (`Listener`, justified). All
reuse decisions defended against the "existing component is too
coupled" anti-pattern. The single CREATE NEW is necessary because
extending the dataplane `Backend` type with spec-layer concerns
would violate the bounded-context boundary (dataplane vs spec
layer) — naming `Listener` as a separate type keeps the contexts
distinct without denying the `Proto` newtype reuse.

---

## Reviewer-flagged decisions resolved

### 1. Slice 06 split — KEEP AS ONE SLICE

**Decision**: Slice 06 ships as a single slice. The architect-fault-line
(parser+echo vs alloc-status-rendering) IS real but does not warrant
a split.

**Effort asymmetry analysis** (from `slice-06-service-listener-fields.md`):
- Listener types + parser deserialisation + uniqueness validation
  ≈ 4h
- CLI submit echo render ≈ 1h
- CLI alloc status render extension ≈ 2h
- OpenAPI derives + property test ≈ 1h
- Test harness + integration tests ≈ 2h
- **Total: ~10h ≈ 1.5 days**

The "alloc status render extension" is small (2h) because the kind-
aware render machinery already exists from Slice 03 (D4); extending
the Service render branch with a Listeners section is mechanical.
Splitting would create a Slice 06a (~6h) and Slice 06b (~4h) where
Slice 06b cannot demonstrate end-to-end value alone (parser without
the second render surface ships half a feature; the property test
requires both surfaces to round-trip).

**Counter-evidence considered**: if the alloc status render extension
were genuinely large (e.g., separate `service_listener` ObservationStore
table — which we explicitly rejected per [D5]), splitting would make
sense. With the embedded-vec shape, the second half is trivial.

### 2. K3 measurement cadence — PRE-RELEASE MANUAL GATE

**Decision**: K3 (≥95% operator comprehension of Failed-Job exit code
from `alloc status`) is a **pre-release manual gate** (one-shot at
first release of this feature), NOT a continuous gate.

**Rationale**:
- The metric is fundamentally usability (operator comprehension), not
  functional correctness. A continuous CI gate cannot measure it; it
  requires a small-sample human study.
- Pre-release / at-release / post-release framing:
  - **Pre-release**: blocks merge if comprehension <95%. Costly
    (must run before each merge); slow feedback loop. Rejected.
  - **At first release** (chosen): one-shot gate at the feature
    landing milestone. If comprehension <95%, render layer is
    iterated until ≥95% before public release. Cheap; right-sized
    for a 5-10 operator study.
  - **Post-release**: feedback loop only. Rejected — operators
    would already have shipped misunderstandings into production.
- The automated parsing-from-fixtures regression test (the K3
  "stretch" measurement in outcome-kpis.md) IS continuous and is
  in scope from Slice 03 onward.

**Implementation note for outcome-kpis.md**: the measurement plan
table line for K3 should read:
> Manual usability check: pre-release one-shot, 5-10 operators read
> a `Failed (backoff exhausted)` alloc-status fixture and state the
> exit code. ≥95% pass = release; <95% = iterate render layer and
> retest. Automated fixture-parse regression test runs on every CI.

### 3. AllocStatusRow listener denormalisation — `Vec<ListenerRow>` ON THE ROW

**Decision**: `listeners: Vec<ListenerRow>` is embedded on the
`AllocStatusRow` (option (a) from review-discuss.md), NOT a separate
`service_listener` ObservationStore table.

**Rationale** (from ADR-0047 §4a):
1. Render-path is the only Phase 1 consumer; one query is simpler
   than join.
2. ObservationStore is full-row write (whitepaper §4); listener
   triples are part of the row's logical state.
3. Cross-alloc listener queries belong to the runtime VIP allocator
   (#167) — when that primitive lands, the allocator owns its own
   state; the `service_listener` table can be added without
   breaking the embedded shape.
4. CR-SQLite LWW (Phase 2+) replicates the embedded vec as a single
   field; no per-listener row-id contention.

`ListenerRow` shape: `{ port: NonZeroU16, protocol: Proto, vip: Option<ServiceVip> }`
— same shape as the spec-layer `Listener`, name distinguishes
intent-side `Listener` (within `ServiceSpec`) from observation-side
`ListenerRow` (within `AllocStatusRow`) per ADR-0011's intent-vs-
observation type-distinctness rule.

---

## ADR plan

| ADR | Action | One-line scope |
|---|---|---|
| **ADR-0047** (NEW) | NEW | Workload kind discriminator: `WorkloadSpec` enum at parser boundary; per-kind streaming protocols; kind denormalised on observation row. Partially supersedes ADR-0011. **Authored.** |
| ADR-0011 | AMEND | Append "Amendment 2026-05-10": partial supersession by ADR-0047; intent-vs-observation type-distinctness preserved verbatim; `Job` struct renamed to `JobSpec`. **Authored.** |
| ADR-0019 | NO CHANGE | ADR-0019 governs `~/.overdrive/config` operator config (YAML→TOML supersession). It does NOT govern workload spec TOML — that is ADR-0031's surface. The DISCUSS handoff list mistakenly named ADR-0019; the correct surface is ADR-0031. **No amendment authored.** |
| ADR-0031 | AMEND | Append "Amendment 2 — 2026-05-10": per-kind `WorkloadSpec` shapes; `[exec]` and `[resources]` retained at top level; `[[listener]]` array-of-tables placement. **Authored.** |
| ADR-0032 | AMEND | Append "Amendment 2026-05-10": per-kind `SubmitEvent` enums; `JobSubmitEvent` has no `ConvergedRunning`. **Authored.** |
| ADR-0033 | AMEND | Append "Amendment 2026-05-10": kind-aware render branches; `AllocStatusRow.kind`; Service listener denormalisation as embedded `Vec<ListenerRow>`. **Authored.** |
| ADR-0037 | AMEND | Append "Amendment 2026-05-10": Job-kind reconciler emits `Completed{exit_code:0}` / `Failed{exit_code:N}` typed `TerminalCondition` variants. Service kind unchanged; Schedule kind defers firing semantics to GH #166. **Authored.** |

**No additional ADRs required.** The `Listener` aggregate is folded
into ADR-0047 §1 (spec layer) and ADR-0033 Amendment (observation
layer denormalisation); a stand-alone "Listener" ADR would be over-
documenting a typed field set whose decisions are well-captured in
the surrounding ADRs.

---

## Forward pointers (intentionally out of scope)

- **GH #166** (Schedule execution semantics) — cron parser, fire-on-
  tick reconciler, ConcurrencyPolicy, history retention. Slice 05
  ships parser + composition validation only; the reconciler-side
  is deferred. Issue verified real per DISCUSS hard gate.
- **GH #167** (VIP allocator primitive) — runtime behaviour when
  Service `vip = None`. Slice 06 ships the spec-layer `Option`-shape
  field; the runtime allocator decision (allocate-at-runtime vs
  reject-at-admission) is deferred. Issue verified real per DISCUSS
  hard gate.
- **GH #163** (Listener dataplane wiring) — the question "where does
  this listener actually become a kernel BPF entry?" is OUT OF
  SCOPE. Slice 06 ships spec-shape + CLI render only; do NOT extend
  to `Dataplane::update_service` / BPF map updates. The dispatch
  prompt explicitly named #163's territory as forbidden.
- **RCA root cause A** (Service can report Running before workload
  stabilises) — tracked at
  [overdrive-sh/overdrive#170](https://github.com/overdrive-sh/overdrive/issues/170)
  ("Service health-check primitive — startup / readiness / liveness
  probes"). The earlier settle-window framing (#169, closed
  2026-05-10) was rejected by the user as wrong-primitive-shape —
  it would have shipped throwaway work superseded by proper health
  checks. The correct primitive is k8s-shaped probes (startup gates
  `Stable`, readiness gates traffic via existing
  `Backend.healthy` consumer, liveness drives restart). Out of
  scope here; approved for follow-up by user 2026-05-10.

---

## Architecture pattern + technology stack

- **Pattern**: hexagonal + state-layer discipline (unchanged from
  brief.md §1 / §4). The kind discriminator does NOT introduce
  microservices, event-driven, or layered patterns. Three workload
  kinds is a domain-model decomposition, not an architecture
  decomposition; the architecture remains one-binary.
- **Technology stack**:
  - Rust 2024 edition + existing workspace deps. **No new deps**.
  - `serde` + custom `Deserialize` for the TOML parser. Already in
    workspace.
  - `thiserror` for `ParseError`. Already in workspace.
  - `utoipa::ToSchema` derives on new types. Already in workspace.
  - `rkyv::Archive`/`Serialize`/`Deserialize` on `WorkloadSpec` for
    intent-store canonicalisation. Already in workspace.
  - No proprietary tech. No license issues — every dep is the same
    OSS already in workspace.
- **Architectural enforcement**:
  - `xtask::dst_lint` extended with the `"live"` grep gate.
  - `cargo openapi-check` runs on the new `*SubmitEvent` schemas
    and `Listener` / `ServiceVip` derives.
  - `cargo nextest run -p overdrive-cli -E 'test(coinflip_honesty)'`
    is the K1 measurement gate (lima-routed).

---

## Constraints established

- `WorkloadSpec` is the single intent-side workload aggregate.
  Future workload kinds (e.g. `Function` for FaaS) append as new
  variants; existing variants are untouched.
- `JobSubmitEvent` is closed; adding a `ConvergedRunning`-shaped
  variant in the future violates the structural bug fix and is
  forbidden.
- `AllocStatusRow.kind` is non-nullable. Phase 1 greenfield; no
  backfill code path may be added.
- `[[listener]]` is invalid for Job and Schedule kinds in this
  feature. Future fold-ins (e.g., listener-attached scheduled
  workloads) are out of scope.
- The `Proto` newtype is the single SSOT for protocol identity
  across spec and dataplane layers. Adding a second `Protocol`-named
  enum is forbidden.
- The `"live"` literal is forbidden in render-path source. The
  grep gate enforces.
- `SCHEDULE_EXECUTION_TRACKING_URL` and `SERVICE_VIP_ALLOCATOR_TRACKING_URL`
  are CLI constants; submit-echo and `alloc status` MUST read from
  them, NOT hardcode the URLs at the call site.

---

## Upstream Changes

DISCUSS assumptions are preserved in shape; the DESIGN wave's
deviations are minor and recorded inline in
`docs/feature/workload-kind-discriminator/design/upstream-changes.md`
per the skill spec. Specifically:

- **K3 measurement cadence** is now pinned (pre-release one-shot);
  outcome-kpis.md needs the measurement plan line updated. PO
  review.
- **AllocStatusRow listener shape** is now pinned (embedded
  `Vec<ListenerRow>`); shared-artifacts-registry.md `${listener_triple}`
  consumer #3 ("AllocStatusRow listener fields denormalised at
  write time (architect to confirm shape)") needs the shape pinned.
  PO review.
- **Slice 06 split** is decided NOT to split; slice-06-service-
  listener-fields.md "Carpaccio shape — single slice, defended"
  section already pre-anticipated this verdict.

No DISCUSS user story is invalidated. No DISCUSS journey is
contradicted. No DISCUSS slice is renumbered.

---

## Quality gates checklist

- [x] Requirements traced to components (US-01..US-08 → §54..§62 of
      brief.md)
- [x] Component boundaries with clear responsibilities (parser,
      streaming dispatcher, render layer, observation row writer)
- [x] Technology choices in ADRs with alternatives (ADR-0047 §1/§3
      Alternatives A-E)
- [x] Quality attributes addressed (ASR-WKD-01..05; perf,
      reliability, maintainability, usability)
- [x] Dependency-inversion compliance (no new ports; existing trait
      surface preserved)
- [x] C4 diagrams (L1+L2 retained from c4-diagrams.md;
      design/c4-diagrams.md ships L3 for the spec parser pipeline)
- [x] Integration patterns specified (intent-store write, NDJSON
      streaming, observation-store row write, CLI render)
- [x] OSS preference validated (no new deps; all existing OSS)
- [x] AC behavioral, not implementation-coupled (UAT scenarios in
      user-stories.md describe operator-observable outcomes)
- [x] External integrations annotated (none; feature is purely
      internal)
- [x] Architectural enforcement tooling recommended (`xtask::dst_lint`
      grep gate; `cargo openapi-check`; KPI K1 honesty integration
      test)
- [ ] Peer review completed and approved (pending — orchestrator
      runs reviewer next)

---

## Changelog

- 2026-05-10 — Initial DESIGN-wave decisions captured. ADR-0047
  authored. Amendments to ADR-0011, ADR-0031, ADR-0032, ADR-0033,
  ADR-0037 authored. brief.md §54-§62 added. C4 design diagrams
  authored. Three reviewer-flagged decisions resolved.
