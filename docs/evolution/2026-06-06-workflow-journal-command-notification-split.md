# Evolution — workflow-journal-command-notification-split

**Finalized:** 2026-06-06
**Wave arc:** DESIGN (GUIDE) → DELIVER (6 steps) → FINALIZE
**Parent primitive:** `Workflow` durable-execution (GH #39, roadmap [3.2])
**Authoritative design records:** [ADR-0063](../product/architecture/adr-0063-workflow-journal-redb-second-table-layout.md),
[ADR-0064](../product/architecture/adr-0064-workflow-trait-ctx-and-engine-reconciler-boundary.md)
**Feature delta (preserved):** `docs/feature/workflow-journal-command-notification-split/feature-delta.md`

---

## Summary

Typed the Workflow journal stream to close a latent replay-corruption
trap. The single 7-variant `JournalEntry` enum became two semantically
distinct enums plus a boundary sum — `JournalCommand` (the replayable,
cursor-advancing class), `JournalNotification` (the `SignalKey`-correlated
class, off the positional walk), and `LoadedEntry = Command | Notification`
(the on-disk/append/load representation). The replay cursor now partitions
the flat loaded run **once** at construction into a `Vec<JournalCommand>`
positional walk plus a `BTreeMap<SignalKey, JournalNotification>` correlated
lookup, retiring the `*cursor += 2` two-positional-entry signal walk. The
engine now writes `Started` as a real command-index-0 entry, a fail-closed
determinism gate (Layers 1+2) replaces the former silent fall-through to
the live path, and the `replay_equivalence_provision_record` DST invariant
was extended to guard the new behaviour on the CI critical path.

**Greenfield single-cut:** slices 01/02/03 of the Workflow primitive were
unlanded, so there were no surviving on-disk journals — no
`#[serde(tag="v")]` envelope bump, no migration path. EXTEND-dominant: 8
EXTEND, 1 CREATE NEW (`LoadedEntry`, a thin boundary sum), zero new
components.

## Business / technical context

The `Workflow` primitive is Overdrive's §18 peer to reconcilers — the
durable-execution surface for terminal, multi-step orchestrations (cert
rotation, region migration). Its correctness rests on **bit-identical
journal replay**: a workflow resumed from its journal must reproduce the
same trajectory it would have produced uninterrupted (the
`assert_replay_equivalent!` property).

### The trap (one line)

`Started` was documented as the journal's first entry, but
`WorkflowEngine::start` never wrote it — and the positional replay cursor
could not consume a non-`await` entry at a walked position. Two
consequences compounded:

1. `Started` / `Terminal` were **second-class** — present in the type but
   never legitimately walked.
2. A variant mismatch at the cursor **silently fell through to the live
   path**, re-executing an effect that the journal had already recorded —
   the exact failure replay-equivalence exists to prevent.

The fix types the journal stream (Restate journal-v2 evidence,
`docs/research/workflow/restate-journal-replay-model.md`) so the cursor
advances only over replayable commands, `Started` / `Terminal` become
legitimate typed commands, and notifications are resolved by `SignalKey`
off the walk.

## Key decisions (D1–D6, LOCKED in DESIGN GUIDE)

| ID | Decision | Record |
|---|---|---|
| **D1** | Taxonomy = two typed enums (`JournalCommand` / `JournalNotification`) + boundary sum `LoadedEntry`; no envelope bump. "Make invalid states unrepresentable" — a notification cannot enter the command walk by type. | ADR-0063 §2 / CA-1 |
| **D2** | Partition **at the cursor**; the store stays a dumb ordered log. `load_journal` returns flat `Vec<LoadedEntry>`; `JournalCursorHandle` partitions once at construction. Retires `*cursor += 2`. | ADR-0064 §3 / CA-5 |
| **D3** | Derived command-index. redb key stays `(WorkflowId, u32)` = storage append-position over **all** entries; `next_step` count-all UNCHANGED; command-index derived at the cursor. `Started` = command-index 0. Conflating append-position with cursor-consumption-position **is** the trap. | ADR-0063 §3 / CA-3 |
| **D4** | Determinism gate Layers 1+2, fail-closed. Layer 1 (type-at-index, Restate RT0016 shape) + Layer 2 (name within `RunResult`) → `WorkflowCtxError::NonDeterministic { expected, actual }`, no advance, no fall-through. Layer 3 (content/digest) deferred → #214. | ADR-0064 §3 / CA-6 |
| **D5** | Drop the in-entry `step: u32`. Identity is structural (position in `Vec<JournalCommand>`; `SignalKey` for notifications); a persisted `step` is a cache of "my own position" (`development.md` § "Persist inputs, not derived state"). DELIVER verified zero per-entry `step` readers. | ADR-0063 §2 / CA-2 |
| **D6** | Minimal notification model — **only** `BTreeMap<SignalKey, JournalNotification>`, no general `NotificationId` correlation model (rejected, not deferred). Extend `replay_equivalence_provision_record` (verbatim name) with the `Started`-at-0 + notification-not-as-command cursor-advance guard. | ADR-0064 §6 / CA-7 |
| **CA-4** | `Started` becomes a real engine-written command-index-0 entry — the trap itself. | ADR-0063 §2 / ADR-0064 §5 |

## Steps completed

All six steps RED → GREEN → COMMIT (single commit each), compiled and
tested green via Lima. Mutation gate run once after 01-06.

| Step | Title | Commit |
|---|---|---|
| 01-01 | Split `JournalEntry` → `JournalCommand` + `JournalNotification` + `LoadedEntry`; drop in-entry `step:u32` (D1/D5) | `88324bf5` `feat!(workflow-journal)` |
| 01-02 | Store deals in `LoadedEntry`; `next_step` count-all unchanged; dumb-ordered-log contract pinned (D2/D3) | `5d1f7ff2` `refactor(workflow-journal)` |
| 01-03 | Partition at the cursor: `Vec<JournalCommand>` walk + `BTreeMap<SignalKey,JournalNotification>` lookup; retire `*cursor += 2` (D2/D6) | `7ed6c42f` `refactor(workflow-cursor)` |
| 01-04 | Determinism gate Layers 1+2 fail-closed → `NonDeterministic` (D4) | `5f8f7716` `feat(workflow-cursor)` |
| 01-05 | `WorkflowEngine::start` writes `Started` at command-index 0 on first start, idempotent on resume (CA-4) | `0dae0998` `feat(workflow-engine)` |
| 01-06 | Extend `replay_equivalence_provision_record` with the Started-at-0 + notification-not-as-command guard; verify no step reader (D5/D6) | `c698b650` `test(workflow-dst)` |

Post-step quality passes landed on top of the chain:

- `36cecf1c` `refactor(workflow-journal)` — L1–L6 cleanup of the
  command/notification split (no behaviour change).
- `772061ee` `test(workflow-journal)` — close diff-scoped mutation gaps in
  journal/cursor/engine (kill-rate gate).

## Lessons learned

- **Outside-In dependency chains produce honest mid-chain RED.** The
  enum split (01-01) intentionally left downstream call sites
  non-compiling until 01-05 closed the chain end-to-end. The step
  boundaries were drawn along *verification-gate* lines (each step lands
  one observable CI signal) so the incomplete intermediate states stayed
  discoverable rather than hidden.
- **A step that lands a behaviour change can diverge a sibling's
  harness — respect the `files_to_modify` boundary.** Step 01-05's first
  GREEN attempt was correct for the engine (control-plane + acceptance
  green) but turned 7 sim invariant tests red: the engine's new
  `Started`-at-0 write diverged the replay-equivalence oracle from the
  hand-built crash-run trajectories in `run_until_crash` and siblings,
  which bypass `engine.start` and never wrote `Started`. Adapting that
  harness was 01-06's *explicit* scope. The crafter correctly refused to
  commit a red workspace **and** refused to reach across the step
  boundary into the 01-06-owned file — it re-scoped 01-05's GREEN to the
  engine-only surface and let 01-06 land the harness extension. The DES
  log captured this as a `COMMIT SKIPPED: BLOCKED_BY_DEPENDENCY` event
  followed by a clean GREEN/COMMIT once the engine-only scope was
  isolated. The boundary held; no out-of-bounds edit, no red commit.
- **"Persist inputs, not derived state" applies to a journal `step`
  field.** Dropping `step:u32` (D5) was not a size optimisation — a
  persisted `step` is a cache of "my own position," and the store
  already derives append-position via count-all (`next_step`) while the
  cursor derives the command-index from partition position. DELIVER's
  in-scope verification confirmed zero per-entry `step` readers remained.
- **Extend the existing DST invariant, do not fork a new family.** The
  D6 guard had to land *inside* `replay_equivalence_provision_record`
  (verbatim name) — a new invariant family would not sit on the existing
  `cargo dst` critical path and would not be the regression the trap
  demands. The structural guard now fails if a future change drops the
  `Started` write or lets a `SignalSeen` notification enter the
  positional command walk.

## Issues / deferrals (verified, cite-by-number — nothing created)

- **HA cross-node resume** — [#205](https://github.com/overdrive-sh/overdrive/issues/205)
  (verified open). The typed split is node-independent: the `LoadedEntry`
  log is `WorkflowId`-keyed CBOR behind a `JournalStore` trait, and the
  partition is a cursor concern, not a store concern — so a future HA
  adapter re-implements the log without re-deriving replay semantics.
  Not precluded by this work.
- **Determinism Layer 3 (content/digest comparison)** —
  [#214](https://github.com/overdrive-sh/overdrive/issues/214) (already
  created). The gate shipped Layers 1+2 (type-at-index + name); Layer 3 is
  out of scope and cited at the code boundary only.

No general `NotificationId` deferral language: D6 **rejected** the general
correlation model rather than deferring it (single-node Phase-1 has exactly
one notification shape).

## Migrated / permanent artifacts

- **ADR-0063** (`docs/product/architecture/adr-0063-workflow-journal-redb-second-table-layout.md`)
  and **ADR-0064**
  (`docs/product/architecture/adr-0064-workflow-trait-ctx-and-engine-reconciler-boundary.md`)
  — already in their permanent location; authored/amended through the
  architect agent, not migrated here.
- **Feature delta** — `docs/feature/workflow-journal-command-notification-split/feature-delta.md`,
  preserved in the feature workspace (the wave matrix derives status from
  that directory). It is the SSOT for the locked D1–D6 GUIDE answers, the
  component decomposition, the reuse analysis, and the C4 component
  diagram.
- **Implementation** — landed across `overdrive-control-plane`
  (`journal/mod.rs`, `journal/redb.rs`, `workflow_runtime/mod.rs`),
  `overdrive-core` (`workflow/mod.rs`), and `overdrive-sim`
  (`adapters/journal.rs`, `invariants/evaluators.rs`) per the eight
  commits above.

No `docs/architecture/`, `docs/scenarios/`, or `docs/ux/` artifacts were
produced by this feature (it is an internal reshape over an existing
primitive, designed via a lean feature-delta rather than the full
DISCUSS/DISTILL/DESIGN artifact set).
