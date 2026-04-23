# Story Map — phase-1-control-plane-core

## User: Ana, Overdrive platform engineer (distributed-systems SRE)

## Goal: Run `overdrive job submit` against a local walking-skeleton control plane and trust that the spec round-trips through a real IntentStore, that a reconciler primitive is registered, and that `overdrive alloc status` honestly reflects what the platform knows.

## Backbone

User activities, left-to-right in chronological order over the lifetime of the feature:

| 1. Model the job shape | 2. Speak REST | 3. Commit intent | 4. Host reconcilers | 5. Drive from the CLI |
|---|---|---|---|---|
| Define the Job / Node / Allocation / Policy aggregate structs backed by the phase-1-foundation newtypes | Define the control-plane REST API (axum + rustls) with an OpenAPI 3.1 schema as single source of truth for SubmitJob, DescribeJob, ClusterStatus, AllocStatus, NodeList | Wire the axum handlers to IntentStore::put/get/txn through the canonical intent-key function | Provide the Reconciler trait + runtime + evaluation-broker-with-cancelable-set | Replace the CLI stub with real SubmitJob, DescribeJob, ClusterStatus, AllocStatus, NodeList handlers |

## Ribs (tasks under each activity)

### 1. Model the job shape

- 1.1 Job aggregate struct (JobId, Resources, driver hint, replicas) with rkyv + serde derives *(Walking Skeleton)*
- 1.2 Node aggregate struct (NodeId, Region, capacity, labels) with rkyv + serde *(Walking Skeleton)*
- 1.3 Allocation aggregate struct (AllocationId, JobId, NodeId, AllocationState) *(Walking Skeleton)*
- 1.4 Policy aggregate struct (PolicyId, scope, Rego/WASM module reference) — placeholder only in Phase 1
- 1.5 Investigation aggregate struct (InvestigationId, trigger, correlation_key) — placeholder only in Phase 1
- 1.6 Canonical intent-key derivation (`IntentKey::for_job(&JobId)` etc.) *(Walking Skeleton)*

### 2. Speak REST

- 2.1 OpenAPI 3.1 schema as single source of truth for the control-plane wire contract (recommended: `api/openapi.yaml`, derived from Rust types via `utoipa` or `aide`) *(Walking Skeleton)*
- 2.2 Schema-lint gate in CI — fails the build if the generated schema drifts from the checked-in document *(Walking Skeleton)*
- 2.3 Server bind / listen / serve over HTTP/2 with `rustls` (axum + tokio + rustls); default endpoint matches CLI default *(Walking Skeleton)*
- 2.4 Structured errors mapped from typed `thiserror` variants to HTTP status codes + JSON error bodies *(Walking Skeleton)*

### 3. Commit intent

- 3.1 axum SubmitJob handler rkyv-archives the Job and calls IntentStore::put through LocalStore *(Walking Skeleton)*
- 3.2 axum DescribeJob / AllocStatus / NodeList handlers read back via IntentStore::get / ObservationStore::read *(Walking Skeleton)*
- 3.3 LocalStore exposes `commit_index()` accessor (monotonic redb transaction sequence) *(Walking Skeleton)*
- 3.4 Validating-constructor gate before any write (rejects malformed IDs / bytes with `400 Bad Request`) *(Walking Skeleton)*

### 4. Host reconcilers

- 4.1 `Reconciler` trait in overdrive-core — pure `reconcile(desired, actual, db) -> Vec<Action>` per whitepaper §18 *(Walking Skeleton)*
- 4.2 `Action` enum (at least the StartWorkflow / HttpCall / Noop variants; job-lifecycle actions deferred) *(Walking Skeleton)*
- 4.3 `ReconcilerRuntime` that registers reconcilers at boot and exposes `registered()` *(Walking Skeleton)*
- 4.4 `EvaluationBroker` with cancelable-eval-set keyed on `(reconciler, target)` — collapses duplicates *(Walking Skeleton)*
- 4.5 Per-primitive private libSQL DB provisioning (per-reconciler `Db`), passed to `reconcile(...)` *(Walking Skeleton)*
- 4.6 A built-in `noop-heartbeat` reconciler that registers + drains evaluations as a living proof the contract holds *(Walking Skeleton)*
- 4.7 DST invariants: `at_least_one_reconciler_registered`, `duplicate_evaluations_collapse` *(Walking Skeleton)*

### 5. Drive from the CLI

- 5.1 `overdrive job submit <spec>` reads TOML, constructs Job, calls API *(Walking Skeleton)*
- 5.2 `overdrive node list` reads node_health rows via API (Phase 1: may return empty — no node agent yet) *(Walking Skeleton)*
- 5.3 `overdrive alloc status --job <id>` or `--alloc <id>` reads alloc_status rows *(Walking Skeleton)*
- 5.4 `overdrive cluster status` reads commit_index + reconciler_registry + evaluation_broker_state *(Walking Skeleton)*
- 5.5 Actionable error rendering ("what / why / how to fix") for connection, validation, and server-side failures *(Walking Skeleton)*
- 5.6 Explicit empty-state rendering when observation tables are empty *(Walking Skeleton)*

## Walking Skeleton

The thinnest end-to-end slice that connects ALL five activities. If any activity has nothing above the skeleton line, the engineer cannot reach "`overdrive job submit` → commit index → `overdrive alloc status`" green:

| 1. Model | 2. REST | 3. Intent | 4. Reconcilers | 5. CLI |
|---|---|---|---|---|
| 1.1 + 1.2 + 1.3 + 1.6 | 2.1 + 2.2 + 2.3 + 2.4 | 3.1 + 3.2 + 3.3 + 3.4 | 4.1 + 4.2 + 4.3 + 4.4 + 4.5 + 4.6 + 4.7 | 5.1 + 5.2 + 5.3 + 5.4 + 5.5 + 5.6 |

This bundle IS the walking skeleton for the control-plane-core feature. The follow-up feature `phase-1-first-workload` adds scheduler + process driver + job-lifecycle reconciler, at which point `overdrive node list` and `overdrive alloc status` start returning non-empty results.

## Release slices (elephant carpaccio)

Each slice ≤1 day of focused work, ships demonstrable end-to-end value, carries a learning hypothesis.

### Slice 1 — Job / Node / Allocation aggregates + canonical intent keys

**Outcome**: Ana imports `Job`, `Node`, `Allocation` from `overdrive-core`, constructs them through validating constructors (newtypes), archives them via rkyv, and derives intent keys through the single canonical function.

**Target KPI**: 100% of the three aggregates round-trip through rkyv archive → access without loss; `IntentKey::for_job(&JobId)` matches the CLI-computed and API-used keys byte-for-byte.

**Hypothesis**: "If the aggregate structs and canonical key function don't exist with rkyv-archived determinism, the commit index, spec digest, and intent key shown by the CLI will drift from what the control plane commits — breaking the walking-skeleton round-trip invariant."

**Delivers**: US-01.

### Slice 2 — Control-plane REST service surface

**Outcome**: A single OpenAPI 3.1 schema — derived from the Rust request / response types via `utoipa` or `aide` — is the single source of truth for the control-plane wire contract. An axum router binds over HTTP/2 + `rustls`, exposes SubmitJob, DescribeJob, ClusterStatus, AllocStatus, NodeList endpoints under the `/v1` prefix, and maps typed `thiserror` errors to HTTP status codes with structured JSON bodies.

**Target KPI**: Round-trip `POST /v1/jobs → GET /v1/jobs/{id}` returns the same Job bytes on any valid input; every `thiserror` variant in the submit path has an HTTP status mapping (tested); schema-lint gate green on every PR; zero hand-rolled request / response types shadow the schema-aligned ones.

**Hypothesis**: "If the CLI and server compile against types that drift from the OpenAPI schema, the walking skeleton silently fails on the first field mismatch. A schema that is always derived from the Rust types and always checked against a canonical document is the only structural mitigation."

**Delivers**: US-02.

### Slice 3 — API handlers commit to IntentStore + ObservationStore reads

**Outcome**: SubmitJob axum handler archives the Job and writes through `IntentStore::put` on `LocalStore`. DescribeJob, AllocStatus, and NodeList axum handlers read back through `IntentStore::get` and `ObservationStore::read`. LocalStore exposes a monotonic `commit_index()` surfaced in JSON responses. Handlers map typed errors to HTTP status codes (400 / 404 / 409 / 500) with structured JSON error bodies.

**Target KPI**: POST `/v1/jobs` then GET `/v1/jobs/{id}` returns bytes equal to what was submitted; `commit_index` strictly increases across successive submits; validating constructors reject malformed input with `400 Bad Request` before any write.

**Hypothesis**: "If the API commits a spec and Describe can't read it back byte-identical, the IntentStore contract is broken and every downstream claim about durability / audit / idempotency fails."

**Delivers**: US-03.

### Slice 4 — Reconciler primitive: trait + runtime + evaluation broker

**Outcome**: `Reconciler` trait lives in overdrive-core with the pure-function contract. `ReconcilerRuntime` registers at-least-one reconciler (`noop-heartbeat`) at boot. `EvaluationBroker` collapses duplicate `(reconciler, target)` evaluations into a cancelable set and drains them. Per-primitive private libSQL DBs are provisioned and passed to `reconcile(...)`. New DST invariants assert these contracts hold.

**Target KPI**: DST passes `at_least_one_reconciler_registered` on every run; `duplicate_evaluations_collapse` holds under N concurrent evaluations on the same key; private libSQL DB path is distinct per reconciler (filesystem isolation); the noop-heartbeat reconciler drains N evaluations without panic.

**Hypothesis**: "If the reconciler primitive isn't shipped with storm mitigation from day one, Phase 2+ reconciler scale becomes a Nomad-shaped incident. The mitigation is cheap to ship native; retrofitting it after a production incident is not."

**Delivers**: US-04.

### Slice 5 — CLI handlers for job / alloc / cluster / node

**Outcome**: The CLI stub is replaced with real handlers that call the REST API over HTTP/2 + rustls (hand-rolled `reqwest`-style client or OpenAPI-generated Rust client per DESIGN's pick). `job submit` reads a TOML spec, constructs a Job, POSTs JSON to `/v1/jobs`, prints commit_index and canonical intent_key. `alloc status` GETs `/v1/jobs/{id}` + `/v1/allocs`, renders zero-row observation with an explicit empty state pointing at the next feature. `cluster status` GETs `/v1/cluster/info`, shows the reconciler registry and evaluation broker counters. `node list` GETs `/v1/nodes`, renders zero-row observation with the same honest empty-state pattern.

**Target KPI**: Round-trip acceptance test: submit a spec via CLI, then `alloc status --job <id>` returns the same spec_digest the CLI computed locally; CLI exit codes map cleanly (0 success, 1 generic error, 2 usage error); error messages answer "what / why / how to fix".

**Hypothesis**: "If the CLI silently hides zero-row observation or computes the spec digest differently from the server, operators lose trust in the platform's honesty long before the platform has anything to lie about."

**Delivers**: US-05.

## Priority Rationale

Ordering by outcome impact and dependencies. Every slice in Release 1 is on the walking skeleton — all five are required to reach `overdrive job submit → overdrive alloc status` round-trip green.

| Priority | Slice | Depends on | Why this order |
|---|---|---|---|
| 1 | Slice 1 — Aggregates + canonical keys | phase-1-foundation newtypes | Foundation: the API, the store, and the CLI all need the same Job struct and the same key-derivation function. If either drifts, every downstream consistency claim breaks. |
| 2 | Slice 2 — REST service surface | Slice 1 | The OpenAPI schema and axum handlers transport the aggregates. The server cannot bind and answer before the request / response types compile and the schema-lint gate is green. |
| 3 | Slice 3 — API handlers + IntentStore commit | Slices 1, 2 | SubmitJob and DescribeJob handlers consume both the aggregates and the axum router + OpenAPI schema. This slice proves the intent-commit path end-to-end on localhost, minus the CLI. |
| 4 | Slice 4 — Reconciler primitive | Slice 1 (Action enum depends on IDs) | Independent of Slices 2-3 mechanically — a reconciler runtime can exist without the API — but part of the walking skeleton because GH #17 is explicitly in scope. Can run in parallel with Slice 3. |
| 5 | Slice 5 — CLI handlers | Slices 2, 3, 4 | Last by definition — the CLI is the driving port and needs every other slice behind it. This is the acceptance gate for the whole feature. |

All five ship in Release 1. There is no Release 2 for this feature.

## Scope Assessment: PASS — 5 stories, 4 crates touched, estimated 4-6 days

- **Story count**: 5 stories (US-01 through US-05). Well within the ≤10 ceiling.
- **Bounded contexts / crates**: 4 (`overdrive-core`, `overdrive-cli`, `overdrive-store-local`, and one new crate — `overdrive-control-plane` for the axum server; DESIGN picks whether the OpenAPI-derived types live alongside the server or in a dedicated shared crate). Plus `overdrive-sim` gets new DST invariants but no structural changes. Within the ≤3-module-or-crate "oversized signal" when interpreted as bounded contexts (API + reconciler runtime + CLI + domain model are one bounded context each, but the domain model and CLI already exist).
- **Walking-skeleton integration points**: 4 — CLI → REST API, REST handlers → IntentStore, REST handlers → ReconcilerRuntime, REST handlers → ObservationStore. Under the >5 signal.
- **Estimated effort**: 4-6 focused days (Slices 1 and 2 can run serial or parallel; Slice 3 and 4 can run in parallel; Slice 5 is last).
- **Multiple independent user outcomes worth shipping separately**: no — without the walking skeleton closing, none of Slices 1-4 deliver operator-observable value on their own. The reconciler primitive without the API is untestable in production; the API without the reconciler primitive leaves GH #17 unclosed.
- **Verdict**: RIGHT-SIZED. Proceed to user story crafting.

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial story map for phase-1-control-plane-core. |
| 2026-04-23 | Transport pivot: activity 2 retitled "Speak REST"; OpenAPI schema + schema-lint gate + axum handlers replacing proto + tonic throughout; slice 2 renamed to "Control-plane REST service surface". |
