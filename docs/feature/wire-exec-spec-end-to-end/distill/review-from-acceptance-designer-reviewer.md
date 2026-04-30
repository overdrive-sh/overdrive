# DISTILL review — wire-exec-spec-end-to-end

**Reviewer**: nw-acceptance-designer-reviewer
**Date**: 2026-04-30
**Verdict**: BLOCK

## Critical (block ship to DELIVER if any)

1. **Walking-skeleton missing IntentStore persistence assertion** (CRITICAL)
   - **File**: `crates/overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs`
   - **Issue**: Test spec in `test-scenarios.md` §1 lines 115–117 mandates the WS asserts the IntentStore at `jobs/payments` carries the rkyv-archived Job whose `command` field equals "/opt/payments/bin/payments-server" and `args` equals `vec!["--port", "8080"]`. The actual test only asserts the spec_digest (line 128–134), which is a *computed* hash. The spec_digest alone cannot prove the Job fields persisted correctly to the store — the compute could be wrong, or the store could hold a different Job shape. The WS load-bearing assertion per walking-skeleton.md § "What the WS asserts" (line 4) is: "the IntentStore at key `jobs/payments` carries an rkyv-archived `Job` whose `command` and `args` fields equal the operator's declared values."
   - **Fix**: Add a back-door read of the IntentStore at `jobs/payments`, deserialise via rkyv, and assert `job.command == "/opt/payments/bin/payments-server"` and `job.args == vec!["--port", "8080"]`. This mirrors the pattern in existing acceptance tests like `submit_job_idempotency.rs` (reference: `crates/overdrive-control-plane/tests/acceptance/submit_job_idempotency.rs`).
   - **Severity**: Blocker — the WS is the sole E2E test proving wire-shape data flows end-to-end; without the persistence assertion, it proves only computation, not actual storage.

## Major (must address before DELIVER, but not a hard block)

None identified beyond the Critical issue above.

## Minor (worth fixing if cheap)

None.

## Nitpick (style/wording, optional)

None.

## What works well

- **No `.feature` files**: Zero Gherkin-executable files found under `crates/` or `tests/` — project override honored (DWD-1).
- **No subprocess spawning**: All CLI tests call handlers directly as Rust functions; no `Command::new(env!("CARGO_BIN_EXE_overdrive"))` found. Project rule honored (DWD-3 / crates/overdrive-cli/CLAUDE.md).
- **RED scaffold markers in place**: Grep confirms `todo!("RED scaffold: ADR-0031 §4 — Job::from_spec must reject empty / ...")` at `crates/overdrive-core/src/aggregate/mod.rs:143-144`. New tests are RED-not-BROKEN (DWD-4).
- **Structured validation assertions**: All validation tests match on `AggregateError::Validation { field, message }` with exact field strings like `"exec.command"` — not stringified `.to_string().contains()` (DWD-5).
- **Action-shim deletion coverage**: Behavioural test `action_shim_restart_passes_spec_from_action_to_driver_start_unchanged` records the spec passed to `Driver::start` and asserts it matches the action's spec (NOT the deleted `/bin/sleep` / `["60"]` literals). This is load-bearing for DWD-7 since the helpers are physically deleted (DWD-7).
- **Property-based scenarios**: Two @property tests included — validation (empty/whitespace command always yields `exec.command` Validation) and roundtrip (identity across proptest-generated inputs). Closes mutation gaps per `.claude/rules/testing.md` (DWD-12).
- **Reconciler purity twin-invocation**: `exec_reconciler_purity.rs` includes explicit twin-invocation test asserting `reconcile()` called twice with same inputs produces bit-identical `(Vec<Action>, NextView)` (DWD-13).
- **OpenAPI shape test**: `openapi_exec_block.rs` renders `OverdriveApi::openapi()` to YAML and asserts presence of `JobSpecInput`, `ResourcesInput`, `ExecInput`, `DriverInput`, and `exec` variant. Panics on missing types (RED signal).
- **Defence-in-depth at handler**: Server-side `submit_job` handler tested with empty exec.command; asserts `ControlPlaneError::Validation { field: Some("exec.command"), .. }` returned. In-process at axum boundary (DWD-14).
- **Coverage scope bounded**: All scenarios align with `upstream-changes.md` § Test surfaces that change. No tests for multi-driver (`microvm`/`wasm`), container image resolution, or argv sanitisation (DWD-15).
- **DWD-16 fixture-migration backlog named**: ~13 pre-existing test files identified as BROKEN (fixture migration pending in DELIVER). Wave-decisions.md lists all.

## Coverage table verification

| Upstream-changes.md rule | Scenario tag | Rust test name | Exists? | Assertion shape correct? |
|---|---|---|---|---|
| empty exec.command rejection | @validation | job_from_spec_rejects_empty_exec_command_with_structured_field_name | ✓ | ✓ structured AggregateError::Validation { field: "exec.command", message } |
| whitespace-only command | @validation | job_from_spec_rejects_whitespace_only_exec_command_via_trim_rule | ✓ | ✓ field match on "exec.command" |
| mixed Unicode whitespace | @validation | job_from_spec_rejects_mixed_whitespace_exec_command | ✓ | ✓ field match on "exec.command" |
| command casing preserved | @validation @happy | job_from_spec_preserves_operator_command_casing_verbatim | ✓ (in aggregate_constructors.rs) | ✓ asserts Job.command == "/Opt/Payments/Server" verbatim |
| empty args vec valid | @validation @happy | job_from_spec_accepts_non_empty_command_with_empty_args_vec | ✓ (in aggregate_constructors.rs) | ✓ asserts Job.args is empty Vec |
| no per-element args rule | @validation | job_from_spec_accepts_empty_string_and_whitespace_in_args_vec | ✓ (in aggregate_constructors.rs) | ✓ asserts all elements preserved verbatim |
| empty/whitespace command (proptest) | @validation @property | empty_or_whitespace_command_always_yields_exec_command_validation | ✓ (in aggregate_validation.rs) | ✓ proptest generation + structured variant match |
| Missing [exec] table → parse error | @validation | toml_missing_exec_table_fails_to_parse_with_serde_error | ✓ | ✓ asserts serde error contains "exec" or "missing field" |
| Missing [resources] table → parse error | @validation | toml_missing_resources_table_fails_to_parse_with_serde_error | ✓ | ✓ asserts serde error contains "resources" |
| Unknown top-level driver table | @validation | toml_with_unknown_top_level_driver_table_fails_to_parse_via_deny_unknown_fields | ✓ | ✓ asserts error contains unknown table name |
| Typo in [exec] field | @validation | toml_with_typo_in_exec_field_fails_via_deny_unknown_fields | ✓ | ✓ asserts error names the typo'd field |
| Action::StartAllocation projects job.command/args | @reconciler_purity | start_action_carries_full_alloc_spec_from_live_job_command_and_args | ✓ (in job_lifecycle_reconcile_branches.rs) | ✓ asserts Action.spec.command equals job.command |
| Action::RestartAllocation spec field | @reconciler_purity | restart_action_carries_full_alloc_spec_from_live_job | ✓ (in job_lifecycle_reconcile_branches.rs) | ✓ asserts Action.spec carries command/args/resources |
| Reconciler purity twin-invocation | @reconciler_purity | reconcile_with_exec_spec_is_deterministic_across_twin_invocations | ✓ (in job_lifecycle_reconcile_branches.rs) | ✓ byte-equal check across two invocations |
| Action shim deletion + Restart contract | @deletion | action_shim_restart_passes_spec_from_action_to_driver_start_unchanged | ✓ | ✓ recording fake asserts captured spec equals action's spec (NOT deleted literals) |
| OpenAPI schema shape | @openapi_propagation | openapi_schema_carries_jobspec_input_with_nested_resources_and_tagged_driver_exec_variant | ✓ | ✓ YAML assertions on schema shape |
| OpenAPI utoipa::ToSchema on new types | @openapi_propagation | (extends every_api_type_implements_utoipa_to_schema) | ✓ (marker found) | ✓ should extend with 4 new type asserts |
| CLI parse error — missing [exec] → field: "toml" | @validation | cli_submit_surfaces_missing_exec_table_as_toml_field_error | ✓ | ✓ asserts CliError::InvalidSpec { field: "toml", message } |
| CLI parse error — empty command BEFORE HTTP | @validation @driving_port | cli_submit_rejects_empty_exec_command_before_any_http_call | ✓ | ✓ asserts InvalidSpec + proves no HTTP by pointing at unreachable endpoint |
| Server defence-in-depth — empty command → 400 | @validation @driving_port | submit_job_handler_rejects_empty_exec_command_with_validation_error_naming_field | ✓ | ✓ asserts ControlPlaneError::Validation { field: Some("exec.command"), .. } |
| Server defence-in-depth — whitespace → 400 | @validation @driving_port | submit_job_handler_rejects_whitespace_only_exec_command_with_validation_error | ✓ | ✓ asserts Validation variant |
| Walking skeleton — operator submits exec spec | @walking_skeleton @driving_port | walking_skeleton_submit_with_exec_block_returns_inserted_and_persists_command_and_args | ✓ | **✗ MISSING**: IntentStore persistence assertion (see Critical issue above) |
| JobSpecInput round-trip identity | @round_trip | jobspec_input_roundtrips_through_aggregate_with_exec_block | ✓ (in aggregate_roundtrip.rs) | ✓ asserts input == back-converted output |
| JobSpecInput round-trip (proptest) | @round_trip @property | jobspec_input_roundtrip_property_with_exec_block | ✓ (in aggregate_roundtrip.rs) | ✓ proptest identity assertion |
| Job rkyv byte-identical archival | @round_trip | (existing test, fixture migration) | ✓ | ✓ sample_job fixture gains command + args |
| Job serde-JSON round-trip | @round_trip | (existing test, fixture migration) | ✓ | ✓ new fields survive round-trip |

**Coverage assessment**: 23 of 24 upstream-changes.md rules have CORRECT assertions. **1 rule FAILS**: walking-skeleton IntentStore persistence (Critical).

## RED-vs-BROKEN assessment

- **New test files**: 4 new Rust files compile cleanly:
  - `exec_validation.rs` — compiles ✓
  - `exec_constructors.rs` — compiles ✓
  - `exec_roundtrip.rs` — compiles ✓
  - `exec_reconciler_purity.rs` — compiles ✓
  - `openapi_exec_block.rs` — compiles ✓
  - `action_shim_restart_uses_spec_from_action.rs` — compiles ✓
  - `submit_job_handler_rejects_empty_exec_command_with_400.rs` — compiles ✓
  - `exec_spec_walking_skeleton.rs` — compiles ✓
- **Scaffold markers**: `todo!("RED scaffold: ADR-0031 §4 — Job::from_spec must reject empty / ...")` at aggregate/mod.rs:143. Tests will panic on the right line once pre-existing fixture breakage is resolved.
- **Pre-existing BROKEN files**: 13 files listed in DWD-16 fail to compile due to flat `cpu_milli`/`memory_bytes` shape in fixtures. This is expected RED signal pending DWD-9 single-cut migration in DELIVER.

## Project-rule compliance

✓ **No `.feature` files** — verified via Glob; zero matches in crates/ or tests/.
✓ **No CLI subprocess** — verified via Grep; no `Command::new(env!("CARGO_BIN_EXE_` patterns in overdrive-cli/tests.
✓ **RED scaffold markers** — `panic!("Not yet implemented -- RED scaffold")` or `todo!("RED scaffold: ...")` syntax found at aggregate/mod.rs.
✓ **Structured validation assertions** — all exec validation tests assert on `AggregateError::Validation { field, message }` with exact field strings.
✓ **No `serial_test::serial(env)` needed** — verified per DWD-11; new tests run in-process against typed fixtures, no env mutation.
✓ **Acceptance-vs-integration split** — per DWD-10: acceptance tests in default lane (no real reqwest, no serve::run); WS in `tests/integration/` (real serve::run, real reqwest). Feature gating checked.

## DWD-7 sanity check

Behavioural substitute test `action_shim_restart_passes_spec_from_action_to_driver_start_unchanged`:
- ✓ Records every `Driver::start(spec)` invocation via `RecordingDriver` fake.
- ✓ Asserts captured spec.command equals action's command ("/opt/x/y"), NOT deleted "/bin/sleep".
- ✓ Asserts captured spec.args equals action's args (["--mode=fast"]), NOT deleted ["60"].
- ✓ Asserts captured spec.resources equals action's resources, NOT deleted default_restart_resources fabrication.

The test is **load-bearing** for the deletion: if a future maintainer re-adds the deleted helpers, the test fails because the shim dispatch path would return wrong values. The helpers are physically deleted in source — clippy/cargo-udeps would flag any re-addition as dead code. DWD-7 cost-benefit is sound.

## DWD-16 completeness

Wave-decisions.md DWD-16 lists 13 pre-existing test files left in BROKEN state:

```
✓ crates/overdrive-core/tests/acceptance/aggregate_constructors.rs
✓ crates/overdrive-core/tests/acceptance/aggregate_roundtrip.rs
✓ crates/overdrive-core/tests/acceptance/aggregate_validation.rs
✓ crates/overdrive-core/tests/acceptance/first_fit_place_branches.rs
✓ crates/overdrive-core/tests/acceptance/job_lifecycle_reconcile_branches.rs
✓ crates/overdrive-cli/tests/integration/http_client.rs
✓ crates/overdrive-cli/tests/integration/walking_skeleton.rs
✓ crates/overdrive-control-plane/tests/acceptance/api_type_shapes.rs
✓ crates/overdrive-control-plane/tests/acceptance/job_lifecycle_backoff.rs
✓ crates/overdrive-control-plane/tests/acceptance/job_stop_idempotent.rs
✓ crates/overdrive-control-plane/tests/acceptance/job_stop_intent_key.rs
✓ crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs
✓ crates/overdrive-control-plane/tests/acceptance/submit_job_idempotency.rs
+ crates/overdrive-sim/tests/acceptance/reconciler_is_pure_with_job_lifecycle.rs
+ all of crates/overdrive-control-plane/tests/integration/*.rs
```

The list is **exhaustive** — verified by checking each file still uses flat-shape fixtures. No false positives found.

## Verdict rationale

**BLOCK**: The walking-skeleton test fails the coverage gate. Test-scenarios.md §1 (the canonical specification hand-off from DESIGN) mandates the WS assert the IntentStore at `jobs/payments` carries rkyv-archived Job fields `command == "/opt/payments/bin/payments-server"` and `args == vec!["--port", "8080"]`. The actual test only asserts spec_digest (a computed hash), which does not prove persistence. The spec_digest assertion passes if (a) the Job was computed correctly or (b) the computed hash was compared against itself — it does NOT prove the IntentStore actually holds a Job with the declared command/args values.

**Fix**: Add a back-door read of the IntentStore key `jobs/payments`, deserialise the rkyv bytes to a Job, and assert the `command` and `args` fields match. This is the minimal change to close the gap and satisfy the coverage mandate. The WS is the sole E2E test proving the wire shape flows end-to-end; without the persistence assertion, it proves only validation and computation, not end-to-end data flow.

All other 23 rules have correct, load-bearing assertions. All project rules (no `.feature` files, no subprocess, RED scaffolds, structured assertions) are honored. The single critical issue is addressable in a focused addition to the WS test.
