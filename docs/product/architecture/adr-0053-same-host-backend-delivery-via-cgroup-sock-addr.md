# ADR-0053 — Same-host backend delivery via `cgroup_sock_addr` connect-time destination rewrite

## Status

Accepted (2026-05-23). Drafted 2026-05-22 by Morgan. Implementation
landed across commits cd5b1644 → 6d62afe6 (closes #175, walking-
skeleton S-BDB-01 e2e). Tags: phase-1, dataplane, lb, cgroup-bpf,
walking-skeleton, j-plat-004.

**Companion ADRs**: ADR-0040 (three-map split + HASH_OF_MAPS atomic
swap — the `SERVICE_MAP` / `BACKEND_MAP` / `MAGLEV_MAP` shape that
the wire-boundary path consumes), ADR-0041 (weighted Maglev +
REVERSE_NAT shape), ADR-0042 (`ServiceMapHydrator` reconciler +
`Action::DataplaneUpdateService`), ADR-0045 (`bpf_redirect_neigh`
datapath), ADR-0049 (platform-issued `ServiceVipAllocator`), ADR-0052
(`BackendDiscoveryBridge` reconciler + `EbpfDataplane` production
boot).

**Tracks**: walking-skeleton TCP round-trip (A4) on the
`backend-discovery-bridge-service-reachability` feature — RCA at
`docs/feature/backend-discovery-bridge-service-reachability/deliver/rca-walking-skeleton-tcp-roundtrip.md`.

## Context

Phase 1 is single-node. Per
`feedback_phase1_single_node_scope.md` workloads share the host
network namespace; per-workload netns isolation is Phase 2+ scope.
This ADR resolves the data-path gap exposed by the walking-skeleton
TCP round-trip assertion (A4) failing while the convergence chain
assertions (A1–A3) pass.

### The defect the convergence chain alone cannot fix

ADR-0052's joint walking-skeleton was designed to gate both
`BackendDiscoveryBridge` (#174) and `EbpfDataplane` production boot
(#175) on a single end-to-end TCP round-trip:

1. Submit Service spec → admission allocates VIP via
   `ServiceVipAllocator`.
2. Alloc reaches Running; bridge writes `ServiceBackendRow`.
3. `ServiceMapHydrator` emits `Action::DataplaneUpdateService`;
   `EbpfDataplane::update_service` populates `BACKEND_MAP` +
   `SERVICE_MAP`.
4. Test issues `connect(<vip>:<port>)`; kernel XDP rewrites dst →
   backend; backend listener echoes; reverse-NAT rewrites src →
   VIP; test receives reply.

Assertions A1–A3 (Running, BACKEND_MAP populated, SERVICE_MAP
populated) all PASS — the intent-to-map convergence chain is wired
end-to-end. A4 (the TCP round-trip) FAILS, and not because of any
bug in the convergence chain. Per the RCA:

- `xdp_service_map_lookup` rewrites dst from VIP → `host_ipv4` of
  the LB-side veth. The post-rewrite destination is **a LOCAL address
  on the ingress iface**.
- `bpf_fib_lookup` against a LOCAL destination returns
  `BPF_FIB_LKUP_RET_NOT_FWDED` by kernel design — the kernel does not
  *forward* packets to itself; local destinations are delivered up the
  stack via the normal `ip_local_deliver` path.
- XDP cannot deliver to local sockets on its own; the documented
  primitives (`XDP_TX`, `XDP_REDIRECT`, `bpf_redirect_neigh`) all
  push the packet out an egress iface, none of them hand the skb to
  the kernel's local-delivery path.
- The walking-skeleton's RCA is reproducible 100% of the time in
  Lima; the sibling `reverse_nat_e2e` × 5 Tier 3 tests against
  `ThreeIfaceTopology` (3-netns transit) PASS in the same VM, same
  session — proving the XDP data path is correct when the destination
  is genuinely remote (across a netns boundary that simulates a
  separate host).

This is not a fixture bug; it is an architecture bug. Phase 1's
LB capability today is "convergence chain works but data path does
not deliver to the only workload type Phase 1 supports
(shared-host-netns)." The walking-skeleton names this honestly: the
LB programs are wired, but no traffic flows.

### What Cilium does, faithfully

The originating research at
`docs/research/dataplane/same-host-backend-delivery-architecture.md`
established the upstream-grounded answer: Cilium runs XDP for *remote*
backends and a **socket-layer** primitive for *same-host* backends.
Per the [Cilium "XDP Acceleration"
docs](https://docs.cilium.io/en/stable/operations/performance/tuning/#xdp-acceleration)
(verified 2026-05-21): *"XDP-based acceleration in standalone mode is
currently only available for the remote backend case … For local
backends, the kube-proxy-replacement socket-layer datapath … delivers
the packet to the pod via the socket layer without any L3 rewrite."*

The specific primitive Cilium uses for the same-host path is
`BPF_CGROUP_INET4_CONNECT` (and `BPF_CGROUP_INET6_CONNECT`,
`SENDMSG`, `RECVMSG`) — BPF programs of type `cgroup_sock_addr`
attached to a cgroup, intercepting the `connect(2)` syscall before
the kernel records a peer. The program reads the syscall's intended
destination from the `bpf_sock_addr` context, looks it up against
a VIP table, and rewrites `user_ip4` / `user_port` to the backend's
real address. The kernel proceeds with the rewritten address; the
application's view via `getpeername(2)` returns the backend IP.
Confirmed by the [Cilium kube-proxy-free
docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/):
*"upon `connect` (TCP, connected UDP), `sendmsg` (UDP), or `recvmsg`
(UDP) system calls, the destination IP is checked for an existing
service IP and one of the service backends is selected as a target."*

### Why this ADR exists, not a topology fix

The walking-skeleton RCA suggested two recovery shapes: extend
`ExecDriver` for netns targeting (the upstream `netns_path:
Option<PathBuf>` parameter, landed in commit `51512d7c`), OR pull
Phase 2 per-workload netns forward into Phase 1. Both miss the
architectural point: **Phase 1 is single-node and shares the host
netns by design.** The `ExecDriver` netns parameter is useful for
Tier 3 test topology and remains a valid pre-requisite for Phase 2
per-workload netns isolation; it does not address the production data
path on a Phase 1 deployment, where the LB and every backend are
guaranteed to share the host netns.

The honest framing is the one Cilium ships: Phase 1's "same host"
case needs a socket-layer LB. The XDP wire-boundary programs stay
exactly as they are — they're the right tool for a different problem
that exists in Phase 2+. The walking-skeleton can shrink to
convergence-only (A1–A3) as an interim, but Phase 1's permanent
end-state needs an actually-delivering data path.

### Existing code surface

- `crates/overdrive-bpf/src/programs/xdp_service_map.rs` (`xdp_service_map_lookup`, `try_xdp_service_map_lookup`) and `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs` (`xdp_reverse_nat_lookup`) are the wire-boundary programs; their semantics are correct for the remote case and remain unchanged by this ADR.
- `crates/overdrive-core/src/traits/dataplane.rs:70-84` defines the `Dataplane` port trait. `update_service(vip, backends)` is the existing method that drives the XDP path. This ADR adds a parallel method; the existing signature is preserved.
- `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator/mod.rs` (and its `overdrive-core` implementation) is the consumer that today emits exactly one `Action::DataplaneUpdateService` per service. Under this ADR it performs per-backend Local-vs-Remote classification and emits a different action variant for each class.
- `crates/overdrive-control-plane/src/reconcilers/backend_discovery_bridge/mod.rs` is unchanged in shape; the `host_ipv4` it already plumbs into the row is the classifier input.
- `crates/overdrive-worker/src/cgroup_manager.rs` already manages the cgroup v2 hierarchy `overdrive.slice/workloads.slice/<alloc>.scope`. The control-plane process itself runs inside `overdrive.slice`. Both the control plane and every workload spawned via `ExecDriver` are descendants of `overdrive.slice` — the natural attach point for a cgroup `connect4` program covering the whole platform.

### Extensions to prior ADRs

- **ADR-0035 (`Reconciler` trait)** — preserved verbatim. The hydrator's `reconcile` stays pure sync; the per-backend classification is in-line logic, not a new method.
- **ADR-0040 (three-map split + HoM)** — preserved verbatim. The new path adds its own map; the existing maps are untouched.
- **ADR-0042 (`ServiceMapHydrator`)** — extended. The reconciler emits two action variants instead of one; the `RetryMemory` shape is unchanged.
- **ADR-0046 (collision-free `BackendId` allocator)** — preserved. `BackendId` is irrelevant to the cgroup path; this ADR keys by `(VIP, port) → SocketAddr`.
- **ADR-0048 (rkyv versioned envelope)** — preserved. The new path adds no persisted rkyv type; `ServiceBackendRow` is unchanged.
- **ADR-0052 (bridge + production boot)** — extended. The walking-skeleton's A4 assertion lands against the new path; the dataplane boot composition gains one Earned-Trust probe target (the cgroup attach + sentinel rewrite).
- **`.claude/rules/development.md` § "Trait definitions specify behavior, not just signature"** — followed; the new trait method's docstring pins preconditions, postconditions, edge cases, and observable invariants.
- **`feedback_phase1_single_node_scope.md`** — honored. The cgroup_sock_addr program covers the Phase 1 shape exactly: one host netns, one LB cgroup ancestor for every process that might issue a VIP connect.

## Decision

### 1. Adopt `cgroup_sock_addr` connect-time destination rewrite

Phase 1's same-host data path is a `BPF_PROG_TYPE_CGROUP_SOCK_ADDR`
program attached to a host-level cgroup that is an ancestor of both
the control-plane process and every workload spawned via
`ExecDriver`. The natural attach point on this codebase is
`overdrive.slice` (per `crates/overdrive-worker/src/cgroup_manager.rs`
the control plane lives there and every workload's `<alloc>.scope`
is a descendant via `workloads.slice`).

Two attach types are in scope:

- `BPF_CGROUP_INET4_CONNECT` — TCP and connected-UDP IPv4 connects.
  In scope for Phase 1 (Service spec listeners are TCP/UDP, IPv4).
- `BPF_CGROUP_INET6_CONNECT` — **out of scope.** Phase 1's
  `ServiceVipAllocator` (ADR-0049) issues IPv4 VIPs only; the IPv6
  attach point lands when IPv6 service VIPs land. Recorded as an
  out-of-scope item below; not a forward pointer.

`SENDMSG` / `RECVMSG` cgroup hooks are not in scope for Phase 1
either — those serve UDP message-by-message rewrite, which is only
needed when an *unconnected* UDP service is consumed by an application
that calls `sendto(VIP, ...)` without `connect()` first. Phase 1's
Service spec only ships TCP listeners; unconnected-UDP support is a
separate concern that admits the same primitive when needed.

#### Kernel-side program

```rust
// crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs
// Type-name precedent matches xdp_service_map.rs.
#[cgroup_sock_addr(connect4)]
pub fn cgroup_connect4_service(ctx: SockAddrContext) -> i32 {
    match try_cgroup_connect4_service(&ctx) {
        Ok(verdict) => verdict,
        Err(_) => 1, // proceed (allow connect) on internal error
    }
}
```

Pipeline:

1. Read `user_ip4` and `user_port` from `bpf_sock_addr` context.
   These are the destination the application named; `user_ip4` is in
   network byte order per kernel UAPI.
2. Look up `LOCAL_BACKEND_MAP[(VIP, port)]`. Miss → return 1 (allow
   connect unchanged; this is non-service traffic).
3. Hit → overwrite `ctx->user_ip4` and `ctx->user_port` with the
   backend's address. Return 1.

The program returns `1` (allow) on every path; `0` (deny) is never
returned. The kernel proceeds with the (possibly-rewritten)
destination. No checksum work, no FIB lookup, no L2 rewrite — those
are wire-boundary concerns the cgroup hook never sees.

#### `LOCAL_BACKEND_MAP`

New map at `crates/overdrive-bpf/src/maps/local_backend_map.rs`:

| Field | Type | Notes |
|---|---|---|
| Map type | `BPF_MAP_TYPE_HASH` | Single global, point-access only. No HoM (the atomic-swap primitive ADR-0040 needs for ordered backend sets is not needed here — a Phase 1 Service has at most a small handful of local backends and the map is updated per (VIP, port) tuple). |
| Key | `LocalServiceKey { vip: u32, port: u16, _pad: u16 }` (host order, `#[repr(C)]`) | Endianness lockstep per ADR-0041: userspace writes host order, kernel-side `u32::from_be_bytes` on the packet bytes converts to the same host order at lookup. |
| Value | `LocalBackendEntry { backend_ip: u32, backend_port: u16, _pad: u16 }` (host order) | Single backend per (VIP, port) for Phase 1. Multiple-backend selection (Maglev-style) on the cgroup path is deferred — see § Out of scope. |
| `max_entries` | 4096 | Same envelope as `SERVICE_MAP` outer; Phase 1 deployments are far below this. |

#### Userspace handle

New handle at
`crates/overdrive-dataplane/src/maps/local_backend_map_handle.rs`,
typed shape mirrors the existing `BackendMapHandle`:

```rust
pub struct LocalBackendMapHandle { /* Map fd */ }

impl LocalBackendMapHandle {
    pub fn upsert(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        backend: SocketAddrV4,
    ) -> Result<(), MapError>;

    pub fn remove(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
    ) -> Result<(), MapError>;

    pub fn entries(&self) -> Result<Vec<(LocalServiceKey, LocalBackendEntry)>, MapError>;
}
```

`SocketAddrV4` is the trait-surface type; IPv4-only is enforced at
the typed-handle boundary. Wider socket-address polymorphism is not
needed at Phase 1's scope.

### 2. New `Dataplane` trait method `register_local_backend`

Parallel to `update_service`, NOT a signature change to it.
Per `.claude/rules/development.md` § "Trait definitions specify
behavior, not just signature":

```rust
// crates/overdrive-core/src/traits/dataplane.rs
#[async_trait]
pub trait Dataplane: Send + Sync + 'static {
    async fn update_policy(...) -> Result<(), DataplaneError>;
    async fn update_service(...) -> Result<(), DataplaneError>;
    async fn drain_flow_events(...) -> Result<Vec<FlowEvent>, DataplaneError>;

    /// Register or replace the local backend for `(vip, vip_port)`.
    ///
    /// # Preconditions
    /// - `vip` is an IPv4 service VIP issued by `ServiceVipAllocator`
    ///   (ADR-0049). The adapter does not validate this; callers
    ///   that pass non-allocator VIPs produce well-defined but
    ///   operator-confusing behavior.
    /// - `backend` is a `SocketAddrV4` reachable from the host
    ///   netns. Phase 1 single-node guarantees this when the
    ///   backend's allocation is Running on the same host.
    ///
    /// # Postconditions on `Ok(())`
    /// - For every subsequent `connect(vip:vip_port)` from a
    ///   process inside the dataplane's attach cgroup, the kernel
    ///   establishes a connection to `backend.ip():backend.port()`
    ///   instead.
    /// - The application's `getpeername(2)` returns `backend`, not
    ///   `(vip, vip_port)`. Per Cilium ClusterIP semantics; see
    ///   "Consequences" below.
    /// - `local_backends()` reflects the (vip, port, backend)
    ///   triple.
    ///
    /// # Edge cases
    /// - Re-registration with the same `backend` is idempotent; the
    ///   map update is the same triple.
    /// - Re-registration with a different `backend` for the same
    ///   `(vip, vip_port)` atomically replaces the existing entry
    ///   (single-map point write; no in-flight readers between the
    ///   syscall returning and the next `connect`).
    ///
    /// # Observable invariants
    /// - After `deregister_local_backend(vip, vip_port)`,
    ///   subsequent `connect(vip:vip_port)` reaches the kernel
    ///   without rewrite — the connect either succeeds against
    ///   whatever the VIP was *originally* routed to (typically
    ///   nothing in Phase 1, producing `ECONNREFUSED`), or fails
    ///   with `EHOSTUNREACH`. The cgroup hook does not deny; it
    ///   only rewrites.
    /// - `update_service(vip, ...)` and `register_local_backend(vip,
    ///   port, ...)` for the same VIP are not mutually exclusive
    ///   at the adapter — the XDP path consumes the first, the
    ///   cgroup path consumes the second. The classifier in
    ///   `ServiceMapHydrator` (§ 4) is responsible for choosing
    ///   exactly one per backend.
    async fn register_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
        backend: SocketAddrV4,
    ) -> Result<(), DataplaneError>;

    async fn deregister_local_backend(
        &self,
        vip: Ipv4Addr,
        vip_port: u16,
    ) -> Result<(), DataplaneError>;
}
```

The docstring pins the four properties the rule requires
(preconditions, postconditions, edge cases, observable invariants).
The DST equivalence harness (per the same rule's "structural guard"
section) will drive `EbpfDataplane` and `SimDataplane` through the
same sequence of `register_local_backend` / `deregister_local_backend`
/ `connect`-equivalent calls and assert observable equivalence.

### 3. New `Action` variant `Action::RegisterLocalBackend`

Per ADR-0023, the reconciler is pure; all side effects flow through
typed Actions. The hydrator's per-backend classification produces a
new variant alongside the existing `DataplaneUpdateService`:

```rust
// in overdrive-core::reconciler::Action
RegisterLocalBackend {
    service_id:  ServiceId,
    vip:         Ipv4Addr,
    vip_port:    u16,
    backend:     SocketAddrV4,
    correlation: CorrelationKey,
},

DeregisterLocalBackend {
    service_id:  ServiceId,
    vip:         Ipv4Addr,
    vip_port:    u16,
    correlation: CorrelationKey,
},
```

The action-shim wrapper lives at
`crates/overdrive-control-plane/src/action_shim/register_local_backend.rs`
— file shape symmetric with `dataplane_update_service.rs`. Dispatch
calls `Dataplane::register_local_backend` / `deregister_local_backend`.
The `action_shim/mod.rs` match gains two new arms; the existing
exhaustive-match property catches every consumer at compile time per
ADR-0023.

### 4. `ServiceMapHydrator` performs Local-vs-Remote classification

The hydrator's `reconcile` body gains a per-backend classifier. Phase
1 single-node means **every** backend on a Running alloc is local;
the classifier is structurally trivial today but remains correct as
Phase 2 lands:

```rust
// Inside ServiceMapHydrator::reconcile, after computing the
// desired backend set per service:
let (local, remote): (Vec<_>, Vec<_>) = backends
    .iter()
    .partition(|b| b.addr.ip() == IpAddr::V4(self.host_ipv4));

if !remote.is_empty() {
    actions.push(Action::DataplaneUpdateService {
        service_id, vip, backends: remote, correlation: ..,
    });
}
for backend in local {
    actions.push(Action::RegisterLocalBackend {
        service_id, vip, vip_port, backend: socketaddrv4(backend.addr),
        correlation: ..,
    });
}
```

Notes:

- `self.host_ipv4` is the same field `BackendDiscoveryBridge` already
  carries (per ADR-0052 § 1); the hydrator gains a mandatory
  constructor parameter per `.claude/rules/development.md` §
  "Port-trait dependencies — Required, not defaulted, at the call
  site". Default-construction is rejected; the dependency is
  explicit.
- A service with mixed local and remote backends (a Phase 2+ shape)
  emits both action kinds for the same `service_id`. The two
  dataplane paths are independent and the trait contract permits
  this concurrent dual-path. Phase 1 single-node never produces this
  case in practice.
- The `RetryMemory` View per `ADR-0042` is unchanged in shape. The
  hydrator already keys its dedup memory by `ServiceId`; the
  per-service decision to emit local-vs-remote is a function of the
  current desired set and the live `host_ipv4` policy — both inputs,
  per `.claude/rules/development.md` § "Persist inputs, not derived
  state".

### 5. Existing XDP programs remain unchanged

`xdp_service_map_lookup` and `xdp_reverse_nat_lookup` are not modified
by this ADR. They continue to attach in production single-mode boot
per ADR-0052. In Phase 1 single-node every backend classifies as
local; the XDP forward path receives no `update_service` calls and
the SERVICE_MAP outer HoM stays empty. Traffic that *would* match a
backend's IP never traverses XDP because the cgroup rewrite happens
before the kernel routes the SYN. The XDP programs are not vestigial
— they are reserved for the Phase 2 remote-backend case (per ADR-0042
§ 4 / ADR-0045 § Decision § 2), where a Service has backends on a
different host than the LB.

Per `.claude/rules/development.md` § "Deletion discipline" this is
*not* a candidate for deletion — the XDP programs defend the Phase
2 remote-backend path that this ADR explicitly preserves. They have
a future consumer; they stay.

### 6. Earned-Trust probe extension

The composition root invariant per ADR-0052 § 3 is "wire then probe
then use." The probe gains a cgroup-side step:

1. Existing probe: write sentinel `BackendEntry` at `BackendId::PROBE`,
   read back, assert byte-equal, delete (BPF maps + bpffs).
2. **New**: cgroup attach probe — confirm
   `BPF_CGROUP_INET4_CONNECT` attached to the configured cgroup path
   succeeds and the `LOCAL_BACKEND_MAP` accepts a sentinel upsert
   round-trip. The probe writes a sentinel `(vip=0.0.0.0,
   vip_port=0)` → `(backend=0.0.0.0:0)` entry, reads it back via
   `LocalBackendMapHandle::entries`, asserts presence, deletes it.

Failure surfaces as a new variant on `DataplaneBootError`:

```rust
#[error("cgroup_sock_addr attach failed (cgroup_path={cgroup_path}): {source}\n\n\
         Try: `bpftool cgroup show {cgroup_path}` to verify pre-existing \
         attachments; ensure CONFIG_CGROUP_BPF is enabled.")]
CgroupSockAddrAttach {
    cgroup_path: String,
    #[source]
    source: aya::programs::ProgramError,
},
```

Per `.claude/rules/development.md` § "Never flatten a typed error to
`Internal(String)`" — the new variant is `#[from]`-routed, never
flattened via `ControlPlaneError::internal`.

### 7. Operator config

The existing `[dataplane]` section (ADR-0052 § 3) gains one optional
field:

```toml
[dataplane]
client_iface = "lb_veth_a"
backend_iface = "lb_veth_b"
# Cgroup path the cgroup_sock_addr program attaches to. Must be an
# ancestor of the control-plane process AND every workload cgroup.
# Default = "/sys/fs/cgroup/overdrive.slice" (matches the slice
# crates/overdrive-worker/src/cgroup_manager.rs already manages).
cgroup_attach_path = "/sys/fs/cgroup/overdrive.slice"
```

The default matches the existing cgroup hierarchy; operators do not
need to set the field. Mis-set (the path does not exist, or is not
an ancestor of the control plane) surfaces as
`DataplaneBootError::CgroupSockAddrAttach` with the structured
remediation hint above.

### 8. Walking-skeleton becomes achievable, with bind-readiness discipline

The walking-skeleton TCP round-trip (A4 — per ADR-0052 § 4)
becomes structurally passable under this ADR. The test process must
run inside (or as a descendant of) the configured `cgroup_attach_path`
for the hook to fire. The Tier 3 integration test fixture already
runs the test binary as a child of the same control-plane process
(both inheriting the test's containing cgroup); making this explicit
in the fixture is a test-side change, not a production-code one.

The flake-mitigation shape ADR-0052 § 4 pinned (bind-readiness
poll-connect-with-timeout, `socat`-equivalent listener) still
applies. The new path adds no new flake surface.

## Consequences

### Positive

- **Walking-skeleton TCP round-trip becomes achievable in Phase 1.**
  The honest e2e gate ADR-0052 set up can land green. The "LB convergence
  works but no data flows" framing dissolves.
- **Cilium-aligned same-host primitive.** Operators familiar with
  Cilium kube-proxy-replacement recognise the shape immediately.
  The cgroup_sock_addr path is the dominant production same-host LB
  in the Kubernetes ecosystem.
- **Verifier surface is dramatically simpler than XDP.** No header
  parsing, no checksums, no FIB lookup, no MAC rewrite — the program
  reads two `u32`s, looks up one map, writes two `u32`s. Tier 4
  verifier-budget baseline is essentially free (≪ 10% of the
  per-program ceiling).
- **`LocalBackendMapHandle` and the trait method are Phase 2
  forward-compatible.** In Phase 2 when workloads land in their own
  netns, the cgroup attach point migrates (config change to
  `cgroup_attach_path`), not a code change. The map and the trait
  method survive into Phase 2+ for any same-cgroup-resident
  workload scenario (system components, host agents, sidecar
  patterns).
- **Earned-Trust probe surface grows by one orthogonal target.**
  The cgroup attach probe joins the existing BPF map probe — the
  composition root refuses to boot if the kernel cannot honour
  either contract.

### Negative

- **App `getpeername(2)` returns backend IP, not VIP.** Canonical
  Cilium / Kubernetes ClusterIP semantic. Applications that
  introspect their peer for TLS-SNI-style decisions, mTLS
  cert-subject matching where the subject was VIP, or audit
  logging of "which VIP did this connection target" will see
  unexpected values. **Phase 1 has no such applications;** if a
  future Overdrive feature requires VIP-preserving semantics
  (per-VIP TLS SNI, mTLS where cert subject = VIP, structured
  audit of VIP traversal), that feature will need to amend the
  cgroup path — likely by switching to `BPF_PROG_TYPE_SK_LOOKUP`
  for the affected services, which preserves the wire-visible VIP
  at the cost of a more invasive socket-layer model. This is a
  real future-amendment cost, surfaced explicitly per the
  research's Finding 1B.
- **Dataplane surface area grows by one program, one map, one
  handle, one action variant, one action shim, one error variant,
  one config field.** Bounded; symmetric with existing patterns
  (`xdp_service_map.rs` ↔ `cgroup_connect4_service.rs`;
  `BackendMapHandle` ↔ `LocalBackendMapHandle`;
  `DataplaneUpdateService` ↔ `RegisterLocalBackend`). Estimated
  ~600 LoC across kernel-side + userspace + reconciler + boot.
- **Cgroup attach point is Phase 1's correct shape and Phase 2's
  amendment surface.** When per-workload netns lands, the
  classifier in §4 still emits `RegisterLocalBackend` for
  same-cgroup-resident peers — but in Phase 2 most workloads will
  classify as remote (different netns from the LB cgroup) and
  flow through the XDP path. The cgroup program's role narrows;
  the program does not disappear, but its match rate drops to
  whatever fraction of workloads still share the LB's cgroup
  ancestor (system components, host agents). Surfaced for the next
  reader; not a deferral.
- **Kernel floor unchanged.** `BPF_CGROUP_INET4_CONNECT` is stable
  since kernel 4.17; Overdrive's floor is 5.10 LTS per
  `.claude/rules/testing.md` § "Kernel matrix". Comfortable margin.
  No kernel-version bump.
- **Tier 2 (`BPF_PROG_TEST_RUN`) coverage**: cgroup_sock_addr
  programs admit `BPF_PROG_TEST_RUN` differently than XDP — the
  context is `bpf_sock_addr` not a packet buffer. The project's
  hand-rolled `prog_test_run` helper at
  `crates/overdrive-dataplane/src/sys/prog_test_run.rs` accepts
  arbitrary `ctx_in` per the libbpf shape; the new program type
  needs its own Tier 2 fixture (PKTGEN replaced by a
  `bpf_sock_addr` builder). No new helper crate; one new fixture
  file at
  `crates/overdrive-bpf/tests/integration/cgroup_connect4_service.rs`.
- **Tier 3 (real-veth integration)**: the existing 3-netns
  `ThreeIfaceTopology` was built for the XDP wire-boundary path and
  remains the right shape for `xdp_service_map` / `xdp_reverse_nat`
  tests. The cgroup path's Tier 3 test is structurally simpler —
  one host netns, the test process runs as a descendant of the
  attach cgroup, the test issues a `connect(VIP, port)` and the
  workload's `ExecDriver`-spawned listener receives the connection
  on its real IP. No new topology helper crate.
- **`ExecDriver` `netns_path: Option<PathBuf>`** (commit `51512d7c`,
  pre-existing) **remains relevant.** This ADR does not retire the
  parameter — it remains the right primitive for Phase 2
  per-workload netns spawning, AND for any Tier 3 test that needs
  netns isolation of the LB-vs-backend boundary. The parameter and
  this ADR's cgroup path are orthogonal, not alternatives.

### Quality-attribute impact

- **Correctness — bug fix structurally closed**: positive (large).
  Walking-skeleton A4 unblocks; the "convergence works, data
  doesn't" framing dissolves. J-PLAT-004's value-delivery shape
  (intent → BPF map → actual TCP round-trip) closes for Phase 1.
- **Maintainability — modifiability**: positive. The reconciler's
  classifier is the single explicit decision site for "same-host
  vs remote"; future routing decisions extend the partition rather
  than adding new dispatch points.
- **Maintainability — testability**: positive. The cgroup program is
  trivial to PKTGEN-style test against `bpf_sock_addr` synthetic
  contexts; the Tier 3 fixture is dramatically simpler than the
  3-netns XDP topology.
- **Reliability — fault tolerance**: neutral. The cgroup attach
  probe joins the existing boot probe; refusal-at-boot semantics
  unchanged.
- **Reliability — recoverability**: neutral. The cgroup attachment
  is RAII via `aya::programs::CgroupSockAddrLinkId::Drop`; clean
  shutdown detaches. SIGKILL leaks a cgroup attachment that
  `bpftool cgroup detach` cleans (operator-side discipline, parallel
  to the XDP leak case in `.claude/rules/debugging.md` § "Leftover
  XDP attachments across runs"; the same recovery shape applies).
- **Operator usability**: positive. The new `cgroup_attach_path`
  field defaults to the slice the operator's `cgroup_manager`
  already manages; honest boot-time refusal with structured
  remediation per existing precedent.
- **Performance — time behaviour**: positive (small). Same-host
  connects skip the kernel routing path entirely — the rewrite
  happens before `ip_route_output_flow`. For Phase 1 single-node
  this is a strict throughput win over the broken XDP-on-local
  attempt.
- **Performance — resource utilisation**: neutral. One BPF program
  + one HASH map of size 4096. The HASH map's memory is upper-
  bounded by the number of (VIP, port) tuples × ~16 bytes; bounded
  by Phase 1 service count.
- **Security**: neutral. The cgroup attach point requires
  `CAP_BPF` + `CAP_NET_ADMIN` which the control-plane process
  already holds (per the existing `EbpfDataplane::new` path).
- **Portability**: neutral. Linux-only via existing
  `#[cfg(target_os = "linux")]` gates on `overdrive-dataplane`.

### Out of scope (explicit)

The shapes below are deliberately not part of Phase 1's
`register_local_backend` contract; mentioning them here so the next
reader does not infer they are implied. No forward pointer to a
future ADR or issue is created — these are simply not in scope
today.

- **IPv6 service VIPs.** `BPF_CGROUP_INET6_CONNECT` attach,
  `SocketAddrV6` in the trait, IPv6 `LOCAL_BACKEND_MAP` key. Lands
  with IPv6 VIP allocator support.
- **Unconnected-UDP services.** `SENDMSG` / `RECVMSG` cgroup hooks.
  Lands with a UDP service shape that requires per-datagram rewrite.
- **Multiple backends per (VIP, port) on the local path.** Phase 1
  Service spec assumes one alloc per Service in steady state; multi-
  alloc selection on the cgroup path (Maglev-style permutation, hash
  selection, weight-aware pick) lands when Service supports replicas.
- **Per-VIP TLS SNI / mTLS where cert-subject = VIP.** Requires
  preserving VIP semantics on the wire; would amend this ADR
  (likely toward `BPF_PROG_TYPE_SK_LOOKUP` for the affected
  services). Out of scope until a feature explicitly needs it.
- **Phase 2 per-workload netns isolation.** Larger architectural
  change (per-alloc netns lifecycle, IP pool allocator, veth
  creation per workload). Lands as its own ADR when Phase 2 is
  scheduled; the `ExecDriver` `netns_path` parameter is its
  prerequisite (already landed in commit `51512d7c`).

## Alternatives Considered

### A — `BPF_PROG_TYPE_SK_LOOKUP` (research recommendation, B-equivalent)

The originating research recommended `SK_LOOKUP` over
`cgroup_sock_addr`, arguing it is "semantically cleaner" because the
application sees its own VIP via `getsockname(2)` and no destination
rewrite is needed.

**Rejected** on grounds the research did not fully work through:

1. **The return path is not free.** SK_LOOKUP preserves the VIP as
   the wire-visible destination on the *ingress* leg — the kernel
   delivers the SYN to the backend's listening socket without
   rewriting the IP header. But the backend's TCP stack then
   constructs its SYN-ACK with `src=<backend_ip>`, NOT `src=VIP`.
   The client's client-side socket (which expects SYN-ACK from
   VIP) drops the packet. Cilium handles this by *also* installing
   cgroup-egress rewriting OR by maintaining per-connection
   conntrack state to fix up the reverse direction.
2. **"Cleaner fit" claim does not survive working through the
   return path.** Both Option A (`cgroup_sock_addr`) and Option B
   (`SK_LOOKUP`) require an end-to-end semantic. Option A's
   semantic is "rewrite at connect, kernel handles the rest
   naturally because the socket peer IS the backend"; Option B's
   semantic requires *additional* per-connection state OR
   *additional* egress rewriting that Option A does not. Strictly
   more complex, not less.
3. **Cilium ships Option A as its same-host LB.** The research
   established this directly. Adopting Option B would be diverging
   from the dominant production reference for an aesthetic
   property (`getpeername` returns VIP) that no Phase 1 application
   needs.
4. **`getpeername` returns backend IP IS the Kubernetes ClusterIP
   semantic.** Applications that expect to see ClusterIP via
   `getpeername` are non-conformant in the broader Kubernetes
   ecosystem too. Aligning with the dominant convention is a
   feature, not a defect.

The research's recommendation reflected a partial analysis of the
return path. This ADR diverges from the research on this specific
point with reasoning; the architect's review of the research is
recorded in the ADR per the user's standing rule that the architect
can push back on the researcher.

### B — TC + `bpf_sk_assign` (user's original tentative pick)

A TC-ingress program selects a listening socket via `bpf_sk_assign`.

**Rejected** because (a) not what Cilium uses for service LB — the
research established Cilium uses `cgroup_sock_addr` for connect-time
rewrite, NOT `bpf_sk_assign`; (b) `bpf_sk_assign` is the kernel
primitive paired with `SK_LOOKUP` (per kernel.org `prog_sk_lookup.rst`)
and inherits the return-path complications described in (A); (c) TC
ingress runs per-packet, after the kernel has begun routing — strictly
more work per connection than catching the connect syscall before any
packet has flown.

### C — Pull Phase 2 per-workload netns forward into Phase 1

Build per-allocation network namespaces in Phase 1 so every backend
becomes "remote" from the LB's POV, and the existing XDP wire-boundary
path applies uniformly.

**Rejected** because:

1. **Substantially larger scope.** Per-allocation netns lifecycle,
   IP-pool allocator, veth creation per workload, route plumbing
   per netns, sysctl hardening per netns, RAII cleanup on alloc
   exit. Easily 5–10× the LoC of this ADR.
2. **Violates `feedback_phase1_single_node_scope.md`.** Phase 1 is
   single-node; per-workload netns isolation is Phase 2 scope by
   the user's explicit framing.
3. **The `ExecDriver::netns_path` upstream half landed in commit
   `51512d7c` as a test-topology primitive AND as the Phase 2
   prerequisite.** That commit stands and is reused by Phase 2 when
   it lands. It is not retired by this ADR.
4. **Phase 2 will need an ADR of its own.** That ADR has not been
   written; no forward pointer to it exists today, and per
   `feedback_no_unilateral_gh_issues` the architect does not
   create issues unilaterally.

### D — Accept the limitation (Phase 1 LB ships convergence-only)

Land the walking-skeleton without A4 (TCP round-trip) and document
"Phase 1 LB ships convergence-only data path; reachability arrives
in Phase 2."

**Rejected** because a "load balancer" that does not deliver traffic
is a misleading product claim. Convergence-only is acceptable as an
*interim* state — the walking-skeleton can land A1–A3 only while
this ADR's implementation is in flight, with A4 added in the same
PR as the new path. As a Phase 1 permanent end-state, this option
is rejected.

### E — `XDP_REDIRECT` to loopback

Push the rewritten packet to `lo` via `XDP_REDIRECT` so the kernel's
local-delivery path picks it up.

**Rejected** per the research's Finding 9C: the upstream BPF community
explicitly rejected XDP-on-`lo` as a supported production pattern;
`lo` is a synthetic device for accounting that does not implement
the per-iface attach surface XDP assumes. Workaround-grade for
synthetic-packet tests only; not a production-shape data path.

### F — iptables / IPVS fallback for the same-host case

Use `iptables -t nat -A OUTPUT -j DNAT` or IPVS for the local-backend
rewrite, alongside the XDP wire-boundary path for remote.

**Rejected** on architectural grounds. Overdrive's whole dataplane
premise is "eBPF, not iptables" (per the whitepaper § 7 framing
that ADR-0040 / ADR-0042 carry forward). The cost of adopting
iptables for the same-host case is permanently coupling the
platform to a deprecated kernel subsystem for the one case the
whole eBPF stack was built to obviate.

### G — Use a separate cgroup program type (e.g., `cgroup_skb`) instead of `cgroup_sock_addr`

`BPF_PROG_TYPE_CGROUP_SKB` runs per-packet inside a cgroup; could
rewrite IP headers on egress.

**Rejected** because `cgroup_skb` runs *per-packet* (every packet
the cgroup's processes emit), not *per-connect*. The verifier
budget and runtime cost are linear in packet rate; the
`cgroup_sock_addr` connect-time hook fires once per connection.
Cilium uses `cgroup_skb` for per-packet *policy* enforcement (the
NetworkPolicy data path), NOT for LB rewrite. Right tool for a
different job.

## Compliance — what survives from prior ADRs

- **ADR-0035 (collapsed `Reconciler` trait)** — preserved verbatim. The hydrator's `reconcile` stays sync; classification is in-line logic.
- **ADR-0040 (three-map split + HoM atomic swap)** — preserved verbatim. New map is a flat HASH; HoM primitive is reserved for the XDP wire-boundary path where atomic backend-set rotation is needed.
- **ADR-0041 (endianness lockstep)** — followed. `LOCAL_BACKEND_MAP` keys/values are host-order; kernel-side converts wire bytes to host order at the boundary.
- **ADR-0042 (`ServiceMapHydrator`)** — extended (per-backend classifier added); existing DST invariants (`HydratorEventuallyConverges`, `HydratorIdempotentSteadyState`) extend naturally over the dual-emit shape.
- **ADR-0045 (`bpf_redirect_neigh` datapath)** — preserved verbatim. Wire-boundary path concern only; cgroup path does no L2 work.
- **ADR-0046 (`BackendId` allocator)** — preserved verbatim. `BackendId` is not consumed on the cgroup path.
- **ADR-0048 (rkyv versioned envelope)** — preserved verbatim. `ServiceBackendRow` schema is unchanged.
- **ADR-0049 (platform-issued Service VIP allocator)** — consumed. The cgroup path's `vip` argument is the allocator-issued VIP, sourced via the same `ServiceVipAllocator::get(&spec_digest)` path the bridge uses.
- **ADR-0050 (intent-side `WorkloadIntent` aggregate)** — preserved verbatim. The cgroup path consumes the same `ServiceV1.listeners` projection.
- **ADR-0052 (bridge + production boot)** — extended. Composition root gains one Earned-Trust probe step; `DataplaneBootError` gains one `#[from]` variant.
- **`.claude/rules/development.md` § Trait definitions specify behavior** — followed; trait method docstrings pin preconditions / postconditions / edge cases / observable invariants.
- **`.claude/rules/development.md` § Persist inputs, not derived state** — followed; no new persisted derived state.
- **`.claude/rules/development.md` § Errors / "Never flatten typed error to `Internal(String)`"** — followed; new `DataplaneBootError::CgroupSockAddrAttach` variant is `#[from]`-routed.
- **`feedback_phase1_single_node_scope.md`** — honored; this ADR is Phase 1-shaped and does not pull Phase 2 work forward.
- **`feedback_single_cut_greenfield_migrations.md`** — honored; no parallel old paths, no feature flag for "disable cgroup path."

## References

- `docs/research/dataplane/same-host-backend-delivery-architecture.md` — originating research (Nova, 2026-05-21). Architect diverges from the researcher's primitive recommendation on the grounds documented in Alternatives § A.
- `docs/research/testing/walking-skeleton-xdp-lb-topology.md` — companion research (Nova, 2026-05-21) on the topology aspect; informs the Phase 1 same-netns shape this ADR uses.
- `docs/research/dataplane/xdp-l4lb-test-topology-comprehensive-research.md` — prior research (Nova, 2026-05-06) establishing the 3-netns canonical XDP topology that remains correct for the Phase 2 remote-backend case.
- `docs/feature/backend-discovery-bridge-service-reachability/deliver/rca-walking-skeleton-tcp-roundtrip.md` — RCA establishing that A4 fails by architecture, not by fixture or convergence-chain bug.
- ADR-0040 (SERVICE_MAP three-map split + HASH_OF_MAPS) — preserved substrate.
- ADR-0041 (weighted Maglev + REVERSE_NAT + endianness lockstep) — preserved substrate.
- ADR-0042 (`ServiceMapHydrator` + `service_hydration_results`) — extended consumer.
- ADR-0045 (`bpf_redirect_neigh` datapath) — preserved substrate.
- ADR-0049 (platform-issued Service VIP allocator) — consumed dependency.
- ADR-0050 (intent-side workload aggregate) — preserved substrate.
- ADR-0052 (backend discovery bridge + EbpfDataplane production boot) — extended substrate.
- Cilium project. "Kubernetes Without kube-proxy". docs.cilium.io. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-05-21.
- Cilium project. "XDP Acceleration". docs.cilium.io. https://docs.cilium.io/en/stable/operations/performance/tuning/#xdp-acceleration. Accessed 2026-05-21.
- Linux Kernel Authors. "BPF — `BPF_PROG_TYPE_CGROUP_SOCK_ADDR`". docs.kernel.org. Accessed 2026-05-21.
- `crates/overdrive-bpf/src/programs/xdp_service_map.rs` (`xdp_service_map_lookup`) — wire-boundary forward path; preserved.
- `crates/overdrive-bpf/src/programs/xdp_reverse_nat.rs` (`xdp_reverse_nat_lookup`) — wire-boundary response path; preserved.
- `crates/overdrive-core/src/traits/dataplane.rs:70-84` — existing `Dataplane` trait; gains two parallel methods.
- `crates/overdrive-worker/src/cgroup_manager.rs` — existing cgroup hierarchy management; the default `cgroup_attach_path` matches the slice this manages.
- Commit `51512d7c` — `feat(worker): add opt-in netns_path to ExecDriver` (Phase 2 prerequisite + Tier 3 test-topology primitive; preserved, not retired by this ADR).
- `.claude/rules/development.md` § Reconciler I/O.
- `.claude/rules/development.md` § Trait definitions specify behavior, not just signature.
- `.claude/rules/development.md` § Persist inputs, not derived state.
- `.claude/rules/development.md` § Errors / "Never flatten typed error to `Internal(String)`".
- `.claude/rules/development.md` § aya-rs XDP / TC kernel-side patterns (the program-shape conventions extend cleanly to `cgroup_sock_addr`).
- `.claude/rules/testing.md` § Tier 2 / Tier 3 (the existing tier discipline applies to the new program).
- `feedback_phase1_single_node_scope.md`.
- `feedback_single_cut_greenfield_migrations.md`.

## Revision 2026-06-03 — LOCAL_BACKEND_MAP key gains L4 protocol (`(VIP, vip_port)` → `(VIP, vip_port, proto)`)

### Status

Amendment. 2026-06-03. Decision-maker: Morgan; user-locked (resolves
P2-Q4 on the `udp-service-support` feature — "do `(vip, port, proto)`
as IPVS"). Tags: phase-1, dataplane, cgroup-bpf, local-backend,
l4-proto-keying, udp-service-support.

**Feature SSOT**:
`docs/feature/udp-service-support/feature-delta.md` § "Wave: DESIGN /
[REF] P2-Q4 resolution — proto in the service-LB map keys".
**Decision record**:
`docs/feature/udp-service-support/design/wave-decisions.md` (P2-Q4).
**Evidence base**:
`docs/research/dataplane/service-map-l4-proto-keying-research.md`
(Nova, 2026-06-03, High confidence). **Companion amendment**:
ADR-0040 revision 2026-06-03 (the same proto dimension threaded into
the SERVICE_MAP *wire-boundary* forward key; this amendment threads
it into the *same-host* cgroup path).

### Why this amendment

Decision 1 above locked `LOCAL_BACKEND_MAP` as "one entry per
`(VIP, vip_port)`" — proto-less. That shape cannot represent a
same-host service that listens on both TCP and UDP on the same
`(VIP, vip_port)`: the canonical DNS case (`tcp/53 + udp/53`) and the
HTTP/3 case (`443/tcp + 443/udp`). A single `LocalServiceKey {
vip_host, port_host, _pad }` collapses the two listeners into one
map slot, so a UDP `connect(VIP:53)` and a TCP `connect(VIP:53)` would
rewrite to the same backend — losing the per-protocol distinction the
operator declared.

The same IPVS-alignment rationale that drives the SERVICE_MAP
amendment (ADR-0040 revision 2026-06-03) applies here: every
kernel-native L4 LB keys on `{protocol, addr, port}` (IPVS UAPI
`ip_vs_service_user`); proto-less keying is the defect Cilium spent
~5.5 years closing (#9207 → #37164). The cgroup same-host path is no
exception — it is a per-`(VIP, port)` rewrite table and must carry
proto to distinguish co-located TCP and UDP services.

The cost is the same near-zero widening: `LocalServiceKey`
(`crates/overdrive-bpf/src/maps/local_backend_map.rs:27-34`) is an
8-byte `#[repr(C)]` POD with a trailing `_pad: u16`; the proto byte
absorbs one pad byte and the struct stays 8 bytes.

### Amendment 1 — `LOCAL_BACKEND_MAP` key gains proto

Decision 1's `LOCAL_BACKEND_MAP` key row is amended:

| Field | Type (amended) | Notes |
|---|---|---|
| Key | `LocalServiceKey { vip: u32, port: u16, proto: u8, _pad: u8 }` (host order, `#[repr(C)]`, 8 bytes) | The `proto: u8` absorbs one of the two reserved `_pad` bytes — same trick as ADR-0040's `ServiceKey`. The trailing `_pad: u8` **MUST stay deterministically zeroed** (BPF hash maps key on raw bytes). `proto` is the IANA L4 number (IPPROTO_TCP=6 / IPPROTO_UDP=17), lowered from ADR-0060's typed `Proto` at the userspace map-write edge via `Proto::as_u8()`. |

The value (`LocalBackendEntry { backend_ip, backend_port, _pad }`) is
**unchanged** — proto is a *key* dimension, not a value dimension.
Capacity (`max_entries = 4096`) is unchanged.

The userspace handle
(`crates/overdrive-dataplane/src/maps/local_backend_map_handle.rs`)
gains a `proto` parameter on `upsert` and `remove`:

```rust
pub fn upsert(&self, vip: Ipv4Addr, vip_port: u16, proto: Proto, backend: SocketAddrV4) -> Result<(), MapError>;
pub fn remove(&self, vip: Ipv4Addr, vip_port: u16, proto: Proto) -> Result<(), MapError>;
```

`Proto` is `overdrive_core::dataplane::backend_key::Proto` (ADR-0060),
reused — NOT a new enum. The typed handle continues to enforce
IPv4-only via `SocketAddrV4` at the boundary.

### Amendment 2 — cgroup_connect4 proto-source contract

This is the load-bearing contract decision the amendment must pin.
The `cgroup_connect4_service` program
(`crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs`)
today reads only `(user_ip4, user_port)` from `bpf_sock_addr` and
keys `LOCAL_BACKEND_MAP` on `(vip_host, port_host)`. To key on proto
it must derive the L4 protocol from the syscall context.

**Verified against the in-tree `bpf_sock_addr` UAPI**
(`aya-ebpf-bindings-0.1.2/.../bindings.rs:2335-2346`): the
`bpf_sock_addr` struct exposes **both** of:

```c
__u32 user_family;
__u32 user_ip4;
...
__u32 user_port;
__u32 family;
__u32 type;        // socket type: SOCK_STREAM=1, SOCK_DGRAM=2
__u32 protocol;    // IANA L4 protocol: IPPROTO_TCP=6, IPPROTO_UDP=17
...
```

**The contract: read `bpf_sock_addr.protocol` as the primary proto
source.** It carries the IANA L4 protocol number directly
(IPPROTO_TCP=6 / IPPROTO_UDP=17), so it maps 1:1 onto the
`LocalServiceKey.proto` byte with **no translation table**. This is
strictly simpler and less error-prone than the socket-type→proto
mapping the DISCUSS framing hypothesised. Concretely the kernel-side
key construction becomes:

```rust
// host-order proto byte, read directly from the syscall context.
// bpf_sock_addr.protocol is the IANA L4 number; no byte-swap (single
// byte), no SOCK_*→IPPROTO_* mapping table.
let proto = unsafe { (*sock_addr).protocol as u8 };   // 6 (TCP) | 17 (UDP)
let key = LocalServiceKey { vip_host, port_host, proto, _pad: 0 };
```

**Fallback / robustness clause.** `bpf_sock_addr.type` (SOCK_STREAM=1
→ IPPROTO_TCP=6, SOCK_DGRAM=2 → IPPROTO_UDP=17) is the documented
fallback derivation **only if** a kernel in the matrix is observed to
leave `protocol` zero/unset for `connect4` (no such kernel is known
on the 5.10+ floor; `protocol` is populated for connect-family hooks).
The crafter MUST verify `protocol` is populated on the Tier 3 kernel
matrix; if any matrix kernel leaves it zero, derive proto from `type`
via the SOCK_*→IPPROTO_* mapping above. Either way the key carries the
IANA byte — the map key shape is identical. **No Tier 2 backstop
exists** for `cgroup_sock_addr` (`BPF_PROG_TEST_RUN` returns ENOTSUPP
for this program type on kernel ≤ 6.8 — see `.claude/rules/development.md`
§ "`bpf_sock_addr.user_port` — low-16-NBO in a u32"); the proto-source
correctness is a **Tier 3** verification (real `connect()` through the
cgroup), not a unit-testable one.

The connect4 hook fires for **both** TCP `connect()` and
**connected-UDP** `connect()` (a UDP socket that calls `connect(2)`
to fix its peer before `send()`), exactly as Decision 1 § 1 scoped.
For both, `bpf_sock_addr.protocol` carries the correct IANA byte, so
the proto-keyed lookup works for connected-UDP services with no
additional hook.

### Amendment 3 — `Action::RegisterLocalBackend` / `DeregisterLocalBackend` gain proto

Decision 3's action variants
(`crates/overdrive-core/src/reconcilers/mod.rs:485-509`) carry no
proto field today. Both gain one, sourced from the same listener-bearing
fact ADR-0060 § "True blast radius (D6)" site #8 pins (the `Listener`
proto, NEVER a silent `Proto::Tcp` default — C3):

```rust
RegisterLocalBackend {
    service_id:  ServiceId,
    vip:         Ipv4Addr,
    vip_port:    u16,
    proto:       Proto,        // NEW — per-listener L4 proto (ADR-0060 Proto)
    backend:     SocketAddrV4,
    correlation: CorrelationKey,
},
DeregisterLocalBackend {
    service_id:  ServiceId,
    vip:         Ipv4Addr,
    vip_port:    u16,
    proto:       Proto,        // NEW
    correlation: CorrelationKey,
},
```

The `ServiceMapHydrator` (Decision 4) already has proto per-listener
once ADR-0060's site #8 lands (the desired projection sourced from a
listener-bearing fact); its Local-vs-Remote classifier threads
`backend.proto` / the listener proto into `RegisterLocalBackend`
alongside the existing fields. The action-shim
(`action_shim/register_local_backend.rs`) and the
`Dataplane::register_local_backend` / `deregister_local_backend`
trait methods gain a `proto: Proto` parameter, threaded to
`LocalBackendMapHandle::upsert` / `remove`.

The `register_local_backend` trait-method rustdoc contract
(Decision 2) is amended: the per-`(vip, vip_port)` postconditions and
edge cases now read per-`(vip, vip_port, proto)`. Specifically:
re-registration with a different `backend` for the same
`(vip, vip_port, proto)` atomically replaces that entry;
`deregister_local_backend(vip, vip_port, proto)` removes only that
proto's entry, leaving a co-located other-proto entry for the same
`(vip, vip_port)` intact (parity with ADR-0060's per-proto purge for
the REVERSE_NAT path — D4).

### Amendment 4 — sendmsg4 / unconnected-UDP remains explicitly out of scope (NOT silently assumed in)

Decision 1 § 1 already scoped `SENDMSG` / `RECVMSG` cgroup hooks as
out of Phase 1 scope ("unconnected-UDP support is a separate
concern"). This amendment **reaffirms and sharpens** that boundary in
light of UDP services becoming first-class:

- **In scope (this amendment):** TCP `connect()` and **connected-UDP**
  `connect()` — both route through `BPF_CGROUP_INET4_CONNECT`
  (`cgroup_connect4_service`), and both now key on proto. A UDP client
  that calls `connect(VIP:port)` before `send()` IS handled.
- **Out of scope (a separate hook, NOT delivered here):**
  **unconnected-UDP** — a UDP client that calls `sendto(VIP:port, ...)`
  / `sendmsg()` **without** a prior `connect()`. The kernel does not
  fire `connect4` for these datagrams; the destination rewrite would
  need a **`BPF_CGROUP_UDP4_SENDMSG`** program (`sendmsg4`), which is a
  **separate hook with a separate `bpf_sock_addr`-shaped context** and
  is **not implemented in this codebase today**. This amendment does
  NOT silently assume sendmsg4 coverage. It is surfaced as an open
  question with a recommendation (see below) — the DELIVER scope for
  proto-keyed UDP covers the *connected*-UDP path only.

This boundary matters operationally: the canonical UDP driver (DNS)
uses **unconnected** UDP from most resolvers (`sendto` per query, no
`connect`). So a DNS *server* deployed as an Overdrive service is
reachable via the connected-UDP path only if the *client* connects
first — many DNS clients do not. The sendmsg4 hook is what closes
that gap. It is a real, named follow-up tracked as
[#200](https://github.com/overdrive-sh/overdrive/issues/200) (see
§ Out of scope), NOT a hand-wavy forward pointer.

### Migration — single-cut, reconciler-repopulated; no shim

Identical posture to ADR-0040's amendment: `LOCAL_BACKEND_MAP` is
repopulated from intent on boot by the hydrator's per-backend
classifier (Decision 4). The migration is "the key struct changes;
the map is recreated on next boot." NO live in-place migration, NO
dual-key shim, NO deprecation path. DELIVER must NOT build a migration
shim — the key struct edit + the hydrator repopulation IS the
migration. Per `feedback_single_cut_greenfield_migrations.md`.

### What this amendment supersedes vs preserves

| Original decision | Status |
|---|---|
| Decision 1 § 1 — adopt `cgroup_sock_addr` connect-time rewrite | **Preserved.** The mechanism is unchanged; only the lookup key widens. |
| Decision 1 § 1 — `LOCAL_BACKEND_MAP` "one entry per `(VIP, vip_port)`" | **Amended** to "one entry per `(VIP, vip_port, proto)`." |
| Decision 1 § 1 — connect4 in scope; SENDMSG/RECVMSG out of scope | **Preserved and sharpened** (Amendment 4): connected-UDP `connect4` is in; unconnected-UDP sendmsg4 is explicitly a separate, undelivered hook. |
| Decision 2 — `register_local_backend` / `deregister_local_backend` trait methods | **Extended** — gain a `proto: Proto` parameter; contract re-pinned per-proto. |
| Decision 3 — `Action::RegisterLocalBackend` / `DeregisterLocalBackend` | **Extended** — gain a `proto: Proto` field. |
| Decision 4 — hydrator Local-vs-Remote classifier | **Extended** — threads per-listener proto into `RegisterLocalBackend`; sources proto from the listener-bearing fact (ADR-0060 site #8), never a `Tcp` default. |
| Decision 5 (XDP programs unchanged), 6 (Earned-Trust probe), 7 (operator config), 8 (walking-skeleton) | **Preserved.** The probe's sentinel upsert gains a proto arg (`(vip=0,port=0,proto=tcp)`); otherwise unchanged. |
| Out of scope § "IPv6 service VIPs" | **Preserved** — IPv6 + `BPF_CGROUP_INET6_CONNECT` still out of scope (GH #155 territory). |

### Consequences

**Positive.**

- A same-host service with co-located TCP and UDP listeners on one
  `(VIP, vip_port)` is representable — the cgroup path rewrites each
  proto's `connect()` to its declared backend.
- The same-host path aligns with the wire-boundary path: SERVICE_MAP
  (ADR-0040 amendment), REVERSE_NAT (ADR-0060), and LOCAL_BACKEND_MAP
  (this amendment) all key on `(…, proto)`. One proto dimension,
  three maps, consistent.
- `bpf_sock_addr.protocol` gives a zero-translation proto source —
  cleaner than the socket-type mapping the DISCUSS framing assumed.
- Zero byte-width cost: 8-byte key before and after.

**Negative / accepted.**

- `LocalServiceKey` layout changes (proto at offset 6, pad narrows to
  1 byte) — single-cut; the kernel-side program, the userspace handle,
  the action variants, the trait methods, and the Tier 3 cgroup test
  update in the same PR. DELIVER concern, noted for blast-radius.
- **Unconnected-UDP (`sendto` without `connect`) is not delivered** by
  this amendment — a real functional gap for UDP clients that do not
  connect first (DNS being the prominent case). Surfaced as an open
  question + recommendation, not silently assumed. See § Out of scope.

### Out of scope (explicit, additive to Decision 1's list)

- **Unconnected-UDP via `sendmsg4`.** A `BPF_CGROUP_UDP4_SENDMSG`
  program to rewrite the destination of `sendto(VIP, ...)` datagrams
  that never call `connect()`. Separate hook, separate context,
  **not implemented today**. **Architect recommendation:** this is a
  genuine follow-up worth a tracked GitHub issue — it is required for
  unconnected-UDP clients (notably DNS resolvers that `sendto` per
  query) to reach a same-host UDP service. Tracked:
  [#200](https://github.com/overdrive-sh/overdrive/issues/200).

### Cross-references

- ADR-0040 revision 2026-06-03 — the companion amendment threading
  the same proto dimension into the SERVICE_MAP wire-boundary forward
  key.
- ADR-0060 — `Proto` reused here; REVERSE_NAT per-proto purge (D4) is
  the parity the local-backend per-proto deregister mirrors; site #8
  (proto from a listener-bearing fact, never `Tcp`-default) is the
  proto source for the `RegisterLocalBackend` action.
- `docs/research/dataplane/service-map-l4-proto-keying-research.md`
  — IPVS / Cilium / Kubernetes evidence base.
- `crates/overdrive-bpf/src/maps/local_backend_map.rs:27-34`
  (`LocalServiceKey` — gains proto byte),
  `crates/overdrive-bpf/src/programs/cgroup_connect4_service.rs:56-76`
  (reads `bpf_sock_addr.protocol`; builds the proto-keyed key),
  `crates/overdrive-dataplane/src/maps/local_backend_map_handle.rs`
  (handle gains proto param),
  `crates/overdrive-core/src/reconcilers/mod.rs:485-509`
  (`Action::RegisterLocalBackend` / `DeregisterLocalBackend` gain
  proto).
- `aya-ebpf-bindings-0.1.2` `bpf_sock_addr` UAPI struct (verified:
  exposes both `protocol` and `type`).

### Changelog (Revision 2026-06-03)

| Date | Change |
|---|---|
| 2026-06-03 | LOCAL_BACKEND_MAP key `(VIP, vip_port)` → `(VIP, vip_port, proto)`, IPVS-style. cgroup_connect4 proto-source contract pinned to `bpf_sock_addr.protocol` (IANA byte, zero-translation; `type` as documented fallback). `Action::RegisterLocalBackend`/`DeregisterLocalBackend` + trait methods gain `proto: Proto`. Connected-UDP in scope; unconnected-UDP via sendmsg4 explicitly out of scope (separate undelivered hook, tracked as [#200](https://github.com/overdrive-sh/overdrive/issues/200)). Single-cut reconciler-repopulated migration; no shim. Resolves P2-Q4 (`udp-service-support`) for the same-host path. — Morgan (user-locked). |

## Revision 2026-06-03 — dispatch-boundary conflict granularity is `(route, key-tuple)`, NOT the shared VIP (cross-route dual-path is blessed, not a conflict)

### Status

Amendment. 2026-06-03. Decision-maker: Morgan. This amendment does NOT
change the architecture — it states authoritatively a property that
Decisions 2, 4, and 5 above already imply, because a later artifact
(the `validate_reconcile_output` runtime validator) contradicted it
and misattributed its provenance. This is a correction of the
design-artifact surface so the next reader does not re-derive the
over-broad rule.

**Triggering RCA**: a completed root-cause analysis found that
`validate_reconcile_output` in
`crates/overdrive-control-plane/src/action_shim/validate.rs` rejects a
reconcile output that emits BOTH a `DataplaneUpdateService` (XDP
`SERVICE_MAP` write) AND a `RegisterLocalBackend` (cgroup
`LOCAL_BACKEND_MAP` write) for the same VIP in one tick — calling it a
"cross-route conflict" (its "Conflict class 2") and citing "the Phase
16 D11 finding." Both the rule and the citation are wrong against this
ADR.

**Evidence base**:
`docs/research/reconcilers/dispatch-boundary-validation-and-attempt-budget-backoff.md`
(Nova, 2026-06-03, High confidence) — Kubernetes Server-Side Apply
field-manager conflict granularity (conflict = collision on an owned
field, never co-residence on the shared parent object) and Cilium
socket-LB (`cgroup connect4`) ⊥ XDP/tc datapath as complementary,
explicitly "transparent" surfaces for one ClusterIP.

### The property this ADR already establishes

Three load-bearing statements above pin that cross-route writes on one
VIP are the **intended dual-path**, not a conflict:

- **Decision 2 (observable invariant, lines ~295–300):** *"`update_service(vip, ...)`
  and `register_local_backend(vip, port, ...)` for the same VIP are not
  mutually exclusive at the adapter — the XDP path consumes the first,
  the cgroup path consumes the second. The classifier in
  `ServiceMapHydrator` (§ 4) is responsible for choosing exactly one
  per backend."*
- **Decision 4 (mixed-backend emission, lines ~390–394):** *"A service
  with mixed local and remote backends (a Phase 2+ shape) emits both
  action kinds for the same `service_id`. The two dataplane paths are
  independent and the trait contract permits this concurrent dual-path."*
- **Decision 5 (no precedence race, lines ~408–413):** *"Traffic that
  would match a backend's IP never traverses XDP because the cgroup
  rewrite happens before the kernel routes the SYN."* `cgroup_connect4`
  fires at `connect(2)` time, before the SYN exists; XDP fires at wire
  ingress. Two disjoint kernel maps, two hooks, disjoint backend sets
  (local vs remote). There is no shared slot and no precedence
  ambiguity.

### Amendment — the dispatch-boundary invariant, stated authoritatively

The runtime validator at the action-shim dispatch boundary (ADR-0023)
MUST detect reconcile-output conflicts at **`(route, key-tuple)`
granularity, never at the shared-VIP level**:

1. **Genuine conflicts — same route, same key-tuple (a real
   last-writer-wins overwrite of one map slot):**
   - **XDP-vs-XDP:** two `DataplaneUpdateService` writes to the same
     `SERVICE_MAP` key `(vip, port, proto)` (per ADR-0040 revision
     2026-06-03; pre-02-01 this key was VIP-only). *Conflict.*
   - **Cgroup-vs-cgroup:** two `RegisterLocalBackend` /
     `DeregisterLocalBackend` writes to the same `LOCAL_BACKEND_MAP`
     key `(vip, vip_port, proto)` (per this ADR's revision 2026-06-03;
     pre-02-02 this key was `(vip, vip_port)`). *Conflict.*
2. **NOT a conflict — cross-route on the same VIP:** an XDP
   `SERVICE_MAP` write AND a cgroup `LOCAL_BACKEND_MAP` write for the
   same VIP in one tick. **This is the blessed dual-path of Decisions
   2/4/5 above.** The two routes are disjoint kernel maps consumed by
   different hooks with no precedence race; the backend sets are
   disjoint (local XOR remote per backend, chosen by the §4
   classifier). A VIP appearing on both routes is the *correct* shape
   for a mixed local+remote service, not a defect. The validator MUST
   NOT reject it.

The key tuples are the *actual map keys* (post-2026-06-03 amendments:
XDP `(vip, port, proto)`, cgroup `(vip, vip_port, proto)`). Disjoint
ports and disjoint proto are distinct slots on either route and do not
conflict — the same per-`(…, proto)` granularity the SERVICE_MAP /
REVERSE_NAT / LOCAL_BACKEND_MAP amendments established.

### External precedent (from the evidence-base research)

- **Kubernetes Server-Side Apply** keys conflict detection on the
  individual owned field, never on the whole object: two managers on
  disjoint fields of one object never conflict (conflict is the *set
  intersection* of owned field paths, computed by
  `sigs.k8s.io/structured-merge-diff`). The owned leaf — the
  `(map, key)` slot here — is the unit of conflict, not the shared
  parent (the VIP). Source: https://kubernetes.io/docs/reference/using-api/server-side-apply/
  (accessed 2026-06-03).
- **Cilium** runs socket-LB (`cgroup/connect4`) and the wire-time
  XDP/tc datapath as complementary surfaces for the same service
  ClusterIP: *"The socket-level loadbalancer acts transparent to
  Cilium's lower layer datapath."* The connect-time rewrite happens
  before any XDP ingress decision, so the two paths cannot race on the
  same key — the dataplane-specific instance of the SSA principle and
  the direct analogue to Overdrive's `RegisterLocalBackend` (cgroup,
  local) + `DataplaneUpdateService` (XDP, remote) pair. Source:
  https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/
  (accessed 2026-06-03).

### Why this lives here and not in a new ADR

The validator is not a new architectural decision — it is a *runtime
enforcement* of an invariant this ADR already specifies (Decisions
2/4/5) plus the same-class observation-row finding D11 (see below).
Its correct conflict granularity is therefore an amendment to this
ADR's existing contract, not a standalone decision record. No new ADR
is warranted; opening one would imply a fresh option-space that does
not exist — the option-space was settled when Decisions 2/4/5 landed.

### D11 provenance correction

The validator cites *"the Phase 16 D11 finding"* as the authority for
its cross-route rule. **The citation is misattributed.** The real D11
finding (`docs/evolution/2026-05-23-backend-discovery-bridge-service-reachability.md`
§ "Reconcile-output invariant at the action_shim boundary (D11,
`6d62afe6`)") is about two **same-class** `WriteServiceBackendRow`
Actions (observation-row writes) targeting one VIP with *conflicting
backend sets* — a genuine last-writer-wins overwrite of one
observation slot. D11 says nothing about XDP+cgroup cross-route
composition. The validator generalised a same-slot-overwrite finding
into a cross-route rule that contradicts this ADR. D11 governs
**same-route same-key** overwrites only (conflict class 1); it does
NOT authorise the cross-route rule (the rejected non-conflict, class
2). The evolution doc carries a matching clarifying note as of
2026-06-03.

### What this amendment changes vs preserves

| Element | Status |
|---|---|
| Decisions 2 / 4 / 5 (cross-route dual-path is intended) | **Preserved and restated authoritatively.** No change; this amendment makes the already-implied property explicit so the validator can be brought into line. |
| `validate_reconcile_output` conflict granularity | **Corrected (code fix is a separate `/nw-deliver`).** Conflict at `(route, key-tuple)`; cross-route-on-same-VIP rule removed. The architect provides the corrected module-doc prose; the code change is out of this amendment's scope. |
| D11 provenance citation in `validate.rs` | **Corrected.** D11 governs same-class observation-row write conflicts only; the cross-route rule is not derived from it. |

### Cross-references

- `docs/research/reconcilers/dispatch-boundary-validation-and-attempt-budget-backoff.md`
  — SSA field-manager + Cilium socket-LB/XDP evidence base.
- `docs/evolution/2026-05-23-backend-discovery-bridge-service-reachability.md`
  § D11 — the same-class observation-row finding the validator
  misattributed (carries a matching clarifying note as of 2026-06-03).
- `crates/overdrive-control-plane/src/action_shim/validate.rs` — the
  validator whose conflict granularity is corrected by the companion
  `/nw-deliver` code fix.
- ADR-0040 revision 2026-06-03 (SERVICE_MAP `(vip, port, proto)` key) /
  this ADR's revision 2026-06-03 (LOCAL_BACKEND_MAP `(vip, vip_port,
  proto)` key) — the actual map key tuples the `(route, key-tuple)`
  granularity refers to.
- ADR-0023 (action-shim dispatch boundary) — the layer the validator
  runs at.

### Changelog (Revision 2026-06-03 — dispatch-boundary granularity)

| Date | Change |
|---|---|
| 2026-06-03 | State authoritatively: cross-route writes (XDP-for-remote + cgroup-for-local) on the same VIP are the blessed dual-path of Decisions 2/4/5 and are explicitly NOT a conflict. The only genuine reconcile-output conflicts are same-route same-key overwrites: two XDP writes to `(vip, port, proto)`, or two cgroup writes to `(vip, vip_port, proto)`. Dispatch-boundary validator MUST detect conflicts at `(route, key-tuple)` granularity, never at the shared VIP. Cite SSA field-manager + Cilium socket-LB/XDP precedent. Correct the validator's D11 misattribution (D11 governs same-class observation-row write conflicts only). Code fix is a separate `/nw-deliver`. — Morgan. |

## Revision 2026-06-05 — unconnected-UDP delivery via sendmsg4 + recvmsg4 (closes #200)

### Status

Amendment. 2026-06-05. Decision-maker: Morgan; **all decisions
user-locked** (GUIDE-mode framing pass complete; this revision WRITES
the locked decisions). Tags: phase-1, dataplane, cgroup-bpf,
local-backend, unconnected-udp, sendmsg4, recvmsg4, reverse-local-map,
j-plat-004, j-ops-004.

**Feature SSOT**:
`docs/feature/unconnected-udp-sendmsg4/feature-delta.md` § "Wave:
DESIGN". **Decision record**:
`docs/feature/unconnected-udp-sendmsg4/design/wave-decisions.md`.
**Evidence base (load-bearing — cited throughout)**:
`docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md`
(Nova, 2026-06-05, High confidence; the recvmsg4 verifier `[1,1]`
cannot-deny finding is the crux that reshapes D3). **Companion**: the
Amendment 4 of the 2026-06-03 revision (which scoped sendmsg4 OUT and
tracked it as #200) is **DELIVERED by this revision**.

### Why this revision

Amendment 4 (2026-06-03) sharpened the connect4-vs-sendmsg4 boundary:
connected-UDP (`connect(VIP)` before `send`) routes through the shipped
`cgroup_connect4_service`; **unconnected-UDP** (`sendto(VIP, ...)` with
no prior `connect()`) does NOT fire `connect4` and was explicitly out
of scope, tracked as [#200](https://github.com/overdrive-sh/overdrive/issues/200).

That gap is operationally decisive: the canonical UDP driver is DNS,
and the dominant resolver idiom (`dig`, glibc `getaddrinfo`, musl) is
**unconnected** — `sendto(VIP)` per query, never `connect()`. A
same-host DNS service deployed today is reachable only by clients that
connect first; most do not. This revision closes that gap with the two
hooks Amendment 4 named: `cgroup/sendmsg4` (forward request rewrite)
and `cgroup/recvmsg4` (reply source rewrite), plus the new reverse
store the reply path requires.

### Decisions (all user-locked)

#### D1 — Reverse store is a second BPF map `REVERSE_LOCAL_MAP`, dual-written in ordered (reverse-first) sequence

The reply path needs a `backend → VIP` lookup. The store is a **second
`BPF_MAP_TYPE_HASH` map, `REVERSE_LOCAL_MAP`** — NOT a reverse scan of
`LOCAL_BACKEND_MAP` (O(N) per datagram, unacceptable on the recvmsg
hot path) and NOT a conntrack / per-flow state table (UDP is stateless;
there is no flow to track — the same `D7` rejection the DISCUSS wave
locked).

`REVERSE_LOCAL_MAP` is written **by the same `register_local_backend`
call that writes the forward `LOCAL_BACKEND_MAP` entry** (D5a). The two
writes are **two separate BPF map syscalls, not one transaction** — the
guarantee is an **ordering** guarantee, not atomicity: they are issued
in **ordered (reverse-first)** sequence, the reverse `backend → VIP`
entry installed *before* the forward `(vip, vip_port, proto) → backend`
entry. Reverse-first ordering guarantees the reply path is never ahead
of the request path — there is no window in which a request could be
forward-rewritten and routed to a backend whose reply has no reverse
entry yet. (The request rewrite is what *causes* the backend to send a
reply; if forward landed first, a fast backend could reply into a
reverse-map gap.) `deregister_local_backend` removes both symmetrically.

| Field | Type | Notes |
|---|---|---|
| Map type | `BPF_MAP_TYPE_HASH` | Single global, point-access only. Mirrors `LOCAL_BACKEND_MAP`'s shape; no HoM (no atomic-swap-of-backend-set requirement on this path). |
| Key | `BackendKey { ip: Ipv4Addr, port: u16, proto: Proto }` (D2) — host-order, the **existing newtype** at `crates/overdrive-core/src/dataplane/backend_key.rs` | Byte-parity with SERVICE_MAP / REVERSE_NAT / LOCAL_BACKEND_MAP. Endianness lockstep per ADR-0041: userspace writes host order; kernel-side `recvmsg4` converts the skb-populated `user_ip4` wire bytes to host order at the lookup boundary. |
| Value | `u32` (host-order `Ipv4Addr` of the VIP) | The VIP the reply source is rewritten to on a hit. Single VIP per backend key. |
| `max_entries` | 4096 | Same envelope as `LOCAL_BACKEND_MAP`. |

New userspace handle
`crates/overdrive-dataplane/src/maps/reverse_local_map_handle.rs`,
typed shape mirrors `LocalBackendMapHandle`:

```rust
pub struct ReverseLocalMapHandle { /* Map fd */ }

impl ReverseLocalMapHandle {
    pub fn upsert(&self, backend: SocketAddrV4, proto: Proto, vip: Ipv4Addr) -> Result<(), MapError>;
    pub fn remove(&self, backend: SocketAddrV4, proto: Proto) -> Result<(), MapError>;
    pub fn get(&self, backend: SocketAddrV4, proto: Proto) -> Result<Option<Ipv4Addr>, MapError>;
    pub fn entries(&self) -> Result<Vec<(BackendKey, Ipv4Addr)>, MapError>;
}
```

#### D2 — Reverse key is `(backend_ip, backend_port, proto)`, reusing the `BackendKey` newtype

The reverse key is the **existing `BackendKey`** triple
`(ip, port, proto)` — the same newtype the XDP REVERSE_NAT path already
keys on. `backend_ip` **alone was rejected**: it is ambiguous the
moment two same-host services share a backend IP on different ports
(e.g. two resolvers on `10.244.0.7:5300` and `10.244.0.7:5301`) — the
reply source would be rewritten to whichever VIP last won the slot.
`(backend_ip, backend_port, proto)` is the minimal unambiguous key, and
reusing `BackendKey` buys: (a) byte-parity with every other dataplane
reverse key; (b) the Sim reply mirror (D5d) reuses
`BTreeMap<BackendKey, Ipv4Addr>` verbatim — the exact shape
`SimDataplane.reverse_nat` already uses for the XDP path.

One degenerate case the single-VIP-per-backend-key value names
explicitly: if two distinct VIPs resolve to the identical
`(backend_ip, backend_port, proto)`, the reverse slot holds whichever
VIP registered last (last-writer-wins) — this is an **operator
misconfiguration / unsupported topology, not a supported shape and not
a silent assumption**. The key design is unchanged; the implication is
named so it is not mistaken for a guarantee.

#### D3 — Reverse-miss handling: rewrite-to-sentinel + counted miss (recvmsg4 CANNOT deny)

The crux finding from the research
(`recvmsg4-reply-source-rewrite-and-miss-semantics-research.md`, Q1,
[VERIFIED-PRIMARY], triangulated across the kernel selftest, the origin
commit `983695fa6765`, and Cilium's unconditional `SYS_PROCEED`):

> **`cgroup/recvmsg4` cannot deny the receive at any layer.** The
> kernel verifier hard-restricts `BPF_CGROUP_UDP4_RECVMSG` programs to a
> return-value range of **exactly `[1,1]`** — a program returning `0`
> is **rejected at load time** with `"At program exit the register R0
> has smin=0 smax=0 should have been in [1, 1]"`. "Drop on miss" is
> therefore impossible at ANY layer for the reply path.

So D3's option-space collapses from "drop vs pass vs sentinel" to
**"sentinel-rewrite vs pass-through"** (drop is off the table). Cilium
chooses pass-through and **leaks the backend source to the app's
`recvfrom`** on a miss (it has no other choice — it cannot drop). Our
design is **strictly stronger**: on a `REVERSE_LOCAL_MAP` miss,
recvmsg4 **rewrites the reply source to a non-backend/non-VIP sentinel
and increments an observable counted miss reason** (the `DropClass`-style
discipline, `crates/overdrive-core/src/dataplane/drop_class.rs`). This
is the *only* no-leak-to-app option recvmsg4 can physically execute.

A reverse miss is a **should-never-happen path** under the D1 ordered
(reverse-first) dual-write — the reverse entry is always installed
before the forward entry, so a backend with a visible forward entry
always has a visible reverse entry. The sentinel path is therefore
corruption/eviction handling
(map eviction under pressure, external `bpftool` tampering), not a
routine branch. The bar is "fail clean and diagnosable," which the
sentinel + counter meets and pass-through does not.

**Sentinel value (DESIGN pins it): `192.0.2.1`** (RFC 5737 TEST-NET-1
documentation range). Chosen over `0.0.0.0` because: (i) it is
guaranteed non-routable, reserved for documentation, so it can never
collide with a real backend IP or a VIP; (ii) `0.0.0.0`
(`Ipv4Addr::UNSPECIFIED`) is already the probe-sentinel VIP/backend
value (D5b / `EbpfDataplane::probe`), and some resolvers special-case
the wildcard — `192.0.2.1` has neither hazard; (iii) it is visibly a
"documentation" address in any `recvfrom`/`tcpdump` inspection, so an
operator immediately reads it as "the platform deliberately substituted
a non-real source," not as a corrupted real address.

**Open question surfaced to DELIVER / Tier-3 (NOT a tracking issue):**
the empirical check that the target resolvers (`dig`, glibc
`getaddrinfo`, musl) cleanly **reject** a `192.0.2.1`-sourced reply
(source-validation mismatch → clean failure) rather than surprisingly
accept it. The *mechanism* (sentinel rewrite + counter) is sound
regardless of which sentinel value is chosen; if the Tier-3 repro shows
`192.0.2.1` is not cleanly rejected by some resolver, the value is
swapped for another reserved-range address with no design change. Per
research Gap 2; per `feedback_no_unilateral_gh_issues` this is surfaced
as a DELIVER open question, not `gh issue create`d.

##### AC reframing — wire-layer → application-sockaddr-layer (back-propagation REQUIRED)

The DISCUSS US-01/US-03 ACs and KPIs K2/K5 were written in **wire-layer**
terms (`tcpdump`, "left the host", "on the wire"). The research (Q4,
[VERIFIED-PRIMARY]) establishes this is the **wrong layer** for
recvmsg4: the hook fires inside `udp_recvmsg()` **after** the kernel has
dequeued the skb and populated the `sockaddr_in` from the backend's
IP/UDP headers — so a `tcpdump -i lo` capture sees the backend-sourced
reply on **every** round-trip, hit or miss, strictly before recvmsg4
runs. Wire-level no-leak is an **XDP** concern (the connected/remote
REVERSE_NAT path, out of scope here), NOT recvmsg4's. recvmsg4's domain
is exactly the **application sockaddr** the app reads via
`recvfrom`/`msg_name`.

The reframing (full verbatim quotes + new wording in
`docs/feature/unconnected-udp-sendmsg4/design/upstream-changes.md`):

- **K2 / US-01** — "the source the client app reads via
  `recvfrom`/`msg_name` is the VIP, not the backend IP" (was: "`tcpdump`
  shows the reply sourced from the VIP / no reply leaves with the
  backend IP").
- **K5 / US-03** — "on a reverse miss the source the app reads is a
  non-backend sentinel (`192.0.2.1`, never the backend IP), and the
  miss is counted" (was: "no backend-IP-sourced reply leaves the
  host / reaches a client").

Intent preserved ("fail clean, never expose the backend IP to the
client app"); these are layer/wording corrections, not scope changes.

#### D4 — Option 3: ONE shared `#[inline(always)]` kernel helper across all three hooks (user override of Morgan's Option-2 recommendation)

Morgan recommended Option 2 (the two new programs duplicate connect4's
lookup body, leaving shipped connect4 untouched). **The user overrode to
Option 3 (shared helper).** The genuinely-shared primitive is **service-key
construction + `user_port` low-16-NBO handling** — factor THAT, and only
that, into ONE `#[inline(always)]` kernel helper — `build_local_service_key`
at `crates/overdrive-bpf/src/shared/build_local_service_key.rs` (the
`shared::sanity` / `shared::access` precedent for cross-program
`#[inline(always)]` helpers) — consumed by **all three** hooks:
`cgroup_connect4_service`, the new `cgroup_sendmsg4_service`, and
`cgroup_recvmsg4_service`. One unified attach orchestration and ONE
Earned-Trust probe set cover all three.

**The map lookup is NOT shared — it differs per hook.** The helper builds
the lookup key and handles the NBO conversion; it does **not** perform
any map lookup. Each hook calls the key/NBO primitive and then does its
own lookup against its own map:

- `cgroup_connect4_service` and `cgroup_sendmsg4_service` look up
  **`LOCAL_BACKEND_MAP`** (forward `(vip, vip_port, proto) → backend`).
- `cgroup_recvmsg4_service` looks up **`REVERSE_LOCAL_MAP`** (reverse
  `BackendKey → vip`).

**The rewrite direction is NOT shared either — it stays in the per-hook
program body.** One helper MUST NOT serve both rewrite directions:

- connect4 / sendmsg4 do a **forward DEST rewrite** (write the backend
  into `user_ip4` / `user_port`).
- recvmsg4 does a **reverse SOURCE rewrite** (write the VIP into
  `user_ip4` / `user_port`).

**This REFACTORS shipped connect4.** `cgroup_connect4_service`'s inline
key construction + NBO handling (Decision 1 § 1 pipeline) is **replaced
by a call to the shared `build_local_service_key` helper**; its
`LOCAL_BACKEND_MAP` lookup and its forward dest-rewrite stay in the
connect4 program body. The refactor is **behavior-preserving** — the
helper does exactly what connect4's key/NBO code does today,
byte-for-byte on the key construction and the NBO handling.

**Honest risk statement (Tier-3-reverified, no Tier-2 backstop).**
`BPF_PROG_TEST_RUN` returns ENOTSUPP for `cgroup_sock_addr` on kernel
≤ 6.8 (`.claude/rules/development.md` § "`bpf_sock_addr.user_port`"),
so there is **no Tier-2 unit backstop** for the connect4 refactor — a
regression in the shared helper would surface only at **Tier 3** (a
real `connect()` through the cgroup). The connect4 refactor MUST be
Tier-3-reverified in the same PR: the shipped walking-skeleton TCP/
connected-UDP round-trip acceptance re-runs green against the
helper-backed connect4. This is the cost the user accepted for the
single-lookup-site win.

This **changes the DISCUSS K4 / DD6 "pure addition / 0 connect4 changes"
claim** — connect4 is now EXTEND (refactored to call the shared helper),
not UNCHANGED. Back-propagation REQUIRED (see
`design/upstream-changes.md` § D4 K4 restatement). Net-new connect4
*behavior* is still 0; the *diff* is non-zero.

#### D5 — bundle (5a–5e), accepted

**D5a — `register_local_backend` writes BOTH maps, reverse-first; no new
trait method.** The SAME `register_local_backend` call writes
`REVERSE_LOCAL_MAP` (reverse, first) AND `LOCAL_BACKEND_MAP` (forward,
second). `deregister_local_backend` symmetrically removes both. **No new
trait method** — the existing two methods (Decision 2, as amended
2026-06-03 to carry `proto`) gain the second map write inside their
bodies. The trait-method rustdoc's 4-property contract is amended:

> **Postconditions on `Ok(())` (amended).** In addition to the forward
> postconditions, after `register_local_backend(vip, vip_port, proto,
> backend)` returns `Ok(())`, the reverse entry
> `BackendKey(backend.ip(), backend.port(), proto) → vip` is installed
> in `REVERSE_LOCAL_MAP`. **Observable invariant (amended):** observers
> never see a forward `LOCAL_BACKEND_MAP` entry without its
> corresponding reverse `REVERSE_LOCAL_MAP` entry — the reverse write
> commits first (reverse-write-first ordering), so any visible forward
> entry implies a visible reverse entry.
> **Edge case (amended):** `deregister_local_backend(vip, vip_port,
> proto)` removes both the forward `(vip, vip_port, proto)` entry and
> the reverse `BackendKey(backend, proto)` entry; a co-located
> other-proto entry on the same `(vip, vip_port)` and the same backend
> is left intact (per-proto granularity, parity with the 2026-06-03
> per-proto purge).

**D5b / D5c — probe extension: attach + sentinel-round-trip BOTH new
hooks; the `attach()` IS the below-floor preflight.** The Earned-Trust
probe (Decision 6, as extended for the cgroup path) gains the two new
hooks on the same `cgroup_attach_path`:

1. `cgroup_sendmsg4_service` and `cgroup_recvmsg4_service` attach to the
   configured `cgroup_attach_path` alongside `cgroup_connect4_service`.
2. `REVERSE_LOCAL_MAP` accepts a sentinel upsert round-trip (write
   `BackendKey(0.0.0.0:0, tcp) → 0.0.0.0`, read back, assert presence,
   delete) — the symmetric counterpart to the existing
   `LOCAL_BACKEND_MAP` probe.

**The `recvmsg4` / `sendmsg4` `attach()` call IS the below-floor
preflight.** `cgroup/sendmsg4` is stable since kernel 4.18; `cgroup/recvmsg4`
since 4.20 — both below the 5.10 LTS floor, so on every supported kernel
the attach succeeds. A host below those floors fails the `attach()`
syscall, which surfaces as the structured `health.startup.refused`
event (composition root "wire then probe then use" refuses to start).
**No `/proc` / `uname` parsing** — that would re-introduce the
`unwrap_or_default` boundary-read footgun (`.claude/rules/development.md`
§ "Distinct failure modes get distinct error variants"); the attach
syscall is the honest, kernel-authoritative floor check.

New `DataplaneError` / `DataplaneBootError` variant(s) cover the new
attach failures, **`#[from]`-routed, never flattened to
`Internal(String)`** (`.claude/rules/development.md` § "Never flatten a
typed error to `Internal(String)`"):

```rust
// DataplaneError (or DataplaneBootError, matching the existing
// CgroupSockAddrAttach variant's home) — one per new hook, OR one
// shared variant carrying the attach-type discriminator. Mirrors the
// shipped CgroupSockAddrAttach shape (ADR-0053 Decision 6).
#[error("cgroup_sock_addr attach failed (attach_type={attach_type}, \
         cgroup_path={cgroup_path}): {source}\n\n\
         {attach_type} requires kernel >= {min_kernel}; verify CONFIG_CGROUP_BPF \
         and the kernel floor. `bpftool cgroup show {cgroup_path}` lists \
         pre-existing attachments.")]
CgroupSendRecvAttach {
    attach_type: &'static str,   // "sendmsg4" | "recvmsg4"
    min_kernel:  &'static str,   // "4.18" | "4.20"
    cgroup_path: String,
    #[source]
    source: aya::programs::ProgramError,
},
```

Plus a probe variant for the `REVERSE_LOCAL_MAP` sentinel round-trip,
symmetric with the shipped `DataplaneError::LocalBackendProbe`:

```rust
#[error("REVERSE_LOCAL_MAP probe round-trip failed: {message}")]
ReverseLocalProbe { message: String },
```

And the miss counter is a counted reason (D3), surfaced via a kernel-side
`REVERSE_LOCAL_MISS_COUNTER` (a `PERCPU_ARRAY`, the `DropClass`/`DROP_COUNTER`
precedent) and a userspace accessor. It is NOT a `DropClass` variant —
recvmsg4 does not drop; the counter records "reverse-miss → sentinel
substituted," a distinct reply-path reason. A single-slot counter
suffices for Phase 1 (one miss reason: reverse-map miss).

**D5d — `SimDataplane` reply mirror, under the SAME mutex acquisition;
test-only accessor; MUST NOT shape production.** `SimDataplane` gains a
reply mirror `BTreeMap<BackendKey, Ipv4Addr>` written under the **SAME
mutex acquisition** as `local_backends` inside `register_local_backend`
(the existing `update_service` `services` + `reverse_nat` lockstep idiom
at `overdrive-sim/src/adapters/dataplane.rs:380` is the template — both
maps mutated under one lock so the dual-write is observably atomic in
the Sim, which models the same observable invariant production's
ordered reverse-first dual-write provides: no observer ever sees a
forward entry without its reverse). The Tier-1 J-PLAT-004 equivalence
invariant (`reply-source-rewrite-lockstep`) asserts against the mirror's
post-state via **two test-only accessors** — the sanctioned Tier-1
equivalence surface for the reply path, both observing the Sim's
post-state, neither part of the production `Dataplane` trait:

- **`reply_source_for(key: BackendKey) -> Option<Ipv4Addr>`** — the
  **forward direction**. Returns the reply source the recvmsg4 path would
  present for a given backend identity; the invariant asserts it equals
  the **VIP**, never the backend (the "reply source = VIP" assertion,
  US-02 AC). `Some(vip)` = reverse hit; `None` = a forward `local_backend`
  entry with no matching reply-mirror entry — the forward-only asymmetry
  the invariant exists to catch. Parity with `reverse_nat_lookup` on the
  XDP path.
- **`reply_mirror_entries() -> Vec<(BackendKey, Ipv4Addr)>`** — the
  **reverse / enumeration direction**. Snapshots every reply-mirror entry
  in `Ord` order on `BackendKey` (the `bpftool map dump REVERSE_LOCAL_MAP`
  equivalent), so the invariant can assert NO **stale reverse entry**
  exists — every reply-mirror entry must map back to a live forward
  `local_backends` entry. This catches the **deregister-leaves-a-reverse**
  asymmetry (a reply-mirror entry orphaned after its forward entry was
  purged) — the mirror image of the forward-only asymmetry
  `reply_source_for` catches. Parity with `reverse_nat_entries` on the XDP
  path.

The mirror models the **observable contract only** (the reply source the
app would read; the set of reverse entries that exist) — it adds NO arm,
NO yield, NO structural concession to production code
(`.claude/rules/development.md` § "Production code is not shaped by
simulation"); production's reverse-first dual-write is written to the
contract, and the Sim mirrors the *post-state*, not production's
mechanics. Both accessors observe the Sim's post-state for the equivalence
invariant; neither belongs on the production `Dataplane` trait.

**D5e — copy the connect4 `user_port` low-16-NBO idiom verbatim into the
shared `build_local_service_key` helper.** The `user_port` field is
low-16-NBO in a `u32`: read via `u16::from_be(ctx.user_port as u16)`,
write via `ctx.user_port = u32::from(host_port.to_be())`
(`.claude/rules/development.md` § "`bpf_sock_addr.user_port` —
low-16-NBO in a u32"). The shipped connect4 read-side idiom is copied
**verbatim** into the shared helper's key-build path (D4), so all three
hooks share one correct read-side NBO site. The write-side NBO (the
rewrite) stays per-hook in each program body — forward dest-rewrite for
connect4/sendmsg4, reverse source-rewrite for recvmsg4.

**recvmsg4 writable source fields confirmed (research Q2,
[VERIFIED-PRIMARY]):** recvmsg4 rewrites the source the app reads via
**`user_ip4` / `user_port`** (4-byte NBO, the same fields connect4/
sendmsg4 use). `msg_src_ip4` is **sendmsg4-only** and is NOT the
recvmsg4 handle. So: sendmsg4 writes the *destination* via `user_ip4`/
`user_port` (forward rewrite, like connect4); recvmsg4 writes the
*source* via `user_ip4`/`user_port` (reply rewrite). The NBO idiom is
identical on all three.

### Kernel-side programs (two new)

```rust
// crates/overdrive-bpf/src/programs/cgroup_sendmsg4_service.rs
#[cgroup_sock_addr(sendmsg4)]
pub fn cgroup_sendmsg4_service(ctx: SockAddrContext) -> i32 {
    match try_sendmsg4(&ctx) {
        Ok(verdict) => verdict,   // always 1 (proceed); deny is available
        Err(_) => 1,              // (sendmsg4 is in the [0,1] range) but
    }                             // this path never denies — forward rewrite
}                                 // or pass-through unchanged on a miss.

// crates/overdrive-bpf/src/programs/cgroup_recvmsg4_service.rs
#[cgroup_sock_addr(recvmsg4)]
pub fn cgroup_recvmsg4_service(ctx: SockAddrContext) -> i32 {
    // MUST return 1 unconditionally — the verifier restricts recvmsg4
    // to the range [1,1] (research Q1). A returned 0 is rejected at
    // load time. Reply-source rewrite happens before the return; a
    // reverse miss substitutes the sentinel + bumps the miss counter.
    let _ = try_recvmsg4_reply_rewrite(&ctx);
    1
}
```

- **sendmsg4 pipeline (forward):** read `(user_ip4, user_port, protocol)`
  and build the lookup key via the shared `build_local_service_key`
  helper → **sendmsg4's own lookup** against
  `LOCAL_BACKEND_MAP[(vip, vip_port, proto)]`. Miss → proceed unchanged
  (non-service `sendto`). Hit → forward dest-rewrite: overwrite
  `user_ip4` / `user_port` with the backend (the same rewrite connect4
  does, on the per-datagram unconnected path). Returns 1.
- **recvmsg4 pipeline (reverse):** the kernel has already populated the
  source sockaddr from the backend's skb. Read `(user_ip4, user_port,
  protocol)` (= the backend source) → build `BackendKey` via the shared
  `build_local_service_key` helper → **recvmsg4's own lookup** against
  `REVERSE_LOCAL_MAP` (a different map from the forward path). Hit →
  reverse source-rewrite: overwrite `user_ip4`/`user_port` with the VIP
  (and the original `vip_port`). Miss → overwrite `user_ip4` with the
  sentinel `192.0.2.1` + bump `REVERSE_LOCAL_MISS_COUNTER`. Returns 1
  **unconditionally**.

### Operator config

No new field. The `cgroup_attach_path` (Decision 7) is the attach point
for all three hooks — sendmsg4 and recvmsg4 attach to the same slice
the operator already configures (default
`/sys/fs/cgroup/overdrive.slice`). One config field, three hooks, one
attach orchestration (D4).

### Migration — single-cut, hydrator-repopulated; no shim

`REVERSE_LOCAL_MAP` is repopulated from intent on boot: the same
`ServiceMapHydrator` Local-vs-Remote classifier (Decision 4) that emits
`RegisterLocalBackend` now causes the dual-write, so the reverse map is
recreated from intent on next boot. **The `REVERSE_LOCAL_MAP` key +
the reverse-write IS the migration** — NO live in-place migration, NO
dual-key shim, NO deprecation path (`feedback_single_cut_greenfield_migrations.md`).
No persisted rkyv type is added (the map is kernel state, repopulated
from intent); no schema-evolution envelope bump.

### What this revision supersedes vs preserves

| Element | Status |
|---|---|
| Amendment 4 (2026-06-03) "sendmsg4 / unconnected-UDP out of scope → tracked #200" | **DELIVERED by this revision.** The hook lands; #200 closes. |
| Decision 1 § 1 — `LOCAL_BACKEND_MAP` forward shape `(vip, vip_port, proto)` | **Preserved.** sendmsg4 reuses it verbatim via the shared helper. |
| Decision 2 — `register_local_backend` / `deregister_local_backend` | **Extended (D5a).** Bodies gain the second-map reverse-first write; NO new method; contract amended for the reverse entry + observable invariant. |
| Decision 3 — `Action::RegisterLocalBackend` / `DeregisterLocalBackend` (+ proto, Amd 3) | **Preserved.** The reverse write is derived inside the adapter from the same action fields (`vip`, `backend`, `proto`); no new action field. |
| Decision 4 — hydrator Local-vs-Remote classifier | **Preserved.** Same emission; the dual-write is an adapter-internal consequence of `register_local_backend`. |
| Decision 5 — XDP programs unchanged | **Preserved.** sendmsg4/recvmsg4 are cgroup-path; the XDP wire-boundary REVERSE_NAT path is untouched and remains the (out-of-scope-here) remote/connected wire no-leak surface. |
| Decision 6 — Earned-Trust probe | **Extended (D5b/c).** Two attach targets + one `REVERSE_LOCAL_MAP` sentinel round-trip; attach IS the below-floor preflight. |
| Decision 1 § 1 — `cgroup_connect4_service` key-build / NBO (inline) | **Refactored (D4).** The inline key construction + NBO handling is replaced by a call to the shared `build_local_service_key` helper; connect4's own `LOCAL_BACKEND_MAP` lookup and forward dest-rewrite stay in its program body. Behavior-preserving; Tier-3-reverified. |
| Out of scope § "IPv6 service VIPs" | **Preserved.** `BPF_CGROUP_UDP6_SENDMSG`/`RECVMSG`, IPv6 `REVERSE_LOCAL_MAP` still out (GH #155 territory). |

### Consequences

**Positive.**

- A same-host UDP service is reachable from the canonical **unconnected**
  resolver (`dig`/`getaddrinfo`/musl `sendto`) — the dominant DNS idiom.
  The half-working-service trap (healthy upstream, unreachable client)
  closes for Phase 1.
- The reply path is **strictly stronger than Cilium** on the
  no-backend-IP-leak axis: Cilium pass-through leaks the backend source
  to the app's `recvfrom` on a reverse miss; our sentinel rewrite never
  exposes it (research Q5).
- One shared `#[inline(always)]` lookup site (D4) means the
  `user_port` NBO idiom, the key construction, and the forward lookup
  have **one** correct implementation across three hooks — the single
  source of truth the user chose over Option 2's duplication.
- `REVERSE_LOCAL_MAP` reuses `BackendKey` (D2) — byte-parity with the
  three existing reverse/forward keys, and the Sim mirror is free.
- Earned-Trust probe grows by two orthogonal attach targets + one map
  round-trip; the composition root refuses to boot on a below-floor
  kernel via the attach syscall itself (D5b), no `/proc` parsing.

**Negative / accepted.**

- **The connect4 refactor (D4) has no Tier-2 backstop.** A regression in
  the shared helper surfaces only at Tier 3. Mitigation: the shipped
  connected round-trip acceptance re-runs against the helper-backed
  connect4 in the same PR. Honest risk, user-accepted.
- **Surface grows by two programs, one map, one handle, one shared
  helper, one miss counter, one or two error variants.** Bounded;
  symmetric with the shipped connect4 / `LOCAL_BACKEND_MAP` patterns.
- **recvmsg4 cannot make a wire-level guarantee** (research Q4). A
  `tcpdump -i lo` shows the backend source on every round-trip, before
  the hook runs. The honest guarantee is application-sockaddr-layer only
  (D3 AC reframing). A wire-level no-leak property is XDP's domain (the
  out-of-scope connected/remote path), not recvmsg4's. Documented so a
  future reader does not re-import wire semantics onto the cgroup hook.
- **App `recvfrom` source on a hit is the VIP; on a miss it is a
  sentinel.** Intended (the whole point — the resolver source-validates
  against the VIP it queried). The sentinel-resolver-rejection empirical
  check is a Tier-3 DELIVER open question (D3).

### Quality-attribute impact

- **Correctness / functional suitability**: positive (large). The
  unconnected-UDP delivery gap closes; K1 (reachability) and K2
  (VIP-sourced reply at the app layer) reach 100% for Phase 1.
- **Maintainability — modifiability**: positive. The shared helper (D4)
  is the single forward-lookup decision site across three hooks.
- **Maintainability — testability**: mixed. Positive: the Sim reply
  mirror (D5d) gives a Tier-1 equivalence pin on the per-PR critical
  path (no kernel needed). Negative: the connect4 refactor and the new
  hooks are Tier-3-only (no Tier-2 for `cgroup_sock_addr`).
- **Reliability — fault tolerance**: positive (small). The reverse-miss
  sentinel + counter fails clean and observably rather than leaking or
  hanging silently.
- **Security**: neutral-positive. No backend IP reaches the client app
  on any path (hit → VIP, miss → sentinel); no new capability beyond the
  `CAP_BPF` + `CAP_NET_ADMIN` the control plane already holds.
- **Performance — time behaviour**: neutral. sendmsg4/recvmsg4 fire
  per-datagram (unlike connect4's per-connect), but each is a single
  map lookup + two `u32` writes; the verifier budget is trivial
  (≪ ceiling), same envelope as connect4.
- **Portability**: neutral. Linux-only via existing gates.

### Out of scope (explicit, additive)

- **IPv6 unconnected-UDP.** `BPF_CGROUP_UDP6_SENDMSG`/`RECVMSG`,
  `SocketAddrV6` reverse keys. Lands with IPv6 VIP support (GH #155).
- **Wire-layer no-leak for the same-host reply.** Physically not
  recvmsg4's domain (research Q4); it is XDP's, on the connected/remote
  path which is out of scope for this feature.
- **A wire-level `tcpdump` no-leak AC.** Removed by the D3 reframing —
  it asserts a property recvmsg4 structurally cannot deliver.

### References (additive)

- `docs/research/dataplane/recvmsg4-reply-source-rewrite-and-miss-semantics-research.md`
  — Nova, 2026-06-05, High confidence. The verifier `[1,1]` cannot-deny
  finding (Q1), the `user_ip4`/`user_port` writable-fields confirmation
  (Q2), the Cilium hit/miss shape (Q3), the wire-before-hook ordering
  (Q4), the sentinel-vs-pass-through recommendation (Q5). Two flagged
  gaps: exact verifier file:line (Gap 1, non-blocking); resolver
  behaviour on the sentinel (Gap 2 → the DELIVER Tier-3 open question).
- Kernel commit `983695fa6765` "bpf: fix unconnected udp hooks" — the
  recvmsg4 hook placement inside `udp_recvmsg()`.
- kselftest "Migrate recvmsg* return code tests" — the `[1,1]`
  return-range conformance spec.
- Cilium `bpf/bpf_sock.c` (`cil_sock4_recvmsg`, `__sock4_xlate_rev`,
  `SYS_PROCEED`/`SYS_REJECT`) — production reference; pass-through-leaks
  on miss, which our sentinel design improves on.
- `docs/feature/unconnected-udp-sendmsg4/feature-delta.md`,
  `.../design/wave-decisions.md`, `.../design/upstream-changes.md` —
  feature SSOT, decision record, and back-propagation.
- `crates/overdrive-core/src/dataplane/backend_key.rs` — `BackendKey`
  reused as the `REVERSE_LOCAL_MAP` key (D2).
- `crates/overdrive-core/src/dataplane/drop_class.rs` — the counted-reason
  discipline the reverse-miss counter follows.
- `crates/overdrive-sim/src/adapters/dataplane.rs:380` — the
  `services`+`reverse_nat` single-lock lockstep idiom the Sim reply
  mirror (D5d) mirrors.
- `.claude/rules/development.md` § "`bpf_sock_addr.user_port` —
  low-16-NBO in a u32" (D5e), § "Distinct failure modes get distinct
  error variants" (D5b), § "Never flatten a typed error to
  `Internal(String)`" (D5b), § "Production code is not shaped by
  simulation" (D5d), § aya-rs kernel-side patterns (the shared helper).

### Changelog (Revision 2026-06-05)

| Date | Change |
|---|---|
| 2026-06-05 | D5d clarification (final-gate review): the Sim reply-mirror test contract documents BOTH sanctioned Tier-1 accessors — `reply_source_for(key: BackendKey) -> Option<Ipv4Addr>` (forward direction; reply source = VIP) AND `reply_mirror_entries() -> Vec<(BackendKey, Ipv4Addr)>` (reverse/enumeration direction; no stale reverse entry — the deregister-leaves-a-reverse asymmetry, mirror of the forward-only asymmetry). Both are test-only (not on the production `Dataplane` trait), parity with `reverse_nat_lookup`/`reverse_nat_entries`. The `reply-source-rewrite-lockstep` invariant's reverse-direction orphan-detection loop calls `reply_mirror_entries()`; this clarification completes the test-contract surface it relies on. No locked decision changed; no production-trait/map/kernel change. — Morgan. |
| 2026-06-05 | Unconnected-UDP delivery DELIVERED (closes #200; supersedes Amendment 4's out-of-scope note). Two new cgroup hooks: `cgroup_sendmsg4_service` (forward request rewrite over `LOCAL_BACKEND_MAP`) + `cgroup_recvmsg4_service` (reply source rewrite over the NEW `REVERSE_LOCAL_MAP`). `REVERSE_LOCAL_MAP` = `BPF_MAP_TYPE_HASH`, key = existing `BackendKey {ip,port,proto}` (D2), value = VIP `u32`; written in ordered (reverse-first) sequence by `register_local_backend` (D1/D5a; two map syscalls, not one transaction — an ordering guarantee, not atomicity; NO new trait method; contract amended for the reverse entry + observable invariant). recvmsg4 CANNOT deny (verifier `[1,1]`, research Q1) → reverse-miss handling = rewrite-to-sentinel `192.0.2.1` (RFC 5737) + counted miss reason, strictly stronger than Cilium's pass-through-leak (D3). AC reframed wire→application-sockaddr layer for US-01/US-03/K2/K5 (D3, back-prop). Option 3 shared `#[inline(always)]` `build_local_service_key` helper (key-build + NBO only; per-hook map lookup + per-hook rewrite direction stay in each program body) across all three hooks; REFACTORS shipped connect4 (behavior-preserving, Tier-3-reverified, no Tier-2 backstop) — changes DISCUSS K4/DD6 "0 connect4 changes" (D4, back-prop). Probe extension: attach both new hooks + `REVERSE_LOCAL_MAP` sentinel round-trip; the attach() IS the below-floor preflight (4.18/4.20 floors, both <5.10), no `/proc`/`uname` parse; new `#[from]`-routed error variant(s) (D5b/c). SimDataplane reply mirror `BTreeMap<BackendKey, Ipv4Addr>` under the same mutex acquisition + `reply_source_for` test accessor; models the observable contract only, does not shape production (D5d). `user_port` low-16-NBO idiom copied verbatim into the shared helper; recvmsg4 writable fields = `user_ip4`/`user_port` (msg_src_ip4 is sendmsg-only), research Q2 (D5e). Single-cut hydrator-repopulated migration; no shim. Sentinel-resolver-rejection empirical check surfaced as a Tier-3 DELIVER open question (not a tracking issue). — Morgan (all decisions user-locked). |

## Changelog

- 2026-05-22 — Initial proposed version. Same-host backend delivery via `cgroup_sock_addr` connect-time destination rewrite. Resolves the walking-skeleton TCP round-trip data-path gap.
