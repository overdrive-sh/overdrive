# STEP 14-01 V6 Service-VIP UDP contract reconciliation

## Status and disposition

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `14-01` |
| Date | 2026-10-30 |
| Role | Bounded DESIGN premise and contract reconciliation |
| Status | **DECISION PACKAGE COMPLETE — USER SELECTED D; CANONICAL CORRECTION PENDING FRESH REVIEW** |
| Affected execution | **STOPPED pending review and DISTILL alignment to the selected deferral** |
| Production change authorized | **None** |
| Contract relaxation authorized | **Only option D's quoted V6 and roadmap Service-VIP UDP clauses** |

## Post-decision disposition

On 2026-10-30 the user selected option D with the exact instruction:

> “yeah, let's not do that now. add all the relevant info to the github issue,
> and then we keep it deferred to #310 and correct the contradictory
> acceptance checklist.”

The canonical scoped amendment and #310 follow-up boundary are recorded in
`design-14-01-udp-deferral.md`. The remainder of this file is preserved as the
independently reviewed pre-selection decision package. Its statements that a
user choice is still required describe that earlier state and are superseded
for current disposition by the approved option-D record; its production-path
audit and rejected fixture substitutes remain valid evidence.

The accepted V6 contract does **not** require the Service-VIP UDP target to
be the peer VM Service. V6 names the dialing actor as a VM Service, but names
the UDP destination only as “a service VIP.” D12, ADR-0145, ADR-0149,
ADR-0164 and ADR-0172 make a VM backend an invalid target for that UDP
action. The iteration-7 DISTILL topology incorrectly made the peer VM
Service own the same VIP's UDP listener and therefore asked the product to
perform inbound UDP service to a VM, the exact outcome deferred to #310.

The accepted outcome can remain unchanged only if the Service-VIP UDP action
uses a separately materialised Service backend at the node's `host_ipv4`.
That is the only current `ServiceMapHydrator` partition that populates
`LOCAL_BACKEND_MAP`, the sole map the accepted owner-socket `connect4` hook
consults. The distinct client VM Service and peer VM Service remain the actors
for the mesh-name, TCP/mTLS, remote-inbound and probe parts of V6.

The current public Service model is VM-only, and the completed production
audit found no `serve` + public-deploy path that can materialise a Running
Service backend at `host_ipv4`. There is no remaining topology-only DISTILL
route. The user must choose one of the bounded options below. No option is
accepted by this record, and no crafter may treat the gap as authority to
change production.

## Accepted contract audit

### The V6 actor and destinations are distinct objects

The accepted V6 row says:

> “a VM service dials a mesh peer by name, reaches a non-mesh address over
> TCP and UDP … reaches a mesh service's VIP over TCP … and a service VIP
> over UDP”

(`feature-delta.md:13180`). The grammatical subject, “a VM service,” is the
dialing client. The row does not say that the mesh-name peer, mesh TCP VIP,
UDP VIP, inbound target and probe target are one Service, one VIP or one
backend. Roadmap criterion 3 preserves the same distinction: it requires a
VM Service client and “a service VIP over UDP,” without assigning that UDP
VIP to the peer VM (`deliver/roadmap.json:1503-1507`).

The following accepted roles therefore remain exact:

| Role / object | Accepted requirement |
|---|---|
| Client VM Service | Owns the guest DNS, mesh-name, mesh-VIP TCP, Service-VIP UDP and off-node dials. |
| Peer VM Service | May own the mesh-name/TCP VIP, remote-inbound application, probes, state control and retained TCP population. |
| Service-VIP UDP target | A separate Service/VIP/listener/backend relation whose selected local backend is `host_ipv4`, so the production hydrator populates `LOCAL_BACKEND_MAP`; it is not required to be the peer VM Service. |

DISTILL iteration 7 strengthened the third row without DESIGN authority by
requiring the client to send UDP to “the same public issued VIP and declared
UDP listener” of the peer VM Service
(`deliver/review-distill-14-01-native-support.md:1547-1564,1645-1656`). That
is test topology, not an accepted V6 outcome, and it conflicts with the
accepted UDP boundary below.

### D12 and the four ADRs exclude VM UDP service delivery

The normative contracts agree:

- D12 says “Inbound UDP service to VMs deferred” (#310)
  (`feature-delta.md:1323`). The scope section repeats that inbound UDP
  service to VMs is out of scope and a UDP destination in the guest prefix is
  `Policy` (`feature-delta.md:13080-13109`).
- ADR-0145 gives a VM no guest NIC, TAP, bridge port or per-allocation host
  netdevice (`adr-0145...md:86-98`) and says inbound UDP service to VMs is not
  carried by this feature (`:175-190`).
- ADR-0149 rejects carrying a peer address in the datagram frame because that
  is needed to multiplex peers behind one association for inbound UDP service
  #310 (`adr-0149...md:48-52`).
- ADR-0164 says the owner connects its host UDP socket so `connect4` may
  rewrite a VIP, then **must refuse `Policy` when the connected kernel peer is
  inside the guest prefix** (`adr-0164...md:49-64`). Its successful spike used
  a non-intercepted backend (`:40-47`), not a VM backend.
- ADR-0172 says an unmarked host socket has no delivery path into the guest
  prefix; leg S and probes reach a VM only through the owner-opened TCP
  `GuestStreamDial` (`adr-0172...md:20-48`). Its constant output rule refuses
  non-diverted UDP rather than permitting another route (`:73-91`).

Making UDP reach the peer VM requires both explicit user approval to
supersede D12 and ADR-0164's guest-prefix refusal **and** a separate DESIGN for
#310's exact association identity, peer-address representation, intake owner,
reply routing, lifecycle and typed errors. ADR-0149 records one rejected
peer-address/multiplexing alternative; it does not preselect the future #310
mechanism. This cannot be called a STEP 14-01 production bug fix under the
accepted design.

## Revalidated production path

### VM Service backend publication

The public aggregate is currently VM-only:

- `WorkloadDriverV1` has only `Vm(Vm)`
  (`crates/overdrive-core/src/aggregate/mod.rs:133-159`);
- `ServiceV1.driver` is that same VM-only enum (`:345-367`); and
- public `DriverInput` accepts only `[vm]` (`:836-848`).

On a successful production start, the action shim writes the transport's
leased `workload_addr` into the Running allocation row
(`crates/overdrive-control-plane/src/action_shim/mod.rs:2445-2458`).
`ServiceLifecycle` reads that address, falling back to the node address only
when the row contains `None`
(`crates/overdrive-reconcilers/src/service_lifecycle.rs:865-914`), and emits
each listener backend at `backend_ip:listener.port`
(`:1040-1103`). A publicly deployed peer VM Service therefore advertises a
`100.95.0.x:<listener>` backend inside `GUEST_NODE_PREFIX`.

The `None -> host_ipv4` branch is not evidence of an available non-VM
production Service. The public model above cannot submit one, and production
Running VM rows carry their transport address. Manufacturing `None` in a
test would reproduce the repository's documented test-only fallback
anti-pattern, not a production topology.

### The accepted `connect4` proof requires `host_ipv4`

An address merely outside `GUEST_NODE_PREFIX` is insufficient. The production
hydrator partitions a non-mesh IPv4 backend as local only when its address
equals `host_ipv4`; only that local partition emits
`RegisterLocalBackend` for `LOCAL_BACKEND_MAP`
(`crates/overdrive-reconcilers/src/service_map_hydrator.rs:336-371,442-475`).
Every other outside-prefix backend enters the remote XDP/REVERSE_NAT
projection (`:373-421`).

The production `cgroup_connect4_service` program consults only
`LOCAL_BACKEND_MAP`. A miss permits the original VIP connect unchanged; it
does not consult the remote XDP `SERVICE_MAP`
(`crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:48-101`). V6's
accepted confirmation that the ancestor cgroup attachment rewrites the owner
socket therefore requires a publicly materialised backend at `host_ipv4`,
published and hydrated through the normal production owners. Selecting a
different outside-prefix backend would not prove this clause.

### Guest UDP owner path

The accepted guest datagram reaches the host owner as a
`FlowRequest::DatagramStream`; the owner dispatches it to
`datagram_associate` without the TCP resolver or mTLS enforcement
(`crates/overdrive-control-plane/src/guest_network/owner_runtime.rs:771-813`).
The datagram path:

1. applies static UDP/host-internal policy;
2. opens one unmarked UDP peer for a VIP;
3. connects that peer to the original VIP;
4. reads the kernel peer after `connect`; and
5. aborts and returns `Refused(Policy)` when that peer is inside the guest
   prefix, except `gateway:53`

(`owner_runtime.rs:1193-1262`). `GuestFlowIo::connect_udp` explicitly says
that a `connect4` rewrite is visible through the following `peer_addr`
(`guest_network/flow_io.rs:337-351`), and the host adapter implements those
operations with real `connect(2)` and `getpeername(2)`
(`guest_network/flow_io/host.rs:642-698`). The accepted owner cell pins both
branches: an outside-prefix rewrite pairs, while an inside-prefix rewrite is
`Policy` (`guest_network/owner_acceptance/datagrams.rs:157-195`).

There is no UDP counterpart to TCP's mesh resolution or leg-S
`GuestStreamDial` in this call path. Adding one is architecture for #310, not
private STEP 14-01 implementation.

### The raw receipt does not isolate a single internal failure stage

The durable Metal receipt
`.context/netns-density-295/14-01/raw/012109f99448d662b35c1760/` records commit
`c9f2f6d6975b236fa4a049093550d3ea8dcf4242` and source digest
`62f6c1832d836a9536a84d89d821f14bd43357fbb5f8a2125f5f8efb16ca3728`.
The client VM reached DNS/name resolution and the TCP VIP outcome, then two
allocations failed with `UDP_TIMEOUT`; the third stayed Running through
`v6-client: vip-tcp ok` and never produced `vip-udp ok` within 120 seconds.
The peer VM logged no UDP request (`output.log:2656`; the earlier exact-source
receipt `a5eb43472f5dc625d0c29e86` supplies the peer-log complement).

That evidence proves the peer-VM UDP topology fails through the real
production entry point. It does not itself expose the flow refusal or the
post-connect kernel peer, so this record does not accept “the post-`connect4`
`Policy` branch fired” as the uniquely reproduced internal cause.

The current `ServiceMapHydrator` still gates only the legacy
`WORKLOAD_SUBNET_BASE` (`10.99.0.0/16`) and partitions other addresses as
local only when they equal `host_ipv4`
(`crates/overdrive-reconcilers/src/service_map_hydrator.rs:263-276,336-370`;
`crates/overdrive-control-plane/src/lib.rs:5338-5349`). A `100.95.0.x` VM
backend can therefore take the remote-dataplane branch instead of producing a
local `connect4` entry. That is another production-reachable explanation for
the missing UDP request. The failed receipt does not distinguish it from the
owner's mandatory post-rewrite refusal.

This distinction prevents a speculative fix. Whichever stage rejected or
lost this particular datagram, the accepted design supplies no legal UDP path
from a Service VIP into a VM backend.

## Native-falsification disposition

The native run **held** the D12/ADR-0145/0149/0164/0172 premise: a Service
VIP mapped toward a VM backend does not deliver UDP to that VM. Those accepted
decisions and their security boundary remain accepted.

The run **falsified** these dependent premises:

1. DISTILL iterations 7–9 treated the peer VM Service's UDP listener as the
   exact V6 Service-VIP UDP target.
2. The V6 validation row classified the full production composition as a
   non-contract-shaping assembly of already proven mechanisms without pinning
   a production-reachable Service backend at `host_ipv4` for
   `LOCAL_BACKEND_MAP` and the owner-socket `connect4` rewrite.
3. STEP 14-01 roadmap criterion 3 was treated as executable with the current
   public-deploy topology, although criterion 4 forbids manufacturing the
   missing backend/map state.

The affected V6 validation disposition and STEP 14-01 roadmap execution
authority must therefore remain pending until independent DESIGN review and
the exact user decision below. Earlier ADR approvals, completed
roadmap steps, STEP 14-01's already-reviewed GUEST-PORTS/EVMOD-PAIR changes,
and the independently valid F-05 oracle correction remain historical evidence;
they are not relabelled or revoked.

## Production-reachability conclusion

The missing production contract is exact: no public Service submission can
produce a Running backend at `host_ipv4` for the
`ServiceLifecycle -> ServiceMapHydrator -> LOCAL_BACKEND_MAP -> owner-socket
connect4` path. The current public model admits only VM Services, every
production Running VM row publishes its guest transport `workload_addr`, and
`ServiceLifecycle` is the sole production backend publisher.

DISTILL cannot close this with private fixture HOW. A direct map write,
injected `ServiceBackendRow`, test-only `workload_addr=None`, private publisher,
adapter override or fabricated reply would manufacture product state forbidden
by roadmap criterion 4. STEP 14-01 therefore remains stopped for the user
decision below.

## User decision required

**Recommendation: A.** It is the only listed direction that preserves both
accepted V6 and D12. This recommendation follows correctness and invariant
precedence. Choosing A authorizes only a separate DESIGN to define and review
the missing production contract. It does not approve a driver variant,
external-endpoint resource, lifecycle, ownership boundary, API, wire or
persisted shape, typed error set, implementation, test or roadmap step.

The options are mutually exclusive at the outcome boundary. None is accepted
by this document.

### A — production host-local Service backend path (recommended)

Preserve V6 and D12 by separately designing a public production path that can
materialise a Service backend at the node's `host_ipv4`. The existing
`ServiceLifecycle` publisher and `ServiceMapHydrator` must then place that
backend in `LOCAL_BACKEND_MAP`, so the already-accepted owner-cgroup
`connect4` hook rewrites the client VM Service's socket.

Approval of A means only: **authorize a separate DESIGN for this missing
host-local Service backend capability while retaining V6 and D12 unchanged.**
That DESIGN must establish the exact user/operator surface, owner, lifecycle,
API and persistence shapes, semantic typed errors, security boundary,
compatibility, tests and roadmap before implementation. No particular
mechanism is selected here.

### B — include inbound UDP service to VM backends (#310)

Approval of B must explicitly supersede both accepted prohibitions:

> D12: “Inbound UDP service to VMs deferred”

(`feature-delta.md:1323`).

> ADR-0164: “If the connected socket's kernel peer lies in the guest prefix,
> the association is refused (`Policy`, #310).”

(`adr-0164-guest-vip-tcp-resolves-as-mesh-and-datagrams-use-the-connect4-rewrite.md:59-64`).

Approval of B means: **include inbound UDP service to VM backends in this
feature, supersede the two quoted prohibitions, and authorize a separate
DESIGN to choose the exact #310 mechanism.** That later DESIGN owns the
association identity, peer-address representation, intake owner, reply
routing, lifecycle, interfaces, typed errors, formal model and native proof.
This record does not preselect a frame, peer-address or multiplexing design.

B preserves the V6 UDP outcome but relaxes an accepted security/scope
boundary and has the largest ownership and protocol impact.

### C — weaken the V6 evidence boundary

Approval of C must replace these exact evidence requirements for the
Service-VIP UDP clause:

> V6: “Walking skeleton through `overdrive serve` + `overdrive deploy`”

(`feature-delta.md:13180`).

> Roadmap criterion 4: “No test installs, binds or programs anything
> production does not; the walking skeleton drives serve + deploy with no
> override seam.”

(`deliver/roadmap.json:1507`).

Approval of C means: **for Service-VIP UDP only, replace the quoted production
composition requirements with a mechanism-level proof using a directly
prepared map and test-owned host UDP backend.** It would no longer validate the
public Service lifecycle that materialises the VIP/backend relation. It
retains nominal UDP behavior text with weaker evidence and therefore carries
the highest evidence-integrity risk.

### D — defer/remove Service-VIP UDP from V6

Approval of D must remove this exact V6 outcome:

> “and a service VIP over UDP (`connect4`; includes the confirmation that the
> ADR-0053 attachment at `overdrive.slice` rewrites owner sockets in
> `overdrive.slice/control-plane.slice`)”

(`feature-delta.md:13180`). It must also remove roadmap criterion 3's
corresponding clause:

> “and a service VIP over UDP”

(`deliver/roadmap.json:1506`).

Approval of D means: **defer/remove the two quoted Service-VIP UDP clauses
from STEP 14-01 while retaining the other V6 outcomes.** This is the smallest
implementation scope and a direct observable-outcome weakening. Only the
user's exact approval can authorize the canonical feature-delta and roadmap
amendments.

“Patch the peer VM path,” “allow this one guest-prefix UDP tuple,” “route it
through leg S,” and “add a second UDP owner method” are not additional
options. Each would require the explicit B supersession and a separate #310
DESIGN.

## Interface, error, lifecycle and formal gates

- **Public/cross-crate API:** unchanged. No new surface is authorized.
- **Typed errors:** unchanged. The reproduced test protocol's
  `UDP_TIMEOUT` is evidence, not an approved new Overdrive error mapping.
- **Lifecycle Gate Ownership:** not applicable to this reconciliation. No
  state gate, meaning, owner or transition changes. Any selected option that
  adds a mechanism must supply its own gate analysis.
- **Formal protocol model:** not applicable to this stop and user-decision
  package. Option B returns to DESIGN for concurrent association and ownership
  analysis; whether it changes the existing wire is not pre-decided here.
- **Security:** ADR-0164's post-connect guest-prefix refusal and ADR-0172's
  constant rules remain fail-closed. They must not be relaxed to obtain GREEN.
- **Validation:** no Lima, mutation test, new lane or new Metal run is
  authorized while this DESIGN disposition is unreviewed.

## Independent-review handoff

The reviewer must verify these points independently:

1. V6 and roadmap criterion 3 bind the VM kind to the client, not to the UDP
   backend.
2. D12 and ADRs 0145/0149/0164/0172 forbid the current peer-VM UDP topology.
3. The raw receipt proves failure but not a unique internal stage.
4. The public Service model and real allocation/backend projection provide no
   production-reachable `host_ipv4` Service backend through public deploy.
5. Only `host_ipv4` enters `LOCAL_BACKEND_MAP` for the accepted owner-socket
   `connect4` proof; a generic outside-prefix backend is insufficient.
6. Options B–D quote every normative relaxation, A is recommended without
   selecting its mechanism, and the user must choose A–D before affected
   source, tests, DES state or accepted contracts change.

Until that review completes, STEP 14-01 remains blocked at Service-VIP UDP and
the peer-VM UDP test topology has no implementation authority.
