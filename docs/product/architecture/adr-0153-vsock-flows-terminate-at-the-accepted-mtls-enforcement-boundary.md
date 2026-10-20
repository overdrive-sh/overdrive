# ADR-0153 — Guest TCP to a mesh destination reaches the node's mTLS boundary only through a registered pair to leg-F, resolved once at pairing

## Status

**Accepted — decision D9 approved by user 2026-10-05; VIP recognition in the
resolution (decision D24a) approved by user 2026-10-06; the inbound side's
path to a VM (INB-1) and the END-STATE ruling for #303 approved by user
2026-10-11.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN*.

Non-mesh guest egress is ADR-0162 (ruling D14, approved 2026-10-05):
non-mesh egress is forwarded in the kernel and leg-F's userspace cleartext
relay is removed. Mesh flows keep leg-F's existing kernel-splice data path,
which the user accepted as kernel forwarding.

This ADR **supersedes ADR-0115** (TCX classification feeds transparent mTLS)
and **ADR-0139** (the R18 intercept-mark guard).

## Context

Today the TCX classifier marks every validated guest TCP flow and nft TPROXY
steers it to leg-F (ADR-0115, ADR-0120, ADR-0125). Leg-F resolves the original
destination through `MtlsResolve` into one of three outcomes:

- `Mesh(backend)` → originate mTLS to the resolved backend;
- `NonMesh` → cleartext pass-through (`spawn_cleartext_passthrough`, a
  `tokio::io::copy_bidirectional` relay);
- `MeshUnreachable` → refuse, never cleartext.

Under ADR-0145 there is no TPROXY interception point for guest traffic. A
guest flow arrives at the host as an explicit request naming its destination,
before any payload. All workloads are VMs, so leg-F's TPROXY outbound path
serves only guest traffic.

The guest-capture spike carried a remote server's banner
(`curl telnet://github.com:22` → `SSH-2.0-…`) and 10,000/10,000 egress
server-speaks-first flows; under the in-kernel forwarder (ADR-0197) bytes a
host-facing peer sends before the attach wait in its socket's own receive
queue and are forwarded by the attach-time run (ADR-0212; V-EV-PAIR, P-71),
so a host-facing peer that speaks first needs no write gate.

## Decision

1. **Resolve once, at pairing.** For each guest TCP request the owner calls
   `MtlsResolve::resolve` on the original destination exactly once:
   - `Mesh(backend)` → this ADR;
   - `NonMesh` → ADR-0162;
   - `MeshUnreachable` or a resolve error → the flow is refused with a typed
     refusal; nothing is connected.

   The resolution recognises a service's frontend address and its service
   VIP alike (ADR-0164), so a guest TCP flow to a mesh service's VIP is a mesh
   flow and never a cleartext connect.
2. **Mesh flows have exactly one host-facing peer: leg-F.** The owner binds
   the host-facing socket, registers its local 4-tuple with the mTLS worker
   together with the allocation, the original destination and the resolved
   backend, and then connects it to leg-F. That socket is attached to the
   flow vsock once its connect completed, before `Paired` (ADR-0212).
3. **Leg-F does not re-resolve.** It claims the registration atomically by the
   accepted connection's 4-tuple and enforces mTLS to the registered backend.
   An accept with no registration is closed. No write gate is needed. Leg-F's
   socket toward the remote peer of a registered flow carries
   `SO_LINGER{1,0}` until a clean close (ADR-0160).
4. **No unregistered mesh path.** If leg-F is unavailable or the allocation is
   not intercept-live, a mesh flow is refused.
5. **Inbound ends at leg-S, then crosses on an owner-opened flow.** Leg-C
   terminates inbound mTLS as today. For a VM backend, leg-S delivers the
   plaintext by dialling the guest-flow owner's `GuestStreamDial` and relaying
   between its kTLS socket and the returned vsock stream (ADR-0173, ADR-0186). That relay
   is temporary and is deleted when #303 lands.
6. **This ADR does not decide #303.** Under the END-STATE ruling, in-guest
   kernel mTLS (#303) adapts to the NIC-less, vsock-only guest: the guest's
   kTLS socket reaches the host through a guest-local loopback peer forwarded
   into vsock, and the host forwards ciphertext through the in-kernel
   forwarder (ADR-0197); no forwarded socket carries kTLS (ADR-0200).

## Alternatives considered

- **Connect every guest TCP flow to leg-F and let leg-F decide.** Keeps a
  userspace relay for non-mesh traffic, contrary to D14. Rejected.
- **Resolve in both the owner and leg-F.** Two reads of a changing backend
  set can disagree (TOCTOU). Rejected.
- **Bypass leg-F for mesh destinations and wait for #303.** VM mesh traffic
  would be cleartext or blocked. Rejected.
- **kTLS on a forwarded host pair socket.** The forwarder refuses to attach a
  socket that carries a ULP, and a ULP installed after attach saves the
  forwarder's callbacks and turns every TLS alert into an abort (ADR-0200).
  Rejected.

## Consequences

- Leg-F resolves a guest flow by its registered 4-tuple instead of TPROXY
  `getsockname`; ADR-0120 is revalidated against that.
- The leg-F outbound TPROXY path, its cleartext pass-through and the
  guest-sourced nft rules 2–3 and `outbound_sources` are deleted. Rules 1 and
  4–8 remain (rules 5 and 8 as ADR-0172 restates them).
- Single-loss fail-closure is re-proven for the forwarder (V-8).
- When #303 lands it amends this ADR and deletes ADR-0186: mesh traffic then
  reaches the guest as ciphertext, and leg-S's relay for VM backends is
  deleted.
