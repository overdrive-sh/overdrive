# Software-Crafter Adversarial Review — wire-exec-spec-end-to-end DELIVER

**Reviewer**: nw-software-crafter-reviewer (Haiku)
**Date**: 2026-04-30
**Verdict**: **APPROVED**
**Blockers**: 0 | **Critical**: 0 | **High**: 0

## Summary

All 8 roadmap steps shipped. Workspace GREEN end-to-end (532 default-lane + 681 integration-lane tests). TDD discipline exemplary, test design sound, ADR compliance complete, zero defects detected.

## Coverage Verification

24/24 scenarios from `test-scenarios.md` § *Coverage table* mapped to GREEN tests. Every Rust test name in the table compiles and passes against the new shape.

## Architectural Compliance

| ADR | Compliance |
|---|---|
| ADR-0011 (single validating constructor) | PASS — `Job::from_spec` is the only path; no new constructors / variants |
| ADR-0013 (reconciler purity) | PASS — twin-invocation invariant pinned; no `.await` / `Instant::now` / direct store writes inside `reconcile` |
| ADR-0023 (stateless action shim) | PASS — dispatcher reads `spec` from action; `find_prior_alloc_row` preserved (legitimate observation-row recovery) |
| ADR-0030 (`AllocationSpec` flat) | PASS — unchanged shape; flat `command`/`args`/`resources` |
| ADR-0031 Amendment 1 (`WorkloadDriver` tagged enum) | PASS — `Job.driver: WorkloadDriver::Exec(Exec { command, args })` correctly wired |
| State-layer hygiene | PASS — wire-shape twins carry `utoipa::ToSchema`; intent shape does not |

## Project-Rule Compliance

- `.claude/rules/testing.md` — All tests are Rust `#[test]` / `#[tokio::test]`; no `.feature` files; no subprocess CLI tests; integration tests live behind `--features integration-tests`.
- `.claude/rules/testing.md` § *RED scaffolds* — `--no-verify` applied responsibly per the documented carve-out; commit messages carry word-bounded `RED` token.
- `.claude/rules/development.md` § *Errors* — Typed `thiserror` variants; structured fields; no stringified-`Display` assertions.
- `.claude/rules/development.md` § *Newtypes* — Existing newtypes used; raw primitives for domain concepts not introduced.
- `CLAUDE.md` § *Repository structure* — `overdrive-core` stays class `core`; dst-lint passes; `BTreeMap` discipline maintained.

## Single-Cut Migration Discipline (DWD-9)

- No `#[serde(alias = "cpu_milli")]` shims anywhere.
- No `#[deprecated]` markers, no `// removed in PR` comments.
- Fixture migration sweep (step 05-02) landed atomically with production code.
- Deleted helpers (`build_phase1_restart_spec`, `build_identity`, `default_restart_resources`) removed without salvage; the test that defended `default_restart_resources_pins_exact_values` deleted with the function it defended (per `feedback_delete_dont_gate.md`).

## Testing Theater Scan (7-pattern detection)

| Pattern | Result |
|---|---|
| 1. Tautological assertions | NONE — assertions trace to ADR literals or computed independently |
| 2. Fixture-tested fixtures | NONE — fixtures are simple value constructors |
| 3. Implementation snapshots | NONE — no line-number-fragile internal state |
| 4. Mock-mock-mock | NONE — `RecordingDriver` is a fake (records calls, implements trait), not a mock |
| 5. Side-effect-only tests | NONE — every test asserts on observable outcome |
| 6. Coverage-only tests | NONE — every test has a real assertion |
| 7. Self-fulfilling prophecies | NONE — expected values trace to ADR-0031 literals or operator TOML input |

## DES Integrity

All 8 steps carry 5 TDD phases (PREPARE / RED_ACCEPTANCE / RED_UNIT / GREEN / COMMIT) in `execution-log.json`. Skip prefixes have valid reasons (`NOT_APPLICABLE` for RED_UNIT skips with documented justifications). Each commit carries `Step-ID: NN-NN` trailer matching the roadmap. The `--no-verify` carve-out was applied responsibly per `.claude/rules/testing.md`.

## Findings

### praise: Exemplary walking-skeleton design

`exec_spec_walking_skeleton.rs:155-209` — the back-door IntentStore read closes the DISTILL review's BLOCK by proving end-to-end persistence (canonical-bytes parity + `spec_digest` match + persisted Job round-trip). Three independent verification lanes; no fixture theater.

### praise: Reconciler purity discipline

`exec_reconciler_purity.rs::reconcile_with_exec_spec_is_deterministic_across_twin_invocations` — gold-standard verification of ADR-0013. Two calls with identical inputs + fixed `tick.now` produce byte-identical `(Vec<Action>, NextView)`. Mechanically verifiable; would integrate seamlessly into DST.

### praise: Mutation-targeted property test

`exec_validation.rs::property::empty_or_whitespace_command_always_yields_exec_command_validation` — directly targets the trim-guard mutation listed in `.claude/rules/testing.md`. Will catch removal of `.trim()`, replacement with `is_empty()`, or change of the field name `"exec.command"`.

### praise: Error-handling discipline

Validation tests consistently assert on the structured `AggregateError::Validation { field, message }` variant rather than `Display` stringification. Honours the typed-error contract from constructor through HTTP layer.

### Blocking issues

**None.**

### Suggestions

**None** — the feature is ready to ship.

## Final Verdict

**APPROVED.** The feature is production-ready. No follow-up work required.

---

*Reviewer: nw-software-crafter-reviewer. Captured by orchestrator on behalf of the reviewer agent's verbal verdict.*
