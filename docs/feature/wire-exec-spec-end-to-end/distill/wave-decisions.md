# DISTILL Decisions — wire-exec-spec-end-to-end

Designer: Quinn. Mode: propose. Date: 2026-04-30.

DESIGN approval: APPROVE
(`docs/feature/wire-exec-spec-end-to-end/design/review-from-architect-reviewer.md`,
0 critical / 0 major).

Derived from: ADR-0031, design `wave-decisions.md` D1–D9, design
`upstream-changes.md`. No DISCUSS / DEVOPS / SPIKE artifacts exist for
this feature — this is a DESIGN-first wave per `design/wave-decisions.md`
§ Upstream Changes.

## Decisions

- [DWD-1] **No `.feature` files; tests are Rust `#[test]` /
  `#[tokio::test]` functions.** Per `.claude/rules/testing.md`
  § *Testing*: "All acceptance and integration tests are written
  directly in Rust using `#[test]` / `#[tokio::test]` functions.
  Gherkin-style scenarios may appear as GIVEN/WHEN/THEN blocks in
  `docs/feature/{id}/distill/test-scenarios.md` for specification
  purposes only — they are never parsed or executed." Overrides the
  nWave skill's default "executable Gherkin via cucumber-rs /
  pytest-bdd" mandate. (see: `test-scenarios.md` header)

- [DWD-2] **Walking-skeleton strategy: C — Real local resources, no
  container.** Single end-to-end happy-path scenario calls
  `overdrive_cli::commands::job::submit(...)` as a Rust async function
  against an in-process `serve::run` server bound to `127.0.0.1:0`.
  Real `LocalIntentStore` (redb under `tempfile::TempDir`); real
  on-disk TOML; real reqwest+rustls HTTP transport; real rcgen-minted
  trust triple. Sim adapters for `ObservationStore` and `Driver`
  because (a) Phase 1 has not landed real Corrosion (per ADR-0012
  revision) and (b) `ExecDriver` integration is exercised by the
  separate `crates/overdrive-control-plane/tests/integration/job_lifecycle/`
  suite under `--features integration-tests`. Detailed in
  `walking-skeleton.md`. (see: `walking-skeleton.md` § *Strategy*)

- [DWD-3] **CLI handler called as a Rust function — no subprocess.**
  Per `crates/overdrive-cli/CLAUDE.md` § *Integration tests — no
  subprocess*: tests call `overdrive_cli::commands::job::submit(SubmitArgs
  { ... })` directly. This is the project-canonical "driving adapter
  verification" form. No `Command::new(env!("CARGO_BIN_EXE_overdrive"))`
  in any test. (see: `crates/overdrive-cli/CLAUDE.md`)

- [DWD-4] **RED scaffolds use `panic!("Not yet implemented -- RED
  scaffold")` or `todo!("RED scaffold: ...")`.** Per `.claude/rules/testing.md`
  § *RED scaffolds and intentionally-failing commits*: the panic IS
  the specification of work not yet done. No `Ok(())` neutral stubs,
  no `#[ignore]` annotations on pre-existing tests that the new
  scaffolds break. Pre-existing test breakage from the new field
  additions (e.g. `aggregate_constructors.rs` literals using flat
  `cpu_milli`) is the desired RED signal — not a test bug.
  Single-cut migration (DWD-9) lands the production code AND the
  fixture updates in the same DELIVER step, so the RED window is
  bounded.

- [DWD-5] **Validation tests assert on the structured `AggregateError::Validation
  { field, message }` variant, NOT a stringified `Display` form.**
  Per `.claude/rules/development.md` § *Errors* and the existing
  `aggregate_validation.rs` pattern: HTTP layer (ADR-0015) consumes
  the variant shape via `#[from]` pass-through to
  `ControlPlaneError::Aggregate`; downstream contract is the variant,
  not the message. Stringified-message assertions would lock the
  feature to a specific punctuation/wording style.

- [DWD-6] **`exec.command` validation message is pinned literally:
  `"command must be non-empty"`.** Per ADR-0031 §4. This is the only
  literal-string assertion the tests pin — `field: "exec.command"`
  is the structured discriminator; the message string is a
  human-facing diagnostic and Ana reads it on the operator surface,
  so it is part of the contract.

- [DWD-7] **Action-shim deletion is proven by behavioural test, not
  trybuild compile-fail.** Adding a trybuild harness to
  `overdrive-control-plane` (currently no compile_fail tests; no
  trybuild dev-dep) would be net-new infrastructure for one
  invariant, plus a `.stderr` golden whose exact diagnostic shape is
  rustc-version-sensitive. Instead, the `action_shim_restart_uses_spec_from_action`
  acceptance test asserts the consumer-side semantics — the
  recording-fake `Driver` captures the `AllocationSpec` the shim
  passes to `start()`, and asserts the captured spec equals the
  action's spec (NOT the deleted `/bin/sleep` baseline). That test
  fails IF the deleted helpers are still in the dispatch path. The
  helpers themselves are deleted in source — a future maintainer
  re-adding them produces dead code that clippy + `cargo +nightly
  udeps` flag at PR time. (see: design D6, ADR-0031 §6,
  `feedback_delete_dont_gate.md`)

- [DWD-8] **OpenAPI propagation tested at two layers.** Layer 1 —
  live-render shape pin via direct call to
  `xtask::openapi::generate_yaml()` (which renders
  `OverdriveApi::openapi()` to YAML). Layer 2 — the existing
  `cargo xtask openapi-check` CI gate that compares against
  `api/openapi.yaml`. Layer 1 is acceptance test; Layer 2 is the CI
  artifact-drift gate. Both are required: Layer 1 catches "the schema
  shape regressed" within the default test lane; Layer 2 catches
  "the checked-in artifact is stale." (see: design D8, ADR-0009)

- [DWD-9] **Single-cut fixture migration lands in the same DELIVER
  step as the production code change.** Per
  `feedback_single_cut_greenfield_migrations.md` and design D9: every
  test fixture, doctest, integration test, OpenAPI snapshot, and CLI
  render path migrates atomically. The crafter does NOT enable a
  one-fixture-at-a-time scaffold. The ~25 source files are mechanical
  literal-by-literal substitutions; none requires logic changes.
  DISTILL writes the new test scaffolds AND leaves pre-existing tests
  in their current shape (the panicking `JobSpecInput { id, replicas,
  cpu_milli, memory_bytes }` literals will be the "BROKEN" signal at
  HEAD, transitioning to RED after the production code lands and to
  GREEN after the migration sweep). DELIVER's first commit IS the
  union of (a) production code with the new `Job` fields, (b) the
  migration sweep over the ~25 fixture files, (c) the new tests added
  by DISTILL going GREEN.

- [DWD-10] **Acceptance tests (in-process, sim-adapter) live in the
  default lane; integration tests (real reqwest, real serve::run)
  live behind `--features integration-tests`.** Per
  `.claude/rules/testing.md` § *Integration vs unit gating* and the
  existing crate layout: per-crate `Cargo.toml` declares
  `integration-tests = []`; `tests/acceptance/*.rs` runs in the
  default `cargo nextest run` pass; `tests/integration/<scenario>.rs`
  runs only with the feature enabled. The walking-skeleton scenario
  (DWD-2) lives in `crates/overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs`
  because it spins up real `serve::run`. The validation, round-trip,
  reconciler-purity, OpenAPI shape, action-shim-deletion, and
  defence-in-depth handler scenarios live in the default lane (no
  real network, no real subprocess).

- [DWD-11] **No `serial_test::serial(env)` annotation needed for new
  tests.** Per `.claude/rules/testing.md` § *Tests that mutate
  process-global state*: serial_test gates env-mutating tests. The
  new acceptance scenarios all run in-process against typed
  fixtures; no test mutates `$HOME`, `$OVERDRIVE_CONFIG_DIR`, cwd, or
  any other process-global. The walking-skeleton scenario (DWD-2)
  uses a per-test `tempfile::TempDir` for the trust triple — same
  pattern as the existing `walking_skeleton.rs` integration test,
  which does not carry `#[serial(env)]` either.

- [DWD-12] **Property-based tests use the workspace `proptest`
  dependency at the workspace-default case count
  (`PROPTEST_CASES=1024`).** Per `.claude/rules/testing.md`
  § *Property-based testing*: do not lower the default case count to
  dodge a slow generator. The two property-shaped scenarios in
  `test-scenarios.md` (validation @property and roundtrip @property)
  use the same `valid_label()` / `arb_job()` strategy patterns as the
  existing `aggregate_validation.rs` and `aggregate_roundtrip.rs`
  tests, extended to carry `command` + `args`.

- [DWD-13] **Reconciler-purity reasserted via twin-invocation, not
  via internal-state inspection.** Per ADR-0013's `ReconcilerIsPure`
  invariant: `reconcile()` is called twice with the same
  `(desired, actual, view, tick)` and the two `(Vec<Action>, NextView)`
  return values are byte-equal. This is the existing DST-side
  invariant; the new acceptance test in `job_lifecycle_reconcile_branches.rs`
  adds the explicit twin-invocation assertion to the default lane
  for the new spec materialisation path. Closes the obvious mutation
  gap ("an `Instant::now()` snuck into spec construction") that DST
  catches at integration boundary but the default lane should also
  defend.

- [DWD-14] **HTTP handler scenarios test in-process at the axum
  handler boundary, NOT through a real server start.** Per the
  existing `submit_job_idempotency.rs` pattern: tests construct an
  `AppState` from `LocalIntentStore` over `TempDir` + `SimObservationStore`
  + `SimDriver`, then call `handlers::submit_job(State(state),
  Json(request))` directly. No reqwest, no TLS handshake, no port
  binding — the typed `Result<Json<SubmitJobResponse>,
  ControlPlaneError>` return is the assertion target. This is the
  cheapest exercise of the server-side validating constructor; the
  end-to-end real-network flow is the WS (DWD-2).

- [DWD-15] **Scope is bounded by `design/upstream-changes.md`.**
  Per the skill's Phase 1 Step 7: scenarios cover behaviours
  enumerated in the feature delta only. Out-of-scope items
  (multi-driver dispatch on `microvm`/`wasm`, container image
  resolution, argv sanitisation, forward-compat policy, `describe`
  render goldens) are explicitly NOT covered by tests in this DISTILL.

- [DWD-16] **The DISTILL HEAD state is BROKEN-by-design for ~13
  pre-existing test files; commit with `git commit --no-verify`.**
  After applying the production-side scaffolds (reshaped
  `JobSpecInput`, new `Job.command`/`args` fields, new
  `Action::RestartAllocation { spec }` field), the following
  pre-existing test files cease to compile because their fixtures use
  the flat `cpu_milli` / `memory_bytes` shape:
  - `crates/overdrive-core/tests/acceptance/aggregate_constructors.rs`
  - `crates/overdrive-core/tests/acceptance/aggregate_roundtrip.rs`
  - `crates/overdrive-core/tests/acceptance/aggregate_validation.rs`
    (the existing tests)
  - `crates/overdrive-core/tests/acceptance/first_fit_place_branches.rs`
  - `crates/overdrive-core/tests/acceptance/job_lifecycle_reconcile_branches.rs`
    (uses old `Action::RestartAllocation { alloc_id }` pattern)
  - `crates/overdrive-cli/tests/integration/http_client.rs`
  - `crates/overdrive-cli/tests/integration/walking_skeleton.rs`
    (uses flat-shape TOML)
  - `crates/overdrive-control-plane/tests/acceptance/api_type_shapes.rs`
  - `crates/overdrive-control-plane/tests/acceptance/job_lifecycle_backoff.rs`
  - `crates/overdrive-control-plane/tests/acceptance/job_stop_idempotent.rs`
  - `crates/overdrive-control-plane/tests/acceptance/job_stop_intent_key.rs`
  - `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs`
  - `crates/overdrive-control-plane/tests/acceptance/submit_job_idempotency.rs`
  - all of `crates/overdrive-control-plane/tests/integration/*.rs`
  - `crates/overdrive-sim/tests/acceptance/reconciler_is_pure_with_job_lifecycle.rs`

  Per `.claude/rules/testing.md` § *RED scaffolds and
  intentionally-failing commits* and the user's explicit hand-off:
  "Do NOT swap them for `Ok(())` neutral stubs. Do NOT mark
  pre-existing tests `#[ignore]` because your scaffold breaks them
  — the breakage is the desired RED signal." The DELIVER crafter
  closes the loop in step (f) of the handoff sequence (single-cut
  fixture migration). Commit with `git commit --no-verify` and call
  out the BROKEN state explicitly in the commit message.

  All NEW tests added by DISTILL DO compile cleanly; the BROKEN
  status is exclusively the documented fixture-migration backlog.
  See `verification` notes below.

## Amendments — 2026-04-30

ADR-0031 received Amendment 1 introducing a tagged-enum
`WorkloadDriver` field on the `Job` aggregate. The amendment was
ratified by the architect-reviewer (verdict: APPROVE; see
`docs/feature/wire-exec-spec-end-to-end/design/review-from-architect-reviewer-amendment-1.md`).
Wire shape (`JobSpecInput`, `DriverInput`, `ExecInput`,
`ResourcesInput`) is UNCHANGED. `AllocationSpec` is UNCHANGED — flat
per ADR-0030 §6, and the Phase-2+ per-driver-class spec split remains
the predicted shape. The validation field name `exec.command` is
UNCHANGED. Action shim is UNCHANGED. Only the **intent aggregate**
and the reconciler's spec-population destructure reshape.

The decisions below SUPERSEDE / EXTEND prior DWDs.

- [DWD-3 SUPERSEDED 2026-04-30 by DWD-18.] ~~Pre-existing test files
  cease to compile because their fixtures use the flat
  `cpu_milli` / `memory_bytes` shape on `JobSpecInput { ... }`
  literals.~~ The BROKEN list still applies, but the NEW shape on
  `Job` literal construction also breaks any test fixture that
  constructs `Job { command: ..., args: ..., .. }` directly. See
  DWD-18 for the updated migration shape.

- [DWD-18] **`Job` literal construction and pattern matching uses
  the nested form.** Every test that constructs a `Job` literal or
  pattern-matches on a `Job` value uses
  `Job { driver: WorkloadDriver::Exec(Exec { command, args }), .. }`
  in place of the originally-flat
  `Job { command, args, .. }` form. Direct field accesses
  `job.command` / `job.args` become a destructure
  `let WorkloadDriver::Exec(exec) = &job.driver;` followed by
  `exec.command` / `exec.args`. Tests that pattern-match on
  `Action::StartAllocation { spec, .. }` /
  `Action::RestartAllocation { alloc_id, spec }` continue to access
  flat `spec.command` / `spec.args` — the action carries
  `AllocationSpec`, which is unchanged per ADR-0030. (see: ADR-0031
  Amendment 1 § Migration impact)

- [DWD-19] **Wire-shape `JobSpecInput { driver: DriverInput::Exec(ExecInput { command, args }) }`
  remains the single wire-side input form.** Server-side and
  client-side handlers continue to construct and accept the wire
  shape verbatim. The intent-side projection (`DriverInput::Exec` →
  `WorkloadDriver::Exec`) happens inside `Job::from_spec` per the
  validating-constructor discipline (DWD-7 unchanged). No test
  scenario constructs a `WorkloadDriver` value at the wire boundary;
  only the intent-side assertions traverse it.

- [DWD-20] **The walking-skeleton's back-door IntentStore read
  asserts the nested driver shape.** The load-bearing assertion in
  `walking_skeleton_submit_with_exec_block_returns_inserted_and_persists_command_and_args`
  reads the rkyv bytes at `jobs/payments`, deserialises to `Job`,
  then destructures `let WorkloadDriver::Exec(exec) = &job.driver;`
  and asserts `exec.command == "/opt/payments/bin/payments-server"`
  and `exec.args == vec!["--port", "8080"]`. The driving port is
  unchanged; the test still calls
  `overdrive_cli::commands::job::submit(...)` as a Rust function
  per `crates/overdrive-cli/CLAUDE.md`. (See `walking-skeleton.md`
  § *What the WS asserts* as amended.)

- [DWD-21] **Validation field name `exec.command` is preserved in
  every assertion.** Operator-facing path matches the TOML the
  operator typed (`[exec]\ncommand = "..."` reads as
  `exec.command`). The internal Rust nesting `job.driver` →
  `WorkloadDriver::Exec` → `Exec.command` does NOT leak into
  diagnostics — surfacing `driver.exec.command` would expose
  internal type structure with no operator benefit. Validation
  message literal `"command must be non-empty"` is also unchanged
  (DWD-6). All existing structured-error assertions continue to
  match `field == "exec.command"` verbatim.

- [DWD-22] **Reconciler-purity scenarios assert on flat
  `spec.command` / `spec.args` (the action's
  `AllocationSpec`).** The §5 scenarios in `test-scenarios.md`
  pattern-match `Action::StartAllocation { spec, .. }` and
  `Action::RestartAllocation { alloc_id, spec }`. `AllocationSpec`
  stays flat per ADR-0030 / ADR-0031 Amendment 1 § AllocationSpec
  UNCHANGED, so `spec.command` / `spec.args` direct field access
  remains the assertion shape. The change is in the **reconciler's
  populating expression** (production code): it now destructures
  `&job.driver` into `WorkloadDriver::Exec(Exec { command, args })`
  and projects to flat `AllocationSpec`. Tests do not observe the
  destructure — they observe the projection's output, which is the
  flat spec on the action.

### BROKEN list extension

DWD-16's BROKEN list still applies. Additionally, after the
amendment lands in DELIVER, the following file gains compile errors
on the OLD-shape `Job` literal (the test fixture
`make_job_with_command_args` constructs `Job { command, args, .. }`):
- `crates/overdrive-core/tests/acceptance/exec_reconciler_purity.rs`
  — fixture amended in this DISTILL re-run to the nested form.

The `From<&Job> for JobSpecInput` projection in `aggregate/mod.rs` is
amended by this DISTILL re-run to destructure `&job.driver`. The two
reconciler spec-population sites in `reconciler.rs` (Restart at line
~1186 and Start at line ~1227) are amended to destructure
`&job.driver` → flat `AllocationSpec`.

### Verification budget

The amendment scope is targeted. Verification expectation post-amendment
(at HEAD with all amendment changes landed):

```
cargo check -p overdrive-core           → clean
cargo check -p overdrive-control-plane  → clean
cargo check -p overdrive-cli            → clean
cargo nextest run -p overdrive-core --no-run
    → DWD-16 pre-existing BROKEN files still BROKEN; new exec_*.rs
      files compile cleanly under the amended shape
cargo nextest run -p overdrive-control-plane --no-run
    → DWD-16 pre-existing BROKEN files still BROKEN; new
      openapi_exec_block / submit_job_handler_rejects_empty_exec_command_with_400
      / action_shim_restart_uses_spec_from_action all compile cleanly
cargo nextest run -p overdrive-cli --features integration-tests --no-run
    → DWD-16 pre-existing http_client.rs still BROKEN; new
      exec_spec_walking_skeleton.rs compiles cleanly
```

The scope deltas vs DWD-17 are: `aggregate/mod.rs` (production
scaffold) and `reconciler.rs` (production scaffold) gain the
`WorkloadDriver` enum and `Exec` struct; their `From<&Job>` projection
and reconciler destructure sites use the amended shape. The 4
`overdrive-core/tests/acceptance/exec_*.rs` files, the 3
`overdrive-control-plane/tests/acceptance/*` files, and the 1
`overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs` file
amended in this DISTILL re-run continue to compile against the new
shape.

- [DWD-17] **Verification at HEAD (after applying scaffolds + new
  tests):**

  ```text
  cargo check -p overdrive-core           → clean
  cargo check -p overdrive-control-plane  → clean
  cargo check -p overdrive-cli            → clean
  cargo check -p overdrive-sim            → clean
  cargo nextest run -p overdrive-core --no-run        → 25 errors,
        all in 5 pre-existing files (DWD-16); 0 errors in new files
  cargo nextest run -p overdrive-control-plane --no-run → 14 errors,
        all in 6 pre-existing files (DWD-16); 0 errors in new files
  cargo nextest run -p overdrive-cli --features integration-tests --no-run
        → 4 errors in 1 pre-existing file (http_client.rs); 0 errors in new files
  ```

  The BROKEN scope is bounded and named (DWD-16). The new tests are
  RED-not-BROKEN — they compile and would panic on the right line
  (the `todo!("RED scaffold")` in `Job::from_spec`'s validation
  path) once the pre-existing test breakage is resolved by the
  fixture-migration sweep in DELIVER.

## Reuse analysis

| Existing test surface | What gets EXTENDED vs CREATED |
|---|---|
| `crates/overdrive-core/tests/acceptance/aggregate_validation.rs` | EXTEND — add §2 + §3 scenarios for exec validation and TOML-parse rejections. Existing tests for replicas / memory / id remain. |
| `crates/overdrive-core/tests/acceptance/aggregate_constructors.rs` | EXTEND — add positive-path scenarios (empty args, casing preserved, args opaqueness). Existing constructor tests migrate to nested fixture shape. |
| `crates/overdrive-core/tests/acceptance/aggregate_roundtrip.rs` | EXTEND — add @round_trip scenarios; sample fixtures gain `command`/`args`; proptest strategies extend. |
| `crates/overdrive-core/tests/acceptance/job_lifecycle_reconcile_branches.rs` | EXTEND — add @reconciler_purity scenarios for `Action::Spawn.spec` and `Action::Restart.spec` projection; existing branch-coverage tests retained verbatim. |
| `crates/overdrive-cli/tests/integration/walking_skeleton.rs` | EXTEND — fixture body migrates from flat → nested `[resources]`/`[exec]`. Existing assertions retained; the new spec carries command/args. |
| `crates/overdrive-cli/tests/integration/job_submit.rs` | EXTEND — add CLI defence-in-depth scenarios (§9). |
| `crates/overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs` | CREATE — dedicated WS scenario asserting persisted command/args (DWD-2). |
| `crates/overdrive-control-plane/tests/acceptance/api_type_shapes.rs` | EXTEND — add `every_api_type_implements_utoipa_to_schema` asserts for `ResourcesInput`, `ExecInput`, `DriverInput`. |
| `crates/overdrive-control-plane/tests/acceptance/openapi_exec_block.rs` | CREATE — Layer 1 OpenAPI live-render shape pin (§7). |
| `crates/overdrive-control-plane/tests/acceptance/submit_job_handler_rejects_empty_exec_command_with_400.rs` | CREATE — defence-in-depth handler rejection (§8). |
| `crates/overdrive-control-plane/tests/acceptance/action_shim_restart_uses_spec_from_action.rs` | CREATE — pins shim reads spec from action, not fabricated baseline (§6). |
| `crates/overdrive-control-plane/tests/compile_fail/build_phase1_restart_spec_deleted.rs` + `.stderr` | CREATE — trybuild compile-fail proof of deletion (§6). |
| `crates/overdrive-control-plane/tests/acceptance/submit_job_idempotency.rs` | EXTEND — fixture migration only (no logic change; idempotency contract unchanged). |
| `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs` | EXTEND — fixture migration only. |
| `crates/overdrive-control-plane/tests/acceptance/job_lifecycle_backoff.rs` | EXTEND — fixture migration only. |

Net new test files: **5** (1 WS + 4 acceptance / compile-fail).
Net extensions: ~10 existing files (mechanical fixture migrations
sweep concurrently per DWD-9).

## Constraints established

- **No external test runners introduced.** No cucumber-rs, no
  pytest-bdd, no `.feature` parser. Per DWD-1.
- **No subprocess in CLI tests.** Per DWD-3.
- **No fixture parametrisation across environments.** The acceptance
  suite is the default unit lane (in-process, sim-adapter-only); the
  one WS-shaped integration scenario uses a single canonical
  fixture (rcgen-minted trust triple under TempDir). DEVOPS missing
  → matrix collapses to "single-node Linux".
- **No internal-component imports in tests.** Tests import driving
  ports (`Job::from_spec`, `commands::job::submit`, `handlers::submit_job`,
  `JobLifecycle::reconcile` via `Reconciler` trait) plus typed
  return-shape variants for assertions.
- **No new wire-shape compatibility shim.** Per DWD-9.

## Wave-collaboration handoff to DELIVER

This DISTILL hands off:

1. `test-scenarios.md` — the SSOT for scenario shapes (Gherkin
   blocks for human review, mapping table for traceability to Rust
   test names).
2. `walking-skeleton.md` — the WS-strategy decision, driving port,
   resources, and "what InMemory cannot model" explanation.
3. `wave-decisions.md` — this document, capturing every DISTILL
   choice that DELIVER must honour.
4. **Rust scaffolds** — minimal RED scaffolds in `overdrive-core/src/`
   (new `Job` fields wired with `panic!` on the unimplemented
   validation path) and the new test files described in the Reuse
   Analysis table above. The scaffolds compile (production code adds
   the new fields with sensible defaults that DON'T silently work)
   and the tests panic on the right line (the validator, the
   projection, the round-trip).
5. **Mandate compliance evidence**: see `test-scenarios.md`
   § *Mandate compliance proof*.

The crafter's first DELIVER step:

a. Replace the panicking validation body in `Job::from_spec` with the
   real predicate (`exec.command.trim().is_empty()` rejection).
b. Replace the literal `/bin/sleep` in `JobLifecycle::reconcile` with
   `job.command.clone()` / `job.args.clone()`.
c. Add the new `spec` field to `Action::RestartAllocation` and
   populate it in the reconciler's restart arm.
d. Delete `build_phase1_restart_spec`, `build_identity`,
   `default_restart_resources`, and `default_restart_resources_pins_exact_values`
   from the action shim.
e. Update the Restart arm of `dispatch_single` to read `spec` off the
   action.
f. Mass-migrate the ~25 fixture files (DWD-9) — flat `cpu_milli` /
   `memory_bytes` → nested `[resources]` + `[exec]`.
g. Regenerate `api/openapi.yaml` via `cargo xtask openapi-gen`.
h. Verify the entire suite goes GREEN: default lane (`cargo nextest
   run`), integration lane (`cargo nextest run --features
   integration-tests` — on Linux or Lima per `.claude/rules/testing.md`).

Per DWD-9 these all land in one DELIVER step, not staged across
multiple commits.
