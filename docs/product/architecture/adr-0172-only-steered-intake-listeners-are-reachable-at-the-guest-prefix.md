# ADR-0172 — Only steered intake listeners are reachable at the guest prefix; constant rules refuse every other guest-prefix packet

## Status

**Proposed — decision D8a approved by user 2026-10-06; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*. The lookup mechanism is
ADR-0171; the route that makes the prefix local is ADR-0152.

Amends on acceptance: ADR-0125 (rules 5 and 8 match the constant guest prefix
instead of `managed_guest_ips`; `managed_guest_ips` and its element lifecycle
removed; an output reject and a prerouting drop of guest-prefix traffic that
is neither exempt nor diverted added).

## Context

ADR-0152 makes the guest prefix host-local. The host kernel delivers a
connection to a local address to any socket that matches it — including a
wildcard-bound host service on `0.0.0.0:port` (for example `sshd` on 22, or
the operator API). Without a decision of its own, a connection to
`workload_addr:port` while no intake listener serves it would reach that host
service:

- a remote mesh client, through leg-C and leg-S (leg-S's mark exempts it from
  the intercept rules), when the VM declares a port that a host service also
  uses and the guest application is not listening;
- a marked TCP probe, which would report healthy while the guest is down;
- any remote client to a guest-prefix address that is not, or no longer, a
  managed guest address.

Under the bridge topology the guest kernel answered; the host never did.

## Decision

- **Reachability.** No connection to a guest-prefix address reaches a host
  socket other than a steered intake listener of that `(address, port)`
  (ADR-0171, ADR-0173) or leg-C. Host-local TCP to an address and port with
  no steered listener is refused at once; every other packet to the prefix is
  dropped or rejected.
- **Constant rules keep each client class on its path.** In output and
  prerouting, every packet to the guest prefix that is neither accepted by the
  leg-S exemption nor diverted to leg-C (`inbound_destinations`) is refused:
  the output rule rejects (TCP reset, ICMP port-unreachable otherwise), so a
  host-local client sees a refusal at once; the prerouting rule drops, as
  today for remote traffic. So an unmarked host-local client reaches an
  intake only through leg-C.
- **The gateway is excluded** from the guest-prefix match of the lookup
  decision and of the rules; it is the DNS address (ADR-0154).
- **No per-allocation membership.** These replace the per-allocation
  `managed_guest_ips` set. ADR-0125's rules 1, 4, 6 and 7 stay; rules 5 and 8
  match the constant guest prefix. (Rules 2–3 and `outbound_sources` are
  removed by ADR-0153 and ADR-0162.)

## Alternatives considered

- **Keep `managed_guest_ips` and add only a reset rule.** Unassigned and
  released guest-prefix addresses keep delivering to wildcard host services
  for remote clients. Rejected.
- **One node-shared transparent intake that recovers the original
  destination.** Changes the meaning of ADR-0125's rules 1 and 6. Rejected.
- **A per-allocation host netdevice per guest address.** Reintroduces per-VM
  netdevices. Rejected (ADR-0145).
- **Decide reachability by a firewall set of admitted `(address, port)`
  elements.** The element and the listener are two kernel objects; ADR-0171
  records why that loses. Rejected.

## Consequences

- A connection to `workload_addr:port` reaches a steered intake listener,
  leg-C, or a refusal — never a host service. A TCP probe fails while no
  intake listener is steered for its port.
- Two constant rules are added; `managed_guest_ips` and its element lifecycle
  are deleted.
- A shared firewall table left partial by a failure of the mTLS worker does
  not open the prefix to host services: the lookup decision (ADR-0171) does
  not depend on the table. A partial table affects only the existing mTLS
  interception rules, under ADR-0124's recovery (ADR-0169).
- Validated by V-19 and V-26 (feature delta); checked by the formal model of
  the guest-flow owner (ADR-0168, modules `intake` and `shared_table`).
