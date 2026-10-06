# ADR-0162 — Non-mesh guest egress is forwarded in the kernel directly to its destination; no userspace payload relay remains on any guest path

## Status

**Proposed — ruling D14 approved by user 2026-10-05; the connect bound
D15-R3 = (b) (independent DESIGN review finding L-5) approved by user
2026-10-06; pending independent DESIGN review.** GH #295. Recorded in the #295
feature delta, § *[REF] vsock Attachment Replacement DESIGN — PROPOSED
2026-10-05*. The ADR becomes Accepted with the D1 transport it depends on. Its
destination set is limited by ADR-0167 (host-internal destinations refused;
decision D26, approved by user 2026-10-06).

## Context

The transport constraint is "no userspace application payload relay". Two
existing leg-F data paths were examined against it:

- **Non-mesh pass-through.** `spawn_cleartext_passthrough` in
  `crates/overdrive-worker/src/mtls_intercept_worker.rs` relays every byte
  through `tokio::io::copy_bidirectional` — a userspace copy.
- **Mesh enforcement.** Per-connection splice pump threads
  (`crates/overdrive-dataplane/src/mtls/splice.rs`) move bytes with `splice(2)`
  between the plaintext leg and the kTLS socket. The user accepted this as
  kernel forwarding.

The guest-capture spike forwarded non-mesh egress entirely in the kernel: the
host opened a TCP socket to the original destination, installed it at
`ACTIVE_ESTABLISHED`, and SockHash-forwarded it to the flow's vsock leg.
Unmodified `curl` fetched `http://example.com/` and a 2 MiB blob byte-exact,
and a real Internet SSH server's banner arrived before any client byte. The
off-host UDP spike (`spike/v11-vip-v14-findings.md`) forwarded guest UDP of
0 to 59,000 bytes through a host UDP socket connected to a destination
behind a real egress interface (a namespace behind veth at MTU 1,500, and
the physical NIC for `dig @1.1.1.1` and wire captures), with the frame
removed as ADR-0165 decides.

## Decision

- A guest TCP flow whose destination resolves `NonMesh` (ADR-0153) is paired
  with a host TCP socket the owner connects **directly to the original
  destination**, unless the destination is host-internal (ADR-0167). That
  socket carries `SO_LINGER{1,0}` from creation (ADR-0160) and is installed
  at `ACTIVE_ESTABLISHED`; payload moves only through the host verdict.
- A guest datagram association is paired with a host UDP socket connected
  directly to the original destination (ADR-0147, ADR-0165), under the same
  host-internal deny set. Datagrams are not resolved against the mesh: mTLS is
  TCP-only, as today.
- The owner always `connect()`s its host-facing socket, and the owner runs
  under the ADR-0053 `connect4` attach point, so a datagram destination that
  is a service VIP is rewritten to a backend in the kernel while the guest
  keeps seeing the VIP. A TCP destination that is a service VIP never reaches
  this path: mesh resolution classifies it first (ADR-0164).
- Leg-F's cleartext pass-through and its outbound TPROXY accept path are
  deleted with their tests. Leg-F's outbound role is mesh flows only.
- **The connect keeps today's bound (D15-R3).** The owner's connect to a
  non-mesh destination is non-blocking and bounded only by the host kernel's
  own SYN retries (`net.ipv4.tcp_syn_retries`, about 2 min by default), as
  leg-F's dial was; the owner adds no timeout, and the guest opener of the
  flow waits for the host's answer without a pairing deadline of its own.
  A refused or unreachable destination fails at once; an unresponsive one
  when the kernel gives up.
- Connect failure to a non-mesh destination refuses the flow with a typed
  refusal carrying the connect error class; the guest application observes a
  reset.

## Alternatives considered

- **Keep leg-F's userspace relay as a scoped exception.** Violates the
  constraint. Rejected by ruling D14.
- **Refuse non-mesh egress.** Removes an existing outcome (VMs reaching
  external addresses). Rejected by ruling D14.
- **Splice-based relay in leg-F for non-mesh.** Still a userspace-driven
  per-connection pump and an extra loopback hop, where the pair can reach the
  destination directly. Rejected.
- **Bound the destination connect at 3 s** (the pairing deadline less the
  request deadline). Releases parked cells and quota sooner, but a dead
  destination would fail after 3 s instead of after the kernel's SYN retries,
  a change the kernel does not force. Rejected by the user (D15-R3 = (b),
  2026-10-06).

## Consequences

- Non-mesh egress costs one host socket per flow and no threads.
- A connect to an unresponsive destination holds its guest parking cell, its
  per-allocation quota slot and two host sockets for up to the host's
  SYN-retry time; the per-allocation quota bounds what one VM can hold
  (ADR-0147), and the cost at density is measured (V-9).
- Guest egress source addressing on the wire: the host socket's source address
  is chosen by host routing, as leg-F's dial chose it today. NAT and egress
  policy for VMs are unchanged by this feature.
- `MtlsResolve` is now consumed by the guest-flow owner as well as the mTLS
  worker (ADR-0153).
