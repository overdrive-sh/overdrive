# ADR-0311 — A volume belongs to its workload and outlives stop and restart

## Status

**Accepted** (decision VD-2, GH #348). Approved by the user on 2026-11-17.

## Context

Allocation ids are minted per attempt (`alloc-<workload>-<n>`, ADR-0104) and every
per-launch VM artifact is keyed on them, so anything keyed on an allocation dies
with the allocation. #348 requires the replacement allocation to attach the same
volume, and asks whether a stopped workload keeps it and where desired state lives.

## Decision

**A volume is identified by its workload's `WorkloadId` (one per workload). Its
desired state is derived from the workload intent: there is no separate volume intent
record and no volume verb. The volume is created when the workload's first allocation
launches (ADR-0318) and survives operator stop, crash restart, `workload restart` and
re-placement on the same node. Withdrawing the workload intent is the only thing that
deletes it.**

## Alternatives considered

- **A first-class `volumes/<id>` intent record with its own create/delete verbs.**
  Rejected: a second owner and a second delete path for something #348 defines as
  owned by the workload.
- **Key the volume on the allocation.** Rejected: the replacement allocation could
  not find it (ADR-0104).
- **Delete the volume on stop.** Rejected: a stopped development box would lose its
  repository checkouts and caches.

## Consequences

- Positive: no new intent surface; the workload's own lifecycle is the volume's.
- Negative: workload withdrawal does not exist yet (GH #211), so nothing deletes a
  volume yet. The reclaim-on-withdrawal mechanism is not part of this slice; the
  contract that slice must meet is in the feature delta § "Entry contract for the
  workload-deletion slice".

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Answers to the issue's DESIGN questions
- [ADR-0104](adr-0104-vm-recreation-fresh-allocation-identity.md), [ADR-0318](adr-0318-the-launch-path-creates-and-activates-the-volume-no-volume-reconciler-in-this-slice.md), GH #211
