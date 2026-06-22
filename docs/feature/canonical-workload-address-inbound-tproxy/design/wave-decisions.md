# DESIGN Decisions — `canonical-workload-address-inbound-tproxy` (GH #241)

**Wave:** DESIGN · **Scope:** Application · **Paradigm:** OOP Rust · **Density:** lean
· **Model:** inherits (ADR-0071 Path A / D-TME-10..13)

The keystone slice of the transparent-mtls-enrollment arc. Three Tier-3 spikes
(`spike/findings{,-cgroup-firing-scope,-vip-lb-inert}.md`) settled every
load-bearing question; this wave pins the two residual implementation-shape
sub-choices and records the locked decisions. The companion artifact is
`docs/feature/canonical-workload-address-inbound-tproxy/feature-delta.md` (the
`[REF]` DDD list, component decomposition, reuse analysis).

---

## Key Decisions

| D# | Decision | Verdict |
|---|---|---|
| D-A1 | Keystone install: `AllocationSpec.{workload_addr: Option<Ipv4Addr>, service_ports: Vec<NonZeroU16>}` + per-port `install_inbound_tproxy` in `start_alloc` (replacing `tproxy_guard = None`) | Production wiring |
| D-BLOCKER1 | Inbound rule keys on `ip daddr <workload_addr> tcp dport <service_port>` = the declared Service listener port (D-TME-10 one-source/two-readers) | Declared service port |
| D-B2 | `BackendDiscoveryBridge` advertises `Backend.addr = workload_addr:port`; `ServiceBackendRow.vip` UNCHANGED | Canonical address |
| D-BLOCKER2 | Persist `workload_addr: Option<Ipv4Addr>` directly on `AllocStatusRow` (V2 envelope), NOT persist-`NetSlot`-and-recompute | Persist the materialized addr |
| D-GATE | Gate `ServiceMapHydrator` off Path-A/mesh backends (no `RegisterLocalBackend`, no `DataplaneUpdateService`) | GATE (reconcile ADR-0053↔ADR-0071) |
| D-GATE-PRED | GATE predicate = `addr.ip() ∈ WORKLOAD_SUBNET_BASE (10.99.0.0/16)`; IN-SCOPE for #241 | Subnet-membership |
| D-C1 | Reuse `ensure_shared_routing_infra` (Bar-1); Bar-2 → #234 | REUSE AS-IS |
| D-D1 | `ip_forward` + /30 routes + `rp_filter` already converged by `veth_provisioner` | Confirm-and-cite |

### BLOCKER-2 rationale (the persist-inputs ruling)

Persist the **materialized** `workload_addr` (option b), not the `NetSlot` to
recompute (option a). Two compounding reasons reject recompute:
1. **Core relocation** — `NetSlot` + `WORKLOAD_SUBNET_BASE` + the
   `base + slot*4 + 2` derivation live in `overdrive-control-plane`; the bridge
   lives in `overdrive-core`. Recompute-at-bridge widens the core surface.
2. **#239 tunable-base divergence** — the inbound nft rule is installed against
   `workload_addr = base_t0 + slot*4 + 2` at C3 provision time; a later
   bridge-tick recompute against a (future-tunable, #239) base_t1 would advertise
   a *different* addr than the rule captures. The addr is a slot×base-at-provision
   join the inbound contract already committed to — only the provision-time
   materialization captures it. Persisting the materialized addr keeps install /
   observe / advertise byte-identical. The #239 risk is a documented
   single-cut-greenfield risk: a base change is a redeploy (re-provision +
   re-observe), not a live re-tune.

→ `AllocStatusRowEnvelope::V2` per `development.md` § "rkyv schema evolution"
6-step (FIXTURE_V1 pinned + V2 + `From<V1> for V2` + golden fixture + re-pinned
discriminant offset). `RunningAllocSet.running` widens
`BTreeSet<AllocationId>` → `BTreeMap<AllocationId, Option<Ipv4Addr>>`.

### GATE-predicate rationale

`addr.ip() ∈ WORKLOAD_SUBNET_BASE` — content-derived, deterministic, extends the
hydrator's existing partition-on-addr model (the `== host_ipv4` LOCAL test).
Cannot misfire: no non-mesh consumer exists (increment-c — every exec alloc is
TPROXY-intercepted, no "mesh flag" exists), and a backend is in the subnet IFF it
is a Path-A `workload_addr` IFF it is mesh. IN-SCOPE for #241 because B2
reclassifies these backends LOCAL→REMOTE, starting dead XDP writes the gate
prevents — shipping B2 without the gate ships dead writes in this slice.

---

## Architecture Summary

Style: hexagonal (ports/adapters), single-process — inherited, unchanged. This
slice closes the **inbound** half of the Path-A bidirectional mTLS loop:

```text
client workload (netns B, /30)                server workload (netns C, /30)
  connect(workload_addr_C:service_port)
    └─ veth egress → host PREROUTING
        └─ nft-TPROXY (ip daddr workload_addr_C tcp dport service_port)   ← D-A1 / D-BLOCKER1 install
            └─ leg-C IP_TRANSPARENT listener (getsockname → orig_dst)      ← reused
                └─ mTLS handshake → splice → server workload

bridge advertises Backend.addr = workload_addr_C:service_port             ← D-B2
egress MtlsResolve.by_addr[workload_addr_C] = Mesh                        ← B2's reader (unchanged port)
ServiceMapHydrator: workload_addr_C ∈ WORKLOAD_SUBNET_BASE → skip LB      ← D-GATE / D-GATE-PRED
  → cgroup_connect4 LOCAL_BACKEND_MAP miss → nft-TPROXY owns delivery
```

End-to-end driven through `overdrive serve` + `overdrive deploy` — no test-only
wiring stands in for a production call site (the C3 seam, the `WorkloadLifecycle`
projection, the `start_alloc` install are all on the production path).

---

## Reuse Analysis

**Zero CREATE-NEW components.** Every touched component is EXTEND or REUSE — see
the feature-delta § "Reuse Analysis (HARD GATE)" for the full table. New named
symbols are confined to additive fields (`AllocationSpec.{workload_addr,
service_ports}`, `AllocStatusRowV2.workload_addr`), one projection function
(`project_service_listen_ports`, mirroring `project_probe_descriptors`), and one
additive ctor param (`ServiceMapHydrator.workload_subnet`). No new trait, port,
adapter, or crate.

---

## Technology Stack

In-tree / OSS only, no new dependency:
- Inbound capture: nft-TPROXY + `IP_TRANSPARENT` + `getsockname` (existing
  production triple, proven increment-a).
- Per-alloc addr channel: `AllocationSpec` in-memory field (no serde/rkyv).
- Persisted observed addr: `AllocStatusRow` rkyv `V2` envelope (rkyv,
  MIT/Apache-2.0).
- GATE: subnet-membership on `WORKLOAD_SUBNET_BASE`.

---

## Constraints

- **No new routing primitive** (increment-a): the install is the existing
  production triple; the #241 gap was production wiring, not mechanism.
- **No `rp_filter` munging** on the inbound path (increment-a: unneeded for the
  host-local-termination shape).
- **Process-global sysctls** (`ip_forward`, `rp_filter`) owned idempotently by
  `veth_provisioner` — confirmed already converged (D-D1), no change here.
- **Kernel pin caveat:** spike verdicts on dev Lima 7.0, not the pinned-6.18
  appliance kernel (ADR-0068); all primitives predate 6.18; authoritative
  re-confirmation is the Tier-3 matrix when the slice lands.
- **One-source/two-readers (D-TME-10):** the `service_port` the inbound rule
  keys on, the port `service_backends` advertises, and the port the egress
  `MtlsResolve` keys on are the SAME declared Service listener port.

---

## Upstream Changes

| Doc / ADR | Change |
|---|---|
| ADR-0071 | **Amendment** — pin the canonical-address one-source contract (D-BLOCKER1), the A1 threading (`AllocationSpec.{workload_addr,service_ports}` + per-port `start_alloc` install), and the B2 bridge change as the production wiring of the `tproxy_guard = None` deferral ADR-0071 named as #241's job. |
| ADR-0053 | **Amendment** — the ADR-0053↔ADR-0071 boundary decision (D-GATE): in Path-A the same-host VIP-LB yields to nft-TPROXY for `workload_addr` backends; the `cgroup_connect4_service` hook stays attached, the hydrator is gated by subnet-membership, full retire deferred until a dialable-VIP path ships (#61; the VIP *allocator* #167 already shipped, this amendment is the durable GATE→TEACH record). Empirically proven safe (increment-c: no live VIP-LB consumer). |
| `AllocStatusRowEnvelope` | V1 → V2 (additive `workload_addr`); golden V1 fixture pinned, V2 added same commit. |

No prior-wave assumption is *reversed* — see § Changed Assumptions for the two
refinements (the `tproxy_guard = None` deferral is now closed; the bridge advertise
addr changes host_ipv4→workload_addr).

---

## Changed Assumptions

### 1. `start_alloc` records `tproxy_guard = None` (ADR-0071 / `mtls_intercept_worker.rs:600`)

**Quoted original** (`mtls_intercept_worker.rs:590-601`): *"The inbound nft-TPROXY
rule install is #241-DEFERRED … `AllocationSpec` carries no listen-addr field …
So `start_alloc` records `tproxy_guard = None` and installs no rule."*

**Replacement** (D-A1 / D-BLOCKER1): `AllocationSpec` now carries
`workload_addr: Option<Ipv4Addr>` + `service_ports: Vec<NonZeroU16>`; `start_alloc`
installs one `install_inbound_tproxy(SocketAddrV4::new(workload_addr, port.get()),
leg_c_addr.port())` per declared service port, retaining the
`TproxyInterceptGuard`(s) for the alloc lifetime. The named #241 production-install
site is wired. The self-referential "virt from ephemeral leg-C port" shape the
original rejected stays rejected — D-BLOCKER1 keys on the declared service port.

### 2. Bridge advertises `host_ipv4:port` (ADR-0053 / `backend_discovery_bridge.rs:349`)

**Quoted original** (`backend_discovery_bridge.rs:343-353`): *"every alloc resolves
to `self.host_ipv4` in Phase 2.2 single-node … `addr: SocketAddr::new(IpAddr::V4(self.host_ipv4),
listener.port.get())`."*

**Replacement** (D-B2): the bridge advertises `Backend.addr = workload_addr:port`
(from the per-alloc `workload_addr` observed via `AllocStatusRowV2`), falling back
to `host_ipv4:port` only for `None` (host-netns / non-Path-A) allocs. The
`ServiceBackendRow.vip` field is UNCHANGED (the dialable-VIP path is #61 territory; the VIP *allocator* #167 already shipped).

### 3. ADR-0053 §5 "XDP programs … reserved for the Phase 2 remote-backend case" (ADR-0053:426-442)

**Quoted original** (ADR-0053 §5): *"In Phase 1 single-node every backend
classifies as local; the XDP forward path receives no `update_service` calls …
The XDP programs are not vestigial — they are reserved for the Phase 2
remote-backend case."*

**Refinement** (D-GATE, NOT a reversal): under B2, Path-A backends would classify
REMOTE (`workload_addr ≠ host_ipv4`) and start receiving `update_service` calls —
but nft-TPROXY owns mesh delivery, so those XDP writes are dead. The hydrator is
gated by subnet-membership so mesh backends program NEITHER the cgroup LOCAL path
NOR the XDP remote path. The XDP programs remain reserved for a genuine
remote-backend case (multi-node VIP-LB — the dialable-VIP territory #61; the VIP *allocator* #167 already shipped) — the gate keeps them empty for
Path-A mesh, it does not retire them. The ADR-0053 same-host cgroup LB hook
(`cgroup_connect4_service`) stays attached and fires (increment-b); the gate makes
it MISS for mesh backends so the dial falls through to nft-TPROXY.

## DESIGN Review

**Verdict: APPROVE-WITH-FIXES.** Engineering-APPROVED — 0 critical, 0 high. The
design (architectural style, component boundaries, C4, ADR amendments, Earned-Trust
posture, OSS/paradigm fit) passed clean. The verdict escalated to NEEDS_REVISION
**solely** on one blocking forward-pointer defect: the open-questions / TEACH-trigger
citations named **#167** (the VIP *allocator*, CLOSED/COMPLETED 2026-05-19 — already
shipped) as a *future, not-yet-shipped* VIP-dial deferral home, violating CLAUDE.md
§ "Deferrals require GitHub issues" ("never copy-paste an issue number; cite an
existing issue whose scope actually covers the deferred work"). **Now fixed** —
every #241-artifact future-path/VIP-dial-trigger citation re-cites the open dialable-VIP
territory **#61** (with the ADR-0053 amendment as the durable GATE→TEACH record);
#167 is reflected as *shipped*, never pending; #243 dropped from VIP-dial-trigger
sites (headless name responder returns `workload_addr`, not a VIP) while remaining
correctly cited elsewhere as the dial-by-name deferral.

- **Reviewer:** `nw-solution-architect-reviewer` (Opus) — read-only design critique
  (0 critical/high). The #167 citation defect was beyond a read-only review's reach;
  it was caught by **orchestrator GitHub verification** (`gh issue view --json state`:
  #167 CLOSED/COMPLETED; #61 OPEN; #243 OPEN; #234/#239/#242 OPEN; #178 CLOSED).
- **Citation fix (this revision):** Morgan. Sites corrected — feature-delta.md
  (D-B2, D-GATE, open-questions VIP-dial row); wave-decisions.md (ADR-0053 amendment
  row, two orthogonality notes); ADR-0071 §"Amendment 2026-06-22 (#241)" (B2 +
  Cross-reference); ADR-0053 §"2026-06-22 boundary amendment" (increment-c, GATE-not-
  retire, why-GATE, deferrals, changelog); brief.md §35a (GATE bullet). Pre-existing
  ADR-body #167 references predating this slice left untouched per scope.

### DELIVER obligations (4 non-blocking review findings — carry into DISTILL/DELIVER)

These do NOT block DESIGN sign-off; they are explicit downstream obligations so
they are not lost.

1. **Port-set equality AC.** DELIVER must carry an acceptance criterion pinning that
   the inbound-rule port-set (`WorkloadLifecycle::project_service_listen_ports` →
   `AllocationSpec.service_ports`) **equals** the advertise port-set
   (`BackendDiscoveryBridge` reading `desired.listeners` at
   `backend_discovery_bridge.rs:336,349`) for a multi-listener Service. Same intent
   source, two code paths → latent drift risk. The AC must assert byte-set equality
   for an N-listener Service (N ≥ 2).

2. **Pin two internal wiring seams in the crafter dispatch.** (a) The `hydrate_actual`
   `RunningAllocSet.running` `BTreeSet<AllocationId>` → `BTreeMap<AllocationId,
   Option<Ipv4Addr>>` population (where the per-alloc `workload_addr` is read into the
   map). (b) The `service_ports` threading site — confirm `obs.alloc_status_rows()`
   already carries the V2 row (so **no new `ObservationStore` method** is needed) and
   thread `service_ports` at the **identical site/shape** as `probe_descriptors`,
   replacing the `-shape` hedge near `reconciler_runtime.rs:2317`. Pin both in the
   dispatch so the crafter does not improvise the seam.

3. **Pinned-6.18 Tier-3 AC.** The DELIVER roadmap must carry an explicit AC that the
   **bidirectional mesh loop passes the pinned-6.18 appliance-kernel Tier-3 matrix**
   (ADR-0068), not merely "tests pass." The `cgroup_connect4` firing + capture is
   proven only on dev-Lima 7.0 so far; the merge-blocking signal is the pinned
   appliance kernel.

4. **Crate-path nit — FIXED in this revision.** `mtls_resolve_adapter.rs:214/:293`
   lives in **`overdrive-control-plane`**, not `overdrive-worker`. The egress-resolve
   `[REF]` sites in feature-delta.md (lines 54, 72) now carry the explicit crate
   qualifier.
