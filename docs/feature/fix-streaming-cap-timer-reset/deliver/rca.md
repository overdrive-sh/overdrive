# RCA — Streaming cap timer resets on every received event

**Bug ID**: fix-streaming-cap-timer-reset
**Reporter**: code review (cited in /nw-bugfix invocation 2026-05-02)
**Severity**: real (resource-exhaustion vector for control-plane HTTP server)
**Confirmed against source**: 2026-05-02

## Defect

`crates/overdrive-control-plane/src/streaming.rs:211-318` — the wall-clock
cap timer for the NDJSON streaming submit loop is documented (lines 22-25,
57) as a **wall-clock deadline from stream creation**, but the implementation
behaves as an **inactivity timeout**: the cap is reset every time a
`LifecycleEvent` arrives.

## Root Cause Chain (5-Whys)

1. **Why does the cap not bound stream lifetime as documented?**
   The `Sleep` future is recreated on every loop iteration.

2. **Why is it recreated?**
   Line 304: `() = clock.sleep(cap) =>` is an *expression* inside
   `loop { tokio::select! { ... } }`. Each iteration evaluates the
   expression afresh, constructing a new `Sleep` from the elapsed-zero
   baseline.

3. **Why does that matter?**
   `tokio::select!` polls a *new* future each pass. The prior `Sleep`'s
   accumulated elapsed time is discarded the moment the select resolves
   on a different arm and the loop iterates.

4. **Why wasn't a single pinned timer used?**
   The `select!` block was authored as if `clock.sleep(cap)` were a
   one-shot, not accounting for `loop { select! }` re-evaluating arm
   expressions each iteration. Common tokio footgun — the canonical
   shape (`tokio::pin!` outside the loop, `&mut fut` inside the select)
   was missed.

5. **Why didn't tests catch it?**
   Existing DST scenarios (verified by inspection of the file's
   neighbouring patterns) cover (a) the no-event timeout path and
   (b) terminal-on-first-event. Neither exercises *non-terminal events
   arriving at sub-cap intervals indefinitely* — the exact shape that
   exposes the bug.

## Behavioural impact

A workload in a sustained restart loop (e.g. a crash-restart job whose
restart budget has not yet exhausted) emits a `LifecycleTransition`
every few seconds. Each transition resets the cap. The streaming
connection stays open indefinitely instead of returning
`ConvergedFailed { Timeout { after_seconds: cap } }` at `cap` seconds
from stream entry.

Resource-exhaustion shape: open file descriptors, broadcast subscribers
on `state.lifecycle_events`, allocator pressure on the per-iteration
`Sleep` futures the runtime keeps re-allocating.

## Proposed Fix

Replace the per-iteration `clock.sleep(cap)` arm with a pinned `Sleep`
constructed once before the loop:

```rust
let cap_future = clock.sleep(cap);
tokio::pin!(cap_future);
loop {
    tokio::select! {
        biased;
        recv = sub.recv() => { /* ... existing arm ... */ }
        () = &mut cap_future => { /* ... existing timeout terminal ... */ }
    }
}
```

`biased;` order preserved (recv arm first). Behaviour under DST is
strictly *more* faithful: `SimClock::sleep` registers a single waker on
the harness's deadline-park primitive, advanced by `tick(cap + ε)` —
matching the production semantics of "60s from when the stream entered
the loop."

## Files affected

- `crates/overdrive-control-plane/src/streaming.rs` — the fix itself
  (lines 211-318) and a new regression test (separate test module or
  integration test crate, see step 02 below).

Doc block (lines 22-25, 57) already reads "wall-clock cap timer" — no
prose change needed; the bug was that the implementation didn't match
the documentation.

## Regression test shape

Per `.claude/rules/testing.md` — Tier 1 DST. The test:

1. Subscribes to `build_stream` with `cap = Duration::from_secs(60)`.
2. Harness drives `SimClock` and injects a non-terminal
   `LifecycleEvent` every 30s (sub-cap interval), e.g. an oscillation
   between `Pending → Running → Failed → Pending`.
3. Advances `SimClock` to `61s` from stream entry.
4. **Asserts** the stream emits `ConvergedFailed { Timeout {
   after_seconds: 60 } }` and the stream ends.

Current code: timer keeps resetting at each 30s event; stream stays
open past `61s` — test FAILS.

Fixed code: pinned timer fires at the absolute 60s deadline regardless
of intervening events — test PASSES.

## Risk

Low. Single-file mechanical change to a documented invariant; pinned
`Sleep` is the canonical tokio shape for cross-iteration timers. No API
shape change, no public surface change. DST semantics preserved or
improved.

## Out of scope

- Doc prose updates (current wording is correct).
- Any other arm of the select.
- The `lagged_recover` / `check_terminal` / pre-subscribe-race
  primitives — orthogonal to this bug.
