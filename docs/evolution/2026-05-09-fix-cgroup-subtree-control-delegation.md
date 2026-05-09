# fix-cgroup-subtree-control-delegation — Feature Evolution

**Feature ID**: fix-cgroup-subtree-control-delegation
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`, BROAD scope)
**Branch**: `marcus-sa/cgroup-eacces-debug`
**Date**: 2026-05-09
**Commits**:
- `5296c06` — `test(control-plane): RED — regression for subtree_control delegation`
- `1dd780e` — `fix(control-plane,worker): delegate cgroup subtree_control to overdrive.slice and workloads.slice`
- `5c7e0e7` — `test(worker): migrate exec_driver tests off TempDir to real /sys/fs/cgroup`
- `e5fc2ed` — `chore(worker): remove dead tempdir-as-cgroupfs fallback now that exec_driver tests use real cgroupfs`

**Status**: Delivered.

---

## Symptom

Operators running `overdrive serve` (under sudo, in Lima or production) saw a recurring per-reconcile-tick warning in the control-plane log:

```
WARN cgroup resource-limit write failed; continuing per ADR-0026 D9
  alloc=alloc-doomed-0
  scope=overdrive.slice/workloads.slice/alloc-doomed-0.scope
  error=Permission denied (os error 13)
```

The warning fired from `crates/overdrive-worker/src/driver.rs`'s `write_resource_limits` after `tokio::fs::write(scope/cpu.weight, ...)` returned `Err(EACCES)`. Per ADR-0026 D9 (warn-and-continue disposition), `ExecDriver::start` proceeded to spawn the workload anyway — but **without the resource limits the operator's spec specified**. CPU weight and memory ceilings were silently absent on every workload from boot.

The bug only surfaced because an operator was investigating an unrelated job-spec failure (`examples/doomed.toml` referencing a non-existent binary) and noticed the recurring warning in the server log.

## Root cause

**The bootstrap that creates `overdrive.slice` (and the `workloads.slice` underneath it) never wrote `+cpu +memory +io +pids` to the parent's `cgroup.subtree_control`.** Per the cgroup v2 contract (`Documentation/admin-guide/cgroup-v2.rst`), child cgroups only expose a controller's interface files (`cpu.weight`, `memory.max`) when the parent has enabled the controller in its `subtree_control`. With both `overdrive.slice/cgroup.subtree_control` and `overdrive.slice/workloads.slice/cgroup.subtree_control` empty, the alloc scope directories existed but the resource files didn't.

`tokio::fs::write` opens with `O_WRONLY | O_CREAT | O_TRUNC`. cgroupfs does not allow userspace to create new files via `O_CREAT`; the kernel's response is **EACCES (Permission denied)** — *not* ENOENT — because the kernel's permission check fires before the inode-existence check, and userspace cannot populate a new inode in cgroupfs regardless of UID. That EACCES is what the operator saw, and it is independent of whether the calling process is root.

### Multi-causal contributing factors

1. **No real-cgroupfs integration test for `ExecDriver`.** All 10 integration tests under `crates/overdrive-worker/tests/integration/exec_driver/*.rs` mounted `tempfile::TempDir` as a fake cgroupfs root. On tmpfs, `tokio::fs::write` with `O_CREAT` *succeeds* on a non-existent path, so the test reads back the value it just wrote and passes. Real cgroupfs rejects the same write. The cgroup-v2 invariant that resource files only exist when the parent delegates the controller was structurally absent from every test fixture.
2. **ADR-0026 D9 warn-and-continue.** The disposition was correct for transient I/O failures (e.g. EROFS during a rare cgroupfs remount) but converted a hard misconfiguration into a per-tick log line. Workloads ran unisolated; the operator saw a warning at WARN level (often filtered) and the system continued. Out of scope for this fix; ADR-0026 D9 disposition unchanged. If a future revisit decides EACCES specifically should escalate vs the recoverable I/O errors D9 was really written for, raise it as a separate ADR amendment.
3. **macOS support recently removed in policy.** The `TempDir`-as-fake-cgroupfs scaffolding had originally been justified as cross-platform-friendly. With macOS gone, that rationale lapsed but the scaffolding hadn't been migrated.

Live evidence at investigation time (Lima VM, `overdrive serve` running):

| Probe | Result |
|---|---|
| `ps -ef \| grep overdrive serve` | uid 0 (root via sudo) — not a permissions-on-process problem |
| `cat /sys/fs/cgroup/overdrive.slice/cgroup.controllers` | `cpuset cpu io memory pids` (controllers inherited from parent) |
| `cat /sys/fs/cgroup/overdrive.slice/cgroup.subtree_control` | empty — never delegated downward |
| `cat /sys/fs/cgroup/overdrive.slice/workloads.slice/cgroup.controllers` | empty (inherits nothing because parent's `subtree_control` is empty) |

## Fix

**Approved fix shape**: BROAD scope (user-approved 2026-05-09 in `/nw-bugfix` Phase 2). Three concerns landed in 4 steps across 2 phases:

### Phase 01 — Production fix (RED → GREEN)

**Step 01-01 (`5296c06`)** — RED scaffold. Added two new typed-error variants to a new `CgroupBootstrapError` enum in `crates/overdrive-control-plane/src/error.rs`:

- `SubtreeControlBusy { source: io::Error }` — kernel returned EBUSY (a process is already in this cgroup; the slice was previously initialised in the wrong order). Operator hint: restart the server cleanly.
- `SubtreeControlWriteFailed { source: io::Error }` — any other I/O error. Operator hint: inspect cgroupfs delegation for the enclosing slice.

Plus a `from_subtree_control_io` constructor that does the EBUSY discrimination via `io::Error::raw_os_error() == Some(libc::EBUSY)`. Added two RED-scaffold integration tests under `crates/overdrive-control-plane/tests/integration/cgroup_isolation/`:

- `alloc_scope_has_writable_cpu_weight_and_memory_max.rs` — boots both inits against real `/sys/fs/cgroup`, drives `ExecDriver::start` for an alloc with `cpu_milli=2000, memory_bytes=128MiB`, asserts `cpu.weight=200` and `memory.max=134217728`.
- Companion test in the same file — uses a custom `tracing::Layer` (adapted from `crates/overdrive-dataplane/tests/integration/veth_attach.rs:307-381`) to assert the WARN line `cgroup resource-limit write failed` did NOT fire on the success path. No new dev-dep introduced (deliberate workspace choice over `tracing-test`, per the `veth_attach.rs` precedent).

Both tests landed `#[serial(cgroup)]` + `#[should_panic(expected = "RED scaffold")]` + leading `panic!("Not yet implemented -- RED scaffold ...")`. Lefthook clean — the `#[should_panic]` shape is hook-compatible per `.claude/rules/testing.md` § "RED scaffolds".

**Step 01-02 (`1dd780e`)** — GREEN production fix:

- `crates/overdrive-control-plane/src/cgroup_manager.rs::create_and_enrol_control_plane_slice_at` extended to a load-bearing 4-step order: (1) `mkdir -p overdrive.slice`, (2) write `+cpu +memory +io +pids\n` to `overdrive.slice/cgroup.subtree_control`, (3) `mkdir -p overdrive.slice/control-plane.slice`, (4) enrol PID into `cgroup.procs`. **Step 2 must complete before any process is enrolled anywhere under `overdrive.slice/`** — the kernel forbids modifying a parent's `subtree_control` while any child cgroup contains a live process (returns EBUSY).
- `crates/overdrive-worker/src/cgroup_manager.rs` gained new public `create_workloads_slice_with_controllers(cgroup_root)`: `mkdir -p overdrive.slice/workloads.slice` then write `+cpu +memory +io +pids\n` to its `subtree_control`. Same order invariant. New typed-error enum `WorkloadsBootstrapError` mirroring the control-plane shape (cross-crate duplication intentional per ADR-0029; the two crates do not share a typed-error surface).
- `crates/overdrive-control-plane/src/lib.rs::run_server_with_obs_and_driver` calls both inits before the convergence loop spawns. Errors propagate to the boot caller via `.map_err(|e| ControlPlaneError::internal(...))` — not swallowed.
- The Phase 01 RED-scaffold transition: dropped `#[should_panic(expected = "RED scaffold")]` + the leading `panic!` line on both tests, dropped the file-top `#![allow(dead_code, unused_imports, unreachable_code, clippy::diverging_sub_expression)]`. Tests transitioned RED → GREEN.
- New integration test `subtree_control_delegation_is_idempotent.rs` verifies both inits are idempotent under repeated boot (the kernel accepts a second `+cpu +memory +io +pids` write on already-enabled subtree_control as a no-op).

User-confirmed during `/nw-deliver` Phase 2 follow-up (Q3): both slices delegate the full four-controller set (`+cpu +memory +io +pids`), not the minimum two (`+cpu +memory`). The `+pids` and `+io` controllers cost nothing to enable now and avoid a "why is `workloads.slice` narrower than `overdrive.slice`" reviewer comment later.

### Phase 02 — Broad-scope cleanup (single-cut migration)

**Step 02-01 (`5c7e0e7`)** — migrated all 10 files under `crates/overdrive-worker/tests/integration/exec_driver/` off `tempfile::TempDir` to real `/sys/fs/cgroup`. Each test:

1. Calls `create_workloads_slice_with_controllers(Path::new("/sys/fs/cgroup"))` at the top of its body (idempotent across tests).
2. Preserves its existing unique `AllocationId` (`alloc-resource-enforcement`, `alloc-warn-ok-0`, `alloc-create-0`, etc.) — no renumbering.
3. Registers an `AllocCleanup` RAII guard for that alloc scope. New helper at `crates/overdrive-worker/tests/integration/exec_driver/cleanup.rs` (cross-crate reuse from `overdrive-control-plane`'s `job_lifecycle/cleanup.rs::AllocCleanup` is not possible — separate compilation units). The guard fires `cgroup.kill` + `waitpid(WNOHANG)` + `rmdir` on Drop, so leaked workloads do not survive test process exit (panic OR clean return).
4. Carries `#[serial(cgroup)]` to serialise within the binary — every test now mutates the same `/sys/fs/cgroup/overdrive.slice/workloads.slice/` namespace.
5. Constructs `ExecDriver::new(Path::new("/sys/fs/cgroup").to_path_buf(), ...)` instead of a tempdir-based root.

`limit_write_failure_warns.rs` continues to assert the warn-and-continue path correctly. The `force_limit_write_failure` injection seam at `crates/overdrive-worker/src/driver.rs` is filesystem-agnostic — it short-circuits the limit-write call with a synthetic EACCES regardless of whether the underlying path is on tmpfs or cgroupfs. Untouched in this commit.

`tempfile` dev-dep retained — `crates/overdrive-worker/src/cgroup_manager.rs::mod tests` (unit tests) still consume it for in-process scope-creation testing where real cgroupfs is not the SUT.

**Step 02-02 (`e5fc2ed`)** — deletion-discipline closing step. Verified the `tempdir-as-cgroupfs` ENOTEMPTY fallback branch in `remove_workload_scope` (`crates/overdrive-worker/src/cgroup_manager.rs`) was reachable ONLY from 5 unit tests that existed solely to defend it (`is_dir_not_empty_*` × 3, `remove_workload_scope_falls_back_to_remove_dir_all_on_enotempty`, `remove_workload_scope_propagates_non_enotempty_non_notfound_errors`). No surviving production callers. Per `.claude/rules/development.md` § "Deletion discipline" — production code AND its defending tests deleted in the same commit. Deletions:

- The `Err(err) if is_dir_not_empty(&err) => remove_dir_all(...)` match arm in `remove_workload_scope`.
- The `is_dir_not_empty(&io::Error) -> bool` helper.
- 5 dead-only unit tests in the same file.
- TempDir-rationale paragraphs from rustdoc on `CgroupPath::resolve`, `create_workloads_slice_with_controllers`, `remove_workload_scope`, `ExecDriver::new`, `ExecDriver::build_command`, and `ExecDriver::stop` step-4 inline comment — replaced with the real-cgroupfs contract description.

Mutation gate first run produced 6/7 caught (85.7%, gate passed). Crafter added `remove_workload_scope_propagates_non_notfound_errors` (regular file at scope path → `NotADirectory`) to kill the surviving `err.kind() == NotFound -> true` mutant. Final: 7/7 caught, 100%.

## Tests added / updated

**New integration tests** (Phase 01):
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/alloc_scope_has_writable_cpu_weight_and_memory_max.rs` — two tests in one file (resource-file readability + WARN-line absence). Real `/sys/fs/cgroup`, `#[serial(cgroup)]`, `AllocCleanup` guard, custom `tracing::Layer` capture.
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/subtree_control_delegation_is_idempotent.rs` — calls each init twice on the same `cgroup_root`, asserts both Ok and verifies post-state delegation explicitly (reads `cgroup.subtree_control` and confirms `cpu`/`memory` are present).

**New test helper** (Phase 02):
- `crates/overdrive-worker/tests/integration/exec_driver/cleanup.rs` — `AllocCleanup` RAII guard. Mirrors the shape of the existing `overdrive-control-plane/tests/integration/job_lifecycle/cleanup.rs::AllocCleanup`; cross-crate duplication required by Rust's compilation-unit boundaries.

**Migrated tests** (Phase 02): all 10 files under `crates/overdrive-worker/tests/integration/exec_driver/` (`cgroup_procs`, `limit_write_failure_warns`, `live_map_bounded`, `missing_binary`, `resize_updates_limits`, `resource_enforcement`, `start_and_running`, `stop_escalates_to_sigkill`, `stop_pid_none_handle_delivers_sigterm`, `stop_with_grace`).

**Deleted tests** (Phase 02): 5 dead-only unit tests in `crates/overdrive-worker/src/cgroup_manager.rs::mod tests` that defended the deleted ENOTEMPTY fallback. Plus 1 new unit test added in 02-02 to close a surviving mutation: `remove_workload_scope_propagates_non_notfound_errors`.

**New unit tests** (Phase 01-02): 16 unit tests in `crates/overdrive-worker/src/cgroup_manager.rs::mod tests` defending the new write paths, EBUSY discrimination, idempotency, and the simplified `remove_workload_scope` happy path. Per `.claude/rules/testing.md` § "Adding a new test — which tier?" these are pure in-process logic and stay in the default lane.

## Quality gates

- **DES integrity** — `verify_deliver_integrity` exit 0; all 4 steps have complete DES traces (PREPARE, RED_ACCEPTANCE, RED_UNIT, GREEN, COMMIT — RED_ACCEPTANCE/RED_UNIT SKIPPED on 02-01 / 02-02 with `NOT_APPLICABLE` rationale per execution-log).
- **Workspace nextest** on Linux via Lima (`cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`): 1126/1126 passed (3 leaky pre-existing tests in `overdrive-cli` and `overdrive-control-plane` unrelated to this change). Pre-cleanup the count was 1131; 5 dead-only tests removed in 02-02.
- **Per-step mutation gate** — 01-02 caught 4/4 (100%) on the new write paths; 02-02 caught 7/7 (100%) on the simplified `remove_workload_scope` Ok-path. Both well over the ≥80% per-feature threshold (CLAUDE.md mutation strategy = per-feature). Per-PR rerun would re-cover the same lines and is structurally redundant — skipped.
- **Clippy** — `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings` clean. Two in-scope clippy fixes during 02-01 (`AllocCleanup::register` made `const fn`; doc comment backticking).
- **Lefthook** — every commit landed cleanly with the full pre-commit gate (fmt, clippy `-D warnings`, doctest, nextest-affected). NO `--no-verify` used anywhere.
- **Adversarial review** (Phase 4 of `/nw-deliver`) — APPROVED. No critical, no significant findings. Reviewer's one minor finding (unused `tempfile` dev-dep) was self-corrected on inspection — `tempfile` is still consumed by `cgroup_manager.rs::mod tests` unit tests. All 7 testing-theater anti-patterns clean. All 10 review focus areas PASS.
- **Leftover-cgroup detection** after the test run (`.claude/rules/testing.md` § "Leaked workload cgroups across runs"): clean for the worker test surface. Pre-cleanup ran in Lima during 02-01 PREPARE to clear 4 stale alloc scopes from prior runs.

## Out of scope (flagged for follow-up)

- **ADR-0026 D9 disposition.** The warn-and-continue policy is the structural amplifier that let this bug ship silently. Out of scope for this delivery per the RCA's "Open question for the crafter to surface, NOT decide" section. The crafter spotted no obvious place to escalate during implementation. If a future revisit decides EACCES specifically should escalate (vs the recoverable I/O errors D9 was really written for), raise as a separate ADR amendment.
- **Cgroup preflight (`crates/overdrive-control-plane/src/cgroup_preflight.rs`).** Already correct after the four prior `fix-cgroup-preflight-*` features (procfs-unreadable, wrong-slice, scope-vs-slice, subtree-unreadable). The preflight verifies the *enclosing* slice has controllers in its `subtree_control` so `overdrive.slice` can inherit them in `cgroup.controllers`; this fix delegates DOWN through the slices Overdrive owns. The two layers are complementary.
- **systemd-managed slice production deployments.** The fix is idempotent against `systemctl --user start overdrive` (or system-level systemd unit) where systemd has already created `overdrive.slice` with controllers delegated. Re-writing `+cpu +memory +io +pids` to an already-enabled `subtree_control` is a no-op accepted by the kernel. No change to systemd unit files required.

## References

- RCA: `docs/feature/fix-cgroup-subtree-control-delegation/bugfix-rca.md`
- Roadmap: `docs/feature/fix-cgroup-subtree-control-delegation/deliver/roadmap.json` (4 steps, validation `approved`)
- Execution log: `docs/feature/fix-cgroup-subtree-control-delegation/deliver/execution-log.json` (20 phase events, all EXECUTED or SKIPPED with valid `NOT_APPLICABLE` reasons)
- ADR: `docs/product/architecture/adr-0026-cgroup-v2-direct-writes.md` (D9 warn-and-continue disposition — unchanged), `docs/product/architecture/adr-0028-cgroup-preflight-refusal.md` (preflight, unchanged), `docs/product/architecture/adr-0029-overdrive-worker-crate-extraction.md` (cross-crate boundaries — explains why `WorkloadsBootstrapError` and `CgroupBootstrapError` are separate enums)
- Prior cgroup-related fixes: `docs/evolution/2026-04-29-fix-cgroup-preflight-wrong-slice.md`, `docs/evolution/2026-05-02-fix-cgroup-preflight-scope-vs-slice.md`
- Commits: `5296c06` (RED), `1dd780e` (GREEN production fix), `5c7e0e7` (test migration), `e5fc2ed` (deletion)
- Test discipline: `.claude/rules/testing.md` § "Cgroup writes need root or delegation", § "Tests that mutate process-global state", § "Leaked workload cgroups across runs", § "RED scaffolds and intentionally-failing commits"
- Development discipline: `.claude/rules/development.md` § "Distinct failure modes get distinct error variants", § "Deletion discipline", § "Single-cut migrations in greenfield", § "No blocking std::fs::* inside async fn"
