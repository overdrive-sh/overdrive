# DISCUSS Wave Decisions — phase-1-control-plane-core

**Wave**: DISCUSS (product-owner)
**Owner**: Luna
**Date**: 2026-04-23
**Status**: COMPLETE — handoff-ready for DESIGN (solution-architect), pending peer review.

---

## Wizard decisions honoured

- **Feature type**: Cross-cutting (control-plane + CLI; consumed in follow-up `phase-1-first-workload` by a driver). Internal operator-facing UX.
- **Walking skeleton**: YES — this feature IS one skeleton slice (per the phase-1-foundation precedent), wired to the walking-skeleton outcome pinned in the wizard.
- **UX research depth**: Lightweight — internal operator-facing CLI + REST API. No web/desktop skills loaded beyond baseline; `nw-ux-tui-patterns` and `nw-ux-emotional-design` applied to CLI output and empty-state design.
- **JTBD analysis**: Skipped. Matches phase-1-foundation precedent. Ran Phase 2 (Journey) → Phase 2.5 (Story Mapping) → Phase 3 (Stories). Operator motivation is clear from whitepaper §4 + §18 — manage cluster intent, observe state.

## Pinned scope (from GitHub roadmap + wizard)

- **GH #9 [1.2]** Control-plane API surface — REST + OpenAPI (external) + `tarpc` / `postcard-rpc` (internal, future). This feature lands the external REST surface only; internal RPC ships with `phase-1-first-workload`.
- **GH #17 [1.9]** Reconciler primitive (trait + runtime + evaluation broker with cancelable-eval-set) — runtime provisions and manages per-primitive private libSQL DBs.
- **GH #18 [1.10]** CLI: `overdrive job submit`, `overdrive node list`, `overdrive alloc status`.

**Out of scope (deferred to `phase-1-first-workload`)**: GH #15 scheduler, GH #14 process driver, real workload execution, GH #21 job-lifecycle reconciler, GH #20 cgroup isolation + scheduler taint.

## Aggregate structs emerge HERE

Per the wizard directive and the GH #7 closure rationale, the aggregate structs (Job, Node, Allocation, Policy-stub, Investigation-stub) materialise in this feature alongside the API / reconciler / CLI consumers that need them. US-01 is the slice that lands them.

## Artifacts produced

### Product SSOT (additive)

- `docs/product/journeys/submit-a-job.yaml` — second canonical journey (alongside `trust-the-sim.yaml`). Added in this wave.
- `docs/product/jobs.yaml` — a new job statement (J-OPS-002) distilled from the walking-skeleton outcome, tagged `served_by_phase: 1` and `status: active`. Added in this wave.

### Feature artifacts (this directory)

- `docs/feature/phase-1-control-plane-core/discuss/journey-submit-a-job-visual.md` — ASCII + TUI mockups + emotional arc.
- `docs/feature/phase-1-control-plane-core/discuss/journey-submit-a-job.yaml` — structured journey with embedded Gherkin per step (NO `.feature` file — project rule honoured).
- `docs/feature/phase-1-control-plane-core/discuss/shared-artifacts-registry.md` — nine shared artifacts tracked, each with SSOT + consumers + integration risk + validation.
- `docs/feature/phase-1-control-plane-core/discuss/story-map.md` — 5-activity backbone, walking-skeleton identified, 5 carpaccio slices, priority rationale, scope-assessment PASS.
- `docs/feature/phase-1-control-plane-core/discuss/prioritization.md` — release priority and intra-release ordering.
- `docs/feature/phase-1-control-plane-core/slices/slice-1-aggregates-and-canonical-keys.md`, `slice-2-rest-service-surface.md`, `slice-3-api-handlers-intent-commit.md`, `slice-4-reconciler-primitive.md`, `slice-5-cli-handlers.md` — one brief per carpaccio slice (≤100 lines each). (Note: `slice-2-grpc-service-surface.md` exists as a rename marker pointing to `slice-2-rest-service-surface.md`; delete on next commit.)
- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md` — five LeanUX stories with a System Constraints header and embedded BDD.
- `docs/feature/phase-1-control-plane-core/discuss/outcome-kpis.md` — seven feature-level KPIs + measurement plan.
- `docs/feature/phase-1-control-plane-core/discuss/dor-validation.md` — 9-item DoR PASS on all 5 stories.
- `docs/feature/phase-1-control-plane-core/discuss/wave-decisions.md` (this file).

## Key decisions

### 1. No DIVERGE artifacts — grounded directly in whitepaper + phase-1-foundation precedent

No `docs/feature/phase-1-control-plane-core/diverge/` directory present. Wizard decision "JTBD: No". Jobs grounded in whitepaper §4 + §18 and in the phase-1-foundation job register (J-PLAT-001 through J-PLAT-003 remain active and are consumed here). **Risk**: operator motivation is inferred, not interview-validated. **Mitigation**: DIVERGE can be retrofitted if any of the walking-skeleton commands turns out wrong.

### 2. No `.feature` files — project rule enforced

Per `.claude/rules/testing.md` and the wizard prompt. All Gherkin lives inside `journey-*.yaml`, `user-stories.md`, or (later) `distill/test-scenarios.md`. The crafter translates to Rust `#[test]` / `#[tokio::test]` in `tests/acceptance/` per phase-1-foundation ADR-0005.

### 3. Walking skeleton = all 5 slices in Release 1

Because this feature IS one walking-skeleton slice of the larger project and the pinned walking-skeleton outcome requires every one of submit → commit → reconciler-registered → alloc-status honest, all five slices must ship before `phase-1-first-workload` can proceed. Slice 1 is a strict prerequisite; Slices 3 and 4 can run in parallel; Slice 5 is last.

### 4. CLI is unauthenticated in Phase 1 (local endpoint only)

Per the wizard decision referencing user memory: operator CLI auth is Phase 5 work (mTLS + operator SPIFFE IDs + Corrosion-gossiped revocation). This feature's CLI connects to `https://127.0.0.1:7001` by default (self-generated local-dev certificate, no authentication). Documented as a System Constraint in `user-stories.md` so DESIGN does not re-derive it and future reviewers see it explicitly.

### 5. Aggregate struct location: `overdrive-core` — emerging organically here

GH #7 (define core data model) was closed with "emerge organically here." US-01 lands `Job`, `Node`, `Allocation` in `overdrive-core`. `Policy` and `Investigation` ship as stubs sufficient to satisfy the whitepaper data-model reference without committing to Phase-specific fields.

### 6. `JobSpec` placeholder in `traits/observation_store.rs` needs a DESIGN decision

The placeholder struct currently in `crates/overdrive-core/src/traits/observation_store.rs` is used as a row shape in ObservationStore test scaffolding. DESIGN must decide whether to rename / replace it or treat it as an observation-side DTO distinct from the intent-side `Job` aggregate. Flagged in US-01 Technical Notes; not blocking the DISCUSS handoff.

### 7. Reconciler primitive ships with the `Action` enum variant for `HttpCall`, even though the runtime shim is Phase 3

Per `.claude/rules/development.md` §Reconciler I/O. The purity contract requires reconcilers to express external calls as `Action::HttpCall` rather than `async fn` bodies. The variant is part of the primitive surface in Phase 1; the shim that actually executes the call is Phase 3 (#3.11). Engineers writing reconcilers in Phase 1 can legitimately only ship reconcilers whose actions are already executable (Noop; StartWorkflow once 3.2 ships); this is acceptable and signals the ordering of the roadmap.

### 8. `ObservationStore` implementation for the Phase 1 server

`ObservationStore::read` in the real server needs *some* implementation. DESIGN picks between: (a) reusing `SimObservationStore` with a wiring layer; (b) a trivial in-process LWW map over the same trait; (c) a tiny stub that always returns zero rows until the real `CorrosionStore` lands in Phase 2. The walking-skeleton tests require only zero-row reads — any of the three satisfies the AC. Flagged; not blocking.

### 9. Scenario titles are business outcomes, not implementation

Every scenario title describes what the operator observes (e.g. "Submit round-trip through the REST API", "Broker collapses duplicate evaluations", "Empty `node list` renders an honest empty state"). None name internal method signatures, trait object types, or protocol tokens as the subject. Luna's contract with DISTILL.

### 10. No regression on phase-1-foundation guardrails

Every phase-1-foundation guardrail (DST wall-clock < 60s, lint-gate false-positive rate at 0, snapshot round-trip byte-identical, no banned API in core crates) applies to this feature verbatim. The three new DST invariants introduced by US-04 (`at_least_one_reconciler_registered`, `duplicate_evaluations_collapse`, `reconciler_is_pure`) compose with the existing catalogue, they do not replace it. The `reconciler_is_pure` invariant specifically closes a gap: the dst-lint gate is syntactic, so a sufficiently creative engineer could smuggle nondeterminism by calling through a re-export; the DST invariant catches it behaviourally.

### 11. Transport split is committed upstream; DESIGN codifies it via the first ADR

External API: REST + JSON over HTTP/2 with `rustls`, described by an OpenAPI 3.1 schema derived from Rust request / response types (via `utoipa` or `aide`). Internal RPC (node-agent control-flow streams; NOT shipped in this feature): `tarpc` or `postcard-rpc` — pure Rust, no `protoc`. The rationale is committed in whitepaper §3/§4 and in GH #9's rewritten body, so DISCUSS does not own the ADR — DESIGN's first ADR for this feature codifies the choice and names the concrete crates (axum, rustls, `utoipa` vs `aide`, the HTTP client shape for the CLI). See § Upstream Changes below for the full record.

## Scope assessment result

- **Stories**: 5 (under the 10-story ceiling).
- **Bounded contexts / crates**: 4 touched (`overdrive-core`, `overdrive-cli`, `overdrive-store-local` for the commit_index accessor, and one new crate — proposed `overdrive-control-plane` for the axum server; DESIGN picks whether the OpenAPI-derived request / response types live alongside the server or in a dedicated shared crate). Under the 3-context oversized signal when counted as bounded contexts (domain model, CLI, control plane, store).
- **Walking-skeleton integration points**: 4 — CLI→REST, REST→IntentStore, REST→ReconcilerRuntime, REST→ObservationStore. Under the >5 signal.
- **Estimated effort**: 4-6 focused days (each slice ≤1 day; US-04 ~1-2 at the upper end; Slices 3 and 4 parallelisable).
- **Multiple independent user outcomes worth shipping separately**: no — none of the slices delivers operator-observable value without the others. The reconciler primitive without the API is invisible; the API without the primitive leaves GH #17 unclosed.
- **Verdict**: **RIGHT-SIZED.** No split required.

## Risks surfaced

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| `JobSpec` placeholder in `observation_store.rs` collides with the new `Job` aggregate | Medium | Low | Flagged in US-01 Technical Notes; DESIGN picks rename / replace path; crafter's call not blocking DISCUSS. |
| US-04 scope may exceed 2 days once DESIGN digs into cancelable-eval-set semantics + libSQL provisioning | Medium | Low | Pre-described split (4A / 4B) in the DoR row. DESIGN can split if they discover material complexity. |
| ObservationStore implementation in the server goes unspecified until DESIGN picks between SimObservationStore / trivial in-process map / zero-row stub | Low | Low | All three satisfy the AC (walking-skeleton reads are zero-row). DESIGN picks. |
| Reconciler primitive ships `Action::HttpCall` variant before the runtime shim in Phase 3 — engineers could think HttpCall-returning reconcilers are executable | Low | Medium | Documented in Technical Notes; no Phase 1 reconciler beyond `noop-heartbeat` ships in this feature, so the risk is confined to future Phase 2 reconciler authors who read whitepaper §18 carefully. |
| Reconciler purity is enforced syntactically by dst-lint and behaviourally by `reconciler_is_pure` DST invariant — a very creative bypass could evade both | Low | High | Acknowledged; defence-in-depth via Anvil-style ESR verification is a future scope target (whitepaper §18 cites USENIX OSDI '24 *Anvil* for this). Phase 1 stops at syntactic + behavioural. |
| CLI output conventions drift from phase-1-foundation (summary-line-last, spinner policy) | Low | Low | `nw-ux-tui-patterns` applied; Luna's scenario titles and mockups enforce the convention. Peer review checks. |
| OpenAPI schema drifts from the Rust request / response types with no CI gate → CLI and server agree in review but disagree on the wire | Medium | High | Schema-lint gate in CI (`openapi-check`-style) is a first-class AC on US-02 and slice 2. First DESIGN ADR codifies the derivation tool (`utoipa` vs `aide`) and the gate shape. No hand-rolled request / response type on either side may shadow the schema-aligned form. |

## What DESIGN wave should focus on

1. **OpenAPI schema location, derivation, and CI gate**: `utoipa` or `aide` for derivation; checked-in document at `api/openapi.yaml` (or equivalent) regenerated on build with a schema-lint gate, vs generated-only at build time? Whichever is picked: the CI gate must fail the build if the generated schema drifts from the canonical document. First ADR.
2. **CLI HTTP client shape**: hand-rolled `reqwest`-style client against the OpenAPI schema, or fully OpenAPI-generated Rust client? Either is acceptable; consistency is not optional.
3. **`Job` aggregate vs `JobSpec` placeholder**: rename, replace, or keep-separate. DESIGN decides and documents via ADR.
4. **ObservationStore server impl**: pick one of the three options in Key Decision 8. ADR if the choice warrants it.
5. **Reconciler runtime crate boundary**: lives inside the control-plane server crate, or in `overdrive-reconciler` alongside `overdrive-core`? Phase 2+ reconciler authors will depend on it.
6. **Per-primitive libSQL path derivation**: the algorithm that turns a reconciler name into a filesystem path, ensuring isolation. DESIGN owns this.
7. **Async trait method shape** (`async_trait` vs native async-in-trait): one decision that applies across new async surfaces in this feature.
8. **Error enum taxonomy for the control-plane server**: one top-level `ControlPlaneError` or per-handler enums passing through via `#[from]`? Per development.md pass-through-not-duplication pattern. Maps to HTTP status codes through one `to_response(err)` function.
9. **`cluster status` JSON body shape**: exact JSON representation for `queued / cancelled / dispatched` broker counters. DESIGN's choice; stable between CLI and server via the OpenAPI schema.

## What is NOT being decided in this wave (deferred to DESIGN)

- Exact Rust module layouts inside the new crates.
- Error variant taxonomy beyond "thiserror + pass-through `#[from]`".
- Trait method signatures beyond what the AC semantically require.
- Concrete libSQL schema for per-primitive DBs (reconciler memory).
- Exact redb layout extensions for the new `commit_index()` accessor.
- Whether the evaluation broker's reaper is a proper reconciler in Phase 1 or an in-runtime loop.

## Upstream Changes

### UC-1: Transport pivot — gRPC/tonic → REST + OpenAPI (external) + tarpc/postcard-rpc (internal, future)

**Document change.** Multiple DISCUSS artifacts were rewritten on 2026-04-23 to reflect a committed upstream architectural decision:

- `user-stories.md` — US-02 retitled from "Control-plane gRPC service surface" to "Control-plane HTTP/REST service surface"; US-03 and US-05 reshaped around axum handlers, JSON bodies, and a REST HTTP client; a new cross-cutting System Constraint captures the REST + OpenAPI external / tarpc internal two-lane split and the JSON-at-edge / rkyv-at-store serialisation discipline.
- `slices/slice-2-grpc-service-surface.md` → `slices/slice-2-rest-service-surface.md` — renamed file with rewritten contents (OpenAPI 3.1 schema as single source of truth, axum router, schema-lint gate in CI, rustls transport).
- `slices/slice-3-api-handlers-intent-commit.md` — handlers become axum handlers with `Json<...>` extractors; status-code mapping is HTTP (400/404/409/500) not gRPC.
- `slices/slice-5-cli-handlers.md` — CLI calls the REST API via a thin HTTP client (hand-rolled `reqwest`-style or OpenAPI-generated); no tonic dependency, no protobuf toolchain.
- `shared-artifacts-registry.md` — `grpc_endpoint` → `rest_endpoint` (https + `/v1` path prefix); `grpc_service_shape` → `openapi_schema` (single source of truth, CI schema-lint gate).
- `journey-submit-a-job.yaml` and `journey-submit-a-job-visual.md` — step outputs / mockups reframed around REST/JSON/OpenAPI. Operator-visible CLI behaviour is unchanged — only the underlying wire format description changed.
- `story-map.md` — activity 2 retitled "Speak REST"; slice 2 rewritten accordingly.

**Reference original.** The first DISCUSS pass (same-day initial commit) framed the API as "Control-plane gRPC service surface" with one `.proto` as single source of truth, a tonic server, and a single generated Rust crate shared by CLI and server:

> *"Land one `.proto` definition for the control-plane service. Generate one Rust crate (proposed: `crates/overdrive-api`). Wire a tonic server that exposes `SubmitJob`, `DescribeJob`, `ClusterStatus`, `AllocStatus`, `NodeList`."* — original US-02 Solution

That framing is superseded by the REST + OpenAPI external / tarpc internal split.

**State new assumption.** The external control-plane API (CLI, operators, future SDKs) is **REST + JSON over HTTP/2 with `rustls`**, described by a single **OpenAPI 3.1** schema derived from the Rust request / response types via `utoipa` or `aide`. Internal node-agent control-flow streams (landing in `phase-1-first-workload`, NOT this feature) will use **`tarpc` or `postcard-rpc`** — pure Rust, no `protoc` in the toolchain.

Rationale (per whitepaper §3/§4 edits and GH #9 body):

- Overdrive is Nomad-shaped (Rust-throughout, single-binary, no committed public multi-language SDK story at v0.12). gRPC's cross-language value does not apply to the current target audience.
- REST is the universal public contract: curl-based exploration works against it; OpenAPI-generated SDKs exist for every mainstream language; HTTP-native auth (OIDC redirect flows, `Authorization: Bearer <biscuit>`) maps cleanly onto the Phase 5 / Phase 7 operator-auth story.
- tarpc (or postcard-rpc) keeps internal paths pure Rust per design principle 7 (*Rust throughout — no FFI to Go or C++ in the critical path*) without requiring `protoc` or a protobuf code-generation step.

**Preserve DISCOVER.** Not applicable — there are no DIVERGE / DISCOVER artifacts for this feature (see Key Decision 1). The persona (Ana), the goal (walking-skeleton submit → commit → reconciler registered → alloc status round-trip), the emotional arc, and the operator-visible CLI behaviour are unchanged by this pivot. Only the wire-format description of how the API is exposed changed; every UAT scenario re-validates.

**Where the rationale lives canonically.** Whitepaper §3 (architecture diagram — CLI/API row) and §4 (Control Plane — server framing). GH #9 body (rewritten). This wave-decisions record is the feature-local pointer, not the source of truth.

**Re-validation.** DoR was re-run against US-02 after the rewrite (see `dor-validation.md` Upstream Changes record). US-03 and US-05 re-validated only where AC scenarios were materially changed (the HTTP status codes in US-03; the HTTP-client framing and endpoint URL in US-05). Outcome KPIs and prioritization were re-reviewed; no changes required (all were transport-neutral).

## Handoff package for DESIGN (solution-architect)

- `docs/product/journeys/submit-a-job.yaml` — second canonical journey
- `docs/product/jobs.yaml` — updated with J-OPS-002 added as an active Phase 1 job
- `docs/feature/phase-1-control-plane-core/discuss/journey-submit-a-job-visual.md` + `.yaml` — journey artifacts
- `docs/feature/phase-1-control-plane-core/discuss/shared-artifacts-registry.md` — integration points
- `docs/feature/phase-1-control-plane-core/discuss/story-map.md` — carpaccio slices + priority
- `docs/feature/phase-1-control-plane-core/discuss/prioritization.md` — release priority
- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md` — LeanUX stories with AC + per-story BDD
- `docs/feature/phase-1-control-plane-core/discuss/outcome-kpis.md` — measurable KPIs
- `docs/feature/phase-1-control-plane-core/discuss/dor-validation.md` — 9-item PASS for all 5 stories
- `docs/feature/phase-1-control-plane-core/slices/slice-{1..5}-*.md` — slice briefs
- Reference: `docs/whitepaper.md` §4, §18
- Reference: `docs/product/architecture/brief.md` — existing Application Architecture section
- Reference: `docs/product/architecture/adr-0001..0007` — existing ADRs (DESIGN extends the index)
- Reference: `docs/feature/phase-1-foundation/discuss/user-stories.md` — System Constraints still apply
- Reference: `docs/feature/phase-1-foundation/design/wave-decisions.md` — reuse analysis precedent
- Reference: `docs/evolution/phase-1-foundation-evolution.md` — what shipped vs what defers

## Open questions surfaced for user

None blocking handoff. All eight items in "What DESIGN wave should focus on" are appropriate for the solution-architect to decide via ADR, not for the user to pre-answer.

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial DISCUSS wave decisions for phase-1-control-plane-core. |
| 2026-04-23 | Transport pivot recorded under § Upstream Changes (UC-1). Key Decision 11 added flagging the DESIGN ADR that will codify the REST + OpenAPI external / tarpc internal split. "What DESIGN wave should focus on" list updated accordingly (schema derivation + CI gate is now item 1; gRPC-specific items removed). |
