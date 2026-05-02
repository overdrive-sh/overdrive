# DESIGN Decisions — wire-exec-spec-end-to-end

Architect: Morgan. Mode: propose. Date: 2026-04-30. Companion ADR:
[ADR-0031](../../../product/architecture/adr-0031-job-spec-exec-block.md).
Upstream type-shape ADR: ADR-0030.

**Amended 2026-04-30** — see *Amendments — 2026-04-30* below for the
type-shape consistency revision (D3 / D5 / D11). User flagged that
`Job` was flat (`command`, `args`) while `JobSpecInput` was nested
with a tagged enum — three layers, two shapes. Architect ratified
B2 ("only Job nested; AllocationSpec stays flat") per ADR-0031
Amendment 1 + ADR-0030 §6. Decisions D3 and D5 below are
SUPERSEDED in part; new decision D11 records the `WorkloadDriver`
tagged enum on `Job`.

## Key Decisions

- [D1] **TOML wire shape: nested `[resources]` + `[exec]` tables; driver
  dispatch implicit-by-table-name.** Matches Nomad operator vocabulary;
  future `[microvm]`/`[wasm]` are additive variants of the same
  tagged-enum field. (see: ADR-0031 §1, §10 Alt A vs B vs C)
- [D2] **`JobSpecInput` reshaped: flat top-level (`id`, `replicas`),
  `resources: ResourcesInput`, `#[serde(flatten)] driver: DriverInput`
  with one variant `Exec(ExecInput)` today.** `deny_unknown_fields` on
  every struct + tagged enum enforces exactly-one driver table at parse
  time. (see: ADR-0031 §2)
- [D3] **SUPERSEDED 2026-04-30 by D11.** ~~`Job` aggregate grows
  `command: String` + `args: Vec<String>` as mandatory rkyv-archived
  fields. No newtypes — the driver passes these to
  `tokio::process::Command::new(impl AsRef<OsStr>).args(...)` which
  already constrains the type signature.~~ Replaced: `Job` carries a
  tagged-enum `driver: WorkloadDriver` field instead of flat
  `command`/`args` — see D11 below and ADR-0031 Amendment 1. The
  no-newtypes rationale carries through (the driver still passes the
  inner `Exec.command` / `Exec.args` to `Command::new` /
  `args(...)`); only the carrying type reshapes from flat fields on
  `Job` to a tagged-enum on `Job`. (see: ADR-0031 Amendment 1,
  §Compliance — Newtypes)
- [D4] **New validation rule at `Job::from_spec`: `exec.command`
  trimmed-non-empty.** Surfaces as
  `AggregateError::Validation { field: "exec.command", message: "command
  must be non-empty" }`. No NUL-byte rejection (kernel `execve(2)`
  enforces); no length cap (kernel `PATH_MAX` enforces); no per-element
  `args` rule. (see: ADR-0031 §4)
- [D5] **PARTIALLY SUPERSEDED 2026-04-30 by D11 (populating
  expression only; variant shape unchanged).** `Action::RestartAllocation`
  grows `spec: AllocationSpec`, mirrors `StartAllocation { spec }`, and
  `AllocationSpec` stays flat per ADR-0030 (this part of D5 is
  unchanged). What is superseded: the **populating expression** in
  `JobLifecycle::reconcile` no longer reads `job.command.clone()` /
  `job.args.clone()`; it destructures `&job.driver` as
  `WorkloadDriver::Exec(Exec { command, args })` and projects to the
  flat `AllocationSpec`. The shim's stateless-dispatcher contract
  (ADR-0023) is preserved verbatim — the action still carries flat
  `AllocationSpec`. (see: ADR-0031 §5, §10 Alt R2 vs Alt E, plus
  Amendment 1 *Action enum revision (Amendment 1 form)*)
- [D6] **`action_shim::build_phase1_restart_spec`,
  `action_shim::build_identity`, and `action_shim::default_restart_resources`
  delete in the same PR.** The Restart arm reads `spec` straight off
  the action; `find_prior_alloc_row` survives for `(job_id, node_id)`
  recovery on the obs-row write. (see: ADR-0031 §6)
- [D7] **Validating-constructor lane preserved (ADR-0011): both CLI and
  server route TOML/JSON parse → `JobSpecInput` → `Job::from_spec`.**
  The new `exec.command` rule fires on both lanes by construction.
  (see: ADR-0031 §7)
- [D8] **OpenAPI propagation via `utoipa::ToSchema` derives on
  `JobSpecInput`, `ResourcesInput`, `ExecInput`, `DriverInput`.** Schema
  regenerated via `cargo xtask openapi-gen`; CI gate `cargo xtask
  openapi-check` catches drift. (see: ADR-0031 §8)
- [D9] **Single-cut migration: no `#[serde(alias)]`, no compatibility
  shim, no two-shape acceptance period.** Per
  `feedback_single_cut_greenfield_migrations`. ~25 source files migrate
  in one PR; rkyv archive break is acceptable (no production data).
  (see: ADR-0031 §9)

## Amendments — 2026-04-30

User flagged shape inconsistency between the three layers
(`JobSpecInput` nested with tagged enum; `Job` originally flat;
`AllocationSpec` flat per ADR-0030). User chose "consistent shape."
Architect ratified Option **B2** ("only Job nested; AllocationSpec
stays flat as the driver-trait input contract") per ADR-0031
Amendment 1. Rationale lives in ADR-0031 Amendment 1 § Rationale;
the headline reason is ADR-0030 §6's already-ratified Phase 2+
posture for `AllocationSpec` (per-driver-class spec types, NOT a
discriminator field on a shared spec) — Option B1 would have
prematurely foreclosed that shape.

- [D11] **`Job` carries `driver: WorkloadDriver` tagged enum; intent
  layer mirrors wire layer; `AllocationSpec` stays flat.** Replaces D3
  (flat `command`/`args` on `Job`). Adds new types `WorkloadDriver`
  enum and `Exec` struct in `crates/overdrive-core/src/aggregate/mod.rs`.
  `WorkloadDriver::Exec(Exec { command, args })` is the single Phase 1
  variant; future `WorkloadDriver::MicroVm(MicroVm)` and
  `WorkloadDriver::Wasm(Wasm)` are additive. Match exhaustiveness on
  `WorkloadDriver` at every reconciler/shim site is the static
  guarantee — the compiler enforces multi-driver coverage when new
  variants land. The naming choice `WorkloadDriver` (not bare `Driver`)
  disambiguates from the `Driver` trait at
  `crates/overdrive-core/src/traits/driver.rs` (per ADR-0030 §1); the
  inner-struct name `Exec` (not `ExecSpec` / `ExecInvocation`) reads
  cleanest under the qualified path `WorkloadDriver::Exec(Exec)`.
  (see: ADR-0031 Amendment 1, all sub-sections)
- [D12] **Validation field name `exec.command` UNCHANGED.** Operator-
  facing path is preserved (matches the TOML the operator typed). The
  Rust-shape nesting `job.driver` → `WorkloadDriver::Exec` →
  `Exec.command` is internal; surfacing `driver.exec.command` to
  operators would leak internal type structure into operator-facing
  diagnostics for no operator benefit. (see: ADR-0031 Amendment 1
  § Validation field name)
- [D13] **Reuse Analysis update: `JobSpecInput` projection.**
  `Job::from_spec` does the wire-shape `DriverInput → WorkloadDriver`
  projection as part of construction (existing `from_spec` body
  extended; not a new constructor — ADR-0011's THE-single-validating-
  constructor invariant is preserved). `From<&Job> for JobSpecInput`
  projects `WorkloadDriver → DriverInput` for the describe round-trip;
  the existing `aggregate_roundtrip` proptest extends to cover the
  enum shape rather than flat fields.
- [D14] **rkyv archive layout reshape.** The `Job` archive layout
  changes: one `WorkloadDriver` enum field instead of two flat fields
  (vs the now-superseded D3). The hash-break posture (single-cut
  greenfield migration; no production data; test snapshots
  regenerate in the same PR) is unchanged from D3 — only the bytes
  archived differ.
- [D15] **`AllocationSpec` UNCHANGED — single-cut deferral to
  ADR-0030's predicted Phase 2+ shape.** The amendment explicitly
  does NOT touch `AllocationSpec` per ADR-0030 §6. The Phase 2+
  per-driver-class spec split (`Spec::Exec(ExecSpec) |
  Spec::MicroVm(MicroVmSpec) | Spec::Wasm(WasmSpec)`) remains the
  predicted shape and a future ADR's concern. The reconciler's
  projection from `&job.driver` to flat `AllocationSpec` is one
  destructure today; when Phase 2+ adds variants the destructure
  becomes a `match` and each arm projects to its per-driver-class
  spec — at which point the `Spec` enum shape becomes the natural
  next ADR.

## Architecture Summary

- **Pattern**: Modular monolith + ports-and-adapters (unchanged from
  brief.md §1). This feature is a wire/intent shape extension within the
  existing core/host/sim crate topology.
- **Paradigm**: OOP (per project CLAUDE.md).
- **Key components touched**: `JobSpecInput`, `ResourcesInput` (new),
  `ExecInput` (new), `DriverInput` (new tagged enum), `Job` aggregate,
  `Action::StartAllocation` (populating expression only),
  `Action::RestartAllocation` (variant shape), `action_shim::dispatch_single`
  (Restart arm), `JobLifecycle::reconcile` (literal substitution).
  `ExecDriver`, `AllocationSpec`, and `Resources` are unchanged in shape
  — the upstream type-shape work landed in ADR-0030.

## Reuse Analysis

| Existing Component | File | Overlap | Decision | Justification |
|---|---|---|---|---|
| `JobSpecInput` (`id`, `replicas`) | `crates/overdrive-core/src/aggregate/mod.rs:125-131` | top-level scalars survive | EXTEND | `id` and `replicas` retained verbatim; `cpu_milli`/`memory_bytes` move into the new `resources` field. |
| `Resources` rkyv intent type | `crates/overdrive-core/src/driver.rs:104-107` | resource envelope already exists | EXTEND (intent side) | `Resources` stays the rkyv-archived intent shape; `ResourcesInput` is the wire-side input twin (state-layer hygiene per `development.md`). `From<ResourcesInput> for Resources` carries the conversion. |
| `ResourcesInput` | none today | wire-shape twin of `Resources` | CREATE NEW | Mixing serde+`utoipa::ToSchema` directly onto `Resources` would conflate intent and wire. The input twin is ~10 LOC + a `From` impl; the price of state-layer hygiene. (ADR-0031 §Reuse) |
| `ExecInput` | none today | exec-driver invocation fields | CREATE NEW | No existing artifact carries exec-driver-specific operator fields. The struct is the natural carrier for `command + args`. (ADR-0031 §Reuse) |
| `DriverInput` (tagged enum) | none today | driver dispatch on `JobSpecInput` | CREATE NEW | Tagged-enum-with-`#[serde(flatten)]` is the mechanism that makes the table implicit-by-name. One variant (`Exec(ExecInput)`) today; future drivers extend additively without restructuring. Alt B (`driver = "exec"` field + generic `[config]`) and Alt C (`[driver.exec]` nesting) were rejected — see ADR-0031 §10. |
| `Job` aggregate | `crates/overdrive-core/src/aggregate/mod.rs` | identity/scale/resource fields exist | EXTEND | Two new mandatory rkyv-archived fields (`command`, `args`) — the smallest possible extension. No new aggregate type. |
| `Job::from_spec` validating constructor | `crates/overdrive-core/src/aggregate/mod.rs:97` | THE single intent-side path (ADR-0011) | EXTEND | New `exec.command` non-empty rule slots into the existing constructor body; `AggregateError::Validation { field, message }` already absorbs the new rule with `field: "exec.command"`. |
| `From<&Job> for JobSpecInput` | `crates/overdrive-core/src/aggregate/mod.rs:141-150` | round-trip projection | EXTEND | Adds `command`, `args`, and `resources` to the projection. `aggregate_roundtrip.rs` proptest extends. |
| `Action::StartAllocation { spec, ... }` | `crates/overdrive-core/src/reconciler.rs:498-510` | already carries `spec` | EXTEND (populating expression only) | No enum shape change. The hardcoded `/bin/sleep` / `["60"]` literals at `reconciler.rs:1194-1195` become `job.command.clone()` / `job.args.clone()`. |
| `Action::RestartAllocation { alloc_id }` | `crates/overdrive-core/src/reconciler.rs:523-527` | restart variant | EXTEND (grow variant) | One new field: `spec: AllocationSpec`. Mirrors `StartAllocation`. The reconciler has the live `Job` in scope; constructing the spec there preserves the shim's stateless dispatcher contract (ADR-0023). |
| `action_shim::dispatch_single` Restart arm | `crates/overdrive-control-plane/src/action_shim.rs` | restart dispatch path | EXTEND (read spec from action) | `find_prior_alloc_row` still needed for `(job_id, node_id)` recovery on the obs-row write; only the spec-rebuild path goes away. |
| `action_shim::build_phase1_restart_spec` | `action_shim.rs:223-231` | placeholder spec builder | DELETE | Production code carrying test-fixture intent. The Restart action carries the spec. |
| `action_shim::build_identity` | `action_shim.rs:239-247` | duplicate of `core::reconciler::mint_identity` | DELETE | The reconciler-side derivation is the SSOT; the shim needs no copy. |
| `action_shim::default_restart_resources` | `action_shim.rs:255-257` | fabricated `100mCPU/256MiB` envelope | DELETE | Live `Job.resources` is authoritative; the action carries it. |
| `default_restart_resources_pins_exact_values` test | `action_shim.rs:284-317` | pins the deleted function | DELETE | Per `feedback_delete_dont_gate.md`: production code becomes unused → delete code AND its test in the same PR. |
| CLI parse path | `crates/overdrive-cli/src/commands/job.rs:104-113` | `toml::from_str::<JobSpecInput>` → `Job::from_spec` | EXTEND (no logic change) | serde drives the new tagged-enum + nested-table shape transparently. |
| Server submit path | `crates/overdrive-control-plane/src/handlers.rs::submit_job` | JSON deserialise → `Job::from_spec` | EXTEND (no logic change) | Server-side defence-in-depth runs the same constructor (ADR-0015). New rules fire on both lanes by construction. |
| OpenAPI schema | `api/openapi.yaml` (auto-regenerated) | utoipa-derived | REGENERATE | `cargo xtask openapi-gen` regenerates; `cargo xtask openapi-check` (CI gate per ADR-0009) catches drift. |
| Test fixtures (TOML / `JobSpecInput { ... }` literals) | ~25 source files (enumerated in upstream-changes.md §test surfaces) | flat-shape literals | MIGRATE single-cut | Per `feedback_single_cut_greenfield_migrations`: every fixture migrates in the same PR. Each diff is mechanical literal-by-literal substitution. |

Net new types: **3** (`ResourcesInput`, `ExecInput`, `DriverInput`). All
other surfaces extend. Net deletions: **3 functions + 1 test** in the
action shim.

## Technology Stack

- Rust 2024 / no new dependencies.
- TOML — existing `toml` crate, no version change.
- `serde` + `utoipa::ToSchema` — existing, used for the new
  `ResourcesInput` / `ExecInput` / `DriverInput` derives.
- `rkyv` — existing, picks up the two new `Job` fields via existing
  derive macros.

## Constraints Established

- **No two-shape acceptance period for `JobSpecInput`** — single-cut
  migration per `feedback_single_cut_greenfield_migrations`. No
  `#[serde(alias = "cpu_milli")]`, no compatibility shim.
- **`Job::from_spec` is THE single validating constructor** — per
  ADR-0011. New `exec.command` rule slots in alongside existing
  replicas/memory rules; no new constructor, no new error variant.
- **`Action` data carries everything the shim needs** — per ADR-0023.
  The Restart variant grows `spec` rather than threading
  `&dyn IntentStore` into `dispatch`; the shim's stateless-dispatcher
  contract is preserved.
- **`reconcile` remains pure** — per ADR-0013. Spec materialisation is
  a deterministic projection of the hydrated `Job`; no `.await`, no
  `Instant::now`, no I/O in `reconcile`.
- **State-layer hygiene** — per `.claude/rules/development.md` §
  State-layer hygiene. `JobSpecInput` / `ResourcesInput` / `ExecInput`
  / `DriverInput` are wire-shape; `Job` / `Resources` / `AllocationSpec`
  are intent-shape. Input twins keep the rkyv-archived intent surface
  clean of serde-only concerns.
- **Newtypes — STRICT by default does NOT mandate a `Command` newtype**
  — per `.claude/rules/development.md` § Newtypes. The driver passes
  `command`/`args` to `tokio::process::Command::new(impl AsRef<OsStr>)
  .args(impl IntoIterator<Item = impl AsRef<OsStr>>)` — the type
  signature already constrains them; the validation rule is
  constructor-side, not type-side.
- **Hashing determinism preserved** — per `.claude/rules/development.md`
  § Hashing. The `Job` rkyv archive grows two fields; per single-cut
  greenfield, no in-production data exists. Test snapshots with pinned
  `spec_digest` values regenerate in the same PR.
- **Phase 1 single-node scope honoured** — per
  `feedback_phase1_single_node_scope`. No node registration, no
  taint/toleration, no multi-region. Cgroup isolation per ADR-0026 /
  ADR-0028 remains in scope.

## Upstream Changes

None — DESIGN-first feature. There is no DISCUSS-wave artifact for
`wire-exec-spec-end-to-end`; the user-supplied TOML shape is the design
seed. ADR-0030 is the upstream type-shape decision (it ratified
`AllocationSpec { command, args }` on the internal driver surface);
this feature wires the operator surface and the action enum that
populate that internal shape from authoritative intent.

Hand-off to DISTILL is captured in
[upstream-changes.md](./upstream-changes.md) — the new validation rules,
observable behaviour deltas, and test surfaces DISTILL must enumerate
scenarios for.
