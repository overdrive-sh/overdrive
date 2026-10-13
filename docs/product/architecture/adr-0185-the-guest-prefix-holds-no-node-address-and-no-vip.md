# ADR-0185 — The guest prefix holds no node address and no VIP

## Status

**Accepted — D8-PREFIX-CFG and D8-PREFIX-VIP approved by user 2026-10-08.**
GH #295. Recorded in the #295 feature delta,
`docs/feature/netns-density-295/feature-delta.md` § *[REF] vsock Attachment
Replacement DESIGN* (decision index; § *[REF] Owner and provisioner —
`overdrive-control-plane::guest_network`* carries the typed refusals).

Related decisions: how host platform clients reach a VM port without the
prefix being host-local (ADR-0173), the gateway route (ADR-0152),
and the constant rules that refuse guest-prefix traffic not diverted to
leg-C (ADR-0172).

## Context

Every VM's `workload_addr` is drawn from the guest prefix. It stays the
persisted, observed backend and probe address, and it is never a local
address of the host (ADR-0173). An address of the prefix that the host also
holds would be both a node address and a possible `workload_addr`.

Two producers could put a node address inside the prefix:

- **A node interface address.** Assigning an address to an interface makes
  the kernel add a `local` route for it
  (`net/ipv4/fib_frontend.c:1139-1156 @ v7.0`). An address configured inside
  the prefix — the node's network configuration, which the operator owns —
  would make that address host-local and a possible `workload_addr`.
- **The configured service VIP range.** `serve`'s own boot assigns gateway
  addresses from it and routes it on-link (ADR-0061), and the mesh resolver
  treats every address in it as a VIP (ADR-0164). A VIP range overlapping the
  prefix would make a workload address an unknown VIP and a gateway address a
  node address inside the prefix.

On the appliance nothing else writes routes or addresses (ADR-0068).

## Decision

- **Refuse before any boot effect.** Startup refuses with a typed
  configuration error when an address configured on a node interface lies
  inside the guest prefix, naming the address and the interface, and when any
  range of the configured service VIP range overlaps the guest prefix, naming
  the range.
- **One check covers `serve`'s own addresses.** Every address `serve`'s own
  boot later assigns in the root namespace is drawn from the VIP range, so the
  second refusal checks it.
- **The checks read configuration, not routes.** A route made by boot itself
  (the gateway route of ADR-0152) is not an
  interface address and is not seen by these checks.

## Alternatives considered

- **Observe the routes covering the prefix and refuse an untagged one.** The
  only producer of an untagged route inside the prefix on the appliance is a
  node address configured inside it; checking the configuration names the
  cause and the fix directly, before any effect. Rejected.
- **Allow an overlap and exclude the overlapping addresses from leasing.**
  Leaves the mesh resolver classifying those addresses as VIPs and a node
  address inside the prefix the constant rules (ADR-0172) refuse. Rejected.

## Consequences

- A node whose network configuration places an address inside the guest
  prefix does not start; the refusal names the address and interface.
- A node configured with a VIP range overlapping the prefix does not start;
  the refusal names the range.
- Observation rows, backend identity and probe targets are unchanged.
