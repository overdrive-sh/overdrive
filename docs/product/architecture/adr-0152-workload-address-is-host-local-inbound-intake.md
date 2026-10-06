# ADR-0152 — `workload_addr` stays the allocation's address on both sides; on the host it is made local, only bound intake listeners can be reached at it, and intake listeners open host-initiated flows that present the real client to the guest

## Status

**Proposed — decision D8 approved by user 2026-10-05; its residual
pre-activation difference D15-R1 acknowledged by user 2026-10-05; decision
D8a — the guest-prefix firewall rules and the boot convergence of the shared
route (independent DESIGN review findings B-2, H-4) — approved by user
2026-10-06; its two corrections from the formal model check
(`spike/quint-owner-findings.md` findings 1 and K-D3) — an element whose
removal fails keeps its listener bound until it is gone (D8a-REVOKE), and
boot keeps or adds the route only when the steering rules are verified
present (D8a-ROUTE) — and the ruling that the route is never removed at
shutdown (U-4) approved by user 2026-10-06; the re-assertion of a wanted
port's element at every serving period (D8a-REASSERT, recorded in ADR-0163),
named quiescence holders (D8a-HOLD, ADR-0169), and the two residual,
fail-closed exposures as now bounded — a `connect()` completing on a port no
longer wanted whose element removal keeps failing (D8a-PROBE) and other
software deleting the shared firewall table (D8a-FLUSH) — approved by user
2026-10-06 (direction: correct design over simple); pending independent
DESIGN review.** GH #295. Recorded in the
#295 feature delta, § *[REF] vsock Attachment Replacement DESIGN — PROPOSED
2026-10-05*. Whether an intake listener exists while no guest application
listens is ADR-0163. Inbound UDP service to VMs is deferred to **#310** (user
ruling D15, 2026-10-05).

Amends on acceptance: ADR-0125 (constant rules: rules 2–3 and
`outbound_sources` removed; `managed_guest_ips` replaced by the constant guest
prefix; the `intake_listeners` set and two rules added) and ADR-0137 (boot
convergence covers `intake_listeners`).

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

1. **Shared local route, converged on boot, only behind verified rules.** The
   guest prefix is host-local through one shared `local` route on `lo` whose
   preferred source is the node's guest gateway address, tagged with
   Overdrive's route protocol identifier. There is no per-allocation route or
   netdevice. Every boot first verifies that the guest-prefix steering rules
   of point 2 are present in the shared firewall table as pinned. Only then
   does it observe the routes covering the prefix and converge: an identical
   route is kept; an Overdrive-tagged route for the prefix with different
   attributes is replaced; a missing route is added; a route not tagged by
   Overdrive that overlaps the prefix refuses startup with a typed error
   naming it. If the rules are not verified present, boot removes an
   Overdrive-tagged route for the prefix and refuses startup with a typed
   error: the prefix is never locally deliverable without the rules. The
   route is node infrastructure: `serve` never removes it at shutdown,
   graceful or not; it persists across restarts and every boot converges it.
   While `serve` is down the persistent firewall rules (point 2) keep the
   prefix fail-closed.
2. **Only bound intake listeners are reachable at the guest prefix.** The
   host firewall holds a set `intake_listeners` of `(workload_addr, port)`
   elements. The owner adds an element only after the listener is bound and
   listening, and removes it before closing the listener, so the set never
   names a port without a bound intake listener. If removing the element
   fails, the listener stays bound and the removal is retried; the listener
   closes only after the element is gone. While its removal is pending, the
   listener pairs nothing: every connection it accepts is reset. Teardown does
   not complete, and the allocation's lease (with its address and CID) is not
   released, while any element of the allocation exists. A wanted port's
   element is asserted present at the start of every serving period and
   retried until it is (ADR-0163), and a repair of these rules runs only while
   the repairing recovery holds quiescence (ADR-0169). The constant rules then
   are:
   - output: a marked (leg-S / probe) TCP connection to the guest prefix whose
     `(address, port)` is not in `intake_listeners` is rejected with a TCP
     reset — placed before the leg-S mark exemption;
   - output and prerouting: every other packet to the guest prefix not
     accepted by the leg-S exemption or diverted to leg-C
     (`inbound_destinations`) is dropped — the output rule rejects (TCP reset,
     ICMP port-unreachable otherwise) so host-local clients see a refusal at
     once; the prerouting rule drops, as today for remote traffic.

   The guest gateway is excluded from the guest-prefix match; it is the DNS
   address (ADR-0154). These rules replace the per-allocation
   `managed_guest_ips` set, whose membership is no longer needed.
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
- **Close the listener when its element's removal fails, and retry the
  removal later.** The element then admits marked connections to a port with
  no intake listener, which reach a wildcard host service; it survives
  quiescence, teardown and lease release, and the next allocation on the
  address inherits a pre-admitted port. Found by the formal model
  (`spike/quint-owner-findings.md`, finding 1). Rejected.
- **Converge the route at boot without checking the rules.** A table removed
  by other software between the rule step and the route step leaves the prefix
  locally delivered with no steering (model `steer_envFlush`). Rejected.
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
- While `overdrive serve` is down the firewall rules persist and keep the
  prefix fail-closed; boot convergence clears `intake_listeners` (ADR-0137
  discipline), reinstalls the rules, verifies them, and only then
  re-converges the route.
- Residual (D8a-PROBE): when a port is no longer wanted (the guest stopped
  listening, quiescence, teardown, session loss) and its element's removal
  keeps failing, the listener stays bound and resets what it accepts, so a
  marked TCP probe to that port can see `connect()` succeed before the reset.
  It never affects a wanted port: a wanted port serves at once and its element
  is re-asserted (ADR-0163). It is irreducible in this design: the element and
  the listener are two kernel objects that cannot be changed in one step;
  fail-closed order removes the element before closing the listener; while
  the element cannot be removed, the port's connections must land on an
  Overdrive listener rather than on a wildcard host service, and a listening
  socket completes the handshake. It lasts only while a kernel write keeps
  failing, affects only that port, pairs no flow and reaches no host service.
  Teardown waits, holding the lease (CleanupPending, ADR-0141). Removing it
  would need a design in which the listener itself is the steering decision
  (no separate element), which is not this ADR's decision.
- Residual (D8a-FLUSH): if other software deletes the shared firewall table
  while the route is present, the prefix is locally delivered without
  steering — a wildcard host service on a declared port can be reached —
  until the rules are restored. Overdrive cannot prevent a root process from
  deleting its table; it can only detect and repair. While `serve` is up the
  window is at most one audit period (1 s) plus the firewall recovery's own
  ADR-0124 bound (5 s), else `serve` fail-stops: the bound is real because no
  other recovery can reopen forwarding while the firewall recovery holds
  quiescence (ADR-0169). While `serve` is down it lasts until the next boot
  reinstalls and verifies the rules. Validation item V-23 measures the
  window; a measured window above these bounds is surfaced to the user again.
- One more firewall set with one element per bound intake listener, and two
  constant rules. `managed_guest_ips` and its element lifecycle are deleted.
- A refusal from the guest toward a transparent client address is delivered at
  once because the guest sets `net.ipv4.fwmark_reflect=1` (ADR-0150).
- The preferred-source behaviour of a `local` route is from kernel source,
  not yet observed (V-12). The steering rules are validation item V-19.
- The steering, boot order and element lifecycle are checked by the formal
  model of the guest-flow owner (ADR-0168, `specs/quint/guest-flow-owner/`).
