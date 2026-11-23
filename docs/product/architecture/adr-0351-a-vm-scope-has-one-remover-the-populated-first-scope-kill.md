# ADR-0351 — A VM scope has one remover: the populated-first scope kill

## Status

**Accepted** 2026-11-23 (decision RB-5 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-23, N-1, option a; the deletion
of the unused bare removal follows from it by the user's instruction of the same day). Not implemented. The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "A VM scope has one remover".

## Context

The guest-address lease is released only when its allocation's scope is absent (ADR-0156, second amendment), and absence proves an empty scope only
if every removal of a VM scope first waited for `populated 0` ([ADR-0349](adr-0349-a-scope-kill-waits-a-bounded-time-for-the-scope-to-settle-and-reports-a-scope-that-is-busy.md)).
The VM driver removes a VM scope itself at a stop and at a start-failure cleanup, with `cgroup.kill` followed by a bare `rmdir` that does not wait.
On the running kernel that `rmdir` removes a scope under a live, stuck task, so the driver's removal would break the gate.

## Decision

**`VmHostState::kill_scope` is the only remover of a VM scope.** The VM driver's cleanup, at a stop and at a start failure, removes the scope
through it and through nothing else. A `ResourceBusy` from it leaves the cleanup's other steps and the stop's outcome unchanged: the scope is left
to `VmReclamation`, whose next pass calls `kill_scope` again. The bare removal method on the driver's cgroup manager has no remaining caller and is
deleted with its tests, and a structural test fails if any other code removes a cgroup directory.

## Alternatives considered

- **Make the cgroup manager's removal populated-first as well, leaving the driver's removal in place.** Rejected by the user: two implementations of
  one discipline; a drift in either breaks the lease gate.
- **The driver stops removing VM scopes and `VmReclamation` is the only remover.** Rejected by the user: every ordinary stop would keep its lease
  slot until the 30 s pass.
- **Keep the bare removal for exec scopes.** Rejected: it has no exec-scope caller; an unused method is deleted, not kept.

## Consequences

- A reaped stop's scope is empty, so the driver's `kill_scope` call returns within milliseconds; a scope that still holds a live task pays the bound
  once and returns busy, and the stop completes.
- After a `KilledNotReaped` stop the driver does not call `kill_scope` (the scope is known to be populated; ADR-0348).
- `VmDriver` takes the `VmHostState` port as a required constructor parameter.
- The appliance has no foreign remover (CLAUDE.md), so the single-remover property is enforced in Overdrive's own code by the structural test.
