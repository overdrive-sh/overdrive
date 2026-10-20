# ADR-0209 — A guest flow's places are released when the host reads its end record

## Status

**Accepted — EVMOD-AB, approved by the user 2026-10-17.** GH #295. Not yet
implemented. Contract: the #295 feature delta, § *[REF] vsock data path —
the in-kernel event-driven forwarder (EVMOD)*, "Pair end", and § *[REF]
Owner and provisioner*.

Related decisions, each separately reversible: the bounds whose places are
released (ADR-0208); the end record (ADR-0202); how an owner abort reaches a
dialled stream (EVMOD-STREAM-ABORT, feature delta); that a trailing end is
inert (ADR-0180).

## Context

A flow holds a place in the node pair capacity and, except a VM probe dial,
in its allocation's quota (ADR-0208). The host must free those places
exactly once, when the flow is over, and must not free them while the flow
can still hold host memory.

- A forwarded flow is over for the host when its pair ends: the forwarder
  has dropped its references to both host sockets and queued the pair's one
  end record (ADR-0202). The owner then closes its own descriptors.
- A dialled flow's host end is a stream held by a platform client (leg-S or
  the VM probe runner, ADR-0214); the host vsock is watched, not forwarded
  (ADR-0216).
  It is over for the host when the holder drops the stream or the owner
  aborts it.
- Under the forwarder the guest holds no fixed pool of per-flow slots: a
  pair's state is allocated at attach (ADR-0197). A guest pair may outlive
  its host flow, for example when a guest application has half-closed and
  stopped reading; the memory it holds is the guest application's own, as on
  any Linux host.

## Decision

- **A forwarded flow's places are released when the host reads the flow's
  end record.**
- **A dialled flow's places are released when its stream is dropped or the
  owner aborts it.**
- **No host place waits for the guest's final close**, and the guest holds
  no per-flow slot pool and claims no slot.

## Alternatives considered

- **Hold a cleanly ended flow's places until the guest's final close.** The
  host pair completes only once its vsock peer also ended its direction, so
  a guest pair that outlives its host flow would count against the quota; it
  would defend guest-internal memory that the guest's own application holds.
  With no fixed guest slot pool it protects nothing. Rejected.
- **A control message from the guest owner reporting each release.** Adds a
  type, a round trip and a dependence on the control session for a fact the
  end record carries. Rejected.
- **Release a flow's quota place at half-close.** A guest that withholds
  credit would then hold node flow capacity beyond its quota (ADR-0183).
  Rejected.

## Consequences

- The owner releases each place exactly once: an end record is produced once
  per pair (ADR-0202), and a trailing reset after it changes nothing
  (ADR-0180).
- `shutdown_write` on a `GuestStream` works for every stream, a probe stream
  included: a cleanly ended probe stream holds no guest resource the host
  must wait for.
- `GUEST_PROBE_SLOTS` (8) and its deploy refusal bound the quota-exempt probe
  dials one allocation holds, a node-capacity bulkhead (ADR-0187, ADR-0057
  § 1).
- The release point is the host's and does not depend on which guest holds
  the other end (ADR-0207; a guest that runs no Linux kernel is
  [GH #321](https://github.com/overdrive-sh/overdrive/issues/321)).
- The Quint module `flow_quota` checks the release point (feature delta,
  § *Formal protocol model*).
