# Slice 02 — Durable `ctx.sleep` across a crash

**Goal (one sentence):** Add `ctx.sleep(duration).await` as a durable
await-point so a sequence that must wait (e.g. DNS propagation in cert
rotation) survives a crash spanning the sleep window — restart resumes the
remaining wait, not the whole sleep, and never re-runs the pre-sleep step.

**Job / outcomes:** J-PLAT-005 → O1 (pre-sleep step not repeated), O3 (waiting
is `ctx.sleep`, not a hand-rolled timer in View), O4 (terminal unchanged by
crash timing), O5 (replay-equivalence holds across the sleep).

## IN scope

- `ctx.sleep(Duration)` routed through the injected `Clock` (DST-controllable
  — SimClock parks on a deadline, harness advances logical time).
- A sleep-checkpoint journal entry recording the sleep DEADLINE (an input),
  per development.md "Persist inputs, not derived state": resume recomputes
  remaining wait from `recorded_deadline - clock.now()`, it does not persist
  "remaining ms."
- Extend the skeleton consumer with a `ctx.call → ctx.sleep → ctx.call`
  3-await shape.
- Extend the DST invariant: kill DURING the sleep window, restart, assert the
  pre-sleep `ctx.call` executed once and the post-sleep call fires only after
  the original deadline.

## OUT of scope

- Signals / activities / composition (slice 03).
- Cross-node resume (#205; multi-node dependency).
- Wall-clock fidelity in production (DST drives logical time; production uses
  SystemClock — same `Clock` trait, no extra surface).

## Learning hypothesis

- **Disproves if it fails:** "the `Clock`-injected durable sleep parks
  correctly under DST and recomputes remaining wait from a persisted deadline
  input." If resume re-runs the full sleep or re-fires the pre-sleep step, the
  await-point journaling model has a hole the skeleton's single-await test
  could not surface.
- **Confirms if it succeeds:** multi-await sequences with time-based
  suspension are sound; cert-rotation's DNS-propagation wait is expressible.

## Acceptance criteria (production data, not synthetic)

- AC1 (O1): Crash during the sleep window → pre-sleep `ctx.call` executes
  exactly once on resume (SimTransport call count).
- AC2 (O4): Post-sleep `ctx.call` fires only at/after the ORIGINAL deadline,
  regardless of when the crash occurred (asserted via SimClock).
- AC3 (O5): `replay_equivalence_*` invariant green across the sleep, seeded,
  reproducible.
- AC4 (inputs-not-derived): the journal records the sleep deadline (input),
  asserted; no persisted "remaining duration" field.

## Dependencies

- BLOCKED BY: slice 01 (engine + journal + replay).
- Enables: a cert-rotation-shaped consumer (needs the propagation wait).

## Effort estimate

≤1 day. **Reference class:** adding one `ctx` await variant + extending an
existing named DST invariant — smaller than slice 01.
