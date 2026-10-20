# ADR-0216 — The host forwarder watches a dialled vsock so its stream keeps the classified end

## Status

**Accepted — the watch clause of EVMOD-END, approved by the user
2026-10-17**, with the spike correction of EV-PINS approved the same day (a
dropped `GuestStream` aborts its watch). GH #295. Not yet implemented.
Contract: the #295 feature delta, § *[REF] vsock data path — the in-kernel
event-driven forwarder (EVMOD)*, "Watch", and § *[REF] Driving port — guest
stream dial `GuestStreamDial`*, "How an end is resolved".

Related decisions, each separately reversible: the dialer holds the host end
of a host-opened flow and the forwarder never pairs it (INB-1, ADR-0214);
the end classes and their computation in the forwarder's callbacks
(ADR-0180); the end record (ADR-0202); what an owner's abort of a watch does
(WATCH-ABORT, ADR-0211); how the stream orders the owner's end, the record
and the wait for an in-flight record, and the record queued before the
holder's wakeup (EVMOD-STREAM-END, feature delta); how an owner abort reaches
the stream and the guest (EVMOD-STREAM-ABORT, feature delta).

## Context

A dialled flow's host vsock, `V_h`, is read and written by its dialer
(leg-S or the VM probe runner) and is not part of a forwarder pair
(ADR-0214). The dialer still needs the class of the guest's end of its
direction — a clean half-close or an abort — to report it to its own peer:
leg-S turns an abort into a reset toward the mesh client, and an HTTP probe
must not read a truncated response as complete.

The vsock socket API does not give that class. An established-state RST runs
`virtio_transport_do_close`, which sets `SOCK_DONE` and the peer's shutdown
mask but no socket error (`net/vmw_vsock/virtio_transport_common.c:1244-1252`,
`:1463-1464 @ v7.0`), so `read()` returns 0 for a guest reset as for a clean
half-close (V-28: 192 of 192; an aborted dial read EOF 2,000 of 2,000, P-69).
The classification exists only in the forwarder's callbacks, which see each
SHUTDOWN and RST as it arrives (ADR-0180).

## Decision

- **The owner watches every dialled vsock on its forwarder context before it
  writes the flow's request**, so no end of the guest's direction precedes
  the watch; an end present when the watch is installed is classified once
  from the attach-time state (ADR-0180).
- **A watch overrides only `sk_state_change` and `sk_error_report`** on
  `V_h`, classifies the guest's end by ADR-0180's table, and yields one end
  record (ADR-0202). It moves no bytes and changes none of the holder's
  wakeups.
- **The stream takes its end from the owner's own end of the watch or from
  the watch's record, never from its `read`.**
- **Dropping the stream aborts its watch, then closes `V_h`**, which the
  guest sees as an abort with a direction open.

## Alternatives considered

- **Leave `V_h` unwatched and classify from the stream's own `read`.** A
  guest reset reads as a clean EOF (V-28, P-69), so a truncated response
  would pass as complete. Rejected.
- **A `HalfClose` message on the control session for dialled flows.** Carries
  a fact the kernel already records, at a control round trip per end, and
  needs an end on the control session ordered against the last bytes on the
  flow's own connection, an ordering at the host that is unproven. Rejected.
- **Keep the BPF raw-tracepoint classifier and its ring for dialled
  vsocks.** A second end mechanism beside the forwarder, with ring loss and
  recursion skips (EVMOD-END alternative (a)). Rejected.

## Consequences

- A dialled flow costs one watch on the owner's forwarder context and one
  end record; its places are released at the stream's drop or abort
  (ADR-0209).
- The watch is the host's and the same for every guest (ADR-0207).
- A watch can end three ways: its override on the guest's classified end, an
  owner's abort (ADR-0211), or the context's release (ADR-0199). Each is
  covered by the late-callback and id-table rules of ADR-0204.
- Measured on Linux 7.0: the record was present at the holder's observation
  6,640 of 6,640 times, 640 of them on the orphan-sweep path (P-69).
