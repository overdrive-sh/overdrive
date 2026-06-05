<!-- markdownlint-disable MD013 MD024 -->
# DISTILL — Test Scenarios (GWT specification companion) — workflow-primitive

Wave: DISTILL (Quinn / nw-acceptance-designer) · Date: 2026-06-05 · Job:
J-PLAT-005 · GH #39 · roadmap [3.2]. Architecture **locked to B′** (designed
OVER, not re-litigated).

> **House-style note (`.claude/rules/testing.md` § "Testing").** This file is
> a **specification companion only — never parsed, never executed.** There are
> NO `.feature` files anywhere in this project. The executable tests are Rust
> `#[test]` / `#[tokio::test]` RED scaffolds under each owning crate's
> `tests/acceptance/*.rs` (default DST lane) or `tests/integration/*.rs` (real
> redb), wired through the inline `mod acceptance { … }` / `mod integration {
> … }` entrypoints per ADR-0005. The GIVEN/WHEN/THEN blocks below are the
> scenario SSOT; the feature-delta `## Wave: DISTILL / [REF]` sections are
> structured pointers into them.

> **Paradigm note (Tier-1 DST, not the Python state-delta paradigm).** The
> load-bearing test surface is the **named `SimInvariant`** on the CI critical
> path (K4). The "universe-bound assertion" discipline (Mandate 8) is
> satisfied natively in this Rust workspace via DST invariants whose
> `evaluate_*` body asserts exact set/byte equality over port-observable
> state — there is no `nwave_ai.state_delta` Rust port here, per the project
> ATDD Infrastructure Policy. PBT lane (Mandate 9): the `replay_equivalence_*`
> invariant IS the property (replay the journal twice → bit-identical for any
> seed); slice 03 emit/signal sad-paths are example-pinned per Mandate 11.

## Scenario ID grammar

`S-WP-<slice>-<NN>` — slice ∈ {01, 02, 03}, NN a two-digit ordinal.

## Tag legend

| Tag | Meaning |
|---|---|
| `@walking_skeleton` | The ONE slice-01 end-to-end durable + crash-resume scenario (Devon's headline journey). |
| `@driving_port` | Entered through the author surface (`impl Workflow` / `ctx.*`) or the `Action::StartWorkflow` lifecycle trigger. |
| `@dst` | A named `SimInvariant` scenario on the `cargo dst` critical path (default DST lane). |
| `@in-memory` | Runs against `Sim*` adapters in-process (default lane); no real I/O. |
| `@real-io` | Runs against a **real redb file** via the `integration-tests` feature (Tier-3 layout under `tests/integration/`). |
| `@error` | Crash / fault / ordering-failure / sad path. |
| `@property` | A quantified invariant (replay-equivalence, exactly-once, fsync-ordering) — Mandate 9 PBT-shaped at the DST layer. |
| `@kpi` | Verifies a K1–K6 outcome contract is observable / asserted. |

## Single-node honesty caveat (D3 / #205) — applies to ALL crash-resume scenarios

Every crash-resume scenario below kills the **process** and restarts on the
**SAME (single) node**, resuming from the local redb journal. **NO scenario
claims cross-node resume** — that is a multi-node / HA property (#205) the
Phase-1 single-node codebase cannot honour. The redb-journal design does not
preclude it; it is simply not demonstrated across nodes here.

---

## Slice 01 — Walking skeleton: one durable step that survives a crash

Engine + journal + replay core. First consumer `ProvisionRecord` (a real,
non-idempotent-to-repeat `ctx.call` write effect). Maps US-WP-1, US-WP-2,
US-WP-3, US-WP-4.

### US-WP-1 — Express a durable sequence as ordinary control flow → O3, O5

#### S-WP-01-01 — Author writes one ordinary async sequence and it drives to a terminal result
`@driving_port @in-memory @kpi`
- **AC**: US-WP-1 AC1 · **KPI/ODI**: K6(O3) · **Scaffold**: `overdrive-core/tests/acceptance/workflow_trait_drives_to_terminal.rs`
- GIVEN a `ProvisionRecord` sequence written as one `impl Workflow { async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult }`, with no hand-written step enum and no transition match
- WHEN the platform drives `run` to completion under DST with `Sim*` ports
- THEN the sequence reaches a terminal `WorkflowResult::Success`
- AND the author surface required exactly one ordinary `async fn` — no bespoke runtime, no step cursor in the author's body.

#### S-WP-01-02 — A durable sequence body contains zero step-machine boilerplate
`@driving_port @in-memory @property @kpi`
- **AC**: US-WP-1 AC1 (K6 metric) · **KPI/ODI**: K6(O3) · **Scaffold**: `overdrive-core/tests/acceptance/workflow_body_has_no_step_machine.rs`
- GIVEN the `ProvisionRecord` workflow impl body
- WHEN an AST/grep check counts step-enum declarations and state-transition `match` arms in the body
- THEN the count is zero (the O3 structural promise, mechanically asserted per Eclipse H1 / L1 — not free-hand review).

#### S-WP-01-03 — Every non-deterministic input flows through `ctx`, never the ambient runtime
`@driving_port @in-memory @property @error`
- **AC**: US-WP-1 AC2 · **KPI/ODI**: O5 (replay precondition) · **Scaffold**: `overdrive-core/tests/acceptance/workflow_body_routes_nondeterminism_through_ctx.rs`
- GIVEN the `ProvisionRecord` workflow body and any future first-party workflow body
- WHEN a `dst-lint`-style scan walks the workflow impl source
- THEN it finds no `Instant::now()`, no `reqwest`, no `tokio::time::sleep`, no `rand::*` — the side effect is performed through `ctx.call(...).await` only (D-INH-4)
- AND a body that smuggles a non-`ctx` non-determinism source is rejected (the failure case is asserted, not just the happy case — negative testing).

### US-WP-2 — Journal the await-point in redb so a completed step is durable → O1, O6

#### S-WP-01-04 — A completed step is recorded in the redb journal before the run suspends
`@driving_port @real-io @kpi`
- **AC**: US-WP-2 AC1 (O6), AC2 (ordering) · **KPI/ODI**: K5(O6) · **Scaffold**: `overdrive-control-plane/tests/integration/workflow_journal/journal_writes_to_redb.rs` (real redb, `integration-tests`)
- GIVEN a `ProvisionRecord` instance running against a **real** `RedbJournalStore` sharing the reconciler redb file
- WHEN `run` reaches its durable `ctx.call` await-point and records the result
- THEN the recorded `CallResult` entry is present in the redb journal when read back through the journal handle (the bytes written are the bytes read — `journal_checkpoint` consistency, journey steps 2↔3)
- AND no libSQL journal table exists (K5: grep/dep-graph clean).

#### S-WP-01-05 — The journal records step inputs/results, not a derived cache
`@in-memory @property`
- **AC**: US-WP-2 AC3 (inputs-not-derived) · **KPI/ODI**: O6 · **Scaffold**: `overdrive-sim/tests/acceptance/journal_records_inputs_not_derived.rs`
- GIVEN a `ProvisionRecord` instance recording its `ctx.call` step against `SimJournalStore`
- WHEN the recorded `JournalEntry` is inspected
- THEN it carries the step's inputs/result digest (per `development.md` "Persist inputs, not derived state")
- AND it carries no derived-deadline / "remaining" cache field.

### US-WP-3 — Resume exactly-once after a single-node crash → O1, O2 (single-node), O4

#### S-WP-01-06 — Devon kills the process mid-run and the completed step is not repeated on restart (WALKING SKELETON)
`@walking_skeleton @driving_port @dst @in-memory @error @property @kpi`
- **AC**: US-WP-3 AC1 (O1), AC2 (O4), AC3 (O2 single-node), AC4 (re-hydrate); slice-01 AC1/AC2/AC5 · **KPI/ODI**: K1(O1), K3(O4), K2(O2 single-node) · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_crash_resume_exactly_once.rs` (the `WorkflowExactlyOnceEffectOnResume` invariant)
- GIVEN a `ProvisionRecord` instance brought up by the workflow-lifecycle reconciler via `Action::StartWorkflow { spec, correlation }`, running under DST on a single node
- WHEN the instance is killed AFTER its `ctx.call` records in the redb journal but BEFORE it reaches terminal, and the process is restarted on the SAME node
- THEN the `ctx.call` external effect executes EXACTLY ONCE across the kill (SimTransport call count == 1, not 2)
- AND the resumed run reaches a `WorkflowResult` byte-identical to the uninterrupted run for the same inputs + seed
- AND after terminal the ObservationStore carries a terminal-result row keyed by the instance's `CorrelationKey`
- AND **no cross-node resume is claimed** — the kill-and-restart is process-local on one node (#205).

> This is the demo-able headline: a non-technical stakeholder reads "Devon kills the node mid-sequence, the record is not double-written, the run still finishes" and confirms "yes, that is what durable execution must do."

#### S-WP-01-07 — A committed step survives the crash (not lost) on resume
`@dst @in-memory @error @kpi`
- **AC**: US-WP-3 AC2; slice-01 AC2 · **KPI/ODI**: K2(O2 single-node) · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_committed_step_survives_crash.rs`
- GIVEN a `ProvisionRecord` instance whose `ctx.call` step has recorded in the redb journal
- WHEN the process is killed and restarted on the same node, and the journal is replayed
- THEN the recorded step's result is read back from the journal (the committed step is NOT lost)
- AND the resumed run continues from the first UNrecorded await, not from the top.

#### S-WP-01-08 — The lifecycle reconciler re-hydrates a running instance from `Action::StartWorkflow` on restart
`@driving_port @in-memory @kpi`
- **AC**: US-WP-3 AC4 · **KPI/ODI**: O2 (single-node) · **Scaffold**: `overdrive-control-plane/tests/acceptance/lifecycle_reconciler_rehydrates_on_restart.rs`
- GIVEN an instance that is `running` in intent but has no live engine task after a process restart
- WHEN the workflow-lifecycle reconciler runs its pure-sync `reconcile`
- THEN it re-emits `Action::StartWorkflow { spec, correlation }` for the instance (the engine's `load_journal` then resumes rather than cold-starts)
- AND the `reconcile` body performs no `.await`, and the `ReconcilerIsPure` DST invariant continues to hold with the workflow-lifecycle reconciler registered alongside the existing reconcilers (purity preserved).

#### S-WP-01-11 — The action-shim hands a `StartWorkflow` action to the engine off the shim, not to a reconcile loop
`@driving_port @in-memory @kpi`
- **AC**: DDD-5 / ADR-0064 §5 (engine↔reconciler boundary — the RATIFY-flagged decision); slice-01 walking-skeleton bring-up · **KPI/ODI**: O3 (two-primitive doctrine, R3) · **Scaffold**: `overdrive-control-plane/tests/acceptance/action_shim_dispatches_start_workflow_to_engine.rs`
- GIVEN the action-shim's `Action::StartWorkflow { spec, correlation }` dispatch arm (`action_shim/mod.rs:446`, today a no-op `Ok(())`)
- WHEN the shim dispatches a committed `StartWorkflow` action
- THEN it hands the instance to `WorkflowEngine::start` (the async executor driven off the shim, exactly as `Action::StartAllocation` → `Driver::start`) — the engine is NOT run as a reconciler
- AND the emitting workflow-lifecycle reconciler stays pure-sync (the upheld two-primitive doctrine: the reconciler manages WHICH instances exist; the engine manages HOW each instance's steps execute).

### US-WP-4 — Prove replay-equivalence from a seed before shipping → O4, O5

#### S-WP-01-09 — Replay-equivalence is a named DST invariant on the CI critical path, green from a seed
`@dst @in-memory @property @kpi`
- **AC**: US-WP-4 AC1 (named invariant), AC2 (replay-equivalent + bounded progress), AC3 (seed-reproducible); slice-01 AC3 · **KPI/ODI**: K4(O5) — load-bearing · **Scaffold**: `overdrive-sim/tests/acceptance/replay_equivalence_provision_record_invariant.rs` (the `ReplayEquivalenceProvisionRecord` invariant, graduating `ReplayEquivalentEmptyWorkflow`)
- GIVEN the `replay_equivalence_provision_record` `SimInvariant` exported from `overdrive-sim` (a named enum variant, no inline string literal)
- WHEN `cargo dst --only replay_equivalence_provision_record` runs
- THEN replaying the journal twice produces a bit-identical trajectory (`assert_replay_equivalent!`)
- AND a paired `assert_eventually!(is_terminal)` proves the run reaches terminal within the declared step budget (bounded progress)
- AND the run prints a seed and reproduces bit-for-bit on a second run on the same SHA + toolchain (`dst_seed` consistency, journey steps 3↔4).

#### S-WP-01-10 — The journal write does not advance the cursor when the fsync fails (write-ordering)
`@dst @in-memory @error @property @kpi`
- **AC**: US-WP-2 AC2 (durability ordering); slice-01 AC2 · **KPI/ODI**: O1/O6 (durability) · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_journal_write_ordering.rs` (the `WorkflowJournalWriteOrdering` invariant)
- GIVEN a `ProvisionRecord` instance and a `SimJournalStore` configured to fail the fsync on the next `append`
- WHEN the engine attempts to record an await result and the fsync fails
- THEN the engine does NOT advance the journal cursor and does NOT suspend with an unrecorded step acknowledged (mirrors ADR-0035 `WriteThroughOrdering`)
- AND on the next boot the journal carries no phantom half-written entry (fsync-then-suspend is load-bearing).

---

## Slice 02 — Durable `ctx.sleep` across a crash

Adds `ctx.sleep(Duration)` through the injected `Clock`. Consumer extended to a
`ctx.call → ctx.sleep → ctx.call` 3-await shape. Maps US-WP-1/3/4 across a sleep.

#### S-WP-02-01 — A waiting sequence survives a crash spanning the sleep window without repeating the pre-sleep step
`@driving_port @dst @in-memory @error @property @kpi`
- **AC**: slice-02 AC1 (O1) · **KPI/ODI**: K1(O1) · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_sleep_crash_pre_sleep_step_not_repeated.rs`
- GIVEN a `ctx.call → ctx.sleep → ctx.call` sequence running under DST
- WHEN the process is killed DURING the sleep window and restarted on the same node
- THEN the pre-sleep `ctx.call` executes exactly once on resume (SimTransport call count == 1)
- AND the sequence resumes the remaining wait, not the whole sleep.

#### S-WP-02-02 — The post-sleep step fires only at/after the original deadline, regardless of crash timing
`@dst @in-memory @error @property @kpi`
- **AC**: slice-02 AC2 (O4) · **KPI/ODI**: K3(O4) · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_sleep_resumes_to_original_deadline.rs`
- GIVEN a sequence suspended on `ctx.sleep` with a recorded deadline
- WHEN the crash occurs at an arbitrary point in the sleep window and the run resumes (SimClock advances logical time)
- THEN the post-sleep `ctx.call` fires only at/after the ORIGINAL deadline, never earlier
- AND the terminal result is unchanged by the crash timing.

#### S-WP-02-03 — The sleep journal entry records the deadline (an input), never a "remaining" cache
`@in-memory @property`
- **AC**: slice-02 AC4 (inputs-not-derived) · **KPI/ODI**: O3/O6 · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_sleep_records_deadline_not_remaining.rs`
- GIVEN a sequence that has armed a `ctx.sleep`
- WHEN the `SleepArmed` journal entry is inspected
- THEN it carries the deadline (`deadline_unix`, an input)
- AND it carries no persisted "remaining duration" field — resume recomputes `recorded_deadline − clock.now()` (`development.md` "Persist inputs, not derived state").

#### S-WP-02-04 — Replay-equivalence holds across the sleep, seeded and reproducible
`@dst @in-memory @property @kpi`
- **AC**: slice-02 AC3 (O5) · **KPI/ODI**: K4(O5) · **Scaffold**: `overdrive-sim/tests/acceptance/replay_equivalence_holds_across_sleep.rs`
- GIVEN the `replay_equivalence_*` invariant extended over the 3-await `ctx.call → ctx.sleep → ctx.call` shape
- WHEN `cargo dst` runs the invariant
- THEN replaying the journal across the sleep produces a bit-identical trajectory, green on the CI critical path, reproducing bit-for-bit from the printed seed.

---

## Slice 03 — Typed signals + workflow→cluster Action emission

Adds `ctx.wait_for_signal(SignalKey)` (typed ObservationStore signal) and
`ctx.emit_action(Action)` (Raft channel, no IntentStore bypass). Maps US-WP-5.

### US-WP-5 — Coordinate via typed signals + emit cluster mutations through Raft → O1, O3, O4, O5

#### S-WP-03-01 — A sequence blocked on a signal re-blocks on the SAME signal after a crash
`@driving_port @dst @in-memory @error @property @kpi`
- **AC**: US-WP-5 AC1 (O1); slice-03 AC1 · **KPI/ODI**: K1(O1) · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_signal_wait_reblocks_after_crash.rs`
- GIVEN a sequence blocked on `ctx.wait_for_signal(key)` under DST with the signal NOT yet present in the ObservationStore
- WHEN the process is killed while blocked and restarted on the same node
- THEN on resume the workflow blocks on the SAME signal (the wait is neither lost nor satisfied prematurely)
- AND no duplicate downstream effect occurs.

#### S-WP-03-02 — A satisfied signal is not re-waited on resume
`@dst @in-memory @error @property`
- **AC**: slice-03 AC1 (resume re-checks satisfaction) · **KPI/ODI**: O1 · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_signal_already_seen_not_rewaited.rs`
- GIVEN a sequence that recorded `SignalSeen` for `key` before the crash
- WHEN the process is killed after the signal was seen and restarted on the same node
- THEN on resume the workflow does NOT re-block on `key` — it reads the recorded signal value and proceeds (check-then-record on replay).

#### S-WP-03-03 — `ctx.emit_action` lands the typed Action in the Raft channel with no direct IntentStore write
`@driving_port @in-memory @property @kpi`
- **AC**: US-WP-5 AC2 (no Raft bypass); slice-03 AC2 · **KPI/ODI**: O3 · **Scaffold**: `overdrive-control-plane/tests/acceptance/workflow_emit_action_lands_in_raft_channel.rs`
- GIVEN a sequence that calls `ctx.emit_action(action)`
- WHEN the action is emitted
- THEN the typed Action lands in the Action channel the reconciler runtime consumes (→ Raft / Phase-1 IntentStore commit path)
- AND the workflow performs NO direct IntentStore write (`development.md` Workflow contract rule 6 — the universe is "Action-channel arrivals" + "IntentStore writes by the workflow"; the latter must be empty).

#### S-WP-03-04 — An emitted Action is not re-emitted after a crash (idempotent emit)
`@dst @in-memory @error @property @kpi`
- **AC**: US-WP-5 AC3 (idempotent emit); slice-03 AC3 · **KPI/ODI**: K1(O1) · **Scaffold**: `overdrive-sim/tests/acceptance/workflow_emit_action_not_re_emitted_after_crash.rs`
- GIVEN a sequence that recorded `ActionEmitted` for a `ctx.emit_action` before reaching terminal
- WHEN the process is killed after the emit records but before terminal, and restarted on the same node
- THEN the Action is NOT re-emitted on resume (the `ActionEmitted` journal entry makes the emit idempotent) — exactly one cluster mutation across the crash.

#### S-WP-03-05 — Replay-equivalence holds across a signal wait and an emit, seeded and reproducible
`@dst @in-memory @property @kpi`
- **AC**: US-WP-5 AC4 (O5); slice-03 AC4 · **KPI/ODI**: K4(O5) · **Scaffold**: `overdrive-sim/tests/acceptance/replay_equivalence_holds_across_signal_and_emit.rs`
- GIVEN the `replay_equivalence_*` invariant extended over a `ctx.wait_for_signal → ctx.emit_action → terminal` shape
- WHEN `cargo dst` runs the invariant
- THEN replaying the journal across the signal wait + emit produces a bit-identical trajectory, green on the CI critical path, reproducing bit-for-bit from the printed seed.

---

## Cross-scenario consistency assertions (journey `integration_validation`)

These are not standalone scenarios — they are invariants the scaffolds above
jointly assert, mirroring the journey YAML's `shared_artifact_consistency`:

- **`journal_checkpoint` bytes written == bytes read** (journey steps 2↔3): asserted by S-WP-01-04 (write path, real redb) + S-WP-01-07 (read-back-on-resume). Any drift means a completed step is lost or re-derived — the O1/O2 core fails.
- **`dst_seed` printed → reproduces bit-for-bit** (journey steps 3↔4): asserted by S-WP-01-09 / S-WP-02-04 / S-WP-03-05 (every `replay_equivalence_*` run prints a seed and reproduces). Matches the `trust-the-sim` discipline.
- **`correlation_key` linkage** (journey step 1): asserted by S-WP-01-06 — the `CorrelationKey` on `Action::StartWorkflow` is the SAME key the ObservationStore terminal-result row is filed under, so the emitting reconciler finds the result deterministically (`development.md` Reconciler I/O rule 2).

## Error / edge-path coverage tally

| Slice | Total scenarios | Error/edge (`@error`) |
|---|---|---|
| 01 | 11 | 4 (S-WP-01-03, 01-06, 01-07, 01-10) — and 01-06 is the headline error path |
| 02 | 4 | 2 (S-WP-02-01, 02-02) |
| 03 | 5 | 4 (S-WP-03-01, 03-02, 03-04 + the crash legs of 03-05) |
| **Total** | **20** | **9 distinct `@error` scenarios + crash legs in property scenarios ≈ 45%** |

≥40% error/edge coverage met (the crash/fsync-failure/ordering-failure/
re-block/idempotent-emit paths are the headline of this feature, not
afterthoughts).
