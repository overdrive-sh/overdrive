# ADR-0121 — Enforce a private fixed 16,384 guest-network admission cap

## Status

**Accepted — user-approved and approved by system design review iteration 5 on 2026-09-16.**
GH #295 DESIGN stage 1. This records F1; ADR-0122 owns the exact typed
infrastructure error carrier.

## Context

ADR-0117 sets 16,384 guest network attachments as the initial measured contract.
Deleting `NetSlot` also deletes its incidental 4,096-entry admission ceiling and
terminal exhaustion disposition. #295 needs a bounded appliance admission rule
without inventing a public heterogeneous network resource before that separate
problem is designed.

## Decision

Enforce a private fixed cap of 16,384 active allocations per node in the
existing placement decision, before any start action or guest-address
assignment. A node at the cap returns the existing `NoCapacity` outcome. The
cap adds no operator field, advertised resource, wire field, or persisted
resource.

The `/16` address pool retains headroom for predecessor/successor overlap and
cleanup residue. Address-pool exhaustion below the admitted cap is
infrastructure drift: refuse the effect through a typed non-terminal
infrastructure error and degraded-health event. It does not author an
allocation `Failed` row or consume restart budget.

## Alternatives considered

### Public heterogeneous `network_ports` resource

Rejected and out of #295 scope. It would model different per-node network
capacities, but requires operator, advertised node-capacity, scheduler, wire,
and persistence changes. That independent design is tracked by
[GH #299](https://github.com/overdrive-sh/overdrive/issues/299); #295 must not
add any part of that surface.

### Treat pool exhaustion as allocation failure

Rejected. Capacity admission precedes allocation start, and below-cap pool
exhaustion is infrastructure drift rather than a permanent workload failure.
Charging restart budget would preserve the obsolete `NetSlotExhausted`
disposition under a different name.

## Consequences

Positive: #295 has a deterministic, private appliance bound aligned with its
measured T1 contract and reuses the existing `NoCapacity` outcome. Negative:
all nodes use the same bound even if their hardware differs; #295 deliberately
does not model that heterogeneity. The exact implementation-facing constant and
failure projection remain in the feature delta.
