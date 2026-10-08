# ADR-0171 — Local delivery to the guest prefix is decided by a pinned socket-lookup program over a listener-keyed socket map

## Status

**Proposed — decision D8a-LOOKUP approved by user 2026-10-07; pending
independent DESIGN review.** GH #295. Recorded in the #295 feature delta,
§ *[REF] vsock Attachment Replacement DESIGN — PROPOSED 2026-10-05*. It is the
mechanism of ADR-0172 (only steered intake listeners are reachable at the
guest prefix). How each boot replaces the program is ADR-0175; how admission
keeps room for every entry is ADR-0176. The exact contract is pinned in the
feature delta (§ *Driven port — guest-prefix steering*).

## Context

ADR-0152 makes the guest prefix host-local through one `local` route, so the
host kernel delivers a connection to `workload_addr:port` to whatever socket
matches it — including a wildcard-bound host service. Something must decide,
for every connection to the prefix, that it reaches only the intake listener
of that `(address, port)` or nothing.

If that decision lives in a firewall element separate from the listener, the
element and the listener are two kernel objects that cannot change in one
step. Fail-closed order then forces a stuck state: when the element cannot be
removed, the listener must stay bound (or connections fall through to a
wildcard host service), so a client sees `connect()` complete on a port the
guest no longer serves, and teardown and lease release must wait for a kernel
write that may keep failing. The decision also lives in the shared firewall
table, which the mTLS worker converges and which a failure of that worker
part-way through a convergence (or a `serve` crash between its batches) can
leave partial; while it is, wildcard host services on declared ports are
reachable until ADR-0124's repair runs, or until the next boot if `serve` is
down. The formal model of the guest-flow owner reached the stuck-listener
state (`spike/quint-owner-findings*.md`).

The kernel already offers a decision point that has neither defect. A
`BPF_PROG_TYPE_SK_LOOKUP` program attached to a network namespace runs when
the kernel looks up the receiving socket for a locally delivered TCP SYN or
UDP datagram that carries no socket yet, before the listener and wildcard
lookups; it may assign a socket from a socket map, drop (TCP is then answered
with a reset, UDP with port-unreachable), or pass to the normal lookup. A
socket map entry is removed by the kernel when its socket closes. A packet
whose socket was already assigned in prerouting (nft TPROXY, leg-C) does not
reach it.

## Decision

- **One steering program per host network namespace.** An Aya
  `sk_lookup` program, attached to the host's root network namespace through a
  BPF link, decides local delivery for every destination in the guest prefix
  except the guest gateway (the DNS address, ADR-0154). Destinations outside
  the prefix, and the gateway, pass to the normal lookup untouched.
- **The listener is the steering decision.** The program reads one socket map
  keyed by `(workload_addr, port)` whose value is the intake listener for that
  port. For a TCP connection arriving on the loopback interface (host-local
  clients: leg-S, probes) whose key has an entry, it assigns that listener.
  Every other lookup for the prefix is dropped: a key with no entry, any UDP,
  and any connection arriving on another interface (remote traffic reaches
  intakes only through leg-C, whose TPROXY assignment precedes the lookup).
- **An entry exists only while its listener is open.** The owner inserts the
  entry after the listener listens; the insertion's success means the entry is
  present. The owner never deletes an entry: closing the listener removes it in
  the kernel, in the same step. Admission keeps room in the map for every
  listener's entry (ADR-0176); an insertion that fails leaves the listener
  unreachable (connections to the port are refused).
- **Node infrastructure, pinned.** The link is pinned under
  `/sys/fs/bpf/overdrive/guest_prefix_steering/` and is never detached or
  unpinned by Overdrive: it outlives `serve` exactly as the `local` route
  does (ADR-0152), and keeps its program and that program's map alive. When
  `serve` exits, its listeners close, the map holds no live socket, and every
  lookup for the prefix drops — the prefix is fail-closed while `serve` is
  down without any firewall rule.
- **No runtime audit.** On the appliance only Overdrive writes the steering
  (ADR-0068). At runtime the owner changes it only by `steer` (whose `Ok`
  means the entry is present) and by closing listeners (whose entries the
  kernel removes in the same step); both outcomes reach the owner directly.
  There is therefore no runtime audit, re-steer or repair of the steering:
  there is no drift for one to find. The program in the link changes only at
  boot (ADR-0175).

## Alternatives considered

- **A firewall set of admitted `(address, port)` elements, added after listen
  and removed before close.** A failed removal
  forces the listener to stay bound, so a probe sees `connect()` complete on a
  port no longer served, and teardown and lease release wait on a failing
  kernel write; and the decision depends on a table that an mTLS worker
  failure can leave partial. Both are states a different contract removes.
  Rejected.
- **A constant firewall rule using the nft `socket` expression (accept only
  connections whose looked-up socket is a non-wildcard socket carrying the
  intake mark).** Removes the separate element, but the rule's socket lookup
  and the stack's delivery lookup are two lookups: a listener that closes
  between them lets the SYN reach a wildcard socket, so the check is not
  atomic with delivery. The decision also still lives in the shared table.
  `sk_lookup` decides at the delivery lookup itself. Rejected.
- **No `local` route; intake listeners reached by TPROXY only.** Leg-S and
  probes are exempt from TPROXY by design (ADR-0152). Rejected.
- **Unpinned program owned by `serve` (ADR-0159 discipline).** The route
  outlives `serve` (U-4) and the program would not: from every `serve` exit
  until the next boot attaches a program, the prefix is local with no lookup
  decision, guarded only by the constant firewall rules, which an mTLS worker
  failure or a crash between batches can leave partial. Rejected.
- **One node-shared intake listener for all addresses, assigned by the
  program.** Would collapse D23's per-port listener mirroring and D7's
  listener-tag identity of intake children. Rejected.
- **Audit the link and its entries at runtime and repair them.** On the
  appliance nothing but the owner writes them, and every runtime write
  reports its outcome to the owner; an audit could detect only software that
  does not exist on the appliance. Rejected.

## Consequences

- No connection to the guest prefix ever reaches a host socket other than an
  open intake listener or leg-C, whatever state the shared firewall table is
  in. What a partial table still affects (leg-C interception, the D26 output
  rule) is ADR-0124's existing recovery of the mTLS intercept rules,
  unchanged by this feature.
- Closing an intake listener ends its reachability in the same kernel step.
  There is no stuck listener, no pending removal, no re-assertion, and
  teardown, quiescence and activation rollback cannot fail on steering; lease
  release never waits on it.
- The firewall carries no per-listener state. Its constant output reject and
  prerouting drop of prefix traffic that is neither exempt nor diverted
  (ADR-0172) keep an unmarked host-local client on leg-C or refused, never
  handed an intake directly.
- Pinned objects are an exception to ADR-0159, which governs forwarding
  objects; the steering program forwards nothing and must outlive `serve` for
  the same reason the route does.
- The design assumes that only Overdrive writes the link, the map and the
  route, which the appliance image guarantees (ADR-0068: no operator shell,
  no third-party daemon). Software that is not on the appliance is not a
  design driver; no audit or repair path exists for it.
- While the shared firewall table is partial (an mTLS worker failure), a
  host-local client that leg-C would intercept can reach the steered intake
  of a serving port directly — as under the bridge topology, never a host
  service or a port the guest does not serve — until ADR-0124's recovery of
  those rules.
- New kernel facts are assumed and validated (feature delta V-26): the
  lookup runs for loopback connections to a `local`-route address before the
  wildcard lookup; a drop yields a reset; a TPROXY-assigned packet bypasses
  it; a closing listener leaves the map, and frees its slot, in the same
  step; a pinned link keeps running with no process. The program's own
  correctness is established by the Tier-3 steering tests in CI on the
  pinned kernel (feature delta A-33). The steering rules are checked by the
  formal model (ADR-0168, `specs/quint/guest-flow-owner/`, modules
  `prefix_boot`, `intake`, `quiescence`, `shared_table` over the shared
  `prefix_landing` predicate).
- The program runs for every TCP SYN and UDP datagram in the host root
  namespace whose receiving socket is looked up, not only for the prefix; it
  passes a non-prefix destination after one comparison. V-9 measures that
  per-lookup cost at density.
