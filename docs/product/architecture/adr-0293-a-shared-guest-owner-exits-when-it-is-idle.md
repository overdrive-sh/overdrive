# ADR-0293 — A shared-guest owner exits when it is idle

## Status

**Accepted — approved by the user, 2026-11-14 (DDD-17, user decision N-2 in the
feature delta).**

## Context

A shared guest otherwise lives until the end of the Metal run, so every module
that ran keeps its VMM for the rest of the command. A `cgroup-default` guest is
configured with 8 vCPU and 4,096 MiB and `nested-cohort` with 12,821 MiB; these
are the guest's configured limits, not its measured consumption. The run's
transient unit (ADR-0281) ends leftovers only when the whole command ends. The number of declared modules is
unmeasured and no budget or cap exists.

## Decision

The owner exits when it has had nothing queued, nothing running and no attached
client for a fixed idle period (`SHARED_GUEST_IDLE`, a single constant in the
shared-guest client module, 30 s). `OwnerCore` decides: it answers the idle event with an
exit effect only when no test is queued and no exec is in flight (a detached
exec counts as in flight). On exit the owner unlinks its socket path before it
closes its listener, so a successor that binds after the close never has its
socket removed. The owner's guest dies with it (ADR-0290). The
owner holds no handle on the nextest process. A client
whose connection ends before the owner acknowledged its request has run nothing
and re-attaches once through the election.

## Alternatives

- **No timer** (exit as soon as the queue is empty and no client is attached):
  the tests of a module are separate processes, so the owner would exit between
  every two of them and the module would boot a guest per test; rejected.
- **No idle exit and a stated maximum**: no measured maximum exists; rejected.
- **An anchor on the nextest process** (a pidfd on the wrapper's parent, opened
  before the PID namespace exists, because a pid resolves in the caller's
  namespace): ends owners when nextest exits but not during a long nextest, and
  needs a wrapper-exported variable and an ordering constraint; rejected.
- **A cap on live shared guests**: a new admission mechanism with its own queue;
  rejected.

## Consequences

The period changes only the boot count, never a result. Closing a listening unix
socket resets every connection still queued for `accept`
(`net/unix/af_unix.c:721-723`, `:708-709` @ v7.0), so an attach racing the exit
can be cut before acknowledgement; the one-time re-attach covers it, and a
deterministic client-side test proves it (a listener that accepts and closes
before the acknowledgement: one re-attach through the election, a second cut
is a `GuestExecution` failure at stage `owner`).

## Links

- [Decision DDD-17](../../feature/metal-test-placement-attribute/feature-delta.md#ddd-17--a-shared-owner-exits-when-it-is-idle-approved-user-decision-n-2-2026-11-14)
- [Formal protocol model](../../../specs/quint/shared-guest-owner/README.md)
- [ADR-0283](adr-0283-the-tests-of-a-module-share-one-guest-through-a-per-module-owner-process.md)
- [ADR-0290](adr-0290-the-shared-guest-owner-is-init-of-its-own-pid-namespace-so-its-guest-dies-with-it.md)
