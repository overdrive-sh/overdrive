# Slice 2 — Control-plane HTTP/REST service surface

**Story**: US-02
**Walking skeleton row**: 2 (Speak REST)
**Effort**: ~1 day
**Depends on**: Slice 1 (aggregate structs are carried in JSON request / response bodies).

## Outcome

The control-plane API is exposed over REST + JSON with an OpenAPI 3.1 schema as the single source of truth. An axum router binds over HTTP/2 with `rustls`, exposes the walking-skeleton endpoints under the `/v1` prefix, and maps typed `thiserror` variants to HTTP status codes through one function. The OpenAPI document is derived from the Rust request / response types and gated by a schema-lint check in CI.

## Value hypothesis

*If* the CLI and the server compile against diverged request / response types (or against an OpenAPI schema that has drifted from the Rust source of truth), *then* the walking skeleton silently fails on the first field mismatch — and the public REST contract loses the "curl-it-to-explore" property that made REST worth picking. A schema that is always derived from the Rust types and always checked against a canonical document is the only structural mitigation; retrofitting a schema after the fact requires coordinating a field rename across the CLI, the server, and any external consumer.

## Scope (in)

- OpenAPI 3.1 schema as the single source of truth for the control-plane wire contract — derived from the Rust request / response types via `utoipa` or `aide` (DESIGN picks) and checked into the workspace (e.g. `api/openapi.yaml`) or regenerated on build with a CI check
- Schema-lint gate in CI (`openapi-check`-style): fails the build if the generated schema drifts from the checked-in document
- Axum router skeleton wired to the endpoints below
- HTTP/2 + `rustls` transport using a self-generated local-dev certificate (operator mTLS is Phase 5)
- Endpoints (walking-skeleton set; DESIGN confirms exact shapes):
  - `POST /v1/jobs` — SubmitJob, accepts `Json<SubmitJobRequest>`, returns `Json<SubmitJobResponse> { job_id, commit_index }`
  - `GET /v1/jobs/{id}` — DescribeJob, returns `Json<JobDescription> { spec, commit_index, spec_digest }`
  - `GET /v1/cluster/info` — ClusterStatus, returns `Json<ClusterStatus> { mode, region, commit_index, reconcilers, broker }`
  - `GET /v1/allocs` — AllocStatus, returns `Json<AllocStatusResponse> { rows }`
  - `GET /v1/nodes` — NodeList, returns `Json<NodeList> { rows }`
- Error mapping: typed `thiserror` enum → `(StatusCode, Json<ErrorBody>)` with a structured JSON body (never a raw stack trace)
- Server lifecycle: binds, listens, serves; clean shutdown on SIGINT with in-flight-request drain
- Server is Phase-1 auth posture: no authentication, localhost-default bind at `https://127.0.0.1:7001`

## Scope (out)

- Request-body validation beyond structural JSON parse — the validating constructor sits in the handler (slice 3)
- Authentication (operator mTLS, SPIFFE operator IDs, Biscuit bearer tokens) — Phase 5
- Internal RPC to a node agent (`tarpc` / `postcard-rpc`) — ships with `phase-1-first-workload`
- Streaming responses (server-sent events, `Watch` endpoints) — deferred
- Public-trust TLS via ACME — Phase 2+ via gateway subsystem
- REST → gRPC-Web bridge — not on the roadmap (tarpc is the internal answer)
- OpenAPI-generated SDKs for non-Rust languages — downstream of v1 stability

## Target KPI

- Round-trip `POST /v1/jobs` → `GET /v1/jobs/{id}` returns bytes equal to what was submitted, for any valid input (proptest through the real server)
- Every `thiserror` variant in the submit path has an HTTP status mapping asserted by a unit test
- CLI and server compile against types derived from (or matching) the same OpenAPI schema (no per-side field adaptation outside the generated module)
- Schema-lint gate green on every PR — the checked-in OpenAPI document never drifts from the Rust source of truth
- Error responses answer "what / why / how to fix" per `nw-ux-tui-patterns` (rendered by the CLI from the structured JSON body)

## Acceptance flavour

See US-02 scenarios. Focus: OpenAPI schema as single source of truth, axum server lifecycle, structured HTTP error mapping, HTTP/2 + rustls local-dev cert.

## Failure modes to defend

- OpenAPI schema drifts from the Rust types and no CI gate catches it → CLI and server agree in review but disagree on the wire
- The CLI hand-rolls request / response types that shadow the schema-generated ones → field renames break silently
- Error mapping loses the underlying variant detail (collapsed to a generic `500 Internal Server Error`)
- Server binds but never responds to SIGINT (shutdown path broken)
- Rust-type response shapes drift between `v1` and an unreleased breaking change without a compatibility check or a `v2` path
- `protoc` or a gRPC dependency sneaks back into the workspace through a transitive import
