# ADR-0008 — Control-plane external API is REST + OpenAPI over axum/rustls

## Status

Accepted. 2026-04-23.

## Context

Phase 1 control-plane-core ships the first operator-observable surface on top
of the walking skeleton. Whitepaper §3 + §4 were edited on 2026-04-23 to
name the external API as **REST + OpenAPI over axum/rustls for external
clients** and `tarpc` / `postcard-rpc` for internal control-flow streams
(node agent — not this feature). The DISCUSS wave recorded the pivot under
Upstream Changes UC-1; DESIGN codifies it.

The prior candidate — gRPC via `tonic` with a `.proto` single source of
truth — was superseded for three reasons recorded in GH #9 and whitepaper
§3/§4:

- Overdrive is Nomad-shaped (Rust-throughout, single-binary, no committed
  public multi-language SDK story at v0.12). gRPC's cross-language value
  does not apply.
- REST is the universal public contract: `curl` exploration works against
  it; OpenAPI-generated SDKs exist for every mainstream language; HTTP-native
  auth (OIDC, `Authorization: Bearer <biscuit>`) maps cleanly onto the
  Phase 5 / Phase 7 operator-auth story.
- `tarpc` / `postcard-rpc` keep internal paths pure Rust (design principle 7)
  without `protoc` or a protobuf code-generation step.

The concrete wiring choices this ADR pins are: the HTTP server framework,
the TLS stack, and the HTTP version posture. Schema derivation is a
separate decision (ADR-0009). TLS bootstrap is a separate decision
(ADR-0010).

## Decision

**The Phase 1 control-plane external API is HTTP + JSON served by
`axum` over `hyper` with `rustls` TLS, HTTP/2 preferred (ALPN `h2`)
with HTTP/1.1 fallback (ALPN `http/1.1`), routes under the `/v1` prefix.**

- Server framework: `axum` (v0.7+) — the idiomatic Rust-native HTTP server
  framework, built on `hyper` + `tower`. Types: request extractors
  (`Json<T>`, `Path<T>`, `Query<T>`), responses `impl IntoResponse`.
- Transport: `hyper` (already in workspace) with `rustls` (already in
  workspace). No `openssl`, no `native-tls`.
- HTTP version: ALPN advertises `h2, http/1.1`. Clients that speak HTTP/2
  get HTTP/2; legacy tooling that insists on HTTP/1.1 still works. No
  HTTP/1.1-only downgrade paths.
- URL prefix: `/v1`. A future breaking change becomes `/v2` served in
  parallel during the deprecation window. Non-breaking additions go
  under `/v1`.
- Binding: `https://127.0.0.1:7001` default, overridable by flag.
- Endpoints (walking-skeleton set; exact shapes fixed by the OpenAPI
  schema per ADR-0009):
  - `POST /v1/jobs` — SubmitJob
  - `GET /v1/jobs/{id}` — DescribeJob
  - `GET /v1/cluster/info` — ClusterStatus
  - `GET /v1/allocs` — AllocStatus
  - `GET /v1/nodes` — NodeList

The server binary lives in a new crate, `crates/overdrive-control-plane`,
class = `adapter-host`. That crate owns: the axum router, the TLS
bootstrap (ADR-0010), the handler module (Slice 3), the ReconcilerRuntime
wiring (ADR-0013), and the `ObservationStore` adapter wiring (ADR-0011).

## Considered alternatives

### Alternative A — Keep gRPC via `tonic`

**Rejected.** Superseded by whitepaper §3/§4 edits and GH #9 body.
Brings `protoc` into the toolchain; no operator reaches for `grpcurl`
first; code-first schema derivation is weaker in tonic than in utoipa.

### Alternative B — `actix-web` or `rocket`

**Rejected.** `axum` is the community default on top of `hyper`/`tower`
with the strongest `utoipa`/`aide` integration (ADR-0009). `actix-web`
has its own runtime-interop quirks relative to `tokio`-only stacks, and
`rocket`'s last stable release shape has lagged the `axum`/`tower`
ecosystem on streaming, HTTP/2, and rustls wiring.

### Alternative C — Hand-rolled `hyper` server without `axum`

**Rejected.** `axum` is a thin layer on top of `hyper` — it does not
add a runtime dependency of meaningful weight, and its extractor / router
model is the standard Rust HTTP pattern. Hand-rolling would lose the
`utoipa-axum` integration entirely.

### Alternative D — HTTP/1.1 only

**Rejected.** Every client already speaks HTTP/1.1 and `curl`'s
`--http2` flag is a one-liner; there is no downside to advertising
`h2` via ALPN. HTTP/2 is future-compatible with gateway subsystem
streaming (whitepaper §11).

## Consequences

### Positive

- Single wire protocol for every external consumer (CLI today, future
  OpenAPI-generated SDKs).
- `curl` exploration works against the local endpoint from day one.
- `axum` is already the target of `utoipa-axum` for OpenAPI derivation
  (ADR-0009).
- `rustls`-based TLS stays pure-Rust — design principle 7.
- HTTP-native auth paths (OIDC, bearer tokens) compose naturally for
  Phase 5 / Phase 7.

### Negative

- `axum` + `hyper` version cadence is faster than a hand-rolled server
  — workspace dependency upgrades become a platform concern.
- Shared type derivation couples server and CLI to a single Rust
  workspace; a non-Rust client must go through the OpenAPI schema
  (which is the point — ADR-0009).

### Quality-attribute impact

- **Performance efficiency — time behaviour**: negligible impact at
  Phase 1 scope. Localhost REST + rustls is <1 ms round-trip; far below
  any relevant threshold.
- **Maintainability — modularity**: positive. The axum router /
  handler split matches the ports-and-adapters hexagon: handlers are
  the primary (driving) port; they invoke the trait-object
  `IntentStore` / `ObservationStore` / `ReconcilerRuntime` behind them.
- **Compatibility — interoperability**: positive. REST + OpenAPI is
  the universal public-API shape.
- **Security — confidentiality / integrity**: `rustls` + TLS 1.3 by
  default. See ADR-0010 for the trust-bootstrap shape.

### Enforcement

- Workspace adds `axum` at the version pinned in `Cargo.toml` (DESIGN
  recommends `axum = "0.7"`, `axum-server = "0.7"` for TLS bind).
- A CI test asserts the server binds on `https://127.0.0.1:7001` (or
  a port chosen at test-time) and advertises `h2, http/1.1` via ALPN.
- The `/v1` prefix is enforced by a single `Router::nest("/v1", ...)`
  call at the top of the router module; a trybuild or unit test
  asserts no handler is mounted at the root.
- No `tonic`, `prost`, or `protoc` appears in the dependency graph
  of any new crate — verified by `cargo tree -e normal` snapshot
  held in CI.

## References

- `docs/whitepaper.md` §3 (architecture diagram), §4 (control plane)
- GH #9 (retitled "REST + OpenAPI (external) and tarpc (internal)")
- `docs/feature/phase-1-control-plane-core/discuss/wave-decisions.md`
  UC-1
- `docs/feature/phase-1-control-plane-core/slices/slice-2-rest-service-surface.md`
- ADR-0009 (OpenAPI schema derivation)
- ADR-0010 (Phase 1 TLS bootstrap)
