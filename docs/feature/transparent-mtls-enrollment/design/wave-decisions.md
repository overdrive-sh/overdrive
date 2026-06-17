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
| D-TME-11 | **Resolve READ MECHANISM (C4; refines D-TME-6)**: `ServiceBackendsResolve` resolves `orig_dst` against an **in-RAM, address-keyed reverse index** of the `running` `service_backends` set (`addr → Backend`) — NOT a per-`ServiceId` point query (the `ServiceId`-keyed `service_backends_rows` is the wrong surface; the adapter holds no `ServiceId`). **REVISED 2026-06-17 (resolve-index-coherence research):** built via **List-then-Watch + relist-on-`Lagged`** (the prior observe-only / "no new trait method" constraint is REVERSED). List leg = the keyless `all_service_backends_rows()` enumerate (SHIPPED `25e7acf3`); List-at-probe closes #237 cold-start; single-owner drain dissolves the F2 take/restore TOCTOU. **F4 / relist-trigger REFINED 2026-06-17 (ratified — option 2, surface `Lagged`):** the lossy `subscribe_all()` (item type `ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>`) cannot carry the loss signal — both adapters strip `RecvError::Lagged` internally — so closing F4 requires a NEW lag-surfacing surface `subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError>` delivering `SubscriptionEvent::{Row, Lagged { missed: u64 }}` (a DOMAIN event; adapter maps `RecvError::Lagged(n) → missed`; no tokio leak); the single-owner drain consumes it and re-Lists on `Lagged`, closing F4 with a *completeness* guarantee. **A miss = `NonMesh`** (cleartext pass-through), NOT `MeshUnreachable`; the residual irreducible convergence window is the **(a) fail-toward-handshake** v1 SECURITY invariant, tracked in **#236**. **#237 CLOSED by this revision** (List-at-probe + relist). PUBLIC `MtlsResolve` API unchanged; growth confined to the `ObservationStore` driven port. | D-TME-6 pinned the resolve *model* but not the *read mechanism*; a DELIVER step surfaced that `resolve(orig_dst)` has no `ServiceId` and no addr→service surface exists. Cilium's `ipcache` (in-RAM addr→identity reverse index, subscribe-populated, List-before-Watch, relist-on-loss) is the canonical precedent; research §4.1 describes the resolve as an in-RAM `service_backends` lookup. Making a miss fail-closed would break legitimate `NonMesh` external egress — forbidden. (Ratified 2026-06-16; read-mechanism REVISED 2026-06-17; F4/relist-trigger REFINED 2026-06-17.) |

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
| **C4** (added 2026-06-16; read-mechanism REVISED 2026-06-17; F4/relist-trigger REFINED 2026-06-17 — all post-DESIGN amendments) | `ServiceBackendsResolve` resolves `orig_dst` against an **in-RAM, address-keyed reverse index** of the `running` `service_backends` set (`addr → Backend`), built via **List-then-Watch + relist-on-`Lagged`** over the `ObservationStore` — NOT a per-`ServiceId` point query. **REVISED 2026-06-17 (resolve-index-coherence research): the prior observe-only / "MUST NOT add a new trait method" constraint is REVERSED.** The mechanism now (1) ADDS a keyless List enumerate `all_service_backends_rows(&self) -> Result<Vec<ServiceBackendRow>, ObservationStoreError>` — symmetric with `alloc_status_rows()`/`node_health_rows()`, SHIPPED `25e7acf3`; (2) **Lists-at-probe** before the Earned-Trust gate opens (closes **#237** cold-start, SHIPPED `25e7acf3`); (3) uses a **single-owner drain** (dissolves the **F2** take/restore TOCTOU per `development.md` § "Check-and-act must be atomic", SHIPPED `25e7acf3`); (4) **relists on a `Lagged` loss signal** to close **F4** lag-drop. **F4 / relist-trigger REFINED 2026-06-17 (ratified — option 2, surface `Lagged`):** the prior wording "relists on `broadcast::RecvError::Lagged`" assumed the loss signal was reachable, but `subscribe_all()` returns the lossy `ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>` and BOTH store adapters strip `RecvError::Lagged` internally — so closing F4 requires a NEW lag-surfacing surface: **`subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError>`** delivering **`SubscriptionEvent::{Row(ObservationRow), Lagged { missed: u64 }}`** (a DOMAIN event; adapter maps `RecvError::Lagged(n) → missed`; the core trait never names a tokio type). The single-owner drain consumes `subscribe_all_events()`; on `Lagged { missed }` it re-Lists via `all_service_backends_rows()` and rebuilds/merges the index — closing F4 with a *completeness* guarantee. A **dedicated method** (not a shared-type change to `ObservationSubscription`) bounds blast radius — only `ServiceBackendsResolve` consumes it; the ~20 existing `subscribe_all()` consumers stay untouched. Miss-classification scoping: a **miss = `NonMesh`** (cleartext pass-through, by design), NOT `MeshUnreachable`; the residual irreducible convergence window is covered by **(a) fail-toward-handshake** — the v1 SECURITY invariant *"a resolve miss must never silently emit cleartext to a should-be-mesh peer,"* whose code lands under **#236**. **#237 CLOSED by this revision**; residual → (a)/#236. **PUBLIC `MtlsResolve` API unchanged** (growth confined to the `ObservationStore` driven port). | feature-delta § "`MtlsResolve` port contract" (C4) + § "C4 — F4 / relist-trigger refinement" + D-TME-11 row; ADR-0071 § "The new driven port" (C4 + F4/relist-trigger refinement); this file (D-TME-11). Consistent with the shipped 01-01 port rustdoc (`crates/overdrive-core/src/traits/mtls_resolve.rs`) — ADDS the read mechanism, does NOT re-classify. Revision evidence: `docs/research/networking/transparent-mtls-resolve-index-coherence-research.md`; F4-trigger evidence: ground-truth `subscribe_all` lossy surface (`observation_backend.rs:506`, `redb_backend.rs:368-373`, `ObservationSubscription` at `observation_store.rs:1149`). |

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
