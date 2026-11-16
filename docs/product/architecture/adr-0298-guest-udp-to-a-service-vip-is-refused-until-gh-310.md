# ADR-0298 — Guest UDP to a service VIP is refused until GH #310

## Status

Proposed. 2026-11-15. Author: Morgan (`nw-solution-architect`), DESIGN wave, APPLICATION
layer, feature `reconciler-convergence-guarantees`. The user approved the supersession
(feature delta D-10) and the refusal (D-11, option B) on 2026-11-15. Independent design
review is pending.

Supersedes the datagram clause and the startup check of
[ADR-0164](adr-0164-guest-vip-tcp-resolves-as-mesh-and-datagrams-use-the-connect4-rewrite.md).
ADR-0164's TCP-as-mesh decision (D24a) stays.

One ADR, not two: the refusal is the behaviour that replaces the removed clause. Removing
the clause without it is the rejected "no refusal" alternative below, so the two cannot be
adopted or reversed independently.

## Context

ADR-0164 says guest datagrams to a service VIP use the ADR-0053 `connect4` rewrite on the
guest-flow owner's connected host sockets, and its startup check refuses to start an owner
whose cgroup is not at or below the `connect4` attach path. [ADR-0297](adr-0297-the-same-host-local-backend-path-is-removed.md)
removes the rewrite, so the clause has nothing to stand on.

The clause never worked for a VM backend. Metal observation (Linux 7.0.0-38, commit
`fb71b400f`, three runs, `spike-scratch/guest-udp-vip-probe/evidence/run{7,8,9}/`): for a
Running UDP VM Service, `LOCAL_BACKEND_MAP` holds 0 entries while `SERVICE_MAP` holds the
VIP's TCP and UDP keys; guest UDP to the VIP gets no reply in 8 of 8 attempts; guest TCP to
the same VIP and port works (it resolves as mesh); `getpeername` on the owner's connected
host UDP socket after `connect` is still the VIP, so no rewrite occurs. The clause's only
prior evidence, the V-11 spike, used a host-local, non-intercepted backend, which production
never produces. The observation is evidence of current behaviour, not a choice between
mechanisms: the refusal below rests on the route-check facts that follow. The gap is tracked
by GH #310 (UDP service delivery to VM workloads).

Without the clause, a guest UDP datagram to a VIP reaches the owner's generic path. Three
facts decide what that path does:

- **Code:** the owner's route check (`owner_runtime.rs`, `datagram_associate`) accepts a
  `Unicast` route and then connects a host socket to the destination. Today a VIP-range
  destination skips this check by a branch this ADR removes.
- **Observed:** the VIP range (`10.96.0.0/16`) is a unicast route out of `ovd-veth-cli`
  (created by `veth_provisioner`; the netlink route dump in
  `spike-scratch/guest-udp-vip-probe/evidence/run9/`).
- **Kernel:** a route lookup result carries the type recorded when the route was installed
  (`vendors/linux` `net/ipv4/fib_trie.c:1604 @ v7.0`), so the check sees `Unicast` for that
  route. A host interface address is installed as a `/32` route of type `RTN_LOCAL`
  (`vendors/linux` `net/ipv4/fib_frontend.c:1156 @ v7.0`, in `fib_add_ifaddr`).

What the check does with a VIP therefore depends on the address:

- A VIP that equals a host address is a local route, not the unicast case. GH #358 records
  one collision: with the default ranges the first VIP, `10.96.0.2`, equals `ovd-veth-bk`'s
  address. The decision below does not depend on how the existing check treats it: the
  refusal covers every VIP-range destination.
- Every other VIP in the range is covered by the connected `10.96.0.0/16` unicast route
  out of `ovd-veth-cli` (observed: `spike-scratch/guest-udp-vip-probe/evidence/run9/evidence.log`,
  about lines 1717-1719). The datagram passes the check and is sent onto the host network
  toward an address nothing answers.

So any refusal by the existing route check is accidental and does not cover the VIP range; for
every other VIP the datagram leaves the host.

## Decision

1. The ADR-0164 datagram clause and its owner cgroup-placement startup check are removed.
   The guest-flow owner no longer treats a VIP-range destination as a datagram case that
   `connect4` rewrites, and no longer refuses to start when its cgroup is outside the
   `connect4` attach path.
2. Until GH #310 delivers guest UDP to a VIP, the owner refuses a guest UDP datagram whose
   destination is in the configured VIP ranges with the existing `FlowRefusal::Policy`,
   before any host socket is created or connected. It mirrors ADR-0164 D24a's fail-closed
   rule for TCP to an unmatched VIP-range address. When #310 lands it replaces the refusal.

Guest UDP to a VIP is a scope limitation of this design, tracked by #310, which requires
kernel-resident forwarding and no userspace relay and which reconciles ADR-0164's
guest-prefix UDP refusal. One option for #310 to evaluate is a service `connect4` that
selects from `SERVICE_MAP` / `BACKEND_MAP` as `cgroup_connect4_gateway` does; this ADR does
not choose it. A comment on #310 records the removal.

The exact behaviour is pinned in the
[feature delta](../../feature/reconciler-convergence-guarantees/feature-delta.md), D-11.

## Alternatives considered

| Option | Why it loses |
|---|---|
| Leave a guest UDP datagram to a VIP on the generic path (no refusal) | Per Context, the existing route check does not cover the VIP range: a datagram to a unicast-routed VIP is sent onto the host network, a silent timeout for the guest and a packet on the host network. The `Policy` refusal covers all VIP-range destinations and is an existing variant that adds no surface. |
| Keep the clause and the startup check | The clause names a rewrite ADR-0297 removes, and it never worked for a VM backend. |
| Keep only the startup check | It guarded only the rewrite. `run_server` enrols the process into `overdrive.slice/control-plane.slice` before the check runs, so no production boot reached its refusal. |
| Serve guest UDP to a VIP now | It needs kernel-resident forwarding (#310). Designing it here is outside this feature. |

## Consequences

Positive:

- A guest sending UDP to a VIP gets an explicit refusal instead of a silent timeout, and no
  datagram leaves the host.
- The owner loses a boot refusal and a post-connect peer check that existed only for the
  removed rewrite.

Negative and bounds:

- Guest UDP to a Service VIP is not served until #310.
- The removed post-connect kernel-peer check also failed closed against any `connect4`
  rewrite of an owner datagram socket into the guest prefix. After ADR-0297 no
  `connect4` program rewrites an owner socket: `cgroup_connect4_gateway`, which stays
  attached at an ancestor cgroup, returns allow without touching the context for any socket
  not registered in its intent map (`cgroup_connect4_gateway.rs:58-60`), and the owner's
  sockets are never registered there. The static destination policy still refuses
  guest-prefix UDP destinations.
- The removed placement check compared the owner's cgroup against the compile-time default
  attach path (`DEFAULT_CGROUP_ATTACH_PATH`), never against the operator setting. That
  setting (`ServerConfig.dataplane_cgroup_attach_path`, ADR-0053 § 7) is read only by the
  boot composition in `lib.rs` and `node_adapters.rs`; no `serve` flag or CLI option sets
  it. A non-default value could therefore never have reached the refusal differently, and
  removing the check exposes nothing the owner depends on: the owner has no `connect4`
  rewrite to be inside.

Links: [feature delta](../../feature/reconciler-convergence-guarantees/feature-delta.md)
(D-10, D-11, slice 02), ADR-0164 (datagram clause and startup check superseded), ADR-0297,
GH #310, GH #358 (first VIP collides with a host address).
