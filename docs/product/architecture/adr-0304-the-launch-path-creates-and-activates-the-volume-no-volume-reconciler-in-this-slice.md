# ADR-0304 — The launch path creates and activates the volume; there is no volume reconciler in this slice

## Status

**Proposed** (decision VD-9, GH #348). Not accepted until the user approves it.

## Context

`.claude/rules/reconcilers.md` requires creation to converge (observe, diff, converge) rather
than apply once. Creation here is one fast operation (reserving extents; nothing is written),
and every volume has exactly one launching allocation at a time (`replicas = 1`, a held claim).
The things a separate volume reconciler, status row and start gate would add (erasing and
removing volumes, a visible volume state, an early volume verdict) all hang on workload
withdrawal, which does not exist yet (GH #211). `CLAUDE.md` § "Build vertical slices" says a
feature that cannot be driven through `serve` + `deploy` is reduced until what remains can.

## Decision

**Before each launch of an allocation that declares a volume, the launch path calls an
idempotent `ensure`: observe whether the volume exists, create it if it does not, and activate
it. A failure (no volume store, no free space, a command failure) is a typed start failure
recorded in the allocation's `Failed` row and charged to the restart budget. This slice adds no
volume reconciler, no volume observation row and no gate on `WorkloadLifecycle`; reclamation,
erasure, a volume status row and a start gate belong to the workload-deletion slice (GH #211).**

## Alternatives considered

- **A host-backed `volume-lifecycle` reconciler, a `VolumeStatus` row and a start gate.**
  Rejected for this slice: its reachable work (create) is already convergent at the point of
  use, and the rest (reclaim) has no production trigger, so it would be a mechanism no entry
  point can reach. Kept as the shape for the #211 slice.
- **Create at workload admission.** Rejected: the node that will hold the volume is not chosen
  yet (placement runs later), and creation would sit in the submit handler.
- **Create in the shim before provisioning the network.** Rejected: the shim's order is fixed by
  ADR-0106 and the launch already owns the VM-specific steps.

## Consequences

- Positive: the whole create-attach-restart loop is driven by `deploy`; far less surface (no new
  observation family, port or interest); a crash between create and attach is repaired by the next
  launch's `ensure`.
- Negative: a volume that can never be created (no space) charges the restart budget and ends as
  `BackoffExhausted` with the typed volume cause as the last row's reason; `describe` shows the
  declared size, and a volume's own state is visible only through allocation rows.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Entry contract for the workload-deletion slice
- [ADR-0297](adr-0297-a-volume-belongs-to-its-workload-and-outlives-stop-and-restart.md), [ADR-0303](adr-0303-a-replacement-waits-for-the-volume-claim-not-for-the-vmm-process.md), GH #211
