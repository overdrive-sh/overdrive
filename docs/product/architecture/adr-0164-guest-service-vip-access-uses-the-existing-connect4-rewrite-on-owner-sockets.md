# ADR-0164 — Guest traffic to a service VIP: TCP is resolved by mesh resolution before classification; datagrams use the existing ADR-0053 `connect4` rewrite on the guest-flow owner's connected host sockets

## Status

**Proposed — decision D24 approved by user 2026-10-06; its startup refusal
outside the `connect4` attach point approved by user 2026-10-06; decision
D24a — TCP to a VIP is resolved as mesh, narrowing D24 to datagrams
(independent DESIGN review finding H-3) — approved by user 2026-10-06; pending
independent DESIGN review.** GH #295. Recorded in the
#295 feature delta, § *[REF] vsock Attachment Replacement DESIGN — PROPOSED
2026-10-05*. Depends on ADR-0151 (the owner), ADR-0153 (mesh flows), ADR-0162
(direct non-mesh forwarding) and ADR-0165 (tuple registration).

## Context

ADR-0053 delivers service VIPs to host-side clients with a cgroup `connect4`
program (`cgroup_connect4_service`) attached at `overdrive.slice`
(`DEFAULT_CGROUP_ATTACH_PATH`). VIPs come from the platform VIP allocator
(default range `10.96.0.0/16`, operator-configurable). Every Service gets both
a mesh frontend address (`10.98.0.0/16`, ADR-0072) and a VIP; the VIP map keys
name no Service, only `(vip, port, proto)`. The `overdrive serve` process
enrols itself in `overdrive.slice/control-plane.slice` at boot (ADR-0028), so
it sits below that attach point. Workload cgroups below
`overdrive.slice/workloads.slice` already receive the rewrite from the same
ancestor attachment in production.

Under ADR-0145 a guest has no NIC. A guest application that dials a VIP is
captured by the guest programs, which record the VIP as the original
destination (ADR-0150). The host guest-flow owner then opens the host-side
socket for the flow; that socket, not the guest's, reaches the network.

The mesh resolution adapter (`ServiceBackendsResolve`, ADR-0071/0072)
classifies a destination by frontend address, then by backend address; it
performs no VIP translation. A guest TCP flow to a mesh service's VIP is
therefore classified non-mesh, connected directly, rewritten by `connect4` to
an intercepted backend's `workload_addr:port`, and diverted by the intercept
rules to leg-C, which expects TLS — the flow fails, and it reached the
diversion as cleartext. The spike's VIP backend was not intercepted, so it did
not exercise this case. The rows the adapter already reads
(`service_backends`) carry each service's VIP.

The V-11/VIP spike (`spike/v11-vip-v14-findings.md`, increment e; qualified
metal) attached a line-for-line mirror of `cgroup_connect4_service` to the
cgroup holding the host owner:

- guest TCP and UDP (0 to 59,000 bytes, connected and unconnected) to a VIP
  with a non-intercepted backend worked, 300/300 stress each;
- the guest application saw the VIP in `getpeername` and in `recvfrom`;
- with no map entry the VIP was unreachable;
- an unframe tuple registered from the requested destination (the VIP) missed
  the rewritten packet; one registered from the socket's kernel peer did not.

## Decision

- **TCP to a VIP is mesh-resolved first.** The owner's single mesh resolution
  (ADR-0153) recognises service VIPs: a TCP destination equal to a service's
  `(VIP, listener port)` is classified exactly as a hit on that service's
  frontend — mesh to its selected backend, or mesh-unreachable when it has no
  healthy backend. A TCP destination inside the configured VIP ranges that
  matches no service is mesh-unreachable (fail closed), mirroring the
  existing frontend-subnet rule. Such a flow therefore reaches leg-F and is
  enforced with mTLS, or is refused; it is never connected in cleartext.
- **Datagrams to a VIP use `connect4`.** The owner always connects its host
  datagram socket, so the existing ADR-0053 `connect4` rewrite resolves a VIP
  to a platform-chosen backend. The owner never needs `sendmsg4` /
  `recvmsg4`. If the connected socket's kernel peer lies in the guest prefix,
  the association is refused (`Policy`, #310). Guest UDP is not mesh-enforced,
  as today: mesh enforcement is TCP-only.
- **Pinned placement.** The guest-flow owner runs inside the `overdrive serve`
  process, whose cgroup is `overdrive.slice/control-plane.slice`: below the
  `connect4` attach point and not the root cgroup.
- **Startup check (user ruling 2026-10-06).** The owner refuses to start
  unless its own cgroup is the `connect4` attach path or below it.
- **The guest keeps the VIP.** The guest's `getpeername4` / `recvmsg4`
  programs restore the recorded original destination, so the application sees
  the VIP; UDP replies come from the VIP as the application expects.

## Alternatives considered

- **Leave TCP VIPs to `connect4` (the earlier text of this ADR).** A mesh
  service's VIP is connected in cleartext and fails at leg-C. Rejected.
- **Refuse every guest TCP flow to a VIP.** Fails closed but removes VIP access
  for TCP. Rejected.
- **Resolve VIPs in the owner's userspace by reading the BPF service map.**
  Duplicates ADR-0053's selection logic and the map's layout outside the
  dataplane. Rejected; the resolution adapter already owns service-to-backend
  selection.
- **A guest-side VIP map consulted by the guest `connect4`.** Needs a per-VM
  copy of the service map and its synchronisation. Rejected.
- **Attach a second `connect4` instance to the owner's own cgroup.**
  Unnecessary while the owner is below the existing attach point. Rejected.
- **Run the owner outside `overdrive.slice`.** Guest datagram VIP access would
  fail. Rejected.

## Consequences

- The resolution adapter gains a VIP key, built from the `service_backends`
  rows it already reads, and the configured VIP ranges as a constructor input
  (one SSOT with the VIP allocator's configuration). The `MtlsResolve` port
  signature is unchanged; its documented classification gains the VIP branch.
- Guest datagram VIP access depends on the owner's cgroup placement; the
  startup check makes a misplacement a refusal, not a silent failure.
- Every other socket of `overdrive serve` was already below the attach point;
  nothing changes for them.
- Ancestor inheritance of the `connect4` attachment is existing production
  behaviour for workload cgroups; its effect on the owner's own sockets is
  confirmed inside the walking skeleton (V-15, folded into V-6).
