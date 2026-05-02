# fix-streaming-pre-subscribe-race

**Date**: 2026-05-02
**Type**: Bugfix
**Scope**: `overdrive-control-plane` — `streaming::build_stream`

## Summary

Streaming `POST /v1/jobs` (`Accept: application/x-ndjson`) hung the
client for the full `streaming_cap` window (default 60s) and emitted a
false `SubmitEvent::ConvergedFailed { terminal_reason: Timeout, ... }`
even when the workload had reached `Running` / `Terminated` /
`BackoffExhausted` within the first reconcile tick (~100ms). The JSON
lane was unaffected.

The defect was a subscriber-registration-versus-publish ordering window:
`build_stream` called `bus.subscribe()` only after emitting `Accepted`,
so any `LifecycleEvent` published in the interval between the upstream
`IntentStore::put_if_absent` (which enqueues an evaluation that triggers
the convergence loop) and that subscribe call was permanently lost.

## Root Cause

`tokio::sync::broadcast::Sender::send` only delivers to receivers
`subscribe()`-d at send-time; pre-subscription messages are dropped on
the floor. In `build_stream`, the subscribe call sat at
`streaming.rs:156` — *causally downstream* of the
`put_if_absent` at `handlers.rs:247` that triggers reconciler
publishers. The convergence loop runs as a separate `tokio::spawn`-ed
task at 100ms cadence; by the time `bus.subscribe()` ran, the
reconcile tick may have already dispatched `Action::StartAllocation`,
the action shim may have written the obs row, and `bus.send(...)` may
have already returned with no subscribers.

The existing `Lagged(_)` recovery path (`streaming.rs:206-227`) was
correctly shaped — read the latest `AllocStatusRow` via
`obs.alloc_status_rows()`, project to terminal, return `None` if not
terminal — but it was gated on the wrong trigger. It only fired on
buffer overflow during a *live* subscription. There was no analogous
bridge at subscribe-time. The architecture doc and module docstring
enumerated four loop-resolution paths (live event, Lagged, Closed,
Timeout) but never enumerated subscribe-race as a fifth missed-events
class.

The acceptance fixture polled `state.lifecycle_events.receiver_count()
>= 1` before manually calling `emit_lifecycle`, which accidentally
synchronised production and test where production has no such barrier;
the symptom was masked.

## Fix

Insert a single `lagged_recover` call between `bus.subscribe()` and the
`loop {`. On `Some(terminal)` → emit the terminal line and `return`. On
`None` → fall through into the loop normally. Reuse `lagged_recover`
directly — no new helper. Emit the terminal `SubmitEvent` directly
(no synthetic `LifecycleTransition`), mirroring the existing Lagged-arm
shape. The KPI-01 first-byte-latency invariant is preserved: `Accepted`
is still yielded synchronously *before* the snapshot read.

The module docstring was updated to enumerate the subscribe-race as a
fifth missed-events class alongside live event, Lagged, Closed, and
Timeout. `docs/feature/cli-submit-vs-deploy-and-alloc-status/design/architecture.md`
§10 received a paragraph documenting the same.

Single-cut migration: no shadow path, no feature flag, no parallel old
behaviour. The pre-existing Lagged-arm call site is unchanged; both
call sites now coexist as designed (one bridges pre-subscribe, the
other bridges buffer-overflow).

## Steps Completed

| Step | Description | Commit |
|------|-------------|--------|
| 01-01 | RED — `#[ignore]`-gated regression test (`s_cp_12`) | `132f877` |
| 01-02 | GREEN — `lagged_recover` snapshot inserted post-subscribe; `#[ignore]` removed; module docstring + architecture.md §10 updated | `8a14fa4` |

## Files Changed

- `crates/overdrive-control-plane/src/streaming.rs` — `lagged_recover`
  snapshot inserted between `bus.subscribe()` and the select loop;
  module docstring updated to enumerate subscribe-race as a fifth
  missed-events class.
- `crates/overdrive-control-plane/tests/acceptance/streaming_submit.rs`
  — new `s_cp_12_pre_subscribe_terminal_does_not_hang_until_cap`
  regression test pinning the symptom; `s_cp_07` reordered to write
  the row *after* `receiver_count >= 1` (matching the `s_cp_01`
  pattern) so its byte-equality assertion against the projected live
  event still holds with the new snapshot path in place.
- `docs/feature/cli-submit-vs-deploy-and-alloc-status/design/architecture.md`
  — §10 paragraph appended enumerating subscribe-race as a missed-events
  class and naming the snapshot-bridge mitigation.

## Lessons

- **Subscribe-race is a structural class, not a one-off race.** Any
  `tokio::sync::broadcast::Receiver` whose subscription is causally
  downstream of a write that triggers publishers needs a snapshot-bridge
  primitive at subscribe-time. Subscribe-then-snapshot-then-stream is
  the load-bearing shape — Kubernetes informers' List-then-Watch and
  etcd revision-bound watches all exist for this reason. The four
  documented loop-resolution paths (live event, Lagged, Closed, Timeout)
  enumerated *steady-state* failure modes but missed the *startup*
  failure mode entirely.
- **The recovery primitive was correctly shaped but mis-gated.**
  `lagged_recover` was authored as a buffer-overflow recovery, but its
  *semantics* (snapshot the latest row and project to terminal) are
  general-purpose. The fix is one new call site, not a new function.
  When a recovery primitive matches the shape of a *new* failure class,
  reuse it before reaching for a new helper — both call sites coexist,
  one per missed-events class.
- **The acceptance fixture's `receiver_count() >= 1` polling masked the
  bug.** Other streaming tests (`s_cp_01`, `s_cp_03`, `s_cp_07`) waited
  for the receiver to register before firing events, accidentally
  synchronising production and test where production has no such
  barrier. The new `s_cp_12` deliberately leaves the bus silent — this
  is the missing test discipline: every broadcast-subscriber endpoint
  needs at least one regression test that pre-publishes (or pre-seeds
  the observable equivalent) *before* subscribe and asserts the stream
  still terminates without the cap timer. The masking pattern itself is
  worth flagging in code review of any future broadcast-bus consumer.
- **The two-step `#[ignore]`-then-uncomment Outside-In TDD shape
  continues to work.** Same as `fix-stop-branch-backoff-pending` and
  `fix-noop-self-reenqueue` — the RED test lands with `#[ignore]` so
  lefthook's nextest-affected pre-commit pass stays green between
  commits, and the GREEN commit applies the fix and removes the marker
  in the same cohesive commit. This is the project's standard shape
  for runtime-assertion RED scaffolds (distinct from the
  `panic!("RED scaffold")` shape used for compile-fail / exhaustive-match
  scaffolds in `testing.md`).

## References

- RCA: `docs/feature/fix-streaming-pre-subscribe-race/deliver/rca.md`
  (preserved in feature workspace for the duration of this PR;
  user-validated 2026-05-02)
- Whitepaper §3 *Architecture Overview* / §10 *Gateway* — broadcast
  wiring path and streaming submit shape
- Architecture SSOT: `docs/feature/cli-submit-vs-deploy-and-alloc-status/design/architecture.md`
  §10 — missed-events class enumeration
- Precedent: `docs/evolution/2026-05-02-fix-stop-branch-backoff-pending.md`
  (similar two-step Outside-In TDD shape, different defect class)
- ADR-0032 §7 — `lagged_recover` primitive design
- Test discipline: `.claude/rules/testing.md` § "RED scaffolds and
  intentionally-failing commits"
