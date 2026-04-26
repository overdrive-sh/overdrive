# Definition of Ready Validation — phase-1-control-plane-core

9-item DoR checklist applied to each of the five user stories in `user-stories.md`. Per the product-owner skill's hard-gate rule, DESIGN wave does not start until every item passes with evidence.

---

## Story: US-01 — Job / Node / Allocation aggregates + canonical intent keys

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with Ana's experience of finding newtypes shipped but aggregates missing; domain language ("round-trip a job spec", "spec digest") is ubiquitous and traceable to whitepaper §4. |
| 2. User/persona with specific characteristics | PASS | Ana, Overdrive platform engineer working across control plane, CLI, and store; motivation named (rely on one aggregate shape, one canonical key). Consistent with phase-1-foundation persona. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) TOML file `payments.toml` with `JobId("payments")` constructor path; (b) intent-key derivation `jobs/payments` matched across CLI and control plane; (c) error boundary rejecting `memory_bytes = 0`. Real names, real fields. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: rkyv round-trip, canonical intent-key match, malformed replicas rejection, Node resource sanity, typed-ID-only aggregate fields. Within 3-7 band. |
| 5. AC derived from UAT | PASS | 8 AC bullets each trace to a scenario (round-trip, canonical derivation, validating constructors, typed fields, reuse of `Resources`). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~1 day effort; 5 scenarios; single concern (aggregate model + canonical key function). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address `JobSpec` placeholder in `traits/observation_store.rs` (DESIGN decides), `Resources` reuse from `traits/driver.rs`, and phase-1-foundation newtype dependency. |
| 8. Dependencies resolved or tracked | PASS | Depends on phase-1-foundation newtypes (all 11 shipped per evolution record). Nothing external pending. |
| 9. Outcome KPIs with measurable targets | PASS | KPI row targets 3 aggregates shipped + 0 duplicate `Resources`/intent-key derivations; measurement via proptest + workspace grep. |

### DoR Status: **PASSED**

---

## Story: US-02 — Control-plane HTTP/REST service surface

Re-validated on 2026-04-23 after the transport pivot from gRPC/tonic to REST + OpenAPI + axum/rustls. Every item passes under the new shape; see § Upstream Changes at the bottom of this file for the delta record.

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the concrete failure mode ("first field rename silently breaks the walking skeleton") and ties to whitepaper §3/§4 (two-lane transport split) + CLI/server split. |
| 2. User/persona with specific characteristics | PASS | Ana as a platform engineer building either CLI or server; motivation ("one wire contract, not two"). |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) happy-path POST `/v1/jobs` returning 200 + JSON body with `commit_index`; (b) validation error mapping to `400 Bad Request` with structured JSON body naming the offending field; (c) server mid-shutdown producing a connection-level error + actionable CLI message. Real endpoint paths, real HTTP status codes. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: OpenAPI schema as single source of truth, submit round-trip, typed error → HTTP status code, SIGINT clean shutdown, unreachable-endpoint actionable error. Within 3-7 band. |
| 5. AC derived from UAT | PASS | 9 AC bullets each trace to a scenario (OpenAPI schema derivation, schema-lint CI gate, axum + rustls bind/serve/shutdown, endpoints enumerated, error mapping table, actionable unreachable render, default https endpoint). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~1 day effort; 5 scenarios; single concern (REST contract, schema as SSOT, axum scaffolding). Scope did not grow with the pivot — axum + utoipa/aide + rustls is not more work than tonic + prost. |
| 7. Technical notes: constraints/dependencies | PASS | Notes name the expected crate stack (axum + rustls + serde + utoipa or aide), the OpenAPI `v1` versioning discipline, deferred streaming, Phase 5 deferred auth, and internal tarpc / postcard-rpc deferred to `phase-1-first-workload`. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-01 (aggregate types carried in JSON bodies). Phase 5 auth explicitly out of scope. |
| 9. Outcome KPIs with measurable targets | PASS | 1 OpenAPI document; schema-lint gate green on every PR; 100% of typed error variants have an HTTP status mapping; measurement via CI schema-lint check + workspace search for duplicate request/response structs + variant-enumeration test. |

### DoR Status: **PASSED** (re-validated post-pivot)

---

## Story: US-03 — API handlers commit to IntentStore + ObservationStore reads

Re-validated on 2026-04-23 after the transport pivot. Scenario titles, AC, and technical notes were updated to reference axum handlers, JSON request/response bodies, and HTTP status codes (400/404/409/500) instead of gRPC codes. Every item still passes; the re-validation is material only to items 4, 5, and 7.

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the round-trip invariant at stake + traceability to phase-1-foundation's DST harness and the walking-skeleton hypothesis. |
| 2. User/persona with specific characteristics | PASS | Three personas named: the server-side engineer, Ana via CLI (indirect), and a future single-mode operator. Each motivation distinct. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) submit-then-describe with `payments.toml` and `commit_index = 17` via POST `/v1/jobs` and GET `/v1/jobs/{id}`; (b) monotonic commit_index across two different specs; (c) rkyv round-trip boundary with a hypothetical archival divergence landing as `409 Conflict`. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 6 scenarios: submit-describe byte-identical, monotonic commit_index, validating gate blocks writes (`400 Bad Request`), empty AllocStatus, empty NodeList, NotFound on unknown JobId (`404 Not Found`). Within band. |
| 5. AC derived from UAT | PASS | 9 AC bullets trace to scenarios: axum handler path, IntentStore put/get, commit_index accessor, validating gate, HTTP status-code discipline (400/404/409/500) with structured JSON error bodies, empty-row discipline, round-trip proptest. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~1 day effort; 6 scenarios; single concern (four handler bodies + one accessor). |
| 7. Technical notes: constraints/dependencies | PASS | Notes: Strategy C real redb per phase-1-foundation DWD-01; commit_index surface cannot leak redb internals; `SimObservationStore` vs trivial in-process LWW is DESIGN's call; HTTP status-code convention is spelled out (400/404/409/500 + structured JSON bodies). |
| 8. Dependencies resolved or tracked | PASS | Depends on US-01 + US-02. Nothing else. |
| 9. Outcome KPIs with measurable targets | PASS | K1 (100% round-trip byte-identical), K3 (100% monotonic commit_index); proptest + monotonicity test. |

### DoR Status: **PASSED** (re-validated post-pivot)

---

## Story: US-04 — Reconciler primitive: trait + runtime + evaluation broker

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the Nomad-incident precedent (whitepaper §18) + the pure-function contract risk. Domain language is whitepaper-exact. |
| 2. User/persona with specific characteristics | PASS | Three personas: Phase 2+ reconciler authors, the DST harness as enforcer, Ana via `cluster status`. Each motivation distinct. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) writing `MyReconciler` against the trait; (b) three evaluations collapsing at the same `(noop-heartbeat, job/payments)` key with dispatched=1, cancelled=2; (c) `Instant::now()` smuggling caught by dst-lint AND `reconciler_is_pure`. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 7 scenarios: trait purity signature, at-least-one registered, duplicate collapse, reaper bounds set, libSQL isolation, `reconciler_is_pure` twin invocation, `cluster status` surface. Exactly at the top of the band. |
| 5. AC derived from UAT | PASS | 11 AC bullets trace to scenarios; each invariant / observable has a dedicated bullet. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | WARN — marked ~1-2 days in the slice brief (at the upper end of right-sized). 7 scenarios is at the band's top edge. Scope remains a single primitive; if effort exceeds 2 days during DESIGN / DELIVER, a split by outcome would be: 4A = trait + runtime + libSQL isolation + noop-heartbeat; 4B = evaluation broker + cancelable-eval-set + new DST invariants. Flagged but not blocking; DESIGN can split if they discover material complexity. |
| 7. Technical notes: constraints/dependencies | PASS | Notes address purity as load-bearing, reaper shape (in-runtime loop acceptable in Phase 1), Db type location deferred to DESIGN, `async_trait` vs native-async deferred. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-01. Independent of US-02/US-03 and can run in parallel. |
| 9. Outcome KPIs with measurable targets | PASS | K4 covers the three DST invariants; measurable pass/fail per CI run. |

### DoR Status: **PASSED** — with a flagged right-sizing note (see item 6 above).

---

## Story: US-05 — CLI handlers for job / alloc / cluster / node

Re-validated on 2026-04-23 after the transport pivot. Scenario titles, AC, and technical notes were updated to describe a REST HTTP client (hand-rolled `reqwest`-style or OpenAPI-generated), `https://127.0.0.1:7001` default endpoint, and actionable errors sourced from HTTP responses (not gRPC `Status` variants).

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the exact tracing::warn line the stub emits today + the walking-skeleton hypothesis the stub fails to meet. |
| 2. User/persona with specific characteristics | PASS | Ana + CI smoke tests + future operator. Each motivation distinct. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) `payments.toml` submit + inspect with real digest and `https://127.0.0.1:7001` endpoint; (b) fresh cluster with zero nodes showing explicit empty states; (c) down endpoint rendered as three concrete next steps (the example error block no longer mentions any transport-specific panic). |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 7 scenarios: submit round-trip, spec-digest parity with local file, honest node list empty state, cluster status reconciler registry, unreachable-endpoint actionable error (no raw `ECONNREFUSED` or `reqwest::Error` debug format), malformed-spec field surface, endpoint precedence (with https default). Exactly at the top of the band. |
| 5. AC derived from UAT | PASS | 10 AC bullets trace to scenarios (round-trip via REST endpoints, empty states, error shape sourced from HTTP responses, exit codes, endpoint precedence with https default, first-output latency, no-blank-table discipline). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~1 day effort; 7 scenarios; single concern (four CLI subcommand handlers + shared error renderer). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address TOML parsing path, HTTP client shape (DESIGN picks between `reqwest`-style and OpenAPI-generated), reuse of existing clap scaffolding, color-eyre rendering, potential error-helper module for shared use. No `tonic` dependency, no `protoc` in the CLI toolchain. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-02, US-03, US-04. All in this feature's scope. |
| 9. Outcome KPIs with measurable targets | PASS | K1 (round-trip), K6 (actionable errors), K7 (empty states). All testable. |

### DoR Status: **PASSED** (re-validated post-pivot)

---

## Overall DoR summary

| Story | Status | Notes |
|---|---|---|
| US-01 | PASSED | — |
| US-02 | PASSED | Re-validated post transport pivot (REST + OpenAPI, axum, rustls). Story retitled; all 9 items still PASS with updated evidence. |
| US-03 | PASSED | Re-validated post transport pivot (axum handlers, JSON bodies, HTTP status codes). Items 4/5/7 updated; all 9 items still PASS. |
| US-04 | PASSED | Flagged right-sizing note — if DESIGN / DELIVER discovers the effort exceeds 2 days, a split path (4A / 4B) is pre-described in the DoR row. No transport-pivot impact. |
| US-05 | PASSED | Re-validated post transport pivot (HTTP client, https default endpoint, HTTP error shape). Items 3/4/5/7 updated; all 9 items still PASS. |

**Feature DoR status: PASSED for all 5 stories.** Handoff to DESIGN (solution-architect) can proceed pending peer review approval.

## Upstream Changes

**UC-1 (2026-04-23) — Transport pivot re-validation.** After the REST + OpenAPI external / tarpc internal split was committed upstream (whitepaper §3/§4 + GH #9 body), US-02 was rewritten (transport-specific content), and US-03 and US-05 had their transport-touching AC / scenarios updated. DoR was re-run against all three affected stories.

**Delta:**

- **US-02**: 9 items PASS post-pivot (was 9 PASS pre-pivot). Item 6 (right-sized) specifically confirmed: the axum + utoipa/aide + rustls stack is not more work than the prior tonic + prost stack; scope did not grow.
- **US-03**: 9 items PASS post-pivot (was 9 PASS pre-pivot). Items 4 (UAT), 5 (AC), 7 (Technical notes) had material updates — HTTP status codes (400/404/409/500) replaced gRPC codes; structured JSON error bodies replaced gRPC status messages.
- **US-05**: 9 items PASS post-pivot (was 9 PASS pre-pivot). Items 3 (examples), 4 (UAT), 5 (AC), 7 (Technical notes) had material updates — REST client shape, https default endpoint, HTTP response-shape errors replaced tonic-specific renderings.
- **US-01, US-04**: not re-validated (no transport-specific content in either; US-01 is pure aggregates + rkyv + canonical keys, US-04 is the reconciler primitive which is internal to the control plane and transport-neutral).

**No items flipped PASS → FAIL.** Zero new blockers. The pivot is a shape change within the same DoR envelope, not a scope expansion.

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial DoR validation for phase-1-control-plane-core. |
| 2026-04-23 | Re-validated US-02, US-03, US-05 after the transport pivot (REST + OpenAPI + axum/rustls external; tarpc / postcard-rpc internal, future). All three stories still PASS. See § Upstream Changes. |
