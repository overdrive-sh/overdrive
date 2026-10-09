# ADR-0178 — An intake listener binds its workload address on a port of a fixed host block; the steering entry carries the declared port

## Status

**Accepted — decision INTAKE-BIND approved under the user's 2026-10-09
remediation directive**, answering finding F-1 of validation item V-26. GH
#295. Recorded in the #295 feature delta, § *[REF] vsock Attachment
Replacement DESIGN* (§ *Core vocabulary*, intake host ports; § *Spike
remediation decisions — 2026-10-09*). Refines ADR-0173 (intake listeners) and
ADR-0171 (the steering's listener-keyed map).

## Context

An intake listener (ADR-0173) serves one declared `workload_addr:port`.
Binding it to exactly that address cannot work on a real node (V-26, feature
delta P-40, `spike/findings-v26.md`): `bind(workload_addr:p)` fails
`EADDRINUSE` while a host listener holds `0.0.0.0:p`, even with address and
port reuse and `IP_FREEBIND` set on the binding side. A VM declaring the port
of a host service would never be served; and a listener bound first would
keep that host service from binding.

The steering does not need the exact bind. The `sk_lookup` program assigns a
connection to the listener its `(address, port)` entry names whatever that
listener is bound to (V-26 assigned listeners bound to `workload_addr:0` and
to `127.0.0.1:34567`), and binds of one port on distinct specific addresses
coexist (V-26 filled 262,144 listeners as 5 ports × 52,429 prefix addresses).

Letting the kernel choose the port (`bind(workload_addr:0)`) draws it from
the ephemeral range. The kernel keeps bind buckets per port number,
node-wide, and an outbound connect's port search skips every port whose
bucket an explicit bind created. Up to 262,144 intake listeners would hold
buckets on most of the range and starve the host's own outbound connects.

## Decision

- **A fixed host block.** Intake listeners bind ports of one block,
  `20,480–24,575`, below the kernel's default ephemeral range.
- **A pure rule.** The listener for the declared port at index `i` of the
  allocation's distinct declared TCP ports, in ascending order, binds
  `workload_addr:(20,480 + i)`. Indices are distinct within an allocation
  and allocations bind different addresses, so the rule needs no allocator
  and cannot race.
- **Steer by the declared port.** The steering entry is `(workload_addr,
  declared port) → listener`. The owner records which declared port each
  listener serves; the bound port is never used as the declared port.
- **Boot check.** `serve` refuses to start when the block meets the host's
  ephemeral range or a TCP port its configuration binds on the unspecified
  address.
- **No late wildcard binds.** Every explicit-port TCP bind to the unspecified
  address happens before boot opens; none happens afterwards.

## Alternatives considered

- **Bind the declared port.** The F-1 collision. Rejected.
- **Bind `workload_addr:0`.** Starves host outbound connects at density, as
  above. Rejected.
- **Bind `127.0.0.1:0`.** The same, and a listener a direct loopback connect
  reaches without the steering deciding. Rejected.
- **An owner-run port allocator.** State and a claim protocol where a pure
  function of the lease's declared ports suffices. Rejected.

## Consequences

- A VM may declare any port a host service also uses; each keeps serving its
  own clients.
- A connection to a workload address reaches an intake only through its
  steering entry; a connect to a listener's block port is decided like any
  other prefix lookup.
- One workload can hold at most as many intake listeners as the block has
  ports; ADR-0179 bounds the declared ports to match.
- Accepted intake children hold `workload_addr:<declared port>` and so a
  bind bucket on that port number, as leg-C's transparent children already
  do; the number of such port numbers is bounded by the distinct declared
  ports on the node.
- The kernel facts are proven by V-26; V-9 measures intake binds and host
  outbound port availability at density.
