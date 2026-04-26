# ADR-0009 — OpenAPI schema is derived from Rust types via `utoipa`, checked-in, and gated in CI

## Status

Accepted. 2026-04-23.

## Context

The DISCUSS wave identified two load-bearing risks for the external REST
API:

- If the OpenAPI schema drifts from the Rust request/response types with
  no CI gate, CLI and server agree in review but disagree on the wire
  (medium probability, high impact — risk register row in
  `wave-decisions.md`).
- If CLI and server each hand-roll their request/response types, the
  first field rename silently breaks the walking skeleton (slice-2
  failure mode).

The structural fix is: (a) a single source of truth for the wire shape,
(b) a CI gate that fails the build on drift. ADR-0008 picks `axum` for
the server. This ADR picks how the OpenAPI document comes into existence
and is kept honest.

Three candidates were evaluated:

### Candidate 1 — `utoipa` (derive-macros, axum integration via `utoipa-axum`)

- Mature Rust crate (>2k ★, MIT-or-Apache-2), active release cadence,
  first-class `axum` integration through `utoipa-axum`.
- `#[derive(ToSchema)]` on request/response types and `#[utoipa::path(...)]`
  on handlers produce a `utoipa::OpenApi` struct whose `openapi()` method
  returns the schema.
- Workspace already has the ecosystem primitives (`serde`, `serde_json`).
- Negative: the `#[utoipa::path]` annotation duplicates some information
  already present in the axum handler signature — discipline is required
  to keep them aligned (but drift is caught by the CI gate).

### Candidate 2 — `aide` (axum-native, implicit derivation)

- Smaller, newer, axum-first. Less boilerplate — the axum router itself
  is introspected.
- Negative: API surface has historically churned (reviewed upstream
  changelog); smaller community; less cross-referenced in the OpenAPI
  tool ecosystem (swagger-ui integrations, client generators prefer
  utoipa's output shape in practice).

### Candidate 3 — Hand-maintained `api/openapi.yaml`

- Full control; no derive-macro surface.
- Negative: *defeats the purpose*. The risk register entry is explicitly
  about drift between Rust types and the schema. A hand-maintained
  schema IS the drift — the whole point of picking code-first derivation
  is that the Rust types are the single source of truth and the schema
  follows.

## Decision

**The OpenAPI 3.1 schema is derived from the Rust request/response types
via `utoipa`, checked into `api/openapi.yaml`, and regenerated + diffed
on every CI run.**

- **Source of truth**: the Rust types — request/response structs,
  error body shape, enum variants — in the `overdrive-control-plane::api`
  module. Each type carries `#[derive(ToSchema)]`. Each handler carries
  `#[utoipa::path(...)]`.
- **Generated artifact**: `api/openapi.yaml` at the workspace root.
  Checked into git as the canonical wire contract.
- **Derivation command**: `cargo xtask openapi-gen` regenerates
  `api/openapi.yaml` by calling `OverdriveApi::openapi()` on the
  `utoipa::OpenApi`-derived root struct and serialising as YAML.
- **CI gate**: `cargo xtask openapi-check` regenerates to a temp file
  and `diff`s against the checked-in `api/openapi.yaml`. Non-empty
  diff fails the build. Message names the out-of-sync type and
  suggests `cargo xtask openapi-gen` to regenerate.
- **Shared types**: both the axum handlers and the CLI HTTP client
  (ADR-0014) import the same Rust types. The OpenAPI document is
  a *report* of the types; the types are the contract.

The generated schema document does NOT belong in any crate's source
— it lives at the workspace root so downstream consumers (future SDK
generators, Swagger UI on the gateway, external documentation builds)
can reference a single known-stable path.

## Considered alternatives

See Candidates 1-3 in Context above. Candidate 2 (`aide`) is the
closest runner-up; decision can be revisited in Phase 2+ if `utoipa`
ecosystem regresses materially.

Candidate 3 (hand-maintained YAML) is explicitly rejected as counter
to the single-source-of-truth invariant.

## Consequences

### Positive

- Request/response type changes are atomic: rename a field in Rust,
  regenerate the schema, CLI and server stay in lockstep.
- `cargo xtask openapi-check` in CI makes drift a merge-blocker.
- `api/openapi.yaml` is a publishable artifact — operators, future
  SDK consumers, and documentation builders reference one path.
- Works cleanly with `utoipa-swagger-ui` as a future gateway add-on
  (whitepaper §11) without changing the derivation shape.

### Negative

- `#[utoipa::path]` annotations require maintenance alongside the axum
  handler signature — the CI gate catches drift, but the annotation
  is not automatically inferred.
- A future major `utoipa` upgrade may require touching every
  annotation. The workspace pin mitigates this.

### Quality-attribute impact

- **Maintainability — analyzability**: positive. The schema is
  machine-readable from day one.
- **Maintainability — modifiability**: positive. Adding a field is a
  single Rust-side edit + regeneration.
- **Compatibility — interoperability**: positive. OpenAPI 3.1 is the
  universal API-description standard.

### Enforcement

- Workspace adds `utoipa = "5"` with `features = ["axum_extras", "yaml"]`
  and `utoipa-axum = "0.1"` (versions pinned at ADR-writing time; to
  be confirmed against current `crates.io` at implementation).
- `xtask openapi-gen` subcommand added; depends on `overdrive-control-plane`
  and writes `api/openapi.yaml`.
- `xtask openapi-check` subcommand added; diffs generated vs checked-in.
  CI runs it on every PR.
- Removal of `utoipa` or `utoipa-axum` from workspace dependencies
  requires a superseding ADR.
- A unit test in `overdrive-control-plane` enumerates all handlers and
  asserts each has a corresponding `#[utoipa::path]` annotation — so
  "handler shipped without schema entry" is a compile-time failure.

## References

- `docs/whitepaper.md` §3, §4
- `docs/feature/phase-1-control-plane-core/discuss/wave-decisions.md`
  (What DESIGN wave should focus on, item 1)
- `docs/feature/phase-1-control-plane-core/slices/slice-2-rest-service-surface.md`
- ADR-0008 (REST + OpenAPI transport)
- <https://docs.rs/utoipa/> / <https://docs.rs/utoipa-axum/>
