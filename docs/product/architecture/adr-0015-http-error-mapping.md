# ADR-0015 — HTTP error mapping: typed `ControlPlaneError` enum with pass-through `#[from]`, bespoke 7807-compatible JSON body

## Status

Accepted. 2026-04-23.

## Context

DISCUSS "What DESIGN wave should focus on" items 8 and 9 rolled up into
this ADR. Two concerns:

1. How are typed `thiserror` variants (from `IntentStoreError`,
   `ObservationStoreError`, aggregate constructor errors, reconciler
   runtime errors) mapped to HTTP status codes and JSON bodies?
2. What is the JSON body shape?

Slice 3 failure mode "Error responses collapse distinct failure
classes into 500 Internal Server Error, losing the information the
CLI needs to render actionable output" rules out `anyhow`-on-the-wire.
development.md §Rust patterns — Errors — pass-through embedding rules
out shadow re-declarations of lower-level variants.

Two body shapes were considered:

- **(a) Bespoke JSON**: `{error: <kind>, message: <string>, field:
  <string | null>}`. Simple, one struct, easy to render in the CLI.
- **(b) RFC 7807 `application/problem+json`**: `{type, title, status,
  detail, instance, …}`. Standardised, more ceremony.

## Decision

### 1. One top-level `ControlPlaneError` enum

`overdrive-control-plane::error::ControlPlaneError` — top-level typed
error with pass-through `#[from]` embedding per development.md:

```rust
#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("validation failed: {message}")]
    Validation {
        message: String,
        field:   Option<String>,
    },

    #[error(transparent)]
    Intent(#[from] overdrive_core::traits::intent_store::IntentStoreError),

    #[error(transparent)]
    Observation(#[from] overdrive_core::traits::observation_store::ObservationStoreError),

    #[error(transparent)]
    Aggregate(#[from] overdrive_core::aggregate::AggregateError),

    #[error("resource not found: {resource}")]
    NotFound { resource: String },

    #[error("conflict: {message}")]
    Conflict { message: String },

    #[error("internal error: {0}")]
    Internal(String),
}
```

`Validation`, `NotFound`, and `Conflict` are local variants (the
handler layer's business-logic concerns). `Intent`, `Observation`,
and `Aggregate` are pass-through embeddings via `#[from]` per
development.md — no duplication of lower-level variant shapes.

### 2. One `to_response` function

```rust
// in overdrive-control-plane::error
pub fn to_response(err: ControlPlaneError) -> (StatusCode, Json<ErrorBody>) {
    match err {
        ControlPlaneError::Validation { message, field } => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody { error: "validation", message, field }),
        ),
        ControlPlaneError::NotFound { resource } => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody { error: "not_found", message: resource, field: None }),
        ),
        ControlPlaneError::Conflict { message } => (
            StatusCode::CONFLICT,
            Json(ErrorBody { error: "conflict", message, field: None }),
        ),
        ControlPlaneError::Intent(IntentStoreError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(ErrorBody { error: "not_found", message: "intent-store key not found".into(), field: None }),
        ),
        ControlPlaneError::Intent(_) | ControlPlaneError::Observation(_) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody { error: "internal", message: "store error".into(), field: None }),
        ),
        ControlPlaneError::Aggregate(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody { error: "validation", message: e.to_string(), field: None }),
        ),
        ControlPlaneError::Internal(msg) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody { error: "internal", message: msg, field: None }),
        ),
    }
}
```

The match is **exhaustive at the enum level** — Rust's exhaustiveness
check catches a forgotten variant at compile time. A unit test
additionally enumerates variants and asserts each maps to a specific
status code.

### 3. Body shape: bespoke, 7807-compatible subset

```rust
// in overdrive-control-plane::api
#[derive(Serialize, Deserialize, ToSchema)]
pub struct ErrorBody {
    /// Stable machine-readable error kind, snake_case.
    /// Examples: "validation", "not_found", "conflict", "internal".
    pub error: String,

    /// Human-readable message.
    pub message: String,

    /// Field name when the error is a validation error pointing at
    /// a specific input field; None otherwise.
    pub field: Option<String>,
}
```

This is intentionally a **subset compatible with RFC 7807**: a future
v1.1 add-on of `type: Uri` and `instance: Uri` fields is additive and
non-breaking. Phase 1 does not need them; Phase 5+ (when the gateway
surfaces these to external consumers) can add them.

Response `Content-Type` is `application/json`. A future version may
upgrade to `application/problem+json` when the additional fields land.

### 4. Status-code matrix

| Condition | Status | `error` kind |
|---|---|---|
| Validation failure (newtype or aggregate constructor) | `400 Bad Request` | `"validation"` |
| Unknown resource (DescribeJob on missing JobId) | `404 Not Found` | `"not_found"` |
| Duplicate intent-key with *different* spec (idempotent when byte-identical) | `409 Conflict` | `"conflict"` |
| Infra failure (store I/O, etc.) | `500 Internal Server Error` | `"internal"` |
| Server not running | transport error (no HTTP response at all) | — (CLI renders the transport error) |

Per Slice 3 AC: **byte-identical re-submission of the same spec at the
same intent key is idempotent success (200 OK, same commit_index as
the original)**. A 409 fires only when a re-submission presents a
*different* spec at a key already occupied.

Note: Phase 1's `LocalStore::put` is last-write-wins — there is no
built-in "reject if a different value is present" surface. The Phase 1
handler implements idempotency + conflict detection as a read-then-write
pattern against `LocalStore`: read the key, compare rkyv-archived
bytes, and return 200 or 409 accordingly. A future ADR can harden
this against racing writers (whitepaper §4 guardrails: additive-only
schema, full-row writes).

## Considered alternatives

### Alternative A — RFC 7807 `application/problem+json` from day one

**Rejected for Phase 1.** The ceremony (`type` Uri, `instance` Uri)
adds fields the CLI does not use yet. Shipping the 7807-compatible
subset now means the upgrade is additive, not a refactor.

### Alternative B — Per-handler error enums, not a top-level `ControlPlaneError`

**Rejected.** Five handlers × ~3 variants = 15 variants duplicating
`IntentStoreError` variants by hand. Pass-through `#[from]` embedding
is the development.md pattern exactly because this duplication bites
on the first refactor.

### Alternative C — `anyhow::Error` at the handler boundary

**Rejected.** development.md §Errors — "Library code never returns
`eyre::Report` (or `anyhow::Error`)". A typed enum is the right shape
for mapping to HTTP status codes exhaustively.

### Alternative D — Collapse all non-validation errors to `500 Internal Server Error`

**Rejected.** Slice 3 failure-mode-to-defend: "Error responses
collapse distinct failure classes into `500 Internal Server Error`,
losing the information the CLI needs to render actionable output."
The CLI's error-rendering AC depends on distinguishing 404 from 500.

## Consequences

### Positive

- Exhaustive error-to-status mapping at compile time.
- One `ErrorBody` shape known to the CLI — no ambiguity in parsing.
- Pass-through `#[from]` preserves the underlying variant detail;
  a future Phase 2 `ControlPlaneError::Reconciler(_)` variant is an
  additive edit.
- RFC 7807 upgrade path stays open.

### Negative

- Adding a new error variant requires updating `to_response`. The
  compiler catches a missed match arm (exhaustiveness), so the cost
  is bounded.
- `Internal(String)` is a catch-all — if abused, it becomes the
  "swallow all errors" pattern. Mitigated by enforcing specific
  pass-through variants for each known error source.

### Quality-attribute impact

- **Security — non-repudiation**: positive. Structured error bodies
  are auditable; stack traces never leak.
- **Usability — operability**: positive. CLI renders structured
  bodies with actionable "what / why / how to fix" per
  `nw-ux-tui-patterns`.
- **Maintainability — analyzability**: positive. Each error class is
  a distinct enum variant + status code.

### Enforcement

- A unit test enumerates every `ControlPlaneError` variant and
  asserts `to_response` returns the expected `(StatusCode, error kind)`
  pair.
- A trybuild test asserts `to_response` is exhaustive — removing a
  match arm fails to compile.
- Axum handlers return `Result<impl IntoResponse, ControlPlaneError>`;
  the error is converted via an `IntoResponse` impl that calls
  `to_response`.

## References

- `.claude/rules/development.md` §Rust patterns — Errors
- `docs/feature/phase-1-control-plane-core/discuss/user-stories.md`
  US-02, US-03
- `docs/feature/phase-1-control-plane-core/slices/slice-3-api-handlers-intent-commit.md`
- RFC 7807 (`application/problem+json`) — future upgrade path
