# Slice 2 — NDJSON streaming submit

**Feature**: `cli-submit-vs-deploy-and-alloc-status`
**Wave**: DISCUSS / Phase 2.5
**Owner**: Luna (will hand to nw-solution-architect for ADR; nw-software-crafter for delivery)

## Goal

Make `overdrive job submit` stream the lifecycle reconciler's
convergence inline as NDJSON, exit non-zero on convergence failure,
and exit zero when the workload reaches Running. The single-verb
inner-loop experience the user asked for.

## IN scope

- `POST /v1/jobs` returns `application/x-ndjson` when the request
  carries `Accept: application/x-ndjson`. Existing
  `application/json` shape retained for back-compat (no Accept
  header → existing JSON ack).
- Typed event enum (`SubmitEvent` or DESIGN-named) in
  `overdrive-control-plane::api` with at minimum: `Accepted`,
  `LifecycleTransition`, `ConvergedRunning`, `ConvergedFailed`.
- Server-side: subscribe to ObservationStore lifecycle events for the
  just-committed JobId, emit one NDJSON line per transition. Server
  wall-clock cap closes the stream with `ConvergedFailed { terminal_reason: timeout, ... }`.
- CLI-side: TTY detection via `isatty(stdout)`. TTY → NDJSON; piped
  → JSON ack only (auto-detach); `--detach` flag forces JSON ack
  regardless. NDJSON consumer is a line-delimited reqwest reader.
- CLI exit-code mapping (per `Acceptance Criteria` table in
  `user-stories.md` US-02):
  - `ConvergedRunning` → 0
  - `ConvergedFailed` (any cause: driver error, backoff exhausted,
    server timeout) → 1
  - Client-side / transport / server-validation errors per
    ADR-0015 → 2
- Regression-target acceptance test: the user's exact session
  (broken-binary submit) reproduces the failure path end-to-end with
  exit code 1.

## OUT scope

- `alloc status --follow`, TUI mode, multi-replica progress
  aggregation (see Slice 1's OUT list).
- New driver kinds beyond ProcessDriver (still single-driver Phase 1).
- ratatui rendering. The NDJSON shape is intended to be
  forward-compatible with a future TUI mode, but no TUI rendering
  ships in this slice.
- CLI flags beyond `--detach` (no `--timeout`, no `--no-stream`,
  no `--quiet`).

## Learning hypothesis

**Disproves**: the Option S DIVERGE recommendation as a whole, if
- the broken-binary case fails to surface inline with the right
  exit code, OR
- the 200 ms first-event budget is unreachable on a healthy local
  control plane, OR
- operators actively prefer the detached-and-poll model after a
  trial week.
**Confirms**: streaming submit reaches Running OR surfaces failure
end-to-end on the regression-target session, with exit code matching
the terminal event, in a single verb. Closes the user's complaint.

## Acceptance criteria

1. With `Accept: application/x-ndjson`, `POST /v1/jobs` returns a
   chunked response whose first line is an `Accepted` event with
   `spec_digest`, `intent_key`, `outcome`.
2. The first NDJSON line lands within 200 ms p95 of the request being
   committed on a healthy local control plane.
3. Each AllocStatusRow transition produces exactly one
   `LifecycleTransition` line carrying `alloc_id`, `from`, `to`,
   `reason`, `source` (`reconciler` | `driver(process)`), `at`.
4. Convergence to Running emits `ConvergedRunning` and the stream
   closes; CLI exits 0 with a one-line summary.
5. Convergence to Failed (any cause) emits `ConvergedFailed` with
   `terminal_reason` and `error`; CLI prints a structured `Error:`
   block and exits 1.
6. Server-side wall-clock cap exceeded → `ConvergedFailed` with
   `terminal_reason: timeout`; CLI exits 1.
7. CLI on a TTY sends `Accept: application/x-ndjson` by default; CLI
   on a non-TTY (piped) sends `Accept: application/json`; CLI with
   `--detach` sends `Accept: application/json` regardless of TTY.
8. The `reason` string in `ConvergedFailed` for a given allocation
   equals the `last_transition.reason` rendered by `alloc status` for
   the same allocation, byte-for-byte.
9. Regression-target acceptance test: a Job spec with
   `exec.command = "/usr/local/bin/payments"` (no such binary) run
   through `overdrive job submit` reaches `ConvergedFailed`,
   surfaces the verbatim ENOENT-class driver error, and exits 1.
10. Without an `Accept` header, `POST /v1/jobs` returns the existing
    JSON ack shape (back-compat).

## Dependencies

- Slice 1 must land first (the `transition_reason` shape and the
  snapshot's `last_transition` field are the sources for AC #8).
- ObservationStore subscription primitive already exists.
- ADR-0023 action shim already in place (no reconciler-purity
  violations).
- ADR-0014 shared-types (CLI/server types live in one module).
- ADR-0015 error shape is the basis for the JSON-error path on
  client-side and server-validation failures.
- ADR-0027 verb-suffix shape is the precedent (this feature touches
  `POST /v1/jobs`, not the verb-suffix endpoints, but the same shape
  discipline applies).

## Effort estimate

≤1 day.

## Reference class

- `nomad job run` default-wait + `--detach` is the closest sibling.
- Observability vendors ship NDJSON over HTTP for streaming feeds
  (Honeycomb, Datadog logs, etc.) — the CLI consumer pattern is
  mature.
- axum's streaming-body shape (used in its `Sse` machinery) plus
  reqwest's `bytes_stream()` cover both sides idiomatically.

## Pre-slice SPIKE

**Optional, recommended**: 30-minute spike to validate that axum's
streaming-body machinery composes cleanly with the existing
`SubmitJobRequest`/`SubmitJobResponse` handler shape, AND that
serde_json's `Deserializer::from_reader` can be driven line-by-line
from a `reqwest::Response::bytes_stream()`. Both are well-precedented
but the spike de-risks Slice 2's first afternoon. If the spike
exceeds 30 minutes, that itself is a signal — DESIGN should
re-evaluate whether the slice is truly ≤1 day or should grow to
include Slice 3's flag-and-pipe scope as a single ≥1.5-day cut.
