# Slice 03 — Typed signals + workflow→cluster Action emission

**Goal (one sentence):** A workflow can `ctx.wait_for_signal(...).await` on a
typed signal in the ObservationStore and `ctx.emit_action(...)` a typed
cluster mutation through the Action channel (landing in Raft), so cross-
workflow coordination and workflow→reconciler intent both survive a crash and
go through the sanctioned channels — never a direct IntentStore write.

**Job / outcomes:** J-PLAT-005 → O1 (signal-wait / action-emit not duplicated
on resume), O3 (coordination is `ctx.*`, not hand-rolled), O4/O5 (replay holds
across a signal wait).

## IN scope

- `ctx.wait_for_signal(SignalKey)` — a first-class typed signal in the
  ObservationStore (whitepaper §18 "Cross-workflow coordination uses typed
  signals … not ad-hoc IntentStore writes"). Journaled await-point.
- `ctx.emit_action(Action)` — workflow → cluster mutation through the SAME
  Action channel the reconciler runtime consumes; lands in Raft. Workflows do
  NOT bypass Raft (development.md Workflow contract rule 6).
- A signal-checkpoint journal entry; resume re-checks whether the signal has
  ALREADY arrived (does not re-block if satisfied).
- Extend the DST invariant: crash while blocked on a signal → restart →
  resume blocks on the SAME signal (not a duplicate emit, not a lost wait).

## OUT of scope

- Parent-child workflow composition (`ctx` awaiting a CHILD workflow result) —
  this is the next natural slice (04) once signals exist.
- Cross-node signal delivery semantics under partition (#207; multi-node
  dependency — Phase-1 single-node delivers signals in-process via the ObservationStore).
- Operator CLI to inject/inspect signals (#206; no CLI verb).

## Learning hypothesis

- **Disproves if it fails:** "typed signals in the ObservationStore and
  Action emission through Raft are both crash-safe await/emit points." If
  resume double-emits an Action (double cluster mutation) or loses a satisfied
  signal, the coordination model is unsafe and parent-child composition
  (slice 04) cannot be built on it.
- **Confirms if it succeeds:** the full whitepaper §18 coordination surface
  (signals + action emission) is durable; composition is additive.

## Acceptance criteria (production data, not synthetic)

- AC1 (O1): Crash while blocked on `ctx.wait_for_signal` → on resume the
  workflow blocks on the SAME signal; no duplicate downstream effect.
- AC2 (Raft, no bypass): `ctx.emit_action` lands the typed Action in the Raft
  channel; asserted that the workflow performs NO direct IntentStore write.
- AC3 (idempotent emit): crash AFTER an `emit_action` records but before
  terminal → the Action is NOT re-emitted on resume (journal records the
  emit).
- AC4 (O5): `replay_equivalence_*` green across a signal wait + an emit,
  seeded, reproducible.

## Dependencies

- BLOCKED BY: slice 01 (engine), benefits from slice 02 (await-surface
  pattern). Slice 02 and 03 are independent of each other given slice 01;
  order by learning leverage (see prioritization).
- Enables: slice 04 parent-child composition.

## Effort estimate

≤1 day. **Reference class:** adding two `ctx` surfaces (signal wait + action
emit) + ObservationStore signal row + extending the DST invariant. Comparable
to the `backend_discovery_bridge` reconciler's observation-row + action work.
