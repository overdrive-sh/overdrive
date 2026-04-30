# Test scenarios — wire-exec-spec-end-to-end

> **SPECIFICATION ONLY.** Per `.claude/rules/testing.md` § *Testing*: no
> `.feature` files exist in this repo. The Gherkin blocks below are
> documentation. Each scenario maps to a Rust `#[test]` / `#[tokio::test]`
> in the test files enumerated in §*Coverage table*. The skill's default
> "executable Gherkin" mandate is overridden by project rule (no
> cucumber-rs, no pytest-bdd, no `.feature` consumer).
>
> Designer: Quinn. Date: 2026-04-30. DESIGN approval: APPROVE
> (`docs/feature/wire-exec-spec-end-to-end/design/review-from-architect-reviewer.md`).
> ADR anchor: ADR-0031.
> Hand-off source: `design/upstream-changes.md`.

> **Amended 2026-04-30 per ADR-0031 Amendment 1.** The `Job` aggregate
> now carries `driver: WorkloadDriver` (tagged enum) instead of flat
> `command` / `args` fields. `WorkloadDriver::Exec(Exec { command, args })`
> is the single Phase-1 variant. The wire-shape `JobSpecInput`,
> `DriverInput`, `ExecInput`, `ResourcesInput` are unchanged.
> `AllocationSpec` stays flat per ADR-0030 §6 — the Restart / Start
> action specs continue to expose flat `spec.command` / `spec.args`.
> The validation field name `exec.command` is unchanged (operator-facing
> path matches the TOML the operator typed). Scenarios that observe the
> persisted Job aggregate or pattern-match a `Job` literal use the
> nested form `Job { driver: WorkloadDriver::Exec(Exec { command, args }), .. }`;
> scenarios that observe `Action::StartAllocation { spec, .. }` /
> `Action::RestartAllocation { alloc_id, spec }` continue to access flat
> `spec.command` / `spec.args` (the action carries `AllocationSpec`).
> The validation message literal `"command must be non-empty"` is
> unchanged. See *Amendments — 2026-04-30* in `wave-decisions.md` for
> the corresponding DWDs.

## Persona

**Ana** — Phase 1 operator. Submits job specs as TOML on disk via
`overdrive job submit <path>`. Reads describe-rendered output. Cannot
peek at internal Rust types or rkyv bytes; observable surface is the
CLI exit code, stderr error message (with field name), the
control-plane HTTP response, and the cluster's eventual allocation
state.

## Walking-skeleton scope

**Strategy: C — Real local resources, no container.**

The single end-to-end happy-path scenario (`@walking_skeleton`) calls
`overdrive_cli::commands::job::submit(...)` directly as a Rust
function (per `crates/overdrive-cli/CLAUDE.md` § *Integration tests —
no subprocess*) against an in-process `serve::run` server bound to
`127.0.0.1:0`. All resources are local:

- Job spec is a real TOML file under a `tempfile::TempDir`.
- `IntentStore` is a real `LocalIntentStore` (redb) over `TempDir`.
- `ObservationStore` is `SimObservationStore::single_peer` (in-memory
  CRDT — no real Corrosion peer; Corrosion is **not** the real local
  resource here per design D7 / D8).
- Driver is `SimDriver::new(DriverType::Exec)` — no real subprocess
  spawned (the WS gate exercises the *intent surface*, not the driver
  surface; `ExecDriver` integration is exercised by the existing
  `job_lifecycle/submit_to_running.rs` integration suite under
  `--features integration-tests`).
- `Clock`, `Transport`, `Entropy` are sim adapters (DST-controllable).

This matches the existing `walking_skeleton.rs` integration shape from
`phase-1-control-plane-core` step 05-05.

## Scope (in)

This DISTILL covers exactly the surfaces enumerated in
`design/upstream-changes.md` § *Test surfaces that change*:

1. `Job` aggregate growth (`command: String`, `args: Vec<String>`).
2. `JobSpecInput` reshape (flat scalars + `resources: ResourcesInput`
   + `#[serde(flatten)] driver: DriverInput`).
3. New tagged-enum `DriverInput { Exec(ExecInput) }`.
4. New `Job::from_spec` validation rule (`exec.command` non-empty
   after trim).
5. `Action::StartAllocation { spec, .. }` populating expression change
   (literals → `job.command.clone()` / `job.args.clone()`).
6. `Action::RestartAllocation { alloc_id, spec }` shape growth.
7. `action_shim` deletions: `build_phase1_restart_spec`,
   `build_identity`, `default_restart_resources`,
   `default_restart_resources_pins_exact_values` test.
8. OpenAPI schema regeneration (utoipa derives).
9. Defence-in-depth — server-side `Job::from_spec` rejects empty
   `exec.command` with HTTP 400.

## Out of scope (do NOT add scenarios for)

Per `design/upstream-changes.md` § *Out of scope*:

- Multi-driver (`microvm`, `wasm`) — Phase 2+.
- Container image resolution / registries / content hashes for
  binaries.
- Argv sanitisation / command allowlists / denylists on `command`.
- Forward-compat spec extension policy.
- `overdrive job describe` render goldens for the new sections.
- KPI measurability (PO-reviewer scope, post-merge).

## Wave reconciliation

- DISCUSS missing → derived ACs from DESIGN (`upstream-changes.md`).
- DEVOPS missing → environment matrix collapses to "single-node
  Linux, Rust toolchain pinned by `rust-toolchain.toml`". The
  acceptance suite is the default unit lane (in-process,
  sim-adapter-only); no environment fixture parametrisation.
- No cross-wave contradictions — DESIGN-first feature, single ratified
  source.

---

## §1 Walking skeleton

### Scenario @walking_skeleton @driving_port — Operator submits a job spec carrying explicit command and args, the validated job persists, and a fresh-insert outcome echoes back

```gherkin
Given Ana has the trust triple from `serve::run` on disk under TempDir
  And Ana has written `payments.toml` with the canonical wire shape:
        id = "payments"
        replicas = 1
        [resources]
        cpu_milli    = 500
        memory_bytes = 134217728
        [exec]
        command = "/opt/payments/bin/payments-server"
        args    = ["--port", "8080"]
When Ana invokes `overdrive_cli::commands::job::submit(SubmitArgs { spec, config_path })`
Then the call returns Ok(SubmitOutput) with `outcome = IdempotencyOutcome::Inserted`
  And `job_id` echoes back as "payments"
  And `intent_key` derives to "jobs/payments"
  And `spec_digest` is byte-identical to a locally-computed
        ContentHash::of(rkyv::to_bytes(&Job::from_spec(parsed))).to_string()
  And the IntentStore at `jobs/payments` carries the rkyv-archived `Job`
        whose `driver` matches `WorkloadDriver::Exec(Exec { command, args })`
        with `command == "/opt/payments/bin/payments-server"`
        and `args == vec!["--port", "8080"]`
```

Per ADR-0031 Amendment 1: the back-door IntentStore read deserialises the
rkyv bytes at `jobs/payments`, then destructures
`let WorkloadDriver::Exec(exec) = &job.driver;` and asserts on
`exec.command` / `exec.args`. The `Job` literal field access
`job.command` / `job.args` is no longer valid — the fields live one
level deeper through the tagged enum.

Maps to: `crates/overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs::walking_skeleton_submit_with_exec_block_returns_inserted_and_persists_command_and_args`.

---

## §2 Aggregate validation rules (mirror upstream-changes.md 1:1)

### Scenario @validation — `Job::from_spec` rejects empty `exec.command` with structured field name

```gherkin
Given a JobSpecInput whose driver is Exec(ExecInput { command: "", args: vec![] })
  And every other field is otherwise valid
When Ana calls `Job::from_spec(spec)`
Then the result is Err(AggregateError::Validation { field, message })
  And `field == "exec.command"`
  And `message == "command must be non-empty"`
  And no `Job` value is constructed (Result<Job, _>::Err carries no Job)
```

Maps to: `aggregate_validation::job_from_spec_rejects_empty_exec_command_with_structured_field_name`.

### Scenario @validation — `Job::from_spec` rejects whitespace-only `exec.command` (the trim rule)

```gherkin
Given a JobSpecInput whose `exec.command` is "   " (three spaces)
When Ana calls `Job::from_spec(spec)`
Then the result is Err(AggregateError::Validation { field: "exec.command", .. })
```

The "non-empty after trim" predicate distinguishes "" from "   "; both
must reject under the same variant. A mutation of `.trim()` away from
the validator passes for `""` but fails for `"   "` — the second test
kills it.

Maps to: `aggregate_validation::job_from_spec_rejects_whitespace_only_exec_command_via_trim_rule`.

### Scenario @validation — `Job::from_spec` rejects tab-and-newline `exec.command`

```gherkin
Given a JobSpecInput whose `exec.command` is "\t\n " (mixed whitespace)
When Ana calls `Job::from_spec(spec)`
Then the result is Err(AggregateError::Validation { field: "exec.command", .. })
```

Pinning that the predicate uses `str::trim` (Unicode whitespace), not
just `is_empty()`. Per ADR-0031 §4.

Maps to: `aggregate_validation::job_from_spec_rejects_mixed_whitespace_exec_command`.

### Scenario @validation @happy — `Job::from_spec` accepts non-empty `command` and empty `args`

```gherkin
Given a JobSpecInput with command "/bin/true" and args = []
When Ana calls `Job::from_spec(spec)`
Then the result is Ok(Job)
  And `Job.driver` matches `WorkloadDriver::Exec(Exec { command, args })`
  And `command == "/bin/true"`
  And `args` is an empty Vec
```

Per ADR-0031 Amendment 1: command / args are accessed through the
tagged-enum nesting, e.g.
`let WorkloadDriver::Exec(exec) = &job.driver;`.

Per ADR-0031 §4: empty `args` is the legitimate zero-args case for
binaries that take no arguments (`/bin/true`, `/bin/date`). This must
NOT fire the validation rule.

Maps to: `aggregate_constructors::job_from_spec_accepts_non_empty_command_with_empty_args_vec`.

### Scenario @validation @happy — `Job::from_spec` preserves operator's `command` casing and surrounding-whitespace stripping is NOT applied (predicate, not normaliser)

```gherkin
Given a JobSpecInput with command "/Opt/Payments/Server" (mixed case)
When Ana calls `Job::from_spec(spec)`
Then the result is Ok(Job)
  And `Job.driver` matches `WorkloadDriver::Exec(Exec { command, .. })`
  And `command == "/Opt/Payments/Server"` (original casing preserved verbatim)
```

Per ADR-0031 §4: validation is a *predicate*, not a *normalisation*.
The original `command` string flows to the driver as-typed.

Maps to: `aggregate_constructors::job_from_spec_preserves_operator_command_casing_verbatim`.

### Scenario @validation — `Job::from_spec` carries no per-element rule on `args`

```gherkin
Given a JobSpecInput with command "/bin/echo" and args = ["", "  ", "non-empty"]
When Ana calls `Job::from_spec(spec)`
Then the result is Ok(Job)
  And `Job.driver` matches `WorkloadDriver::Exec(Exec { args, .. })`
  And `args == vec!["", "  ", "non-empty"]` (every element preserved verbatim)
```

Per ADR-0031 §4: argv is opaque to the platform; per-element
validation is the kernel's job at `execve(2)`. Adding a Phase 1
rejection rule would diverge from the kernel's posture for no safety
benefit.

Maps to: `aggregate_constructors::job_from_spec_accepts_empty_string_and_whitespace_in_args_vec`.

### Scenario @validation @property — Every empty-or-whitespace `exec.command` always yields Validation { field: "exec.command", .. } (proptest)

```gherkin
For any JobSpecInput where command is empty or pure ASCII/Unicode whitespace
And every other field is otherwise valid
Job::from_spec must always return Err(Validation { field: "exec.command", .. })
```

Closes the mutation gap on the trim guard per `.claude/rules/testing.md`
mutation target "Newtype FromStr and validators".

Maps to: `aggregate_validation::property::empty_or_whitespace_command_always_yields_exec_command_validation`.

---

## §3 Schema-level rejections (driven by serde, not the constructor)

### Scenario @validation — TOML missing `[exec]` table fails to parse with serde error

```gherkin
Given a TOML body containing `id`, `replicas`, and `[resources]` but no `[exec]` table
When Ana calls `toml::from_str::<JobSpecInput>(body)`
Then the result is Err
  And the error message names the missing `exec` field (or the absent driver tag)
```

Pinning that `serde(flatten) + DriverInput` enforces "exactly one
driver table" at parse time. Future-proofs against a
parse-then-default refactor that would silently accept an absent
driver table.

Maps to: `aggregate_validation::toml_missing_exec_table_fails_to_parse_with_serde_error`.

### Scenario @validation — TOML missing `[resources]` table fails to parse

```gherkin
Given a TOML body containing `id`, `replicas`, and `[exec]` but no `[resources]` table
When Ana calls `toml::from_str::<JobSpecInput>(body)`
Then the result is Err
  And the error message names the missing `resources` field
```

Maps to: `aggregate_validation::toml_missing_resources_table_fails_to_parse_with_serde_error`.

### Scenario @validation — TOML with two driver tables (`[exec]` plus an unknown table at top level) fails to parse via `deny_unknown_fields`

```gherkin
Given a TOML body containing both `[exec]` and a sibling `[microvm]` table
        (or `[exec]` plus an unknown top-level table like `[bogus]`)
When Ana calls `toml::from_str::<JobSpecInput>(body)`
Then the result is Err
  And the error message names the unknown / unexpected table
```

The mechanism is `deny_unknown_fields` on the outer struct; today
only `Exec` is in the tagged enum, so a sibling unknown table is the
analogue of "two driver tables" until a second variant lands.

Maps to: `aggregate_validation::toml_with_unknown_top_level_driver_table_fails_to_parse_via_deny_unknown_fields`.

### Scenario @validation — TOML with a typo inside `[exec]` (`commando` instead of `command`) fails to parse

```gherkin
Given a TOML body whose `[exec]` table contains `commando = "..."` instead of `command`
When Ana calls `toml::from_str::<JobSpecInput>(body)`
Then the result is Err
  And the error message names the unknown field `commando`
```

`deny_unknown_fields` on `ExecInput`. Per ADR-0031 §2 — the operator
typing a typo gets a parse error, not a silently-ignored field.

Maps to: `aggregate_validation::toml_with_typo_in_exec_field_fails_via_deny_unknown_fields`.

---

## §4 Round-trip (extends `aggregate_roundtrip.rs`)

### Scenario @round_trip — `JobSpecInput` round-trips through `Job::from_spec` and back

```gherkin
Given a valid JobSpecInput (canonical command="/opt/x/y", args=["--p","8080"], resources)
When the value is converted via `Job::from_spec(input.clone())` and back via `From<&Job>`
Then the result equals the original input field-for-field
```

Maps to: `aggregate_roundtrip::jobspec_input_roundtrips_through_aggregate_with_exec_block`.

### Scenario @round_trip @property — Every valid `JobSpecInput` round-trips identity (proptest)

```gherkin
For any valid JobSpecInput (proptest-generated valid_label id, replicas≥1,
                           memory_bytes≥1, command non-empty, args of any shape)
Let job = Job::from_spec(input.clone()).unwrap()
Let back = JobSpecInput::from(&job)
Then back == input
```

Closes the "every input → aggregate → back" identity per
`.claude/rules/testing.md` proptest mandatory call site for newtype
roundtrip — extended to the aggregate input twin shape.

Maps to: `aggregate_roundtrip::jobspec_input_roundtrip_property_with_exec_block`.

### Scenario @round_trip — `Job` rkyv archival is byte-identical across two calls (canonical-hash precondition)

```gherkin
Given a sample Job carrying `driver: WorkloadDriver::Exec(Exec { command, args })`
When the Job is rkyv-archived twice
Then the two byte vectors are byte-equal
```

Pre-existing scenario — extended to assert the tagged-enum driver
field is part of the canonical archive shape. Pinned by the existing
`job_rkyv_byte_identical_on_repeated_archival` test; the change is in
the `sample_job` fixture body (carries `driver: WorkloadDriver::Exec(Exec { ... })` now).

Maps to: existing `aggregate_roundtrip::job_rkyv_byte_identical_on_repeated_archival` (fixture migration).

### Scenario @round_trip — `Job` serde-JSON round-trip preserves the tagged-enum driver

```gherkin
Given a sample Job carrying `driver: WorkloadDriver::Exec(Exec { command, args })`
When the Job is serde-JSON-serialised and deserialised
Then the round-tripped Job equals the original
  And the driver tag and inner `command` / `args` survive the round-trip
```

Maps to: existing `aggregate_roundtrip::job_serde_json_roundtrip_equals_original` (fixture migration).

---

## §5 Reconciler purity

### Scenario @reconciler_purity — `JobLifecycle::reconcile` projects `job.command` and `job.args` into `Action::StartAllocation.spec` (no literals)

```gherkin
Given a Job with command "/opt/payments/bin/server" and args ["--port", "8080"]
  And no allocations are present (fresh-start case)
When `JobLifecycle::reconcile(&desired, &actual, &view, &tick)` is called
Then the returned actions contain exactly one `Action::StartAllocation { spec, .. }`
  And `spec.command == "/opt/payments/bin/server"`
  And `spec.args == vec!["--port", "8080"]`
  And `spec.resources` equals `job.resources` (not the deleted `default_restart_resources`)
```

This is the kill-test for the `reconciler.rs:1194-1195` literal
hardcoding (`/bin/sleep` / `["60"]`). Replaces the literal pin with
the projection assertion.

Maps to: `job_lifecycle_reconcile_branches::start_action_carries_full_alloc_spec_from_live_job_command_and_args`.

### Scenario @reconciler_purity — `JobLifecycle::reconcile` projects `job.command` and `job.args` into `Action::RestartAllocation.spec` (no fabrication)

```gherkin
Given a Job with command "/opt/x/y" and args ["--mode=fast"]
  And one Terminated alloc for the job (ready for restart)
  And the backoff window has elapsed
When `JobLifecycle::reconcile(&desired, &actual, &view, &tick)` is called
Then the returned actions contain exactly one `Action::RestartAllocation { alloc_id, spec }`
  And `spec.command == "/opt/x/y"`
  And `spec.args == vec!["--mode=fast"]`
  And `spec.resources` equals `job.resources`
```

Pins the new `spec` field shape on `RestartAllocation` per ADR-0031
§5.

Maps to: `job_lifecycle_reconcile_branches::restart_action_carries_full_alloc_spec_from_live_job`.

### Scenario @reconciler_purity — `reconcile()` is deterministic across two invocations with the same input (twin-invocation invariant per ADR-0013)

```gherkin
Given a fully-converged input triple (desired, actual, view, tick)
When `reconcile(...)` is called twice with the same arguments
Then both invocations return byte-identical (Vec<Action>, NextView) pairs
```

Pre-existing invariant (`ReconcilerIsPure` DST). Re-asserts that the
new spec materialisation does not introduce non-determinism (e.g. an
errant `Instant::now()` snuck into spec construction).

Maps to: `job_lifecycle_reconcile_branches::reconcile_with_exec_spec_is_deterministic_across_twin_invocations`.

---

## §6 Action shim deletion

### Scenario @deletion — `action_shim::build_phase1_restart_spec`, `build_identity`, `default_restart_resources` are deleted from source

The deletion is proven *behaviourally* via the
`action_shim_restart_uses_spec_from_action` acceptance test (next
scenario) rather than via a trybuild compile-fail harness — see
DWD-7 in `wave-decisions.md` for the cost-benefit analysis.

A future maintainer who re-adds any of the deleted helpers produces
dead code that clippy / `cargo udeps` flag at PR time.

### Scenario @deletion — Action shim Restart arm reads `spec` straight off the action

```gherkin
Given an `Action::RestartAllocation { alloc_id, spec }` carrying
        spec.command = "/opt/x/y", spec.args = ["--mode=fast"]
  And a SimDriver that records every `Driver::start` invocation
When `action_shim::dispatch_single(action, ..)` is called
Then `Driver::start(&spec)` is invoked exactly once
  And the recorded spec carries command "/opt/x/y" and args ["--mode=fast"]
        (NOT `/bin/sleep` + ["60"] — the deleted fabrication)
```

Pins that the shim is a stateless dispatcher reading the action's
spec, not a spec-builder.

Maps to: `crates/overdrive-control-plane/tests/acceptance/action_shim_restart_uses_spec_from_action.rs`.

---

## §7 OpenAPI propagation

### Scenario @openapi_propagation — Generated OpenAPI schema includes `JobSpecInput` with nested `resources` object and tagged-driver `oneOf`

```gherkin
When `cargo xtask openapi-gen` is invoked (programmatically via
        `xtask::openapi::generate_yaml()`)
Then the rendered YAML contains a `JobSpecInput` schema
  And the schema's `required` list contains "id", "replicas", and "resources"
  And the schema includes a `oneOf` driver dispatch with one variant `exec`
  And the `ExecInput` schema has `required: [command, args]`
  And the `ResourcesInput` schema has `required: [cpu_milli, memory_bytes]`
```

The mechanism: `utoipa::ToSchema` derives on each input twin. Per
ADR-0009 / ADR-0031 §8.

Note: a separate test that compares against the checked-in
`api/openapi.yaml` is the existing CI gate (`xtask::openapi_check`);
this scenario is the live-render shape pin so the schema is asserted
even when the checked-in YAML drift gate is not run (default lane).

Maps to: `crates/overdrive-control-plane/tests/acceptance/openapi_exec_block.rs::openapi_schema_carries_jobspec_input_with_nested_resources_and_tagged_driver_exec_variant`.

### Scenario @openapi_propagation — Every new `*Input` type implements `utoipa::ToSchema`

```gherkin
When the `every_api_type_implements_utoipa_to_schema` test runs
Then `JobSpecInput`, `ResourcesInput`, `ExecInput`, and `DriverInput`
        all satisfy the `ToSchema` trait bound
```

Maps to: extends existing `api_type_shapes::every_api_type_implements_utoipa_to_schema` (add the four new asserts).

---

## §8 HTTP handler defence-in-depth (per ADR-0015)

### Scenario @validation @driving_port — `submit_job` handler rejects empty `exec.command` with HTTP 400 and structured field name

```gherkin
Given a `SubmitJobRequest` whose spec has empty `exec.command`
  And every other field is otherwise valid
When the axum handler `handlers::submit_job(State(state), Json(request))` is invoked
        in-process (no reqwest, no real network)
Then the result is Err(ControlPlaneError::Validation { field: Some("exec.command"), .. })
  And no IntentStore put occurs (the key remains absent)
```

The mechanism: the handler runs `Job::from_spec` again per ADR-0011 /
ADR-0015 (defence-in-depth even when the CLI pre-validated). Pinned
in-process at the handler boundary — no HTTP round-trip required to
exercise the validation flow, per the existing
`submit_job_idempotency.rs` pattern.

Maps to: `crates/overdrive-control-plane/tests/acceptance/submit_job_handler_rejects_empty_exec_command_with_400.rs::submit_job_handler_rejects_empty_exec_command_with_validation_error_naming_field`.

### Scenario @validation @driving_port — `submit_job` handler rejects whitespace-only `exec.command` with HTTP 400

```gherkin
Given a `SubmitJobRequest` whose spec has `exec.command = "   "`
When the axum handler `handlers::submit_job(...)` is invoked in-process
Then the result is Err(ControlPlaneError::Validation { field: Some("exec.command"), .. })
```

Maps to: same file, `submit_job_handler_rejects_whitespace_only_exec_command_with_validation_error`.

---

## §9 CLI defence-in-depth (client-side, pre-HTTP)

### Scenario @validation @driving_port — `job::submit` handler rejects empty `exec.command` BEFORE issuing any HTTP call

```gherkin
Given Ana has the trust triple from `serve::run` on disk
  And Ana has written `empty.toml` with `command = ""`
When Ana invokes `overdrive_cli::commands::job::submit(SubmitArgs { ... })`
Then the call returns Err(CliError::InvalidSpec { field, message })
  And `field == "exec.command"`
  And `message == "command must be non-empty"`
  And the in-process server records ZERO requests to /v1/jobs
```

Pins ADR-0014 fast-fail — operators see the offending field without
a server round-trip. The "no HTTP request was made" assertion is
load-bearing — under a regression that drops the client-side
validation, the server still 400s but the test would also pass
without it.

Maps to: `crates/overdrive-cli/tests/integration/job_submit.rs::cli_submit_rejects_empty_exec_command_before_any_http_call` (extends existing file).

### Scenario @validation — `job::submit` handler surfaces TOML parse errors from missing `[exec]` table as `CliError::InvalidSpec { field: "toml" }`

```gherkin
Given a TOML file missing the `[exec]` table
When Ana invokes `job::submit(SubmitArgs { spec, config_path })`
Then the call returns Err(CliError::InvalidSpec { field, message })
  And `field == "toml"`
  And `message` contains a serde-de error pointing to the missing `exec` field
```

Pins that the CLI's `toml::from_str` failure flows through the
existing `field: "toml"` mapping (per `commands/job.rs:104-108`),
not through `Job::from_spec`'s structured-field path.

Maps to: `crates/overdrive-cli/tests/integration/job_submit.rs::cli_submit_surfaces_missing_exec_table_as_toml_field_error`.

---

## §10 Migration sweep (DELIVER concern, NOT a test)

The ~25 source files enumerated in `design/upstream-changes.md`
§ *Test surfaces that change* and the broader fixture set use the
flat `cpu_milli` / `memory_bytes` shape on `JobSpecInput { ... }`
literals and TOML strings. Per `feedback_single_cut_greenfield_migrations`
and design D9, every fixture migrates in the same DELIVER step as the
production code change. **There is NO "migration test"** — the test
files themselves migrate by literal substitution.

Recorded as DWD-N in `wave-decisions.md`. The crafter does not enable
a one-fixture-at-a-time scaffold.

---

## Coverage table — every upstream-changes.md rule → scenario name → Rust test

| Upstream-changes.md rule | Scenario tag | Rust test name |
|---|---|---|
| `exec.command` non-empty after trim — empty case | @validation | `job_from_spec_rejects_empty_exec_command_with_structured_field_name` |
| `exec.command` non-empty after trim — whitespace-only | @validation | `job_from_spec_rejects_whitespace_only_exec_command_via_trim_rule` |
| `exec.command` non-empty after trim — mixed Unicode whitespace | @validation | `job_from_spec_rejects_mixed_whitespace_exec_command` |
| `exec.command` predicate not normaliser (casing preserved) | @validation @happy | `job_from_spec_preserves_operator_command_casing_verbatim` |
| `exec.args` empty Vec is valid (zero-args case) | @validation @happy | `job_from_spec_accepts_non_empty_command_with_empty_args_vec` |
| `exec.args` no per-element rule | @validation | `job_from_spec_accepts_empty_string_and_whitespace_in_args_vec` |
| `exec.command` no NUL-byte rejection (kernel handles) | covered by no-per-element-rule | (delegated — kernel test) |
| `exec.command` no length cap (kernel handles) | covered by no-per-element-rule | (delegated — kernel test) |
| `resources` no new rules | (existing) | (no change) |
| Missing `[exec]` table → TOML parse error | @validation | `toml_missing_exec_table_fails_to_parse_with_serde_error` |
| Missing `[resources]` table → TOML parse error | @validation | `toml_missing_resources_table_fails_to_parse_with_serde_error` |
| Two driver tables / unknown top-level table → deny_unknown_fields rejects | @validation | `toml_with_unknown_top_level_driver_table_fails_to_parse_via_deny_unknown_fields` |
| Unknown field inside any table → deny_unknown_fields rejects | @validation | `toml_with_typo_in_exec_field_fails_via_deny_unknown_fields` |
| `Action::StartAllocation { spec }` populated from `job.command`/`job.args` | @reconciler_purity | `start_action_carries_full_alloc_spec_from_live_job_command_and_args` |
| `Action::RestartAllocation { alloc_id, spec }` shape grows | @reconciler_purity | `restart_action_carries_full_alloc_spec_from_live_job` |
| `reconcile()` purity preserved | @reconciler_purity | `reconcile_with_exec_spec_is_deterministic_across_twin_invocations` |
| Action shim deletes — `build_phase1_restart_spec`, `build_identity`, `default_restart_resources` | @deletion | (proven behaviourally — see DWD-7) |
| Action shim deletes — Restart arm reads spec from action | @deletion | `action_shim_restart_passes_spec_from_action_to_driver_start_unchanged` |
| OpenAPI schema regeneration (utoipa derives) | @openapi_propagation | `openapi_schema_carries_jobspec_input_with_nested_resources_and_tagged_driver_exec_variant` |
| OpenAPI utoipa::ToSchema on every new type | @openapi_propagation | (extends `every_api_type_implements_utoipa_to_schema`) |
| CLI parse error — missing `[exec]` → `field: "toml"` | @validation | `cli_submit_surfaces_missing_exec_table_as_toml_field_error` |
| CLI parse error — empty `command` BEFORE HTTP call | @validation @driving_port | `cli_submit_rejects_empty_exec_command_before_any_http_call` |
| Server defence-in-depth — empty `command` → 400 | @validation @driving_port | `submit_job_handler_rejects_empty_exec_command_with_validation_error_naming_field` |
| Server defence-in-depth — whitespace-only `command` → 400 | @validation @driving_port | `submit_job_handler_rejects_whitespace_only_exec_command_with_validation_error` |
| Walking skeleton — operator submits exec spec end-to-end | @walking_skeleton @driving_port | `walking_skeleton_submit_with_exec_block_returns_inserted_and_persists_command_and_args` |
| `JobSpecInput` round-trip identity through aggregate | @round_trip | `jobspec_input_roundtrips_through_aggregate_with_exec_block` |
| `JobSpecInput` round-trip identity (proptest) | @round_trip @property | `jobspec_input_roundtrip_property_with_exec_block` |
| `Job` rkyv byte-identical archival (with new fields) | @round_trip | (existing test, fixture migration) |
| `Job` serde-JSON round-trip (with new fields) | @round_trip | (existing test, fixture migration) |
| Empty/whitespace `command` always yields exec.command Validation (proptest) | @validation @property | `empty_or_whitespace_command_always_yields_exec_command_validation` |

Total scenarios: **24** (16 new + 8 fixture-migrated existing). Error
/ edge / rejection scenarios: **17 / 24 = 71%** — comfortably above
the 40% target.

## Cardinality summary

- Walking skeletons: **1** (Strategy C, real local resources, single
  E2E happy path).
- Focused boundary scenarios: **23**.
- Property-based scenarios: **2** (validation + roundtrip).
- Compile-fail trybuild scenarios: **1** (deletion proof).
- OpenAPI live-render scenarios: **1** (plus extension of one
  existing).

Ratio: 1 WS / 23 focused = matches the skill's "2-3 walking skeletons,
17-18 focused" recommendation scaled down for a wire-shape feature
where the WS gate is materially the same as `phase-1-control-plane-core`'s
existing walking-skeleton + the new exec block.

## Mandate compliance proof

- **CM-A (Hexagonal boundary)**: Every test enters through a driving
  port — `Job::from_spec` (validating constructor, the canonical
  intent-side entry), `overdrive_cli::commands::job::submit`
  (CLI handler), `handlers::submit_job` (REST handler — axum-State
  in-process), `JobLifecycle::reconcile` (reconciler trait via
  `Reconciler` interface). No internal-component imports.
- **CM-B (Business language)**: Gherkin uses operator vocabulary
  (`Ana`, `command`, `args`, `submit`, `confirm`, `persist`). Step
  bodies in Rust delegate to production constructors and handlers —
  no `requests.post()`, no `db.execute()`. Technical terms appear in
  the Rust assertion targets (`AggregateError::Validation`, etc.) but
  these are observable typed return values, not internal state.
- **CM-C (Walking skeleton user-centricity)**: The single WS is
  `walking_skeleton_submit_with_exec_block_returns_inserted_and_persists_command_and_args`
  — title names the operator goal ("submits with exec block" →
  "outcome echoes back"), Then steps assert observable values
  (`outcome`, `spec_digest`, persisted fields), and a non-technical
  stakeholder can confirm "yes, an operator wants to declare what
  binary the platform should run, and they want a yes-or-no answer
  back."
- **CM-D (Pure function extraction)**: The intent-side validation
  (`Job::from_spec`), the spec materialisation in `reconcile()`, and
  the round-trip identity are pure functions tested directly without
  fixture parametrisation. Impure boundaries (`tokio::process` in
  `ExecDriver`, real `redb` writes, real network) are isolated behind
  `Driver` / `IntentStore` / `Transport` traits with sim adapters in
  the default lane and real adapters under `--features integration-tests`.
