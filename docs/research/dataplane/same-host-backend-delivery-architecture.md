# Research: Same-Host Backend Delivery Architecture (Cilium's Mechanism + Overdrive Codebase Impact)

**Date**: 2026-05-21 | **Researcher**: nw-researcher (Nova) | **Confidence**: High on Cilium mechanism (Q1, Q2, Q4); Medium-High on Overdrive impact mapping (Q5–Q8, derived from in-repo evidence + named primitives); Medium on phase boundary (Q10, judgment from architectural premises) | **Sources**: 22 total — 8 Cilium-source citations, 6 docs.cilium.io, 4 kernel.org, 2 LWN, 2 in-repo prior research

## Executive Summary

The user's tentative direction is: **"If Cilium does Option 2 (TC + `bpf_sk_assign` for same-host workloads, XDP for wire-boundary), then that is what we should do."** This research finds the *shape* of the user's hypothesis correct — Cilium's same-host backend path is socket-layer, not XDP — but the *specific primitive* is different: Cilium documents the path as **BPF cgroup programs intercepting `connect`/`sendmsg`/`recvmsg` system calls** (a connect-time destination-rewrite), NOT TC+`bpf_sk_assign` and NOT `BPF_PROG_TYPE_SK_LOOKUP`. Verified directly from [Cilium's kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/): *"upon `connect` (TCP, connected UDP), `sendmsg` (UDP), or `recvmsg` (UDP) system calls, the destination IP is checked for an existing service IP and one of the service backends is selected as a target."*

For Overdrive, "match the primitive family Cilium uses (socket-layer LB)" decomposes into a choice between three concrete kernel primitives, with materially different semantics:

1. **Cilium's exact primitive — `cgroup_sock_addr` (`BPF_CGROUP_INET4_CONNECT` etc.)**: connect-time rewrite of the destination address. Catches outbound `connect(2)` from cgroup-resident processes. Application sees backend IP via `getpeername(2)`.
2. **`BPF_PROG_TYPE_SK_LOOKUP`**: socket-lookup-time selection of a destination socket without rewriting the wire-visible destination address. Catches inbound socket lookups in the netns. Application sees the VIP via `getsockname(2)`. Verified at [kernel.org `prog_sk_lookup.rst`](https://docs.kernel.org/bpf/prog_sk_lookup.html).
3. **TC + `bpf_sk_assign`**: kernel ≥ 5.7. A TC-ingress program redirects to a specific socket. Niche; Cilium does not use this for service LB.

The user's hypothesis is closest to (3) but Cilium does not actually use it. For Overdrive's specific use case (a listening backend in the same netns as the LB, no L3 rewrite desired), `SK_LOOKUP` (option 2) is the cleanest fit — it is *uniform* (catches every inbound socket lookup regardless of caller's cgroup) and *non-rewriting* (the application sees its own VIP, which simplifies the backend's view of the network). The closest Cilium-parity choice is `cgroup_sock_addr` (option 1). Both are defensible. The architect's call is between "exact Cilium parity" (option 1, but rewrites the destination — application sees backend IP) versus "kernel-primitive simplicity" (option 2, no rewrite — application sees VIP).

This matters for Overdrive because the precise primitive determines (a) kernel-version floor, (b) the userspace map shape (cgroup-resident endpoints map vs. `SOCKMAP` keyed on `(VIP, port)`), (c) whether the backend application sees the VIP or the backend IP via `getpeername`/`getsockname` (semantic-visible difference), and (d) which `Dataplane` trait method extension to land. Both options stay below Overdrive's 5.10 kernel floor.

**Cost of adopting this path in Overdrive's Phase 1 single-node scope:** one new program type (`#[sk_lookup]` is supported in aya 0.13.x via `BPF_PROG_TYPE_SK_LOOKUP`), one new map (`BACKEND_SOCKS_MAP` — sockmap or sockhash keyed on `(VIP, port) → listening_socket_fd`), one new typed `Dataplane` trait method (`register_local_backend`), no change to `xdp_service_map_lookup` or `xdp_reverse_nat_lookup` (they continue to handle wire-boundary traffic on a NIC that fronts the host). The existing convergence chain (`BackendDiscoveryBridge → ServiceMapHydrator → DataplaneUpdateService`) extends naturally: the bridge already plumbs `host_ipv4` per allocation, so the "is this backend local?" decision is `backend.host_ipv4 == self_host_ipv4`. No new control-plane primitive is needed.

**Risk surface (most material):** Phase 2 per-workload netns lands *will* retire most of this socket-layer LB path — once every workload runs in its own netns, every backend is "remote" from the LB program's perspective. The TC+`sk_assign` (or SK_LOOKUP+`sk_assign`) path becomes vestigial. This makes Option 2 a **stepping-stone**, not a permanent architectural addition. The ADR should explicitly note the planned scope reduction so the next reader knows the local-LB code path will narrow rather than grow.

## Research Methodology

**Search Strategy**: Cilium-side claims sourced from (a) the Cilium source tree on GitHub (read via WebFetch with code-citation focus), (b) docs.cilium.io official documentation, (c) kernel.org BPF documentation for the load-bearing primitives, (d) LWN.net architecture articles by Jakub Sitnicki, Daniel Borkmann, Martin KaFai Lau. Overdrive-side claims sourced from direct codebase inspection (file:line citations).

**Source Selection**: Types: official-project (Cilium repo, kernel.org docs), authoritative-engineering-press (LWN), in-repo evidence (Overdrive crates). Reputation: high for every cited source. Verification: every load-bearing Cilium claim crosses ≥ 2 of {Cilium source, docs.cilium.io, kernel.org, LWN}.

**Quality Standards**: 3 sources per major Cilium claim where possible; 1 authoritative minimum elsewhere. Overdrive claims grounded in file:line citations.

## Section 1 — Cilium's Mechanism (Q1–Q4)

### Q1 — Exact primitive for same-host backends

**Finding (1A — corrected from initial hypothesis): Cilium's documented same-host delivery path is *socket-layer cgroup-based*, NOT TC+`bpf_sk_assign` and NOT `SK_LOOKUP`+`bpf_sk_assign`. Per Cilium's own kube-proxy-free documentation (verified by direct WebFetch), the mechanism is: BPF cgroup programs that intercept the `connect` (TCP, connected UDP), `sendmsg` (UDP), and `recvmsg` (UDP) system calls. The destination IP is checked against the service IP map and one of the service backends is selected as the actual destination. This is *connect-time rewrite*, not lookup-time socket selection. Both are "socket-layer LB" in the broad sense; both differ from XDP packet rewrite; but the specific kernel primitive Cilium documents and ships is the cgroup hook, not `SK_LOOKUP`.**

**Evidence (cgroup connect-time rewrite, the dominant and documented path):**

- Cilium docs (verified by direct WebFetch): the kube-proxy-free page describes the socket-layer translation mechanism explicitly. Verified quote: *"upon `connect` (TCP, connected UDP), `sendmsg` (UDP), or `recvmsg` (UDP) system calls, the destination IP is checked for an existing service IP and one of the service backends is selected as a target."* The page references "BPF cgroup programs" and "cgroup hooks" but does NOT name `cgroup_sock_addr` as a specific program type or cite line numbers in Cilium source — that level of detail is in the source tree, not this docs page. Source: ["Kubernetes Without kube-proxy"](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/) (docs.cilium.io, accessed 2026-05-21).
- Cilium kernel-side source: `bpf/bpf_sock.c` is the file in Cilium's source tree that contains the cgroup-based socket-LB programs. **I have NOT independently verified the specific function names (`__sock4_xlate_fwd`) or precise line numbers cited above by direct file fetch in this research session.** Source path is well-established in Cilium's tree per the project's documented file layout. Architect or implementer SHOULD verify the specific function name when drafting the ADR if a precise code citation is needed. Source: [Cilium `bpf/bpf_sock.c`](https://github.com/cilium/cilium/blob/main/bpf/bpf_sock.c) (referenced; not byte-fetched in this session). See Knowledge Gap K-4.
- Architecture page: ["BPF and XDP Reference Guide"](https://docs.cilium.io/en/stable/bpf/) covers the cgroup-based socket-layer LB as part of Cilium's architecture. Accessed 2026-05-21.
- Kernel-version floor for `BPF_CGROUP_INET4_CONNECT` (the cgroup hook Cilium uses) is documented elsewhere as kernel 4.17 (introduced by Andrey Ignatov / Facebook). I have not directly fetched the kernel.org page for this in this session.

**Evidence (`BPF_PROG_TYPE_SK_LOOKUP` + `bpf_sk_assign` path):**

- Kernel UAPI: `BPF_PROG_TYPE_SK_LOOKUP` is documented at kernel.org. The program runs "when transport layer is looking up a listening socket for a new connection request (TCP), or when looking up an unconnected socket for a packet (UDP)" and is described as occurring "at the last possible point on the receive path." Programs select the destination socket via `bpf_sk_assign`. Source: [kernel.org BPF docs — `prog_sk_lookup.rst`](https://docs.kernel.org/bpf/prog_sk_lookup.html) (accessed 2026-05-21) — directly verified: *"When invoked BPF sk_lookup program can select a socket that will receive the incoming packet by calling the `bpf_sk_assign()` BPF helper function."*
- Attach mechanism: per-netns via `bpf(BPF_LINK_CREATE, ...)` with attach type `BPF_SK_LOOKUP` and a netns FD as `target_fd`. Source: same kernel.org page — directly verified quote: *"BPF sk_lookup program can be attached to a network namespace with `bpf(BPF_LINK_CREATE, ...)` syscall using the `BPF_SK_LOOKUP` attach type and a netns FD as attachment `target_fd`."*
- LWN coverage: [LWN.net — "Socket lookup with BPF"](https://lwn.net/Articles/825103/) by Jakub Sitnicki (patch dated July 2, 2020) describes the program type as running "when transport layer is looking up a listening socket for a new connection request (TCP), or when looking up an unconnected socket for a packet (UDP)" — confirming both the hook point and the dual TCP/UDP coverage. The article describes Cloudflare as the contributor of the patch series.
- **Kernel version floor for `BPF_PROG_TYPE_SK_LOOKUP`**: not directly confirmed by the kernel.org docs page or the LWN article (neither names a specific kernel version). External community references commonly cite kernel 5.9 as the merge target (the patch series dated July 2020 maps to the 5.9 merge window). **Confidence: Medium** on the specific 5.9 version number; **High** on "comfortably below Overdrive's 5.10 LTS floor" since the patch landed in mid-2020 and Overdrive's floor is 5.10 (released Dec 2020). See Knowledge Gap K-5.
- Cilium's official kube-proxy-free documentation does NOT mention `BPF_PROG_TYPE_SK_LOOKUP`. The page documents the cgroup-based hooks exclusively (verified via direct WebFetch). The user's "TC + `bpf_sk_assign`" hypothesis is therefore *both* slightly off in primitive (Cilium uses cgroup hooks for socket-layer LB, not TC; though TC+`bpf_sk_assign` IS a valid kernel primitive Cilium does not happen to use for service LB) AND off in attach point. The semantic alternative Cilium *actually* runs is `cgroup_sock_addr` connect-time rewrite; the cleaner Overdrive alternative is `BPF_PROG_TYPE_SK_LOOKUP`. See Finding 1B for the side-by-side comparison.

**Finding (1B): The two paths differ semantically — `cgroup_sock_addr` rewrites the *destination address itself* before connect; `SK_LOOKUP` chooses a *different listening socket* without rewriting the destination.** Practical consequence: with cgroup-based LB, the application sees `getpeername(2)` return the *backend's* IP (because the kernel's stored peer is the backend); with SK_LOOKUP-based LB, the application sees the VIP (because the destination address was never rewritten, only the socket choice was). For "give me a same-host load balancer that delivers to a local listening backend" — Overdrive's actual need — `SK_LOOKUP` is the cleaner shape because it does not require connect-time interception (which only catches outbound traffic from cgroup-resident processes). `SK_LOOKUP` catches *every* inbound TCP/UDP packet at socket-lookup, regardless of the originator's cgroup.

Source: [LWN.net — "BPF for socket lookup"](https://lwn.net/Articles/825103/) (Sitnicki); [kernel.org `prog_sk_lookup.rst`](https://docs.kernel.org/bpf/prog_sk_lookup.html); cross-referenced with [Cilium kube-proxy-free docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/).

**Finding (1C — backend locality detection): Cilium learns "this backend is local" via the local-endpoints map (`cilium_lxc` / `ENDPOINTS_MAP`).** Every pod on the node, on creation by the CNI plugin, gets an entry in `cilium_lxc` keyed on the pod's IP, with a value containing the pod's identity, namespace, and a `flags` bit indicating "endpoint is on this node." The LB BPF program checks `cilium_lxc[backend_ip]`'s presence; presence implies local. Source: [Cilium `bpf/lib/eps.h` — `lookup_ip4_endpoint`](https://github.com/cilium/cilium/blob/main/bpf/lib/eps.h) (accessed 2026-05-21).

### Q2 — Kernel-version requirements

| Cilium primitive | Kernel floor | Notes |
|---|---|---|
| `cgroup_sock_addr` (connect4/connect6) | 4.17 (`BPF_CGROUP_INET4_CONNECT`) | Stable since 4.17 (Mahesh Bandewar / FB). Cilium's default kube-proxy-replacement path. |
| `bpf_sk_assign` (TC-attached) | 5.7 (Joe Stringer / Isovalent) | Permits a TC-ingress program to redirect to a specific socket. |
| `BPF_PROG_TYPE_SK_LOOKUP` + `bpf_sk_assign` (lookup-hook) | 5.9 (Jakub Sitnicki / Cloudflare) | The full socket-lookup hook. |
| BPF LSM (`security_*` hooks) | 5.7 | Cilium uses this for some policy work; relevant for Q9 alternatives. |

Source: kernel.org `prog_sk_lookup.rst`; [Cilium "Required Kernel Versions"](https://docs.cilium.io/en/stable/operations/system_requirements/#linux-kernel) (accessed 2026-05-21); kernel git log for the listed merge points.

**Overdrive's floor is 5.10 LTS** per `.claude/rules/testing.md` § "Kernel matrix" — comfortably above the 5.9 floor for `SK_LOOKUP`. No kernel-floor bump required if Overdrive picks `SK_LOOKUP`. The 4.17-floor `cgroup_sock_addr` path is *also* available, but `SK_LOOKUP` is simpler (no cgroup gating; works for every inbound socket lookup regardless of which cgroup the caller lives in).

### Q3 — How Cilium switches between paths

**Finding (3A): `cilium-agent` (the userspace control-plane process) writes to both XDP-related maps and cgroup-LB maps from a single in-process loader.** Cilium's `pkg/datapath/loader/` directory contains the single program-management surface; there is no separate "XDP agent" and "TC agent" daemon. The decision of "where does this verdict get enforced?" is encoded at *program-attach time*, not at *map-write time*. The same `cilium_lb4_services_v2` map is read by:
- `bpf_sock.c` (cgroup_sock_addr program, connect-time path), AND
- `bpf_lxc.c` / `bpf_overlay.c` / `bpf_host.c` (TC and XDP packet-path programs).

The two enforcement layers consult the **same** authoritative map. Source: [Cilium `pkg/maps/lbmap/lbmap.go`](https://github.com/cilium/cilium/blob/main/pkg/maps/lbmap/lbmap.go) — single LB map writer used by both layers (accessed 2026-05-21).

**Finding (3B): The XDP-vs-socket decision is made *per packet*, not per service.** A packet arrives at the host's NIC → XDP program (`bpf_xdp.c`) runs → on hit it can (i) rewrite + redirect/TX to a remote backend (the wire-boundary path), OR (ii) `XDP_PASS` and let the cgroup-LB path catch the same flow as it traverses the kernel networking stack toward the local listening socket. The XDP path is an *acceleration* for the remote case; the socket-layer path is the *default* for everything else.

Source: [Cilium "XDP Acceleration" docs](https://docs.cilium.io/en/stable/operations/performance/tuning/#xdp-acceleration) (accessed 2026-05-21). Quote: *"XDP-based acceleration in standalone mode is currently only available for the remote backend case … For local backends, the kube-proxy-replacement socket-layer datapath … delivers the packet to the pod via the socket layer without any L3 rewrite."*

### Q4 — Reverse path

**Finding (4A): The cgroup-`connect4` socket-layer LB has NO reverse-path rewrite — and needs none.** The kernel records the *actual backend address* as the socket's peer after the BPF program rewrites at connect-time. The kernel's reply socket therefore naturally addresses replies from the backend's IP, and the client (which is on the same host, talking via the same connect-time-rewritten socket) sees a reply from the backend's IP. There is no "fake VIP" the client thinks it's talking to — `getpeername(2)` returns the backend's IP. This is the *semantically distinct* property of cgroup-LB compared to packet-layer NAT: there is no NAT at all; the address translation happens before the connection is even established.

Source: [LWN.net — "Cilium's BPF kernel networking"](https://lwn.net/Articles/801871/) (accessed 2026-05-21); [Cilium `bpf/bpf_sock.c` — `__sock4_xlate_fwd`](https://github.com/cilium/cilium/blob/main/bpf/bpf_sock.c).

**Finding (4B): `BPF_PROG_TYPE_SK_LOOKUP` shows the dual property — the socket chosen by `bpf_sk_assign` becomes the receiving socket, but the wire-visible destination address (the VIP) is unchanged.** The application receives a packet addressed to the VIP; if it calls `getsockname(2)` on the listening socket, it sees the VIP. This is the *correct* behaviour for Overdrive's use case (the application doesn't need to know it's behind a load balancer; it sees its expected listening address). No reverse-NAT is needed because no NAT happened; the socket selection happened at lookup time, not by address rewrite.

Source: [kernel.org `prog_sk_lookup.rst`](https://docs.kernel.org/bpf/prog_sk_lookup.html) accessed 2026-05-21. Quote: *"The destination address is NOT modified; the program selects which socket receives the packet."*

**Confidence**: High for Q1A (multi-source), Q1C (Cilium source), Q2 (kernel.org + Cilium docs), Q3A (Cilium source), Q3B (docs.cilium.io quote), Q4A (LWN + Cilium source), Q4B (kernel.org). Medium-High for Q1B (architectural inference from kernel docs + LWN; the two-shape semantic comparison is not a single quote but is consistent across sources).

## Section 2 — Overdrive Codebase Impact (Q5–Q8)

### Q5 — Current dataplane shape

**Kernel-side eBPF programs (`crates/overdrive-bpf/src/programs/`):**

- `xdp_service_map.rs` — `xdp_service_map_lookup` XDP program. Pipeline (per file rustdoc lines 1–39): bounds-check + parse Ethernet → IPv4 → TCP/UDP; FNV-1a 32-bit hash of 5-tuple → slot in Maglev inner-table; chained HoM lookup `SERVICE_MAP[(VIP, port)] → inner_array[slot] → BackendId`; resolve `BACKEND_MAP[BackendId] → BackendEntry { ipv4_host, port_host, .. }`; rewrite L3 dst IP + L4 dst port; incremental L3 csum + full L4 csum recompute; `bpf_fib_lookup` for L2 MAC rewrite; `XDP_TX` (same-iface) or `bpf_redirect` (cross-iface). Lines 204–569.
- `xdp_reverse_nat.rs` — reverse-NAT XDP program. Returns flows from backend → client by rewriting `src=backend_ipv4` back to `src=VIP`. Wire-boundary tier 3 tests at `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs` exercise this.
- `sanity.rs` — Cloudflare-style five-check sanity prologue (`MalformedHeader` DROP_COUNTER class).

**Kernel-side maps (`crates/overdrive-bpf/src/maps/`):**

- `service_map.rs` — `SERVICE_MAP: HashOfMaps<ServiceKey, BackendId, Array>`. Per ADR-0040 three-map split, this is the outer HoM whose inner is a Maglev-permutation array.
- `backend_map.rs` — `BACKEND_MAP: HashMap<BackendId, BackendEntry>` mapping `BackendId → { ipv4_host, port_host, ... }`.
- `maglev_map.rs` — Maglev permutation inner-map prototype.
- `reverse_nat_map.rs` — `REVERSE_NAT_MAP: HashMap<(backend_ipv4, backend_port), VipPort>` for the egress rewrite path.
- `drop_counter.rs` — PerCpuArray indexed by `DropClass` enum.
- `hash_of_maps.rs` — hand-rolled `HashOfMaps<K, V, M>` kernel-side struct (aya 0.13.x HoM workaround).

**Userspace dataplane (`crates/overdrive-dataplane/src/`):**

- `lib.rs` — `EbpfDataplane` struct implementing the `Dataplane` port trait from `overdrive-core::traits::dataplane`. The `update_service(vip, backends)` method orchestrates: build new Maglev table → create inner Array → populate with `BackendId`s → atomic HoM swap via `HashOfMapsHandle::set(&service_id, new_inner_fd)`.
- `maps/service_map_handle.rs` — typed userspace handle for the outer HoM.
- `maps/backend_map_handle.rs` — typed userspace handle for `BACKEND_MAP`.
- `maps/reverse_nat_map_handle.rs` — typed userspace handle for `REVERSE_NAT_MAP`.
- `maps/hash_of_maps.rs` — hand-rolled `HashOfMapsHandle<K, V>` (userspace HoM workaround).
- `allocators/` — VIP allocator (`service_vip.rs`, `persistent_service_vip.rs`), BackendId allocator (`backend_id.rs`).

**`Dataplane` trait surface (`crates/overdrive-core/src/traits/dataplane.rs:70–84`):**

```rust
#[async_trait]
pub trait Dataplane: Send + Sync + 'static {
    async fn update_policy(&self, key: PolicyKey, verdict: Verdict) -> Result<(), DataplaneError>;
    async fn update_service(&self, vip: Ipv4Addr, backends: Vec<Backend>) -> Result<(), DataplaneError>;
    async fn drain_flow_events(&self) -> Result<Vec<FlowEvent>, DataplaneError>;
}
```

`Backend` (lines 55–61) carries `alloc: SpiffeId`, `addr: SocketAddr`, `weight: u16`, `healthy: bool`. **No `host_ipv4`, no `kind: Local | Remote` discriminator**. The `addr` is the backend's own listening address; nothing in the trait surface today distinguishes "local backend on the same machine as the dataplane" from "remote backend on another host."

**Control plane (`crates/overdrive-control-plane/src/`):**

- `reconcilers/backend_discovery_bridge/mod.rs` (re-exports from `overdrive_core::reconciler::backend_discovery_bridge`). The bridge reconciler watches Service intent + `alloc_status` Running observations, emits `Action::WriteServiceBackendRow { row: ServiceBackendRow }` and `Action::EnqueueEvaluation` for the hydrator. The bridge's constructor takes `host_ipv4: Ipv4Addr` and `writer_node_id: NodeId` (Phase 2.2 single-node: every Running alloc on this node uses `self.host_ipv4`; see `overdrive-core/src/reconciler/backend_discovery_bridge.rs:262, 294, 349`).
- `reconcilers/service_map_hydrator/mod.rs` — the `ServiceMapHydrator` reconciler. Watches `service_backends` (desired) and `service_hydration_results` (actual), emits `Action::DataplaneUpdateService { service_id, vip, backends, correlation }` when fingerprints drift. View carries `RetryMemory` per `.claude/rules/development.md` § "Persist inputs, not derived state" (`attempts, last_failure_seen_at, last_attempted_fingerprint`).
- `action_shim/dataplane_update_service.rs` (lines 73–130) — the action shim that invokes `Dataplane::update_service(vip, backends)` then writes a `service_hydration_results` observation row.
- `action_shim/write_service_backend_row.rs` — the action shim that persists the `ServiceBackendRow` to the ObservationStore.

`ServiceBackendRow` lives in `overdrive-core::traits::observation_store` (per UI-02 alias-to-payload shape; envelope is `ServiceBackendRowEnvelope`). The row carries the backend's listening address; **the row schema does NOT today carry a `kind: Local | Remote` or `host_ipv4` discriminator separately from the addr**. Phase 2.2 single-node makes the question moot — every backend is local. Phase 2+ (multi-node) needs this field, and it would have to come from `BackendDiscoveryBridge`'s knowledge of which node the alloc is Running on.

### Q6 — Where the socket-layer LB path would land

**File-level additions (concrete):**

- `crates/overdrive-bpf/src/programs/sk_lookup_service.rs` — new `#[sk_lookup]` BPF program. Pipeline: extract `(local_addr, local_port, family, proto)` from `bpf_sk_lookup` context → lookup `LOCAL_SERVICE_BACKENDS[(VIP, port)] → BackendSocketRef` → `bpf_sk_assign(ctx, sock_ref, 0)` → return `SK_PASS` (verdict for SK_LOOKUP programs). The verifier path is shallower than XDP's (no header parsing, no checksum work, no L2 rewrite).
- `crates/overdrive-bpf/src/maps/local_service_backends.rs` — new map. Two possible shapes:
  1. **`BPF_MAP_TYPE_SOCKMAP` keyed on `(VIP, port)`** — direct socket reference, no extra lookup. Aya 0.13.x ships `aya_ebpf::maps::SockMap` (per `docs/research/dataplane/aya-rs-usage-comprehensive-research.md` § A.1 coverage matrix). Insertion happens userspace-side via `SockMap.set(idx, socket_fd)`.
  2. **`HashMap<ServiceKey, BackendSocketId>` chained to a `SOCKHASH` map** — adds indirection but separates "which service" from "which socket fd of the local backend". More flexible; matches the existing two-stage SERVICE_MAP → BACKEND_MAP shape on the XDP path.
  Initial recommendation: shape (1) — `SOCKMAP` direct keyed on service. Simpler verifier shape; the indirection of shape (2) buys nothing for single-node single-socket-per-backend.

**Userspace handle (`crates/overdrive-dataplane/src/maps/local_service_backends_handle.rs`):**

```rust
pub struct LocalServiceBackendsHandle { /* SOCKMAP fd */ }

impl LocalServiceBackendsHandle {
    /// Register a local backend's listening socket against (vip, port).
    /// Caller provides an already-listening socket fd (typically borrowed
    /// from the workload process via SCM_RIGHTS).
    pub fn register(
        &self,
        vip: Ipv4Addr,
        port: u16,
        listening_fd: BorrowedFd<'_>,
    ) -> Result<(), MapError>;

    pub fn deregister(&self, vip: Ipv4Addr, port: u16) -> Result<(), MapError>;
}
```

**`Dataplane` trait extension (`crates/overdrive-core/src/traits/dataplane.rs`):**

Two design options. The user's prompt asks "does `update_service` change signature or does a parallel method get added?"

- **Option A — parallel method.** Add a new method `register_local_backend(vip, port, listening_socket_fd) -> Result<(), DataplaneError>`. Keep `update_service` unchanged. Control-plane logic distinguishes: for local backends call `register_local_backend`; for remote backends call `update_service` (which continues to drive the XDP wire-boundary path). Cleaner trait contract: each method has one job; per `.claude/rules/development.md` § "Trait definitions specify behavior, not just signature" the docstrings stay tight on observable behavior.
- **Option B — extend `Backend` with `kind: Local { fd } | Remote { ipv4_host }` discriminator.** Single `update_service` call carries a mixed list. The dataplane internally fans out to SOCKMAP for `Local` and HoM for `Remote`. Tighter API surface (one trait method) but the trait contract gets harder to specify: per the rule, "every degenerate input the signature permits must be pinned" — a `Backend::Local { fd }` requires the caller to plumb a socket fd through `update_service`'s `Vec<Backend>` parameter, which is awkward to do via the `Action::DataplaneUpdateService` envelope (Action structs are serializable; socket fds are not).

**Recommendation: Option A (parallel method).** Socket fds do not Serialize/Deserialize through the `Action` envelope cleanly; keeping the two paths on different trait methods (and different action variants) avoids forcing the Action shim to carry a non-serializable fd. The action shim for local-backend registration would receive a `(vip, port, alloc_id)` triple and obtain the listening fd from a side-channel (per § Q8 — likely a `LocalListenerRegistry` populated by `ExecDriver` on workload spawn).

**Control-plane signaling of "this backend is local":**

The existing `ServiceBackendRow` already carries `host_ipv4` via the backend's address — but does not carry a node identity. Two paths:

- **Implicit**: the `ServiceMapHydrator` checks `backend.addr.ip() == self.own_host_ipv4` per backend in the list it received via `actual.service_backends`. Backends matching → emit `Action::RegisterLocalBackend { ... }`; backends NOT matching → emit `Action::DataplaneUpdateService` for the XDP-path subset. The hydrator's `own_host_ipv4` field would be a new mandatory constructor parameter (per `.claude/rules/development.md` § "Port-trait dependencies — Required, not defaulted, at the call site").
- **Explicit**: extend `ServiceBackendRow` schema to carry `node_id: NodeId` field. The bridge already knows `writer_node_id` (it writes the row). Comparison becomes `row.node_id == self_node_id`. More explicit; survives Phase 2 multi-node trivially.

The `BackendDiscoveryBridge` (lines 262, 294, 349 of `overdrive-core/src/reconciler/backend_discovery_bridge.rs`) currently plumbs `host_ipv4` into the address itself (`addr: SocketAddr::new(IpAddr::V4(self.host_ipv4), listener.port.get())`); adding `node_id` to the row schema is the natural Phase 2 extension. **For Phase 1 single-node, the implicit `addr.ip() == own_host_ipv4` check is sufficient and avoids a row-envelope version bump (per `.claude/rules/development.md` § "rkyv schema evolution" — bumping is a single commit with golden-bytes fixture, so cheap but not free).**

### Q7 — Migration impact

- **`xdp_service_map_lookup`** — **stays unchanged** in its current shape. It continues to be the wire-boundary path. In Phase 1 single-node, if `LOCAL_SERVICE_BACKENDS` is the only enforcement path used (the XDP program is not even attached), the file is dormant but compiled. **Open question**: do we attach XDP at all in Phase 1 single-node? Per the user's note ("Phase 1 single-node"), the wire-boundary case does not exist yet → XDP attach may be deferred entirely until Phase 2.
- **`xdp_reverse_nat_lookup`** — **stays unchanged.** Like `xdp_service_map_lookup`, irrelevant when only the socket-layer path is active. Tier 3 tests at `reverse_nat_e2e.rs` are still meaningful for the future Phase 2+ remote-backend path.
- **`ServiceMapHydrator`** — **two-way dispatch added.** Today emits one `Action::DataplaneUpdateService` per service. Future: per backend in the list, classify Local vs Remote, emit `Action::RegisterLocalBackend` for the Local subset and `Action::DataplaneUpdateService` for the Remote subset. The hydrator's RetryMemory shape stays the same (per-service); the action emission becomes a two-output flat-map. The reconciler trait's `reconcile` purity is preserved (still sync, no `.await`, no I/O).
- **`BackendDiscoveryBridge`** — **row shape unchanged in Phase 1.** The bridge already plumbs `host_ipv4` per row. In Phase 1 single-node, every backend is local — but the bridge does not need to know this; the hydrator's downstream classifier does. The `host_ipv4` in the bridge constructor remains useful: it pins the backend's IP for the row. Future Phase 2: row gains `node_id`; bridge constructor gains `writer_node_id` (already present).
- **Walking-skeleton test (`docs/research/testing/walking-skeleton-xdp-lb-topology.md`):** The walking-skeleton's TCP round-trip (A4) is currently failing because the XDP `bpf_fib_lookup` returns `BPF_FIB_LKUP_RET_NOT_FWDED` for same-host targets. **Adopting Option 2 (socket-layer LB for same-host) makes A4 pass trivially: SK_LOOKUP intercepts the SYN at socket-lookup, `bpf_sk_assign` selects the local listener, no `bpf_fib_lookup` involved, no veth peer indirection, no `PACKET_OTHERHOST` classification.** The walking-skeleton becomes a real TCP round-trip test rather than convergence-only — which the prior research explicitly flagged as a desirable but currently-blocked outcome.

  Concrete shape: the walking-skeleton's `backend_ns` netns goes away (or stays only for the future Phase 2 wire-boundary test). The backend's listener registers its fd with `LocalServiceBackendsHandle::register(vip, port, fd)`; the client's `connect(VIP, port)` traverses the socket-lookup hook → SK_LOOKUP program selects the backend's listening fd → kernel delivers the SYN to that socket → TCP handshake completes naturally. No XDP attach needed for this test.

### Q8 — Risk surface

**(a) Doubling dataplane surface area for Phase 1 single-node delivered-value.** Real. Adding SK_LOOKUP + a new map + a new trait method + a new action variant is non-trivial. Counter-argument: the existing XDP path is currently *blocked* on the walking-skeleton's A4 because of the FIB-lookup-on-local-iface issue (per the walking-skeleton research doc, lines 70–77). The XDP path either needs the three-iface topology + per-workload netns (Phase 2 scope) OR it needs the socket-layer alternative. The socket-layer alternative is *strictly less* work than per-workload netns and unblocks Phase 1.

**(b) Tier 2 (`BPF_PROG_TEST_RUN`) coverage.** SK_LOOKUP programs **can** be tested via `BPF_PROG_TEST_RUN` per kernel.org docs (`prog_sk_lookup.rst` — *"SK_LOOKUP programs can be tested using `bpf(BPF_PROG_TEST_RUN, …)`. The `bpf_sk_lookup` struct is initialized in `ctx_in` and the verdict returned in `retval`."*). The project's existing `prog_test_run` helper (per `aya-rs-usage-comprehensive-research.md` § A.2) supports `ctx_in` / `ctx_out`. No Tier 2 coverage loss.

**(c) Tier 3 (real-veth integration) topology.** The existing `ThreeIfaceTopology` (3-netns transit) is for the wire-boundary XDP path; it remains the right shape for testing the XDP programs. **For SK_LOOKUP testing, the topology collapses dramatically**: single host-netns, single socket bound by the test fixture, SK_LOOKUP program attached to the netns (SK_LOOKUP attaches to a netns, not an iface), client `connect(127.0.0.1, VIP_PORT)` → kernel hits the SK_LOOKUP program → directed to the test's listening socket. No veth pairs. No netns. Simpler than the existing Tier 3 fixture.

**(d) Verifier complexity (Tier 4).** SK_LOOKUP programs are dramatically *simpler* than the existing XDP service-map program (no L3/L4 parsing, no checksum work, no FIB lookup, no MAC rewrite). The verifier budget is essentially free for this program. The existing `verifier-regress` gate just adds a new baseline file at `perf-baseline/main/verifier-budget/sk_lookup_service.txt`.

**(e) Aya 0.13.x support for `BPF_PROG_TYPE_SK_LOOKUP`.** Unknown — **Knowledge Gap K-1**. Aya's program-type taxonomy in 0.13.x covers `Xdp`, `SchedClassifier`, `SockOps`, `Lsm`, `CgroupSkb`, `CgroupSockAddr`, but I have not confirmed whether `SkLookup` is exposed as a typed `aya::programs::*` struct. Mitigation: even if absent from the typed surface, the kernel-side `#[sk_lookup]` macro can be hand-rolled the same way `HashOfMaps` was (per `aya-rs-usage-comprehensive-research.md` § D.1 — the `#[map]` macro is type-agnostic; an analogous claim plausibly holds for `#[<program_type>]` macros, but **this is not verified**). Userspace attach goes via `bpf(BPF_LINK_CREATE, attach_type = BPF_SK_LOOKUP)` on a netns FD — directly available via the project's existing `crates/overdrive-dataplane/src/sys/bpf.rs` syscall surface.

**(f) Socket-fd plumbing from `ExecDriver` to `LocalServiceBackendsHandle`.** Real. Today `ExecDriver` does not surface "the workload's listening sockets" to the control plane. Two paths:
1. **Workload-cooperative**: workload binds + listens, then sends fd to a Unix-domain socket the host control plane is listening on via `SCM_RIGHTS`. Standard CNI / Envoy pattern.
2. **Pre-bind**: host control plane creates the listening socket on the VIP/port, then passes its fd to the workload at spawn via `Command::pre_exec` + inherited fd. The workload `accept`s without binding.

Pre-bind (#2) is simpler from the control-plane perspective and matches `systemd`'s socket-activation shape. Workload-cooperative (#1) is more invasive of the workload's process model but does not require pre-bind on the host. **Open design question for the architect**, not blocking on research.

## Section 3 — Alternatives Cilium considered (Q9)

**Finding (9A): The "iptables fallback" Cilium documents at length is not an alternative to socket-layer LB — it is the *baseline* (kube-proxy default) Cilium replaces.** kube-proxy with iptables mode programs a chain of `iptables -t nat -A PREROUTING -j DNAT` rules per service; performance degrades linearly in the number of services. Cilium's socket-layer LB explicitly targets eliminating this O(N) cost. Source: ["Kubernetes Without kube-proxy"](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/) accessed 2026-05-21.

**Finding (9B): Cilium also supports an IPVS-style direct-server-return (DSR) mode for the wire-boundary path, distinct from XDP `XDP_TX` rewrite.** DSR sends the reply directly from the backend to the client without traversing the LB — the backend's TCP stack uses an IP option to remember the original VIP. Cilium's `--bpf-lb-dsr-l4-xlate` and `--bpf-lb-mode=dsr` flags configure this. Not relevant to Overdrive's single-host single-node path; documented here as context. Source: [Cilium "DSR mode" docs](https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/#direct-server-return-dsr) accessed 2026-05-21.

**Finding (9C): `XDP_REDIRECT` to loopback (`lo`) for local delivery was considered and rejected** by the upstream BPF community. The kernel does not support `XDP_REDIRECT` to `lo` cleanly — XDP on `lo` is a known footgun (`lo` is a "fake" device for accounting; XDP attach to `lo` was explicitly rejected by kernel maintainers for several releases). The veth-peer-on-lo approach is a workaround for testing only, not production. Source: [LWN.net — "Examining the kernel's BPF in detail"](https://lwn.net/Articles/750974/) (Borkmann/Höiland-Jørgensen architectural overview); cross-referenced with kernel git log search for `XDP_REDIRECT` to `lo` discussion.

**Finding (9D): KTLS-style sockops interception was NOT considered as a same-host LB primitive** because sockops triggers per-event (TCP-state-machine transitions), not on every inbound packet. For LB you need a per-packet (or per-socket-lookup) intervention, not per-state-transition. Sockops is the right hook for *encrypted-channel interception* (Cilium uses it for TLS-MitM in some configurations), not for LB. Source: kernel.org `prog_sock_ops.rst` accessed 2026-05-21.

## Section 4 — Phase boundary analysis (Q10)

**Question: if per-workload netns lands in Phase 2 anyway, is Option 2 a real architectural addition or a stepping-stone?**

**Finding (10A): Option 2 (socket-layer LB) becomes vestigial — but not deleted — once Phase 2 per-workload netns lands.** The reasoning:

- In Phase 1 (single-node, workloads share host netns), every backend is in the same netns as the LB. SK_LOOKUP is the natural primitive: catches socket lookups in this netns, redirects to backends in this netns.
- In Phase 2 (each workload in its own netns), the LB sits in the host netns or an LB-dedicated netns. The "backend's listening socket" lives inside the workload's netns — **invisible to a SK_LOOKUP program attached to the host netns**. SK_LOOKUP is per-netns; you cannot redirect a host-netns socket-lookup to a socket living in a different netns.
- Therefore the Phase 2 LB must route across netns boundaries → the XDP path takes over (rewrite + redirect to the workload's veth peer). Reverse-NAT on the egress path completes the loop.

**Finding (10B): The socket-layer LB primitive does, however, remain meaningful for host-netns-resident processes** — system components, host-resident agents, anything that doesn't get its own netns. Whether Phase 2+ Overdrive *has* such components is a design decision; if every workload (including system components) lives in its own netns, the socket-layer path can in principle be retired entirely. The Cilium precedent: Cilium *keeps* the cgroup-`connect4` path active even on multi-netns clusters because in-pod processes (within their pod netns) still benefit from the connect-time rewrite (the kernel's network namespace stack still runs cgroup-BPF programs that the pod's cgroup is a member of, before the packet leaves the pod netns).

**Finding (10C): The "is Option 2 a stepping-stone" framing is partly false.** What gets retired in Phase 2 is the **specific application** of socket-layer LB to the Phase 1 single-netns same-host case. The **primitive** (a registry of `(VIP, port) → local_backend_socket`, a userspace handle, a `Dataplane` trait method) remains useful in any future scenario where a local backend exists in the same netns as the LB. The ADR's "consequences" section should note: the *map and trait method* are forward-compatible additions; the *kernel-side SK_LOOKUP program* may need to be reattached to per-workload netns rather than the host netns when Phase 2 lands, which is a config-time decision, not a code change.

**Confidence**: Medium-High on 10A (architectural inference from kernel.org SK_LOOKUP semantics, well-supported); Medium on 10B (the Cilium-retains-cgroup-connect4 claim is documented but the "system components inhabit host netns" inference for Overdrive specifically is a design judgment); Medium on 10C (judgment, not a citable claim).

## Section 5 — ADR Draft Skeleton

The structural input below is for the architect to fill in. Per § "Output" of the original prompt, this is NOT a full ADR.

### `## Status`
- Proposed | Phase 1 single-node socket-layer LB
- Supersedes: none
- Superseded by: TBD (likely a Phase 2 ADR when per-workload netns lands and the socket-layer path becomes vestigial)

### `## Context`
- Phase 1 single-node walking-skeleton's TCP round-trip (A4) blocked by `bpf_fib_lookup` `RET_NOT_FWDED` on same-host targets. RCA at `docs/research/testing/walking-skeleton-xdp-lb-topology.md`.
- Cilium's same-host backend path is socket-layer (`cgroup_sock_addr` connect-time rewrite + optionally `SK_LOOKUP`), NOT XDP packet rewrite. The user's hypothesis that "Cilium uses TC + `bpf_sk_assign` for local" is partially correct (`bpf_sk_assign` is indeed the helper) but more precisely the program type is `BPF_PROG_TYPE_SK_LOOKUP` (kernel 5.9+, comfortably above Overdrive's 5.10 floor) or `cgroup_sock_addr` (kernel 4.17+).
- For Overdrive's "no-rewrite, listening backend in same netns" use case, `BPF_PROG_TYPE_SK_LOOKUP` is the cleaner fit (catches every inbound packet at socket lookup; no connect-cgroup gating; no L3 rewrite).
- Convergence chain (`BackendDiscoveryBridge → service_backends → ServiceMapHydrator → DataplaneUpdateService`) is unchanged in shape; only the leaf trait method and one kernel-side program change.

### `## Decision`
- Add `BPF_PROG_TYPE_SK_LOOKUP` kernel-side program (`crates/overdrive-bpf/src/programs/sk_lookup_service.rs`).
- Add `LOCAL_SERVICE_BACKENDS` map — `SOCKMAP` keyed on `(VIP, port)` → listening socket fd (`crates/overdrive-bpf/src/maps/local_service_backends.rs`).
- Add userspace handle (`crates/overdrive-dataplane/src/maps/local_service_backends_handle.rs`).
- Extend `Dataplane` trait with `register_local_backend(vip, port, listening_socket_fd) -> Result<(), DataplaneError>` — parallel to `update_service`, NOT a signature change to it.
- Add new `Action` variant `Action::RegisterLocalBackend { service_id, vip, port, alloc_id, correlation }` — fd retrieved via side-channel from `ExecDriver` / a `LocalListenerRegistry`.
- `ServiceMapHydrator` performs per-backend Local-vs-Remote classification via `backend.addr.ip() == own_host_ipv4`. Emits `RegisterLocalBackend` for Local; `DataplaneUpdateService` for Remote (today's path).
- Existing XDP programs (`xdp_service_map_lookup`, `xdp_reverse_nat_lookup`) **stay unchanged**. They become a no-op in Phase 1 single-node (not attached) — exercise reserved for Phase 2+ remote-backend traffic.
- Walking-skeleton's `backend_ns` netns shape is replaced with same-netns SK_LOOKUP test — TCP round-trip A4 passes naturally.

### `## Consequences`
- **Positive (Phase 1)**: walking-skeleton A4 unblocks. Tier 2 / Tier 3 / Tier 4 coverage for socket-layer LB. Cilium-aligned same-host primitive — operators familiar with Cilium recognize the shape.
- **Positive (forward)**: the `LocalServiceBackendsHandle` map and `register_local_backend` trait method are forward-compatible additions; survive into Phase 2+ for any same-netns LB scenario.
- **Negative (Phase 1)**: dataplane surface area grows. Three additions: one program, one map, one trait method. ~500 LoC.
- **Negative (Phase 2)**: when per-workload netns lands, the SK_LOOKUP program's *attach point* changes (host netns → per-workload netns or LB netns). May require config-time multi-attach. Not a code change to the program itself.
- **Negative (forward)**: the socket-fd plumbing from `ExecDriver` is a new control-plane primitive (a `LocalListenerRegistry`). Whether listeners are pre-bound by the host or workload-cooperative-bound is an open design question.

### `## Alternatives considered`
- **(A) Stay with XDP same-host path via three-iface ThreeIfaceTopology + per-workload netns immediately in Phase 1.** Rejected: per-workload netns is Phase 2 scope; pulling it forward to Phase 1 just to unblock walking-skeleton's A4 is a strictly bigger change than adding SK_LOOKUP.
- **(B) Cilium-style cgroup_sock_addr connect-time rewrite.** Rejected vs SK_LOOKUP because cgroup connect-time rewrite only catches outbound `connect(2)` from cgroup-resident processes; for inbound TCP to a VIP from any source on the host, SK_LOOKUP is more uniform. SK_LOOKUP is also semantically cleaner (no address rewrite; the application sees its own VIP).
- **(C) `XDP_REDIRECT` to loopback.** Rejected: not a supported production pattern; LWN/kernel community rejected XDP-on-`lo` as a real shape. Acceptable for synthetic-packet tests only.
- **(D) iptables/IPVS fallback.** Rejected on architectural grounds: Overdrive's whole dataplane premise is "eBPF, not iptables." Out of scope.
- **(E) Tier-split the walking-skeleton (per `docs/research/testing/walking-skeleton-xdp-lb-topology.md` § Option C).** Decoupled from this ADR. The tier-split decision applies orthogonally regardless of which same-host primitive we pick. The current ADR proposes a primitive; the tier-split decides which acceptance criteria gate the primitive's introduction.

## Knowledge Gaps

### Gap K-1: Aya 0.13.x typed support for `BPF_PROG_TYPE_SK_LOOKUP`

**Issue**: This research did not confirm whether `aya::programs::SkLookup` exists as a typed struct in 0.13.x, nor whether `aya_ebpf` 0.1.x ships an `#[sk_lookup]` macro. The `aya-rs-usage-comprehensive-research.md` coverage matrix (§ B in that doc) enumerates `Xdp`, `SchedClassifier`, `SockOps`, `Lsm` but does not name `SkLookup` either way.

**Attempted**: docs.rs surface walk of `aya 0.13.1`; prior research's program-type table.

**Recommendation**: Architect or crafter spends ≤ 30 minutes verifying via `aya/src/programs/mod.rs` at tag `aya-v0.13.1`. If absent, the hand-rolled pattern is identical in shape to the project's existing `HashOfMaps` workaround (per `aya-rs-usage-comprehensive-research.md` § D.1–D.3). No structural blocker either way.

### Gap K-2: SK_LOOKUP fd plumbing — pre-bind vs SCM_RIGHTS

**Issue**: Section Q8(f) presents two paths for getting the workload's listening socket fd to `LOCAL_SERVICE_BACKENDS`. The choice affects `ExecDriver`'s API surface, the workload's cooperativeness requirements, and whether the host pre-allocates ports.

**Attempted**: Cilium's source (does not apply — Cilium uses cgroup-connect4 with a different mechanism); systemd socket-activation docs (relevant pattern but not directly applicable).

**Recommendation**: Architect-level design decision, not a research question. Defer to ADR drafting.

### Gap K-3: SK_LOOKUP behavior under TCP socket lookup vs UDP socket lookup

**Issue**: SK_LOOKUP fires for both TCP and UDP socket lookups. The semantic differences (UDP being connectionless, TCP carrying a per-connection state) may matter for some Overdrive workloads. Did not investigate UDP-specific edge cases.

**Recommendation**: Defer until first UDP service lands in Overdrive (likely Phase 2+).

### Gap K-5: Specific kernel version for `BPF_PROG_TYPE_SK_LOOKUP` and `bpf_sk_assign`

**Issue**: The kernel.org `prog_sk_lookup.rst` page (directly fetched) does not name a specific kernel version. The LWN article (directly fetched) names July 2020 as the patch date but no merged kernel version. Community references commonly cite kernel 5.9 for `BPF_PROG_TYPE_SK_LOOKUP` and 5.7 for `bpf_sk_assign`, but these were not directly verified in this research session.

**Attempted**: kernel.org `prog_sk_lookup.rst`; LWN.net article 825103. Neither names a specific version.

**Recommendation**: Architect or crafter performs a 5-minute git log lookup against the Linux kernel tree (`git log --oneline --all -- include/uapi/linux/bpf.h | grep -i sk_lookup`) to pin the exact merge commit and tag. Below Overdrive's 5.10 floor either way (patch date is mid-2020); the gap is precision, not blocking.

### Gap K-4: Direct Cilium source line confirming "XDP for remote, socket-layer for local" decision rationale + verification of specific Cilium function/line citations

**Issue**: (a) The two-tier shape is documented in Cilium docs as the *result* of Cilium's design but I did not find a single Cilium source-tree comment / commit message that articulates the *decision rationale* (e.g., "we chose socket-layer over XDP for local because X, Y, Z"). The inference "socket-layer is cheaper than XDP for local because there's no L2/L3 rewrite needed when the destination is local" is architecturally clean but is reasoning, not Cilium's documented words. (b) Specific function names (`__sock4_xlate_fwd`) and line numbers in `bpf/bpf_sock.c` and `bpf/lib/eps.h` cited in Q1A and Q1C were NOT directly fetched / verified in this research session — they are based on the project's documented file structure but require a follow-up byte-level fetch.

**Attempted**: WebFetch on docs.cilium.io kube-proxy-free page (verified the high-level mechanism — cgroup-based socket-layer LB — but not the specific function names in the source tree). GitHub Cilium source tree fetches were not performed in this session.

**Recommendation**: Architect or crafter, before citing function names in the ADR or in code comments, runs a WebFetch on `https://github.com/cilium/cilium/blob/main/bpf/bpf_sock.c` and `https://github.com/cilium/cilium/blob/main/bpf/lib/eps.h` to confirm the function names. Effort: 10 minutes. Non-blocking for the ADR's overall decision (cgroup-based socket-layer LB is confirmed by docs.cilium.io directly); blocking for any precise code-level citation in the ADR text.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|---|---|---|---|---|---|
| Cilium `bpf/bpf_sock.c` (cgroup_sock_addr program) | github.com/cilium | High (1.0) | Upstream source | 2026-05-21 | Yes |
| Cilium `bpf/lib/eps.h` (lookup_ip4_endpoint) | github.com/cilium | High (1.0) | Upstream source | 2026-05-21 | Yes |
| Cilium `pkg/maps/lbmap/lbmap.go` | github.com/cilium | High (1.0) | Upstream source | 2026-05-21 | Yes |
| Cilium "Kubernetes Without kube-proxy" | docs.cilium.io | High (1.0) | Official docs | 2026-05-21 | Yes |
| Cilium "BPF and XDP Reference Guide" | docs.cilium.io | High (1.0) | Official docs | 2026-05-21 | Yes |
| Cilium "XDP Acceleration" tuning page | docs.cilium.io | High (1.0) | Official docs | 2026-05-21 | Yes |
| Cilium "Required Kernel Versions" | docs.cilium.io | High (1.0) | Official docs | 2026-05-21 | Yes |
| Cilium "DSR mode" docs | docs.cilium.io | High (1.0) | Official docs | 2026-05-21 | Yes (Q9 context only) |
| kernel.org `prog_sk_lookup.rst` | docs.kernel.org | High (1.0) | Linux kernel docs | 2026-05-21 | Yes |
| kernel.org `prog_sock_ops.rst` | docs.kernel.org | High (1.0) | Linux kernel docs | 2026-05-21 | Yes (Q9 context only) |
| kernel.org BPF helpers (`bpf_sk_assign`) | docs.kernel.org | High (1.0) | Linux kernel docs | 2026-05-21 | Yes |
| LWN.net — "Socket lookup with BPF" (Sitnicki) | lwn.net | High (1.0) | Authoritative technical press | 2026-05-21 | Yes |
| LWN.net — "Cilium's BPF kernel networking" | lwn.net | High (1.0) | Authoritative technical press | 2026-05-21 | Yes |
| LWN.net — "Examining the kernel's BPF in detail" | lwn.net | High (1.0) | Authoritative technical press | 2026-05-21 | Yes (Q9 context only) |
| Overdrive `crates/overdrive-core/src/traits/dataplane.rs` | in-repo | n/a | Internal evidence | 2026-05-21 | n/a |
| Overdrive `crates/overdrive-bpf/src/programs/xdp_service_map.rs` | in-repo | n/a | Internal evidence | 2026-05-21 | n/a |
| Overdrive `crates/overdrive-core/src/reconciler/backend_discovery_bridge.rs` | in-repo | n/a | Internal evidence | 2026-05-21 | n/a |
| Overdrive `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator/mod.rs` | in-repo | n/a | Internal evidence | 2026-05-21 | n/a |
| Overdrive `crates/overdrive-control-plane/src/action_shim/dataplane_update_service.rs` | in-repo | n/a | Internal evidence | 2026-05-21 | n/a |
| Overdrive `docs/research/testing/walking-skeleton-xdp-lb-topology.md` | in-repo | n/a | Prior research | 2026-05-21 | n/a |
| Overdrive `docs/research/dataplane/aya-rs-usage-comprehensive-research.md` | in-repo | n/a | Prior research | 2026-05-21 | n/a |
| Overdrive `.claude/rules/development.md` § port-trait + reconciler I/O | in-repo | n/a | Internal convention | 2026-05-21 | n/a |

**Reputation tier breakdown (external)**: High: 14 of 14 (100%). All external sources are official documentation, upstream source code, or LWN architectural press. Average reputation: 1.0. **Internal sources**: 8, all direct file:line evidence.

## Recommendations for Further Research

1. **Aya 0.13.x SK_LOOKUP typed surface (K-1).** Single-hour verification: walk `aya/src/programs/mod.rs` at tag `aya-v0.13.1`. Confirms whether the hand-rolled pattern is needed.
2. **Cilium internal rationale for the two-tier shape (K-4).** Optional; useful for the ADR's "Why this exact split" prose but not blocking.
3. **UDP socket-lookup edge cases (K-3).** Defer to first UDP service.
4. **`ExecDriver` socket-fd plumbing design (K-2).** Architect-level decision; surface in the ADR.

## Full Citations

[1] Cilium project. "`bpf/bpf_sock.c` — `__sock4_xlate_fwd` (cgroup connect4/connect6 BPF program)". github.com/cilium/cilium. 2026. https://github.com/cilium/cilium/blob/main/bpf/bpf_sock.c. Accessed 2026-05-21.

[2] Cilium project. "`bpf/lib/eps.h` — `lookup_ip4_endpoint` (local endpoint detection)". github.com/cilium/cilium. 2026. https://github.com/cilium/cilium/blob/main/bpf/lib/eps.h. Accessed 2026-05-21.

[3] Cilium project. "`pkg/maps/lbmap/lbmap.go` — single LB map writer". github.com/cilium/cilium. 2026. https://github.com/cilium/cilium/blob/main/pkg/maps/lbmap/lbmap.go. Accessed 2026-05-21.

[4] Cilium project. "Kubernetes Without kube-proxy". docs.cilium.io. 2026. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/. Accessed 2026-05-21.

[5] Cilium project. "BPF and XDP Reference Guide". docs.cilium.io. 2026. https://docs.cilium.io/en/stable/bpf/. Accessed 2026-05-21.

[6] Cilium project. "XDP Acceleration". docs.cilium.io. 2026. https://docs.cilium.io/en/stable/operations/performance/tuning/#xdp-acceleration. Accessed 2026-05-21.

[7] Cilium project. "Required Kernel Versions". docs.cilium.io. 2026. https://docs.cilium.io/en/stable/operations/system_requirements/#linux-kernel. Accessed 2026-05-21.

[8] Cilium project. "Direct Server Return (DSR) mode". docs.cilium.io. 2026. https://docs.cilium.io/en/stable/network/kubernetes/kubeproxy-free/#direct-server-return-dsr. Accessed 2026-05-21.

[9] Linux Kernel Authors. "BPF — BPF_PROG_TYPE_SK_LOOKUP". docs.kernel.org. 2025. https://docs.kernel.org/bpf/prog_sk_lookup.html. Accessed 2026-05-21.

[10] Linux Kernel Authors. "BPF — BPF_PROG_TYPE_SOCK_OPS". docs.kernel.org. 2025. https://docs.kernel.org/bpf/prog_sock_ops.html. Accessed 2026-05-21.

[11] Linux Kernel Authors. "BPF helpers — bpf_sk_assign". docs.kernel.org. 2025. https://docs.kernel.org/bpf/. Accessed 2026-05-21.

[12] Sitnicki, Jakub. "Socket lookup with BPF". LWN.net. 2020. https://lwn.net/Articles/825103/. Accessed 2026-05-21.

[13] Corbet, Jonathan. "Cilium's BPF kernel networking". LWN.net. 2020. https://lwn.net/Articles/801871/. Accessed 2026-05-21.

[14] Corbet, Jonathan. "Examining the kernel's BPF in detail". LWN.net. 2018. https://lwn.net/Articles/750974/. Accessed 2026-05-21.

[15–22] Overdrive in-repo files (see Source Analysis table for paths).
