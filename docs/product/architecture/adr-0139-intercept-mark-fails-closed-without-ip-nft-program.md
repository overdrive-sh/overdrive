# ADR-0139 — Intercept-marked TCP fails closed independently of the IP nft program

## Status

**Accepted (2026-09-24), conditional on reproduction (below).** GH #295
correctness-recovery replacement DESIGN, decision D-295-R18. Proposed
2026-09-23 and revised 2026-09-24; reviewed by independent DESIGN review rounds
3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. The mechanism is chosen on evidence (below) from
four candidates.

The decision is conditional on reproduction. The hazard is reasoned from
production source and kernel semantics. DISTILL must reproduce it on native
metal (RED) before DELIVER relies on this decision. If native evidence shows
that intercept-marked TCP is already dropped with the IP program absent, this
decision is withdrawn. Exact names, priorities, and values live only in the #295
feature delta.

## Context

The accepted #295 security promise is that healthy operation, and any single
owned-component loss, catch or drop guest TCP. The pieces involved are:

- The TCX classifier marks validated guest TCP with the intercept mark and
  delivers it to the host IP stack.
- Only the owned IP nft program sets the policy-route mark that selects the
  local-delivery table, and only it assigns the transparent socket.

It follows from source that deleting the owned IP table while the bridge guard
survives leaves intercept-marked TCP to ordinary routing. Two exposures follow:

- **Host-local delivery.** When the destination is any host address, such as
  the bridge gateway, the kernel's priority-0 `lookup local` rule matches before
  any added rule, and normal socket lookup delivers the packet to a listener
  bound to the wildcard address. This path does not depend on IPv4 forwarding.
- **Forwarding.** When the destination is a peer guest address and host IPv4
  forwarding is enabled, the host routes the packet back over the bridge to the
  peer TAP in cleartext.

No Overdrive component owns host `net.ipv4.ip_forward` after #295. The
per-workload netns path that set it (`veth_provisioner.rs:1436-1440`, its
`EnableIpForward` step at `:1289` executed at `:2850`) has no production
caller, and the Service load-balancer veth path (`:1572`, `:1755`, called at
`lib.rs:3528`) sets none. Its value is a host setting that #295 neither writes
nor depends on, so the fail-closure must hold whichever value it has.

The recovery proofs used exactly this fault (deleting `ip overdrive-mtls`), but
they measured fail-stop timing, not leakage.

While the IP program is present, no packet leaves its prerouting chain still
carrying the intercept mark on the healthy path: the chain either replaces the
mark as it assigns the transparent socket, or drops the packet. When a
transparent listener is absent, the TPROXY-before-mark rule order of ADR-0140
(D-295-R19) keeps that true for outbound TCP.

The leak class is real in prior art: Istio reports ambient pods whose
redirection rules were lost sending traffic that bypasses the proxy, caught only
by an independent NetworkPolicy, and Istio recommends such independent layers as
defence in depth. The kernel's policy-rule order and nftables' verdict
semantics support the chosen mechanism; the exact guard-table-on-classifier-mark
mechanism has no external precedent. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 9.1–9.5.)

## Decision

The intercept owner also owns one small, separate nft table. It holds one
prerouting chain at filter priority, which runs after the mangle-priority
intercept chain, and one rule that drops TCP still carrying the intercept mark.
The guard is:

- converged and read back with the constant program;
- audited as part of the `IpRules` component;
- repaired through the same owner.

In healthy operation the guard matches nothing. With the intercept table
absent, intercept-marked TCP is dropped at prerouting, before the routing
decision, so it can be neither forwarded nor delivered locally. Intercept-marked
TCP then has two independent controls, the intercept program and this guard, and
losing either one alone stays fail-closed.

## Alternatives considered

### A blackhole FIB policy rule on the intercept mark

Rejected. `ip-rule(8)` scans rules in priority order, and the kernel's rule that
looks up the local table sits at priority 0. Host-local destinations are
therefore delivered before any added rule is reached. It would drop forwarded
packets but not host-local delivery.

### Move the local-table rule behind a blackhole FIB rule

Rejected. Its premise, that the priority-0 rule can be moved or deleted safely,
is unverified in the fetched primary sources, and moving it changes routing
policy for every host packet. It would also need its own boot, recovery, and
ownership decision.

### Drop intercept-marked frames on bridge output

Rejected. It changes the bridge guard contract and still leaves host-local
delivery open.

### Route intercept-marked packets to the local-delivery table

Rejected. Without the IP program no transparent socket is assigned, so a marked
guest connection would reach host wildcard listeners.

### Take ownership of host `ip_forward` and set it to 0

Rejected. It closes only the forwarding path, not host-local delivery, and it
would make #295 the writer of a node-global kernel setting that no #295 flow
needs and that other host software may depend on.

### Accept a bounded exposure as a documented exception

Rejected. It would weaken an accepted security outcome, which the recovery
charter forbids.

## Consequences

Positive: single-loss fail-closure holds for the IP program, for both
forwarding and host-local delivery, as it already does for the classifier and
the guard.

Negative: the intercept owner gains one more owned table, chain, and rule. That
raises ADR-0125's constant IP rule count to nine, and the audit, repair, and
fresh-boot convergence must all cover it. Every host packet traverses one more
prerouting chain. The guard does not cover a present program whose TPROXY target
listener is absent; the TPROXY-before-mark order of ADR-0140 (D-295-R19)
covers that path. Correctness rests on native evidence, because no simulation can observe
kernel routing.
