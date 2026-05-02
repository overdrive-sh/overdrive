# RCA — `TerminalReason::Timeout { after_seconds: 0 }` misrepresents channel closure

**Defect site**: `crates/overdrive-control-plane/src/streaming.rs:281-297`
**Wire type site**: `crates/overdrive-control-plane/src/api.rs:400-418`
**Specifying ADR**: `docs/product/architecture/adr-0032-ndjson-streaming-submit.md` §8 (HTTP error semantics)

---

## Problem statement

When the `lifecycle_events` broadcast channel is dropped while a streaming
submit is in-flight (typical: server shutdown), the streaming handler emits

```
SubmitEvent::ConvergedFailed {
    alloc_id: None,
    terminal_reason: TerminalReason::Timeout { after_seconds: 0 },
    reason: None,
    error: Some("lifecycle channel closed"),
}
```

The `terminal_reason` discriminant — the field consumers branch on
programmatically — is `Timeout`, not a channel-closed indicator. The
human-readable `error` string clarifies the cause for an operator who
reads it, but every machine consumer (CLI exit-code mapping, hint
selection, future TUI) sees a 0-second timeout, which is both
semantically wrong and contradicts the spec.

---

## 5 Whys — multi-causal chain

### Branch A — semantic misclassification on the wire

```
WHY 1A: Channel-closed events report TerminalReason::Timeout { after_seconds: 0 }.
        [Evidence: streaming.rs:284-291 — the Closed arm constructs
         `terminal_reason: TerminalReason::Timeout { after_seconds: 0 }`.]

  WHY 2A: The streaming handler had no other variant available that fits
          "stream interrupted server-side; no specific cause."
          [Evidence: api.rs:403-418 enumerates exactly three variants —
           DriverError { cause }, BackoffExhausted { attempts, cause },
           Timeout { after_seconds }. None represents channel closure.
           The doc-comment table at api.rs:384-388 lists only those three.]

    WHY 3A: TerminalReason was designed against the convergence outcomes
            the reconciler can produce, not against the streaming
            handler's *transport* failure modes.
            [Evidence: every variant's docstring frames it from the
             reconciler's perspective: "Streaming handler observed
             {restart_count == max | unrecoverable driver error | wall-clock
             cap fired}". The handler's own failure modes (broadcast bus
             closed, serialiser panic, peer disconnect) are not in the
             enum's domain.]

      WHY 4A: ADR-0032 §8 distinguished "broadcast channel closed
              unexpectedly" from "wall-clock cap fired" but mapped the
              former to TerminalReason::DriverError, not to a dedicated
              variant.
              [Evidence: adr-0032-ndjson-streaming-submit.md:508 —
               "Streaming-side internal failure AFTER `Accepted` line
               (broadcast channel closed unexpectedly, serialiser panic)
               | `SubmitEvent::ConvergedFailed { terminal_reason:
               DriverError, ... }`". The ADR specifies DriverError; the
               implementation emits Timeout. The implementation drifted
               from spec.]

        WHY 5A: TerminalReason::DriverError requires a `cause:
                TransitionReason` payload, but the channel-closed code
                path has no TransitionReason in scope to satisfy that
                contract — the bus is gone, so there is nothing to
                hydrate. The implementer reached for the only
                payload-free variant available, which is Timeout
                { after_seconds: u32 }, and hard-coded `0`.
                [Evidence: api.rs:407 — DriverError { cause:
                 TransitionReason } is non-optional. api.rs:417 —
                 Timeout takes only u32. streaming.rs:286-288 sets
                 after_seconds: 0 with no fallback cause; the whole
                 sub-event is synthesised inline without observation
                 hydration. The Closed arm has no `cause` to put in
                 DriverError; it picked the variant whose payload
                 it *could* satisfy.]

        ROOT CAUSE A: The TerminalReason wire enum has a domain gap —
                      it models reconciler outcomes only, with no
                      variant for stream-transport failure. ADR-0032 §8
                      papered over the gap by routing channel closure
                      into DriverError, but the routing was never wired
                      because DriverError requires a cause payload the
                      Closed arm cannot construct.
```

### Branch B — CLI consumer logic is discriminant-driven

```
WHY 1B: CLI rendering and hint selection branch on the TerminalReason
        discriminant, so a wrongly-discriminated event produces wrong
        operator-facing text.
        [Evidence: render.rs:390-403 (`derive_reason_from_terminal`) —
         match on TerminalReason variants; Timeout produces
         `format!("workload did not converge within {after_seconds}s")`,
         which on the buggy path renders as "workload did not converge
         within 0s". render.rs:428-435 (`derive_hint`) — Timeout maps
         to "workload did not converge within the server cap; consider
         --detach for long-running submits", which is misleading guidance
         for a server shutdown.]

  WHY 2B: The standalone `error: Option<String>` field carries the
          accurate human-readable cause, but the renderer prefers the
          structured discriminant for `reason:` text and hint selection.
          [Evidence: render.rs:413-420 — `derive_hint` consults
           reason then terminal_reason, never the freeform `error`.
           job.rs:531-537 — format_failed_block forwards both `error`
           and `terminal_reason`, but render.rs:381 `derive_hint`
           does not consult `error`. The `error` field is for tail
           context, not classification.]

    WHY 3B: This is by design and correct: ADR-0032 §9 mandates
            programmatic consumers branch on the structured discriminant,
            not on free-text. Free-text strings are not a stable API.
            [Evidence: api.rs:380-392 doc-comment — "the inner
             `terminal_reason` controls *rendering*, not exit code".
             The structured field IS the wire contract; the freeform
             `error` is a tail-context hint.]

      WHY 4B: The CLI's correct discipline (branch on discriminant)
              is exactly what makes the misclassification observable
              and harmful — a CLI that branched on the freeform string
              would coincidentally render the right thing, but that
              would be the wrong fix.

        ROOT CAUSE B: Downstream consumers correctly trust the
                      structured discriminant. The bug is upstream —
                      the discriminant must be honest.
```

### Branch C — drift between ADR-0032 §8 and the implementation

```
WHY 1C: ADR-0032 §8 specifies channel-closed → DriverError; the
        implementation emits Timeout.
        [Evidence: adr-0032-ndjson-streaming-submit.md:508 vs.
         streaming.rs:286.]

  WHY 2C: There is no test that asserts the Closed arm produces
          a specific TerminalReason variant. Searching for tests
          touching channel closure on the streaming bus yielded
          no matches.
          [Evidence: grep 'Closed|drop|TerminalReason|after_seconds|
           channel closed' across crates/overdrive-control-plane/tests/
           returned zero hits in lifecycle_broadcast.rs. tests/acceptance/
           submit_event_serialization.rs only round-trips the existing
           three variants (lines 129-135).
           tests/integration/streaming_submit_broken_binary.rs (CLI side)
           explicitly asserts NOT Timeout for the broken-binary path, but
           never exercises bus-closed.]

    WHY 3C: The Closed arm code-path is only reachable by dropping the
            broadcast sender mid-stream — a server-shutdown scenario
            that no test fixture currently constructs. Phase 1 tests
            exercise convergence outcomes, not transport failures.

      WHY 4C: The wire-shape regression suite (subset of property tests
              on `arb_terminal_reason`, submit_event_serialization.rs:129-135)
              proves round-trip equality but cannot detect that an
              emit-site picked the *wrong* variant.

        ROOT CAUSE C: The streaming handler's transport-failure arms
                      are untested. The Closed arm's variant choice
                      was never validated against ADR-0032 §8.
```

### Cross-validation

- A + B + C are consistent. A is the structural cause (missing wire
  variant + ADR routing that requires a payload the call site cannot
  produce). B is why the bug surfaces operator-visibly. C is why the
  bug shipped without being caught.
- All three explain the symptom collectively: A produces the wrong
  discriminant; B turns the wrong discriminant into wrong rendering
  and wrong hints; C is why no gate caught it.
- No contradictions: B does not imply A is wrong (the CLI is right to
  trust the discriminant); C does not imply A is wrong (untested code
  is still wrong code).

---

## Contributing factors

1. **Type-system gap.** `TerminalReason` (api.rs:400-418) was modelled
   against reconciler convergence outcomes. Stream-transport failures
   (broadcast bus closed, serialiser panic per ADR-0032 §8) are a
   separate failure class with no native wire representation.
2. **ADR routing needs a payload the call site does not have.**
   ADR-0032 §8 routes channel-closed → DriverError, but DriverError
   requires `cause: TransitionReason`. The Closed arm has no
   TransitionReason in scope (the bus is gone, observation hydration
   is not attempted), so the spec-mandated routing is impossible to
   honour without inventing a synthetic cause.
3. **Variant-selection by elimination, not by meaning.** The implementer
   reached for `Timeout { after_seconds: 0 }` because it was the only
   payload-free variant. The `0` is a sentinel; the contract bans
   sentinels (development.md "Sum types over sentinels" — "do not use
   sentinels (None, -1, empty Vec) to carry semantic meaning"). This
   is exactly the rule-violation shape the convention exists to forbid.
4. **`#[non_exhaustive]` on `TerminalReason` is already in place**
   (api.rs:402), and the renderer (render.rs:401) and CLI test
   (streaming_submit_broken_binary.rs uses `other => panic!(...)`)
   already handle unknown variants gracefully — adding a new variant
   is a backward-compatible change.
5. **Test gap.** No acceptance test exercises `RecvError::Closed` on
   the streaming path. The wire-shape property tests round-trip the
   existing three variants but cannot catch wrong emit-site classification.
6. **Conflation across timer arms.** The legitimate timeout path at
   streaming.rs:304-317 emits `Timeout { after_seconds: cap.as_secs() as u32 }`
   with a non-zero cap. The buggy arm emits `Timeout { after_seconds: 0 }`.
   A consumer doing `match terminal_reason { Timeout { after_seconds }
   if after_seconds > 0 => ..., Timeout { after_seconds: 0 } => ... }`
   would have to guard on a magic value to distinguish them — exactly
   the smell `enum`-as-state is supposed to eliminate.

---

## Proposed fix

### 1. Add a new `TerminalReason` variant

`crates/overdrive-control-plane/src/api.rs:403-418` — extend the enum:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum TerminalReason {
    DriverError { cause: TransitionReason },
    BackoffExhausted { attempts: u32, cause: TransitionReason },
    Timeout { after_seconds: u32 },
    /// The streaming handler's lifecycle-events broadcast channel was
    /// closed before a terminal event arrived (typical: server
    /// shutdown while a stream is in-flight). Distinct from
    /// `Timeout` (no wall-clock cap fired) and from `DriverError`
    /// (no `TransitionReason` is available — the bus that would
    /// carry one is gone). Operators should re-issue the submit;
    /// the underlying job state is still recoverable from
    /// `overdrive alloc status`.
    StreamInterrupted,
}
```

Naming: `StreamInterrupted` (not `ChannelClosed` — `channel` is
implementation; `stream` is the wire concept). Payload-free is
correct: there is no further structured information to surface; the
human-readable cause stays in `error: Some(...)`.

### 2. Update the emit site

`crates/overdrive-control-plane/src/streaming.rs:281-297`:

```rust
Err(broadcast::error::RecvError::Closed) => {
    // Channel closed — emit the dedicated terminal variant. Distinct
    // from Timeout (no wall-clock cap fired) and DriverError (no
    // TransitionReason in scope). Per ADR-0032 §8 and the
    // `TerminalReason::StreamInterrupted` docstring.
    let terminal = SubmitEvent::ConvergedFailed {
        alloc_id: None,
        terminal_reason: TerminalReason::StreamInterrupted,
        reason: None,
        error: Some("lifecycle channel closed".to_string()),
    };
    match emit_line(&terminal) {
        Ok(line) => yield Ok(line),
        Err(err) => yield Err(err),
    }
    return;
}
```

### 3. Update CLI rendering

`crates/overdrive-cli/src/render.rs:390-403` — extend
`derive_reason_from_terminal`:

```rust
fn derive_reason_from_terminal(terminal: &TerminalReason) -> Option<String> {
    match terminal {
        TerminalReason::BackoffExhausted { cause, .. }
        | TerminalReason::DriverError { cause } => Some(cause.human_readable()),
        TerminalReason::Timeout { after_seconds } => {
            Some(format!("workload did not converge within {after_seconds}s"))
        }
        TerminalReason::StreamInterrupted => {
            Some("server-side stream interrupted before convergence".to_owned())
        }
        _ => None,
    }
}
```

`crates/overdrive-cli/src/render.rs:428-435` — extend
`derive_hint`:

```rust
match terminal_reason {
    TerminalReason::Timeout { .. } => {
        "workload did not converge within the server cap; consider --detach for \
         long-running submits"
            .to_owned()
    }
    TerminalReason::StreamInterrupted => {
        "server-side stream was interrupted; re-run `overdrive job submit` or \
         consult `overdrive alloc status --job <id>` for the current state"
            .to_owned()
    }
    _ => "see alloc status for full context".to_owned(),
}
```

### 4. Update ADR-0032 §8

`docs/product/architecture/adr-0032-ndjson-streaming-submit.md:508`
replace the row mapping channel-closed-unexpectedly to DriverError
with a row mapping it to `StreamInterrupted`. Note: this is the ADR
correction, not a divergence from it — the ADR's prior mapping was
unimplementable (DriverError needs a `cause` the call site cannot
produce). The architect agent owns the ADR edit per project
convention.

### 5. Update the OpenAPI surface

`api/openapi.yaml:1079` — `TerminalReason.oneOf` gets a fourth
member with `kind: stream_interrupted` and an empty `data` object
(or no `data`, matching whatever serde emits for a unit variant under
`tag = "kind", content = "data"` — verify with the existing
`generate-openapi` xtask). The schema regenerates from the Rust type
via `utoipa::ToSchema`, so the manual edit is to regenerate, not to
hand-author.

### 6. Add regression tests

- **Property test.** Extend `arb_terminal_reason` at
  `crates/overdrive-control-plane/tests/acceptance/submit_event_serialization.rs:129-135`
  with `Just(TerminalReason::StreamInterrupted).boxed()` so the
  round-trip property covers the new variant.
- **Acceptance test on the streaming handler.** New test under
  `crates/overdrive-control-plane/tests/acceptance/lifecycle_broadcast.rs`
  (or a new file `streaming_channel_closed.rs`): construct an
  `AppState`, kick off the stream, drop the lifecycle broadcast
  sender, and assert the terminal NDJSON line deserialises to
  `SubmitEvent::ConvergedFailed { terminal_reason:
  TerminalReason::StreamInterrupted, .. }`. This is the gap C
  identifies.
- **CLI render test.** Extend `crates/overdrive-cli/tests/acceptance/streaming_submit_cli_render.rs`
  with a case that constructs `TerminalReason::StreamInterrupted` and
  asserts the rendered `Reason:` and `Hint:` lines match the new
  arms in render.rs.

---

## Files affected

| Path | Change |
|---|---|
| `crates/overdrive-control-plane/src/api.rs` | Add `TerminalReason::StreamInterrupted` variant + docstring |
| `crates/overdrive-control-plane/src/streaming.rs` | Switch the `Err(RecvError::Closed)` arm to emit the new variant |
| `crates/overdrive-cli/src/render.rs` | Add match arm in `derive_reason_from_terminal` and `derive_hint` |
| `docs/product/architecture/adr-0032-ndjson-streaming-submit.md` | §8 row update — channel-closed maps to `StreamInterrupted`, not `DriverError` (architect agent owns the edit) |
| `api/openapi.yaml` | Regenerate via xtask — `TerminalReason.oneOf` gains the new member |
| `crates/overdrive-control-plane/tests/acceptance/submit_event_serialization.rs` | Extend `arb_terminal_reason` proptest generator |
| `crates/overdrive-control-plane/tests/acceptance/lifecycle_broadcast.rs` (or new file) | New acceptance test for the closed-channel path |
| `crates/overdrive-cli/tests/acceptance/streaming_submit_cli_render.rs` | New render case for the new variant |

---

## Risk assessment

### Wire-format compatibility

- `TerminalReason` is `#[non_exhaustive]` (api.rs:402). Adding a
  variant is a backward-compatible change for *new* server → old
  client only when old clients tolerate unknown variants. Two
  considerations:
  - **Server emits, CLI reads** — same workspace, lock-stepped versions
    in Phase 1. No skew risk in Phase 1.
  - **External NDJSON consumers** — third-party clients reading the
    stream see a new `kind: "stream_interrupted"`. Per ADR-0032 §3
    the wire shape is `tag = "kind", content = "data"`; older clients
    that exhaustively match (and reject unknown kinds) will fail. This
    is the cost of any additive enum change. Phase 1 has no external
    consumers; document the addition in the ADR's wire-evolution
    section.
- The OpenAPI schema regenerates from the Rust type. Re-run the xtask
  that produces `api/openapi.yaml` and commit the diff atomically with
  the Rust change.

### CLI exit-code semantics

- ADR-0032 §9 maps `ConvergedFailed → 1` regardless of `terminal_reason`
  (api.rs:390-392 doc-comment confirms). Exit code is unchanged for
  the channel-closed path: still 1. Risk: zero.

### Schema / OpenAPI surface

- Regeneration is the only safe path. Hand-editing `api/openapi.yaml`
  to add the new variant inline risks drift from the Rust type. The
  build should fail loudly if `openapi.yaml` is stale (this is the
  canonical xtask-gated check).

### Existing tests that may need updating

I searched the workspace for tests asserting `TerminalReason::Timeout
{ after_seconds: 0 }` against the channel-closed arm:

- `grep 'after_seconds: 0'` across `crates/overdrive-control-plane/tests/`,
  `crates/overdrive-cli/tests/`, and the workspace root: **zero
  matches.** No test currently asserts the buggy shape.
- `grep 'channel closed|RecvError::Closed'` across the same paths:
  **zero matches** in test sources. The Closed arm is currently
  un-tested.
- `submit_event_serialization.rs:129-135` — `arb_terminal_reason`
  generates the existing three variants. The proptest does not assert
  variant-set exhaustiveness; adding a fourth generator branch is
  additive and does not invalidate prior cases.
- `streaming_submit_broken_binary.rs:142-145` — explicitly asserts
  `terminal_reason must NOT be Timeout` for the broken-binary path.
  Adding `StreamInterrupted` does not affect this assertion (broken
  binary triggers `BackoffExhausted` / `DriverError`, not bus
  closure).
- `submit_event_serialization.rs:259, 274` — assert `Timeout {
  after_seconds: 60 }` round-trips and produces the wire bytes
  `"terminal_reason":{"kind":"timeout","data":{"after_seconds":60}}`.
  These assertions stay valid; they exercise `Timeout`, not the
  Closed arm.

**No existing test will fail or need rewriting.** The fix is purely
additive on both the Rust and test side. The tests being added are
new regression tests — gap C closes.

### Renderer match exhaustiveness

`render.rs:390-403` and `render.rs:428-435` already use a `_ =>`
fallback on `#[non_exhaustive]` `TerminalReason`. Adding the variant
without updating the renderer would compile cleanly and produce
generic text — graceful degradation. Updating the renderer in the
same PR is the load-bearing change for *correct* operator output.

### DST / observability

- The streaming handler runs under DST via injected `Clock` and
  the `tokio::sync::broadcast` channel (which is deterministic under
  turmoil's executor). Adding a variant changes no nondeterminism
  source; DST coverage is purely additive.

---

## Summary

The defect is a small, contained type-system gap: `TerminalReason`
omits a variant for stream-transport failure, ADR-0032 §8's
spec-routing was unimplementable as written, and the implementer
fell back to `Timeout { after_seconds: 0 }` — a sentinel — which
the CLI then renders as a 0-second timeout with the wrong hint.

The fix is additive on a `#[non_exhaustive]` enum: introduce
`TerminalReason::StreamInterrupted`, switch the emit site, extend
two CLI render arms, regenerate OpenAPI, and add the missing
acceptance test. No existing test asserts the buggy shape; no wire
contract is broken; CLI exit-code semantics are preserved.
