# ADR-0297 — The same-host local-backend path is removed

## Status

Proposed. 2026-11-15. Author: Morgan (`nw-solution-architect`), DESIGN wave, APPLICATION
layer, feature `reconciler-convergence-guarantees`. The user approved the decision on
2026-11-15 (feature delta D-9). Independent design review is pending.

Supersedes [ADR-0053](adr-0053-same-host-backend-delivery-via-cgroup-sock-addr.md) in full.
The consequence for ADR-0164's datagram clause is a separate decision:
[ADR-0298](adr-0298-guest-udp-to-a-service-vip-is-refused-until-gh-310.md).

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
  never populated it never rewrites anything. Metal observation (Linux 7.0.0-38, commit
  `fb71b400f`, `spike-scratch/guest-udp-vip-probe/evidence/run{7,8,9}/`): `LOCAL_BACKEND_MAP`
  holds 0 entries for a Running UDP VM Service.
- The public ingress gateway does not use the path. It has its own program,
  `cgroup_connect4_gateway` (ADR-0224), which reads `GATEWAY_CONNECT_INTENT_MAP`,
  `GATEWAY_SELECTION_RECEIPT_MAP`, `SERVICE_MAP` and `BACKEND_MAP`, and acts only on sockets
  registered in the intent map (any other socket is allowed untouched).

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

Kept unchanged: `cgroup_connect4_gateway` and everything it reads, the remote
`SERVICE_MAP` / `BACKEND_MAP` path, and the attach-cgroup operator setting that ADR-0053 § 7
introduced (the gateway still attaches at that cgroup).

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
- The ADR-0164 datagram clause depended on this rewrite and cannot stand; ADR-0298 records
  what replaces it.
- The `reconcile_conflict` observation row's route tag has a cgroup value that only the
  deleted actions could produce. Its fate is pinned in the feature delta (D-13).
- Accepted ADRs that mention the removed path are not edited, because ADRs are immutable.
  This ADR supersedes the path as they describe it: ADR-0040, ADR-0062, ADR-0069,
  ADR-0071, ADR-0072, ADR-0101, ADR-0224 and ADR-0232 (each mentions `LOCAL_BACKEND_MAP`,
  `cgroup_connect4_service`, the local-backend actions or `connect4`). ADR-0224 and
  ADR-0232 describe the gateway arm as sitting beside, or being, a
  `cgroup_connect4_service` arm; the gateway program is separate (ADR-0224 P-10), and its
  ordering argument ("`LOCAL_BACKEND_MAP` holds no VM backend, so the existing program
  misses") is moot, since there is no other program to order against. Living documents
  (`brief.md`, `c4-diagrams.md`, journeys, the rules docs) are updated in the deletion
  commit (feature delta, slice 02).

Links: [feature delta](../../feature/reconciler-convergence-guarantees/feature-delta.md)
(D-9, slice 02), ADR-0053 (superseded), ADR-0298, ADR-0224, ADR-0276.
