# ADR-0208 — Guest flows are bounded by a node pair capacity and a per-allocation quota

## Status

**Accepted.** The flow bounds of decision D3, approved by the user
2026-10-05. GH #295. Not yet implemented. The constants and the typed
refusal are in the #295 feature delta, § *[REF] Core vocabulary* and
§ *[REF] Owner and provisioner*.

Related decisions, each separately reversible: one forwarded pair per flow
(ADR-0147, the rest of D3); when the host releases a flow's places
(ADR-0209); that a half-closed flow keeps its places until its pair ends
(ADR-0183, R2-DRAIN); the VM probe dial's exemption from the quota and its
own cap (ADR-0187).

## Context

Each transport flow is its own forwarded pair (ADR-0147), so host sockets,
forwarder pair state and kernel memory grow with the number of concurrent
flows, not with the number of VMs. Every VM on a node draws on the same
host: one allocation that opens flows without bound would exhaust the node's
descriptors and memory for every other allocation.

The density target is 16,384 attachments per node (ADR-0155). The bytes a
flow holds are already bounded by its own sockets' buffers, because the
forwarder consumes from a source only what its target accepted (ADR-0197);
what remains unbounded without a limit is the number of flows.

## Decision

- **Two bounds.** Each node bounds concurrent guest flows twice: a global
  pair capacity sized from the attachment target, `GUEST_FLOW_MAX_PAIRS =
  65,536` (four flows per attachment at 16,384), and a per-allocation quota
  of 4,096 flows.
- **A flow holds one place in each bound from admission until the host
  releases it** (ADR-0209), whatever its state: pending pairing, open, or
  half-closed and not yet ended. Dialled flows count like any other flow;
  the one exception is a VM probe dial, which takes a place in the node
  capacity only (ADR-0187).
- **Exhausting either bound refuses that flow** with a typed capacity
  refusal. It never fails the allocation.

## Alternatives considered

- **No per-allocation quota; the node capacity alone.** One allocation could
  hold every node place and refuse every other allocation's flows. Rejected.
- **Fair-share flow accounting** (each allocation's share recomputed from
  the allocations present). A noisy-neighbour policy beyond what the bulkhead
  needs, with a moving limit an application cannot plan against. Not part of
  this feature. Rejected.
- **A byte budget instead of a flow count.** The bytes of each flow are
  already bounded by its sockets' buffers (ADR-0197); a byte budget would add
  per-flow accounting in the forwarder for a quantity the kernel already
  bounds. Rejected.

## Consequences

- **The quota is a bulkhead, not a fair share.** `capacity / quota`
  allocations can exhaust the global pool.
- **The host descriptor budget is checked at startup** against the pinned
  pair capacity (feature delta, § *[REF] Owner and provisioner*).
- **A slow reader holds quota.** A guest that withholds credit or an
  application that does not read holds at most its allocation's quota of
  node flow capacity — what it can already hold with open flows
  (ADR-0183).
- The bounds are the host's and do not depend on which guest holds the other
  end (ADR-0207; a guest that runs no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321)).
- Real per-node capacity is
  [GH #299](https://github.com/overdrive-sh/overdrive/issues/299); V-9
  measures the sizing.
