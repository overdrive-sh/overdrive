# DESIGN Wave Decisions — phase-1-control-plane-core

**Wave**: DESIGN (solution-architect)
**Owner**: Morgan
**Date**: 2026-04-23
**Status**: COMPLETE — handoff-ready for DISTILL (acceptance-designer) + DEVOPS (platform-architect), pending peer review.

---

## Wizard decisions honoured

- **D0 Scope**: Application / components (not system-level, not domain-level).
  Crate layout, trait boundaries, tech-stack ADRs, reconciler-primitive
  design.
- **D1 Mode**: Propose. Read DISCUSS + research; recorded 2–3 options per
  decision point with trade-offs; recommended one; authored ADRs directly
  (no `AskUserQuestion` tool available in this subagent context).
  Recommendations are flagged clearly — the parent agent may redirect if
  the user wants to override any ADR before peer review.
- **Rigor profile**: `inherit` session model; `review_enabled = true`;
  `mutation_enabled = true` (phase-1-foundation `testing.md` discipline
  still applies).

## Pinned inputs

- DISCUSS artifacts (user-stories, 5 slices, journey, outcome-kpis, dor-validation,
  story-map, shared-artifacts-registry).
- Talos TLS research (R1–R5) — `docs/research/security/talos-bootstrap-tls-strategy-comprehensive-research.md`.
- Whitepaper §3 + §4 (edited 2026-04-23 to codify REST + OpenAPI / tarpc split).
- Project rules (`development.md`, `testing.md`, `CLAUDE.md` project conventions).
- Phase 1 foundation ADRs 0001–0007 + brief.md Application Architecture.

## Reuse Analysis

Per the skill Hard Gate. New components were designed only when no
existing component could be extended.

| Area | Existing | Action | Justification |
|---|---|---|---|
| Port traits (`IntentStore`, `ObservationStore`, `Clock`, `Transport`, `Entropy`, `Dataplane`, `Driver`, `Llm`) | `overdrive-core::traits` | **EXTEND** | Adds `Reconciler` trait + `Action` enum + `Db` handle + `ReconcilerName` newtype in new sibling module `overdrive-core::reconciler`. |
| Newtypes (JobId, NodeId, AllocationId, …) | `overdrive-core::id` | **REUSE** | Aggregates wrap them; no replacement. |
| `JobSpec` placeholder in `observation_store.rs` | Present | **RESOLVED** (ADR-0011) | Kept separate: intent-side `Job` lives in `overdrive-core::aggregate`; observation-side `AllocStatusRow` stays where it is. Any vestigial `JobSpec`-named struct is deleted or renamed. |
| `Resources` struct | `overdrive-core::traits::driver` | **REUSE** | Single source; no duplicate. Slice 1 AC mandates this. |
| `LocalStore` | `overdrive-store-local` | **EXTEND** | Adds `commit_index()` read-only accessor; no new abstraction. |
| `SimObservationStore` | `overdrive-sim` | **REUSE** (ADR-0012) | Becomes the Phase 1 server's observation-store impl via wiring adapter. |
| Sim adapters (Clock/Transport/Entropy/Driver/Dataplane/Llm) | `overdrive-sim` | **REUSE** | DST harness composes them unchanged. |
| `xtask dst` harness | `xtask` | **EXTEND** | Three new invariants (`AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`, `ReconcilerIsPure`). |
| `xtask dst-lint` | `xtask` | **REUSE** | Reconciler purity inherits via crate-class labelling of core code. |
| `xtask` subcommands | `xtask` | **EXTEND** | Adds `openapi-gen` and `openapi-check`. |
| Error pattern (`thiserror` + `#[from]` pass-through) | `overdrive-core::error` | **REUSE** | `ControlPlaneError` follows the pattern. |
| Test layout (per-crate `tests/*.rs`, `tests/acceptance/*.rs` for acceptance) | ADR-0005 | **REUSE** | Round-trip + CLI acceptance tests land under this convention. |
| `overdrive-cli` | Placeholder scaffolding | **EXTEND** | Fill in handlers; no refactor of clap scaffolding. |
| Workspace deps: `rcgen`, `rustls`, `hyper`, `reqwest` | Already in workspace | **REUSE** | All needed primitives already staged. |
| Workspace deps: `axum`, `utoipa`, `utoipa-axum`, `libsql` | NOT present | **CREATE NEW** (workspace entries) | No existing equivalent; each has a clear role (ADRs 0008, 0009, 0013). |
| Control-plane crate | NONE | **CREATE NEW — `crates/overdrive-control-plane`** | No existing home. Isolates axum handlers, TLS bootstrap, reconciler runtime from the DST-pure `overdrive-core`. Required by the walking skeleton. |

**Counts**: REUSE = 11, EXTEND = 6, CREATE NEW = 3 (all justified; each with
ADR citation).

## Key decisions

### D1 — REST + OpenAPI external API (ADR-0008)

Codifies DISCUSS UC-1. `axum` + `rustls` over `hyper`; HTTP/2 preferred with
HTTP/1.1 fallback; `/v1` prefix non-negotiable; five walking-skeleton endpoints.
Internal RPC (tarpc / postcard-rpc) deferred to `phase-1-first-workload`.

**Recommendation posture**: PINNED — no options, codifying committed decision.

### D2 — OpenAPI schema derivation (ADR-0009)

**Recommendation**: `utoipa` + `utoipa-axum`; checked-in at `api/openapi.yaml`;
`cargo xtask openapi-check` CI gate; diffs regenerated vs checked-in.

Alternatives considered: `aide` (smaller, churnier API surface);
hand-maintained YAML (rejected as defeating the purpose).

### D3 — Phase 1 TLS bootstrap (ADR-0010)

**Recommendation**: Adopt Talos research R1–R5 wholesale. Ephemeral
in-process CA at `cluster init`; base64-embedded trust triple in
`~/.overdrive/config`; multi-SAN server cert; **no `--insecure`**; rotation /
revocation / roles / persistence deferred to Phase 5. One deliberate
divergence from Talos: role is NOT encoded in cert O-field (whitepaper §8
SPIFFE URI SANs).

**Recommendation posture**: PINNED — research is DESIGN-ready; R1–R5 were
self-contained from the review.

### D4 — `JobSpec` collision resolution (ADR-0011)

**Recommendation**: Keep intent-side `Job` (in `overdrive-core::aggregate`)
and observation-side `AllocStatusRow` (in `overdrive-core::traits::observation_store`)
as separate types in separate modules. Delete or rename any vestigial
`JobSpec`-named struct in `observation_store.rs`.

Alternatives considered: rename intent-side to `WorkloadSpec` (rejected —
whitepaper ubiquitous language is `Job`); one struct across both layers
(rejected — contradicts intent/observation split).

### D5 — ObservationStore server impl (ADR-0012)

**Recommendation**: Reuse `SimObservationStore` behind a wiring adapter in
`overdrive-control-plane`. Phase 2 cutover to `CorrosionStore` is a single
`Box<dyn ObservationStore>` trait-object swap.

Alternatives considered: build a new trivial in-process LWW map (rejected —
duplicates `SimObservationStore`); zero-row stub (rejected — lies about
the AC and blocks future internal heartbeats).

### D6 — Reconciler primitive slicing (ADR-0013)

**Recommendation**: Ship Slice 4 whole (trait + runtime + broker + DST
invariants + libSQL per-primitive memory + noop-heartbeat reconciler +
observed via `ClusterStatus`). 4A / 4B split remains a crafter-time
escape hatch if material complexity surfaces.

Alternatives considered: split 4A (trait+runtime+libSQL+noop-heartbeat)
+ 4B (broker+DST invariants) (rejected as DESIGN default — produces two
half-useful PRs; broker without its invariants has no proof of value).

### D7 — CLI HTTP client (ADR-0014)

**Recommendation**: Hand-rolled `reqwest`-based thin client; CLI and server
share Rust request/response types imported from `overdrive-control-plane::api`.
OpenAPI schema is a byproduct of the types, not a parallel contract.

Alternatives considered: OpenAPI Generator (Java — rejected, violates
Rust-throughout); Progenitor (rejected for Phase 1, deferred to Phase 2+
if a second Rust REST consumer appears).

### D8 — HTTP error mapping (ADR-0015)

**Recommendation**: One top-level `ControlPlaneError` enum with pass-through
`#[from]` embedding per `development.md`. Exhaustive `to_response()` function
maps variants to `(StatusCode, Json<ErrorBody>)`. Body shape is bespoke
`{error, message, field}` — a deliberate RFC 7807-compatible subset so
v1.1 upgrade is additive.

Idempotency: byte-identical re-submission at same intent-key is 200 OK
with original commit_index; 409 fires only when a *different* spec
collides.

Alternatives considered: RFC 7807 from day one (rejected as excess ceremony
for Phase 1); per-handler error enums (rejected — duplicates `#[from]`
embedding); collapse non-validation errors to 500 (rejected — breaks
actionable CLI rendering).

### Transverse decisions folded into ADR-0013

- **Reconciler runtime crate boundary**: `Reconciler` trait in
  `overdrive-core::reconciler`; `ReconcilerRuntime` + `EvaluationBroker` +
  libSQL provisioner in `overdrive-control-plane::reconciler_runtime`.
- **Per-primitive libSQL path**: `<data_dir>/reconcilers/<name>/memory.db`,
  isolation enforced by `ReconcilerName` regex + canonicalised-path
  `starts_with` check.
- **Async trait shape**: N/A at the `Reconciler` level — the trait is
  synchronous by design (purity contract). The runtime's internal scheduling
  loop uses native `async fn` against concrete types; no `async_trait` vs
  native-async-in-trait trade-off at the public trait surface.

## Tech stack — added / confirmed

| Dep | Version (proposed) | License | Role | Origin |
|---|---|---|---|---|
| `axum` | ≥0.7 | MIT | HTTP server framework | New (ADR-0008) |
| `axum-server` | ≥0.7 | MIT-or-Apache-2 | rustls TLS binding | New (ADR-0008) |
| `utoipa` | ≥5 | MIT-or-Apache-2 | OpenAPI schema derivation | New (ADR-0009) |
| `utoipa-axum` | ≥0.1 | MIT-or-Apache-2 | axum integration for utoipa | New (ADR-0009) |
| `libsql` | ≥0.5 | MIT | Per-primitive private memory | New (ADR-0013) |
| `rcgen` | 0.13 | MIT-or-Apache-2 | Ephemeral CA + leaf certs | Already in workspace; now used |
| `rustls` | 0.23 | MIT-or-Apache-2 | TLS 1.3 | Already in workspace; now used |
| `reqwest` | 0.12 | MIT-or-Apache-2 | CLI HTTP client | Already in workspace; now used |
| `hyper` | 1 | MIT | HTTP/1.1+HTTP/2 | Already in workspace; now used |

All MIT or MIT-or-Apache-2. No proprietary dependencies. All above 1k ★
and actively maintained.

## Constraints / System Constraints re-validated

Re-checked every system constraint from DISCUSS user-stories.md:

- **Reconcilers are pure**: ADR-0013 enforces at trait level + DST invariant
  `ReconcilerIsPure` + dst-lint inheritance.
- **Intent vs Observation compile-time boundary**: ADR-0011 keeps the split
  at the module level; existing non-substitutability tests unchanged.
- **rkyv canonicalisation**: ADR-0011 derives `rkyv::Archive` on every
  aggregate; `ContentHash::of(archived_bytes)` is the single spec-digest path.
- **External API REST + OpenAPI**: ADR-0008 + ADR-0009.
- **Aggregate serialisation lane-specific**: ADR-0011 — serde-JSON at REST
  edge, rkyv at IntentStore boundary.
- **Internal RPC tarpc / postcard-rpc (future)**: deferred out-of-scope;
  ADR-0008 explicitly avoids wiring choices that would force gRPC back in.
- **Auth posture unauthenticated local**: ADR-0010 (Phase 5 boundary).
- **Empty states honest**: Slice 5 AC + ADR-0012 make server observation
  reads round through a real store (not short-circuit lies).
- **Paradigm OOP Rust trait-based**: unchanged from phase-1-foundation
  ADR-0001 precedent; no re-prompt.

## Residuality / stressor posture

Phase 1 control-plane-core adds **one** named residual stressor beyond
phase-1-foundation's:

- **`axum` / `utoipa` major-version upgrade**: new workspace deps whose
  churn could break the router-derivation pipeline. Mitigation: pin both
  to exact versions in workspace `Cargo.toml`; `cargo xtask openapi-check`
  CI gate catches schema drift immediately on any version bump.

The phase-1-foundation residual stressor (turmoil upstream drift)
remains. No other stressors rise to the level requiring a hidden
residuality pass.

## Quality gates

Before handoff, all must pass:

- [x] Requirements traced to components (user stories → slices → ADRs)
- [x] Component boundaries with clear responsibilities (C4 Container + L3)
- [x] Technology choices in ADRs with alternatives (8 new ADRs)
- [x] Quality attributes addressed (brief.md §22 — ISO 25010 mapping)
- [x] Dependency-inversion compliance (ports-and-adapters; handlers depend
      on `&dyn IntentStore` / `&dyn ObservationStore` / `&ReconcilerRuntime`)
- [x] C4 diagrams (L1 unchanged, L2 updated, L3 component for
      `overdrive-control-plane`)
- [x] Integration patterns specified (REST + JSON, in-process trait-object
      adapters)
- [x] OSS preference validated (all deps MIT / Apache-2)
- [x] AC behavioural, not implementation-coupled (handlers own "validate →
      archive → commit"; no private-method references)
- [x] External integrations annotated: **NONE in Phase 1** — no contract
      tests recommended
- [x] Architectural enforcement tooling: `cargo xtask dst-lint` (existing),
      `cargo xtask openapi-check` (new), exhaustive `to_response` (Rust
      compiler), crate-class labelling (existing)
- [ ] Peer review completed and approved — pending separate `nw-solution-architect-reviewer` dispatch

## Upstream changes

**None.** All DISCUSS artifacts (user-stories, slices, outcome-kpis,
journey) are honoured as written — the ADRs layer on top without
changing any AC, story wording, or journey step. The DISCUSS-wave UC-1
(REST pivot) is codified, not re-decided.

If the peer reviewer surfaces a gap requiring an AC edit, a
`design/upstream-changes.md` will be created; otherwise the handoff to
DISTILL is clean.

## Handoff package for DISTILL (acceptance-designer)

- `docs/product/architecture/brief.md` (§14–§23 extension, C4 Container
  update, new component diagram)
- `docs/product/architecture/adr-0008-rest-openapi-transport.md`
- `docs/product/architecture/adr-0009-openapi-schema-derivation.md`
- `docs/product/architecture/adr-0010-phase-1-tls-bootstrap.md`
- `docs/product/architecture/adr-0011-aggregates-and-jobspec-collision.md`
- `docs/product/architecture/adr-0012-observation-store-server-impl.md`
- `docs/product/architecture/adr-0013-reconciler-primitive-runtime.md`
- `docs/product/architecture/adr-0014-cli-http-client-and-shared-types.md`
- `docs/product/architecture/adr-0015-http-error-mapping.md`
- All DISCUSS artifacts (user-stories, slices 1–5, journey, outcome-kpis)
  remain the AC source of truth

DISTILL wave may use concrete trait names, aggregate module paths, error
variant shapes, endpoint paths, and HTTP status codes in GIVEN clauses.
None of these require crafter consultation.

## Handoff package for DEVOPS (platform-architect)

- Paradigm: OOP (Rust trait-based).
- Architecture style: hexagonal (ports and adapters), single-process.
- New CI required checks:
  - `cargo xtask dst` (unchanged)
  - `cargo xtask dst-lint` (unchanged)
  - `cargo xtask openapi-check` (new — fails on schema drift)
  - `cargo nextest run --workspace` + `cargo test --doc --workspace`
  - Mutation-testing kill-rate gate ≥80% on Phase 1 applicable targets
- External integrations: **none**. No contract tests recommended at this
  phase. The annotation remains empty.
- Quality-attribute thresholds: DST wall-clock < 60 s (K1); lint-gate FP
  rate 0 (K2); OpenAPI drift 0 (new); CLI round-trip < 100 ms localhost
  (new, Slice 5 AC).
- Workspace deps to add: `axum`, `axum-server`, `utoipa`, `utoipa-axum`,
  `libsql` (versions per `Cargo.toml` pin).
- New crate: `crates/overdrive-control-plane`, class `adapter-host`
  (renamed from `adapter-real` on 2026-04-23; see ADR-0016).
- TLS/CA posture: in-process ephemeral (ADR-0010); no CI secret
  management in Phase 1.

## Open questions surfaced for user

None blocking handoff. The following design decisions have clear
recommendations in the ADRs but are RELATIVELY reversible if the user
wants to override:

1. **ADR-0009 utoipa vs aide** — utoipa recommended; aide is the viable
   runner-up. Swap is additive to the workspace but requires re-touching
   every handler's `#[aide::something]`/`#[utoipa::path]` annotation.
2. **ADR-0013 slice-4 whole vs split 4A / 4B** — DESIGN recommends whole;
   the user (or crafter) can split at implementation time without any
   ADR change (the split path is documented as an escape hatch).
3. **ADR-0014 hand-rolled reqwest vs Progenitor** — DESIGN recommends
   hand-rolled for Phase 1; Progenitor remains a Phase 2+ option if a
   second Rust REST consumer appears.

Other ADRs (0008, 0010, 0011, 0012, 0015) have no recommended override
path — they're pinned by upstream commitments (whitepaper § edits,
research R1–R5, or downstream design invariants).

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial DESIGN wave decisions for phase-1-control-plane-core. 8 new ADRs (0008–0015). Brief.md §14–§23 extension. C4 container diagram updated. New L3 component diagram for `overdrive-control-plane`. |
