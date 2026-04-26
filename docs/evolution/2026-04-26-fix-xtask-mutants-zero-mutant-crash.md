# Evolution: fix-xtask-mutants-zero-mutant-crash

**Date**: 2026-04-26
**Branch**: `marcus-sa/phase-1-control-plane-core`
**Wave shape**: bugfix (RCA → /nw-deliver, single-phase 2-step roadmap)
**Status**: shipped, all DES phases EXECUTED/SKIPPED with PASS verdict

---

## Defect

`cargo xtask mutants --diff origin/main --package <pkg> --file <path>`
crashed when the filter intersection (`--in-diff` ∩ `--file` ∩
`--package`) produced zero candidate mutants. cargo-mutants logs
`INFO No mutants to filter`, exits 0, and skips creating
`target/xtask/mutants.out/outcomes.json`. The xtask wrapper then
unconditionally tried to parse the absent file and aborted with
`bail!("no outcomes.json … — cargo-mutants did not produce a report")`.

The kill-rate gate is vacuously satisfied in this state — there is
nothing to evaluate — so the wrapper should exit 0 with a "no mutants
in scope" annotation rather than crash. The bug surfaced during inner
loop per-step DELIVER discipline (`.claude/rules/testing.md` § "Per-step
vs per-PR scoping"), which actively encourages narrow `--file` scopes
and therefore makes empty filter intersections the common shape.

## Business context

Per-step mutation runs are part of the project's TDD inner loop. A
crash on the common "no mutants in scope" shape blocked agents and
operators reflexively from running scoped checks; the only escape was
to widen the scope or skip the gate entirely. Both options eroded
the discipline `.claude/rules/testing.md` is meant to enforce. The
fix preserves the kill-rate gate at full strength while making the
vacuous-pass path first-class.

## Root cause (three independent branches, all required)

1. **Wrapper-side parsing assumption (Branch A)**. `parse_outcomes`
   bails on absent file. The wrapper conflated two file-absence
   semantics — "cargo-mutants crashed" vs "cargo-mutants successfully
   short-circuited" — into one error.
2. **No positive success signal (Branch B)**. cargo-mutants does not
   write a stub `outcomes.json` on the empty-filter short-circuit; it
   exits 0 with only a stdout log line. The wrapper had no
   machine-readable channel to discriminate clean no-op from crash.
3. **Test coverage gap (Branch C)**. All gate tests constructed
   `RawReport` in-memory; none routed through `parse_outcomes` against
   an absent file. The wrapper-↔-cargo-mutants contract lived only in
   inline comments, never as an executable assertion.

Cross-validation: all three branches are required for the crash to
manifest. Without B the file would always exist; without A the
absence would be handled; without C either A or B would have been
caught pre-merge. Two prior runs in the same session (90.9%, 100.0%
kill-rate against non-empty filters) succeeded because they never
reached the short-circuit.

## Decision

**Add a zero-mutant-no-report branch to `run()` rather than rely on
upstream cargo-mutants behaviour.** The fix is wrapper-local and
testable; depending on a future cargo-mutants PR to write a stub
`outcomes.json` would tie our gate to a 3rd-party version we cannot
control.

Implementation shape:

- Capture the `ExitStatus` from `invoke_cargo_mutants` (already typed
  `Result<ExitStatus>` at the existing call site).
- Add `read_outcomes_or_short_circuit(path, ExitStatus) ->
  Result<Option<RawReport>>` — `Some(report)` on the populated path,
  `None` on `exit==0 && file absent`, hard-error on
  `exit!=0 && file absent` ("subprocess likely crashed").
- Add `finalise_zero_mutant_run(path, mode)` which writes a
  `mutants-summary.json` with `total_mutants=0`, `caught=0`,
  `missed=0`, `status="pass"`, `reason="no mutants in scope"`.
- Route `run()` through both helpers; existing
  `evaluate_diff_gate` / `evaluate_workspace_gate` paths untouched.
- Synthetic `RawReport` for `total_mutants==0` rides the existing
  `kill_rate_percent` 100% path — no new gate logic.

Crash-detection preserved: absent file + non-zero exit still bails
with "subprocess likely crashed".

## Steps completed (per execution-log.json)

| Step | Phase | Status | Outcome |
|---|---|---|---|
| 01-01 | PREPARE | EXECUTED | PASS |
| 01-01 | RED_ACCEPTANCE | SKIPPED | bug-fix scope; no acceptance test layer for xtask wrapper |
| 01-01 | RED_UNIT | EXECUTED | PASS — regression test compiles against missing function (RED signal) |
| 01-01 | GREEN | SKIPPED | RED scaffold step; GREEN belongs to 01-02 |
| 01-01 | COMMIT | EXECUTED | PASS — `6de0be8` test(xtask): regression test for empty-filter zero-mutant short-circuit (RED) |
| 01-02 | PREPARE | EXECUTED | PASS |
| 01-02 | RED_ACCEPTANCE | SKIPPED | bug-fix scope |
| 01-02 | RED_UNIT | SKIPPED | regression test already authored in 01-01 |
| 01-02 | GREEN | EXECUTED | PASS — `read_outcomes_or_short_circuit` + `finalise_zero_mutant_run` introduced; four unit tests added |
| 01-02 | COMMIT | EXECUTED | PASS — `75454da` fix(xtask): handle cargo-mutants empty-filter short-circuit gracefully (GREEN) |

Phase 3 refactor and Phase 5 mutation testing are deliberately out of
scope per `roadmap.json` exclusions: bug-fix discipline is minimal
edits, and `xtask` is excluded from `cargo-mutants` per project
CLAUDE.md ("xtask excluded from mutation testing" memory).

## Lessons learned

1. **Subprocess success is a non-binary state.** "exited 0" and
   "wrote a file" must be tracked as independent dimensions when the
   subprocess can legitimately produce one without the other. Future
   xtask wrappers around third-party CLIs should make this explicit
   from day one rather than retrofitting after a crash.
2. **Inline-comment contracts rot silently.** Both root-cause
   branches A and C trace back to assumptions documented only as
   inline comments at the call site. The fix turns the contract into
   four executable unit tests that fail loudly when the assumption
   breaks.
3. **Per-step scoping shifts edge cases to the centre.** The
   project's per-step DELIVER discipline makes narrow `--file`
   filters the common shape. Edge cases under wide-scope invocation
   (`--workspace`) become the dominant shape under per-step
   invocation; design for the per-step shape first.
4. **Vacuous-pass is a first-class verdict.** A gate that crashes on
   "nothing to evaluate" is indistinguishable from a gate that
   crashed on real input. Writing
   `target/xtask/mutants-summary.json` with `total_mutants=0` makes
   the vacuous-pass observable to downstream consumers (CI step
   summaries, future tooling).

## Documentation updates

`.claude/rules/testing.md` § "Mutation testing" gained an "Empty
filter intersection is a vacuous pass" subsection codifying the
behaviour, so future maintainers do not re-derive the contract from
the implementation.

## Commits (chronological)

- `6de0be8` — test(xtask): add regression test for empty-filter
  zero-mutant short-circuit (RED)
- `75454da` — fix(xtask): handle cargo-mutants empty-filter
  short-circuit gracefully (GREEN)
- `7361285` — docs(rules,xtask): document empty-filter vacuous-pass +
  archive bugfix RCA

## Migrated artifacts

- `docs/research/fix-xtask-mutants-zero-mutant-crash-rca.md` — full
  5-Whys multi-causal RCA (Branches A/B/C, contributing factors,
  exact before/after snippets, change list 1.1–1.4) preserved for
  reference.
