# RED classification — native-benchmark-program (DISTILL, revised 2026-11-17 after the final review and the final DESIGN alignment)

Runs on the native test host (`cargo xtask metal run -- cargo nextest run ... --features integration-tests`), kernel 7.0.0-38-generic. Every pending test is `#[should_panic(expected = ...)]`; it passes only if its body panics with the named text, so a fixture or setup error fails the run instead of passing. Class: MISSING_FUNCTIONALITY = correct RED (the assertion for the missing behaviour fired). No nextest test runs the runner over a cohort (user ruling 2026-11-17); the families crate has no hand-written test.

| Crate / binary | Tests | Result | Class |
|---|---|---|---|
| overdrive-bench (`integration` and `compile_fail`) | 60 (report 26, validate 24, smoke 3, docs 2, trybuild 5) | 60 passed | 58 MISSING_FUNCTIONALITY; V1 `a_wellformed_record_validates...` and V17 `the_histogram_shape_the_malformed_cases_start_from_is_valid` GREEN (vacuous against the scaffold, guard an over-strict validator) |
| overdrive-bench-families | 0 | no tests | the macro-generated smoke tests land with each family's DELIVER step |
| overdrive-control-plane (`dns_query_observer`) | 3 | 3 passed | P1, P2 MISSING_FUNCTIONALITY (observer never called); P3 GREEN regression lock (byte-identical replies) |
| overdrive-dataplane (`an_outbound_connection_names_its_pump_threads_by_role`) | 1 | passed | MISSING_FUNCTIONALITY (pump threads inherit the runtime worker name) |
| xtask (`benchmark_form`) | 6 | 6 passed | MISSING_FUNCTIONALITY (see "Pre-flag rejection" below) |

Summaries (pasted from the Metal runs of 2026-11-17): `Summary [2.993s] 60 tests run: 60 passed, 0 skipped` (overdrive-bench, after the DESIGN alignment: R14 histogram pin, R12c failed probe, V17-V24 histogram rules, per-family smoke probes, clippy fixes); earlier runs, unchanged and not re-run: `Summary [18.882s] 3 tests run: 3 passed, 1288 skipped` (control-plane `dns_query_observer`); `Summary [6.922s] 1 test run: 1 passed, 239 skipped` (dataplane pump names; only a comment changed since); `Summary [0.893s] 6 tests run: 6 passed, 429 skipped` (xtask `benchmark_form`). Compile-check `cargo check --workspace --all-targets --features integration-tests`: finished, exit 0. `cargo xtask placement-lint`: exit 0, no lines. Clippy (`-p overdrive-bench -p overdrive-bench-families -p overdrive-testing-macros -p overdrive-control-plane -p overdrive-dataplane --all-targets --features integration-tests`): 0 warnings.

Semantic RED reasons: the report tests fail at the first assertion of the missing report (`the report must have a section for \`Table: t\`` or the named header line); the validate tests, including the eight histogram-rule tests (V17 is the valid control), at their own named refusal (`validate must refuse a record with ...`; the scaffold exits 0); the smoke tests at `the generated smoke test must ...` (the macro emits no `smoke_<name>`, so the child runs no test and reports no replicate or panic message); the DNS tests at the observer never being called.

## Weak RED: the trybuild tests

trybuild's panic carries no per-fixture text, so no `should_panic(expected = ...)` can name the semantic reason. Each of the five tests pends on trybuild's summary with its own fixture count (`2 of 2 tests failed`, `1 of 1`, `2 of 2`, `3 of 3`, `5 of 5`), which excludes trybuild's bare `tests failed` (the project could not be prepared: a wrong RED) and a partial failure, but it would also be satisfied by a fixture that fails to compile for a wrong reason. The semantic reason was therefore read from a `--no-capture` run: all 13 fixtures report `Expected test case to fail to compile, but it succeeded.` against the passthrough attribute (none fails for a compile error of its own). This is a weaker RED than the others and is recorded as such.

**DELIVER activation note (trybuild).** In the step that lands the attribute, remove the `should_panic` of each trybuild test, regenerate the `.stderr` files once with `TRYBUILD=overwrite`, and verify with `git diff` that every pinned `error:` message line is unchanged (only the `-->`, source and label lines may differ); a changed `error:` line is a contract change, not a regeneration. `an_unregistered_or_guest_substrate...` and `a_substrate_written_as_a_string_literal...` pin the bare-identifier rule: an unregistered bare identifier is an `E0080` constant-evaluation error, a string literal is a macro `error:` line.

## Pre-flag rejection (W7, W8)

W7 and W8 pend on `unknown arg: --benchmark`: their RED is the script rejecting the not-yet-existing `--benchmark` flag before any form logic runs. That is a correct RED for the missing flag, but not for the closed-argv behaviour the tests assert; the argv and exclusive-class assertions are exercised for the first time when the flag lands, so the crafter confirms those two assertions with the pending marker removed, not just that the flag is accepted. X1a-c fail at their own named refusal (no `benchmark_*_invalid` or `invalid_mode` name is printed for the malformed requests today), and X2 at the missing clap conflict.

Not a wrong RED: no IMPORT_ERROR, FIXTURE_BROKEN or SETUP_FAILURE. The guest-placed tests need `cargo xtask bpf-build` and `guest-kit-prepare` first (documented prerequisites).

Caveats DELIVER must keep: V1 and P3 stay GREEN and must stay green. The `.stderr` files carry the pinned `error:` lines; the span lines are the expected rustc rendering. W9 and W10 can only pass once the generated smoke test runs the body (their children announce each replicate on the test output).

## DELIVER activation notes (final alignment)

- **R14 and the fixture encoder.** `support.rs` bucketing reads the `f64` exponent from the bits (the `frexp` exponent minus 1; bucket 0 below 1; bucket 2048 from 2^64), and `histogram_percentile` follows the pinned order: bucket interpolation, then the clamp to the stored min and max, then R-7. R14 asserts the report cell is byte-equal to that computation, so the report renderer must keep that order and `f64` expression order.
- **V18-V24 wording.** The tests pin the line number and the offending field name (`buckets`, `histogram`, `min`, `max`); the rest of the reason is not pinned. V19 assumes the ascending rule is checked before the min/max range rules (the DDD-6 listing order).
- **P4 (`an_outbound_connection_names_its_pump_threads_by_role`).** Production `teardown` joins both pump threads before it returns, so the after-teardown assertion stays; the note in the test says to poll with a bound only if it is ever seen to fail with the pumps already joined. No seam is added.
