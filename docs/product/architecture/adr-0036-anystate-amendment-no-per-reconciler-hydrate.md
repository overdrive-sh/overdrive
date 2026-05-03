# ADR-0036 — Amendment to ADR-0021: remove the per-reconciler `hydrate(target, db)` surface; runtime owns all hydration

## Status

Accepted. 2026-05-03. Decision-makers: Morgan (proposing); user
ratification 2026-05-03 (mode: propose, single-pass option
selection — Option A from
`docs/feature/reconciler-memory-redb/design/wave-decisions.md`).
Tags: phase-1, reconciler-primitive, application-arch.

**Amends ADR-0021** ("Reconciler `State` shape: per-reconciler typed
`AnyState` enum mirroring `AnyReconcilerView`"). The per-reconciler
`hydrate(target, db)` async surface is removed; the runtime now owns
*all* hydration (intent + observation + view memory).

**Companion**: ADR-0035 (the structural decision; this ADR is the
amendment for the State / View interaction).

## Context

ADR-0021 established the per-reconciler typed `State` shape via the
`AnyState` enum, with three async hydration surfaces in the runtime:

1. `AnyReconciler::hydrate_desired(target)` — runtime reads
   IntentStore, returns `AnyState` variant.
2. `AnyReconciler::hydrate_actual(target)` — runtime reads
   ObservationStore, returns `AnyState` variant.
3. `Reconciler::hydrate(target, db)` — *reconciler-author-owned*
   read of the per-reconciler libSQL private memory, returns
   `Self::View`.

The third surface was the explicit reason for the libSQL machinery
in ADR-0013 §6 — it was the only async surface the reconciler
author *owned*. The runtime owned the other two.

ADR-0035 collapses the trait to a single sync `reconcile` method and
moves View persistence into a runtime-owned `ViewStore`. The third
async surface — the reconciler's `hydrate(target, db)` — is removed
in the same change.

This ADR documents the consequence for ADR-0021's contracts.

## Decision

### 1. The per-reconciler `Reconciler::hydrate` is removed

ADR-0021 §3 documents the lifecycle:

> The reconciler's existing `hydrate(target, db)` method retains its
> narrow remit (the libSQL private-memory read) — it is NOT extended
> to read other stores. This preserves the ADR-0013 hygiene that
> puts the reconciler author in charge of *one* async surface (its
> own private DB) and the runtime in charge of all the others.

**Per ADR-0035, the reconciler is no longer in charge of any async
surface.** The runtime owns all hydration:

| Source | Owner | Trigger | Mechanism |
|---|---|---|---|
| Intent (Job, Node) | Runtime | Per tick | `AnyReconciler::hydrate_desired` reads IntentStore |
| Observation (allocations, node_health) | Runtime | Per tick | `AnyReconciler::hydrate_actual` reads ObservationStore |
| Reconciler memory (View) | Runtime | Bulk-load at register; in-memory `BTreeMap` thereafter | `ViewStore::bulk_load` once + `BTreeMap::get` per tick |

### 2. The runtime tick contract under the amendment

ADR-0021 §3 documented the tick lifecycle:

```
1. Pick reconciler from registry by name           (enum dispatch)
2. Open (or reuse) LibsqlHandle for name           (path from ADR-0013 §5)
3. tick <- TickContext::snapshot(clock)            (ADR-0013 §2c)
4. desired <- runtime.hydrate_desired(self, target)  (NEW — async; runtime owns)
5. actual  <- runtime.hydrate_actual(self, target)   (NEW — async; runtime owns)
6. view    <- reconciler.hydrate(target, db).await   (per ADR-0013)
7. (actions, next_view) =
       reconciler.reconcile(&desired, &actual, &view, &tick)
8. Persist diff(view, next_view) to libsql
9. Dispatch actions to the action shim (see ADR-0023)
```

**Replaced under ADR-0035 + this amendment** (which replaces step 2
+ step 6 + step 8):

```
1. (any_reconciler, in_memory_views) =
       registry.lookup(name)                       (no LibsqlHandle)
2. tick <- TickContext::snapshot(clock)            (ADR-0013 §2c — survives)
3. desired <- AnyReconciler::hydrate_desired(...)   (ADR-0021 — survives)
4. actual  <- AnyReconciler::hydrate_actual(...)    (ADR-0021 — survives)
5. view    <- in_memory_views.get(target).cloned()
                .unwrap_or_else(R::View::default)   (NEW — sync map lookup)
6. (actions, next_view) =
       reconciler.reconcile(&desired, &actual, &view, &tick)
                                                    (ADR-0021 — survives)
7. ViewStore::write_through(name, target, &next_view).await?
                                                    (durable fsync; ADR-0035)
8. in_memory_views.insert(target.clone(), next_view)
                                                    (after fsync OK)
9. action_shim::dispatch(actions, ...)              (ADR-0023 — survives)
```

The two surfaces ADR-0021 explicitly created (`hydrate_desired` and
`hydrate_actual`) survive; the third surface that ADR-0021 explicitly
*preserved* (`Reconciler::hydrate`) is gone.

### 3. The `AnyState` enum and `JobLifecycleState` shape are unchanged

ADR-0021's structural decisions all survive:

- `AnyState` enum with one variant per reconciler kind, mirroring
  `AnyReconcilerView`.
- `JobLifecycleState { job, nodes, allocations }` shape with shared
  `desired`/`actual` projection.
- Per-tick I/O proportional to the running reconciler, not the
  registered set.
- Compile-time exhaustiveness via match arms.
- `desired` and `actual` collapse into one struct per reconciler.
- The runtime owns hydration of `desired` and `actual`; the
  reconciler does not.

The only deletion is the third sentence in ADR-0021 §3 (the
"reconciler's existing `hydrate(target, db)` retains its narrow
remit" claim) — that surface no longer exists.

## Consequences

### Positive

- **Symmetric ownership.** All three of `desired`, `actual`, and
  `view` are runtime-hydrated; the reconciler sees pre-computed
  inputs across the board. The pre-hydration property is uniform.
- **One fewer async surface in the trait.** `Reconciler::hydrate`
  is removed alongside `migrate` and `persist` per ADR-0035.
- **One fewer error envelope in the runtime contract.**
  `HydrateError` (the per-reconciler libSQL read error) is replaced
  by `ViewStoreError` (the runtime's storage error). Both errors
  surface the same way (`ControlPlaneError::Internal`); the
  reduction is in the number of error types crossing the trait
  boundary.
- **Storage tier consistency.** All three ADR-0021 hydration paths
  now follow the same pattern: runtime reads the underlying store,
  match-dispatches to the typed projection, returns the typed
  output. View hydration becomes a sync `BTreeMap::get`; intent and
  observation remain async store reads.

### Negative

- **None of substance for the State shape.** This ADR is a
  recordkeeping amendment.

### Quality-attribute impact

- **Maintainability — modifiability**: positive (small). One fewer
  trait method to author per reconciler.
- **Maintainability — testability**: neutral. The State shape
  testing approach (twin invocation against typed `AnyState`
  variants) is unchanged.
- **All other attributes**: neutral.

## Compliance — what survives from ADR-0021

ADR-0021 is **amended**, not superseded. Every ADR-0021 section
*except* the third sentence of §3 survives unchanged:

- **§1 (Per-reconciler typed `AnyState` enum, mirroring
  `AnyReconcilerView`)** — preserved verbatim.
- **§2 (`desired` and `actual` collapse into one struct per
  reconciler)** — preserved verbatim.
- **§3 (Hydration owned by the runtime, not the reconciler)** —
  PARTIAL. The runtime continues to own `hydrate_desired` and
  `hydrate_actual` exactly as ADR-0021 specified. The third
  sentence — about `Reconciler::hydrate(target, db)` retaining its
  narrow remit — is removed. Per ADR-0035, the reconciler has no
  async surface at all; the runtime owns all three hydration paths.
- **Considered alternatives (A: Generic, B: God-object struct, C:
  per-reconciler typed)** — preserved; the rejected/accepted
  rationale is unchanged.
- **Compliance section (ADR-0013 §2 / §2b, ADR-0013 §2a, the
  `ReconcilerIsPure` invariant, the ordered-collection rule, the
  newtype-STRICT rule, the trait-signature compile-fail test)** —
  all preserved. The compile-fail test gains one new assertion
  (the `&LibsqlHandle` parameter is rejected anywhere it appears
  in the reconciler trait's parameter list — covered by ADR-0035 §7).

## References

- ADR-0021 — the amended ADR.
- ADR-0035 — the structural decision this amendment follows from.
- ADR-0013 — superseded by ADR-0035; cited here only for the
  historical lineage of the `Reconciler::hydrate` surface.
- ADR-0017 — `ReconcilerIsPure` invariant; preserved.
- `docs/feature/reconciler-memory-redb/design/wave-decisions.md`
  — Option A rationale.
- `docs/feature/reconciler-memory-redb/design/upstream-changes.md`
  — full enumeration of artifacts that need updating.

## Changelog

- 2026-05-03 — Initial accepted version. Amends ADR-0021.
