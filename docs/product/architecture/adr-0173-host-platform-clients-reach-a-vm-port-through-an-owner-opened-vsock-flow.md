# ADR-0173 — Host platform clients reach a VM port through an owner-opened vsock flow

## Status

**Accepted — INB-1 (no host-local delivery to the guest prefix; the per-kind
dial) approved by user 2026-10-11** (user rulings "Finding 1" and "Finding 1
amended", recorded on
#303: https://github.com/overdrive-sh/overdrive/issues/303#issuecomment-6113302506);
D15-R1 acknowledged by user 2026-10-05. GH #295. Inbound UDP to VMs is #310.

Amends ADR-0094 (the VM probe target), which is operative and carries a
dated amendment note. How leg-S uses the dial for a VM backend until #303 is
ADR-0186.

The interface contract lives in the #295 feature delta,
`docs/feature/netns-density-295/feature-delta.md`: § *[REF] Driving port —
guest stream dial `GuestStreamDial`*, § *[REF] leg-S over vsock (temporary
bridge until #303) and the mTLS forwarded-outbound port*, § *[REF] VM probe
runner*, § *[REF] Wire contracts — flow request, control message, datagram
frame, flow event record* and § *[REF] Typed error taxonomy and who
branches*.

## Context

Under ADR-0145 the guest has no NIC. Two host platform clients connect to a
VM's `workload_addr:port` today:

- **leg-S**, which delivers a remote mesh client's connection in plaintext to
  the backend after leg-C terminates its mTLS (ADR-0120);
- **the VM probe runner**, whose TCP-connect and HTTP probes target
  `workload_addr:port` (ADR-0090, ADR-0094).

Both must reach the guest application's listener with the same observable
outcomes as a TCP connection to it: established, refused, timed out, reset,
half-closed.

The legs-over-vsock spike (V-28, `spike/findings-v28-legs-vsock.md`, Linux
7.0, real microVM) ran this path end to end: a host-opened vsock STREAM, a
guest agent that binds to the requested peer address and connects to the
application, and a pairing reply carrying paired, refused with an errno, or
timed out with an errno. Streams were byte-exact in both directions;
half-close matched TCP in every case over 2,016 stress flows; probes gave the
same answer as a direct probe in every guest state, including a listener that
never accepts.

Reset fidelity does not cross vsock STREAM by itself. An established-state
`RST` runs `virtio_transport_do_close`, which sets `SOCK_DONE` and the peer's
shutdown mask but no socket error (`net/vmw_vsock/virtio_transport_common.c:1244-1252,
1463-1464 @ v7.0`), so `read()` returns 0; and `vsock_poll` reports only
`EPOLLRDHUP` / `EPOLLHUP` for a peer shutdown (`net/vmw_vsock/af_vsock.c:1247-1259
@ v7.0`). With the plain socket API a guest reset arrives as a clean EOF, and
the peer sees silent truncation (V-28: 192 of 192). A flow's end is therefore
classified from the kernel's vsock SHUTDOWN flags by a raw tracepoint
(ADR-0180), which V-28 showed restores reset fidelity (192 of 192 in each
abort scenario).

## Decision

1. **The guest prefix is not host-local.** No address of the guest prefix
   is delivered to a host socket by a `local` route: there is no `local`
   route for the prefix and no per-allocation route or netdevice. (The
   gateway alone is host-local, ADR-0152. No other route covers the prefix:
   a same-node mesh leg-B dial to a VM address reaches leg-C through the
   intercept's mark rule and a `127.0.0.1` source, ADR-0190.)
2. **Per-kind dial.** An exec or process workload is dialled as today: leg-S
   and the probe runner make a plaintext TCP connection to
   `workload_addr:port`, unchanged. A VM workload — a destination inside the
   guest prefix — is dialled through the guest-flow owner's driving port,
   `GuestStreamDial` (feature delta). Leg-S and the VM probe runner are its
   callers.
3. **Admission before any effect.** The owner resolves the target address to
   the lease of an Active VM allocation and checks that the target port is one
   of its declared TCP listen ports (the PORT-295-C projection). A target that
   is not an Active VM lease or not a declared port, a node whose forwarding is
   quiesced (ADR-0169), an allocation whose control session is not live, and a
   dial past the allocation's flow quota (ADR-0147) each return a distinct
   typed result at once and change no state.
4. **One host-opened flow per dial.** The owner opens a vsock STREAM to the
   guest's inbound port with a `TcpAccept` request that carries the guest
   gateway address with port 0 as the peer (ADR-0174), the declared port, and
   the live launch token of the CID's lease (ADR-0177). The guest connects to
   the application, its own connect bounded by a guest connect timeout, and
   replies on the control session (ADR-0166): paired, refused with the
   connect errno, or timed out with the connect errno. The owner's pairing
   deadline for a host-opened flow is strictly greater than the guest's
   connect timeout, so a guest reply always arrives before the owner gives
   up. Paired yields a stream; every other end — refused, timed out, the
   pairing deadline, a vsock connect or I/O failure (the VMM is gone) — is a
   typed dial error. A dial the owner ends while it is in flight reports the
   cause with the admission result of that cause: quiescence as quiesced,
   allocation teardown as not serving, an observed control-session loss as
   no session; a VMM exit resets the host vsock (the device-release orphan
   sweep) and the dial reports the VM unreachable.
5. **The dialer owns the stream.** The stream owns the connected host vsock.
   The host side of a host-opened flow is not installed into the host sockmap;
   the dialer reads and writes it. The owner delivers the classified end of
   the guest's direction — clean half-close or abort — from the flow-end
   classifier (ADR-0180) for that socket; the classification precedes the EOF
   wake. Dropping the stream closes the vsock, which the guest sees as an
   abort; shutting down its write side sends a clean half-close.
6. **A dial holds a forwarder slot, not a host pair.** A dialled stream
   installs no host sockmap pair, but it takes one forwarder slot from the
   node's flow-slot space (its classifier registration and drain cells,
   whose epoch keys its end records, ADR-0180) and counts against its
   allocation's flow quota (ADR-0147) until it ends. A dial is refused for
   capacity when either is full: the allocation's quota or the node's flow
   slots, which every VM on the node shares. If PROBE-NOSAMPLE (ADR-0187,
   proposed) is confirmed, the quota half applies to relay dials only: a
   probe dial, which measures the application rather than carrying a
   client's connection, takes a node flow slot but stands outside the
   allocation's quota, bounded by one dial in flight per declared probe.
7. **Probes (amends ADR-0094).** A TCP-connect probe of a VM target is healthy
   if and only if the dial yields a stream, which the probe then drops. An HTTP
   probe runs over the stream. A listener that never accepts is healthy while
   its backlog has room and unhealthy once it is full, as a direct TCP probe
   is today (V-28 Q4). Under ADR-0187 (PROBE-NOSAMPLE) a probe
   dial refused for node flow-slot capacity or because node forwarding is
   quiesced records no sample rather than a failure, so a node condition
   never fails a VM's probe, while a hung application at its own quota is
   still reached by its probe and fails it. The probe-socket mark of ADR-0094
   applies to the exec/process path only.
8. **Before activation (D15-R1).** A host platform client that dials a VM
   before its allocation is Active receives the typed not-serving result at
   once; a remote client sees leg-C close the connection, the same failure
   class a remote client sees for a backend that is not yet serving.

The dial gates no lifecycle state; it yields a typed result to its caller
(feature delta § *Lifecycle Gate Ownership*).

## Alternatives considered

- **Make the guest prefix host-local and terminate host-local connections at
  host intake listeners**, reached through a socket-lookup steering program,
  with listener-state mirroring from the guest and capacity admission of
  declared ports. Every piece exists only to let a host TCP client connect
  to an address that is not on the host; the owner can open the flow
  directly. Rejected (INB-1).
- **Carry the leg-C TLS peer as the `TcpAccept` peer.** Works (V-28 Q3), but
  changes the client address a guest application sees from today's (ADR-0174).
  Rejected.
- **Install the host side of a host-opened flow into the host sockmap.** The
  dialers consume the bytes in userspace (the leg-S relay, the HTTP probe), so
  a kernel redirect has no target. Rejected.

## Consequences

- VM probes and leg-S see refused, timed out, reset, half-close and VMM loss
  as typed results or stream ends with the same meaning as a TCP connection
  to the application.
- Host-local connects to a VM's `workload_addr` are refused by the constant
  output rule (ADR-0172); no host service is reachable through a guest-prefix
  address.
- No intake listener, steering program, listener-state message or host port
  block exists; the guest reports no listen state to the host.
- Inbound UDP to VMs remains #310.
