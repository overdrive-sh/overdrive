# RCA — Engine re-drives completed workflow when journal already holds a `Terminal`

**Status**: user-reviewed and APPROVED (Phase 2, `/nw-bugfix`). Approved fix = **Option 1** (store the full `WorkflowResult` in the journal `Terminal`, add a terminal short-circuit guard at the top of `WorkflowEngine::start`).

**Defect site**: `crates/overdrive-control-plane/src/workflow_runtime/mod.rs` — `WorkflowEngine::start` (~292-449) and its spawned task body.

---

## Symptom

Under a persistent terminal **observation-store write failure**, the engine re-executes a *completed* workflow's author body on every reconciler tick and appends a fresh `JournalCommand::Terminal` each time. The journal has no GC, so `Terminal` entries accumulate **unboundedly**, and the author body is needlessly re-run instead of short-circuiting.

## Two compounding root causes (both required)

### A — No terminal short-circuit in `start` (primary)

- `run_has_started` (`mod.rs:530-532`) suppresses a duplicate `Started` (any-command-present check) but there is **no** guard against a pre-existing `Terminal`.
- `JournalStore::append` is append-only with no dedup (`journal/mod.rs:503-545`): each re-drive lands a new `Terminal` at the next slot.
- The spawn at `mod.rs:371` proceeds on every `start`; the body replays deterministically to the same terminal and appends `Terminal` again at `mod.rs:407-410`.

### B — Non-fatal obs-write failure drives infinite re-emit

- The obs `WorkflowTerminal` row is an **in-memory** convergence signal, never persisted to redb (`overdrive-store-local/src/observation_backend.rs:387-405`). On write failure it is simply lost.
- The obs write failure is logged non-fatal (`mod.rs:427-434`), but `_teardown` (`LiveInstanceGuard::drop`, `mod.rs:483-486`) still removes the instance from `live_instances`.
- Next tick: `hydrate_workflow_actual_instances` reads `has_live_task=false` (teardown ran) and `terminal=None` (write lost); `WorkflowLifecycle::reconcile` (`overdrive-core/src/reconcilers/workflow_lifecycle.rs:150-167`) hits `running_in_intent && !has_live_task && terminal.is_none()` and re-emits `Action::StartWorkflow`.
- The comment at `mod.rs:435-445` assumes "the next resume re-writes idempotently" — true for the obs row (keyed by `CorrelationKey`) but **false for the journal `Terminal` append**, which is not idempotent. That conflation is the bug.

Either alone is insufficient: without B the engine is never re-driven after success; without A a re-drive would short-circuit. Both are real; one coordinated fix closes both.

## The label-is-lossy obstacle (why Option 1)

`JournalCommand::Terminal` stores only `result: String` — a stable **label** via `workflow_result_label` (`mod.rs:538`) that folds `Failed { reason }` to `"Failed"`, discarding `reason`. There is no inverse mapping. So "skip the spawn and write the obs row directly" needs a full `WorkflowResult`, which the label cannot reconstruct losslessly.

The reconciler only branches on `terminal.is_some()` (never the variant/reason), so a lossy reconstruction would converge correctly — but it would permanently drop the `Failed` reason on the skip-path obs row and violate persist-inputs-not-derived-state in spirit. The slice-01 code already flagged the label as temporary ("the engine maps these back … in later slices", `mod.rs:536`).

**Approved decision — Option 1**: make the durable terminal carry the full value. Change `JournalCommand::Terminal { result: String }` → `{ result: WorkflowResult }`, derive `Serialize`/`Deserialize` on `WorkflowResult`, delete `workflow_result_label`, and add a `start`-time short-circuit that re-publishes the obs row losslessly from the journal `Terminal`. Greenfield single-cut (delete on-disk journal; no envelope/migration).

## The fix

At the top of `start`, after `load_journal`: if `replay_buffer` contains a `JournalCommand::Terminal { result }`, **short-circuit** —
- do NOT write `Started`, build the cursor, spawn the body, or insert into `live_instances`;
- re-publish `ObservationRow::WorkflowTerminal { correlation, result }` from the journal's full `WorkflowResult` (idempotent under `CorrelationKey`; non-fatal on failure, retried next tick);
- return `Ok(())`.

The skip-path appends **no** journal entry, so the journal halts at exactly one `Terminal`; only the cheap idempotent obs re-write retries each tick. Correct steady state.

## Blast radius (wider than first estimated — same fix shape)

The `String → WorkflowResult` type change on `JournalCommand::Terminal` ripples beyond the 3 core files:

Production:
- `overdrive-control-plane/src/journal/mod.rs:320-324` — field type + docstring + import.
- `overdrive-control-plane/src/workflow_runtime/mod.rs` — build `Terminal` with full result (`:407`); delete `workflow_result_label` (`:538`); add guard + `terminal_result` helper at top of `start`.
- `overdrive-core/src/workflow/mod.rs:82` — add `Serialize, Deserialize` to `WorkflowResult`.
- `overdrive-sim/src/adapters/journal.rs:188` (terminal sentinel) + `:259` (proptest `Arbitrary` generator → generate `WorkflowResult`, not arbitrary string).
- `overdrive-sim/src/invariants/evaluators.rs:3898` (`Terminal { result: ... }` construction); `:1574` / `mod.rs:572` label-match arms use `{ .. }` and are unaffected.

Tests:
- DELETE `overdrive-control-plane/tests/acceptance/workflow_engine_terminal_labels.rs` (tests the removed label — deletion discipline) + remove its `mod` decl and the stale comment in `tests/acceptance.rs:263-266`.
- `overdrive-control-plane/tests/acceptance/action_shim_dispatches_start_workflow_to_engine.rs:154` (`result == "Success"` → `WorkflowResult::Success`).
- `overdrive-sim/tests/acceptance/journal_records_inputs_not_derived.rs:151` (`"Completed"` string → a `WorkflowResult`).
- `matches!(.., Terminal { .. })` sites across the sim acceptance suite use `{ .. }` and need no change.

## Risk

- **`live_instances` semantics**: the skip-path must NOT insert into `live_instances` (the instance is terminal, not live). `reconcile` checks `terminal.is_some()` first, so convergence does not depend on `has_live_task`. No live-entry leak (nothing inserted).
- **Ordering invariants**: terminal-then-remove (ADR-0064 §5) and fsync-then-suspend (ADR-0066 §4) both concern the *spawn* path; the skip-path has no live entry and no journal append, so neither hazard arises. Its only durable interaction is the idempotent obs write, carrying the same non-fatal discipline.
- **Determinism gate**: skip-path builds no cursor → replay layers not exercised (correct; no replay).
- **Schema change**: `Terminal { result: WorkflowResult }` needs serde on `WorkflowResult`; greenfield single-cut, no migration. A CBOR round-trip test for `Failed { reason }` covers the new payload.
- **`start` contract**: already returns `Ok(())` once *spawned*, not completed; returning `Ok(())` on the skip-path (no task) is consistent — `join_all` sees no task.

## Regression test (driving port = `JournalStore::load_journal`)

Drive a workflow to terminal with `SimObservationStore::inject_write_failure` queued for the terminal write; restart `start` over the **same** journal (models the reconciler re-emit); assert exactly **one** `Terminal` command.

- With the bug: first start appends `Terminal` #1 (obs write fails); resume has no guard → spawns → replays → appends `Terminal` #2 → `count == 2` → FAIL.
- With the fix: resume detects the `Terminal`, short-circuits, re-writes the obs row (fails again, non-fatal), appends nothing → `count == 1` → PASS.

Companion (strengthens "body not re-run"): a `SimTransport`/spy call counter on the workflow body asserts the body ran exactly **once** across both starts.
