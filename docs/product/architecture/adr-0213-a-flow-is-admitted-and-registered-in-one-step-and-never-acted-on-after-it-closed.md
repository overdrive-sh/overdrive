# ADR-0213 — A flow is admitted and registered in one step, and a pairing continuation never acts on a flow that has closed

## Status

**Accepted — U-1, approved by the user 2026-10-06.** GH #295. Not yet
implemented. Contract: the #295 feature delta, § *[REF] Owner and
provisioner* and § *[REF] Per-kind total orders*.

Related decisions, each separately reversible: out-of-band pairing (D18,
ADR-0158); attach at pairing (EVMOD-PAIR, ADR-0212); stop and abort
semantics (ADR-0160); counted quiescence (ADR-0169); the control session
whose loss aborts its flows (ADR-0166).

## Context

Three events must abort every flow of an allocation or node: forwarding
quiescence (ADR-0169), allocation teardown, and control-session loss
(ADR-0166). Each aborts the flows it finds registered. An acceptor that
checked admission (allocation Active, live control session, forwarding not
quiesced, quota) and registered the flow afterwards could admit a flow on a
check one of these events had already made false, and the event would miss
it: the flow would run after a quiescence or teardown that claimed to have
stopped everything.

Pairing waits on a non-blocking connect (ADR-0151). The flow can be closed
by any of those events, or by `Abort`, while the connect is in flight.

## Decision

- **Admission and registration are one step.** The acceptor's admission
  check for a flow and the flow's registration in its allocation's flow
  table happen as one step relative to quiescence, teardown and
  control-session loss, so every flow those events must abort is already
  registered when they run. A flow is never admitted on a check that one of
  them has since made false. The owner applies the same step to a dial
  before it opens the flow.
- **A continuation stops for a closed flow.** When an acceptor's
  non-blocking connect completes, it attaches and sends `Paired` only if the
  flow is still open; for a flow already closed (aborted by quiescence,
  teardown, session loss or `Abort`) it closes the connected socket and
  sends nothing. A `Paired` that reaches a host-opened flow already closed is
  discarded.

## Alternatives considered

- **Check, then register.** The window between them is the race above: an
  abort event runs between the check and the registration and misses the
  flow. Rejected.
- **Re-check at the continuation instead of at admission.** The flow is not
  in the table while its connect is pending, so quiescence and teardown
  cannot abort it and quota does not count it. Rejected.
- **Let the continuation attach and send `Paired`, and abort afterwards.**
  Forwards bytes on a flow the abort event has already reported stopped, and
  sends the guest a `Paired` it must then undo. Rejected.

## Consequences

- `QuiescedHoldsNoFlow` and the teardown invariants hold at the event, not
  eventually.
- Checked in `specs/quint/guest-flow-owner` (module `owner_flows`; feature
  delta, § *Formal protocol model*).
- The rule is the host owner's and holds for every guest (ADR-0207).
