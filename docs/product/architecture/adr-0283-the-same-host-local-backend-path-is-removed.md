# ADR-0283 — The same-host local-backend path is removed

## Status

Proposed. 2026-11-15. Author: Morgan (`nw-solution-architect`), DESIGN wave, APPLICATION
layer, feature `reconciler-convergence-guarantees`. The user approved the decision on
2026-11-15 (feature delta D-9). Independent design review is pending.

Supersedes [ADR-0053](adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md) in full.
Supersedes the datagram clause and the startup check of
[ADR-0164](adr-0164-guest-vip-tcp-resolves-as-mesh-and-datagrams-use-the-connect4-rewrite.md)
(user ruling 2026-11-15). ADR-0164's TCP decision (D24a) stays.

## Context

ADR-0053 delivers a Service VIP to a client on the same host. The `ServiceMapHydrator`
classifies each backend of a Service. A backend whose address equals the node's
`host_ipv4` is local. For a local backend the hydrator emits `Action::RegisterLocalBackend`,
and the dataplane writes `LOCAL_BACKEND_MAP` and `REVERSE_LOCAL_MAP`. Three `cgroup_sock_addr`
programs read them: `cgroup_connect4_service`, `cgroup_sendmsg4_service` and
`cgroup_recvmsg4_service`, attached at the node's attach cgroup.

No production path produces a local backend:

- Since ADR-0276 a Service backend address is the Running allocation's `workload_addr`. The
  action shim takes it from the guest network plan. The plan allocates from the guest node
  prefix (`100.95.0.0/16`). It is never `host_ipv4`.
- Non-VM provisioning was deleted (`fc3bc7d04`). No workload runs on the host.
- The hydrator's local partition (`service_map_hydrator.rs:356-370`) therefore always yields
  an empty set. No other caller of `register_local_backend` exists.
- `cgroup_connect4_service` reads only `LOCAL_BACKEND_MAP` and allows every miss. With the map
  never populated it never rewrites anything.
- The public ingress gateway does not use the path. It has its own program,
  `cgroup_connect4_gateway` (ADR-0224), which reads `GATEWAY_CONNECT_INTENT_MAP`,
  `GATEWAY_SELECTION_RECEIPT_MAP`, `SERVICE_MAP` and `BACKEND_MAP`. A userspace-proxy gateway
  would select a backend in process and dial its guest address; it does not need the path
  either.

ADR-0164 builds on this path. Its datagram clause says guest datagrams to a VIP use the
ADR-0053 `connect4` rewrite on the guest-flow owner's connected host sockets, and its startup
check refuses to start an owner whose cgroup is not at or below the `connect4` attach path.
That clause has never worked for a VM backend. Metal reproduction (Linux 7.0.0-38, commit
`fb71b400f`, three runs; evidence `spike-scratch/guest-udp-vip-probe/evidence/run{7,8,9}/`):
for a Running UDP VM Service, `LOCAL_BACKEND_MAP` holds 0 entries while `SERVICE_MAP` holds
the VIP's TCP and UDP keys; guest UDP to the VIP gets no reply in 8 of 8 attempts; guest TCP
to the same VIP and port works (it resolves as mesh); `getpeername` on the owner's connected
host UDP socket after `connect` is still the VIP, so no rewrite occurs. ADR-0164's only
evidence for the clause, the V-11 spike, used a host-local, non-intercepted backend, which
production never produces. This is the gap tracked by GH #310 (UDP service delivery to VM
workloads).

The path is dead code that still costs something at every boot. The dataplane loads and
attaches the three programs, probes both maps (the ADR-0053 Earned-Trust extension), and
carries the register and deregister executors, two read-back handles and a hydrator View
field. That field, `last_applied_local_fingerprint`, records what the reconciler emitted and
is consulted as the diff. It is the defect class this feature removes from reconcilers
(`.claude/rules/reconcilers.md`).

## Decision

Remove the same-host local-backend path. The node no longer loads or attaches
`cgroup_connect4_service`, `cgroup_sendmsg4_service` or `cgroup_recvmsg4_service`. The
`LOCAL_BACKEND_MAP` and `REVERSE_LOCAL_MAP` maps, their miss counter, their userspace
handles and probes, the dataplane's local-backend registration operations, the
`RegisterLocalBackend` and `DeregisterLocalBackend` actions with their executors, and the
hydrator's local partition and View fingerprint are deleted, together with every test that
defends them.

The ADR-0164 datagram clause and its startup check are removed with it. The guest-flow owner no
longer treats a destination in the VIP range as a datagram case that `connect4` rewrites, and it
no longer refuses to start when its cgroup is outside the `connect4` attach path. Guest UDP to a
VIP is not served until GH #310 lands. That is a scope limitation of this design, tracked by
#310, which requires kernel-resident forwarding and no userspace relay, and which reconciles
ADR-0164's guest-prefix UDP refusal.

Kept unchanged: `cgroup_connect4_gateway` and everything it reads, the remote
`SERVICE_MAP` / `BACKEND_MAP` path, the attach-cgroup operator setting that ADR-0053 § 7
introduced (the gateway still attaches at that cgroup), and ADR-0164's TCP decision (D24a): a
guest TCP destination equal to a service's `(VIP, port)` resolves as mesh, and an unmatched TCP
destination in the VIP range is mesh-unreachable.

The scope inventory and its acceptance live in the
[feature delta](../../feature/reconciler-convergence-guarantees/feature-delta.md), slice 02.

## Alternatives considered

| Option | Why it loses |
|---|---|
| Keep the path and converge it (the hydrator reads the two maps back through a read-port and drops the View fingerprint) | It hardens code no production path reaches. It adds a read-port, a hydration field, a result row and a model, to repair a failure that cannot occur. |
| Keep the path and re-point the classifier at node-local guest addresses | No host-side client dials a VIP. VMs reach backends through the guest network, and the gateway has its own hook. The result would be a rewrite with no client. |
| Keep the path dormant | The three programs stay attached and both maps stay probed at every boot, and the View latch stays as the defect class this feature removes. Dormant code is reintroduced by the next reader as if it worked. |

## Consequences

Positive:

- The boot loads, attaches and probes three fewer programs and two fewer maps. The BPF
  object and the verifier baselines shrink.
- One View latch is removed by deletion, not by convergence, and the reconciler surface
  loses a dispatch pair.
- `Dataplane` loses two operations and the error variants only they use.

Negative and bounds:

- A host-side client that dials a Service VIP is not served by a rewrite. None exists today.
  A future one is designed on its own terms, from the then-current Service dataplane.
- Guest UDP to a Service VIP is not served. The mechanism ADR-0164 names is
  `cgroup_connect4_service`, which finds no entry in production because no VIP key is ever
  written to `LOCAL_BACKEND_MAP`; the metal reproduction above shows the guest gets no reply.
  This decision removes a mechanism that never worked for a VM backend. The limitation is
  tracked by GH #310 (deferred by the user 2026-10-30), which requires kernel-resident
  forwarding with no userspace relay. One option for #310 to evaluate is a service `connect4`
  that selects from `SERVICE_MAP` / `BACKEND_MAP` as `cgroup_connect4_gateway` does; this ADR
  does not choose it. A comment on #310 records the removal.
- The owner's cgroup-placement startup check is removed. It guarded only the removed
  rewrite, and `run_server` enrols the process into `overdrive.slice/control-plane.slice`
  before the check runs, so no production boot reached its refusal. Nothing else depends on
  the owner's placement; the gateway's attach is independent (slice 02 of the feature delta).
- ADR-0224 and ADR-0232 still describe the gateway arm as sitting beside, or being, a
  `cgroup_connect4_service` arm. The gateway program is separate (ADR-0224 P-10). Its
  ordering argument ("`LOCAL_BACKEND_MAP` holds no VM backend, so the existing program
  misses") becomes moot, since there is no other program to order against.

Links: [feature delta](../../feature/reconciler-convergence-guarantees/feature-delta.md)
(D-9, D-10, slice 02), ADR-0053 (superseded), ADR-0164 (datagram clause and startup check
superseded), ADR-0224, ADR-0276, GH #310.
