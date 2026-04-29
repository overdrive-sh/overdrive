# Upstream Issues — exec-driver-rename DELIVER

## Issue 1: ADR amendments already committed before DELIVER start

**Source**: `docs/feature/exec-driver-rename/deliver/roadmap.json` step 01-02,
edit class (6) "ADR doc commit" — instructs the crafter to stage and commit
`docs/product/architecture/adr-0026-cgroup-v2-direct-writes.md` and
`docs/product/architecture/adr-0029-overdrive-worker-crate-extraction.md`
alongside the code change.

**Discovery**: At DELIVER orchestration start, commit `a7ab6ad`
(`docs(architecture): ADR-0030 + amendments — ExecDriver rename + command/args`)
on the current branch already contains:

- ADR-0026 amendment (Amendment 2026-04-28)
- ADR-0029 amendment (Amendment 2026-04-28)
- ADR-0030 (NEW; was not in the roadmap text)
- `brief.md` §29/§30 + AllocationSpec.image references migrated
- `whitepaper.md` §6 driver table row rename
- This roadmap and `wave-decisions.md` themselves

The roadmap was authored assuming the ADR amendments would be unstaged
in the working tree at DELIVER start, but the architect agent committed
them separately as part of the DESIGN wave.

**Resolution**: Step 01-02 will NOT re-commit the ADR markdown files.
They are already on the branch. The crafter's responsibility for step
01-02 narrows from six edit classes (per the roadmap) to five:

1. `crates/overdrive-core/src/traits/driver.rs` — rename `image` → `command`,
   add `args: Vec<String>`.
2. `crates/overdrive-worker/src/driver.rs` — rewrite `build_command`'s body;
   delete the magic image-name dispatch tree; make setsid pre-exec
   unconditional.
3. `crates/overdrive-control-plane/src/action_shim.rs` — migrate
   `build_phase1_restart_spec` constructor.
4. `crates/overdrive-core/src/reconciler.rs` — migrate the one
   `JobLifecycle::reconcile` call site.
5. Test fixture inline-args migration across `tests/integration/exec_driver/`,
   `cluster_status_under_burst.rs`, the acceptance fixtures.

**Acceptance criterion adjustment**: Step 01-02 AC #6
("ADR doc commit lands in the same commit") is satisfied by the prior
commit `a7ab6ad`. The crafter does NOT need to verify
`git diff HEAD~1 --stat -- docs/product/architecture/` shows the ADR
files modified — they were modified two commits earlier.

## Issue 2: Roadmap migration surface is non-exhaustive

**Source**: `wave-decisions.md` § "Migration surface (informative —
non-exhaustive)" + `roadmap.json` step 01-01 `files_to_modify`.

**Discovery**: `git grep -l "ProcessDriver" crates/` and `git grep -l
"DriverType::Process" crates/` reveal sites the roadmap's
`files_to_modify` does not enumerate, including:

- `crates/overdrive-control-plane/Cargo.toml`
- `crates/overdrive-worker/Cargo.toml`
- `crates/overdrive-control-plane/src/lib.rs`
- `crates/overdrive-control-plane/tests/integration/job_lifecycle/{cleanup,convergence_loop_spawned_in_production_boot,crash_recovery,stop_to_terminated,submit_to_running}.rs`
- `crates/overdrive-control-plane/tests/acceptance/{cluster_status_lists_both_reconcilers,job_lifecycle_backoff,runtime_convergence_loop,runtime_registers_noop_heartbeat,submit_job_idempotency}.rs`
- `crates/overdrive-control-plane/tests/integration/observation_empty_rows.rs`
- `crates/overdrive-sim/src/harness.rs`

**Resolution**: The roadmap's `wave-decisions.md` explicitly anticipates
this — the migration surface is informative, "non-exhaustive". The
crafter is expected to grep and find every site. AC #1 of step 01-01 is
the load-bearing constraint: `git grep -n 'ProcessDriver' crates/`
returns zero matches, `git grep -n 'DriverType::Process' crates/`
returns zero matches. The crafter must follow that until both grep
returns are zero, regardless of which files appeared in the list.
