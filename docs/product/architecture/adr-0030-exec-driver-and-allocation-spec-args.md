# ADR-0030 — `ExecDriver` rename + `AllocationSpec { command, args }` shape; magic image-name dispatch removed

## Status

Accepted. 2026-04-28. Decision-makers: User-proposed during
`phase-1-first-workload` PR #135 review (Option C); architect-agent
authored; ratified 2026-04-28. Tags: phase-1, first-workload-followup,
application-arch, vocabulary, type-shape.

## Context

The Phase 1 first-workload feature (PR #135) shipped `ProcessDriver`
in `overdrive-worker` with the following `AllocationSpec` shape on
`overdrive-core`:

```rust
pub struct AllocationSpec {
    pub alloc:     AllocationId,
    pub identity:  SpiffeId,
    pub image:     String,         // ← misnamed; container-land jargon
    pub resources: Resources,
                                   // ← MISSING: argv
}
```

Three problems compound on this surface:

1. **The driver type name is wrong by precedent.** The wider workload-
   orchestration community calls this driver class `exec` — Nomad's
   first-class driver of this shape is named exactly that
   ([HashiCorp `exec` task driver](https://developer.hashicorp.com/nomad/docs/deploy/task-driver/exec)),
   and Talos, the closest immutable-OS precedent for Overdrive, uses
   the same vocabulary. "Process" was an internal-implementation noun
   we picked because the underlying primitive is `tokio::process` —
   but the operator-facing concept is "execute a binary directly,"
   and the operator-canonical noun for that concept is `exec`. The
   `Driver::r#type()` return value and the job-spec `driver` field
   are both operator-facing identities; using a non-canonical name
   here costs every operator who already speaks the wider community's
   vocabulary.

2. **The spec field name is wrong by category.** `image` is borrowed
   from container land where `docker.io/library/postgres:15` is
   genuinely a content-addressed image identifier. For an exec driver
   running binaries directly, `/bin/sleep` is a *host filesystem
   path*, not an image. The current `build_command` body reads
   `Command::new(&spec.image)` — that line is self-documenting
   evidence the field is misnamed: the driver passes the value
   verbatim to `Command::new`, which expects an executable path, not
   an image. The field name lies about its semantics.

3. **Missing `args` field forced magic image-name dispatch.** Because
   the spec cannot carry argv, the driver hardcoded argv per blessed
   "image":

   ```rust
   fn build_command(spec: &AllocationSpec) -> Command {
       let mut cmd = Command::new(&spec.image);
       if spec.image == "/bin/sleep" {
           cmd.arg("60");
       } else if spec.image == "/bin/sh" {
           cmd.arg("-c").arg("trap '' TERM; sleep 60");
       } else if spec.image == "/bin/cpuburn" {
           cmd.arg("-c").arg("for i in $(seq 1 $(nproc)); do …");
       }
       cmd
   }
   ```

   Production code is reading test-fixture intent — three blessed
   path strings carry implicit argv-shaped contracts the tests rely
   on. Two consequences. First, the driver is not extensible: any
   real workload (`/usr/bin/python my-app.py`, `/opt/payments/bin/
   payments-server --port 8080`, `/usr/local/bin/redis-server /etc/
   redis/redis.conf`) cannot run because `AllocationSpec` cannot
   express its argv. Second, the test surface is artificially
   constrained: every test must use one of the three blessed image
   names, and adding a new test that needs a fourth shape forces
   either editing production code (to add a fourth dispatch arm)
   or fighting the magic dispatch in unmaintainable ways.

These three problems are coupled — they are caused by a single
root, the spec carrying an "image" field with no argv companion.
Fixing them coherently requires a single-cut migration that renames
the type, renames the field to honest vocabulary, and adds the
missing argv carrier.

The existing ADR-0026 (cgroup v2 direct writes) and ADR-0029
(`overdrive-worker` extraction) carried interim 2026-04-28 amendments
documenting this rename when it was first surfaced in the PR review,
and used `binary` as the new field name. **This ADR supersedes those
interim amendments**: the user-confirmed Nomad-canonical name is
`command`, not `binary`. Both interim amendments now point to this
ADR rather than carrying the substantive content themselves.

## Decision

Three coordinated renames + one additive field, landed single-cut.
Authoritative naming choice: **Nomad's `exec` driver semantics**.

### 1. `ProcessDriver` → `ExecDriver`

```rust
// crates/overdrive-worker/src/driver.rs
pub struct ExecDriver { /* … same fields … */ }

#[async_trait]
impl Driver for ExecDriver {
    fn r#type(&self) -> DriverType { DriverType::Exec }
    /* … same trait surface … */
}
```

The crate (`overdrive-worker`) does NOT rename. The crate is named
for its *role* (the worker subsystem), not for the driver class it
hosts; future `MicroVm` and `Wasm` drivers will live in the same
crate alongside `ExecDriver`. ADR-0029's crate boundary is preserved
verbatim.

### 2. `DriverType::Process` → `DriverType::Exec`

```rust
// crates/overdrive-core/src/traits/driver.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DriverType {
    /// Native binary executed directly under cgroups v2 (`tokio::process`).
    Exec,           // ← was Process
    MicroVm,
    Vm,
    Unikernel,
    Wasm,
}
```

The wire form changes from `process` to `exec` everywhere
`DriverType` is serialised — the `driver` field of the operator's
job-spec TOML, the `Driver::r#type()` return rendered in CLI output,
the OpenAPI schema's enum. Phase 1 ships pre-1.0 with no public
operators in production; this is the right moment to make the
operator-canonical choice. Future `DriverType::*` variants are
appended; existing variants never change wire form again from this
point forward.

### 3. `AllocationSpec.image: String` → `AllocationSpec.command: String`

```rust
// crates/overdrive-core/src/traits/driver.rs
pub struct AllocationSpec {
    pub alloc:     AllocationId,
    pub identity:  SpiffeId,
    /// Host filesystem path or program name to invoke. Passed verbatim
    /// to `Command::new` (resolved via `$PATH` if not an absolute path).
    /// Container drivers (Phase 2+ MicroVm / Wasm) carry their own
    /// `ContentHash`-typed `image` field on per-driver-type spec
    /// types — distinct from the exec driver's `command`.
    pub command:   String,         // ← was `image: String`
    /// Argv passed verbatim to the command; the driver invokes
    /// `Command::new(&self.command).args(&self.args)`. An empty
    /// `Vec` means "no arguments," which is a meaningful and
    /// well-formed shape for binaries that take no arguments
    /// (`/usr/bin/whoami`, `/bin/true`).
    pub args:      Vec<String>,    // ← NEW; mandatory; empty = no-args
    pub resources: Resources,
}
```

The field is named `command` to match Nomad's `exec` task driver
schema exactly:

```hcl
# Nomad job spec — exec driver
task "payments" {
  driver = "exec"
  config {
    command = "/opt/payments/bin/payments-server"
    args    = ["--port", "8080"]
  }
}
```

Operators who already know Nomad map this to Overdrive without a
vocabulary translation. The ADR's interim amendment that proposed
`binary` is reversed by this ADR: `binary` over-specifies what the
field is (Nomad's `command` accepts binaries, scripts with shebang
lines, wrapper invocations, anything `Command::new` accepts; the
driver does not care which) and breaks the cross-tool mapping
operators rely on.

`args: Vec<String>` is mandatory — no `Default`, no `Option`. An
empty `Vec` is the zero-args case; a missing field is a
deserialisation error. The mandatoriness is load-bearing: making it
optional invites a class of bug where a test fixture forgets the
field and the driver invokes the binary with whatever happens to be
left over on the stack (defensible only because Rust does not work
that way; defensiveness is still the point — the missing-args case
must be expressed, not implicit).

### 4. `ExecDriver::build_command` — magic dispatch removed

```rust
// crates/overdrive-worker/src/driver.rs
fn build_command(spec: &AllocationSpec) -> Command {
    let mut cmd = Command::new(&spec.command);
    cmd.args(&spec.args);
    cmd.kill_on_drop(false);
    // SAFETY: setsid() places the spawned child in its own process
    // group, so SIGKILL on stop reaches the entire workload tree.
    // Was previously gated behind `if spec.image == "/bin/sh"`; that
    // gate was a side effect of the magic dispatch needing a switch
    // site, not an architectural intent. Every exec workload deserves
    // its own process group.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd
}
```

The entire `if spec.image == "/bin/sleep" / "/bin/sh" / "/bin/cpuburn"`
dispatch tree is **deleted**. Test fixtures construct `command + args`
directly:

| Test scenario | Pre-rename (magic) | Post-rename (explicit) |
|---|---|---|
| Long-lived sleep | `image: "/bin/sleep"` | `command: "/bin/sleep", args: vec!["60".into()]` |
| SIGTERM-trap | `image: "/bin/sh"` | `command: "/bin/sh", args: vec!["-c".into(), "trap '' TERM; sleep 60".into()]` |
| CPU-burst (cgroup-isolation 4.2) | `image: "/bin/cpuburn"` (does not exist on the Lima image) | `command: "/bin/sh", args: vec!["-c".into(), "for i in $(seq 1 $(nproc)); do (while :; do :; done) & done; wait".into()]` |
| Missing-binary error path | `image: "/this/does/not/exist"` | `command: "/this/does/not/exist", args: vec![]` |

The CPU-burst script body is preserved verbatim from the pre-rename
`build_command` cpuburn branch — the exact bytes that previously
ran via magic dispatch now live in the test fixture. The kernel
pressure exercised is identical pre- and post-rename; only the
plumbing changes.

### 5. Wire shapes and operator-facing surfaces

The CLI's job-spec TOML schema and the HTTP API's `JobSpecInput`
both gain `args` and rename `image` → `command`:

```toml
# CLI job spec (Phase 1)
[job]
driver  = "exec"                          # was "process"
command = "/opt/payments/bin/payments"    # was image = "..."
args    = ["--port", "8080"]              # NEW (mandatory; empty = [])

[job.resources]
cpu_milli    = 500
memory_bytes = 268435456
```

The OpenAPI 3.1 schema is regenerated from the Rust types via
`utoipa` per ADR-0009; the regenerated `api/openapi.yaml` is
committed alongside the code change. CLI render strings (the
`overdrive job describe` output, `overdrive job submit` confirmation)
likewise migrate from `image` to `command`.

### 6. Phase 2+ posture — driver-class spec divergence is acceptable

`DriverType::MicroVm` and `DriverType::Wasm` will arrive with their
own image surface. A `MicroVm` allocation needs a `ContentHash`-typed
`image` field referencing a content-addressed VM rootfs in Garage
(per whitepaper §17 *Persistent Rootfs*); a `Wasm` allocation needs a
`ContentHash`-typed `module` field referencing a WASM module in
Garage. Both are fundamentally different surfaces from the exec
driver's `command + args`.

The right shape when those drivers land is per-driver-type spec
types — likely a `Spec` enum with `Spec::Exec(ExecSpec)`,
`Spec::MicroVm(MicroVmSpec)`, `Spec::Wasm(WasmSpec)` — rather than
forcing all three into a shared `AllocationSpec`. This ADR's shared
`AllocationSpec` is a Phase 1 simplification that holds because only
one driver class exists; it will likely split in the Phase 2+ PR
that introduces the second driver. The split is a future ADR's
concern; this ADR explicitly does not anticipate it.

What this ADR commits to is: the *exec driver's* spec uses
`command + args`, not `image`. Whatever the surrounding type
hierarchy looks like in Phase 2+, the operator-canonical
`command + args` shape for the exec driver is fixed.

## Alternatives considered

### Alternative A — Keep `image`; add `args`

Add `args: Vec<String>` to `AllocationSpec`; leave `image` named as
is; rewrite `build_command` to use `Command::new(&spec.image).args(&spec.args)`.

**Rejected.** This solves the magic-dispatch problem but leaves the
misleading container-land terminology. The field name still lies
about its semantics — operators reading the spec see `image`, expect
a content-addressed identifier, find a host filesystem path. The
cross-tool mapping with Nomad (the most likely incoming-operator
context) would translate `command` → `image`, costing every operator
that translation forever. Half-fix; rejected.

### Alternative B — Rename `image` → `binary`

Rename `image` → `binary`; add `args`. This was the user's initial
proposal and the shape carried by the interim 2026-04-28 amendments
to ADR-0026 and ADR-0029.

**Rejected on user reconsideration.** `binary` over-specifies: a
Nomad `command = "/usr/bin/env python"` is not a binary in the
strict sense, but it is a perfectly valid `Command::new` argument
and a perfectly valid Nomad `command` field. Naming the field
`binary` invites pedantic disagreement about whether shell scripts,
wrapper invocations, or symlinks count — when in fact the driver
does not care; it passes the value verbatim to `Command::new` and
lets the kernel sort out execve semantics. Nomad's `command` is the
right precedent: it accepts whatever `execve` accepts, and the
field name reflects that breadth honestly. The interim amendments
that used `binary` are now reversed by this ADR.

### Alternative C — Leave the magic dispatch; rename only the type

Rename `ProcessDriver` → `ExecDriver` and `DriverType::Process` →
`DriverType::Exec`; leave `AllocationSpec` untouched (with `image`
and the magic dispatch).

**Rejected.** The type rename is the cosmetic half of the problem;
the spec shape is the substantive half. Half-fixing leaves the
driver unable to run real workloads (anything not in the three
blessed image names) and leaves production code reading
test-fixture intent. The blast radius of a spec-shape change is
small — Phase 1 is single-node, single-driver, no public operators —
and deferring it means refactoring under pressure when the second
driver class lands. Single-cut now.

### Alternative D — Per-driver-type spec types now (eager `Spec` enum)

Introduce `enum Spec { Exec(ExecSpec), … }` immediately rather than
keeping the shared `AllocationSpec` and renaming its field.

**Rejected as premature.** Only one driver class exists in Phase 1.
A `Spec` enum with one variant is structurally identical to the
flat struct (the variant tag is the implicit `DriverType`); the
type hierarchy adds zero expressive power until the second driver
lands. The Phase 2+ PR that introduces `MicroVm` is the natural
home for the split; doing it now is YAGNI applied to the type
system. The `command + args` shape this ADR commits to is what
`ExecSpec` will end up looking like when the split happens; the
interim shape is exactly what would be carved out of
`AllocationSpec` then.

## Consequences

### Positive

- **Operator-canonical vocabulary.** Operators speaking Nomad's
  vocabulary map their mental model to Overdrive without
  translation: `driver = "exec"`, `command = "..."`, `args = [...]`
  is identical between the two systems.
- **Honest field naming.** `command` honestly describes what the
  driver passes to `Command::new`. The previous `image` lied;
  `binary` would have over-specified. `command` is exact.
- **Real workloads supported.** Any combination of binary path +
  argv runs: `/usr/bin/python my-app.py`, `/opt/payments/bin/
  payments-server --port 8080`, `/bin/sh -c '<arbitrary script>'`.
  The driver is no longer artificially constrained to three blessed
  test-fixture shapes.
- **Production code stops reading test-fixture intent.** The magic
  image-name dispatch was the most prominent piece of technical debt
  in the worker crate; removing it brings `build_command` to one
  line plus the `setsid` pre-exec hook (~5 lines total, down from
  ~40).
- **Test fixtures spell out their argv.** A future contributor adding
  a new test fixture writes `command: "...", args: vec!["..."]`
  inline rather than fighting magic dispatch in production code.
  Fewer surprises; faster iteration.
- **`setsid` unconditional.** Every exec workload now lives in its
  own process group, matching the SIGKILL-reaches-the-tree contract
  that was previously only true for `/bin/sh`-class workloads. This
  is a side-benefit of removing the magic dispatch — the conditional
  `setsid` was a side effect of the dispatch needing a switch site,
  not an architectural intent; the unconditional shape is the
  correct posture and was always the right default.
- **Phase 2+ MicroVm and Wasm drivers inherit a clean precedent.**
  `command + args` is the operator-canonical exec shape; future
  drivers carry their own image surfaces (`ContentHash`-typed
  `image` for MicroVm, `module` for Wasm) without spec-shape
  collision.

### Negative

- **Breaking type-shape change for any caller pinned to the pre-
  rename surface.** Internal-only impact in Phase 1 — no public
  operators, no checked-in third-party code. Every caller in the
  workspace migrates in the same single-cut PR. The interim
  amendments to ADR-0026 and ADR-0029 are superseded by this ADR.
- **OpenAPI schema regeneration required.** The `api/openapi.yaml`
  delta is mechanical (`image` → `command`; new `args` array of
  strings) and the existing CI check (`cargo xtask openapi-check`)
  catches drift. One-time cost.
- **Test fixtures grow by one line each.** Every fixture that
  previously relied on magic dispatch via three-character image
  shorthand now carries an explicit `args: vec![...]`. The diff
  is mechanical and the new shape is more honest about what the
  test exercises.

### Quality-attribute impact (ISO 25010)

- **Maintainability — modifiability**: positive. Real workloads
  become expressible; production code stops carrying test-fixture
  intent.
- **Maintainability — analyzability**: positive. `command + args`
  is the operator-canonical surface; rustdoc on the renamed fields
  reads against the operator's existing mental model from Nomad.
- **Maintainability — testability**: positive. Test fixtures
  construct argv inline; new test shapes do not require editing
  production code to add a dispatch arm.
- **Compatibility — interoperability**: positive. Nomad-spec-to-
  Overdrive-spec mapping is direct (`command` ↔ `command`, `args`
  ↔ `args`). The interim `binary` would have been actively
  misleading in that future.
- **Functional Suitability — appropriateness**: positive. The driver
  honestly does what its name says (exec a command with given args);
  the spec field name reflects what is passed to `Command::new`.
- **Performance — time behaviour**: neutral. `Command::new(...).args(...)`
  is the standard `tokio::process` invocation shape; no measurable
  change.
- **Reliability — fault tolerance**: neutral. Cgroup mechanics, error
  handling, signal escalation — all unchanged. Per ADR-0026
  amendment, the cgroup write surface is untouched.
- **Security — confidentiality / integrity**: neutral. The argv
  surface is no more (and no less) trustworthy than the binary
  path was; both flow through the same intent-side validation
  (`Job::from_spec`) and cross the same SPIFFE-identity-bound
  action shim.

### Migration shape

Single-cut greenfield migration per
`feedback_single_cut_greenfield_migrations`. Two cohesive commits
in the same PR (decomposed for reviewer-friendly diff scoping):

- **Commit 1** (`refactor(worker): rename ProcessDriver → ExecDriver,
  DriverType::Process → DriverType::Exec`): purely mechanical type
  rename with zero behaviour change. Test directory rename
  (`tests/integration/process_driver/` → `tests/integration/exec_driver/`)
  and per-file test-fn name rename land here.
- **Commit 2** (`feat(driver): rename AllocationSpec.image → command;
  add args: Vec<String>; drop magic image-name dispatch (ADR-0030)`):
  the substantive change. `command + args` lands; magic dispatch
  is deleted; every constructor migrates; test fixtures spell out
  argv inline; OpenAPI regenerates; ADR-0026 + ADR-0029 amendments
  + this ADR commit alongside the code.

No `#[deprecated]` aliases, no compatibility shim, no
feature-flagged old-name path, no `pub use ProcessDriver = ExecDriver`
re-export shadow, no `pub use binary as image` shadow field. The
old names are gone; the new names are in.

## Compliance

- **Whitepaper §6 (Workload Drivers)**: the `process` row in the
  driver table renames to `exec`. The trait surface
  (`Driver::start/stop/status/resize`) is unchanged; only the
  variant identity changes. Whitepaper §6 updated accordingly.
- **ADR-0022 (`AppState::driver: Arc<dyn Driver>`)**: the trait-
  object swap surface is preserved verbatim. The injected impl is
  named `ExecDriver` rather than `ProcessDriver`, but the
  composition pattern is unchanged.
- **ADR-0023 (action shim placement)**: `dispatch` calls
  `Driver::start/stop/status/resize` against `&dyn Driver`. The
  spec type passed to `start` reshapes (`command + args` instead
  of `image` + magic), but the trait method signature is unchanged
  and the shim contract is preserved.
- **ADR-0024 (`overdrive-scheduler` extraction)**: scheduler reads
  `node_health` and emits placement decisions; never sees the spec
  shape. Untouched.
- **ADR-0025 (single-node startup wiring)**: the `node_health` row
  writer's relocation to worker startup is preserved; the writer
  does not touch `AllocationSpec`.
- **ADR-0026 (cgroup v2 direct writes)**: unchanged in substance —
  cgroup-v2-only, direct cgroupfs writes, `cpu.weight` + `memory.max`
  derivation, warn-and-continue posture, limits-then-PID ordering,
  five-filesystem-operation surface. The interim 2026-04-28
  amendment is superseded by this ADR (the field is named `command`,
  not `binary`); the cgroup body is untouched.
- **ADR-0027 (job-stop HTTP shape)**: untouched. The stop path does
  not reference the spec shape; it operates on the
  `IntentKey::for_job_stop` key and `Driver::stop(handle)`.
- **ADR-0028 (cgroup pre-flight)**: untouched.
- **ADR-0029 (`overdrive-worker` extraction)**: crate boundary
  unchanged. The driver type living inside the crate renames
  (`ProcessDriver` → `ExecDriver`), but the crate itself
  (`overdrive-worker`), its dependency direction
  (`overdrive-core ← overdrive-worker ← overdrive-cli`), its
  `crate_class = "adapter-host"` declaration, and the
  `overdrive-control-plane`-does-NOT-depend-on-`overdrive-worker`
  invariant all carry through. The interim 2026-04-28 amendment is
  superseded by this ADR.
- **`development.md` § Newtypes / Newtype completeness**: `AllocationSpec`
  is a domain struct, not a newtype, but the new `command` and
  `args` fields adopt the same discipline — `command: String`
  validated non-empty at `Job::from_spec`; `args: Vec<String>`
  with no per-element validation (argv is opaque to the driver).
- **`testing.md` § Integration vs unit gating**: the integration
  suite under `crates/overdrive-worker/tests/integration/exec_driver/`
  remains gated by the `integration-tests` feature; the renamed
  directory is the only structural change.

## Supersession relationship

This ADR **supersedes the interim 2026-04-28 amendments** to
ADR-0026 and ADR-0029 that initially proposed the field name
`binary`. ADR-0026 and ADR-0029 are NOT superseded as ADRs — their
substantive bodies (cgroup mechanics, crate extraction) remain
Accepted and authoritative. What is superseded is just the interim
amendment text on each: the user-confirmed Nomad-canonical name is
`command`, not `binary`, and the substantive narrative for the
rename lives here in ADR-0030 rather than in the amendments.

ADR-0026 and ADR-0029 each carry a brief **Amendment 2026-04-28
(Revised)** subsection pointing to this ADR, replacing the prior
amendment text. The amendment-in-place pattern matches the
established convention; this ADR is the substantive home for the
rename narrative, and the amendments are pointers.

## References

- Whitepaper §6 — Workload Drivers (the driver table; `process` row
  renames to `exec`).
- ADR-0022 — `AppState::driver: Arc<dyn Driver>` extension.
- ADR-0023 — Action shim placement.
- ADR-0026 — cgroup v2 direct writes (Amendment 2026-04-28 revised
  to point here).
- ADR-0027 — Job-stop HTTP shape.
- ADR-0028 — cgroup pre-flight.
- ADR-0029 — `overdrive-worker` crate extraction (Amendment
  2026-04-28 revised to point here).
- HashiCorp Nomad — `exec` task driver:
  https://developer.hashicorp.com/nomad/docs/deploy/task-driver/exec
- `docs/feature/exec-driver-rename/design/wave-decisions.md` — the
  DESIGN-wave summary for this rename feature.
- `docs/feature/exec-driver-rename/deliver/roadmap.json` — the
  two-step crafter roadmap.
- User-supplied "Option C" framing during PR #135 review
  (`phase-1-first-workload`) — the proposal this ADR records.
