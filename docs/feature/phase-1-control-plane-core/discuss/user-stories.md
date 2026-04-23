<!-- markdownlint-disable MD024 -->

# User Stories — phase-1-control-plane-core

Five LeanUX stories, each delivering a single carpaccio slice from `story-map.md`. All stories share the persona and the vision context from `docs/product/vision.md` and `docs/product/jobs.yaml`, and build on the phase-1-foundation walking skeleton (newtypes, trait ports, `LocalStore`, `SimObservationStore`, DST harness).

## System Constraints (cross-cutting)

These apply to every story below. They extend the phase-1-foundation system constraints — every constraint from `docs/feature/phase-1-foundation/discuss/user-stories.md` still applies verbatim. The additional constraints specific to this feature:

- **Reconcilers are pure.** `reconcile(desired, actual, db) -> Vec<Action>` is a pure function over its inputs per whitepaper §18 and `.claude/rules/development.md`. No `.await`, no wall-clock, no subprocess, no direct store write. External calls emit `Action::HttpCall`; multi-step sequences become workflows (Phase 3).
- **Intent vs Observation is a compile-time boundary.** The IntentStore / ObservationStore non-substitutability compile-fail tests shipped in phase-1-foundation must still pass. Nothing in this feature widens either store's public surface.
- **Canonical archival for aggregates is rkyv.** Per ADR-0002 (phase-1-foundation). Any new aggregate struct that crosses the IntentStore boundary derives `rkyv::Archive + Serialize + Deserialize`. `ContentHash::of(archived_bytes)` is the single deterministic hashing path.
- **External API is REST + OpenAPI.** The control-plane API exposed to CLI, operators, and future SDKs is REST + JSON over HTTP/2 with `rustls`, described by a single OpenAPI 3.1 schema that is the single source of truth for the wire contract. The OpenAPI document is derived from the Rust request/response types (via `utoipa` or `aide` — DESIGN picks) and consumed by the CLI either through a hand-rolled `reqwest`-style client or an OpenAPI-generated Rust client. Any field change is a single-source edit on the Rust types that generate the schema.
- **Aggregate serialisation is lane-specific.** Aggregates (Job, Node, Allocation, Policy, Investigation) serialise via serde-JSON at the REST boundary and via rkyv at the IntentStore boundary. The two serialisations are decoupled and non-substitutable — an aggregate is deserialised from JSON once at the handler edge, validated through the constructor, then archived via rkyv before any store write. The spec digest is always `ContentHash::of(rkyv_archived_bytes)`; serde-JSON is never hashed.
- **Internal RPC (future) is tarpc / postcard-rpc.** Node-agent control-flow streams (starting in `phase-1-first-workload`) will use a pure-Rust internal RPC — `tarpc` or `postcard-rpc` — over HTTP/2 with `rustls`. No `protoc` in the toolchain. This feature does not ship any node-agent path, but DESIGN should avoid wiring choices that would later force gRPC back in.
- **Auth posture: unauthenticated local endpoint.** Operator mTLS + SPIFFE operator IDs are explicitly deferred to Phase 5. The walking-skeleton control plane binds to `https://127.0.0.1:7001` by default using a self-generated local-dev certificate and does not require authentication. DESIGN must not build around auth that isn't there.
- **Empty states are honest.** Any CLI output that reflects a zero-row observation (no allocations, no nodes) must render an explicit empty state that names the next feature or the reason for emptiness. Silent blank output is a UAT failure per `nw-ux-emotional-design` + `nw-ux-tui-patterns`.
- **Paradigm is OOP (Rust trait-based).** Confirmed against phase-1-foundation ADRs 0001–0006. No functional-first pull; trait objects for ports, `enum` for errors under `thiserror`, newtypes for domain IDs. Axum handlers are idiomatic Rust functions at the edge that return trait-object responses — consistent with the phase-1-foundation precedent.

---

## US-01: Job / Node / Allocation aggregates + canonical intent keys

### Problem

Ana reads whitepaper §4's core data model — `Job`, `Node`, `Allocation`, `Policy`, `Investigation` — and finds the newtypes (JobId, NodeId, …) shipped in phase-1-foundation but no aggregates. Every subsystem that needs to round-trip a "job spec" — the API, the store, the CLI — would invent its own struct. The first two would disagree on field order; the third would serialize with serde_json instead of rkyv; and the spec digest `overdrive alloc status` prints would drift from what the operator can compute locally. The structural answer is to land the aggregates once, in `overdrive-core`, with rkyv as the canonical archival path and a single intent-key derivation function that every caller routes through.

### Who

- Overdrive platform engineer, working across the control plane, the CLI, and the store | motivated to rely on one aggregate shape and one canonical key derivation rather than re-inventing them at each crate boundary.

### Solution

Introduce `Job`, `Node`, `Allocation` aggregate structs in `overdrive-core`, each constructed from the phase-1-foundation newtypes through validating constructors. Derive `rkyv::Archive`, `rkyv::Serialize`, `rkyv::Deserialize`, and `serde::Serialize` / `serde::Deserialize`. Add an `IntentKey` type (or module) exposing one function per aggregate that derives the canonical key (e.g. `jobs/<JobId::display()>`). Ship `Policy` and `Investigation` as stub structs sufficient to satisfy the whitepaper data model reference without committing to their phase-specific fields.

### Domain Examples

#### 1: Happy Path — Ana constructs a Job from a TOML file

Ana reads `./payments.toml` into a serde-derived `JobSpec` struct. She uses the canonical `Job::from_spec(spec)` constructor, which invokes `JobId::from_str("payments")` and the resources constructor. She gets `Ok(Job { id: JobId("payments"), replicas: 1, resources: Resources { cpu_cores: 2, memory_bytes: 4 GiB }, … })`. She archives it via rkyv and computes `ContentHash::of(archived_bytes)` — which is stable across invocations.

#### 2: Edge Case — Ana derives the same intent key in the CLI and in the control plane

Ana has a `JobId("payments")`. She calls `IntentKey::for_job(&job_id)` in the CLI and prints the result as `jobs/payments`. The control-plane submit handler calls the SAME function with the same `JobId` and writes to `IntentStore` at the same key. When `alloc status` later calls `IntentStore::get` with the same derivation, the byte-for-byte same key reaches the store.

#### 3: Error Boundary — Ana tries to construct a Node with zero-byte memory capacity

Ana writes `capacity.memory_bytes = 0` in a Node construction call. The `Resources::new(...)` validating constructor returns `Err(ResourceError::ZeroMemory)` and no `Node` instance is constructed. The same rejection fires before any REST request is sent or any store write attempted.

### UAT Scenarios (BDD)

#### Scenario: Aggregate structs round-trip through rkyv without loss

Given a valid `Job` aggregate constructed through the validating constructor
When Ana archives it via rkyv, accesses the archived bytes, and deserialises the archive
Then the resulting `Job` is equal to the original
And two archivals of the same logical `Job` produce byte-identical bytes

#### Scenario: Canonical intent-key derivation matches across callers

Given a `JobId` with `Display` output `"payments"`
When the CLI calls `IntentKey::for_job(&job_id)` and the control-plane submit handler calls the same function with the same `JobId`
Then both call sites produce the same key byte-for-byte
And the key's canonical string form is `"jobs/payments"`

#### Scenario: Validating constructor rejects malformed aggregate input

Given a TOML spec with `replicas = 0`
When Ana calls `Job::from_spec(spec)`
Then `Err(JobError::InvalidReplicaCount { got: 0 })` is returned
And no `Job` value is constructed

#### Scenario: Node aggregate enforces resource sanity

Given a Node spec with `capacity.memory_bytes = 0`
When Ana calls `Node::new(...)`
Then an error variant naming the zero-memory violation is returned
And no `Node` value is constructed

#### Scenario: Allocation links a Job and a Node through typed IDs only

Given a `JobId` and a `NodeId`
When Ana constructs an `Allocation` pairing them with `AllocationId::new()`
Then the struct's fields are the typed newtypes
And no raw `String` or `u64` identifiers are exposed in the struct's public fields

### Acceptance Criteria

- [ ] `Job`, `Node`, `Allocation` structs exist in `overdrive-core` with validating constructors returning `Result`
- [ ] Each aggregate derives `rkyv::Archive`, `rkyv::Serialize`, `rkyv::Deserialize`, and serde `Serialize` + `Deserialize`
- [ ] `Policy` and `Investigation` stubs exist with the ID newtype as the primary field (no behavioural stubs yet)
- [ ] `IntentKey::for_job(&JobId)` (and `for_node` / `for_allocation` equivalents) exists in exactly one module
- [ ] A proptest asserts `Job` rkyv round-trip equality across arbitrary valid inputs
- [ ] A proptest asserts `IntentKey::for_job` output is stable for any valid `JobId` (byte-identical across calls)
- [ ] No aggregate field is a raw `String` where a newtype exists in `overdrive-core`
- [ ] The `Resources` struct used by `Job` is the same one already exposed by `traits/driver.rs` — not a duplicate

### Outcome KPIs

- **Who**: Overdrive platform engineer consuming the domain model across control plane, CLI, and store
- **Does what**: constructs any whitepaper-referenced aggregate through one struct, archives it through one rkyv path, and derives intent keys through one function
- **By how much**: 3 aggregates (Job / Node / Allocation) shipped with full rkyv + serde + validating-constructor coverage; 0 duplicate `Resources` or intent-key derivations in the workspace
- **Measured by**: proptest for rkyv round-trip; grep for duplicate key-derivation functions (zero expected)
- **Baseline**: none — aggregate structs do not exist today (only newtypes and a minimal `JobSpec` test shape in `traits/observation_store.rs`)

### Technical Notes

- `JobSpec` as it currently exists in `crates/overdrive-core/src/traits/observation_store.rs` is a placeholder used by the ObservationStore row shape. DESIGN must decide whether to rename / replace it or keep it as an observation-side DTO distinct from the intent-side `Job` aggregate.
- `Resources` lives in `traits/driver.rs` today. Phase 1 should use it as the single source; any enrichment (labels, selectors) is Phase 2+.
- **Depends on**: phase-1-foundation newtypes (11 types; all shipped).

---

## US-02: Control-plane HTTP/REST service surface

### Problem

Without a single wire contract shared by the CLI and the server, each side defines its own request and response types, and the first field rename silently breaks the walking skeleton. Whitepaper §3 now names the two-lane split explicitly: **REST + OpenAPI over axum/rustls for external clients** (CLI, operators, future SDKs), and `tarpc` / `postcard-rpc` for internal control-flow streams (node-agent — not in this feature). Ana needs one OpenAPI 3.1 schema that is the single source of truth for the wire contract, an axum router that binds and answers the walking-skeleton endpoints, and one function that maps typed `thiserror` variants to HTTP status codes.

### Who

- Overdrive platform engineer building either the CLI or the control plane | motivated by the need for one wire contract, not two.

### Solution

Derive a single OpenAPI 3.1 schema from the Rust request/response types using `utoipa` or `aide` (DESIGN picks). The schema document is the single source of truth — checked into the workspace and regenerated on build; a schema-lint gate (`openapi-check`-style) runs in CI. Wire an axum router exposing `POST /v1/jobs` (SubmitJob), `GET /v1/jobs/{id}` (DescribeJob), `GET /v1/cluster/info` (ClusterStatus), `GET /v1/allocs` (AllocStatus), `GET /v1/nodes` (NodeList). Serve it over HTTP/2 with `rustls`. Add one `to_response(err: impl Into<ControlPlaneError>) -> (StatusCode, Json<ErrorBody>)` function that maps typed error variants to HTTP status codes with a structured JSON body.

### Domain Examples

#### 1: Happy Path — Ana submits a Job via the REST API

Ana's CLI POSTs a JSON body to `https://127.0.0.1:7001/v1/jobs` with the serialised Job spec. The handler deserialises `Json<SubmitJobRequest>`, calls `Job::from_spec(...)`, archives via rkyv, commits through `IntentStore::put`, and responds with `200 OK` and `{"job_id": "payments", "commit_index": 17}`. No hand-rolled encoding — axum + serde do the JSON round-trip; `utoipa` / `aide` keep the OpenAPI schema locked in step with the Rust types.

#### 2: Edge Case — Ana submits a Job whose spec fails validation

Ana submits a spec with `replicas = 0`. The handler decodes the JSON body, calls `Job::from_spec(...)`, gets `Err(JobError::InvalidReplicaCount { got: 0 })`, and routes the error through `to_response(...)`. The CLI receives `400 Bad Request` with `{"error": "invalid_argument", "field": "replicas", "message": "replicas must be ≥ 1 (got: 0)"}`. The CLI surfaces the field + message and exits 1.

#### 3: Error Boundary — Ana submits against a server that has shut down mid-request

Ana submits while the server is being stopped. The HTTP client returns a connection-refused or connection-reset error before any response arrives. The CLI prints "The control plane is not accepting requests. Check it is running on https://127.0.0.1:7001 and try again." and exits 1. No raw stack trace, no opaque "transport error".

### UAT Scenarios (BDD)

#### Scenario: OpenAPI schema is the single source of truth for the wire contract

Given the control-plane OpenAPI 3.1 schema is derived from the Rust request/response types
When both `overdrive-cli` and the control-plane server build
Then the schema document is byte-identical to the one checked into the workspace
And no CLI-side hand-rolled request or response struct mirrors a server-side one outside of the schema-generated types

#### Scenario: Submit round-trip through the REST API

Given an axum server running on `https://127.0.0.1:7001`
When the CLI POSTs a valid Job spec to `/v1/jobs`
Then the server responds with `200 OK` and a JSON body whose `commit_index` is ≥ 1
And the server's write hit the real `LocalStore` (not a mock)

#### Scenario: Typed error maps to the right HTTP status code

Given a request whose spec fails newtype validation
When the server handler returns the typed error
Then the HTTP response carries status `400 Bad Request`
And the JSON error body names the offending field

#### Scenario: Server shuts down cleanly on SIGINT

Given a running server
When a SIGINT is delivered
Then the server stops accepting new requests
And in-flight requests complete before the process exits
And the exit code is 0

#### Scenario: Unreachable endpoint renders an actionable error

Given the server is not running
When the CLI calls any endpoint
Then the CLI renders an error naming the endpoint
And the error suggests a concrete next action
And the CLI exit code is 1

### Acceptance Criteria

- [ ] One OpenAPI 3.1 schema document defines the control-plane API (location chosen by DESIGN); it is generated from the Rust request/response types via `utoipa` or `aide`
- [ ] A schema-lint gate runs in CI and fails the build if the generated schema drifts from the checked-in document
- [ ] The axum server binds over HTTP/2 with `rustls`, serves, and shuts down on SIGINT with in-flight-request drain
- [ ] Endpoints exposed: `POST /v1/jobs` (SubmitJob), `GET /v1/jobs/{id}` (DescribeJob), `GET /v1/cluster/info` (ClusterStatus), `GET /v1/allocs` (AllocStatus), `GET /v1/nodes` (NodeList) — exact paths confirmed by DESIGN; the `/v1` version prefix is non-negotiable
- [ ] An `ErrorToResponse` mapping (single function or trait impl) covers every `thiserror` variant in the submit / describe / alloc-status / cluster-status paths
- [ ] Every variant in the error enum has a test asserting its HTTP status code
- [ ] The CLI renders connection-level failures (refused / reset) with an actionable message naming the endpoint
- [ ] Default bind address matches the CLI's default endpoint (`https://127.0.0.1:7001`)
- [ ] The server binary is part of a crate DESIGN will name (proposed: `crates/overdrive-control-plane`)

### Outcome KPIs

- **Who**: Overdrive platform engineer making API calls (CLI today, OpenAPI-generated SDKs later)
- **Does what**: depends on one OpenAPI schema, not hand-rolled mirrors on each side
- **By how much**: 1 OpenAPI 3.1 document; schema-lint gate green on every PR; 100% of typed error variants have an HTTP status mapping
- **Measured by**: CI schema-lint check; workspace search for duplicate request/response structs outside the generated module; unit test enumerating error variants and asserting status mapping
- **Baseline**: none — no API or server exists today (CLI stub warns "command not yet wired to control plane")

### Technical Notes

- Expected crate stack: `axum` (router + handlers) + `rustls` (HTTP/2 TLS, local-dev cert) + `serde` / `serde_json` (body encode/decode) + `utoipa` or `aide` (OpenAPI schema derivation). DESIGN picks between `utoipa` and `aide`; both are idiomatic Rust and both derive schema from types.
- The OpenAPI document URI version is `v1`; any future breaking change becomes `v2` with both served in parallel during the deprecation window.
- Streaming responses (`Watch` / server-sent events) are deferred; the walking skeleton is request-response only.
- Internal RPC to a node agent (tarpc / postcard-rpc) is explicitly out of this feature — it ships with `phase-1-first-workload`. DESIGN should avoid router decisions that would later force gRPC back into the external API path.
- **Depends on**: US-01 (aggregate types carried in the JSON request / response bodies).

---

## US-03: API handlers commit to IntentStore + ObservationStore reads

### Problem

phase-1-foundation proved `IntentStore` + `LocalStore` work through the DST harness. The commercial and technical value of that work is zero until an operator-observable surface actually writes through the store and reads back. The whitepaper §4 submit path — validate → wrap in rkyv → commit — has to exist as code in a handler, not just as a diagram. If Submit commits a spec and Describe can't read it back byte-identical, the walking-skeleton round-trip invariant fails and Phase 2+ reconciler work can't be trusted.

### Who

- Overdrive platform engineer building the server handlers | Ana via the CLI (indirect user) | operator (future) running `overdrive job submit` against a single-mode control plane.

### Solution

Implement SubmitJob, DescribeJob, AllocStatus, NodeList as axum handlers in the control-plane binary. Submit extracts `Json<SubmitJobRequest>`, validates the Job through the aggregate constructor, archives via rkyv, and calls `IntentStore::put` on `LocalStore`. Describe reads back and recomputes the spec digest via `ContentHash::of(archived_bytes)`. AllocStatus / NodeList call `ObservationStore::read` against the phase-1-foundation-locked row shapes. Expose `LocalStore::commit_index()` as a new read-only accessor. Gate every write with the validating constructors — the validator fires before any store write.

### Domain Examples

#### 1: Happy Path — Ana submits a spec, then describes the same job

Ana POSTs `payments.toml`'s serialised form to `/v1/jobs`. The handler decodes the JSON body, calls `Job::from_spec(...)` (constructors fire), archives via rkyv, calls `IntentStore::put("jobs/payments", archived_bytes)`. Commit index is 17. Ana then GETs `/v1/jobs/payments`. The handler calls `IntentStore::get("jobs/payments")`, rkyv-accesses the bytes, recomputes `ContentHash::of(archived_bytes) = sha256:7f3a9b12…`, and responds `200 OK` with `{"spec": <same>, "commit_index": 17, "spec_digest": "sha256:7f3a9b12…"}`.

#### 2: Edge Case — Ana submits twice and sees monotonic commit indexes

Ana submits `payments.toml`, gets commit_index 17. She submits a (different) `frontend.toml`, gets commit_index 18. She describes `payments` — the commit_index returned is still 17 (the submit's), not the latest store index. The commit counter is monotonic; reads do not invent a new one.

#### 3: Error Boundary — Ana submits a Job whose JobId would round-trip-fail

Ana constructs a malformed spec that somehow evades serde-JSON schema validation but fails the rkyv canonical form (hypothetical: a field containing mixed-case variant that `JobId::from_str` would normalise, causing archival bytes to differ from CLI-local compute). The server refuses to commit: the typed validator fires *inside the handler*, returns `409 Conflict` (FailedPrecondition-class), and the CLI-side digest check would disagree with the server.

### UAT Scenarios (BDD)

#### Scenario: Submit-then-Describe round-trips the spec byte-identical

Given the server is running in single mode
And Ana has submitted a valid Job spec
When Ana GETs `/v1/jobs/{id}` with the returned JobId
Then the response spec equals the submitted spec after rkyv access
And the response `spec_digest` equals `ContentHash::of` of the archived submitted bytes

#### Scenario: Commit index strictly increases across submits

Given the server is running
When Ana submits three different valid Job specs in sequence
Then each submit response's `commit_index` is strictly greater than the previous one

#### Scenario: Validating constructor rejects before IntentStore is touched

Given a Job spec with `replicas = 0`
When Ana submits it
Then the server responds with `400 Bad Request`
And the IntentStore has no new entry for the malformed input

#### Scenario: AllocStatus returns zero rows in Phase 1

Given the server is running in single mode with no scheduler or driver shipped yet
When Ana calls AllocStatus for any valid JobId
Then the response's `rows` field is empty
And the response does NOT contain fabricated placeholder rows

#### Scenario: NodeList returns zero rows in Phase 1

Given the server is running in single mode with no node agent yet
When Ana calls NodeList
Then the response's `rows` field is empty
And the response does NOT contain a fabricated "local" node

#### Scenario: DescribeJob on an unknown JobId returns NotFound

Given the server has no Job committed under `unknown-id`
When Ana GETs `/v1/jobs/unknown-id`
Then the response carries `404 Not Found`
And the IntentStore is unchanged

### Acceptance Criteria

- [ ] SubmitJob handler archives via rkyv, commits via `IntentStore::put`, returns the commit_index in a JSON response
- [ ] DescribeJob handler reads via `IntentStore::get`, rkyv-accesses the bytes, recomputes spec_digest
- [ ] AllocStatus handler calls `ObservationStore::read` against `alloc_status` (schema from phase-1-foundation brief §6)
- [ ] NodeList handler calls `ObservationStore::read` against `node_health`
- [ ] `LocalStore::commit_index()` accessor exists and is strictly monotonic
- [ ] Validating constructors fire BEFORE any IntentStore write (test asserts no new entry on malformed input)
- [ ] Unknown-JobId Describe returns `404 Not Found` with no store side effects
- [ ] Validation failures return `400 Bad Request`; duplicate-intent-key conflicts (if DESIGN surfaces them) return `409 Conflict`; unexpected infrastructure failures return `500 Internal Server Error` with a structured error body (never a raw stack trace)
- [ ] Empty-observation responses carry no placeholder rows
- [ ] A round-trip proptest: submit a valid Job, describe it, assert `response.spec == input.spec`

### Outcome KPIs

- **Who**: Overdrive platform engineer + operator running `overdrive job submit` end-to-end
- **Does what**: submits a spec, sees the platform commit it, and reads it back byte-identical
- **By how much**: 100% of valid submit/describe round-trips return byte-identical specs; 100% of malformed submits reject before any write; commit_index strictly monotonic
- **Measured by**: round-trip proptest in `tests/acceptance/`; commit_index monotonicity test; negative test asserting no-write on rejection
- **Baseline**: none — no server handlers exist today

### Technical Notes

- Per phase-1-foundation DWD-01 (Walking Skeleton Strategy C): `LocalStore` in these tests is the real redb-backed adapter against `tempfile::TempDir`. No mocks.
- The `commit_index` surface is new but should not leak redb internals — the accessor returns a `u64` sequence, not a redb transaction handle.
- `ObservationStore::read` in Phase 1 goes through `SimObservationStore` in DST, and through whichever implementation DESIGN picks for the server. For the walking-skeleton CLI round-trip test, using `SimObservationStore` is acceptable (no scheduler writes are expected); DESIGN may also wire a trivial in-process LWW map for the server if a real Corrosion adapter is out of scope.
- HTTP status-code convention: `400 Bad Request` for validation failures, `404 Not Found` for unknown resources, `409 Conflict` for duplicate-intent-key scenarios if DESIGN surfaces them, `500 Internal Server Error` for infra failures. Error bodies are structured JSON (`{"error": "...", "message": "...", "field": "..."}`). No raw stack traces.
- **Depends on**: US-01, US-02.

---

## US-04: Reconciler primitive — trait + runtime + evaluation broker

### Problem

Whitepaper §18 commits to a §18 reconciler primitive whose differentiating property against Kubernetes and Nomad is the **evaluation broker with cancelable-eval-set** shipped native, not retrofitted. Nomad documents exactly the failure mode this prevents (500 flapping nodes × 20 allocations × 100 system jobs = 60 000 evaluations per heartbeat window). Shipping the primitive without this mitigation repeats Nomad's incident; shipping the primitive at all with a muddied pure-function contract makes every Phase 2+ reconciler a verification target that can never be met. Ana needs the trait, the runtime, the broker, and the per-primitive private libSQL DBs all shipped together with DST invariants that catch regressions.

### Who

- Overdrive platform engineer who will write Phase 2+ reconcilers | DST harness (asserts the primitive contract) | Ana via `overdrive cluster status` (operator-visible proof the runtime is alive).

### Solution

Land `Reconciler` trait in `overdrive-core::reconciler::Reconciler` with the pure-function signature from whitepaper §18. Add `Action` enum variants (at minimum `Noop`, `HttpCall`, `StartWorkflow`). Land `ReconcilerRuntime` + `EvaluationBroker` with cancelable-eval-set semantics inside the control-plane crate. Provision per-primitive private libSQL databases. Register a `noop-heartbeat` reconciler at boot. Add DST invariants `at_least_one_reconciler_registered`, `duplicate_evaluations_collapse`, `reconciler_is_pure`.

### Domain Examples

#### 1: Happy Path — Ana writes a reconciler against the trait

Ana defines a struct `MyReconciler` and implements `impl Reconciler for MyReconciler { fn reconcile(&self, desired: &State, actual: &State, db: &Db) -> Vec<Action> { … } }`. The implementation has no `async fn`, no `.await`, no wall-clock reads, no direct store writes. It returns an `Vec<Action>` with `Action::HttpCall { … }` when it needs to reach an external system. The code compiles against stable Rust; DST replays it deterministically.

#### 2: Edge Case — Ana queues three evaluations at the same target

The harness fires three `Evaluation { reconciler: "noop-heartbeat", target: "job/payments" }` messages within one broker tick. The broker accepts the first, moves it to pending, accepts the second and cancels the first (moves to cancelable set), accepts the third and cancels the second. On the next tick the reaper clears the cancelable set. The dispatched counter is 1, the cancelled counter is 2, the queued counter is 0.

#### 3: Error Boundary — Ana tries to smuggle `Instant::now()` into a reconciler

Ana writes `let now = std::time::Instant::now();` inside a reconciler's `reconcile(...)` body. The `cargo xtask dst-lint` gate (shipped in phase-1-foundation) flags the line because the reconciler lives in a core-class crate. Ana switches to `db.read("SELECT last_eval FROM history WHERE key = ?", …)` or emits an Action that surfaces the time dependency explicitly. Separately, even if a future reconciler dodges the lint somehow, the `reconciler_is_pure` DST invariant catches it: twin invocation with the same inputs must produce identical outputs, and a smuggled `now()` causes divergence.

### UAT Scenarios (BDD)

#### Scenario: Reconciler trait enforces the pure-function contract

Given the `Reconciler` trait in `overdrive-core`
When Ana writes a `impl Reconciler for MyReconciler`
Then the `reconcile(...)` method signature has no `async`
And the method parameters do not include any `&dyn Clock` or equivalent I/O port

#### Scenario: Runtime registers at least one reconciler at boot

Given a control plane starting in single mode
When boot completes
Then `ReconcilerRuntime::registered()` returns a non-empty set
And the set contains the `noop-heartbeat` reconciler

#### Scenario: Evaluation broker collapses duplicates

Given three evaluations arrive at the same `(reconciler, target)` key within one broker tick
When the broker drains the queue
Then exactly one evaluation is dispatched
And the cancelled counter increments by exactly two

#### Scenario: Cancelable-eval-set reaper bounds the set

Given N cancelled evaluations accumulate across K ticks
When the reaper runs
Then the cancelable set is emptied in bulk
And the `cancelled` counter does not grow unboundedly across long runs

#### Scenario: Per-primitive libSQL databases are filesystem-isolated

Given two reconcilers `alpha` and `beta` are registered
When the runtime provisions private databases
Then `alpha`'s DB path and `beta`'s DB path are distinct
And `alpha` cannot read from `beta`'s DB through its injected `&Db` handle

#### Scenario: reconciler_is_pure invariant catches smuggled nondeterminism

Given a seeded DST run that invokes the registered reconcilers on the same inputs twice
When the `reconciler_is_pure` invariant evaluates both invocations
Then the two output `Vec<Action>` sequences are equal element-for-element

#### Scenario: cluster status surfaces the reconciler registry

Given a running control plane with the noop-heartbeat reconciler registered
When Ana runs `overdrive cluster status`
Then the Reconcilers section lists `noop-heartbeat`
And the output shows the broker's queued / cancelled / dispatched counters

### Acceptance Criteria

- [ ] `Reconciler` trait in `overdrive-core` has the signature `fn reconcile(&self, desired: &State, actual: &State, db: &Db) -> Vec<Action>` with no `async`
- [ ] `Action` enum exists with at minimum `Noop`, `HttpCall { … }` (per development.md), `StartWorkflow { spec, correlation }` (placeholder; workflow runtime is Phase 3)
- [ ] `ReconcilerRuntime` exposes `registered() -> impl Iterator<Item=&dyn ReconcilerHandle>` (exact shape DESIGN owns)
- [ ] `EvaluationBroker` is keyed on `(reconciler_name, target_resource)`; implements cancelable-eval-set semantics
- [ ] Broker surfaces `queued`, `cancelled`, `dispatched` counters readable by the `ClusterStatus` RPC
- [ ] Per-primitive private libSQL databases are provisioned with distinct paths and isolated `&Db` handles
- [ ] A `noop-heartbeat` reconciler is registered at boot as living proof of the contract
- [ ] DST invariant `at_least_one_reconciler_registered` passes on every run
- [ ] DST invariant `duplicate_evaluations_collapse` passes under N (≥3) concurrent evaluations at the same key
- [ ] DST invariant `reconciler_is_pure` passes (twin invocation with identical inputs produces identical outputs)
- [ ] The reconciler trait is enforced by the phase-1-foundation dst-lint gate (no `Instant::now()` / `rand::random()` in reconciler bodies)

### Outcome KPIs

- **Who**: Overdrive platform engineer writing future reconcilers + operator observing cluster state
- **Does what**: ships reconcilers that verify the §18 contract and survive evaluation storms from day one
- **By how much**: storm-mitigation in the broker is live before the first real reconciler; DST invariants cover both the boot-time registry and the broker's collapse property; zero regressions on the pure-function contract (dst-lint + DST both catch it)
- **Measured by**: DST harness (three new invariants); CLI `cluster status` renders the counters; per-DB path isolation test
- **Baseline**: none — no reconciler primitive exists today; phase-1-foundation DST has placeholder `ReplayEquivalentEmptyWorkflow` for workflows but no reconciler-specific invariants

### Technical Notes

- The pure-function contract is load-bearing. Per `.claude/rules/development.md` §Reconciler I/O, external calls are `Action::HttpCall` (runtime shim lands Phase 3, but the variant is part of the Phase 1 surface).
- The broker's reaper in whitepaper §18 is itself a reconciler (`evaluation-broker-reaper`). In Phase 1 it can be a simple in-runtime loop — the important property is that the cancelable set is bounded. DESIGN decides the exact shape.
- `Db` is the per-primitive libSQL handle type. DESIGN names the crate boundary (`overdrive-primitive-db` or inline in `overdrive-core`). Phase 1 requires filesystem isolation but not advanced features (migrations, pooling).
- `async_trait` vs native-async-in-trait is a DESIGN decision. Phase 1 should pick whichever minimises churn; either is defensible.
- **Depends on**: US-01 (Action variants reference aggregate IDs).

---

## US-05: CLI handlers for job / alloc / cluster / node

### Problem

The CLI stub that phase-1-foundation left in place greets every invocation with `tracing::warn!(endpoint = %cli.endpoint, "command not yet wired to control plane")`. Operators cannot actually submit a job, inspect allocations, or check cluster state. Until the CLI rounds-trips through the real control plane and the server honestly reports what it does and doesn't know, the walking-skeleton hypothesis — "a platform engineer can submit a job and observe what the platform knows about it" — is unmet. Ana needs real handlers that compute the spec digest via the same rkyv canonical path the server uses, render empty states honestly, and map error conditions to actionable output.

### Who

- Overdrive platform engineer running `overdrive` from a laptop | CI running smoke tests against a local control plane | operator (Phase 5+) running `overdrive job submit` once auth lands.

### Solution

Replace the CLI stub body with real handlers that call the REST API through a thin HTTP client (either hand-rolled `reqwest`-style or the OpenAPI-generated Rust client — DESIGN picks). `job submit` reads TOML, constructs a Job, POSTs JSON to `/v1/jobs`, prints commit_index and canonical intent_key. `node list` GETs `/v1/nodes`, renders empty state or table. `alloc status` GETs `/v1/jobs/{id}` then `/v1/allocs?job={id}`, renders spec digest and alloc rows (or explicit empty state). `cluster status` GETs `/v1/cluster/info` and renders the reconciler registry and broker counters. Map HTTP error responses (status code + structured JSON body) to actionable output answering "what / why / how to fix".

### Domain Examples

#### 1: Happy Path — Ana submits a Job and inspects it

Ana runs `overdrive job submit ./payments.toml`. The CLI prints `Accepted. Job ID: payments, Intent key: jobs/payments, Commit index: 17, Endpoint: https://127.0.0.1:7001, Next: overdrive alloc status --job payments`. She runs `overdrive alloc status --job payments` and sees `Spec digest: sha256:7f3a9b12…` matching what she can compute locally from the same file, plus `Allocations: 0 (none placed — scheduler lands in phase-1-first-workload)`.

#### 2: Edge Case — Ana inspects a fresh cluster with zero nodes

Ana runs `overdrive node list` against a just-started control plane. The CLI prints: `No nodes registered yet — node agent lands in phase-1-first-workload.` She runs `overdrive cluster status` and sees the commit_index, mode, and the `noop-heartbeat` reconciler registered with zero-queued evaluations. She understands from the output exactly what is and isn't alive.

#### 3: Error Boundary — Ana submits against a down endpoint

Ana starts `overdrive job submit ./payments.toml` without the server running. The CLI prints:

```
Error: Could not connect to control plane at https://127.0.0.1:7001

  The connection was refused. The control plane may not be running.

  Try:
    1. Start the control plane:       overdrive-control-plane
    2. Verify the endpoint:           overdrive cluster status
    3. Override the endpoint:         overdrive --endpoint <URL> job submit ...
```

The CLI exits with code 1. No raw `ECONNREFUSED` token, no raw `reqwest::Error` debug format, no stack trace reaches the operator.

### UAT Scenarios (BDD)

#### Scenario: `job submit` round-trips a spec and prints actionable next steps

Given a running control plane on the default endpoint
And a file `payments.toml` containing a valid Job spec
When Ana runs `overdrive job submit ./payments.toml`
Then the CLI exits 0
And the output contains the Job ID, the canonical intent key, and the commit index
And the output ends with a "Next: overdrive alloc status --job <id>" line

#### Scenario: `alloc status` shows the same spec digest as the local file

Given Ana has submitted `payments.toml` and received a commit_index
When Ana runs `overdrive alloc status --job payments`
Then the output contains a spec digest equal to the digest derivable locally from the input file under the same rkyv canonical path
And the output explicitly states that zero allocations are placed

#### Scenario: Empty `node list` renders an honest empty state

Given a control plane with zero registered nodes
When Ana runs `overdrive node list`
Then the CLI exits 0
And the output is NOT a blank table
And the output names the reason (node agent lands in phase-1-first-workload)

#### Scenario: `cluster status` surfaces the reconciler registry

Given a running control plane with the noop-heartbeat reconciler registered
When Ana runs `overdrive cluster status`
Then the output lists `noop-heartbeat` in the Reconcilers section
And the output reports the evaluation broker's queued / cancelled counters

#### Scenario: Unreachable endpoint renders an actionable error

Given the control plane is not running
When Ana runs `overdrive job submit ./payments.toml`
Then the CLI exits with code 1
And the output explains what happened, why, and three concrete next steps
And the output does not contain a raw Rust panic, a raw `ECONNREFUSED` token, or a raw `reqwest::Error` debug format

#### Scenario: Malformed spec produces a validation error pointing at the field

Given a file `broken.toml` with `replicas = 0`
When Ana runs `overdrive job submit ./broken.toml`
Then the CLI exits with code 1
And the error message names the field `replicas` and the invalid value `0`

#### Scenario: `--endpoint` flag overrides the env override overrides the default

Given the `OVERDRIVE_ENDPOINT` env var is set to a non-default value
When Ana runs `overdrive --endpoint <other-url> cluster status`
Then the CLI connects to `<other-url>`
And the effective endpoint is echoed in the CLI output

### Acceptance Criteria

- [ ] `overdrive job submit <file>` reads the TOML, constructs a Job via validating constructors, POSTs JSON to `/v1/jobs`, prints commit_index and canonical intent_key
- [ ] `overdrive alloc status --job <id>` GETs `/v1/jobs/{id}` and `/v1/allocs`, renders spec_digest + replicas + alloc list (possibly empty with explicit state)
- [ ] `overdrive node list` GETs `/v1/nodes`; zero rows render as an explicit empty state naming the next feature
- [ ] `overdrive cluster status` GETs `/v1/cluster/info`; renders mode, region, commit_index, reconciler registry, broker counters
- [ ] All error paths answer "what / why / how to fix" per `nw-ux-tui-patterns`
- [ ] Exit codes: 0 success, 1 generic error, 2 usage error
- [ ] Endpoint precedence: `--endpoint` flag > `OVERDRIVE_ENDPOINT` env > default `https://127.0.0.1:7001`
- [ ] First output within 100ms on localhost (no artificial spinner)
- [ ] The spec_digest the CLI prints equals what the operator can compute locally from the same input file under the rkyv canonical path
- [ ] No empty state renders as a blank table or silent exit

### Outcome KPIs

- **Who**: Overdrive platform engineer or operator running `overdrive` from a laptop
- **Does what**: submits a job and inspects cluster state through a CLI that honestly reports what the platform knows
- **By how much**: 100% of walking-skeleton commands round-trip through the real control plane; 0 silent empty states; 100% of error paths answer "what / why / how to fix"
- **Measured by**: end-to-end acceptance test running CLI subcommands against a real control plane (Strategy C); review of every empty-state and error rendering
- **Baseline**: current CLI stub warns and exits 0 for every command

### Technical Notes

- TOML parsing lives in the CLI (serde_derive). The result is converted to the canonical `Job` aggregate through the same constructor the server uses, guaranteeing validation parity.
- HTTP client: DESIGN picks between a hand-rolled `reqwest`-style client against the OpenAPI schema and an OpenAPI-generated Rust client. Either is acceptable; both go over the same rustls-backed HTTP/2 transport. No `tonic` dependency; the CLI does not pull in a protobuf toolchain.
- JSON request and response bodies go through serde — serialising the `Job` aggregate to JSON once at the edge, and deserialising responses into the shared request/response types (either hand-rolled to match the OpenAPI schema, or generated from it).
- The CLI reuses the existing clap scaffolding in `crates/overdrive-cli/src/main.rs` — Subcommand structure is already in place; this story fills in the handlers.
- `color-eyre` renders the actionable error sections. Custom help text may be wired via `Section` if clarity demands it.
- DESIGN may choose to move error-rendering into a shared helper module if multiple binaries (CLI + future `overdrive-control-plane` + future node agent) would reuse it.
- **Depends on**: US-02 (REST API surface), US-03 (server handlers), US-04 (cluster status surfaces reconciler registry).

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial five user stories for phase-1-control-plane-core DISCUSS wave. |
| 2026-04-23 | Transport pivot: US-02 retitled to "Control-plane HTTP/REST service surface" (was gRPC); US-03 and US-05 re-shaped around axum handlers and a REST client; System Constraints updated to capture the REST + OpenAPI external / tarpc internal two-lane split and the JSON-at-edge / rkyv-at-store serialisation discipline. |
