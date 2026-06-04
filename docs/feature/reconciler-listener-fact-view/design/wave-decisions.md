# DESIGN Wave Decisions — `reconciler-listener-fact-view`

**Architect**: Morgan | **Mode**: propose | **Date**: 2026-06-03 | **ADR**: adr-0062
**Direction (locked upstream)**: candidate (d) — in-memory + edge-maintained listener facts; zero steady-state redb reads.

## Locked sub-decisions

| # | Decision | Choice | One-line rationale |
|---|---|---|---|
| 1 | Where the view lives | **(ii) in-memory `Arc<Mutex<ListenerFactStore>>`, boot-rebuilt, edge-maintained; primary `BTreeMap<ServiceId, ListenerRow>` + secondary `BTreeMap<WorkloadId, Vec<ServiceId>>` cleanup index** | Mirrors `PersistentServiceVipAllocator` lifecycle minus persistence; honors "persist inputs, not derived state" (intent SSOT re-projected at boot). Keyed by `ServiceId` (the read-path key); secondary index serves the stop path (holds only `WorkloadId`). (i) abuses the View contract; (iii) breaks the allocator's SRP. |
| 2 | Who owns the write edge | **`submit_workload`/`stop_workload` handler, co-located with the VIP-memo update** | The handler already holds both inputs (`s.listeners` + the allocator VIP) on the intent-write edge — single atomic edge. Shim/reconciler run on the tick, re-coupling to the timer. |
| 3 | Fate of `gather_service_listener_facts` | **Relocate + rename → `ListenerFactStore::rebuild_from_intent` (boot path); delete the per-tick caller** | Its body IS the cold-boot rebuild; the per-tick call site at `reconciler_runtime.rs:1335` is deleted with its scan-behavior tests. |
| 4 | Read-path shape | **Per-row O(1) `store.get(&row.service_id)`, guard→clone→drop** | Store keyed by `ServiceId` (the read path holds no `WorkloadId`); `store.get(&row.service_id)` directly yields `(port, protocol)`, eliminating the per-row `vip == row.vip` scan; mirrors `hydrate_bridge_desired_listeners` lock discipline + its `ServiceId::derive(&vip, port, "service-map")` call; C3 guard preserved. |
| 5 | DST / determinism | **`BTreeMap` keying (both maps); DELIVER invariants A (zero steady-state `scan_prefix`) + B (byte-equiv to re-scan, multi-listener) + C (guard never across `.await`); counting `IntentStore` decorator (sim, `scan_prefix` is trait-public)** | Makes "zero steady-state scan" a loud regression gate; B defends edge/rebuild drift over all `ServiceId` entries; C defends the read/edge mutex contention. |

## Reuse verdict

1 CREATE NEW (`ListenerFactStore`), 4 EXTEND (`gather_*`→boot rebuild, `AppState`, `submit_workload`/`stop_workload`, `ServiceMapHydrator` arm), 2 DO-NOT-REUSE-with-rationale (ViewStore Views; action-shim). The CREATE NEW is justified: no existing structure can host cluster-wide-keyed listener facts without abusing the View contract or the allocator's SRP.

## Cross-references

- Extends **ADR-0035** (zero steady-state durable reads — achieved without a persisted View).
- Amends **ADR-0042** (ServiceMapHydrator desired-hydration source).
- References **ADR-0049** (allocator boot-rebuild + edge-maintain pattern imitated; allocator NOT extended).
- Preserves **ADR-0060** C3 (unresolvable-proto → skip, never default Tcp).
- Research: `docs/research/control-plane/reconciler-desired-hydration-efficiency.md`.
- RCA anchors: `reconciler_runtime.rs:1322-1364,1646-1715,1733-1796`; `handlers.rs:323-331,424-432`; `lib.rs:203,285`.

## Deferrals

None requiring a GitHub issue. Research Gap 2 (intent-store write hook / generation) resolved in-tree: the handler IS the write hook; no new intent-store surface required.

## Peer review

2026-06-03: Peer review (APPROVE-WITH-CHANGES) applied — blocking ADR-0062 rationale rewrite (§ "Why in-memory, not a persisted ViewStore View": persistence-unnecessary-before-synthetic-reconciler-burden, acknowledging option (i) technically fits the View contract) + 2 should-fixes (brief.md §88 integration as a numbered Application Architecture subsection; ADR-0062 crash-consistency Negative/risks bullet) + 1 nit (feature-delta.md Bar-1 → hydration-layer reframe). No decisions or scope changed. — Morgan.

2026-06-03 (second review, REJECTED_PENDING_REVISIONS → resolved): **CRITICAL read-key mismatch fixed** — the store is **rekeyed from `WorkloadId` to `ServiceId`** (`BTreeMap<ServiceId, ListenerRow>` primary). Verified the `ServiceMapHydrator` read path holds a `ServiceId` (`reconciler_runtime.rs:1323`, keys desired by `row.service_id` at 1347), never a `WorkloadId`, so the prior `WorkloadId` keying forced a reverse lookup; the rekey makes the read `store.get(&row.service_id)` genuinely O(1) and **eliminates** the per-row `vip == row.vip` scan in `project_service_desired`. The edge derives the key via `ServiceId::derive(&vip, listener.port, "service-map")` (verified `id.rs:825`, used at `reconciler_runtime.rs:1705`), taking the submit handler's `service_vip: Option<ServiceVip>`. **Stop-path counterpart**: added secondary `BTreeMap<WorkloadId, Vec<ServiceId>>` cleanup index (approach b) — `stop_workload` (`handlers.rs:642-681`) holds only the `WorkloadId` and does not decode intent / hold the VIP, so it evicts via the index with no allocator lock; boot `rebuild_from_intent` reconstructs both maps. Plus 3 clarifications: (HIGH) invariant B asserts byte-equivalence over the full set of `ServiceId` entries incl. multi-listener (one entry per listener); (MEDIUM) lock discipline promoted to **DST invariant C** (guard never across `.await`; read/edge mutex contention); (MEDIUM) **verified `scan_prefix` is a public `IntentStore` trait method** (`overdrive-core/src/traits/intent_store.rs:255`) — invariant A's counting decorator on `&dyn IntentStore` is a stated fact, not a DELIVER contingency; (LOW) feature-delta primary framing reframed to hydration-layer-efficiency (candidate (d) kept as label). The high-level decision (in-memory, boot-rebuilt, edge-maintained `ListenerFactStore`; candidate (d)/option ii) is unchanged; the four prior fixes were not touched. Propagated through ADR-0062, brief.md §88, feature-delta.md. — Morgan.
