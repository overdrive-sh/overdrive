# exec-driver-rename - Feature Evolution

**Feature ID**: exec-driver-rename
**Type**: Internal architectural cleanup (nWave DESIGN -> DELIVER)
**Branch**: `marcus-sa/phase1-first-workload`
**Date**: 2026-04-29

**Commits** (in order on branch):

- `a7ab6ad` - DESIGN wave doc commit (ADR-0030 + amendments to ADR-0026/0029, brief.md, whitepaper.md)
- `bab946c` - DELIVER step 01-01: rename ProcessDriver to ExecDriver, DriverType::Process to DriverType::Exec
- `e4e95c9` - DELIVER step 01-02: rename AllocationSpec.image to command; add args; drop magic image-name dispatch
- `11a3921` - DELIVER mutation gate revision: tighten signal-helper assertions; mutants:skip cgroup-error idempotency guards

**Status**: Delivered. All quality gates green. Mutation kill rate 83.3% (gate >= 80%).

---

## Summary

A focused, single-PR architectural cleanup of the exec-style workload driver shipped by `phase-1-first-workload`. Three coordinated changes in two cohesive commits: rename `ProcessDriver` to `ExecDriver` in `overdrive-worker`, rename `DriverType::Process` to `DriverType::Exec` in `overdrive-core`, and rename `AllocationSpec.image` to `AllocationSpec.command` while adding `AllocationSpec.args: Vec<String>` and dropping the magic image-name dispatch in `ExecDriver::build_command`. Single-cut greenfield migration per `feedback_single_cut_greenfield_migrations`: no `#[deprecated]` aliases, no compatibility shims, no two-phase rollout.

## Context

Three forces converged on this cleanup, all surfaced during `phase-1-first-workload` PR review:

**Vocabulary alignment.** Nomad exec-style task driver is named `exec` (HashiCorp canonical naming); Talos uses the same vocabulary. "Process" was an internal-implementation noun (the driver uses `tokio::process` underneath), but the operator-facing concept is "execute a binary directly," which the wider operator community already calls `exec`. Carrying internal-implementation vocabulary in operator-facing types creates friction every time an operator from another orchestrator reads the code or the docs.

**Honest field naming.** `image` is borrowed from container land, where `docker.io/library/postgres:15` is genuinely an image identifier. For an exec driver running binaries directly, `/bin/sleep` is a binary path, not a content-addressed image. The `build_command` body literally read `Command::new(&spec.image)` - self-documenting evidence that the field was misnamed.

**Removing magic dispatch.** Because `AllocationSpec` could not carry argv prior to this feature, `ExecDriver::build_command` papered over the gap with hardcoded image-name routing. Production code was reading test-fixture intent. The right shape is the spec carries argv (`args: Vec<String>`), the driver runs `Command::new(&spec.command).args(&spec.args)`, and test fixtures construct argv inline.

This was approved directly by the user as "Option C" during PR review. DISCUSS / DISTILL waves were skipped at user direction; the roadmap criteria fields carried the AC load.

## Key decisions (from wave-decisions.md)

- **D1**: Rename `ProcessDriver` to `ExecDriver` (`overdrive-worker`). Aligns with Nomad `exec` driver vocabulary and Talos terminology; "Process" was an internal-implementation noun. ADR-0029 amendment 2026-04-28.
- **D2**: Rename `DriverType::Process` to `DriverType::Exec` (`overdrive-core`). Operator-facing identity; matches D1; future variants (`MicroVm`, `Wasm`) are operator-canonical too. ADR-0029 amendment 2026-04-28.
- **D3**: Rename `AllocationSpec.image` to `AllocationSpec.command`; add `args: Vec<String>`. Container-image terminology is wrong for an exec driver; missing args forced magic dispatch (technical debt). `command` matches Nomad `exec` task driver field name. ADR-0026 amendment 2026-04-28; ADR-0030.
- **D4**: Drop magic image-name dispatch in `ExecDriver::build_command`; body becomes `Command::new(&spec.command).args(&spec.args)`; setsid pre-exec hook becomes unconditional. Production code stops reading test-fixture intent; every workload gets its own process group. ADR-0029 amendment 2026-04-28.

Three further structural choices reinforced the discipline:

- **No crate boundaries moved.** `overdrive-worker` stays; `overdrive-core` stays. The `Driver` trait surface (`start` / `stop` / `status` / `resize`) is unchanged. Only the impl struct name and the spec field shape change.
- **Wire shape unaffected.** `AllocationSpec` is internal - the HTTP API and OpenAPI schema never exposed `image` directly. The rename is purely intra-process.
- **Two-commit decomposition.** Type-rename (mechanical) ships separately from spec-shape change (substantive). Reviewer-friendly diffs without weakening single-cut discipline.

## Steps completed

### Step 01-01 - Mechanical type rename (commit `bab946c`)

Phase events:

- `PREPARE` - EXECUTED, PASS (2026-04-29 04:08:51Z)
- `RED_ACCEPTANCE` - SKIPPED (mechanical type rename; zero behaviour change; existing integration suite is the contract)
- `RED_UNIT` - SKIPPED (no new behaviour to defend with a unit test)
- `GREEN` - EXECUTED, PASS (2026-04-29 04:27:44Z)
- `COMMIT` - EXECUTED, PASS (2026-04-29 04:32:16Z)

Surface: every `ProcessDriver` reference to `ExecDriver`; every `DriverType::Process` to `DriverType::Exec`; integration test directory rename `tests/integration/process_driver/` to `tests/integration/exec_driver/`; per-file test-fn renames; re-export adjustments in `overdrive-worker/src/lib.rs`. AC #1 (zero `ProcessDriver` matches in `crates/`) and AC #2 (zero `DriverType::Process` matches) were the load-bearing constraints.

### Step 01-02 - Spec-shape change + magic-dispatch removal (commit `e4e95c9`)

Phase events:

- `PREPARE` - EXECUTED, PASS (2026-04-29 04:36:56Z)
- `RED_ACCEPTANCE` - SKIPPED (refactor; existing integration suite is the contract)
- `RED_UNIT` - SKIPPED (refactor; pre/post behaviour identical for every test fixture)
- `GREEN` - EXECUTED, PASS (2026-04-29 05:34:26Z)
- `COMMIT` - EXECUTED, PASS (2026-04-29 05:47:32Z)

Surface (narrowed from six edit classes to five - see Issues):

1. `crates/overdrive-core/src/traits/driver.rs` - rename `image` to `command`, add `args: Vec<String>`.
2. `crates/overdrive-worker/src/driver.rs` - rewrite `build_command` body; delete the magic image-name dispatch tree; make setsid pre-exec hook unconditional.
3. `crates/overdrive-control-plane/src/action_shim.rs` - migrate `build_phase1_restart_spec` constructor.
4. `crates/overdrive-core/src/reconciler.rs` - migrate the one `JobLifecycle::reconcile` call site.
5. Test fixture inline-args migration across `tests/integration/exec_driver/`, `cluster_status_under_burst.rs`, and acceptance fixtures.

## Mutation gate journey

The mutation gate ran on step 01-02 diff scope (`cargo xtask mutants --diff origin/main --features integration-tests`) under `cargo xtask lima run --` per `.claude/rules/testing.md` Mutation testing section.

**First run**: 75.0% kill rate. Nine missed mutants surfaced, but on inspection the missed mutations were not in the rename-touched code - they were in surrounding `phase-1-first-workload` code that fell into the diff scope because step 01-01 type-rename commit modified module-level doc-comments in those files. cargo-mutants treats any file whose diff touches a mutable site as in-scope; rename-induced doc-comment migrations expanded the surface from "the rename change" to "every function in any rename-touched file."

The missed mutations clustered in two areas:

- **Signal-handling helpers** in `overdrive-worker/src/driver.rs` `stop_with_grace` and `stop_escalates_to_sigkill` paths - the surrounding tests asserted on terminal allocation state but did not pin process-tree behaviour (was the SIGKILL escalation actually needed? Were grandchild PIDs reaped via process-group SIGKILL?).
- **Cgroup-error idempotency guards** in `cgroup_kill` and `remove_workload_scope` - async I/O race-handling paths whose intermediate states are genuinely unobservable without injecting a fault, which the test harness does not yet support for these call sites.

**Revision pass (commit `11a3921`)**: tightened the signal-helper assertions in `tests/integration/exec_driver/stop_with_grace.rs` (now asserts both exit-within-grace AND that no SIGKILL escalation was needed) and `tests/integration/exec_driver/stop_escalates_to_sigkill.rs` (now asserts grandchild PIDs reaped via process-group SIGKILL). Added `// mutants: skip` annotations on `cgroup_kill` and `remove_workload_scope` for the genuinely-untestable async I/O race-handling paths, with explanatory comments above each annotation.

**Final kill rate**: 83.3% - gate cleared.

## Lessons learned

1. **Doc-comment migrations expand the mutation gate diff scope.** Renaming a type in module-level doc-comments touches every file in which the type is documented, which pulls every function in those files into cargo-mutants diff scope on the next per-PR run. Single-cut migrations must anticipate this when the rename is in a heavily-documented module. The DESIGN wave prediction that the type-rename commit would have "essentially nil" mutation surface was incorrect for exactly this reason.

2. **cargo-mutants v27.0.0 does NOT honour `// mutants: skip` on async functions** in this configuration. The annotations remain in source as documented review-time signal but did not affect the kill-rate computation. Future per-feature mutation gates should rely on test strengthening (the path that actually moved the gate from 75.0% to 83.3% in this feature), not on skip annotations.

3. **`// mutants: skip` annotations remain load-bearing as DOCUMENTATION.** Even though they did not change the kill-rate computation here, the annotations and their explanatory comments document why specific guards are genuinely untestable. Future readers see the rationale; future toolchain upgrades that *do* honour the annotations on async fns will pick them up automatically.

4. **Single-cut greenfield rename worked cleanly.** No shims, no deprecations, no shadow re-exports. The two-commit decomposition (mechanical type rename then substantive spec-shape change) gave reviewer-friendly diffs without compromising single-cut discipline. The pattern is reusable for future renames of the same shape.

5. **Roadmap migration surface should be explicitly marked non-exhaustive.** The roadmap `files_to_modify` list missed sites the crafter found via `git grep` (control-plane crate `lib.rs`, integration tests, sim harness, etc.). The DESIGN wave `wave-decisions.md` explicitly anticipated this and labelled the migration surface "non-exhaustive" - the acceptance-criterion zero-grep clauses (AC #1: `git grep -n ProcessDriver crates/` returns zero matches) were the load-bearing constraint, not the file list. Future roadmaps covering renames should keep this discipline: zero-grep clauses as ACs, file lists as informative.

## Issues encountered

### Process violation: `git stash` use during step 01-01 GREEN phase

The crafter for the type-rename commit used `git stash` once during quality-gate verification. Per `feedback_no_git_stash` (2026-04-25 explicit user feedback) `git stash` should not be used to scope commits in this codebase; the documented pattern is `git add <paths>` + `git restore --staged`. Both `stash push` and `stash pop` succeeded so no work was lost; the crafter transparently self-reported the violation in their phase report. The commit landed clean, with no extraneous files staged.

### DESIGN/DELIVER divergence: ADR amendments pre-committed

The roadmap step 01-02 instructed the crafter to stage and commit ADR markdown files (`adr-0026`, `adr-0029`) alongside the spec-shape code change, treating them as a sixth edit class. At DELIVER orchestration start, commit `a7ab6ad` on the current branch already contained the ADR-0026 amendment, the ADR-0029 amendment, ADR-0030 (NEW; was not in the roadmap text), brief.md sections 29/30 + AllocationSpec.image references migrated, and whitepaper.md section 6 driver table row rename. The roadmap was authored assuming the ADR amendments would be unstaged in the working tree at DELIVER start, but the architect agent committed them separately as part of the DESIGN wave.

This was documented in `docs/feature/exec-driver-rename/deliver/upstream-issues.md` at DELIVER start. Step 01-02 edit-class count narrowed from six to five; AC #6 ("ADR doc commit lands in the same commit") was satisfied by the prior commit `a7ab6ad`.

### Migration surface non-exhaustive (anticipated by DESIGN)

The roadmap `files_to_modify` for step 01-01 was non-exhaustive. `git grep` revealed sites the list did not enumerate: `crates/overdrive-control-plane/Cargo.toml`, `crates/overdrive-worker/Cargo.toml`, `crates/overdrive-control-plane/src/lib.rs`, integration tests under `crates/overdrive-control-plane/tests/integration/job_lifecycle/`, acceptance tests under `crates/overdrive-control-plane/tests/acceptance/`, `crates/overdrive-control-plane/tests/integration/observation_empty_rows.rs`, and `crates/overdrive-sim/src/harness.rs`. The DESIGN wave explicitly labelled the migration surface "informative - non-exhaustive" and pointed to the AC #1 zero-grep clause as the load-bearing constraint. The crafter followed the grep until both `ProcessDriver` and `DriverType::Process` returned zero matches in `crates/`. No process change required.

## Links to permanent artifacts

The DESIGN wave migrated the lasting documentation directly via commit `a7ab6ad` (architect agent), so the post-finalize state already has the artifacts at their permanent paths:

- **ADR-0030** (NEW): `docs/product/architecture/adr-0030-exec-driver-and-allocation-spec-args.md`
- **ADR-0026 amendment 2026-04-28**: appended to `docs/product/architecture/adr-0026-cgroup-v2-direct-writes.md`
- **ADR-0029 amendment 2026-04-28**: appended to `docs/product/architecture/adr-0029-overdrive-worker-crate-extraction.md`
- **Architecture brief updates**: `docs/product/architecture/brief.md` sections 29/30 references migrated; `AllocationSpec.image` references migrated.
- **Whitepaper update**: `docs/whitepaper.md` section 6 driver table row rename `process` to `exec`.

ADR-0021 (`adr-0021-state-shape-for-reconciler-runtime.md`) was investigated during DESIGN; no amendment was required. The `AnyState` enum body does not reference `image` or `ProcessDriver`.

The amendment-in-place pattern matches the precedent that ADR-0026 and ADR-0029 already established (each was amended in place rather than spawning a successor ADR). ADR-0030 was added because the spec-shape change (D3) is structurally a new architectural commitment about the `AllocationSpec` type evolution as more driver classes land, not just an extension of the cgroup or worker-extraction narratives.

## Final verification (at HEAD `11a3921`)

- `cargo nextest run --workspace` (default lane): 507/507 passing
- `cargo test --doc --workspace`: passing
- `cargo xtask dst`: passing
- `cargo xtask dst-lint`: passing
- `cargo clippy --workspace --all-targets --no-deps -- -D warnings`: passing
- Mutation gate: 83.3% (gate >= 80% cleared)
- `git grep -n ProcessDriver crates/`: zero matches
- `git grep -n DriverType::Process crates/`: zero matches

The feature is fully delivered.

## Addendum (commit `2210866`): replaced `mutants: skip` with real coverage

After the mutation gate cleared at 83.3% via test strengthening, a follow-up review pass replaced the `// mutants: skip` annotations on `cgroup_kill` and `remove_workload_scope` with real targeted unit tests. The annotations were misleading on two fronts: cargo-mutants v27 did not honour them (so the prior revision pass cleared the gate on the strength of the signal-helper improvements alone), and the justification ("fault-injection required") overstated the case. Targeted unit tests with `tempfile` fixtures kill the tractable mutants without new infrastructure.

Three new unit tests in `crates/overdrive-worker/src/cgroup_manager.rs`:

- `cgroup_kill_writes_one_to_cgroup_kill_file` - pins the side effect by reading `cgroup.kill` back from disk (kills body to `Ok(())` mutation).
- `cgroup_kill_propagates_non_notfound_errors` - places a regular file at the scope path so `tokio::fs::write` returns `NotADirectory`; pins error propagation (kills the outer `NotFound` match-guard mutation).
- `remove_workload_scope_propagates_non_enotempty_non_notfound_errors` - places a symlink at the scope path so `remove_dir` returns `NotADirectory` while `remove_dir_all` would succeed; pins that the outer call returns the `remove_dir` error rather than routing through the fallback (kills the `is_dir_not_empty` match-guard mutation).

The inner `remove_dir_all` `NotFound` guard at line 299 was over-defensive for a single-writer subsystem: `ExecDriver` owns the workload scope lifecycle through its `live` mutex (`driver.rs`), and the race the guard handled - scope removed by an out-of-band actor between the outer `remove_dir` call and the inner `remove_dir_all` call within one cleanup pass - is unreachable in production. Per `.claude/rules/development.md` Deletion discipline section, deleting unreachable defensive code is the correct shape; the remaining outer guards (NotFound and is_dir_not_empty) are sufficient. Removing the arm eliminates mutants 4, 5, 6 from the surface entirely.

**Lesson reinforced**: Lessons 2 and 3 above remain accurate (cargo-mutants v27 does not honour `// mutants: skip` on async fns; the annotations were valuable as DOCUMENTATION). The follow-up adds a sixth lesson:

6. **Prefer real coverage over `mutants: skip`, even when the gate already passes.** The first revision pass (commit `11a3921`) cleared the gate at 83.3% via signal-helper test strengthening alone, then added `mutants: skip` annotations to the cgroup-error guards as belt-and-braces "documentation" of why they were untestable. On review, the annotations were both ineffective (cargo-mutants v27 ignores them on async) AND overstated the testability problem - targeted unit tests with `tempfile` fixtures kill the tractable mutants without fault injection. Where a real test is reachable, write the real test. The annotation lives on only when the call genuinely cannot be exercised by a deterministic test.

Final HEAD is `2210866`; quality gates remain green; mutation kill rate remains >= 80% with the `mutants: skip` annotations replaced by real test coverage.
