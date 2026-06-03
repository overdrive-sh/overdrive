# DISTILL — Test Scenarios — `reconciler-listener-fact-view`

**Wave**: DISTILL (escalation chain: bugfix → research → design → distill; no DISCUSS / DEVOPS).
**Acceptance designer**: Quinn. **Paradigm**: OOP (Rust). **Density**: lean.
**Source of truth**: `feature-delta.md` (locked sub-decisions 1–5 + Component Decomposition),
`adr-0062-listener-fact-in-memory-view.md` (§ Testability invariants A/B/C),
`design/wave-decisions.md`.

> **Specification-only.** Per `.claude/rules/testing.md` § "Testing" — **there are
> NO `.feature` files in this project**, and none are produced here. The
> GIVEN/WHEN/THEN blocks below are prose specification only; they are never
> parsed or executed. The DELIVER crafter translates each into a Rust
> `#[test]` / `#[tokio::test]` (default-lane unit or Tier-1 DST) at the named
> file path. The generic DISTILL skill's `pytest-bdd` / `steps_*.py` /
> `nwave_ai.state_delta` machinery is explicitly NOT used (confirmed in
> `docs/architecture/atdd-infrastructure-policy.md` polyglot note).

> **No `__SCAFFOLD__` markers, no RED-scaffold stubs created by DISTILL here.**
> The Rust RED-scaffold convention is `#[should_panic(expected = "RED scaffold")]`
> on the test body (`.claude/rules/testing.md` § "RED scaffolds"), authored in
> DELIVER's RED phase against the to-be-created `src/listener_facts.rs`. DISTILL
> produces the spec; DELIVER authors the scaffolds. (Mandate 7's
> assertion-error stub shape is the Python contract — superseded by the
> project's `#[should_panic]` convention.)

---

## Wave-Decision Reconciliation — HARD GATE

| Source read | Result |
|---|---|
| `discuss/wave-decisions.md` | **absent** (no DISCUSS wave — escalation path). WARN, proceed; ACs derived from DESIGN. |
| `design/wave-decisions.md` | present — locked sub-decisions 1–5, reuse verdict, peer-review history (incl. the `WorkloadId`→`ServiceId` rekey). |
| `devops/wave-decisions.md` | **absent** (no DEVOPS wave). WARN, proceed; tiers from `.claude/rules/testing.md`. |
| `docs/architecture/atdd-infrastructure-policy.md` | present (`--policy=inherit`) — Rust four-tier mapping already records `ServiceMapHydrator.reconcile` as an in-process driving port and `SimDataplane`/`SimObservationStore` as the Tier-1 driven-internal adapters. No new port in this delta (the design adds no external/driven port — `ListenerFactStore` reads `IntentStore` + allocator at boot only). No policy row to append. |

**Reconciliation result: PASSED — 0 contradictions.** DESIGN sub-decisions are
internally consistent and consistent with the existing infra policy. No
DISCUSS/DEVOPS exist to contradict. Proceeding to scenario design.

---

## Wave: DISTILL / [REF] Driving ports & test tiers

This feature is a **hydration-layer efficiency fix on an existing reconciler**.
There is no operator-facing CLI/endpoint *new surface* — the observable
behaviour is internal: the `ServiceMapHydrator` hydrate path, the
`ListenerFactStore` type, and the `submit_workload`/`stop_workload` edge.
Accordingly the scenarios live at **Tier 1 (DST) / default-lane unit**, per the
project's tier-selection table — the invariants depend on in-process logic, the
`Sim*` traits, and seed-deterministic iteration; **no real kernel / network /
subprocess is required**.

**Driving ports exercised** (all in-process — from the infra policy):
- `ListenerFactStore::{rebuild_from_intent, upsert, remove_workload, fact_for}` — the new in-process API (the unit-level driving surface).
- `reconciler_runtime::hydrate_desired_for_test(...)` (`reconciler_runtime.rs:1599`) — the existing in-process driving port for the `ServiceMapHydrator` hydrate arm; invariants A and the behavior-equivalence pins drive through it.
- `handlers::submit_workload` / `handlers::stop_workload` — the intent-write edge that maintains the store (exercised via `AppState` in-process, the same shape `service_backends_hydrate_desired.rs` and `service_vip_submit_acceptance.rs` use).

**No `@walking_skeleton` scenario is added in this delta.** The walking
skeleton (`overdrive deploy <spec.toml>` UDP reverse wire path) is owned by the
`udp-service-support` feature and recorded in the infra policy; this efficiency
fix rides under it and does not introduce a new end-to-end user journey. The
litmus test ("can a non-technical stakeholder confirm this is what users need?")
is not met by an internal hydration cache — the user-visible behaviour
(service-map programming) is **unchanged**; only its cost is. Adding a synthetic
WS here would be Testing Theater. (Behaviour-equivalence pin BE-1 is the
guard that user-visible behaviour is preserved.)

---

## Wave: DISTILL / [REF] Scenario list (tags + tier + Rust fn + file)

Tags: `@property` (PBT / proptest), `@example` (single-example), `@error`
(C3 / negative / edge), `@dst-invariant` (graduated A/B/C), `@regression-pin`
(behaviour-equivalence to pre-change), `@in-memory` (pure in-process, Tier 1 /
default-lane).

| # | Scenario | Tier | Tags | Rust fn | File |
|---|---|---|---|---|---|
| **A** | Zero steady-state `scan_prefix` after boot | Tier 1 DST | `@dst-invariant @in-memory @property` | `hydrator_issues_zero_scan_prefix_in_steady_state` | `tests/acceptance/listener_fact_zero_scan_invariant.rs` |
| **B** | Byte-equivalence: edge-maintained == boot-rebuild (multi-listener) | Tier 1 DST | `@dst-invariant @in-memory @property` | `edge_maintained_store_byte_equivalent_to_rebuild` | `tests/acceptance/listener_fact_byte_equivalence_invariant.rs` |
| **B-ex** | Byte-equivalence — fixed multi-listener example (fallback) | Tier 1 DST | `@dst-invariant @in-memory @example` | `edge_maintained_store_byte_equivalent_to_rebuild_three_listener_example` | `tests/acceptance/listener_fact_byte_equivalence_invariant.rs` |
| **C** | Guard never held across `.await` (read/edge contention) | Tier 1 DST | `@dst-invariant @in-memory` | `listener_fact_guard_never_held_across_await_under_contention` | `tests/acceptance/listener_fact_lock_discipline_invariant.rs` |
| U1 | `upsert` multi-listener → one primary entry per listener + matching secondary `Vec` | unit | `@in-memory @property` | `upsert_multi_listener_creates_one_primary_entry_per_listener` | `src/listener_facts.rs` `#[cfg(test)] mod tests` |
| U2 | `remove_workload` evicts exactly that workload's `ServiceId`s; others untouched | unit | `@in-memory` | `remove_workload_evicts_only_target_service_ids` | `src/listener_facts.rs` `#[cfg(test)] mod tests` |
| U3 | `fact_for` returns the row for a present service, `None` for absent | unit | `@in-memory` | `fact_for_returns_row_for_present_and_none_for_absent` | `src/listener_facts.rs` `#[cfg(test)] mod tests` |
| U4 | `rebuild_from_intent` over a fixed intent set → expected two maps (skip semantics) | unit | `@in-memory @example` | `rebuild_from_intent_projects_service_intents_only` | `src/listener_facts.rs` `#[cfg(test)] mod tests` |
| U5 | Both maps are `BTreeMap` (seed-deterministic iteration; no `HashMap`) | unit | `@in-memory` | `listener_fact_store_iteration_order_is_seed_deterministic` | `src/listener_facts.rs` `#[cfg(test)] mod tests` |
| U6 | `upsert` is a no-op on the conflict-release path (insert only on `Inserted`) | unit | `@error @in-memory` | `conflict_release_does_not_insert_facts` | `src/listener_facts.rs` `#[cfg(test)] mod tests` |
| BE-1 | Hydrate desired for a row WITH a fact carries the right `(port, protocol)` — equal to pre-change cluster-gather result | Tier 1 DST | `@regression-pin @in-memory` | `hydrate_desired_with_fact_matches_pre_change_projection` | `tests/acceptance/listener_fact_hydrate_equivalence.rs` |
| BE-2 | Hydrate row whose `ServiceId` has NO fact → service skipped, `unresolvable_proto` emitted, NO defaulted-`Tcp` `update_service` (C3) | Tier 1 DST | `@error @regression-pin @in-memory` | `hydrate_desired_unresolvable_proto_skips_and_emits_no_tcp_default` | `tests/acceptance/listener_fact_hydrate_equivalence.rs` |
| BE-3 | VIP uniqueness → distinct services derive distinct `ServiceId`s; no fact collision | unit / Tier 1 | `@error @in-memory @property` | `distinct_service_vips_derive_distinct_service_ids_no_collision` | `tests/acceptance/listener_fact_hydrate_equivalence.rs` |

**Counts**: 13 scenarios. By tier — **Tier 1 DST**: 8 (A, B, B-ex, C, BE-1,
BE-2, BE-3, and U-cases run in the same default lane). **Default-lane unit
(`src/listener_facts.rs` mod tests)**: 6 (U1–U6; BE-3 has a unit form too).
Error/edge ratio: U6, BE-2, BE-3 are negative/edge = **≥ 40%** of the
behaviour-pinning set (3 of 7 non-invariant scenarios + the C3 path).

**Graduated DST invariants (load-bearing — the A/B/C gates):** **A**
(zero steady-state `scan_prefix`), **B** + **B-ex** (multi-listener
byte-equivalence), **C** (guard never across `.await`). These are the regression
gates DESIGN sub-decision 5 and ADR-0062 § Testability mandate; DELIVER must
keep all three green.

---

## Wave: DISTILL / [REF] Scenario prose (GIVEN/WHEN/THEN)

### A — Invariant A: zero steady-state `scan_prefix` *(GRADUATED DST invariant)*

> **Mechanism corrected 2026-06-03 (back-propagated from DELIVER).** The original
> mechanism for this property — a counting `IntentStore` decorator over
> `&dyn IntentStore` asserting zero steady-state `scan_prefix` calls — was found
> structurally infeasible: the hydrate read seam is `AppState.store`, a concrete
> `Arc<LocalIntentStore>`, not `Arc<dyn IntentStore>`, so a `&dyn` decorator has
> nowhere to attach (widening the field is out of scope). `scan_prefix` being
> trait-public is irrelevant. The shipped, equivalent-or-stronger mechanism is a
> **delete-intent-then-tick behavioral proof**. The GIVEN/WHEN/THEN below is
> updated to that mechanism; the property is unchanged.

```
@dst-invariant @in-memory @property
Property: ServiceMapHydrator hydrate produces correct desired with the intent
          record ABSENT — i.e. steady-state hydrate does not depend on the intent store

  Given S Service workloads (S drawn from a proptest strategy, S in 1..=8) each
        submitted through the submit_workload edge, each with 1..=3 listeners
  And   the ListenerFactStore boot-rebuilt once via rebuild_from_intent, so the
        keyed fact + the service_backends row are in place for every service
  And   the intent record for each service is then REMOVED from the intent store
        (the OLD scan path would now resolve nothing; the NEW keyed read resolves
         from the in-memory ListenerFactStore)
  When  N convergence ticks (N in 1..=10) drive the ServiceMapHydrator hydrate
        path via hydrate_desired_for_test for every service target
  Then  hydrate yields a correct NON-EMPTY desired for every service on every
        tick — reachable ONLY if hydrate reads memory, not the intent store
  And   reverting the read-switch (hydrate reading the intent store again) makes
        this RED (empty desired once the record is gone)
  And   the property holds for every (S, N, listener-shape) the strategy emits
  And   on failure the proptest seed is printed for bit-exact reproduction
```

Universe (port-observable, per Mandate 8 Rust mapping): the hydrate path's
desired output (a port-exposed observable via `hydrate_desired_for_test`) with
the intent record absent — correct non-empty desired across N ticks is the
observable that proves steady-state hydrate is independent of the intent store.
NOT any `ListenerFactStore` private field. Seed-reproducible
(`PROPTEST_REPLAY=<seed>`). This is equivalent-or-stronger than a `scan_prefix`
counter: it proves the intent record is entirely unnecessary in steady state,
not merely that no scan call fired.

### B — Invariant B: byte-equivalence, multi-listener *(GRADUATED DST invariant)*

```
@dst-invariant @in-memory @property
Property: the edge-maintained store equals a fresh boot-rebuild, entry-for-entry

  Given a proptest strategy over a set of Service intents — distinct spec_digests
        → distinct allocator VIPs (BE-3) → distinct ServiceIds; each service has
        1..=4 [[listener]] entries with arbitrary (port: NonZeroU16, protocol)
        and AT LEAST ONE service in the set has >= 2 listeners
  When  the store is built two ways over the SAME intent set:
          (1) incrementally — each workload pushed through the submit-edge upsert path
          (2) from scratch — ListenerFactStore::rebuild_from_intent over the same intents
  Then  the primary BTreeMap<ServiceId, ListenerRow> of (1) equals that of (2)
        entry-for-entry (key set + each ListenerRow { vip, port, protocol })
  And   the secondary BTreeMap<WorkloadId, Vec<ServiceId>> of (1) equals that of
        (2), INCLUDING inner Vec ordering (insertion / ServiceId order)
  And   a multi-listener service contributes exactly one primary entry per
        listener and one secondary ServiceId per listener (count == listener count)
  And   the property holds for every intent set the strategy emits; seed printed on failure
```

Universe: both maps' full observable contents (key sets, every `ListenerRow`,
secondary inner-`Vec` order) — these are the `ListenerFactStore`'s public
read-surface (`fact_for` + an equivalent enumeration accessor the store
exposes), not private struct internals. `BTreeMap` makes iteration
seed-deterministic so the byte-equivalence is well-defined.

### B-ex — byte-equivalence, fixed 3-listener example *(fallback / regression repro)*

```
@dst-invariant @in-memory @example
Scenario: three-listener Service yields identical store via edge-upsert and boot-rebuild

  Given one Service "web" with three listeners — (80, Tcp), (443, Tcp), (53, Udp)
        — assigned VIP 10.96.0.7
  When  the store is built (1) via upsert(workload_web, 10.96.0.7, &[3 listeners])
        and (2) via rebuild_from_intent over the committed "web" intent
  Then  both primary maps hold exactly three entries keyed by
        ServiceId::derive(&vip, 80, "service-map"),
        ServiceId::derive(&vip, 443, "service-map"),
        ServiceId::derive(&vip, 53, "service-map") with matching ListenerRows
  And   both secondary maps map workload_web -> a 3-element Vec<ServiceId> in the
        same order
  And   the two stores are equal
```

### C — Invariant C: guard never held across `.await` *(GRADUATED DST invariant)*

```
@dst-invariant @in-memory
Scenario: ListenerFactStore guard is released before any .await under read/edge contention

  Given an AppState whose ListenerFactStore is contended by both the hydrate
        read path (store.get/fact_for) and concurrent submit/stop edge updates
        (both acquire the same Arc<Mutex<ListenerFactStore>>)
  When  the hydrate read path runs while a submit-edge upsert and a stop-edge
        remove_workload are driven concurrently (turmoil / multi-task DST)
  Then  no task deadlocks and every task makes progress (assert_eventually!:
        each spawned op completes within the DST budget)
  And   the read path's observable shape is "acquire guard -> clone the small
        value -> drop guard BEFORE the next .await" (mirroring
        hydrate_bridge_desired_listeners:1686-1691)
```

> **Honesty note on the C observable (recommend the most defensible form).**
> "A lock is not held across `.await`" is awkward to assert as a pure runtime
> postcondition — there is no portable runtime hook that says "this guard
> crossed an await point." Two defensible observables, **both** recommended for
> DELIVER, neither alone sufficient:
>
> 1. **Behavioural (primary):** a DST contention scenario (above) — concurrent
>    read + submit + stop on the same mutex under turmoil. If a guard is held
>    across an `.await`, the contending tasks deadlock or stall and
>    `assert_eventually!(all ops complete)` fails. This is the loud, runtime,
>    seed-reproducible gate and is the one that catches the real hazard.
> 2. **Structural (backstop):** a source-shape check — the read path's guard
>    binding is dropped (explicit `drop(guard)` or scope end) on the line
>    *before* any `.await`. The codebase's idiom for this is the
>    acquire→clone→`drop` pattern at `hydrate_bridge_desired_listeners`. A
>    lightweight check (code review + a grep/lint-style assertion that the
>    hydrate read does not bind the guard across an await) documents intent.
>    This is NOT a substitute for the behavioural gate — it is a cheap
>    early-warning. The graduated invariant is the behavioural DST scenario;
>    the structural check is advisory.

### U1 — `upsert` multi-listener

```
@in-memory @property
Property: upsert of a multi-listener workload creates one primary entry per
          listener and one secondary entry with a matching-length Vec

  Given an empty ListenerFactStore and a workload with a VIP and L listeners
        (L from a strategy, L in 1..=5; arbitrary (port, protocol))
  When  upsert(workload_id, vip, &listeners) is called
  Then  the primary map gains exactly L entries, each keyed by
        ServiceId::derive(&vip, listener.port, "service-map") with
        ListenerRow { vip: Some(vip), port, protocol } matching that listener
  And   the secondary map maps workload_id -> a Vec<ServiceId> of length L whose
        order matches the listener order
```

### U2 — `remove_workload` isolation

```
@in-memory
Scenario: remove_workload evicts exactly the target workload's ServiceIds; others untouched

  Given a ListenerFactStore holding facts for two workloads W1 (2 listeners) and
        W2 (1 listener), all distinct ServiceIds
  When  remove_workload(&W1) is called
  Then  the primary map no longer contains either of W1's ServiceIds
  And   the secondary map no longer contains W1
  And   W2's single primary entry and its secondary entry are unchanged
```

### U3 — `fact_for` lookup

```
@in-memory
Scenario: fact_for returns the expected row for a present service and None for an absent one

  Given a ListenerFactStore where ServiceId Sx maps to
        ListenerRow { vip: Some(10.96.0.3), port: 8080, protocol: Tcp }
  When  fact_for(&Sx) is called
  Then  it returns Some(ListenerRow { vip: Some(10.96.0.3), port: 8080, protocol: Tcp })
  When  fact_for(&Sy) is called for an Sy never inserted
  Then  it returns None
```

### U4 — `rebuild_from_intent` skip semantics

```
@in-memory @example
Scenario: rebuild_from_intent projects Service intents only, honoring the gather skip rules

  Given an IntentStore committed with:
          - a Service intent "svc-a" (2 listeners) with an allocator VIP memo
          - a Job intent "job-b" (no listeners)
          - a Schedule intent "sched-c"
          - a Service intent "svc-d" with NO allocator VIP memo (allocator.get == None)
          - the sub-keys workloads/svc-a/stop and workloads/svc-a/kind
  When  ListenerFactStore::rebuild_from_intent(state) runs
  Then  the primary map holds exactly the 2 entries from "svc-a" (one per listener)
  And   "job-b" and "sched-c" contribute nothing (not Service intents)
  And   "svc-d" contributes nothing (no allocator VIP — matches current gather's
        `let Some(assigned_vip) = ... else { continue }` skip)
  And   the /stop and /kind sub-keys contribute nothing (suffix.contains('/') skip)
  And   the secondary map maps only workload "svc-a" -> its 2 ServiceIds
```

### U5 — `BTreeMap` determinism

```
@in-memory
Scenario: both ListenerFactStore maps iterate in seed-deterministic ServiceId / WorkloadId order

  Given a ListenerFactStore populated (via upsert) with several workloads whose
        ServiceIds and WorkloadIds are inserted in a deliberately scrambled order
  When  the primary and secondary maps are iterated
  Then  primary iteration yields ServiceIds in Ord order (BTreeMap guarantee)
  And   secondary iteration yields WorkloadIds in Ord order
  And   the store type uses BTreeMap for both maps (no HashMap; per
        development.md § "Ordered-collection choice" — observed by DST invariants)
```

### U6 — conflict-release no-op

```
@error @in-memory
Scenario: a conflict-release does not insert listener facts (insert only on Inserted)

  Given a submit attempt whose put_if_absent returns a conflict (KeyExists) — a
        different spec already registered at the key
  When  the conflict-release path runs (VIP released; no intent committed)
  Then  the ListenerFactStore primary and secondary maps are unchanged (no
        entries added for the rejected spec) — symmetric with the VIP, which is
        allocated-then-released on conflict
```

### BE-1 — hydrate behaviour-equivalence (row WITH a fact) *(regression pin)*

```
@regression-pin @in-memory
Scenario: hydrate desired for a backend row whose ServiceId has a fact carries the
          right (port, protocol) — equal to the pre-change cluster-gather result

  Given a service target whose service_backends row carries
        row.service_id = Sx and row.vip = V
  And   the ListenerFactStore holds Sx -> ListenerRow { vip: Some(V), port: P, protocol: Pr }
  When  the ServiceMapHydrator hydrate arm runs via hydrate_desired_for_test
  Then  desired[Sx] carries (port = P, protocol = Pr)
  And   this result is identical to what the pre-change cluster-wide
        gather_service_listener_facts + project_service_desired produced for the
        same row and the same intent set (regression-equivalence: the O(1) keyed
        read replaces the scan WITHOUT changing the projection)
```

### BE-2 — C3 unresolvable-proto guard preserved *(regression pin, error path)*

```
@error @regression-pin @in-memory
Scenario: a backend row whose ServiceId has NO fact skips the service, emits
          unresolvable_proto, and never defaults to Tcp (ADR-0060 C3)

  Given a service target whose service_backends row carries row.service_id = Sz
  And   the ListenerFactStore has NO entry for Sz (fact_for(&Sz) == None)
  When  the ServiceMapHydrator hydrate arm runs via hydrate_desired_for_test
  Then  desired does NOT contain Sz (the service is skipped)
  And   a tracing event named "service_map_hydrator.desired.unresolvable_proto"
        is emitted carrying service_id = Sz
  And   NO desired entry carrying a silently-defaulted Proto::Tcp is produced for Sz
        (C3 guard preserved verbatim through the read-path change)
```

### BE-3 — VIP uniqueness → distinct ServiceIds *(error/edge, no collision)*

```
@error @in-memory @property
Property: distinct service VIPs derive distinct ServiceIds — no fact collision in the store

  Given a strategy over >= 2 services with pairwise-distinct allocator VIPs
        (the allocator's per-spec_digest uniqueness guarantee) and overlapping
        listener ports (e.g. two services both on port 80)
  When  facts are upserted for all of them
  Then  every service's port-80 listener maps to a DISTINCT ServiceId
        (ServiceId::derive folds the VIP into the hash, so equal ports across
         distinct VIPs do not collide)
  And   no upsert overwrites another service's primary entry
  And   the primary map's entry count equals the total listener count across all services
```

---

## Wave: DISTILL / [REF] Adapter / driven-port coverage

No NEW driven adapter is introduced (DESIGN: "Driven port (outbound): none new").
`ListenerFactStore` reads the existing `IntentStore` + allocator at boot only;
steady state is pure in-memory. Coverage of the existing driven-internal
adapters touched:

| Driven-internal adapter | Exercised by | Tier |
|---|---|---|
| `IntentStore` (`scan_prefix`) — boot rebuild; invariant A proven via delete-intent-then-tick, NOT a counting decorator (the read seam `AppState.store` is concrete `Arc<LocalIntentStore>`, not `dyn` — see scenario A note) | A (intent-store-independence), U4 (rebuild skip semantics) | Tier 1 / unit, `Sim` / `LocalIntentStore` |
| `IntentStore` (`put_if_absent`, `get`) — submit/stop edge | B, U6, BE-1/BE-2 via `AppState` | Tier 1 |
| `PersistentServiceVipAllocator` (`get` at boot, `allocate`/`release` at edge) | B, U4 (VIP-memo skip), BE-3 | Tier 1 |
| `ObservationStore` (`service_backends_rows`) — `SimObservationStore` | BE-1, BE-2 | Tier 1 |

All exercised through in-process `Sim*` / `Local*` adapters in the default
lane — no `integration-tests` feature gate needed (no real kernel/network).

---

## Wave: DISTILL / [REF] Test placement (files the roadmap should reference)

**Production (NEW — DELIVER creates; DISTILL does not stub):**
- `crates/overdrive-control-plane/src/listener_facts.rs` — the `ListenerFactStore` type + its `#[cfg(test)] mod tests` (homes U1–U6). DELIVER's RED phase authors `#[should_panic(expected = "RED scaffold")]` bodies here.

**Tier-1 DST / acceptance (NEW — wired via the existing `tests/acceptance.rs` inline `mod acceptance { ... }` block per ADR-0005; one file per scenario group, matching the `service_backends_hydrate_desired.rs` / `service_vip_submit_acceptance.rs` precedent):**
- `crates/overdrive-control-plane/tests/acceptance/listener_fact_zero_scan_invariant.rs` — **invariant A** via the delete-intent-then-tick behavioral proof (the counting-decorator mechanism was infeasible — see scenario A note; filename retained as shipped).
- `crates/overdrive-control-plane/tests/acceptance/listener_fact_byte_equivalence_invariant.rs` — **invariant B** + B-ex.
- `crates/overdrive-control-plane/tests/acceptance/listener_fact_lock_discipline_invariant.rs` — **invariant C** (DST contention).
- `crates/overdrive-control-plane/tests/acceptance/listener_fact_hydrate_equivalence.rs` — BE-1, BE-2, BE-3 (read-path behaviour pins).
- Each new file is added as a `mod <name>;` inside the `mod acceptance { ... }` block in `crates/overdrive-control-plane/tests/acceptance.rs`.

**Deleted in the same DELIVER change (per deletion discipline + sub-decision 3):**
- The per-tick caller of `gather_service_listener_facts` at `reconciler_runtime.rs:1335` and any test asserting the *per-tick scan behaviour*. The rebuild logic's behaviour is re-asserted fresh as U4 (boot-rebuild) — a genuinely different requirement; do NOT salvage-rename old per-tick-scan tests.

**Existing in-tree references the roadmap/crafter should anchor to:**
- `reconciler_runtime::hydrate_desired_for_test` (`reconciler_runtime.rs:1599`) — driving port for A / BE-1 / BE-2.
- `gather_service_listener_facts` (`reconciler_runtime.rs:1733-1800`) — body relocated to `ListenerFactStore::rebuild_from_intent`.
- `ServiceId::derive` (`overdrive-core/src/id.rs:825`); call site precedent `reconciler_runtime.rs:1705`.
- `project_service_desired` (`overdrive-core/src/reconcilers/service_map_hydrator.rs:97`) — the C3-guard projection BE-1/BE-2 pin equivalence against.
- `scan_prefix` trait method (`overdrive-core/src/traits/intent_store.rs:255`) — the boot-rebuild scan call; NOT a decorator target (the counting-decorator mechanism for A was infeasible — `AppState.store` is concrete `Arc<LocalIntentStore>`, not `dyn`; see scenario A note).
- `ListenerRow` (`overdrive-core/src/traits/observation_store.rs:321`).

---

## Wave: DISTILL / [REF] Pre-requisites

- **Delete-intent-then-tick harness (invariant A).** *(Corrected 2026-06-03 —
  the originally-specified counting `IntentStore` decorator is structurally
  infeasible: the hydrate read seam `AppState.store` is a concrete
  `Arc<LocalIntentStore>`, not `Arc<dyn IntentStore>`, so a `&dyn` decorator has
  nowhere to attach without widening the field — out of scope. `scan_prefix`
  being trait-public does not help.)* The invariant-A test instead drives the
  real `AppState` / `LocalIntentStore`: submit S services, boot-rebuild the
  `ListenerFactStore`, then **remove the intent record(s)** from the store and
  run N hydrate ticks asserting a correct non-empty `desired` every tick. No
  test-only adapter is required — the harness reuses the same `AppState` test
  builder (below) and removes intent keys via the store's existing delete
  surface. Default lane, no `integration-tests` gate.
- **proptest generators (invariants B, BE-3, U1, A).** Strategies for: a set of
  Service intents with pairwise-distinct VIPs and `1..=4` listeners each (≥1
  multi-listener), arbitrary `(NonZeroU16, Proto)` listeners, and `(S, N)`
  counts for the tick loop. Generators live next to the types they produce per
  `.claude/rules/testing.md` § "Property-based testing" (generators next to the
  type). `Proto` / `NonZeroU16` / `ServiceVip` need `Arbitrary`-style strategies
  (reuse existing ones if present; otherwise add local strategies in the test
  module).
- **`AppState` test builder.** Reuse the `build_app_state(tmp, obs)` shape from
  `service_backends_hydrate_desired.rs` (Sim adapters + `LocalIntentStore` +
  `TempDir`), extended to construct the new `listener_facts:
  Arc<Mutex<ListenerFactStore>>` field via `rebuild_from_intent` next to the
  allocator `bulk_load` — the same wiring the production `run_server_*` path
  adds.
- **Seed reproduction.** All `@property` scenarios print the proptest/DST seed on
  failure and reproduce via `PROPTEST_REPLAY=<seed>` / `cargo dst --seed <N>`
  per project discipline. Flaky DST/proptest is a bug, not a rerun.

---

## Wave: DISTILL / [REF] Graduation summary

| Class | Scenarios | Role |
|---|---|---|
| **Load-bearing DST invariants (graduate as A/B/C)** | **A** (zero steady-state `scan_prefix`), **B** + **B-ex** (multi-listener byte-equivalence), **C** (guard never across `.await`) | The regression gates DESIGN sub-decision 5 + ADR-0062 § Testability mandate. DELIVER must keep all three green; they are the fix's structural defense (drift, scan-reintroduction, deadlock). |
| **Default-lane unit (`ListenerFactStore` contract)** | U1, U2, U3, U4, U5, U6 | Pin the store's own API contract (upsert / remove / fact_for / rebuild / ordering / conflict-no-op). Cheap, fast, in `src/listener_facts.rs mod tests`. |
| **Behaviour-equivalence pins (preserve pre-change behaviour)** | BE-1 (fact present), BE-2 (C3 unresolvable-proto), BE-3 (VIP uniqueness) | Prove the O(1) keyed read replaces the scan WITHOUT changing the observable projection, and that the ADR-0060 C3 guard survives verbatim. BE-2 is the error-path guard. |

**Mandate compliance (Rust mapping):**
- *Mandate 8 (universe-bound state-delta)* — satisfied natively per the infra-policy mapping: each state-mutating scenario asserts over **port-observable** universes (hydrate's desired output with the intent record absent for A; both maps' public contents for B; desired-map keys + emitted tracing event for BE-1/BE-2), never private struct fields. No Python `assert_state_delta` port used.
- *Mandate 9 (layer-dependent PBT)* — these are layer 1–2 (in-process, `Sim*`), so PBT-full (`@property` via proptest) is permitted and used for A, B, BE-3, U1. No layer-3+ scenario exists, so no example-only-at-layer-3 constraint applies.
- *Mandate 11 (layer-3+ sad paths example-only)* — N/A (no layer-3 scenario). The sad paths (U6, BE-2, BE-3) are at layer 1–2; BE-2/U6 are pinned as named examples, BE-3 as a property.
- *Pillar 1 (domain language)* — scenario prose uses domain terms (service, listener, VIP, hydrate, fact); the Rust type names appear only as the testable surface, per project convention (no `.feature` business-language purity layer exists here).
