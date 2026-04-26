# ADR-0014 — CLI HTTP client is a hand-rolled `reqwest`-based thin client; CLI and server share the Rust request/response types

## Status

Accepted. 2026-04-23.

## Context

DISCUSS Key Decision 7 + wave-decisions.md "What DESIGN wave should
focus on" item 2 asked: hand-rolled `reqwest`-style client against the
OpenAPI schema, or a fully OpenAPI-generated Rust client?

`reqwest` (0.12) is already in the workspace root (`hyper`-backed,
`rustls`-enabled). Two candidate generator toolchains were reviewed:

- **OpenAPI Generator** — Java-based CLI tool; runs against the
  OpenAPI document; produces a Rust crate as output. Pulls `reqwest`
  under the hood. Adds a Java / Maven (or Gradle) build-time dep
  unless shelled to a prebuilt Docker image; neither option is
  Rust-native.
- **Progenitor** (Oxide Computer) — pure-Rust. `progenitor-client`
  generates code via a build script or proc-macro. Mature, MIT,
  active; designed for Rust-to-Rust REST bridges.

## Decision

**Phase 1 CLI uses a hand-rolled `reqwest`-based thin client. The CLI
and the server share the same Rust request/response types, imported
from a common module.**

Concretely:

- **Shared types module**: `overdrive-control-plane::api` exports every
  request and response type (`SubmitJobRequest`, `SubmitJobResponse`,
  `JobDescription`, `ClusterStatus`, `AllocStatusResponse`,
  `NodeList`, `ErrorBody`) with `#[derive(Serialize, Deserialize,
  ToSchema)]` derives.
- **The OpenAPI schema is a report of the types, not their source.**
  `utoipa` (ADR-0009) derives the schema from these types; the types
  remain the contract.
- **The CLI imports from `overdrive-control-plane::api`** as a regular
  Rust dependency. No code generation step. No second source of truth.
- **The CLI HTTP client** is a thin module in `overdrive-cli` (or a
  tiny sibling crate `overdrive-api-client` if a second consumer
  appears — not Phase 1). Responsibilities:
  - Load `~/.overdrive/config` to obtain endpoint + trust triple
    (ADR-0010).
  - Build a `reqwest::Client` with CA pinned, client cert + key
    attached for future mTLS (Phase 5 adds the actual verification;
    Phase 1 presents the cert but the server does not yet validate
    it — noted in ADR-0010).
  - Expose one method per endpoint: `client.submit_job(req) ->
    Result<SubmitJobResponse>`, `client.describe_job(id) -> …`, etc.
  - Map HTTP error responses (400 / 404 / 409 / 500 with structured
    `ErrorBody`) to a typed `CliError` enum. No bare string errors.
  - Under ~200 LoC total.

### Why not a generator

- Generator toolchains (OpenAPI Generator, Progenitor) add a build
  step for a 5-endpoint surface — over-engineered at current scope.
- The CLI and server are one workspace; shared Rust types are the
  native Rust-to-Rust contract. Code generation is for
  cross-language SDKs, which DESIGN explicitly defers to Phase 2+.
- Progenitor is the right tool *if* a second language needs a client.
  For Phase 1 Rust-to-Rust, type-sharing is lighter and better.

### Why shared types, not independent per-side types

- Slice 2's failure mode "The CLI hand-rolls request/response types
  that shadow the schema-generated ones, causing field drift from
  the server" is avoided by construction.
- A rename on the server is a rename on the CLI by Rust's type system.
- The OpenAPI schema is a byproduct of the types, not a parallel
  contract, so drift between the schema and the types is the only
  risk (addressed by ADR-0009's CI gate).

## Considered alternatives

### Alternative A — OpenAPI Generator (Java-based)

**Rejected.** Brings a Java / Maven / Docker build-time dep into a
Rust-throughout workspace. Violates design principle 7. Generator
output would still need to compile against the same workspace
dependencies.

### Alternative B — Progenitor (Rust-native generator)

**Rejected for Phase 1**; kept as a Phase 2+ option. The value of
a generator is cross-consumer consistency; with one consumer (the
CLI) and shared types via direct import, there is nothing to
generate. Re-evaluate in Phase 2 if a second Rust client appears
(node agent over tarpc is not a REST consumer, so this is unlikely
to reactivate).

### Alternative C — CLI defines its own request/response types

**Rejected.** The exact drift-bug the DISCUSS-wave risk register
flagged. Shipping two definitions of `SubmitJobRequest` is the
antipattern the OpenAPI CI gate cannot catch (the schema describes
only the server's definition).

### Alternative D — Place shared types in a separate crate `overdrive-api-types`

**Considered viable**, rejected for Phase 1 on YAGNI grounds. The
`overdrive-control-plane::api` module is the current home; promotion
to a separate crate is cheap and can happen in Phase 2 if a second
binary consumes the types (likely candidates: a future REST-based
test fixture, a future OpenAPI playground).

## Consequences

### Positive

- Zero build-time code generation; zero non-Rust toolchain deps.
- CLI / server type parity is compiler-enforced across the workspace.
- `reqwest` is already a workspace dep — no new dependency.
- ~200 LoC of CLI client is trivially auditable.

### Negative

- The CLI crate depends on `overdrive-control-plane` (for the api
  module). Adds a compile-time coupling. Mitigated by splitting api
  into a submodule that could be promoted to a separate crate
  later.
- No auto-generated SDK for non-Rust consumers in Phase 1. The
  OpenAPI schema is still published for future generation.

### Quality-attribute impact

- **Maintainability — modifiability**: positive. Request/response
  changes are one place.
- **Maintainability — modularity**: neutral. The CLI / server
  coupling is real but bounded to one module.
- **Compatibility — interoperability**: deferred to the OpenAPI
  schema; non-Rust consumers generate their own clients from it in
  Phase 2+.

### Enforcement

- A unit test in the CLI crate enumerates every endpoint method and
  asserts it matches a handler in `overdrive-control-plane`.
- The acceptance test suite round-trips submit → describe through
  the real HTTP client against the real axum server (no mocks).
- The CLI client does NOT import anything from `axum` or `hyper` —
  it goes through `reqwest`. A cargo-deny-like test asserts this.

## References

- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
  US-05
- `docs/feature/phase-1-control-plane-core/slices/slice-5-cli-handlers.md`
- ADR-0008 (REST + OpenAPI transport)
- ADR-0009 (OpenAPI schema derivation)
- ADR-0010 (Phase 1 TLS bootstrap)
