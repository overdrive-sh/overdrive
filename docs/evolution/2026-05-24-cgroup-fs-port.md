# cgroup-fs-port — Feature Evolution

**Feature ID**: `cgroup-fs-port`
**Branch**: `marcus-sa/cgroup-fs-port`
**Duration**: 2026-05-24 (DISCUSS opened) — 2026-05-25 (finalize close)
**Status**: Delivered — 8/8 DELIVER steps complete (DES integrity 8/8),
Phase 4 adversarial review APPROVED (no blocking findings),
Phase 3 refactor pass (RPP L1-L6) landed, mutation testing skipped
per user decision (orchestrator-approved), 1467/1467 tests green
post-cleanup with leftover-cgroup detection clean.
**Closes**: GH [#136](https://github.com/marcus-sa/overdrive/issues/136)
**ADRs**: [ADR-0054 — Narrow `CgroupFs` port for fault-injection testability](../product/architecture/adr-0054-cgroup-fs-port.md)
(amended once mid-implementation, 2026-05-24)
**Brief changelog**: `docs/product/architecture/brief.md` lines 3024–3025
(initial row + amendment row)

---

## What shipped

A narrow `CgroupFs` port-trait abstraction over the small set of
`tokio::fs::*` calls the cgroup-writing path makes, with a production
adapter (`RealCgroupFs` in `overdrive-host`) and a simulation adapter
(`SimCgroupFs` in `overdrive-sim`). `ExecDriver::new` now takes
`fs: Arc<dyn CgroupFs>` as a mandatory constructor parameter — there
is no `Default`, no `with_fs(...)` builder, and no `new_with_default_fs`
factory. The composition root in `overdrive-cli`'s `serve` subcommand
instantiates `RealCgroupFs`, calls its Earned Trust `probe()` BEFORE
the worker subsystem starts, and threads `Arc<dyn CgroupFs>` through
`run_server` into `ExecDriver::new`. Probe failure surfaces as a
`health.startup.refused` event and the binary exits non-zero.

The free-function shape in `crates/overdrive-worker/src/cgroup_manager.rs`
(eight standalone fns, each taking a `cgroup_root: &Path` plus per-call
arguments) collapses into methods on a new `CgroupManager { fs: Arc<dyn
CgroupFs>, cgroup_root: PathBuf }` struct. The old free-fn signatures
are *gone* — single-cut greenfield migration per
`feedback_single_cut_greenfield_migrations`, no `#[deprecated]`
markers, no compatibility shims.

The `SimCgroupFs` adapter is backed by a `BTreeMap<PathBuf, (SimEntry,
Vec<u8>)>` byte store under a `parking_lot::Mutex`, with a per-`(SimOp,
PathBuf)` injectable error schedule (`BTreeMap`-keyed `VecDeque<io::
ErrorKind>`) for fault-injection testing. The map is **byte-write
only** — it deliberately does NOT model kernel-side effects (`cgroup.kill`
mass-kill, `cgroup.subtree_control` EBUSY-on-live-child, controller-value
rejection, kernel-managed pseudo-files). Tier 3 integration tests under
`cargo xtask lima run --` remain mandatory for kernel-semantic coverage;
the trait surface does not pretend otherwise.

35 acceptance scenarios across 6 classes:

- **Class A** — trybuild compile-fail fixtures (2 fixtures: `A1`
  ExecDriver missing `fs` arg, `A2` SimCgroupFs leaks past test deps).
- **Class B** — SimCgroupFs unit-level scenarios (17 scenarios at the
  trait surface; bytes-only contract).
- **Class C** — Tier 3 real-cgroupfs kernel-semantics scenarios (7,
  including `C-probe-success`, `C-probe-with-custom-root`, `C-cgroup-
  kill`, `C-subtree-control-ebusy`, `C-pseudo-file-rules`).
- **Class D** — Real/Sim equivalence proptest (D1: 1024 default cases
  per CI run; caught 4 real Sim semantic gaps at first run, all fixed
  inline in the same step).
- **Class E1** — inline-test triage matrix on the existing 12
  tempfile-backed `cgroup_manager` unit tests (8 convert to SimCgroupFs,
  4 stay tempfile-backed against `RealCgroupFs` for ENOTDIR error-kind
  discrimination that requires real kernel VFS semantics).
- **Class F1** — K3 determinism guard (same seed → bit-identical SimCgroupFs
  trajectory across two harness runs).

---

## Why now — business context

GH [#136](https://github.com/marcus-sa/overdrive/issues/136) tracked
the absence of fault-injection testability on the cgroup-writing path.
Prior to this feature, the worker's `ExecDriver::start` /
`ExecDriver::stop` paths called `tokio::fs::{create_dir, write,
remove_dir, read, remove_file}` directly. There was no way to drive
EACCES, EBUSY, EROFS, ENOSPC, or ENOTDIR through the convergence loop
under DST control — the test fixtures either touched real
`/sys/fs/cgroup` (Tier 3, Lima-only, slow, single-machine) or mounted
`tempfile::TempDir` as a fake cgroupfs (broken contract: tmpfs's
`O_CREAT` semantics do not match kernel cgroupfs, which is exactly what
caused the `fix-cgroup-subtree-control-delegation` evolution
2026-05-09 — see `docs/evolution/2026-05-09-fix-cgroup-subtree-
control-delegation.md`).

The narrow port trait is the structural fix: production wires
`RealCgroupFs`, tests wire `SimCgroupFs` with whatever error schedule
the scenario needs, and the convergence loop's reaction to a permissions
failure or a kernel-busy state is exercisable in-process under deterministic
control. This aligns the cgroup-writing path with every other source of
nondeterminism in the codebase (`Clock`, `Transport`, `Entropy`,
`Driver`, `Dataplane`) per `.claude/rules/development.md` §
"Port-trait dependencies".

---

## Architectural decisions

**ADR-0054** at `docs/product/architecture/adr-0054-cgroup-fs-port.md`
records the full decision (rationale, alternatives, trade-offs, probe
semantics). Highlights worth surfacing here:

1. **Narrow trait, not broad `Filesystem`.** Five methods (`create_dir`,
   `write`, `read`, `remove_dir`, `remove_file`) sized to the cgroup-
   writing path's actual surface. Alternative B (a generic `Filesystem`
   trait covering all `tokio::fs::*` usage in the workspace) was
   rejected — it would have widened the SimCgroupFs replacement contract
   to file systems we have no current production wiring for, and the
   added surface buys no current testability.

2. **`Arc<dyn CgroupFs>` is mandatory in `ExecDriver::new`.** No
   `Default` impl, no `with_fs(...)` builder, no `new_with_default_fs`
   factory function. Per `.claude/rules/development.md` § "Port-trait
   dependencies" → "Required, not defaulted, at the call site":
   builder-pattern overrides on port traits silently inherit production
   bindings into tests that forget to override, which is exactly the
   failure mode the trait exists to prevent. trybuild fixture A1
   structurally pins this — a call site that omits `fs` fails to
   compile, and the `.stderr` baseline is checked in.

3. **Composition root probes before worker startup.** `serve` constructs
   `RealCgroupFs`, calls `probe()` (Earned Trust per CLAUDE.md
   principle 12 — round-trip a payload through the substrate), and
   only on `Ok(())` proceeds to `run_server`. A failed probe emits a
   `health.startup.refused` event and exits non-zero. The probe runs
   BEFORE any worker spawns, so a misconfigured cgroupfs delegation
   never silently degrades into per-tick warnings during convergence
   (the exact failure mode that motivated the 2026-05-09 evolution).

4. **SimCgroupFs is bytes-only and explicitly NON-replacement.** ADR-
   0054 § "Non-replacement contract" pins this: SimCgroupFs does NOT
   model `cgroup.kill` mass-kill, `subtree_control` EBUSY-on-live-child,
   controller-value rejection, or kernel-managed pseudo-file rules.
   Tier 3 kernel-semantics tests (Class C, 7 scenarios) remain
   mandatory; they exercise the kernel surface SimCgroupFs deliberately
   refuses to model. ADR-0034's removal of `--allow-no-cgroups` continues
   to hold — SimCgroupFs cannot smuggle in as production wiring because
   the existing `cgroup_preflight` v2-delegation gate from ADR-0028
   still requires real `/sys/fs/cgroup`.

5. **Single-cut greenfield migration.** Old free-fn signatures
   `create_workloads_slice_with_controllers(cgroup_root: &Path)`,
   `enrol_pid_into_alloc_scope(cgroup_root: &Path, alloc_id, pid)`,
   etc. **deleted** at step 01-04 in the same commit that introduced
   the method shape on `CgroupManager`. No `#[deprecated]` markers, no
   cfg-flagged compatibility shims, no parallel API surfaces — per
   `feedback_single_cut_greenfield_migrations`. Transitional inner-loop
   compile glue (`cgroup_manager_legacy_*` shims) bridged step 04 → 05
   and was deleted at step 05.

---

## ADR-0054 mid-implementation amendment (2026-05-24)

The first probe spec landed in step 01-01: write `b"probe\n"` to a
regular `probe-file` inside the probe cgroup directory, read back, assert
byte-equality. DELIVER step 01-02 empirically falsified this against
real `/sys/fs/cgroup` in Lima: **cgroupfs only permits kernel-managed
pseudo-files inside cgroup directories.** The regular-file write was
rejected at the kernel substrate (`EACCES` at `openat(O_CREAT)`
regardless of UID, because the kernel's permission check fires before
the inode-existence check and userspace cannot populate a new inode in
cgroupfs).

Step 01-02 hit COMMIT-PHASE-FAIL and escalated to the user per
`.claude/rules/development.md` § "Deferrals require GitHub issues —
AND user approval BEFORE creation" (no autonomous tracking issue
created). User-approved 2026-05-24: pivot the probe shape to round-trip
on `cgroup.subtree_control` (a kernel-managed pseudo-file the production
code already touches):

1. `create_dir` the probe leaf cgroup.
2. `write(&probe_dir.join("cgroup.subtree_control"), b"")` — empty
   controller-diff, kernel-supported no-op.
3. `tokio::fs::read` and assert "no error + valid UTF-8 response" — NOT
   byte-equality with what was written. The kernel returns its own
   canonical controller-list payload; asserting byte-equality with `b""`
   would always fail.
4. `remove_dir` the probe leaf. No `remove_file` — the kernel forbids
   unlinking its own pseudo-files; they GC on rmdir.

This is **still faithful to Earned Trust** because the new round-trip
exercises the same kernel surface production code touches every boot
(`cgroup.subtree_control` writes during slice bootstrap per the
2026-05-09 evolution). The amendment is recorded in two places: the
ADR itself (§ Production probe + § Alternatives considered →
Alternative F documenting the rejected regular-file approach with
empirical disproof) and the brief.md changelog (line 3025).

`ProbeError::RoundTripMismatch { wrote, read }` was repurposed rather
than renamed: for RealCgroupFs `wrote = vec![]` and the variant fires
on non-UTF-8 kernel response (substrate-lying signal); for SimCgroupFs
the original byte-equality semantics are preserved. Single error
shape, two adapter interpretations.

---

## Wave execution summary

15 commits across all four waves + finalize:

| # | Commit | Wave | Step | What |
|---|---|---|---|---|
| 1 | `88e7ded0` | DESIGN | — | Feature-delta + ADR-0054 + brief.md changelog row |
| 2 | `deaf72fa` | DISTILL | — | 35 acceptance scenarios across 6 classes |
| 3 | `7dfe81d5` | DISTILL | — | Clarify E1 triage scope (excludes `cgroup_path_*` ATs) |
| 4 | `9f3aae88` | DELIVER | Phase 1 plan | Roadmap.json, 8 steps, 26h advisory |
| 5 | `b57c8bdd` | DELIVER | 01-01 | CgroupFs trait + ProbeError + A1/A2 trybuild scaffold |
| 6 | `9602b0a3` | DESIGN amend | — | ADR-0054 amendment: round-trip `cgroup.subtree_control` (user-approved mid-impl) |
| 7 | `ee94d37c` | DELIVER | 01-02 | RealCgroupFs adapter (Earned Trust probe) |
| 8 | `2f437e25` | DELIVER | 01-03 | SimCgroupFs adapter (17 Class B + F1) |
| 9 | `17e02040` | DELIVER | 01-04 | `cgroup_manager` refactor: free fns → `CgroupManager` methods |
| 10 | `d343e9c0` | DELIVER | 01-05 | `ExecDriver::new` mandatory `fs` param; A1+A2 trybuild green |
| 11 | `7e0b0c92` | DELIVER | 01-06 | Composition root probes RealCgroupFs before worker startup |
| 12 | `3c3672df` | DELIVER | 01-06 fix-up | Removed `new_with_default_fs` factory anti-pattern (user-flagged) |
| 13 | `599e6145` | DELIVER | 01-07 | Bootstrap async migration + D1 Real/Sim equivalence proptest |
| 14 | `1c9837ac` | DELIVER | 01-08 | 7 Class C Tier 3 kernel-semantics scenarios |
| 15 | `2070251e` | DELIVER Phase 3 | — | RPP L1-L6 polish pass (3 helpers, 1 docstring align, 2 breadcrumb removals) |
| 16 | `22c94ada` | DELIVER chore | — | Skip untestable mutant on `build_probe_adapter` guard (96% → 100% on testable surface) |

Each DELIVER step landed five DES TDD phases (`PREPARE`,
`RED_ACCEPTANCE`, `RED_UNIT`, `GREEN`, `COMMIT`). `RED_UNIT` was
correctly `SKIPPED` on every step with a documented `NOT_APPLICABLE`
disposition — at this trait-surface layer the structural defense is
the AT scenarios themselves (Class A trybuild, Class B SimCgroupFs,
Class C Tier 3, Class D equivalence proptest); no PBT-worthy unit
logic exists separately. DES integrity verifier returned exit 0 on
the final pre-finalize check.

---

## Lessons learned

1. **Probe spec verification belongs in DESIGN, not at first GREEN.**
   The original regular-file probe spec passed architect review and the
   first DELIVER step landed without anyone noticing the kernel
   substrate would reject it. The empirical falsification arrived at
   step 01-02 COMMIT — by which point the trait, the error type, and
   the probe scaffold had all been built around the wrong spec. The
   amendment was clean (4 changes to one ADR, one extra brief.md row)
   but the structural lesson is that any probe that round-trips through
   a kernel-managed substrate should be **executed against the real
   substrate during DESIGN review**, not deferred to GREEN. ADR-0054 §
   Alternatives → Alternative F now documents the original-spec
   rejection with the kernel evidence; future probe-shape decisions in
   the same area can cite it.

2. **Mid-implementation ADR amendments are honest forward pointers when
   user-approved at the moment of falsification.** The amendment ran
   through the standard "surface, ask, get explicit approval, then act"
   flow per CLAUDE.md § "Deferrals require GitHub issues — AND user
   approval BEFORE creation". The COMMIT phase failed honestly
   (`d`-disposition `BLOCKED_BY_DEPENDENCY` with full RCA in the DES
   trace), the working tree retained the uncommitted RED scaffold + adapter
   for inspection, and the amendment landed in a separate commit that
   referenced the empirical evidence. No autonomous issue created; no
   silent deferral; no "we'll figure this out later" handwaving.

3. **The `new_with_default_fs` factory anti-pattern was caught by user
   review, not by the trybuild fixture.** Step 01-06 originally
   introduced a `new_with_default_fs(...)` factory function as a
   convenience for the `serve` subcommand's wiring. The trybuild A1
   fixture proved `ExecDriver::new` rejected omission of `fs`, but it
   did NOT catch that a sibling factory restored the implicit-default
   behaviour. User flagged it (`feedback_delegate_to_architect`-style
   review) as structurally equivalent to a `Default` impl on a port
   trait, violating `.claude/rules/development.md` § "Port-trait
   dependencies" → "Builder-pattern overrides are an anti-pattern". The
   fix (commit `3c3672df`) threaded `Arc<dyn CgroupFs>` through
   `run_server`'s signature properly, deleting the factory in the same
   commit. The structural defense against recurrence is the section in
   `development.md` itself — the trybuild fixture catches the
   constructor signature, the rules document catches the surrounding
   API design.

4. **D1 equivalence proptest validated its own design — caught 4 real
   Sim semantic gaps on its first GREEN run.** Step 01-07 added a
   proptest that drives both `RealCgroupFs` (against `tempfile::TempDir`,
   not real cgroupfs — the proptest is a sim-vs-sim contract check, not
   a kernel-semantics check) and `SimCgroupFs` through the same randomly
   generated operation sequence, asserting observable equivalence
   through each adapter's own accessors. First run found 4 Dir-vs-File
   confusion cases in `SimCgroupFs` (`create_dir` on an existing file
   path, `write` to a directory path, `remove_dir` on a file path,
   `read` on a directory path) where the sim had been overly permissive
   relative to real VFS semantics. All four fixed inline in the same
   commit. The proptest IS the structural defense it was designed to be;
   shipping it without those four fixes would have left a future bug
   silently waiting in the equivalence claim.

5. **`SKIPPED` with `NOT_APPLICABLE` is the honest RED_UNIT disposition
   when the structural defense lives elsewhere.** Every DELIVER step
   logged `RED_UNIT: SKIPPED` with a free-text justification naming the
   actual structural defense (trybuild fixtures, AT-level scenarios,
   D1 equivalence proptest, F1 K3 determinism guard, real-kernel Tier
   3 scenarios). At this trait-surface layer the unit-level PBT surface
   genuinely is empty — there is no pure-logic decision that PBT could
   exercise separately from the trait contract. The discipline lets
   the DES integrity verifier track "every step that SHOULD have a
   RED_UNIT phase HAS one" without forcing fake unit tests onto a layer
   that doesn't want them.

---

## Test coverage shape

35 scenarios across 6 classes, distributed across the four-tier stack
per `.claude/rules/testing.md`:

| Class | Layer | Count | Notes |
|---|---|---|---|
| A | Compile-time | 2 | trybuild fixtures; `.stderr` baseline checked in |
| B | Tier 1 (DST surface) | 17 | SimCgroupFs unit-level at trait surface |
| C | Tier 3 (real kernel) | 7 | Run via `cargo xtask lima run --`; gated on `integration-tests` feature |
| D | Tier 1 equivalence | 1 (D1 proptest, 1024 cases) | Real vs Sim contract check on tempdir-backed `RealCgroupFs` |
| E1 | Triage | 12 (inline) | 8 → SimCgroupFs, 4 stay tempfile-backed against `RealCgroupFs` |
| F1 | DST K3 | 1 | Same seed → bit-identical SimCgroupFs trajectory |

**Mutation testing** was intentionally skipped at the orchestrator level
per user decision (single-step Phase 5 deferral logged in the DELIVER
audit log). The follow-up mutant-skip annotation (commit `22c94ada`)
addressed one untestable `!path.is_empty()` match guard in
`build_probe_adapter` — the `overdrive-cli` crate enforces `#![forbid(
unsafe_code)]` and `std::env::set_var` is `unsafe` in Rust 2024, so the
set-path branch is unreachable from a unit test. Integration tests
already cover it; the `// mutants: skip` annotation brings the testable-
surface kill rate to 100%.

**Test count post-cleanup**: 1467/1467 green in Lima. Leftover-cgroup
detection (per `.claude/rules/testing.md` § "Leaked workload cgroups
across runs") clean after the standard pre-merge sweep.

---

## Permanent artifacts

All architectural artifacts for this feature already live at their
permanent locations — Phase B of the standard nw-finalize migration
matrix is a **no-op** for this feature, by design:

| Artifact | Permanent location | Status |
|---|---|---|
| ADR-0054 | `docs/product/architecture/adr-0054-cgroup-fs-port.md` | Already permanent (initial + amendment landed in commits `88e7ded0` + `9602b0a3`) |
| brief.md changelog rows | `docs/product/architecture/brief.md` lines 3024–3025 | Already permanent (initial + amendment) |
| Acceptance scenarios | `crates/overdrive-{core,host,sim,worker,cli}/tests/` (Rust files) | Already permanent — the executable shape IS the spec post-DELIVER |
| Test-scenarios narrative | `docs/feature/cgroup-fs-port/distill/test-scenarios.md` | Workspace history — preserved per nw-finalize Phase C |

This project's lean v3.14 nWave layout (`feature-delta.md` instead of
the multi-file `discuss/`, `design/`, `distill/`, `deliver/` split)
collapses the migration matrix substantially: the design narrative
already merged into the ADR + brief.md at DESIGN time, and the test
narrative already projected into Rust source at DELIVER time. The
workspace files (`feature-delta.md`, `roadmap.json`, `execution-log.
json`, `distill/test-scenarios.md`) stay in place as the history of
how the feature was built; the ADR + brief.md changelog rows are the
SSOT for what the feature does.

---

## Links

- **Issue**: GH [#136 — Narrow CgroupFs port for fault-injection testability](https://github.com/marcus-sa/overdrive/issues/136)
- **ADR**: [ADR-0054 — Narrow `CgroupFs` port for fault-injection testability](../product/architecture/adr-0054-cgroup-fs-port.md)
  (amended 2026-05-24 — see § Production probe and § Alternatives → Alternative F)
- **Brief.md changelog**: `docs/product/architecture/brief.md` lines 3024–3025
- **Closest-precedent evolution**: [`2026-05-09-fix-cgroup-subtree-control-delegation.md`](2026-05-09-fix-cgroup-subtree-control-delegation.md)
  — the bug that motivated the testability gap this feature fills
- **Companion rule**: `.claude/rules/development.md` § "Port-trait dependencies"
  — required-not-defaulted, no builders, no `Default` impls on port traits
