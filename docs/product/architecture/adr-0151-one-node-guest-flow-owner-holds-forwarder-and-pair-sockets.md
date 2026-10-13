# ADR-0151 — One node-scoped guest-flow owner exclusively holds the host forwarder, all transport listeners and every host pair socket

## Status

**Accepted — decision D7 approved by user 2026-10-05; its scope under INB-1
(the owner opens host-initiated flows for host platform clients) and the
flow-end and drain programs (R2-END, R2-DRAIN) approved by user
2026-10-11.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN*. The owner's cgroup placement under the
service-VIP rewrite is ADR-0164. Object lifetime is ADR-0159; stop and restore
semantics are ADR-0160; quiescence is ADR-0169.

## Context

ADR-0114 created one shared guest-network owner per server, constrained by the
bounded recovery of ADR-0124 and the activation ordering of ADR-0131. Those
ownership and recovery rules do not depend on the bridge; their component
lists and containment primitive do.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`) ran the host
side as one owner process that held SK_SKB verdict programs on its own maps, a
`sock_ops` program on a cgroup holding only the owner process, and a counter
on `skb_send_sock`. Every run restored the host's program count, links, root
cgroup programs and module set. One harness bug showed that per-flow drain
state must be exact: an LRU counter map evicted a long-lived entry and stalled
one half-close of 10,000.

Under INB-1 a host platform client that reaches a VM port asks the owner to
open a flow toward the guest
(`GuestStreamDial`, ADR-0173) and holds the host end itself. So every host
socket the owner's `sock_ops` program must act on is an active socket the
owner itself connects.

## Decision

- **One owner, one role.** The node-scoped guest-flow owner in
  `overdrive-control-plane` keeps the existing `SharedGuestNetworkOwner` /
  `GuestNetworkProvisioner` role and implements the driving port
  `GuestStreamDial` for host platform clients (ADR-0173).
- **Exclusive ownership.** The owner alone holds:
  - the host forwarding programs, maps and links: the SK_SKB verdict programs
    and their socket maps, `sock_ops`, the flow-end classifier on vsock
    packet receipt (ADR-0180), the drain programs and the per-flow drain cells
    (ADR-0183), and the flow-event ring buffer with its drop counters;
  - the host `AF_VSOCK` flow listeners, the control listener and the per-VM
    control sessions (ADR-0166);
  - every host-side pair socket, and every host vsock it opens toward a guest
    for a dial until it hands that vsock to the dialer as its stream.
- **No other writer** inserts into or removes from the forwarding maps. The
  host end of a dialled flow is never installed in the forwarding maps; the
  dialer reads and writes it (ADR-0173).
- **`sock_ops` scope.** The host `sock_ops` program attaches to the owner
  process's own cgroup, never the root cgroup. That cgroup is shared with the
  rest of `overdrive serve` (ADR-0164), so the program identifies its sockets
  by identity, never by port or address: it acts on an active socket only if
  the owner armed that socket, by its socket cookie, before `connect()`. It
  acts on no passive socket. Every other socket of the process — leg-C's
  transparent children, the API server, leg-F and leg-S — is untouched. The
  owner's placement check (ADR-0164) admits only a cgroup at or below the
  `connect4` attach path, which excludes the root cgroup.
- **Non-blocking owner I/O.** Every owner `connect()` is non-blocking
  (ADR-0158); the owner's descriptor budget is checked at startup and covers
  bursts of host platform dials such as probe floods.
- **Exact drain state.** Per-flow drain state lives in non-evicting cells
  indexed by flow slot, sized from the pair capacity, each initialised with a
  new epoch when a flow takes the slot (ADR-0183).
- **Per-allocation lifecycle.** A CID moves through Provisioned → Active →
  Retiring → released, driven by provision, activate and teardown together
  with the admission lease (ADR-0132/0133). The lease is released only after
  the VMM has exited (ADR-0146).
- **Recovery is unchanged.** ADR-0124's cadence, EXEC gate, bounded retry and
  fail-stop are retained; its containment primitive is ADR-0160's stop under
  ADR-0169's counted hold.

## Alternatives considered

- **A per-allocation owner task.** Multiplies tasks by VM count and splits
  authority over one shared map set. Rejected.
- **Put the forwarding owner inside the mTLS worker.** Couples transport and
  enforcement, which meet through narrow ports instead (ADR-0153,
  ADR-0173). Rejected.
- **Attach host `sock_ops` to the root cgroup.** Every host TCP socket would
  run the program. Rejected.
- **Identify owner sockets by local port or address.** In the shared `serve`
  cgroup other sockets of the process can share an address and port with an
  owner socket. Rejected in favour of the armed cookie.
- **Run the owner in its own cgroup.** Needs a separate process or a threaded
  cgroup subtree, which conflicts with the in-process owner (ADR-0164) and
  the domain controllers of `control-plane.slice` (ADR-0028). Rejected.
- **LRU drain-state maps.** Eviction stalls half-close (spike). Rejected.

## Consequences

- Supervisor components change: bridge, TAP TCX, pins and bridge guard are
  removed; the forwarder, the flow listeners and the control listener are
  added. The forwarder's links (`sock_ops`, the flow-end classifier, the drain
  programs) are not components: they are held by the owner and not audited
  (ADR-0159). The guest gateway's host-local route (GW-LOCAL, ADR-0152) is
  written at boot only.
- Fail-stop and EXEC-gate semantics are unchanged. The 1 s audit and 5 s bound
  are re-measured (V-9).
- The host kernel carries Overdrive programs on the owner's cgroup, on vsock
  packet receipt, on `skb_send_sock` and on `sock_poll`. Each is a link the
  owner holds (ADR-0159).
