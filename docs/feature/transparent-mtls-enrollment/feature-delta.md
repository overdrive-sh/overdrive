# Feature-delta — transparent-mtls-enrollment (GH #236)

**Wave: DESIGN** · Tier-1 `[REF]` · Mode: propose · Paradigm: OOP · Architect: Morgan

This feature is the **ENFORCE / interception layer** for east-west transparent
mTLS under the **enrollment / capture-and-resolve** model (#236), built on
**Path A** (the spike's ratified direction). It does NOT design the resolve
*primitive* (#178) or the *name-layer daemon* (#61) — it defines the **boundary
contract** with them and consumes them per-connection.

**Status (2026-06-16)**: Q1–Q4 **RATIFIED** by the user (Q1 = spike now; Q2 =
extend `veth_provisioner`; Q3 = `MtlsResolve` port + `service_backends` shell,
fail-closed; Q4 = both directions, intended-peer pinning deferred to #178). Q5a
**chosen**: the DNS name-layer **integration** is folded into THIS design (the
name-layer daemon and the VIP allocator remain separate builds — only the
integration shape lives here). See § "Name-layer integration (Q5a)" and the
single remaining open item in § "Open questions" (the DNS-return shape
recommendation — headless — is stated there for sign-off, not a blocker).

---

## [REF] Prior-wave confirmation checklist

| Input | Read | Verdict it carries |
|---|---|---|
| `spike/wave-decisions.md` | ✓ | Probe A verdict **DOESN'T-WORK**; Cilium reconciliation (`main @ dac977e678`); chosen direction **Path A** (netns+veth + nft-TPROXY both directions); per-destination map **RETIRED**; enrollment model confirmed. |
| `spike/findings.md` | ✓ | Real-kernel evidence (Lima 7.0): `connect4`+`getsockopt(SO_ORIGINAL_DST)` = `ENOENT`; three independent fatal walls; recommends TPROXY+`getsockname` (the proven inbound mechanism). |
| research `…transparent-mtls-interception-mechanism-2026…` | ✓ | Enrollment *model* confirmed (Istio+Cilium converge); recommended Probe A (cookie-stash) — **superseded** by the spike's DOESN'T-WORK verdict; its **Probe B (TPROXY-outbound) fallback IS Path A**. |
| research `…stable-service-naming-and-transparent-mtls…` | ✓ | name/resolve/enforce layering; `service_backends` (BUILT) data source; the `{(backend_addr, expected_svid)}`-filtered-to-`running` resolve contract (#178, NOT built); `expected_peer`/`PeerIdentityMismatch` reserved. |
| ADR-0069 (agent-light L4 proxy) | ✓ | Bidirectional agent-light proxy SHIPPED; OUTBOUND framed on `cgroup/connect4`-rewrite — **the framing Path A amends**; 4-method `MtlsEnforcement` port; inbound nft-TPROXY+`getsockname` proven (increment-i). |
| ADR-0070 (liveness) | ✓ | (C) kernel TCP timeouts + (B) per-connection self-supervision; central enumerator retired; 4-method port unchanged. |
| ADR-0068 (pinned kernel) | ✓ | Pinned 6.18 LTS is the authoritative merge signal; dev Lima 7.0; TPROXY/`IP_TRANSPARENT`/`getsockname` far under any floor. |
| `brief.md` (SSOT) | ✓ | `## Application Architecture` has no transparent-mTLS subsection yet → append (§35, this feature). |
| Code anchors (`mtls_intercept.rs`, `mtls_intercept_worker.rs`, `mtls/inbound.rs`, `veth_provisioner.rs`, `driver.rs`, `mtls_enforcement.rs`) | ✓ | EXTEND targets confirmed live; see Reuse Analysis. |

⊘ DISCUSS wave — skipped (no `jobs.yaml`/`requirements.md`); spike artifacts ARE the requirements per dispatch.
⊘ DISTILL — not run (this dispatch ends at DESIGN).

---

## [REF] SETTLED (designed around; NOT relitigated)

- **Path A interception** = per-workload **netns+veth** + **nft-TPROXY + `IP_TRANSPARENT` + `getsockname` for BOTH directions**. Workload egress *ingresses* the host-side veth (PREROUTING) where nft-TPROXY applies → leg-F → `getsockname` recovers orig-dst — **symmetric with the proven inbound half**.
- **Retired**: `cgroup/connect4`-rewrite + cross-socket `SO_ORIGINAL_DST` recovery (spike DOESN'T-WORK); the per-destination `MTLS_REDIRECT_DEST` map; the test-only `program_declared_peer_redirect` seam; the `MtlsDataplane` outbound attach/`program_redirect` surface.
- **Interception model** = enrollment / capture-and-resolve: per-connection `getsockname` → resolve(orig_dst) → backend+identity. NOT a per-destination map.
- **Enforcement substrate** (ADR-0069) = agent-light kTLS proxy, UNCHANGED. The 4-method `MtlsEnforcement` port (`probe`/`enforce`/`liveness`/`teardown`) is UNCHANGED — Path A changes how the worker *obtains* `Routed::Outbound { peer }` (now `getsockname`, not a declared-peer slot), not the port contract.

---

## [REF] Domain-Driven Design — ubiquitous language additions

This feature is one bounded context (**east-west transparent-mTLS enforcement**)
already owned by `overdrive-dataplane`/`overdrive-worker`/`overdrive-bpf` per
ADR-0069 OQ-2. Path A adds/clarifies these terms:

| Term | Meaning |
|---|---|
| **Workload netns** | A per-allocation Linux network namespace the exec workload is born into (via the `ExecDriver` `setns` hook); its only egress/ingress path is its veth pair. |
| **Workload veth pair** | `(ovd-wl-<4hex-slot>` in-netns ↔ `ovd-hv-<4hex-slot>` host-side) — both ends named from the allocation's **`NetSlot`** (4-char zero-padded hex), so each name is 11 chars ≤ IFNAMSIZ (15) and **distinct-slot ⇒ distinct-name by construction** (D-TME-12; NOT `ovd-{hv,wl}-<alloc>`, which overflows IFNAMSIZ for any alloc id ≥ 9 chars and would collide under truncation). The host-side end is where nft-TPROXY PREROUTING intercepts BOTH the workload's egress (now ingressing the host veth) and inbound traffic to the workload. |
| **Workload `NetSlot`** | A host-unique bounded per-allocation network slot (`NetSlot(u16)`, `0..=4095`). The single axis from which the netns name, both veth iface names, AND the point-to-point /30 subnet derive — collision-free by construction, NOT a hash of the `AllocationId` (pigeonhole — a 253-char id cannot collision-free-map into a 15-char iface name, nor into a ≤255-char netns name without overflow at 260). Assigned smallest-free at the C3 `on_alloc_running` hook, released at `on_alloc_terminal`. The netns name is ALSO slot-keyed: `ovd-ns-<4hex-slot>` (11 chars ≤ `NAME_MAX`=255 AND ≤ IFNAMSIZ, bounded by construction, identical shape to the veth names — B3 resolution, ratified option (a) 2026-06-17; NOT `ovd-ns-<alloc>`, which overflows `NAME_MAX` at 260 chars for a 253-char alloc id, the same pigeonhole/ceiling class as B1). The 4096-slot space tiles a /18 within the /16 base (not the whole /16). `ip netns list` shows `ovd-ns-<4hex>`; the human-readable alloc identity lives in the 02-04 slot↔alloc map, rendered by tooling (the Cilium `lxc<hex>` + `cilium endpoint list` model). See D-TME-12. |
| **Egress capture** | The workload's outbound `connect()` leaves its netns via the veth, ingresses the host-side veth, and is nft-TPROXY-redirected to the agent's leg-F `IP_TRANSPARENT` listener — the active-side mirror of the inbound passive-side capture. |
| **Per-connection resolve** | The agent, on each captured connection, resolves `orig_dst → MtlsResolution` (a 3-variant sum type: `Mesh(ResolvedBackend{addr, expected_svid})` / `NonMesh` / `MeshUnreachable`) filtered to `running` via the **resolve port** (the #178 boundary) — the enrollment model's replacement for the per-destination map. See § "`MtlsResolve` port contract" (C1/C2). |
| **Fail-closed (enrollment)** | The `MeshUnreachable` arm: a captured connection whose orig_dst SHOULD be a mesh peer but cannot be reached/verified is refused (never silent cleartext). Distinct from the `NonMesh` arm (orig_dst resolves to no mesh peer → non-mesh egress, passes through in cleartext by design — the classification arm). A binary `Option` cannot tell these two apart; the 3-variant type makes the distinction structural (C1). |
| **Changed assumption** | v1 single-node moves OFF host-netns (`veth_provisioner.rs:36-37`) ONTO per-workload netns+veth. See § Back-propagation. |
| **netns resolver injection** | The per-workload netns (Q2) is also the **DNS injection point**: a node-local DNS responder address is written into the workload netns's `resolv.conf` (the Fly.io `fdaa::3` model), so the workload's libc `getaddrinfo` reaches it with zero app config. |
| **DNS-return shape** | What the node-local responder returns for `<job>.svc.overdrive.local`: **headless** (a `running` backend addr straight from `service_backends`) vs **VIP** (a per-service stable virt address). v1 = **headless** (see § "Name-layer integration"). The returned address IS the `orig_dst` that `MtlsResolve.resolve` later recognizes — the two contracts are kept consistent. |

DDD verdict: **no new bounded context, no new aggregate.** Path A is a
mechanism change inside the existing enforce context. The resolve context
(`service_backends`, #178) and the name context (#61) are **separate, named
dependencies** (anti-corruption boundary = the resolve port). The name-layer
*daemon* (#61) and the VIP allocator (#167) remain separate builds; only the
*integration* of a node-local responder into the per-workload netns (the
injection mechanism + the return shape + its composition with `MtlsResolve`)
lives in this feature.

---

## [REF] Component decomposition

EXTEND is the default. CREATE-NEW is challenged in the Reuse Analysis below.

| Component | Crate / path | EXTEND / CREATE-NEW | Responsibility under Path A |
|---|---|---|---|
| **Workload netns+veth provisioner** | `overdrive-control-plane/src/veth_provisioner.rs` (EXTEND, Q2 ratified) | EXTEND | Create/converge a per-allocation netns + veth pair; **BOTH ends up + `lo` up in-netns** (a veth forwards only when both ends are up, and a fresh netns has `lo` down — B2); routes; `tx off` (csum invariant, `bpf.md` Rule 2); `ip_forward`; `rp_filter` relax (split global vs per-host-veth, S3). Pure `derive_workload_netns_plan(slot, responder_addr)` (slot-derived names — netns + both veths — + /30, D-TME-12; `alloc_id` DROPPED from the signature once the netns name is also slot-keyed, B3) / `workload_converge_steps` + idempotent `provision`, mirroring the existing single-node shape. **Lifecycle call site PINNED (C3)**: created at the action-shim **`on_alloc_running`** (alloc → Running), **BEFORE `MtlsInterceptWorker::start_alloc` and BEFORE `start_alloc`/`Driver::start`** — the netns+veth must exist before the workload process is spawned into it (the `ExecDriver` `setns` seam ENTERS an already-created netns). Torn down on `on_alloc_terminal`. |
| **Per-host `NetSlot` allocator** | `overdrive-control-plane` (CREATE-NEW, justified — D-TME-12) | CREATE-NEW | A per-host free-list assigning a host-unique bounded `NetSlot` (`0..=4095`) at the C3 `on_alloc_running` hook (smallest-free) and releasing it at `on_alloc_terminal`. The slot is the single axis the veth names AND the /30 subnet derive from (collision-free by construction; NOT a hash — pigeonhole). Single-node trivial; NOT distributed IPAM, NOT the #167 VIP allocator (which stays deferred). Justified CREATE-NEW: no existing host-unique per-alloc integer exists (`alloc-{workload_id}-{attempt}` is workload-scoped; the cgroup scope keys on the full id string). |
| **Outbound nft-TPROXY install (egress capture)** | `overdrive-worker/src/mtls_intercept.rs` (EXTEND) | EXTEND | New `install_outbound_tproxy(host_veth, agent_leg_f_port)` sibling to `install_inbound_tproxy`: nft PREROUTING rule on the host-side veth matching the workload's egress, redirecting to the agent's leg-F `IP_TRANSPARENT` listener; reuses `ensure_shared_routing_infra`, the shared fwmark/table, F5 exemption, by-handle RAII teardown. |
| **leg-F orig-dst recovery** | `overdrive-worker/src/mtls_intercept.rs::accept_outbound_leg` (EXTEND) | EXTEND | Recover the real peer via `getsockname` on the TPROXY-intercepted leg-F socket (symmetric with `accept_inbound_leg`), building `Routed::Outbound { peer }`. Removes the `real_peer` declared-peer dependency. |
| **Per-alloc intercept lifecycle** | `overdrive-worker/src/mtls_intercept_worker.rs::MtlsInterceptWorker` (EXTEND) | EXTEND | `start_alloc`: install BOTH nft-TPROXY rules (outbound on host-veth + inbound on the workload virt), make leg-F + leg-C `IP_TRANSPARENT` listeners, spawn accept→`enforce` loops. DELETE: `attach_alloc`(cgroup), the `real_peer`/`leg_f_addr` slots, `program_declared_peer_redirect`, `AcceptOutcome::Dropped` no-declared-peer arm. |
| **Per-connection resolve consumer** | `overdrive-worker` (within the outbound accept loop) (EXTEND) | EXTEND | After `getsockname`, call the resolve port: `orig_dst → MtlsResolution` (3-variant). `Mesh(b)` → set `Routed::Outbound { peer: b.addr }` (+ `expected_peer` once #178 supplies it) and `enforce`; `NonMesh` → pass-through (cleartext, by design); `MeshUnreachable` → fail-closed (refuse, NO cleartext). Per-arm semantics PINNED — see § "`MtlsResolve` port contract" (C1). v1 leaves `expected_peer = None` (authn-only, Q4/D-TME-8). |
| **Resolve port (driven)** | `overdrive-core/src/traits/` — new `MtlsResolve` port (CREATE-NEW, justified) | CREATE-NEW | The #178 anti-corruption boundary: `resolve(orig_dst) -> Result<MtlsResolution>` where `MtlsResolution::{Mesh(ResolvedBackend), NonMesh, MeshUnreachable}` (3-variant, C1), `ResolvedBackend { addr, expected_svid }` bounded to exactly two fields (C2; v1 `expected_svid: None`). Filtered to `running`. THIS feature defines the port + a v1 host adapter reading `service_backends`; the #178 *expected-SVID join* internals are NOT designed here. See § "`MtlsResolve` port contract" + § Driven ports. |
| **`MtlsEnforcement` port + `HostMtlsEnforcement`** | `overdrive-core/src/traits/mtls_enforcement.rs` + `overdrive-dataplane` (UNCHANGED contract) | reuse, no change | The 4-method contract is direction-agnostic; `Routed::Outbound { peer }` is still the input. Path A changes the *worker's* obtain-`peer` path, not the port. |
| **kernel-side `cgroup_connect4_mtls` program** | `overdrive-bpf` (DELETE) | delete | Retired with the per-destination map (spike). No kernel-side program is added by Path A — nft-TPROXY is a `nft`/`ip` userspace install, not an eBPF program. |
| **resolv.conf injection (name-layer integration, Q5a)** | per-alloc netns provisioner (same home as netns+veth, Q2 → `veth_provisioner` EXTEND) | EXTEND | When creating the per-alloc netns, write the node-local DNS responder address into the netns's `/etc/resolv.conf` (per-netns mount, the standard `ip netns` convention) so the workload's libc `getaddrinfo` reaches the responder with no app config (Fly.io `fdaa::3` model). The **responder daemon itself is #61** (a separate build) — this feature wires only the injection. |
| **DNS-return contract alignment (name-layer integration, Q5a)** | design-only; the contract `MtlsResolve` recognizes | n/a (contract, not code) | The responder returns a `running` backend addr from `service_backends` (**headless**, v1) — that returned address IS the `orig_dst` `MtlsResolve.resolve(orig_dst)` later recognizes. This feature OWNS keeping the two contracts consistent; it does NOT own the responder's query path (#61) nor a VIP allocator (#167). |

---

## [REF] Driving ports (inbound / primary)

This feature has **no new driving port**. Its activation is driven by the
existing action-shim allocation lifecycle (`on_alloc_running` / `on_alloc_terminal`)
which already invokes `MtlsInterceptWorker::start_alloc` / `stop_alloc`.

**Netns-creation call site PINNED (C3)**: the per-workload netns+veth (and its
resolv.conf injection, Q5a) is provisioned at the action-shim **`on_alloc_running`**
hook (alloc → Running), **BEFORE `MtlsInterceptWorker::start_alloc`** and **BEFORE
`start_alloc` / `Driver::start`** spawns the workload process into the netns. The
ordering is load-bearing: the `ExecDriver` `setns(CLONE_NEWNET)` seam ENTERS an
already-created netns (CNI-aligned, `driver.rs:181-198`), so the provisioner MUST
have created+converged the netns/veth before the driver's `pre_exec setns` fires.
Teardown is at `on_alloc_terminal`. No CLI verb, no HTTP surface (consistent with
ADR-0069: the feature's only observability is telemetry/metrics).

---

## [REF] Driven ports + adapters

| Port | Status | Host adapter | Sim adapter | Contract |
|---|---|---|---|---|
| `MtlsEnforcement` | UNCHANGED (ADR-0069/0070) | `HostMtlsEnforcement` (`overdrive-dataplane`) | `SimMtlsEnforcement` (`overdrive-sim`) | 4 methods. Path A removes the cgroup-attach internals from the host adapter's intercept-setup surface; the trait is untouched. |
| `IdentityRead` (#35) | UNCHANGED | (existing) | (existing) | #26 stays a reader; the agent presents the workload's held SVID. |
| **`MtlsResolve` (NEW)** | CREATE-NEW (this feature defines the contract) | `ServiceBackendsResolve` (reads `service_backends` via `ObservationStore`) — v1 SHELL; the expected-SVID join is #178 | `SimMtlsResolve` (scriptable `orig_dst → MtlsResolution`) | `resolve(orig_dst: SocketAddrV4) -> Result<MtlsResolution>` — a **3-variant sum type** return, NOT a binary `Option` (see § "`MtlsResolve` port contract" below). Filtered to `running`. The #178 boundary: THIS feature owns the port + the `service_backends`-reading shell; #178 owns the expected-SVID join and the multi-backend LB-pick policy. |

**Probe contract (Earned Trust, principle 12) for `MtlsResolve` host adapter**:
`probe()` must demonstrate it can read the `ObservationStore` `service_backends`
surface and return a structured `health.startup.refused` on an unreadable store —
NOT a silent empty result (an empty resolve degrading to silent pass-through is
the silent-cleartext footgun the enrollment model exists to remove). The
`MtlsEnforcement::probe` (kTLS-arm round-trip) is UNCHANGED. The netns+veth
provisioner's converge-on-boot already refuses boot on non-benign failure
(the `veth_provisioner` precedent) — that IS its probe.

**External integration annotation**: NONE. All boundaries are intra-cluster
kernel/observation-store surfaces; there is no third-party API. (Contract-test
recommendation N/A for this feature.)

---

## [REF] `MtlsResolve` port contract — PINNED for DELIVER (C1 + C2 + C4)

These sub-decisions pin the `MtlsResolve` contract so a crafter
implements-to-design and does not invent the classification mapping, the
`ResolvedBackend` shape, or the read mechanism (CLAUDE.md § "Implement to the
design — never invent API surface"; § "Trait definitions specify behavior, not
just signature"). **C1–C3** were the DESIGN-review DELIVER-handoff conditions
(non-blocking suggestions 1–3, 2026-06-16, § "DESIGN review" below). **C4** (the
resolve READ MECHANISM + the miss-classification scoping note) was added
2026-06-16 as a tight amendment after a DELIVER step surfaced that the *model*
was pinned but the *read mechanism* was not, and **REVISED 2026-06-17** (the
resolve-index-coherence research) from observe-only to **List-then-Watch +
relist-on-`Lagged`** — see § "C4 — resolve read mechanism" below. C4 leaves the
**`MtlsResolve` public contract UNCHANGED** (trait signature, 3-variant
`MtlsResolution`, `ResolvedBackend` shape, and probe all stand exactly as C1/C2
pin them); the 2026-06-17 revision DOES add one keyless `ObservationStore`
enumerate (`all_service_backends_rows`) — consistent surface growth mirroring the
existing `alloc_status_rows()`/`node_health_rows()`, NOT a change to the resolve
port itself.

### C1 — `resolve` returns a 3-variant sum type, NOT a binary `Option`

A binary `Option<ResolvedBackend>` **cannot** distinguish "the dialed dst is
genuinely not a mesh peer → pass-through in cleartext, by design" from "it
should be a mesh peer but cannot be resolved/reached/validated → fail-closed,
NO cleartext." Collapsing both into `None` re-introduces the exact ambiguity the
enrollment model exists to remove. Per CLAUDE.md § "Type-driven design — sum
types over sentinels / make invalid states unrepresentable," the return is a
**3-variant sum type** — the canonical names chosen for this design:

```rust
/// Outcome of resolving a captured connection's original destination
/// (`orig_dst`, recovered via `getsockname` on the TPROXY-intercepted leg-F
/// socket) against the mesh's `running` backend set.
///
/// THREE arms, each with a DISTINCT enforce/pass-through/fail-closed semantic.
/// The worker's decision rule is pinned by the variant — a crafter MUST NOT
/// infer it from a sentinel. (Sharpens Q3 "fail-closed, not silent-cleartext"
/// into the type.)
pub enum MtlsResolution {
    /// **mesh → ENFORCE.** `orig_dst` mapped to a `running` mesh backend.
    /// The worker sets `Routed::Outbound { peer: backend.addr }` (+
    /// `expected_peer` when #178's join supplies it) and calls `enforce`
    /// (mTLS to that backend). The only arm that drives a handshake.
    Mesh(ResolvedBackend),

    /// **non-mesh → PASS-THROUGH (cleartext, by design).** The dialed dst is
    /// genuinely NOT a mesh peer (no `running` mesh backend for `orig_dst`).
    /// Egress proceeds in cleartext — this is the classification arm, NOT an
    /// error and NOT a fail-closed. (e.g. a workload dialing an external
    /// address, or a non-meshed local port.)
    NonMesh,

    /// **unreachable-or-invalid mesh → FAIL-CLOSED (NO cleartext).** `orig_dst`
    /// SHOULD be a mesh peer but cannot be resolved/reached, or its identity
    /// cannot be validated. The worker REFUSES the connection — it does NOT
    /// fall back to cleartext. This is the footgun the enrollment model exists
    /// to remove: a should-be-mesh peer is never silently leaked in the clear.
    MeshUnreachable,
}
```

- **`Mesh(ResolvedBackend)` → enforce** (a `running` mesh backend resolved).
- **`NonMesh` → pass-through** (dialed dst is genuinely not a mesh peer; egress
  proceeds in cleartext, by design — the classification arm).
- **`MeshUnreachable` → fail-closed** (should-be-mesh but unresolvable/
  unreachable/invalid → refuse, NO cleartext).

The per-arm enforce/pass-through/fail-closed semantics above are the port's
rustdoc contract; they MUST be reproduced verbatim on the trait method's
rustdoc (DST equivalence test exercises every arm). v1's
`ServiceBackendsResolve` host adapter distinguishes `NonMesh` (the
`service_backends` lookup found no `running` mesh backend for `orig_dst`) from
`MeshUnreachable` (the store read itself failed at resolve time, or a mesh
backend is present but its address is unreachable / its required identity facts
are absent) — the boundary between "not a mesh peer" and "should-be-mesh-but-
can't" lives in the adapter, classified into the type, never inferred by the
worker.

### C2 — `ResolvedBackend` is bounded to exactly `{ addr, expected_svid }`

```rust
/// A single `running` mesh backend the captured connection resolves to.
/// Bounded to EXACTLY two fields — no more. Multi-backend candidate sets +
/// LB-pick are #178's concern, not this struct's.
pub struct ResolvedBackend {
    /// The concrete `running` backend address (v1 headless: the same
    /// `service_backends` addr DNS returned — one source, two readers).
    pub addr: SocketAddrV4,
    /// The peer's expected SPIFFE identity for SAN-pinning. **v1 = `None`**
    /// for every backend: the v1 `ServiceBackendsResolve` adapter is
    /// authn-only (chain-to-bundle) and does NOT join identity facts. The
    /// expected-SVID join is **#178**; filling it in here = boundary
    /// divergence across the anti-corruption boundary. The field exists so the
    /// SAN-pin wires the moment #178 supplies the join (Q4 / D-TME-8).
    pub expected_svid: Option<SpiffeId>,
}
```

- Exactly `{ addr, expected_svid }` — no third field.
- **v1 `ServiceBackendsResolve` returns `expected_svid: None`** for every
  `running` backend. The adapter is a SHELL: it reads `service_backends`
  filtered to `running` and does NOT thread `IdentityRead` to fill the SVID.
  A crafter adding the identity join "while we're here" has diverged across the
  #178 anti-corruption boundary — explicitly forbidden (consistent with Q4: v1
  is authn-only, intended-peer pinning deferred to #178).

### C4 — resolve READ MECHANISM: an in-RAM address-keyed reverse index (NOT a per-`ServiceId` point query)

C1/C2 pinned the classification *model* and the `ResolvedBackend` *shape*;
ADR-0071 / this feature-delta said the v1 adapter "reads `service_backends`
filtered to `running`" and classifies `orig_dst` — but left the *read mechanism*
underspecified. The gap is concrete: the only `ObservationStore` read surface
for backends is keyed by `ServiceId` (`service_backends_rows(service_id)`),
while `MtlsResolve::resolve(orig_dst: SocketAddrV4)` is handed an **arbitrary
address** and holds **no `ServiceId`**. There is no addr→service reverse index,
no enumerate-all-services method, and no `service_backends_by_addr`. A crafter
left to improvise would either stall or invent new `ObservationStore` trait API
(a boundary-divergence rejection per CLAUDE.md § "Implement to the design").
This sub-decision closes that gap (ratified 2026-06-16).

- **`ServiceBackendsResolve` resolves against an in-RAM, address-keyed,
  ownership-aware projection** of the `running` `service_backends` set, built via
  **List-then-Watch** over the `ObservationStore` and refreshed on incremental
  updates. `resolve()` is then a **point-lookup into the in-RAM index** — NOT a
  per-`ServiceId` store query. The `ServiceId`-keyed `service_backends_rows` point
  query is the WRONG surface for an arbitrary `orig_dst` (the adapter holds no
  `ServiceId`). The index tracks each contributing service's backend at an addr
  separately (`addr → {service → Backend}`), NOT a flat `addr → Backend` with
  global last-writer-wins eviction — see § "C4 — F-A: ownership-aware index"
  below.
- **Read mechanism = List-then-Watch + relist-on-`Lagged` (REVISED 2026-06-17,
  reverses the prior "no new trait method" constraint).** The original C4 pinned
  an *observe-only* mechanism (forward-only `subscribe_all()`, no bulk-load) and
  forbade a keyless enumerate. The resolve-index-coherence research
  (`docs/research/networking/transparent-mtls-resolve-index-coherence-research.md`,
  2026-06-17) established observe-only-without-resync over a forward-only,
  bounded, lossy watch as **the one shape no production mesh uses** — #237
  cold-start, F2 concurrency, F4 lag-drop are three faces of one "coherent local
  cache over a lossy forward-only watch" problem whose textbook fix (k8s informer,
  etcd watch, Envoy xDS, Cilium kvstore) is **List-then-Watch + relist-on-loss**:
  - **List-at-probe** — at `probe()` (the Earned-Trust gate), bulk-load the
    current `service_backends` snapshot into the index BEFORE the gate opens /
    before any `resolve` is served, so the index is never empty-but-trusted
    (closes **#237** cold-start; mirrors Cilium `ListDone`-gates-`synced`).
  - **Watch** — observe the NEW lag-surfacing subscription `subscribe_all_events()`
    (NOT the lossy `subscribe_all()`) for incremental updates. The lossy surface
    cannot carry the loss signal the relist trigger needs — see § "C4 — F4 /
    relist-trigger refinement" below.
  - **relist-on-`Lagged`** — on a `SubscriptionEvent::Lagged { missed }` loss
    signal delivered by `subscribe_all_events()`, the adapter MUST re-List
    (re-acquire the authoritative snapshot, rebuild/merge the index); it MUST NOT
    silently discard the loss (closes **F4**; mirrors Cilium
    `ErrCompacted → goto reList`; the tokio-idiomatic recovery).
  - **Concurrency = single-owner drain (no take-and-replace).** ONE owner (a
    background drain task) exclusively owns the subscription and writes the index
    under the existing `RwLock`; per-connection `resolve` readers take the read
    lock. Dissolves the F2 take/restore TOCTOU by open-once / single-owner
    (`development.md` § "Check-and-act must be atomic").
- **ADDS two `ObservationStore` surfaces** (both confined to that driven port):
  a keyless **List** enumerate and a lag-surfacing **Watch** subscription, each
  symmetric with the EXISTING unkeyed `alloc_status_rows()` / `node_health_rows()`
  / `subscribe_all()`. **Pinned signatures:**
  ```rust
  // List leg — keyless enumerate. SHIPPED (commit 25e7acf3).
  async fn all_service_backends_rows(&self)
      -> Result<Vec<ServiceBackendRow>, ObservationStoreError>;

  // Watch leg — lag-surfacing subscription. NEW (this refinement, 2026-06-17).
  pub enum SubscriptionEvent {
      Row(ObservationRow),
      Lagged { missed: u64 },        // domain event; adapter maps RecvError::Lagged(n) → missed
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
  enumerators / subscription), not novel. **`subscribe_all_events()` (yielding
  `SubscriptionEvent`) is now the SOLE observation-subscription surface — the
  lossy `subscribe_all()` and the `ObservationSubscription` alias were DELETED
  single-cut in commit `36a79762`, every consumer migrated** (see § "C4 — F4 /
  relist-trigger refinement" → "F-B reconciliation" for the dated history). The
  read mechanism is otherwise **adapter-internal** (the in-RAM index is a private
  detail of `ServiceBackendsResolve`); the **PUBLIC `MtlsResolve` contract is
  UNCHANGED** (trait signature + `MtlsResolution` + `ResolvedBackend` + probe all
  stand exactly as C1/C2 pin them).
- **Classification onto the index** (consistent with C1 and the shipped 01-01
  port rustdoc `crates/overdrive-core/src/traits/mtls_resolve.rs` — C4 ADDS the
  read mechanism, it does NOT re-classify):
  - `orig_dst` **hits** a `running` mesh backend in the index →
    `Mesh(ResolvedBackend { addr, expected_svid: None })` (`expected_svid` is
    `None` for every backend in v1 — the identity join is #178; C2).
  - `orig_dst` **misses** (no `running` mesh backend), index readable →
    `NonMesh` (cleartext pass-through, by design). **A miss is `NonMesh`, NOT
    `MeshUnreachable`.**
  - A matched backend is **present-but-unreachable** / required identity facts
    absent → `MeshUnreachable` (fail-closed, no cleartext). A **store-layer read
    fault** (poisoned handle, corrupt table, errored subscription) surfaces per
    the shipped 01-01 error split — `Err(MtlsResolveError::StoreUnreadable)`,
    NOT `MeshUnreachable` (C4 preserves the 01-01 rustdoc asymmetry verbatim).
- **v1 `orig_dst == backend addr` (no VIP→backend translation).** In headless v1
  (D-TME-10) the addr DNS returns IS the backend addr, so the in-RAM index is
  keyed by the backend addr **directly** — there is NO VIP→backend translation
  in the resolve path (that is #167/#61, out of scope here; one source, two
  readers).

**C4 — F-A: ownership-aware index (ratified 2026-06-17 — option (b)).** A flat
`addr → Backend` index with global last-writer-wins eviction would rely on an
**unstated cross-component invariant** — *"a given `(IP:port)` belongs to at most
one service"* — on the silent-cleartext boundary. That invariant holds
structurally in v1 (`Backend.addr` IS the alloc's serving addr; each alloc is one
workload's replica → in exactly one service; one listener per `(IP:port)`), but a
correctness property that leans on an unstated cross-component invariant — plus a
last-writer-wins eviction that is non-deterministic when two services disagree
about an addr's health — is a smell on this boundary. The index is therefore
**ownership-aware**, so it does NOT rely on addr-exclusivity:

- **Keyed so each contributing service's backend at an addr is tracked
  separately** (e.g. `addr → {service → Backend}`), NOT one `Backend` per addr
  with global LWW eviction.
- **A service's backend-set shrink evicts only THAT service's contribution** at
  the addr; an addr stays resolvable as long as **any** service still claims a
  healthy backend there.
- **Classification is `any-healthy-at-addr`** — deterministic, NOT
  last-writer-wins: `orig_dst` hits `Mesh` iff some service still claims a
  `running`/healthy backend at that addr.

This makes the index correct **independent** of addr-exclusivity, removing the
unstated cross-component invariant and the last-writer-wins healthy-disagreement
determinism smell. v1 single-node is structurally addr-exclusive (so the per-addr
service set is size-1 today); the ownership-aware shape is **defensive against
multi-node / future writers**, not a change to today's observable behaviour. The
structure is an **adapter-internal detail** — the public `MtlsResolve` contract
and the `NonMesh`/`MeshUnreachable`/`Err` classification arms (C1) are
**UNCHANGED**.

**C4 — F4 / relist-trigger refinement (REFINED 2026-06-17, ratified — option 2:
surface `Lagged` for event-driven relist).** The 2026-06-17 read-mechanism
revision folded F4 in as "relist-on-`Lagged` closes F4 now" — wording that
silently assumed the `Lagged` loss signal was already reachable on the watch
surface. Implementation proved it **unimplementable on the current surface**:
`subscribe_all()` returns
`ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>` — **no loss
signal** — and BOTH store adapters strip `broadcast::RecvError::Lagged` *inside*
`subscribe_all` before any consumer sees it (sim `ok_or_skip`;
`overdrive-store-local` `filter_map(Result::ok)` / `filter_map`-on-`Lagged`). So
event-driven relist requires **surfacing the `Lagged` loss signal through a
subscription API** — a deliberate, ratified subscription-surface addition
(**option 2**), NOT the zero-cost fold the prior wording implied.

Option 2 was chosen over periodic resync because the loss signal **exists** at
the broadcast layer, and reacting to it gives *completeness* — a dropped update
is always either delivered (`Row`) or signaled-then-relisted (`Lagged`), never
silently lost — which polling cannot guarantee (the etcd-`ErrCompacted` /
k8s-reflector-`Gone` canon applied to tokio `broadcast`).

What this refinement authorizes (the surface pinned above):

- A **lag-surfacing method** `subscribe_all_events()` returning a
  `LagAwareSubscription` of `SubscriptionEvent`.
- `SubscriptionEvent::Lagged { missed }` is a **domain** event, NOT a tokio leak:
  the host/sim adapter maps `broadcast::RecvError::Lagged(n)` to it (`n → missed`)
  at the adapter boundary; the core trait never names `tokio::...::RecvError`.

**F-B reconciliation — `subscribe_all` DELETED single-cut, superseding the
bounded-blast-radius framing (recorded 2026-06-17 as dated history, not a silent
overwrite).** When this refinement was authored (commit `36652ace`) it justified
`subscribe_all_events()` as a *dedicated method* whose point was to **bound blast
radius** — keeping the lossy `subscribe_all()` + `ObservationSubscription` alias
AS-IS for their "~20 existing consumers (DST invariants, store test-harness,
streaming)" so only `ServiceBackendsResolve` would consume the new surface and the
migration would touch one consumer. **The very next commit `36a79762` did the
opposite, and that single-cut is the SHIPPED, intended state: `subscribe_all` and
the `ObservationSubscription` alias were DELETED outright and ALL ~20 consumers
were migrated to `subscribe_all_events()`** (now yielding `SubscriptionEvent`).
Keeping a lossy `subscribe_all()` beside the lag-aware surface would have been the
deprecated-parallel-path anti-pattern the project forbids
(`feedback_single_cut_greenfield_migrations` / `feedback_delete_dont_gate`:
removed is removed; no parallel old path). So `subscribe_all_events()` is now the
**SOLE** observation-subscription surface; the "~20 consumers stay untouched /
bounded blast radius / not a shared-type change" rationale was a point-in-time
decision the subsequent single-cut superseded, preserved above only as honest
history. There is **no remaining "migrate the other consumers" follow-up** — that
work is DONE (`36a79762`), not deferred.

**Relist TRIGGER (the only missing leg, now wired):** the single-owner drain
consumes `subscribe_all_events()`; on `SubscriptionEvent::Row(row)` it applies
the row; on `SubscriptionEvent::Lagged { missed }` it re-Lists via the
already-shipped `all_service_backends_rows()` (commit `25e7acf3`) and
rebuilds/merges the index. The `relist()` machinery already exists and is already
exercised at probe and on watch-close — this leg wires its TRIGGER only. F4 is
closed with the *completeness* guarantee. List-at-probe (#237 closed,
`25e7acf3`), the single-owner drain (F2 dissolved structurally), and
watch-close → `Err(StoreUnreadable)` already shipped; the `Lagged` trigger is
this amendment's sole authorization.

**C4 miss-classification scoping note (the load-bearing new contract clause).**
A pure `running`-backends reverse index cannot, *on a miss*, distinguish a
"genuinely external addr" (→ `NonMesh`, correct) from a "should-be-mesh addr the
index has not yet converged on" (→ would-be `MeshUnreachable`). **v1
deliberately classifies a miss as `NonMesh`.** With List-then-Watch +
relist-on-`Lagged` closing the cold-start (#237) and lag (F4) windows, the
residual exposure shrinks to the **irreducible convergence window** (a backend
that came up microseconds ago, its `service_backends` row still in flight). The
richer **fail-toward-handshake** miss semantic (treat an un-converged miss as
`MeshUnreachable`) covers that residual and is **#236-coupled** (it depends on
the agent being able to *attempt* a handshake on an ambiguous miss — multi-node-
shaped, not yet in tree). An implementer **MUST NOT** make a miss fail-closed in
v1 — that would break legitimate external / non-mesh egress, the entire purpose
of the `NonMesh` arm.

**C4 — (a) fail-toward-handshake as the stated v1 SECURITY invariant (added
2026-06-17).** The contract the miss-classification must eventually satisfy:
***a resolve miss must never silently emit cleartext to a should-be-mesh peer.***
This is the **miss-meaning** lever, orthogonal to the **coherence** lever
List-then-Watch engages — the posture every production mesh safe on this boundary
adopts (Cilium fail-CLOSES an auth-required flow on an auth-map miss,
`bpf/lib/auth.h:45-53`; ztunnel drops on a missing source workload). For
single-node v1 with the full coherence fix, the residual irreducible window is
local and bounded and is the accepted v1 posture; the CODE that realizes (a) (the
capture-all handshake-attempt path) lands under **#236**. (a) is the load-bearing
backstop — even a perfectly coherent cache leaks during its own irreducible
convergence window unless what a miss MEANS is changed.

**C4 — #237 disposition (corrected 2026-06-17).** List-at-probe seeds the index
before the Earned-Trust gate opens, so on a control-plane restart the boot-time
List captures any pre-existing `service_backends` rows (gossiped or persisted)
and the index is never empty-but-trusted; relist-on-`Lagged` closes the lag
window for single-node AND multi-node burst. **#237 (the cold-start restart-window
instance) is CLOSED by this revision** (the GH issue closes once the crafter lands
the List-at-probe + relist mechanism — not in this doc edit). The only residual is
the irreducible convergence window → (a) / #236. **This corrects the prior
wording**, which described the v1 adapter as a *forward-only `subscribe_all()`
index with no `service_backends` bulk-load* and accepted the empty-on-restart
window as a tracked-but-open v1 edge — that mechanism is the one this revision
replaces. The in-house **"bulk-load-then-observe"** precedent the original C4 text
cited (the reconciler-runtime `bulk_load` + `write_through`) is itself
List-then-Watch over an *authoritative* surface — its `bulk_load` IS the List leg
— so it argues **FOR** List-then-Watch (now what ships), **against** observe-only.

**Evidence (the read mechanism is the industry-canonical shape, not a
convenience):**
1. **Cilium is the canonical implementation of exactly this pattern.** Its
   `ipcache` (`pkg/ipcache/ipcache.go` `ipToIdentityCache`; `LookupSecIDByIP`)
   is an in-RAM, **address-keyed reverse index** from IP → identity, populated
   by *subscribing* to endpoint/CIDR/node/FQDN allocation events (not by
   point-querying a service store per connection) and mirrored to a read-only
   BPF LPM trie consulted inline per connection. Its kvstore watcher does
   **List-before-Watch** with a one-time `ListDone` gating a `synced` flag and
   **relists on `ErrCompacted` or any watch error** (`goto reList`) with stale
   mark/sweep (`pkg/kvstore/etcd.go`, `pkg/kvstore/store/watchstore.go`, research
   A5–A6) — precisely the List-then-Watch + relist-on-loss mechanism C4 now pins.
   Cilium also splits `addr→identity` (ipcache) from `identity→peer-material`
   (auth map / SVID store) — validating v1's `expected_svid: None` two-stage
   deferral of the identity join to #178; and it fail-CLOSES an auth-required
   flow on an auth-map miss (`DROP_POLICY_AUTH_REQUIRED`, `bpf/lib/auth.h:45-53`,
   research A9) — the (a) miss-meaning posture.
2. **Our own interception research states this in words.** `…transparent-mtls-
   interception-mechanism-2026-research.md` §4.1 (refuting the "per-connection
   resolve is a bottleneck" attack): the resolve is "a local in-memory
   `service_backends` lookup (Corrosion-gossiped, already in RAM per the
   reconciler-runtime bulk-load model) — no xDS round-trip, no network hop." An
   in-RAM lookup, NOT a per-service point query.
3. `…stable-service-naming-and-transparent-mtls-comprehensive-research.md`
   (resolve-then-pin; SPIFFE identity / naming split) corroborates the two-stage
   shape (addr→backend, then backend→identity at #178).

---

## [REF] Technology choices (OSS-first; all in-tree at the 6.18 pin)

| Choice | License | Rationale | Alternatives rejected |
|---|---|---|---|
| `nftables` (`nft` shell-out) TPROXY + `IP_TRANSPARENT` | GPL (kernel/userspace tool; runtime dep, not linked) | Already the proven inbound mechanism (`mtls_intercept.rs`); Path A unifies both directions on it; zero new primitive. | `bpf_sk_assign` TPROXY (Cilium's path) — rejected: new BPF surface, no in-tree precedent for us, spike chose nft-TPROXY because we ALREADY run it. `cgroup/connect4`+`getsockopt` — rejected: spike DOESN'T-WORK. |
| Linux netns + `veth` (`ip netns`/`ip link`) | GPL (kernel) | CNI-aligned (the `ExecDriver` `setns` hook ENTERS an existing netns); matches Cilium's workload-in-netns topology; veth XDP already in use single-node. | `netkit` (kernel ≥6.7) — orthogonal datapath optimization, not an interceptor (research §3.2); a separate additive win, NOT this feature. |
| `getsockname` orig-dst recovery | libc (kernel syscall) | Proven inbound (increment-i); under TPROXY the orig-dst IS the socket's local addr — no conntrack, no cross-socket key, no cgroup-scope mismatch (the three walls that killed Probe A). | `SO_ORIGINAL_DST` — `ENOENT` under a non-DNAT rewrite (spike). |
| kTLS / rustls / `splice` (enforcement) | (existing, ADR-0069) | UNCHANGED substrate. | (settled in ADR-0069.) |

---

## [REF] Decisions table

| ID | Decision | Status |
|---|---|---|
| D-TME-1 | Interception = nft-TPROXY + `IP_TRANSPARENT` + `getsockname`, BOTH directions (Path A). | SETTLED (spike) |
| D-TME-2 | v1 moves OFF host-netns ONTO per-workload netns+veth (back-propagation, § below). Shape = **extend `veth_provisioner`** (Q2 ratified). | SETTLED (spike + Q2) |
| D-TME-3 | `cgroup/connect4`-rewrite + `MTLS_REDIRECT_DEST` map + `program_declared_peer_redirect` are RETIRED. | SETTLED (spike) |
| D-TME-4 | Outbound `accept_outbound_leg` recovers `peer` via `getsockname` (symmetric with inbound); `real_peer` slot deleted. | SETTLED (follows from D-TME-1) |
| D-TME-5 | `MtlsEnforcement` 4-method port UNCHANGED; `Routed::Outbound { peer }` still the input. No new enforcement-port surface. | SETTLED |
| D-TME-6 | A new `MtlsResolve` driven port is the #178 anti-corruption boundary; this feature defines the contract + a v1 `service_backends`-reading host adapter; **fail-closed (not silent)** at the boundary; #178 owns the expected-SVID join, #61 owns the name layer. | SETTLED (Q3 ratified) |
| D-TME-7 | Egress-on-per-workload-veth nft-TPROXY is UNVALIDATED on our topology; no Tier-2 backstop → Tier-3 only. **Validate via a thin Tier-3 spike NOW (`increment-b/`)** before DELIVER (Q1 ratified). | SETTLED (Q1 ratified) |
| D-TME-8 | v1 scope = **BOTH directions**; intended-peer SVID pinning (`expected_peer`/`PeerIdentityMismatch`) **deferred to #178** (v1 = authn-only) (Q4 ratified). | SETTLED (Q4 ratified) |
| D-TME-9 | **Name-layer integration (Q5a)**: a node-local DNS responder is injected into the per-workload netns `resolv.conf` (Fly.io `fdaa::3` model); the responder *daemon* is #61 (separate build), only the injection + return-shape contract live here. | SETTLED (Q5a folded in) |
| D-TME-10 | **DNS-return shape**: v1 = **headless** — the responder returns a `running` backend addr from `service_backends`; that address IS the `orig_dst` `MtlsResolve.resolve` recognizes (no VIP allocator, #167, pulled into v1). VIP is the multi-node evolution. | RECOMMENDED (single open item; no new v1 dependency) |
| D-TME-11 | **Resolve READ MECHANISM (C4)**: `ServiceBackendsResolve` resolves `orig_dst` against an **in-RAM, address-keyed reverse index** of the `running` `service_backends` set (`addr → Backend`), built via **List-then-Watch + relist-on-`Lagged`** over the `ObservationStore` — NOT a per-`ServiceId` point query. **REVISED 2026-06-17 (resolve-index-coherence research): the prior observe-only / "no new trait method" constraint is REVERSED** — the mechanism ADDS a keyless `all_service_backends_rows(&self) -> Result<Vec<ServiceBackendRow>, ObservationStoreError>` enumerate (List leg; symmetric with `alloc_status_rows()`/`node_health_rows()`; SHIPPED `25e7acf3`), Lists-at-probe before the Earned-Trust gate opens (closes #237 cold-start), and uses a single-owner drain (dissolves the F2 take/restore TOCTOU). **F4 / relist-trigger REFINED 2026-06-17 (ratified — option 2):** the prior wording claimed relist-on-`Lagged` was free, but `subscribe_all()` returns the lossy `ObservationSubscription = Box<dyn Stream<Item = ObservationRow>>` and both adapters strip `RecvError::Lagged` internally — so closing F4 requires a NEW lag-surfacing surface: `subscribe_all_events(&self) -> Result<LagAwareSubscription, ObservationStoreError>` delivering `SubscriptionEvent::{Row(ObservationRow), Lagged { missed: u64 }}` (a DOMAIN event; adapter maps `RecvError::Lagged(n) → missed`; no tokio leak in the core trait). The single-owner drain consumes `subscribe_all_events()`; on `Lagged { missed }` it re-Lists via `all_service_backends_rows()` and rebuilds/merges the index — closing F4 with a *completeness* guarantee (every dropped update is delivered OR signaled-then-relisted, never silently lost). **`subscribe_all_events()` is now the SOLE observation-subscription surface: the lossy `subscribe_all()` + `ObservationSubscription` alias were DELETED single-cut in commit `36a79762` and every consumer migrated** (superseding the earlier "dedicated method bounds blast radius / ~20 consumers stay untouched" framing — see § "C4 — F4 / relist-trigger refinement" → "F-B reconciliation"). A **miss = `NonMesh`** (cleartext pass-through), NOT `MeshUnreachable`; the residual irreducible convergence window is covered by **(a) fail-toward-handshake** — the v1 SECURITY invariant "a resolve miss must never silently emit cleartext to a should-be-mesh peer," whose code lands under **#236**. **#237 is CLOSED by this revision** (List-at-probe + relist); the residual is the irreducible window → (a)/#236. PUBLIC `MtlsResolve` API unchanged (growth confined to the `ObservationStore` driven port). | SETTLED (Q3/D-TME-6 refinement, ratified 2026-06-16; read-mechanism REVISED 2026-06-17; F4/relist-trigger REFINED 2026-06-17) |

---

## [REF] Reuse Analysis (MANDATORY)

Default EXTEND. Every CREATE-NEW carries an evidence line that extending is impossible.

| Target | Verdict | Evidence |
|---|---|---|
| `MtlsInterceptWorker` (`mtls_intercept_worker.rs`) | **EXTEND** | The per-alloc install/teardown lifecycle, accept→`enforce` loops, and fail-closed install are all live and reused. Path A swaps the outbound install (cgroup-attach → nft-TPROXY-on-host-veth) and deletes the declared-peer slots — a within-component change, not a new component. |
| `install_inbound_tproxy` + `ensure_shared_routing_infra` (`mtls_intercept.rs`) | **EXTEND** | The entire shared nft-TPROXY routing infra (table, chain, fwmark, F5 exemption, by-handle RAII teardown, idempotent ensure, dump-parse unit tests) is reused verbatim. The new `install_outbound_tproxy` is a sibling that appends one more per-virt rule to the SAME shared chain. Extending is the obvious move — the egress rule differs only in its match (host-veth ingress) and shares all routing scaffolding. |
| `accept_outbound_leg` / `getsockname_orig` (`mtls_intercept.rs`) | **EXTEND** | `getsockname_orig` already exists and is proven for inbound; `accept_outbound_leg` already builds `Routed::Outbound`. Path A calls `getsockname_orig` from the outbound path too — pure reuse. |
| `MtlsEnforcement` port + adapters | **EXTEND (no contract change)** | The 4-method contract is direction-agnostic and already takes `Routed::Outbound { peer }`. No new method, no new variant. The host adapter loses its cgroup-attach intercept-setup internals (deletion, not addition). |
| `veth_provisioner` (`derive_veth_plan`/`converge_steps`/`provision`) | **EXTEND** (recommended; Q2 alternatives) | The pure-derivation + idempotent converge-on-boot shape is the exact template for per-workload veth; extending it to a per-alloc, in-netns pair is a parameterization + a netns-create step. (Alternatives: a new per-alloc network reconciler, or drive via the `ExecDriver` setns hook — Q2.) |
| `ExecDriver` `setns(CLONE_NEWNET)` hook (`driver.rs:181-198`) | **EXTEND (reuse the seam)** | The opt-in `netns_path: Option<PathBuf>` + `pre_exec setns` hook is exactly the seam: the provisioner creates the netns, the driver enters it. CNI-aligned (driver enters, never creates). Reuse as-is; the only change is wiring `netns_path` to the per-alloc netns the provisioner created. |
| #234 shared-routing infra | **EXTEND** | Path A's egress rule lives in the SAME shared `prerouting` chain + fwmark + table the inbound infra (tracked at #234 for the Bar-2 reconciler promotion) already owns. Outbound extends the existing shared infra, not a parallel one. |
| **`MtlsResolve` port** | **CREATE-NEW (justified)** | No existing port returns `orig_dst → {backend_addr, expected_svid}` filtered to `running`. The closest surfaces are `Dataplane` (map writes, wrong shape) and the `service_backends` table (a data source consumed only by the LB hydrator, no client-facing resolve call). The enrollment model REQUIRES a per-connection resolve consumer; no existing port fits. The port is the #178 anti-corruption boundary (the research's R1/R2 named gap). Extending `MtlsEnforcement` is rejected — it would entangle the resolve concern into the enforcement contract the orchestrator explicitly froze at 4 methods. |
| **Per-host `NetSlot` allocator** | **CREATE-NEW (justified — D-TME-12)** | No existing host-unique per-alloc integer exists: `alloc-{workload_id}-{attempt}` is workload-scoped (two jobs both have attempt 0), and the cgroup scope (`overdrive.slice/workloads.slice/<alloc>.scope`) keys on the full id string, not a host-unique handle. The slot is genuinely needed because the veth iface names MUST fit IFNAMSIZ (15) and a 253-char `AllocationId` cannot collision-free-map into 15 chars by any pure function (pigeonhole) — a hash makes collisions merely unlikely (CLAUDE.md § "One shared length ceiling" forbids that hand-wave). A bounded host-unique slot makes distinct-slot ⇒ distinct-name AND distinct-/30 structural. Single-node trivial free-list (assign-smallest-free / release); NOT distributed IPAM, NOT the #167 VIP allocator. |
| **resolv.conf injection (name-layer, Q5a)** | **EXTEND** | The per-alloc netns provisioner (Q2 = extend `veth_provisioner`) already owns netns creation; writing the responder address into the netns's `/etc/resolv.conf` is one more idempotent converge step in the same converge-on-boot shape (a per-netns mount, the stock `ip netns exec` convention). No new component — it is a parameter + a step on the provisioner. The responder *daemon* (#61) is a separate build, NOT created here. |
| **DNS responder daemon** | **DEPENDENCY (NOT built here, #61)** | The process that answers `<job>.svc.overdrive.local` and reads `service_backends` is the #61 name-layer build. This feature consumes its *address* (for injection) and aligns its *return shape* (headless) with `MtlsResolve` — it does NOT build the daemon. Challenged CREATE-NEW: building a responder here would duplicate #61 and cross the scope boundary the dispatch fixes. |
| **VIP allocator** | **DEPENDENCY (NOT built here, #167)** | The headless v1 return-shape decision (D-TME-10) deliberately AVOIDS pulling #167 into v1 — DNS returns a concrete `running` backend addr, not a VIP, so no VIP-allocator dependency exists for v1. #167 enters only with the multi-node VIP evolution. |

**CREATE-NEW count: 2** (`MtlsResolve` port; the per-host `NetSlot` allocator —
added 2026-06-17 by D-TME-12, justified: no existing host-unique per-alloc
integer, and a hash cannot satisfy the IFNAMSIZ collision-freedom requirement by
pigeonhole). The name-layer integration (Q5a) adds **zero** new CREATE-NEW:
resolv.conf injection is an EXTEND of the netns provisioner, and the DNS
responder daemon (#61) + VIP allocator (#167) are named DEPENDENCIES, not builds
in this feature. Every other change is EXTEND or
DELETE.

---

## [REF] Name-layer integration (Q5a — the DNS name → resolve → enforce fold-in)

Q5a folds the **integration** of the DNS name layer into this design (NOT the
heavy #61 implementation). Path A's per-workload netns (Q2) is the injection
point; the name layer's job is to give an unmodified workload a stable name to
dial that lands at an address `MtlsResolve` recognizes.

### Injection mechanism (the Fly.io `fdaa::3` model)

Each per-workload netns gets its own `/etc/resolv.conf` (a per-netns mount, the
stock `ip netns exec` convention — the same netns the provisioner creates in
Q2). The provisioner writes the **node-local DNS responder's address** into it.
The workload's libc `getaddrinfo("<job>.svc.overdrive.local")` then reaches the
node-local responder with **zero app config** — exactly the Fly.io model where
"we inject the IP of that DNS server into your `resolv.conf` … always `fdaa::3`"
(research §2.7). Overdrive ships its own appliance OS (ADR-0068), so it can do
this for every workload netns without an in-pod agent (sidecarless — research
§1.6).

**This feature owns** the injection step (one idempotent converge step on the
Q2 provisioner). **#61 owns** the responder daemon (the process that answers the
query by reading `service_backends`). The boundary is the responder's *address*
(injected here) and its *return shape* (aligned here, built there).

### The load-bearing sub-decision — what does DNS return?

The DNS-return shape IS the `orig_dst` input to `MtlsResolve.resolve(orig_dst)`
(D-TME-6). The two contracts MUST be consistent: whatever DNS hands the workload
is what `MtlsResolve` must recognize after `getsockname` recovers it. Two
candidate shapes:

| | **Headless** (recommended, v1) | **VIP** |
|---|---|---|
| DNS returns | A `running` backend addr straight from `service_backends` (K8s-headless / Fly.io endpoint-set / #178-shaped — research §2.5, §2.7) | A per-service stable virt address (`<job>.svc.overdrive.local → fdc2:…::N`, K8s-ClusterIP / Consul-`virtual.consul` shape — research §2.5) |
| `orig_dst` after getsockname | A concrete backend addr → `MtlsResolve` does an **identity-only** lookup (addr → expected_svid); **no LB** | A VIP → `MtlsResolve` (or XDP `SERVICE_MAP`) must **LB-pick** a backend, then learn THAT backend's expected SVID |
| New v1 dependency | **NONE** — DNS reads the same `service_backends` the resolve port reads | **#167 VIP allocator** (allocate `fdc2::/16` virts) + the VIP×intercept ordering (research R5/Q1) |
| v1 single-node fit | All backends are local — VIP indirection buys nothing; the headless addr IS the deliverable target | VIP indirection is pure overhead single-node (research §1.6) |
| Multi-node evolution | DNS returns the cross-node backend set (`service_backends` already carries addresses — research §3.4 step 6); client/libc picks | VIP is a stable handle independent of backend churn; the platform LBs (research §3.3 R3) — the cleaner multi-node UX |
| Keeps `MtlsResolve` v1 thin | YES — v1 resolve = identity join, NOT LB (and LB-pick is the **#178-deferred** multi-backend policy, D-TME-8) | NO — v1 resolve would have to own the VIP→backend LB-pick this feature explicitly defers |

**Recommendation: HEADLESS for v1.** Rationale, in priority order:

1. **No new hard v1 dependency.** Headless keeps `orig_dst` = a concrete
   `running` backend addr read from the *same* `service_backends` source the
   `MtlsResolve` port already reads — so DNS and resolve are byte-consistent by
   construction, and **#167 (VIP allocator) is NOT pulled into v1 scope.** The
   dispatch flags exactly this: a VIP choice "pulls a NEW hard v1 dependency
   (#167)" and would need separate sign-off; headless does not.
2. **Keeps `MtlsResolve` v1 honest with the deferred-LB boundary.** v1 resolve
   is an identity-only join (addr → expected_svid, and v1 returns
   `expected_svid = None` until #178 — D-TME-8). A VIP return would force the
   resolve port to own a VIP→backend LB-pick, which is the multi-backend policy
   D-TME-8 explicitly defers to #178. Headless avoids the contradiction.
3. **Single-node v1 makes VIP indirection valueless.** All backends are local
   (research §1.6); the stable-handle benefit of a VIP (decoupling the name from
   backend churn, platform-side LB) only pays off multi-node. Paying its cost
   (an allocator, the VIP×intercept ordering spike R5/Q1) at v1 is premature.
4. **Industry precedent ships BOTH, headless-first for the native case.** K8s
   exposes ClusterIP (VIP) AND headless for different consumers (research §2.5);
   Fly.io's `.internal` returns the **endpoint set, not a VIP** (research §2.7).
   Headless first, VIP as the additive multi-node path, is the battle-tested
   sequence.

**Multi-node evolution path (not foreclosed).** When multi-node lands, the same
node-local responder returns the cross-node `running` backend set
(`service_backends` already carries addresses — research §3.4 step 6); a VIP
shape (the #61/#167 stable-handle model) can be added *alongside* headless
(K8s ships both) without reworking the v1 enforce path — `MtlsResolve` simply
gains a VIP-recognizing arm fed by the #167 allocator and the XDP `SERVICE_MAP`
LB. The headless v1 choice is forward-compatible, not a dead end.

**Sign-off note:** the headless recommendation is the single remaining open item
(D-TME-10) — but it is a recommendation that adds **no new v1 dependency**, so
it is not a blocker. The VIP alternative WOULD add #167 to v1 and is flagged for
explicit sign-off if the user prefers it.

### End-to-end flow (name → resolve → enforce), v1 headless

```
workload getaddrinfo("payments.svc.overdrive.local")
  → resolv.conf (injected into the per-workload netns) points at the
    node-local DNS responder (#61 daemon)
  → responder reads service_backends ∩ running → returns a running backend addr B   (HEADLESS)
  → workload connect(B)  [B is a real running backend, not a VIP]
  → egress leaves the netns via veth, ingresses the host-side veth
  → nft-TPROXY PREROUTING captures → agent leg-F IP_TRANSPARENT listener
  → getsockname(leg-F) recovers orig_dst = B
  → MtlsResolve.resolve(B): Some{ addr: B, expected_svid }   (v1 expected_svid = None, authn-only)
        — orig_dst B is recognized because DNS returned the same service_backends addr
  → enforce Routed::Outbound { peer: B }: rustls client handshake to B presenting workload SVID,
    chain-to-bundle verify (+ SAN==expected_svid once #178 lands), kTLS arm, agent-light pumps
server side: nft-TPROXY (inbound, UNCHANGED) → leg-C → getsockname orig-dst → server SVID handshake
  → kTLS-RX splice plaintext to the server workload
```

The consistency invariant is load-bearing: **DNS returns a `service_backends`
addr, and `MtlsResolve` resolves a `service_backends` addr — one source, two
readers, no translation.** A VIP return would break this single-source property
(DNS returns a VIP; resolve reads `service_backends`; a VIP→backend translation
must bridge them — the research's R5/Q1 ordering hazard).

---

## [REF] Back-propagation — changed assumption (skill contract)

**Original (quoted)** — `crates/overdrive-control-plane/src/veth_provisioner.rs:36-37`:
> "Single-node runs entirely in the host netns — there is no netns machinery here
> (no `ip netns add`, no `ip link set <if> netns <ns>`)."

And ADR-0069 § Decision fact 1 (OUTBOUND) frames interception as:
> "the workload's `connect()` is rewritten to the agent's leg-F listener via the
> `cgroup/connect4`-rewrite shape … reusing the established `cgroup_connect4`
> program family."

**New assumption (Path A)**: v1 gives each exec workload its **own netns + veth
pair**; the workload's egress ingresses the host-side veth where nft-TPROXY
PREROUTING captures it (the active-side mirror of inbound) → leg-F →
`getsockname`. There is NO `cgroup/connect4` rewrite in the production outbound
path.

**Rationale**: the spike proved `cgroup/connect4`+`getsockopt(SO_ORIGINAL_DST)`
cannot recover orig-dst on the appliance kernel (three independent walls). The
only proven kernel-native recovery is TPROXY+`getsockname`, which requires the
workload's egress to traverse a routing point the agent controls — i.e. a
per-workload netns+veth (Cilium's topology). This unifies both directions on the
one proven mechanism we already run inbound.

**Affected artifacts (architect does NOT edit jobs.yaml; flagged for relay)**:
- `veth_provisioner.rs:36-37` host-netns claim — superseded by per-workload netns (this feature wires the new provisioner).
- ADR-0069 OUTBOUND framing (Decision fact 1, topology diagram, § "intercept-recursion / agent-leg-B exemption", Enforcement outbound invariants) — **amended by the new Path-A ADR** (see § Deliverables). The F5 exemption now governs the agent's leg-B dial NOT being re-captured by the *egress nft-TPROXY rule* (not the `cgroup_connect4` program).
- ADR-0069 § "no kernel-side program is added" note — the `cgroup_connect4_mtls` program is now also retired (it was the last outbound kernel-side program).

---

## [REF] Resolved sub-decisions (Q1–Q4 RATIFIED 2026-06-16)

Q1–Q4 are no longer open — the user ratified each. Recorded here for the
decision trail; the rationale that backed each recommendation is preserved.

| Q | Ratified outcome | Decision ID |
|---|---|---|
| **Q1** — validate egress nft-TPROXY now vs at DELIVER? | **Thin Tier-3 spike NOW (`increment-b/`)**, before DELIVER. The egress-in-netns routing shape is the single novel, no-Tier-2-backstop piece; cheapest place to find an `ip rule`/route/F5-exemption collision (the research's Probe B falsification path). | D-TME-7 |
| **Q2** — per-workload netns+veth shape/owner? | **Extend `veth_provisioner`** (the proven pure-derive + converge-on-boot shape, parameterized per-alloc + a netns-create step; lifecycle driven by the action-shim). A per-alloc network reconciler is the **Bar-2 promotion when runtime drift matters** (the #197/#234 host-infra-reconciler family). Driver-creates REJECTED (driver ENTERS, never creates — CNI-aligned, `driver.rs:190-197`). | D-TME-2 |
| **Q3** — `MtlsResolve` contract + #178 boundary? | **Port in `overdrive-core` + a v1 `service_backends`-reading host adapter**; **fail-closed (not silent)** at the boundary (a resolve adapter that cannot read the store refuses boot — it does NOT silently return empty, which would re-introduce the silent-cleartext footgun). Inlining the read in the worker REJECTED (DST-uninjectable; hard-couples enforce to the observation schema). **#178 owns** the expected-SVID join + multi-backend LB-pick + the SAN-match wiring of `expected_peer`; **#61 owns** the name→virt layer upstream of `orig_dst`. | D-TME-6 |
| **Q4** — v1 scope; intended-peer pinning? | **BOTH directions in v1** (Path A's point is symmetry on one mechanism; the inbound nft-TPROXY install is the proven template the outbound mirrors). Intended-peer SVID pinning (`expected_peer`/`PeerIdentityMismatch`) **DEFERRED to #178** — v1 stays authn-only (chain-to-bundle); the resolve port carries `expected_svid` so the pin wires the moment #178 supplies the join. Docs/tests MUST NOT call the wrong-but-valid-peer case "protected" until #178. | D-TME-8 |

---

## [REF] Open questions — decisions needing user sign-off

Only ONE genuinely-open item remains after Q1–Q4 ratification and the Q5a
fold-in. It carries a recommendation that adds **no new v1 dependency**, so it
is not a blocker.

### Q5a-DNS — DNS-return shape: headless (recommended) vs VIP?
The single remaining open call (D-TME-10). Full analysis + the comparison table
+ the multi-node evolution path live in § "Name-layer integration (Q5a)" above.

- **(a) Headless (RECOMMENDED)** — the node-local responder returns a `running`
  backend addr straight from `service_backends`. `orig_dst` after `getsockname`
  is a concrete backend addr → `MtlsResolve` does an identity-only lookup, no LB.
  **No new v1 dependency** (no VIP allocator); DNS and `MtlsResolve` read the
  same `service_backends` source (one source, two readers, byte-consistent).
  Keeps `MtlsResolve` v1 thin and honest with the #178-deferred LB boundary.
  Single-node v1 makes VIP indirection valueless (all backends local). K8s and
  Fly.io both ship headless for the native case.
- **(b) VIP** — the responder returns a per-service stable virt; `MtlsResolve`
  (or XDP `SERVICE_MAP`) must LB-pick a backend and learn its SVID. **Pulls a NEW
  hard v1 dependency: #167 (VIP allocator)** + the VIP×intercept ordering
  (research R5/Q1). The cleaner *multi-node* stable-handle UX, but premature at
  v1.
- **Recommendation: (a) headless for v1; VIP as the additive multi-node path**
  (K8s ships both alongside each other — the v1 headless choice is
  forward-compatible, `MtlsResolve` later gains a VIP-recognizing arm fed by #167
  + the XDP LB without reworking the enforce path).
- **Sign-off flag**: choosing **(b) VIP** would ADD #167 to v1 scope and needs
  explicit user sign-off. Choosing **(a) headless** adds nothing new — it is the
  zero-new-dependency default and can proceed without a separate decision.

---

## [REF] Quality attributes (ISO 25010, mapped)

| Attribute | How Path A serves it |
|---|---|
| **Security — confidentiality/authenticity** | TLS 1.3 on the wire (kTLS, ADR-0069 UNCHANGED); fail-closed enrollment (no silent cleartext on a mesh-resolve miss); per-workload netns isolates the capture point. |
| **Functional suitability — universality** | One interception mechanism (nft-TPROXY) for both directions and (via the netns+veth topology) for guest-stack kinds later — the Cilium-aligned shape. |
| **Reliability — losslessness/no-RST** | The agent's userspace handshake-window capture is lossless (ADR-0069 UNCHANGED); per-connection self-supervision (ADR-0070). |
| **Maintainability — testability** | All nondeterminism behind ports (`MtlsEnforcement`, `MtlsResolve`, `IdentityRead`); sim adapters + DST equivalence; netns/veth Tier-3 fixtures already exist (`overdrive-testing`). |
| **Performance — agent-light** | Steady state UNCHANGED (kernel kTLS; agent-light pumps). Path A adds one per-connection resolve lookup (in-RAM `service_backends`; research §4.1 = negligible). |

---

## [REF] Proven-mechanism traceability

| Path-A claim | Proven by |
|---|---|
| nft-TPROXY + `IP_TRANSPARENT` + `getsockname` recovers orig-dst | `findings-inbound-intercept.md` (increment-i, inbound) + `mtls_intercept.rs` (shipped inbound install) |
| `cgroup/connect4`+`SO_ORIGINAL_DST` does NOT recover orig-dst | `spike/findings.md` (Probe A DOESN'T-WORK, kernel 7.0) |
| Workload-in-netns + agent-controls-routing-point is the consensus topology | research `…interception-mechanism…` §3.1/§3.3 (Istio/Cilium); spike Cilium reconciliation (`main @ dac977e678`) |
| Egress-on-per-workload-veth nft-TPROXY | **UNVALIDATED on our topology** — Q1 (Tier-3 spike `increment-b/`, ratified; no Tier-2 backstop) |
| `ExecDriver` setns hook ENTERS an existing netns | `driver.rs:181-198` (shipped) |
| Node-local DNS responder injected into the workload `resolv.conf` is the consensus placement | research §2.7 (Fly.io `fdaa::3` — node-local Rust DNS server injected into `resolv.conf`); §1.6 (Overdrive ships its own appliance OS → can inject per-netns) |
| Headless (endpoint-set) DNS return is the native-case industry default | research §2.5 (K8s headless / EndpointSlice), §2.7 (Fly.io `.internal` returns the endpoint set, not a VIP), §2.3 (Linkerd resolve-to-endpoint-then-pin) |
| VIP return would add #167 + the VIP×intercept ordering hazard | research §3.3 R5 / §3.5 Q1 (VIP × agent-light-intercept composition is the trickiest wiring; needs #167 + a Tier-3 ordering spike) |

---

## DESIGN review — 2026-06-16 (nw-solution-architect-reviewer)

**Verdict: APPROVED for DELIVER handoff.** No blocking issues. Reviewer:
`nw-solution-architect-reviewer`. Scope reviewed: this feature-delta + ADR-0071 +
`design/wave-decisions.md` + `spike/{findings,wave-decisions}.md` + brief §35 +
the live code anchors (`mtls_intercept.rs`, `veth_provisioner.rs`,
`driver.rs:181-198`, `mtls_enforcement.rs`).

**Executive summary.** Path A is a sound, evidence-grounded design that correctly
pivots from the proven-unviable `cgroup/connect4`+`SO_ORIGINAL_DST` recovery
(Probe A spike: three independent fatal walls, real kernel 7.0) to a unified
nft-TPROXY+`getsockname` interception model for both directions (inbound already
proven via increment-i; outbound the active-side mirror). The topology shift
(host-netns → per-workload netns+veth) is load-bearing, justified by the spike
evidence, and CNI-aligned. The reuse analysis is rigorous (1 CREATE-NEW —
`MtlsResolve` port, justified; everything else EXTEND or DELETE) and the EXTEND
targets verify against live code. Q5a correctly defers the #61 responder daemon
and #167 VIP allocator to separate builds while folding only the resolv.conf
injection + the headless v1 return-shape into scope. The single open item
(D-TME-10) carries a recommendation (headless) that adds no new v1 dependency and
is non-blocking. All artifacts (feature-delta, ADR-0071, wave-decisions, brief
§35) agree on D-TME-1..10 and Q1..Q5a. Back-propagation is handled correctly
(ADR-0069 outbound framing amended via ADR-0071; `jobs.yaml` flagged for the
product-owner, not edited). Deferral discipline is tight — every forward pointer
cites a real issue (#178, #61, #167, #197, #234).

### praise

- **Evidence discipline.** The Probe-A DOESN'T-WORK verdict is decision-grade
  (real kernel, real aya-rs Rust, errno + bpftool ground truth, Cilium
  reconciliation confirming the pivot). The design does **not** over-claim:
  increment-b is correctly scoped as "UNVALIDATED on **our exact topology**"
  (Cilium proves the model; our wiring is unproven) — not "the model is
  unproven." This is exactly the calibration the project's standing lesson
  ("verify unproven claims against the evidence") demands.
- **Path-A symmetry / reuse-over-novelty.** One proven inbound mechanism becomes
  the outbound template by flipping the interface; egress joins the same shared
  `prerouting` chain + fwmark + table (#234). The design resists reaching for
  `bpf_sk_assign` (Cilium's path) precisely because nft-TPROXY is already shipped
  and proven in-house.
- **Deferral discipline.** Q5a folding adds zero CREATE-NEW; every forward
  pointer has a real issue number; the product-owner notice is explicit.

### issue (blocking) / issue (blocking, security)

- **None.** The enrollment model (per-connection resolve, fail-closed on miss)
  removes the silent-cleartext footgun; the authn-only v1 boundary (chain-to-
  bundle, intended-peer pinning deferred to #178) is documented and tests are
  flagged to NOT call the wrong-but-valid-peer case "protected" until #178.

### suggestion (non-blocking) — DELIVER-handoff conditions

1. **Pin the `MtlsResolve` `None` classification arm.** Q3 ratified fail-closed
   (not silent), but the port sketch under-specifies the worker's decision rule.
   Add to the port trait rustdoc / ADR-0071 fact 4 + Q3: "`resolve` returns
   `None` when `orig_dst` does not map to a `running` mesh backend → worker
   classifies as non-mesh egress (pass-through, not an error); fail-closed only
   when a mesh peer's backend address is unreachable or its SVID verification
   fails." Crafters should not have to infer this (CLAUDE.md § "Implement to the
   design — never invent API surface"; § "Trait definitions specify behavior").
2. **Bound the `ResolvedBackend` shape to exactly `{addr, expected_svid}`.** Note
   explicitly that multi-backend candidate sets + LB-pick are #178's concern, so
   the v1 `ServiceBackendsResolve` adapter returns `expected_svid: None` for every
   `running` backend and does **not** join identity facts — filling the join in
   here would be design divergence across the anti-corruption boundary.
3. **Name the exact netns-creation call site.** Q2 ratified "extend
   `veth_provisioner`, provision before `Driver::start`, teardown on terminal."
   DELIVER should pin the precise hook (e.g. action-shim `on_alloc_running`,
   before `MtlsInterceptWorker::start_alloc` and `Driver::start`) — the netns must
   exist before the `setns` hook fires.
4. **Pin the `increment-b/` Tier-3 spike acceptance criteria** before it lands:
   (1) workload `connect()` on the per-workload veth redirects to leg-F;
   (2) `getsockname(leg-F)` recovers orig-dst; (3) the agent's marked leg-B/leg-S
   dials are NOT re-captured by the egress rule **and** a workload cannot
   self-exempt; (4) a basic round-trip completes without RST/corruption.

### question (non-blocking)

1. **Forward-compatibility of headless → VIP (D-TME-10).** The "additive VIP arm,
   no rework" claim holds only if the v1 resolve code can disambiguate an
   `orig_dst` that is a concrete `service_backends` addr vs a future #167 VIP.
   When #167/#178 land, a design-amendment must pin the disambiguation mechanism
   (disjoint VIP range, or a typed DNS-return hint). Not a v1 blocker — flag for
   the #167 architect.
2. **Q1 spike de-risk depth.** If `increment-b/` uncovers an egress
   route/`ip rule`/F5-exemption collision (the Probe-B falsification path), DELIVER
   sequencing may need to absorb it — correctly ratified as pre-DELIVER, so this
   is a gate, not a decision.

### nitpick (non-blocking)

- ADR-0071 title "Path A" may read as a pre-designed choice rather than the
  promoted Probe-B fallback after Probe-A falsification; the body is clear, so
  cosmetic only.
- Back-propagation quotes `veth_provisioner.rs:36-37` by line number (accurate
  today; line-number quotes can drift on rebase — prefer a function-name anchor).

### thought (non-blocking)

- Anti-corruption boundaries are enforced by discipline, not the compiler:
  re-clarify in DELIVER that the v1 resolve adapter is explicitly a shell (join is
  #178), so a crafter is not tempted to thread `IdentityRead` and fill
  `expected_svid` "while we're here."

**Conditions for DELIVER** (all non-blocking): items 1–4 under *suggestion*
above. None blocks landing the design; they are crafter-handoff sharpenings.

### DELIVER-handoff conditions — PINNED into the design (2026-06-16 fold-in)

The non-blocking *suggestion* conditions above were folded into the contract so
crafters implement-to-design (do NOT invent). Status:

- **Suggestion 1 (C1) — PINNED.** `MtlsResolve.resolve` returns a 3-variant sum
  type `MtlsResolution::{Mesh(ResolvedBackend), NonMesh, MeshUnreachable}` with
  per-arm enforce/pass-through/fail-closed rustdoc semantics — § "`MtlsResolve`
  port contract" (C1). The binary `Option` framing is replaced everywhere
  (this feature-delta, ADR-0071, brief.md §35).
- **Suggestion 2 (C2) — PINNED.** `ResolvedBackend` is bounded to exactly
  `{ addr, expected_svid }`; the v1 `ServiceBackendsResolve` adapter returns
  `expected_svid: None` (authn-only; the expected-SVID join is #178; filling it
  here = boundary divergence) — § "`MtlsResolve` port contract" (C2).
- **Suggestion 3 (C3) — PINNED.** Netns creation at the action-shim
  `on_alloc_running` hook, BEFORE `MtlsInterceptWorker::start_alloc` and BEFORE
  `start_alloc`/`Driver::start` — § "Driving ports" + the provisioner component
  row.
- **Suggestion 4 — NOT pinned in this fold-in (outstanding).** Pinning the
  `increment-b/` Tier-3 spike acceptance criteria (workload `connect()` redirects
  to leg-F; `getsockname` recovers orig-dst; marked leg-B/leg-S dials NOT
  re-captured AND a workload cannot self-exempt; basic round-trip without
  RST/corruption) was NOT part of the relayed contract-pinning pass. It remains a
  pre-DELIVER spike gate (Q1/D-TME-7); the four AC bullets are already enumerated
  in ADR-0071 § Enforcement → "Tier-3 obligations" and in the DESIGN review
  suggestion 4 above. Flagged to the orchestrator as the possible 4th condition.
