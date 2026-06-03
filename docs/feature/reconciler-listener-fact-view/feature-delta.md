# Feature Delta — `reconciler-listener-fact-view`

**Wave**: DESIGN (escalation from /nw-bugfix → /nw-research → /nw-design; no DISCUSS).
**Architect**: Morgan. **Mode**: propose. **Density**: lean.
**Paradigm**: OOP (Rust) — project SSOT, not re-litigated.
**Decided direction (locked upstream)**: a **hydration-layer efficiency fix** (decision label: candidate (d)) — `ServiceMapHydrator` is already a full reconciler; this fixes how its `hydrate_desired` sources one input. Listener facts are folded into a runtime in-memory store maintained on the intent-change edge, so steady-state `hydrate_desired` pays **zero redb reads**.

Design inputs (no DISCUSS artifacts exist):
- RCA: `reconciler_runtime.rs:1733-1796` `gather_service_listener_facts` — full `scan_prefix(b"workloads/")` + rkyv-decode of every Service intent + `allocator.lock().await` per intent, **once per `ServiceMapHydrator` target per ~100 ms tick** → O(S²) decodes + O(S²) lock acquisitions per active tick.
- Research: `docs/research/control-plane/reconciler-desired-hydration-efficiency.md` (18 sources, High) — Kubernetes informer/indexer model; field-indexer scoped lookup; **writer-bumped monotonic generation** as the canonical invalidation key; ranks (d) first.

---

## Wave: DESIGN / [REF] Problem Restatement

The derived value `ListenerRow { vip: Some(_), port, protocol }` is **stable between operator spec submissions** — it changes only on `submit_workload` / `stop_workload`. It is recomputed from two inputs every tick:

1. The Service intent's `listeners` (the per-listener `(port, protocol)` SSOT — `ServiceBackendRow` and `alloc_status` carry no protocol; ADR-0060 C3).
2. The allocator-issued VIP (`PersistentServiceVipAllocator::get(&spec_digest)`).

Both inputs are present and already touched at the intent-change write edge in `handlers.rs:323-331` (allocate) and `handlers.rs:424-432` (release on conflict). The defect is that the read path re-derives on a timer instead of reading a cache maintained on the write edge — the exact anti-pattern the research names.

This is a **hydration-layer efficiency fix**, not a reconciler triage (`.claude/rules/reconcilers.md`): `ServiceMapHydrator` is already a full reconciler; we are fixing how its `hydrate_desired` sources one input.

---

## Wave: DESIGN / [REF] Locked Sub-Decisions (1–5)

### Sub-decision 1 — Where the listener-fact view lives  ⟶  **LOCKED: (ii) in-memory-only `Arc<Mutex<ListenerFactStore>>`, rebuilt at boot, maintained on the intent-change edge**

A new, dedicated in-memory store, wrapped `Arc<tokio::sync::Mutex<…>>`, held on `AppState` beside `allocator`. It holds two `BTreeMap`s: a **primary** `BTreeMap<ServiceId, ListenerRow>` (the read-path key — see sub-decision 4) and a **secondary** `BTreeMap<WorkloadId, Vec<ServiceId>>` cleanup index used only by the stop/conflict-release path (which holds a `WorkloadId`, not the `ServiceId`s). Rebuilt at boot by an existing `gather`-style scan (the *only* surviving scan call site), which derives each listener's `ServiceId` during the scan and populates both maps, then mutated incrementally on submit/stop.

| Option | What | Verdict |
|---|---|---|
| (i) First-class `ViewStore`-persisted reconciler View | Full ADR-0035 write-through CBOR blob, survives reboot, bulk-loaded at register; needs a reconciler to *own* a View. | **Rejected.** No reconciler owns "cluster-wide listener facts" — `ServiceMapHydrator`'s View is keyed per service target, not a cluster aggregate. Fabricating a synthetic reconciler-with-no-`reconcile` to host a View abuses the ADR-0035 contract (View = a reconciler's typed memory). The facts are a *projection of intent the allocator already re-derives at boot*, not durable reconciler memory — persisting them duplicates the intent SSOT (violates "persist inputs, not derived state": the derived `ListenerRow` would be a stale cache of `s.listeners` the moment the listener-projection logic changes). Heaviest surface for zero durability benefit. |
| (ii) In-memory `Arc<Mutex<…>>`, boot-rebuilt, edge-maintained | Mirrors `PersistentServiceVipAllocator`'s lifecycle (rebuilt at boot from a scan, mutated on the edge) — *except* it does NOT persist (the intent store is the SSOT; cold boot re-projects). | **SELECTED.** Cold-boot rebuild reuses the existing scan (renamed/relocated), zero new persistence schema, zero rkyv envelope, and the facts are re-derived from the live intent SSOT at boot — exactly honoring "persist inputs, not derived state." Memory is negligible (a few bytes × S, research Finding 11). Steady-state read = O(1) keyed lookup → zero redb reads (the locked goal). |
| (iii) Extend `PersistentServiceVipAllocator` to carry `spec_digest → (VIP, Vec<Listener>)` | Smallest diff — listeners ride alongside the VIP they're derived with, in the one structure already locked on this edge and already keyed by `spec_digest`. | **Rejected on cohesion.** The allocator's single responsibility is VIP issuance + range management: IPv4-only invariant, Earned-Trust range-projection boot probe, pool-exhaustion semantics, `ServiceVipAllocatorEntryV2 { spec_digest, vip }` rkyv envelope. Folding `listeners` in (a) muddies that SRP (an "allocator" that is secretly a service-facts store), (b) forces a `ServiceVipAllocatorEntryV3` rkyv version bump to persist listeners the allocator does not need persisted, (c) couples listener-fact lifecycle to allocator lifecycle (release-on-conflict, range-narrowing refusal) for no benefit. The keying is also wrong: the hydrator reads facts *by service / VIP*, while the allocator is keyed by `spec_digest`. |

**Why (ii) over (i):** (i) is the textbook informer-cache shape *for reconciler Views*, but listener facts are not a reconciler's memory — they are a cluster-wide projection of intent that the allocator's own boot rebuild already demonstrates can be reconstructed from the intent SSOT without persistence. (ii) gets the same steady-state property (zero durable reads) as (i) at a fraction of the surface, and it is the closest existing precedent (`PersistentServiceVipAllocator` minus persistence). ADR-0035's "zero steady-state durable reads" contract is *satisfied in spirit and in effect*; we are not adding a new persisted View because there is no durable state to persist — the intent store already is it.

### Sub-decision 2 — Who owns the write edge  ⟶  **LOCKED: the `submit_workload` / `stop_workload` post-allocate hook, co-located with the existing VIP-memo update**

The listener-fact update is performed in `handlers.rs` immediately adjacent to the allocator mutation that already runs on this edge:
- **On submit** (`handlers.rs:323-331`, `PutOutcome::Inserted` path): after `allocate(digest)` returns the `ServiceVip` and the intent is committed via `put_if_absent`, for **each** listener compute `ServiceId::derive(&vip, listener.port, "service-map")` and insert `ServiceId → ListenerRow { vip: Some(vip), port, protocol }` into the primary map, recording the workload's `ServiceId`s in the secondary index.
- **On conflict release** (`handlers.rs:424-432`): the rejected spec's VIP is released; no facts are inserted (insert happens only on the `Inserted` outcome, after the intent commit — symmetric with the VIP, which is allocated before commit but released on conflict).
- **On stop** (`stop_workload`, `handlers.rs:642-681`): `remove_workload(&workload_id)` evicts every `ServiceId` the secondary index maps for that workload, then drops the secondary entry. `stop_workload` holds only the `WorkloadId` (it writes a stop sentinel; it does NOT decode the intent or hold the VIP — the VIP release is downstream on the convergence tick via `Action::ReleaseServiceVip`), so the secondary index is what makes the eviction possible without re-deriving the `ServiceId`s.

| Option | Verdict |
|---|---|
| `submit_workload`/`stop_workload` post-commit hook (direct) | **SELECTED.** The handler already decodes `WorkloadIntent::Service(s)` (has `s.listeners`) AND holds/acquires the allocator (has the VIP) on this exact edge. Co-locating the fact update with the VIP-memo update makes a **single atomic intent-change edge** — the property the research calls "writer-bumped invalidation." No new indirection. |
| Action-shim (`Action::UpdateListenerFacts`) | **Rejected.** The shim runs *after* `reconcile` on the convergence tick — it is downstream of the timer, not on the intent-write edge. Routing the fact update through an Action re-introduces tick-coupling and a round-trip the handler can do synchronously with data already in hand. |
| Dedicated reconciler observing intent changes | **Rejected.** Over-engineering: a reconciler whose only job is to mirror intent → facts is a second copy of the projection `gather` already does, plus a View, plus eval-broker plumbing — to replace a two-line handler insert. Reconcilers converge real resources; this is a pure in-memory projection update. |

**Ordering discipline:** the fact insert happens **after** the intent `put_if_absent` returns `Inserted` (intent SSOT committed first), mirroring the allocator's fsync-then-memory ordering. On a crash between intent-commit and fact-insert, the next boot's rebuild re-projects from the committed intent — no fact is lost. (The facts are NOT persisted, so there is no fsync to order against; the boot rebuild is the recovery path.)

### Sub-decision 3 — Fate of `gather_service_listener_facts`  ⟶  **LOCKED: retained as the cold-boot rebuild path, relocated + renamed; its per-tick caller deleted**

Not deleted entirely. The function body (scan `workloads/` → decode Service intents → join allocator VIP → emit `ListenerRow`s) is **exactly** the cold-boot rebuild (ii) needs — identical to how `PersistentServiceVipAllocator::bulk_load` rebuilds from a scan. It is:
- **Relocated** to the `ListenerFactStore` boot constructor (e.g. `ListenerFactStore::rebuild_from_intent(state) -> Self`), called once at startup wiring next to allocator `bulk_load`.
- **Renamed** to reflect "boot rebuild," not "per-tick gather."
- Its **per-tick caller deleted** (`reconciler_runtime.rs:1335` — the `let listeners = gather_service_listener_facts(state).await?;` inside the `ServiceMapHydrator` hydrate arm), replaced by the O(1) keyed read (sub-decision 4).

Per project deletion discipline: the per-tick call site and any test that asserts the *per-tick scan behavior* are deleted in the same change; the rebuild logic's tests migrate to assert the *boot-rebuild* behavior (a genuinely different requirement — write new assertions, do not salvage-rename). The function does not become dead code because the boot path is a real, exercised caller.

### Sub-decision 4 — Read-path shape  ⟶  **LOCKED: O(1) keyed read scoped to the target's service, from the in-memory store**

The `ServiceMapHydrator` hydrate arm (`reconciler_runtime.rs:1322-1364`) currently scans cluster-wide then filters per row via `project_service_desired(&row, &listeners)` (matching the listener whose `vip == Some(row.vip)`). The arm **never holds a `WorkloadId`** — it resolves `service_id` from the target (line 1323) and keys its desired map by `row.service_id` (line 1347). So the store MUST be keyed by `ServiceId`, not `WorkloadId`. New shape:

1. The arm already resolves `service_id` from the target and reads `service_backends_rows(&service_id)`; each row carries `row.vip` (`Ipv4Addr`) and `row.service_id` (`ServiceId`).
2. Replace the cluster-wide `gather_service_listener_facts(state).await?` with a **per-row keyed read** of the primary `BTreeMap<ServiceId, ListenerRow>`: `store.get(&row.service_id)` — a genuine O(1) lookup that **directly yields** the `(port, protocol)` for that service. The per-row `vip == row.vip` scan over a cluster-wide `Vec<ListenerRow>` is **eliminated**, not relocated.
3. The C3-guard logic stays: a row whose `ServiceId` has no fact entry (unresolvable protocol) skips the service and emits `service_map_hydrator.desired.unresolvable_proto`, refusing to default to `Tcp` (ADR-0060 C3).

The edge derives the key with the exact call `hydrate_bridge_desired_listeners` already uses at `reconciler_runtime.rs:1705` — `ServiceId::derive(&assigned_vip, listener.port, "service-map")`, where `ServiceId::derive(vip: &ServiceVip, port: NonZeroU16, purpose: &str)` (`overdrive-core/src/id.rs:825`) takes the submit handler's `service_vip: Option<ServiceVip>` with no wrapping. (A fallback `Ipv4Addr`-keyed store was considered and rejected — it would still force the hydrator to wrap `row.vip: Ipv4Addr` into `ServiceVip` and apply a port filter; `ServiceId` keying sidesteps both. `WorkloadId` keying is rejected outright — it matches no read-path key.)

**Lock discipline:** acquire the `ListenerFactStore` guard, clone the small value for the row's `ServiceId`, drop the guard before any further `.await` (identical to the allocator-guard pattern at `hydrate_bridge_desired_listeners:1686-1691`). This is **DST invariant C** (sub-decision 5): the read path and the submit/stop edge contend on the same mutex, so a guard held across `.await` is a deadlock hazard, not a style note.

The C3-guard semantics are preserved verbatim: an unresolvable protocol still skips the service and emits `service_map_hydrator.desired.unresolvable_proto` — refusing to default to `Tcp` (ADR-0060 C3).

### Sub-decision 5 — DST / determinism  ⟶  **LOCKED: `BTreeMap` keying; two DELIVER-asserted invariants; sim store supports counting**

- **Collection:** **both** `ListenerFactStore` maps MUST be `BTreeMap` — the primary `BTreeMap<ServiceId, ListenerRow>` and the secondary `BTreeMap<WorkloadId, Vec<ServiceId>>` (the inner `Vec` in insertion/`ServiceId` order) per `.claude/rules/development.md` § "Ordered-collection choice" — the facts are iterated by the boot rebuild and observed by DST invariants; iteration order must be seed-deterministic. No `HashMap` escape hatch.
- **DELIVER regression invariant A (zero steady-state intent-store dependence):** *"`ServiceMapHydrator::hydrate_desired` does not depend on the intent store in steady state."* The property is unchanged; the mechanism is corrected (2026-06-03 — see § Changed Assumptions). The counting-`scan_prefix`-decorator mechanism was found structurally infeasible (the read seam `AppState.store` is concrete, not `dyn`). What shipped: a **delete-intent-then-tick** test — submit S services (1..=3 listeners each), remove the intent record, run N convergence ticks, assert hydrate yields a correct non-empty `desired` every tick. Reachable only if hydrate reads the in-memory `ListenerFactStore`, not the intent store; reverting the read-switch makes it RED. Equivalent-or-stronger than a scan counter (proves the intent record is entirely unnecessary in steady state). This mirrors the existing `WriteThroughOrdering` DST-invariant discipline (research recommendation 2).
- **DELIVER regression invariant B (byte-equivalence, multi-listener):** *"the `ListenerFactStore` contents (both maps) are byte-equivalent to a full re-scan via the boot rebuild, over the full set of `ServiceId` entries for all services including services with multiple `[[listener]]` entries."* Each listener contributes one `ServiceId` entry, so a multi-listener service is asserted entry-by-entry. Assertable by running the boot rebuild against the same intent set and asserting equality with the edge-maintained store — guards against the edge-update path drifting from the rebuild path (the same contract `PersistentServiceVipAllocator` upholds between `allocate` and `bulk_load`).
- **DELIVER regression invariant C (lock never held across `.await`):** the `ListenerFactStore` `Arc<Mutex<…>>` guard is never held across an `.await`. Load-bearing, not stylistic: the hydrator read path and the concurrent submit/stop edge update contend on the same mutex, so a guard crossing `.await` is a deadlock/latency hazard (`.claude/rules/development.md` § "Concurrency & async" → "Never hold a lock across `.await`"). Discipline: acquire → clone → drop before any `.await`.
- **Sim support (mechanism corrected 2026-06-03 — see § Changed Assumptions):** `scan_prefix` IS a public method on the `IntentStore` trait (`overdrive-core/src/traits/intent_store.rs:255`: `async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError>`) — but that is **irrelevant** to invariant A, because the seam the hydrate path reads is `AppState.store`, a **concrete `Arc<LocalIntentStore>`**, not `Arc<dyn IntentStore>`. A counting decorator over `&dyn IntentStore` cannot be injected at that field without widening it to a trait object (out of scope). The decorator seam does not exist. What shipped instead is a **delete-intent-then-tick behavioral proof**: remove the intent record after the keyed fact + `service_backends` row are in place, then assert hydrate yields a correct non-empty `desired` across N ticks — reachable only if hydrate reads memory, not the intent store (reverting the read-switch makes it RED). Equivalent-or-stronger than a scan counter; proptest over S × L × N.

---

## Wave: DESIGN / [REF] Component Decomposition

```
ListenerFactStore                      (NEW — overdrive-control-plane, src/listener_facts.rs)
├── primary:   BTreeMap<ServiceId, ListenerRow>     (read-path key)
├── secondary: BTreeMap<WorkloadId, Vec<ServiceId>> (cleanup index for stop)
├── rebuild_from_intent(state) -> Self              (boot; relocated gather_* body; both maps)
├── upsert(workload_id, vip, &listeners)            (submit edge; one ServiceId entry per listener)
├── remove_workload(&workload_id)                   (stop / conflict-release edge; evicts via secondary)
└── fact_for(&service_id) -> Option<ListenerRow>    (O(1) hydrate read, per backend row)

AppState                               (EXTEND — add `listener_facts: Arc<Mutex<ListenerFactStore>>`)
handlers::submit_workload              (EXTEND — upsert on Inserted; no-op on conflict)
handlers::stop_workload                (EXTEND — remove alongside ReleaseServiceVip path)
reconciler_runtime::ServiceMapHydrator hydrate arm (EXTEND — scoped read replaces scan)
reconciler_runtime::gather_service_listener_facts  (RELOCATE+RENAME → boot rebuild; delete per-tick caller)
startup wiring (run_server_*)          (EXTEND — construct ListenerFactStore::rebuild_from_intent next to allocator bulk_load)
```

**Driving port (inbound):** the REST submit/stop handlers (existing `IntentStore`-write edge) — they now also drive the in-memory projection.
**Driven port (outbound):** none new. `ListenerFactStore` reads the existing `IntentStore` + allocator at boot only; steady state is pure in-memory.

No external integrations. No contract-test annotation needed (no third-party API surface in this delta).

---

## Wave: DESIGN / [WHY] Reuse Analysis (HARD GATE)

| Existing component | Overlap | Verdict | Evidence |
|---|---|---|---|
| `PersistentServiceVipAllocator` (`overdrive-dataplane/src/allocators/persistent_service_vip.rs`) | Same lifecycle shape (boot-rebuild-from-scan + edge-maintained in-memory memo); already locked on this edge; holds the VIP input | **REUSE as pattern, do NOT extend** | `bulk_load` (scan → restore) and `allocate`/`release` (edge mutation) are the exact lifecycle `ListenerFactStore` copies. Extending it (option iii) rejected on SRP/cohesion (sub-decision 1). The new store *imitates* its lifecycle, *reads* its VIP at boot, but is a separate type. |
| `ViewStore` / `RedbViewStore` / `SimViewStore` + ADR-0035 View machinery (`overdrive-control-plane/src/view_store/`) | Informer-cache shape (bulk-load + write-through + in-memory `BTreeMap`) | **DO NOT REUSE** | Option (i) rejected — no reconciler owns a cluster-wide listener-fact View; persisting a derived projection violates "persist inputs, not derived state." The View machinery is for per-reconciler typed memory, which this is not. |
| `gather_service_listener_facts` (`reconciler_runtime.rs:1733`) | Is the exact projection logic the new store needs at boot | **EXTEND (relocate+rename)** | Body reused verbatim as `ListenerFactStore::rebuild_from_intent`; per-tick caller deleted (sub-decision 3). |
| `hydrate_bridge_desired_listeners` (`reconciler_runtime.rs:1646`) | Already does the **efficient single-key** `state.store.get(key)` + allocator-memo join for the *same* `(vip, port, protocol)` projection, per workload — **and derives the `ServiceId` via `ServiceId::derive(&assigned_vip, listener.port, "service-map")` (line 1705)**, the exact derivation the new store's key uses | **REUSE as precedent** | Proof the O(1) read shape and the `ServiceId` derivation work and are already in-tree. The new hydrate read mirrors its lock discipline (acquire guard → clone → drop before `.await`). The bridge reads per-workload from the store directly; the hydrator reads per-`ServiceId` from the in-memory `ListenerFactStore`. |
| Action-shim VIP path (`action_shim/release_service_vip.rs`, `mod.rs`) | The convergence-tick I/O boundary | **DO NOT REUSE** | The fact update belongs on the *intent-write* edge (handler), not the *convergence-tick* edge (shim) — sub-decision 2. |
| `AppState` (`lib.rs:203,285`) | Holds `allocator: Arc<Mutex<…>>` | **EXTEND** | Add `listener_facts: Arc<Mutex<ListenerFactStore>>` beside it; constructed at the same wiring site. |

**Verdict: 1 CREATE NEW (`ListenerFactStore`), 4 EXTEND, 2 DO-NOT-REUSE-with-rationale.** The single CREATE NEW is justified: no existing structure can host cluster-wide-keyed listener facts without either abusing the View contract (i) or violating the allocator's SRP (iii). The new type is a thin `BTreeMap` wrapper that *imitates* the allocator's proven boot-rebuild + edge-maintain lifecycle.

---

## Wave: DESIGN / [WHY] Quality-Attribute Impact (ISO 25010)

| Attribute | Before | After |
|---|---|---|
| Performance efficiency (time behaviour) | O(S²) decodes + O(S²) lock acquisitions per active tick; full redb scan per target per 100 ms | O(1) in-memory keyed read per hydrate; zero redb reads steady-state; O(S) one-time boot rebuild |
| Maintainability (analyzability) | Projection logic split between per-tick gather and per-workload bridge | Single boot-rebuild projection + symmetric edge maintenance, mirroring the allocator |
| Reliability (recoverability) | (scan re-derives every tick — trivially "self-heals" but at cost) | Cold-boot rebuild re-projects from intent SSOT; crash between intent-commit and fact-insert recovered by next boot rebuild |
| Testability | Hard to assert "no scan" — scan is the design | Counting-store DST invariant makes "zero steady-state scan" a loud regression gate |

Trade-off (ATAM sensitivity point): the edge-maintained store can drift from the boot-rebuild if a write path forgets to upsert/remove. Mitigated by DELIVER invariant B (byte-equivalence to re-scan) — the same defense the allocator uses between `allocate` and `bulk_load`.

---

## Wave: DESIGN / [HOW] ADR + Cross-References

**ADR written: `adr-0062-listener-fact-in-memory-view.md`** (next after 0061).
Amends/extends:
- **ADR-0035** (reconciler-memory-collapse-to-typed-view-redb) — extends its "zero steady-state durable reads" contract to listener-fact hydration *without* adding a persisted View (rationale: no durable state to persist; intent store is SSOT).
- **ADR-0042** (service-map-hydrator-reconciler) — amends the desired-hydration source for the `ServiceMapHydrator` arm.
- **ADR-0049** (platform-issued-service-vip-allocator) — references the allocator's boot-rebuild + edge-maintain lifecycle as the imitated pattern; clarifies the allocator is NOT extended to carry listeners.
- **ADR-0060** (service-frontend-update-service-signature) — the C3 unresolvable-proto guard is preserved verbatim through the read-path change.

---

## Wave: DESIGN / [REF] Open Questions / Deferrals

None requiring a GitHub issue at design time. The DELIVER invariant-A test is in-scope for the fix, not a deferral. (The counting-`scan_prefix`-decorator mechanism originally claimed here as a "verified fact" was found infeasible during DELIVER — the read seam `AppState.store` is concrete, not `dyn`; corrected 2026-06-03 in § Changed Assumptions and sub-decision 5. The shipped mechanism is the delete-intent-then-tick behavioral proof.) No operator-tunable knobs introduced. No forward pointers.

One item to surface to the orchestrator (NOT a deferral, a confirmation for DELIVER): research Gap 2 asked whether the intent store exposes a write hook / monotonic generation. Answer found in-tree: **it does not need one** — the handler IS the write hook (sub-decision 2), and the allocator already proves edge-maintenance works without a store-level generation primitive. No new intent-store surface is required.

---

## Changed Assumptions

### 2026-06-03 — Invariant A test mechanism: counting `scan_prefix` decorator → delete-intent-then-tick behavioral proof

Back-propagated from DELIVER per the nWave Document Update / back-propagation contract and CLAUDE.md "no aspirational docs." DELIVER revealed the originally-specified mechanism is structurally infeasible. The **property** (zero steady-state intent-store dependence) is unchanged; only the **mechanism** is corrected.

**Original claim (verbatim), sub-decision 5 "Sim support":**

> **Sim support:** **verified fact** — `scan_prefix` is a public method on the `IntentStore` trait (`overdrive-core/src/traits/intent_store.rs:255`: `async fn scan_prefix(&self, prefix: &[u8]) -> Result<Vec<(Bytes, Bytes)>, IntentStoreError>`), so a thin counting decorator wraps `&dyn IntentStore` directly. No new sim trait surface, no host-adapter detail to expose; invariant A is testable as designed.

**Original claim (verbatim), invariant-A bullet, sub-decision 5:**

> **DELIVER regression invariant A (zero-scan):** *"`ServiceMapHydrator::hydrate_desired` issues zero `scan_prefix` calls in steady state."* Assertable via a counting `IntentStore` decorator (sim) that records `scan_prefix` invocations; the test submits S services, runs N convergence ticks, asserts the scan counter is 0 after boot.

**Corrected claim + rationale:** `scan_prefix` IS a public trait method, but that is irrelevant to the decorator's feasibility. A counting decorator must be injected at the seam the hydrate path actually reads — `AppState.store` — and that field is a **concrete `Arc<LocalIntentStore>`**, not `Arc<dyn IntentStore>`. A decorator over `&dyn IntentStore` has nowhere to attach without widening `AppState.store` to a trait object, which is out of scope for this fix. The concrete-vs-`dyn` field type, not the trait method's visibility, is what determines whether the decorator seam exists — and it does not.

The shipped mechanism (accepted in DELIVER adversarial review) is a **delete-intent-then-tick behavioral proof** in `crates/overdrive-control-plane/tests/acceptance/listener_fact_zero_scan_invariant.rs`: after the keyed fact + `service_backends` row are in place, the test removes the intent record, then runs N hydrate ticks asserting a correct non-empty `desired` every tick. The OLD scan path would resolve nothing (empty desired) once the record is gone; the NEW keyed read resolves from the in-memory `ListenerFactStore`. A correct non-empty `desired` across N ticks is reachable ONLY if hydrate reads memory, not the intent store; reverting the read-switch makes it RED. This is **equivalent-or-stronger** than a scan counter — it proves the intent record is entirely unnecessary in steady state, not merely that no scan call fired. proptest over S × L × N.

No change to the decision, store shape, keying, or invariants B / C / BE-* descriptions.
