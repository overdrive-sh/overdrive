# ADR-0255: The component that observes an allocation's end fires the terminal hook

## Status

Accepted, 2026-11-02 (approval: feature delta § Approval record, D1, D2, D4 and
BC-1). GH #318, GH #334. Not implemented. Exact interface text, ordering,
lifecycle-gate ownership, the formal protocol model and the DISTILL obligations
live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).
Related: ADR-0054 § 2, ADR-0083, ADR-0087, ADR-0100, ADR-0105. Companion
decisions: [ADR-0256](adr-0256-the-shim-registers-probe-supervision-before-it-releases-the-exit-event-gate.md)
(registration order) and
[ADR-0257](adr-0257-service-lifecycle-announces-a-startup-failure-only-for-a-live-allocation.md)
(startup-failure guard).

**Why one ADR.** The decision is "who fires the terminal hook for each way an
allocation ends". Its three clauses are not adoptable apart: the observer call
(D1) contradicts the old hook contract unless the contract is rewritten to allow
plural authors and a row-independent hook (D2); and the assignment is only
complete with the statement of which existing calls stay and which path gets none
(D4). D3, which can be adopted or reversed alone, is ADR-0256.

## Context

`Driver::on_alloc_terminal` is the only mechanism that cancels an allocation's
probe supervisor. Its contract says the action shim fires it after the shim
writes a terminal row, and only the shim's `StopAllocation` and `FinalizeFailed`
arms do. A natural guest exit is ended by neither: the exit observer writes the
`Failed` or `Terminated` row and has no hook. Two consequences follow, both
reachable under a real `serve` and `deploy`:

- a crashed allocation keeps probing a dead guest for the restart backoff, or
  for the process lifetime when no restart comes (#334);
- every crash-restart leaks the predecessor's supervisor, because the
  successor has a fresh allocation id (#318, ADR-0105).

The root cause is that the "release per-allocation resources" step is a sequence
copied into each ending path, with no statement of who owns it for the path the
shim does not run. The relevant forces: the observer already holds the exact
driver through `Weak<dyn Driver>`; `VmDriver::stop` produces no `ExitEvent`, so
the shim's stop path and the observer's exit path are disjoint; the hook is
infallible, in-memory and idempotent.

## Decision

**Whoever observes or authors an allocation's end fires the terminal hook for
that allocation on the driver that owns it. For a natural exit that is the exit
observer, which fires it once per consumed exit event, independently of whether
a row write succeeds.** The shim keeps firing it for the endings it authors. A
restart's cleanup of its predecessor needs no hook call of its own.

Scope: this decision covers who owns the hook. It does not decide the order of
`on_alloc_running` relative to the exit-event gate
([ADR-0256](adr-0256-the-shim-registers-probe-supervision-before-it-releases-the-exit-event-gate.md)),
what `ServiceLifecycle` does with a terminal allocation
([ADR-0257](adr-0257-service-lifecycle-announces-a-startup-failure-only-for-a-live-allocation.md)),
and it does not change any lifecycle state meaning.

## Alternatives considered

- **ProbeRunner observes terminal rows and stops itself (B).** Covers every row
  author, including future ones, but needs a List-then-Watch with a relist
  backstop, misses endings where no row lands (write exhaustion, no prior row),
  and leaves start hook-driven. More machinery and less coverage.
- **A shared shim-side release helper (C).** Cannot reach the observer, so a
  crash with no restart keeps probing; at restart time it is the RCA's minimal
  fix and fixes #318 only.
- **Row-driven hook dispatch in the control plane (D).** Same costs as B plus a
  lookup through the alloc-to-driver index, which falls back to every driver on a
  miss; the observer already holds the exact driver.
- **The VM driver stops the supervisor where it emits the exit event (E).** Covers
  natural exit only, leaves `FinalizeFailed` on a live VM with the shim, and
  threads a probe-runner handle into the exit watcher.
- **Fire the hook only when the observer authors a row.** The literal reading of
  "the row author owns the hook". Rejected: `NoWrite` and exhausted-retry exits
  write no row yet end the guest.

## Consequences

- Positive: #318 and #334 close at the one component that knows the guest died.
  The supervised-allocation count tracks live allocations. No new component,
  port, type or error. The hook contract becomes row-independent and idempotent.
- Positive, and an approved behaviour change: a crashed service allocation's
  startup-probe failure streak no longer grows by probing a dead guest. The
  crash row is the recorded cause and `WorkloadLifecycle`, the single restart
  authority (ADR-0087), decides restart or give-up. The user approved this on
  2026-11-02. A streak that already reached its limit before the crash is a
  separate edge, decided by ADR-0257.
- Negative: ownership stays distributed over a small, enumerated set of authors
  (observer, shim stop, shim finalize). A future ending path that omits the hook
  repeats the defect. The feature delta's per-author scenarios and the formal
  model are the enforcement; no lint is proposed (feature delta P-1).
- Negative: correctness relies on registration preceding exit-event emission,
  which ADR-0256 decides; without it a fast exit can leave an orphan.
- Neutral: operators observe no new API; a crashed service allocation's probe
  rows stop growing at the crash.
