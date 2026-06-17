# DESIGN decisions — transparent-mtls-enrollment (GH #236)

**Wave: DESIGN** · Architect: Morgan · Mode: propose · Paradigm: OOP ·
Scope: application/components (the ENFORCE/interception layer + the resolve-port boundary)

## What was designed

Path A (the spike's ratified direction) as the production transparent-mTLS
enrollment mechanism: **per-workload netns+veth + nft-TPROXY + `IP_TRANSPARENT`
+ `getsockname` for BOTH directions**, with per-connection **enrollment**
resolve replacing the retired per-destination map. The agent-light kTLS
enforcement substrate (ADR-0069/0070) is reused UNCHANGED. Q1–Q4 are RATIFIED;
Q5a (the DNS name-layer **integration** — resolv.conf injection into the
per-workload netns, **headless** DNS-return for v1) is folded in (the #61
responder daemon and the #167 VIP allocator remain separate builds / dependencies).

## SETTLED (designed around; not relitigated)

| # | Decision | Source |
|---|---|---|
| D-TME-1 | Interception = nft-TPROXY + `IP_TRANSPARENT` + `getsockname`, both directions. | spike wave-decisions |
| D-TME-3 | `cgroup/connect4`-rewrite + `MTLS_REDIRECT_DEST` map + `program_declared_peer_redirect` RETIRED. | spike (Probe A DOESN'T-WORK) |
| D-TME-4 | Outbound `accept_outbound_leg` recovers `peer` via `getsockname` (symmetric with inbound). | follows D-TME-1 |
| D-TME-5 | 4-method `MtlsEnforcement` port UNCHANGED; `Routed::Outbound { peer }` still the input. | ADR-0069/0070 locked core |
| — | Enrollment capture-and-resolve is the model (NOT a per-destination map). | spike + research |

## RATIFIED (this DESIGN wave — Q1–Q4 + Q5a ratified by the user 2026-06-16)

| # | Decision | Rationale |
|---|---|---|
| D-TME-2 | v1 moves OFF host-netns ONTO per-workload netns+veth; shape = **extend `veth_provisioner`** (Q2 ratified). | TPROXY+`getsockname` needs an agent-controlled routing point per workload (Cilium topology). |
| D-TME-6 | New `MtlsResolve` driven port = the #178 anti-corruption boundary; this feature defines the contract + a v1 `service_backends`-reading host adapter, **fail-closed (not silent)** (Q3 ratified). | The enrollment model requires a per-connection resolve consumer; no existing port fits; entangling it into `MtlsEnforcement` (frozen at 4 methods) is forbidden; a silent-empty resolve re-introduces the silent-cleartext footgun. |
| D-TME-7 | Egress-on-per-workload-veth nft-TPROXY is UNVALIDATED + has no Tier-2 backstop → **validate via a thin Tier-3 spike NOW (`increment-b/`)** before DELIVER (Q1 ratified). | Single novel piece; cheapest place to find an `ip rule`/route/F5-exemption collision (the research's Probe B falsification path). |
| D-TME-8 | v1 scope = **BOTH directions**; intended-peer SVID pinning (`expected_peer`/`PeerIdentityMismatch`) **deferred to #178** (v1 = authn-only) (Q4 ratified). | Path A's point is symmetry on one mechanism; the inbound nft-TPROXY install is the proven template the outbound mirrors; the resolve port carries `expected_svid` so the pin wires the moment #178 supplies the join. |
| D-TME-9 | **Name-layer integration (Q5a)**: a node-local DNS responder is injected into the per-workload netns `resolv.conf` (Fly.io `fdaa::3` model); the responder *daemon* is #61 (separate build), only the injection + return-shape contract live here. | The per-workload netns (Q2) IS the DNS injection point — one topology, two wins; sidecarless (Overdrive ships its own appliance OS, ADR-0068). |
| D-TME-10 | **DNS-return shape = HEADLESS for v1**: the responder returns a `running` backend addr from `service_backends` — that address IS the `orig_dst` `MtlsResolve.resolve` recognizes (one source, two readers, byte-consistent). **No #167 (VIP allocator) v1 dependency.** VIP is the multi-node evolution. | Headless keeps `MtlsResolve` v1 thin (identity-only, no LB — LB-pick is the #178-deferred policy), pulls no new v1 dependency, and is forward-compatible (VIP arm added alongside later, K8s ships both). VIP for v1 was REJECTED (would add #167 + the VIP×intercept ordering hazard). |
| D-TME-12 | **Per-allocation network SLOT model (resolves the 02-01 review B1+S1+S2; refines D-TME-2/C3)**: the per-allocation netns name, the two veth-end iface names, AND the point-to-point /30 subnet are ALL derived from a single **host-unique bounded network slot** — a new `NetSlot` newtype (`overdrive-control-plane`), NOT a hash of the `AllocationId`. **A hash is WRONG by pigeonhole** (`AllocationId` is `LABEL_MAX`=253-bounded; a Linux iface name is `IFNAMSIZ`=15-usable-bounded; no pure function of a 253-char id can collision-free-map into a 15-char name — a hash makes collisions merely *unlikely*, the exact hand-wave CLAUDE.md § "One shared length ceiling for label-shaped ids" forbids). The slot makes collision-freedom **structural**: distinct slot ⇒ distinct names ⇒ distinct /30 (single-source). **Slot domain**: `NetSlot(u16)`, valid range `0..=4095` (4096 concurrent per-alloc netns slots — single-node bounded concurrency; ample, and 4 hex chars renders ≤15). **Rendering**: 4-char lowercase zero-padded hex (`{:04x}`), so `ovd-hv-<4hex>` / `ovd-wl-<4hex>` = 11 chars ≤ 15 IFNAMSIZ by construction; the 4-char ceiling is DERIVED from `IFNAMSIZ - PREFIX.len()` (15−7=8 budget; 4 used), not a magic number. **/30 derivation**: the slot indexes a /30 block inside a fixed per-host `WORKLOAD_SUBNET_BASE` (`10.99.0.0/16`): subnet = base + `slot * 4` as a /30 → `host_addr` = base+slot*4+1, `workload_addr` = +2, `gateway` = `host_addr`. 4096 slots × /30 = the full `/16`. **netns name keeps the readable `ovd-ns-<alloc>`** (a `/var/run/netns/` filename, ≤255, NOT IFNAMSIZ-bound) for `ip netns list` traceability — the slot is the iface/subnet axis, the alloc id is the human axis; both resolve to the same allocation via the allocator's slot↔alloc map. **`derive_workload_netns_plan` takes the slot as a PURE input** (`(alloc_id, slot, responder_addr) -> WorkloadNetnsPlan`); the subnet is no longer a caller parameter (S1 resolved: the derivation owns slot→/30, the allocator owns slot assignment). The STATEFUL slot allocator (assign-smallest-free / release-on-teardown, a per-host free-list — NOT distributed IPAM, NOT the #167 VIP allocator) lives at the **C3 `on_alloc_running` lifecycle hook** (release at `on_alloc_terminal`), the same hook that owns netns creation. S2 resolved: a /30 always has two usable hosts, so `workload_addr` is non-degenerate by construction (no `Option`, no `network()` fallback). | The 02-01 review (B1) ground-truthed that the literal `ovd-hv-<alloc>` overflows IFNAMSIZ for any alloc id ≥ 9 chars (the golden test's own `ovd-hv-payments-0` = 17 chars is uncreatable) and a naïve truncation collides two allocs onto one veth. B1+S1+S2 are one problem: the missing host-unique handle that both names AND the /30 must derive from. No existing host-unique per-alloc integer exists (`alloc-{workload_id}-{attempt}` is workload-scoped, not host-unique; the cgroup scope keys on the full id string). The slot model makes collision-freedom by-construction and resolves all three findings in one coherent decision. (Ratified by the user 2026-06-17; resolves review `deliver/reviews/02-01.md` B1+S1+S2.) |
| D-TME-11 | **Resolve READ MECHANISM (C4; refines D-TME-6)**: `ServiceBackendsResolve` resolves `orig_dst` against an **in-RAM, address-keyed, ownership-aware reverse index** of the `running` `service_backends` set (`addr → {service → Backend}`, NOT a flat `addr → Backend` with global last-writer-wins — see F-A below) — NOT a per-`ServiceId` point query (the `ServiceId`-keyed `service_backends_rows` is the wrong surface; the adapter holds no `ServiceId`). **REVISED 2026-06-17 (resolve-index-coherence research):** built via **List-then-Watch + relist-on-`Lagged`** (the prior observe-only / "no new trait method" constraint is REVERSED). List leg = the keyless `all_service_backends_rows()` enumerate (SHIPPED `25e7acf3`); List-at-probe closes #237 cold-start; single-owner drain dissolves the F2 take/restore TOCTOU. **F4 / relist-trigger REFINED 2026-06-17 (ratified — option 2, surface `Lagged`):** the lossy `subscribe_all()` (item type `ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>`) could not carry the loss signal — both adapters stripped `RecvError::Lagged` internally — so closing F4 needed a lag-surfacing surface `subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError>` delivering `SubscriptionEvent::{Row, Lagged { missed: u64 }}` (a DOMAIN event; adapter maps `RecvError::Lagged(n) → missed`; no tokio leak); the single-owner drain consumes it and re-Lists on `Lagged`, closing F4 with a *completeness* guarantee. **F-B (reconciled 2026-06-17): `subscribe_all_events()` is now the SOLE observation-subscription surface — the lossy `subscribe_all()` + `ObservationSubscription` alias were DELETED single-cut in commit `36a79762` and every consumer migrated** (superseding the earlier "dedicated method bounds blast radius / ~20 consumers stay untouched / not a shared-type change" framing, which the single-cut overtook — that framing is preserved only as dated honest history in the C4 condition row below). **F-A (ratified 2026-06-17 — option (b)): ownership-aware index** — keyed per contributing service at an addr; a service's backend-set shrink evicts only THAT service's contribution; classification is `any-healthy-at-addr` (deterministic, NOT last-writer-wins). This removes the unstated "one `(IP:port)` belongs to at most one service" cross-component invariant and the LWW healthy-disagreement determinism smell; v1 single-node is structurally addr-exclusive (per-addr service set size-1 today), so the shape is defensive against multi-node / future writers, NOT a behaviour change. The structure is adapter-internal; the public `MtlsResolve` contract + the `NonMesh`/`MeshUnreachable`/`Err` arms are UNCHANGED. **A miss = `NonMesh`** (cleartext pass-through), NOT `MeshUnreachable`; the residual irreducible convergence window is the **(a) fail-toward-handshake** v1 SECURITY invariant, tracked in **#236**. **#237 CLOSED by this revision** (List-at-probe + relist). PUBLIC `MtlsResolve` API unchanged; growth confined to the `ObservationStore` driven port. | D-TME-6 pinned the resolve *model* but not the *read mechanism*; a DELIVER step surfaced that `resolve(orig_dst)` has no `ServiceId` and no addr→service surface exists. Cilium's `ipcache` (in-RAM addr→identity reverse index, subscribe-populated, List-before-Watch, relist-on-loss) is the canonical precedent; research §4.1 describes the resolve as an in-RAM `service_backends` lookup. Making a miss fail-closed would break legitimate `NonMesh` external egress — forbidden. (Ratified 2026-06-16; read-mechanism REVISED 2026-06-17; F4/relist-trigger REFINED 2026-06-17; F-A ownership-aware index + F-B `subscribe_all` single-cut reconciled 2026-06-17.) |

## D-TME-12 — pinned API contract for the crafter (02-01 re-implementation)

The 02-01 review (`deliver/reviews/02-01.md`) returned NEEDS_REVISION on two
blocking defects (B1, B2) + S1/S2/S3 + N1. D-TME-12 (above) is the coherent
resolution of B1+S1+S2; B2/S3/N1 are pinned here. The crafter builds ONLY the
surface named below (CLAUDE.md § "Implement to the design — never invent API
surface").

### New newtype — `NetSlot` (`overdrive-control-plane`, domain-bearing)

```rust
/// A host-unique, bounded per-allocation network slot. The single axis from
/// which a workload's netns iface names AND its point-to-point /30 subnet
/// derive — collision-free by construction (distinct slot ⇒ distinct names ⇒
/// distinct subnet). Bounded to `0..=NET_SLOT_MAX` so the rendered veth names
/// fit IFNAMSIZ (see `derive_workload_netns_plan`). NOT a hash of the
/// AllocationId (pigeonhole — see D-TME-12).
pub struct NetSlot(u16);

/// Inclusive upper bound — 4096 concurrent per-alloc slots (single-node
/// bounded concurrency). 4 hex chars renders ≤ 15 IFNAMSIZ; the full slot
/// space tiles `WORKLOAD_SUBNET_BASE` (/16) into 4096 /30 blocks.
pub const NET_SLOT_MAX: u16 = 4095;
```

Newtype completeness (development.md § "Newtype completeness"): `FromStr`
(validates `0..=NET_SLOT_MAX`, returns `Result<Self, _>`), `Display` (the
canonical decimal form), `Serialize`/`Deserialize` matching `Display`/`FromStr`,
a validating `new(u16) -> Result<Self, NetSlotError>` (rejects `> NET_SLOT_MAX`).
A `to_hex4(&self) -> String` (`format!("{:04x}", self.0)`) renders the
iface-name fragment. The 4-char hex ceiling is DERIVED — assert in a doctest /
unit that `WORKLOAD_HOST_VETH_PREFIX.len() + 4 <= 15` (IFNAMSIZ) so a future
prefix change that would overflow fails the build, not a runtime `ip link add`.

### Pinned `derive_workload_netns_plan` signature

```rust
/// Per-host base block all per-alloc /30s are carved from. 4096 /30s tile the
/// whole /16. NOT operator-tunable in v1 (single-node, fixed).
pub const WORKLOAD_SUBNET_BASE: Ipv4Net = /* 10.99.0.0/16 */;

#[must_use]
pub fn derive_workload_netns_plan(
    alloc_id: &overdrive_core::AllocationId,
    slot: NetSlot,
    responder_addr: Ipv4Addr,
) -> WorkloadNetnsPlan;
```

Derivation rules (PURE, total — no `Option`, no `network()` fallback, because a
/30 always has two usable hosts):

- `netns`         = `format!("ovd-ns-{}", alloc_id.as_str())` — readable, ≤255 (NOT IFNAMSIZ-bound).
- `host_veth`     = `format!("ovd-hv-{}", slot.to_hex4())` — 11 chars ≤ 15.
- `workload_veth` = `format!("ovd-wl-{}", slot.to_hex4())` — 11 chars ≤ 15.
- `subnet`        = the /30 at `WORKLOAD_SUBNET_BASE.network() + (slot.0 as u32 * 4)`, prefix-len 30.
- `host_addr`     = `subnet.network() + 1` (first usable host).
- `workload_addr` = `subnet.network() + 2` (second usable host).
- `gateway`       = `host_addr` (in-netns default route points back at the host-side end).
- `responder_addr` flows through verbatim (carried for D-TME-9 resolv.conf injection; NOT derived state).

The `WorkloadNetnsPlan` struct keeps its existing field set (`netns`,
`host_veth`, `workload_veth`, `host_addr`, `workload_addr`, `gateway`, `subnet`,
`responder_addr`). It MAY carry the `slot: NetSlot` it was derived from (an
input, useful for the executor's teardown + the slot↔alloc map); the crafter
adds the field only if the executor/teardown needs it — do not add it
speculatively.

### B2 (blocking) — in-netns end up + loopback up

A veth pair forwards only when BOTH ends are up, and a fresh netns has `lo`
down. The model must express both. Pinned additions:

`ObservedWorkloadVeth` gains TWO observed facts:
- `workload_veth_up: bool` — the in-netns end is administratively UP.
- `lo_up: bool` — the netns's loopback (`lo`) is UP.

`WorkloadVethStep` gains TWO variants, BOTH ordered AFTER
`MoveWorkloadEndIntoNetns` (the end must be in the netns before it can be
brought up in-netns):
- `SetWorkloadVethUp` — `ip -n <netns> link set <workload_veth> up`.
- `SetLoopbackUp`     — `ip -n <netns> link set lo up`.

`workload_converge_steps` emits each when (`pair_rebuilt` OR the respective fact
is false). The named proptest's iff-emit clauses extend to both new facts; the
complete-observation baseline sets both `true`.

### S3 (architect contract call) — split `rp_filter_relaxed`

Split the lossy single bool into TWO observed facts (mirroring the per-end
`tx_offload` shape), so the executor cannot guess:
- `rp_filter_global_relaxed: bool` — `net.ipv4.conf.all.rp_filter` AND `net.ipv4.conf.lo.rp_filter` are relaxed (host-global).
- `host_veth_rp_filter_relaxed: bool` — `net.ipv4.conf.<host_veth>.rp_filter` is relaxed (per-allocation; strict by default on a fresh veth).

`WorkloadVethStep` correspondingly splits `RelaxRpFilter` into:
- `RelaxGlobalRpFilter` — relax `all` + `lo` (emit when `!rp_filter_global_relaxed`).
- `RelaxHostVethRpFilter` — relax the per-veth knob (emit when `pair_rebuilt OR !host_veth_rp_filter_relaxed`; a freshly-created veth defaults strict).

This removes the correctness burden the single bool pushed onto 02-02's observer
(a new alloc on a host where `all`/`lo` are already relaxed still needs ITS OWN
host-veth knob relaxed).

### N1 (confirm) — `let _ = plan;` is INTENTIONAL

The pure diff keys only on observed facts; `plan` carries the names/addresses
the executor needs and mirrors the sibling `converge_steps(&plan, &observed)`
signature. KEEP `plan` in the signature; the `let _ = plan;` (or a `#[expect]`)
is deliberate. Once `RelaxHostVethRpFilter` / the slot-derived names are wired,
the executor reads `plan.host_veth` etc., so the param is genuinely consumed at
02-02 — the pure diff still keys only on observed facts.

### Slot-allocator home (flagged — adds a roadmap step; user veto point)

The stateful slot allocator is a NEW concern not covered by the current
phase-02 steps. It lives at the C3 `on_alloc_running` hook (assign-smallest-free)
/ `on_alloc_terminal` (release), a per-host free-list. It is NOT in 02-01 (pure
derivation), NOT in 02-02 (real `ip` execution of a GIVEN plan), and NOT in
02-03 (resolv.conf). A new step **02-04 "per-host NetSlot allocator + C3
lifecycle wiring"** is drafted in the roadmap (flagged for user veto — it widens
phase-02 scope). The allocator is single-node trivial; the #167 VIP allocator
stays deferred and is NOT pulled in.

## Reuse Analysis verdict

**1 CREATE-NEW** (`MtlsResolve` port — justified: no existing port returns
`orig_dst → {backend_addr, expected_svid}` filtered to `running`; it is the
#178 boundary). The Q5a name-layer integration adds **zero** new CREATE-NEW:
**resolv.conf injection** is an EXTEND of the Q2 netns provisioner (one
idempotent converge step), and the **DNS responder daemon (#61)** + **VIP
allocator (#167)** are named DEPENDENCIES, not builds here. Everything else is
**EXTEND** (`MtlsInterceptWorker`, `install_inbound_tproxy`+shared routing infra
→ `install_outbound_tproxy`, `accept_outbound_leg`/`getsockname_orig`,
`veth_provisioner` + resolv.conf injection, the `ExecDriver` setns hook, the
#234 shared infra) or **DELETE** (`cgroup_connect4_mtls` program,
`MTLS_REDIRECT_DEST`/`MtlsDataplane` outbound surface,
`program_declared_peer_redirect`). The `MtlsEnforcement` port is reused with
**no contract change**.

## Back-propagation (changed assumption)

`veth_provisioner.rs:36-37` ("single-node runs entirely in the host netns") and
ADR-0069's `cgroup/connect4`-rewrite OUTBOUND framing are superseded by Path A
(per-workload netns+veth; nft-TPROXY both directions). Amended via ADR-0071;
`jobs.yaml` re-grounding (if any) flagged for the product-owner, not edited by
the architect.

## Deferrals / blockers surfaced (no GH issues created)

- Egress nft-TPROXY Tier-3 validation (Q1) — RATIFIED: thin Tier-3 spike NOW
  (`increment-b/`) before DELIVER (D-TME-7). NOT a new issue.
- The #178 expected-SVID join, #61 name-layer **responder daemon**, #167 VIP
  allocator (NOT a v1 dependency under headless, D-TME-10), #234 Bar-2 reconciler
  are PRE-EXISTING named dependencies (cited, not created).
- No new GitHub issues created (per project rule — agents do not create issues
  without explicit user approval).

## DELIVER-handoff conditions — PINNED (design-review fold-in, 2026-06-16)

The DESIGN review (`nw-solution-architect-reviewer`, 2026-06-16) was
**non-blocking / APPROVED for DELIVER handoff**. Its *suggestion* section carried
crafter-handoff sharpenings to pin in the design (so crafters implement-to-design
and do not invent API surface — CLAUDE.md § "Implement to the design"). Three
were relayed and are now **pinned consistently across feature-delta + ADR-0071 +
brief.md §35** (and recorded here):

| Cond | What was pinned | Where |
|---|---|---|
| **C1** | `MtlsResolve.resolve` returns a **3-variant sum type** `MtlsResolution::{Mesh(ResolvedBackend), NonMesh, MeshUnreachable}` (NOT a binary `Option`), with per-arm enforce / pass-through / fail-closed rustdoc semantics. A binary `Option` cannot distinguish non-mesh pass-through from unreachable-mesh fail-closed; the type makes the Q3 "fail-closed not silent-cleartext" decision structural (CLAUDE.md § "sum types over sentinels"). | feature-delta § "`MtlsResolve` port contract" + Driven ports + DDD terms + component rows; ADR-0071 fact 4 + § "The new driven port" + Consequences; brief.md §35 prose + Q3 + C4 L2. |
| **C2** | `ResolvedBackend` bounded to **exactly `{ addr, expected_svid }`**; the v1 `ServiceBackendsResolve` adapter returns **`expected_svid: None`** (authn-only shell; the expected-SVID join is **#178** — filling it here = boundary divergence; consistent with Q4/D-TME-8). | feature-delta § "`MtlsResolve` port contract" (C2) + Driven ports; ADR-0071 § "The new driven port" (C2); brief.md §35 prose + C4 L2. |
| **C3** | Netns creation at the action-shim **`on_alloc_running`** hook, **BEFORE `MtlsInterceptWorker::start_alloc` and BEFORE `start_alloc`/`Driver::start`** — the netns+veth must exist before the `ExecDriver` `setns` seam (which ENTERS, never creates) spawns the workload into it. Teardown at `on_alloc_terminal`. Replaces the prior unspecified-owner / "lifecycle OPEN (Q2)" wording. | feature-delta § "Driving ports" + provisioner component row + Q2 ratified row; ADR-0071 fact 1 + Q2 ratified; brief.md §35 component row + Q2 sub-decision. |
| **C4** (added 2026-06-16; read-mechanism REVISED 2026-06-17; F4/relist-trigger REFINED 2026-06-17; F-A ownership-aware index + F-B `subscribe_all` single-cut reconciled 2026-06-17 — all post-DESIGN amendments) | `ServiceBackendsResolve` resolves `orig_dst` against an **in-RAM, address-keyed, ownership-aware reverse index** of the `running` `service_backends` set (`addr → {service → Backend}`, NOT a flat `addr → Backend` with global last-writer-wins — see "F-A" below), built via **List-then-Watch + relist-on-`Lagged`** over the `ObservationStore` — NOT a per-`ServiceId` point query. **REVISED 2026-06-17 (resolve-index-coherence research): the prior observe-only / "MUST NOT add a new trait method" constraint is REVERSED.** The mechanism now (1) ADDS a keyless List enumerate `all_service_backends_rows(&self) -> Result<Vec<ServiceBackendRow>, ObservationStoreError>` — symmetric with `alloc_status_rows()`/`node_health_rows()`, SHIPPED `25e7acf3`; (2) **Lists-at-probe** before the Earned-Trust gate opens (closes **#237** cold-start, SHIPPED `25e7acf3`); (3) uses a **single-owner drain** (dissolves the **F2** take/restore TOCTOU per `development.md` § "Check-and-act must be atomic", SHIPPED `25e7acf3`); (4) **relists on a `Lagged` loss signal** to close **F4** lag-drop. **F4 / relist-trigger REFINED 2026-06-17 (ratified — option 2, surface `Lagged`):** the prior wording "relists on `broadcast::RecvError::Lagged`" assumed the loss signal was reachable, but `subscribe_all()` returns the lossy `ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>` and BOTH store adapters strip `RecvError::Lagged` internally — so closing F4 requires a NEW lag-surfacing surface: **`subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError>`** delivering **`SubscriptionEvent::{Row(ObservationRow), Lagged { missed: u64 }}`** (a DOMAIN event; adapter maps `RecvError::Lagged(n) → missed`; the core trait never names a tokio type). The single-owner drain consumes `subscribe_all_events()`; on `Lagged { missed }` it re-Lists via `all_service_backends_rows()` and rebuilds/merges the index — closing F4 with a *completeness* guarantee. **F-B reconciliation (dated honest history, 2026-06-17):** this refinement was authored (commit `36652ace`) with the rationale that a **dedicated method** (not a shared-type change to `ObservationSubscription`) **bounds blast radius** — only `ServiceBackendsResolve` would consume it, the ~20 existing `subscribe_all()` consumers stay untouched. **The very next commit `36a79762` superseded that decision and is the SHIPPED, intended state: `subscribe_all` and the `ObservationSubscription` alias were DELETED single-cut and ALL ~20 consumers were migrated to `subscribe_all_events()`** (now the SOLE observation-subscription surface, yielding `SubscriptionEvent`). Keeping the lossy `subscribe_all()` beside the lag-aware surface would have been the deprecated-parallel-path anti-pattern the project forbids (`feedback_single_cut_greenfield_migrations` / `feedback_delete_dont_gate`). The "bounded blast radius / ~20 consumers untouched / not a shared-type change" framing was a point-in-time decision the single-cut overtook; it is preserved here only as history. There is **no remaining "migrate the other consumers" follow-up** — that work is DONE (`36a79762`), not deferred. **F-A (ratified 2026-06-17 — option (b)): the index is ownership-aware** — keyed `addr → {service → Backend}` so each contributing service's backend at an addr is tracked separately; a service's backend-set shrink evicts only THAT service's contribution; an addr stays resolvable as long as ANY service still claims a healthy backend there; classification is `any-healthy-at-addr` (deterministic, NOT last-writer-wins). This removes the unstated "one `(IP:port)` belongs to at most one service" cross-component invariant the flat index relied on (and the LWW healthy-disagreement determinism smell). v1 single-node is structurally addr-exclusive (per-addr service set size-1 today), so the ownership-aware shape is defensive against multi-node / future writers, NOT a behaviour change; it is adapter-internal — the public `MtlsResolve` contract + the `NonMesh`/`MeshUnreachable`/`Err` arms are UNCHANGED. Miss-classification scoping: a **miss = `NonMesh`** (cleartext pass-through, by design), NOT `MeshUnreachable`; the residual irreducible convergence window is covered by **(a) fail-toward-handshake** — the v1 SECURITY invariant *"a resolve miss must never silently emit cleartext to a should-be-mesh peer,"* whose code lands under **#236**. **#237 CLOSED by this revision**; residual → (a)/#236. **PUBLIC `MtlsResolve` API unchanged** (growth confined to the `ObservationStore` driven port). | feature-delta § "`MtlsResolve` port contract" (C4) + § "C4 — F-A: ownership-aware index" + § "C4 — F4 / relist-trigger refinement" (→ "F-B reconciliation") + D-TME-11 row; ADR-0071 § "The new driven port" (C4 + F-A ownership-aware index + F-B reconciliation + F4/relist-trigger refinement); this file (D-TME-11). Consistent with the shipped 01-01 port rustdoc (`crates/overdrive-core/src/traits/mtls_resolve.rs`) — ADDS the read mechanism, does NOT re-classify. Revision evidence: `docs/research/networking/transparent-mtls-resolve-index-coherence-research.md`; F4-trigger evidence: ground-truth `subscribe_all` lossy surface (`observation_backend.rs:506`, `redb_backend.rs:368-373`, `ObservationSubscription` at `observation_store.rs:1149`); F-B evidence: commit `36a79762` (delete + migrate). |

**Canonical names chosen for the 3-variant type (C1):** `MtlsResolution` with
variants `Mesh(ResolvedBackend)` (→ enforce) / `NonMesh` (→ pass-through) /
`MeshUnreachable` (→ fail-closed).

**Possible 4th condition — OUTSTANDING (NOT pinned in this fold-in).** The user
reported **four** conditions; only three (C1–C3) were relayed for pinning. The
DESIGN review's *suggestion 4* — pin the `increment-b/` Tier-3 spike acceptance
criteria (workload `connect()` redirects to leg-F; `getsockname` recovers
orig-dst; marked leg-B/leg-S dials NOT re-captured AND a workload cannot
self-exempt; basic round-trip without RST/corruption) — is the most likely 4th.
It was NOT part of this contract-pinning pass; it remains a pre-DELIVER spike
gate (Q1/D-TME-7), already enumerated in ADR-0071 § Enforcement → "Tier-3
obligations" and the review suggestion 4. **Flagged to the orchestrator** — not
invented here.

## Deliverables

- `docs/feature/transparent-mtls-enrollment/feature-delta.md` (Tier-1 `[REF]`).
- `docs/product/architecture/adr-0071-transparent-mtls-enrollment-path-a-….md` (amends ADR-0069).
- `docs/product/architecture/brief.md` § 35 (Application Architecture extension) + C4 L1/L2.
- This file.
