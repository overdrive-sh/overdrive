# DESIGN Decisions — `cli-submit-vs-deploy-and-alloc-status`

**Wave**: DESIGN (solution-architect / Morgan)
**Date**: 2026-04-30
**Status**: COMPLETE — handoff-ready for DEVOPS / DISTILL pending Echo
peer review.

---

## Locked carryovers (DO NOT re-open)

These were ratified upstream and are constraints, not decisions:

- **C1** Option S — submit streams convergence by default. (DIVERGE, ratified DISCUSS [D2].)
- **C2** NDJSON over SSE; `Accept: application/x-ndjson` gates the stream;
  no Accept header → existing JSON ack (back-compat). (DISCUSS [D1].)
- **C3** CLI exit codes are 0 / 1 / 2; sysexits.h reserved. (DISCUSS [D3].)
- **C4** `alloc status --follow` is OUT of scope. (DISCUSS [D4].)
- **C5** A server-side wall-clock cap MUST exist; value is a DESIGN
  call. (DISCUSS [D5].)
- **C6** Single source of truth for `transition_reason` across the
  streaming `LifecycleTransition` and the snapshot's
  `last_transition.reason`. (DISCUSS [D7].)
- **C7** Walking skeleton waived (brownfield). (DISCUSS [D8].)
- **C8** Phase 1 is single-node — no scheduler placement, no
  multi-region, no node registration. (Phase 1 inherited.)
- **C9** Greenfield migration discipline — no deprecation periods, no
  feature-flagged old paths. (`feedback_single_cut_greenfield_migrations.md`.)
- **C10** All new types live in `overdrive-control-plane::api` per
  ADR-0014. CLI imports them directly; OpenAPI is a byproduct.

---

## New decisions (this wave)

### [D1] `convergence_event` granularity — flat enum, structured `reason` (Call A → A1)

**Decision**: `SubmitEvent` is a flat top-level enum with four variants
(`Accepted`, `LifecycleTransition`, `ConvergedRunning`,
`ConvergedFailed`). `LifecycleTransition` and `ConvergedFailed` carry a
**structured `TransitionReason` enum** (NEW in `overdrive-core`) plus an
opaque `detail: String` slot for verbatim driver text. `ConvergedFailed`
additionally carries a `terminal_reason: TerminalReason` enum (also
NEW).

```rust
pub enum SubmitEvent {
    Accepted        { spec_digest: String, intent_key: String, outcome: IdempotencyOutcome },
    LifecycleTransition {
        alloc_id: String, from: AllocStateWire, to: AllocStateWire,
        reason: TransitionReason, detail: Option<String>,
        source: TransitionSource, at: String,
    },
    ConvergedRunning  { alloc_id: String, started_at: String },
    ConvergedFailed   {
        alloc_id: Option<String>, terminal_reason: TerminalReason,
        reason: Option<TransitionReason>, error: Option<String>,
    },
}
```

**Why A1 not A2 / A3**:

- **A2 (per-cause discriminated union)** would balloon the top-level
  variant count to ~10 and force the CLI's terminal-event match arm to
  enumerate every driver-domain cause. The exit-code mapping is
  `Running → 0`, `Failed (any cause) → 1` — the cause is *not* part of
  the dispatch shape; making it the dispatch shape would invert the
  contract.
- **A3 (two-level `Outcome { kind, detail }`)** trades wire bytes for
  CLI ergonomics but obscures the structured nature of `kind` (it would
  need to be the same enum either way). A3 collapses to A1 once the
  inner `kind` is the structured `TransitionReason` enum below; the
  remaining difference is purely cosmetic.
- **A1 wins** because the enum-level dispatch (Running vs Failed vs
  Lifecycle) is what the CLI actually branches on for exit-code
  mapping; the `reason` enum is what the CLI renders inside an `Error:`
  block. The two concerns are independent and the type system reflects
  that.

**Why NEW `TransitionReason` and not extend `DriverError`**: the
streaming surface and the snapshot surface BOTH need to render the
reason. `DriverError` is a thiserror error type — its `Display`
formatting is intended for log lines, and it has no canonical wire
shape. `TransitionReason` is a wire-typed enum with `Serialize` /
`Deserialize` / `ToSchema` derives, mirroring `IdempotencyOutcome` /
`StopOutcome`. `DriverError::StartRejected.reason: String` is the
*input* to `TransitionReason::DriverStartFailed { detail }`; the
mapping is mechanical.

`TransitionReason` variants for Phase 1 (additive going forward):

| Variant | When emitted | Source layer |
|---|---|---|
| `Scheduling` | placement decided, action emitted | reconciler |
| `Starting` | driver invocation underway | reconciler |
| `Started` | driver returned `Ok(handle)` | driver(exec) |
| `DriverStartFailed` | driver returned `StartRejected` | driver(exec) |
| `BackoffPending` | reconciler holding off restart | reconciler |
| `BackoffExhausted` | restart budget hit | reconciler |
| `Stopped` | reconciler observed terminal stop | reconciler |
| `NoCapacity` | scheduler returned `NoCapacity` | reconciler |

`TerminalReason` for `ConvergedFailed`:

| Variant | When |
|---|---|
| `DriverError` | unrecoverable driver error after one attempt |
| `BackoffExhausted` | restart budget (5 attempts) hit |
| `Timeout` | server wall-clock cap hit |

The CLI maps `ConvergedRunning → 0` and `ConvergedFailed → 1`
regardless of the inner `terminal_reason`; the terminal reason controls
*rendering*, not exit code. (Aligns with [C3].)

### [D2] Snapshot enrichment — extend `AllocStatusResponse` in place (Call B → B1)

**Decision**: `AllocStatusResponse.rows: Vec<AllocStatusRowBody>` is
extended in place with the new fields. `AllocStatusRowBody` becomes:

```rust
pub struct AllocStatusRowBody {
    pub alloc_id: String,
    pub job_id: String,
    pub node_id: String,
    pub state: String,                  // existing
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,         // existing — Pending/no-capacity surface

    // --- NEW per [D2] ---
    pub resources: ResourcesBody,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_transition: Option<TransitionRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

Plus top-level additions on the response envelope:

```rust
pub struct AllocStatusResponse {
    pub job_id: Option<String>,                   // NEW
    pub spec_digest: Option<String>,              // NEW
    pub replicas_desired: u32,                    // NEW
    pub replicas_running: u32,                    // NEW
    pub rows: Vec<AllocStatusRowBody>,            // existing, extended
    pub restart_budget: Option<RestartBudget>,    // NEW
}
```

**Why B1 not B2 / B3**:

- **B2 (replace)** is mechanically achievable under [C9]'s
  greenfield-migration rule, but `rows: Vec<AllocStatusRowBody>` is
  already sparse — every existing client pulls the same field; the new
  fields are pure additions. There is nothing to delete. B2's
  difference from B1 reduces to "remove `Default`/optional fields,"
  which buys nothing.
- **B3 (split into two endpoints)** would require a `GET
  /v1/allocations/{id}` companion. In Phase 1 single-node with
  `replicas=1` per the slice-01 OUT scope, the cardinality of
  `rows` is always 1; a per-allocation endpoint duplicates handler code
  with no operator-facing benefit. Defer to Phase 2+ if a
  multi-replica use case warrants it.
- **B1 wins**: every new field is `Option` or has a sensible default;
  the CLI re-render takes care of the new fields cleanly; the OpenAPI
  surface stays at 6 paths instead of growing to 7.

**Idempotency-shape inheritance**: top-level `job_id`, `spec_digest`,
and `replicas_*` fields mirror the existing `JobDescription`
shape — readers that already consume `JobDescription` re-cognise the
fields immediately.

### [D3] Streaming wall-clock cap — 60 s in handler-local `select!` (Call C → C1, value 60 s)

**Decision**: the streaming-submit handler races the event-emitting
future against a `tokio::time::sleep(WALL_CLOCK_CAP)` future inside a
`tokio::select!`. **`WALL_CLOCK_CAP = Duration::from_secs(60)`**,
configurable via `ServerConfig::streaming_submit_cap`.

The cap timer uses the **injected `Clock` trait per ADR-0013 §2c** —
`SystemClock` in production, `SimClock` under DST. Concretely the
streaming handler holds an `Arc<dyn Clock>` (already on `AppState`
implicitly via the runtime; promoted to a direct field on `AppState`
if it isn't already) and awaits `clock.sleep(WALL_CLOCK_CAP)` rather
than `tokio::time::sleep`. This is the single seam DST advances
simulated time through; the production code path is identical.

**Why C1 not C2 / C3**:

- **C2 (axum tower layer)** is correct in spirit (future streaming
  endpoints would inherit it), but `cli-submit-vs-deploy-and-alloc-status`
  is the only streaming endpoint in Phase 1. A layer that handles one
  caller is a YAGNI violation; promoting the timer to a layer when the
  second streaming endpoint appears is a straight refactor. Punt.
- **C3 (push the cap into the subscription primitive)** entangles the
  cap with the ObservationStore — an ObservationStore subscription that
  knows about wall clock would have no other reason to. The cap is a
  *streaming-handler* concern (the operator's terminal is the resource
  being protected), not a subscription concern.
- **C1 wins**: the handler is the natural owner of the cap; it has the
  request span; it owns the response stream; cancelling the cap on a
  clean terminal event is `select!`-natural.

**Value 60 s rationale**: aligns with the operator emotional arc
captured in US-02 Example 3 — the operator should not be left
staring. ExecDriver in Phase 1 is `tokio::process` on localhost;
cold-starts are sub-second, so the cap is *not* driven by driver
launch latency. 60 s preserves headroom for reconciler restart
attempts (≤5 attempts at 5-s backoff = 25 s) plus operator-patience
headroom before "is this hung?" sets in. 60 s is deliberately *not*
the median user expectation (≤2 s for happy path); it is the upper
bound after which the operator wants out. Operators whose workloads
need longer convergence (canary first-run with heavy initialisation,
or Phase 2+ container/VM drivers with non-trivial cold-start budgets)
override via `--detach` (US-03) or via the `[server].streaming_submit_cap`
config knob.

### [D4] Subscription mechanism — push via `tokio::sync::broadcast` from action shim (Call D → D1)

**Decision**: the action shim emits a `LifecycleEvent` on a
process-global `tokio::sync::broadcast::Sender<LifecycleEvent>` AFTER
each successful `obs.write(ObservationRow::AllocStatus(_))`. The
streaming-submit handler subscribes via `.resubscribe()` filtered by
`job_id == request.job_id` and forwards events as NDJSON lines.

```rust
// AppState gains:
pub lifecycle_events: Arc<tokio::sync::broadcast::Sender<LifecycleEvent>>,

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

**Why D1 not D2 / D3**:

- **D2 (handler polls the ObservationStore)** introduces a polling
  cadence (per-tick or N ms) that is in tension with the 200 ms
  first-event KPI (KPI-01). It also re-derives the `from →to` transition
  from successive snapshots, which means computing diffs against the
  last-seen state per subscriber — extra bookkeeping for no
  expressivity gain. The ObservationStore's existing `subscribe_all()`
  surface returns a flat row stream, not transition events; reproducing
  the from-state would require holding the prior row.
- **D3 (hybrid: push for `Accepted`, pull for the rest)** is the
  conservative compromise but inherits D2's polling problem for
  `LifecycleTransition`, and the `Accepted` event already arrives
  synchronously from the `submit_job` handler's IntentStore put result —
  no push channel needed for it.
- **D1 wins**: the action shim is the single async boundary that
  *witnesses* every state transition (it is the only writer of
  `AllocStatusRow`s — see action_shim.rs). It already has the
  `(from, to)` pair (the `to` state is what it just decided to write;
  the `from` state is what `find_prior_alloc_row` returns). Wiring
  a broadcast send at the existing `obs.write(...)` site is a one-line
  addition. Subscribers get push semantics with sub-tick latency.

**Reconciler purity preservation**: the broadcast send happens in the
action shim, NOT in `reconcile()`. ADR-0023 still holds — `reconcile`
remains pure; the shim is allowed all the I/O and side-effects, and
this is one more side-effect of an already-side-effecting layer. The
existing `ReconcilerIsPure` DST invariant is unaffected.

**Lagging-subscriber discipline**: `tokio::sync::broadcast` returns
`RecvError::Lagged(n)` when a slow consumer drops events. The
streaming handler treats `Lagged` as "fall back to a one-shot
ObservationStore snapshot to recover the current state, then resume
the broadcast subscription." For Phase 1 single-node with one
streaming subscriber, lag is unrealistic; the handling exists for
future multi-tenant cases.

### [D5] CLI TTY auto-detection — CLI-side `is_terminal()` + explicit `--detach` (Call E → E2)

**Decision**: the CLI-side `submit` command computes:

```rust
let stream = !args.detach && std::io::IsTerminal::is_terminal(&std::io::stdout());
let accept = if stream { "application/x-ndjson" } else { "application/json" };
```

`--detach` always wins (sends `application/json`). On a TTY without
`--detach`, stream. On a non-TTY without `--detach`, send
`application/json`.

**Why E2 not E1 / E3**:

- **E1** is functionally identical to E2 minus the `--detach` flag.
  But US-03 requires the explicit flag (CI scripts often want to
  declare intent rather than rely on TTY heuristics; some CI runners
  *do* allocate TTYs and the heuristic would mislead). E1 is a strict
  subset of E2.
- **E3 (server-side detection)** is wrong on multiple axes: there is
  no signal the server can use that the CLI cannot (the server only
  sees TCP/TLS/HTTP), and it inverts the principle of least
  surprise — the *client* knows whether its own stdout is a terminal.
- **E2 wins**: matches `docker run -d`, `nomad job run --detach`, and
  every Unix-tradition CLI tool. The reference class is uniform.

The detection uses `std::io::IsTerminal` (Rust 1.70+, in workspace).
No `atty` or `isatty`-via-libc dependency is added.

**Server stays Accept-driven** — this is the back-compat surface
[D1]+[C2] established. Server does not branch on User-Agent, query
param, or any other implicit signal. Header negotiation is the
contract.

### [D6] No new endpoint — `POST /v1/jobs` is polymorphic on Accept

**Decision**: the streaming surface lives at the **existing
`POST /v1/jobs` path**. The `Accept` header decides response shape.
No `POST /v1/jobs/stream`, no `POST /v1/jobs:submit`, no second
endpoint.

**Why**: the slice-02 brief locks this implicitly via [C2]'s
back-compat constraint; making it explicit prevents the well-known
"two endpoints diverge over time" failure mode. The
single-endpoint-with-content-negotiation pattern is the REST shape
ADR-0008 already commits to (see brief.md §14: HTTP/2 with ALPN, axum
+ rustls). The OpenAPI surface gains a second `responses` row on the
existing `submit_job` operation, not a new operation.

### [D7] `restart_budget.max` — Phase 1 hard-coded to 5 attempts

**Decision**: `RestartBudget.max = 5` (matching the existing
`RESTART_BUDGET_MAX` constant in `JobLifecycle::reconcile`). Made
configurable in Phase 2+ via job spec when right-sizing reconcilers
land. Phase 1 surfaces it on the wire as a `u32` field; the CLI
renders `Restart budget: N / 5 used`.

**Why**: matches the existing reconciler logic; the journey TUI mockup
already names "5" as the canonical max.

### [D8] OpenAPI representation of NDJSON

**Decision**: the streaming response is declared via OpenAPI 3.1 as a
second response media type on `submit_job`:

```yaml
/v1/jobs:
  post:
    responses:
      '200':
        content:
          application/json:
            schema: { $ref: '#/components/schemas/SubmitJobResponse' }
          application/x-ndjson:
            schema: { $ref: '#/components/schemas/SubmitEvent' }
        x-ndjson-stream: true   # vendor extension; one SubmitEvent per line
```

`utoipa` 5.x supports multiple `content` types per response via the
`#[utoipa::path(responses(...))]` macro. The vendor extension
`x-ndjson-stream` is informational; tooling that ignores it falls back
to "the response body is a single SubmitEvent JSON object," which is
a documented over-approximation of the wire shape (NDJSON IS
line-delimited single objects). No tooling consumes the vendor
extension in Phase 1; it is annotation for human readers.

The `cargo xtask openapi-check` gate (ADR-0009) covers the addition
unchanged — the new media-type entry is part of the schema derivation.

---

## Architecture-enforcement note

| Style chosen | Hexagonal + ports-and-adapters (per brief.md §1) — unchanged |
| Language | Rust |
| New enforcement targets | (a) trait-signature compile-fail test asserting `LifecycleEvent` does not leak observation-class types (`AllocStatusRow`) onto the streaming wire; (b) `dst-lint` already gates `Instant::now()` in core — the `WALL_CLOCK_CAP` timer must use `clock.sleep(...)` not `tokio::time::sleep` |
| Tooling | existing `xtask::dst_lint`; existing `xtask::openapi_check`; new trybuild fixture `streaming_event_does_not_leak_observation_types.rs` |

---

## Summary of new types (under `overdrive-control-plane::api`)

| Type | Purpose | Schema-derived |
|---|---|---|
| `SubmitEvent` (enum, 4 variants) | NDJSON line shape | yes |
| `TransitionReason` (enum, 8+ variants) | structured `reason` for both surfaces | yes |
| `TerminalReason` (enum, 3 variants) | streaming `ConvergedFailed.terminal_reason` | yes |
| `TransitionSource` (enum, 2 variants Phase 1) | `reconciler` \| `driver(exec)` | yes |
| `AllocStateWire` (enum, 5 variants) | wire-shaped projection of `AllocState` | yes |
| `TransitionRecord` (struct) | snapshot last-transition block | yes |
| `RestartBudget` (struct: used, max, exhausted) | snapshot restart-budget field | yes |
| `ResourcesBody` (struct: cpu_milli, memory_bytes) | snapshot per-row resources | yes |

`AllocStatusResponse` and `AllocStatusRowBody` are **extended in place**
per [D2]. `SubmitJobRequest`, `SubmitJobResponse`, `IdempotencyOutcome`,
`StopJobResponse`, `StopOutcome`, `JobDescription`, `ClusterStatus`,
`NodeList` are **unchanged**.

## New types under `overdrive-core`

| Type | Purpose |
|---|---|
| `TransitionReason` (re-export from above; lives in core for shared use) | the same enum used by the action shim and by the wire types |
| `LifecycleEvent` (struct) | broadcast channel payload (NOT on the wire) |

Note on placement: `TransitionReason` is the load-bearing type for [C6]
single-source-of-truth. It lives in `overdrive-core` (where the action
shim and reconciler can produce it) and is re-exported through
`overdrive-control-plane::api` (where it carries the `ToSchema` derive
for the wire). Both producers (`reconciler::JobLifecycle::reconcile`,
`action_shim::dispatch`) construct the same enum value; both consumers
(streaming endpoint, snapshot endpoint) serialise it identically.

## AllocStatusRow extension

The `overdrive-core::traits::observation_store::AllocStatusRow`
**gains a `reason: Option<TransitionReason>` field** AND a
`detail: Option<String>` field. Without this, [C6] is unimplementable —
the snapshot reads from the ObservationStore, the streaming surface
reads from the broadcast channel, and the only single source of truth
that flows through both is the row itself.

This is a Phase 1 internal type change. The rkyv `Archive` derive
on `AllocStatusRow` requires `TransitionReason` to derive `Archive +
Serialize + Deserialize` — additive. The existing
`AllocStatusRowBody::pending_with_reason` constructor (which already
produced `Some(String)` for capacity-exceeded) is updated to take
`TransitionReason::NoCapacity` plus a `String` detail, projecting both
into the wire shape.

The action shim is amended to capture `DriverError::StartRejected.reason`
(currently `reason: _` — discarded) and write it into
`AllocStatusRow.detail` while setting `AllocStatusRow.reason =
Some(TransitionReason::DriverStartFailed)`. The previously-unwritten
information becomes wire-visible by construction.

## AllocState extension

`AllocState::Failed` is added as a fifth variant. Today the action
shim collapses driver failures to `Terminated`, which conflates "the
operator stopped this" with "the driver could not start this." Adding
`Failed` lets the CLI render terminal-failure status distinctly from
operator-stop. Display string: `"failed"`.

This is the smallest possible cut — three sites change: enum decl,
`Display` impl, action shim's `StartRejected` arm.

---

## Slice mapping

| Slice | Owns | Consumes |
|---|---|---|
| 01 — alloc status enrichment | ADR-0033; `AllocStatusResponse` extension; `TransitionReason`, `TransitionRecord`, `RestartBudget`, `ResourcesBody` types; CLI renderer rewrite; `AllocStatusRow` row-shape extension; `AllocState::Failed`; action-shim row-write amendment | none from this feature |
| 02 — NDJSON streaming submit | ADR-0032; `SubmitEvent`, `TerminalReason`, `TransitionSource`, `AllocStateWire`; broadcast channel on `AppState`; streaming handler with `select!` cap timer; CLI NDJSON consumer | Slice 01 (`TransitionReason`, `TransitionRecord` already landed) |
| 03 — `--detach` + pipe detect | CLI flag wiring + `IsTerminal` detection | Slice 02 |

Slice 01 ships first, alone, ungated. Slice 02 depends on 01 and ships
second. Slice 03 is conditional per its brief; if Slice 02 lands within
budget, fold in.

---

## ADR follow-ons

- **ADR-0032** — NDJSON streaming submit shape. (See
  `docs/product/architecture/adr-0032-ndjson-streaming-submit.md`.)
- **ADR-0033** — `alloc status` snapshot enrichment. (See
  `docs/product/architecture/adr-0033-alloc-status-snapshot-enrichment.md`.)

Both follow ADR-0014 (shared types) and ADR-0015 (error shape)
precedents. Both are amendments to brief.md §22 and §32 (quality
attributes).

---

## Handoff

- → DEVOPS (`nw-platform-architect`): receive `outcome-kpis.md` only;
  no new telemetry surface required.
- → DISTILL (`nw-acceptance-designer`): receives this `wave-decisions.md`,
  `architecture.md`, the two ADRs, and the journey YAML with embedded
  Gherkin. Tier-1 (DST) covers reconciler purity unchanged plus a new
  invariant `StreamingSubmitTerminalEventBoundedByCap`. Tier-3
  (real-kernel) covers the broken-binary regression-target session.

---

## Changelog

| Date | Change |
|---|---|
| 2026-04-30 | Initial DESIGN wave artifacts. Decisions D1–D8 above; ADR-0032 + ADR-0033 produced. Reuse Analysis EXTEND-only (zero CREATE NEW unjustified). Echo peer review pending. |
