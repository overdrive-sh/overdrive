# ADR-0151 — One node-scoped guest-flow owner exclusively holds the host forwarder, all transport listeners and every host pair socket

## Status

**Proposed — decision D7 approved by user 2026-10-05, with its correction —
`sock_ops` identifies intake children by listener tag, not port (independent
DESIGN review finding M-1) — confirmed by user 2026-10-06; pending
independent DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*. The owner's cgroup
placement under the service-VIP rewrite is ADR-0164. Object lifetime is
ADR-0159; stop and restore semantics are ADR-0160.

## Context

ADR-0114 created one shared guest-network owner per server, constrained by the
bounded recovery of ADR-0124 and the activation ordering of ADR-0131. Those
ownership and recovery rules do not depend on the bridge; their component
lists and containment primitive do.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`) ran the host
side as one owner process that held, in the physical host kernel:

- SK_SKB verdict and strparser programs on its own maps;
- a TCX program on host `lo` that is a no-op unless a datagram tuple is
  registered;
- an `fexit(skb_send_sock)` program that only counts sent bytes per socket;
- a `sock_ops` program on a cgroup that held only the owner process, which
  installed host intake children at `PASSIVE_ESTABLISHED` and armed
  destination sockets at `ACTIVE_ESTABLISHED`;
- host parking cells for host intake children.

Every run restored the host's program count, links, `lo` TCX state, root
cgroup programs and module set. One harness bug shows the drain counters must
be exact: LRU counter maps evicted a long-lived cell's entry and stalled one
half-close of 10,000.

## Decision

- **One owner, one role.** The node-scoped guest-flow owner in
  `overdrive-control-plane` keeps the existing `SharedGuestNetworkOwner` /
  `GuestNetworkProvisioner` role.
- **Exclusive ownership.** The owner alone holds:
  - the host forwarding programs, maps and links (verdict, strparser, the
    egress unframe program on `lo` and every root-namespace interface
    (ADR-0165), `sock_ops`, drain counter);
  - the host `AF_VSOCK` flow listeners and the per-VM control sessions;
  - the per-allocation intake listeners (ADR-0152);
  - the host parking cells;
  - every host-side pair socket.
- **No other writer** inserts into or removes from the forwarding maps.
- **`sock_ops` scope.** The host `sock_ops` program attaches to the owner
  process's own cgroup, never the root cgroup. That cgroup is shared with the
  rest of `overdrive serve` (ADR-0164), so the program identifies its sockets
  by identity, never by port or address:
  - an active socket is acted on only if the owner armed that socket (by its
    socket cookie) before `connect()`;
  - a passive child is acted on only if its listener is one of the owner's
    intake listeners. The owner tags each intake listener with a socket-local
    storage entry created with the clone flag, so every child accepted on it
    carries the tag from the moment it is created; the program checks the
    child's tag at `PASSIVE_ESTABLISHED`.

  Every other socket of the process — leg-C's transparent children, whose
  local address and port equal an intake's, the API server, leg-F and leg-S —
  is untouched. If the owner's enclosing cgroup is the root cgroup, the owner
  refuses to start.
- **Non-blocking owner I/O.** Every owner `connect()` is non-blocking
  (ADR-0158); the owner's descriptor budget is checked at startup and covers
  bursts of host-local connects such as probe floods.
- **Exact drain accounting.** Drain counters are exact per socket: non-evicting
  maps sized from the pair capacity, with entries deleted when the socket
  closes.
- **Per-allocation lifecycle.** A CID moves through Provisioned → Active →
  Retiring → released, driven by provision, activate and teardown together
  with the admission lease (ADR-0132/0133).
- **Recovery is unchanged.** ADR-0124's cadence, EXEC gate, bounded retry and
  fail-stop are retained; its containment primitive is ADR-0160's.

## Alternatives considered

- **A per-allocation owner task.** Multiplies tasks by VM count and splits
  authority over one shared map set. Rejected.
- **Put the forwarding owner inside the mTLS worker.** Couples transport and
  enforcement, which meet through a narrow port instead (ADR-0153). Rejected.
- **Attach host `sock_ops` to the root cgroup.** Every host TCP socket would
  run the program. Rejected.
- **Identify intake children by local port (as the spike did, with an
  owner-only cgroup).** In the shared `serve` cgroup a leg-C transparent child
  has the same local address and port as an intake child and would be
  installed by mistake. Rejected.
- **Run the owner in its own cgroup.** Needs a separate process or a threaded
  cgroup subtree, which conflicts with the in-process owner (ADR-0164) and
  the domain controllers of `control-plane.slice` (ADR-0028). Rejected.
- **LRU counter maps.** Eviction stalls half-close (spike). Rejected.

## Consequences

- Supervisor components change: bridge, TAP TCX, pins and bridge guard are
  removed; forwarder, `sock_ops` link, drain counter, flow and control
  listeners, intake listeners, the egress unframe links and the local route
  are added.
- Fail-stop and EXEC-gate semantics are unchanged. The 1 s audit and 5 s bound
  are re-measured (V-9).
- The owner's per-flow state machine is tested through a private effect seam
  under seeded simulation, following `GuestNetworkAllocationIo`
  (D-295-DISTILL-12A); kernel effects are Tier-3 evidence.
- The host kernel carries Overdrive programs on the egress of `lo` and every
  root-namespace interface, on the owner's cgroup, and on `skb_send_sock`.
  Each is a link the owner holds (ADR-0159).
- Clone-flagged socket storage inheritance at accept is a kernel facility the
  spikes did not exercise; it is validation item V-20.
