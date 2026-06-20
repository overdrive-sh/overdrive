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

**RELATED — the `ExecDriver`→netns join. DECISION CHANGED 2026-06-18: FOLDED INTO
the C3-wiring step, NOT deferred.** The prior cut of this block (disposition iii)
flagged the join as a SEPARATE tracked concern + a candidate NEW GitHub issue. The
user has decided the join is **in scope for the C3-wiring step (02-05)** — there
will be **no GitHub issue**. The full pinned shape (channel + type, injection
point, ExecDriver per-spec refactor, construction-site sweep, Tier-3 obligation)
is the **"Amended 2026-06-18 (join folded into C3-wiring step)"** block at the END
of this D-TME-12 amendment section (below the G3 boundary). The verified ground
truth that motivated the change is preserved here as honest history:

"Provision before `Driver::start`" only achieves ADR-0071's goal if the driver
actually spawns the workload INTO the per-workload netns. The join *seam* EXISTS —
`ExecDriver::with_netns_path(PathBuf)` opens the netns as an `OwnedFd` and installs
a `pre_exec` `setns(fd, CLONE_NEWNET)` hook (`overdrive-worker/src/driver.rs:185-198,
317-318, 430-434, 486-494`), CNI-aligned (ENTERS, never creates). BUT it is not
wired per-alloc: (1) `with_netns_path` is a builder set ONCE at driver
construction, and the production composition (`compose_production_driver`,
lib.rs:1333-1336) constructs `ExecDriver::new(...)` with NO `.with_netns_path(...)`
→ `netns_path: None` → the driver never enters any netns; (2) `AllocationSpec`
(`overdrive-core` driver.rs:131-156) carries NO netns field, so the slot-derived
per-alloc netns name (known only at the C3 provision site) has no channel to reach
the per-alloc `driver.start(&spec)`. **The folded-in block below closes both** —
it adds the channel (an `AllocationSpec` netns field), the injection (at the
action-shim C3 site), and the per-spec `ExecDriver::start` setns refactor. No GH
issue is created (the work is now in-scope, not a deferral). The end-to-end mTLS
interception path remains independently gated on #61 (DNS resolution) and the
Tier-3 egress spike (D-TME-7) — the join makes the workload LAND in its netns; it
does not by itself complete the interception datapath.

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
  on a fresh process boot nothing is held). **The "still-Running allocs re-assign on
  their next `on_alloc_running`, criterion 6" claim here is CORRECTED by the 02-06
  adopt-on-restart amendment below (§ "C6 was wrong: there is no re-assign
  trigger"): the reconciler does NOT re-drive a Running survivor, so the slot is
  rebuilt by a dedicated boot-time adopt pass (02-06), not by a re-assign on the
  lifecycle. The G3 plumbing here is unchanged either way (a default-constructed
  allocator that 02-06's boot pass populates by `adopt`).** So the production
  `AppState` construction at lib.rs:1935 either inherits the default or sets the
  field explicitly post-construct.
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

**Amended 2026-06-18 (join folded into C3-wiring step).** The user folded the
`ExecDriver`→per-alloc-netns join (flagged as "disposition iii / separate concern"
in the G2 RELATED block above) INTO the C3-wiring step — **no GitHub issue; it is
in scope now.** This makes the consolidated step (now **02-05**) the single
coherent unit: G1+G2+G3 wiring (above) PLUS the join pinned in the five JOIN
decisions below. The boundary widens to `overdrive-core` (the `AllocationSpec`
field) and `overdrive-worker` (the per-spec `ExecDriver` setns refactor) beyond the
G3 control-plane boundary. Each decision is pinned to an EXACT shape so the crafter
builds only the named surface (CLAUDE.md § "Implement to the design — never invent
API surface"); all five are verified against shipped code (re-confirmed at this
amendment, not assumed).

#### JOIN-1 (the channel) — a new `AllocationSpec.netns: Option<String>` field (NO newtype)

The slot-derived netns name reaches `driver.start` via a **new field on
`AllocationSpec`** (`overdrive-core/src/traits/driver.rs`, the struct at
:130-156). `AllocationSpec` derives ONLY `#[derive(Debug, Clone, PartialEq, Eq)]`
— NO serde, NO rkyv — and is recomputed each reconcile tick, crossing
reconciler→action-shim→driver purely in-memory (never persisted). Adding a field
is therefore a pure in-memory change: **NO rkyv schema-evolution discipline, NO
"persist derived state" violation** (the spec is never stored; the slot-derived
netns name injected at the shim is transient, recomputed-on-restart per criterion
6). Pinned exact shape:

```rust
// overdrive-core/src/traits/driver.rs — appended to AllocationSpec.
/// Target network namespace NAME this allocation's workload is spawned
/// INTO (the `ExecDriver` `setns(CLONE_NEWNET)` seam ENTERS it; it must
/// already exist — the action-shim C3 site provisions it before
/// `Driver::start`). `Some(plan.netns)` only when the C3 site provisioned
/// a per-workload netns (the production mTLS boot); `None` for every
/// non-netns workload (every current test fixture, and any boot where the
/// mTLS composition gate is off). The driver opens `/var/run/netns/<name>`
/// when `Some`; a `None` spec yields the pre-join host-netns behaviour.
///
/// `Option<String>`, NOT a `NetnsName` newtype: the value is already a
/// validated, bounded, slot-derived name (`ovd-ns-<4hex>`, 11 chars ≤
/// NAME_MAX) minted ONLY by `derive_workload_netns_plan` — it has no parse
/// surface, no operator-typed entry point, and no FromStr round-trip to
/// defend (see the JOIN-1 newtype rationale in wave-decisions.md D-TME-12).
pub netns: Option<String>,
```

**Newtype decision (CLAUDE.md § "Newtypes — STRICT by default") — explicit, not
dodged.** The rule's normal verdict for a domain-bearing identifier is "raw
`String`/`Option<String>` is a violation; introduce a newtype." Here the verdict is
**`Option<String>` is acceptable, NO `NetnsName` newtype**, for the rule's own
stated exception shape ("the newtype's job is validation + a canonical FromStr
round-trip; a value with no parse surface gains nothing"):

- The netns name is **already validated and bounded by construction** at its SOLE
  mint site (`derive_workload_netns_plan` → `format!("ovd-ns-{}", slot.to_hex4())`,
  bounded by the `NetSlot` newtype that DOES carry the discipline — `0..=NET_SLOT_MAX`,
  4-hex IFNAMSIZ ceiling). The string is a pure projection of an
  already-newtyped value; the discipline lives one layer up, on `NetSlot`.
- There is **no parse surface** — no operator ever types a netns name, no wire
  format carries one, no `FromStr` reconstructs one. A `NetnsName::from_str` would
  validate input that, by construction, only ever arrives pre-validated. The
  newtype would be ceremony with no invariant to enforce.
- **Cost of the newtype is real and one-directional:** `WorkloadNetnsPlan.netns`
  is currently `String` (`veth_provisioner.rs:462`); a `NetnsName` newtype would
  force changing that field AND every consumer of `plan.netns` (the 02-02 executor's
  `ip netns add <name>` / `ip -n <ns> link …` steps, the 02-03 resolv.conf write,
  the teardown) — a wide ripple for zero new safety, since the value is identical
  bytes either way. The `Option<String>` field threads the SAME `plan.netns` value
  with no conversion.

If a future need introduces a netns-name parse surface (an operator-supplied netns,
a wire-carried name), promoting to `NetnsName` is a localized follow-up — but it is
NOT justified now and inventing it would be speculative surface the design does not
need.

#### JOIN-2 (the injection point) — the action-shim C3 site sets `spec.netns` before `driver.start`

The reconciler stays **netns-agnostic**: both production `AllocationSpec` builders
(`overdrive-core/src/reconcilers/workload_lifecycle.rs:665` + `:750`, and the
`reconciler_runtime.rs:2924` spec-from-action helper) construct the field as
`netns: None`. The netns name is runtime slot state the pure reconciler must not
hold — the slot is assigned at the C3 site on the START lifecycle (a fresh alloc),
NOT carried in intent. (The earlier "re-assigned at the C3 site on each lifecycle
pass / criterion 6 rebuilt-on-restart" framing is CORRECTED by the 02-06
adopt-on-restart amendment below: a Running survivor is never re-driven through the
C3 site, so its slot is rebuilt by 02-06's dedicated boot adopt pass. The
netns-agnostic-reconciler decision here is unchanged regardless.)

**Only the action-shim C3 site injects.** At the TOP of each alloc arm (the G2
provision seam — `StartAllocation` before :887, `RestartAllocation` before :1045),
AFTER `derive_workload_netns_plan` yields `plan`, the local `spec` binding becomes
`mut spec` and the shim sets the netns name before the `driver.start(&spec)` match:

```rust
// action-shim StartAllocation / RestartAllocation arm, at the G2 provision site,
// before `driver.start(&spec)` (StartAllocation :887 / RestartAllocation :1045):
let slot = net_slot_allocator.assign(alloc_id.clone())?;              // G3
let plan = derive_workload_netns_plan(slot, responder_addr_for_slot(slot)); // G1
// … provision + resolv.conf-inject the netns (G2) …
spec.netns = Some(plan.netns.clone());   // JOIN-2: inject the slot-derived name
// … existing `match driver.start(&spec).await { … }` now spawns INTO the netns.
```

Verified: `spec` is a local binding at both `driver.start(&spec)` call sites
(`action_shim/mod.rs:887` / `:1045`); making it `mut spec` at the arm top and
setting `spec.netns` is a local, non-rippling change. The injection happens ONLY
on the netns-provisioning path (gated by the existing `mtls_worker.is_some()`
composition gate, per G1); a non-mTLS boot never reaches the injection and `spec.netns`
stays `None`.

#### JOIN-3 (the ExecDriver per-spec refactor) — `start` reads `spec.netns`; delete `with_netns_path`

`ExecDriver::start` (`overdrive-worker/src/driver.rs:450`) currently opens the netns
from the **construction-time** `self.netns_path: Option<PathBuf>` field (set once by
the `with_netns_path` builder; production never sets it). Refactor to **per-spec**:

```rust
// overdrive-worker/src/driver.rs — ExecDriver::start, replacing the
// `self.netns_path.as_ref()` open at :486-499 with a per-spec open.
let netns_fd = match spec.netns.as_deref() {
    None => None,
    Some(name) => {
        let path = std::path::Path::new("/var/run/netns").join(name);
        match tokio::fs::File::open(&path).await {
            Ok(f) => Some(std::os::fd::OwnedFd::from(f.into_std().await)),
            Err(source) => {
                let _ = self.cgroup_manager.remove_workload_scope(&scope).await;
                return Err(DriverError::NetnsEntry {
                    driver: DriverType::Exec,
                    netns_path: path.display().to_string(),
                    source,
                });
            }
        }
    }
};
```

Pinned specifics:

- **Path construction.** The spec carries the netns NAME (`ovd-ns-<4hex>`), not a
  path; `start` joins it onto the stock `/var/run/netns/<name>` location (where
  `ip netns add` places it — the 02-02 executor uses stock `ip netns add`). The
  driver still ENTERS, never creates (CNI-aligned).
- **Error variant — REUSE the existing `DriverError::NetnsEntry`, do NOT invent.**
  `DriverError::NetnsEntry { driver, netns_path, source }`
  (`overdrive-core/src/traits/driver.rs:97-107`) ALREADY exists and fits exactly:
  its rustdoc describes "configured with a target network namespace path … but the
  `pre_exec` hook could not enter it — either the path could not be opened … or
  `setns(CLONE_NEWNET)` failed." Both the missing/unopenable-path branch (above)
  and the in-`pre_exec` `setns` failure (the `build_command` closure at :430-438,
  which surfaces as an `io::Error` from `spawn()`) map to it. **No new variant.**
- **DELETE the now-dead `with_netns_path` builder + its tests (single-cut /
  deletion discipline).** Verified its ONLY callers are two test fixtures
  (`overdrive-worker/tests/integration/exec_driver/netns_entry.rs:134` + `:189`) —
  there is NO production caller (`compose_production_driver` never calls it). Once
  `start` reads `spec.netns`, `with_netns_path` and the `self.netns_path` field are
  dead production surface. Delete `with_netns_path`, the `netns_path:
  Option<PathBuf>` struct field (and its `None` init in `new()` at :270), AND
  rewrite the two `netns_entry.rs` fixtures to drive the netns via
  `spec.netns = Some(<name>)` instead of `.with_netns_path(<path>)` — same observable
  assertion (`/proc/<pid>/ns/net` symlink target; `DriverError::NetnsEntry` on a
  missing netns), new channel. Do NOT leave a stub, a re-export, or a
  `#[deprecated]` shim (CLAUDE.md § "Deletion discipline" / "single-cut greenfield
  migrations"). The two fixtures are REWRITTEN (the new channel tests the same
  behaviour through the production seam), NOT salvaged — the netns-entry behaviour
  is still genuinely under test, now via `spec.netns`.
- **Capability note (not a design blocker).** `setns(CLONE_NEWNET)` needs
  `CAP_SYS_ADMIN`; fine under `cargo xtask lima run` (root) and the production
  worker already runs privileged. No new privilege surface.

#### JOIN-4 (the construction-site sweep) — 2 production sites + ~31 fixture sites

Adding the `AllocationSpec.netns` field forces every construction site to add it.
**PRODUCTION sites (the real boundary the crafter owns — both set `netns: None`,
per JOIN-2):**

- `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs` — TWO literals
  (`:665` StartAllocation-spec, `:750` RestartAllocation-spec). Reconciler-emitted
  → `netns: None`.
- `crates/overdrive-control-plane/src/reconciler_runtime.rs:2924` — the spec-from-
  action builder fn. Reconciler-side → `netns: None`.

**TEST-FIXTURE sites (~31 files, mechanical `netns: None` additions, compile-driven):**
every other file in the `AllocationSpec { … }` construction set — the
`overdrive-worker` / `overdrive-control-plane` / `overdrive-sim` / `overdrive-core`
`tests/` + the `netns_entry.rs` fixtures (which ALSO migrate to `spec.netns =
Some(<name>)` per JOIN-3). **Two of these live IN `src/` but are `#[cfg(test)]`
helpers, NOT production sites** — the `sample_spec` fns in
`overdrive-worker/src/driver.rs:1230` (ExecDriver's `mod tests`) and
`overdrive-sim/src/adapters/driver.rs:492` (SimDriver's `mod tests`); treat them as
fixtures (`netns: None`), do NOT mistake their `src/` location for production. These
are not a design concern; the crafter adds `netns: None` (or `Some(<name>)` for the
netns_entry fixtures) as the compiler demands.

**Constructor/builder recommendation — NOT required.** A field add is simpler than
introducing an `AllocationSpec::new(...)` constructor or a builder; the ~31
fixture additions are a one-time mechanical sweep and a constructor would itself
ripple every site. Plain field add is the pinned shape. (If the crafter judges a
`#[non_exhaustive]` + constructor would localize FUTURE additions and wants to
propose it, that is a separate decision to surface — do NOT introduce it
unprompted under this pin.)

#### JOIN-5 (the Tier-3 obligation) — a real workload LANDS in its per-alloc netns

Acceptance: a real workload started through the full alloc lifecycle (Lima, root,
`--features integration-tests`) actually LANDS in its per-alloc netns — asserted on
the **observable kernel/ns side effect**, never on program-internal reachability
(`.claude/rules/testing.md` § "Assertion rules"). Either observable proof suffices:

- `ip netns identify <workload_pid>` == `ovd-ns-<slot>` (the workload's
  `/proc/<pid>/ns/net` resolves to the provisioned netns's inode, NOT the host's), OR
- the workload's traffic egressing through the per-alloc veth (e.g. a raw-IP
  `connect()` from inside the workload reaching the host-side veth gateway).

**DNS resolution is NOT part of this proof** (gated on #61) — use a raw-IP connect
or `ip netns identify`, never a `getaddrinfo`/DNS round-trip. Record `uname -r` in
the test (the verdict is kernel-pinned per `.claude/rules/spike.md` / ADR-0068).
This is the end-to-end proof the join works; it lives in the consolidated step's
Tier-3 test file(s) under `overdrive-control-plane/tests/integration/` (the
lifecycle-level seam) — gated behind `integration-tests`, run via `cargo xtask
lima run --`.

**Amended 2026-06-18 (02-06 adopt-on-restart — cross-restart slot rebuild).**
The five JOIN decisions above (02-05) wire a workload INTO its per-alloc netns on
the normal alloc lifecycle. They are silent on what happens on a **`serve`
restart**, where the in-RAM `NetSlotAllocator` map (G3) is lost but the workloads
SURVIVE. This block designs the **adopt-on-restart** step (02-06) that closes that
hazard. It is PURELY ADDITIVE on the 02-05 frozen shape (a new allocator method, a
new observe surface, a new boot-recovery pass) — it requires NO change to any
G1/G2/G3 or JOIN-1..JOIN-5 surface (`AllocationSpec.netns`, the
`AppState.net_slot_allocator` field, the `dispatch`/`dispatch_single` param, the
provision/teardown seams, the `ExecDriver` per-spec `setns`). The C6 "rebuilt on
restart by re-assigning for every still-Running alloc" wording in the `NetSlotAllocator`
rustdoc (`veth_provisioner.rs:636-639`) and elsewhere is **CORRECTED by this block**
— see § "C6 was wrong: there is no re-assign trigger" below; the corrected model is
a dedicated boot pass, NOT a reconciler re-drive. **The runtime survival/adopt
semantics are Tier-3-spike-gated; this block pins what is settled and marks what the
spike must settle (§ "Spike boundary").**

The same boot pass also reconciles a SECOND surviving-resource class the netns decisions
are silent on — the per-workload nft-TPROXY rules in the node-global shared `overdrive-mtls`
chain, whose in-RAM RAII guards are lost on restart (the nft-rule twin of the netns-slot
survivor). That is § 5 below (folding 03-01 adversarial-review finding D2, commit `c1d5f9d`);
it too is additive and re-uses the landed by-handle delete + dump-parse predicates, inventing
no new surface.

#### The hazard (verified ground truth)

On a `serve` restart the in-RAM `NetSlotAllocator` (`Arc<Mutex<BTreeMap<AllocationId,
NetSlot>>>`, `veth_provisioner.rs:652-658`) is reconstructed empty
(`NetSlotAllocator::new()`). But:

- **Workloads SURVIVE the restart.** `ExecDriver` spawns with `setsid()`
  (`driver.rs:413-414`), `kill_on_drop(false)` (`driver.rs:395`), and lives in its
  own cgroup scope (`overdrive.slice/workloads.slice/<alloc>.scope`) — detached from
  the CP process lifetime. A surviving workload keeps running in its old
  `ovd-ns-<slot>` netns. (Survival on a real `serve` restart is SPIKE-A below; the
  `setsid`+`kill_on_drop(false)`+cgroup-detach mechanism strongly implies it but is
  not yet ground-truthed against a real CP restart.)
- ⇒ A naive empty-allocator restart hands out **smallest-free from 0** to the next
  NEW alloc → `assign` returns slot 0 → `derive_workload_netns_plan(0, …)` →
  `provision_workload_netns` for `ovd-ns-0000` while a SURVIVING pre-restart alloc
  still occupies `ovd-ns-0000` (it had slot 0). That is the **B1 collision
  resurrected across restart** — two live allocs on one netns/veth/`/30`. Plus an
  **orphan-netns leak**: a pre-restart `ovd-ns-<slot>` whose workload DID die during
  the restart window is never torn down (its terminal arm never ran).
- **B3 complication:** the netns name is slot-keyed (`ovd-ns-<4hex>`), carrying NO
  alloc identity (deliberate — the Cilium `lxc<hex>` model, D-TME-12). So the netns
  NAME alone cannot tell you which alloc owns `ovd-ns-0005` after a restart. The
  slot↔alloc binding must be RECOVERED by correlating each surviving alloc's PIDs
  (read from its cgroup `cgroup.procs`) to that PID's netns
  (`/proc/<pid>/ns/net` inode → match against `/var/run/netns/ovd-ns-<slot>` inode,
  the `ip netns identify` mechanism).

#### C6 was wrong: there is no re-assign trigger (the premise gap, resolved)

The C6 model ("the held set is rebuilt on restart by re-assigning for every
still-Running alloc on its next `on_alloc_running`") assumed a rebuild trigger that
**does not fire**. Verified in `reconcilers/workload_lifecycle.rs:614-769`: the
`WorkloadLifecycle` reconciler emits `Action::StartAllocation` ONLY in the "**No
Running, no failed-needs-restart → schedule a fresh allocation**" branch (:708-765).
An alloc whose observation row is already `Running` (which a survivor's row IS,
post-restart) takes the Running branch and emits **no Start action** — so the C3
assign+provision seam at the action-shim alloc arms (G2) **never fires for a surviving
alloc**. The slot is never re-assigned by the normal path. C6's "re-assign on next
`on_alloc_running`" is therefore **false** — and unlike `IdentityMgr` (which CAN rely
on reconciler re-drive because a `¬held` SVID is harmless to re-issue and the
`SvidLifecycle` reconciler DOES re-issue on `¬held`), the netns case has BOTH (a) no
re-drive trigger AND (b) a SURVIVING resource that must NOT be re-created from slot 0.
**Conclusion: adopt-on-restart MUST be a dedicated boot-time recovery pass, not a
reconciler re-drive.** This is the per-alloc-netns analogue of #197's observed-state
hydration, but its own step (#197 is the host-pair network reconciler — see § "#197
relation" below).

#### 1. Observe-actual surface — `adopt_observe` (slot↔alloc recovery)

A new boot-time observe function reconstructs the surviving slot↔alloc bindings by
correlating three already-available facts. It lives in `veth_provisioner.rs`
(co-located with the slot derivation + the existing `ip netns list` parser
`netns_exists` at :1870, and the `provision`/`teardown` it mirrors). PINNED shape:

```rust
// veth_provisioner.rs — boot-time observe of surviving per-alloc netns bindings.
// Pure-ish thin observer (real `ip netns list` + procfs reads, no decision logic);
// the decision logic (adopt-vs-GC) is the pure `adopt_plan` below, default-lane
// unit + mutation testable.

/// One surviving netns observed at boot: its slot, and the alloc that owns it
/// (recovered via PID→netns correlation) if any live PID claims it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedAdoptNetns {
    pub slot: NetSlot,
    /// `Some(alloc)` when a live PID inside `<alloc>.scope` resolves
    /// (`/proc/<pid>/ns/net`) to this netns's inode; `None` = orphan (no live
    /// owner → GC candidate).
    pub owner: Option<AllocationId>,
}
```

The observe walks:
1. **Enumerate `ovd-ns-*` netns.** `ip netns list`, filter to the
   `WORKLOAD_NETNS_PREFIX` (`ovd-ns-`), parse the 4-hex slot back to a `NetSlot`
   (reuse the `netns_exists` line-parser shape at :1879-1881). This is the SET of
   provisioned per-alloc netns surviving the restart.
2. **Read the Running alloc set from the ObservationStore.** `obs.alloc_status_rows()`
   (exists — `observation_store.rs:1332`) filtered to `state == Running`
   (`AllocStatusRow` carries `alloc_id` + `state`, NOT the slot — `:652-656`). This
   is the SET of allocs the CP believes are Running.
3. **Correlate PID→netns per Running alloc.** For each Running `alloc`, read its
   surviving PIDs from `CgroupPath::for_alloc(alloc).resolve(root).join("cgroup.procs")`
   (no new cgroup-manager surface needed — the path is `CgroupPath::for_alloc`,
   `cgroup_manager.rs:68`; reading `cgroup.procs` is a plain procfs read). For each
   PID, resolve `/proc/<pid>/ns/net` to its netns inode (the proven in-tree mechanism
   `read_proc_netns_inode`, `overdrive-worker/tests/integration/exec_driver/netns_entry.rs:96-102`
   — promote a production copy into `veth_provisioner.rs`, do NOT depend on the test
   module). Match that inode against each enumerated `ovd-ns-<slot>`'s inode
   (`/var/run/netns/ovd-ns-<slot>` is itself a namespace handle whose inode is read
   the same way) → the `(slot, owner=alloc)` binding.

The output is `Vec<ObservedAdoptNetns>`: each enumerated netns tagged with its
recovered owner (or `None` = orphan). **CGROUP ENUMERATION IS NOT NEEDED** — we do
NOT list all alloc scopes (no such surface exists, and we don't add one); we drive
the correlation from the ObservationStore Running set, reading each known alloc's
`cgroup.procs` by its derived path. This keeps 02-06 free of any new
`cgroup_manager` surface.

**Spike-gated:** steps 1+3's runtime reliability (does `ip netns list` survive a CP
restart with all the per-alloc netns intact? does the PID→netns inode match recover
the slot reliably for a SURVIVING workload, not just a freshly-spawned one?) is
SPIKE-A/C below. The SHAPE above is pinned; the runtime fidelity is the spike's job.

#### 2. Allocator adopt method — `NetSlotAllocator::adopt` (ADDITIVE, atomic)

An ADDITIVE method on the LANDED `NetSlotAllocator` (02-04, `9f7d35ce`) that claims a
SPECIFIC `(alloc, slot)` binding — NOT smallest-free. PINNED signature:

```rust
// veth_provisioner.rs — ADDITIVE to the landed NetSlotAllocator impl. Does NOT
// touch assign/release/snapshot (the 02-04/02-05 frozen surface).

/// Claim the SPECIFIC `(alloc, slot)` binding observed surviving a restart
/// (adopt-on-restart, 02-06) — the inverse of [`assign`](Self::assign)'s
/// smallest-free pick. Used ONLY by the boot recovery pass to rebuild the
/// held map from the recovered slot↔alloc correlation BEFORE any
/// smallest-free `assign` can run, so a subsequent `assign` cannot hand a
/// surviving slot to a new alloc (the cross-restart B1 collision).
///
/// **Atomic check-and-act (`development.md` § "Check-and-act must be atomic"):**
/// ONE locked critical section checks whether `slot` is already held by a
/// DIFFERENT alloc and, only if free (or already held by THIS alloc — idempotent
/// re-adopt), inserts the binding. The conflict verdict is the insert's own
/// outcome, never a separate pre-check.
///
/// # Errors
///
/// Returns [`NetSlotAdoptConflict`] when `slot` is already held by a DIFFERENT
/// alloc — the boot pass treats this as a fatal correlation bug (two survivors
/// claiming one slot is impossible by construction; distinct slots ⇒ distinct
/// netns) and refuses to boot rather than silently overwrite. Re-adopting the
/// SAME `(alloc, slot)` is an idempotent no-op success.
pub fn adopt(&self, alloc: AllocationId, slot: NetSlot) -> Result<(), NetSlotAdoptConflict>;
```

with the companion error:

```rust
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("net slot {slot} already held by {held_by}, cannot adopt for {requested_by}")]
pub struct NetSlotAdoptConflict {
    pub slot: NetSlot,
    pub held_by: AllocationId,
    pub requested_by: AllocationId,
}
```

ADDITIVE confirmation: `assign` / `release` / `snapshot` are UNCHANGED; `adopt`
shares the same `Arc<Mutex<BTreeMap<AllocationId, NetSlot>>>` held map and the same
one-locked-critical-section discipline. The held-by-different-alloc check IS the
atomic verdict (scan the map's values for `slot` while holding the guard; insert iff
free-or-self). PINNED as a single locked op — no contains-then-insert TOCTOU.

#### 3. Boot sequence — a recovery pass in `serve`, adopt-then-GC, BEFORE serving

WHO drives it: a **boot-time recovery pass in `run_server_with_obs_and_driver`**
(mirroring where the host-netns `veth_provisioner::provision` already sits in the boot
order, `lib.rs:1507`), NOT a reconciler observe. Rationale: (a) the C6 finding above —
there is no reconciler re-drive trigger for a Running survivor; (b) the pass must
complete BEFORE the convergence loop or the action-shim can hand out a smallest-free
slot, which is a boot-ordering guarantee a once-per-boot pass gives and a steady-state
reconciler tick does not; (c) it mirrors `IdentityMgr`'s "fresh boot" handling site
(boot composition) even though the netns MECHANISM differs (adopt-survivor, not
re-drive). PINNED order (additive, sits after `AppState` construction at `lib.rs:1935`
where `net_slot_allocator` is available, and BEFORE the convergence loop / exit-observer
spawn at :1956+, gated by the same `mtls_worker.is_some()` composition gate G1 uses —
the recovery pass is a no-op on a non-mTLS boot where no per-alloc netns exist):

1. **`adopt_observe`** → `Vec<ObservedAdoptNetns>` (§1).
2. **Adopt every owned binding** — for each `ObservedAdoptNetns { slot, owner: Some(alloc) }`,
   `net_slot_allocator.adopt(alloc, slot)?`. This rebuilds the held map so the very
   next smallest-free `assign` cannot collide with a survivor. A `NetSlotAdoptConflict`
   refuses the boot (`health.startup.refused`, reason `netns.adopt`) — two survivors on
   one slot is a correlation bug, never silently resolved.
3. **GC every orphan** — for each `ObservedAdoptNetns { slot, owner: None }`, derive the
   plan (`derive_workload_netns_plan(slot, responder_addr_for_slot(slot))`) and
   `teardown_workload_netns(&plan)` (exists, idempotent, slot-derived — `:1671`). The
   orphan's slot becomes free for a future `assign`. Teardown-not-release: there is no
   held binding to release (orphan = no owner), so GC is teardown-only.
4. **THEN serve** — the convergence loop / action-shim start. By construction every
   surviving slot is held (adopted) and every orphan is reaped, so the first
   smallest-free `assign` picks a genuinely-free slot.

Adopt-BEFORE-GC ordering is load-bearing: adopt first so the held map reflects every
survivor before any free-slot scan; GC second so orphan slots return to the free pool.
Order 2→3 within the pass; both strictly before serving (4).

**Spike-gated:** SPIKE-B (what does `serve` boot do TODAY with already-Running allocs —
does anything re-adopt / re-drive / nothing?) confirms the pass has no pre-existing
behaviour to conflict with. The pass SHAPE + ordering is pinned; its interaction with
whatever boot already does is the spike's to confirm before the rest is locked.

#### 4. IdentityMgr coupling

`IdentityMgr` rebuilds its own held state on restart by the **opposite** mechanism: it
boots EMPTY (`IdentityMgr::new`, `identity_mgr.rs:66-74`) and relies on the
`SvidLifecycle` reconciler re-issuing for every still-Running alloc that reads as
`¬held` (`identity_mgr.rs:9`, `:69`). **02-06 does NOT mirror that** — and the contrast
is the whole point: an SVID is cheap and safe to re-mint on `¬held` (a fresh cert, no
surviving kernel resource), so reconciler re-drive works; a netns is a SURVIVING kernel
resource that must NOT be re-created from slot 0, and (per the C6 finding) the reconciler
does not even re-drive a Running survivor. So 02-06 adopts the survivor rather than
re-creating it. The two restart models are deliberately different and both correct for
their resource. **#26 coupling:** whether the workload's mTLS kernel material (kTLS /
bpffs-pinned, the kernel-mediated identity) survives a CP restart is the #26-coupled,
Tier-3-spike question flagged in CLAUDE.md § "Workload identity model"; 02-06's adopt
recovers the NETNS/SLOT binding only — it does NOT claim to recover or re-supply the
mTLS crypto, which is #26's concern. 02-06's scope is the network-slot rebuild; the
identity-material survival is out of scope and explicitly NOT assumed.

#### 5. Surviving per-workload nft-TPROXY rules — the nft-rule twin of the survivor problem

§1–§3 reconcile the surviving per-alloc **netns/veth/slot** resources across a `serve`
restart. There is a SECOND class of surviving per-workload kernel resource the
five-decision 02-05 shape and §1–§4 are SILENT on: the **per-workload nft-TPROXY
rules** in the node-global shared `overdrive-mtls` `prerouting` chain. This sub-section
folds in the 03-01 adversarial-review finding D2
(`docs/feature/transparent-mtls-enrollment/deliver/reviews/03-01.md` § "issue
(blocking): D2", commit `c1d5f9d`) — the nft-rule analogue of the netns-slot survivor
problem §1–§3 solve.

**The hazard (verified ground truth).** Structurally identical to §1–§3's
surviving-resource-plus-lost-in-RAM-handle shape:

- **The rules SURVIVE the restart.** Each per-workload TPROXY rule (egress via
  `install_outbound_tproxy`, inbound via `install_inbound_tproxy`) is APPENDED to the
  SHARED `overdrive-mtls` `prerouting` chain, which is converge-on-boot infra ensured
  idempotently (`ensure_shared_routing_infra`, `mtls_intercept_worker.rs:402-469`) and
  **NEVER torn down per-workload** (`NFT_TABLE` rustdoc `:47-56`: *"ensured idempotently
  … NEVER torn down per-workload"*). So on a `serve` restart these rules remain in the
  kernel — exactly like the surviving netns the §1–§3 pass adopts.
- **The in-RAM handle is LOST.** Each rule's lifetime is owned by an in-RAM RAII
  `TproxyInterceptGuard` whose `Drop` deletes the rule by its kernel-assigned handle
  (`:679-695`). On a CP restart the worker's per-alloc state is reconstructed empty and
  every guard is dropped without its `Drop` ever running against the kernel (the process
  died), so the in-RAM handle map is GONE — the precise analogue of the empty
  `NetSlotAllocator` map (§The hazard).
- **The re-bound legs choose NEW ephemeral ports.** Leg-C (inbound) and leg-F (outbound)
  are `IP_TRANSPARENT`/plain listeners bound to `127.0.0.1:0` — **kernel-chosen ephemeral
  per bind** (verified: `mtls_intercept_worker.rs:361` binds `"127.0.0.1:0"` and reads the
  assigned port back at `:369`; the `install_outbound_tproxy` caller-contract rustdoc
  `:321-332` states leg-F is *"a worker-chosen ephemeral port … NOT node-stable across
  re-binds"*). So after a restart the re-bound leg ports are **NOT** the surviving rules'
  redirect targets.
- ⇒ **Egress: stale-survivor + duplicate-on-re-install.** The egress install is
  idempotent keyed on `(host_veth, agent_leg_f_port)` — BOTH the `iifname` match AND the
  `tproxy to 127.0.0.1:<port>` redirect (verified: `dump_has_egress_rule` `:661-665`,
  `find_egress_rule_handle_in_dump` `:638-648`, both conjoin iifname AND redirect-port).
  A re-install for a SURVIVING `host_veth` with a CHANGED leg-F port does NOT match the
  old `(veth, oldPort)` rule → the presence-check reads **absent** → it **APPENDS A
  SECOND egress rule**. Two TPROXY rules then fire for one workload's veth; the first
  redirects to a now-dead leg-F listener (chain-order dependent — the kernel evaluates the
  surviving rule first if it sits earlier). This is finding D2's *"two TPROXY rules fire
  for one workload's veth"* shape, reachable at the re-install path (04-03 `start_alloc` +
  this 02-06 boot pass), NOT in 03-01's isolation.
- ⇒ **Inbound twin: stale-survivor (distinct shape).** `install_inbound_tproxy(virt,
  agent_port)` appends UNCONDITIONALLY (no per-rule presence-check — it relies on distinct
  `virt`s producing distinct rule text; `:239-277`), and its handle recovery keys on
  `ip daddr <vip>` + `tcp dport <vport>` + the leg-C redirect port (`find_virt_rule_handle`
  `:591-613`). On a restart it ALSO leaves a stale survivor (lost guard) and, when re-run
  for the same `virt` with a changed leg-C port, appends a fresh rule alongside the
  survivor. **Caveat (verified):** the inbound *production* install is currently
  **#178-DEFERRED** — at 04-01 `start_alloc` records `tproxy_guard = None` and installs NO
  production inbound rule (`mtls_intercept_worker.rs:391-417`), so the inbound survivor is
  not reachable until #178 lands the production virt source. But it shares the survivor
  CLASS exactly, so the reconcile this sub-section pins must cover BOTH directions to be
  forward-correct.

This is the per-workload-nft-rule analogue of §The hazard's per-alloc-netns survivor:
**a surviving node-global-chain resource whose in-RAM owner-handle is lost on restart**,
which a naive empty-state re-install duplicates (egress) or strands (both). It is in-scope
for the SAME boot-recovery pass that already adopts surviving netns and GCs orphans (§3) —
reconciling the surviving per-workload nft rules is the natural extension of that pass.

**The pinned reconcile (option (i) — adopt-pass tears down surviving per-workload nft
rules; the rule analogue of orphan-GC).** The §3 recovery pass, BEFORE serving, sweeps the
shared `overdrive-mtls` `prerouting` chain and **removes every per-workload TPROXY rule**
(every `iifname`-matched egress rule and every `ip daddr`/`tcp dport`-matched inbound rule),
leaving the shared infra (the F5 `meta mark <MTLS_LEG_S_DIAL_MARK> accept` exemption at the
chain head, the `ip rule fwmark`, the `ip route local … table`, the table+chain themselves)
UNTOUCHED. A `nft -a list chain ip overdrive-mtls prerouting` dump enumerates the survivors;
each per-workload rule is deleted by its handle (the same by-handle `nft delete rule …
handle <N>` the guard's `Drop` uses, `:685-695`); the shared infra is recognised-and-kept by
the existing predicates (`dump_has_leg_s_exemption` `:552-557` distinguishes the exemption
from a per-workload rule). The subsequent 04-03 `start_alloc` re-install for each
still-Running alloc then runs against a CLEAN chain — its append is unconditionally correct
and its returned guard owns the fresh handle. This is **symmetric with the netns orphan-GC
already in §3** (teardown-the-survivor, re-create-clean) and re-uses the existing by-handle
delete + dump-parse predicates verbatim — no new public surface, no new keying model.

Ordering within the §3 pass: the nft-rule sweep is a THIRD recovery action, ordered with
the netns adopt/GC. Because the nft sweep tears down ALL per-workload rules (it does not
adopt — there is no in-RAM guard to rebuild, and a guard cannot be reconstructed from a bare
kernel handle without the alloc binding it would need to outlive the re-install anyway), it
is independent of the adopt-vs-GC netns decision and may run in the same pass before serving.
PINNED order: **(netns) adopt-then-GC, (nft) sweep-all-per-workload-rules, THEN serve** — by
construction, after the pass every surviving slot is held, every orphan netns is reaped, and
the shared chain carries ONLY the shared infra, so the first 04-03 re-install appends exactly
one clean rule per direction per alloc.

**Rejected alternatives** (house style — record why not):

- **(ii) Pin a stable-per-veth (slot-keyed) leg-C/leg-F port** so a re-install matches the
  survivor and is genuinely idempotent. REJECTED for 02-06: it changes the ephemeral-port
  model the worker deliberately adopted (`127.0.0.1:0` kernel-chosen, `:361`; the
  `install_outbound_tproxy` rustdoc `:321-332` documents the ephemeral choice as
  intentional), is a larger cross-cutting change touching `mtls_intercept_worker.rs`'s leg
  bind + the per-alloc bookkeeping, and would have to derive a collision-free stable port
  from the `NetSlot` (a second slot-keyed derivation axis, re-opening the IFNAMSIZ/bound
  analysis D-TME-12 settled for names/subnets). The survivor-teardown in (i) achieves
  forward-correctness with NO change to the port model and re-uses the landed by-handle
  delete. (ii) may still be the right LONG-TERM shape if leg-port stability is wanted for
  reasons beyond restart (e.g. observability), but it is NOT needed to close this hazard and
  is out of 02-06 scope.
- **(iii) Re-key teardown/recovery so a changed port REPLACES rather than stacks** (adopt the
  surviving rule's handle by `iifname`-only / `daddr/dport`-only match, ignoring the port,
  then delete-or-refresh). REJECTED as strictly more complex than (i) for no benefit here:
  it requires a SECOND, port-blind variant of every dump-parse predicate (the landed ones
  conjoin the port deliberately, to distinguish a re-install with a changed port from a
  genuine duplicate — `find_egress_rule_handle_in_dump` `:638-648`), and the "refresh"
  branch still ends in delete-old + append-new, i.e. (i)'s teardown plus extra matching
  machinery. Sweep-all-then-clean-re-install (i) gets the same end state with the existing
  port-keyed predicates untouched. (iii)'s port-blind match would ALSO be the wrong primitive
  to leave lying around — it is exactly the over-broad needle the 03-01 tests
  (`egress_predicate_does_not_mistake_an_inbound_daddr_rule_for_an_egress_rule`,
  `mtls_intercept_worker.rs:1172-1187`) were written to forbid.

**IdentityMgr / netns contrast (consistency with §4).** Like the netns survivor (§4), the
nft-rule survivor is a SURVIVING kernel resource — so, like netns, it cannot rely on a
cheap re-mint the way `IdentityMgr` re-issues an SVID on `¬held`. But UNLIKE the netns
survivor (which §1–§3 ADOPT, because the running workload still lives in it and re-creating
it from slot 0 would collide), the nft rule is TORN DOWN not adopted: the rule's only
purpose is to redirect to a leg port that the restart invalidated, so the surviving rule is
DEAD weight (it points at a dead listener) and there is nothing to preserve — the clean
re-install at 04-03 is what restores a correct rule. Adopt the netns (live), reap the nft
rule (dead): both correct for their resource, the same split-by-survival-semantics §4 draws.

#### Spike boundary — PINNED vs SPIKE-GATED

PINNED (settled by the codebase grounding above, build to these now):
- The `adopt` method signature + `NetSlotAdoptConflict` (§2) — pure in-RAM, no runtime
  unknown.
- The `ObservedAdoptNetns` shape + the three-fact correlation WALK (§1) — the
  mechanism is `ip netns list` (proven parser at `:1870`) + `cgroup.procs` read +
  `/proc/<pid>/ns/net` inode (proven at `netns_entry.rs:96`).
- The boot-pass ORDERING (adopt-then-GC-then-serve) + its home (`run_server`, after
  `AppState`, before the convergence loop) + adopt-conflict-refuses-boot (§3).
- The IdentityMgr contrast (§4).
- The nft-rule reconcile DECISION (§5): option (i) — the boot pass sweeps every
  per-workload TPROXY rule (both directions) from the shared chain by handle, leaving the
  shared infra untouched, so the 04-03 re-install is clean. The MECHANISM is the landed
  by-handle delete (`mtls_intercept_worker.rs:685-695`) + the landed dump-parse predicates
  (`dump_has_egress_rule`/`find_egress_rule_handle_in_dump`/`dump_has_leg_s_exemption`) — no
  new public surface. The rejected alternatives (ii)/(iii) and the adopt-netns-vs-reap-rule
  contrast are pinned.

SPIKE-GATED (do NOT fully pin past these — see the spike recommendation in the
orchestrator report):
- **SPIKE-A:** do workloads actually SURVIVE a real `serve` restart on the Lima
  kernel (not just "setsid + kill_on_drop(false) + cgroup-detach implies it")? If they
  do NOT survive, the whole adopt model collapses to "GC everything + let the
  reconciler re-Start from a fresh slot pool" — a materially simpler 02-06.
- **SPIKE-B:** what does `serve` boot do TODAY with already-Running allocs (re-adopt /
  re-drive / nothing)? Confirms the recovery pass has no pre-existing conflicting
  behaviour.
- **SPIKE-C:** does `/proc/<surviving-pid>/ns/net` inode → `/var/run/netns/ovd-ns-<slot>`
  inode reliably recover the slot for a SURVIVING workload across the restart (the
  proven in-tree mechanism is for a FRESHLY-spawned child; the restart-survivor case is
  unverified)?
- **SPIKE-D (§5 — the nft-rule twin of SPIKE-A):** do the per-workload nft-TPROXY rules
  in the shared `overdrive-mtls` `prerouting` chain actually SURVIVE a real `serve` restart
  on the Lima kernel (the shared-chain "never torn down per-workload" claim implies it, but
  the survival-across-a-real-CP-restart is unverified, exactly as SPIKE-A is for netns)? And
  does the chosen reconcile — sweep every per-workload rule by handle while keeping the F5
  exemption + shared infra — behave on the real kernel (the by-handle `nft delete rule …
  handle <N>` fires against a SURVIVING rule whose guard never ran; the post-sweep chain
  carries ONLY the shared infra; a subsequent clean re-install appends exactly one rule)?
  Cross-checks against the same `nft -a list chain` dump-parse the landed predicates use.
  If the rules do NOT survive a real restart (kernel/netns teardown takes them with it),
  §5 collapses to a no-op (nothing to sweep) and the 04-03 re-install is already clean —
  a materially simpler 02-06, the same way SPIKE-A's negative would simplify the netns side.

Until SPIKE-A/B/C/D return, §1/§3/§5's runtime fidelity is provisional. §2/§4 are pinnable
regardless (pure / contrast-only), and §5's reconcile DECISION (option (i): sweep-all) is
pinned regardless of SPIKE-D — the spike only settles whether there is anything to sweep and
whether the by-handle delete fires on a guard-less survivor, not WHICH reconcile is correct.

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

**Amended 2026-06-18 (02-06 adopt-on-restart).** The allocator's CROSS-RESTART
rebuild is its own step, **02-06** (designed in the "Amended 2026-06-18 (02-06
adopt-on-restart)" block under D-TME-12 above). The 02-04 `NetSlotAllocator`
rustdoc claims the held map is "rebuilt on restart by re-assigning for every
still-Running alloc" (`veth_provisioner.rs:636-639`) — that premise is **FALSE**
(the reconciler does not re-drive a Running survivor; see the 02-06 "C6 was wrong"
analysis), and the rustdoc is flagged for the 02-06 crafter to correct in-code
(architect does not edit code). 02-06 adds an ADDITIVE `NetSlotAllocator::adopt`
(claim a specific `(alloc, slot)`, atomic), a boot-time `adopt_observe` +
adopt-then-GC-then-serve recovery pass in `run_server`, and is PURELY ADDITIVE on
the 02-05 frozen shape. **02-06's survival/adopt RUNTIME semantics are
Tier-3-spike-gated** (SPIKE-A/B/C in the D-TME-12 block) — the orchestrator should
run an `nw-spike` PROBE before dispatching the 02-06 crafter.

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

---

## Amended 2026-06-20 (phase-04 re-plan — two verified dependency inversions resolved)

DELIVER stalled at step 04-01 on two **verified** (not assumed) circular
dependencies in the phase-04 roadmap graph. The crafter correctly STOPPED both
times rather than invent API surface (`execution-log.json`: `02-05` COMMIT
SKIPPED `BLOCKED_BY_DEPENDENCY`; `04-01` PREPARE..COMMIT all SKIPPED
`BLOCKED_BY_DEPENDENCY`). This block records the corrected acyclic structure.
It changes **roadmap sequencing only** — no DESIGN decision (D-TME-1..D-TME-12,
C1..C4, G1..G3, JOIN-1..JOIN-5, the 02-06 adopt-on-restart design) is
reopened. The orchestrator translates the step list below into `roadmap.json`;
the architect does not edit `roadmap.json` / `execution-log.json` / code.

### The two inversions (verified against HEAD + `wip/02-05-netns-join @ a4c2a61d`)

**Inversion 1 — `04-01` ⇄ `04-03` are mutually dependent (a true cycle).**

- `04-01` AC1 requires `start_alloc` to call `install_outbound_tproxy(host_veth,
  leg_f_port)`. `install_outbound_tproxy` takes `host_veth: &str`
  (`crates/overdrive-worker/src/mtls_intercept.rs:340-343`).
- But HEAD `start_alloc(self: &Arc<Self>, spec: &AllocationSpec)` receives ONLY
  `spec` (`mtls_intercept_worker.rs:341-344`), and `AllocationSpec` has NO
  netns / slot / host_veth field at HEAD (`overdrive-core/src/traits/driver.rs:131-156`
  — fields are exactly `alloc, identity, command, args, resources,
  probe_descriptors`). The `host_veth` (`ovd-hv-<4hex-slot>`) is produced by the
  per-workload netns/veth provisioning — i.e. `04-03`'s C3 wiring plus the
  `AllocationSpec.netns` channel (JOIN-1) on `wip/02-05-netns-join`. **So `04-01`
  needs `04-03`'s output.**
- And `roadmap.json:364` declares `04-03 deps: ["02-02","02-03","04-01"]` — **so
  `04-03` needs `04-01`**. Closing the cycle: `04-03`'s C3 wiring breaks the
  `mtls_production_activation` e2e whose deletion lives in `04-01`
  (`execution-log.json` `02-05` COMMIT skip names this verbatim). Neither can
  cleanly precede the other.
- **Resolution: MERGE `04-01` + `04-03` into ONE atomic step.** This is the only
  atomic resolution, and it is well-formed: the merged step's C3 wiring provides
  the `host_veth` the swap consumes, and the merged step's deletion removes the
  broken `mtls_production_activation` e2e in the same commit — so the workspace is
  never red mid-step. The merge is a *single-commit single-cut* (CLAUDE.md
  § "Deletion discipline" + `feedback_single_cut_greenfield_migrations`): orphan
  the cgroup path AND delete it AND wire its replacement, atomically.

**Inversion 2 — the production TPROXY install is NOT #178-entangled for OUTBOUND;
INBOUND's production `virt` source IS #178-gated (but is not a merge blocker).**

This was the load-bearing question. Settled definitively against the code:

- **OUTBOUND install is a production path buildable WITHIN this feature
  (authn-only), realized across `04-01` + `04-02` — NOT complete after `04-01`
  alone.** `install_outbound_tproxy` matches `iifname "<host_veth>"` + `meta
  l4proto tcp` and TPROXY-redirects ALL the workload's egress to leg-F
  (`mtls_intercept.rs:340-388`, rustdoc :280-300). It has **NO per-destination
  match** — "there is no per-destination match because the workload's destination
  is unknown at install time; TPROXY preserves the original destination, which the
  agent recovers per-flow via `getsockname` downstream (03-02)" (rustdoc
  :289-292). The destination is then classified **per-connection** by THIS
  feature's own v1 resolve adapter (D-TME-6 / `ServiceBackendsResolve`, built
  01-03; consumed `04-02`) — NOT at install time, NOT from #178. **`04-01`
  installs the outbound rule + the leg-F/leg-C listeners + the accept loop; the
  outbound traffic path does not complete until `04-02` wires that resolve
  consumer — until then `real_peer` is `None` and leg-F traffic
  fail-closed-drops.** (Honesty clarification 2026-06-20, per the 04-01 review.) The `host_veth` is
  the only input the outbound install needs, and it comes from the merged step's
  C3 wiring (`derive_workload_netns_plan(slot).host_veth`). **No #178 source is
  required for the outbound production install.**
- **INBOUND production install is #178-gated for its `virt` match-key.**
  `install_inbound_tproxy(virt, agent_port)` keys on `ip daddr <virt> tcp dport
  <vport>` (`mtls_intercept.rs:240,256-261`) — it needs the server workload's
  listen `virt` at install time. HEAD `start_alloc` is explicit that v1 has NO
  production source for it: "`AllocationSpec` carries no listen-addr field and the
  workload binds its own socket at runtime (the same east-west service-resolution
  gap that defers the outbound peer set; #178 … names the inbound
  orig-dst→real-backend resolution and the `server_dial_addr` / D-MTLS-15
  replacement site as #178's job)" (`mtls_intercept_worker.rs:398-416`). So HEAD
  records `tproxy_guard = None` and installs NO inbound rule; the
  `install_inbound_tproxy` free fn "stays the named #178 production-install site,
  exercised today only by the worker integration tests (which supply a real,
  distinct virt)" (module rustdoc :53-59).
- **What this means for the merged step's ACs.** The leg-C `IP_TRANSPARENT`
  listener + the inbound accept→`enforce` loop ARE production and ARE wired by the
  merged step (unchanged from HEAD `start_alloc`). The *production inbound nft
  rule* is the only inbound piece with no v1 source — it stays test-only-until-#178
  exactly as HEAD already has it. **The merged step does NOT regress inbound** (it
  preserves the existing inbound-listener-production / inbound-rule-#178-deferred
  posture) and **fully lands the OUTBOUND production path**. The walking skeletons
  (05-*) drive the inbound rule via the worker integration tests' real distinct
  virt (the established "only test callers until #178" shape), and exercise the
  outbound path as a genuine production flow. This is consistent with D-TME-8 / Q4
  (v1 = authn-only; `expected_peer` / intended-peer pinning deferred to #178) and
  ADR-0071 fact 3 (inbound UNCHANGED from ADR-0069: per-`virt`, not `iifname`).

  **Scope note on the merged step's AC wording.** The feature-delta line 89 / ADR
  framing "`start_alloc` installs BOTH nft-TPROXY rules (outbound on host-veth +
  inbound on the workload virt)" describes the *eventual* both-directions
  production shape. For v1 the *inbound* production rule has no virt source, so the
  merged step's AC must read: **install the OUTBOUND rule as production
  (`iifname host_veth`, no #178 dependency); stand up the leg-C listener + inbound
  accept loop as production; the INBOUND production nft rule stays #178-deferred
  (`install_inbound_tproxy` remains the named #178 install site, test-callers
  only), exactly as HEAD.** The merged step DELETES the cgroup outbound surface
  and the declared-peer surface; it does NOT invent an inbound virt source. Stating
  "installs BOTH rules in production" as a v1 AC would force inventing a `virt`
  source — forbidden (CLAUDE.md § "Implement to the design — never invent API
  surface").

**#178 verdict (one line):** the OUTBOUND production install is **fully buildable
now within this feature**, authn-only, using this feature's own v1
`ServiceBackendsResolve` (04-02) for per-connection classification — it needs NO
#178 source. The INBOUND production *nft-rule* `virt` source IS genuinely
#178-gated and stays test-only-until-#178 (unchanged from HEAD); the leg-C
listener + accept loop are production. The expected-SVID / intended-peer pin is
separately #178 (authn-only v1 is fine, D-TME-8). **The merged step (`04-01`)
installs the OUTBOUND production rule + the leg-F/leg-C listeners + the accept
loop; the outbound *traffic path* completes once `04-02` wires the per-connection
resolve consumer (`ServiceBackendsResolve` → `Mesh`/`NonMesh`/`MeshUnreachable`).
Until `04-02`, `real_peer` is always `None` and leg-F traffic fail-closed-drops —
so "production outbound path" means realized within THIS feature across `04-01` +
`04-02`, NOT complete after `04-01` alone, with NO #178 dependency.**
*(Honesty clarification 2026-06-20, prompted by the 04-01 review: the prior
"LANDS NOW as a working outbound production path" wording was misreadable as
"complete after 04-01 alone." It is not — 04-01 lands the install + listeners +
accept loop; 04-02 lands the per-connection classification that completes the
traffic path. The load-bearing point — the whole outbound path is buildable
WITHIN this feature with NO #178 source — is unchanged.)* *(Orchestrator: before landing, confirm #178's
scope with `gh issue view 178 --comments` per CLAUDE.md — the inbound-virt
attribution is from the HEAD code comment + this feature's design framing; #178's
own body/comments should be checked to confirm it covers the inbound
orig-dst→virt resolution and not only the expected-SVID join. The architect could
not run `gh` in this doc-only dispatch.)*

### Corrected phase-04 step structure (acyclic)

The merge of `04-01`+`04-03` is renumbered **`04-01`** (the consolidated
production-rewire + C3-wiring step). `04-02` (resolve consumer) stays separate and
moves AFTER the merged step. `02-05` is removed as a step (its work — the C3
wiring + ExecDriver join on `wip/02-05-netns-join @ a4c2a61d` — is folded INTO the
merged `04-01`). The spike-validated 02-06 adopt-on-restart becomes **`04-04`**
(its true position: it depends on the merged C3 wiring being live). Final list:

| id | name | deps | one-line scope |
|---|---|---|---|
| `04-01` | **Merged production rewire + C3 netns wiring + single-cut deletions** | `02-03`, `03-01`, `03-02` | The atomic resolution of Inversion 1. Re-apply `wip/02-05 @ a4c2a61d` (G1+G2+G3 + JOIN-1..JOIN-5 + review findings 1&2) for the per-alloc netns/veth C3 wiring + `AllocationSpec.netns` channel + ExecDriver per-spec `setns` (delete `with_netns_path`); swap `start_alloc` to install the OUTBOUND `install_outbound_tproxy(host_veth=plan.host_veth, leg_f_port)` rule as production (the `host_veth` value reaches `start_alloc` via the JOIN-1-sibling `AllocationSpec.host_veth: Option<String>` field — see JOIN-6 below) + stand up leg-F/leg-C listeners + accept loops (inbound nft rule stays #178-deferred per Inversion 2); single-cut DELETE the cgroup surface (`cgroup_connect4_mtls`, `MTLS_REDIRECT_DEST`, the whole `MtlsDataplane` struct, `attach_alloc`/`program_redirect`/`MtlsCgroupLink`, the orphaned `MtlsBootError::Load`/`OutboundAttach` error variants) AND delete the `mtls_production_activation` e2e + `mtls_e2e_helpers` + the OLD-mechanism test files (the 04-01 deletion list) it breaks, in the SAME commit. Tier-3: JOIN-5 (workload lands in netns) + start_alloc installs the outbound rule on a real alloc; re-run boot fixtures under Lima (NOT `--no-run`). |
| `04-02` | Per-connection resolve consumer + DELETE declared-peer surface | `01-02`, `03-02`, `04-01` | Unchanged from the existing 04-02 EXCEPT `deps` drops the now-merged `04-01`-as-cgroup-deleter (still `04-01`, now the merged step) — wire `MtlsResolve` (mandatory `new()` param) into the outbound accept loop: `Mesh`→enforce, `NonMesh`→pass-through, `MeshUnreachable`→fail-closed; single-cut DELETE `program_declared_peer_redirect`, `real_peer`/`leg_f_addr` slots, `AcceptOutcome::Dropped`, `accept_drop_outbound`, the `MtlsInterceptError` enum + inline tests; fix the `lib.rs:981-986` broken doc link. Default-lane DST + mutation ≥80% on the 3-arm decision. |
| `04-03` | *(removed — merged into `04-01`)* | — | The old `04-03` C3-wiring step is the second half of the merged `04-01`; it no longer exists as a separate step. |
| `04-04` | Adopt-on-restart cross-restart slot+rule rebuild (the former "02-06") | `04-01` | The spike-validated (PROCEED-AS-DESIGNED, kernel 7.0.0, `spike/findings-adopt-restart.md`) adopt-on-restart pass: `NetSlotAllocator::adopt` (additive, atomic) + `adopt_observe` (cgroup→PID→`/proc/ns/net` slot recovery) + a `run_server` boot recovery pass (adopt-then-GC-then-serve, BEFORE the convergence loop) + the §5 surviving-nft-rule sweep. Depends on `04-01` because the C3 wiring (the `NetSlotAllocator` on `AppState`, the provision/teardown seams, the per-workload nft rules) must be LIVE for there to be slots/rules to adopt/reap. Tier-3 under Lima. |

`02-05` is **removed from the roadmap as a step** — see "02-05 disposition" below.

### New dependency graph — every edge justified (acyclic)

Phase-04 (and the two cross-phase feeders) edges:

- `04-01 → 02-03`: the merged step provisions the per-workload netns and the
  resolv.conf injection (02-03) is part of the provisioner converge surface it
  reuses.
- `04-01 → 03-01`: the merged step's `start_alloc` swap calls
  `install_outbound_tproxy` (built in 03-01).
- `04-01 → 03-02`: the merged step's outbound accept loop relies on
  `accept_outbound_leg`'s `getsockname` orig-dst recovery (built in 03-02).
- `04-02 → 04-01`: the resolve consumer runs in the outbound accept loop the
  merged step stands up; and the merged step has already deleted the
  cgroup/`MtlsDataplane` surface + `MtlsInterceptError::Dataplane` source 04-02's
  remaining declared-peer deletion would otherwise collide with.
- `04-02 → 01-02`: the consumer's default-lane DST drives `SimMtlsResolve` (01-02).
- `04-02 → 03-02`: the consumer reads the `getsockname`-recovered orig_dst (03-02).
- `04-04 → 04-01`: adopt-on-restart rebuilds the `NetSlotAllocator` map + reaps
  surviving netns/nft rules — all of which only EXIST once the merged step's C3
  wiring + nft installs are live.
- `05-01 → 04-02, 04-01, 03-03, 02-03`: the composed walking skeleton needs the
  resolve consumer (04-02), the merged C3 wiring + both installs (04-01), the
  egress capture proof (03-03), and resolv.conf injection (02-03). **The old
  `05-01 deps: ["02-03","03-03","04-02","04-03"]` rewrites to
  `["02-03","03-03","04-02","04-01"]`** (`04-03` → merged `04-01`).
- `05-03 → 04-02, 03-03`: unchanged (no `04-03`/`04-01`-cgroup edge).
- `05-02 → 02-03, 05-01`: unchanged.

No edge points both ways; the graph is a DAG. The cycle is broken because the two
formerly-mutually-dependent steps are now one node.

### 04-02 placement — SEPARATE-AFTER the merged step (NOT folded)

`04-02` (resolve consumer) is kept **separate and sequenced after** the merged
`04-01`, not folded in. Rationale: (a) the merged step is already the largest
single-cut in the feature (re-apply the whole wip branch + the swap + the cgroup
deletion + the e2e deletion); folding the resolve consumer + the declared-peer
deletion in would make one commit that is hard to review and hard to bisect; (b)
04-02's deletion (declared-peer slots, `program_declared_peer_redirect`,
`AcceptOutcome::Dropped`) is orphaned by **the resolve consumer**, not by the
cgroup swap — so by the dispatch-sequencing rule ("the step that orphans a surface
deletes it") it belongs with the resolve-consumer step; (c) 04-02 is default-lane
DST + mutation, a different test tier from the merged step's Tier-3 — separable
cleanly. The merged step leaves the declared-peer slots in place (HEAD shape minus
cgroup); 04-02 removes them when it adds the resolve consumer that supersedes them.
This matches the existing roadmap's intent (04-02 already deps on 04-01 and already
owns the declared-peer deletion); only the cycle through 04-03 is removed.

### 04-04 (adopt-on-restart) placement and id

The step the design calls "02-06" is **not** a phase-02 step — it depends on the
C3 wiring (the `NetSlotAllocator` on `AppState`, the provision/teardown seams, the
per-workload nft rules) being LIVE, which is the merged `04-01`'s output. Its true
position is **after** the merged step, so it is renumbered **`04-04`**, `deps:
["04-01"]`. (Keeping the "02-06" label would assert a phase-02 position the
dependency graph contradicts.) Its design is fully pinned + spike-validated
(PROCEED-AS-DESIGNED, kernel 7.0.0); SPIKE-A/B/C returned positive. **Residual
flagged:** SPIKE-D (do the per-workload nft-TPROXY rules SURVIVE a real `serve`
restart, and does the by-handle sweep fire on a guard-less survivor?) was NOT run
in `spike/findings-adopt-restart.md` (only A/B/C). Per the design, §5's reconcile
DECISION (sweep-all by handle) is pinned regardless of SPIKE-D — SPIKE-D only
settles whether there is anything to sweep. So `04-04` is not blocked, but its §5
nft-rule-sweep leg carries an un-probed runtime assumption the crafter should
ground-truth (or the orchestrator may run SPIKE-D before dispatching `04-04`'s §5
half). This is a residual to confirm, not a blocker.

### 02-05 disposition — REMOVED as a step

`02-05` is **removed from the roadmap**. Its implementation
(`wip/02-05-netns-join @ a4c2a61d`: G1+G2+G3 + JOIN-1..JOIN-5 + review findings
1&2, default-lane GREEN, JOIN-5 Tier-3 passed) is **re-applied / folded into the
merged `04-01`**. `02-05` could never land in its phase-02 position: it breaks the
`mtls_production_activation` e2e that only `04-01` deletes (the
`execution-log.json` `02-05` COMMIT SKIP records this). It was a premature
duplicate of `04-03`'s C3 wiring; with `04-03` now merged into `04-01`, `02-05`'s
work lands there. The branch is the IMPLEMENTATION the merged step reuses — the
merged step does NOT re-derive the G1/G2/G3/JOIN design.

### Execution-log reconciliation — FLAG to the orchestrator (do NOT touch here)

`execution-log.json` carries entries that reference removed/redefined steps after
this restructure; the architect does NOT edit it (doc-only dispatch). The
orchestrator must reconcile:

- **`02-05` entries** (PREPARE..GREEN EXECUTED PASS, COMMIT SKIPPED
  `BLOCKED_BY_DEPENDENCY`, all `2026-06-18`): the step is removed. Its GREEN work
  re-lands under the merged `04-01`. The orchestrator decides whether to (a) leave
  the `02-05` events as historical record of the premature attempt (preferred —
  they are the honest record, and CLAUDE.md "kept so the execution-log entries are
  not orphaned" already governs this), or (b) annotate them as superseded by
  `04-01`. Do NOT delete them.
- **`04-01` entries** (PREPARE EXECUTED PASS; RED_ACCEPTANCE / RED_UNIT / GREEN /
  COMMIT all SKIPPED `BLOCKED_BY_DEPENDENCY`, `2026-06-19`): the step id `04-01`
  is REDEFINED (now the merged production-rewire + C3 step). The orchestrator must
  decide whether the prior `04-01` skip events stand as the record of the
  pre-merge blocker, or are reset for the redefined step. The redefined `04-01`
  is a NEW execution against the merged scope; its events will be appended fresh.
- **`04-03` entries:** none exist (the step never executed). No reconciliation
  needed beyond removing the step definition.

The architect leaves `roadmap.json` and `execution-log.json` UNTOUCHED; this block
is the corrected step list the orchestrator translates.

### JOIN-6 (the `host_veth` channel) — a new `AllocationSpec.host_veth: Option<String>` field (NO newtype) — user-approved 2026-06-20

**Context — a genuine signature gap, not an already-decided point.** The merged
`04-01` scope row (~line 1286) pins
`install_outbound_tproxy(host_veth=plan.host_veth, leg_f_port)` inside
`MtlsInterceptWorker::start_alloc`. That row pins the **VALUE source**
(`derive_workload_netns_plan(slot).host_veth`) but is silent on the **CHANNEL** —
how the control-plane-derived `host_veth` string reaches the worker's
`start_alloc`. The crafter surfaced this and escalated rather than inventing
surface (`execution-log.json` `04-01` blocked; CLAUDE.md § "Implement to the
design — never invent API surface"). This block records the user-approved
resolution.

- `host_veth` (`ovd-hv-<4hex-slot>`) is produced by
  `derive_workload_netns_plan(slot).host_veth` in `overdrive-control-plane` (the
  action-shim C3 provision seam).
- `overdrive-worker` does NOT depend on `overdrive-control-plane`, so the worker
  cannot re-derive `host_veth` (it would need a forbidden dep edge or a duplicated
  prefix constant).
- `MtlsInterceptWorker::start_alloc(self: &Arc<Self>, spec: &AllocationSpec)` is an
  INHERENT method (not a port-trait), called directly by the action-shim's
  `StartAllocation`/`RestartAllocation` arms (`action_shim/mod.rs` ~:980 / ~:1133).

**Resolution: add `AllocationSpec.host_veth: Option<String>` — a JOIN-1 SIBLING
field, symmetric with the existing `netns: Option<String>` (JOIN-1).** Pinned exact
shape:

```rust
// overdrive-core/src/traits/driver.rs — appended to AllocationSpec, beside `netns`.
/// Host-side veth interface NAME for this allocation's per-workload veth
/// pair (`ovd-hv-<4hex-slot>`), the `iifname` the outbound nft-TPROXY rule
/// matches to redirect the workload's egress to leg-F
/// (`MtlsInterceptWorker::start_alloc` →
/// `install_outbound_tproxy(host_veth, leg_f_port)`). `Some(plan.host_veth)`
/// ONLY when the action-shim C3 site provisioned a per-workload netns/veth
/// (the production mTLS-composed boot); `None` for every non-netns workload
/// (every current test fixture, and any boot where the mTLS composition gate
/// is off) — the pre-join host-netns behaviour, exactly like `netns`.
///
/// `Option<String>`, NOT a newtype — the SAME rationale as JOIN-1's `netns`
/// (see the JOIN-1 newtype-decision block above): the value is already a
/// validated, bounded, slot-derived name minted ONLY by
/// `derive_workload_netns_plan` (a pure projection of the already-newtyped
/// `NetSlot`); it has no parse surface, no operator-typed entry point, and no
/// `FromStr` round-trip to defend.
pub host_veth: Option<String>,
```

**Injection point — set at the SAME C3 provision seam as `spec.netns` (JOIN-2).**
In the action-shim provision path (`provision_and_inject_netns`), add the
`host_veth` assignment beside the existing `netns` one — reading `plan.host_veth`
before the `plan` local is dropped/moved:

```rust
// action-shim C3 provision seam, beside the JOIN-2 `spec.netns = Some(plan.netns)`:
spec.netns     = Some(plan.netns.clone());      // JOIN-2 (existing)
spec.host_veth = Some(plan.host_veth.clone());  // JOIN-6 (this amendment)
```

**Read point — `start_alloc` keeps its 2-arg signature UNCHANGED.**
`MtlsInterceptWorker::start_alloc(self: &Arc<Self>, spec: &AllocationSpec)` reads
`spec.host_veth.as_deref()` to feed `install_outbound_tproxy`. No new parameter, no
worker signature change, no `overdrive-worker → overdrive-control-plane` dep edge.

**Construction-site sweep — extends the JOIN-4 `netns: None` sweep.** The
`host_veth: None` additions ride the SAME ~31 construction sites JOIN-4 already
sweeps for `netns: None` — one more field each, off the mTLS-composed boot gate.
The two production reconciler sites (workload_lifecycle.rs `:665`/`:750`,
reconciler_runtime.rs `:2924`) get `host_veth: None` (reconciler stays
netns/veth-agnostic, per JOIN-2); the `netns_entry.rs` JOIN-3 fixtures that drive
`spec.netns = Some(<name>)` need no `host_veth` value (their assertion is the
netns-entry seam, not the outbound rule) and take `host_veth: None`.

**Why Option A (this) over the rejected alternative (a `host_veth` parameter on
`start_alloc` + threading `plan` out of the provision helper):**

- The merged-step row pinned the VALUE source (`plan.host_veth`), not the CHANNEL —
  so this was a genuine signature gap to fill, not a decided point being
  re-litigated.
- `netns` and `host_veth` are the **same category of data**: per-alloc,
  slot-derived strings from the same `plan`, set at the same provision seam.
  JOIN-1 already ratified putting that category on `AllocationSpec` as
  `Option<String>` (no newtype); this is the faithful, symmetric extension — one
  line beside the existing `spec.netns` assignment.
- The "a host artifact doesn't belong on a workload spec" objection was already
  decided when JOIN-1 put `netns` (an identical host artifact) on `AllocationSpec`.
  This stays consistent with that ratified boundary rather than reopening it for one
  more field.
- The rejected alternative diverges more from the re-applied
  `wip/02-05-netns-join @ a4c2a61d` C3 code and changes the worker method
  signature, for no architectural gain.

**Scope note:** this amendment adds EXACTLY ONE field
(`AllocationSpec.host_veth: Option<String>`) and no other API surface. It does not
reopen any D-TME / C / G / JOIN-1..JOIN-5 decision or the 02-06/04-04
adopt-on-restart design; the merged `04-01` row's substance (`host_veth =
plan.host_veth`) is unchanged — this block names the channel that carries it.
