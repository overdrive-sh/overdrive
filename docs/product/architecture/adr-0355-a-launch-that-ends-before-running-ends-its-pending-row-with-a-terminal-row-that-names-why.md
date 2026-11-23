# ADR-0355 — A launch that ends before running ends its `Pending` row with a terminal row that names why

## Status

**Accepted** 2026-11-23 (decision RB-8 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-23, N-4, option b). Not
implemented. Amends ADR-0334 (implemented through its driver side; the amendment is explicit there). The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "Launches that end before running".

## Context

A VM launch writes a `Pending` row before the VMM exists ([ADR-0353](adr-0353-a-vms-guest-address-is-recorded-on-its-pending-row-before-the-vmm-exists.md)).
A launch can end before it runs: superseded by a stop, a restart or node shutdown (ADR-0334), lost with the control plane, or ended by an error.
With no row written for it, ADR-0334's shim wrote nothing and ADR-0335's handled-generation rule read "has a row"; with the `Pending` row, a launch
that ends must not leave that row behind, because nothing is completing it and an operator cannot tell it from a live launch.

## Decision

**Every launch that ends before running ends its `Pending` row with a `Terminated` row, `terminal` absent, whose reason names why, and boot closes
the `Pending` rows a crash left.** Two reasons name the causes: the launch was superseded (by a stop, a restart or node shutdown), or it was lost
with the control plane. An abandoned launch charges nothing: it is classified as a platform reclamation, so it is not a natural exit, is exempt from
the restart ceiling, and its relaunch carries the abandoned allocation's own count. `WorkloadLifecycle`'s choice of the current allocation is
unchanged; the abandoned row is a restartable row it relaunches under the existing path. After a stop nothing is relaunched.

The row is never `Failed`: an abandoned launch is not a start failure, and `Failed` is charged and finalised by the failure rules (ADR-0337).
`terminal` is absent for the reason the platform-reclaimed row has none: the workload's intent still stands.

ADR-0334 is amended: the shim's `StartSuperseded` arm, which wrote no row, writes the abandoned row (the driver side of ADR-0334 is unchanged). The
charge is stated in ADR-0335, item 4.

## Alternatives considered

- **Skip `Pending` rows when choosing the current allocation.** Rejected by the user (option a): a stale `Pending` row per abandoned launch stays in
  the allocation list, and the operator cannot tell a live launch from an abandoned one.
- **Relaunch a `Pending` row after a timeout.** Rejected by the user (option c): a timer for a fact the convergence lane already gives.
- **Write the abandoned row as `Failed`.** Rejected: it would charge the restart budget for a launch no fault ended and finalise a Job that never
  ran.

## Consequences

- `TransitionReason` gains two fieldless variants appended at the tail; no envelope version changes (archived size unchanged).
- The relaunch of an abandoned crash-restart successor is charged once, not twice.
- A boot step after the executors closes every `Pending` row still `Pending`; a `Pending` row whose VM left a scope is closed by the executor's
  ordinary platform-reclaimed row.
- The `restart-generation` and `node-local-volume` models read the abandoned launch as a terminal, uncharged row instead of "no row" (feature delta,
  § "Quint changes").
