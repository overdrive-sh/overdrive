# ADR-0354 — A busy boot scope writes its terminal row

## Status

**Accepted** 2026-11-23 (decision RB-7 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-23, N-3, option a). Not
implemented. The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "A busy boot scope writes its terminal row".

## Context

Boot reclamation kills each leftover VM's scope and, for a settled scope, writes the allocation's terminal row. A scope that stays populated at the
bound (ADR-0349) holds a VMM that is killed but not reaped. ADR-0349's rule for a busy scope is that the caller writes no row and takes no further
step, which is right for the steady-state reclamation tick (its row is already terminal when a scope is stuck after a stop). At boot the row may
still read `Running`.

## Decision

**When boot reclamation's `kill_scope` returns busy, the boot executor step writes the allocation's terminal row, carrying the address, and leaves
the artifact discard to `VmReclamation`.** The kill is delivered, so the VMM can run no guest code (ADR-0348) and the row is true; the stuck VM's
workload becomes eligible for restart at once, its replacement waiting on the volume claim (ADR-0317). This is the one caller exempt from "busy means
no row write"; the steady-state tick still writes none.

## Alternatives considered

- **Leave the row `Running` until the scope empties.** Rejected by the user: the row would say a VM that can run no guest code is running, and its
  workload would not restart until the reap.

## Consequences

- After `converge` returns on a busy scope the row is terminal and carries the address; after a tick meets the same busy scope the row is unchanged.
- The scope, the run directory and the clone stay until `VmReclamation` removes them after the reap.
- How the two callers of the reclaim executor differ is internal structure.
