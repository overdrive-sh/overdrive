# Test scenarios — `cgroup-fs-port`

DISTILL-wave specification artifact. **GIVEN / WHEN / THEN prose only —
never parsed or executed.** Per `.claude/rules/testing.md` § "Testing":
no `.feature` files anywhere in this workspace; the DELIVER-wave crafter
materialises Rust `#[test]` / `#[tokio::test]` / `proptest!` / `trybuild`
cases from this document.

Scope: GitHub issue #136 acceptance criteria (8 ACs) + ADR-0054 D1-D5
+ DESIGN reviewer carry-overs (3) + the 4-tier test model from
`.claude/rules/testing.md`.

Tier reminder (project-specific):

| Tag | Tier | Lane | Driving port |
|---|---|---|---|
| `@tier-1` | DST in-process, pure Rust | default (no `--features integration-tests`) | direct method call on the trait object via `Arc::new(SimCgroupFs::new())` |
| `@tier-3` | Real-kernel integration, Lima sudo | `--features integration-tests`, real `/sys/fs/cgroup` | `RealCgroupFs` via `cargo xtask lima run -- cargo nextest run -p overdrive-worker --features integration-tests` |
| `@property` | proptest harness | inherits its tier from where it lives | n/a |
| `@compile-fail` | trybuild | default (no `--features integration-tests`) | n/a (the build IS the test) |
| `@walking-skeleton` | composition-root probe gate (Tier 3) | `--features integration-tests` | `overdrive serve` subprocess; structured `health.startup.refused` event |

`@property` scenarios at Tier 1 use `proptest`; at Tier 3 they stay
example-pinned per `.claude/rules/testing.md` § "Mandate 11: layer 3+ sad
paths are named example-based tests" (workspace-extended discipline).

Cross-cutting wiring: every command shown is the Lima-wrapped form per
`.claude/rules/testing.md` § "Running tests — Lima VM".

---

## Coverage matrix — AC traceability

| AC | Statement | Scenarios | Verdict |
|---|---|---|---|
| AC1 | ADR captures trait-shape decision with rationale | (satisfied by ADR-0054; no runtime test) | met-by-artifact |
| AC2 | `CgroupFs` trait exists with documented contract | A1, A2, B-* (every trait method exercised) | covered |
| AC3 | `RealCgroupFs` in adapter-host wraps `tokio::fs::*` | C-cgroup-kill, C-subtree-control-ebusy, C-controller-validation, C-pseudo-file-synthesis, C-rmdir-auto-reap, C-procs-pid-movement, C-probe-success, C-probe-with-custom-root, C-write-to-readonly-cgroup-file | covered |
| AC4 | `SimCgroupFs` with error-injection schedule (K3-compatible) | B-create_dir, B-write, B-remove_dir, B-probe, B-kind, B-error-schedule, F1 (K3 determinism guard) | covered |
| AC5 | `cgroup_manager` + every adapter-host call site migrated | A1, A2, C-existing-integration-suite-passes, C-write-to-readonly-cgroup-file (real-substrate propagation through `cgroup_manager`), E2 (composition-root probe gate) | covered |
| AC6 | Existing tempfile unit tests converted to SimFs (or kept deliberately) | E1 (authoritative triage matrix — 7 convert / 2 keep-and-move / 2 keep-tempfile / 5 keep-inline) | covered |
| AC7 | DST harness includes filesystem fault scenarios (NotFound, ENOSPC, EBUSY, cancellation) | B-* (NotFound, PermissionDenied, AlreadyExists, DirectoryNotEmpty); D4 cancellation = method-entry deterministic per ADR-0054; partial-writes EXPLICITLY OUT of scope per ADR-0054 § D4 | covered (partial-write scenarios deliberately absent) |
| AC8 | Mutation gate ≥ 80% on `overdrive-worker` post-migration | (not a scenario — surfaced as crafter constraint in handoff) | crafter-constraint |
| AC9 (implicit) | Lima sudo integration suite still green | C-existing-integration-suite-passes (regression-not-broken assertion) | covered |

Total scenarios: **35** across 6 classes (A: 3, B: 18, C: 10, D: 1, E: 2, F: 1).

---

## Class A — trait surface compile + structural enforcement (`@compile-fail`)

### A1: `ExecDriver::new` without `fs` parameter fails to compile

```
@compile-fail @tier-1 @us-AC2 @us-AC5

GIVEN the post-migration `ExecDriver::new` signature is
        `pub fn new(cgroup_root: PathBuf, clock: Arc<dyn Clock>, fs: Arc<dyn CgroupFs>) -> Self`
WHEN  a downstream caller invokes
        `ExecDriver::new(PathBuf::from("/sys/fs/cgroup"), Arc::new(SystemClock))`
        (omitting the third argument)
THEN  rustc rejects the call site with an error matching the pattern
        "this function takes 3 arguments but 2 arguments were supplied"
AND   the trybuild expected-error fixture pins the exact diagnostic
        text so a future signature change requires deliberate fixture
        regeneration.

NOTES
- Implementation: trybuild fixture under
  `crates/overdrive-worker/tests/compile_fail/exec_driver_missing_fs.rs`
  + paired `exec_driver_missing_fs.stderr`.
- Enforces `.claude/rules/development.md` § "Port-trait dependencies"
  → "Required, not defaulted, at the call site".
- Pin trybuild exactly per `.claude/rules/testing.md` § "Compile-fail
  testing (trybuild)" → "Pin trybuild exactly".
```

### A2: Cannot construct `RealCgroupFs` via a default constructor of `ExecDriver`

```
@compile-fail @tier-1 @us-AC2 @us-AC5

GIVEN `ExecDriver` exposes NO `Default` impl AND NO builder method
        whose body inserts `Arc::new(RealCgroupFs::new())`
WHEN  a downstream caller writes `let driver: ExecDriver = Default::default();`
THEN  rustc rejects with "the trait bound `ExecDriver: Default` is not satisfied"
AND   the trybuild fixture under
        `crates/overdrive-worker/tests/compile_fail/exec_driver_no_default.rs`
        pins the error.

NOTES
- Structural defense against the anti-pattern in
  `.claude/rules/development.md` § "Port-trait dependencies" →
  "Builder-pattern overrides (`with_clock`, `with_transport`) are an
  anti-pattern".
- Complements A1: A1 catches "wrong arity"; A2 catches "production
  binding sneaking in via Default".
```

### A3: Workspace dep gate (informational only — already enforced by dst-lint)

```
@informational @us-AC2

NOTE — NOT a new scenario; documented here so the crafter does not
duplicate enforcement.

GIVEN `overdrive-sim` is listed only in `[dev-dependencies]` of every
        consumer (per `.claude/rules/development.md` § "Port-trait
        dependencies")
AND   `overdrive-host` is listed only in `[dependencies]` of binaries
        (never `[dev-dependencies]`)
WHEN  the existing `xtask::dst_lint` AST scanner runs on PR
THEN  the constraint is structurally enforced

The crafter does NOT need to add a new test for A3 — the dst-lint
gate at `xtask/src/dst_lint.rs` already fires on violations. If
`overdrive-sim` is mistakenly added to `[dependencies]` of any
production crate, the lint fails the PR.
```

---

## Class B — SimCgroupFs Tier 1 default-lane scenarios

All B scenarios run via `cargo xtask lima run -- cargo nextest run -p overdrive-worker`
(no `--features integration-tests` flag). They live under
`crates/overdrive-worker/tests/acceptance/sim_cgroup_fs/` and import
`overdrive_sim::adapters::cgroup_fs::SimCgroupFs` (dev-dep only).

Every scenario constructs `let fs = Arc::new(SimCgroupFs::new())` then
exercises the trait surface through `Arc<dyn CgroupFs>` (no concrete-
type access — proves the trait is the contract). Snapshot inspection
uses the test-only `SimCgroupFs::snapshot()` accessor per ADR-0054.

### B-create_dir-happy: creates the directory and parents (mkdir -p semantics)

```
@tier-1 @us-AC2 @us-AC4

GIVEN a fresh SimCgroupFs (empty BTreeMap)
WHEN  the test calls
        `fs.create_dir(Path::new("/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-0.scope")).await`
THEN  the call returns `Ok(())`
AND   `fs.snapshot()` contains entries for every intermediate parent
        (`/sys`, `/sys/fs`, ..., `/sys/fs/cgroup/overdrive.slice/workloads.slice/alloc-0.scope`),
        each typed `SimEntry::Dir` with empty bytes
AND   a subsequent `create_dir` against the same path returns `Ok(())`
        (idempotency post-condition per ADR-0054 D1)
```

### B-create_dir-injected-permission-denied: injection schedule pops one entry

```
@tier-1 @us-AC4 @us-AC7

GIVEN a fresh SimCgroupFs
AND   the test calls
        `fs.inject_error(SimOp::CreateDir, PathBuf::from("/blocked"), io::ErrorKind::PermissionDenied)`
WHEN  the first call `fs.create_dir(Path::new("/blocked")).await` runs
THEN  the call returns `Err(e)` where `e.kind() == ErrorKind::PermissionDenied`
AND   the snapshot does NOT contain `/blocked`
WHEN  a second call `fs.create_dir(Path::new("/blocked")).await` runs
THEN  it returns `Ok(())` (schedule queue is now empty; the call proceeds normally)
AND   the snapshot now contains `/blocked` typed `SimEntry::Dir`
```

### B-write-happy: full content equals bytes; idempotent overwrite

```
@tier-1 @us-AC2 @us-AC4

GIVEN a fresh SimCgroupFs with `/dir` created
WHEN  `fs.write(Path::new("/dir/file"), b"hello\n").await` is called
THEN  the call returns `Ok(())`
AND   the snapshot at `/dir/file` is `(SimEntry::File, b"hello\n".to_vec())`
WHEN  `fs.write(Path::new("/dir/file"), b"world").await` is called
THEN  the call returns `Ok(())`
AND   the snapshot at `/dir/file` is `(SimEntry::File, b"world".to_vec())`
        (overwrite, NOT append; per ADR-0054 D1 postcondition)
```

### B-write-empty-bytes: writes empty file; not an error

```
@tier-1 @us-AC2

GIVEN a fresh SimCgroupFs with `/dir` created
WHEN  `fs.write(Path::new("/dir/empty"), b"").await` is called
THEN  the call returns `Ok(())`
AND   the snapshot at `/dir/empty` is `(SimEntry::File, Vec::new())`

NOTE — per ADR-0054 D1 `write` edge case: "`bytes.is_empty()`: writes
an empty file; not an error."
```

### B-write-missing-parent-returns-notfound

```
@tier-1 @us-AC2 @us-AC7

GIVEN a fresh SimCgroupFs (empty BTreeMap; no parent directory created)
WHEN  `fs.write(Path::new("/no/such/parent/file"), b"x").await` is called
THEN  the call returns `Err(e)` where `e.kind() == ErrorKind::NotFound`
AND   the snapshot does NOT contain `/no/such/parent/file`

NOTE — pins the "matches tokio::fs::write on Real" semantics from the
ADR-0054 sim adapter sketch.
```

### B-write-injected-other-returns

```
@tier-1 @us-AC4 @us-AC7

GIVEN a fresh SimCgroupFs with `/dir` created
AND   `fs.inject_error(SimOp::Write, PathBuf::from("/dir/file"), io::ErrorKind::Other)` has been called
WHEN  `fs.write(Path::new("/dir/file"), b"x").await` is called
THEN  the call returns `Err(e)` where `e.kind() == ErrorKind::Other`
AND   the snapshot does NOT contain `/dir/file` (injection short-circuits before mutation)
```

### B-remove_dir-happy

```
@tier-1 @us-AC2

GIVEN a fresh SimCgroupFs with `/dir` created and no children
WHEN  `fs.remove_dir(Path::new("/dir")).await` is called
THEN  the call returns `Ok(())`
AND   the snapshot does NOT contain `/dir`
AND   a subsequent `fs.create_dir(Path::new("/dir")).await` returns `Ok(())`
        (idempotent re-create per ADR-0054 D1 postcondition)
```

### B-remove_dir-notfound

```
@tier-1 @us-AC2 @us-AC7

GIVEN a fresh SimCgroupFs (empty BTreeMap)
WHEN  `fs.remove_dir(Path::new("/missing")).await` is called
THEN  the call returns `Err(e)` where `e.kind() == ErrorKind::NotFound`

NOTE — the `cgroup_manager::remove_workload_scope` wrapper swallows
NotFound to Ok; the trait itself surfaces it. The wrapper's NotFound
tolerance is exercised by E1.scope-removal.
```

### B-remove_dir-directory-not-empty

```
@tier-1 @us-AC2

GIVEN a fresh SimCgroupFs with `/dir` and `/dir/child` both created (file or dir)
WHEN  `fs.remove_dir(Path::new("/dir")).await` is called
THEN  the call returns `Err(e)` where `e.kind() == ErrorKind::DirectoryNotEmpty`
AND   the snapshot still contains both `/dir` and `/dir/child`

NOTE — per ADR-0054 D1 `remove_dir` edge case: "`path` non-empty:
`Err(DirectoryNotEmpty)` on Real." SimCgroupFs MUST honor this check
to keep the equivalence with Real meaningful.
```

### B-remove_dir-injected-permission-denied

```
@tier-1 @us-AC4 @us-AC7

GIVEN a fresh SimCgroupFs with `/dir` created
AND   `fs.inject_error(SimOp::RemoveDir, PathBuf::from("/dir"), io::ErrorKind::PermissionDenied)` is set
WHEN  `fs.remove_dir(Path::new("/dir")).await` is called
THEN  the call returns `Err(e)` where `e.kind() == ErrorKind::PermissionDenied`
AND   the snapshot STILL contains `/dir` (injection short-circuits before mutation)
```

### B-probe-happy: round-trip succeeds against in-memory store

```
@tier-1 @us-AC2 @us-AC4

GIVEN a fresh SimCgroupFs with no errors injected
WHEN  `fs.probe().await` is called
THEN  the call returns `Ok(())`
AND   after the probe, the snapshot does NOT contain the probe
        directory (probe is self-cleaning per ADR-0054)
```

### B-probe-injected-substrate-error: returns ProbeError::Substrate

```
@tier-1 @us-AC4

GIVEN a fresh SimCgroupFs
AND   `fs.inject_error(SimOp::CreateDir, PathBuf::from("/sim-probe-root"), io::ErrorKind::PermissionDenied)` is set
WHEN  `fs.probe().await` is called
THEN  the call returns `Err(ProbeError::Substrate { source })`
AND   `source.kind() == ErrorKind::PermissionDenied`

NOTE — exercises the DST refuse-to-start path. The same error class
fires at the composition root in scenario E2.
```

### B-probe-round-trip-mismatch: returns ProbeError::RoundTripMismatch

```
@tier-1 @us-AC2

GIVEN a SimCgroupFs whose probe round-trip would observe a byte mismatch
        (test hook: a SimCgroupFs variant whose `Write` stores corrupted
         bytes — implementation detail; the simplest path is a second
         injection variant `SimOp::Write` with a "corrupt-payload" sentinel,
         OR a test-only `inject_corruption(path)` helper that overwrites
         the BTreeMap entry between probe write and probe read-back)
WHEN  `fs.probe().await` is called
THEN  the call returns `Err(ProbeError::RoundTripMismatch { wrote, read })`
AND   `wrote == b"probe\n"`
AND   `read != wrote`

NOTE — this is the load-bearing Earned Trust shape. It defends against
the substrate-lies-about-write failure mode (overlayfs fsync no-op,
WSL2 DrvFs caching) at the abstraction-level pattern. The DELIVER
crafter decides the exact injection mechanism; the SCENARIO pins the
observable contract.
```

### B-kind-returns-stable-string

```
@tier-1 @us-AC2

GIVEN a fresh SimCgroupFs
WHEN  `fs.kind()` is called
THEN  the call returns `"overdrive_sim::SimCgroupFs"`
AND   the return value is `&'static str` (no allocation per call)

NOTE — operators grep on this string in startup logs per ADR-0054 D1
trait docstring. Pin the exact value to catch accidental rename.
```

### B-error-schedule-determinism: injection key ordering is BTreeMap

```
@tier-1 @us-AC4

GIVEN a fresh SimCgroupFs with two injections set:
        - `(SimOp::Write, "/a")` -> PermissionDenied
        - `(SimOp::Write, "/b")` -> Other
WHEN  two test runs execute the same sequence of calls
        (`write(/a, ...)`, `write(/b, ...)`, snapshot, drop)
THEN  both runs produce bit-identical error sequences (PermissionDenied
        then Other) and bit-identical final snapshots

NOTE — pins the K3 reproducibility property (seed → bit-identical
trajectory) at the SimCgroupFs level. The full DST guard is F1; this
scenario is the per-injection-schedule micro-check.
```

### B-write-then-snapshot-is-deterministic-across-runs

```
@tier-1 @us-AC4

GIVEN a fresh SimCgroupFs
WHEN  the test inserts paths `["/zeta", "/alpha", "/middle"]` via
        `create_dir` then `write` against each
AND   the test calls `fs.snapshot()`
THEN  the snapshot's iteration order is sorted (`/alpha`, `/middle`, `/zeta`)
        because the internal store is `BTreeMap` per ADR-0054 § Sim adapter
AND   two runs against the same insertion sequence produce
        bit-identical snapshot iteration order

NOTE — pins the `BTreeMap` choice required by
`.claude/rules/development.md` § "Ordered-collection choice".
A `HashMap` regression would surface as a flaky test on this scenario.
```

### B-write-respects-cgroup-procs-pid-payload

```
@tier-1 @us-AC2

GIVEN a fresh SimCgroupFs with the workload scope path created
WHEN  the test calls
        `fs.write(scope.resolve(root).join("cgroup.procs"), b"1234\n").await`
THEN  the call returns `Ok(())`
AND   the snapshot at `<scope>/cgroup.procs` is `(SimEntry::File, b"1234\n".to_vec())`

NOTE — this is the SimCgroupFs-side replacement for the existing
`place_pid_in_scope_writes_pid_to_cgroup_procs` tempfile test (E1 row 5).
Asserts on byte payload, NOT on PID movement (which is a kernel-side
effect SimCgroupFs cannot model per ADR-0054 § D3).
```

### B-write-cgroup-kill-stores-one-newline

```
@tier-1 @us-AC2

GIVEN a fresh SimCgroupFs with the workload scope path created
WHEN  the test calls `fs.write(<scope>/cgroup.kill, b"1\n").await`
THEN  the call returns `Ok(())`
AND   the snapshot at `<scope>/cgroup.kill` is `(SimEntry::File, b"1\n".to_vec())`

NOTE — this is the SimCgroupFs-side replacement for the existing
`cgroup_kill_writes_one_to_cgroup_kill_file` tempfile test (E1 row 4).
Asserts on byte payload, NOT on process termination (per ADR-0054 § D3).
The kernel-side mass-kill is exercised by C-cgroup-kill.
```

---

## Class C — RealCgroupFs Tier 3 scenarios (Lima sudo, `integration-tests` feature)

All C scenarios run via:
```
cargo xtask lima run -- cargo nextest run -p overdrive-worker \
  --features integration-tests -E 'binary(integration)'
```

They live under `crates/overdrive-worker/tests/integration/real_cgroup_fs/`
(new directory under the existing `tests/integration/` tree per
`.claude/rules/testing.md` § "Layout — integration tests live under
`tests/integration/`"). Each C scenario:
- Constructs `let fs = Arc::new(RealCgroupFs::new())`.
- Uses the per-test `AllocCleanup` RAII guard pattern from
  `crates/overdrive-worker/tests/integration/exec_driver/cleanup.rs`
  to guarantee leftover scopes are reaped on panic / SIGKILL (per
  `.claude/rules/testing.md` § "Leaked workload cgroups across runs").
- Asserts on KERNEL-side effects per ADR-0054 § D3.

### C-cgroup-kill: write `1\n` to `cgroup.kill` terminates every PID in the scope

```
@tier-3 @real-io @adapter-integration @us-AC3

GIVEN a real Linux host (Lima VM) with cgroup v2 delegated to the test UID
AND   `RealCgroupFs::new()` constructed
AND   a workload scope `overdrive.slice/workloads.slice/alloc-killC-0.scope`
        created under `/sys/fs/cgroup` containing a `/bin/sleep 3600` PID
        (spawned via `nix::unistd::fork` + `execve` into the scope per
         the existing integration-test pattern)
WHEN  the test calls
        `fs.write(<scope>/cgroup.kill, b"1\n").await`
THEN  the call returns `Ok(())`
AND   within ≤ 2 seconds wall-clock (polling `waitpid(WNOHANG)`),
        the `/bin/sleep 3600` PID has been reaped by the kernel
AND   `cleanup` (the RAII guard) reaps the empty scope on Drop

NOTE — exercises kernel-side semantic ADR-0054 § D3 row 1
(cgroup.kill atomic mass-kill).
SimCgroupFs CANNOT exercise this; this scenario is the WHY SimFs is
non-replacement.
```

### C-subtree-control-ebusy: writing `+memory` with a live child returns EBUSY

```
@tier-3 @real-io @adapter-integration @us-AC3

GIVEN a real Linux host with cgroup v2 delegated
AND   `overdrive.slice/workloads.slice/alloc-EBUSY-0.scope` exists
        and contains a live `/bin/sleep 3600` PID
WHEN  the test calls
        `fs.write(overdrive.slice/workloads.slice/cgroup.subtree_control, b"+memory\n").await`
THEN  the call returns `Err(e)` where one of:
        - `e.kind() == ErrorKind::ResourceBusy`, OR
        - `e.raw_os_error() == Some(libc::EBUSY)`
AND   the typed `WorkloadsBootstrapError::from_subtree_control_io(e)`
        classifier dispatches to `SubtreeControlBusy { source }`

NOTE — kernel-side ADR-0054 § D3 row 2. Pins the same EBUSY
discrimination the existing `WorkloadsBootstrapError` typed enum
performs — the trait-object refactor must not regress this.
```

### C-controller-validation: writing malformed value to `cpu.weight` returns EINVAL

```
@tier-3 @real-io @adapter-integration @us-AC3

GIVEN a real Linux host with cgroup v2 delegated
AND   workload scope `<scope>` exists with `cpu.weight` writable
WHEN  the test calls `fs.write(<scope>/cpu.weight, b"99999999\n").await`
        (cgroup v2 accepts 1..=10000; this value is out-of-range)
THEN  the call returns `Err(e)` where `e.kind() == ErrorKind::InvalidInput`
        OR `e.raw_os_error() == Some(libc::EINVAL)`

NOTE — kernel-side ADR-0054 § D3 row 3. The kernel parses and validates
the value; SimCgroupFs accepts arbitrary bytes. The `cpu_weight_for`
clamp helper means production never writes out-of-range values — this
scenario is the structural defense against a future refactor that
removes the clamp.
```

### C-pseudo-file-synthesis: opening `cgroup.events` returns a real file

```
@tier-3 @real-io @adapter-integration @us-AC3

GIVEN a real Linux host with cgroup v2 delegated
AND   workload scope `<scope>` exists
WHEN  the test reads `<scope>/cgroup.events` via the host adapter
        (NOT via `CgroupFs::write` — this scenario uses `tokio::fs::read`
         directly because the trait does not expose a `read` method,
         only the probe does internally; the scenario asserts on the
         substrate behaviour the probe relies on)
THEN  the file exists and contains a body matching the regex
        `(?m)^(populated|frozen) [01]$`

NOTE — kernel-side ADR-0054 § D3 row 4 (the kernel synthesises
pseudo-files at mkdir time; SimCgroupFs creates only the directory).
The cgroup manager doesn't read this file today; the scenario defends
the substrate assumption that future code (e.g. an `EventsObserver`
reconciler) would rely on.
```

### C-rmdir-auto-reap: empty scope `rmdir` succeeds and the kernel reclaims it

```
@tier-3 @real-io @adapter-integration @us-AC3

GIVEN a real Linux host with cgroup v2 delegated
AND   workload scope `<scope>` exists with NO live PIDs in cgroup.procs
WHEN  the test calls `fs.remove_dir(<scope>).await`
THEN  the call returns `Ok(())`
AND   `<scope>` no longer exists in `/sys/fs/cgroup/...`
AND   the test does NOT need to remove individual pseudo-files
        (`cgroup.procs`, `cpu.weight`, `memory.max`, etc.) — the
        kernel reaps them automatically

NOTE — kernel-side ADR-0054 § D3 row 5. Pins the assumption baked into
`cgroup_manager::remove_workload_scope` rustdoc: "the kernel-managed
virtual files inside a workload scope cannot be `unlink`ed individually
and are reaped automatically by `rmdir(2)`."
```

### C-procs-pid-movement: writing PID to `cgroup.procs` actually moves the process

```
@tier-3 @real-io @adapter-integration @us-AC3

GIVEN a real Linux host with cgroup v2 delegated
AND   workload scope `<scope>` exists
AND   a `/bin/sleep 3600` child PID `P` running in the test process's cgroup
        (NOT yet in `<scope>`)
WHEN  the test calls `fs.write(<scope>/cgroup.procs, format!("{P}\n").as_bytes()).await`
THEN  the call returns `Ok(())`
AND   reading `/proc/<P>/cgroup` shows the PID is now in `<scope>`
        (the line ending with `<scope>` path)
AND   `cleanup` reaps via `cgroup.kill` + `rmdir`

NOTE — kernel-side ADR-0054 § D3 row 6 (PID movement). The `place_pid_in_scope`
cgroup_manager method depends on this. SimCgroupFs only stores the byte
payload; this scenario is the structural defense at the substrate level.
```

### C-probe-success: `RealCgroupFs::probe()` succeeds against real `/sys/fs/cgroup`

```
@tier-3 @real-io @walking-skeleton @us-AC3

GIVEN a real Linux host with cgroup v2 delegated AND
        `/sys/fs/cgroup` writable by the test UID (i.e. the Lima sudo
         environment per `.claude/rules/testing.md` § "Running tests
         — Lima VM")
AND   `RealCgroupFs::new()` constructed (default probe root = `/sys/fs/cgroup`)
WHEN  `fs.probe().await` is called
THEN  the call returns `Ok(())`
AND   after the probe, `/sys/fs/cgroup/.overdrive-probe-<uuid>/` does NOT exist
        (probe is self-cleaning)
AND   the round-trip read of `b"probe\n"` matched the write

NOTE — proves the Earned Trust probe works in the canonical
production-equivalent environment. The `<uuid>` portion guarantees
parallel-test isolation.
```

### C-probe-with-custom-root: builder override scopes the probe directory

```
@tier-3 @real-io @us-AC3 @reviewer-carryover-2

GIVEN a Lima environment with `/tmp` writable by the test UID
AND   `let tmp = tempfile::TempDir::new().expect("tempdir")`
AND   `let fs = RealCgroupFs::new().with_probe_root(tmp.path().to_path_buf())`
WHEN  `fs.probe().await` is called
THEN  the call returns `Ok(())`
AND   the probe directory `<tmp>/.overdrive-probe-<uuid>/` was created and
        then removed (transient — the assertion fires before/after via
        an `fs::read_dir(tmp.path())` snapshot showing an empty dir
        post-probe)
AND   the probe did NOT touch `/sys/fs/cgroup` at all
        (asserted via inspecting `/sys/fs/cgroup` contents pre/post —
         the contents must be byte-identical)

NOTE — reviewer carry-over #2. Validates that the builder override
genuinely scopes the probe and is therefore usable as a test fixture
for non-Lima environments (e.g. CI runners without sudo). Also exercises
the `with_probe_root` field assignment is consumed by `probe()`.
```

### C-existing-integration-suite-passes: regression-not-broken after constructor change

```
@tier-3 @real-io @regression @us-AC5 @us-AC9

GIVEN the post-migration `ExecDriver::new(cgroup_root, clock, fs)` signature
WHEN  the existing 12 integration tests under
        `crates/overdrive-worker/tests/integration/exec_driver/*.rs`
        are run via
        `cargo xtask lima run -- cargo nextest run -p overdrive-worker --features integration-tests`
THEN  all 12 tests PASS without modification of their assertions
AND   the only diff vs. pre-migration is the constructor call site
        threading `Arc::new(RealCgroupFs::new())` as the third argument
        (mechanical migration)
AND   leftover-cgroup detection per `.claude/rules/testing.md`
        § "Leaked workload cgroups across runs" shows zero leftovers
        after the suite completes

NOTE — this is a meta-assertion to the crafter: AC9 (implicit from
ADR-0054 "Lima sudo integration suite still green") is satisfied
when the migration is mechanical. The crafter's commit sequence per
ADR-0054 § D5 step 5 lists this explicitly: "Every test fixture
migrates in lockstep". The scenario name in `test-scenarios.md` exists
so the DELIVER checklist has a row for this.
```

### C-write-to-readonly-cgroup-file: real `EACCES` from kernel pseudo-file propagates through `cgroup_manager`

```
@tier-3 @real-io @adapter-integration @us-AC3 @us-AC5

GIVEN a real Linux host (Lima VM) with cgroup v2 delegated to the test UID
AND   `RealCgroupFs::new()` constructed
AND   a workload scope `overdrive.slice/workloads.slice/alloc-roC-0.scope`
        created under `/sys/fs/cgroup` (the kernel synthesises the
        full set of pseudo-files at mkdir time per ADR-0054 § D3 row 4,
        including `cgroup.events` which the kernel marks read-only by
        design — see cgroup v2 docs)
AND   a `CgroupManager` constructed with `Arc::new(RealCgroupFs::new())`
        as its `fs` field
WHEN  the test invokes a `cgroup_manager` operation that ends up calling
        `fs.write(<scope>/cgroup.events, ...)` (test mechanism: either
        an exposed test-only helper on `CgroupManager` that takes a
        relative pseudo-file name, OR direct invocation of the same
        underlying `fs.write` path the public surface uses — the
        DELIVER crafter picks the cleanest exposure; the SCENARIO pins
        the observable that the *real* `io::Error` propagates back
        through `cgroup_manager` to the caller unmodified-in-kind)
THEN  the call returns `Err(e)` where one of:
        - `e.kind() == ErrorKind::PermissionDenied`, OR
        - `e.raw_os_error() == Some(libc::EACCES)`
AND   the error has propagated through `cgroup_manager` from the real
        `tokio::fs::write` syscall — proving the substrate-boundary
        propagation chain (real kernel VFS → `tokio::fs::*` → trait
        method → `cgroup_manager` caller) is intact
AND   `cleanup` (the RAII guard) reaps the empty scope on Drop

NOTES
- Lives at `crates/overdrive-worker/tests/integration/cgroup_manager/write_to_readonly_cgroup_file.rs`
  gated behind `--features integration-tests`.
- Production-realistic trigger: `cgroup.events` is read-only by design
  per cgroup v2 documentation (the kernel rejects writes with `EACCES`);
  a buggy reconciler writing the wrong field name would trip this
  exact path. The error is real-substrate, not contrived (unlike the
  ENOTDIR-via-regular-file-in-dir-slot mechanism of E1 rows 8 and 10).
- This scenario is the **candidate replacement** for E1 row 8
  (`cgroup_kill_propagates_non_notfound_errors`) and E1 row 10
  (`remove_workload_scope_propagates_non_notfound_errors`). Both stay
  as KEEP-TEMPFILE in the matrix until this scenario lands and proves
  equivalent substrate-boundary coverage in production-realistic shape.
  Retirement of rows 8 and 10 is a follow-on DELIVER decision once
  this scenario is green; DISTILL does NOT prescribe the retirement
  in the same PR.
- Option β (C-remove-scope-with-live-process — `EBUSY` from `rmdir`
  on a scope containing a live PID) was considered as an alternative
  realistic trigger and remains a worthwhile follow-on if reconciler-
  removal flake protection becomes a concern. Skipped from this pass
  to keep the new scenario minimal-setup (no process spawning, no
  waitpid polling, no SIGKILL race surface).
- Asserts on KERNEL-side effects per ADR-0054 § D3 (specifically the
  read-only pseudo-file enforcement that's part of the cgroup v2
  semantic SimCgroupFs CANNOT model).
```

---

## Class D — Real/Sim equivalence proptest (reviewer carry-over #1)

### D1: Real and Sim adapters observe identical byte side effects for any sequence of ops

```
@property @tier-3 @real-io @us-AC2 @reviewer-carryover-1

GIVEN a proptest strategy generating sequences of ops:
        `Op = CreateDir(PathBuf) | Write(PathBuf, Vec<u8>) | RemoveDir(PathBuf)`
        with PathBuf restricted to a small bounded alphabet (e.g.
        `/{a,b,c}/{0,1,2}`) and Vec<u8> bounded to ≤ 16 bytes
AND   two adapter constructions:
        - `let real_root = tempfile::TempDir::new().expect("tempdir")`
        - `let real_fs = Arc::new(RealCgroupFs::new())`
        - `let sim_fs = Arc::new(SimCgroupFs::new())`
        (RealCgroupFs operates against the tempdir root — NOT against
         real cgroupfs — because this property tests the BYTE-STORE
         contract, not kernel semantics)
WHEN  the same op sequence is applied to both adapters, prefixing
        each Real path with `real_root.path()` and each Sim path with `/`
THEN  for every op, the Ok/Err verdict matches (same error kind on
        Err; same Ok on Ok)
AND   after the full sequence, for every path P touched:
        - if Real's `tokio::fs::read(real_root.path().join(P))` returns
          `Ok(bytes)`, then Sim's `snapshot().get(P).map(|(_,b)| b)`
          equals `Some(bytes)`
        - if Real returns `Err(NotFound)`, then Sim's snapshot does
          NOT contain P
AND   the proptest runs at the default case count (1024) per
        `.claude/rules/testing.md` § "Property-based testing (proptest)"
        → CI runs default case count per PR

LIMITATIONS (explicitly noted in scenario docstring)
- This property validates the BYTE-STORE contract only. Kernel-side
  effects (cgroup.kill mass-kill, subtree_control EBUSY,
  pseudo-file synthesis on mkdir, EINVAL on malformed values) are
  EXPLICITLY OUT OF SCOPE per ADR-0054 § D3 — those live in Class C.
- RealCgroupFs is rooted at the tempdir, not at `/sys/fs/cgroup`.
  This is INTENTIONAL: the equivalence harness validates that
  RealCgroupFs is a well-behaved tokio::fs::* wrapper, which is the
  contract SimCgroupFs is meant to mirror at byte level.
- Lives behind `--features integration-tests` because it touches real
  filesystem (tempfile). Layer-3 PBT mode is example-only per
  `.claude/rules/testing.md` § "Mandate 11"; this proptest gets a
  special-case waiver because the input space is small (3×3=9 paths,
  ≤16 byte payloads) AND the property runs against in-process
  tokio::fs::* against a tempdir, not real /sys/fs/cgroup. The
  shrinker is the structural defense against generator bugs.
- File path: `crates/overdrive-worker/tests/integration/cgroup_fs_equivalence.rs`
```

---

## Class E — migration regression coverage

### E1: Authoritative tempfile-test triage matrix (7 convert / 2 keep-and-move / 2 keep-tempfile / 5 keep-inline)

```
@authoritative-matrix @us-AC6

NOTE — this is NOT a test scenario per se. It is the binding triage
decision for AC6: per the feature-delta § "Reuse Analysis" row 6
(12 tests today), each test is classified into one of three buckets.
The DELIVER crafter MUST honor this matrix.

The 12 existing tests in `crates/overdrive-worker/src/cgroup_manager.rs`
mod `tests`:

| # | Test name                                                                   | Bucket | Destination                                                                                |
|---|-----------------------------------------------------------------------------|--------|--------------------------------------------------------------------------------------------|
| 1 | `cgroup_path_as_str_returns_canonical_string`                               | KEEP   | Stays in `src/cgroup_manager.rs#[cfg(test)] mod tests` — pure logic, no FS touch          |
| 2 | `cpu_weight_for_pins_division_and_clamp`                                    | KEEP   | Stays inline — pure arithmetic, no FS touch                                                |
| 3 | `from_subtree_control_io_discriminates_ebusy_via_resource_busy_kind`        | KEEP   | Stays inline — pure error discrimination, no FS touch                                      |
| 4 | `from_subtree_control_io_discriminates_ebusy_via_raw_os_error`              | KEEP   | Stays inline — pure error discrimination, no FS touch                                      |
| 5 | `from_subtree_control_io_routes_non_ebusy_to_write_failed`                  | KEEP   | Stays inline — pure error discrimination, no FS touch                                      |
| 6 | `cgroup_kill_is_idempotent_on_missing_scope`                                | CONVERT | New SimCgroupFs-backed test in `tests/acceptance/cgroup_manager/cgroup_kill_idempotent.rs` |
| 7 | `cgroup_kill_writes_one_to_cgroup_kill_file`                                | CONVERT | New SimCgroupFs-backed test (covered by B-write-cgroup-kill-stores-one-newline at the trait level; the CgroupManager-level test asserts the manager invokes `fs.write(<scope>/cgroup.kill, b"1\n")` via SimCgroupFs snapshot) |
| 8 | `cgroup_kill_propagates_non_notfound_errors`                                | KEEP-TEMPFILE | Stays as tempfile-backed test against `RealCgroupFs` in `crates/overdrive-worker/tests/integration/cgroup_manager/cgroup_kill_propagates_real_io_error.rs` (gated behind `--features integration-tests`). Rationale: tests the substrate boundary — that `cgroup_manager::cgroup_kill` correctly propagates a real `io::Error` from a real `tokio::fs::*` syscall against a real kernel VFS, not just the logic of propagation from a fake injected error. ENOTDIR-via-regular-file-in-dir-slot is a contrivance to *trigger* the error; the test *boundary* (real-substrate `io::Error` → propagation through `cgroup_manager`) is real and load-bearing. Candidate for retirement once C-write-to-readonly-cgroup-file (new) proves the same propagation against a production-realistic kernel error. |
| 9 | `remove_workload_scope_is_idempotent_on_missing_scope`                      | CONVERT | New SimCgroupFs-backed test — the `cgroup_manager::remove_workload_scope` wrapper swallows NotFound to Ok; SimCgroupFs returns NotFound when the path is absent (per B-remove_dir-notfound), so the wrapper-level test asserts the wrapper's swallow logic on top |
| 10| `remove_workload_scope_propagates_non_notfound_errors`                      | KEEP-TEMPFILE | Stays as tempfile-backed test against `RealCgroupFs` in `crates/overdrive-worker/tests/integration/cgroup_manager/remove_workload_scope_propagates_real_io_error.rs` (gated behind `--features integration-tests`). Rationale: same as row 8 — tests the substrate boundary that `cgroup_manager::remove_workload_scope` correctly propagates a real `io::Error` from a real `tokio::fs::*` syscall against a real kernel VFS. ENOTDIR-via-regular-file-in-dir-slot is a contrivance to trigger the error; the test boundary is real and load-bearing. Candidate for retirement once C-write-to-readonly-cgroup-file (new) proves the same propagation against a production-realistic kernel error. |
| 11| `create_workload_scope_writes_a_real_directory`                             | CONVERT | New SimCgroupFs-backed test asserting via `snapshot().contains_key(<scope>)` + `SimEntry::Dir` |
| 12| `place_pid_in_scope_writes_pid_to_cgroup_procs`                             | CONVERT | New SimCgroupFs-backed test (the manager-level equivalent of B-write-respects-cgroup-procs-pid-payload) |
| 13| `write_resource_limits_writes_cpu_weight_and_memory_max`                    | CONVERT | New SimCgroupFs-backed test asserting both byte payloads via `snapshot()` |
| 14| `write_resource_limits_warn_on_error_writes_files_on_success`               | CONVERT | New SimCgroupFs-backed test asserting `snapshot()` reflects both files |
| 15| `create_workloads_slice_with_controllers_creates_dir_and_writes_subtree_control` | KEEP-AND-MOVE | Stays tempfile-backed BUT moves to `tests/integration/real_cgroup_fs/workloads_slice_bootstrap.rs` and gates behind `--features integration-tests`. Rationale: `create_workloads_slice_with_controllers` is currently a SYNC function using `std::fs::*`. The async-fn rewrite (to use `&Self.fs` via the manager struct) moves the function under the `CgroupManager` async surface; the CONVERT path is appropriate at that point. **For the migration commit specifically**: this test moves to the SimCgroupFs surface alongside item #16 |
| 16| `create_workloads_slice_with_controllers_is_idempotent`                     | KEEP-AND-MOVE | Same disposition as #15. Idempotency is asserted via two sequential SimCgroupFs invocations + snapshot comparison |

Note — the count above is 16, not 12. The original feature-delta claim
"12 tests" was approximate; the actual count is 16 inline `#[test]` /
`#[tokio::test]` bodies in `src/cgroup_manager.rs` mod `tests`. The
triage classification matches DESIGN feature-delta § Reuse Analysis
verbatim (the only adjustment is this recount from 12 → 16; the
per-test bucket assignments are unchanged). Net effect for AC6:

- **5 stay inline as pure-logic unit tests** (rows 1-5).
- **7 convert to SimCgroupFs-backed tests** under
  `crates/overdrive-worker/tests/acceptance/cgroup_manager/*.rs`
  (rows 6, 7, 9, 11, 12, 13, 14).
- **2 keep-and-move to SimCgroupFs-backed tests** that exercise the
  bootstrap function via the new `CgroupManager` async surface
  (rows 15-16).
- **2 stay tempfile-backed against `RealCgroupFs`** (rows 8, 10) under
  `crates/overdrive-worker/tests/integration/cgroup_manager/*.rs`,
  gated behind `--features integration-tests`. These tests defend the
  substrate boundary — that `cgroup_manager` correctly propagates a
  REAL `io::Error` from a REAL `tokio::fs::*` syscall against the
  REAL kernel VFS, a concern distinct from the *logic* of propagation
  (which the SimCgroupFs-injection tests cover). The ENOTDIR-via-
  regular-file-in-dir-slot mechanism is a contrivance to *trigger*
  the error; the test *boundary* is real. Both are candidates for
  retirement once **C-write-to-readonly-cgroup-file** (new Class C
  scenario added in this pass) proves the same propagation against
  a production-realistic kernel error (`EACCES` from writing a
  kernel-read-only pseudo-file).

Total: 5 + 7 + 2 + 2 = 16.

NOTE — DELIVER step 7 of ADR-0054 § D5 ("Triage existing tempfile
tests") consumes this matrix. The crafter MUST NOT add new tempfile
tests against RealCgroupFs as part of the migration — every byte-
side-effect concern is now covered either by SimCgroupFs (B-class +
the converted manager-level tests) OR by Tier 3 real-kernel scenarios
(Class C).
```

### E2: composition-root probe gate — startup fails on probe failure

```
@tier-3 @walking-skeleton @us-AC3 @us-AC5

GIVEN a `overdrive serve` subprocess invocation in a Lima environment
        where `/sys/fs/cgroup/.overdrive-probe-*` is structurally unwritable
        (test mechanism: invoke `overdrive serve` configured with
         `RealCgroupFs::new().with_probe_root(read_only_tmpdir.path())`
         via a test-only CLI flag OR an env var the binary honors;
         the DELIVER crafter chooses the mechanism, but the scenario
         pins the observable)
WHEN  the binary attempts to start
THEN  the binary exits with non-zero status within ≤ 5 seconds
AND   the stderr (or structured-log capture) contains a
        `health.startup.refused` event whose body carries a
        `ProbeError::Substrate { source }` cause
AND   `source.kind()` is one of `PermissionDenied`, `ReadOnlyFilesystem`,
        or similar (matching the test's chosen unwritable mechanism)
AND   the worker convergence loop never started (no allocations
        are accepted; the gRPC/HTTP port is never bound)

NOTE — this is the Earned Trust composition-root contract per
ADR-0054 § "Composition root wiring". The probe runs BEFORE
`cgroup_preflight` (which itself runs BEFORE the convergence loop),
so a probe failure short-circuits at the earliest possible boot
phase. Lives under
`crates/overdrive-worker/tests/integration/composition_root_probe_gate.rs`
OR `crates/overdrive-cli/tests/integration/serve_probe_refusal.rs`
depending on whether the test exercises the binary subprocess directly
(preferred — matches the driving-adapter coverage requirement) or
in-process via `Worker::start` (acceptable — the AppState already
mediates the same wiring).

Also exercises the structural defense around `ProbeError::Substrate`
serialization into the `health.startup.refused` event body — the
existing event taxonomy must accept the new variant; if it doesn't,
the DELIVER crafter adds a new event field rather than collapsing the
typed `ProbeError` to `Display` (per `.claude/rules/development.md`
§ "Never flatten a typed error to `Internal(String)` at a composition
boundary").
```

---

## Class F — DST K3 determinism guard

### F1: bit-identical trajectory across two SimCgroupFs runs with same op sequence

```
@property @tier-1 @us-AC4 @us-AC7

GIVEN a fixed sequence of operations `ops: Vec<Op>` of length 100
        (generated by proptest at the SUITE level — same seed
         produces same sequence)
AND   two fresh SimCgroupFs instances `fs_a = Arc::new(SimCgroupFs::new())`
        and `fs_b = Arc::new(SimCgroupFs::new())`
WHEN  the same sequence `ops` is applied to both instances
        (each Op is one of `CreateDir`, `Write`, `RemoveDir`, `Probe`
         with deterministic inputs per Op index)
THEN  for each op index i, the `Result` returned by `fs_a` is
        bit-identical to the `Result` from `fs_b`
        (same Ok/Err variant; same ErrorKind on Err)
AND   `fs_a.snapshot()` equals `fs_b.snapshot()` at every index i
AND   the snapshot iteration order is identical (consequence of
        BTreeMap; pinned at B-write-then-snapshot-is-deterministic-
        across-runs but this scenario asserts the FULL trajectory)

NOTE — this is the structural guard for K3 (seed → bit-identical
trajectory) per ADR-0054 § D4 + whitepaper §21. The property is
trivially satisfied today (SimCgroupFs has no nondeterminism source —
no RNG, no clock, no real I/O), but the proptest is the structural
defense against a future change that introduces a HashMap or a real-
clock dependency. A regression on either would fail the equivalence
assertion.

Implementation: proptest at default case count (1024). Lives in
`crates/overdrive-worker/tests/acceptance/sim_cgroup_fs/k3_determinism.rs`.
Tier 1 / default lane — no real I/O, fast.
```

---

## Out of scope (explicitly NOT scenarios)

Per the ADR-0054 commitments, the following are deliberately NOT
covered and the crafter MUST NOT add scenarios for them:

1. **Partial-write resilience.** ADR-0054 § D4: "Partial-write
   modelling is explicitly out of scope: the kernel guarantees
   `write(2)` on cgroup pseudo-files is atomic at the byte-payload
   level." A scenario asserting partial-write behaviour would be
   production code shaped by simulation.
2. **`cgroup.events` notification polling.** ADR-0054 § D3 row 7
   lists this as a kernel-side effect SimCgroupFs cannot model; the
   cgroup manager doesn't read this file today, so no Tier 3 scenario
   exercises notifications either. C-pseudo-file-synthesis is the
   scope-limit: prove the file appears; do NOT exercise its
   notification stream.
3. **Multi-arch (aarch64) probe success.** ADR-0054 § "Compliance"
   notes Lima is x86_64 by convention; aarch64 lives in the per-
   release matrix. Per-PR scenarios assume x86_64 Lima.
4. **`overdrive-host::RealCgroupFs` adapter against macOS / non-Linux.**
   `RealCgroupFs` is Linux-only (matches the worker subsystem's
   `#[cfg(target_os = "linux")]` posture). The trait surface compiles
   on macOS (since it's `overdrive-core`); the host adapter does not
   need to.
5. **`SimCgroupFs` thread-safety stress.** SimCgroupFs uses
   `parking_lot::Mutex`; the safety property is structural. No
   stress-test scenario needed.

---

## DELIVER-step scaffold ordering

Per ADR-0054 § D5, the migration is a single PR with commits sequenced
1-8. The DISTILL scenarios above map to the commits as follows (this
is informational for the crafter; the actual commit boundaries are
the crafter's call):

| ADR-0054 D5 step | Commit name (suggested) | Scenarios scaffolded |
|---|---|---|
| 1 | `feat(core): introduce CgroupFs trait + ProbeError` | A1 (compile-fail trybuild fixture lands here so the crafter can hit RED first), B-kind-returns-stable-string |
| 2 | `feat(host): RealCgroupFs adapter wrapping tokio::fs` | C-probe-success, C-probe-with-custom-root |
| 3 | `feat(sim): SimCgroupFs adapter with injection schedule` | B-create_dir-happy, B-write-happy, B-write-empty-bytes, B-write-missing-parent-returns-notfound, B-remove_dir-happy, B-remove_dir-notfound, B-remove_dir-directory-not-empty, B-probe-happy, all B-injected-* scenarios, B-error-schedule-determinism, B-write-then-snapshot-is-deterministic-across-runs, F1, B-write-respects-cgroup-procs-pid-payload, B-write-cgroup-kill-stores-one-newline |
| 4 | `refactor(worker): CgroupManager struct with Arc<dyn CgroupFs>` | E1 rows 6-16 (the converted/moved tests land in this commit) |
| 5 | `refactor(worker): ExecDriver::new gains fs parameter` | A2 (compile-fail no-Default), C-existing-integration-suite-passes (regression PASS expected after lockstep migration) |
| 6 | `refactor(cli): compose RealCgroupFs at binary boundary; probe-then-use` | E2 |
| 7 | `test(worker): retire tempfile cgroup_manager tests (except rows 8 + 10); introduce SimCgroupFs unit tests; relocate rows 8 + 10 to tests/integration/cgroup_manager/ behind --features integration-tests` | (no new scenarios — E1 already accounted for; this is the cleanup commit. Rows 8 + 10 stay tempfile-backed against `RealCgroupFs`; they are MOVED out of the inline `mod tests` into the integration test tree, not retired) |
| 8 | `test(integration): kernel-semantics Tier 3 coverage for new CgroupFs port` | C-cgroup-kill, C-subtree-control-ebusy, C-controller-validation, C-pseudo-file-synthesis, C-rmdir-auto-reap, C-procs-pid-movement, C-write-to-readonly-cgroup-file, D1 (equivalence proptest) |

The crafter's per-step mutation gate
(`cargo xtask mutants --diff origin/main --package overdrive-worker --file <step's-touched-files>`)
should produce kill rate ≥ 80% at every step.
