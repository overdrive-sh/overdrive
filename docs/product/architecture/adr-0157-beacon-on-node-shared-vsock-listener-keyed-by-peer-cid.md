# ADR-0157 — The guest beacon is accepted by one node-shared host `AF_VSOCK` listener and attributed by the kernel-reported peer CID

## Status

**Accepted — decision D17 approved by user 2026-10-05.** GH #295. Recorded in
the #295 feature delta, § *[REF] vsock Attachment Replacement DESIGN*. Depends
on ADR-0146.

This ADR **amends ADR-0082 §D4's transport** on acceptance: the beacon
reaches the host over a node-shared `AF_VSOCK` listener instead of a per-VM
Unix socket. The beacon's message set and meanings are unchanged; which
launch a session belongs to is ADR-0177 (the launch token READY carries).

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

Below the socket API, the vhost transport delivers a guest TX packet only
when its `src_cid` equals the device's guest CID and its `dst_cid` is the
host's (`drivers/vhost/vsock.c:551-556 @ v7.0`); any other packet is freed.
A crafted guest driver forging `src_cid` was not exercised (V-10). A host vsock listener bound on CID 2 before vhost-vsock loaded failed
with `EADDRNOTAVAIL` (guest-capture spike, increment a); binding
`VMADDR_CID_ANY` and checking the peer CID worked.

## Decision

- **One listener.** The VM driver binds exactly one host `AF_VSOCK` stream
  listener on `VMADDR_CID_ANY`, port 1234, when it is composed.
- **Attribution.** Each accepted session is attributed to the VM whose live
  launch holds the kernel-reported peer CID **and** whose launch token the
  session's READY carries (ADR-0177).
- **Rejection.** A session from a CID no live launch holds is closed with no
  effect, as is a session whose READY carries another launch's token, and a
  second session from a CID that already has a live session. Accepting a
  session is one atomic claim on the CID, taken in one per-CID step after
  its READY is read, which requires both that READY's token equals the token of
  the CID's live launch (started, not yet stopped) and that the claim is
  free (ATTR-LIVE-CLAIM, ADR-0177). The live-launch check at accept only
  closes connections early; the driver's stop ends the launch's liveness in
  the step that closes its session and releases its claim. So two
  concurrent sessions from one CID cannot both be accepted, a session a
  dead launch left queued can never hold the claim, and a connection
  accepted before a stop and read after it takes nothing; the claim is
  released when the session ends.
- **Protocol.** The beacon message set and its semantics are unchanged;
  READY's line carries the launch token (ADR-0177).

## Alternatives considered

- **Per-VM ports.** The port would become the identity; a guest could still
  dial another VM's port. Rejected.
- **An authentication secret on the beacon in place of the peer CID.** Adds
  secret material to the guest to defend against a CID-spoofing guest
  driver, which V-10 tests at packet level; a falsified V-10 is a security
  escalation to the user. Rejected as the trust anchor. (ADR-0177's launch
  token is not a secret and does not replace the CID: it tells one CID's
  launches apart.)
- **Bind on CID 2.** Fails before the vhost transport module is loaded.
  Rejected.

## Consequences

- The beacon's trust anchor moves from filesystem-path isolation to kernel
  vhost source-CID validation: shown at the socket API, cited from source
  below it (V-10, packet level open). Across launches of one CID the peer
  CID is not enough — a connection a dead VM left queued is accepted with
  the CID's next holder as peer and delivers its buffered READY (V-22,
  feature delta P-37) — so the launch token decides (ADR-0177).
- Per-VM Unix beacon sockets and run-directory vsock paths are deleted.
- A listener bind failure refuses driver composition at startup.
