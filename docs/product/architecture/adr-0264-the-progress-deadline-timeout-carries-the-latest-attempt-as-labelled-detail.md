# ADR-0264: The progress-deadline timeout carries the latest attempt as labelled detail

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D10, shape
choices 5 and 6). Split out of
[ADR-0262](adr-0262-the-service-deploy-stream-is-bounded-by-a-progress-deadline-derived-from-the-startup-deadline.md)
under one decision per ADR: it can be adopted or reversed without the deadline
derivation. Not implemented. GH #318, GH #334. The exact wire surface, the render
text, the lifecycle gate (G-7) and the DISTILL obligations (S-35, S-36) live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).

## Context

When the Service deploy stream reaches its time bound it ends with a failure that
names the elapsed seconds and nothing else. With a startup-probe failure named only
at give-up (ADR-0259), about six startup deadlines after the first attempt, the
timeout is the operator's only terminal at the default deadline, and it says nothing
about what the platform did in the meantime. The stream already receives every
lifecycle event of its workload, including each attempt's ending, so the most recent
one is available without a read.

The timeout failure's reason type is archived: adding a field to it changes its
archived layout and forces a versioned-envelope bump.

## Decision

**The Service stream's synthesised timeout carries the latest attempt-ending
lifecycle event the stream observed, as optional, labelled detail. It is not the
verdict: the restart authority's give-up stays the only authoritative failure
reason.**

- The detail is optional and present only on the synthesised timeout. A stream that
  observed no attempt-ending event reports none.
- It travels as an additive field on the stream's failure event, composed of
  existing types, so the archived reason type is unchanged.
- The render labels it as the last attempt and never as the cause of the timeout.

## Alternatives considered

- **Add a field to the timeout reason.** Changes an archived type: versioned
  envelope, fixtures and migration, for a value that exists only on a stream.
- **A new stream event variant for the last attempt.** The consumer would have to
  join two events to describe one failure, and a client that ignores the new variant
  loses the detail.
- **Read the allocation and probe rows when the timeout fires.** A second read path
  in the stream and an attempt-selection rule, for information the stream already
  holds in memory.
- **No detail.** The operator learns that the wait ended and must open
  `workload describe` to learn why.

## Consequences

- Positive: the timeout tells the operator what the last attempt did, without
  claiming the workload failed.
- Positive: no archived, persisted or intent change; the wire field is additive and
  a client without it decodes the event unchanged.
- Negative: new public surface (one optional stream-wire field and one wire struct);
  the exact list is in the feature delta.
- Negative: the detail reflects only events this stream received after it opened (a
  lagged subscriber misses events), so it is the latest attempt this stream saw, not
  the latest attempt the workload had.
