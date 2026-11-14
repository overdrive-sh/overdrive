# ADR-0292 — Every shared exec has a bound

## Status

**Accepted — approved by the user, 2026-11-14 (DDD-16, user decision N-1 in
the feature delta).**

## Context

The only existing attempt bound is 86,400 s on the per-test path; nextest owns
the real timeout. When nextest kills a test process at its slow-timeout, an
isolated guest dies with that process, but a shared guest outlives it and the
owner never cancels a running test (ADR-0284). A hung in-guest test would hold
the shared slot for up to 24 h, and every queued test of the declaration would
wait out its own nextest timeout.
`.claude/rules/reconcilers.md` § "Superseded in-flight work runs to its bound"
requires a bound.

## Decision

Each shared exec runs under its profile's exec bound, which is the cell bound
the same bodies already ran under in the batched dispatcher (300 s; the nested
profiles keep their 600 s and 1,200 s). The owner arms a timer per exec. At the
bound it discards the guest (the existing cleanup path) and, if the client is
still attached, replies with `exec_bound`, which the client maps to a typed
`GuestExecution` failure with that stage. An isolated guest keeps the existing
attempt bound.

## Alternatives

- **Rely on nextest alone**: the slot is held up to 24 h; rejected.
- **Kill only the in-guest test process**: needs a guest-side kill channel, more
  mechanism than discarding the guest; rejected.
- **A per-test configurable bound**: a new surface with no reachable need;
  rejected.

## Consequences

A test that legitimately runs longer than its profile's bound cannot share a
guest and takes an explicit isolated `guest = ".."` form. One timer per exec and
one owner event; the model proves liveness with a hung exec possible and shows
the hazard without the bound.

## Links

- [Decision DDD-16](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-16--every-shared-exec-has-a-bound-approved-user-decision-n-1-2026-11-14)
- [Formal protocol model](../../../specs/quint/shared-guest-owner/README.md)
- [ADR-0284](adr-0284-a-shared-guest-serves-one-test-at-a-time-and-never-cancels-a-running-test.md)
- [ADR-0291](adr-0291-a-shared-guest-is-discarded-after-a-failed-lost-bounded-or-detached-test.md)
