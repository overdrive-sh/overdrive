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
| **D8** | US-05 forward-key granularity (per-(VIP,port) vs VIP-only) **deferred to US-05 DESIGN**. Disagreement flagged in ADR-0060: shipped validator says SERVICE_MAP forward key is VIP-only (`validate.rs:218`), feature-delta US-05 / phase-2 architecture.md §5 Drift-3 say `(VIP, port)`. Not resolved here. |

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
