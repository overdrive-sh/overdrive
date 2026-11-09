# ADR-0263: A Service deploy stream first reports the verdict the workload already has

## Status

Accepted, 2026-11-03 (approval: feature delta § Approval record, D11, all seven
shape choices; the defect is finding F-5 in the same document). **Amended by
D11-E1, user-approved 2026-11-09** (user: “Approve all three exact contracts”):
the in-process Service stream returns a typed read error; the original
no-new-public-surface statement below is superseded only for that Rust error
type. The exact signature and error mapping are in the feature delta § D11
interface changes. Bounded technical review iteration 2 and independent full
DESIGN review iteration 4 approved the exact amendment
([review](../../feature/service-allocation-readiness-and-ending/design/review.md)). Not implemented.
GH #318, GH #334. The Job stream's related defects (F-6) are GH #340 and are not
changed here. The exact contract, the lifecycle gate (G-8), the formal protocol
model and the DISTILL obligations (S-39 to S-42) live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Related: [ADR-0262](adr-0262-the-service-deploy-stream-is-bounded-by-a-progress-deadline-derived-from-the-startup-deadline.md)
(the deadline this decision runs before), ADR-0056 and ADR-0059 Q4 (what the Service
stream reports), ADR-0037 §4 (row and event carry one terminal value).

## Context

`overdrive deploy` of a spec whose bytes are already stored returns `Unchanged` and
still opens the Service deploy stream. That stream subscribes to the lifecycle event
broadcast after it emits `Accepted` and waits for an event whose terminal it can
project. It reads no observation row. Lifecycle events are published once, by the
action shim, after a row write. A Service that is already `Stable` committed and
published its `Stable` once, and the reconciler that announces it never announces it
again, so the stream waits for its time bound and ends with a timeout failure for a
healthy service. The same blindness loses a terminal committed between the handler's
enqueue and the stream's subscription on a first deploy, and a terminal missed by a
lagged subscriber. The Job stream already recovers a recorded terminal after
subscribing; the Service stream does not.

## Decision

**After subscribing to the lifecycle events, the Service deploy stream reads the
verdict the workload's current attempt already carries and, if it projects to a
stream terminal, reports it and ends; otherwise it waits as before.**

- Order: emit `Accepted`, subscribe, read the recorded verdict, then arm the
  progress deadline (ADR-0262) only if there was none. Subscribing first closes the
  window in which a terminal could be committed and published unseen.
- The recorded verdict is the terminal of the current attempt, selected with the
  restart authority's own rule, projected with the projection the live path uses. A
  restartable stop and a superseded attempt's terminal are not a verdict.
- On a lagged subscriber the stream reads again.
- A failed read ends the stream with a stream error. It is not treated as "no
  verdict". D11-E1 preserves the typed observation-store cause at the Rust
  stream boundary and keeps line-emission failures a separate error variant.
- The owner is the stream handler. The deploy handler, the reconcilers, the wire,
  the persisted state and the CLI wire do not change. D11-E1 adds one public
  in-process Rust stream error type and changes the stream item's error type.

## Alternatives considered

- **The handler does not open a stream for `Unchanged`.** The outcome says the
  intent bytes match, not what the workload is doing. The handler would need the
  same read and the same projection to decide, and the first-deploy race and the
  lagged path would remain.
- **`ServiceLifecycle` announces `Stable` again on a redeploy.** A second terminal
  write and event per redeploy, and it removes a dedup the reconciler keeps for its
  own loop guard.
- **The CLI describes the workload after `Accepted`.** The verdict would live on one
  client behind a second read path; the race and the lagged path would remain.
- **Rely on the progress deadline.** It reports a healthy service failed.
- **Read before subscribing.** A terminal committed between the two is lost.
- **Choose the latest row by update stamp, as the Job stream does.** It orders rows
  of different allocations by a per-key counter; the restart authority's rule needs
  no such assumption.

## Consequences

- Positive: `overdrive deploy` of an unchanged, settled Service ends at once with
  its recorded verdict instead of a false timeout; a recorded give-up is reported at
  once.
- Positive: the first-deploy race and the lagged-subscriber wait are closed by the
  same read.
- Positive: the observation-read failure is typed at the in-process stream
  boundary (D11-E1); no wire, persisted or CLI wire change.
- Negative: the stream now reports a recorded `Stopped` for an operator-stopped
  workload (exit 0) on an unchanged redeploy; it does not resume the workload.
- Negative: the Job stream keeps its own recovery, which absorbs a read failure and
  orders rows by update stamp; this decision does not change it.
- Negative: correctness depends on the shim committing a row before publishing its
  event. That is the existing order; the DISTILL obligations check it.
