# Bugfix RCA: cgroup `subtree_control` is never delegated, so child cgroups have no resource files

**Status**: User-approved 2026-05-09. **Approved fix shape: BROAD scope.**

- Production fix #1 — `crates/overdrive-control-plane/src/cgroup_manager.rs::create_and_enrol_control_plane_slice_at` writes `+cpu +memory +io +pids` to `overdrive.slice/cgroup.subtree_control` AFTER the `mkdir overdrive.slice` and BEFORE creating `control-plane.slice` underneath OR enrolling the server PID. Failures surface as discrete typed-error variants (per `.claude/rules/development.md` § "Distinct failure modes get distinct error variants").
- Production fix #2 — `crates/overdrive-worker/src/cgroup_manager.rs` gains a `create_workloads_slice_with_controllers` init the worker calls once at startup. Mirrors the control-plane init; ensures `+cpu +memory` are enabled in `overdrive.slice/workloads.slice/cgroup.subtree_control` before any `alloc-*.scope` is created.
- Regression test (real cgroupfs, gated `integration-tests`, runs through `cargo xtask lima run --`) — boots both inits + drives `ExecDriver::start` for an alloc with `cpu_milli=2000, memory_bytes=128MiB` and asserts `cpu.weight=200`, `memory.max=134217728`, AND that the WARN log line "cgroup resource-limit write failed" did NOT fire.
- Broad-scope cleanup — migrate the 10 `crates/overdrive-worker/tests/integration/exec_driver/*.rs` files off `tempfile::TempDir` to real `/sys/fs/cgroup`. Delete the `tempdir-as-cgroupfs` ENOTEMPTY fallback branch in `remove_workload_scope` (`crates/overdrive-worker/src/cgroup_manager.rs:268-...`) + `is_dir_not_empty` helper if no other callers remain. Update docstrings on `create_workload_scope`, `remove_workload_scope`, `ExecDriver::build_command` etc. that still reference the TempDir scaffolding.

---

## Bug summary

The control-plane log emits a recurring WARN per reconcile tick:

```
WARN cgroup resource-limit write failed; continuing per ADR-0026 D9
  alloc=alloc-doomed-0
  scope=overdrive.slice/workloads.slice/alloc-doomed-0.scope
  error=Permission denied (os error 13)
```

The warning fires from `crates/overdrive-worker/src/driver.rs:299-305` after `write_resource_limits` (`crates/overdrive-worker/src/cgroup_manager.rs:195`) returns `Err(EACCES)` on writes to `cpu.weight` and `memory.max` under the alloc's scope directory. ADR-0026 D9 designates limit-write failure as warn-and-continue — the workload still spawns, but **resource limits are silently absent**.

## Root cause chain

### Live evidence (Lima VM, `overdrive serve` running)

| Probe | Result |
|---|---|
| `ps -ef \| grep overdrive serve` | `root        2888    2887  0 17:21 pts/1    /home/marcus.guest/.cargo-target-lima/debug/overdrive serve` — server runs as **uid 0**. The wrapper (`cargo xtask lima run --`) defaults to `sudo`. Not a permissions-on-process problem. |
| `cat /sys/fs/cgroup/overdrive.slice/cgroup.controllers` | `cpuset cpu io memory pids` — controllers are inherited from the parent. |
| `cat /sys/fs/cgroup/overdrive.slice/cgroup.subtree_control` | **empty** — no controllers ever delegated to children. |
| `cat /sys/fs/cgroup/overdrive.slice/workloads.slice/cgroup.controllers` | **empty** — child inherits nothing because parent's `subtree_control` is empty. |
| `cat /sys/fs/cgroup/overdrive.slice/workloads.slice/cgroup.subtree_control` | **empty** — never touched. |

### A. Production code never delegates `subtree_control`

`crates/overdrive-control-plane/src/cgroup_manager.rs:32-42` (`create_and_enrol_control_plane_slice_at`) does exactly two things: `std::fs::create_dir_all(overdrive.slice/control-plane.slice)` then writes the server PID into `cgroup.procs`. Nothing writes `+cpu +memory +io +pids` to `overdrive.slice/cgroup.subtree_control`.

`crates/overdrive-worker/src/cgroup_manager.rs:158-161` (`create_workload_scope`) does exactly one thing: `tokio::fs::create_dir_all(scope)`. Nothing creates `workloads.slice` with controllers enabled either.

### B. cgroup v2 contract — child cgroups only have a controller's resource files when the *parent* enabled the controller in its `subtree_control`

Per `Documentation/admin-guide/cgroup-v2.rst`: a cgroup directory exposes a controller's interface files (e.g. `cpu.weight`, `memory.max`) only when the controller is listed in the parent's `cgroup.subtree_control`. With `overdrive.slice/cgroup.subtree_control` empty, `workloads.slice` has no `cpu.*` or `memory.*` files, and neither do the alloc scope directories underneath it. The directories exist; the resource files don't.

### C. `tokio::fs::write` opens with `O_WRONLY | O_CREAT | O_TRUNC` → cgroupfs returns EACCES on `O_CREAT` of a non-existent kernel-managed file

`tokio::fs::write` is documented as equivalent to `File::create(path).write_all(data)`. `File::create` opens with `O_WRONLY | O_CREAT | O_TRUNC | mode 0o644`. cgroupfs is a kernel-managed virtual filesystem; it does not allow userspace to create new files via `O_CREAT`. The kernel's response is **EACCES (Permission denied)** — *not* ENOENT — because the kernel's permission check fires before the inode-existence check, and userspace lacks the privilege to populate a new inode in cgroupfs regardless of UID. This is what the warning's "Permission denied (os error 13)" actually means at the syscall level — and it is independent of whether the calling process is root.

### D. Tests use `tempfile::TempDir` as a fake cgroupfs root, masking the failure

All 10 integration tests under `crates/overdrive-worker/tests/integration/exec_driver/*.rs` create a `TempDir`, `mkdir -p tempdir/overdrive.slice/workloads.slice`, then point `ExecDriver::new(cgroup_root, ...)` at the tempdir. On tmpfs, `tokio::fs::write(scope/cpu.weight, "200\n")` *succeeds* — `O_CREAT` is honored on a regular filesystem — and the test reads back the value it just wrote. The cgroupfs invariant that the resource files only exist when the parent delegated the controller is structurally absent from the test fixture.

The Tier-3-equivalent path (`cargo xtask lima run -- cargo nextest run … --features integration-tests`) hits real cgroupfs but the production response to the EACCES is the warn-and-continue per ADR-0026 D9 — workload still spawns, no test bar fires. The bug surfaces only as a silent resource-limit absence at runtime; no test asserts on the absence of the warning.

### E. ADR-0026 D9 warn-and-continue masks the production symptom

The warn-and-continue disposition (intended for *recoverable* limit-write failures) converts a hard misconfiguration into a per-tick log line. Workloads run unisolated; the operator sees a warning at WARN level (often filtered out) and the system continues. The bug only surfaced because the user investigated a separate issue (a `doomed` workload spec referencing a non-existent binary) and noticed the recurring warning in the server log.

### F. Why `O_CREAT` and not `O_WRONLY` alone

Even if production used `OpenOptions::new().write(true).truncate(true).open(path)` (no `O_CREAT`), the failure shape on a missing resource file would be ENOENT, not EACCES — but ENOENT is just as fatal for the limit write. The structural fix is to delegate `subtree_control` so the file exists; bypassing `O_CREAT` only changes the error code, not the bug.

## Multi-causal factors

1. **No real-cgroupfs integration test for `ExecDriver`** — the entire test surface for exec-driver behaviour runs against TempDir fixtures, so the cgroup-v2 semantics that production depends on are never exercised pre-merge. This is the structural amplifier that let the bug ship.
2. **ADR-0026 D9 disposition** — the per-tick WARN with continuation is correct for transient I/O failures (e.g. EROFS during a rare cgroupfs remount) but converts a hard-misconfiguration EACCES into noise. Out of scope for this bugfix; if the crafter encounters an obvious place where EACCES specifically should escalate, surface as a follow-up issue (do NOT change ADR-0026 D9 disposition without explicit user sign-off).
3. **macOS support has been removed in policy** — there is no longer a structural reason for the `TempDir`-as-fake-cgroupfs scaffolding (the cross-platform argument is gone). Tests that exercise cgroupfs semantics should hit real `/sys/fs/cgroup` via the Lima VM.

## Approved fix shape

### Production fix #1 — `crates/overdrive-control-plane/src/cgroup_manager.rs`

Extend `create_and_enrol_control_plane_slice_at`:

```text
1. mkdir -p overdrive.slice                      (idempotent)
2. write "+cpu +memory +io +pids\n"
   to overdrive.slice/cgroup.subtree_control     (NEW — order is load-bearing)
3. mkdir -p overdrive.slice/control-plane.slice  (idempotent)
4. write pid to overdrive.slice/control-plane.slice/cgroup.procs
```

**Order is load-bearing.** A cgroup with a process in `cgroup.procs` cannot have controllers added to its parent's `subtree_control` — the kernel returns EBUSY. Step 2 must complete before step 4 enrolls a process anywhere under `overdrive.slice`.

**Idempotent.** Writing `+cpu +memory +io +pids` on an already-enabled subtree_control is a no-op (the kernel accepts the write and re-confirms enablement). A second call to `create_and_enrol_control_plane_slice_at` is safe.

**Typed errors.** Per `.claude/rules/development.md` § "Distinct failure modes get distinct error variants", the `subtree_control` write surfaces as a discrete error variant. Distinguish:

- `Ok` — write succeeded (controllers enabled, idempotent re-enable).
- `SubtreeControlBusy { source: io::Error }` — kernel returned EBUSY (a process is already in this cgroup; the slice was previously initialised in the wrong order). Operator hint: restart the server cleanly.
- `SubtreeControlWriteFailed { source: io::Error }` — any other I/O error. Operator hint: inspect cgroupfs delegation for the enclosing slice.

The control-plane crate will need a `CgroupBootstrapError` enum (or extend an existing one). Not absorbed into a generic `io::Error`.

### Production fix #2 — `crates/overdrive-worker/src/cgroup_manager.rs`

New entry point `create_workloads_slice_with_controllers(cgroup_root)`:

```text
1. mkdir -p overdrive.slice/workloads.slice      (idempotent)
2. write "+cpu +memory\n"
   to overdrive.slice/workloads.slice/cgroup.subtree_control
```

`+pids` and `+io` are not strictly required at this level for the present resource-limit surface (`cpu.weight`, `memory.max`); enable them too if the architect's roadmap deems them defensible (forward compatibility), otherwise keep the surface minimal.

The worker crate's startup wiring (in `overdrive-control-plane/src/lib.rs:436` where `ExecDriver` is constructed for the production `cgroup_root`) calls this new init alongside `create_and_enrol_control_plane_slice` at boot, before the convergence loop accepts any allocations.

Same typed-error treatment as fix #1.

### Regression test — real cgroupfs (RED then GREEN)

New test under `crates/overdrive-control-plane/tests/integration/cgroup_isolation/` (or `crates/overdrive-worker/tests/integration/exec_driver/`, whichever fits the existing structure better — architect to decide).

Shape:

```rust
#![cfg(target_os = "linux")]
// Per crates/overdrive-worker/tests/integration.rs gating + workspace
// integration-tests feature.

#[tokio::test]
#[serial(cgroup)]  // mutates real /sys/fs/cgroup; serialise within binary
async fn alloc_scope_has_writable_cpu_weight_and_memory_max() {
    let cgroup_root = Path::new("/sys/fs/cgroup");

    create_and_enrol_control_plane_slice_at(cgroup_root, std::process::id())
        .expect("control-plane bootstrap succeeds");
    create_workloads_slice_with_controllers(cgroup_root)
        .expect("workloads.slice bootstrap succeeds");

    let driver = Arc::new(ExecDriver::new(
        cgroup_root.to_path_buf(),
        Arc::new(SystemClock),
    ));
    let alloc = AllocationId::new("alloc-subtree-control-regression").expect("valid");
    let spec = AllocationSpec {
        alloc: alloc.clone(),
        identity: SpiffeId::new("spiffe://overdrive.local/job/regression/alloc/0").unwrap(),
        command: "/bin/sleep".to_owned(),
        args: vec!["60".to_owned()],
        resources: Resources { cpu_milli: 2_000, memory_bytes: 128 * 1024 * 1024 },
    };
    let _cleanup = AllocCleanup::register(cgroup_root.to_path_buf(), alloc.clone());

    let handle = driver.start(&spec).await.expect("start succeeds against real cgroupfs");

    let scope_dir = cgroup_root.join(format!(
        "overdrive.slice/workloads.slice/{alloc}.scope"
    ));
    let cpu_weight = std::fs::read_to_string(scope_dir.join("cpu.weight"))
        .expect("cpu.weight readable")
        .trim()
        .to_owned();
    assert_eq!(cpu_weight, "200", "cpu_milli=2000 must produce cpu.weight=200");
    let memory_max = std::fs::read_to_string(scope_dir.join("memory.max"))
        .expect("memory.max readable")
        .trim()
        .to_owned();
    assert_eq!(memory_max, format!("{}", 128 * 1024 * 1024));

    driver.stop(&handle).await.expect("stop succeeds");
    // _cleanup drops -> cgroup.kill + rmdir
}
```

PLUS a companion test that asserts the WARN log line "cgroup resource-limit write failed" did NOT fire during the success path (use `tracing-subscriber`'s test capture or the existing `tracing_test` integration if present).

Gated `#[cfg(feature = "integration-tests")]` via the crate's existing entrypoint. Runs through `cargo xtask lima run --` per `.claude/rules/testing.md` § "Cgroup writes need root or delegation".

### Broad-scope cleanup — migrate `exec_driver/*` tests off `TempDir`

Every test under `crates/overdrive-worker/tests/integration/exec_driver/*.rs` (10 files) currently mounts `tempfile::TempDir` as a fake cgroupfs root and `mkdir -p tempdir/overdrive.slice/workloads.slice`. Migrate them to use real `/sys/fs/cgroup` via the new `create_workloads_slice_with_controllers` helper.

Per-test isolation:
- Each test already uses a unique `AllocationId` (`alloc-resource-enforcement`, `alloc-warn-ok-0`, `alloc-create-0`, etc.); preserve that.
- Each test wraps its cleanup in an `AllocCleanup`-style guard (analogous to the existing `crates/overdrive-control-plane/tests/integration/job_lifecycle/cleanup.rs::AllocCleanup`) so the alloc scope is removed via `cgroup.kill` + `rmdir` even if the test panics.
- `overdrive.slice` and `workloads.slice` themselves stay across tests (idempotent across runs). The Lima leftover-cgroup cleanup discipline (`.claude/rules/testing.md` § "Leaked workload cgroups across runs") still applies.

Dead-code removal (per `.claude/rules/development.md` § "Deletion discipline"):
- `remove_workload_scope` (`crates/overdrive-worker/src/cgroup_manager.rs:268-...`) carries an ENOTEMPTY fallback (`is_dir_not_empty` → `remove_dir_all`) that exists *only* for the tempdir-as-cgroupfs path. With every test on real cgroupfs, the fallback is unreachable. Delete the branch AND `is_dir_not_empty` if no other callers remain.
- Any test that exists *only* to cover that fallback branch goes too.
- Update docstrings on `create_workload_scope`, `remove_workload_scope`, `ExecDriver::build_command` etc. that reference the TempDir rationale.

Single-cut migration per `.claude/rules/development.md` § "Single-cut migrations in greenfield" — delete the TempDir scaffolding and land the real-cgroupfs scaffolding in the same PR. No deprecation comments, no feature-flagged old paths.

### `force_limit_write_failure` injection seam

The seam at `crates/overdrive-worker/src/driver.rs:291-296` simulates EACCES from the limit write to test the warn-and-continue ADR-0026 D9 path. Keep it; verify its associated test (`limit_write_failure_warns.rs`) still works after the migration off TempDir (the seam fires synthetic EACCES regardless of the underlying filesystem, so it should be insensitive to the migration).

## What this RCA does NOT cover

- **ADR-0026 D9 disposition.** The warn-and-continue policy is the structural amplifier that let this bug ship silently, but reconsidering it (e.g. fail-fast on EACCES at first alloc) is OUT OF SCOPE for this bugfix. If the crafter encounters an obvious place where EACCES specifically should escalate (vs the more recoverable I/O errors D9 was really written for), surface as a follow-up issue for user approval. Do NOT change ADR-0026 D9 disposition without explicit user sign-off.
- **systemd-managed slices.** Some production deployments may use `systemctl --user start overdrive` or a system-level systemd unit, in which case systemd creates `overdrive.slice` with controllers already delegated. The fix here is idempotent against that case (re-writing `+cpu +memory +io +pids` to an already-enabled subtree_control is a no-op). No change to the systemd unit files is required.
- **Other slices.** `overdrive.slice/control-plane.slice` does not need its own `subtree_control` write — the control-plane process does not create child cgroups under itself. Only the workload-bearing slice (`workloads.slice`) needs its own delegation.

## Cross-references

- ADR-0026 — cgroup v2 direct writes
- ADR-0028 — cgroup preflight refusal
- `.claude/rules/development.md` § "Distinct failure modes get distinct error variants"
- `.claude/rules/development.md` § "Deletion discipline"
- `.claude/rules/development.md` § "Single-cut migrations in greenfield"
- `.claude/rules/testing.md` § "Cgroup writes need root or delegation"
- `.claude/rules/testing.md` § "Leaked workload cgroups across runs"
- Prior cgroup-related bugfix RCAs:
  - `docs/feature/fix-cgroup-preflight-procfs-unreadable/bugfix-rca.md`
  - `docs/feature/fix-cgroup-preflight-wrong-slice/bugfix-rca.md`
  - `docs/feature/fix-cgroup-preflight-scope-vs-slice/bugfix-rca.md`
  - `docs/feature/fix-cgroup-preflight-subtree-unreadable/bugfix-rca.md`
