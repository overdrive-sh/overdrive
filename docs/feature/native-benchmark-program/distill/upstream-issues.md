# Upstream issues — native-benchmark-program (DISTILL)

Gaps found while authoring, and their resolution. None was filled with invented API. All were resolved by the user ruling of 2026-11-17 and the DESIGN pins in `feature-delta.md` (DDD-5, DDD-6, DDD-7, DDD-9 and "Fault paths that have no nextest obligation").

- **G1 (pure internals unpinned): RESOLVED.** DDD-5 states the schedule, statistics, canary verdict, host probe and eligibility decision are internal structure, with unit and proptest coverage crafter-owned. No acceptance test pins them; the over-cap refusal test was removed (not a DDD-6 validate rule).
- **G2 (record nested shapes): RESOLVED by DDD-6.** The fixtures carry the pinned shapes, including `host.irq_time_accounting` and the `{ "absent": "<reason>" }` form; report wording is asserted as DDD-6 pins it.
- **G3 (comparison form): RESOLVED by DDD-4 / CA-10.** No command takes two records; the cross-substrate scenario and `Error::SubstrateMismatch` are dropped.
- **G4 (refusal names): RESOLVED by DDD-9.** `benchmark_family_invalid`, `benchmark_spike_invalid` (status 64) and the existing `invalid_mode`. Open follow-up for the orchestrator, not DISTILL: `testing.md`'s status-64 list gains the two names (DESIGN "Rule text the orchestrator edits", item d).
- **G5 (report production): RESOLVED by DDD-9.** The runner prints `OVERDRIVE_BENCH_RESULT_DIR=<repo-relative>` before the canary and writes `report.md` after the terminal line; observed in the retained runs (E-1), not in a test.
- **G6 (fault seams): RESOLVED by DESIGN.** Four no-obligation rulings, recorded with their reasons in `test-scenarios.md`; no test and no seam.
- **G7 (smoke handle): RESOLVED by DDD-2.** A test runs the generated `smoke_<name>`, never `smoke` on a chosen declaration; the smoke tests run it as a child process of the test binary.
- **G8 (absent pinned setting): RESOLVED by DDD-7.** A pinned setting with no source is refused with `found` = `absent`; the report line is asserted (R12), the host probe is crafter-owned unit coverage, and the real-host refusal is evidence run E-2.
- **G9 (diagnostic wording): RESOLVED by DDD-2 / DDD-6.** The macro messages and the `validate` message format are pinned and asserted; `.stderr` span lines are regenerated once.
- **G10 (dev-only gate): confirmed none.** No mechanical gate keeps `overdrive-testing` or the new crates dev-only; DDD-1 records that none is added.
- **D-1 (placement of the families tests): CLOSED by the user ruling.** No nextest test is `native_launcher`; the families crate keeps no test and the families nextest override is removed.
- **D-2 (long runs): CLOSED.** There is no long nextest run; the cohorts are retained `--benchmark` runs.
- **D-3: CLOSED** (G1, G2 above).

## Remaining notes for the orchestrator

- US-BENCH-04 acceptance criterion (b) ("cross-substrate comparison refused by name") still reads as before in the DISCUSS text; only the domain example and the scenario were reworded to match CA-10 (instruction: change only those lines). The product owner should drop AC (b).
- U1-U5 (runner usage refusals) are not nextest-testable without running the runner; they are crafter-owned unit tests of the argument parser and the error to exit-status mapping (internal structure). No acceptance test pins the exit statuses 64 / 78 / 70 of the runner binary; they are observed in evidence runs E-1 and E-2 only for 0 and 78.

## Final-review resolutions (2026-11-17)

- **Weak trybuild RED: recorded**, not removable (trybuild's panic is generic). Each test pends on its own `<n> of <n> tests failed`, the semantic reason was read from a `--no-capture` run, and the DELIVER activation note (regenerate `.stderr` with `TRYBUILD=overwrite`, verify the pinned `error:` lines unchanged) is in `red-classification.md` and in `compile_fail.rs`.
- **Substrate is a bare identifier (DDD-2/DDD-4):** `unknown_substrate.rs` now uses `guest_xyz`; `substrate_string_literal.rs` pins the string-literal diagnostic.
- **No-marker refusal (exit 78, no record): evidence, not a test.** No nextest test can reach the runner entry without running the benchmark binary, which the user ruling places in the evidence lane (DDD-9); `bench-report` takes a record and there is none. Scenario W12 / run E-8; the marker reader is crafter-owned unit coverage (G1). If the orchestrator wants a nextest test, it needs a test-support runner binary, which is a design decision (a new target), not something DISTILL adds.
- **Terminal-line wording:** DESIGN pins `bench-report: invalid record: line <n>: <reason>` and that the reason names the offending field or name where one exists; the line-after-the-terminal case asserts only the prefix and line number.
- **P2:** the former `< 500 ms` bound could not fail under `SimClock` (it never advances); the test now uses a stepping clock and asserts each service time is a positive whole number of steps.
