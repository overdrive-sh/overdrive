# DESIGN Decisions — udp-service-support

> **Location note.** DISCUSS decisions live in `../feature-delta.md`
> (D1–D8, § Wave decisions). DIVERGE decisions live in
> `../diverge/wave-decisions.md` (scored option study). This file holds the
> DESIGN-wave decisions — Phase A enumerated and the user LOCKED them;
> Phase B (this wave) wrote the SSOT. The canonical SSOT is **ADR-0060** +
> `brief.md` § "UDP service support extension" + `c4-diagrams.md`; this file
> is the decisions summary and does not supersede them.

**Architect:** Morgan. **Date:** 2026-06-02. **Mode:** Propose (decisions
pre-locked). **Density:** lean / Tier-1.

## Locked decisions

| ID | Decision |
|---|---|
| **D1a** | `ServiceFrontend { vip: ServiceVip, port, proto }` — a **literal re-absorb** of `ServiceVip` (which wraps `std::net::IpAddr`, `id.rs:650`). **V4-guaranteed-by-construction**: fallible `ServiceFrontend::new(vip, port, proto) -> Result<Self, ParseError>` validates the VIP is IPv4 **at the action-shim** — the existing operator-visible rejection site (`action_shim/dataplane_update_service.rs:160`, `ipv4_from_vip` → `ServiceHydrationStatus::Failed`). Adapters narrow `IpAddr → Ipv4Addr` infallibly via `vip_v4()` (documented invariant / `unreachable!`). The operator-visible Failed-row path is unchanged; IPv6 is **not** demoted to a late opaque `DataplaneError`. Rustdoc states: "the embedded `ServiceVip` is guaranteed IPv4 by construction; adapters may narrow infallibly." |
| **D1b** | `port: NonZeroU16` (matches `Listener.port`, `aggregate/workload_spec.rs:544`; port=0 unrepresentable). Semantics = service listener port. Project to `BackendKey.u16` via `.get()`. |
| **D2** | Derives `#[derive(Debug, Clone, Copy, PartialEq, Eq)]` only. No serde/utoipa/rkyv (not wire, not persisted), no Hash (add on demand). |
| **D3** | New file `crates/overdrive-core/src/dataplane/service_frontend.rs` (sibling of `backend_key.rs`). |
| **D4** | Empty-backends purge is **per-proto**: `update_service(frontend_udp, [])` purges only `frontend.proto`'s REVERSE_NAT keys for the VIP; other protos of the same VIP (separate per-listener calls) are untouched; cross-service shared-backend keys preserved via the existing `live_keys` difference check (`sim/.../dataplane.rs:343-347`). |
| **D5** | New numbered ADR — **ADR-0060** (next free; latest core-platform ADR was 0059). Supersedes phase-2 §5 Q-Sig locked-A (paper, never landed). |
| **D6** | Proto plumbing folds into **US-01** (NOT US-04). True blast radius = **8 sites**: trait + EbpfDataplane + SimDataplane + action-shim dispatch + ReverseNatLockstep + **`Action::DataplaneUpdateService`** + **`ServiceDesired`** + **observation→desired projection**. The DISCUSS "5 sites / hydrator unchanged" claim is corrected (C3 — "Proto NEVER defaulted to Tcp" is satisfiable only this way). |
| **D7** | No new endianness discipline. `Proto` is a single byte / IANA scalar (`Proto::as_u8()` → 6/17); §11 lockstep continues to govern ip/port only. |
| **D8** | US-05 forward-key granularity (per-(VIP,port) vs VIP-only) **deferred to US-05 DESIGN**. Disagreement flagged in ADR-0060: shipped validator says SERVICE_MAP forward key is VIP-only (`validate.rs:218`), feature-delta US-05 / phase-2 architecture.md §5 Drift-3 say `(VIP, port)`. **RESOLVED by P2-Q4 below** (2026-06-03): the forward key is `(VIP, port, proto)`; OQ-1/D8 is subsumed. |

## P2-Q4 resolution — proto in the service-LB map keys (2026-06-03, user-locked)

**Decision (LOCKED by the user).** Add L4 protocol to **both** eBPF
service-LB map keys, IPVS-style:

- `SERVICE_MAP` outer key: `(ServiceVip, u16 port)` → **`(ServiceVip,
  u16 port, Proto)`** (the wire-boundary XDP forward path).
- `LOCAL_BACKEND_MAP` key: `(VIP, vip_port)` → **`(VIP, vip_port,
  proto)`** (the same-host cgroup `connect4` path).

**User rationale (verbatim, confirmed):** *"we don't want to fix
incorrect architecture — do `(vip, port, proto)` as IPVS."* Proto-less
keying is the wrong architecture; we match IPVS, which keys every
virtual service on `{protocol, addr, port}`.

**This was the open question slice-05 owned.** Slice 05
(`slices/slice-05-hydrator-multi-listener-fanout.md` § Pre-slice SPIKE)
asked: *"does each listener get its own `(VIP, port)` SERVICE_MAP key …
(P2-Q4/P2-Q5)?"* — answered: **each listener gets its own `(VIP, port,
proto)` key.** A TCP listener and a UDP listener on the same
`(VIP, port)` occupy two distinct map slots. The hydrator's
per-listener fan-out (one `update_service` per listener, ADR-0060) maps
1:1 onto distinct proto-keyed slots; no listener overwrites another.

**Why (evidence-weighted).** Per
`docs/research/dataplane/service-map-l4-proto-keying-research.md`
(Nova, High confidence, 13 trusted-domain sources):

1. **Linux IPVS keys virtual services on `{protocol, addr, port}`
   natively** (UAPI `ip_vs_service_user`); kube-proxy iptables mode is
   per-protocol. Proto-in-key is the default in the two oldest, most
   deployed k8s dataplanes.
2. **Cilium carried a proto-less `lb4_key` as a KNOWN DEFECT for ~5.5
   years** — issue #9207 (Sept 2019) → fix PR #37164 (Jan 2025). The
   proto byte sat reserved-but-unused the whole time; TCP+UDP-on-same-
   port could not coexist. Proto-less was a bug they spent half a
   decade closing, not a valid model.
3. **Kubernetes treats TCP+UDP-on-same-port as first-class** — CoreDNS
   declares `tcp/53 + udp/53`; the `MixedProtocolLBService` feature
   gate; HTTP/3 QUIC (`443/udp` + `443/tcp`). DNS is the canonical
   day-one driver; a `(vip, port)` key *cannot represent the DNS
   service correctly*.
4. **Widening a HASH_OF_MAPS outer key is structurally free** — any
   POD outer key, no nesting/size penalty (kernel `map_of_maps` docs).
   Overdrive's `ServiceKey` / `LocalServiceKey` are already 8-byte
   `#[repr(C)]` PODs with a zeroed `_pad`; the proto byte consumes one
   reserved pad byte with no byte-width change.

**Design sub-choices (recommendations, not open).**

- **Struct layout:** `proto: u8` (IANA: IPPROTO_TCP=6 / IPPROTO_UDP=17)
  absorbs one of the two reserved `_pad` bytes; struct stays 8 bytes;
  trailing `_pad: u8` stays deterministically zeroed for stable BPF
  hashing (mirrors Cilium's `__u8 proto; __u8 scope; __u8 pad[2]`).
- **Proto source on the cgroup path:** read `bpf_sock_addr.protocol`
  (verified present in the in-tree UAPI — carries the IANA byte
  directly; zero translation). `bpf_sock_addr.type` (SOCK_STREAM/
  SOCK_DGRAM) is the documented fallback only if a matrix kernel
  leaves `protocol` unset. Verified Tier 3 (no Tier 2 backstop for
  `cgroup_sock_addr`).
- **`Action` proto field:** `Action::DataplaneUpdateService` (forward
  path, ADR-0060 site #7) and `Action::RegisterLocalBackend` /
  `DeregisterLocalBackend` (same-host path, ADR-0053 amendment) all
  gain a `Proto` dimension, sourced from a **listener-bearing fact**
  (ADR-0060 site #8) — NEVER a silent `Proto::Tcp` default (C3).
- **Migration:** single-cut, reconciler-repopulated — the key structs
  change, the maps are recreated on next boot. NO live in-place
  migration, NO dual-key shim, NO deprecation path (per
  `feedback_single_cut_greenfield_migrations.md`).

**Reuse disposition.** Every touched component is **EXTEND** of an
existing map/struct/program/handle — **zero CREATE NEW**. `Proto` is
REUSE (ADR-0060's `backend_key::Proto`). See feature-delta § "Wave:
DESIGN / [REF] P2-Q4 reuse analysis".

**Topology.** No topology change — same maps, wider key. No new C4
component diagram warranted (explicitly noted in the deliverables; a
redundant diagram would add noise, not signal).

**SSOT.** Amended ADRs are the SSOT:
- **ADR-0040** revision 2026-06-03 — SERVICE_MAP outer key.
- **ADR-0053** revision 2026-06-03 — LOCAL_BACKEND_MAP key +
  cgroup_connect4 proto-source contract + `RegisterLocalBackend`
  proto field + sendmsg4 scope note.
- **ADR-0060** — already carries `ServiceFrontend { vip, port, proto }`
  at the boundary; **no change needed**; referenced as the companion
  that put proto on the boundary and as the source of the `Proto` type.

**Open deferral — tracked as
[#200](https://github.com/overdrive-sh/overdrive/issues/200).**
Unconnected-UDP (`sendto(VIP, ...)` without `connect()`) is NOT
delivered — it needs a separate `sendmsg4` (`BPF_CGROUP_UDP4_SENDMSG`)
hook, not implemented today (DNS resolvers `sendto` per query without
connecting). See ADR-0053 amendment § "Out of scope".

## `ServiceFrontend` — final shape

```rust
// crates/overdrive-core/src/dataplane/service_frontend.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceFrontend {
    vip: ServiceVip,        // V4-guaranteed by construction
    port: NonZeroU16,       // service listener port
    proto: Proto,           // reused from backend_key
}

impl ServiceFrontend {
    pub fn new(vip: ServiceVip, port: NonZeroU16, proto: Proto)
        -> Result<Self, ParseError>;   // validates IPv4 at the action-shim
    pub fn vip_v4(&self) -> Ipv4Addr;  // infallible narrow (invariant)
    pub const fn vip(&self) -> ServiceVip;
    pub const fn port(&self) -> NonZeroU16;
    pub const fn proto(&self) -> Proto;
}
```

New trait signature:

```rust
async fn update_service(
    &self,
    frontend: ServiceFrontend,
    backends: Vec<Backend>,
) -> Result<(), DataplaneError>;
```

## Reuse Analysis (HARD GATE — see feature-delta § DESIGN for full table)

`ServiceFrontend` = **CREATE NEW**, justified: no existing type expresses
`(service VIP, listener port, proto)`. Rejected reuse of `Listener` (intent
wire type, no VIP) and `BackendKey` (backend-side REVERSE_NAT key,
semantically inverted — `ip` is the backend, not the VIP). All other sites
EXTEND or REUSE. `Proto`, `BackendKey`, `Listener` = REUSE.

## Technology stack

No new third-party dependency. `NonZeroU16` (std), `Proto` (in-repo,
shipped #164). Enforcement: dst-lint + the three-tier `ReverseNatLockstep`
gate (`cargo dst` T1 / `cargo xtask bpf-unit` T2 / `cargo xtask lima run`
T3). No external API integration → no consumer-driven contract test.

## Constraints carried from DISCUSS (C1–C8)

All honored. Notably: **C2** (single typed source of `(vip,port,proto)`;
`service_id`/`correlation` on the Action by design), **C3** (proto never
defaulted to `Tcp` — satisfied by D6's end-to-end plumbing), **C5**
(production not shaped by simulation — lockstep reshapes the invariant, not
production), **C6** (single-cut migration — all 8 sites in the US-01 PR).

## Upstream (back-prop) changes

Two DISCUSS corrections recorded in `upstream-changes.md` (this directory):
(a) US-02 Example 3 "empty backend set removes BOTH protos" → **per-proto**
purge (D4); (b) "5 sites / hydrator unchanged" → **8-site** blast radius,
proto plumbed end-to-end in US-01 (D6).

## Handoff

DESIGN baseline is ready for DISTILL (acceptance-designer) and the
DEVOPS/platform-architect handoff. No external integrations → no contract-
test annotation. The consolidated peer review fires at end of DISTILL (not
run here).
