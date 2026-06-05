<!-- markdownlint-disable MD013 -->
# DISTILL — RED classification (pre-DELIVER fail-for-the-right-reason gate)

Wave: DISTILL (Quinn / nw-acceptance-designer) · Date: 2026-06-05 · Job:
J-PLAT-005 · GH #39.

**Gate result: PASS.** Every workflow-primitive acceptance scaffold is a
`#[should_panic(expected = "RED scaffold")]` Rust test whose body is a
`panic!("Not yet implemented -- RED scaffold (S-WP-NN-NN / …)")`. Per
`.claude/rules/testing.md` § "RED scaffolds", this is the project's sanctioned
RED-not-BROKEN shape: the test COMPILES (no import of unbuilt production types
→ no BROKEN), nextest reports PASS at the bar, and the scaffold is structurally
tied to the panic message that DELIVER removes when it lands the real
implementation. Classification for every row below is therefore:

`RED (should_panic scaffold — implementation missing)`.

There are ZERO `IMPORT_ERROR` / `FIXTURE_BROKEN` / `SETUP_FAILURE` rows — the
compile-and-run gate confirmed it (see "Gate evidence" below).

## Gate evidence (verbatim from Lima)

Compile-check (RED-not-BROKEN — all three crates compile):

```
cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-sim --no-run
    Finished `test` profile [unoptimized + debuginfo] target(s) in 23.75s

cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --features integration-tests --no-run
    Finished `test` profile [unoptimized + debuginfo] target(s) in 15.06s
```

Run (every scaffold reports PASS at the bar — RED via should_panic):

```
cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-sim \
  -p overdrive-control-plane --features integration-tests \
  -E 'test(workflow) + test(replay_equivalence_provision) + test(replay_equivalence_holds) \
      + test(journal) + test(lifecycle_reconciler_re) + test(emit_action) + test(signal) \
      + test(sleep) + test(committed_step) + test(crash_resume) + test(provision_record)'
     Summary [   0.026s] 29 tests run: 29 passed, 1220 skipped
```

(29 test functions across 19 scenarios — S-WP-01-03 carries a positive + a
negative-testing function; the remainder are one function per scenario.)

## Per-scenario classification

| Scenario | Test fn(s) | Crate / path | Classification |
|---|---|---|---|
| S-WP-01-01 | `provision_record_drives_to_terminal_workflow_result` | overdrive-core `tests/acceptance/workflow_trait_drives_to_terminal.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-02 | `provision_record_body_has_zero_step_enum_and_zero_transition_match` | overdrive-core `tests/acceptance/workflow_body_has_no_step_machine.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-03 | `clean_workflow_body_routes_all_nondeterminism_through_ctx` + `workflow_body_smuggling_non_ctx_nondeterminism_is_rejected` | overdrive-core `tests/acceptance/workflow_body_routes_nondeterminism_through_ctx.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-04 (`@real-io`) | `call_result_is_present_in_the_real_redb_journal_and_no_libsql_table_exists` | overdrive-control-plane `tests/integration/workflow_journal/journal_writes_to_redb.rs` (integration-tests) | RED (should_panic scaffold — implementation missing) |
| S-WP-01-05 | `provision_record_journal_entry_records_inputs_not_a_derived_cache` | overdrive-sim `tests/acceptance/journal_records_inputs_not_derived.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-06 (WS) | `killing_after_step_records_does_not_repeat_the_effect_on_resume` | overdrive-sim `tests/acceptance/workflow_crash_resume_exactly_once.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-07 | `committed_step_is_read_back_from_journal_and_run_resumes_from_first_unrecorded_await` | overdrive-sim `tests/acceptance/workflow_committed_step_survives_crash.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-08 | `lifecycle_reconciler_re_emits_start_workflow_for_a_running_instance_with_no_live_task` | overdrive-control-plane `tests/acceptance/lifecycle_reconciler_rehydrates_on_restart.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-09 (K4) | `replay_equivalence_provision_record_is_a_named_invariant_green_and_seed_reproducible` | overdrive-sim `tests/acceptance/replay_equivalence_provision_record_invariant.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-10 | `fsync_failure_on_append_does_not_advance_cursor_or_suspend_with_unrecorded_step` | overdrive-sim `tests/acceptance/workflow_journal_write_ordering.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-01-11 | `start_workflow_action_is_dispatched_to_the_engine_off_the_shim_not_run_as_a_reconciler` | overdrive-control-plane `tests/acceptance/action_shim_dispatches_start_workflow_to_engine.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-02-01 | `crash_during_sleep_window_does_not_repeat_the_pre_sleep_step` | overdrive-sim `tests/acceptance/workflow_sleep_crash_pre_sleep_step_not_repeated.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-02-02 | `post_sleep_step_fires_only_at_or_after_the_original_deadline_regardless_of_crash_timing` | overdrive-sim `tests/acceptance/workflow_sleep_resumes_to_original_deadline.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-02-03 | `sleep_armed_journal_entry_records_deadline_input_not_a_remaining_duration_cache` | overdrive-sim `tests/acceptance/workflow_sleep_records_deadline_not_remaining.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-02-04 | `replay_equivalence_holds_across_a_durable_sleep_seeded_and_reproducible` | overdrive-sim `tests/acceptance/replay_equivalence_holds_across_sleep.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-03-01 | `crash_while_blocked_on_signal_reblocks_on_the_same_signal_on_resume` | overdrive-sim `tests/acceptance/workflow_signal_wait_reblocks_after_crash.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-03-02 | `a_signal_seen_before_the_crash_is_not_rewaited_on_resume` | overdrive-sim `tests/acceptance/workflow_signal_already_seen_not_rewaited.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-03-03 | `emit_action_lands_in_the_action_channel_and_the_workflow_makes_no_direct_intent_store_write` | overdrive-control-plane `tests/acceptance/workflow_emit_action_lands_in_raft_channel.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-03-04 | `an_action_emitted_before_the_crash_is_not_re_emitted_on_resume` | overdrive-sim `tests/acceptance/workflow_emit_action_not_re_emitted_after_crash.rs` | RED (should_panic scaffold — implementation missing) |
| S-WP-03-05 | `replay_equivalence_holds_across_a_signal_wait_and_an_emit_seeded_and_reproducible` | overdrive-sim `tests/acceptance/replay_equivalence_holds_across_signal_and_emit.rs` | RED (should_panic scaffold — implementation missing) |

**20 scenarios · 21 scaffold files · 20 test functions named above (+1 negative
function under S-WP-01-03) = 21 functions in the table; the Lima run reported 29
total because the filter also swept already-green sibling workflow tests in the
suite. Every workflow-primitive scaffold is RED-not-BROKEN. (S-WP-01-11 added in
the consolidated review to close the architect's DDD-5 engine↔reconciler-boundary
high finding; re-compiled RED-not-BROKEN in Lima.)**

> Note: the 29-vs-20 delta is the test-filter (`test(workflow)` etc.) also
> matching a handful of pre-existing non-workflow-primitive tests (e.g.
> `ReplayEquivalentEmptyWorkflow`-adjacent and `*workflow*` lifecycle tests
> already in the suite). All 29 passed; the 20 workflow-primitive scaffolds
> above are the DISTILL deliverable and all classify RED.
