# Bugfix RCA: cgroup preflight delegation check reads root cgroup, not the user's slice

**Status**: User-approved 2026-04-29. Approved fix shape is **Option B + injected `proc_self_cgroup` parameter** (read `/proc/self/cgroup` to discover the enclosing slice; inject the path for testability). NOT (a) hardcoded `user.slice/user-{uid}.slice/` derivation; NOT (c) injecting a slice path computed by the caller.

---

## Bug summary

`cgroup_preflight::run_preflight_at` (`crates/overdrive-control-plane/src/cgroup_preflight.rs:168`) computes `subtree_control = cgroup_root.join("cgroup.subtree_control")`. With production `cgroup_root = /sys/fs/cgroup` (line 26 + line 212), this resolves to `/sys/fs/cgroup/cgroup.subtree_control` — the **kernel-root** cgroup. On every modern Linux kernel the root cgroup's `subtree_control` lists every controller (`cpu cpuset hugetlb io memory misc pids rdma`), so the check passes unconditionally for every non-root user.

ADR-0028 §4 step 4 requires reading `/proc/self/cgroup` to discover the *enclosing* slice and inspecting that slice's `subtree_control`. The implementation does not match the ADR. The §4 isolation claim — "no deployment runs with broken cgroup configuration silently" — is structurally false: an unprivileged operator on a host without `Delegate=yes` passes preflight, the server starts without `--allow-no-cgroups`, then every `Driver::start` later fails at the cgroup `mkdir` with `EACCES`.

The doc comment at `cgroup_preflight.rs:164-167` *says* "production (root path skipped above) reads the user slice's `subtree_control`" — describing behavior the code does not implement. This aspirational-doc shape contributed to the bug surviving review.

## Root cause chain (3 compounding causes)

### A. Production code reads the wrong file

- `run_preflight_at` (`cgroup_preflight.rs:141-194`) takes `cgroup_root: &Path` plus `uid: u32` but no representation of the *enclosing* slice.
- Step 4 at line 168 joins `cgroup.subtree_control` to `cgroup_root`, treating the parameter as if it were the enclosing slice — but in production it is the cgroupfs mount root.
- The function was written by collapsing two distinct concepts — (i) the *cgroupfs mount root* used for step 1/2 existence checks, and (ii) the *enclosing slice path* required for the step 4 delegation check — into a single `cgroup_root` parameter.
- ADR-0028 §4 step 4 explicitly says "Read `/proc/self/cgroup`; extract the cgroup path the process is in. Read `<that_path>/cgroup.subtree_control`". The implementation skips both reads — there is no `/proc/self/cgroup` access anywhere in the module.

### B. Tests fabricate fixtures congruent with the buggy production read

- `tests/integration/cgroup_isolation/preflight_no_delegation.rs:31` writes `cgroup_root.join("cgroup.subtree_control")` with `cgroup_root = tmp.path()`. Same flat layout in `preflight_missing_cpu.rs:25,57`.
- The tests pass a tempdir as `cgroup_root` and write `cgroup.subtree_control` directly under that tempdir — there is no `user.slice/user-1000.slice/` layout in any fixture.
- The fixtures are congruent with the bug, not with the ADR-specified production layout: the test layout matches whatever the code happens to read, so the tests cannot distinguish "code reads kernel-root" from "code reads enclosing slice."
- No oracle test exercises the production layout. The Lima/CI integration lane runs as root via `sudo` (`xtask/src/main.rs:240-247`), tripping the step-3 root short-circuit at `cgroup_preflight.rs:160-162` — so even the real-kernel lane never enters step 4.
- A previously-flagged missed mutation (`replace run_preflight -> Result<(), CgroupPreflightError> with Ok(())`) was acknowledged at `docs/evolution/2026-04-29-fix-convergence-loop-not-spawned.md:236-241` and never closed — the production preflight path was already known to be structurally untested.

### C. ADR-implementation drift was not caught at review

- The doc comment at `cgroup_preflight.rs:164-167` describes behavior the code does not implement.
- The error variant's `slice` field at `cgroup_preflight.rs:185` is populated with whatever path `subtree_control` was inspected at — so even the *rendered error message* would mislead an operator if it ever fired.
- No automated check (lint, doc-test extraction, ADR↔code traceability assertion) verifies that the implementation honors the named ADR. The ADR cross-reference in `lib.rs:23-26` is unidirectional and prose-only.

## Approved fix: Option B + injected `proc_self_cgroup` parameter

**Rejected: Option A** ("derive `user.slice/user-{uid}.slice/` from `cgroup_root` + `uid`") — encodes a systemd-specific layout into the safety floor; breaks under non-systemd init, nested slices, container runtimes, user-namespaced sessions. ADR-0028 explicitly says read `/proc/self/cgroup`.

**Rejected: Option C** (caller computes the slice path; preflight just receives it) — moves the discovery logic out of preflight and forces every caller to reimplement the `/proc/self/cgroup` parse correctly. Discovery belongs *in* preflight; only the path source is injected.

### Code changes

1. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — signature change to `run_preflight_at`:**
   - Add a fourth parameter `proc_self_cgroup: &Path` (production: `/proc/self/cgroup`; tests: tempdir-fabricated file).
   - The signature becomes:
     ```rust
     pub fn run_preflight_at(
         cgroup_root: &Path,
         uid: u32,
         proc_filesystems: &Path,
         proc_self_cgroup: &Path,
     ) -> Result<(), CgroupPreflightError>
     ```

2. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — production wrapper update:**
   - `run_preflight()` (line 202-213) passes `Path::new("/proc/self/cgroup")` as the new fourth argument.

3. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — step 4 body rewrite:**
   - Replace lines 164-191 with logic that:
     1. Reads `proc_self_cgroup` (returns `CgroupPathDiscoveryFailed` on I/O error).
     2. Parses the cgroup v2 line via a helper `parse_cgroup_v2_path(contents) -> Option<&str>` that finds the line beginning with `0::` and returns its tail.
     3. Returns `CgroupPathDiscoveryFailed` if no `0::` line is present (cgroup-v1-only host shape, malformed file).
     4. Joins the parsed (relative-stripped) path to `cgroup_root` to compute the enclosing slice directory.
     5. Reads `<enclosing_slice>/cgroup.subtree_control` and applies the same missing-controller logic as today.
   - The `DelegationMissing.slice` field now carries the enclosing slice **directory** (not the file path). The error template at line 87-88 (`subtree_control of {slice}`) still reads naturally; update the field doc comment.

4. **`crates/overdrive-control-plane/src/cgroup_preflight.rs` — new error variant `CgroupPathDiscoveryFailed`:**
   - Wraps `std::io::Error` via `#[source]`.
   - Display form names the failure ("could not discover enclosing cgroup from /proc/self/cgroup"), the cause, and the `--allow-no-cgroups` escape hatch — same shape as the existing variants per `nw-ux-tui-patterns`.

### Test updates (existing — must be updated, not just retained)

5. **`crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_no_delegation.rs`** — update fixture: write `<tmp>/user.slice/user-1000.slice/cgroup.subtree_control` (empty, or with controllers other than `cpu`/`memory`) AND a fake `/proc/self/cgroup` containing `0::/user.slice/user-1000.slice`. Pass the fake file path as the new fourth argument. Same uid (1000) as today.

6. **`crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_missing_cpu.rs`** — same fixture-shape update for both `preflight_names_missing_cpu_controller_specifically` and `preflight_names_missing_memory_controller_specifically`.

7. **`crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_v1_host.rs`** — call-site update: pass an additional placeholder `proc_self_cgroup` path. Step 1 fails before step 4 so the contents do not matter.

### Test additions (new — close the oracle gap from Root Cause B)

8. **NEW: `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_reads_enclosing_slice.rs`** — direct oracle for the bug. Fixture writes:
   - `<tmp>/cgroup.subtree_control` containing **all controllers** (`cpu memory io`) — simulates the kernel-root state.
   - `<tmp>/user.slice/user-1000.slice/cgroup.subtree_control` containing **neither `cpu` nor `memory`** (e.g. just `io pids`) — simulates a user slice without `Delegate=yes`.
   - `<tmp>/proc/self-cgroup` containing `0::/user.slice/user-1000.slice`.
   
   Asserts `DelegationMissing` returned. This test FAILS on the buggy code (kernel-root has all controllers → preflight passes) and PASSES on the fix (enclosing slice is missing both → preflight refuses). It is the missing oracle that should have caught the bug.

9. **NEW: `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_proc_self_cgroup_malformed.rs`** — covers `CgroupPathDiscoveryFailed`. Two cases:
   - `/proc/self/cgroup` containing only cgroup v1 lines (no `0::` line) → `CgroupPathDiscoveryFailed`.
   - `/proc/self/cgroup` empty → `CgroupPathDiscoveryFailed`.

### ADR/docs

- **`docs/product/architecture/adr-0028-cgroup-preflight-refusal.md`** — no change required; the ADR is correct; the implementation drifted.

## Files affected (absolute paths)

### Production
- `crates/overdrive-control-plane/src/cgroup_preflight.rs` — signature + body + new error variant.

### Existing tests (fixture/call-site updates)
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_no_delegation.rs`
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_missing_cpu.rs`
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_v1_host.rs`

### New tests (close the oracle gap)
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_reads_enclosing_slice.rs`
- `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_proc_self_cgroup_malformed.rs`

### Reference (read-only; informs the fix and the latent-issue note)
- `crates/overdrive-control-plane/src/cgroup_manager.rs`
- `crates/overdrive-control-plane/src/lib.rs` (boot sequence, lines 376-391; ADR cross-reference at lines 23-26)
- `docs/product/architecture/adr-0028-cgroup-preflight-refusal.md`
- `docs/evolution/2026-04-29-fix-convergence-loop-not-spawned.md` (mutation-testing context, lines 236-241)
- `xtask/src/main.rs` (Lima-runs-as-root evidence, lines 240-247)

## Risk assessment

| Risk | Disposition |
|---|---|
| Public API change to `run_preflight_at` | The function is `pub`; signature gains a fourth parameter. Search confirms it is called only from `run_preflight` (same module) and from the three integration tests in this same crate. No downstream crate imports it. Low risk; mechanical update to four call sites. |
| `--allow-no-cgroups` behavior | Untouched. The flag still bypasses preflight at the boot site. The fix changes only what preflight checks when it runs, not whether it runs. |
| ADR-0028 §4 boot ordering | Untouched. Preflight still runs before any on-disk side effects, still gates listener bind, still produces no on-disk artefacts on failure. |
| Production behavior change for non-root operators | This is the **intended** behavior change. Operators on hosts without `Delegate=yes` will, after the fix, be correctly refused at boot — the §4 isolation claim becomes honest. Operators following ADR-0028 §4 remediation 2 (`sudo systemctl set-property user-1000.slice Delegate=yes`) continue to pass preflight. |
| CI Lima VM (root-only execution) | Lima runs as root → step 3 short-circuits → step 4 never executes → fix is invisible to existing real-kernel integration tests. The new oracle test (`preflight_reads_enclosing_slice.rs`) executes the step-4 code path with `uid = 1000` against tempdir fixtures, so it runs in the default test lane regardless of the executing user. |
| Mutation testing kill rate | The pre-existing missed mutation flagged at `docs/evolution/2026-04-29-fix-convergence-loop-not-spawned.md:236-241` is closed by the new oracle test, since the new test exercises the production-shaped `run_preflight_at` path with a non-zero UID and asserts a specific failure shape. |
| Latent issue (out of scope) | `cgroup_manager::create_and_enrol_control_plane_slice` writes `overdrive.slice/...` directly at `/sys/fs/cgroup` root, which non-root operators cannot do *even with* `Delegate=yes` on their `user-<uid>.slice/`. This is a deeper Phase 1 question (where does the control plane's slice live for unprivileged single-node?) that ADR-0028 implicitly assumes is solved. **Not in scope for this fix** — flag as a separate open question. |

## Prevention strategy (per root cause)

- **A** — Fix the signature collapse: introduce `proc_self_cgroup: &Path` as a first-class injectable parameter. ADR-conformant production behavior; testable in isolation.
- **B** — Add the missing oracle test (`preflight_reads_enclosing_slice.rs`) that distinguishes "code reads root cgroup" from "code reads enclosing slice." Future regressions in either direction fail this test deterministically.
- **C** — Strengthen ADR↔code traceability via test naming/doc-comment convention. Out of scope for this PR; flag as a process improvement.
