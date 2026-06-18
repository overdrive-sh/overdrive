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
| D-TME-9 | **Name-layer integration (Q5a)**: a node-local DNS responder is injected into the per-workload netns `resolv.conf` (Fly.io `fdaa::3` model); the responder *daemon* is #61 (separate build), only the injection + return-shape contract live here. **Responder ADDRESS pinned 2026-06-18 (02-04 C3-wiring gaps, G1): `responder_addr` = the per-netns gateway (`plan.host_addr = subnet.network()+1`)** — the Overdrive analogue of Fly's single `fdaa::3` (one well-known node-local address *per netns*, reachable by construction as the default route, zero new converge step, collision-free as the slot's own /30 host addr). #61's daemon answers on each host-veth gateway. See the G1 amendment under D-TME-12 for the full rationale + the rejected fixed-constant alternative. | The per-workload netns (Q2) IS the DNS injection point — one topology, two wins; sidecarless (Overdrive ships its own appliance OS, ADR-0068). |
| D-TME-10 | **DNS-return shape = HEADLESS for v1**: the responder returns a `running` backend addr from `service_backends` — that address IS the `orig_dst` `MtlsResolve.resolve` recognizes (one source, two readers, byte-consistent). **No #167 (VIP allocator) v1 dependency.** VIP is the multi-node evolution. | Headless keeps `MtlsResolve` v1 thin (identity-only, no LB — LB-pick is the #178-deferred policy), pulls no new v1 dependency, and is forward-compatible (VIP arm added alongside later, K8s ships both). VIP for v1 was REJECTED (would add #167 + the VIP×intercept ordering hazard). |
| D-TME-12 | **Per-allocation network SLOT model (resolves the 02-01 review B1+S1+S2, and B3 from the 02-01 re-review; refines D-TME-2/C3)**: the per-allocation netns name, the two veth-end iface names, AND the point-to-point /30 subnet are ALL derived from a single **host-unique bounded network slot** — a new `NetSlot` newtype (`overdrive-control-plane`), NOT a hash of the `AllocationId`. **A hash is WRONG by pigeonhole** (`AllocationId` is `LABEL_MAX`=253-bounded; a Linux iface name is `IFNAMSIZ`=15-usable-bounded; no pure function of a 253-char id can collision-free-map into a 15-char name — a hash makes collisions merely *unlikely*, the exact hand-wave CLAUDE.md § "One shared length ceiling for label-shaped ids" forbids). The slot makes collision-freedom **structural**: distinct slot ⇒ distinct names ⇒ distinct /30 (single-source). **Slot domain**: `NetSlot(u16)`, valid range `0..=4095` (4096 concurrent per-alloc netns slots — single-node bounded concurrency; ample, and 4 hex chars renders ≤15). **Rendering**: 4-char lowercase zero-padded hex (`{:04x}`), so `ovd-hv-<4hex>` / `ovd-wl-<4hex>` = 11 chars ≤ 15 IFNAMSIZ by construction; the 4-char ceiling is DERIVED from `IFNAMSIZ - PREFIX.len()` (15−7=8 budget; 4 used), not a magic number. **/30 derivation**: the slot indexes a /30 block inside a fixed per-host `WORKLOAD_SUBNET_BASE` (`10.99.0.0/16`): subnet = base + `slot * 4` as a /30 → `host_addr` = base+slot*4+1, `workload_addr` = +2, `gateway` = `host_addr`. 4096 /30s = 16384 addresses = a **/18** within the `/16` base (slots 0–4095 occupy `10.99.0.0`–`10.99.63.255`), leaving 3/4 of the `/16` unused; the slot ceiling is the 4-hex IFNAMSIZ budget (`< 0x1000`), NOT the `/16` size (the `/16` could carry up to 16383 /30s, so `NET_SLOT_MAX = 4095` is a deliberate conservative cap with ample headroom — single-node bounded concurrency). **netns name is ALSO slot-keyed: `ovd-ns-<4hex>`** (11 chars ≤ `NAME_MAX`=255 AND ≤ IFNAMSIZ, bounded by construction, identical to the two veth names). **All THREE derived names (netns + both veths) are uniformly slot-keyed** — the slot is the iface/subnet/netns axis; the alloc id is the human axis, held in the allocator's slot↔alloc map (02-04), NOT embedded in any kernel/filesystem name. `ip netns list` now shows `ovd-ns-<4hex>` (hex, like the veths); the human-readable alloc identity is rendered by tooling against the slot↔alloc map — the Cilium `lxc<hex>` + `cilium endpoint list` model. This is a deliberate, accepted ergonomics shift (B3 resolution, ratified option (a) 2026-06-17), not an oversight. **`derive_workload_netns_plan` is PURELY slot-derived** (`(slot, responder_addr) -> WorkloadNetnsPlan`); the `alloc_id` parameter is DROPPED — with the netns name slot-keyed, the alloc id no longer derives anything (netns + both veths + subnet are all slot-keyed; `responder_addr` is passthrough), so carrying it would be a speculative unused parameter (the alloc↔slot binding's correct home is the 02-04 allocator map). The subnet is also no longer a caller parameter (S1 resolved: the derivation owns slot→/30, the allocator owns slot assignment). The STATEFUL slot allocator (assign-smallest-free / release-on-teardown, a per-host free-list — NOT distributed IPAM, NOT the #167 VIP allocator) lives at the **C3 `on_alloc_running` lifecycle hook** (release at `on_alloc_terminal`), the same hook that owns netns creation and holds the `alloc_id`. S2 resolved: a /30 always has two usable hosts, so `workload_addr` is non-degenerate by construction (no `Option`, no `network()` fallback). | The 02-01 review (B1) ground-truthed that the literal `ovd-hv-<alloc>` overflows IFNAMSIZ for any alloc id ≥ 9 chars (the golden test's own `ovd-hv-payments-0` = 17 chars is uncreatable) and a naïve truncation collides two allocs onto one veth. B1+S1+S2 are one problem: the missing host-unique handle that both names AND the /30 must derive from. The 02-01 re-review (B3) found the FIRST cut of D-TME-12 left the netns name embedding the unbounded `AllocationId` (`ovd-ns-<alloc>`) with an arithmetically false "≤255" reassurance (7-char prefix + 253-char alloc id = 260 > 255 → `ENAMETOOLONG` from `ip netns add` for any alloc id ≥ 249 chars, reachable via a ~244-char workload name through `reconcilers/workload_lifecycle.rs:838`'s `alloc-{workload_id}-{attempt}` mint) — the IDENTICAL pigeonhole/ceiling defect class as B1, on the one derived name the first cut left out. Resolved by slot-keying the netns name too (option (a)), making the overflow unrepresentable by construction — the same lever the slot used to beat the hash for B1. No existing host-unique per-alloc integer exists (`alloc-{workload_id}-{attempt}` is workload-scoped, not host-unique; the cgroup scope keys on the full id string). The slot model makes collision-freedom by-construction and resolves all four findings in one coherent decision. (Ratified by the user 2026-06-17; resolves review `deliver/reviews/02-01.md` B1+S1+S2 and the re-review B3.) |
| D-TME-11 | **Resolve READ MECHANISM (C4; refines D-TME-6)**: `ServiceBackendsResolve` resolves `orig_dst` against an **in-RAM, address-keyed, ownership-aware reverse index** of the `running` `service_backends` set (`addr → {service → Backend}`, NOT a flat `addr → Backend` with global last-writer-wins — see F-A below) — NOT a per-`ServiceId` point query (the `ServiceId`-keyed `service_backends_rows` is the wrong surface; the adapter holds no `ServiceId`). **REVISED 2026-06-17 (resolve-index-coherence research):** built via **List-then-Watch + relist-on-`Lagged`** (the prior observe-only / "no new trait method" constraint is REVERSED). List leg = the keyless `all_service_backends_rows()` enumerate (SHIPPED `25e7acf3`); List-at-probe closes #237 cold-start; single-owner drain dissolves the F2 take/restore TOCTOU. **F4 / relist-trigger REFINED 2026-06-17 (ratified — option 2, surface `Lagged`):** the lossy `subscribe_all()` (item type `ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>`) could not carry the loss signal — both adapters stripped `RecvError::Lagged` internally — so closing F4 needed a lag-surfacing surface `subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError>` delivering `SubscriptionEvent::{Row, Lagged { missed: u64 }}` (a DOMAIN event; adapter maps `RecvError::Lagged(n) → missed`; no tokio leak); the single-owner drain consumes it and re-Lists on `Lagged`, closing F4 with a *completeness* guarantee. **F-B (reconciled 2026-06-17): `subscribe_all_events()` is now the SOLE observation-subscription surface — the lossy `subscribe_all()` + `ObservationSubscription` alias were DELETED single-cut in commit `36a79762` and every consumer migrated** (superseding the earlier "dedicated method bounds blast radius / ~20 consumers stay untouched / not a shared-type change" framing, which the single-cut overtook — that framing is preserved only as dated honest history in the C4 condition row below). **F-A (ratified 2026-06-17 — option (b)): ownership-aware index** — keyed per contributing service at an addr; a service's backend-set shrink evicts only THAT service's contribution; classification is `any-healthy-at-addr` (deterministic, NOT last-writer-wins). This removes the unstated "one `(IP:port)` belongs to at most one service" cross-component invariant and the LWW healthy-disagreement determinism smell; v1 single-node is structurally addr-exclusive (per-addr service set size-1 today), so the shape is defensive against multi-node / future writers, NOT a behaviour change. The structure is adapter-internal; the public `MtlsResolve` contract + the `NonMesh`/`MeshUnreachable`/`Err` arms are UNCHANGED. **A miss = `NonMesh`** (cleartext pass-through), NOT `MeshUnreachable`; the residual irreducible convergence window is the **(a) fail-toward-handshake** v1 SECURITY invariant, tracked in **#236**. **#237 CLOSED by this revision** (List-at-probe + relist). PUBLIC `MtlsResolve` API unchanged; growth confined to the `ObservationStore` driven port. | D-TME-6 pinned the resolve *model* but not the *read mechanism*; a DELIVER step surfaced that `resolve(orig_dst)` has no `ServiceId` and no addr→service surface exists. Cilium's `ipcache` (in-RAM addr→identity reverse index, subscribe-populated, List-before-Watch, relist-on-loss) is the canonical precedent; research §4.1 describes the resolve as an in-RAM `service_backends` lookup. Making a miss fail-closed would break legitimate `NonMesh` external egress — forbidden. (Ratified 2026-06-16; read-mechanism REVISED 2026-06-17; F4/relist-trigger REFINED 2026-06-17; F-A ownership-aware index + F-B `subscribe_all` single-cut reconciled 2026-06-17.) |

## D-TME-12 — pinned API contract for the crafter (02-01 re-implementation)

The 02-01 review (`deliver/reviews/02-01.md`) returned NEEDS_REVISION on two
blocking defects (B1, B2) + S1/S2/S3 + N1. D-TME-12 (above) is the coherent
resolution of B1+S1+S2; B2/S3/N1 are pinned here. The crafter builds ONLY the
surface named below (CLAUDE.md § "Implement to the design — never invent API
surface").

**Amended 2026-06-17 (B3 + S4 + N2; 02-01 re-review).** The re-review
(`deliver/reviews/02-01.md` § "Adversarial Re-Review") confirmed B1/B2/S1/S2/S3/N1
RESOLVED and found one new blocking defect **B3** (the netns name still embedded
the unbounded `AllocationId`) plus non-blocking S4/N2. B3 is resolved by
slot-keying the netns name too (option (a), ratified) — pinned below; the
`alloc_id` parameter is DROPPED from `derive_workload_netns_plan` (it no longer
derives anything). S4 (the `SetLoopbackUp` emit-condition) and N2 (the `/18` not
`/16` tiling) are corrected below. The remaining re-review items are code-side
fixes for the crafter (S5 stale module docstring, N3 `NetSlot::get()`, N4
"fourteen"→fifteen bools, N5 IFNAMSIZ `#[test]`→`const _` assert) — the architect
does NOT edit code; they are flagged for the next crafter dispatch.

**Amended 2026-06-18 (02-04 C3-wiring gaps).** Step 02-04 landed the PURE
`NetSlotAllocator` (smallest-free assign/release, idempotent re-assign,
`NetSlotExhausted`; committed `9f7d35ce` in `veth_provisioner.rs`), but its
**criterion 3 — the C3 lifecycle wiring that calls `provision_workload_netns`
from the alloc lifecycle — was BLOCKED** by three gaps the contract
under-specified. The crafter correctly STOPPED rather than invent API surface
(CLAUDE.md § "Implement to the design — never invent API surface"). Each gap is
pinned below, verified against the shipped code (not assumed). These three
amendments are what the **C3-wiring follow-up step** (a NEW roadmap step,
separate from the pure-allocator 02-04 already landed) builds to.

#### G1 (responder address) — `responder_addr` = the per-netns gateway (`plan.host_addr`)

`derive_workload_netns_plan(slot, responder_addr: Ipv4Addr)` requires a concrete
`Ipv4Addr`; the design pinned the *model* (a fixed node-local responder, à la
Fly's `fdaa::3`) but never an Overdrive IPv4 constant, and the only supplier of a
value today is a test fixture (`responder()`). feature-delta:594-597 places the
responder's *address* in-scope for THIS feature (only the *daemon* — #61 — is
out of scope). **Decision:** **`responder_addr == plan.host_addr == plan.gateway
== subnet.network()+1`** — the per-netns gateway (host-side veth end).

The C3 call site computes the gateway from the slot and passes it as
`responder_addr`. To keep the call site from re-deriving the `base + slot*4 + 1`
arithmetic that `derive_workload_netns_plan` already runs, add a thin pure helper
co-located in `veth_provisioner.rs` and call it at the C3 site:

```rust
// veth_provisioner.rs — pure, slot-derived, mirrors the plan's own gateway math.
// NOT new public *domain* surface — it exposes the same `base + slot*4 + 1` the
// plan already computes, as a named function so the call site stays a single line.
#[must_use]
pub fn responder_addr_for_slot(slot: NetSlot) -> Ipv4Addr;

// At the C3 provision call site (action-shim StartAllocation/RestartAllocation):
let slot = net_slot_allocator.assign(alloc_id.clone())?;          // G3 below
let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot));
debug_assert_eq!(plan.responder_addr, plan.host_addr); // gateway == responder
```

The helper is the pinned shape; if the crafter prefers to inline
`responder_addr_for_slot`'s body at the single call site that is acceptable (same
arithmetic, no new public surface) — but it must NOT invent any *other* responder
address. The value MUST equal `plan.host_addr`.

**Why the gateway, not a single fixed well-known constant.** This was a real
trade-off; the gateway wins on three counts and the literal-Fly fidelity loss is
cosmetic:

- **Reachability is by construction, with ZERO new converge step.** The gateway
  IS the in-netns default route (`plan.gateway = plan.host_addr`); a packet to it
  is reachable the moment the veth pair is up — which the existing 02-02 converge
  steps already guarantee. The fixed-constant alternative (e.g.
  `10.99.64.1` in the unallocated /18+ headroom, or a `169.254.x.x` link-local)
  would need an **ADDITIONAL idempotent per-netns route** to that address via the
  gateway — i.e. a new `WorkloadVethStep` variant + a new `ObservedWorkloadVeth`
  fact + the matching iff-emit proptest clause, widening 02-02's frozen converge
  surface. Simplest-solution-first (and minimal-converge) rejects that.
- **Collision-free by construction.** The gateway is the slot's own `/30` host
  address — it is already allocated to this netns and cannot collide with any
  other slot's `/30` (distinct slots ⇒ distinct `/30`s), nor with the spike's
  real-backend range (`10.200.0.0/8`-region, a different block entirely), nor with
  the `WORKLOAD_SUBNET_BASE` headroom. A hand-picked constant has to be audited
  against all three; the gateway needs no audit.
- **It IS the Overdrive analogue of `fdaa::3`.** Fly injects one fixed address
  because their host fabric is uniform. Overdrive's per-workload `/30` makes the
  *gateway* the single well-known node-local address *as seen from inside each
  netns* — every workload's `resolv.conf` points at "my gateway," which is the
  one host-side address that netns can always reach. The divergence from a single
  global constant is cosmetic: each netns still has exactly ONE responder address,
  and it is the most-reachable one.

**Cost, stated explicitly (the #61 binding implication):** the #61 responder
daemon must answer on each per-workload host-veth gateway address (or bind the
host-side wildcard and reply on whichever gateway the query arrived at) — it is
NOT a single global listen address. This is a #61-daemon concern, recorded here so
#61's build knows its listen surface; it does NOT change the wiring step. **No
route converge step is in scope for the C3-wiring step** (the gateway needs none).

**#61 is NOT a wiring blocker.** The provisioning + resolv.conf injection land
behind the existing mTLS composition gate (`mtls_worker.is_some()` — `Some` only
on the production mTLS boot, `run_server` lib.rs:1925-1933). The `resolv.conf`
write (`nameserver <gateway>`) is an idempotent converge step that does not
require a live daemon at the address; only **end-to-end DNS resolution** (a
workload's `getaddrinfo` getting an answer) is gated on #61 shipping. The wiring +
injection do not wait on #61 — they write a correct, reachable `nameserver`
line that #61 will answer once it lands. (#61 is the pre-existing, design-cited
name-layer responder daemon — D-TME-9; cited here consistent with that existing
scope, not newly introduced.)

#### G2 (provision seam) — provision at the TOP of each alloc arm, BEFORE `driver.start()`

C3's requirement (ADR-0071 fact 1: the netns+veth must exist before the workload
is spawned into it) is **provision BEFORE `MtlsInterceptWorker::start_alloc` AND
BEFORE `Driver::start`**. The prior C3 wording ("at the `on_alloc_running` hook")
is **WRONG and is corrected here.** Verified flow in BOTH alloc arms of
`dispatch_single` (`action_shim/mod.rs`):

- `Action::StartAllocation`: `driver.start()` :887 → `worker.start_alloc()` :980
  → `driver.on_alloc_running()` :1002.
- `Action::RestartAllocation`: `driver.stop()` :1027 → `driver.start()` :1045 →
  `worker.start_alloc()` :1133 → `driver.on_alloc_running()` :1152.

The `on_alloc_running` callback fires AFTER both `driver.start()` and
`start_alloc()` — so it is the WRONG seam for a "provision before
`Driver::start`" requirement; provisioning there would create the netns *after*
the workload was already spawned. **Corrected seam: provision at the TOP of each
arm**, before the `driver.start(&spec)` match (`StartAllocation` before :887;
`RestartAllocation` before :1045, i.e. after the stop-half :1027 but before the
start-half :1045). Provision MUST succeed (or fail-closed) before the driver
spawns the process.

**Teardown seam:** at the terminal arms, AFTER the driver stop, **tear down the
netns+veth, THEN release the slot** (release-after-teardown, so a crash between
the two leaves the slot HELD = the resource still exists and is reclaimable, never
a released-but-undestroyed leak). The two terminal arms:

- `Action::StopAllocation`: `driver.stop()` :1187 → `driver.on_alloc_terminal()`
  :1227 → `worker.stop_alloc()` :1231 → (NEW) teardown netns+veth → (NEW)
  `net_slot_allocator.release(&alloc_id)`.
- `FinalizeFailed` (the budget-exhausted terminal): `driver.on_alloc_terminal()`
  :851 → `worker.stop_alloc()` :856 → (NEW) teardown → (NEW) release.

Teardown is idempotent (converge-on-boot shape: tear down what exists, no-op what
does not), and `release()` is already idempotent (`BTreeMap::remove` of an absent
key — `veth_provisioner.rs:719-724`), so a double-terminal or a terminal for an
alloc that never provisioned is benign.

**RELATED — the `ExecDriver`→netns join is a SEPARATE tracked concern (disposition
iii), surfaced for user approval.** "Provision before `Driver::start`" only
achieves ADR-0071's goal if the driver actually spawns the workload INTO the
per-workload netns. Verified: the join *seam* EXISTS — `ExecDriver::with_netns_path(PathBuf)`
opens the netns as an `OwnedFd` and installs a `pre_exec` `setns(fd,
CLONE_NEWNET)` hook (`overdrive-worker/src/driver.rs:185-198, 317-318, 430-434,
486-494`), CNI-aligned (ENTERS, never creates). **BUT it is not wired per-alloc:**
(1) `with_netns_path` is a builder set ONCE at driver construction, and the
production composition (`compose_production_driver`, lib.rs:1333-1336) constructs
`ExecDriver::new(...)` with NO `.with_netns_path(...)` → `netns_path: None` → the
driver never enters any netns; (2) `AllocationSpec` (`overdrive-core` driver.rs:131-156)
carries NO netns field, so the slot-derived per-alloc netns name (known only at
the C3 provision site) has no channel to reach the per-alloc `driver.start(&spec)`.
Closing this needs EITHER an `AllocationSpec.netns: Option<String>` field threaded
from the C3 site through the driver, OR a per-alloc driver-targeting mechanism —
both OUTSIDE the C3-wiring step's netns-provisioning scope and touching
`overdrive-core` + `overdrive-worker`. **This is flagged to the orchestrator/user
as a candidate NEW GitHub issue** ("wire `ExecDriver` to spawn workloads into the
per-alloc netns: thread the slot-derived netns name through `AllocationSpec` to
the `with_netns_path`/`setns` seam"). The C3-wiring step provisions+injects+tears
down the netns and is independently valuable (the netns/veth/resolv.conf exist and
converge correctly) even before the join lands; without the join the workload runs
in the host netns and the mTLS-interception path is not yet end-to-end, which is
ALSO gated on #61 and the Tier-3 spike (D-TME-7). Do NOT create the issue without
user approval (CLAUDE.md § "Deferrals require GitHub issues — AND user approval
BEFORE creation").

#### G3 (allocator plumbing) — `NetSlotAllocator` on `AppState`, threaded as an explicit `dispatch_single` param

`dispatch_single` sources ports from `AppState`; the `NetSlotAllocator` must reach
the C3 call site. **Decision: mirror the `mtls_worker` / `IdentityMgr` shape
EXACTLY** — held-state on `AppState`, threaded to `dispatch`/`dispatch_single` as
a new explicit param (the established per-call port-passing pattern; bundling into
a struct is forbidden by `development.md` § "Port-trait dependencies", as the
existing `#[allow(clippy::too_many_arguments)]` rationale on `dispatch` states).

- **Held-state shape:** `NetSlotAllocator` is already `#[derive(Clone, Default)]`
  and holds `Arc<Mutex<BTreeMap<AllocationId, NetSlot>>>` INTERNALLY
  (`veth_provisioner.rs:652-658`) — it self-shares on clone, exactly like
  `IdentityMgr`'s `Arc<RwLock<BTreeMap<...>>>`. So the `AppState` field is a plain
  value: `pub net_slot_allocator: NetSlotAllocator` (no outer `Arc<Mutex<…>>`
  wrapper needed — its internal `Arc` IS the shared handle; contrast
  `PersistentServiceVipAllocator`, which is NOT internally-shared and so needs the
  outer `Arc<tokio::sync::Mutex<…>>`). It is NOT an `Option` — unlike
  `mtls_worker`, the allocator is harmless on the non-mTLS fixture surface (it
  just hands out slots nobody provisions), so a non-optional `Default`-constructed
  field keeps every fixture ripple-free.
- **Construction (ripple-free for the ~42 fixtures):** default-construct the field
  INSIDE the `AppState` constructors with `NetSlotAllocator::new()` — do NOT add it
  as a parameter to `AppState::new` / `new_with_workflow_engine`. This is the SAME
  ripple-avoidance `mtls_worker` (defaulted to `None` in `Self::new`) and
  `workflow_engine` (default empty-registry engine in `Self::new`) already use, so
  the ~42 non-mTLS fixtures and the `reconciler_runtime.rs`/`listener_facts.rs`
  callers (`AppState::new` at reconciler_runtime.rs:3212/3687) need NO change. The
  production boot composes/holds the same default (it carries no boot-time state —
  on a fresh process boot nothing is held; still-Running allocs re-assign on their
  next `on_alloc_running`, criterion 6), so the production `AppState` construction
  at lib.rs:1935 either inherits the default or sets the field explicitly post-construct.
- **Threading:** add `net_slot_allocator: &NetSlotAllocator` as a new explicit
  param to `dispatch(...)` (action_shim/mod.rs:474-489) and `dispatch_single(...)`
  (:682-697), passed at the loop call site (:493-508) and from
  `dispatch_with_workflow_intent` as `&state.net_slot_allocator` (alongside the
  existing `state.mtls_worker.as_ref()` at :665). Extend the existing
  `#[allow(clippy::too_many_arguments)]` rationale to name it.

**Resulting file boundary the C3-wiring follow-up step must be granted** (this is
the widened boundary G3 surfaces — beyond the prior 3-file netns boundary):

- `crates/overdrive-control-plane/src/action_shim/mod.rs` — the G2 provision/teardown
  seams in both alloc arms + both terminal arms; the G3 new param on
  `dispatch`/`dispatch_single`/their call sites; the G1 `responder_addr = gateway`
  at the provision call.
- `crates/overdrive-control-plane/src/lib.rs` — the G3 `AppState.net_slot_allocator`
  field + its default construction in `new_with_workflow_engine` (and the
  production `AppState` construction at :1935 if set explicitly).
- `crates/overdrive-control-plane/src/veth_provisioner.rs` — ONLY if the G1
  `responder_addr_for_slot(slot)` helper is added (optional; the arithmetic can
  live inline at the call site instead).

The `reconciler_runtime.rs` / `listener_facts.rs` / ~42-fixture callers are
DELIBERATELY out of the boundary by the default-construct-in-constructor choice
above — if a dispatch finds itself editing them, the plumbing approach has
drifted from this pin (they should compile untouched).

### New newtype — `NetSlot` (`overdrive-control-plane`, domain-bearing)

```rust
/// A host-unique, bounded per-allocation network slot. The single axis from
/// which a workload's netns NAME, both veth iface names, AND its
/// point-to-point /30 subnet derive — collision-free by construction (distinct
/// slot ⇒ distinct names ⇒ distinct subnet). Bounded to `0..=NET_SLOT_MAX` so
/// the rendered names fit both IFNAMSIZ (15) and NAME_MAX (255) (see
/// `derive_workload_netns_plan`). NOT a hash of the AllocationId (pigeonhole —
/// see D-TME-12).
pub struct NetSlot(u16);

/// Inclusive upper bound — 4096 concurrent per-alloc slots (single-node
/// bounded concurrency). 4 hex chars renders ≤ 15 IFNAMSIZ; the slot space
/// tiles a /18 within `WORKLOAD_SUBNET_BASE` (/16) — 4096 /30 blocks =
/// `10.99.0.0`–`10.99.63.255`. (The /16 could carry up to 16383 /30s, so 4096
/// is a deliberate conservative cap with ample headroom.)
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
/// Per-host base block all per-alloc /30s are carved from. The 4096-slot space
/// tiles a /18 within this /16 (4096 /30s = `10.99.0.0`–`10.99.63.255`),
/// leaving 3/4 of the /16 unused as headroom. NOT operator-tunable in v1
/// (single-node, fixed).
pub const WORKLOAD_SUBNET_BASE: Ipv4Net = /* 10.99.0.0/16 */;

#[must_use]
pub fn derive_workload_netns_plan(
    slot: NetSlot,
    responder_addr: Ipv4Addr,
) -> WorkloadNetnsPlan;
```

**`alloc_id` is DROPPED from the signature (B3 resolution).** With the netns
name slot-keyed (below), the alloc id no longer derives any plan field — all
three names + the subnet are slot-derived, and `responder_addr` is passthrough.
The plan is now PURELY slot-derived. Do NOT carry `alloc_id` as a speculative
field (consumer check: 02-02's executor uses `plan.netns`/`plan.*_veth`/addrs;
02-04's C3 hook holds the `alloc_id` in its own hand as the lifecycle subject
and owns the slot↔alloc map; 02-03's resolv.conf write keys on
`plan.netns`/`responder_addr` — no consumer needs `alloc_id` FROM the plan).
This applies the same "do not add speculatively" discipline the contract
already applied to the `slot` plan field.

Derivation rules (PURE, total — no `Option`, no `network()` fallback, because a
/30 always has two usable hosts):

- `netns`         = `format!("ovd-ns-{}", slot.to_hex4())` — 11 chars ≤ NAME_MAX (255) AND ≤ IFNAMSIZ (15), bounded by construction, identical shape to the veth names (B3: slot-keyed, NOT `ovd-ns-<alloc>` — the alloc id would overflow NAME_MAX at 260 chars for a 253-char alloc id, the same pigeonhole/ceiling class as B1).
- `host_veth`     = `format!("ovd-hv-{}", slot.to_hex4())` — 11 chars ≤ 15.
- `workload_veth` = `format!("ovd-wl-{}", slot.to_hex4())` — 11 chars ≤ 15.
- `subnet`        = the /30 at `WORKLOAD_SUBNET_BASE.network() + (slot.0 as u32 * 4)`, prefix-len 30.
- `host_addr`     = `subnet.network() + 1` (first usable host).
- `workload_addr` = `subnet.network() + 2` (second usable host).
- `gateway`       = `host_addr` (in-netns default route points back at the host-side end).
- `responder_addr` flows through verbatim (carried for D-TME-9 resolv.conf injection; NOT derived state).

`ip netns list` now shows `ovd-ns-<4hex>` (hex, like the veths); the
human-readable alloc identity is rendered by tooling against the 02-04 slot↔alloc
map (the Cilium `lxc<hex>` + `cilium endpoint list` model) — a deliberate,
accepted ergonomics shift, not an oversight. **B3 is the LAST derived-name axis:
after this, netns + both veths are all slot-bounded and the subnet/addresses are
slot-derived; no other unbounded-`AllocationId`-into-bounded-grammar mapping
remains in the D-TME-12 surface.**

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

Emit conditions (S4-corrected 2026-06-17 to match the shipped code, which is
MORE correct than the original pinning):
- `SetWorkloadVethUp` emits when (`pair_rebuilt` OR `!workload_veth_up`) — the
  in-netns end is a property of the veth pair, so a pair rebuild must re-up it.
- `SetLoopbackUp` emits when (`!netns_present` OR `!lo_up`) — **NOT**
  `pair_rebuilt`. `lo` is a property of the *netns*, not the *veth pair*: it
  survives a veth-only rebuild (netns present, pair recreated), so keying it on
  `pair_rebuilt` would re-emit a non-minimal `ip -n <ns> link set lo up` on an
  already-up `lo` (a corrupted-pair rebuild), violating criterion 5's "minimal"
  + "never re-touch a usable resource." (The first cut pinned both on
  `pair_rebuilt || fact-false`; the crafter correctly chose `!netns_present ||
  !lo_up` for `lo` because of its netns-scoped lifetime — this amendment makes
  the SSOT agree with that correct choice so the next reader does not "fix" the
  code back to the over-emitting form.)

The named proptest's iff-emit clauses extend to both new facts; the
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

**Amended 2026-06-18 (02-04 C3-wiring gaps).** The "lives at the C3
`on_alloc_running` hook" wording above is SUPERSEDED for the *assign* seam by the
"02-04 C3-wiring gaps" G2 amendment under D-TME-12 (above): **assign+provision
happen at the TOP of the `StartAllocation`/`RestartAllocation` arms, BEFORE
`driver.start()`** (NOT at the `on_alloc_running` callback, which fires after the
driver spawn and is the wrong seam for a "provision before `Driver::start`"
requirement). Release+teardown DO happen at the terminal arms
(`StopAllocation`/`FinalizeFailed`), after the driver stop, **teardown-then-release**.
Step 02-04 split in execution: the PURE `NetSlotAllocator` (assign/release logic)
landed (`9f7d35ce`); the C3 lifecycle WIRING (the seams + plumbing) is the
follow-up step pinned by the three G1/G2/G3 amendments under D-TME-12.

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
| **C3** | Netns creation at the action-shim alloc lifecycle, **BEFORE `MtlsInterceptWorker::start_alloc` and BEFORE `Driver::start`** — the netns+veth must exist before the `ExecDriver` `setns` seam (which ENTERS, never creates) spawns the workload into it. Teardown at the terminal arms (`StopAllocation`/`FinalizeFailed`), teardown-then-release. Replaces the prior unspecified-owner / "lifecycle OPEN (Q2)" wording. **SEAM CORRECTED 2026-06-18 (02-04 C3-wiring gaps, G2):** the original "at the `on_alloc_running` hook" naming was WRONG and is struck — that callback fires AFTER `driver.start()` (verified `action_shim/mod.rs` StartAllocation :1002 / RestartAllocation :1152), contradicting "BEFORE `Driver::start`". The provision seam is the **TOP of each `StartAllocation`/`RestartAllocation` arm, before `driver.start()`** (StartAllocation before :887; RestartAllocation before :1045). The "BEFORE `Driver::start`" ordering requirement was always correct and is authoritative; only the hook name was wrong. See the G2 amendment under D-TME-12 for the full pinned seams (provision + teardown) and the `ExecDriver`→netns-join separate-concern disposition. | feature-delta § "Driving ports" + provisioner component row + Q2 ratified row; ADR-0071 fact 1 + Q2 ratified; brief.md §35 component row + Q2 sub-decision. (Seam-naming correction propagates to those sites on next touch; the authoritative seam is the D-TME-12 G2 amendment.) |
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
