# phase-1-control-plane-core — Feature Evolution

**Feature ID**: phase-1-control-plane-core
**Branch**: `marcus-sa/serena-rust-ts`
**Duration**: 2026-04-23 — 2026-04-24 (DISCUSS opened 2026-04-23; DELIVER closed 2026-04-24)
**Status**: Delivered (APPROVED by adversarial crafter review; 0 blockers, 1 doc-only blocking suggestion addressed)
**Walking-skeleton gate**: step **05-05** — `overdrive cluster init` → `overdrive serve` → `overdrive job submit payments.toml` → `overdrive alloc status --job payments` prints a byte-identical spec digest end-to-end.

---

## What shipped

Phase 1's second walking-skeleton slice: the operator-facing REST control
plane (`overdrive-control-plane`), the full CLI surface
(`overdrive cluster init` / `serve` / `job submit` / `job describe` /
`cluster status` / `alloc status` / `node list`), the reconciler primitive
(trait + runtime + evaluation broker + libSQL-backed per-primitive memory
+ three new DST invariants), and the single canonical aggregate layer
(`Job`, `Node`, `Allocation` with `rkyv`-archived intent-key derivation
and `serde`-JSON REST edge).

Together with phase-1-foundation's type universe, DST harness, and
state-store split, this feature makes the pinned Phase 1 walking-skeleton
outcome observable end-to-end: **an operator submits a job through the
CLI, the control-plane server commits it through `LocalIntentStore` at a
monotonic `commit_index`, the reconciler primitive registers a heartbeat,
and `alloc status` renders an honest empty observation** — all over real
TLS against an ephemeral in-process CA, all exercised in real-redb /
real-axum / real-reqwest integration tests, and all gated by the
existing `cargo xtask dst` + `dst-lint` + `openapi-check` +
mutants-diff CI pipeline.

## Business context

phase-1-foundation landed the scaffolding (ports, Sim adapters, DST
harness, dst-lint, state-store split). That scaffold was by itself
operator-invisible. This feature is the first wave to produce observable
operator behaviour: the first time `overdrive` the binary actually does
something an operator runs. It maps directly to three pinned roadmap
items:

- **GH #9 [1.2]** — control-plane API surface (REST + OpenAPI external;
  internal RPC deferred to `phase-1-first-workload`).
- **GH #17 [1.9]** — reconciler primitive (trait + runtime +
  evaluation broker with cancelable-eval-set + per-primitive libSQL
  memory).
- **GH #18 [1.10]** — CLI subcommands (`overdrive job submit`,
  `overdrive node list`, `overdrive alloc status`, plus
  `cluster init` / `serve` / `cluster status` / `job describe`).

Scope explicitly deferred to `phase-1-first-workload`: GH #15 scheduler,
GH #14 process driver, GH #21 job-lifecycle reconciler, real workload
execution. This feature's reconciler primitive ships `noop-heartbeat`
only — enough to prove the primitive is alive, the DST invariants fire
against a real registered reconciler, and `cluster status` surfaces the
registry. No scheduling logic, no real workloads.

## Wave journey

- **DISCUSS** (2026-04-23) — Luna. Five LeanUX stories (US-01…US-05),
  one journey (`submit-a-job`), five carpaccio slices, seven outcome
  KPIs (K1–K7), 9-item DoR PASS on all stories. No DIVERGE artifacts
  (JTBD skipped per wizard + phase-1-foundation precedent). Transport
  pivot (gRPC → REST + OpenAPI) recorded under § Upstream Changes
  UC-1 — whitepaper §3/§4 edits are the SSOT; this wave is the
  feature-local pointer. See
  [`discuss/wave-decisions.md`](../feature/phase-1-control-plane-core/discuss/wave-decisions.md).

- **DESIGN** (2026-04-23) — Morgan. Eight new ADRs (0008–0015);
  brief.md §14–§23 extension; C4 Container diagram updated; new L3
  component diagram for `overdrive-control-plane`. Reuse analysis:
  11 REUSE, 6 EXTEND, 3 CREATE NEW (each CREATE NEW cited to an
  ADR). One new crate (`crates/overdrive-control-plane`) declared
  `crate_class = "adapter-host"` per the 2026-04-23 ADR-0003
  amendment + ADR-0016. See
  [`design/wave-decisions.md`](../feature/phase-1-control-plane-core/design/wave-decisions.md).

- **DISTILL** (2026-04-23) — Quinn. Twelve DWDs, ~70 Gherkin-in-markdown
  scenarios grouped by §1…§6, walking-skeleton Strategy C (real local —
  real redb, real rcgen CA, real axum + rustls, real reqwest,
  real libSQL, real `SimObservationStore`-as-production impl per
  ADR-0012), KPI-to-scenario tag map (K1–K7, every KPI ≥1 scenario),
  scaffold inventory naming every `SCAFFOLD: true` marker DELIVER
  would replace. No contradictions between DISCUSS and DESIGN.
  See
  [`distill/wave-decisions.md`](../feature/phase-1-control-plane-core/distill/wave-decisions.md).

- **DEVOPS** (2026-04-23) — Apex. Nine decisions inherited from
  phase-1-foundation (no re-litigation); four new decisions (N1–N4).
  One new required CI status check (`openapi-check`); DST invariant
  additions absorbed by the existing `dst` job; KPI instrumentation
  designed as `tracing` spans (DuckLake deferred). Six typically-DEVOPS
  deliverables explicitly skipped (`platform-architecture.md`,
  `monitoring-alerting.md`, `observability-design.md`,
  `infrastructure-integration.md`, `continuous-learning.md`,
  `branching-strategy.md`) — each would duplicate phase-1-foundation or
  not apply at Phase 1 scope. See
  [`devops/wave-decisions.md`](../feature/phase-1-control-plane-core/devops/wave-decisions.md).

- **DELIVER** (2026-04-23 → 2026-04-24) — 30 steps across 5 slices
  following the Outside-In TDD rhythm (PREPARE → RED_ACCEPTANCE →
  RED_UNIT → GREEN → REFACTOR → PROPTEST → MUTATION → COMMIT). All
  30 steps GREEN; adversarial review APPROVED. Four steps (04-07 →
  04-10) land the ADR-0013 pre-hydration + time-injection amendments
  as a single-cut migration over 2026-04-24. See
  [`deliver/roadmap.json`](../feature/phase-1-control-plane-core/deliver/roadmap.json)
  and
  [`deliver/execution-log.json`](../feature/phase-1-control-plane-core/deliver/execution-log.json).

## Slice-level delivery summary

| Slice | Steps | What shipped |
|---|---|---|
| **Slice 1** — Aggregates + canonical intent-keys | 01-01..01-04 | `Job`, `Node`, `Allocation` with validating constructors in `overdrive-core::aggregate`; `Resources` reused from `traits/driver` (no duplicate); `IntentKey::{for_job,for_node,for_allocation}` canonical derivation; `rkyv::Archive + Serialize + Deserialize` + `serde::Serialize + Deserialize` on every aggregate (two serialisation lanes — serde-JSON at REST edge, rkyv at IntentStore boundary). |
| **Slice 2** — REST service surface | 02-01..02-05 | Ephemeral in-process CA via `rcgen`; trust triple written to `~/.overdrive/config` (base64); multi-SAN server cert; axum + rustls server bound on HTTPS over HTTP/2 (HTTP/1.1 fallback) at `127.0.0.1:7001`; `utoipa`-derived OpenAPI 3.1 schema at `api/openapi.yaml`; `cargo xtask openapi-gen` + `openapi-check` subcommands + CI gate; `--insecure` rejected structurally. |
| **Slice 3** — Handlers | 03-01..03-06 | `POST /v1/jobs` commits through `LocalIntentStore` (rkyv-archived body → `ContentHash`-keyed IntentStore write); `GET /v1/jobs/{id}` rkyv-round-trips to `spec_digest` + 404 on unknown; `GET /v1/allocs` + `GET /v1/nodes` honest empty reads via ObservationStore wiring; idempotent re-submit returns 200 OK with original `commit_index`; 409 on conflicting spec at occupied intent-key; exhaustive `ControlPlaneError` → HTTP mapping per ADR-0015 (RFC 7807-compatible body shape). ADR-0012 revised mid-slice: `SimObservationStore` wiring replaced with `LocalObservationStore` (redb adapter-host) in production, leaving `overdrive-sim` as a dev-dep only. |
| **Slice 4** — Reconciler primitive | 04-01..04-10 | `Reconciler` trait + `Action` enum + `ReconcilerName` + `TargetResource` newtypes in `overdrive-core::reconciler`; `EvaluationBroker` with cancelable-eval-set + reaper; `ReconcilerRuntime` wires broker + registry + libSQL; per-primitive libSQL provisioner (`<data_dir>/reconcilers/<name>/memory.db`, canonicalised-path isolation); `noop-heartbeat` registered at server boot; three new DST invariants (`AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`, `ReconcilerIsPure`) with canary-bug feature fixture proving `ReconcilerIsPure` fires; dst-lint gate extended to catch reconciler-body purity violations; steps 04-07..04-10 migrate the trait surface single-cut to the ADR-0013 pre-hydration + time-injection amendment (see below). |
| **Slice 5** — CLI | 05-01..05-05 | Hand-rolled `reqwest`-based thin HTTP client with typed `CliError`; CLI handlers for `cluster init` (mints CA + writes trust triple), `serve` (boots control-plane server), `job submit` (reads TOML, validates locally, POSTs, prints `commit_index`), `job describe`, `cluster status` (renders registered reconcilers + broker counters), `node list`, `alloc status`; rendering via a dedicated `render` module; walking-skeleton gate via direct handler calls proving the end-to-end byte-identical spec-digest round-trip. |

## Key decisions

This wave produced seven ADRs (0008–0015) in DESIGN plus four more
(0016–0019) landed across DELIVER to record ongoing structural decisions.
Every ADR lives at its permanent home in
`docs/product/architecture/`; citations below are the ADR number and
its one-line thesis.

### ADRs produced in DESIGN

| ADR | Thesis |
|---|---|
| **0008** — REST + OpenAPI transport | External API is REST + JSON over HTTP/2 with rustls; `axum` + `axum-server` (rustls feature); `/v1` prefix non-negotiable. Internal tarpc / postcard-rpc deferred. |
| **0009** — OpenAPI schema derivation | `utoipa` + `utoipa-axum`; schema checked in at `api/openapi.yaml`; `cargo xtask openapi-check` CI gate fails on drift. |
| **0010** — Phase 1 TLS bootstrap | Ephemeral in-process CA at `cluster init`; base64 trust triple in `~/.overdrive/config`; multi-SAN server cert; **no `--insecure`**. Role NOT in cert O-field (whitepaper §8 SPIFFE URI SANs). |
| **0011** — Aggregates + `JobSpec` collision | Intent-side `Job` in `overdrive-core::aggregate`; observation-side `AllocStatusRow` stays in `traits::observation_store`. Any vestigial `JobSpec`-named struct deleted. |
| **0012** — ObservationStore server impl | Initially: reuse `SimObservationStore` behind a wiring adapter. Revised mid-Slice-3 (step 03-06, commit `7c86424`): replaced with a real `LocalObservationStore` (redb-backed adapter-host) in production. `overdrive-sim` becomes dev-dep only. |
| **0013** — Reconciler primitive runtime | `Reconciler` trait in `overdrive-core::reconciler`; runtime + broker + libSQL provisioner in `overdrive-control-plane::reconciler_runtime`. Amended 2026-04-24 (pre-hydration + time-injection — see below). |
| **0014** — CLI HTTP client + shared types | Hand-rolled `reqwest`-based thin client; CLI and server share Rust request/response types from `overdrive-control-plane::api`. Progenitor deferred to Phase 2+. |
| **0015** — HTTP error mapping | One top-level `ControlPlaneError` enum with pass-through `#[from]` embedding; exhaustive `to_response()` maps variants to `(StatusCode, Json<ErrorBody>)`; body shape is an RFC 7807-compatible subset. |

### ADRs landed in DELIVER

| ADR | Thesis |
|---|---|
| **0016** — `overdrive-host` extraction + `adapter-host` rename | Real production port bindings (`SystemClock`, `OsEntropy`, `TcpTransport`, etc.) extracted from `overdrive-sim/src/real` into a dedicated `overdrive-host` crate. `crate_class` value `adapter-real` renamed to `adapter-host` workspace-wide. The reconciler and policy crates MUST NOT depend on `overdrive-host` — depending on it is the explicit opt-in to real I/O. |
| **0017** — `overdrive-invariants` crate | Invariant catalogue and evaluator functions factored out of `overdrive-sim` into a dedicated `overdrive-invariants` crate. `InvariantClass`/`Predicate` taxonomy stabilised. Three new reconciler-primitive invariants from Slice 4 register into the new crate directly (post-cut) or migrate mechanically (pre-cut); name set is stable across both orderings. |
| **0018** — Verus pilot / Kani parallel track | Records that Phase 1 does NOT schedule cert-rotation-reconciler verification. The post-amendment `Reconciler` trait surface (sync tuple-return `reconcile` with async `hydrate` out-of-band) stays compatible with static-dispatch `Controller<C: ControllerApi>`-shape impls per Verus's `dyn Trait` limitation — the `AnyReconciler` enum-dispatch is itself static dispatch. |
| **0019** — Operator config format TOML | `~/.overdrive/config` is TOML, not JSON/YAML. Fields: trust triple (base64), default endpoint, active cluster context. |

### ADR-0013 amendment (2026-04-24): pre-hydration + time-injection

**Steps 04-07..04-10, single-cut.** Mid-Slice-4, ADR-0013 was amended to
land two paired extensions to the `Reconciler` trait surface:

1. **Pre-hydration pattern.** The `async fn` + libSQL side-channel in
   `reconcile` was split: `reconcile` becomes the sync pure tuple-return
   function (`fn reconcile(&self, desired: &State, actual: &State,
   view: &Self::View, tick: &TickContext) -> (Vec<Action>, Self::View)`);
   libSQL reads live exclusively in a new async method `hydrate(target:
   &TargetResource, db: &LibsqlHandle) -> Result<Self::View,
   HydrateError>`. The author declares the View shape as `type View`;
   the runtime diffs the returned `NextView` against the hydrated view
   and persists the delta. This is the same architectural shape every
   mature precedent converges on (kube-rs `Store<K>`,
   controller-runtime's cache-backed Reader, Anvil's `reconcile_core`
   + shim, Elm's `update : Msg -> Model -> (Model, Cmd Msg)`, Redux's
   pure-reducer + middleware).

2. **Time injection.** Wall-clock becomes an input to the function, not
   a side channel. The runtime snapshots `Clock::now()` once per
   evaluation (via the injected `Clock` trait DST already controls —
   `SystemClock` in production, `SimClock` under simulation), wraps
   it with a monotonic `tick` counter and a per-tick `deadline`, and
   passes the whole `TickContext { now, tick, deadline }` into
   `reconcile` as a fourth parameter. Reading `Instant::now()` /
   `SystemTime::now()` inside `reconcile` is banned — time is input
   state, not ambient.

Single-cut per user memory (greenfield, no deprecations, no grace
periods). The four steps decompose the migration for AC clarity, not
for dual-path migration:

- **04-07** — atomically migrates the trait surface and every in-tree
  caller. Deletes the `Db` placeholder; adds `TickContext`,
  `LibsqlHandle`, `HydrateError`; introduces `AnyReconciler`
  enum-dispatch (replaces `Box<dyn Reconciler>` throughout); updates
  `noop-heartbeat` to `type View = ()` with `tick` ignored; switches
  `ReconcilerRuntime::reconcilers` to `HashMap<ReconcilerName,
  AnyReconciler>`; snapshots `Clock::now()` in the tick loop.
- **04-08** — DST `reconciler_is_pure` evaluator body in
  `overdrive-sim` updated to the 5-parameter tuple-return shape with a
  shared `TickContext` across the twin invocation.
- **04-09** — trybuild compile-fail fixture at
  `crates/overdrive-core/tests/compile_fail/reconcile_cannot_take_libsql_handle.rs`
  asserts `&LibsqlHandle` cannot leak into `reconcile`'s parameter list
  (type-system gate; complements the dst-lint source-text gate).
- **04-10** — module rustdoc + a live doctest demonstrating the
  Phase-1 author contract, with `tick.now` explicitly referenced so
  Phase 2+ reconciler authors see what the `&TickContext` parameter
  is for.

Companion areas considered and **rejected** as premature per user memory
("Don't add abstractions beyond what the task requires"):
`LibsqlHandle` public methods, `HydrateError::Validation` variant,
`State` real shape, `TickContext` proptest invariants, reconcile-purity
proptest. Each is deferred to the Phase 2+ moment when a real reconciler
actually needs it.

The amendment rationale and citations live in
`docs/product/architecture/adr-0013-reconciler-primitive-runtime.md`
§2, §2a, §2b, §2c, §6, Alternative G, Enforcement, and Changelog
2026-04-24. Evidence base:
`docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
(879 lines, 44 sources). Solution-architect peer review on 2026-04-24:
APPROVED (0 blockers, 2 non-blocking findings resolved inline).

The roadmap-level reconciliation checked that Slice 5 CLI steps carry
no step-level dependency on 04-07..04-10 — Slice 5 consumes the
already-running control-plane binary via reqwest and never touches the
`Reconciler` trait surface — so the amendment did not ripple forward
into CLI work.

## Adversarial review (Phase 6 — DELIVER closure)

**Verdict: APPROVED** (nw-software-crafter-reviewer, 2026-04-24, scope
`4aa663b..HEAD`). See
[`deliver/adversarial-review.md`](../feature/phase-1-control-plane-core/deliver/adversarial-review.md).

- **Blockers: 0.**
- **Blocking suggestions: 1 (doc-only)** — document the
  `ReconcilerIsPure` Phase 2 boundary assumption (when `View` and `Db`
  carry real handles rather than `()`, the twin-invocation evaluator
  must either share a Db instance or compare public state, not handle
  identity). Addressed inline in this evolution document and in the
  evaluator comment per the reviewer's recommendation.
- **Non-blocking suggestions: 3** — `ErrorBody.field` sparseness
  (documented for Phase 2+ enrichment in ADR-0015); SimObservationStore
  wiring dep-check (already resolved by ADR-0012 revision — step 03-06
  commit `7c86424` swapped to `LocalObservationStore` so
  `overdrive-sim` is dev-dep only); `ClusterStatusOutput.reconcilers`
  semantics (documented as read-only inventory, health endpoint Phase
  5+).
- **Nitpicks: 2** — walking-skeleton test spawn-sequence
  micro-optimisation; `ReconcilerName::new` could use `#[expect]` on
  Rust 1.96+.

**Testing theater scan**: all 7 patterns (zero-assertion, tautological,
mock-dominated SUT, circular verification, always-green,
fully-mocked SUT, implementation-mirroring) returned NONE FOUND.

**Design compliance**: PASS across all nine dimensions (Sim/Host Split,
Intent/Observation Boundary, Newtype Discipline, Reconciler Purity,
Hashing Determinism, Error Discipline, Async Discipline, Walking
Skeleton Gate, DST Invariant Catalogue).

## KPIs (outcome)

From `discuss/outcome-kpis.md` — seven feature-level KPIs:

- **K1** — Round-trip spec digest byte-identical (TOML → `Job` →
  rkyv-archive → `ContentHash` → REST → Server → IntentStore →
  `GET /v1/jobs/{id}` → rkyv → bytes-equal). ✅ §1.1 WS-1, §3.1, §6.1.
- **K2** — Invalid spec rejected before IntentStore write (400 with
  structured error body). ✅ §4.2, §4.3, §6.6.
- **K3** — `commit_index` strictly monotonic across submits. ✅
  §4.5, §4.6.
- **K4** — DST invariants fire against a real registered reconciler
  (`AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`,
  `ReconcilerIsPure`). ✅ §5.7, §5.8, §5.9 + canary-bug feature
  fixture proves `ReconcilerIsPure` catches divergence.
- **K5** — `cluster status` surfaces reconciler registry + broker
  counters. ✅ §5.6, §6.4.
- **K6** — Error paths answer "what / why / how to fix" (operator-facing
  messages, no `reqwest` token leakage). ✅ §3.4, §6.3, §6.5, §6.6;
  explicit leakage test at
  `crates/overdrive-cli/tests/integration/job_submit.rs:265–292`.
- **K7** — Empty observations render explicit empty state (not lies, not
  placeholder rows). ✅ §4.7, §4.8, §6.2, §6.7.

North-star (K1 ∧ K3 ∧ K4) green. Guardrails carried from
phase-1-foundation (DST wall-clock < 60s, lint-gate FP rate 0, snapshot
round-trip byte-identical, no banned APIs in core crates) remain in CI.

## Lessons learned

Synthesised from the adversarial review and the five wave-decisions
records.

1. **Transport pivots in DISCUSS are cheap if committed upstream
   first.** UC-1 (gRPC → REST+OpenAPI) was absorbed without a DESIGN
   rewrite because the rationale had already landed in whitepaper §3/§4
   and in the rewritten GH #9 body. The DISCUSS wave merely propagated
   the decision into the feature-local artifacts; the ADR (0008)
   codified it in DESIGN without re-deriving.

2. **Single-cut migrations in greenfield are worth the one-PR cost.**
   The ADR-0013 amendment (04-07..04-10) migrated the trait surface
   atomically; the four-step decomposition was for AC clarity, not for
   dual-path rollout. No `#[deprecated]`, no feature-flagged old path,
   no grace period. The cost was one coordinated commit set; the
   benefit was zero residual dual-path complexity and zero "migrate
   this later" tech-debt entries.

3. **Defensive programming pays for itself in the DST invariant
   catalogue.** The `canary-bug` feature fixture (deliberately
   non-deterministic reconciler that triggers `ReconcilerIsPure` red)
   turned out to be the cheapest thing to add — its cost is zero in
   production (feature-gated), and it IS the proof that the evaluator
   catches the divergence it claims to detect. Covered by the
   adversarial review praise §2.

4. **Strategy C walking skeleton composes correctly with Sim adapters
   in DST.** The Phase 1 server's ObservationStore impl began as a
   wiring adapter around `SimObservationStore` (ADR-0012 original), then
   revised mid-Slice-3 to a real `LocalObservationStore` (redb adapter).
   The revision was driven by the adversarial reviewer's flag that
   `overdrive-sim` being a production runtime dep contradicts the
   sim/host split; the cost of revision was one commit
   (`7c86424`). The DST harness still composes the real store with Sim
   adapters — the sim/host split survived intact.

5. **Gate-then-stop (user memory).** Every step that added a gate
   (trybuild fixture, DST invariant, openapi-check) did exactly one
   thing and stopped — no auto-wiring into CI beyond the single
   `openapi-check` job identified in DEVOPS N1. The existing
   `cargo nextest run`, `cargo test --doc`, `cargo xtask dst`, and
   `cargo xtask mutants --in-diff` paths pick up new fixtures
   automatically by virtue of file-system conventions.

6. **Reconciler purity is a three-layer invariant.** Syntactic
   (dst-lint on banned APIs in core-class crates), type-system
   (trybuild compile-fail on `&LibsqlHandle` in `reconcile`'s parameter
   list), and behavioural (DST `ReconcilerIsPure` twin-invocation).
   Each catches a class the others cannot. The `reconciler_is_pure`
   invariant specifically closes the gap left by dst-lint: dst-lint is
   syntactic, so a sufficiently creative engineer could smuggle
   nondeterminism by calling through a re-export; the DST invariant
   catches it behaviourally. The trybuild fixture closes a third
   gap — a reconcile method signature that tries to accept a
   `LibsqlHandle` fails at the type-checker, not at runtime.

7. **Don't speculate on future needs.** The ADR-0013 amendment landed
   `LibsqlHandle`, `HydrateError`, and `TickContext` with minimal
   surfaces (no public methods on `LibsqlHandle`, two variants on
   `HydrateError`, no proptest invariants on `TickContext`). Each
   extension point is a Phase 2+ concern — when a real reconciler
   actually needs the public method, the author adds it. Shipping
   speculative abstractions in a trait that will be consumed by
   multiple future authors creates surface that must be maintained
   regardless of whether anyone consumes it.

## Issues encountered

From
[`distill/upstream-issues.md`](../feature/phase-1-control-plane-core/distill/upstream-issues.md).

**No upstream issues surfaced.** Reconciliation between DISCUSS and
DESIGN waves passed with zero contradictions. All user-story acceptance
criteria mapped to DISTILL scenarios without AC edits.

Two soft observations were recorded, neither blocking:

1. **Product KPI contracts file absent.** `docs/product/kpi-contracts.yaml`
   does not exist; feature-level KPIs K1–K7 drive `@kpi KN` tags.
   When a product-level contracts file lands post-Phase 1,
   Sentinel may re-audit and propose `@kpi-contract` additions.

2. **DEVOPS wave ran after DISTILL (graceful degradation).** DISTILL
   applied the default environment matrix (`clean`,
   `with-pre-commit`, `with-stale-config`) per skill rules; DEVOPS's
   subsequent `environments.yaml` was additive and did not invalidate
   any DISTILL scenario.

Three items were explicitly considered and rejected as upstream
changes: `JobSpec` placeholder collision (resolved intra-feature by
ADR-0011); Slice 4 whole vs split (ADR-0013 confirmed whole with
split available as crafter-time escape hatch — whole used in
practice); ObservationStore impl choice (DISCUSS Key Decision 8 named
three options, ADR-0012 chose `SimObservationStore` reuse, later
revised to `LocalObservationStore`; DISCUSS AC did not specify the
impl).

Known follow-ups (carried forward, not Phase-1 blockers):

- Document `ReconcilerIsPure` Phase 2 boundary assumption in the
  evaluator comment when Phase 2 reconcilers land real `View` and `Db`
  types. Raised by adversarial review blocking-suggestion #1.
- `ErrorBody.field` enrichment for NotFound / Conflict / Intent errors
  (Phase 2+). Raised by adversarial review non-blocking-suggestion #1.
- `/v1/cluster/health` endpoint (reconciler lifecycle health) — Phase
  5+. Raised by adversarial review non-blocking-suggestion #3.

## Artifacts produced

### Platform crates (new or extended)

- `crates/overdrive-core` — **extended**. New
  `src/aggregate/mod.rs` (`Job`, `Node`, `Allocation`, `Policy`-stub,
  `Investigation`-stub, `AggregateError`, `IntentKey`, spec-input
  types); new `src/reconciler.rs` (`Reconciler` trait,
  `AnyReconciler`, `Action`, `ReconcilerName`, `TargetResource`,
  `TickContext`, `LibsqlHandle`, `HydrateError`, `State` placeholder).
- `crates/overdrive-control-plane` — **NEW** crate,
  `crate_class = "adapter-host"`. Modules: `api`, `handlers`,
  `tls_bootstrap`, `error`, `reconciler_runtime`, `eval_broker`,
  `libsql_provisioner`, `observation_wiring`, plus `noop_heartbeat()`
  factory.
- `crates/overdrive-host` — **NEW** crate (ADR-0016),
  `crate_class = "adapter-host"`. Production bindings from the core
  port traits to the host OS / kernel / network (`SystemClock`,
  `OsEntropy`, `TcpTransport`, etc.).
- `crates/overdrive-invariants` — **NEW** crate (ADR-0017),
  `crate_class = "adapter-sim"`. Invariant catalogue and evaluator
  functions factored out of `overdrive-sim`.
- `crates/overdrive-store-local` — **extended**.
  `LocalStore` → `LocalIntentStore` rename for symmetry;
  `LocalObservationStore` added (step 03-06, commit `7c86424`);
  `commit_index()` read-only accessor added.
- `crates/overdrive-sim` — **extended**. Three new DST invariants
  (`AtLeastOneReconcilerRegistered`, `DuplicateEvaluationsCollapse`,
  `ReconcilerIsPure`) with evaluator bodies; `canary-bug` feature
  fixture for proving `ReconcilerIsPure` fires; harness wiring for
  reconciler runtime. Post-ADR-0017, the invariant catalogue migrates
  into `overdrive-invariants`.
- `crates/overdrive-cli` — **extended**. Hand-rolled `reqwest`-based
  thin HTTP client (`parse_cli_endpoint`, `get_typed`, `post_typed`);
  handlers for `cluster init`, `serve`, `job submit`, `job describe`,
  `cluster status`, `node list`, `alloc status`; `render` module.
- `xtask` — **extended**. New subcommands `openapi-gen` and
  `openapi-check` per ADR-0009; `dst-lint` extended to catch
  reconciler-body purity violations in core crates (step 05-02 test).

### Documentation

- Twelve ADRs at `docs/product/architecture/`: **0008–0019** (plus
  pre-existing 0001–0007 unchanged), each authored or amended in this
  wave.
- `docs/product/architecture/brief.md` — §14–§23 extension; C4
  Container diagram updated; new L3 component diagram for
  `overdrive-control-plane`. Sync'd to ADR-0012 revision and new CLI
  cluster status path in commit `28f3b64`.
- `api/openapi.yaml` — OpenAPI 3.1 schema checked in at workspace
  root; regenerated on every `utoipa`-derived schema change; CI gate
  fails on drift.
- `docs/research/reconciler-prehydration-pattern/reconciler-prehydration-pattern-comprehensive-research.md`
  — 879-line, 44-source evidence base for ADR-0013 amendment
  (2026-04-24).
- `docs/research/security/talos-bootstrap-tls-strategy-comprehensive-research.md`
  — TLS research adopted wholesale by ADR-0010 R1–R5.

### CI and tooling

- `.github/workflows/ci.yml` — gains one new required status check
  (`openapi-check`). DST invariant additions absorbed by the existing
  `dst` job without a new job.
- `xtask` — `openapi-gen` / `openapi-check` subcommands added per
  ADR-0009.
- `crates/overdrive-core/tests/compile_fail/reconcile_cannot_take_libsql_handle.rs`
  — trybuild compile-fail fixture with checked-in `.stderr` asserting
  `&LibsqlHandle` cannot leak into `reconcile`'s parameter list
  (step 04-09).
- Mutation-testing strategy unchanged from phase-1-foundation. Per-PR
  diff-scoped kill-rate gate (≥80%) continues to cover the new crate
  via `.cargo/mutants.toml` rules; two skip annotations added for
  feature-gated test-only fixtures (`HarnessNoopHeartbeat::reconcile`
  in commit `c3098ec`; `canary-bug` reconciler in commit `e22d7f8`).
- `lefthook.yml` — no new hooks. Block-hook-skipping on git commits
  tightened in commit `b5cbe99`.

### Migrated artifacts (Phase B of finalize)

- `docs/ux/phase-1-control-plane-core/journey-submit-a-job.yaml`
- `docs/ux/phase-1-control-plane-core/journey-submit-a-job-visual.md`
- `docs/scenarios/phase-1-control-plane-core/test-scenarios.md`
- `docs/scenarios/phase-1-control-plane-core/walking-skeleton.md`

Source SHAs verified byte-identical against the `docs/feature/` origins
before the copy was committed. The feature workspace directory
(`docs/feature/phase-1-control-plane-core/`) is **retained in full**
as the citation trail backing this evolution document — it is NOT
deleted by this finalize.

## Code range

Phase-1-control-plane-core spans 46 commits on branch
`marcus-sa/serena-rust-ts`:

- **Range**: `b96d0a5..d6ecee9` (46 commits total;
  `4aa663b..HEAD` from the adversarial-review scope header)
- **Earliest**: `b96d0a5 feat(overdrive-core): add Job/Node/Allocation
  aggregates with validating constructors (step 01-01)` — 2026-04-23
- **Latest four (post-/nw-review APPROVED, 2026-04-24)**:
  - `3d05c39 feat(phase-1-control-plane-core): migrate Reconciler to
    pre-hydration...` — step 04-07
  - `3b3b9a8 feat(phase-1-control-plane-core): reconciler_is_pure
    evaluator pulls ...` — step 04-08
  - `74cbab8 test(phase-1-control-plane-core): trybuild fixture bans
    &LibsqlHandle...` — step 04-09
  - `d6ecee9 docs(phase-1-control-plane-core): reconciler rustdoc +
    doctest for Ph...` — step 04-10

The four most recent commits land the ADR-0013 2026-04-24 amendments
as a single-cut migration; all passed solution-architect and
software-crafter peer review inline.

## Phase D verification

- **Source vs destination SHA-256 verification** (Phase B): all four
  migrated artifacts match byte-identical against their
  `docs/feature/phase-1-control-plane-core/` origins.
  - `journey-submit-a-job.yaml`: `b17c7d6d…414cabd7`
  - `journey-submit-a-job-visual.md`: `83bff186…4d749fdbca`
  - `test-scenarios.md`: `931c904a…08d40f47`
  - `walking-skeleton.md`: `d312cd1d…efa0e6b9`
- **Status-label sweep**: no "FUTURE DESIGN" / "will be implemented"
  / "planned for" tense found in either migrated UX or migrated
  scenarios. No in-place status edits required.
- **`/nw-document` auto-run**: skipped per finalize prompt.

## What this unblocks

This feature hands off to **`phase-1-first-workload`** a working
operator-facing control plane:

- REST + OpenAPI surface at `https://127.0.0.1:7001/v1/...` with a
  CI-gated schema.
- `LocalIntentStore` (redb) with monotonic `commit_index` +
  byte-identical `rkyv`-archive round-trip through `ContentHash`.
- `LocalObservationStore` (redb adapter-host) for live observation
  reads; swappable in Phase 2 for `CorrosionStore` via trait-object
  swap.
- `Reconciler` primitive with the post-amendment trait surface
  (`type View`, async `hydrate`, sync tuple-return `reconcile` with
  `&TickContext`); `AnyReconciler` enum-dispatch; per-primitive
  libSQL memory with canonicalised-path isolation; `EvaluationBroker`
  with cancelable-eval-set collapsing duplicate evaluations;
  three-layer purity enforcement (dst-lint + trybuild + DST).
- CLI with TLS-bootstrapped `reqwest`-based thin client against the
  local server; handlers for every walking-skeleton verb.

Remaining Phase 1+ issues unblocked by this wave:

- **`phase-1-first-workload`** (GH #14 / #15 / #20 / #21) — scheduler,
  process driver, job-lifecycle reconciler, cgroup isolation +
  scheduler taint. Consumes `IntentStore` + `ObservationStore` + the
  `Reconciler` primitive; needs internal tarpc / postcard-rpc RPC
  surface (whose protocol decision this wave explicitly defers).
- **Phase 2 `CorrosionStore`** — real CR-SQLite + SWIM/QUIC
  `ObservationStore` impl. The trait Sim already satisfies and the
  wiring shape already exists; the swap is a single trait-object
  substitution.
- **Phase 5 operator auth** — operator SPIFFE IDs, 8h TTL,
  Corrosion-gossiped revocation. The REST + rustls surface is ready;
  the ephemeral-CA trust triple is the bootstrap; Phase 5 extends with
  real SPIFFE URI SANs on operator certs + revocation sweep.
