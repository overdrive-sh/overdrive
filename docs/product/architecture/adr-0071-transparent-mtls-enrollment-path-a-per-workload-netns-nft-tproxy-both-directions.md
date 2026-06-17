# ADR-0071: Transparent-mTLS enrollment (Path A) — per-workload netns+veth + nft-TPROXY both directions; retire `cgroup/connect4`-rewrite outbound recovery

## Status

**Accepted** (2026-06-16). Decision-maker: Morgan (solution-architecture);
**ratified by the user** — Q1–Q4 ratified and Q5a (fold the DNS name-layer
*integration* into this design) chosen. The four prior open sub-decisions are
now baked into the Decision section below (§ "Ratified sub-decisions (Q1–Q4)");
the name-layer integration + the DNS-return decision are recorded in §
"Name-layer integration (Q5a)". One recommendation remains open for sign-off —
the DNS-return shape (headless, recommended) — and adds no new v1 dependency, so
it is not a blocker. Tags: phase-2, dataplane, mtls, ktls, tproxy, netns,
enrollment, dns, name-layer, #236.

**Amends ADR-0069** ("Transparent mTLS via a universal agent-light L4 proxy")
on exactly one axis: the **OUTBOUND interception + original-destination
recovery mechanism**. ADR-0069 framed the outbound active side as a
`cgroup/connect4`-rewrite to the agent's leg-F listener (Decision fact 1
OUTBOUND; the outbound topology diagram; § "intercept-recursion / agent-leg-B
exemption"; the Enforcement outbound invariants). **This ADR replaces that
framing with Path A**: per-workload **netns + veth** + **nft-TPROXY +
`IP_TRANSPARENT` + `getsockname` for BOTH directions** (the active-side mirror
of the already-shipped, already-proven inbound passive side).

**What this ADR does NOT touch — the locked core of ADR-0069/0070 is
UNCHANGED**: the universal agent-light L4 proxy model, the fold of #222 into
#26, the 4-method `MtlsEnforcement` contract (`probe`/`enforce`/`liveness`/
`teardown`), the agent-light asymmetry (zero-copy `splice` decrypt / `write_all`
copy encrypt; `mtls/splice.rs`), the no-psock invariant, the F4/F7 resource
limits, the kTLS-arm probe sentinel, the (C)+(B) supervision shape (ADR-0070),
and the **authn-only v1 boundary** (chain-to-bundle authn + encryption, no
intended-peer pinning; #178 upgrade). The `Routed::Outbound { peer }` input to
`enforce` is UNCHANGED — only how the worker *obtains* `peer` changes
(`getsockname`, not a declared-peer slot). **No new `MtlsEnforcement` method or
variant is added.**

## Context

ADR-0069 left the OUTBOUND original-destination recovery as an OPEN,
spike-gated question. The shipped outbound path is *documented-lossy*: the
`cgroup_connect4_mtls` program rewrites `connect(real_peer) → connect(leg_F)`
in place, and the original destination is **NOT recoverable** from the accepted
leg-F socket. Production never armed the per-destination `MTLS_REDIRECT_DEST`
map (an empty map = cleartext passthrough); a test-only
`program_declared_peer_redirect` seam *supplied* `real_peer` as the #178
stand-in. The enrollment model (#236) requires the agent to *learn* the dialed
destination per-connection with no pre-programmed entry.

### The spike settled the recovery mechanism (DOESN'T-WORK → PIVOT)

`docs/feature/transparent-mtls-enrollment/spike/` (Probe A, real-kernel Lima
7.0, gitignored `spike-scratch/increment-a/`, zero `crates/` touch):

- **Probe A verdict: DOESN'T-WORK.** A `cgroup/connect4` redirect + kernel-side
  stash + `cgroup/getsockopt(SO_ORIGINAL_DST)` cannot recover the workload's
  original outbound destination on the agent's accepted leg-F socket. Three
  independent fatal walls, any one sufficient:
  1. `connect4` fires BEFORE the ephemeral source port binds → no 4-tuple key
     the accept side can reconstruct.
  2. A `connect4` sockaddr rewrite is NOT a netfilter DNAT → conntrack holds no
     original tuple → `SO_ORIGINAL_DST` = `ENOENT` (errno 2, measured).
  3. `cgroup/getsockopt` fires only for tasks INSIDE the attached cgroup; the
     agent runs OUTSIDE the workload cgroup (the F5 exemption), and
     `bpf_get_socket_cookie` is verifier-forbidden in `cgroup/getsockopt`.
- **Cilium reconciliation** (read-only, Cilium `main @ dac977e678`,
  v1.20.0-dev): ZERO `cgroup/getsockopt` and ZERO `SO_ORIGINAL_DST` in
  Cilium's BPF tree. Its mediating-proxy path is **TPROXY via `bpf_sk_assign` +
  fwmark → `IP_TRANSPARENT` listener → `getsockname`** — independently
  confirming the pivot to TPROXY+`getsockname`, not `connect4`+`getsockopt`.

(The prior research doc
`transparent-mtls-interception-mechanism-2026-research.md` *recommended* Probe
A as the cleaner mechanism; the real-kernel spike falsified it on the appliance
kernel. The research's stated **fallback — Probe B, TPROXY-outbound — IS Path
A.** The research's *model* verdict — enrollment capture-and-resolve — stands.)

### The lever and the constraint

ADR-0068 pins kernel 6.18 (prod) / 7.0 (dev Lima). TPROXY / `IP_TRANSPARENT` /
`getsockname` are ancient (pre-3.0) — far under any floor. **The
`cgroup_sock_addr`/`cgroup_sockopt` hook families have NO `BPF_PROG_TEST_RUN`
backstop** (`ENOTSUPP`), so any kernel-interception change is Tier-3-validated
on a real connect under Lima; a `--no-run`/compile-only gate is not an honest
signal. Path A *removes* the `cgroup_sock_addr` outbound hook entirely — but the
NEW egress nft-TPROXY-on-a-per-workload-veth wiring is itself UNVALIDATED on our
topology (Cilium proves the model; our wiring is unproven), so it is a Tier-3
obligation (see § Open sub-decisions Q1).

### Quality attributes driving the decision (ISO 25010)

| Attribute | Why it dominates |
|---|---|
| **Security — confidentiality** | The enrollment model removes the silent-cleartext-on-miss footgun of the per-destination map: a captured connection either reaches a resolved mesh peer over mTLS or fail-closes; non-mesh egress is classified and passed through, never silently leaked. |
| **Functional suitability — one mechanism** | nft-TPROXY+`getsockname` already runs inbound; Path A unifies BOTH directions on it (no `bpf_sk_assign`, no `connect4`, no per-destination map). |
| **Reliability** | TPROXY+`getsockname` recovery has no conntrack dependency, no cross-socket correlation, no cgroup-scope mismatch — the three failure classes that killed Probe A. |
| **Maintainability** | Deletes the `cgroup_connect4_mtls` program, the `MTLS_REDIRECT_DEST`/`MtlsDataplane` outbound surface, and the declared-peer stand-in. Net deletion. |

## Decision

**Adopt Path A: give each exec workload its own network namespace + veth pair,
and intercept BOTH directions with nft-TPROXY + `IP_TRANSPARENT` +
`getsockname`. Resolve the captured connection's original destination →
backend + expected identity per-connection (enrollment), via a new
`MtlsResolve` driven port.**

The structural facts, binding on DELIVER:

1. **Per-workload netns+veth topology.** Each exec allocation is born into its
   own netns (the `ExecDriver` `setns(CLONE_NEWNET)` hook — `driver.rs:181-198`
   — ENTERS an already-created netns; CNI-aligned). Its sole egress/ingress is
   a veth pair whose host-side end is where nft-TPROXY PREROUTING applies. v1
   single-node therefore moves OFF the host netns
   (`veth_provisioner.rs:36-37`) — see § "Changed assumption". The provisioner
   creates+converges the netns/veth (idempotent converge-on-boot, the existing
   `veth_provisioner` shape); the driver enters it. **Netns-creation call site
   PINNED (C3; DESIGN-review handoff condition):** the provisioner runs at the
   action-shim **`on_alloc_running`** hook (alloc → Running), **BEFORE
   `MtlsInterceptWorker::start_alloc` and BEFORE `start_alloc` / `Driver::start`**
   — i.e. the netns+veth exists before the workload process is spawned into it
   (the `ExecDriver` `setns` seam enters the already-created netns; the
   provisioner creates it). Teardown is at `on_alloc_terminal`. Lifecycle shape =
   extend `veth_provisioner` (Q2 RATIFIED); a per-alloc network reconciler is the
   Bar-2 promotion when runtime drift matters (#197/#234 family).

   **Amended 2026-06-17 (D-TME-12; resolves DELIVER 02-01 review B1+S1+S2 and the
   02-01 re-review B3).** The netns name, BOTH veth iface names, AND the per-alloc
   point-to-point /30 subnet derive from a **host-unique bounded network slot**
   (`NetSlot`, `0..=4095`), NOT from the `AllocationId` directly. A Linux iface
   name is `IFNAMSIZ`=15-usable-bounded; a netns name is a `/run/netns/` path
   component bounded by `NAME_MAX`=255; `AllocationId` is `LABEL_MAX`=253-bounded;
   **no pure function of a 253-char id can collision-free-map into a 15-char name**
   (pigeonhole), so the original `ovd-{hv,wl}-<alloc>` literal both overflowed
   IFNAMSIZ (any alloc id ≥ 9 chars) and would collide two allocations onto one
   veth under truncation — and a *hash* makes collisions merely *unlikely*, the
   exact hand-wave CLAUDE.md § "One shared length ceiling for label-shaped ids"
   forbids. The slot makes collision-freedom **structural**: `ovd-hv-<4hex-slot>` /
   `ovd-wl-<4hex-slot>` (11 chars ≤ 15, the 4-char hex ceiling DERIVED from
   `IFNAMSIZ − prefix`), and the slot indexes a /30 block in a fixed per-host base
   (`WORKLOAD_SUBNET_BASE` /16 → the 4096-slot space tiles a /18 within that /16,
   NOT the whole /16). **The netns name is ALSO slot-keyed: `ovd-ns-<4hex-slot>`**
   (11 chars ≤ `NAME_MAX` AND ≤ IFNAMSIZ, bounded by construction — B3 resolution,
   ratified option (a). The first cut of D-TME-12 left the netns name embedding the
   unbounded `AllocationId` (`ovd-ns-<alloc>`) with an arithmetically false "≤255"
   reassurance — 7-char prefix + 253-char alloc id = 260 > 255 → `ENAMETOOLONG`
   from `ip netns add`, the IDENTICAL pigeonhole/ceiling defect class as B1, on the
   one derived name the first cut left out; slot-keying it makes the overflow
   unrepresentable, the same lever that beat the hash for B1). `ip netns list` shows
   `ovd-ns-<4hex>`; the human-readable alloc identity lives in the allocator's
   slot↔alloc map, rendered by tooling (the Cilium `lxc<hex>` + `cilium endpoint
   list` model) — a deliberate, accepted ergonomics shift. `derive_workload_netns_plan`
   is therefore PURELY slot-derived (`(slot, responder_addr) → WorkloadNetnsPlan`);
   the `alloc_id` parameter is DROPPED (it derives nothing once the netns name is
   slot-keyed). A **per-host `NetSlot` allocator** (assign-smallest-free at
   `on_alloc_running`, release at `on_alloc_terminal` — the C3 hook above) hands out
   the slot and holds the alloc↔slot binding; it is single-node trivial, NOT
   distributed IPAM and NOT the #167 VIP allocator. The converge model ALSO brings
   the **in-netns veth end up and netns `lo` up** (B2 — a veth forwards only when
   both ends are up; a fresh netns has `lo` down) and splits `rp_filter` relaxation
   into a global (`all`+`lo`) and a per-host-veth fact/step (S3). The exact
   `NetSlot` / `derive_workload_netns_plan` / `WorkloadVethStep` /
   `ObservedWorkloadVeth` surface is pinned in `feature-delta.md` § "D-TME-12 —
   pinned API contract" and `design/wave-decisions.md` D-TME-12.

2. **OUTBOUND interception = nft-TPROXY at the host-side veth (the active-side
   mirror of inbound).** The workload's outbound `connect()` leaves its netns
   via the veth, ingresses the host-side veth, and is nft-TPROXY-redirected to
   the agent's leg-F `IP_TRANSPARENT` listener. The agent recovers the original
   destination via `getsockname()` on the accepted leg-F socket — IDENTICAL to
   the inbound leg-C recovery (`accept_inbound_leg`). This REPLACES the
   `cgroup/connect4`-rewrite framing of ADR-0069 Decision fact 1 OUTBOUND.

3. **INBOUND interception = nft-TPROXY (UNCHANGED, ADR-0069 fact 1 INBOUND).**
   Already shipped + proven (increment-i; `install_inbound_tproxy`). Path A's
   egress rule joins the SAME shared `prerouting` chain + fwmark + table + F5
   exemption (#234 shared routing infra).

4. **Enrollment per-connection resolve replaces the per-destination map.** On
   each captured connection, after `getsockname`, the agent calls the new
   `MtlsResolve` port: `orig_dst → MtlsResolution`, a **3-variant sum type** (NOT
   a binary `Option`), filtered to `running` (read from `service_backends`):
   - **`Mesh(ResolvedBackend)` → ENFORCE** — `orig_dst` mapped to a `running`
     mesh backend. The worker sets `Routed::Outbound { peer: backend.addr }`
     (+ `expected_peer` when #178's join supplies it) and calls `enforce`.
   - **`NonMesh` → PASS-THROUGH** — the dialed dst is genuinely not a mesh peer;
     egress proceeds in cleartext, by design (the classification arm, not an
     error).
   - **`MeshUnreachable` → FAIL-CLOSED** — `orig_dst` should be a mesh peer but
     cannot be resolved/reached/validated; the worker refuses the connection,
     NO cleartext.

   A binary `Option` cannot distinguish `NonMesh` (non-mesh pass-through) from
   `MeshUnreachable` (should-be-mesh fail-closed) — the 3-variant type makes the
   Q3 "fail-closed, not silent-cleartext" decision structural (CLAUDE.md §
   "Type-driven design — sum types over sentinels"). See § "The new driven port"
   below for the pinned shape. This RETIRES `MTLS_REDIRECT_DEST`,
   `MtlsDataplane::{attach_alloc,program_redirect}`, and
   `program_declared_peer_redirect`.

5. **F5 intercept-recursion exemption, restated for Path A.** The agent's own
   leg-B (outbound) / leg-S (inbound) dials carry the `SO_MARK`
   (`MTLS_LEG_S_DIAL_MARK`) the shared chain's head-exemption accepts — so the
   agent's dials are NOT re-captured by the egress/ingress nft-TPROXY rules
   (the same agent-private bypass ADR-0069 F5 names, now governing the nft rule
   rather than the retired `cgroup_connect4` program). Two Tier-3 obligations
   (UNCHANGED): the agent's dial is not re-intercepted, AND a workload cannot
   self-exempt (the mark is agent-private, unreachable from the workload).

6. **The `cgroup_connect4_mtls` kernel-side program is RETIRED.** It was the
   last outbound kernel-side program; Path A adds NO kernel-side program (nft
   rules are `nft`/`ip` userspace installs). The `overdrive-bpf` outbound mTLS
   surface is deleted.

7. **The `MtlsEnforcement` port is UNCHANGED.** `enforce` still takes
   `Routed::Outbound { peer }`; Path A changes the worker's obtain-`peer` path
   (`getsockname` instead of the declared-peer slot). No new method, no new
   variant. (Designing a NEW port — `MtlsResolve` — for the resolve concern is
   the correct move; smuggling resolve into `MtlsEnforcement` is forbidden.)

### The new driven port — `MtlsResolve` (the #178 anti-corruption boundary)

A new driven port in `overdrive-core/src/traits/` carries the enrollment
resolve. The existing `MtlsEnforcement` does NOT fit (it is the per-connection
crypto/socket contract, frozen at 4 methods); the existing `Dataplane` does NOT
fit (map writes, not a resolve query).

**Shape — PINNED for DELIVER (C1 + C2; DESIGN-review handoff conditions, now
baked into the contract so a crafter implements-to-design and does not invent
the classification mapping or the struct shape).** The return is a **3-variant
sum type**, NOT a binary `Option` — a binary `Option<ResolvedBackend>` cannot
distinguish "non-mesh pass-through" from "unreachable-mesh fail-closed," which
is exactly the Q3 ambiguity the enrollment model removes (CLAUDE.md §
"Type-driven design — sum types over sentinels / make invalid states
unrepresentable"). Canonical names chosen for this design:

```rust
resolve(orig_dst: SocketAddrV4) -> Result<MtlsResolution>

pub enum MtlsResolution {
    Mesh(ResolvedBackend), // mesh        → ENFORCE
    NonMesh,               // non-mesh    → PASS-THROUGH (cleartext, by design)
    MeshUnreachable,       // should-be-mesh, unreachable/invalid → FAIL-CLOSED (no cleartext)
}

pub struct ResolvedBackend {           // bounded to EXACTLY two fields
    pub addr: SocketAddrV4,
    pub expected_svid: Option<SpiffeId>, // v1 adapter returns None (join = #178)
}
```

**C1 — the three arms (each rustdoc-pinned with its enforce/pass-through/
fail-closed semantic; reproduced verbatim on the trait method's rustdoc):**
- **`Mesh(ResolvedBackend)` → ENFORCE** — a `running` mesh backend resolved.
  The worker sets `Routed::Outbound { peer: backend.addr }` (+ `expected_peer`
  when #178 supplies it) and calls `enforce`. The only arm that drives a
  handshake.
- **`NonMesh` → PASS-THROUGH** — `orig_dst` is genuinely not a mesh peer; egress
  proceeds in cleartext, by design. The classification arm — not an error, not a
  fail-closed.
- **`MeshUnreachable` → FAIL-CLOSED** — `orig_dst` should be a mesh peer but
  cannot be resolved/reached, or its identity cannot be validated. The worker
  refuses; NO cleartext fallback. This is the silent-cleartext footgun the
  enrollment model exists to remove.

**C2 — `ResolvedBackend` bounded to exactly `{ addr, expected_svid }`:** no
third field. **The v1 `ServiceBackendsResolve` adapter returns
`expected_svid: None`** for every `running` backend — it is a SHELL that reads
`service_backends` filtered to `running` and does NOT join identity facts. The
expected-SVID join is **#178**; filling it in this feature's adapter is boundary
divergence across the anti-corruption boundary (consistent with Q4 / authn-only
v1). The field exists so the SAN-pin wires the moment #178 supplies the join.

**C4 — resolve READ MECHANISM: an in-RAM, address-keyed reverse index, NOT a
per-`ServiceId` point query.** ADR-0071 (and the feature-delta) pinned the
classification *model* (the 3 arms, C1) and the `ResolvedBackend` *shape* (C2)
but left the *read mechanism* — how the v1 `ServiceBackendsResolve` adapter maps
an arbitrary `orig_dst` to a backend — underspecified. The gap is concrete: the
only `ObservationStore` read surface for backends is keyed by `ServiceId`
(`service_backends_rows(service_id)`), while `MtlsResolve::resolve(orig_dst:
SocketAddrV4)` is handed an *arbitrary address* and holds **no `ServiceId`**.
There is no addr→service reverse index, no enumerate-all-services method, and no
`service_backends_by_addr`. A crafter left to improvise would either stall or
invent new `ObservationStore` trait API — a boundary-divergence rejection
(CLAUDE.md § "Implement to the design — never invent API surface"). This
sub-decision closes that gap and is pinned for DELIVER:

- **`ServiceBackendsResolve` resolves against an in-RAM, address-keyed,
  ownership-aware projection** of the `running` `service_backends` set, built via
  **List-then-Watch** over the `ObservationStore` and refreshed on incremental
  updates. `resolve()` is then a **point-lookup into the in-RAM index** — NOT a
  per-`ServiceId` store query. (The index is keyed so each contributing service's
  backend at an addr is tracked separately — `addr → {service → Backend}`, NOT a
  flat `addr → Backend` with global last-writer-wins eviction. See "F-A —
  ownership-aware index" below for why; the public `MtlsResolve` contract and the
  `NonMesh`/`MeshUnreachable`/`Err` classification arms are UNCHANGED by it.)
- **Read mechanism = List-then-Watch + relist-on-`Lagged` (REVISED 2026-06-17,
  reverses the prior "no new trait method" constraint; the relist-TRIGGER leg
  REFINED 2026-06-17 — see "F4 / relist-trigger refinement" below).** The
  original C4 pinned an *observe-only* mechanism (forward-only `subscribe_all()`,
  no bulk-load) and forbade any keyless enumerate surface. The
  resolve-index-coherence research
  (`docs/research/networking/transparent-mtls-resolve-index-coherence-research.md`,
  2026-06-17) established that observe-only-without-resync over a forward-only,
  bounded, lossy watch is **the one shape no production mesh uses** — #237
  cold-start, F2 concurrency, and F4 lag-drop are three faces of the same
  "coherent local cache over a lossy forward-only watch" problem whose textbook
  fix (Kubernetes informer, etcd watch, Envoy xDS, and Cilium's kvstore in
  production) is **List-then-Watch + relist-on-loss**. The revised mechanism:
  - **List-at-probe** — at `probe()` (the Earned-Trust gate), bulk-load the
    current `service_backends` snapshot into the in-RAM `addr → Backend` index
    **BEFORE the gate opens / before any `resolve` is served**, so the index is
    never empty-but-trusted (closes **#237** cold-start; mirrors Cilium's
    `ListDone`-gates-`synced`, research A5).
  - **Watch** — observe the NEW lag-surfacing subscription `subscribe_all_events()`
    (NOT the lossy `subscribe_all()`) for incremental updates. See "F4 /
    relist-trigger refinement" for why a new surface is required.
  - **relist-on-`Lagged`** — on a `SubscriptionEvent::Lagged { missed }` loss
    signal delivered by `subscribe_all_events()`, the adapter MUST re-List
    (re-acquire the authoritative snapshot and rebuild/merge the index); it MUST
    NOT silently discard the loss (closes **F4**; mirrors Cilium's
    `ErrCompacted → goto reList`, research A6; the tokio-idiomatic recovery,
    research B4).
  - **Concurrency = single-owner drain (no take-and-replace).** The subscription
    has ONE owner (a background drain task that exclusively owns the subscription
    and writes the index under the existing `RwLock`; per-connection `resolve`
    readers take the read lock). This dissolves the F2 take/restore TOCTOU by
    open-once / single-owner (per `development.md` § "Check-and-act must be
    atomic"). Mirrors Cilium's single-`RWMutex` reader/writer model (research A8).
- **This ADDS a keyless `ObservationStore` enumerate surface** (the List leg) AND
  a lag-surfacing subscription surface (the Watch leg's loss signal) — both
  symmetric with the EXISTING unkeyed enumerators `alloc_status_rows()` /
  `node_health_rows()` (research A2). **Pinned signatures:**
  ```rust
  // List leg — keyless enumerate. SHIPPED (commit 25e7acf3).
  async fn all_service_backends_rows(&self)
      -> Result<Vec<ServiceBackendRow>, ObservationStoreError>;

  // Watch leg — lag-surfacing subscription. NEW (this refinement, 2026-06-17).
  // A subscription item: an observation row, OR a gap signal telling the
  // consumer it missed `missed` rows and must re-List (the etcd-`ErrCompacted`
  // / k8s-`Gone` recovery contract). `Lagged { missed }` is a DOMAIN event —
  // the adapter maps `broadcast::RecvError::Lagged(n)` to it (n → missed); the
  // core trait NEVER names a tokio type.
  pub enum SubscriptionEvent {
      Row(ObservationRow),
      Lagged { missed: u64 },
  }
  pub type LagAwareSubscription =
      Box<dyn Stream<Item = SubscriptionEvent> + Send + Unpin>;
  async fn subscribe_all_events(&self)
      -> Result<LagAwareSubscription, ObservationStoreError>;
  ```
  `all_service_backends_rows` returns ALL LWW-winner `service_backends` rows
  across all services; its name MUST NOT collide with the existing keyed
  `service_backends_rows(&self, service_id: &ServiceId)` — `all_`-prefixed, and
  consistent with the `*_rows()`-no-arg convention. Both surfaces touch every
  `ObservationStore` implementor (`overdrive-store-local`, `overdrive-sim`, any
  test doubles) — *consistent* surface growth (they mirror the existing
  enumerators / subscription), not novel surface. **`subscribe_all_events()`
  (delivering `SubscriptionEvent`) is now the SOLE observation-subscription
  surface — the lossy `subscribe_all()` and the `ObservationSubscription` alias
  were DELETED single-cut in commit `36a79762`** (see "F-B reconciliation" in the
  refinement clause below for the dated history). The `ServiceId`-keyed
  `service_backends_rows` point query remains the WRONG surface for an arbitrary
  `orig_dst` (the adapter holds no `ServiceId`); the keyless List is the right
  one. The in-RAM index is still **adapter-internal**; the **PUBLIC `MtlsResolve`
  contract is UNCHANGED** (the trait signature, the 3-variant `MtlsResolution`,
  the `ResolvedBackend` shape, and the probe all stand exactly as C1/C2 pin
  them). The growth is confined to the `ObservationStore` driven port.
**C4 — F4 / relist-trigger refinement (REFINED 2026-06-17, ratified — option 2:
surface `Lagged` for event-driven relist).** The 2026-06-17 read-mechanism
revision above folded F4 in with the wording "relist-on-`Lagged` closes F4 now,"
which silently assumed the loss signal was already reachable on the watch
surface. Implementation proved that assumption FALSE: the watch surface is
`subscribe_all()`, whose item type is
`ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>` — it carries
**no loss signal** — and BOTH store adapters strip `broadcast::RecvError::Lagged`
*inside* `subscribe_all` before any consumer can see it (the sim adapter via
`ok_or_skip`; `overdrive-store-local` via `filter_map(Result::ok)` /
`filter_map`-on-`Lagged`). So wiring an event-driven relist requires **surfacing
the `Lagged` loss signal through a subscription API** — a deliberate
subscription-surface addition, NOT the zero-cost fold the prior wording implied.

The user has ratified **option 2 — surface `Lagged` for event-driven relist**
(chosen over periodic resync because the loss signal exists at the broadcast
layer, and reacting to it gives *completeness* — a dropped update is always
either delivered or signaled-then-relisted, never silently lost — which polling
cannot guarantee; this is the etcd-`ErrCompacted` / k8s-reflector-`Gone` canon
applied to the tokio `broadcast` channel).

What this refinement authorizes (the surface pinned above):

- A **lag-surfacing subscription method** `subscribe_all_events()` returning a
  `LagAwareSubscription` of `SubscriptionEvent`.
- `SubscriptionEvent::Lagged { missed }` is a **domain** event, NOT a tokio leak:
  the host/sim adapter maps `broadcast::RecvError::Lagged(n)` to it (`n → missed`)
  at the adapter boundary; `tokio::...::RecvError` never appears in the core trait
  (`development.md` § "Trait definitions specify behavior" — the contract is a
  domain vocabulary, not the transport's error type).

**F-B reconciliation — `subscribe_all` DELETED single-cut, superseding the
bounded-blast-radius framing (recorded 2026-06-17 as dated history, not a silent
overwrite).** When this refinement was authored (commit `36652ace`), it justified
`subscribe_all_events()` as a *dedicated method* whose point was to **bound blast
radius** — keeping the lossy `subscribe_all()` + `ObservationSubscription` alias
in place AS-IS for their "~20 existing consumers (DST invariants, store
test-harness, streaming)," so only `ServiceBackendsResolve` would consume the new
surface and the migration would touch one consumer. **The very next commit
`36a79762` did the opposite, and that single-cut is the SHIPPED, intended state:
`subscribe_all` and the `ObservationSubscription` alias were DELETED outright and
ALL ~20 consumers were migrated to `subscribe_all_events()`** (which now yields
`SubscriptionEvent`). Keeping a lossy `subscribe_all()` beside the lag-aware
surface would have been exactly the deprecated-parallel-path anti-pattern the
project forbids (`feedback_single_cut_greenfield_migrations` /
`feedback_delete_dont_gate`: removed is removed; no parallel old path). So:
`subscribe_all_events()` is now the **SOLE** observation-subscription surface; the
"~20 consumers stay untouched / bounded blast radius / not a shared-type change"
rationale was a **point-in-time decision the subsequent single-cut superseded**,
and is preserved above only as honest history. There is no remaining "migrate the
other consumers" follow-up — that work is DONE (`36a79762`), not deferred.

The relist TRIGGER is then concrete: the single-owner drain consumes
`subscribe_all_events()`; on `SubscriptionEvent::Row(row)` it applies the row to
the index; on `SubscriptionEvent::Lagged { missed }` it re-Lists via the
already-shipped `all_service_backends_rows()` (commit `25e7acf3`) and
rebuilds/merges the index — the `relist()` machinery already exists and is
already exercised at probe and on watch-close; this leg just wires its TRIGGER.
F4 is thereby closed with the *completeness* guarantee. The rest of C4
(List-at-probe, single-owner drain, the classification split, the (a)
fail-toward-handshake invariant for the irreducible window) is intact and
unchanged.

- **Classification maps onto the index** consistently with C1 and the shipped
  01-01 port rustdoc (`crates/overdrive-core/src/traits/mtls_resolve.rs`) — this
  decision ADDS the read mechanism, it does NOT re-classify:
  - `orig_dst` **hits** a `running` mesh backend in the index →
    `Mesh(ResolvedBackend { addr, expected_svid: None })` (v1 `expected_svid` is
    `None` for every backend — the identity join is #178; C2).
  - `orig_dst` **misses** (no `running` mesh backend), index readable → `NonMesh`
    (cleartext pass-through, by design). **A miss is `NonMesh`, NOT
    `MeshUnreachable`.**
  - A matched backend is **present-but-unreachable** / its required identity
    facts are absent → `MeshUnreachable` (fail-closed, no cleartext). A
    **store-layer read fault** (poisoned handle, corrupt table, errored
    subscription) surfaces per the shipped 01-01 error split — `Err(MtlsResolveError::StoreUnreadable)`
    for a store-layer fault that is NOT a per-connection classification; the
    per-connection should-be-mesh-but-can't outcome is `MeshUnreachable`. (This
    preserves the 01-01 rustdoc's asymmetry verbatim; C4 does not alter it.)
- **v1 `orig_dst == backend addr` (no VIP→backend translation).** In headless v1
  (D-TME-10) the addr DNS returns IS the backend addr, so the in-RAM index is
  keyed by the backend addr **directly** — there is NO VIP→backend translation
  in the resolve path (that is #167/#61, out of scope here; one source, two
  readers — the DNS-returned `service_backends` addr is the same addr the index
  is keyed by).

**C4 — F-A: ownership-aware index (ratified 2026-06-17 — option (b)).** A flat
`addr → Backend` index with global last-writer-wins eviction would rely on an
**unstated cross-component invariant** — *"a given `(IP:port)` belongs to at most
one service"* — on the silent-cleartext boundary. That invariant *does* hold
structurally in v1 (`Backend.addr` IS the alloc's serving addr; each alloc is one
workload's replica → in exactly one service; one listener per `(IP:port)`), but a
correctness property that leans on an unstated cross-component invariant — and a
last-writer-wins eviction that is non-deterministic when two services disagree
about an addr's health — is a smell on this boundary. The index is therefore
**ownership-aware**, so it does not rely on addr-exclusivity:

  - **Keyed so each contributing service's backend at an addr is tracked
    separately** (e.g. `addr → {service → Backend}`), NOT a single `Backend` per
    addr with global LWW eviction.
  - **A service's backend-set shrink evicts only THAT service's contribution** at
    the addr; an addr stays resolvable as long as **any** service still claims a
    healthy backend there.
  - **Classification is `any-healthy-at-addr`** — deterministic, NOT
    last-writer-wins: `orig_dst` hits `Mesh` iff some service still claims a
    `running`/healthy backend at that addr.

  This makes the index correct **independent** of addr-exclusivity, removing the
  unstated cross-component invariant and the last-writer-wins healthy-disagreement
  determinism smell. v1 single-node is structurally addr-exclusive (so the
  per-addr service set is size-1 today); the ownership-aware shape is **defensive
  against multi-node / future writers**, not a change to today's observable
  behaviour. The structure is an **adapter-internal detail** — the public
  `MtlsResolve` contract and the `NonMesh`/`MeshUnreachable`/`Err` classification
  arms (C1, above) are **UNCHANGED**.

**C4 — miss-classification scoping note (the load-bearing new contract clause).**
A pure `running`-backends reverse index cannot, *on a miss*, distinguish a
"genuinely external addr" (→ `NonMesh`, correct) from a "should-be-mesh addr the
index has not yet converged on" (→ would-be `MeshUnreachable`). **v1
deliberately classifies a miss as `NonMesh`.** With List-then-Watch +
relist-on-`Lagged` closing the cold-start (#237) and lag (F4) windows, the
residual exposure shrinks to the **irreducible convergence window** — a backend
that came up microseconds ago, its `service_backends` row still in flight. The
richer **fail-toward-handshake** miss semantic (treat an un-converged miss as
`MeshUnreachable` rather than `NonMesh`) covers that residual and is
**#236-coupled** (it depends on the agent being able to *attempt* a handshake on
an ambiguous miss — the ztunnel-shaped capture-all path, which is multi-node-
shaped and not yet in tree). An implementer **MUST NOT** make a miss fail-closed
in v1 — that would break legitimate external / non-mesh egress, which is the
entire purpose of the `NonMesh` arm.

**C4 — (a) fail-toward-handshake as the stated v1 SECURITY invariant (added
2026-06-17).** Record the contract the miss-classification must eventually
satisfy: ***a resolve miss must never silently emit cleartext to a should-be-mesh
peer.*** This is the **miss-meaning** lever (orthogonal to the **coherence**
lever the List-then-Watch mechanism above engages); it is the posture every
production mesh that is safe on this boundary adopts — Cilium fail-CLOSES an
auth-required flow on an auth-map miss (`DROP_POLICY_AUTH_REQUIRED`,
`bpf/lib/auth.h:45-53`, research A9); ztunnel drops on a missing source workload
(research B5). For single-node v1 with the full coherence fix above, the residual
irreducible window is **local and bounded** and is the accepted v1 posture; the
CODE that realizes (a) (the capture-all handshake-attempt path) lands under
[#236](https://github.com/overdrive-sh/overdrive/issues/236) — so (a) is recorded
here as the contract, with its closure tracked at #236. (a) is the load-bearing
backstop: even a perfectly coherent cache still leaks during its own irreducible
convergence window unless what a miss MEANS is changed; that is why (a) is the
security floor and List-then-Watch is the coherence hardening, not vice versa.

**C4 — #237 disposition (corrected 2026-06-17).** The List-at-probe leg seeds the
index before the Earned-Trust gate opens, so on a control-plane restart the
boot-time List captures any pre-existing `service_backends` rows (gossiped or
persisted) and the index is never empty-but-trusted during convergence;
relist-on-`Lagged` closes the lag window for single-node AND multi-node burst.
**#237 (the cold-start restart-window instance) is CLOSED by this revision** (the
GH issue is closed once the crafter lands the List-at-probe + relist mechanism —
not closed in this doc edit). The only residual is the irreducible convergence
window above → (a) / #236. **This corrects the prior wording**, which described
the v1 implementation as a *forward-only `subscribe_all()` index with no
`service_backends` bulk-load* and accepted the empty-on-restart window as a
tracked-but-open v1 edge — that mechanism is the one this revision replaces.

The in-house **"bulk-load-then-observe"** precedent the original C4 text cited as
justification (the reconciler-runtime `ViewStore` `bulk_load` + `write_through`,
`development.md` § "Reconciler I/O") is itself List-then-Watch over an
*authoritative* surface — its `bulk_load` IS the List leg (research A4 /
Conflict 2). The original C4 invoked that precedent's name while dropping its
load-bearing half (the authoritative snapshot/List), shipping observe-only. The
corrected reading: the precedent argues **FOR** List-then-Watch — which is now
what ships — **against** observe-only.

**Evidence (this read mechanism is the industry-canonical shape, not a
convenience):**
- **Cilium is the canonical implementation of exactly this pattern.** Its
  `ipcache` (`pkg/ipcache/ipcache.go` `ipToIdentityCache`; `LookupSecIDByIP`) is
  an in-RAM, **address-keyed reverse index** from IP → identity, populated by
  *subscribing* to endpoint/CIDR/node/FQDN allocation events (not by
  point-querying a service store per connection) and mirrored to a read-only BPF
  LPM trie consulted inline per connection. Its kvstore watcher does
  **List-before-Watch** with a one-time `ListDone` signal gating a `synced` flag
  (`pkg/kvstore/etcd.go`, `pkg/kvstore/store/watchstore.go`) and **relists on
  `ErrCompacted` or any watch error** (`goto reList`) with stale mark/sweep — the
  cache is never empty-but-trusted and never strands a stale entry (research
  A5–A6). This is precisely the List-then-Watch + relist-on-loss mechanism C4 now
  pins. Cilium also splits `addr→identity` (ipcache) from `identity→peer-material`
  (auth map / SVID store) — which independently validates v1's
  `expected_svid: None` two-stage deferral of the identity join to #178.
- **Our own interception research already states this in words.**
  `docs/research/networking/transparent-mtls-interception-mechanism-2026-research.md`
  §4.1 (refuting the "per-connection resolve is a bottleneck" attack): the
  resolve is "a local in-memory `service_backends` lookup (Corrosion-gossiped,
  already in RAM per the reconciler-runtime bulk-load model) — no xDS round-trip,
  no network hop." That is an in-RAM lookup, NOT a per-service point query.
- `docs/research/networking/stable-service-naming-and-transparent-mtls-comprehensive-research.md`
  (resolve-then-pin pattern; SPIFFE identity / naming split) corroborates the
  two-stage shape (addr→backend, then backend→identity at #178).

- **This feature owns**: the port trait + the 3-variant `MtlsResolution` type +
  the 2-field `ResolvedBackend`, a v1 host adapter (`ServiceBackendsResolve`)
  reading `service_backends` via `ObservationStore`, a sim adapter, and the
  fail-closed semantic + Earned-Trust probe (a resolve adapter that cannot read
  the store refuses boot with `health.startup.refused` — it does NOT silently
  return empty / `NonMesh`, which would re-introduce the silent-cleartext
  footgun). The adapter classifies the store-read outcome into the type: a
  no-`running`-mesh-backend lookup is `NonMesh`; a store-read failure at resolve
  time, or a present-but-unreachable mesh backend, is `MeshUnreachable`.
- **#178 owns** (NOT designed here): the expected-SVID join (`service_backends`
  × identity facts), the multi-backend candidate-set + LB-pick policy, the
  SAN-match wiring of `expected_peer` (so v1 returns `expected_svid = None`,
  authn-only, consistent with ADR-0069).
- **#61 owns** (NOT designed here): the VIP/DNS name → virt resolution upstream
  of `orig_dst` (the responder *daemon*); this ADR designs only the *integration*
  of that responder into the per-workload netns (Q5a, next section).

### Name-layer integration (Q5a) — node-local DNS responder injected into the per-workload netns

Q5a folds the **integration** of the DNS name layer into this design (NOT the
#61 daemon implementation, NOT the #167 VIP allocator). Path A's per-workload
netns (Q2) IS the injection point.

8. **Injection mechanism = resolv.conf injection (the Fly.io `fdaa::3` model).**
   When the Q2 provisioner creates a per-workload netns, it writes the
   **node-local DNS responder's address** into that netns's own `/etc/resolv.conf`
   (a per-netns mount, the stock `ip netns` convention). The workload's libc
   `getaddrinfo("<job>.svc.overdrive.local")` then reaches the node-local
   responder with **zero app config** — Fly.io's documented model ("we inject the
   IP of that DNS server into your `resolv.conf` … always `fdaa::3`"). Overdrive
   ships its own appliance OS (ADR-0068), so it can do this for every workload
   netns sidecarlessly. **THIS feature owns** the injection step (one idempotent
   converge step on the Q2 provisioner). **#61 owns** the responder daemon.

9. **DNS-return shape = HEADLESS for v1 (recommended; the single open item).**
   What the responder returns IS the `orig_dst` that `MtlsResolve.resolve` later
   recognizes (fact 4) — the two contracts MUST be consistent. v1 returns a
   **`running` backend addr straight from `service_backends`** (headless /
   endpoint-set, K8s-headless + Fly.io-`.internal` shaped), NOT a per-service VIP.
   Consequences that make this the v1 choice:
   - **Single source, two readers.** DNS reads `service_backends`; `MtlsResolve`
     reads `service_backends`. `orig_dst` is byte-consistent by construction — no
     translation layer between name and resolve.
   - **No new v1 dependency.** Headless needs no VIP allocator. A VIP return
     would pull **#167** (allocate `fdc2::/16` virts) into v1 PLUS the VIP×
     intercept ordering hazard (research §3.3 R5 / §3.5 Q1).
   - **Keeps `MtlsResolve` v1 honest with the #178-deferred LB boundary.** v1
     resolve is an identity-only lookup (addr → expected_svid; `expected_svid =
     None` until #178). A VIP return would force the resolve port to own a
     VIP→backend LB-pick — the multi-backend policy this ADR explicitly defers to
     #178 (Q4).
   - **Single-node v1 makes VIP indirection valueless** (all backends local —
     research §1.6).
   - **Forward-compatible.** Multi-node can add a VIP shape *alongside* headless
     (K8s ships both); `MtlsResolve` later gains a VIP-recognizing arm fed by #167
     + the XDP `SERVICE_MAP` LB without reworking the v1 enforce path. Headless is
     not a dead end.

   **Sign-off note:** headless adds nothing new and is the zero-new-dependency
   default. Choosing **VIP** instead would add #167 to v1 scope and needs explicit
   user sign-off — flagged, not assumed.

10. **End-to-end coherence (name → resolve → enforce), v1 headless.** The flow
    `workload getaddrinfo("<job>.svc.overdrive.local") → DNS-in-netns (returns a
    running service_backends addr B) → connect(B) → veth-egress → host-veth
    ingress → nft-TPROXY → leg-F → getsockname(orig_dst = B) → MtlsResolve(B) →
    enforce mTLS to B` is coherent: B is recognized by `MtlsResolve` precisely
    because DNS returned the same `service_backends` addr. The C4 Container
    diagram (brief.md §35) shows this end-to-end. **Scope boundary kept**: the
    responder daemon (#61) and the VIP allocator (#167) are named dependencies,
    not builds in this feature — only the injection + the return-shape contract
    alignment + the `MtlsResolve` composition live here.

## Alternatives Considered

### A1. `cgroup/connect4` cookie-stash + `cgroup/getsockopt(SO_ORIGINAL_DST)`-revert

The smallest-delta-to-existing-code mechanism the research recommended (reuse
the existing outbound hook; stash orig-dst in `bpf_sk_storage`; answer it on
`getsockopt`). **Rejected**: the spike proved it DOESN'T-WORK on the appliance
kernel — three independent fatal walls (connect-before-bind; non-DNAT rewrite →
conntrack `ENOENT`; getsockopt-hook scoped to the in-cgroup caller, not the
out-of-cgroup agent). Confirmed against Cilium's production tree (zero
`SO_ORIGINAL_DST`). This is the decisive evidence; it is not a preference.

### A2. `bpf_sk_assign` TPROXY (Cilium's exact mediating-proxy primitive)

Cilium's mediating proxy uses `bpf_sk_assign` + fwmark → `IP_TRANSPARENT` →
`getsockname`. **Rejected for us**: it adds a new BPF program surface we have no
in-tree precedent for. nft-TPROXY beats `bpf_sk_assign` *for Overdrive* not
because the kernel primitive is better, but because we ALREADY run nft-TPROXY
inbound — Path A unifies both directions on the one mechanism we have already
proven and shipped. (The recovery primitive — `getsockname` — is identical
either way.)

### A3. Keep the per-destination `MTLS_REDIRECT_DEST` map (the single-node bridge)

The shipped single-node shape: a `cgroup/connect4` rewrite gated on a
programmed per-destination map. **Rejected**: its miss = silent cleartext (the
footgun the enrollment model exists to remove), it has a cardinality cost at
mesh scale, and its recovery half is the now-falsified Probe A. The research
correctly classed it as a single-node bridge only.

### A4. netkit-device redirect

A veth-replacement datapath device (kernel ≥6.7). **Rejected as the
interception answer**: netkit is a *datapath performance* device, not an
interceptor — it does not rewrite `connect()` nor recover orig-dst for an
L7-terminating proxy (research §3.2; Cilium docs verbatim "concentrates on
datapath performance, not traffic interception"). It is an ORTHOGONAL, additive
win Overdrive may adopt for its veths independently — NOT this feature.

### A5. DNS returns a per-service VIP (the ClusterIP / #61-#167 shape) — rejected FOR v1

For the name-layer integration (Q5a), the alternative to headless: the node-local
responder returns a stable per-service VIP (`<job>.svc.overdrive.local →
fdc2:…::N`), and `MtlsResolve` (or the XDP `SERVICE_MAP`) LB-picks a backend.
**Rejected for v1** (not rejected forever): it pulls **#167 (VIP allocator)** into
v1 scope as a NEW hard dependency, plus the VIP×agent-light-intercept ordering
hazard (research §3.3 R5 / §3.5 Q1 — the trickiest wiring, needing its own Tier-3
ordering spike); it forces the v1 resolve port to own a VIP→backend LB-pick that
Q4 explicitly defers to #178; and single-node v1 (all backends local) gets zero
value from VIP indirection. VIP is the cleaner **multi-node** stable-handle UX
and can be added alongside headless later (K8s ships both) without reworking the
v1 enforce path. (See § "Name-layer integration (Q5a)" fact 9.)

## Consequences

### Positive

- **One proven interception mechanism, both directions.** Outbound mirrors the
  shipped+proven inbound nft-TPROXY+`getsockname`. No `connect4`, no
  `bpf_sk_assign`, no per-destination map.
- **No silent-cleartext-on-miss.** The enrollment resolve classifies each
  connection into the 3-variant `MtlsResolution` (C1): `Mesh` → mTLS, `NonMesh`
  → pass-through, `MeshUnreachable` → fail-closed. The empty-map cleartext
  footgun is gone, and the distinction is structural in the type — a binary
  `Option` could not carry it.
- **Net deletion of kernel surface.** The `cgroup_connect4_mtls` program, the
  `MTLS_REDIRECT_DEST` map, the `MtlsDataplane` outbound attach/program surface,
  and the `program_declared_peer_redirect` stand-in are all removed.
- **The enforcement contract is untouched.** 4-method `MtlsEnforcement`,
  agent-light pumps, kTLS substrate, probe sentinel, supervision (ADR-0070) all
  carry forward verbatim.
- **CNI-aligned topology** sets up the multi-node / guest-stack future without a
  second mechanism (the workload-in-netns shape the industry converged on).
- **The name layer composes for free at the same injection point.** The
  per-workload netns (needed for Path A interception) is ALSO the DNS injection
  point — one topology, two wins. The headless v1 return shape (a
  `service_backends` addr) keeps DNS and `MtlsResolve` on one source with no
  translation, and pulls NO new v1 dependency (no VIP allocator). Any unmodified
  workload reaches `<job>.svc.overdrive.local` encrypted, zero app changes.

### Negative

- **Per-workload netns+veth is new lifecycle surface.** v1 single-node gains a
  per-alloc netns+veth provisioner (vs today's single host-netns veth). Shape =
  extend `veth_provisioner` (Q2 ratified); the converge-on-boot template exists.
- **Egress-on-per-workload-veth nft-TPROXY is UNVALIDATED on our topology.** No
  Tier-2 backstop → Tier-3 only. The novel risk is an egress route / `ip rule`
  / F5-exemption collision (the research's Probe B falsification path). Q1
  ratified: validate via a thin Tier-3 spike (`increment-b/`) NOW, before DELIVER.
- **One per-connection resolve lookup** added to the capture path (in-RAM
  `service_backends`; negligible per research §4.1, but a new dependency on the
  resolve port's availability — mitigated by the resolve probe refusing boot on
  an unreadable store).
- **Back-propagation to ADR-0069 and `veth_provisioner.rs`.** See § Changed
  assumption. The architect amends ADR-0069's outbound framing via this ADR;
  `jobs.yaml` re-grounding (if any) is flagged for the product-owner, not
  edited here.
- **The name-layer integration depends on an unbuilt responder daemon (#61).**
  This ADR designs the injection + the return-shape contract, but the process
  that answers `<job>.svc.overdrive.local` by reading `service_backends` is the
  #61 build. v1 enforcement (resolve port + interception) does NOT block on #61 —
  `MtlsResolve` works against any `orig_dst` the workload connects to (responder,
  hard-coded addr, or future VIP); the name layer is the ergonomic front-end, not
  a correctness dependency.
- **One open recommendation remains (DNS-return shape).** The headless-vs-VIP
  call (§ Name-layer fact 9) is recommended-headless and adds no new v1
  dependency, so it is not a blocker; a VIP choice would add #167 and needs
  explicit sign-off.

### Changed assumption (back-propagation)

**Quoted original** (`veth_provisioner.rs:36-37`): "Single-node runs entirely in
the host netns — there is no netns machinery here." **New**: per-workload
netns+veth. **Rationale**: TPROXY+`getsockname` (the only proven recovery) needs
the workload's egress to traverse an agent-controlled routing point — a
per-workload netns+veth. **Affected**: ADR-0069 OUTBOUND framing (amended by
this ADR); the `cgroup_connect4_mtls` program (retired); the F5 exemption (now
governs the nft egress rule, not the `cgroup_connect4` program). **Name-layer
corollary (Q5a)**: the same per-workload netns becomes the DNS injection point
(resolv.conf injection), so the host-netns retirement also unlocks per-workload
node-local DNS — a corollary win, not a new assumption.

## Ratified sub-decisions (Q1–Q4)

The four prior open sub-decisions are RATIFIED (user, 2026-06-16) and are now
part of the Decision. The feature-delta § "Resolved sub-decisions" carries the
full rationale.

- **Q1 (RATIFIED) = thin Tier-3 spike NOW (`increment-b/`)**. Validate Path A
  egress nft-TPROXY + `getsockname` orig-dst recovery on our exact topology
  *before* DELIVER. It is the single novel, no-Tier-2-backstop piece; the
  cheapest place to find an `ip rule`/route/F5-exemption collision (the
  research's Probe B falsification path). The spike is gitignored
  (`spike-scratch/increment-b/`), zero `crates/` touch, real-kernel Lima.
- **Q2 (RATIFIED) = extend `veth_provisioner`** for per-workload netns+veth
  (parameterize the existing pure-derive + idempotent converge-on-boot shape
  per-alloc; add a netns-create step; lifecycle driven by the action-shim —
  **provision at `on_alloc_running`, BEFORE `MtlsInterceptWorker::start_alloc`
  and BEFORE `start_alloc`/`Driver::start`** (call site PINNED, C3), teardown on
  terminal). A per-alloc network reconciler is the **Bar-2 promotion when runtime
  drift matters** (the #197/#234 host-infra-reconciler family — `reconcilers.md`).
  Driver-creates is REJECTED: the `ExecDriver` setns hook ENTERS an existing
  netns, never creates one (CNI-aligned, `driver.rs:190-197`).
- **Q3 (RATIFIED) = `MtlsResolve` port in `overdrive-core` + a v1
  `service_backends`-reading host adapter**; **fail-closed (not silent)** at the
  boundary. A resolve adapter that cannot read the store refuses boot
  (`health.startup.refused`); it does NOT silently return empty (silent-empty =
  the silent-cleartext footgun the enrollment model exists to remove). #178 owns
  the expected-SVID join + multi-backend LB-pick; #61 owns the name layer.
- **Q4 (RATIFIED) = BOTH directions in v1**; intended-peer SVID pinning
  (`expected_peer`/`PeerIdentityMismatch`) **deferred to #178** (v1 = authn-only,
  chain-to-bundle). Path A's point is symmetry on one mechanism; the inbound
  nft-TPROXY install is the proven template the outbound mirrors. The resolve
  port carries `expected_svid` so the pin wires the moment #178 supplies the
  join; docs/tests MUST NOT call the wrong-but-valid-peer case "protected" until
  #178.

## Enforcement

- **Architectural rule (ArchUnit-style, Rust)**: the nft-TPROXY / `ip` install
  surface lives in `overdrive-worker/src/mtls_intercept.rs` (both directions);
  the netns+veth provisioning AND the resolv.conf injection in
  `overdrive-control-plane/src/veth_provisioner.rs` (Q2 ratified — extend the
  existing provisioner); the resolve port in `overdrive-core` (no I/O), its host
  adapter in an `adapter-host` crate, its sim adapter in `overdrive-sim`.
  `dst-lint` keeps all of it off any `core`-class compile path.
- **Earned-Trust probes (principle 12, mandatory)**:
  - `MtlsEnforcement::probe` (kTLS-arm forward-encrypt round-trip) — UNCHANGED
    (ADR-0069).
  - `MtlsResolve::probe` (NEW) — reads the `service_backends` surface and
    refuses boot (`health.startup.refused`) on an unreadable store; NEVER
    silently returns empty (silent-empty = the silent-cleartext footgun).
  - The netns+veth provisioner's converge-on-boot refuses boot on non-benign
    failure (the existing `veth_provisioner` precedent) — that IS its probe. The
    resolv.conf injection step (Q5a) is part of the same converge — a netns whose
    resolv.conf cannot be written refuses on the same boot path.
- **Tier-3 obligations (no Tier-2 backstop for the interception path)**:
  - OUTBOUND egress nft-TPROXY on a per-workload veth recovers orig-dst via
    `getsockname` (the Q1 validation; Cilium proves the model, our wiring is
    unproven).
  - F5: the agent's leg-B dial is NOT re-captured by the egress rule; a workload
    cannot self-exempt.
  - The composed bidirectional walking skeleton in the real netns/veth topology
    (ADR-0069 Slice 00, now on the Path-A mechanism — no RST post-arm, both
    directions, normal + traced timing).
  - Enrollment: a captured connection resolving to a `running` mesh backend
    drives mTLS to that backend; a connection resolving to no mesh peer is
    classified (pass-through/fail-closed per Q3, fail-closed ratified); NEVER
    silent cleartext to a should-be-mesh peer.
  - Name→resolve→enforce consistency (Q5a, DELIVER — needs the #61 responder or a
    test stand-in): a workload `getaddrinfo("<job>.svc.overdrive.local")` against
    the injected resolv.conf returns a `running` `service_backends` addr B
    (headless), and `MtlsResolve.resolve(getsockname-recovered B)` returns
    `Some{B, …}` — i.e. the DNS-returned addr IS the addr the resolve port
    recognizes (the single-source invariant). Until #61's responder lands, this
    is exercised with the DNS step stubbed (the workload connects to a known
    `service_backends` addr directly) so the resolve-recognizes-orig_dst half is
    validated independently of the responder.
- **Authn-only v1 boundary (UNCHANGED, F1/F5/#178)**: chain-to-bundle authn +
  encryption; `expected_peer` stays `None` until #178's join supplies it; docs
  and tests MUST NOT call the wrong-but-valid-peer case "protected" until #178.

## References

- Spike: `docs/feature/transparent-mtls-enrollment/spike/{wave-decisions,findings}.md`
  (Probe A DOESN'T-WORK; Path A; Cilium `main @ dac977e678`).
- Research: `docs/research/networking/transparent-mtls-interception-mechanism-2026-research.md`
  (enrollment model confirmed; Probe B = Path A);
  `docs/research/networking/stable-service-naming-and-transparent-mtls-comprehensive-research.md`
  (name/resolve/enforce; `service_backends`; the `{addr, expected_svid}`-filtered-to-`running` resolve contract;
  **§1.5 name layer / no-DNS-responder gap; §2.5 K8s ClusterIP-VIP vs headless fork; §2.7 Fly.io `fdaa::3` resolv.conf-injection model; §3.3 R3/R4/R5 DNS-responder placement; §3.5 Q1 VIP×intercept ordering — the Q5a name-layer-integration inputs**).
- Amends ADR-0069 (universal agent-light L4 proxy — outbound framing).
  Refined by ADR-0070 (liveness, UNCHANGED). Built on ADR-0068 (pinned 6.18 kernel).
- Code anchors: `overdrive-worker/src/mtls_intercept.rs` (inbound TPROXY +
  `getsockname`, the EXTEND base), `mtls_intercept_worker.rs` (per-alloc
  lifecycle; declared-peer seam to retire), `overdrive-dataplane/src/mtls/inbound.rs`,
  `overdrive-control-plane/src/veth_provisioner.rs:36-37` (host-netns claim
  superseded), `overdrive-worker/src/driver.rs:181-198` (setns hook),
  `overdrive-core/src/traits/mtls_enforcement.rs` (4-method port, unchanged).
- Named dependencies (NOT designed here):
  [#178](https://github.com/overdrive-sh/overdrive/issues/178) (east-west
  SPIFFE-ID resolution — the expected-SVID join + SAN-match),
  [#61](https://github.com/overdrive-sh/overdrive/issues/61) (VIP/DNS name layer
  — the **DNS responder daemon**; Q5a integrates its injection + return shape, it
  does not build the daemon),
  [#167](https://github.com/overdrive-sh/overdrive/issues/167) (VIP allocator —
  NOT a v1 dependency under the headless return shape; enters only with the
  multi-node VIP evolution / a VIP return choice),
  [#234](https://github.com/overdrive-sh/overdrive/issues/234) (shared TPROXY
  routing infra Bar-2 reconciler), [#236](https://github.com/overdrive-sh/overdrive/issues/236)
  (this feature), [#26](https://github.com/overdrive-sh/overdrive/issues/26)
  (transparent kernel mTLS).
