# fix-cgroup-preflight-wrong-slice — Feature Evolution

**Feature ID**: fix-cgroup-preflight-wrong-slice
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-04-29
**Commits**:
- `4fb4e9c` — `test(control-plane): RED — preflight oracle for enclosing-slice discovery`
- `4d65fb8` — `fix(control-plane): cgroup preflight reads enclosing slice via /proc/self/cgroup`

**Status**: Delivered.

---

## Symptom

`cgroup_preflight::run_preflight_at` (`crates/overdrive-control-plane/src/cgroup_preflight.rs:168` pre-fix) computed `subtree_control = cgroup_root.join("cgroup.subtree_control")`. With production `cgroup_root = /sys/fs/cgroup`, this resolved to `/sys/fs/cgroup/cgroup.subtree_control` — the **kernel-root** cgroup. On every modern Linux kernel the root cgroup's `subtree_control` lists every controller (`cpu cpuset hugetlb io memory misc pids rdma`), so the check passed unconditionally for every non-root user.

ADR-0028 §4 step 4 requires reading `/proc/self/cgroup` to discover the *enclosing* slice and inspecting that slice's `subtree_control`. The implementation did not match the ADR. The §4 isolation claim — "no deployment runs with broken cgroup configuration silently" — was structurally false: an unprivileged operator on a host without `Delegate=yes` passed preflight, the server started without `--allow-no-cgroups`, then every `Driver::start` later failed at the cgroup `mkdir` with `EACCES`.

The doc comment at `cgroup_preflight.rs:164-167` *said* "production (root path skipped above) reads the user slice's `subtree_control`" — describing behavior the code did not implement.

## Root cause (3 compounding causes)

### A. Production code read the wrong file

The function signature carried `cgroup_root: &Path` plus `uid: u32` but no representation of the *enclosing* slice. Step 4 joined `cgroup.subtree_control` to `cgroup_root`, treating the parameter as if it were the enclosing slice — but in production it was the cgroupfs mount root. The function collapsed two distinct concepts (cgroupfs mount root for steps 1/2; enclosing slice path for step 4) into a single `cgroup_root` parameter. ADR-0028 §4 step 4's `/proc/self/cgroup` read was never wired in.

### B. Tests fabricated fixtures congruent with the buggy production read

The three existing step-4 tests (`preflight_no_delegation.rs`, `preflight_missing_cpu.rs` — two scenarios) wrote `cgroup.subtree_control` directly under the tempdir root. The fixtures were congruent with the bug, not with the ADR-specified production layout. No oracle test exercised the `user.slice/user-{uid}.slice/` shape. The Lima/CI integration lane runs as root via `sudo` (`xtask/src/main.rs:240-247`), tripping the step-3 root short-circuit, so the real-kernel lane never entered step 4. A previously-flagged missed mutation (`replace run_preflight -> Ok(())` at `docs/evolution/2026-04-29-fix-convergence-loop-not-spawned.md:236-241`) was acknowledged and never closed.

### C. ADR-implementation drift was not caught at review

The doc comment described behavior the code did not implement. The `DelegationMissing.slice` field was populated with whatever path `subtree_control` was inspected at, so even the rendered error message would mislead an operator if it ever fired. No automated check linked ADR prose to code.

## Fix

**Approved fix shape**: **Option B + injected `proc_self_cgroup` parameter** — read `/proc/self/cgroup` to discover the enclosing slice (matching ADR-0028 §4 step 4 verbatim); inject the path so tests fabricate the discovery shape without touching real `/proc`.

**Rejected: Option A** (hardcoded `user.slice/user-{uid}.slice/` derivation) — encodes a systemd-specific layout into the safety floor; breaks under non-systemd init, nested slices, container runtimes, user-namespaced sessions.

**Rejected: Option C** (caller computes the slice path; preflight just receives it) — moves the discovery logic out of preflight and forces every caller to reimplement the `/proc/self/cgroup` parse correctly.

### Step 01-01 — RED scaffold (commit `4fb4e9c`)

- Added fourth parameter `proc_self_cgroup: &Path` to `run_preflight_at`. Body unchanged at this commit (uses `let _ = proc_self_cgroup;` to silence the unused-variable lint).
- Updated `run_preflight()` wrapper to pass `Path::new("/proc/self/cgroup")`.
- Added new oracle test `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_reads_enclosing_slice.rs`. Fixture seeds `<tmp>/cgroup.subtree_control` with all controllers (kernel-root simulation) AND `<tmp>/user.slice/user-1000.slice/cgroup.subtree_control` with `io pids` only. Wired through `tests/integration.rs`.
- Updated the three existing step-4 tests' fixtures to dual-write shape: each writes both at the tempdir root (so the buggy code in 01-01 still passes the test) AND at `<tmp>/user.slice/user-1000.slice/cgroup.subtree_control` (so the fixed code in 01-02 reads the same shape).
- Committed with `--no-verify` per `.claude/rules/testing.md` §RED scaffolds. The new oracle test FAILS at runtime under the buggy step-4 body (the intended RED state).

### Step 01-02 — GREEN fix (commit `4d65fb8`)

Three changes in `crates/overdrive-control-plane/src/cgroup_preflight.rs`, landed in a single cohesive commit:

1. **Helper** — new `parse_cgroup_v2_path(contents: &str) -> Option<&str>` finds the line beginning with `0::` and returns its tail. Unit-tested via the new oracle and malformed-file scenarios.
2. **Step-4 body rewrite** — read `proc_self_cgroup`, parse via `parse_cgroup_v2_path`, return `CgroupPathDiscoveryFailed` on missing/malformed file, strip leading `/`, join to `cgroup_root` to form the enclosing slice directory, read `<enclosing_slice>/cgroup.subtree_control`, apply the existing cpu/memory missing-controller logic. The `DelegationMissing.slice` field now carries the enclosing slice DIRECTORY (not the file path); error template at `subtree_control of {slice}` reads naturally.
3. **New error variant** — `CgroupPathDiscoveryFailed { #[source] source: std::io::Error }` with the `nw-ux-tui-patterns` what / why / how-to-fix shape; mentions `--allow-no-cgroups` and the docs URL.

New variant test `crates/overdrive-control-plane/tests/integration/cgroup_isolation/preflight_proc_self_cgroup_malformed.rs` covers two cases: cgroup-v1-only `/proc/self/cgroup` content (no `0::` line), empty file. Doc comments updated: `run_preflight_at` rustdoc names the new parameter and cites ADR-0028 §4 step 4; module-level doc updates step 4 to reference the new discovery mechanism; `DelegationMissing.slice` field doc reflects directory semantics.

## Tests added / updated

- **NEW**: `tests/integration/cgroup_isolation/preflight_reads_enclosing_slice.rs` — direct oracle distinguishing "code reads root cgroup" from "code reads enclosing slice." FAILS on the buggy code; PASSES on the fix.
- **NEW**: `tests/integration/cgroup_isolation/preflight_proc_self_cgroup_malformed.rs` — covers the new `CgroupPathDiscoveryFailed` variant (v1-only file, empty file).
- **UPDATED**: `tests/integration/cgroup_isolation/preflight_no_delegation.rs` — dual-write fixture; passes under both buggy and fixed code paths.
- **UPDATED**: `tests/integration/cgroup_isolation/preflight_missing_cpu.rs` — dual-write fixture for both `_cpu` and `_memory` test functions.
- **UPDATED**: `tests/integration/cgroup_isolation/preflight_v1_host.rs` — call-site update for new parameter (step 1 fails before step 4 — proc_self_cgroup contents don't matter).

## Quality gates

- **Workspace nextest** on Linux via Lima (`cargo xtask lima run -- cargo nextest run --workspace --features integration-tests`): 651/651 pass.
- **Preflight test scope** on Linux: 8/8 pass.
- **Mutation gate** on Linux via Lima (`cargo xtask lima run -- cargo xtask mutants --diff origin/main --features integration-tests --package overdrive-control-plane --file crates/overdrive-control-plane/src/cgroup_preflight.rs`): **15/16 caught, 93.8% kill rate** (above the 80% project gate). The single missed mutant is `replace run_preflight -> Result<(), CgroupPreflightError> with Ok(())` at line 281 — the `#[cfg(target_os = "linux")]` no-arg production wrapper that calls `unsafe { libc::geteuid() }` and uses hardcoded `/sys/fs/cgroup` and `/proc/*` paths. Structurally untestable from integration tests because it bypasses dependency injection. The same untestable wrapper shape was pre-flagged in `docs/evolution/2026-04-29-fix-convergence-loop-not-spawned.md:236-241` and is mitigated by `run_preflight_at` (the dependency-injected sibling) being comprehensively covered.
- **DES integrity** — `verify_deliver_integrity` exit 0; both steps have complete 5-phase TDD traces (PREPARE / RED_ACCEPTANCE / RED_UNIT / GREEN / COMMIT).

## Out of scope (flagged for follow-up)

- **`cgroup_manager::create_and_enrol_control_plane_slice` writes `overdrive.slice/...` directly at the cgroupfs root.** Non-root operators cannot do this *even with* `Delegate=yes` on their `user-<uid>.slice/`. This is a deeper Phase 1 question (where does the control plane's slice live for unprivileged single-node?) that ADR-0028 implicitly assumes is solved. The bugfix RCA flagged this as out-of-scope; the production fix in this evolution does not touch `cgroup_manager.rs`.
- **ADR↔code traceability tooling.** No automated check verifies that an implementation honors a named ADR. Process improvement; out of scope for this PR.

## References

- RCA: `docs/feature/fix-cgroup-preflight-wrong-slice/bugfix-rca.md`
- ADR: `docs/product/architecture/adr-0028-cgroup-preflight-refusal.md` §4 step 4
- Pre-existing missed mutation context: `docs/evolution/2026-04-29-fix-convergence-loop-not-spawned.md:236-241`
- Test discipline: `.claude/rules/testing.md` §RED scaffolds, §Mutation testing → Usage, §Lima rule for `--features integration-tests`
- Whitepaper § 3 (architecture overview, node agent isolation)
