# Acceptance-Designer Review — wire-exec-spec-end-to-end DELIVER Roadmap

**Reviewer**: nw-acceptance-designer-reviewer (Haiku)
**Date**: 2026-04-30
**Artifact reviewed**: `docs/feature/wire-exec-spec-end-to-end/deliver/roadmap.json`
**Verdict**: **APPROVED**

## Summary

The DELIVER-wave roadmap correctly implements all 24 test scenarios from
DISTILL's `test-scenarios.md` Coverage table, with proper scenario-to-step
assignment. The critical blocker from the DISTILL review — missing
IntentStore persistence assertion in the walking-skeleton test — is
explicitly addressed in step 05-01's acceptance criteria. ADR-0031
Amendment 1 (nested `Job.driver: WorkloadDriver` instead of flat
`command`/`args`) is correctly threaded through all five phases.
Project-canonical Rust testing discipline is honoured: no `.feature`
files, no subprocess CLI tests, RED scaffolds documented, and single-cut
fixture migration per DWD-9. The walking-skeleton placement in Phase 05
is architecturally justified by the data-flow dependency chain. All 13
items in the DWD-16 BROKEN file list are accounted for and scheduled for
fixture migration in step 05-02.

## Findings

### praise: Complete DISTILL coverage with explicit Amendment-1 threading

Every scenario named in `test-scenarios.md` § *Coverage table* is assigned
to exactly one roadmap step. The roadmap's `amendments` section
explicitly enumerates which step ACs were rewritten for ADR-0031
Amendment 1 (steps 01-01, 02-01, 05-01) and which ones stay unchanged
(steps 01-02, 04-02). Spot-check of 5 step ACs confirms the nested
`WorkloadDriver::Exec(Exec { command, args })` form throughout — no
flat-form regressions.

### praise: DISTILL BLOCK-fix is explicit in step 05-01

Step 05-01's AC includes the back-door IntentStore read pattern
verbatim: "deserialise rkyv at `jobs/payments` → destructure
`let WorkloadDriver::Exec(exec) = &job.driver` → assert
`exec.command == "/opt/payments/bin/payments-server"` and
`exec.args == vec!["--port", "8080"]`." This closes the BLOCK from
the prior DISTILL review.

### praise: Single-cut migration discipline encoded inline

Step 05-02's AC explicitly forbids `#[serde(alias = "cpu_milli")]`
shims, two-shape acceptance periods, and "migrate one fixture per PR"
gradualism — citing both DWD-9 and `feedback_single_cut_greenfield_migrations.md`.
The step also documents the `git commit --no-verify` exception with
its reason (per the documented `.claude/rules/testing.md` carve-out).

### praise: Walking-skeleton placement is defensible

Standard nWave practice puts the walking-skeleton in early phases, but
this feature's WS is fundamentally end-to-end (CLI handler → real HTTP
→ server-side validating constructor → IntentStore put → reconciler →
SimDriver). The architect placed it in step 05-01 with
`dependencies: ["03-01"]` — exactly when the bones are connected.
DISTILL `walking-skeleton.md` Strategy C explicitly notes this
dependency. Acceptable as-is.

### praise: Project-rule overrides explicitly honoured

No step requires a `.feature` file, cucumber-rs, pytest-bdd, or
subprocess-based CLI test. DWD-1 / DWD-3 are referenced inline; the
WS test in step 05-01 explicitly notes the call-as-Rust-function
pattern per `crates/overdrive-cli/CLAUDE.md` § *Integration tests —
no subprocess*.

### Blocking issues

**None.**

### Suggestions / nitpicks

**None.** Sizing of step 05-02 (~14 files) is appropriate for a
mechanical fixture sweep per DWD-9; the `@sizing-review-needed` flag
does not apply since the count reflects file-touches, not independent
scenarios.

## Verdict

**APPROVED.** No blocking issues. Crafter dispatch may proceed.

---

*Reviewer: nw-acceptance-designer-reviewer. Captured by orchestrator on
behalf of the reviewer agent's verbal verdict.*
