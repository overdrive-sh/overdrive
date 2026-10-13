# ADR-0186 — Leg-S reaches VM backends through a temporary vsock relay until #303

## Status

**Accepted — the temporary leg-S bridge approved by user 2026-10-11** (user
ruling "Finding 1 amended", recorded on #303:
https://github.com/overdrive-sh/overdrive/issues/303#issuecomment-6113302506).
GH #295. This ADR is deleted when #303 lands.

Amends ADR-0120 (leg-S plaintext delivery to a VM backend), which is
operative and carries a dated amendment note. The interface contract lives in
the #295 feature delta, `docs/feature/netns-density-295/feature-delta.md`,
§ *[REF] leg-S over vsock (temporary bridge until #303) and the mTLS
forwarded-outbound port*.

## Context

Leg-C terminates a remote mesh client's mTLS, and leg-S delivers the
connection in plaintext to the backend (ADR-0120). For a VM backend the
guest prefix is not host-local, and a host platform client reaches a VM port
only through an owner-opened vsock flow (ADR-0173). The end state for VM
mesh traffic is in-guest kernel mTLS (#303), after which the host forwards
ciphertext over the existing sockmap redirect and no plaintext crosses the
host. #295's VM cutover cannot wait for #303 without leaving inbound VM mesh
traffic with no path.

The legs-over-vsock spike (V-28, `spike/findings-v28-legs-vsock.md`,
Linux 7.0, real microVM) measured a stand-in relay between a host TCP leg and
a host-opened vsock stream: byte-exact in both directions, half-close
matching TCP in every case over 2,016 stress flows, and reset fidelity
restored by the flow-end classifier (ADR-0180; 192 of 192 in each abort
scenario).

## Decision

- **Leg-S relays plaintext for a VM backend.** For a VM backend, leg-S dials
  the guest-flow owner (ADR-0173) and relays between its kTLS socket and the
  returned stream in userspace. This relay is the one exception to the
  feature charter's "no userspace payload relay".
- **Ends map both ways.** A TLS peer's reset makes leg-S close the vsock
  while it is open, and the guest aborts the application connection with a
  reset; a guest application's reset makes the guest owner close its vsock,
  the stream reports an abort, and leg-S resets the TLS client; a FIN and a
  clean half-close map to each other in both directions.
- **Temporary.** The relay and this ADR are deleted when #303 lands.

## Alternatives considered

- **Carry the VM mesh data path only by in-guest mTLS (#303).** Correct as
  the end state, but the VM cutover could not ship before #303 without
  leaving inbound VM mesh traffic with no path. Rejected for #295 (ruling
  "Finding 1 amended").
- **An explicit owner-to-guest abort message instead of the flow-end
  classifier.** Not tested (V-28 "Not run"), and the classifier is required
  anyway for every guest flow's end (ADR-0180). Rejected.

## Consequences

- Leg-S to a VM backend costs about +0.5 ms per connection and 12–37 % less
  bulk client-to-guest throughput than loopback TCP (V-28 Q1). vsock STREAM
  has no `splice_read`, so `splice` from it falls back to a kernel copy
  (`net/socket.c:1129-1140 @ v7.0`); there is no zero-copy on the vsock leg.
- The measurements ran with the classifier off and a stand-in relay; the
  production relay with the classifier attached is not yet measured (V-28
  "Not run").
- Plaintext of VM mesh connections passes through leg-S's userspace until
  #303.
