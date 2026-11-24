# ADR-0355 — A launch that ends before running ends its `Pending` row with a terminal row that names why

## Status

**Accepted** 2026-11-23 (decision RB-8 of the node-local persistent volume feature, GH #348; user rulings of 2026-11-23: N-4, option b; RB-11, one
reason for a launch lost with the control plane; 2026-11-24, RB-13, a launch that never ran does not count). Not implemented. Amends ADR-0334 (implemented through its driver side; the amendment is explicit there). The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "Launches that end before running".

## Context

A VM launch writes a `Pending` row before the VMM exists ([ADR-0353](adr-0353-a-vms-guest-address-is-recorded-on-its-pending-row-before-the-vmm-exists.md)).
A launch can end before it runs: superseded by a stop, a restart or node shutdown (ADR-0334), lost with the control plane, or ended by an error.
With no row written for it, ADR-0334's shim wrote nothing and ADR-0335's handled-generation rule read "has a row"; with the `Pending` row, a launch
that ends must not leave that row behind, because nothing is completing it and an operator cannot tell it from a live launch.

## Decision

**Every launch that ends before running ends its `Pending` row with a `Terminated` row, `terminal` absent, whose reason names why, and boot closes
every `Pending` row a crash left, first.** Two reasons name the causes: the launch was superseded (by a stop, a restart or node shutdown), or it was
lost with the control plane. The boot step runs before boot reclamation observes or kills anything and ends every `Pending` row of the node with
the one reason `LaunchLostWithControlPlane`, whether or not its VM left host artifacts, so the reclamation executors meet a terminal row and never
write a second reason for the same fact. A launch that never ran does not count: the restart count of a new launch is derived from the newest
row that ran, skipping every abandoned row, so deleting abandoned rows from an allocation chain changes no later launch's count, `charged` or
`exhausted`. The count is that row's count plus one for a crash restart; that row's count for an operator restart (a generation replacement), where a
`Failed` row is forgiven (ADR-0335); and 0 for a fresh placement. One structural predicate on the row's reason, `is_launch_abandoned`, names the
abandoned rows, and every site that derives a count, a restart-pending generation or a natural exit uses it. An abandoned row is not a natural
exit either, so a Job whose launch never ran is not finalised. The abandoned rows stay in history: `WorkloadLifecycle`'s choice of the current
allocation is unchanged (the newest row, which may be abandoned, and which `describe` shows), and the abandoned row is a restartable row it
relaunches under the existing path. After a stop nothing is relaunched.

The row is never `Failed`: an abandoned launch is not a start failure, and `Failed` is charged and finalised by the failure rules (ADR-0337).
`terminal` is absent for the reason the platform-reclaimed row has none: the workload's intent still stands.

ADR-0334 is amended: the shim's `StartSuperseded` arm, which wrote no row, writes the abandoned row (the driver side of ADR-0334 is unchanged). The
charge is stated in ADR-0335, item 4.

## Alternatives considered

- **Skip `Pending` rows when choosing the current allocation.** Rejected by the user (option a): a stale `Pending` row per abandoned launch stays in
  the allocation list, and the operator cannot tell a live launch from an abandoned one.
- **Relaunch a `Pending` row after a timeout.** Rejected by the user (option c): a timer for a fact the convergence lane already gives.
- **Close the `Pending` rows after the executors, leaving a `Pending` row whose VM left a scope to the executor's platform-reclaimed row.**
  Rejected by the user: one fact, a launch lost with the control plane, would carry two reasons depending on whether the VM left artifacts.
- **Write the abandoned row as `Failed`.** Rejected: it would charge the restart budget for a launch no fault ended and finalise a Job that never
  ran.
- **Classify the abandoned row as a platform reclamation and carry its own count into the relaunch.** Rejected by the user (2026-11-24): the row is
  not `Failed`, so when an operator restart supersedes the launch of a crash restart, the operator's launch carries the crash restart's charge and
  the crash is never forgiven. Kubernetes, Nomad and OTP keep a launch that never ran out of budget accounting and keep history apart from the
  budget (`docs/research/orchestration/restart-budget-for-never-ran-launches-comprehensive-research.md`).

## Consequences

- `TransitionReason` gains two fieldless variants appended at the tail; no envelope version changes (archived size unchanged).
- A crash restart whose launch is abandoned is charged once, not twice: the relaunch's count is the crash row's count plus one, whatever number of
  abandoned rows lie between. An operator restart that supersedes such a launch forgives the crash.
- `is_platform_reclaimed` does not read the two reasons; a platform reclamation ran, and its row is a count basis like any row that ran.
- The first step of boot reclamation closes every `Pending` row; the executors then discard what a VM left behind (`DiscardStrandedArtifacts`
  authors no row) and the reservation of a stuck VM reads its address from the closed row, which carries it.
- The `restart-generation` and `node-local-volume` models read the abandoned launch as a terminal, uncharged row instead of "no row" (feature delta,
  § "Quint changes").
