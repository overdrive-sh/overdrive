# ADR-0151 — One node-scoped guest-flow owner exclusively holds the host forwarder context, all transport listeners and every host pair socket

## Status

**Accepted.** Decision D7 approved by the user 2026-10-05; its scope under
INB-1 (the owner opens host-initiated flows for host platform clients) and
the flow-end and half-close rules (R2-END, R2-DRAIN) approved 2026-10-11;
the host forwarder as the in-kernel forwarder (ADR-0197, EVMOD-ADOPT)
approved 2026-10-16; its control context (EVMOD-CTL, ADR-0199) and the
guest-neutral host side (ADR-0207) approved 2026-10-17. GH #295. Recorded in
the #295 feature delta, § *[REF] Owner and provisioner* and § *[REF] vsock
data path — the in-kernel event-driven forwarder (EVMOD)*, "Owners' use of
the forwarder". The owner's cgroup placement under the service-VIP rewrite
is ADR-0164. Object lifetime is ADR-0159; stop and restore semantics are
ADR-0160; quiescence is ADR-0169. Not yet implemented.

## Context

ADR-0114 created one shared guest-network owner per server, constrained by the
bounded recovery of ADR-0124 and the activation ordering of ADR-0131. Those
ownership and recovery rules do not depend on the bridge; their component
lists and containment primitive do.

The host forwarder of ADR-0197 is reached through a control context, one open
file of the module's device; every pair attached through it belongs to it,
and its release aborts them (ADR-0199). One owner per side is what makes that
release the owner's fail-closed death.

Under INB-1 a host platform client that reaches a VM port asks the owner to
open a flow toward the guest (`GuestStreamDial`, ADR-0173) and holds the
host end itself.

## Decision

- **One owner, one role.** The node-scoped guest-flow owner in
  `overdrive-control-plane` keeps the existing `SharedGuestNetworkOwner` /
  `GuestNetworkProvisioner` role and implements the driving port
  `GuestStreamDial` for host platform clients (ADR-0173).
- **Exclusive ownership.** The owner alone holds:
  - the host forwarder context, by value (ADR-0199), through which it
    attaches every host pair and watches every dialled vsock;
  - the host `AF_VSOCK` flow listener, the control listener and the per-VM
    control sessions (ADR-0166);
  - every host-side pair socket, and every host vsock it opens toward a guest
    for a dial until it hands that vsock to the holder as its stream.
- **No other attacher.** No other component opens a forwarder context or
  attaches a pair on the host. The host end of a dialled flow is watched, not
  forwarded; the holder reads and writes it (ADR-0173).
- **Guest-neutral.** The owner pairs, attaches, watches and releases flows
  the same way for every guest; it never branches on guest type (ADR-0207).
- **Non-blocking owner I/O.** Every `connect()` either owner issues while
  pairing is non-blocking and completes on readiness: a blocking connect
  stalls every other flow of the same owner (spike `v11-vip-v14`, increment
  c). The owner's descriptor budget is checked at startup and covers bursts
  of host platform dials such as probe floods.
- **The owner learns each flow's end from its end record** (ADR-0202) and
  releases the flow then (ADR-0209).
- **Per-allocation lifecycle.** A CID moves through Provisioned → Active →
  Retiring → released, driven by provision, activate and teardown together
  with the admission lease (ADR-0132/0133). The lease is released only after
  the VMM has exited (ADR-0146).
- **Recovery is unchanged.** ADR-0124's cadence, EXEC gate, bounded retry and
  fail-stop are retained; its containment primitive is ADR-0160's stop under
  ADR-0169's counted hold. The `Forwarder` component's repair closes the old
  context before it opens and probes the new one (ADR-0199).

## Alternatives considered

- **A per-allocation owner task.** Multiplies tasks by VM count and splits
  authority over one forwarder context. Rejected.
- **Put the forwarding owner inside the mTLS worker.** Couples transport and
  enforcement, which meet through narrow ports instead (ADR-0153,
  ADR-0173). Rejected.
- **Several contexts on the host, one per allocation.** Multiplies descriptors
  and readiness sources and makes the owner's death a sequence of releases
  rather than one. Rejected.

## Consequences

- Supervisor components change: bridge, TAP TCX, pins and bridge guard are
  removed; the forwarder context, the flow listener and the control listener
  are added. The guest gateway's host-local route (GW-LOCAL, ADR-0152) is
  written at boot only.
- Fail-stop and EXEC-gate semantics are unchanged. The 1 s audit and 5 s bound
  are re-measured (V-9).
- The host kernel carries the forwarder module (ADR-0198) and the owner's one
  context; no BPF forwarding program is held on the host.
