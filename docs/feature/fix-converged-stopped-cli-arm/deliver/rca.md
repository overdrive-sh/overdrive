# RCA: `ConvergedStopped` silently ignored by streaming CLI consumer

**Status**: APPROVED — proceeding to regression test + fix
**Date**: 2026-05-02
**Approval**: User confirmed root cause and fix direction; exit code 0 across all `StoppedBy` variants.

---

## Bug

`overdrive job submit` with the streaming lane (`--stream`) exits 2 with
a `BodyDecode` error when the workload reaches a clean terminal stop
(operator-initiated, reconciler-initiated, or natural process exit).

Operator-visible symptom: a concurrent `overdrive job stop` against a
streaming submit prints

```
Error: streaming submit response closed without ConvergedRunning or ConvergedFailed
```

to stderr and exits 2, instead of a clean "job was stopped" message
with exit 0.

---

## Root cause

`crates/overdrive-cli/src/commands/job.rs:553-560` — the streaming
consumer's match falls through to `_ =>` for `SubmitEvent::ConvergedStopped`,
treating a *present-day terminal event* as if it were a forward-compat
`#[non_exhaustive]` future variant. The catch-all only logs the event
and continues looping. The server then closes the HTTP body, the
`while let Some(...) = stream.next().await` loop exits, and the
function returns
`Err(CliError::BodyDecode { cause: "streaming submit response closed without ConvergedRunning or ConvergedFailed" })`
at `commands/job.rs:566-569`.

`ConvergedStopped` was added to `SubmitEvent` in the same PR
(`crates/overdrive-control-plane/src/api.rs:627` — variant) and is
emitted by the server when an alloc reaches `Terminated`
(`crates/overdrive-control-plane/src/streaming.rs:411,466`). The CLI
consumer was not updated alongside the server.

### Five Whys

1. CLI exits 2 with a body-decode error → because `consume_stream`
   returns `Err(CliError::BodyDecode)` after the server closes the
   stream cleanly.
2. Stream-close error fires → because no terminal arm matched
   `ConvergedStopped`; the loop ran out of input.
3. No terminal arm matched → `ConvergedStopped` falls through to the
   `_ =>` catch-all (line 558), which only logs.
4. Catch-all swallowed it → comment claims the catch-all is for
   future `#[non_exhaustive]` variants, but `ConvergedStopped` is a
   *present* terminal variant that needs explicit handling.
5. No test caught this → no CLI acceptance test drives `consume_stream`
   against an NDJSON sequence ending in `ConvergedStopped`. The
   render test suite covers only `format_failed_block` paths.

---

## Fix direction

Three additive changes:

1. **`crates/overdrive-cli/src/commands/job.rs`** — explicit
   `SubmitEvent::ConvergedStopped { alloc_id: _, by }` arm before the
   `_ =>` catch-all. Mirrors the `ConvergedFailed` arm shape:
   accumulates `accepted`, builds a summary via the new render
   function, returns `SubmitStreamingOutput { exit_code: 0, summary,
   terminal_reason: None, streaming_reason: None, streaming_error: None,
   ... }`. Add `StoppedBy` to the import set.

2. **`crates/overdrive-cli/src/render.rs`** — new pure function
   `format_stopped_summary(job_name: &str, by: StoppedBy) -> String`.
   Mirrors `format_running_summary` shape; one-line operator-facing
   message naming the initiator (operator / reconciler / process).

3. **Tests** — regression test in
   `crates/overdrive-cli/tests/acceptance/` (or `tests/integration/`)
   that drives `consume_stream` against an NDJSON sequence ending in
   `ConvergedStopped`, asserts `exit_code == 0`, asserts the summary
   contains the job id and the initiator. Also `format_stopped_summary`
   pure-function tests (one per `StoppedBy` variant).

### Exit-code choice

**Exit code 0 across all `StoppedBy` variants.** ADR-0032 §9 reserves
exit 1 for `ConvergedFailed` ("did not converge to running"); a clean
stop is a successful terminal outcome from the operator's perspective.
User explicitly approved this choice.

---

## Files affected

- `crates/overdrive-cli/src/commands/job.rs` — new match arm + import
  (~25 lines added)
- `crates/overdrive-cli/src/render.rs` — new `format_stopped_summary`
  pure fn (~10 lines added)
- `crates/overdrive-cli/tests/acceptance/streaming_submit_cli_render.rs`
  *or* a new sibling — regression coverage for the new arm and the
  new render fn

No production behaviour changes outside the previously-unhandled
`ConvergedStopped` event. Wire format is unchanged.

---

## Risk assessment

**Low.** Purely additive: new explicit arm before an existing
catch-all, new pure render fn. No existing arms or render functions
modified. The `_ =>` catch-all stays in place to handle genuine
future `#[non_exhaustive]` variants.
