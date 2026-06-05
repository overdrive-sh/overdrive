# Slice 01 — Walking skeleton: one durable step that survives a crash

**Goal (one sentence):** A `Workflow` trait + `WorkflowCtx` with ONE durable
operation, journaled to redb at the await-point, brought up by the
workflow-lifecycle reconciler via `Action::StartWorkflow`, with single-node
crash-resume demonstrated under DST (kill mid-run → restart → completed step
NOT repeated → terminal result matches) and the terminal result observable in
the ObservationStore.

This is the hard core every later slice depends on — per the carpaccio taste
test "if every slice depends on a new abstraction, ship the abstraction FIRST
as its own slice." It is shipped end-to-end, not as an `@infrastructure`-only
engine.

**Job / outcomes:** J-PLAT-005 → O1 (no repeated side effect on resume), O4
(resumed terminal == uninterrupted terminal), O5 (provable replay-equivalence
before ship), O6 (journal on the existing redb substrate). O3 partial (one
`async fn run`, no step enum).

## IN scope

- `Workflow` trait: `async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult`.
- `WorkflowCtx` with ONE durable operation (recommend `ctx.call(...)` against
  the injected `Transport` — the existing `Action::HttpCall` correlation
  machinery is the precedent). All non-determinism through `ctx`.
- A per-instance append-only journal in redb (distinct layout from the
  reconciler `View` store, same backend). Write checkpoint BEFORE suspend.
- `WorkflowSpec` made concrete (replacing the `Action::StartWorkflow`
  placeholder shape at `reconcilers/mod.rs:373`).
- Workflow-lifecycle reconciler brings up exactly ONE instance from
  `Action::StartWorkflow { spec, correlation }`; emits a terminal-result row
  to the ObservationStore.
- First real consumer: **`ProvisionRecord` — a minimal 2-step durable
  sequence** (`ctx.call` to write one record, then return a terminal result).
  Justification: it is the thinnest sequence that has a *real, non-idempotent-
  to-repeat external effect* (the write) — which is the whole point of O1.
  A cert-rotation slice is the wrong skeleton: it needs ACME + DNS-propagation
  `ctx.sleep` + multi-await, i.e. it depends on slice 02/03 surface that does
  not exist yet.
- A named `replay_equivalence_provision_record` SimInvariant in
  `overdrive-sim` (no inline literal) + a paired bounded-progress
  `assert_eventually!(is_terminal)`, on the CI critical path.

## OUT of scope

- `ctx.sleep`, `ctx.wait_for_signal`, `ctx.activity` (slices 02/03).
- Cross-workflow signals / parent-child composition (slice 03).
- Cross-NODE resume (multi-node / HA — sequencing dependency, NOT this slice) — #205.
- Operator `overdrive workflow` CLI verb (#206; none in cli.rs). Observability
  here is the ObservationStore row + lifecycle event.
- Journal retention/compaction policy (#208).
- WASM workflow SDK + version-skew code-graph hashing (#209; dispatch-forbidden).

## Learning hypothesis

- **Disproves if it fails:** "a distinct durable-async `Workflow` primitive
  journaled in redb can deliver exactly-once crash-resume on the existing
  reconciler substrate without a second persistence engine." If the redb
  per-instance journal cannot express append-only await checkpoints cleanly,
  or replay-equivalence cannot be made a DST invariant, the locked B′
  direction is in trouble and we learn it on the smallest possible surface.
- **Confirms if it succeeds:** the engine + journal + replay core is sound;
  every later await-surface slice (sleep, signal, activity) is additive on a
  proven foundation.

## Acceptance criteria (production data, not synthetic)

- AC1 (O1): Under DST, killing the instance AFTER the `ctx.call` records but
  BEFORE terminal, then restarting, results in the `ctx.call` external effect
  executing EXACTLY ONCE (asserted on the SimTransport call count), not twice.
- AC2 (O4): The resumed run reaches a `WorkflowResult` byte-identical to the
  uninterrupted run's result for the same inputs + seed.
- AC3 (O5): `cargo dst --only replay_equivalence_provision_record` is green,
  prints a seed, and reproduces bit-for-bit on a second run.
- AC4 (O6): The journal row is written to the redb substrate (asserted via the
  ViewStore-sibling journal handle), NOT to libSQL.
- AC5 (observable): After terminal, the ObservationStore carries a
  terminal-result row keyed by the instance's `CorrelationKey`.

## Dependencies

- EXISTS (brownfield): reconciler runtime, redb ViewStore, Action channel,
  ObservationStore, DST harness, `Action::StartWorkflow` placeholder.
- BLOCKS: slices 02, 03, 04 (all build on this engine).

## Effort estimate

≤1 day of crafter dispatch is OPTIMISTIC for the full skeleton — flag as the
one slice that may run 1–1.5 days. Reference class: the
`service_map_hydrator` reconciler + its DST invariant landed as a comparable
"new primitive + named DST invariant" unit. If a pre-slice SPIKE is warranted
it is here: a half-day SPIKE on "can the redb journal express per-instance
append-only await checkpoints with the fsync-before-suspend ordering" de-risks
the whole feature.

**Reference class:** `service-map-hydrator` (new reconciler + named DST
invariant, single feature slice).
