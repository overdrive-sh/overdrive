# ADR-0026 — cgroup v2 direct cgroupfs writes (no `cgroups-rs` dep); resource enforcement via `cpu.weight` + `memory.max` from `AllocationSpec::resources`

## Status

Accepted. 2026-04-27. Decision-makers: Morgan (proposing), user
ratification 2026-04-27. Tags: phase-1, first-workload,
application-arch.

## Context

The Phase 1 ProcessDriver (US-02) places workloads in cgroup v2
scopes:

```
overdrive.slice/workloads.slice/<alloc_id>.scope/
```

Two implementation decisions attach:

- **Mechanism**: `cgroups-rs` crate (a maintained MIT-or-Apache-2
  Rust wrapper over the cgroup v2 unified hierarchy) vs direct
  cgroupfs writes (writing to `/sys/fs/cgroup/.../cgroup.procs`
  etc. via plain `std::fs`). Both are viable.
- **Resource enforcement**: write `cpu.weight` and `memory.max`
  on the workload scope at start time using
  `AllocationSpec::resources`, vs defer to a §14 right-sizing
  follow-on.

User decision (D6, D9):

- **D6**: cgroup v2 ONLY (operator confirmed). No cgroup v1
  fallback. Direct cgroupfs writes, no `cgroups-rs` dep.
- **D9**: write `cpu.weight` + `memory.max` in Slice 2 from
  `AllocationSpec::resources`; warn-and-continue on limit-write
  failure.

This ADR records both.

## Decision

### 1. cgroup v2 ONLY

The Phase 1 ProcessDriver supports cgroup v2 (unified hierarchy)
only. cgroup v1 hosts are detected at the pre-flight check
(ADR-0028) and the server refuses to start with an actionable
error.

Rationale: cgroup v2 is the kernel-supported path going forward
(unified hierarchy stable since 4.5; sole hierarchy on every
modern systemd distro). cgroup v1's per-controller mount-point
sprawl, double-accounting, and lack of a stable subtree-control
delegation model add complexity Phase 1 does not need. The
operator confirms cgroup v2 is the deployment target.

The Linux kernel's cgroup v2 unified hierarchy is mounted at
`/sys/fs/cgroup/` on every supported distro
(systemd-cgroup-mount default since Fedora 31, RHEL 9, Ubuntu
22.04, Debian 12). The mount point is detected at runtime (the
pre-flight check verifies it); it is not hard-coded.

### 2. Direct cgroupfs writes — no `cgroups-rs` dep

The driver writes cgroup files directly via `std::fs::write` /
`std::fs::create_dir_all`. The Phase 1 cgroup interactions are:

```
mkdir overdrive.slice/workloads.slice/<alloc_id>.scope    (create)
echo <pid> > .../cgroup.procs                             (place)
echo <weight> > .../cpu.weight                            (limit)
echo <bytes>  > .../memory.max                            (limit)
rmdir overdrive.slice/workloads.slice/<alloc_id>.scope    (remove)
```

Five filesystem operations; no controller discovery, no rich
typing of resource limits, no observable deltas — none of the
features `cgroups-rs` provides earn their keep. A direct
implementation is ~100 lines of Rust against `std::fs` with
explicit error handling and is fully covered by the existing
`integration-tests`-gated Linux integration test (US-02 AC).

### 3. Resource enforcement at start time

When `Driver::start(&AllocationSpec)` is called, the driver:

1. Creates the workload scope directory.
2. Writes `cpu.weight` derived from `spec.resources.cpu_milli`.
3. Writes `memory.max` derived from `spec.resources.memory_bytes`.
4. Writes the child PID into `cgroup.procs`.
5. Returns the `AllocationHandle`.

The order above is deliberate: limits are written *before* the
PID is placed in the scope. If the child process exceeds its
memory limit between fork and the limit write, the kernel's
default unlimited budget applies — but with the limits-first
ordering, the moment the PID lands in the scope it is already
under the declared bounds.

`cpu.weight` derivation: cgroup v2's `cpu.weight` accepts values
in `[1, 10000]` with default `100`. `cpu_milli` is a millicore
count (1000 = one core). The derivation maps `cpu_milli` linearly
into the weight space:

```
cpu.weight = clamp((cpu_milli * 100) / 1000, 1, 10000)
           = clamp(cpu_milli / 10, 1, 10000)
```

The mapping is approximate by design. cgroup v2 cpu weight is a
proportional share, not a hard cap; jobs with higher weight get
more CPU when contention exists, but they do not get a per-job
hard ceiling. Phase 2+ may switch to `cpu.max` (period+quota)
for hard caps when right-sizing arrives; Phase 1 weight is the
honest single-node default (no contention => everyone gets what
they need; under contention, weight ratios apply).

`memory.max` derivation: direct mapping. `memory.max` accepts a
byte count or the literal string `max`. Phase 1 always writes
the byte count; the spec's `memory_bytes` is validated non-zero
at `Job::from_spec` so the write is always meaningful.

### 4. Warn-and-continue on limit-write failure

If the limit write fails — e.g. the controller is delegated but
not enabled in `cgroup.subtree_control` for the parent slice;
the file is read-only for the running UID; the kernel rejects
the value — the driver:

1. Logs a structured warning naming the failed write
   (`cpu.weight` or `memory.max`), the alloc id, the underlying
   error, and the actionable fix.
2. Proceeds with the start — placing the PID into the scope
   *without* the limit applied.
3. The `AllocStatusRow` is written with `state: Running` (not
   `Failed`) because the workload itself is healthy.

Warn-and-continue is the right disposition because:

- A scope-creation failure or a `cgroup.procs` write failure is
  fatal — the workload is not isolated and may interfere with
  the control plane. The driver returns
  `DriverError::SpawnFailed` and the action shim writes
  `state: Failed`.
- A limit-write failure is recoverable in operator-actionable
  ways (enable the controller, grant delegation, fix permissions)
  and the workload itself is correctly placed in the scope. The
  scope provides isolation; the limit provides bounding. Phase 1
  prioritises isolation over bounding when the two diverge.

The warning is logged via `tracing::warn!` and is structured —
the operator's log filter can grep for the specific warning
shape. Phase 2+ may surface this as an observation row
(`alloc_status.warnings` or similar) so the operator sees the
under-limit state in `overdrive alloc status`; Phase 1 keeps it
in logs.

### 5. Cleanup: `rmdir` after process reap

When `Driver::stop(handle)` completes, the driver waits for
process reap, then `std::fs::remove_dir(&handle.cgroup_path)`.
The kernel allows the rmdir only when the scope is empty (no
PIDs); the driver waits for the kill+reap to complete before
attempting removal.

If the rmdir fails (race with another process inheriting the
cgroup, manual operator intervention, etc.), the driver logs a
structured warning and returns `Ok(())`. The workload is dead;
the orphaned cgroup directory is cosmetic, not functional. A
future Phase 2+ janitor reconciler may sweep stale scopes; Phase
1 leaves them for the operator (a `find /sys/fs/cgroup/overdrive.slice
-name '*.scope' -empty -delete` cron is the manual mitigation).

## Alternatives considered

### Alternative A — `cgroups-rs` crate

Use `cgroups-rs` (MIT-or-Apache-2, ~30k downloads/month, last
release 2024) for cgroup interactions.

**Rejected.** `cgroups-rs` is a richer abstraction over both
cgroup v1 and v2, with controller discovery, stat structs, and
event-fd integration. Phase 1 needs none of that. The five-file
direct-write surface is small enough that the dep cost
(transitive deps including `nix`, `libc` features, etc.) is not
justified. Direct writes are also easier to debug — the
operator can manually reproduce any cgroup operation the driver
does (`echo 100 > /sys/fs/cgroup/.../cpu.weight`), there is no
abstraction layer to step through.

This may flip in Phase 2+ when right-sizing introduces
event-fd-based pressure detection (`memory.pressure`,
`cpu.stat`, `io.pressure` poll loops). At that point
`cgroups-rs`'s event-fd integration earns its keep. Phase 1
direct writes are extractable into a thin wrapper if the
migration ever becomes worthwhile.

### Alternative B — cgroup v1 fallback

Detect cgroup v2 vs v1 at startup; use v1 controllers when v2 is
not delegated.

**Rejected.** cgroup v1 is the legacy hierarchy on every modern
distro — RHEL 8 (released 2019) is the last major LTS where v1
was default-mounted, and even there v2 is usable with
`systemd.unified_cgroup_hierarchy=1`. The complexity of dual
support — dual mount-point discovery, dual file layouts, dual
controller semantics, dual delegation models — is wholly
disproportionate to the actual operator base who would benefit.
Operators on cgroup v1 hosts get an actionable error from the
ADR-0028 pre-flight check telling them how to enable v2.

### Alternative C — Defer resource enforcement to §14

Slice 2 lands ProcessDriver without limit writes; a Phase 2+
right-sizing reconciler writes limits later.

**Rejected on user pre-decision (D9).** The data is already
present on `AllocationSpec::resources` — `cpu_milli` and
`memory_bytes` are validated by `Job::from_spec`. Writing the
two cgroup files at start time costs two `fs::write` calls and
matches the `Resources`-on-spec promise. Deferring means the
spec carries declared resource bounds that *do nothing* until
Phase 2+ wires them; an operator running under load would see
the workload using more memory than they declared and reasonably
ask "what does the spec field do?" — and the answer would be
"nothing yet." That answer is not honest enough for a Phase 1
walking-skeleton convergence demonstration. Limits in Phase 1.

### Alternative D — Hard-fail on limit-write failure

Treat limit-write failure as a fatal error; return
`DriverError::SpawnFailed`; do not start the workload.

**Rejected.** A delegated cgroup hierarchy with the `cpu`
controller enabled but not `memory` (or vice versa) is a
common mid-rollout state — a developer running on a host where
they granted delegation but did not enable both controllers.
Hard-failing makes Phase 1 brittle in exactly the cases where
it should be most forgiving (developer machines, non-systemd
test VMs). Warn-and-continue is the right disposition — it
gives the operator information without blocking the
walking-skeleton demo.

The hard refusal pattern is reserved for cases where the
unsafe-to-proceed property is genuinely architectural:
ADR-0028's cgroup delegation pre-flight refuses to start when
*no* delegation is present (because the control-plane slice
itself cannot be created). That is unsafe to proceed; partial
controller enablement is not.

## Consequences

### Positive

- **No dep added.** `overdrive-host`'s Cargo.toml gains nothing
  for cgroup work; the existing tokio + std deps cover the
  needed I/O.
- **Operator-debuggable.** Every cgroup operation maps to a
  shell-equivalent command (`echo X > /sys/fs/cgroup/...`) the
  operator can reproduce manually.
- **Resource enforcement at start time matches the spec
  contract.** The `Resources` field on `AllocationSpec` does
  what it says.
- **Linux-only failure path is explicit.** macOS / Windows dev
  hosts run in the default lane via `SimDriver`; cgroup-
  specific code never compiles into the production driver
  binary on non-Linux targets (the `ProcessDriver` module is
  `#[cfg(target_os = "linux")]`).
- **Phase 2+ migration path is open.** If a future ADR adopts
  `cgroups-rs` for richer features, the existing five direct-
  write call sites are mechanically refactored — the
  `Driver::start` signature stays the same.

### Negative

- **Phase 1 cgroup error handling is hand-rolled.** Each of
  the five operations needs its own error variant in
  `DriverError`. `cgroups-rs` would have given a single typed
  error envelope. Acceptable cost — the variants are
  enumerable and stable.
- **Partial limit application is observably silent in Phase
  1.** A workload running without its memory limit applied
  shows `Running` in `overdrive alloc status`; the operator
  has to read logs to discover the under-limit state. Phase 2+
  observation surfacing closes this gap; Phase 1 logs are the
  Phase 1 channel.
- **`cpu.weight` is a proportional share, not a hard cap.** A
  spec declaring `cpu_milli: 100` (one tenth of a core) does
  not get capped at 100m of CPU under low contention — it gets
  whatever it can use. This matches cgroup v2's design
  philosophy (weights for fair sharing, `cpu.max` for hard
  caps); Phase 2+ right-sizing may adopt `cpu.max` when
  bounded enforcement is needed.

### Quality-attribute impact

- **Reliability — fault tolerance**: positive. Warn-and-continue
  on limit-write failures keeps the convergence loop closing
  even when the cgroup configuration is partial.
- **Maintainability — analyzability**: positive. Direct cgroupfs
  writes match shell commands operators already know.
- **Maintainability — modifiability**: marginally positive.
  Direct writes localised to one Rust file in `overdrive-host`;
  changes are mechanical.
- **Performance — time behaviour**: positive. Five `fs::write`
  syscalls per workload start; sub-millisecond on warm cache.
- **Security — confidentiality / integrity**: neutral. Cgroup
  files are kernel-managed; the writes are no more privileged
  than the running process already is.

## Compliance

- **Whitepaper §4** (workload isolation on co-located nodes):
  the workload scope is created at
  `overdrive.slice/workloads.slice/<alloc_id>.scope`. Compliant.
- **Whitepaper §6** (workload drivers): `Driver::start` /
  `Driver::stop` signatures unchanged. The cgroup operations
  are internal implementation; the trait surface stays pure.
- **`development.md` § Errors**: limit-write failures use the
  existing `DriverError` envelope; no new error variants
  outside the `thiserror`-pattern.
- **`development.md` § Tests gating**: real cgroup operations
  go behind the `integration-tests` feature; default-lane
  tests use `SimDriver`. Compliant.
- **ADR-0023** (action shim): the shim continues to call
  `Driver::start` / `Driver::stop` synchronously. The cgroup
  operations are inside those calls; the shim sees only the
  trait surface.
- **ADR-0028** (cgroup pre-flight refusal): cgroup v2
  detection at boot is hard-refusal; this ADR's runtime
  operations are within the post-pre-flight scope where v2 is
  guaranteed.

## References

- ADR-0022 — `AppState::driver` extension; the Phase 1 driver
  is `ProcessDriver`.
- ADR-0023 — Action shim placement; the shim's `Driver::start`
  call site.
- ADR-0027 — Job-stop HTTP shape; the stop side of the same
  cgroup lifecycle.
- ADR-0028 — cgroup pre-flight refusal; the v1-vs-v2 detection
  this ADR depends on.
- Whitepaper §4 — Workload isolation on co-located nodes.
- Whitepaper §6 — Workload drivers; ProcessDriver row.
- Whitepaper §14 — Right-sizing; future home for richer cgroup
  manipulation.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — Priority Two items 6 + 9 enumerate the decisions; D6 + D9
  user ratification.
- `docs/feature/phase-1-first-workload/discuss/user-stories.md`
  — US-02 ProcessDriver acceptance criteria.
- Linux kernel `Documentation/admin-guide/cgroup-v2.rst` — the
  authoritative source for `cpu.weight` and `memory.max`
  semantics.

## Amendment 2026-04-27 — Worker Crate Extraction

This ADR's body is unchanged in substance — cgroup v2 is still the
sole supported hierarchy, direct cgroupfs writes remain the chosen
mechanism, `cpu.weight` and `memory.max` are still written from
`AllocationSpec::resources` at start time, and warn-and-continue is
still the disposition for limit-write failures. **What changes is
which crate hosts which half of the cgroup work.** The amendment
splits the responsibility cleanly along the control-plane vs worker
boundary established by ADR-0029:

| Cgroup half | Original (this ADR's body) | Amended (ADR-0029) |
|---|---|---|
| Workload cgroups (`overdrive.slice/workloads.slice/<alloc_id>.scope` — create, write `cpu.weight` / `memory.max`, place PID, `rmdir` on stop) | `overdrive-host::driver::process` | **`overdrive-worker`** (e.g. `overdrive-worker::cgroup_manager::workload`; precise path is the crafter's call) |
| Control-plane cgroup (`overdrive.slice/control-plane.slice/` — create, enrol the control-plane process at `overdrive serve` boot, pre-flight check) | `overdrive-control-plane` | **`overdrive-control-plane`** (e.g. `overdrive-control-plane::cgroup_manager::control_plane`; unchanged) |

The split is **cleaner than the unified placement** in this ADR's
original body. Each subsystem owns its own cgroup hierarchy: the
worker manages workload scopes for the allocations it runs, and the
control plane manages its own slice for the control-plane process
isolation. This mirrors whitepaper §4 *Workload Isolation on
Co-located Nodes* exactly — the diagram

```
/overdrive.slice/
  control-plane.slice/    ← control-plane subsystem owns this
    raft.service
    scheduler.service
    ca.service
  workloads.slice/        ← worker subsystem owns this
    job-payments.scope
    job-frontend.scope
```

draws the same boundary the crate graph now reflects.

**The boundary is fixed, the path is the crafter's call.** The
exact module paths inside each crate (e.g. `cgroup_manager` vs
`cgroup` vs `cgroupfs`) are an implementation decision; what is
architecturally fixed is *which crate hosts which half*.

The cgroup-v2-only constraint (operator confirmed; v1 hosts get an
actionable error from ADR-0028's pre-flight), the direct
cgroupfs-writes choice (no `cgroups-rs` dep), the `cpu.weight` /
`memory.max` derivation rules, and the warn-and-continue posture on
limit-write failures all carry across the split unchanged. ADR-0028's
pre-flight check stays in the control-plane crate (it gates whether
*any* of the `overdrive.slice/*` hierarchy can be written) and runs
before either subsystem begins its own cgroup work.

See ADR-0029 for the extraction rationale and the binary-composition
pattern.

## Amendment 2026-04-28 — Exec driver rename + `AllocationSpec.args`

This ADR's body is unchanged in substance — cgroup v2 is still the
sole supported hierarchy, direct cgroupfs writes remain the chosen
mechanism, `cpu.weight` and `memory.max` are still written from
`AllocationSpec::resources` at start time, and warn-and-continue is
still the disposition for limit-write failures. **What changes is
the driver type name and the spec field surface the cgroup
operations are keyed off.**

### Renames

| Old | New | Rationale |
|---|---|---|
| `ProcessDriver` (struct, `overdrive-worker`) | **`ExecDriver`** | Aligns with Nomad's `exec` driver (https://developer.hashicorp.com/nomad/docs/deploy/task-driver/exec) and Talos's vocabulary. "Process" was an internal-implementation noun (we use `tokio::process`); the operator-facing concept is "execute a binary directly," which Nomad and the wider operator community already call `exec`. |
| `DriverType::Process` (enum variant, `overdrive-core`) | **`DriverType::Exec`** | Same. The driver-type enum is the operator-facing identity (it appears in job specs and in `Driver::r#type()`); using the operator-canonical noun matters more here than internal symmetry with `tokio::process`. |
| `AllocationSpec.image: String` | **`AllocationSpec.command: String`** | Container-image terminology is wrong for an exec driver — `/bin/sleep` is a binary path, not a content-addressed image identifier (`docker.io/library/postgres:15`). The current field is read by `ExecDriver` as `Command::new(&spec.image)`, which is the give-away that the field is misnamed. Rename to match Nomad's `exec` task driver field name (`command`). Container drivers (Phase 2+ MicroVm + Wasm) will carry their own ContentHash-typed `image` field — distinct from the exec driver's `command`. |
| `AllocationSpec` (no argv) | **`AllocationSpec.command: String` + `AllocationSpec.args: Vec<String>`** | The original spec couldn't carry argv; `ExecDriver::build_command` papered over the gap with magic image-name dispatch (`/bin/sleep` → hardcoded `["60"]`; `/bin/sh` → hardcoded `["-c", "trap '' TERM; sleep 60"]`; `/bin/cpuburn` → hardcoded busy-loop sh script). That dispatch is technical debt: the driver's production code is reading test-fixture intent. The right shape is the spec carries argv, the driver runs `Command::new(&spec.command).args(&spec.args)`, and test fixtures construct argv inline at the test (e.g. SIGTERM-trap test → `command: "/bin/sh", args: ["-c", "trap '' TERM; sleep 60"]`). |

### Why this lives as an amendment to ADR-0026, not a new ADR

The cgroup-v2 mechanism, the `cpu.weight` / `memory.max` derivation,
the warn-and-continue posture, the limits-then-PID ordering, and the
five-filesystem-operation surface are all unchanged. The amendment
records a *naming* and *spec-shape* change that affects which symbols
the driver writes against, but the cgroup work itself — the body of
this ADR — stays exactly as written. ADR-0026 amendment-in-place
matches the established pattern (ADR-0026's first amendment by
ADR-0029 used the same shape; ADR-0022 / ADR-0025 / ADR-0023 have
similar amendments-in-place).

### Single-cut greenfield migration

Per `feedback_single_cut_greenfield_migrations`, the migration is a
single cohesive PR per step, no compatibility shim. Every fixture in
the workspace migrates in lockstep with the spec field rename:

- `crates/overdrive-core/src/traits/driver.rs` — `AllocationSpec`
  field rename (`image` → `command`) and `args: Vec<String>` field
  add land in the same edit.
- `crates/overdrive-core/src/reconciler.rs` — the one
  `AllocationSpec { ..., image: "/bin/sleep".to_string(), ... }`
  construction in `JobLifecycle::reconcile` migrates to the new
  shape (`command: "/bin/sleep".to_string(), args: vec!["60".to_string()]`).
- `crates/overdrive-control-plane/src/action_shim.rs` —
  `build_phase1_restart_spec` migrates to the new shape.
- `crates/overdrive-worker/src/driver.rs` — `build_command`'s magic
  image-name dispatch is **deleted entirely**; the new body is a
  simple `Command::new(&spec.command).args(&spec.args)`. The setsid
  + TERM-trap workaround at the existing `if spec.image == "/bin/sh"`
  branch becomes unconditional (every exec workload gets its own
  process group; that is the correct shape and was only conditional
  because the magic dispatch needed a switch site).
- `crates/overdrive-sim/tests/acceptance/sim_adapters_deterministic.rs`,
  `crates/overdrive-worker/tests/acceptance/sim_driver_only_in_default_lane.rs`,
  every fixture under
  `crates/overdrive-worker/tests/integration/process_driver/*.rs`,
  `crates/overdrive-control-plane/tests/integration/cgroup_isolation/cluster_status_under_burst.rs`
  — every test fixture constructing `AllocationSpec` migrates to
  `command` + `args`. The `cluster_status_under_burst` test (which
  used `image: "/bin/cpuburn"` to trigger the magic dispatch) now
  constructs the CPU-burst command directly:
  `command: "/bin/sh".to_string(), args: vec!["-c".to_string(), "<busy loop script>".to_string()]`.

### What does NOT change

- The five filesystem operations (`mkdir scope`, `cgroup.procs`,
  `cpu.weight`, `memory.max`, `rmdir`).
- The cgroup-v2-only constraint and ADR-0028's pre-flight.
- The warn-and-continue posture on limit-write failures.
- The cgroup-hierarchy ownership split established by the prior
  amendment (worker owns `workloads.slice/<alloc>.scope`;
  control-plane owns `control-plane.slice/`).
- The `overdrive-worker` crate boundary (the renamed `ExecDriver`
  still lives there per ADR-0029; the crate itself is not renamed).
- The `Driver` trait method signatures (`start` / `stop` / `status` /
  `resize`) — the trait surface is unchanged; only the spec type
  carried by `start` changes shape.
- The `DriverType` enum is the structural surface; a future
  `DriverType::MicroVm` / `DriverType::Wasm` will be added without
  reshaping the enum.

See ADR-0029's amendment 2026-04-28 for the type-rename surface
(`ProcessDriver` → `ExecDriver`) inside the worker crate; this
ADR amendment is the cgroup-side narrative.

