# ADR-0163 — A host intake listener exists only while the guest application listens on that port, mirrored from a guest kernel listener map

## Status

**Proposed — decision D23 approved by user 2026-10-05 (conditional on V-14,
since proven) with its mechanism, the kernel-map listener set, approved by
user 2026-10-06; port scope (the guest reports every listening port, the host
acts only on declared ports) approved by user 2026-10-06; its correction —
quiescence closes intake listeners, and each allocation's events are applied
in one serialized order (independent DESIGN review findings M-7, H-2) —
confirmed by user 2026-10-06; a listener whose firewall element cannot be
removed stays bound until it is (D8a-REVOKE, formal model finding 1), and
activation binds all reported listeners or none (U-6), approved by user
2026-10-06; a wanted port's element is asserted at the start of every serving
period and a wanted port never waits on a pending removal, and firewall-rule
repair runs only inside the repairing recovery's quiescence hold
(D8a-REASSERT; model findings r2-2 and
r3 item 3), approved by user 2026-10-06 (direction: correct design over
simple); pending independent DESIGN review.** GH #295. Recorded in the
#295 feature delta, § *[REF] vsock Attachment Replacement DESIGN — PROPOSED
2026-10-05*.

It follows from ruling **D15 (APPROVED 2026-10-05)**: guests keep today's
network behaviour wherever the kernel allows it.

## Context

Under ADR-0152 the host terminates every host-local TCP connection to
`workload_addr:port`: the host kernel completes the handshake on the intake
listener, and only then does the owner ask the guest to connect to the
application. If the guest application is not listening, the guest refuses and
the owner resets the already-established intake connection.

Two existing behaviours depend on the difference:

- **TCP readiness and liveness probes.** The probe runner's TCP mechanic
  (`crates/overdrive-worker/src/probe_runner/`) declares success when
  `connect()` to `workload_addr:port` succeeds. With an always-bound host
  intake, `connect()` succeeds whenever the allocation is Active, so a TCP
  probe reports healthy for an application that is not listening. Service
  `Stable` and backend eligibility would follow a false signal.
- **Host-local platform clients** (leg-S after mTLS termination, marked
  probes) see an established-then-reset connection where today they see a
  refusal.

Remote clients already terminate at leg-C first, so for them an
established-then-reset outcome is today's behaviour.

The V-14 spike (`spike/v11-vip-v14-findings.md`, increments e–h; stock
7.0.0-29 guest; one VM) compared three ways for the guest to learn listen
state without reading payload:

- **State carried in ring-buffer events.** Lag p99 1.4 ms, but after 2,000
  rapid listen/close cycles the ring overflowed, stop events were lost and the
  host listener stayed bound (stale). Intermittently wrong under churn.
- **Re-derive from a `sock_diag` dump on every wake.** Correct in later runs,
  but one run lagged by exactly one transition for unexplained reasons, and
  `/proc/net/tcp` re-derivation took 5–11 ms per wake.
- **Kernel map.** `fexit` on `inet_csk_listen_start` (return 0) and
  `inet_csk_listen_stop` maintain a BPF hash map of listeners reachable at the
  workload address (`socket cookie → port`); the ring buffer only wakes the
  owner. Lag p50 0.23 / p99 1.39 ms up and 0.19 / 1.27 ms down over 180
  untraced cycles, 0 missing; correct after 4,000 + 4,000 churn cycles with
  8,828 ring records lost; `SO_REUSEPORT` churn never flapped the host
  listener; kill, terminate and restart of the application and of either
  owner re-synchronised; only control-sized socket I/O crossed the guest owner.

The spike also found two guest requirements: `sock_diag` dumps need
`inet_diag` and `tcp_diag` in the guest image (without them every dump
failed and, wrongly treated as empty, no listener ever appeared), and the
guest refusal toward a transparent client address needs
`net.ipv4.fwmark_reflect=1` to be delivered at once rather than after a 3 s
timeout.

## Decision

- **Guest listener set lives in a kernel map.** Guest `fexit` programs on
  `inet_csk_listen_start` (successful return only) and `inet_csk_listen_stop`
  insert and delete `socket cookie → port` in a guest BPF hash map for every
  TCP listener reachable at the workload address (bound to `0.0.0.0` or to the
  workload address; IPv6 stays disabled in the guest, #308). The guest's own
  reserved TCP intake listener (ADR-0150) is never inserted: the programs and
  the seeding skip its port. A ring-buffer record is only a wake signal;
  losing one never loses state.
- **Seeded at owner start.** `overdrive-init` seeds the map once per start
  from `NETLINK_SOCK_DIAG` listener dumps (insert without overwriting, then
  delete seeded entries missing from a second dump), so listeners that exist
  before it starts are included. A failed dump is a setup failure, never an
  empty result.
- **Reported on the control session.** The guest owner reads the map on each
  wake and at a level-triggered audit, and sends an 8-byte `ListenState` for a
  port only when its state changes. A control session opens with every port
  treated as not listening; the guest then sends the full state — every port
  in its map as listening (ADR-0166). The guest does not need the declared
  port list; the host acts only on declared ports.
- **Host mirrors exactly.** The owner holds the intake listener for a declared
  `(allocation, port)` while the allocation is Active, node forwarding is not
  quiesced, **and** the guest's last report for that port says listening, and
  closes it when any of the three stops being true; connections already
  accepted continue unless the cause also aborts them (quiescence and session
  loss do). On control-session loss it closes every intake listener of that
  allocation, and the allocation's flows are aborted (ADR-0166).
- **A wanted port's element is asserted at the start of every serving
  period, and a wanted port never waits on a removal.** Every time a port
  becomes wanted — at bring-up, after a restore, or while its take-down is
  pending — the owner asserts its firewall element present: a new listener is
  admitted; a listener that still holds its admission (its removal was
  pending) re-asserts it with an idempotent add whose success means the
  element is present in the kernel now, and its pending removal is dropped. A
  failed assertion is retried at the audit cadence until it succeeds or the
  port stops being wanted. Meanwhile the listener pairs every connection it
  accepts: a connection reaches it only through an element that is present,
  so pairing it is always correct, and no serving period relies on an element
  from an earlier one (a foreign table deletion may have removed it).
  Activation keeps its stronger postcondition (all or nothing, below).
- **Repair inside the recovery's hold.** Every repair of the shared firewall
  rules runs while the repairing recovery holds forwarding quiescence
  (ADR-0169): quiesce, repair, audit, restore. Restore reopens forwarding only
  when no holder remains, and then brings back, from the last reported state,
  every listener that should serve, under the rule above.
- **Reachability follows the listener.** The owner adds the listener's
  firewall element only after it listens and removes it before closing it
  (ADR-0152), so a connection reaches a host listener only while one exists.
  When the element's removal fails, the listener stays bound, resets every
  connection it accepts, and the removal is retried at the audit cadence; the
  listener closes only once the element is gone. A bind or admission failure
  leaves no element and is retried at the audit cadence while the guest still
  reports listening.
- **Activation is all or nothing.** Activation marks the allocation Active
  together with a bound, admitted listener for every declared port the guest
  currently reports listening, or returns an error and leaves the allocation
  Provisioned; the listeners it bound are taken down by the same removal
  rule. After activation, a listener that fails to come up is retried and never
  fails the allocation.
- **One writer per allocation.** Activation, teardown, quiescence, restore,
  `ListenState` handling and control-session open/loss for one allocation are
  applied by the owner in one serialized order; activation binds from the
  `ListenState` snapshot current at that point in the order.
- **Lag bound: ≤ 2 ms** from the application's `listen()` / `close()` returning
  to the host listener being bound / closed, at the load profile the feature
  delta pins (measured p99 1.39 ms on one VM, including a 1 ms harness poll).
  Whether it holds at density is measured (V-9); a miss there is surfaced for
  a user decision, not silently relaxed.
- A host-local connect to a port with no guest listener is therefore refused
  on the host, as it was refused by the guest kernel under the bridge
  topology, and a TCP probe fails until the application listens and while
  forwarding is quiesced.

## Alternatives considered

- **State carried in ring-buffer events.** Stale after ring overflow under
  churn. Rejected.
- **Re-derive from a `sock_diag` dump per wake.** A dump per wake, and an
  unexplained one-transition lag in one run. Rejected.
- **Always bind at activation, and change TCP probes on VM ports to a typed
  pairing check through the owner.** Changes the probe runner's contract for
  one workload kind and leaves leg-S with established-then-reset. Rejected.
- **Always bind at activation and accept the difference.** TCP probes would
  pass while the application is down. Rejected.
- **Delay the handshake until the guest confirms.** An ordinary listening
  socket cannot defer `SYN-ACK` on a per-connection decision. Rejected.
- **Let a kept-bound listener serve again on its existing element.** If other
  software deleted the firewall table meanwhile, the element is gone and
  nothing re-adds it: the listener serves with no element, and marked clients
  (leg-S, probes) are reset until teardown (`spike/quint-owner-findings-r2.md`,
  finding r2-2). Rejected.
- **Complete the pending removal, then admit again.** Serves only on a fresh
  element, but ties the wanted port to the unwanted operation: while the
  removal keeps failing, the port the guest is listening on stays resetting
  after the restore. The removal is not needed for the port to serve; the
  idempotent re-assertion is. Rejected.
- **Let the firewall-rule repair re-derive the elements from the owner's
  admitted listeners.** A second writer of the elements outside the owner's
  per-allocation order. Rejected.

## Consequences

- One control message type, two guest `fexit` programs, one guest map and a
  ring buffer are added.
- A listener is bound and closed as the guest application starts and stops
  listening; the number of bound listeners never exceeds the declared ports.
- Between the application's `listen()` and the host bind there is a window of
  at most the lag bound in which a connect is refused; under the bridge
  topology the same connect would have succeeded slightly earlier.
- Guest requirements: `fexit` with BTF; `CONFIG_INET_DIAG` and
  `CONFIG_INET_TCP_DIAG` present in the guest image; the `sock_common` field
  offsets the programs read pinned to the guest kernel and checked against its
  BTF at load; `net.ipv4.fwmark_reflect=1`.
- The listen-state path never shares a loop with a blocking call (ADR-0158).
- A listener kept bound across a quiescence serves again at the restore,
  whatever the state of its earlier removal; its element is re-asserted and
  the assertion retried until it succeeds. While an assertion fails, marked
  clients are reset by the firewall (fail-closed) and nothing else changes.
- Proven on one VM on 7.0.0-29 only.
