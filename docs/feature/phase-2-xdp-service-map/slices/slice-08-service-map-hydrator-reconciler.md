# Slice 08 — SERVICE_MAP hydrator reconciler converges Dataplane port

**Story**: US-08
**Backbone activity**: 8 (Close the J-PLAT-004 reconciler loop — cross-cutting against activities 2-6)
**Effort**: 1 day
**Depends on**: Slice 02 (`Dataplane::update_service` body must do something for the hydrator's emitted Action to have effect).
**Parallel-able with**: Slices 03, 04, 05, 06, 07 (the hydrator does not need to know about HASH_OF_MAPS / Maglev / REVERSE_NAT / sanity-prologue / perf-gate mechanics — it calls `update_service(service_id, backends)` and `EbpfDataplane` owns the swap).

## Outcome

A `ServiceMapHydrator` reconciler lands in
`crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs`
per ADR-0035 / ADR-0036. Sync `reconcile`, no `.await`, no wall-clock
reads inside the function (`tick.now` snapshot from the runtime), no
DB handle held by the reconciler — the runtime owns hydration of
intent, observation, and View end-to-end. Typed
`ServiceMapHydratorState` projects desired backends per `ServiceId`
(from IntentStore / spec-derived projection thereof) and actual
backends per `ServiceId` (from `service_backends` rows in
ObservationStore); typed `ServiceMapHydratorView` carries retry
inputs (`attempts`, `last_failure_seen_at`, last-seen
`service_backends` generation per service) — never derived deadlines
or pre-computed backoff windows per "Persist inputs, not derived
state."

The reconciler emits `Action::DataplaneUpdateService` (or
DESIGN-chosen Action variant — flagged in `wave-decisions.md`'s
"What is NOT being decided in this wave") per service whose backend
set has drifted; the action shim dispatches against
`Arc<dyn Dataplane>` so production wiring uses `EbpfDataplane`, DST
uses `SimDataplane`. Two new DST invariants in
`overdrive-sim::invariants`: `HydratorEventuallyConverges` (eventual:
from any seeded combination of `service_backends` rows + starting
`SimDataplane` state, repeated reconcile ticks drive `actual ==
desired`) and `HydratorIdempotentSteadyState` (always: once
`actual == desired`, no further action is emitted on subsequent ticks
given unchanged inputs). The existing `ReconcilerIsPure` invariant
continues to pass with the hydrator added to the catalogue.

## Value hypothesis

*If* ESR convergence holds against the new dataplane port — `actual`
driven from `service_backends` rows, `desired` driven from
IntentStore-derived projection, hydrator emits
`Action::DataplaneUpdateService` — then the §18 reference shape works
for every later dataplane reconciler. *Conversely*, a regression here
means the SimDataplane ↔ EbpfDataplane port shape is wrong and every
later slice (POLICY_MAP / IDENTITY_MAP / conntrack #154 / sockops /
kTLS) inherits that confusion. Disproving here is cheap; disproving
later compounds.

## Disproves (named pre-commitment)

- **"The hydrator can wait until Phase 2.3+."** No — without it,
  every earlier slice's `Dataplane` port plumbing is untested
  against an ESR-shaped consumer; J-PLAT-004's activation is
  performative rather than evidenced.
- **"The hydrator needs to know about HASH_OF_MAPS / Maglev /
  atomic-swap mechanics."** No — it calls
  `update_service(service_id, backends)` and `EbpfDataplane` owns the
  swap (Slice 03's 5-step shape, Slice 04's Maglev table generation).
  This is the structural reason the hydrator can land in parallel
  with Slices 03-07.

## Scope (in)

- `ServiceMapHydrator` reconciler with `type State = ServiceMapHydratorState` and `type View = ServiceMapHydratorView` per ADR-0035 / ADR-0036.
- Sync `reconcile` (no `async fn`, no `.await` inside the body); no direct wall-clock reads (`tick.now` only); no IntentStore / ObservationStore / ViewStore handle held by the reconciler — runtime owns hydration end-to-end.
- `ServiceMapHydratorView` derives `Serialize + DeserializeOwned + Default + Clone + Send + Sync`; carries inputs only (`attempts`, `last_failure_seen_at`, last-seen `service_backends` generation per service) — no derived deadlines persisted.
- `Action::DataplaneUpdateService` emission (or DESIGN-chosen Action variant) per service whose backend set has drifted; action shim dispatches against `Arc<dyn Dataplane>`.
- DST invariants `HydratorEventuallyConverges`, `HydratorIdempotentSteadyState` in `overdrive-sim::invariants`.
- `ReconcilerIsPure` continues to pass with the hydrator added to the catalogue.
- `dst-lint` clean on `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs`.
- Tier 1 (DST) is the primary surface; Tier 2 / Tier 3 secondary — exercised end-to-end by Slice 02 / 03 / 04's existing integration tests because `EbpfDataplane::update_service` is the call site the action shim drives.

## Scope (out)

- Maglev table generation (Slice 04 — the hydrator does not generate the table; `EbpfDataplane::update_service` owns that work).
- Atomic-swap mechanics (Slice 03 — the hydrator does not implement HASH_OF_MAPS swaps; it triggers them by calling `update_service`).
- Cross-node propagation — a hydrator on Node A reacting to `service_backends` rows written by Node B (deferred to Phase 5.20 — see #156, alongside HA / Corrosion-driven map hydration / multi-node consensus).
- The exact `Action` variant name and shape if a new variant vs reusing an existing one — DESIGN-time concern (see `wave-decisions.md` § "What is NOT being decided in this wave").
- Conntrack-aware reconciliation (#154).
- Kernel matrix (#152).

## Target KPI

- 100% pass rate of `HydratorEventuallyConverges` across every DST seed on every PR.
- 100% pass rate of `HydratorIdempotentSteadyState` across every DST seed on every PR.
- `ReconcilerIsPure` continues to pass with the hydrator added to the catalogue.
- 0 `.await` / direct wall-clock / DB-handle violations in `reconcile` (`dst-lint` clean).

## Acceptance flavour

See US-08 scenarios. Focus: `HydratorEventuallyConverges` (eventual)
and `HydratorIdempotentSteadyState` (always) on every DST seed;
`ReconcilerIsPure` continues to pass; `dst-lint` enforces the
sync-`reconcile` discipline.

## Failure modes to defend

- Stale ObservationStore / transient gossip miss: every tick acts on the snapshot it sees; the next tick converges. No spurious intermediate state.
- Transient `update_service` failure (e.g. `MapAllocFailed`): View bumps `attempts` and records `last_failure_seen_at`; subsequent ticks honor the backoff window per persisted inputs (deadline recomputed from `last_failure_seen_at + backoff_for_attempt(attempts)`, never persisted). Once successful, `attempts` resets to 0.
- Concurrent reconcile ticks against the same service: per ADR-0036, the runtime serialises tick dispatch per `(reconciler_name, target)`; the reconciler does not need to defend against this shape internally.
- Schema evolution of `ServiceMapHydratorView`: additive fields use `#[serde(default)]`; breaking changes use a versioned envelope (no breaking-change history yet — first breaking change ships with the envelope).

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — `ServiceMapHydrator` reconciler + `ServiceMapHydratorState` + `ServiceMapHydratorView` + 2 paired DST invariants (4) |
| No hypothetical abstractions landing later | PASS — depends on Slice 02's `Dataplane::update_service` body; uses the existing §18 reconciler runtime and ADR-0035 ViewStore. The §18 reference shape is published; this is the first non-trivial use of it against a real dataplane port |
| Disproves a named pre-commitment | PASS — see "Disproves" above |
| Production-data-shaped AC | PASS — DST invariants on every PR; Tier 2 / Tier 3 exercised end-to-end by Slice 02-04's existing integration tests against `EbpfDataplane::update_service` |
| Demonstrable in single session | PASS — `cargo xtask dst` against the new invariant pair runs in single-digit minutes on the developer's Lima VM |
| Same-day dogfood moment | PASS — Linux developer iterates the reconciler against `SimDataplane` + DST seeds; replays seeded counter-examples bit-identically |
