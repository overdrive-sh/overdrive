# ADR-0031 — Job spec wire shape: `[resources]` + `[exec]` tables; tagged-driver `JobSpecInput`; `Spawn`/`Restart` actions carry validated `AllocationSpec`

## Status

Accepted. 2026-04-30. Decision-makers: User-proposed during
`wire-exec-spec-end-to-end` DESIGN wave (Option A); architect-agent
authored. Tags: phase-1, application-arch, wire-shape, type-shape,
operator-surface.

**Amended 2026-04-30 (Amendment 1).** §3 (Job aggregate extension), §4
(validation rule field name), §5 (Action enum revision), §6 (action
shim deletions), §10 (C4 component diagram) are SUPERSEDED by
Amendment 1 below. The amendment makes `Job` carry a tagged-enum
`driver: WorkloadDriver` field (mirroring `JobSpecInput.driver:
DriverInput`) instead of flat `command`/`args` fields. AllocationSpec
remains flat per ADR-0030 §6 — the per-driver-class spec split is
the Phase 2+ shape ADR-0030 ratified, not a discriminator on a
shared spec. The historical §§3-6, §10 below remain as supersession
record; Amendment 1 is the new SSOT for the affected decisions.

## Amendment 1 — 2026-04-30: tagged-enum `WorkloadDriver` on `Job` for type-shape consistency across wire / intent layers

**Decision-maker.** User-flagged shape inconsistency between
`JobSpecInput` (wire — nested with tagged-enum `DriverInput`),
`Job` (intent — was flat `command`/`args` per original §3),
`AllocationSpec` (driver-trait input — flat per ADR-0030).
User chose "consistent shape." Architect ratified: `Job` joins
`JobSpecInput` in the nested-with-tagged-enum shape; `AllocationSpec`
stays flat per ADR-0030 §6 (per-driver-class spec types are the
Phase 2+ shape ADR-0030 already committed to).

**What changed.** `Job` carries `driver: WorkloadDriver` instead of
flat `command: String` + `args: Vec<String>`. The existing wire-shape
`DriverInput` projects into `WorkloadDriver` at `Job::from_spec`. The
existing flat `AllocationSpec { command, args, ... }` is unchanged;
the action shim still consumes `Driver::start(&AllocationSpec)` per
ADR-0030. The reconciler's spec-population at action-emit time
projects from `job.driver.exec()` (or future variants) to flat
`AllocationSpec` — a one-line projection.

### Rationale

1. **ADR-0030 §6 is explicit on the Phase 2+ shape for `AllocationSpec`.**
   ADR-0030 §6 ratified the shared `AllocationSpec` as a Phase 1
   simplification because only one driver class exists; the predicted
   Phase 2+ shape is **per-driver-class spec types** (a future `Spec`
   enum with `Spec::Exec(ExecSpec) | Spec::MicroVm(MicroVmSpec) |
   Spec::Wasm(WasmSpec)`), NOT a discriminator on a shared spec.
   Adding a `WorkloadDriver` enum field to `AllocationSpec` now would
   prematurely foreclose ADR-0030's predicted shape.
2. **Two different boundaries, two different forces.** `Job` lives at
   the intent-vs-wire boundary — operators type-check against the
   schema, structural exclusivity matters, the wire/intent shapes
   should mirror each other. `AllocationSpec` lives at the
   intent-vs-driver-trait boundary — at trait-dispatch time the
   implementing driver knows its own class (the `impl Driver for
   ExecDriver` is the discriminator), and a discriminator field on the
   spec adds nothing the trait dispatch doesn't already encode.
3. **`make invalid states unrepresentable` (development.md) cuts both
   ways.** On `Job`, the original §3 shape (`Job { command: String,
   args: Vec<String> }`) is fine for the single-driver case but Phase
   2+ would force either per-driver flat `Option` fields (where
   `Job { command: Option<String>, microvm_image: Option<ContentHash> }`
   makes both-Some representable — invalid states representable) or a
   late refactor touching every reconciler/shim site reading
   `job.command`. The amendment makes the exclusivity structural at
   the intent layer immediately.
4. **Don't design for hypothetical future requirements (CLAUDE.md).**
   Applies on the AllocationSpec side, not the Job side. The Job
   reshape is responsive to a present requirement (wire-shape
   consistency the user explicitly named). The AllocationSpec
   reshape would be speculative — ADR-0030 already ratified what its
   Phase 2+ shape will be, and that shape is not a `WorkloadDriver`
   discriminator.

### `Job` aggregate (BEFORE → AFTER)

**BEFORE** (original §3, now SUPERSEDED):

```rust
pub struct Job {
    pub id:        JobId,
    pub replicas:  NonZeroU32,
    pub resources: Resources,
    pub command:   String,        // ← flat, removed by Amendment 1
    pub args:      Vec<String>,   // ← flat, removed by Amendment 1
}
```

**AFTER** (Amendment 1):

```rust
pub struct Job {
    pub id:        JobId,
    pub replicas:  NonZeroU32,
    pub resources: Resources,
    pub driver:    WorkloadDriver,    // ← NEW; tagged enum
}

/// Validated intent-side counterpart to wire-shape `DriverInput`.
/// One variant per driver class; new variants append in Phase 2+
/// (`MicroVm(MicroVm)`, `Wasm(Wasm)`).
///
/// Naming: `WorkloadDriver`, not `Driver`, to disambiguate from the
/// `Driver` *trait* at `crates/overdrive-core/src/traits/driver.rs`
/// (per ADR-0030 §1). The trait is the driver implementation surface
/// (`Driver::start(&AllocationSpec)`); this enum is the operator's
/// declared driver-class intent on the Job aggregate.
pub enum WorkloadDriver {
    Exec(Exec),
    // Future Phase 2+: MicroVm(MicroVm), Wasm(Wasm).
}

/// Exec-driver invocation fields. Mirror `ExecInput` on the wire side.
/// Naming: bare `Exec`, not `ExecSpec` / `ExecInvocation` — the
/// `WorkloadDriver::Exec(Exec)` qualified path disambiguates from
/// the `[exec]` TOML table identifier and from the `ExecDriver` trait
/// impl in `overdrive-worker`. The bare noun reads cleanest in context.
pub struct Exec {
    pub command: String,
    pub args:    Vec<String>,
}
```

Both `WorkloadDriver` and `Exec` derive the same set as the rest of
the aggregate (`Debug, Clone, PartialEq, Eq, rkyv::Archive,
rkyv::Serialize, rkyv::Deserialize`). The rkyv archive layout for
`Job` changes shape (one `WorkloadDriver` enum field instead of two
flat fields) — the hash break per single-cut greenfield migration
discipline (development.md § Hashing) is unchanged in posture from
the original ADR; the bytes archived are different but the migration
shape (regenerate test snapshots, no production data) is identical.

### `AllocationSpec` — UNCHANGED

```rust
// crates/overdrive-core/src/traits/driver.rs — unchanged from ADR-0030
pub struct AllocationSpec {
    pub alloc:     AllocationId,
    pub identity:  SpiffeId,
    pub command:   String,
    pub args:      Vec<String>,
    pub resources: Resources,
}
```

ADR-0030's per-driver-class Phase 2+ posture is preserved verbatim.
When MicroVm and Wasm drivers land, the shared `AllocationSpec` will
likely split into per-driver-class spec types (or a `Spec` enum) per
ADR-0030 §6 — this amendment does not pre-empt that future ADR.

### Validation field name — UNCHANGED (`exec.command` stays)

The validation rule continues to surface as `AggregateError::Validation
{ field: "exec.command", message: "command must be non-empty" }`. The
field name is the **operator-facing path through the spec** per
ADR-0011 — operators read it against the TOML they typed
(`[exec]\ncommand = "..."` reads as `exec.command`). The Rust-shape
nesting (`job.driver` → `WorkloadDriver::Exec` → `Exec.command`) is
internal; surfacing `driver.exec.command` to operators would leak
internal type structure into operator-facing diagnostics for no
operator benefit. The shim from `JobSpecInput.driver` to
`Job.driver` is also a Rust-shape nesting — neither shape is what
the operator typed.

### `Job::from_spec` projection (illustrative)

```rust
impl Job {
    pub fn from_spec(input: JobSpecInput) -> Result<Self, AggregateError> {
        // existing rules: replicas != 0, memory_bytes != 0, JobId parse
        // (...)

        let driver = match input.driver {
            DriverInput::Exec(exec_input) => {
                let trimmed = exec_input.command.trim();
                if trimmed.is_empty() {
                    return Err(AggregateError::Validation {
                        field: "exec.command".to_string(),
                        message: "command must be non-empty".to_string(),
                    });
                }
                WorkloadDriver::Exec(Exec {
                    command: exec_input.command,
                    args:    exec_input.args,
                })
            }
        };

        Ok(Self {
            id, replicas, resources, driver,
        })
    }
}
```

Future driver variants append to the `match` exhaustively — the
compiler enforces that a new `DriverInput::MicroVm` variant either
gets a `match` arm or fails the build, which is the static guarantee
the tagged-enum shape buys. Match exhaustiveness on `WorkloadDriver`
likewise catches every reconciler/shim site if a future amendment
adds a variant — no `if job.command.is_some()` style checks scattered
through the codebase.

### `Action` enum revision (Amendment 1 form)

`Action::StartAllocation { spec: AllocationSpec, ... }` and
`Action::RestartAllocation { alloc_id, spec: AllocationSpec }` are
unchanged in shape from the original §5 — both still carry flat
`AllocationSpec` (per ADR-0030; per Amendment 1 rationale point 1).
What changes is the **populating expression** in
`JobLifecycle::reconcile`:

```rust
// BEFORE (original §5, Amendment 1 supersedes)
let spec = AllocationSpec {
    alloc, identity,
    command: job.command.clone(),    // ← flat field access
    args:    job.args.clone(),
    resources: job.resources.clone(),
};

// AFTER (Amendment 1)
let WorkloadDriver::Exec(Exec { command, args }) = &job.driver;  // exhaustive today
let spec = AllocationSpec {
    alloc, identity,
    command:   command.clone(),
    args:      args.clone(),
    resources: job.resources.clone(),
};
```

When Phase 2+ adds `WorkloadDriver::MicroVm` or `::Wasm` variants,
the irrefutable `let` becomes a `match` and each arm projects to its
per-driver-class spec — at which point ADR-0030 §6's predicted
`Spec::Exec/MicroVm/Wasm` per-driver-class spec split is the natural
shape, and `Action::StartAllocation { spec: Spec }` becomes the
typed dispatcher. That is a future ADR's concern; today the
projection is one line.

### Action shim deletions — UNCHANGED

`build_phase1_restart_spec`, `build_identity`,
`default_restart_resources`, and `default_restart_resources_pins_exact_values`
all delete in the same PR per original §6. The Restart arm reads
`spec` straight off the action; only the projection at action-emit
time inside `reconcile` differs from the original ADR.

### C4 component diagram (Amendment 1 form)

The data-flow types reshape inside `JobAgg`. The flow shape and node
identities are otherwise unchanged from §10.

```mermaid
flowchart TB
    Operator["Operator (CLI user)"]
    TOML["job.toml<br/>id / replicas / [resources] / [exec]"]
    CLI["overdrive-cli<br/>commands::job::submit"]
    JSI1["JobSpecInput<br/>driver: DriverInput<br/>(client-side)"]
    JFromSpec1["Job::from_spec<br/>(client-side validation)"]
    HTTP["POST /v1/jobs<br/>(JSON body: JobSpecInput)"]
    Handler["overdrive-control-plane<br/>handlers::submit_job"]
    JSI2["JobSpecInput<br/>driver: DriverInput<br/>(server-side)"]
    JFromSpec2["Job::from_spec<br/>(server-side defence-in-depth)"]
    JobAgg["Job aggregate<br/>+ driver: WorkloadDriver<br/>WorkloadDriver::Exec(Exec)"]
    Intent["IntentStore<br/>(rkyv-archived)"]
    Reconciler["JobLifecycle::reconcile<br/>(pure)<br/>projects driver.exec() →<br/>flat AllocationSpec"]
    Action["Action::StartAllocation<br/>{spec: AllocationSpec}<br/><br/>Action::RestartAllocation<br/>{alloc_id, spec: AllocationSpec}<br/>(spec stays flat per ADR-0030)"]
    Shim["action_shim::dispatch<br/>(I/O boundary)"]
    Driver["ExecDriver<br/>Command::new(command).args(args)"]

    Operator -- writes --> TOML
    TOML -- "toml::from_str" --> CLI
    CLI -- "deserialise" --> JSI1
    JSI1 -- "validate + project DriverInput → WorkloadDriver" --> JFromSpec1
    JFromSpec1 -- "POST" --> HTTP
    HTTP -- "deserialise JSON" --> Handler
    Handler -- "" --> JSI2
    JSI2 -- "validate (defence-in-depth)" --> JFromSpec2
    JFromSpec2 -- "construct" --> JobAgg
    JobAgg -- "rkyv archive + put" --> Intent
    Intent -- "read + access" --> Reconciler
    Reconciler -- "emit (flat AllocationSpec)" --> Action
    Action -- "consume" --> Shim
    Shim -- "Driver::start(&spec)" --> Driver

    classDef new fill:#dff5dc,stroke:#3a7d44,stroke-width:2px
    classDef changed fill:#fff4d6,stroke:#9b6e0a,stroke-width:2px
    class JobAgg new
    class JSI1,JSI2,Reconciler,Action,Shim changed
```

Green = new tagged-enum field on intent aggregate; yellow = type
carries reshaped fields through unchanged contract.

### What the amendment does NOT change

- TOML wire shape (§1) — operators see the same `[exec]` table.
- `JobSpecInput` Rust shape (§2) — already nested with `DriverInput`
  tagged enum.
- `ResourcesInput` / `ExecInput` / `DriverInput` (§2) — unchanged.
- `AllocationSpec` (§3 cross-reference) — flat per ADR-0030;
  unchanged.
- Validation rule predicate (§4) — `exec.command` trimmed-non-empty
  is the same rule; only the construction path inside `Job::from_spec`
  reshapes (project into `WorkloadDriver::Exec(Exec)`).
- Validation field name (§4) — `"exec.command"` operator-facing path
  preserved.
- Action shim contract (§6) — same signature, same deletions.
- Validating-constructor lane (§7) — `Job::from_spec` is still THE
  single intent-side path.
- OpenAPI propagation (§8) — `utoipa::ToSchema` derives extend to
  `WorkloadDriver` and `Exec` for the *describe* response shape; the
  *submit* request schema is the existing `JobSpecInput` family.
- Single-cut migration discipline (§9) — every `Job { command, args }`
  call site migrates to `Job { driver: WorkloadDriver::Exec(Exec
  { command, args }) }` in the same PR. No `#[serde(alias)]`, no
  compatibility shim.
- Compliance section — every prior compliance entry holds. The
  amendment adds one: development.md § Type-driven design / "make
  invalid states unrepresentable" — the tagged-enum on `Job` makes
  driver-class exclusivity structural at the intent layer.
- Alternatives A-E — Alt D ("per-driver-spec types now (eager `Spec`
  enum on `Job`)") was rejected as premature for the **AllocationSpec
  side**; Amendment 1 takes the *opposite* posture for the **Job
  side** (the aggregate that operators reason about and that future
  drivers extend additively). The two are not in tension — different
  types, different boundaries, different forces. Alt D's reasoning
  applies to AllocationSpec because the trait dispatch already encodes
  the discriminator; on Job the operator's wire-shape exclusivity has
  no equivalent dispatch surface to inherit it from, so the structural
  enforcement must live on the type.

### Compliance addenda (additive over original Compliance section)

- **`development.md` § Type-driven design — sum types over sentinels /
  make invalid states unrepresentable**: Amendment 1 makes the
  driver-class field on `Job` a sum type (`WorkloadDriver`) so that
  Phase 2+ multi-driver expansion cannot accidentally introduce
  per-driver `Option<...>` flat fields whose both-Some / both-None
  states would be representable.
- **ADR-0030 §6 (per-driver-class Phase 2+ posture)**: preserved.
  Amendment 1 explicitly does NOT touch `AllocationSpec`; the Phase
  2+ split into per-driver-class spec types remains the predicted
  shape.

### Migration impact (for downstream artifact propagation)

This amendment lands in the wire-exec-spec-end-to-end DELIVER wave.
The downstream artifacts that must update to pick up Amendment 1 (the
orchestrator runs these as separate dispatches; they are NOT part of
the architect's scope):

**DISTILL artifacts** (`docs/feature/wire-exec-spec-end-to-end/distill/`):
- `test-scenarios.md` — any scenario referencing `Job.command` or
  `Job.args` directly switches to `Job.driver` enum match. The
  validation field name `exec.command` does NOT change — operator-facing
  path is preserved.
- Acceptance criteria mentioning `Job { command, args }` literal
  shape need to mention `Job { driver: WorkloadDriver::Exec(Exec
  { command, args }) }`.

**Production scaffolds** (existing files, modify in DELIVER):
- `crates/overdrive-core/src/aggregate/mod.rs` — `Job` struct definition
  (replace flat `command`, `args` with `driver: WorkloadDriver`). Add
  `WorkloadDriver` enum and `Exec` struct definitions. Update
  `Job::from_spec` to project `DriverInput → WorkloadDriver`. Update
  `From<&Job> for JobSpecInput` projection.
- `crates/overdrive-core/src/reconciler.rs` — `JobLifecycle::reconcile`
  populating expression for `Action::StartAllocation` and
  `::RestartAllocation` switches from `job.command.clone()` /
  `job.args.clone()` to destructure `&job.driver` as
  `WorkloadDriver::Exec(Exec { command, args })`.
- `crates/overdrive-control-plane/src/action_shim.rs` — no change
  beyond the original §6 deletions; the shim still consumes flat
  `AllocationSpec`.

**Existing scaffold files added pre-amendment** (in
`crates/overdrive-core/tests/acceptance/exec_*.rs`,
`crates/overdrive-control-plane/tests/acceptance/*`,
`crates/overdrive-cli/tests/integration/exec_spec_walking_skeleton.rs`):
all references to `Job { command, args }` shape switch to nested
`Job { driver: WorkloadDriver::Exec(Exec { command, args }) }`.
Pattern matches on `Job.command` / `Job.args` switch to nested
destructure. The DISTILL re-run is the orchestrator's next dispatch.

**Roadmap step ACs**
(`docs/feature/wire-exec-spec-end-to-end/deliver/roadmap.json`):
ACs for steps 01-01, 01-02, 02-01, 04-02, 05-01 mention the flat field
shape on `Job` — these update to the nested `WorkloadDriver` form. The
roadmap update is the orchestrator's next dispatch (do NOT update in
this design amendment).

---

## Context

ADR-0030 ratified the *internal* shape of `AllocationSpec` — the type
the node-agent driver consumes:

```rust
pub struct AllocationSpec {
    pub alloc:     AllocationId,
    pub identity:  SpiffeId,
    pub command:   String,
    pub args:      Vec<String>,
    pub resources: Resources,
}
```

The `exec-driver-rename` feature shipped that change. What remained is
the **operator surface** and the **propagation path** from a TOML job
spec on the operator's disk down through CLI parse → server validation
→ IntentStore → reconciler hydrate → `Action::StartAllocation` →
action shim → `Driver::start`.

Three concrete defects survived ADR-0030:

1. **`JobSpecInput` does not carry `command` or `args`.** It carries
   only `id`, `replicas`, `cpu_milli`, `memory_bytes`
   (`crates/overdrive-core/src/aggregate/mod.rs:125-131`). An operator
   cannot express "run `/opt/x/bin/y --port 8080`" through this surface.
2. **The reconciler hardcodes `/bin/sleep` / `["60"]`.**
   `JobLifecycle::reconcile` populates `Action::StartAllocation.spec`
   with literals (`crates/overdrive-core/src/reconciler.rs:1194-1195`).
   This is direct code carrying test-fixture intent.
3. **The action shim's restart path fabricates a baseline spec.**
   `build_phase1_restart_spec`
   (`crates/overdrive-control-plane/src/action_shim.rs:223-231`) builds
   `/bin/sleep` + `["60"]` from the prior obs row's `job_id` alone,
   ignoring whatever `command`/`args` the operator actually declared.
   `default_restart_resources` likewise fabricates a fixed
   `100mCPU + 256MiB` envelope regardless of the live `Job.resources`.

These defects are the same root: the `Job` aggregate cannot carry
operator-facing exec invocation, and downstream code papers over the
gap with literals. Fixing them coherently requires (a) a new wire shape
that carries the operator's full intent, (b) an `Action` enum revision
so Restart carries a fully-populated spec the same way Start does,
(c) deletion of the placeholder builders in the action shim.

The user's proposed TOML shape during the DESIGN wave:

```toml
id = "payments"
replicas = 1

[resources]
cpu_milli    = 500
memory_bytes = 134217728

[exec]
command = ""
args    = []
```

Two structural moves vs the Phase 1 shape: `cpu_milli`/`memory_bytes`
move into a `[resources]` table, and a new `[exec]` table carries
`command + args` with implicit driver dispatch by table name.

## Decision

Adopt the user's proposed TOML shape verbatim. Add `command` and
`args` to the `Job` aggregate. Reshape `JobSpecInput` as a flat struct
with two nested input twins (`ResourcesInput`, `ExecInput`) bundled
through a tagged-enum field for driver dispatch. Grow
`Action::RestartAllocation` to carry the validated `AllocationSpec`.
Delete the placeholder spec-builders from the action shim.

### 1. TOML wire shape (operator-canonical)

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
`[resources]` (resource envelope) and `[exec]` (driver invocation).
Future drivers add new sibling tables (`[microvm]`, `[wasm]`); exactly
one driver table per spec is enforced by serde at parse time.

### 2. `JobSpecInput` Rust shape

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct JobSpecInput {
    pub id:        String,
    pub replicas:  u32,
    pub resources: ResourcesInput,
    #[serde(flatten)]
    pub driver:    DriverInput,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
pub enum DriverInput {
    Exec(ExecInput),
    // Future: MicroVm(MicroVmInput), Wasm(WasmInput)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourcesInput {
    pub cpu_milli:    u32,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecInput {
    pub command: String,
    pub args:    Vec<String>,
}
```

`#[serde(flatten)]` on `driver: DriverInput` is what makes the table
implicit-by-name in TOML: serde deserialises the top-level `[exec]`
table directly into `DriverInput::Exec(ExecInput)` without an
intervening discriminator field. This matches the Nomad mental model
(`task { driver = "exec"; config { command = ...; args = [...] } }`)
and the operator-canonical shape ratified by ADR-0030.

`deny_unknown_fields` on every struct — the operator typing a typo
(`commando = "..."`) gets a parse error, not a silently-ignored field.

### 3. `Job` aggregate extension

```rust
pub struct Job {
    pub id:        JobId,
    pub replicas:  NonZeroU32,
    pub resources: Resources,
    pub command:   String,        // ← NEW
    pub args:      Vec<String>,   // ← NEW
}
```

Both new fields are mandatory. `command` is validated non-empty at
`Job::from_spec`; `args` carries no per-element validation (argv is
opaque to the driver per ADR-0030 §Compliance).

The rkyv archive surface grows two fields. Per
`feedback_single_cut_greenfield_migrations` and the explicit Phase 1
posture, no production IntentStore data exists yet; the resulting
content-hash change is acceptable, and test snapshots that pin
`spec_digest` values regenerate in the same PR.

### 4. New validation rule at `Job::from_spec`

```rust
let trimmed = exec.command.trim();
if trimmed.is_empty() {
    return Err(AggregateError::Validation {
        field: "exec.command",
        message: "command must be non-empty".to_string(),
    });
}
let command = exec.command;  // preserve original casing/whitespace post-trim-check
```

The trimmed-empty check rejects both `""` and whitespace-only inputs.
The original `command` string is preserved for `tokio::process::
Command::new` — the validation is *predicate*, not *normalisation*.

The existing rules are unchanged: `replicas != 0`, `memory_bytes != 0`,
`JobId` parse via `#[from]`. `cpu_milli` continues to allow zero
(cgroup `cpu.weight` derivation per ADR-0026 handles zero gracefully;
`memory.max` is the Phase 1 enforcement boundary).

NUL-byte rejection in `command` or `args` elements is **not** added —
the kernel's `execve(2)` rejects NUL-bearing argv with `EINVAL` and
the driver surfaces it as `DriverError::StartRejected` per ADR-0023.
Adding a platform-side cap before the kernel sees the value would
diverge from the kernel's posture for no safety benefit.

### 5. `Action` enum revision

```rust
// Before
pub enum Action {
    // ...
    RestartAllocation { alloc_id: AllocationId },
}

// After
pub enum Action {
    // ...
    RestartAllocation {
        alloc_id: AllocationId,
        spec:     AllocationSpec,    // ← NEW; mirrors StartAllocation.spec
    },
}
```

`StartAllocation { ..., spec }` is unchanged in shape — only the
*populating* expression in `JobLifecycle::reconcile` changes from
literal `/bin/sleep`/`["60"]` to `job.command.clone()`/`job.args.clone()`.

`RestartAllocation` grows the `spec` field. The reconciler's reconcile
body has the live `Job` in scope when emitting the restart (it hydrated
it from intent); building the spec at emit time, in the pure
reconciler, is cheap and correct. The action's "carry everything the
shim needs" property is preserved (ADR-0023 §2 — the shim is a stateless
dispatcher of typed actions).

### 6. Action shim deletions

`crates/overdrive-control-plane/src/action_shim.rs` deletes:

- `build_phase1_restart_spec` (lines 223-231) — the Phase 1 baseline
  builder. The Restart variant now reads `spec` straight off the
  action.
- `build_identity` (lines 239-247) — the duplicate of `core::reconciler::
  mint_identity`. The reconciler-side derivation is the SSOT; the shim
  needs no copy.
- `default_restart_resources` (lines 255-257) — the fabricated
  `100mCPU/256MiB` envelope. The live `Job.resources` is the
  authoritative source via the spec on the action.
- The `default_restart_resources_pins_exact_values` test (lines 284-317)
  — pins a function that no longer exists.

The Restart arm of `dispatch_single` after the change:

```rust
Action::RestartAllocation { alloc_id, spec } => {
    let handle = AllocationHandle { alloc: alloc_id.clone(), pid: None };
    let _ = driver.stop(&handle).await;

    let Some(prior_row) = find_prior_alloc_row(obs, &alloc_id).await? else {
        return Err(ShimError::HandleMissing { alloc_id });
    };

    let state = match driver.start(&spec).await {
        Ok(_handle) => AllocState::Running,
        Err(DriverError::StartRejected { .. }) => AllocState::Terminated,
        Err(other) => return Err(ShimError::Driver(other)),
    };
    let row = build_alloc_status_row(
        alloc_id, prior_row.job_id, prior_row.node_id, state, tick,
    );
    obs.write(ObservationRow::AllocStatus(row)).await?;
    Ok(())
}
```

`find_prior_alloc_row` survives — it is still needed to recover
`(job_id, node_id)` for the `AllocStatusRow` write. Only the
spec-rebuild path goes away.

### 7. Validating-constructor lane preservation

Per ADR-0011 the validating constructor is THE single path into the
intent-side `Job` aggregate. Both lanes route through it:

- **CLI lane** (`crates/overdrive-cli/src/commands/job.rs:104-113`):
  `toml::from_str::<JobSpecInput>(...)` → `Job::from_spec(spec_input)`.
  No logic change beyond the type reshape.
- **Server lane** (`crates/overdrive-control-plane/src/handlers.rs::
  submit_job`): JSON deserialise `JobSpecInput` → `Job::from_spec(...)`.
  Defence-in-depth per ADR-0015 — the same constructor runs server-side
  even when the CLI pre-validated.

The new `exec.command` validation rule fires on both lanes by
construction.

### 8. OpenAPI propagation (per ADR-0009)

`utoipa::ToSchema` derives on `JobSpecInput`, `ResourcesInput`,
`ExecInput`, and `DriverInput` (the tagged enum). `cargo xtask
openapi-gen` regenerates `api/openapi.yaml`; `cargo xtask
openapi-check` (CI gate) catches drift.

The generated schema renders:

- `JobSpecInput` — object, `required: [id, replicas, resources]`, plus
  the flattened driver one-of.
- `ResourcesInput` — object, `required: [cpu_milli, memory_bytes]`.
- `ExecInput` — object, `required: [command, args]` (note: `args`
  is required even when empty — an absent `args` field is a parse
  error, not "default to no args").
- `DriverInput` — `oneOf` with currently one entry, externally tagged
  by serde-rename (`exec`).

### 9. Single-cut migration

Per `feedback_single_cut_greenfield_migrations.md` and project
CLAUDE.md: no `#[serde(alias = "cpu_milli")]`, no compatibility shim,
no two-shape acceptance period. Every fixture, doctest, integration
test, OpenAPI schema, and CLI render path migrates in the same PR.

The migration scope is enumerated in
`docs/feature/wire-exec-spec-end-to-end/design/wave-decisions.md` §7.

### 10. Component data-flow (C4 component)

```mermaid
flowchart TB
    Operator["Operator (CLI user)"]
    TOML["job.toml<br/>id / replicas / [resources] / [exec]"]
    CLI["overdrive-cli<br/>commands::job::submit"]
    JSI1["JobSpecInput<br/>(client-side)"]
    JFromSpec1["Job::from_spec<br/>(client-side validation)"]
    HTTP["POST /v1/jobs<br/>(JSON body: JobSpecInput)"]
    Handler["overdrive-control-plane<br/>handlers::submit_job"]
    JSI2["JobSpecInput<br/>(server-side)"]
    JFromSpec2["Job::from_spec<br/>(server-side defence-in-depth)"]
    JobAgg["Job aggregate<br/>+ command, args"]
    Intent["IntentStore<br/>(rkyv-archived)"]
    Reconciler["JobLifecycle::reconcile<br/>(pure)"]
    Action["Action::StartAllocation<br/>{spec: AllocationSpec}<br/><br/>Action::RestartAllocation<br/>{alloc_id, spec}"]
    Shim["action_shim::dispatch<br/>(I/O boundary)"]
    Driver["ExecDriver<br/>Command::new(command).args(args)"]

    Operator -- writes --> TOML
    TOML -- "toml::from_str" --> CLI
    CLI -- "deserialise" --> JSI1
    JSI1 -- "validate" --> JFromSpec1
    JFromSpec1 -- "POST" --> HTTP
    HTTP -- "deserialise JSON" --> Handler
    Handler -- "" --> JSI2
    JSI2 -- "validate (defence-in-depth)" --> JFromSpec2
    JFromSpec2 -- "construct" --> JobAgg
    JobAgg -- "rkyv archive + put" --> Intent
    Intent -- "read + access" --> Reconciler
    Reconciler -- "emit" --> Action
    Action -- "consume" --> Shim
    Shim -- "Driver::start(&spec)" --> Driver

    classDef new fill:#dff5dc,stroke:#3a7d44,stroke-width:2px
    classDef changed fill:#fff4d6,stroke:#9b6e0a,stroke-width:2px
    class JobAgg,Action new
    class JSI1,JSI2,Reconciler,Shim changed
```

Green = new fields/variants; yellow = type carries reshaped fields
through unchanged contract.

## Alternatives considered

### Alternative A — Implicit-by-table-name (CHOSEN)

The chosen shape. `[exec]`, future `[microvm]`, `[wasm]` are top-level
tables; `serde(flatten)` on a tagged enum surfaces the table name as
the discriminator.

### Alternative B — Explicit `driver = "exec"` field + `[config]` block

```toml
id = "payments"
replicas = 1
driver = "exec"

[resources]
cpu_milli    = 500
memory_bytes = 134217728

[config]
command = "/opt/payments/bin/payments-server"
args    = ["--port", "8080"]
```

**Rejected.** The operator types the discriminator twice — once as the
`driver` field, once implicitly via "the `[config]` table contains
exec config because `driver = "exec"`." This is the structure tagged
enums exist to replace. The `[config]` block is a generic bag whose
contents change shape based on a sibling field; reading the spec, an
operator cannot tell what fields belong inside `[config]` without
cross-referencing `driver`. The implicit-by-table-name shape (chosen)
makes the table identity self-documenting.

### Alternative C — Tagged-enum nesting `[driver.exec]`

```toml
id = "payments"
replicas = 1

[resources]
cpu_milli    = 500
memory_bytes = 134217728

[driver.exec]
command = "/opt/payments/bin/payments-server"
args    = ["--port", "8080"]
```

**Rejected.** Cleanest serde shape — it IS literally a tagged enum, no
flatten gymnastics. But adds a level of nesting that buys nothing for
the single-driver case and reads more verbosely than `[exec]`. Diverges
from the Nomad mental model the rest of the platform aligns on
(per ADR-0030). For a Phase 1 single-driver spec the ceremony is
unjustified.

### Alternative D — Per-driver-spec types now (eager `Spec` enum on `Job`)

Introduce `enum AllocationSpec { Exec(ExecAllocSpec), MicroVm(...) }`
on the intent side now, anticipating the Phase 2+ split.

**Rejected as premature.** Per ADR-0030 §6, the per-driver-type spec
split is the natural shape when the second driver class lands; doing
it now is YAGNI applied to the type system. The current shared
`AllocationSpec` carries `command + args + resources` — exactly the
fields exec needs and exactly the fields that will become `ExecSpec`
in the Phase 2+ split. Pre-emptive enum-ifying adds cardinality
without expressive power.

### Alternative E — Carry `&dyn IntentStore` into the action shim for restart

Pass `&dyn IntentStore` to `dispatch`. On Restart, look up the live
`Job` from intent (via `IntentKey::for_job`) and rebuild the spec from
authoritative state.

**Rejected.** The action shim is currently a pure stateless dispatcher
(`(driver, obs, tick) → Vec<Action> → I/O`). Introducing an
IntentStore handle adds a new I/O dependency and an additional
async lookup per restart. The chosen Alt R2 (the action carries the
spec) keeps the shim's surface unchanged and offloads the spec
construction to the reconciler — which already has the `Job` in scope
and is where every other "what should the workload look like" decision
lives. The reconciler is the right home for spec materialisation
because it owns the desired-vs-actual reasoning; the shim is the right
home for executing the materialised decision.

## Consequences

### Positive

- **Operator can express any binary + argv combination.** The platform
  is no longer artificially constrained to whatever literals are
  hardcoded in the reconciler. `/opt/x/bin/y --mode=fast --port 8080`
  runs as readily as `/bin/sleep 60`.
- **Production code stops carrying test-fixture intent.** Both literal
  fabrications (`reconciler.rs:1194-1195` and
  `action_shim.rs:223-231`) delete in this PR. The reconciler reads
  the operator's declared intent; the action shim dispatches it.
- **Restart respects operator intent.** Today, a failed allocation
  restarts as `/bin/sleep` regardless of what the operator declared —
  obviously broken once any non-sleep workload exists. After this ADR,
  restart re-uses the live `Job.command`/`.args`/`.resources`.
- **Spec round-trip is honest.** `JobSpecInput → Job::from_spec →
  From<&Job> for JobSpecInput` is the identity function on every
  valid input. The `describe` endpoint (ADR-0008) renders the
  operator's declared spec back faithfully.
- **Operator-facing error surface is structured.** Empty `command`
  surfaces as `AggregateError::Validation { field: "exec.command" }`
  on both client and server lanes — before any HTTP call, before any
  `execve(2)` failure. Operators see field-named diagnostics, not
  kernel-syscall errno strings.
- **Phase 2+ multi-driver expansion is additive.** A future PR adding
  `[microvm]` extends `DriverInput` with one variant and one input
  twin (`MicroVmInput`); the rest of the surface — `JobSpecInput`,
  `Job`, `Action::StartAllocation { spec }`, the action shim — is
  untouched. Same posture for `[wasm]`.
- **OpenAPI schema is honest.** The `oneOf` for the driver block
  documents the multi-driver future even with one variant present
  today; clients reading the schema see the extension shape.

### Negative

- **Breaking shape change for every test fixture, every doctest, every
  inline `JobSpecInput { ... }`.** The migration scope is enumerated
  in `docs/feature/wire-exec-spec-end-to-end/design/wave-decisions.md`
  §7 — roughly 25 source files. Each file's diff is mechanical
  (literal-by-literal substitution); none requires logic changes.
- **rkyv archive break.** The `Job` archive layout grows two fields;
  any pinned `spec_digest` in test snapshots regenerates. Per
  single-cut greenfield, no production data exists; the break is
  intentional.
- **`#[serde(flatten)]` on a tagged enum has known sharp edges.** It
  works, but `utoipa`'s rendering of the resulting `oneOf` may need
  a `#[schema(value_type = ...)]` workaround if macro expansion has
  trouble with flatten-on-enum. The risk is a one-line attribute
  fix in DELIVER, not a structural problem.
- **`deny_unknown_fields` rejects forward-compatible spec extensions.**
  An operator running an old client against a newer server that
  introduced a new optional field would get a parse error on the
  client. For Phase 1 there are no public operators and the CLI ships
  with the server; not a Phase 1 concern. Phase 2+ may revisit if
  cross-version compat becomes a concern, with a separate ADR.

### Quality-attribute impact (ISO 25010)

- **Functional Suitability — appropriateness**: positive. The wire
  shape lets operators express what they actually want.
- **Compatibility — interoperability**: positive. Nomad operators map
  their mental model directly: `[resources]` ↔ `resources {}`,
  `[exec]` ↔ `task { driver = "exec"; config { command, args } }`.
- **Maintainability — modifiability**: positive. Multi-driver
  expansion is additive; no rewrite of existing code.
- **Maintainability — analyzability**: positive. Each TOML table
  carries one cohesive concept; rustdoc on `Job.command`/`.args`
  reads against the operator's existing mental model.
- **Maintainability — testability**: positive. Test fixtures carry
  explicit `command`/`args`/`resources`; no magic dispatch hides
  intent from the reader.
- **Reliability — fault tolerance**: positive. Empty-command rejection
  at the constructor gives operators structured field-named errors at
  client-side validation, never as opaque exec failures.
- **Security — confidentiality / integrity**: neutral. The security
  boundary (SPIFFE identity + LSM + cgroup envelope per ADR-0026,
  ADR-0028) is unaffected. `command` is no more or less trustworthy
  than the binary path was; both flow through the same intent-side
  validation.
- **Performance — time behaviour**: neutral. The reshape adds two
  rkyv-archived fields and one serde tagged-enum dispatch; both are
  deserialise-cost only, not hot-path.

## Compliance

- **ADR-0030 (`ExecDriver` rename + `AllocationSpec { command, args }`)**:
  this ADR is the operator-surface counterpart. ADR-0030 ratified the
  internal shape; this ADR wires the operator surface and the action
  enum to populate that internal shape from authoritative intent.
- **ADR-0011 (Aggregates and `JobSpecInput`-collision)**: the validating
  constructor `Job::from_spec` remains THE single path into the
  intent-side `Job` aggregate. The new `exec.command` rule slots in
  alongside the existing replicas/memory rules; no new constructor.
- **ADR-0014 (CLI HTTP client and shared types)**: `JobSpecInput` is
  shared verbatim across CLI and server lanes. The CLI deserialises
  TOML; the server deserialises JSON; both route through the same
  validating constructor. The reshape preserves this property.
- **ADR-0015 (HTTP error mapping)**: the new `AggregateError::Validation
  { field: "exec.command" }` flows through the existing
  `ControlPlaneError::Validation` mapping; HTTP body shape per
  RFC 7807 is unchanged.
- **ADR-0009 (OpenAPI schema derivation)**: `utoipa::ToSchema` on the
  reshaped types regenerates `api/openapi.yaml`; the existing
  `cargo xtask openapi-check` CI gate catches drift.
- **ADR-0023 (Action shim placement)**: the shim's
  `(driver, obs, tick)` signature is unchanged. The Restart arm reads
  `spec` off the action; `build_phase1_restart_spec` deletes. The
  shim contract — typed actions in, observation rows out, errors
  via `ShimError` — is preserved verbatim.
- **ADR-0013 (Reconciler primitive runtime)**: `reconcile` remains
  pure; the spec construction is a deterministic projection of the
  hydrated `Job` aggregate. No `.await`, no Instant::now, no I/O —
  the new code is `let action = Action::Start/RestartAllocation
  { ..., spec: AllocationSpec { command: job.command.clone(), ... } }`.
- **ADR-0027 (`POST /v1/jobs/{id}:stop`)**: the stop path operates on
  `IntentKey::for_job_stop` and `Driver::stop(handle)`; it never
  references the spec shape. Unaffected by this ADR.
- **ADR-0026 (cgroup v2 direct writes)**: cgroup mechanics
  (`cpu.weight` derivation, `memory.max`, warn-and-continue posture)
  are unaffected. The `Resources` envelope on `Job` is unchanged in
  shape; only its TOML surfacing moves into the `[resources]` table.
- **ADR-0028 (cgroup pre-flight)**: pre-flight refusal path is
  unaffected.
- **`development.md` § Newtypes — STRICT by default**: `command:
  String` and `args: Vec<String>` are NOT newtype-warranted. The
  driver passes them verbatim to `tokio::process::Command::new(impl
  AsRef<OsStr>).args(impl IntoIterator<Item = impl AsRef<OsStr>>)` —
  the type signature already constrains them appropriately. Wrapping
  in newtypes would add ceremony without expressive power; the
  validation rule (non-empty `command`) is constructor-side, not
  type-side.
- **`development.md` § Hashing requires deterministic serialization**:
  the `Job` rkyv archive layout grows two fields. Per single-cut
  greenfield migration discipline, no in-production data exists; the
  hash break is acceptable. Test snapshots with pinned `spec_digest`
  values regenerate in the same PR.
- **`development.md` § State-layer hygiene**: `JobSpecInput`,
  `ResourcesInput`, `ExecInput`, `DriverInput` are wire-shape;
  `Job`, `Resources`, `AllocationSpec` are intent-shape. The two never
  merge — input twins exist precisely to keep the rkyv-archived intent
  surface clean of serde-only concerns.
- **`feedback_single_cut_greenfield_migrations`**: no `#[serde(alias)]`,
  no compatibility shim. The migration scope (~25 source files)
  lands in one PR.
- **`feedback_phase1_single_node_scope`**: this ADR ships
  in single-node Phase 1 — no node registration, no multi-region, no
  taint/toleration. Cgroup isolation per ADR-0026 / ADR-0028
  remains in scope.

## Future work

- **Per-driver-type spec types** (Phase 2+, when MicroVm/Wasm land).
  The shared `AllocationSpec` likely splits into a `Spec` enum per
  ADR-0030 §6. The chosen `DriverInput` tagged-enum shape is the
  natural mirror on the input side; `Spec::Exec(ExecSpec) /
  Spec::MicroVm(MicroVmSpec)` mirrors `DriverInput::Exec(ExecInput) /
  DriverInput::MicroVm(MicroVmInput)`.
- **Forward-compat spec extension policy** (Phase 2+). If
  cross-version operator/server compat becomes a concern, revisit
  `deny_unknown_fields` posture — possibly with a per-field opt-in
  via `#[serde(default, skip_serializing_if = ...)]` for fields
  added in newer schemas.
- **Render-layer goldens for `overdrive job describe`**. The render
  output reshapes (two new sections, "Resources" and "Exec"); if
  golden tests for render output land, they pin the new shape.
  Out-of-scope for this design unless DISTILL elects to include them.
- **Argv per-element validation policy.** If a future workload class
  introduces semantic argv constraints (e.g. structured `--key=value`
  parsing the platform validates upstream), the rule is added to
  `Job::from_spec` and the field is named (`field: "exec.args[N]"`).
  No such constraint exists today.

## References

- ADR-0030 — `ExecDriver` rename + `AllocationSpec { command, args }`
  (the upstream type-shape decision; this ADR is the operator-surface
  wiring counterpart).
- ADR-0011 — Aggregates and `JobSpecInput`-collision.
- ADR-0014 — CLI HTTP client and shared types.
- ADR-0015 — HTTP error mapping.
- ADR-0009 — OpenAPI schema derivation.
- ADR-0023 — Action shim placement.
- ADR-0013 — Reconciler primitive runtime (purity contract).
- ADR-0027 — `POST /v1/jobs/{id}:stop` HTTP shape.
- ADR-0026 — cgroup v2 direct writes (resource envelope source).
- ADR-0028 — cgroup pre-flight refusal.
- `docs/feature/wire-exec-spec-end-to-end/design/wave-decisions.md` —
  DESIGN-wave summary, options analysis, Reuse Analysis table.
- `docs/feature/wire-exec-spec-end-to-end/design/upstream-changes.md` —
  hand-off to DISTILL acceptance-designer.
- HashiCorp Nomad — `exec` task driver & `resources` block (operator
  vocabulary precedent):
  https://developer.hashicorp.com/nomad/docs/deploy/task-driver/exec
- `feedback_single_cut_greenfield_migrations.md` — no two-shape
  acceptance period.
- `feedback_phase1_single_node_scope.md` — single-node scope.
