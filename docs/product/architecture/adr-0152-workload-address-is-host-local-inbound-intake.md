# ADR-0152 — `workload_addr` stays the allocation's address on both sides; on the host it is made local, only bound intake listeners can be reached at it, and intake listeners open host-initiated flows that present the real client to the guest

## Status

**Proposed — decision D8 approved by user 2026-10-05; its residual
pre-activation difference D15-R1 acknowledged by user 2026-10-05; decision
D8a — only bound intake listeners are reachable at the guest prefix, and the
boot convergence of the shared route (independent DESIGN review findings B-2,
H-4) — approved by user 2026-10-06; boot keeps or adds the route only behind
attached steering (D8a-ROUTE) and the route is never removed at shutdown
(U-4), approved by user 2026-10-06, D8a-ROUTE restated for the steering
mechanism and approved by user 2026-10-07; the steering mechanism — a pinned
socket-lookup program over a listener-keyed socket map (D8a-LOOKUP,
ADR-0171) — approved by user 2026-10-07; the steering program loaded and
swapped in, with the route written only after (D8a-LOAD-SWAP, D8a-ROUTE
restated), approved by user 2026-10-08; pending independent DESIGN
review.** GH #295.
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN — PROPOSED 2026-10-05*. Whether an intake listener exists while no
guest application listens is ADR-0163. Inbound UDP service to VMs is deferred
to **#310** (user ruling D15, 2026-10-05).

Amends on acceptance: ADR-0125 (constant rules: rules 2–3 and
`outbound_sources` removed; `managed_guest_ips` replaced by the constant guest
prefix; an output reject and a prerouting drop of guest-prefix traffic that is
neither exempt nor diverted added).

## Context

`workload_addr` is the persisted, observed backend and probe address. Its
consumers are the backend discovery bridge, cgroup service delivery
(ADR-0053), probes (ADR-0094), leg-S plaintext delivery (ADR-0120) and the
intercept sets of ADR-0125. Under ADR-0145 the guest has no NIC, but the guest
still carries `workload_addr` on a dummy device (ADR-0150).

To terminate host-local connections to `workload_addr:port` on the host, the
guest prefix must be local to the host. A `local` route makes every address
of the prefix local, and the host kernel delivers a connection to a local
address to any socket that matches it — including a wildcard-bound host
service on `0.0.0.0:port` (for example `sshd` on 22, or the operator API).
Without further steering, a connection to `workload_addr:port` while no intake
listener is bound for it would reach that host service:

- a remote mesh client, through leg-C and leg-S (leg-S's mark exempts it from
  the intercept rules), when the VM declares a port that a host service also
  uses and the guest application is not listening;
- a marked TCP probe, which would report healthy while the guest is down;
- any remote client to a guest-prefix address that is not, or no longer, a
  managed guest address.

Under the bridge topology the guest kernel answered; the host never did. The
spikes bound intake listeners on the host's public address and never
exercised a `local` prefix route, so they give no evidence either way.

A `local` route is a kernel object that outlives the process that installed
it: after a SIGKILL of `overdrive serve` it is still present at the next boot.

Under the bridge topology a platform client's connection reached the guest
from the bridge gateway address. For a destination covered by a `local` route,
the kernel selects the route's preferred source if it has one, and otherwise
the destination address itself.

The guest-capture spike showed the inbound path end to end: a host intake
child installed at establishment and parked, a host-initiated vsock flow, and
a guest socket bound with `IP_TRANSPARENT` to the client address the host
intake observed. `sshd` in the guest logged the real client. 10,000/10,000
inbound flows passed with the server speaking first, and 10,000/10,000 with
the client writing immediately.

## Decision

1. **Shared local route, converged on boot, only behind attached steering.**
   The guest prefix is host-local through one shared `local` route on `lo`
   whose preferred source is the node's guest gateway address, tagged with
   Overdrive's route protocol identifier. There is no per-allocation route or
   netdevice. Every boot first loads this binary's steering program (the
   kernel verifier checks it) and swaps it into the pinned link (point 2,
   ADR-0171). Only then does it observe the routes covering the prefix and
   converge: an identical route is kept; an Overdrive-tagged route for the
   prefix with different attributes is replaced; a missing route is added; a
   route not tagged by Overdrive that overlaps the prefix refuses startup
   with a typed error naming it. If the steering cannot be loaded or
   attached, boot refuses startup with a typed error and does not write the
   route: the link still runs the program an earlier boot attached, so a
   `local` route left by that boot stands only over that steering, and
   before the first successful boot there is no route. The prefix is never
   locally deliverable without a steering program deciding every lookup;
   that program's correctness is established by its Tier-3 tests in CI on
   the pinned kernel, not at boot (ADR-0171). Boot is the only writer of the route and of the steering program:
   on the appliance only Overdrive writes these kernel objects (ADR-0068), and
   at runtime the owner changes the steering only by inserting entries and by
   closing listeners, both of which report their outcome to the owner, so
   neither the route nor the steering has a runtime audit or repair. A crash
   at any boot step leaves either the earlier state or the new one, and the
   next boot converges it. The route is node infrastructure: `serve` never
   removes it at shutdown, graceful or not; it persists across restarts and
   every boot converges it. While `serve` is down the pinned steering
   (point 2) keeps the prefix fail-closed.
2. **Only bound intake listeners are reachable at the guest prefix.** Local
   delivery to the prefix is decided by a pinned socket-lookup program whose
   socket map names, per `(workload_addr, port)`, the open intake listener
   (ADR-0171): a host-local TCP connection is assigned to that listener or
   refused with a reset; everything else to the prefix is dropped. The owner
   inserts a listener's entry after it listens; closing the listener removes
   the entry in the same kernel step. No connection to the prefix ever
   reaches a host socket other than an open intake listener or leg-C.
   Constant firewall rules keep host-local and remote traffic on its path:
   - output and prerouting: every packet to the guest prefix not accepted by
     the leg-S exemption or diverted to leg-C (`inbound_destinations`) is
     dropped — the output rule rejects (TCP reset, ICMP port-unreachable
     otherwise) so host-local clients see a refusal at once; the prerouting
     rule drops, as today for remote traffic. So an unmarked host-local client
     reaches an intake only through leg-C.

   The guest gateway is excluded from the guest-prefix match of both the
   program and the rules; it is the DNS address (ADR-0154). These replace the
   per-allocation `managed_guest_ips` set, whose membership is no longer
   needed.
3. **Intake listeners.** A host TCP listener on `workload_addr:port` may exist
   only for a declared TCP listen port (the PORT-295-C projection), only while
   the allocation is Active and node forwarding is not quiesced, and only under
   the condition ADR-0163 sets. No listener exists before activation; teardown
   and quiescence close them all.
4. **Establishment-time install.** The host `sock_ops` program installs each
   intake child at `PASSIVE_ESTABLISHED` and parks its bytes in a host cell;
   the owner re-arms the child after `accept()` (ADR-0151, ADR-0158). A child
   is recognised as an intake child by its listener's identity, never by port
   (ADR-0151).
5. **Host-initiated flow.** For each accepted connection the owner opens a
   vsock STREAM to the guest inbound port with a `TcpAccept` request carrying
   the kernel-reported peer address of the intake connection and the declared
   port. The guest connects to the application from that address (ADR-0150).
6. **Client identity seen by the guest.** The guest sees the intake
   connection's real peer. For platform clients on the host (leg-S, probes)
   that peer is the guest gateway address, as under the bridge topology; for
   remote clients it is whatever leg-S presents today, unchanged.
7. **Other rules.** ADR-0125's rules 1, 4, 6 and 7 stay as they are. Rules 2–3
   and `outbound_sources` are removed (ADR-0153, ADR-0162). Rules 5 and 8 now
   match the constant guest prefix instead of `managed_guest_ips`.

## Alternatives considered

- **No `local` prefix route; intake listeners bind with `IP_FREEBIND` and
  inbound reaches them by TPROXY only.** Avoids local delivery to wildcard
  host services, but leg-S and probes are exempt from TPROXY by design, so
  they could not reach the intake at all without changing the meaning of
  rules 1 and 6. Rejected.
- **Per-allocation `/32` local routes added at activation.** Still delivers to
  wildcard host services on those addresses while no intake is bound.
  Rejected.
- **Keep `managed_guest_ips`, add only the reset rule.** Leaves unassigned and
  released guest-prefix addresses delivering to wildcard host services for
  remote clients. Rejected.
- **One node-shared transparent intake that recovers the original
  destination.** Changes the meaning of rules 1 and 6. Rejected.
- **A per-allocation host netdevice per guest address.** Reintroduces per-VM
  netdevices. Rejected (ADR-0145).
- **Steering by a firewall set of admitted `(address, port)` elements.** The
  element and the listener are two kernel objects: a failed removal forces the
  listener to stay bound (a probe sees `connect()` complete on a port no
  longer served; teardown and lease release wait). ADR-0171 records the
  comparison. Rejected.
- **Converge the route at boot before the steering is attached.** On a first
  boot whose steering convergence fails — a program the verifier rejects, a
  refused attach, or a crash part-way — the prefix would be locally
  delivered with no lookup decision. Rejected.
- **Remove the route when the steering cannot be converged.** The prefix is
  then not local: host-local connects and arriving packets follow the default
  route and leave the host instead of being refused. Rejected.
- **Fence the prefix with a tagged `prohibit` route when the steering
  convergence fails.** A failed boot leaves the earlier program and its
  route, which already refuse every lookup while `serve` is down, and a
  crash right after a swap leaves `local` over this binary's program, which
  its CI tests establish (ADR-0171); the fence would add a write — and a
  failing write — that covers no exposed state. Rejected.
- **Audit and repair the steering and the route at runtime.** On the
  appliance nothing but the owner writes them, and every runtime write (an
  entry insert, a listener close) reports its outcome to the owner, so a
  runtime audit could detect only a change made by software that does not
  exist on the appliance. Boot convergence covers Overdrive's own crashes.
  Rejected.
- **Remove the route at graceful shutdown.** Adds a shutdown step that a crash
  skips anyway, so boot convergence and the persistent rules must already
  handle a left-behind route; removing it buys nothing and makes graceful and
  crash restarts differ. Rejected.
- **Address backends by CID instead of `workload_addr`.** Ripples through
  persisted rows, discovery, probes and dataplane maps. Rejected.
- **No preferred source on the local route.** Platform clients would present
  `workload_addr` itself as the client address. Rejected.

## Consequences

- Observation rows, backend identity and probe targets are unchanged.
- A connection to `workload_addr:port` never reaches a host service: it
  reaches a bound intake listener, leg-C, or a refusal. A TCP probe fails
  while no intake listener is bound.
- Before activation no intake listener exists, so a host-local connect is
  refused at once; under the bridge topology it went unanswered while the TAP
  was down. This is the residual pre-activation difference D15-R1, which the
  user acknowledged on 2026-10-05.
- While `overdrive serve` is down the pinned steering keeps the prefix
  fail-closed: every listener closed with the process, so every lookup for the
  prefix drops, and the same holds after a boot refusal, which leaves the
  earlier program attached. The next boot loads its own program, swaps it
  in, and only then re-converges the route to `local`.
- Closing an intake listener ends its reachability in the same kernel step, so
  a port the guest stopped serving refuses at once; teardown, quiescence and
  lease release never wait on a steering write (ADR-0171).
- A shared firewall table left partial by a failure of the mTLS worker does
  not open the prefix to host services; it affects only the existing mTLS
  interception rules, under ADR-0124's existing recovery.
- Only Overdrive writes the route and the steering on the appliance; the
  image configuration (ADR-0068) carries no network manager or daemon that
  removes routes it did not create. That is an assumption of the design,
  discharged by the image, not by a runtime check.
- Two constant rules; `managed_guest_ips` and its element lifecycle are
  deleted.
- A refusal from the guest toward a transparent client address is delivered at
  once because the guest sets `net.ipv4.fwmark_reflect=1` (ADR-0150).
- The preferred-source behaviour of a `local` route is from kernel source,
  not yet observed (V-12). The steering is validation items V-19 and V-26.
- The steering, boot order and listener lifecycle are checked by the formal
  model of the guest-flow owner (ADR-0168, `specs/quint/guest-flow-owner/`).
