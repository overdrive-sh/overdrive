# ADR-0173 — Inbound TCP terminates at a host intake listener on `workload_addr:port`, which opens one host-initiated vsock flow per accepted connection

## Status

**Proposed — approved by user (D8 2026-10-05; its residual pre-activation
difference D15-R1 acknowledged by user 2026-10-05); pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*. When a listener exists
while the guest does or does not listen is ADR-0163; how it becomes reachable
is ADR-0171 / ADR-0172. Inbound UDP service to VMs is deferred to **#310**
(user ruling D15, 2026-10-05).

## Context

Under ADR-0145 the guest has no NIC. Inbound TCP to `workload_addr:port`
arrives on the host: from leg-S after mTLS termination (remote mesh clients),
and from host-local platform clients (marked probes). Something on the host
must terminate it and carry its bytes to the guest application without a
userspace relay.

The guest-capture spike showed the inbound path end to end: a host intake
child installed at establishment and parked, a host-initiated vsock flow, and
a guest socket bound with `IP_TRANSPARENT` to the client address the host
intake observed. 10,000/10,000 inbound flows passed with the server speaking
first, and 10,000/10,000 with the client writing immediately.

## Decision

1. **Intake listeners.** A host TCP listener on `workload_addr:port` may exist
   only for a declared TCP listen port (the PORT-295-C projection), only while
   the allocation is Active and node forwarding is not quiesced, and only
   under the condition ADR-0163 sets. No listener exists before activation;
   teardown and quiescence close them all.
2. **Establishment-time install.** The host `sock_ops` program installs each
   intake child at `PASSIVE_ESTABLISHED` and parks its bytes in a host cell;
   the owner re-arms the child after `accept()` (ADR-0151, ADR-0158). A child
   is recognised as an intake child by its listener's identity, never by port
   (ADR-0151).
3. **Host-initiated flow.** For each accepted connection the owner opens a
   vsock STREAM to the guest inbound port with a `TcpAccept` request carrying
   the kernel-reported peer address of the intake connection and the declared
   port. The guest connects to the application from that address (ADR-0150,
   ADR-0174).

## Alternatives considered

- **Always-bound listener from activation.** A TCP probe would pass while the
  application is down; ADR-0163 records the comparison. Rejected.
- **Bind listeners before activation.** Host-local clients could park
  plaintext connections before the intercept rules exist. Rejected.
- **A userspace relay from the intake to the guest.** Forbidden by the
  feature's charter (no userspace payload relay). Rejected.
- **One node-shared intake listener for all addresses.** Collapses the
  per-port listen-state mirroring (ADR-0163) and the listener-tag identity of
  intake children (ADR-0151). Rejected.

## Consequences

- Before activation no intake listener exists, so a host-local connect is
  refused at once; under the bridge topology it went unanswered while the TAP
  was down. This is the residual difference D15-R1, acknowledged by the user
  on 2026-10-05.
- Early client bytes park in the host kernel until the guest pairs the flow
  (ADR-0158); no byte crosses userspace.
- The number of intake listeners on a node is bounded by the declared TCP
  ports of held leases, which admission counts (ADR-0176).
