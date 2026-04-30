# Upstream Changes — wire-exec-spec-end-to-end

Hand-off to the DISTILL acceptance-designer. Lists the new validation
rules, observable behaviour deltas, and test surfaces this design
introduces. Source of truth:
[`docs/product/architecture/adr-0031-job-spec-exec-block.md`](../../../product/architecture/adr-0031-job-spec-exec-block.md).

**Amended 2026-04-30** — ADR-0031 received Amendment 1 introducing a
tagged-enum `WorkloadDriver` field on the `Job` aggregate (replacing
the originally-flat `command`/`args` fields). The wire shape
(`JobSpecInput`, `DriverInput`, `ExecInput`, `ResourcesInput`) is
unchanged; the validation rule predicate is unchanged; the validation
**field name** `exec.command` is unchanged (operator-facing path is
preserved against the TOML the operator typed). The `AllocationSpec`
driver-trait input also stays flat per ADR-0030 §6. Only the **intent
aggregate** reshapes — `Job` carries `driver: WorkloadDriver` carrying
`WorkloadDriver::Exec(Exec { command, args })`. See ADR-0031 Amendment
1 § Migration impact for the downstream-propagation enumeration. The
sections below are factually correct against Amendment 1 except where
they show literal-field-on-Job patterns; treat any `Job.command` /
`Job.args` reference in the test-surface enumeration as
`job.driver` destructured into `WorkloadDriver::Exec(Exec { command,
args })`.

## Wire-shape change (operator-facing)

**Before** (Phase 1, flat):

```toml
id           = "payments"
replicas     = 1
cpu_milli    = 500
memory_bytes = 134217728
```

`command` / `args` are not expressible; the reconciler hardcodes
`/bin/sleep` + `["60"]`.

**After** (this design):

```toml
id = "payments"
replicas = 1

[resources]
cpu_milli    = 500
memory_bytes = 134217728

[exec]
command = "/opt/payments/bin/payments-server"
args    = ["--port", "8080"]
```

Top-level scalars: `id`, `replicas`. Two mandatory tables —
`[resources]` and `[exec]`. Driver dispatch is implicit-by-table-name;
future `[microvm]` / `[wasm]` are additive variants. Exactly one driver
table per spec is enforced by serde at parse time
(`deny_unknown_fields` + tagged-enum). Full rationale: ADR-0031 §1, §10
(Alternatives A vs B vs C).

## New validation rules at `Job::from_spec`

Each fires at the validating constructor (ADR-0011) and surfaces as
`AggregateError::Validation { field, message }` on both CLI and server
lanes:

- **`exec.command` — non-empty after trim.** Rejects `""` and
  whitespace-only.
  `field: "exec.command"`, `message: "command must be non-empty"`.
- **`exec.args` — no per-element rule.** Empty `Vec` is valid (the
  zero-args case for binaries that take no arguments). NUL-byte
  rejection is NOT added — the kernel's `execve(2)` rejects
  NUL-bearing argv with `EINVAL` and the driver surfaces it as
  `DriverError::StartRejected` per ADR-0023; a platform-side cap
  before the kernel sees the value would diverge from the kernel's
  posture for no safety benefit.
- **`exec.command` length cap — none added.** `Command::new` accepts
  any `OsStr`; the Linux `PATH_MAX` is 4096 and the kernel enforces
  at `execve(2)`.
- **`resources` — no new rules.** Existing rules unchanged
  (`replicas != 0`, `memory_bytes != 0`, `cpu_milli` allowed zero per
  ADR-0026 cgroup `cpu.weight` derivation handling).

Schema-level rejections (driven by serde, not by the constructor):

- Missing `[exec]` table → TOML/JSON parse error → `CliError::InvalidSpec
  { field: "toml" }` on CLI lane / `ControlPlaneError::Validation
  { field: "spec" }` on server lane (ADR-0015 RFC 7807 body).
- Missing `[resources]` table → same pattern.
- Two driver tables (`[exec]` + `[microvm]`) or unknown top-level
  table → `deny_unknown_fields` + tagged-enum dispatch rejects at
  deserialise time.
- Unknown field inside any table (e.g. `commando = "..."`) →
  `deny_unknown_fields` rejects.

## `Action::Spawn` shape change

`Action::StartAllocation { spec: AllocationSpec, ... }` is unchanged in
shape — only the populating expression in `JobLifecycle::reconcile`
changes. Per ADR-0031 Amendment 1, the populating expression
destructures `&job.driver` into `WorkloadDriver::Exec(Exec { command,
args })` and constructs the flat `AllocationSpec` from the inner
fields. `AllocationSpec` itself stays flat per ADR-0030 §6.

`Action::RestartAllocation { alloc_id }` **grows** to
`Action::RestartAllocation { alloc_id, spec: AllocationSpec }` (still
flat `AllocationSpec`). The reconciler has the live `Job` in scope at
emit time; constructing the spec there (via the same
`WorkloadDriver::Exec(Exec { command, args })` destructure) is cheap,
pure, and matches the action shim's
"carry-everything-the-shim-needs" contract (ADR-0023). The action shim's
`build_phase1_restart_spec`, `build_identity`, and
`default_restart_resources` delete; the Restart arm reads `spec`
straight off the action. `find_prior_alloc_row` survives — it is still
needed to recover `(job_id, node_id)` for the `AllocStatusRow` write,
but only the spec-rebuild path goes away.

## Test surfaces that change in this feature

Verified to exist via Glob 2026-04-30. DISTILL must enumerate new
scenarios for each:

**`overdrive-core` acceptance**:

- `crates/overdrive-core/tests/acceptance/aggregate_roundtrip.rs` —
  proptest extends to cover `command`, `args`, `resources` round-trip.
- `crates/overdrive-core/tests/acceptance/aggregate_validation.rs` —
  new scenarios for empty `command`, missing `[exec]`, missing
  `[resources]`, two-driver-tables.
- `crates/overdrive-core/tests/acceptance/aggregate_constructors.rs` —
  fixture migration to nested shape; new positive case for
  non-empty `command` + non-empty `args`.
- `crates/overdrive-core/tests/acceptance/job_lifecycle_reconcile_branches.rs`
  — branch coverage for the new `spec` populating expression on
  `Action::StartAllocation` and the new `spec` field on
  `Action::RestartAllocation`.

**`overdrive-cli` integration**:

- `crates/overdrive-cli/tests/integration/walking_skeleton.rs` —
  end-to-end TOML → submit → describe round-trip on the new shape.
- `crates/overdrive-cli/tests/integration/job_submit.rs` — empty
  `command` produces a structured `exec.command` error before any
  HTTP call.

**`overdrive-control-plane` acceptance**:

- `crates/overdrive-control-plane/tests/acceptance/api_type_shapes.rs`
  — `JobSpecInput` JSON shape, `ResourcesInput` / `ExecInput` /
  `DriverInput` schemas, OpenAPI regeneration.
- `crates/overdrive-control-plane/tests/acceptance/runtime_convergence_loop.rs`
  — Spawn carries `job.command` / `job.args` (not `/bin/sleep`).
- `crates/overdrive-control-plane/tests/acceptance/submit_job_idempotency.rs`
  — fixture migration; idempotency property unchanged.
- `crates/overdrive-control-plane/tests/acceptance/job_lifecycle_backoff.rs`
  — Restart carries `spec` populated from the live `Job`.

## CLI parse-error surface (operator-readable)

A TOML missing `[exec]`:

```
$ overdrive job submit broken.toml
Error: spec invalid: failed to parse TOML: missing field `exec`
```

A TOML with empty `command = ""`:

```
$ overdrive job submit empty-cmd.toml
Error: spec invalid: exec.command: command must be non-empty
```

Both fire **before** the HTTP call (client-side validation per
ADR-0014).

## OpenAPI schema regeneration

`cargo xtask openapi-gen` regenerates `api/openapi.yaml`. The schema
carries:

- `JobSpecInput` — `required: [id, replicas, resources]` plus the
  flattened `oneOf` driver dispatch (currently single-variant `exec`).
- `ResourcesInput` — `required: [cpu_milli, memory_bytes]`.
- `ExecInput` — `required: [command, args]` (note: `args` is required
  even when empty — an absent `args` field is a parse error, not
  "default to no args").
- `DriverInput` — `oneOf` with one entry today, externally tagged via
  serde-rename (`exec`).

`cargo xtask openapi-check` (CI gate per ADR-0009) catches drift.

## Out of scope (do NOT add tests for these)

- **Multi-driver (`microvm`, `wasm`)** — Phase 2+. ADR-0031 §"Future
  work". The tagged-enum shape on `DriverInput` makes future variants
  additive; this PR ships one variant (`Exec`).
- **Container image resolution / registries / content hashes for
  binaries** — separate concern; `command` is a host filesystem path
  per ADR-0030.
- **Argv sanitisation / command allowlists / denylists on `command`**
  — security boundary is SPIFFE + LSM + cgroup envelope (ADR-0026 /
  ADR-0028), not the spec. ADR-0031 §"Future work".
- **Forward-compat spec extension policy** — Phase 2+ may revisit
  `deny_unknown_fields` posture. ADR-0031 §"Future work".
- **`overdrive job describe` render goldens for the new sections** —
  the rendered output adds two new sections ("Resources" and "Exec")
  but render goldens are not in the committed scope; informational
  only unless DISTILL elects to pin them.
