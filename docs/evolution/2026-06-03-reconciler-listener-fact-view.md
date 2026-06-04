# reconciler-listener-fact-view — Feature Evolution

**Feature ID**: `reconciler-listener-fact-view`
**Branch**: `marcus-sa/udp-support`
**Duration**: 2026-06-03 (ADR-0062 + research landed → DELIVER + finalize close,
single day)
**Status**: Delivered — 4/4 DELIVER roadmap steps complete (`01-01..01-04`).
DES integrity verified: all 4 steps with all 5 TDD phases (PREPARE,
RED_ACCEPTANCE, RED_UNIT, GREEN, COMMIT) logged EXECUTED PASS;
`des-verify-integrity` returned exit 0 ("All 4 steps have complete DES
traces"). 436 control-plane tests green on Lima; invariants A/B/C green;
adversarial review NEEDS_REVISION → resolved; per-feature mutation gate 100%
kill (3 caught, 1 unviable, 0 missed).
**ADR**: [ADR-0062](../product/architecture/adr-0062-listener-fact-in-memory-view.md)
(Accepted 2026-06-03; extends ADR-0035, amends ADR-0042, references ADR-0049,
preserves ADR-0060 C3).
**Research**: [reconciler-desired-hydration-efficiency](../research/control-plane/reconciler-desired-hydration-efficiency.md)
(18 sources, High confidence — Kubernetes informer/indexer precedent).

---

## What shipped

A **hydration-layer efficiency fix** on the existing `ServiceMapHydrator`
reconciler. The hydrator's desired-state hydration previously called
`gather_service_listener_facts` (`reconciler_runtime.rs:1733-1796`) on every
target on every ~100 ms convergence tick — a full `scan_prefix(b"workloads/")`
+ rkyv-decode of every Service intent + an `allocator.lock().await` per decoded
intent. With one target per service (S services), that is **O(S²) decodes +
O(S²) lock acquisitions per active tick**, realized at boot, backend-churn
waves, and multi-service rollouts. The derived value
`ListenerRow { vip: Some(_), port, protocol }` is *stable between operator spec
submissions* — it changes only on `submit_workload` / `stop_workload`.

The fix replaces the per-tick scan with an in-memory, boot-rebuilt +
edge-maintained `ListenerFactStore`:

- **Primary read index** `BTreeMap<ServiceId, ListenerRow>` — keyed by the
  *same `ServiceId` the hydrator reads by*, so the steady-state read is a
  genuine O(1) `store.get(&row.service_id)`, eliminating (not relocating) the
  prior per-row `vip == row.vip` listener scan inside `project_service_desired`.
- **Secondary cleanup index** `BTreeMap<WorkloadId, Vec<ServiceId>>` — used
  only by the stop / conflict-release path, which holds a `WorkloadId` (not the
  per-listener `ServiceId`s) and would otherwise need an intent decode +
  allocator lock to find what to evict.

Steady-state `ServiceMapHydrator` hydrate now pays **zero redb reads** and
**zero per-row listener scan** — restoring ADR-0035's "zero steady-state
durable reads" contract *without* adding a persisted View (there is no durable
state to persist; the intent store is already the SSOT, so the boot rebuild
re-derives the whole view for free — honoring "persist inputs, not derived
state"). The ADR-0060 C3 unresolvable-proto guard (skip the service, never
silently default to `Proto::Tcp`) is preserved verbatim through the read-path
change.

### Production code (`crates/overdrive-control-plane`)

- **`src/listener_facts.rs` (NEW)** — `ListenerFactStore` with two `BTreeMap`s
  (deterministic iteration per `.claude/rules/development.md` §
  "Ordered-collection choice"; no `HashMap`). `upsert(workload_id, &vip,
  &listeners)` derives one `ServiceId::derive(&vip, listener.port,
  "service-map")` per listener and populates both maps; `remove_workload(&id)`
  evicts via the secondary index; `fact_for(&service_id)` is the O(1) read;
  `rebuild_from_intent(&store, &intent_redb_path, &allocator)` is the relocated
  `gather_*` projection body (same scan, same `ServiceId::derive`, same
  allocator-lock-drop-before-`.await` discipline), populating both maps from the
  intent SSOT + allocator memo at boot.
- **`src/lib.rs`** — `AppState` gains a mandatory
  `listener_facts: Arc<tokio::sync::Mutex<ListenerFactStore>>` field beside
  `allocator`, boot-rebuilt *after* the allocator's `bulk_load` (ordering is
  load-bearing — the rebuild joins each Service's allocator-issued VIP). Mandatory
  constructor param (no default, no builder) per § "Port-trait dependencies" — a
  forgotten boot rebuild fails to compile. Single-cut migration of 31 AppState
  call sites / fixtures (control-plane + overdrive-sim invariants).
- **`src/handlers.rs`** — edge maintenance on the single atomic intent-change
  write edge: `upsert` in the `submit_workload` `PutOutcome::Inserted` branch
  (after the intent commits, intent SSOT first); a deliberate store **no-op** on
  the conflict-release branch (symmetric with the allocated-then-released VIP);
  `remove_workload` in `stop_workload` (via the secondary index — no intent
  decode, no allocator lock). Lock discipline: acquire → mutate → drop before any
  `.await`.
- **`src/reconciler_runtime.rs`** — the `ServiceMapHydrator` hydrate arm flipped
  to the per-row keyed read; the keyed `Option<ListenerRow>` threaded through the
  existing `project_service_desired` seam as a single-element slice (preserving
  the C3 path verbatim); `gather_service_listener_facts` **deleted in full**
  (single-cut, same commit — its projection body relocated to
  `rebuild_from_intent` in 01-01). `project_service_desired` (overdrive-core) left
  intact: it has other callers (`service_frontend_provenance`).

### Test coverage (13 scenarios, all default-lane / Tier 1 DST — no real
kernel/network)

- **Invariant A** (`listener_fact_zero_scan_invariant.rs`) — zero steady-state
  intent-store dependence, via a **delete-intent-then-tick behavioral proof**
  (proptest over S × L × N): establish the keyed fact + `service_backends` row,
  remove the intent record, then assert N hydrate ticks still project a correct
  non-empty `desired`. Reachable only if hydrate reads memory; reverting the
  read-switch makes it RED.
- **Invariant B + B-ex** (`listener_fact_byte_equivalence_invariant.rs`) —
  multi-listener byte-equivalence: the edge-maintained store equals a fresh
  `rebuild_from_intent` over the same intent set, both maps entry-for-entry
  (incl. secondary inner-`Vec` order), with ≥1 service carrying ≥2 listeners.
- **Invariant C** (`listener_fact_lock_discipline_invariant.rs`) — the store
  guard is never held across `.await`: concurrent hydrate-read + upsert + remove
  under multi-task contention complete within budget; a guard-across-`.await`
  stalls and the timeout elapses.
- **BE-1/BE-2/BE-3** (`listener_fact_hydrate_equivalence.rs`) — with-fact
  projection matches the pre-change result; no-fact row skips + emits
  `service_map_hydrator.desired.unresolvable_proto` with no defaulted-Tcp (C3);
  distinct VIPs derive distinct `ServiceId`s, no collision.
- **U1–U6** (`src/listener_facts.rs` `#[cfg(test)] mod tests`) — the store's own
  API contract (upsert / remove / fact_for / rebuild skip semantics / BTreeMap
  determinism / conflict no-op).

### Mutation testing

Per-feature gate met: **100% kill** (3 caught, 1 unviable, 0 missed). The single
surviving equivalent mutant — `||` → `&&` at the `workloads/` sub-key prefix
guard — is genuinely unkillable (the decode + `Service(_)` match below rejects
sub-keys identically, so facts are byte-identical), and is excluded via
`.cargo/mutants.toml` `exclude_re`. The exclusion entry was *relocated* from the
deleted `gather_service_listener_facts` to its new home
`ListenerFactStore::rebuild_from_intent` (see Lessons 2 below).

---

## Provenance arc

This feature did not originate from a DISCUSS wave. It escalated through the
nWave pipeline:

1. **/nw-bugfix** — surfaced the O(S²)-per-tick scan in `ServiceMapHydrator`
   hydration as a performance defect (RCA pinned the call site at
   `reconciler_runtime.rs:1733-1796`, run once per target per ~100 ms tick).
2. **/nw-research** — `reconciler-desired-hydration-efficiency.md` (18 sources):
   mapped the defect to the Kubernetes informer/indexer anti-pattern ("re-list
   the durable store inside the reconcile path"), surveyed field-indexer scoped
   lookup and writer-bumped monotonic generation as the canonical invalidation
   key, and ranked candidate (d) — fold the projection into an in-process cache
   kept current by write events — first.
3. **/nw-design** — three review rounds. **Review #2 caught a
   `WorkloadId`-vs-`ServiceId` read-key mismatch** before it reached
   implementation: the hydrator read path never holds a `WorkloadId` (it resolves
   `service_id` from the target and keys its desired map by `row.service_id`), so
   a `WorkloadId`-keyed store would have forced a reverse lookup or scan on every
   read — falsifying the "O(1) keyed read" claim. The store was rekeyed to
   `ServiceId` primary + a `WorkloadId → Vec<ServiceId>` secondary cleanup index.
4. **/nw-deliver** — 4 steps, bottom-up green bar at each COMMIT.

---

## Key decisions (feature-delta sub-decisions 1–5)

1. **In-memory store (option ii), not a persisted View (i) nor an allocator
   extension (iii).** A persisted `ViewStore` View *would* technically fit the
   ADR-0035 contract, but is rejected because (a) the facts are a pure derivation
   of already-persisted inputs — persisting buys zero durability and violates
   "persist inputs, not derived state"; (b) a View needs an owning reconciler with
   a `reconcile()`, so hosting edge-maintained facts would need a synthetic
   reconciler-with-no-`reconcile`. Extending `PersistentServiceVipAllocator` is
   rejected on cohesion (its SRP is VIP issuance + range management; folding in
   listeners forces a rkyv version bump and the wrong keying — `spec_digest`, not
   `ServiceId`). The store *imitates* the allocator's proven boot-rebuild +
   edge-maintain lifecycle as a separate, cohesive type.
2. **`ServiceId`-keyed primary + `WorkloadId → Vec<ServiceId>` secondary cleanup
   index.** The read path holds a `ServiceId`; the stop path holds a `WorkloadId`.
   The secondary index lets the stop edge evict without an intent decode or
   allocator lock.
3. **Edge maintenance co-located with the existing VIP-memo mutation** in
   `submit_workload` / `stop_workload` — the writer-bumped-invalidation discipline
   the research names. Insert only on `Inserted` (after intent commit); no-op on
   conflict-release (symmetric with the VIP).
4. **O(1) keyed read** `store.get(&row.service_id)` replaces the cluster-wide
   scan + per-row VIP filter; C3 guard preserved.
5. **Single-cut deletion** of `gather_service_listener_facts` in the read-switch
   step (01-04), its projection body having been relocated to `rebuild_from_intent`
   in 01-01 — no salvage-rename of the per-tick-scan tests; the boot-rebuild
   behavior is re-asserted fresh as U4. `BTreeMap` for both maps; invariants A/B/C
   graduated as DELIVER gates.

---

## Steps completed (4)

| Step  | Commit     | Outcome |
|-------|------------|---------|
| 01-01 | `043fff53` | `ListenerFactStore` type + boot-rebuild projection (relocated `gather_*` body) + unit contract U1–U6 + invariant B. Original `gather_*` left intact (per-tick caller still drives hydration). |
| 01-02 | `a157d5fd` | `AppState.listener_facts` field + boot rebuild at startup wiring (after allocator `bulk_load`); 31 AppState call-site/fixture single-cut migration; mandatory constructor param. |
| 01-03 | `2c293923` | Edge maintenance in `handlers.rs`: `upsert` on submit `Inserted`, no-op on conflict-release, `remove_workload` on stop. Edge-half of invariant B asserted against the boot rebuild. |
| 01-04 | `e0f98181` (+ `d8a38256`, `b27181cb`, `2231c59e`) | Hydrator read-path flip to O(1) keyed read; `gather_service_listener_facts` deleted in full; invariants A + C + behavior-equivalence pins BE-1/BE-2/BE-3; review-resolution refactor (`d8a38256`); mutants:skip relocation (`b27181cb`, `2231c59e`). |

The roadmap's 4-step plan held shape. 01-04's DES log shows four
GREEN→COMMIT cycles — the keystone read-switch step absorbed the
review-resolution refactor and the two mutation-annotation corrections as
follow-on green-bar commits under the same step ID.

---

## Lessons learned / issues encountered

These are the lasting value of the feature — each is a recurrence-prone trap
the next contributor would otherwise re-hit.

1. **A public trait method is not a decorator seam when the holding field is
   concrete.** Invariant A was originally specified (in DESIGN) as a counting
   `IntentStore` decorator over `&dyn IntentStore` asserting zero steady-state
   `scan_prefix` calls — justified as feasible because `scan_prefix` *is* a
   public method on the `IntentStore` trait. DELIVER found this structurally
   infeasible: the seam the hydrate path actually reads is `AppState.store`, a
   **concrete `Arc<LocalIntentStore>`**, not `Arc<dyn IntentStore>`. A `&dyn`
   decorator has nowhere to attach without widening the field to a trait object
   (out of scope). Trait-method *visibility* cannot substitute for a
   concrete-typed *field*. The shipped mechanism is a stronger behavioral proof:
   delete the intent record, then assert N hydrate ticks still project correct
   desired — proving the intent record is *entirely unnecessary* in steady state,
   not merely that no scan call fired. Back-propagated to ADR-0062 § Testability
   and feature-delta § Changed Assumptions per the no-aspirational-docs contract.
   (The brief.md §88 DST/determinism note + its 2026-06-03 changelog row still
   describe the original decorator mechanism; the §88 status marker flags the
   correction rather than rewriting the locked DESIGN prose.)
2. **`cargo-mutants` does not honor inline `// mutants: skip` for mid-body
   expression mutations in this repo — the equivalent-mutant suppression
   mechanism is `.cargo/mutants.toml` `exclude_re`.** The inline comment is
   documentation only; cargo-mutants only honors the marker on the
   immediately-adjacent line and not for the `||`→`&&` sub-key-guard mutation at
   issue. The genuine equivalent mutant (the `workloads/` prefix guard whose
   decode + `Service(_)` match rejects sub-keys identically) had its
   `exclude_re` entry *relocated* from the deleted `gather_service_listener_facts`
   to its new home `ListenerFactStore::rebuild_from_intent` when the projection
   moved in 01-01 — the annotation was dropped in the relocation and restored in
   `b27181cb` / `2231c59e`.
3. **A typed-error recursive cycle (E0072) is resolved with `Box`, not by
   flattening to `String`.** Wiring the boot-rebuild failure into
   `ControlPlaneError::ListenerFactRebuild` formed a `ControlPlaneError ↔
   ConvergenceError` type cycle (`ConvergenceError` already carries a
   `ViewPersist(ControlPlaneError)` arm). The fix carries
   `Box<ConvergenceError>` — preserving the typed, `matches!`-able error all the
   way down rather than collapsing it into `Internal(String)` (per
   `development.md` § "Never flatten a typed error to `Internal(String)`").

---

## Quality gates

- **DES integrity** — 4 steps × 5 TDD phases (PREPARE, RED_ACCEPTANCE, RED_UNIT,
  GREEN, COMMIT), all EXECUTED PASS; `des-verify-integrity` exit 0.
- **Test suite** — full control-plane crate: 436 passed, 2 skipped, on Lima;
  invariants A/B/C green (proptest + multi-thread contention run on Linux).
- **Adversarial review** — NEEDS_REVISION → resolved (`d8a38256`: typed boot
  error boxing; `#[cfg(test)] snapshot()` accessor so invariant-B equivalence
  asserts via the observable projection, not private field layout; no production
  surface widened).
- **Mutation** — per-feature gate 100% kill (3 caught, 1 unviable, 0 missed); one
  genuine equivalent mutant excluded with inline justification.
- **clippy `-D warnings`** clean.

---

## Links to permanent artifacts

- **ADR-0062** — Listener-fact in-memory view, maintained on the intent-change
  edge (Accepted 2026-06-03):
  `docs/product/architecture/adr-0062-listener-fact-in-memory-view.md`
- **Research** — reconciler desired-hydration efficiency (informer/indexer
  precedent, field-indexer, generation-as-invalidation-key):
  `docs/research/control-plane/reconciler-desired-hydration-efficiency.md`
- **brief.md §88** — Listener-fact in-memory view extension (DESIGN prose +
  IMPLEMENTED status marker): `docs/product/architecture/brief.md`
- **Commit chain (9 commits, `3bdb3618..99733646`)**:
  - `3bdb3618` — docs: ADR-0062 + research
  - `043fff53` — feat: `ListenerFactStore` + boot-rebuild projection (01-01)
  - `a157d5fd` — feat: wire `ListenerFactStore` onto `AppState` + boot rebuild (01-02)
  - `2c293923` — feat: maintain `ListenerFactStore` on the intent-change edge (01-03)
  - `e0f98181` — feat: switch `ServiceMapHydrator` to O(1) keyed read + delete scan (01-04)
  - `d8a38256` — refactor: box typed boot error + test-observability accessor (01-04 review-resolution)
  - `b27181cb` — test: restore `mutants:skip` on the equivalent sub-key guard (01-04)
  - `2231c59e` — test: place `mutants:skip` marker adjacent (01-04)
  - `99733646` — chore: DELIVER artifacts (roadmap, test-scenarios, execution-log, mutants.toml, back-propagation)

---

## Related upstream/downstream features

- **ADR-0035 / ADR-0036** — reconciler trait shape + runtime ViewStore ownership;
  this feature restores ADR-0035's "zero steady-state durable reads" contract for
  the `ServiceMapHydrator` hydrate path.
- **ADR-0042** — `ServiceMapHydrator` reconciler (desired-hydration source
  amended here).
- **ADR-0049** — `PersistentServiceVipAllocator` (the VIP this store reads at
  boot; its boot-rebuild + edge-maintain lifecycle is the imitated pattern, NOT
  extended).
- **ADR-0060** — service-frontend `update_service` signature (the C3
  unresolvable-proto guard preserved verbatim).
