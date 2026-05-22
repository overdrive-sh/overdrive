# ADR-0053 — Same-host backend delivery via `cgroup_sock_addr` connect-time destination rewrite

## Status

Proposed. 2026-05-22. Decision-makers: Morgan (drafting); pending
user ratification. Tags: phase-1, dataplane, lb, cgroup-bpf,
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

## Changelog

- 2026-05-22 — Initial proposed version. Same-host backend delivery via `cgroup_sock_addr` connect-time destination rewrite. Resolves the walking-skeleton TCP round-trip data-path gap.
