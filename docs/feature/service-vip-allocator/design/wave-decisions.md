# Wave Decisions — service-vip-allocator (DESIGN)

**Wave**: DESIGN
**Feature**: service-vip-allocator
**Date**: 2026-05-14
**Author**: Morgan (nw-solution-architect)
**Mode**: Propose

## Configuration captured at dispatch

| Field | Value |
|---|---|
| feature_id | `service-vip-allocator` |
| interaction_mode | propose |
| ssot | GH #167 + `docs/feature/service-vip-allocator/discuss/` |
| rigor_profile | inherit (lean / standard / thorough / exhaustive defaults — see `.nwave/des-config.json`) |
| output_directory | `docs/feature/service-vip-allocator/design/` |

## Architecture Summary

**Style**: Hexagonal (ports and adapters), single-process, single-node (Phase 1).
Same style as the existing brief.md § 1.

**Approach**: Two-layer allocator primitive in
`crates/overdrive-dataplane/src/allocators/`:

- Pure in-memory core `PoolAllocator<T: Token>` — sync, no I/O, BTreeMap
  memo + monotonic counter.
- Persistence shim `IntentBackedAllocator<T>` — wraps the core, writes
  through to `IntentStore` (fsync-then-memory ordering matching
  ADR-0035 § "Step ordering 7 → 8 is load-bearing").

**Two instantiations**:

- `BackendIdAllocator` = `PoolAllocator<BackendId>` directly (no
  persistence — re-hydrates via `ServiceMapHydrator` per ADR-0042).
  Replaces the existing `allocator.rs:31-82`; single-cut migration.
- `ServiceVipAllocator` = `IntentBackedAllocator<ServiceVip>` (persists;
  required by AC-02).

**Admission flow** (resolves Open Q2):
operator submit → parser (the `Listener` struct has no `vip` field per
the 2026-05-14 amendment of Q5 — operator-supplied `vip = "..."` fails
at TOML deserialise with `unknown field`) → spec-digest compute over
the operator spec directly → `ServiceVipAllocator.allocate(spec_digest)`
returns `vip` (allocator's `allocator_entries` row is the durable
record — § 1a, § 5a) → IntentStore admission write of the spec as-is →
submit-echo consults `ServiceVipAllocator::get(&spec_digest)` and
renders the assigned VIP at Service level (AC-01).

**Reclamation flow** (resolves Open Q1):
`WorkloadLifecycle` reconciler observes terminal-state row →
emits `Action::ReleaseServiceVip { spec_digest }` →
action-shim dispatches `ServiceVipAllocator.release(&spec_digest)` →
VIP returns to pool.

**Operator config** (resolves Open Q3):
new TOML subsection `[dataplane.vip_allocator]` with `ranges = [...]` +
optional `reserved = [...]`. Required — boot fails with a typed error
if missing.

**Shared trait shape** (resolves Open Q4):
Option (b) — pure core + persistence shim. Persistence is a structural
boundary at the type level; `BackendIdAllocator` compile-time-cannot-persist;
`ServiceVipAllocator` must persist. Rejected: (a) generic-with-trait
hides the distinction; (c) two independent types fail AC-05.

## Key Decisions

| ID | Decision | Rationale | Open Q resolved |
|---|---|---|---|
| K1 | Shared primitive at `crates/overdrive-dataplane/src/allocators/` — pure core + persistence shim | AC-05; persistence is a load-bearing structural distinction | Q4 |
| K2 | Single-cut migration of existing `allocator.rs` into `allocators/backend_id.rs` | `feedback_single_cut_greenfield_migrations.md` | Q4 |
| K3 | Persistence in IntentStore (not ViewStore) | The allocator is not a reconciler; allocation IS intent; whitepaper § 4 state-layer discipline | Q4 |
| K4 | rkyv versioned envelope per ADR-0048 for persisted state | Crosses redb persistence boundary; project rule | Q4 |
| K5 | Submit-time admission (before IntentStore admission) | AC-01 echo, AC-02 idempotency, AC-04 synchronous rejection | Q2 |
| K6 | Pool config in `[dataplane.vip_allocator]` subsection; required `ranges` list; optional `reserved` list | ADR-0019 TOML precedent; #175 `[dataplane]` nesting precedent | Q3 |
| K7 | Reclamation via `WorkloadLifecycle` reconciler + `Action::ReleaseServiceVip` | Reconcilers converge; idempotent on retry; matches ADR-0013 primitive | Q1 |
| K8 | **Parser-level removal of the `vip` field from `Listener`** (amended 2026-05-14) | `.claude/rules/development.md` § "Type-driven design" → "make invalid states unrepresentable"; the prior admission-level rejection defended a state the type system can exclude structurally. Operator-pinned VIPs are a feature explicitly decided against, so the "Option-shaped field is forward-compatible" framing is preserving a defense for a non-feature (the deferral-without-issue shape CLAUDE.md forbids). Greenfield single-cut: field, validator, error variant, and slice-06's defending tests delete in one commit. | Q5 |
| K8a | **Assigned VIP lives in the allocator's own persisted memo, not on the spec aggregate** (amended 2026-05-14 — placement decision for the assigned VIP after removing it from `Listener`) | Option C of three considered (A: aggregate field — rejected because it puts an operator-shape field that's not operator-set on the aggregate; B: observation-only — rejected because it creates a second source of truth and requires synchronous observation-write at admission). The `IntentBackedAllocator<ServiceVip>` already persists `(spec_digest → ServiceVip)` via `allocator_entries`; submit-echo and `alloc status` consult `ServiceVipAllocator::get(&spec_digest)` at render time. The `Job`/`ServiceSpec` aggregate stays operator-spec-only — operator-spec data structurally cannot represent the assigned VIP. | Q5 cascade |
| K9 | Earned Trust probe at boot: IntentStore reachable + range non-empty + persisted state consistent with current range | Project's load-bearing principle | (cross-cutting) |
| K10 | `ServiceVip` newtype consolidated to single declaration at `overdrive-core::id::ServiceVip(Ipv4Addr)` — duplicate at `workload_spec.rs:360` deleted in same commit | Two inconsistent declarations exist today (one IpAddr, one Ipv4Addr); single-cut consolidation | (incidental discovery during reuse analysis) |

## Reuse Analysis (HARD GATE)

Every proposed component checked against existing components for
overlap. Zero unjustified CREATE NEW decisions.

| Existing Component | File / Reference | Overlap | Decision | Justification |
|---|---|---|---|---|
| `BackendIdAllocator` (monotonic counter + memo, BTreeMap, `allocate(ip,port,proto)`, `release(id)`) | `crates/overdrive-dataplane/src/allocator.rs:31-82` (ADR-0046) | Entire allocator concept overlaps — this IS the primitive being generalised | **EXTEND** (refactor) | AC-05: "the underlying allocator logic is shared between BackendId and ServiceVip allocators." Single-cut migration relocates the existing type into the shared `allocators/` module without API change. |
| `ServiceVip` newtype (Ipv4Addr-based, ADR-0047 spec layer) | `crates/overdrive-core/src/aggregate/workload_spec.rs:360` | Direct overlap with the allocator's output Token | **EXTEND / CONSOLIDATE** | Discovered during reuse analysis: TWO `ServiceVip` declarations exist (the other at `id.rs:647` wraps `IpAddr`, not `Ipv4Addr`). Consolidating to one canonical `ServiceVip(Ipv4Addr)` at `overdrive-core::id` is required for the allocator's `Token` impl to be unambiguous. Single-cut consolidation per `feedback_single_cut_greenfield_migrations.md`. |
| `ServiceMapHydrator` reconciler (ADR-0042) | `docs/product/architecture/adr-0042-service-map-hydrator-reconciler.md` | Observes Service state; downstream consumer of allocated VIPs (passes VIP into `Dataplane::update_service`) | **REUSE AS-IS** | No change needed. The hydrator reads the post-allocation VIP from the persisted spec; the allocator boundary is upstream of it. |
| `WorkloadLifecycle` reconciler (ADR-0013) | `docs/product/architecture/adr-0013-reconciler-primitive-runtime.md` + ADR-0021 / ADR-0035 / ADR-0036 | Terminal-state convergence is its existing job | **EXTEND** (new View field + new Action variant) | Reclamation is convergence-shaped; this reconciler is the natural emitter. Extension: `released_for_terminal: BTreeSet<ServiceSpecDigest>` field on the View (input — records past emissions, not a derived deadline); new `Action::ReleaseServiceVip` variant. No new reconciler created. |
| `IntentStore` trait (whitepaper § 4; ADR-0011 / ADR-0035) | `crates/overdrive-core/src/traits/intent_store.rs` + `crates/overdrive-store-local` | Persistence boundary for allocator state | **REUSE AS-IS** | The persistence shim layers atop the existing IntentStore trait without modifying it. New table `allocator_entries` is added under the existing `LocalStore` impl per the project's standard table-per-namespace pattern. |
| `Action` enum (`overdrive-core::reconciler`) | `crates/overdrive-core/src/reconciler.rs` (ADR-0023, ADR-0042) | Action dispatch surface | **EXTEND** (new variant) | Adding `Action::ReleaseServiceVip { spec_digest, correlation }` — the same pattern ADR-0042 used for `Action::DataplaneUpdateService`. Action-shim arm added; no shape change. |
| Action-shim dispatcher | `crates/overdrive-control-plane/src/reconciler_runtime/action_shim/` (ADR-0023) | Dispatches Actions to side-effecting components | **EXTEND** (new arm) | One additional match arm for `Action::ReleaseServiceVip` calling `ServiceVipAllocator::release`. Existing exhaustive-match shape preserved. |
| `[dataplane]` config block (forthcoming, GH #175) | `crates/overdrive-control-plane/src/config.rs` (or equivalent) | TOML config surface | **EXTEND** (new subsection) | Adding `[dataplane.vip_allocator]` under the existing `[dataplane]` block — preserves the "dataplane sub-component" mental model. |
| `Listener.vip: Option<ServiceVip>` (ADR-0047 § 4a / slice-06 brief) | `crates/overdrive-core/src/aggregate/workload_spec.rs` (Slice 06 of `workload-kind-discriminator`) | Field shape on the operator-input spec | **DELETE (amended 2026-05-14)** | Per ADR-0049 § 5 amendment: the field is removed at the parser/spec layer to make invalid states unrepresentable. The `Listener` struct becomes `(port, protocol)`-only. The "Option-shaped field is forward-compatible" framing from slice-06 R6.1 is moot — its mitigation no longer applies because the field is gone. Cascade: uniqueness rule simplifies to `(port, protocol)`; render shape drops the per-listener VIP slot; slice-06's mixed-pinned-and-pending parser test + integration test delete; property test re-targets `(port, protocol)` pairs. |
| `AdmissionError::VipNotOperatorAssignable` | (was new in prior resolution; never landed) | Admission-level rejection of operator-supplied `vip = Some(...)` | **DELETED (amended 2026-05-14)** | No longer needed. With the `vip` field removed, operator-supplied `vip = "..."` fails at TOML deserialise with `unknown field` + named guidance; no runtime check at admission time, no `AdmissionError` variant, no admission-walk loop. Per `.claude/rules/development.md` § "Deletion discipline" the variant and any test that would have defended it are absent from the landing commit. |
| `CorrelationKey::derive` (`overdrive-core::id`) | `crates/overdrive-core/src/id.rs` (ADR-0042 § 1) | Correlation between Action emission and observation | **REUSE AS-IS** | New `Action::ReleaseServiceVip` carries a `CorrelationKey` per ADR-0042's precedent — same constructor surface. |
| `VersionedEnvelope` trait + `EnvelopeError` (ADR-0048) | `crates/overdrive-core/src/codec/envelope.rs` | rkyv persistence-boundary envelope discipline | **REUSE AS-IS** | New `AllocatorEntryEnvelope` follows the existing per-type envelope shape; codec-internal, alias-to-payload, golden-bytes fixture per ADR-0048 § "Version-bump procedure". |
| `PoolAllocator<T: Token>` core | (NEW) | n/a | **CREATE NEW** | Justified: no existing generic allocator type exists. The existing `BackendIdAllocator` is monomorphic; generalising it to accept `Token` is the EXTEND on the existing type. The `PoolAllocator<T>` IS that generalisation. |
| `IntentBackedAllocator<T>` persistence shim | (NEW) | n/a | **CREATE NEW** | Justified: persistence is a new requirement (AC-02). No existing IntentStore-backed wrapper exists; the shim is the structural boundary that distinguishes persistent allocators from non-persistent. Could not be implemented as an extension of an existing type because no existing type has this concern. |
| `Token` trait | (NEW) | n/a | **CREATE NEW** | Justified: the trait surface required to factor BackendId + ServiceVip uniformly does not exist. The trait is the load-bearing abstraction enabling AC-05. |
| `VipRange` (CIDR + reserved set) | (NEW) | n/a | **CREATE NEW** | Justified: no existing CIDR-range-with-reserved-set type in the codebase. `ipnet::Ipv4Net` from the existing workspace dependency surface provides the CIDR primitive; `VipRange` composes it with a `BTreeSet<Ipv4Addr>` for exclusions. Could not be reused. |
| `ServiceSpecDigest` newtype | (NEW or REUSE) | n/a | **REUSE** (likely `SpecDigest` or `ContentHash` already exists) | The project has `ContentHash` newtype in `overdrive-core::id` for SHA-256 digests. `ServiceSpecDigest` is either an alias to `ContentHash` or its own newtype-around-`[u8;32]`. Discovery deferred to crafter; ADR specifies "spec digest" semantics; the existing type satisfies. |

**Verdict (post 2026-05-14 amendment)**: 7 EXTEND, 4 REUSE AS-IS, 4 CREATE NEW, 2 DELETE (`Listener.vip` field; the previously-proposed `AdmissionError::VipNotOperatorAssignable` variant). Reviewer gate compliance: every CREATE NEW carries explicit "no existing alternative" justification; every DELETE is single-cut per greenfield migration discipline.

## Technology Stack

All existing workspace dependencies; **no new third-party dependencies** introduced.

| Concern | Choice | License | Existing dep? |
|---|---|---|---|
| Memo / pool tables | `std::collections::BTreeMap` | std | Yes (used everywhere) |
| Mutex on persistence shim | `parking_lot::Mutex` | MIT/Apache-2.0 | Yes (workspace dep) |
| CIDR primitive | `ipnet::Ipv4Net` | MIT/Apache-2.0 | Yes (workspace dep — used by `Listener` validation) |
| Persistence wire format | `rkyv` per ADR-0048 | MIT | Yes (project SSOT for archived persistence) |
| Persistence backend | `IntentStore` trait → `LocalStore` (redb) | MPL-2.0 (redb) | Yes (existing) |
| Config format | TOML per ADR-0019 | — | Yes (existing) |
| Error types | `thiserror` typed enums | MIT/Apache-2.0 | Yes (project rule) |
| Content hashing | existing `ContentHash` (SHA-256) | — | Yes |

No proprietary technology. Open Source First validated.

## Constraints Established

| # | Constraint | Source | Compliance |
|---|---|---|---|
| C1 | Allocator primitive lives in `crates/overdrive-dataplane/`, not control-plane | DISCUSS D3 (user direction 2026-05-14) | ADR-0049 § 1 |
| C2 | Reconciler I/O contract (sync, no `.await`, no DB handle) | `.claude/rules/development.md` § Reconciler I/O; ADR-0035 | ADR-0049 § 6 (reconciler emits Action, action-shim dispatches; reconciler never touches allocator directly) |
| C3 | rkyv persistence-boundary envelope discipline | ADR-0048; `.claude/rules/development.md` § rkyv schema evolution | ADR-0049 § 1a |
| C4 | Phase 1 single-node; no cross-node allocator consensus | DISCUSS D8; `feedback_phase1_single_node_scope.md` | ADR-0049 § 1 (no Raft replication of allocator state — single-node IntentStore is the SSOT) |
| C5 | Greenfield single-cut migration of `BackendIdAllocator` | `feedback_single_cut_greenfield_migrations.md` | ADR-0049 § 7 |
| C6 | `Persist inputs, not derived state` in any reconciler View extension | `.claude/rules/development.md` | ADR-0049 § 6 (`released_for_terminal` is an input set of past emissions, not a derived deadline) |
| C7 | `BTreeMap` not `HashMap` in core memo tables | `.claude/rules/development.md` § Ordered-collection choice | ADR-0049 § 1 (PoolAllocator uses BTreeMap) |
| C8 | Newtype completeness on `ServiceVip` (FromStr, Display, Serialize/Deserialize, validating constructor) | `.claude/rules/development.md` § Newtype completeness | ADR-0049 § 2 (consolidates to existing complete newtype) |
| C9 | Earned Trust probe at composition root | Project core principle 12 | ADR-0049 § 8 |
| C10 | Port-trait dependencies injected mandatorily in `new()`, not via builders | `.claude/rules/development.md` § Port-trait dependencies | `IntentBackedAllocator::bulk_load` takes `Arc<dyn IntentStore>` as required parameter |
| C11 | No `gh issue create` without user approval | `feedback_no_unilateral_gh_issues.md` | This DESIGN wave creates no GH issues; deferrals (if any) surfaced to user for approval |
| C12 | xtask MUST NOT depend on `overdrive-*` crates | `.claude/rules/development.md` § xtask | ADR-0049 § 8 enforcement xtask scanner is purely syntactic (AST walk) — no overdrive-* dep |

## Upstream Changes

The DISCUSS wave back-propagated one Changed Assumption against
upstream Slice 06 of `workload-kind-discriminator` (platform-issued
VIPs only). **Amended 2026-05-14**: DESIGN's resolution of Open Q5
shifts from admission-level rejection (preserving the field) to
**parser-level removal of the `vip` field** on `Listener`. Per
`.claude/rules/development.md` § "Type-driven design" → "make
invalid states unrepresentable", the parsed-spec shape no longer
carries the `vip` field at all. Consequences for slice-06:

- **Slice-06 `Listener` struct shape changes** — `vip:
  Option<ServiceVip>` is removed; `Listener` becomes `(port,
  protocol)`-only. Operator-supplied `vip = "..."` fails at TOML
  deserialise with `unknown field` + named guidance.
- **Slice-06 uniqueness rule simplifies** — from `(vip, port,
  protocol)` to `(port, protocol)`.
- **Slice-06 render shape changes** — submit-echo and `alloc
  status` per-listener line becomes `<port>/<protocol>`; the
  allocator-assigned VIP renders **at the Service level** via
  `ServiceVipAllocator::get(&spec_digest)`.
- **Slice-06 already-shipped tests delete** (single-cut per
  `feedback_single_cut_greenfield_migrations.md`): mixed-pinned-
  and-pending parser unit test; one-pinned-one-pending integration
  test; property test updates to round-trip `(port, protocol)`
  pairs instead of listener triples with `vip`.
- **No new `AdmissionError` variant** (the prior K8 admission-level
  validator + `AdmissionError::VipNotOperatorAssignable` are
  deleted; they never land).
- **One `ServiceVip` declaration is deleted** (the duplicate at
  `crates/overdrive-core/src/aggregate/workload_spec.rs:360`); the
  canonical lives at `crates/overdrive-core/src/id.rs:647`. This
  is a single-cut consolidation; references in the codebase point
  to the surviving canonical newtype.

See `upstream-changes.md` for the operator-facing description of
this change set, including verbatim line-number references into
slice-06's brief for the product owner.

## Open Questions Resolution Index

| # | Question (from DISCUSS § Open questions for DESIGN) | Resolution | ADR / brief.md reference |
|---|---|---|---|
| Q1 | Reclamation trigger | `WorkloadLifecycle` reconciler emits `Action::ReleaseServiceVip` on terminal-state observation | ADR-0049 § 6 |
| Q2 | When admission allocates | Submit-time (admission handler, before IntentStore admission write) | ADR-0049 § 4 |
| Q3 | Pool config shape | `[dataplane.vip_allocator]` subsection; required `ranges` list; optional `reserved` list | ADR-0049 § 3 |
| Q4 | Shared allocator trait shape | Pure-core + persistence shim (option b); `PoolAllocator<T: Token>` + `IntentBackedAllocator<T>`; module at `crates/overdrive-dataplane/src/allocators/` | ADR-0049 § 1 |
| Q5 | Upstream slice-06 spec shape | **Parser-level removal of the `vip` field on `Listener`** (amended 2026-05-14 from admission-level rejection). Operator-supplied `vip` is structurally unrepresentable. Assigned VIP lives in the allocator's `allocator_entries` memo, not on the spec. | ADR-0049 § 5 + § 5a |

All five resolved. No DESIGN-wave concern punted to DELIVER.

## C4 Diagrams

Component-level diagram for the allocator subsystem lives in
`docs/product/architecture/c4-diagrams.md` § Phase 1 — Service VIP
Allocator. System Context (L1) and Container (L2) inherit from the
existing Phase 2.2 diagrams unchanged — no new external actors, no
new containers (the allocator is internal to `overdrive-dataplane`).

## Quality Attribute Scenarios

| KPI (from DISCUSS) | Architecture-side support | Source |
|---|---|---|
| K1 — successful-allocation rate 100% on non-empty pool | Synchronous allocation at admission; pool size validated at boot; pool-empty case surfaces typed error not silent failure | ADR-0049 § 4, § 8 |
| K2 — p50 ≤ 5 ms, p99 ≤ 25 ms allocator-induced admission latency | In-memory `PoolAllocator` is O(log N) BTreeMap; single redb write + fsync; no network, no per-tick polling | ADR-0049 § 1 |
| K3 — p50 ≤ 1 s, p99 ≤ 5 s VIP reclamation lag | Reconciler tick cadence is 100 ms (ADR-0023); reclamation is one tick after terminal-state observation + action-shim dispatch + write-through fsync | ADR-0049 § 6 |
| K4 — 0 pool-exhaustion rejections per 24 h under nominal load | Pool capacity is operator-configured; boot probe validates persisted state fits within range; KPI surfaces as a typed counter the platform-architect instruments | ADR-0049 § 3, § 8 |

## Handoff package

### To DISTILL (`@nw-acceptance-designer`)

Inputs:

- `docs/feature/service-vip-allocator/discuss/` — full DISCUSS artifacts (user-stories.md, outcome-kpis.md, story-map.md, wave-decisions.md, dor-validation.md, peer-review.md)
- `docs/feature/service-vip-allocator/design/wave-decisions.md` — this document
- `docs/feature/service-vip-allocator/design/upstream-changes.md` — minimal back-propagation summary
- `docs/product/architecture/adr-0049-platform-issued-service-vip-allocator.md` — the design SSOT
- `docs/product/architecture/brief.md` — extended `## Application Architecture` section
- `docs/product/architecture/c4-diagrams.md` — Phase 1 Service VIP Allocator component diagram

Acceptance-test scenario shape: each AC (AC-01 through AC-06) maps to
one or more Rust integration test scenarios. The crafter writes Rust
`#[test]` / `#[tokio::test]` per project convention; no `.feature`
files. Specification-level GIVEN/WHEN/THEN blocks live in DISTILL's
`test-scenarios.md`.

### To DEVOPS (`@nw-platform-architect`)

Inputs:

- `docs/feature/service-vip-allocator/discuss/outcome-kpis.md` — KPIs K1–K4 with instrumentation guidance
- KPI-shaping decisions from this DESIGN:
  - K1: counter on admission entry (Service-kind submissions reaching the allocator), counter on allocator allocation, counter on admission success
  - K2: span timing isolated to `IntentBackedAllocator::allocate` (in-memory + persistence latency)
  - K3: two timestamps per allocation lifecycle — terminal-state-row write timestamp and `Action::ReleaseServiceVip` dispatch timestamp
  - K4: pool-utilisation gauge sampled per minute; typed `pool_exhausted` rejection counter

No external integrations; no contract-test annotations.

## Deferrals

**None.** All five open questions resolved within this wave; no
forward pointers in the artifacts; no new GitHub issues created.
Per the project rule (`feedback_no_unilateral_gh_issues.md` +
CLAUDE.md "Deferrals require GitHub issues — AND user approval
BEFORE creation"), if the user identifies a residual deferral
during review, they must approve issue creation before any
follow-up.

## Skipped artifacts (with rationale)

| Artifact | Skip rationale |
|---|---|
| `solution-architecture.md` standalone | The architecture lives in ADR-0049 + brief.md § Phase 1 service-vip-allocator extension. A separate document would duplicate. |
| `c4-diagrams.md` standalone | Diagrams land in the existing `docs/product/architecture/c4-diagrams.md` per the project pattern (one canonical file, per-feature sections). |
| ATAM / mini-ATAM | Single-decision feature; trade-offs documented in ADR-0049 § Considered alternatives + § Consequences. A standalone ATAM artifact would duplicate. |
| Threat model | No new attack surface — allocator is internal to a single-node binary; identifier-domain only. Phase 5+ multi-node will revisit. |
| DDD strategic-patterns artifact | Domain-Model section of brief.md is placeholder per current architect ownership; allocator does not warrant a DDD aggregate-discovery pass. |

## Changelog

- 2026-05-14 — Initial DESIGN wave decisions captured. ADR-0049
  written. All five open questions resolved. Reuse Analysis table
  produced (8 EXTEND, 5 REUSE AS-IS, 4 CREATE NEW — each justified).
  One incidental discovery: two `ServiceVip` newtype declarations
  exist in the codebase; consolidation captured as K10 + reuse
  analysis row.
- 2026-05-14 (amendment) — Q5 resolution changed from admission-level
  rejection (preserving the `vip: Option<ServiceVip>` field on
  `Listener`) to parser-level removal of the field entirely. Driver:
  `.claude/rules/development.md` § "Type-driven design" → "make
  invalid states unrepresentable" — the prior resolution defended a
  state the type system can exclude structurally, and its
  forward-compatibility argument preserved a defense for a feature
  (operator-pinned VIPs) the project has explicitly decided against.
  K8 rewritten; new K8a added for the placement decision (assigned
  VIP lives in the allocator's `allocator_entries` memo via
  `ServiceVipAllocator::get(&spec_digest)`, not on the aggregate).
  Reuse Analysis updated: `Listener.vip` row changes to DELETE; new
  DELETE row for the never-landing
  `AdmissionError::VipNotOperatorAssignable` variant. Verdict
  totals: 7 EXTEND, 4 REUSE AS-IS, 4 CREATE NEW, 2 DELETE. Upstream
  Changes summary rewritten — real spec-shape back-propagation now;
  slice-06's already-shipped tests delete in the same commit as the
  field removal per single-cut migration. ADR-0049 § 5 rewritten;
  § 5a added for the placement decision.
