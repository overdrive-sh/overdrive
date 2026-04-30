# ADR-0032 — NDJSON streaming submit: `Accept`-gated content negotiation on `POST /v1/jobs`; flat `SubmitEvent` enum with structured `TransitionReason`

## Status

Accepted. 2026-04-30. Decision-makers: Morgan (proposing), DISCUSS-wave
ratification of [D1] / [D5] / [D7] (carried into DESIGN as constraints
[C2] / [C5] / [C6]).

Tags: phase-1, cli-submit-vs-deploy-and-alloc-status, application-arch,
http-shape.

## Context

Feature `cli-submit-vs-deploy-and-alloc-status` ratified Option S
(submit streams convergence by default) in DIVERGE and locked NDJSON
over SSE in DISCUSS. Both decisions are out of scope to re-open; this
ADR records the **HTTP wire shape, the typed event enum, the wall-clock
cap mechanism, the subscription mechanism, and the OpenAPI declaration**
that the streaming-submit slice (Slice 02) is built against.

The user's actual complaint was a single session:

```
$ overdrive job submit ./payments-v2.toml
Accepted.

$ overdrive alloc status --job payments-v2
Allocations: 1
```

The binary was missing. The platform reported neither. This ADR is the
HTTP-layer half of the answer; ADR-0033 is the snapshot half. Both
share a single source of truth for `transition_reason` per [D7] / [C6].

The DISCUSS wave fixed five things that this ADR honours as locked:

- NDJSON over SSE for the streaming wire format.
- `Accept: application/x-ndjson` is the gating header.
- CLI exit codes are 0 / 1 / 2; sysexits.h reserved.
- Server-side wall-clock cap MUST exist (value = DESIGN call).
- Existing JSON ack shape is RETAINED for back-compat.

DESIGN-wave open questions resolved here: granularity of the event
enum (flat vs discriminated union vs two-level); placement of the
wall-clock cap (handler-local vs middleware vs subscription primitive);
subscription mechanism (push vs pull vs hybrid); OpenAPI declaration
of NDJSON; HTTP error semantics in the streaming context.

## Decision

### 1. Endpoint shape — content-negotiated `POST /v1/jobs`

The streaming surface lives at the **existing `POST /v1/jobs` path**.
The `Accept` request header decides the response shape:

| `Accept` header | Response `Content-Type` | Response body |
|---|---|---|
| `application/x-ndjson` | `application/x-ndjson` (chunked) | One `SubmitEvent` per line; stream closes on terminal event |
| `application/json` (or absent) | `application/json` | Single `SubmitJobResponse` JSON object (existing shape, unchanged) |

No new endpoint. No `POST /v1/jobs/stream`, no `POST /v1/jobs:submit`.
The single-endpoint-with-content-negotiation pattern is the REST shape
ADR-0008 §versioning rule already commits to. A future v1.1 could
emit `application/problem+json` for error responses (per ADR-0015
positive-future-direction), and the same content-negotiation shape
absorbs that addition.

A request that explicitly sends `Accept: application/x-ndjson` AND
fails *before* any `SubmitEvent` is emitted (validation error,
conflict, internal error during `IntentStore::put_if_absent`) returns
the existing 4xx/5xx `ErrorBody` JSON shape — the handler has not yet
switched to chunked transfer at that point. Once the first
`SubmitEvent::Accepted` line is on the wire, every subsequent failure
mode is structured as a `SubmitEvent` line, not as an HTTP error
(see §6 below).

### 2. Event enum — flat top-level with structured `reason` (Call A → A1)

```rust
// in overdrive-control-plane::api

/// One line on the NDJSON wire. `#[serde(tag = "kind")]` makes each
/// line self-describing; consumers can match on `kind` without
/// trial-deserialization.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubmitEvent {
    /// First line. Carries the same fields the existing
    /// `SubmitJobResponse` does — the back-compat surface from the
    /// JSON lane is the first line on the streaming lane.
    Accepted {
        spec_digest: String,
        intent_key:  String,
        outcome:     IdempotencyOutcome,
    },

    /// Per-AllocStatusRow transition. Both `from` and `to` are
    /// rendered, plus the structured `reason`, the optional opaque
    /// `detail` (verbatim driver text on the failure path), the
    /// source layer, and an RFC 3339 timestamp.
    LifecycleTransition {
        alloc_id: String,
        from:     AllocStateWire,
        to:       AllocStateWire,
        reason:   TransitionReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail:   Option<String>,
        source:   TransitionSource,
        at:       String,                       // RFC 3339
    },

    /// Terminal success. Stream closes after this line.
    ConvergedRunning {
        alloc_id:   String,
        started_at: String,                     // RFC 3339
    },

    /// Terminal failure. Stream closes after this line. The CLI
    /// exits 1 regardless of the inner `terminal_reason`; the
    /// reason controls *rendering*, not exit code.
    ConvergedFailed {
        #[serde(skip_serializing_if = "Option::is_none")]
        alloc_id:        Option<String>,
        terminal_reason: TerminalReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason:          Option<TransitionReason>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error:           Option<String>,        // verbatim driver text or "did not converge in 60s"
    },
}
```

### 3. Structured reason types

**Amended 2026-04-30**: `TransitionReason` is **cause-class** —
failure variants carry typed payloads naming the structured cause;
progress markers retain their phase-naming form. The enum is no
longer `Copy + Hash` (cause-class payloads include `String` /
non-`Copy` data) and `Box<TransitionReason>` is rejected for any
recursive shape (rkyv `Archive` cannot resolve recursion). See
`Amendment 2026-04-30` at the foot of this ADR for the full
rationale; the original state-class shape (`DriverStartFailed`,
`BackoffExhausted`, `Stopped`, `NoCapacity` — all unit variants)
is now the rejected alternative captured under `Alternative D`.

```rust
// in overdrive-core (so both action shim and reconciler can produce);
// re-exported through overdrive-control-plane::api with ToSchema.

/// Tagged-payload wire shape: serde emits
///   {"kind":"scheduling"}                                              for unit
///   {"kind":"exec_binary_not_found","data":{"path":"/usr/local/..."}}  for cause
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema,
         rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransitionReason {
    // --- Progress markers (payload-less or minimal payload) ----------
    /// Reconciler picked a placement; action was emitted.
    Scheduling,
    /// Driver invocation underway.
    Starting,
    /// Driver returned `Ok(handle)`.
    Started,
    /// Reconciler holding off restart per backoff window. `attempt`
    /// is the 1-indexed retry that fires when the backoff elapses.
    BackoffPending { attempt: u32 },
    /// Reconciler observed terminal stop. `by` distinguishes operator
    /// stop intent from converged terminal state.
    Stopped { by: StoppedBy },

    // --- Cause-class failure variants (Phase 1 ExecDriver-observable)-
    /// `spawn(2)` returned ENOENT. Replaces the old `DriverStartFailed`
    /// for the missing-binary case (US-02 KPI-02 regression target).
    ExecBinaryNotFound { path: String },
    /// `spawn(2)` returned EACCES — binary exists but is not executable.
    ExecPermissionDenied { path: String },
    /// `spawn(2)` returned ENOEXEC / ELIBBAD — binary is invalid for
    /// this kernel/architecture. `kind` ∈ {"not_executable","bad_elf",
    /// "wrong_arch"}.
    ExecBinaryInvalid { path: String, kind: String },
    /// Cgroup setup failed (mkdir, place_pid, write_limits). `source`
    /// carries the verbatim `std::io::Error` Display text.
    CgroupSetupFailed { kind: String, source: String },
    /// Uncategorised driver failure. Falls back on verbatim Display.
    /// Operators seeing this signal a missing specific variant — the
    /// driver should grow one.
    DriverInternalError { detail: String },
    /// Reconciler hit restart budget. `last_cause_summary` is the
    /// `human_readable()` rendering of the most recent cause variant
    /// the reconciler observed (rendered at observe time — `Box<Self>`
    /// is rejected because rkyv `Archive` cannot resolve recursive
    /// types). Per-attempt history lives in reconciler private libSQL.
    RestartBudgetExhausted { attempts: u32, last_cause_summary: String },
    /// Operator submitted stop intent and the reconciler converged the
    /// allocation. `by` ∈ {Operator, Cluster}; Cluster is Phase 2+.
    Cancelled { by: CancelledBy },
    /// Scheduler returned `NoCapacity`. Carries typed requested / free
    /// envelopes — replaces the previous string-formatted diagnostic.
    NoCapacity { requested: ResourceEnvelope, free: ResourceEnvelope },

    // --- Cause-class failure variants (Phase 2 emit-deferred) --------
    /// Cgroup OOM-killed. Phase 2 emit (requires cgroup-events
    /// subscription); defined now for wire-shape forward-compat.
    OutOfMemory { peak_bytes: u64, limit_bytes: u64 },
    /// Workload exited within post-spawn settle window. Phase 2 emit
    /// (requires post-spawn `wait()` + classification); defined now.
    /// Mirrors the `exit_code: Option<i32>` field on
    /// `AllocStatusRowBody` (also Phase-2-populated, see ADR-0033 §2).
    WorkloadCrashedImmediately {
        exit_code:   Option<i32>,
        signal:      Option<u8>,
        stderr_tail: Option<String>,
    },
}

/// Initiator of a `Stopped` transition. Distinct enum (not a String
/// field) so the renderer dispatches on a closed set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
         ToSchema, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StoppedBy { Operator, Reconciler }

/// Initiator of a `Cancelled` transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
         ToSchema, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CancelledBy { Operator, Cluster }

/// Resource envelope carried by the `NoCapacity` cause variant. Mirrors
/// `traits::driver::Resources` but defined here so `TransitionReason`
/// is self-contained at the wire-typed boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize,
         ToSchema, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct ResourceEnvelope { pub cpu_milli: u32, pub memory_bytes: u64 }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalReason {
    /// Streaming handler observed `restart_count == max` and latest
    /// row state is Failed. The inner `cause` carries the cause-class
    /// `TransitionReason` of the final failed attempt — duplicating
    /// the most recent `LifecycleTransition.reason` so a CLI rendering
    /// only the terminal line still has structured cause data.
    BackoffExhausted { attempts: u32, cause: TransitionReason },
    /// Streaming handler observed an unrecoverable driver error on a
    /// path the reconciler will not retry. The inner `cause` is the
    /// originating cause-class variant.
    DriverError { cause: TransitionReason },
    /// Streaming handler's wall-clock cap fired before any terminal
    /// event arrived. Carries the configured cap so CLI render can
    /// say "did not converge in 60s" without re-derivation.
    Timeout { after_seconds: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransitionSource {
    Reconciler,
    Driver(DriverType),                        // existing enum from overdrive-core::traits::driver
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AllocStateWire {
    Pending,
    Running,
    Draining,
    Suspended,
    Terminated,
    Failed,                                    // NEW per §5 below
}
```

`#[non_exhaustive]` on every enum so future additive variants are
non-breaking for downstream consumers (the CLI is a workspace member
and will track changes mechanically; future external SDK consumers
won't break on a new variant).

### 4. Single source of truth for `reason` ([C6])

`AllocStatusRow` (in `overdrive-core::traits::observation_store`)
gains two fields:

```rust
pub struct AllocStatusRow {
    pub alloc_id:    AllocationId,
    pub job_id:      JobId,
    pub node_id:     NodeId,
    pub state:       AllocState,
    pub updated_at:  LogicalTimestamp,
    pub reason:      Option<TransitionReason>,    // NEW
    pub detail:      Option<String>,              // NEW — verbatim driver text or NoCapacity diagnostic
}
```

The action shim (per ADR-0023) is the single writer of this row. It
constructs both fields at the point of writing.

For driver-domain reasons the shim **classifies the
`DriverError::StartRejected.reason` text into a cause-class variant**
at write time — the verbatim text becomes `detail` (preserved for
audit), AND the typed cause becomes the enum variant. The
classification is a small string-prefix matcher run inside the
shim against the verbatim driver text:

| Prefix substring (verbatim driver text) | `TransitionReason` variant |
|---|---|
| `"spawn ..."` containing `"No such file or directory"` (ENOENT) | `ExecBinaryNotFound { path }` |
| `"spawn ..."` containing `"Permission denied"` (EACCES) | `ExecPermissionDenied { path }` |
| `"spawn ..."` containing `"Exec format error"` (ENOEXEC) | `ExecBinaryInvalid { path, kind: "not_executable" }` |
| `"create workload scope: ..."` | `CgroupSetupFailed { kind: "create_scope", source }` |
| `"place pid in scope: ..."` | `CgroupSetupFailed { kind: "place_pid", source }` |
| Any other `StartRejected.reason` | `DriverInternalError { detail }` |

The shim parses the path out of the `"spawn <path>: ..."` prefix the
ExecDriver constructs (cf. `crates/overdrive-worker/src/exec_driver.rs`
`start_rejected(format!("spawn {}: {err}", spec.command))`). On the
happy path the variant is `Started` (no payload) and `detail` is
`None`. For reconciler-domain reasons the reconciler emits the cause-
class variant directly on the `Action::*` payload and the shim
threads it through verbatim — `NoCapacity { requested, free }` and
`RestartBudgetExhausted { attempts, last_cause_summary }` originate
in the reconciler.

**Future-proofing**: when Phase 2 ExecDriver gains structured error
classes (rather than `String` reasons), the shim's classification
table collapses to a `From<DriverError> for TransitionReason` impl
and the prefix-matching logic deletes. The intermediate string-prefix
shape is a Phase 1 cost the cause-class refactor pays once.

The streaming endpoint reads `reason` + `detail` off the
`LifecycleEvent` broadcast payload (which is constructed from the row
the shim just wrote). The snapshot endpoint reads `reason` + `detail`
off the row directly. **Both serialise the same `TransitionReason`
enum value identically**; byte-equality is structurally guaranteed.

The `Action` enum gains `reason: TransitionReason` on
`StartAllocation`, `RestartAllocation`, `StopAllocation` so the
reconciler can declare its rationale at action emit time. Phase 1
defaults: `Scheduling` for first start, `Scheduling` for restart
(driver outcome refines to `Started` on success or to a cause-class
variant on `DriverError::StartRejected` per the classification table
above), `Stopped { by: Reconciler }` or `Stopped { by: Operator }`
on stop depending on whether the stop intent was operator-driven.
Future reconcilers (right-sizing, cert-rotation) extend the variant
set additively under `#[non_exhaustive]`.

### 5. `AllocState::Failed` variant addition

`AllocState` (in `overdrive-core::traits::observation_store`) gains:

```rust
pub enum AllocState {
    Pending,
    Running,
    Draining,
    Suspended,
    Terminated,
    Failed,                                       // NEW
}
```

Display string: `"failed"`. The action shim, when handling
`DriverError::StartRejected`, writes `state: Failed` (instead of
`Terminated`). This is the smallest cut that distinguishes "operator
stopped" from "driver could not start" on the wire.

Per ADR-0021 the `JobLifecycleState` projection (which
holds `BTreeMap<AllocationId, AllocStatusRow>`) carries the new variant
mechanically.

### 6. Wall-clock cap — handler-local `select!` with injected `Clock` (Call C → C1)

```rust
// in overdrive-control-plane::handlers (or a new streaming.rs sibling)

const DEFAULT_STREAMING_SUBMIT_CAP: Duration = Duration::from_secs(60);

async fn streaming_submit_loop(state: AppState, job_id: JobId)
    -> impl Stream<Item = Result<Bytes, Error>>
{
    let mut bus = state.lifecycle_events.subscribe();
    let cap = state.streaming_cap.unwrap_or(DEFAULT_STREAMING_SUBMIT_CAP);

    async_stream::try_stream! {
        // first line: Accepted (synchronously known)
        yield serialize(&SubmitEvent::Accepted { ... })?;

        loop {
            tokio::select! {
                event = bus.recv() => {
                    match event {
                        Ok(ev) if ev.job_id == job_id => {
                            yield serialize(&SubmitEvent::LifecycleTransition { ... })?;
                            if let Some(terminal) = check_terminal(&state, &ev).await {
                                yield serialize(&terminal)?;
                                break;
                            }
                        }
                        Ok(_) => continue, // event for another job; drop
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            // fallback: re-snapshot from observation store
                            // and synthesize transition events for any
                            // change since last seen state. Then resubscribe.
                            ...
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = state.clock.sleep(cap) => {
                    yield serialize(&SubmitEvent::ConvergedFailed {
                        alloc_id: last_seen_alloc_id,
                        terminal_reason: TerminalReason::Timeout,
                        reason: None,
                        error: Some(format!("did not converge in {}s", cap.as_secs())),
                    })?;
                    break;
                }
            }
        }
    }
}
```

**Default cap**: 60 s. Configurable via
`ServerConfig::streaming_submit_cap_seconds`. Justification: 60 s
aligns with the operator emotional arc captured in US-02 Example 3.
ExecDriver in Phase 1 is `tokio::process` on localhost — cold-starts
are sub-second; the cap is not driven by driver launch latency. 60 s
preserves headroom for reconciler restart attempts (≤5 attempts at
5 s backoff = 25 s) plus operator-patience headroom before
"is this hung?" sets in. Operators whose workloads need longer
convergence use `--detach` or configure `[server].streaming_submit_cap`.
Rejected: 30 s (too tight for heavy-init workloads); 120 s (operator
emotional-arc threshold for "is this hung?" exceeded). Full rationale
in `wave-decisions.md` [D3].

**Why injected `Clock`**: ADR-0013 §2c established the `Arc<dyn Clock>`
seam. Using `tokio::time::sleep` would create a new DST gap (the
streaming cap could not be advanced under simulation). Using
`clock.sleep` reuses the same seam.

**Why handler-local and not middleware**: §Considered alternatives B
below.

### 7. Subscription mechanism — push via broadcast channel (Call D → D1)

`AppState` gains:

```rust
pub lifecycle_events: Arc<tokio::sync::broadcast::Sender<LifecycleEvent>>,
```

The action shim (existing module, EXTENDED per Slice 02) calls
`bus.send(LifecycleEvent { ... })` after every successful
`obs.write(ObservationRow::AllocStatus(row))`. The `LifecycleEvent`
is constructed from the row the shim just wrote plus the `from` state
the shim already knows (`find_prior_alloc_row` returns it for
Restart; for first-time Start, `from` is the previous absent state
projected as a synthetic `Pending` predecessor).

```rust
pub struct LifecycleEvent {
    pub alloc_id: AllocationId,
    pub job_id:   JobId,
    pub from:     AllocState,
    pub to:       AllocState,
    pub reason:   TransitionReason,
    pub detail:   Option<String>,
    pub source:   TransitionSource,
    pub at:       LogicalTimestamp,
}
```

`LifecycleEvent` lives in `overdrive-core` (next to the trait
definitions); it does NOT derive `Serialize` / `Deserialize` /
`ToSchema` — it is internal, never on the wire. The streaming handler
projects it into `SubmitEvent::LifecycleTransition` for serialisation.

**Lagging-subscriber discipline**: Phase 1 has at most one streaming
subscriber per process (one operator running streaming submit at a
time on a single-node deployment). The broadcast channel is sized
generously (default capacity 256). If a `Lagged(n)` arrives, the
handler falls back to a one-shot `obs.alloc_status_rows()` snapshot,
synthesises any transitions since last seen state, and resubscribes.
This is defensive — Phase 1 single-subscriber is unlikely to lag —
but written into the contract because the broadcast channel becomes
multi-subscriber in Phase 2+ when a future TUI mode lands.

Lagged-recovery synthesis depends on the prior-state hydrator defined
in [ADR-0033 §2 (Field source map — `rows[].last_transition`)](./adr-0033-alloc-status-snapshot-enrichment.md#2-field-source-map-server-side-hydration),
which extends `JobLifecycleView` to cache per-alloc prior state. The
Slice 01 implementation lands ADR-0033 first; Slice 02 (this ADR)
depends on the hydrator already being present.

### 8. HTTP error semantics in the streaming context

| When | Body | Status |
|---|---|---|
| Validation error (bad TOML, bad spec) — caught by `Job::from_spec` BEFORE any line emitted | `ErrorBody` per ADR-0015 | 400 Bad Request |
| Conflict (different spec at occupied key) — caught by `put_if_absent` BEFORE any line emitted | `ErrorBody` | 409 Conflict |
| IntentStore I/O failure BEFORE any line emitted | `ErrorBody` | 500 Internal Server Error |
| Streaming-side internal failure AFTER `Accepted` line (broadcast channel closed unexpectedly, serialiser panic) | `SubmitEvent::ConvergedFailed { terminal_reason: DriverError, ... }` followed by stream close | n/a (200 already on wire) |
| Server wall-clock cap fires | `SubmitEvent::ConvergedFailed { terminal_reason: Timeout, ... }` | n/a (200 already on wire) |
| Convergence to Failed (BackoffExhausted) | `SubmitEvent::ConvergedFailed { terminal_reason: BackoffExhausted, ... }` | n/a (200 already on wire) |

The transition between "HTTP error mode" and "structured terminal
event mode" is **the moment the first byte is written to the response
body**. Up to that moment, the JSON-ack `ErrorBody` path applies. After
that moment, every error becomes a `ConvergedFailed` event. This is the
same shape `nomad job run` follows for streaming RPCs.

ADR-0015's `ControlPlaneError → (StatusCode, ErrorBody)` exhaustive
mapping is **unchanged**. The streaming lane's pre-`Accepted` errors
flow through the same `to_response` function. The mid-stream failure
mode emits a structured event instead.

### 9. CLI exit-code mapping ([C3])

The CLI maps:

| Streaming terminal | Exit code |
|---|---|
| `SubmitEvent::ConvergedRunning` | 0 |
| `SubmitEvent::ConvergedFailed` (any `terminal_reason`) | 1 |
| pre-`Accepted` HTTP 4xx with `ErrorBody` | 2 |
| pre-`Accepted` HTTP 5xx with `ErrorBody` | 2 |
| transport error (no HTTP response) | 2 |

The CLI does NOT branch on `terminal_reason` for exit code; it
branches for *rendering* (the `Error:` block names the
`terminal_reason` and the optional `reason` + `error`).

### 10. CLI TTY auto-detection ([D5])

CLI-side, in the `submit` command:

```rust
let stream = !args.detach && std::io::IsTerminal::is_terminal(&std::io::stdout());
let accept = if stream { "application/x-ndjson" } else { "application/json" };
```

Server stays Accept-driven. No User-Agent inspection, no query param,
no environment-variable heuristic. Operator override via `--detach` is
unconditional.

### 11. OpenAPI declaration ([D8])

The `submit_job` `#[utoipa::path(...)]` annotation gains a second
content-type entry on the `200` response:

```rust
#[utoipa::path(
    post,
    path = "/v1/jobs",
    request_body = api::SubmitJobRequest,
    responses(
        (status = 200, description = "Job accepted",
         content(
             ("application/json" = api::SubmitJobResponse),
             ("application/x-ndjson" = api::SubmitEvent),
         )),
        (status = 400, description = "Validation error", body = api::ErrorBody),
        (status = 409, description = "Conflict at existing key", body = api::ErrorBody),
        (status = 500, description = "Internal error", body = api::ErrorBody),
    ),
    tag = "jobs",
)]
```

`utoipa` 5.x supports the `content(...)` shape per the
[OpenAPI 3.1 multiple-content-types-per-response spec](https://spec.openapis.org/oas/v3.1.0#response-object).
The `api::SubmitEvent` schema describes one event; the streaming
nature (line-delimited concatenation of these events) is declared via
a vendor extension `x-ndjson-stream: true` on the response object.
External tooling that ignores the vendor extension treats the response
as "a single SubmitEvent JSON object," which is a safe over-
approximation (NDJSON IS line-delimited single objects).

`cargo xtask openapi-check` (ADR-0009) catches any drift in the
checked-in `api/openapi.yaml` mechanically. No new gate.

## Considered alternatives

### Alternative A — SSE instead of NDJSON

**Rejected upstream** in DISCUSS [D1]; recorded here for completeness.
Rationale (DISCUSS-side): single CLI consumer (no browser); one-shot
not long-lived feed; `serde_json::Deserializer::from_reader` driven
line-by-line is mature; `application/x-ndjson` is the natural
OpenAPI media type. SSE remains revisitable in Phase 2+ if a polyglot
or browser consumer appears.

### Alternative B — Wall-clock cap as an axum tower layer

**Rejected** for Phase 1. A `StreamingTimeoutLayer` that wraps every
streaming endpoint and races a timer against the inner future is
correct in spirit and would let future streaming endpoints inherit
the cap mechanically. But Phase 1 has **one streaming endpoint**;
factoring out a layer that handles one caller is YAGNI. When the
second streaming endpoint lands (likely Phase 2+ with `alloc status
--follow` if that ever gets reactivated, currently [C4]-out, or a
node-agent control-flow stream), the refactor is mechanical:
`tokio::select!` arms become `Layer::call` shapes. Punt.

### Alternative C — Subscription via ObservationStore polling

**Rejected.** Polling at a 50–100 ms cadence would meet the 200 ms
first-event KPI (KPI-01) on average but is fragile against
ObservationStore latency spikes and adds a per-subscriber poll cost.
The action shim is the natural broadcast site (it is the only writer
of `AllocStatusRow`); adding a `bus.send(...)` is a one-line addition
at an already-side-effecting layer. Push-via-broadcast also avoids
the from-state derivation cost: the shim already knows the prior
state via `find_prior_alloc_row`.

### Alternative D — State-class `TransitionReason` (originally A1, retired 2026-04-30)

**Rejected — was the initial 2026-04-30 decision; superseded by the
Amendment 2026-04-30 below.** The original variant set was unit-only
and named the *lifecycle phase* rather than the cause:
`Scheduling`, `Starting`, `Started`, `DriverStartFailed`,
`BackoffPending`, `BackoffExhausted`, `Stopped`, `NoCapacity`. Cause-
specific data (binary path, errno class, cgroup setup stage,
requested-vs-free capacity, OOM peak vs limit) was relegated to a
free-form `detail: Option<String>` field on the row.

Two structural problems forced the retirement:

1. **The cause taxonomy belongs in the type system, not in opaque
   strings.** Every renderer that distinguished "binary not found"
   from "permission denied" had to re-parse the `detail` string.
   Type erasure at the wire boundary defeats the [C6] single-source-
   of-truth pin: the typed enum agreed across surfaces, but the
   actual *cause* drifted because both surfaces re-stringified
   independently.
2. **`DriverStartFailed` collapsed five distinguishable failure
   modes.** ENOENT, EACCES, ENOEXEC, cgroup setup failure, and
   uncategorised driver error all serialised to `kind:
   "driver_start_failed"`; operators got the same label for every
   class. The cause-class refactor lifts the distinction into the
   variant.

Splitting the top-level *event* enum into per-cause variants —
`SubmitEvent::ExecFailed { detail }`, `SubmitEvent::OOMKilled
{ ... }` — was *also* rejected (originally as A2 and remains
rejected): the CLI's exit-code dispatch is `Running → 0` /
`Failed → 1` / pre-`Accepted` error → 2, never branched on the
cause. The cause is a *rendering* concern, not an event-type one.
The current shape keeps `SubmitEvent` flat (4 top-level variants)
and pushes the cause taxonomy down into `TransitionReason`'s
payload — which is the right layer.

### Alternative E — Two-level event with `Outcome { kind, detail }` (Call A3)

**Rejected.** Reduces to A1 once the inner `kind` is the structured
`TransitionReason` enum. The remaining differences are cosmetic
(extra wrapping struct on the wire). A1 is the simpler shape.

### Alternative F — Reuse `DriverError` as the wire `reason` enum

**Rejected.** `DriverError` is a `thiserror::Error` type intended for
log-line `Display` formatting and Rust error chaining via `#[from]`.
It has no canonical wire shape, no `Serialize`/`Deserialize`/`ToSchema`
derives, no rkyv archive shape. Promoting it to the wire would
conflate "this is an error type for `?` propagation" with "this is a
state-transition reason on the wire" and cross the development.md
"errors are typed at the boundary" rule. The mapping
`DriverError::StartRejected.reason: String` →
`AllocStatusRow.detail: Option<String>` plus `reason:
TransitionReason::DriverStartFailed` is mechanical.

### Alternative G — A new endpoint `POST /v1/jobs/stream`

**Rejected.** Splitting the path bypasses the back-compat surface
[C2] protects (existing `Accept: application/json` clients see the
existing JSON ack). It also forces the CLI to know about two paths,
adds an OpenAPI surface row, and breaks the
"polymorphism on Accept is the REST shape" pattern ADR-0008
implicitly commits to. The single-path content-negotiation shape is
the standard and the contract.

## Consequences

### Positive

- One verb does the inner-loop job. The wait IS the submit.
- Back-compat surface is structural (`Accept` header), not a feature
  flag — existing JSON-ack consumers see no change.
- Single source of truth for `transition_reason` ([C6]) is enforced
  by the row schema, not by convention; integration test asserts
  byte-equality across surfaces.
- Wall-clock cap is DST-controllable via the existing `Clock`
  injection; no new DST seam.
- `AllocState::Failed` distinguishes "operator stopped" from "driver
  could not start," which the journey TUI mockup needs.
- `TransitionReason` enum is `#[non_exhaustive]` and additive going
  forward; new reconcilers (Phase 2+ right-sizing, cert-rotation,
  chaos) extend it mechanically.

### Negative

- The `AllocStatusRow` rkyv archive shape changes (adds two `Option`
  fields). Existing redb files are forward-compatible (rkyv
  `Option<T>` archives `None` as zero-sized), but a Phase 0 fixture
  asserts the migration is non-destructive.
- The streaming handler is the largest async surface in
  `overdrive-control-plane` to date — `tokio::select!` over a
  broadcast subscription and a clock timer plus the `Lagged`
  fallback path. Tested at Tier 1 (DST: cap-fires invariant) and
  Tier 3 (real-kernel: broken-binary regression).
- The action shim now calls `bus.send(...)` after `obs.write(...)`.
  A broadcast-send error is logged and discarded (the row was
  written; the snapshot will see it; the streaming subscriber
  reconnects on the next event). Per-action error isolation
  preserved.

### Quality-attribute impact

- **Performance — first-event latency**: positive. Push channel
  delivers events sub-tick; KPI-01 (200 ms p95) achievable.
- **Reliability — convergence honesty**: positive. KPI-02 (broken
  binary surfaces inline) becomes a structural property of the
  shim's `obs.write + bus.send` pair.
- **Reliability — surface coherence**: positive. KPI-04 (streaming
  reason == snapshot reason) is byte-equality on the same enum
  value pulled from the same row.
- **Maintainability — testability under DST**: preserved. Wall-clock
  cap rides the existing `Clock` injection; broadcast channel is
  single-process and trivially DST-replayable.
- **Compatibility — back-compat**: preserved. JSON-ack lane
  unchanged.
- **Security — non-repudiation**: preserved. Structured terminal
  events are auditable; HTTP-level errors flow through the same
  `ErrorBody` shape ADR-0015 already audits.

### Migration

This is a single-cut change per [C9]. No deprecation period, no
feature flag. The existing JSON-ack lane is the back-compat surface;
existing CLI clients receive no change unless they opt into NDJSON.
The CLI's `submit` command updates atomically with the server change.
Under-development branches that touch the action shim or the
`AllocStatusRow` shape rebase against the new fields.

### Enforcement

- `cargo xtask openapi-check` catches any drift in
  `api/openapi.yaml`.
- A `trybuild` compile-fail fixture asserts `LifecycleEvent` cannot
  derive `Serialize` (it is internal only — leaking it onto the wire
  would bypass the `SubmitEvent` projection).
- A unit test enumerates every `SubmitEvent` variant and asserts the
  serialised JSON has a `kind` discriminator.
- An integration test (Tier 3) submits a broken-binary spec, captures
  both the streaming `ConvergedFailed.error` and the snapshot
  `last_transition.detail`, asserts byte-equality.
- `dst-lint` enforces that the streaming handler's cap timer uses
  `clock.sleep(...)` not `tokio::time::sleep`.

## Slice 02 back-prop

This ADR provides Slice 02 with:

- The exact wire shape of `SubmitEvent` (4 variants, structured
  reason).
- The wall-clock cap value (60 s, configurable) and placement
  (handler-local `select!` with injected `Clock`).
- The subscription mechanism (broadcast channel from action shim).
- The OpenAPI declaration shape.
- The HTTP error semantics (pre-`Accepted` flows through ADR-0015;
  post-`Accepted` becomes `ConvergedFailed`).

Slice 02 AC #2 (200 ms first-event budget) is structurally supported
by the push-via-broadcast subscription. Slice 02 AC #5 (server
wall-clock cap) is structurally supported by the `clock.sleep` race.
Slice 02 AC #8 (byte-for-byte reason equality) is structurally
supported by the row-schema single-source-of-truth.

## References

- DISCUSS-wave decisions [D1] / [D5] / [D7] / [D8] / [C2] / [C6] /
  [C9] in
  `docs/feature/cli-submit-vs-deploy-and-alloc-status/discuss/wave-decisions.md`.
- DESIGN-wave decisions D1 / D3 / D4 / D5 / D6 / D8 in
  `docs/feature/cli-submit-vs-deploy-and-alloc-status/design/wave-decisions.md`.
- ADR-0008 — REST + OpenAPI transport.
- ADR-0009 — OpenAPI schema derivation; `cargo xtask openapi-check`
  CI gate.
- ADR-0013 — Reconciler primitive runtime; `Arc<dyn Clock>` injection
  seam.
- ADR-0014 — CLI HTTP client + shared types.
- ADR-0015 — HTTP error mapping; `ControlPlaneError` /
  `ErrorBody` exhaustive mapping (used unchanged on the JSON lane and
  the pre-`Accepted` NDJSON lane).
- ADR-0021 — Reconciler State shape (`AnyState::JobLifecycle` carries
  the extended `AllocStatusRow`).
- ADR-0023 — Action shim placement.
- ADR-0027 — Job-stop HTTP shape (verb-suffix shape; precedent for
  AIP-136-style endpoint conventions).
- ADR-0029 — `overdrive-worker` crate extraction.
- ADR-0030 — ExecDriver and AllocationSpec args.
- ADR-0033 — `alloc status` snapshot enrichment (companion ADR).
- Feature artifacts:
  `docs/feature/cli-submit-vs-deploy-and-alloc-status/discuss/journey-submit-streams-default.yaml`,
  `slices/slice-02-ndjson-streaming-submit.md`,
  `slices/slice-03-detach-flag-and-pipe-detect.md`.
- RFC 8259 (JSON), no formal RFC for NDJSON but de facto convention
  documented at `https://github.com/ndjson/ndjson-spec`.
- OpenAPI 3.1 spec on multiple `content` types per response.

## Amendment 2026-04-30 — `TransitionReason` is cause-class, not state-class

**Trigger**: post-acceptance review the user surfaced the cost of
`detail: Option<String>` carrying cause-specific data the type system
should own. The original `[D1]` variant set named the lifecycle phase
(`Scheduling` / `Starting` / `Started` / `DriverStartFailed` / ...);
cause-specific information (binary path, errno class, cgroup stage,
requested-vs-free capacity, OOM peak/limit) was free-form text. The
amendment moves that data into typed payloads on cause-class variants.

**What changed**:

- `TransitionReason`'s variant set is now mixed: progress markers
  (unit) for the healthy-path phases, and cause-class (typed payloads)
  for failure transitions. See §3 above for the full enumeration.
- The enum drops `Copy` and `Hash` derives — cause variants carry
  `String` payloads. Consumers that previously pattern-matched by
  value clone or take by reference. Only call sites today are the
  scaffold itself and the action shim's writer; cost is contained.
- `TerminalReason` extends with structured payloads
  (`BackoffExhausted { attempts, cause }`, `DriverError { cause }`,
  `Timeout { after_seconds }`) so the streaming terminal line
  carries the cause without depending on the immediately-preceding
  `LifecycleTransition` line.
- `RestartBudgetExhausted` carries `last_cause_summary: String`
  (rendered via `human_readable()` at observe time), NOT
  `Box<TransitionReason>` — rkyv `Archive` cannot resolve a
  recursive enum. Per-attempt structured cause history lives in
  reconciler private libSQL.
- The action shim grows a small string-prefix matcher that
  classifies `DriverError::StartRejected.reason` text into the
  right cause-class variant at write time. The verbatim text is
  preserved in `AllocStatusRow.detail` for audit. Phase 2
  ExecDriver structured-error refactor collapses the matcher to a
  `From<DriverError>` impl.
- Phase 2 emit-deferred variants (`OutOfMemory`,
  `WorkloadCrashedImmediately`) ship in the enum now for forward
  wire-compatibility, mirroring the `exit_code: Option<i32>`
  pattern on `AllocStatusRowBody` (also Phase-2-populated).

**What did not change**:

- The `LifecycleTransition.reason: TransitionReason` field shape
  (still one field, still the same type — only the variant set
  grows).
- The single-source-of-truth pin ([C6]) — both surfaces still
  serialise the same enum value via the same `Serialize` derive.
- The streaming wire shape (`SubmitEvent` is still the 4-variant
  flat enum).
- The wall-clock cap mechanism (§6), the subscription mechanism
  (§7), the OpenAPI declaration (§11). The CLI exit-code dispatch
  (§9) still does NOT branch on cause — `terminal_reason` controls
  rendering, exit code is `Running → 0` / `Failed → 1`.
- DISCUSS [D1]–[D8] all stay locked. The amendment is to
  `TransitionReason`'s shape, not to any DISCUSS commitment.

**Compile-cleanliness during the GREEN transition**: the scaffold at
`crates/overdrive-core/src/transition_reason.rs` compiles with
`panic!("Not yet implemented -- RED scaffold")` on
`human_readable()` and `is_failure()`. The cause-class enum
declaration itself is fully typed — downstream consumers
(`AllocStatusRow.reason: Option<TransitionReason>`,
`SubmitEvent::LifecycleTransition.reason: TransitionReason`,
`TerminalReason::{BackoffExhausted, DriverError}.cause:
TransitionReason`) reference it by name and compile against the new
shape immediately.

## Changelog

| Date | Change |
|---|---|
| 2026-04-30 | Initial ADR. Decisions D1 / D3 / D4 / D5 / D6 / D8 from the DESIGN wave; constraints carried from DISCUSS wave-decisions. Slice 02 back-prop completed. Echo peer review pending. |
| 2026-04-30 | **Amendment** — `TransitionReason` refactored from state-class to cause-class. `TerminalReason` extended with structured payloads. Original variant set retired and captured under Alternative D as the rejected predecessor. See `Amendment 2026-04-30` section above. Slice 02 back-prop list (in `docs/feature/cli-submit-vs-deploy-and-alloc-status/design/upstream-changes.md`) catalogues the consequent updates needed in DISCUSS / DISTILL / roadmap. |
