# Feature Delta — `canonical-workload-address-inbound-tproxy`

**Wave:** DESIGN · **Paradigm:** OOP Rust (CLAUDE.md) · **Scope:** Application ·
**Density:** lean (Tier-1 `[REF]` sections) · **Mode:** propose (model inherits;
the design is FULLY LOCKED by three Tier-3 spikes — this artifact pins the last
two implementation-shape sub-choices and records the locked decisions)

The **keystone slice** of the transparent-mtls-enrollment arc (GH #236 / ADR-0071
Path A). It productionises the inbound nft-TPROXY install that ADR-0071's
`start_alloc` deferred (`tproxy_guard = None`) and flips the
`BackendDiscoveryBridge` advertise addr to the canonical per-workload
`workload_addr` so the egress `MtlsResolve` index classifies a dial to that
address as Mesh. With this slice, a workload can be reached at its canonical
`workload_addr:service_port` over mTLS, driven end-to-end through `overdrive
serve` + `overdrive deploy` — closing the inbound half of the loop that
ADR-0071 named as #241's job.

**Builds on** the already-shipped Path-A per-workload netns/veth + /30 routing
(ADR-0071, D-TME-12) and the egress `MtlsResolve` enrollment port (ADR-0071 C1–C4).
This feature is *production wiring + one canonical-address contract*, not a new
subsystem. **Zero CREATE-NEW components** — every touched component is EXTEND or
REUSE (see § Reuse Analysis).

The three load-bearing questions are settled empirically, not by review:

- **increment-a** (`spike/findings.md`): /30 routing + inbound TPROXY capture
  PROVEN on kernel 7.0; the recipe is EXACTLY the existing production triple
  (`ensure_shared_routing_infra` + `install_inbound_tproxy` +
  `make_transparent_listener`) — **no new routing primitive.**
- **increment-b** (`spike/findings-cgroup-firing-scope.md`):
  `cgroup_connect4_service` (ADR-0053 LB hook) **FIRES** for Path-A
  netns+cgroup connects → "just retire it" is FALSIFIED → the reconciliation
  must GATE or TEACH.
- **increment-c** (`spike/findings-vip-lb-inert.md`): under a real `serve` +
  `deploy`, the VIP/XDP-LB path has **no live v1 consumer** → B2 is SAFE → **GATE
  is correct and sufficient; TEACH is unnecessary** until a VIP-dial path ships.

---

## [REF] Verified facts (settled at WRITE time)

| Fact | Verified value | Source |
|---|---|---|
| `AllocationSpec` is pure in-memory | derives only `Debug, Clone, PartialEq, Eq` — **NO serde, NO rkyv**, never persisted, recomputed each reconcile tick | `crates/overdrive-core/src/traits/driver.rs:130-196` |
| `AllocationSpec` already carries slot-derived `Option<String>` channels | `netns`, `host_veth` set ONLY at the C3 site off `plan` | `driver.rs:177,195` + `action_shim/mod.rs:822-823` |
| C3 provision site | `provision_and_inject_netns` sets `spec.netns`/`spec.host_veth` off `plan`; `plan.workload_addr` already computed | `action_shim/mod.rs:792-824`, `veth_provisioner.rs:539` |
| Inbound deferral site | `start_alloc` records `tproxy_guard = None`, comment names "`AllocationSpec` carries no listen-addr field" as the v1 gap #241 closes | `mtls_intercept_worker.rs:590-609` |
| `install_inbound_tproxy` signature | `fn install_inbound_tproxy(virt: SocketAddrV4, agent_port: u16) -> Result<TproxyInterceptGuard>` | `mtls_intercept.rs:248` |
| `leg_c_addr` available inline in `start_alloc` | captured at `mtls_intercept_worker.rs:585` BEFORE the listener moves into the accept loop; comment says #241 reads the inline local, NOT `self.leg_c_addr(alloc)` | `mtls_intercept_worker.rs:576-588` |
| Bridge advertise addr | `addr: SocketAddr::new(IpAddr::V4(self.host_ipv4), listener.port.get())` — the host_ipv4→workload_addr flip site | `backend_discovery_bridge.rs:349` |
| Bridge `actual.running` shape | `RunningAllocSet.running: BTreeSet<AllocationId>` — carries alloc IDs but **NO per-alloc address** today | `backend_discovery_bridge.rs:137-148` |
| Bridge hydrate source | `hydrate_actual` reads `obs.alloc_status_rows()`, filters `state == Running` | `reconciler_runtime.rs:2560` |
| Hydrator LOCAL/REMOTE partition | `partition(\|b\| match b.addr.ip() { V4(v4) => v4 == host_ipv4, V6 => false })`; LOCAL → `push_register_local_backend_actions`, REMOTE → `DataplaneUpdateService` | `service_map_hydrator.rs:340-375` |
| Egress resolve index key | `by_addr: BTreeMap<SocketAddrV4, BTreeMap<ServiceId, Backend>>` — ownership-aware, keyed on `Backend.addr` (B2's reader) | `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:214` (NOT `overdrive-worker`) |
| `Listener.port` type | **`NonZeroU16`** | `aggregate/workload_spec.rs:544` |
| `project_probe_descriptors` mirror | `Service(svc) => extend(probes); Job/Schedule => Vec::new()` | `workload_lifecycle.rs:1078-1094` |
| `NetSlot` / `WORKLOAD_SUBNET_BASE` / `base+slot*4+2` | all in **`overdrive-control-plane`**, NOT `overdrive-core` (the bridge's crate) | `veth_provisioner.rs:296-564` |
| `AllocStatusRowEnvelope` | currently `V1`-only; `discriminant_offset_from_end() = 212` | `observation_store.rs:643,768` |
| `ip_forward` + /30 routes + `rp_filter` | already owned & converged by `veth_provisioner`'s `workload_converge_steps` (`EnableIpForward`) | ADR-0071 D-TME-12 / spike `findings.md` Net recipe |

---

## [REF] DDD — decision list

This feature touches one bounded context (mesh dataplane / workload identity)
and the control-plane composition root + node-agent worker. No new aggregates,
no new ports. DDD verdicts:

| D# | Decision | Verdict | Rationale |
|---|---|---|---|
| **D-A1** | Keystone install: add `pub workload_addr: Option<Ipv4Addr>` + `pub service_ports: Vec<NonZeroU16>` to `AllocationSpec` (pure in-memory, same channel as `netns`/`host_veth`); in `start_alloc` replace `tproxy_guard = None` with one `install_inbound_tproxy` per declared service port | **Production wiring** | `AllocationSpec` is the existing slot-derived per-alloc channel (no serde/rkyv); `workload_addr` set at the C3 `provision_and_inject_netns` site from `plan.workload_addr`; `service_ports` set by `WorkloadLifecycle` via a new `project_service_listen_ports` mirroring `project_probe_descriptors`. N listeners → N inbound rules; Job-kind (0 listeners) → 0 rules. Closes the named `tproxy_guard = None` deferral. |
| **D-BLOCKER1** | Inbound rule keys on `ip daddr <workload_addr> tcp dport <service_port>`; `service_port` = the declared Service listener port (option (a), per D-TME-10) | **Declared service port** | D-TME-10 one-source/two-readers: the same value `service_backends` advertises and the egress `MtlsResolve` keys on (`MtlsResolve::resolve` keys on full `(addr,port)`, `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:293`). NOT the ephemeral `leg_c_addr.port()` (a self-referential rule matching no real inbound connection — the inert shape `start_alloc` rejected), NOT all-TCP, NOT a hardcoded port. |
| **D-B2** | `BackendDiscoveryBridge` advertises `Backend.addr = workload_addr:port` instead of `host_ipv4:port` | **Canonical address** | What makes the egress `ServiceBackendsResolve` index classify a dial to the canonical `workload_addr` as Mesh (else inbound fail-closed). The `vip` field on `ServiceBackendRow` is **UNCHANGED** (the dialable-VIP path is #61 territory, orthogonal; the VIP *allocator* #167 already shipped). Proven SAFE by increment-c (no live VIP-LB consumer to break). |
| **D-BLOCKER2** | Bridge `workload_addr` input source: **persist `workload_addr: Option<Ipv4Addr>` directly on `AllocStatusRow` (option b)**, NOT persist-`NetSlot`-and-recompute | **Persist the materialized addr** | See § BLOCKER-2 analysis below. The bridge lives in `overdrive-core`; the slot→addr derivation + `WORKLOAD_SUBNET_BASE` live in `overdrive-control-plane`. Recompute-at-bridge (option a) requires a core relocation AND re-derives against a future-tunable base (#239) — the SAME inputs read at two different times can diverge → installs the inbound rule on one addr while advertising another. Persisting the *materialized* addr the C3 site already computed keeps the addr the rule was installed on byte-identical with the addr the bridge advertises. → `AllocStatusRowEnvelope::V2`. |
| **D-GATE** | Gate `ServiceMapHydrator` so Path-A/`workload_addr` backends are NOT registered into `LOCAL_BACKEND_MAP` (no `RegisterLocalBackend`) and NOT programmed into the XDP `SERVICE_MAP`/`REVERSE_NAT_MAP` (no `DataplaneUpdateService`) | **GATE (reconcile ADR-0053↔ADR-0071)** | increment-b: the `cgroup_connect4_service` hook FIRES for Path-A connects → a `LOCAL_BACKEND_MAP` miss is needed so the dial falls through to nft-TPROXY. increment-c: B2 reclassifies these backends LOCAL→REMOTE (`workload_addr ≠ host_ipv4`) → they would start programming the XDP path — **dead writes** no dial consults. Both branches dead → gate them. TEACH rejected (no VIP-dial consumer to keep serving until a dialable-VIP path ships — #61; the VIP *allocator* #167 already shipped, the headless name responder #243 returns the `workload_addr` not a VIP). |
| **D-GATE-PRED** | GATE predicate: skip LB programming for any backend whose `addr.ip() ∈ WORKLOAD_SUBNET_BASE (10.99.0.0/16)` | **Subnet-membership** | See § GATE-predicate analysis below. With B2 every Path-A backend is `workload_addr ∈ WORKLOAD_SUBNET_BASE` by construction; no non-mesh consumer exists (increment-c: all exec allocs are TPROXY-intercepted, no mesh flag exists). IN-SCOPE for #241 (prevents the dead XDP writes B2 introduces — "don't ship dead writes in your slice"). |
| **D-C1** | Reuse the existing lazy idempotent `ensure_shared_routing_infra` (already production-reached via the outbound install); no new boot call site | **REUSE AS-IS** | Bar-1 converge-on-boot shared infra. Bar-2 boot-time promotion stays deferred to **#234**. |
| **D-D1** | `ip_forward` + `/30` routes + `rp_filter` already owned & converged by `veth_provisioner`'s `workload_converge_steps` (`EnableIpForward`); this slice changes nothing | **Confirm-and-cite** | Spike `findings.md`: capture is forwarding-independent; `ip_forward` is the separate workload↔workload reachability concern, already converged. No change here. |

---

## [REF] BLOCKER-2 — the bridge `workload_addr` input source (PINNED: option b)

**Decision: persist `workload_addr: Option<Ipv4Addr>` directly on `AllocStatusRow`,
recompute NOTHING at the bridge.**

The `BackendDiscoveryBridge` (`overdrive-core`) advertises `Backend.addr`. Today
it synthesizes the addr from a single `host_ipv4` scalar; its actual-side input
`RunningAllocSet.running: BTreeSet<AllocationId>` carries **no per-alloc
address**. For B2 the bridge needs the per-alloc `workload_addr` as an OBSERVED
input. The observation surface is `AllocStatusRow` (the bridge hydrates from
`obs.alloc_status_rows()`). Two candidate shapes:

- **(a) persist `NetSlot`, recompute `workload_addr = base + slot*4 + 2` at the
  bridge.** Nominally "persist-inputs-correct" (the slot is the input; the addr
  is derived). **REJECTED for two compounding reasons:**
  1. **Core relocation.** `NetSlot`, `WORKLOAD_SUBNET_BASE`, and the
     `base + slot*4 + 2` derivation all live in `overdrive-control-plane`
     (`veth_provisioner.rs`); the bridge lives in `overdrive-core`. Recompute
     at the bridge requires relocating the base const + derivation into core —
     widening the core surface for a value core does not otherwise own.
  2. **The #239 tunable-base hazard inverts the persist-inputs argument.** The
     base is a future operator-tunable (**#239**, OPEN, phase/2+). Persist-inputs
     says "recompute from inputs + the *live* policy" — but here the live policy
     is the base, and the addr was *already materialized* against the base in
     force at provision time. If #239 lets the base change between the C3
     provision (which installs the inbound nft rule on `workload_addr =
     base_t0 + slot*4 + 2`) and a later bridge reconcile-tick recompute
     (`base_t1 + slot*4 + 2`), the recomputed advertised addr **diverges from the
     addr the inbound rule was installed on** — the egress resolve would classify
     Mesh on an addr no inbound rule captures. The slot is a *stable* input, but
     the addr is a *join* of slot × base-at-provision-time, and only the
     provision-time materialization captures that join. Recompute re-joins
     against possibly-drifted base → wrong addr. (This is the `next_attempt_at`
     anti-pattern's mirror image: here the *materialized join* IS the input the
     downstream contract depends on, because the inbound rule already committed
     to it.)
- **(b) persist `workload_addr: Option<Ipv4Addr>` directly on `AllocStatusRow`.**
  **PINNED.** The `workload_addr` the C3 site computes (`plan.workload_addr`) is
  the SAME value the inbound nft rule is keyed on AND the value the bridge must
  advertise — one materialization, three readers (inbound rule, persisted row,
  bridge advertise). Persisting it keeps the addr byte-identical across the
  install, the observation, and the advertise. The `#239`-tunable-base risk is
  a **documented single-cut-greenfield risk**: Phase-1 single-node ships ONE
  base; a future base change is a redeploy of every alloc (its netns + rule are
  re-provisioned against the new base, re-observing the new `workload_addr`),
  not a live re-tune of running allocs — so the "stale derived value" failure
  mode of persisting a derived addr does not bite within a deployment's life.
  The addr is the slot×base join the inbound contract *already committed to*;
  persisting that exact join is persisting the input the contract depends on.

**Field + envelope shape (PINNED):**

- New field on `AllocStatusRowV2`: `pub workload_addr: Option<Ipv4Addr>`. `Some`
  only on a Path-A (mTLS-composed) alloc that provisioned a netns; `None` for
  every host-netns workload (every current fixture) — symmetric with
  `AllocationSpec.netns`/`host_veth`. The exit-observer write path copies
  `spec.workload_addr` → row (an observed input: the addr the node provisioned
  this alloc into).
- **`AllocStatusRowEnvelope::V2`** per the 6-step procedure in `development.md`
  § "rkyv schema evolution":
  1. Append `V2(AllocStatusRowV2)` to the envelope enum; re-alias
     `pub type AllocStatusRow = AllocStatusRowV2`.
  2. `pub type AllocStatusRowLatest = AllocStatusRowV2`.
  3. `latest(p) -> Self { Self::V2(p) }`.
  4. `From<AllocStatusRowV1> for AllocStatusRowV2` (additive: `workload_addr:
     None`); `into_latest()` chains `V1 => Ok(v1.into())`, `V2 => Ok(v2)`.
  5. Add `FIXTURE_V1` golden-bytes test pinning the V1 archived bytes
     (`tests/schema_evolution/alloc_status_row.rs`) — **never touch any existing
     fixture**; add the V2 fixture + assertion in the same commit.
  6. Re-pin `discriminant_offset_from_end()` + `GOLDEN_DISCRIMINANT_OFFSET_V1`
     via the triangulation test (adding `Option<Ipv4Addr>` — 4 bytes behind the
     `Option` discriminant — shifts the trailing root footprint).
- **Hydrate path:** `hydrate_actual` carries the per-alloc addr into the bridge
  state. `RunningAllocSet.running` becomes
  `BTreeMap<AllocationId, Option<Ipv4Addr>>` (was `BTreeSet<AllocationId>`) — the
  Running alloc → its `workload_addr`. The bridge reads
  `actual.running[alloc]` and advertises `workload_addr:port` when `Some`,
  falling back to `host_ipv4:port` when `None` (the non-mesh / host-netns alloc,
  unchanged behaviour). `BTreeMap` not `HashMap` per § "Ordered-collection
  choice" (the bridge fingerprints the iterated backend set deterministically).

---

## [REF] GATE-predicate — how `ServiceMapHydrator` identifies backends to skip (PINNED)

**Predicate: skip LB programming (both `RegisterLocalBackend` and
`DataplaneUpdateService`) for any backend whose `addr.ip()` is an IPv4 address
within `WORKLOAD_SUBNET_BASE` (`10.99.0.0/16`).**

With B2, every Path-A backend is `workload_addr = WORKLOAD_SUBNET_BASE.network()
+ slot*4 + 2` by construction — so subnet membership is the structural,
content-derived signal that a backend is a Path-A mesh workload owning its own
netns (delivery owned by nft-TPROXY), not a host-netns backend (delivery owned
by the cgroup/XDP LB). The hydrator gains a `workload_subnet: Ipv4Net` mandatory
constructor parameter (the same `WORKLOAD_SUBNET_BASE` the provisioner uses —
ONE source) per `development.md` § "Port-trait dependencies — Required, not
defaulted." The partition becomes a three-way split applied BEFORE the existing
LOCAL/REMOTE partition:

```text
mesh   = backends where addr.ip() ∈ workload_subnet   → emit NOTHING (nft-TPROXY owns delivery)
local  = remaining backends where addr.ip() == host_ipv4 → RegisterLocalBackend (unchanged)
remote = remaining backends otherwise                  → DataplaneUpdateService (unchanged)
```

**Why subnet-membership and not another signal:**

- **No "mesh flag" exists** (increment-c). There is no per-backend boolean to
  key on; the addr's subnet IS the classification, exactly as
  `addr == host_ipv4` IS the LOCAL classification today. This extends the
  hydrator's existing partition-on-addr model rather than inventing new state.
- **It cannot misfire on a non-mesh consumer** because there is none
  (increment-c: every exec alloc is TPROXY-intercepted; the VIP/LB path has no
  live v1 consumer). A backend lands in `workload_subnet` IFF it is a Path-A
  workload_addr, which IFF it is mesh.
- **It is deterministic and content-derived** — no last-writer-wins, no
  cross-component invariant, consistent with the egress index's
  any-healthy-at-addr determinism (ADR-0071 C4 F-A).

**Why IN-SCOPE for #241 (not deferred):** B2 (in this slice) reclassifies these
backends LOCAL→REMOTE, which without the gate starts emitting
`DataplaneUpdateService` → live XDP `SERVICE_MAP`/`REVERSE_NAT_MAP`/`BACKEND_MAP`
writes that no dial ever consults (increment-c). Shipping B2 without the gate
ships **dead writes** in this slice — the exact "don't ship dead writes in your
slice" trap. The gate is the cost of B2's correctness, not a separable concern;
they land together.

---

## [REF] Component decomposition (paths + change type)

| Component | Path | Change type | What changes |
|---|---|---|---|
| `AllocationSpec` | `crates/overdrive-core/src/traits/driver.rs:130-196` | **EXTEND** | Add `pub workload_addr: Option<Ipv4Addr>` + `pub service_ports: Vec<NonZeroU16>` (pure in-memory, no serde/rkyv — SAME channel as `netns`/`host_veth`). Both `None`/empty for host-netns workloads (every fixture). |
| C3 provision seam | `crates/overdrive-control-plane/src/action_shim/mod.rs:792-824` | **EXTEND** | Add `spec.workload_addr = Some(plan.workload_addr)` beside the existing `spec.netns`/`spec.host_veth` injection (off the same `plan`). |
| `WorkloadLifecycle` desired projection | `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs:1078` (+ hydrate-desired site) | **EXTEND** | New `project_service_listen_ports(intent) -> Vec<NonZeroU16>` mirroring `project_probe_descriptors`: `Service(svc) => svc.listeners.iter().map(\|l\| l.port).collect()`, `Job/Schedule => Vec::new()`. Threaded into the emitted `AllocationSpec.service_ports` at the hydrate-desired boundary (`reconciler_runtime.rs:2317`-shape). |
| `MtlsInterceptWorker::start_alloc` | `crates/overdrive-worker/src/mtls_intercept_worker.rs:590-609` | **EXTEND** | Replace `tproxy_guard = None`: for each `port` in `spec.service_ports`, when `spec.workload_addr` is `Some(addr)`, call `install_inbound_tproxy(SocketAddrV4::new(addr, port.get()), leg_c_addr.port())` and retain the returned `TproxyInterceptGuard`(s) for the alloc lifetime (one guard per port). N ports → N rules; `None` addr / empty ports → 0 rules (the host-netns/Job path, unchanged). Reuses the inline `leg_c_addr` local (mtls_intercept_worker.rs:585). |
| `install_inbound_tproxy` | `crates/overdrive-worker/src/mtls_intercept.rs:248` | **REUSE AS-IS** | Already the named #241 production-install site; signature `(virt: SocketAddrV4, agent_port: u16)` unchanged. Now *called* from `start_alloc` per port. |
| `AllocIntercept` guard set | `crates/overdrive-worker/src/mtls_intercept_worker.rs:260-265` | **EXTEND** | `_tproxy_guard: Option<TproxyInterceptGuard>` becomes `_inbound_tproxy_guards: Vec<TproxyInterceptGuard>` (N listeners → N RAII guards, dropped on alloc teardown). |
| `BackendDiscoveryBridge` | `crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs:343-353` | **EXTEND** | Advertise `addr = workload_addr:port` (from `actual.running[alloc]`) when `Some`, else `host_ipv4:port` (unchanged). `vip` field UNCHANGED. |
| `RunningAllocSet` | `crates/overdrive-core/src/reconcilers/backend_discovery_bridge.rs:137-148` | **EXTEND** | `running: BTreeSet<AllocationId>` → `running: BTreeMap<AllocationId, Option<Ipv4Addr>>` (per-alloc `workload_addr`). |
| `AllocStatusRow` | `crates/overdrive-core/src/traits/observation_store.rs:642-783` | **EXTEND** | `AllocStatusRowEnvelope::V2` + `AllocStatusRowV2 { …, workload_addr: Option<Ipv4Addr> }` + `From<V1> for V2` + golden fixtures + re-pinned discriminant offset (BLOCKER-2 6-step). |
| Exit-observer / status write path | `crates/overdrive-worker/src/exit_observer.rs` (alloc-status write) | **EXTEND** | Copy `spec.workload_addr` into the written `AllocStatusRowV2.workload_addr` (an observed input). |
| `hydrate_actual` | `crates/overdrive-control-plane/src/reconciler_runtime.rs:2529-2666` | **EXTEND** | Populate `RunningAllocSet.running` map with each Running row's `workload_addr`. |
| `ServiceMapHydrator` | `crates/overdrive-core/src/reconcilers/service_map_hydrator.rs:340-375` | **EXTEND** | Add `workload_subnet: Ipv4Net` mandatory ctor param; three-way split (mesh→skip / local→`RegisterLocalBackend` / remote→`DataplaneUpdateService`) — the GATE. |
| Hydrator construction site | `crates/overdrive-control-plane/src/` (hydrator `canonical(host_ipv4)` call site) | **EXTEND** | Pass `WORKLOAD_SUBNET_BASE` (the one source) to the new ctor param. |
| `ensure_shared_routing_infra` | `crates/overdrive-worker/src/mtls_intercept.rs` | **REUSE AS-IS** | Lazy idempotent shared infra; already production-reached via the outbound install (D-C1). |
| `veth_provisioner` `ip_forward`/routes/`rp_filter` | `crates/overdrive-control-plane/src/veth_provisioner.rs` | **REUSE AS-IS** | Already owned & converged (D-D1). No change. |

**Zero CREATE-NEW.** The only *new* named symbols are a function
(`project_service_listen_ports`), an additive struct field
(`AllocStatusRowV2.workload_addr`, `AllocationSpec.{workload_addr,service_ports}`),
and an additive ctor param (`ServiceMapHydrator.workload_subnet`) — all on
EXTENDED components. No new trait, no new port, no new adapter, no new crate.

---

## [REF] Driving ports (inbound)

| Port | Adapter | Notes |
|---|---|---|
| Operator CLI — `overdrive serve` | `overdrive-cli::commands::serve` → `run_server` | Boot composition root; the gated hydrator + the reused shared-routing infra stand up here. No new boot call site (D-C1). |
| Operator CLI — `overdrive deploy <SPEC>` | `commands::deploy` → action-shim `StartAllocation` | The C3 seam injects `workload_addr`; `WorkloadLifecycle` injects `service_ports`; `start_alloc` installs the inbound rule(s). The keystone loop is driven here end-to-end. |

No new driving ports — both are existing verbs gaining the inbound-capture
behaviour they were missing.

---

## [REF] Driven ports / adapters

No new driven ports. The egress `MtlsResolve` port (ADR-0071) is the
**downstream reader** of B2's `workload_addr` advertise — UNCHANGED in shape; its
`by_addr` index now classifies a canonical-addr dial as Mesh because
`service_backends` now carries `workload_addr`. The `Dataplane` port (ADR-0053)
is gated at the hydrator (no new method; the existing `register_local_backend` /
`update_service` are simply NOT called for mesh backends).

**External integration surface:** none. This is an internal kernel/dataplane
wiring slice — no third-party API, no contract-test annotation needed.

---

## [REF] Technology choices

| Choice | Selection | Rationale | License |
|---|---|---|---|
| Inbound capture | nft-TPROXY + `IP_TRANSPARENT` + `getsockname` (existing) | Proven (increment-a); EXACTLY the production triple; no new primitive | n/a (kernel/`nft`/`ip` CLI) |
| Per-alloc addr channel | `AllocationSpec` in-memory field | Existing slot-derived channel; no schema/persistence cost | n/a |
| Persisted observed addr | `AllocStatusRow` rkyv `V2` envelope | The state-layer-correct observation surface; versioned-envelope discipline | rkyv (MIT/Apache-2.0) |
| GATE classification | subnet-membership on `WORKLOAD_SUBNET_BASE` | Content-derived, deterministic; extends the existing partition-on-addr model | n/a |

All OSS / in-tree. No proprietary technology. No new dependency.

---

## [REF] Decisions table (summary)

| D# | One-line | In-scope #241? |
|---|---|---|
| D-A1 | `AllocationSpec.{workload_addr,service_ports}` + per-port `install_inbound_tproxy` in `start_alloc` | YES (keystone) |
| D-BLOCKER1 | inbound rule keys on declared `service_port` (one source / two readers) | YES |
| D-B2 | bridge advertises `workload_addr:port`; `vip` unchanged | YES |
| D-BLOCKER2 | persist `workload_addr` on `AllocStatusRow` (V2), not recompute from slot | YES |
| D-GATE | gate the hydrator off mesh backends (no LB programming) | YES |
| D-GATE-PRED | gate predicate = `addr ∈ WORKLOAD_SUBNET_BASE` | YES |
| D-C1 | reuse `ensure_shared_routing_infra` (Bar-2 → #234) | YES (reuse) |
| D-D1 | `ip_forward`/routes/`rp_filter` already converged | confirm-only |

---

## [REF] Reuse Analysis (HARD GATE — every touched component)

| Component | EXTEND / REUSE / NEW | Justification (no existing alternative for NEW) |
|---|---|---|
| `AllocationSpec` | **EXTEND** | Existing per-alloc in-memory channel; add two fields alongside `netns`/`host_veth`. No NEW spec type warranted. |
| C3 provision seam | **EXTEND** | Existing site already sets slot-derived fields off `plan`; one more assignment. |
| `WorkloadLifecycle` projection | **EXTEND** | Mirror the existing `project_probe_descriptors` projection; no new reconciler. |
| `start_alloc` inbound install | **EXTEND** | The named #241 production-install site; replace the `tproxy_guard = None` deferral. |
| `install_inbound_tproxy` | **REUSE** | Already the install primitive; signature unchanged. |
| `BackendDiscoveryBridge` advertise | **EXTEND** | One addr-source change at line 349; no new reconciler. |
| `RunningAllocSet` | **EXTEND** | Set→Map widening to carry the addr; no new type. |
| `AllocStatusRow` | **EXTEND** | Additive `V2` envelope per existing schema-evolution discipline; no new row type. |
| Exit-observer write path | **EXTEND** | Existing write path copies one more observed input. |
| `hydrate_actual` | **EXTEND** | Existing hydrate path populates one more field. |
| `ServiceMapHydrator` GATE | **EXTEND** | Extends the existing partition-on-addr classifier with a third arm; no new reconciler. |
| `ensure_shared_routing_infra` | **REUSE** | Existing shared-infra converge, already production-reached. |
| `veth_provisioner` ip_forward/routes | **REUSE** | Already converged (D-D1). |

**Zero NEW (CREATE) components.** The prior framing established this; this slice
holds it. Every change is additive-on-existing or a pure reuse.

---

## [REF] Open questions / deferrals

| Item | Disposition | Issue |
|---|---|---|
| Shared-routing-infra Bar-2 reconciler | Deferred (Bar-1 converge-on-boot suffices for single-node v1) | **#234** (OPEN) |
| Tunable `WORKLOAD_SUBNET_BASE` | Deferred; the BLOCKER-2 single-cut-greenfield risk (persisted derived addr vs a future tunable base) is documented and accepted for single-node v1 | **#239** (OPEN, phase/2+) |
| Intended-peer SVID pinning (`expected_peer` SAN-match) | Out of scope (v1 authn-only) | **#242** |
| In-agent name responder (dial-by-name) | Out of scope (workload dials concrete `workload_addr` directly — the thin live loop) | **#243** |
| VIP-dial path / multi-node VIP-LB | Out of scope (no live VIP-dial consumer; TEACH not needed until a dialable-VIP path ships — the VIP *allocator* #167 already shipped, so the trigger is the open dialable-VIP territory #61, recorded durably by the ADR-0053 amendment) | **#61** (OPEN) |
| (Context) #178 | CLOSED — split into #241 (this inbound half) / #242 / #243 / #244 | **#178** (CLOSED) |

**No NEW deferral surfaced.** No hand-wavy forward pointers; every cited issue is
verified OPEN/CLOSED per the dispatch. If a crafter finds a genuine gap needing a
new issue, surface it as a blocker — do not invent.
