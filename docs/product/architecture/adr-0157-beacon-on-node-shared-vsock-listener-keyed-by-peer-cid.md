# ADR-0157 — The guest beacon is accepted by one node-shared host `AF_VSOCK` listener and attributed by the kernel-reported peer CID

## Status

**Proposed — decision D17 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*. Depends on ADR-0146.

If accepted, this ADR **amends ADR-0082 §D4's transport**: the beacon reaches
the host over a node-shared `AF_VSOCK` listener instead of a per-VM Unix
socket. The beacon's messages and meanings are unchanged.

## Context

ADR-0082 §D4 defines the guest→host READY / EXEC / EXIT / SHUTDOWN beacon on
vsock port 1234. Today the VM driver binds one Unix listener per VM at
`<run_dir>/vsock_1234`, and the Cloud Hypervisor Unix muxer connects to it;
the file path carries the VM's identity.

Under kernel vhost-vsock the beacon arrives on host `AF_VSOCK`, and port 1234
can be bound once per host. EXEC and EXIT carry authority over the workload,
so attributing a session to the wrong VM would let one guest release or
terminate another.

The CH fork spike (`spike/ch-vhost-vsock-findings.md`, P5–P6) showed:

- guest binds to a foreign CID (own + 100, 2, 1) fail with `EADDRNOTAVAIL`;
  unbound, `VMADDR_CID_ANY` and own-CID connects are all seen by the host
  with the assigned CID;
- one guest cannot reach another guest's CID.

Below the socket API, upstream v7.0 `drivers/vhost/vsock.c` delivers guest TX
packets only when `src_cid` equals the device's guest CID. A crafted guest
driver forging `src_cid` was not exercised; the distro kernel may carry
patches. A host vsock listener bound on CID 2 before vhost-vsock loaded failed
with `EADDRNOTAVAIL` (guest-capture spike, increment a); binding
`VMADDR_CID_ANY` and checking the peer CID worked.

## Decision

- **One listener.** The VM driver binds exactly one host `AF_VSOCK` stream
  listener on `VMADDR_CID_ANY`, port 1234, when it is composed.
- **Attribution.** Each accepted session is attributed to the VM whose live
  launch holds the kernel-reported peer CID.
- **Rejection.** A session from a CID no live launch holds is closed with no
  effect, as is a second session from a CID that already has a live session.
  Accepting a session is one atomic claim on the CID — the check and the
  claim are the same operation, so two concurrent sessions from one CID
  cannot both be accepted; the claim is released when the session ends.
- **Protocol.** The beacon message set and its semantics are unchanged.

## Alternatives considered

- **Per-VM ports.** The port would become the identity; a guest could still
  dial another VM's port. Rejected.
- **An authentication token on the beacon.** Adds secret material to the
  guest and changes ADR-0082's wire format. Held as the fallback if
  packet-level CID spoofing is shown possible (V-10). Rejected as primary.
- **Bind on CID 2.** Fails before the vhost transport module is loaded.
  Rejected.

## Consequences

- The beacon's trust anchor moves from filesystem-path isolation to kernel
  vhost source-CID validation: shown at the socket API, cited from source
  below it (V-10, packet level open).
- Per-VM Unix beacon sockets and run-directory vsock paths are deleted.
- A listener bind failure refuses driver composition at startup.
