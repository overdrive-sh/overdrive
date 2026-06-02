# ADR-0060: `ServiceFrontend` newtype on `Dataplane::update_service`

## Status

Accepted (2026-06-02)

Supersedes the **paper** `Q-Sig locked-A` decision recorded in
`docs/feature/phase-2-xdp-service-map/design/architecture.md` § 5 and
mirrored at `docs/product/architecture/brief.md` § 46 — see *Supersession*
below. That decision was never landed on the trait; this ADR records the
true shipped from-state (option C) and the chosen to-state.

## Context

GitHub issue #163: `REVERSE_NAT_MAP` lockstep populates only TCP entries;
UDP backend responses silently bypass source-rewrite (`XDP_PASS` with no
rewrite → client receives a reply sourced from the backend IP and the
connection breaks). The root cause is that the protocol is **absent from
the dataplane boundary**: the shipped `Dataplane::update_service` carries
no L4 protocol, so the kernel-side reverse-NAT key — which is
`BackendKey { ip, port, proto }` — cannot be derived for the declared
protocol.

Three facts about the from-state, each verified against the live tree:

1. **Shipped signature is option C, not locked-A.** The trait at
   `crates/overdrive-core/src/traits/dataplane.rs:101` is:

   ```rust
   async fn update_service(
       &self,
       vip: Ipv4Addr,
       backends: Vec<Backend>,
   ) -> Result<(), DataplaneError>;
   ```

   Raw `Ipv4Addr`, no `service_id`, no `ServiceVip`, no protocol. The
   phase-2 `architecture.md` § 5 "Q-Sig locked-A" — three explicit args
   `(service_id, vip: ServiceVip, backends)` — was a **paper decision that
   never landed**. brief.md § 46 still prints the paper shape; this ADR
   corrects the record.

2. **`service_id` + `correlation` already live on the Action envelope,
   not the dataplane call.** `Action::DataplaneUpdateService { service_id,
   vip: ServiceVip, backends, correlation }`
   (`crates/overdrive-core/src/reconcilers/mod.rs:440`). The action-shim
   dispatch (`crates/overdrive-control-plane/src/action_shim/dataplane_update_service.rs:100,130`)
   destructures `service_id`/`correlation` for routing/observation and
   calls `update_service(v4, backends)` with neither. They are
   action-routing concerns, **not** dataplane-key concerns: the
   `SERVICE_MAP` key is `(VIP, port)`, the `REVERSE_NAT` key is
   `BackendKey { ip, port, proto }` — neither is keyed by `service_id`.

3. **The Sim over-installs.** `reverse_nat_keys_for`
   (`crates/overdrive-sim/src/adapters/dataplane.rs:266,277`) hardcodes
   `[Proto::Tcp, Proto::Udp]`, so a TCP-only service's Sim REVERSE_NAT key
   set carries a phantom `udp` key. Production `EbpfDataplane` installs
   `tcp` only. The two adapters were never compared — exactly the gap that
   let #163 ship.

The IPv6-rejection precondition is load-bearing and must be preserved.
Today an IPv6 VIP is rejected with an **operator-visible** `Failed`
observation row at the action-shim, via `ipv4_from_vip`
(`dataplane_update_service.rs:160-167`,
`ServiceHydrationDispatchError::Ipv6Unsupported`), which writes a
`ServiceHydrationStatus::Failed { reason }` row
(`dataplane_update_service.rs:112-127`); the hydrator
(`service_map_hydrator.rs:232-249`) parallels this by bumping retry/failure
memory for the IPv6 case. Any signature change must keep IPv6 rejection at
this operator-visible site and **must not** demote it to a late opaque
`DataplaneError` deep in an adapter.

## Decision

Introduce a `ServiceFrontend` newtype carrying the per-service L4 frontend
triple, and thread it as a **typed field of the existing call**:

```rust
async fn update_service(
    &self,
    frontend: ServiceFrontend,
    backends: Vec<Backend>,
) -> Result<(), DataplaneError>;
```

### The `ServiceFrontend` type

New module `crates/overdrive-core/src/dataplane/service_frontend.rs`
(sibling of `backend_key.rs`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceFrontend {
    /// Service virtual IP. **Guaranteed IPv4 by construction** — see
    /// `ServiceFrontend::new`. Adapters may narrow `IpAddr → Ipv4Addr`
    /// infallibly.
    vip: ServiceVip,        // wraps std::net::IpAddr (id.rs:650)
    /// Service listener port. `NonZeroU16` keeps port=0 unrepresentable
    /// as a type guarantee, matching `Listener.port`
    /// (aggregate/workload_spec.rs:544). Project to `BackendKey`'s `u16`
    /// via `.get()`.
    port: NonZeroU16,
    /// L4 protocol (tcp/udp). Reuses `overdrive_core::dataplane::backend_key::Proto`.
    proto: Proto,
}

impl ServiceFrontend {
    /// Fallible constructor — validates the VIP is IPv4 at the action-shim
    /// (the existing operator-visible rejection site). On an IPv6 `vip`
    /// returns the error that maps to the existing operator-visible
    /// `Failed` observation row; the action-shim writes that row exactly
    /// as it does today via `ipv4_from_vip`.
    pub fn new(vip: ServiceVip, port: NonZeroU16, proto: Proto)
        -> Result<Self, ParseError>;

    /// Infallible accessor — the embedded `ServiceVip` is guaranteed IPv4
    /// by construction (`new` rejects IPv6). Adapters call this and narrow
    /// via a documented invariant (`unwrap_or_else(|| unreachable!(...))`),
    /// never re-validating.
    pub fn vip_v4(&self) -> Ipv4Addr;

    pub const fn vip(&self) -> ServiceVip;
    pub const fn port(&self) -> NonZeroU16;
    pub const fn proto(&self) -> Proto;
}
```

**Derives: `Debug, Clone, Copy, PartialEq, Eq` only** (D2). No
serde/utoipa/rkyv — `ServiceFrontend` is neither a wire type nor a
persisted type; it is an ephemeral call argument constructed at the
action-shim and consumed at the adapter. No `Hash` (add on demand). This
is deliberately narrower than `BackendKey` (which is serde+rkyv because it
is the persisted REVERSE_NAT key) and `Listener` (serde+utoipa+rkyv
because it is the intent/spec wire shape).

### V4-guaranteed-by-construction (resolves D1a)

`ServiceFrontend::new` validates the VIP is IPv4 **at the action-shim** —
the same place `ipv4_from_vip` rejects IPv6 today. On success the embedded
`ServiceVip` is guaranteed IPv4; adapters narrow infallibly via `vip_v4()`
(`unreachable!` on the structurally-impossible V6 arm, with a documented
invariant). On an IPv6 VIP, `new` returns the error that the action-shim
maps to the existing operator-visible `Failed` row — IPv6 rejection stays
at the operator-visible site (`dataplane_update_service.rs`), is **not**
demoted to a late opaque `DataplaneError`, and the hydrator's parallel
retry/failure bump (`service_map_hydrator.rs:232-249`) is unchanged. The
rustdoc states the precondition verbatim:
*"the embedded `ServiceVip` is guaranteed IPv4 by construction; adapters
may narrow infallibly."*

### `update_service` trait contract

Per `.claude/rules/development.md` § "Trait definitions specify behavior,
not just signature", the trait-method rustdoc pins the observable contract
both adapters MUST honor:

> Atomically replace the backend set for the service frontend
> `(frontend.vip, frontend.port, frontend.proto)`.
>
> **Preconditions.** `frontend` is V4-guaranteed by construction
> (`ServiceFrontend::new` rejected IPv6 at the action-shim); adapters
> narrow `frontend.vip → Ipv4Addr` infallibly. `backends` MAY be empty
> (see edge cases). Each `Backend.addr` is a `SocketAddr`; non-IPv4
> backend addresses are skipped from the REVERSE_NAT key set (GH #155
> deferral), as the Sim does today.
>
> **Postconditions on `Ok(())`.** After return, the adapter's
> `REVERSE_NAT` key set **for `frontend.proto`** equals exactly the keys
> derived from `backends`: `{ BackendKey { ip, port: backend.addr.port(),
> proto: frontend.proto } : backend ∈ backends, backend.addr is IPv4 }`,
> each mapping to `frontend.vip_v4()`. Keys for **other** protocols of the
> same VIP — installed by separate per-listener `update_service` calls —
> are untouched.
>
> **Edge cases.**
> - `backends.is_empty()` ⇒ **per-proto purge** (D4). The adapter removes
>   the prior `frontend.proto` REVERSE_NAT keys for this VIP that are not
>   still live in another service's backend set (the existing `live_keys`
>   difference check, `sim/.../dataplane.rs:343-347`). REVERSE_NAT keys for
>   *other* protocols of the same VIP are **not** removed.
> - Idempotent re-apply: calling `update_service(frontend, backends)`
>   twice with identical arguments yields the same post-state.
> - A backend with an IPv6 `addr` contributes no REVERSE_NAT key (silently
>   skipped, parity with the Sim's `reverse_nat_keys_for`).
>
> **Observable invariant (cross-adapter).** For the same `(frontend,
> backends)`, `SimDataplane` and `EbpfDataplane` install the **identical**
> `(ip, port, proto) → vip` REVERSE_NAT set. The `ReverseNatLockstep`
> three-tier gate (Tier-1 Sim set-equality + Tier-3 Ebpf `bpftool map dump`
> acceptance + Tier-2 `BPF_PROG_TEST_RUN` triptych) enforces this. Per
> `.claude/rules/development.md` § "The DST equivalence test is the
> structural guard", this gate **is** the equivalence guard the contract
> demands; there is no single-process two-adapter DST (the real adapter
> needs a kernel + bpffs).

### Per-proto purge (resolves D4)

`update_service(frontend_udp, [])` purges only `frontend.proto`'s
REVERSE_NAT keys for the VIP. Other protocols of the same VIP — installed
by separate per-listener calls (US-05) — are untouched. Cross-service
shared-backend keys are preserved by the existing `live_keys` difference
check. This is implemented by narrowing the Sim's `reverse_nat_keys_for`
`[Tcp, Udp]` hardcode to `frontend.proto` (US-01) and mirroring the
per-proto fan-out in `EbpfDataplane` Step 4b (US-02); the
`prior_keys.difference(&new_keys)` / `live_keys` purge logic is unchanged
in shape, only its key-generation closure narrows to one proto.

### True blast radius (resolves D6)

The DISCUSS "5 sites / hydrator unchanged" claim is **corrected**. Because
proto must be plumbed end-to-end (C3 — "Proto NEVER defaulted to Tcp")
without ever defaulting, the protocol dimension is added to the
**Action** and the **desired projection**, not just the trait. The true
US-01 blast radius is **8 sites**:

| # | Site | Path | Change |
|---|------|------|--------|
| 1 | `Dataplane::update_service` trait | `overdrive-core/src/traits/dataplane.rs:101` | signature → `(frontend, backends)` |
| 2 | `ServiceFrontend` newtype | `overdrive-core/src/dataplane/service_frontend.rs` | **CREATE NEW** |
| 3 | `SimDataplane::update_service` + `reverse_nat_keys_for` | `overdrive-sim/src/adapters/dataplane.rs:266,289` | narrow `[Tcp,Udp]`→`frontend.proto` |
| 4 | `EbpfDataplane::update_service` | `overdrive-dataplane/src/lib.rs` | consume `frontend`; Step 4b proto fan-out lands in US-02 |
| 5 | action-shim dispatch | `action_shim/dataplane_update_service.rs:100,130,160` | build `ServiceFrontend` (carries the IPv6-rejection site); call `update_service(frontend, …)` |
| 6 | `ReverseNatLockstep` invariant | `overdrive-sim/src/invariants/reverse_nat_lockstep.rs` | drive via `frontend`; assert per-proto set |
| 7 | **`Action::DataplaneUpdateService`** | `overdrive-core/src/reconcilers/mod.rs:440` | **+ protocol dimension** (port + proto, or a per-listener frontend payload) |
| 8 | **`ServiceDesired` + observation→desired projection** | `overdrive-core/src/reconcilers/service_map_hydrator.rs:40,235,263` | **+ protocol dimension** sourced from a **listener-bearing fact** — `ListenerRow` (`overdrive-core/src/traits/observation_store.rs:321`: `port`/`protocol`/`vip`) and/or the `BackendDiscoveryBridge` per-listener projection (`overdrive-control-plane/src/reconciler_runtime.rs:2569`, keyed `ServiceId::derive(vip, port, "service-map")`) — **NOT** `service_backends` (`ServiceBackendRowV1`, `observation_store.rs:875`), which carries neither port nor proto. The proto MUST be sourced from a listener-bearing fact; if no listener proto can be resolved for the desired projection, that is an error (Failed/structured), NEVER a silent `Proto::Tcp` default (C3). |

Sites 7–8 are the correction: the DISCUSS text said the hydrator was
unchanged for US-01. It is not — C3 (no `Tcp` default) is satisfiable
**only** if the Action and the desired projection carry proto sourced from
a **listener-bearing fact** — `ListenerRow` (`observation_store.rs:321`)
and/or the `BackendDiscoveryBridge` per-listener projection
(`reconciler_runtime.rs:2569`, keyed `ServiceId::derive(vip, port,
"service-map")`). It is **not** sourced from `service_backends`: the
`service_map_hydrator`'s current desired projection reads only
`service_backends_rows` (`reconciler_runtime.rs:1322-1348`), and
`ServiceBackendRowV1` (`observation_store.rs:875`) carries neither port nor
proto — so the proto cannot come from there. `service_map_hydrator.rs`
emits two `DataplaneUpdateService` actions today (IPv6-reject path at :235,
remote path at :263); both gain the protocol dimension. The proto MUST be
sourced from a listener-bearing fact; if no listener proto can be resolved
for the desired projection, that is an error (Failed/structured), NEVER a
silent `Proto::Tcp` default (C3). Whether the Action gains two scalar
fields `(port, proto)` or an embedded per-listener frontend payload is a
US-01 DESIGN detail; the dimension itself is locked here.

**Write-path provenance note (ATLAS-2, forward-pointer).** The existing
`ServiceBackendRow` write path collapses a Service's listeners to the first
(`reconciler_runtime.rs:2015-2019`: first-listener-only, `backend_port`
defaults to `0` when no listener, no proto recorded). US-01 must therefore
source proto from the listener-bearing fact above, **NOT** from the
proto-less `ServiceBackendRow`. The multi-listener generalization — and the
resulting extra `ServiceBackendRow` write-path site — is **US-05** scope.

US-05 (per-listener fan-out — one `update_service` per `Listener`) is a
later slice; this ADR forward-points to it for the multi-listener emission
granularity.

## Alternatives Considered

Scored taste matrix from `docs/feature/udp-service-support/recommendation.md`
(DIVERGE). Higher is better.

| Option | Score | Disposition |
|---|---|---|
| **Opt 6 — `ServiceFrontend` newtype field** (chosen) | 4.17 | Type-safe home for the validated VIP + non-zero port + proto; smallest typed surface; re-absorbs locked-A's `ServiceVip` intent without folding `service_id`/`correlation` into a dataplane key. |
| Opt 1 — positional `proto` arg | 4.13 | Tied on score, rejected: a bare positional `Proto` re-strands the VIP as a raw `Ipv4Addr` and gives port/proto no typed home; reintroduces the positional-arg sprawl C2 forbids. The newtype was chosen for the type-safe VIP/port/proto home. |
| Opt 2 — typed aggregate (frontend + backends + service_id + correlation in one struct) | 3.57 | Rejected: folds action-routing/correlation (`service_id`, `correlation`) into the dataplane call, which is not a dataplane-key concern. **Recorded dissent** (H3): the aggregate wins ONLY if (a) multi-listener becomes a *trait-surface* concern, OR (b) the team commits to `update_service`-as-typed-SSOT, OR (c) an explicit industry-alignment reweight — none established by J-OPS-004 / J-PLAT-004. |
| Opt 4 — no signature change; carry proto on the Action only | (lower) | Rejected: the dataplane boundary still cannot see proto; the kernel-side REVERSE_NAT key cannot be derived. This is the from-state's defect, not a fix. |

## Consequences

**Positive.**
- The dataplane boundary carries the protocol; the #163 reverse-NAT
  asymmetry becomes expressible and gateable.
- `(vip, port, proto)` has a single typed home; no call site reconstructs
  the triple from scattered positional args (C2).
- IPv4 guarantee is structural: adapters narrow infallibly; the
  operator-visible IPv6 rejection site is preserved unchanged.
- The Sim's phantom `udp` key for TCP-only services is corrected.

**Negative / accepted.**
- True blast radius is 8 sites (Action + desired projection included), not
  the 5 the DISCUSS text claimed — single-cut migration per C6, all in the
  US-01 PR. Documented in
  `docs/feature/udp-service-support/design/upstream-changes.md`.
- `service_id`/`correlation` continue to travel separately on the Action.
  This is by design (action-routing, not dataplane-key) but means readers
  must know the frontend is **not** the whole envelope.

## Endianness note (D7)

No new endianness discipline. `Proto` is a single-byte IANA scalar
(`Proto::as_u8()` → 6/17, `backend_key.rs:76`); it has no byte-order
concern. The § 11 host-order/network-order lockstep (architecture.md)
continues to govern **ip and port only**. `ServiceFrontend.port`
(`NonZeroU16`) projects to `BackendKey.port` (`u16`, host-order on the
userspace side) via `.get()`; the kernel-side egress program does the
network-order conversion at the read boundary exactly as today.

## Flagged for US-05 (D8 — do not resolve here)

The shipped validator says the `SERVICE_MAP` forward key is **VIP-only**
(`crates/overdrive-control-plane/src/action_shim/validate.rs:218,230-231`
— `Action::DataplaneUpdateService { vip, .. } ⇒ WriteKey { port_opt: None,
route: Xdp }`), while feature-delta US-05 and phase-2 `architecture.md` § 5
Drift-3 say the forward key is `(VIP, port)`. This forward-key-granularity
disagreement is **deferred to US-05 DESIGN** to reconcile. It does not
affect this ADR: the REVERSE_NAT key (the #163 surface) is unambiguously
`BackendKey { ip, port, proto }` regardless of how the forward key is
keyed.

## Enforcement

The three-tier `ReverseNatLockstep` gate is the structural guard
(`.claude/rules/development.md` § "The DST equivalence test is the
structural guard"):

- **Tier 1** — `ReverseNatLockstep` invariant (`overdrive-sim/src/invariants/reverse_nat_lockstep.rs`),
  per-PR critical path via `cargo dst`: SimDataplane installs exactly the
  declared-`frontend.proto` `BTreeSet<BackendKey>`; fails if the fan-out
  drops the proto key.
- **Tier 2** — `BPF_PROG_TEST_RUN` triptych (`overdrive-bpf` tests) via
  `cargo xtask bpf-unit`: `xdp_reverse_nat_lookup` rewrites a proto=17
  response source to the VIP.
- **Tier 3** — real-veth Ebpf acceptance (`overdrive-dataplane/tests/integration`,
  `integration-tests` feature) via `cargo xtask lima run`: `bpftool map
  dump REVERSE_NAT_MAP` shows `(ip,port,udp)` + a wire capture sourced from
  the VIP.

No external third-party integration in this feature — no consumer-driven
contract test is warranted (the dataplane boundary is an internal trait,
not an external API).

## References

- GH #163 (source bug).
- `docs/feature/udp-service-support/feature-delta.md` (DISCUSS).
- `docs/feature/udp-service-support/recommendation.md` (DIVERGE taste matrix).
- ADR-0041 (weighted Maglev + reverse-NAT shape), ADR-0042
  (`ServiceMapHydrator`), ADR-0049 (`ServiceVip` allocator / IPv4-only),
  ADR-0053 (cgroup same-host backend delivery).
- Superseded: `docs/feature/phase-2-xdp-service-map/design/architecture.md`
  § 5 Q-Sig locked-A; `docs/product/architecture/brief.md` § 46.
