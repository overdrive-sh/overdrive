# C4 Level 3 — Component Diagram (streaming-submit endpoint)

**Wave**: DESIGN
**Date**: 2026-04-30

Zoom into the `overdrive-control-plane` container — specifically the
streaming-submit subsystem. The snapshot endpoint (`alloc_status`
handler) is simpler and is documented in `architecture.md` §7; no L3
needed for it. This L3 covers the streaming-submit path because it has
five distinct components interacting around the broadcast channel and
the wall-clock cap timer — the kind of complexity that warrants a
component diagram per the methodology rule.

```mermaid
C4Component
  title Component Diagram — streaming submit (overdrive-control-plane)

  Container_Ext(cli, "overdrive-cli", "Rust binary", "Reqwest client; line-delimited NDJSON consumer")
  ContainerDb_Ext(intent, "IntentStore", "redb")
  ContainerDb_Ext(obs, "ObservationStore", "redb")

  Container_Boundary(ctrl, "overdrive-control-plane") {
    Component(handler, "submit_job (handler)", "axum::handler", "Content-negotiates on `Accept` header. JSON lane: returns SubmitJobResponse. NDJSON lane: delegates to streaming_submit_loop")
    Component(loop_, "streaming_submit_loop", "async fn", "Subscribes to broadcast channel filtered by job_id; races event-emitting future against clock.sleep(cap); emits SubmitEvent lines as chunked body; decides ConvergedRunning / ConvergedFailed terminal events")
    Component(serializer, "ndjson_serializer", "fn", "Serializes SubmitEvent -> bytes + '\\n'; pure")
    Component(bus_recv, "broadcast::Receiver<LifecycleEvent>", "tokio::sync", "Filtered subscription to lifecycle events; .recv().await")
    Component(timer, "wall_clock_cap_timer", "Arc<dyn Clock>::sleep(WALL_CLOCK_CAP)", "DST-controllable cap; SystemClock in production, SimClock under DST")
    Component(view_read, "view_cache.read(JobLifecycle, target)", "fn", "Synchronous read of JobLifecycleView for restart_budget probe at terminal-decision time")
    Component(shim, "action_shim::dispatch (existing, EXTENDED)", "async fn", "After each obs.write, sends LifecycleEvent on the broadcast channel. Per-action error isolation preserved")
    Component(bus_send, "broadcast::Sender<LifecycleEvent>", "tokio::sync", "Process-global; in-process; held on AppState. Lagged subscribers fall back to one-shot snapshot")
    Component(error_map, "ControlPlaneError -> ErrorBody (ADR-0015)", "fn", "Existing exhaustive map; covers 400/404/409/500 on the JSON-ack lane only. NDJSON lane errors become SubmitEvent::ConvergedFailed events post-Accepted")
    Component(idempotency, "submit_job.idempotency_branch (existing)", "fn", "Read-then-write put_if_absent; emits IdempotencyOutcome::Inserted | Unchanged. Both lanes share this prefix")
  }

  Rel(cli, handler, "POST /v1/jobs", "rustls HTTPS")
  Rel(handler, idempotency, "writes IntentStore via put_if_absent")
  Rel(idempotency, intent, "put_if_absent")
  Rel(handler, error_map, "JSON-lane errors only (400/404/409/500)")
  Rel(handler, loop_, "NDJSON lane: enters streaming_submit_loop after sending SubmitEvent::Accepted")
  Rel(loop_, bus_recv, "subscribes (filtered by job_id)")
  Rel(bus_recv, bus_send, "in-process broadcast subscription")
  Rel(loop_, timer, "races against event-emitting future via tokio::select!")
  Rel(loop_, view_read, "reads restart_budget to decide BackoffExhausted terminal")
  Rel(loop_, serializer, "serializes each SubmitEvent")
  Rel(serializer, cli, "writes line + '\\n' to chunked response body")
  Rel(shim, bus_send, "broadcasts LifecycleEvent after every obs.write")
  Rel(shim, obs, "writes AllocStatusRow with reason+detail fields")
```

## Component responsibilities

| Component | Owns | Does not own |
|---|---|---|
| `submit_job` handler | Content negotiation; idempotency-prefix; error mapping for the JSON lane | The streaming loop body; the broadcast send |
| `streaming_submit_loop` | Subscription, terminal-event decision, cap timer race, line emission | The broadcast send (action shim does that); the row write (action shim does that); the IntentStore put (handler does that before entering the loop) |
| `ndjson_serializer` | `SubmitEvent → bytes + '\n'` serialisation; serde_json::to_vec + push '\n' | Anything else; pure |
| `broadcast::Receiver` (subscription) | Per-handler instance; filtered by `job_id` | Production rate limiting; lagged-subscriber recovery (loop handles via `Lagged` arm) |
| `wall_clock_cap_timer` | `clock.sleep(cap).await` — DST-controllable | The cap *value* (config) and the cap *response* (loop emits ConvergedFailed) |
| `view_read` | Synchronous read of the lifecycle view's `restart_counts` | Mutating the view; the runtime's hydration; the libSQL connection |
| `action_shim::dispatch` (extended) | Driver call + obs.write + broadcast send; per-action error isolation | The terminal-event decision (handler); the wall-clock cap (handler) |
| `broadcast::Sender` | Process-global send; AppState-held | Per-subscriber state; lagging recovery |

## Why this L3 and not deeper

- **Why component-level (L3)** — five interacting components inside a
  single subsystem with a non-trivial concurrency story
  (`tokio::select!` over a broadcast subscription and a clock timer).
  The methodology rule (Example 1 in the agent prompt) calls L3 for
  "complex subsystems" specifically.
- **Why no L4** — the implementation details (the precise
  `tokio::select!` arm structure, the `Lagged` recovery pseudocode)
  belong in the crafter's GREEN phase, not in the design. The
  contracts and the dispatch shape are what DESIGN owns.

## Verb labels on every arrow (per methodology Example 1)

Every arrow on the diagram above carries a verb label
(`subscribes / writes / serializes / races / broadcasts / reads`).
Reviewers can challenge any label; the verb is what binds the
component to its responsibility.

## What this diagram pins for crafter

- The broadcast channel is **in-process** (no network hop). The
  `Sender` is on `AppState`; the `Receiver` is per-streaming-handler.
- The wall-clock cap is a `tokio::select!` arm racing the
  event-emitting future against `clock.sleep(cap)`. **Not** a tower
  layer, **not** a subscription-primitive concern.
- Terminal-event decision lives **in the loop**, not in the
  reconciler. The reconciler dispatches `Action::*` (intent shape);
  the streaming surface decides `Converged*` (streaming shape).
- Error mapping (ADR-0015) covers the JSON lane only. NDJSON lane
  errors *after* the `Accepted` line is emitted become structured
  `SubmitEvent::ConvergedFailed` events. NDJSON lane errors *before*
  the `Accepted` line (validation, conflict, internal) flow through
  the same `error_map` because at that point the handler hasn't
  switched to chunked transfer yet — it can still return a single
  JSON `ErrorBody` with the appropriate 4xx/5xx status. (See
  ADR-0032 §HTTP error semantics in the streaming context.)
