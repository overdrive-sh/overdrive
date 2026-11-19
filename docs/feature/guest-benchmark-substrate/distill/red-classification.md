# RED classification — guest-benchmark-substrate (DISTILL)

Runs on the native Metal host (`cargo xtask metal run -- cargo nextest run ... --features integration-tests`), kernel 7.0.0-38-generic, checkout state of 2026-11-19. Every pending test is `#[should_panic(expected = ...)]` with a full executable body: it passes only if its body panics with the named text, so a fixture or setup error fails the run. Class: MISSING_FUNCTIONALITY = correct RED (the assertion for the missing behaviour fired); the observed panic text of each was read from `--no-capture` runs (below).

73 tests: 66 RED, 7 GREEN guards (GR13, GM1, GV7, GP7, GP8, GK2, GW2). The six guest failure-handling tests (GF1-GF6) are guest-placed and run only with `integration-tests,kvm-tests`; the 67 others run with `integration-tests`. No IMPORT_ERROR, FIXTURE_BROKEN or SETUP_FAILURE. Every pending body panics on its own assertion, with one exception that is stated, not hidden: the **shared-control tests** (GV1-GV6, GP1-GP6, and GA6, GA7, GA10, GA11, GA17) reach only their control assertion today, because the scaffold cannot parse or accept the control and a refusal asserted against it would pass vacuously. Their per-rule assertions have never run; see "Activate together" below.

| Crate / binary | Tests | Result | Class |
|---|---|---|---|
| overdrive-bench (lib: runner, record, registry, replicate unit tests) | 12 runner + 5 record + 2 registry + 4 replicate (+ 108 existing) | all pass | 21 MISSING_FUNCTIONALITY; GR13 and GM1 GREEN |
| overdrive-bench (`integration`: validation, report, documents) | 7 validation + 8 report + 4 documents (+ existing) | all pass | 16 MISSING_FUNCTIONALITY; GV7, GP7, GP8 GREEN |
| overdrive-bench (`compile_fail`) | 3 new/split (+ 6 unchanged) | all pass | 3 MISSING_FUNCTIONALITY (weak RED, below) |
| overdrive-testing (lib: `guest_app`, `guest_kit`, `vm_fixture`) | 17 parser + 3 kit + 2 workspace root (+ 57 existing) | all pass | 20 MISSING_FUNCTIONALITY; GK2, GW2 GREEN |
| overdrive-testing (`integration`, `integration-tests,kvm-tests`: `guest_program_failures`, six `cgroup-default` `nested-launch` guests) | 6 (GF1-GF6) | all pass (GF1-GF5 `Summary [18.873s] 5 tests run: 5 passed`; with GF6 `Summary [22.805s] 6 tests run: 6 passed`) | 6 MISSING_FUNCTIONALITY, each against the inert `run_in_guest_program` scaffold |
| overdrive-bench-families | 0 | no tests | the macro-generated smoke test lands with the family's DELIVER step |

Summary (Metal run of 2026-11-19 after the end-of-DISTILL review fixes): `cargo xtask metal run -- cargo nextest run -p overdrive-bench -p overdrive-testing --features integration-tests --lib --test integration --test compile_fail` gave `Summary [27.367s] 309 tests run: 309 passed, 0 skipped`. `cargo check --workspace --all-targets --features integration-tests`: finished, exit 0. `cargo clippy --workspace --all-targets --features integration-tests -- -D warnings`: finished, exit 0. `cargo xtask placement-lint`: exit 0, no lines. (All three re-run after the review fixes.)

## Observed failures (the semantic RED of each)

| Test | Panic observed on the unimplemented scaffold |
|---|---|
| GR1 the_first_reported_guest_view_is_written_once... | `run_started must state substrate guest for a guest benchmark` (left `"native"`) |
| GR2 a_later_guest_view_writes_no_second_line... | `exactly one guest_observed line, later views write none` (left 0, right 1) |
| GR3 a_guest_replicate_that_returns_ok_without_a_view... | `a guest replicate that returned Ok without a view must be failed with the reported no guest view reason` (left `(Some("ok"), None)`) |
| GR4 a_view_reported_by_a_replicate_that_then_fails... | `exactly one guest_observed line` (0 lines) |
| GR5 a_replicate_that_fails_before_reporting... | `the guest_observed line comes right before the replicate that reported the view` (the failed replicate is followed by an ok replicate line) |
| GR6 a_drifted_host_never_calls_a_guest_benchmarks_body... | `a refused guest benchmark's run_started must carry the static guest block` (left `Null`) |
| GR7 a_native_benchmark_writes_no_guest_block... | `a native replicate that reports a guest view must fail` (left `"ok"`) |
| GR8 a_replicates_console_is_retained... | `every replicate's console must be written as an artifact in every outcome` (left 0, right 9) |
| GR9 retaining_a_console_twice... | `a second retain_console must fail the replicate` (left `"ok"`) |
| GR10 a_console_write_that_fails... | `a failed console write must fail the replicate` (left `"ok"`) |
| GR11 the_file_sink_writes_an_artifact... | `the file sink must write the artifact and its parents` (left `None`) |
| GR12 the_guest_record_ordering_holds... | `Test failed: guest record ordering invariant violated: ["0 guest_observed lines, but a line exists exactly when a replicate reported a view"]`; minimal input: nine `Report` steps |
| GR14 smoke_fails_a_guest_body_that_returns_ok_without_reporting_a_view | `a guest body that returns Ok without a view must fail its smoke run` (the control, a well-formed guest body, passed `smoke`; the broken body passed it too) |
| GR15 smoke_fails_a_guest_body_that_retains_its_console_twice | `a second retain_console must fail its smoke run` (control passed) |
| GR16 smoke_fails_a_native_body_that_reports_a_guest_view | `a view reported by a native benchmark must fail its smoke run` (control passed) |
| GM2 | `Test failed: assertion failed: (left == right) left: Err(GuestMemoryMibError) right: Ok(GuestMemoryMib(1)): the text of a size must parse back to the size`; minimal input `GuestMemoryMib(1)` |
| GM3 | `left: "0", right: "1": the record carries the size as the JSON integer of its digits`; minimal input `GuestMemoryMib(1)` |
| GM4 | `assertion left == right failed: the largest count parses` (left `Err(GuestMemoryMibError)`, right `Ok("4294967295")`); the control runs before the refusal table |
| GM5 | `assertion left == right failed: a positive JSON integer deserialises to its size` (left `None`, right `Some("512")`); the control runs before the refusal table |
| GS1 | `guest must be a registered substrate` |
| GS2 | `assertion left == right failed` with right `...the registered substrates are: native, guest` (the registry still lists `native` only) |
| GV1-GV6 (shared control) | `the valid guest record must validate: status Some(65): bench-report: invalid record: line 1: clock.guest \`CLOCK_MONOTONIC (in-guest Instant, 1 ns)\` is not null or \`CLOCK_MONOTONIC (overdrive-init Instant::elapsed, 1 ms)\``. The first refusal today is the guest clock string; the validator also refuses the substrate and the `guest` block behind it. Each case is the valid control with one edit, so after DELIVER each test exercises its own rule. Their per-rule refusal assertions have not run (see "Activate together"). |
| GP1-GP6 (shared control) | `the report must render for a valid record; stderr: ... clock.guest ... is not null or ...` (same first refusal) Their per-rule assertions have not run (see "Activate together"). |
| GD1 | `the methodology must have a \`## Guest substrate\` section` |
| GD2 | `the version policy must say an addition for a new substrate does not bump the version` |
| GD3 | `the catalogue must have a row for the guest family` |
| GD4 | `the catalogue must state that a family that launches VMs is not a guest benchmark` |
| GA1, GA2, GA3, GA4, GA5 | `<control> must parse: BlockMissing` (the scaffold parser finds no block in any console; the consoles are printed by the scaffold `print_program_return`, which prints nothing) |
| GA6, GA7, GA10, GA11, GA17 (shared control) | `the unglued control must parse: BlockMissing` / `the control console must parse: BlockMissing` (a refusal test cannot pass vacuously against the always-`Err` scaffold; their refusal assertions have not run) |
| GA8 | `two blocks must be BlockRepeated` (left `Err(BlockMissing)`) |
| GA9 | `a missing reading must name its key (kernel)` (left `Err(BlockMissing)`) |
| GA12 | `a short or long block must be a count mismatch` (left `BlockMissing`, right `CountMismatch { expected: 5000, found: 4999 }`) |
| GA13 | `a changed value must be a checksum mismatch` (left `Err(BlockMissing)`) |
| GA14 | `ReadingMissing must name its key` (the scaffold `Display` is `RED scaffold: ProgramReturnError display`) |
| GA15 | `Test failed: what was printed must parse back: Err(BlockMissing)`; minimal input `values = [997745002]` (a single value, 64 cases, seed printed on failure) |
| GA16 | `Test failed: a damaged block must never parse to a different series (flip): Err(BlockMissing)`; minimal input `values = [1, 97737]` |
| GK1 | `the benchmark guest band must be registered` |
| GK3 | `assertion left == right failed: the benchmark image must be production init with both modules and no extra files` (left `loopback_module: false`, right `loopback_module: true`) |
| GW1 | `workspace_root must use the exported source root in the exclusive class` (left: the cargo-located checkout, right `/nonexistent/overdrive-exported-source-root`) |

| GF1 a_guest_that_sends_no_ready_is_a_failed_run... | `a guest that never sent READY must not report an exit status, got Ok(0)` |
| GF2 a_guest_stalled_past_the_ready_bound... | `a guest stalled past the READY bound must not report an exit status, got Ok(0)` |
| GF3 a_guest_that_dies_after_ready... | `a guest that died after READY must not report an exit status, got Ok(0)` |
| GF4 a_program_that_misses_the_exit_bound... | `a program that misses the exit bound must not report an exit status, got Ok(0)` |
| GF6 a_vmm_that_outlives_its_guests_exit... | `a VMM that outlives its guest's EXIT must not report an exit status, got Ok(0)` (observed on Metal, run `16560e285cdf01de7be8da07`, `guest_program_failures.rs:452`; the inert scaffold never boots, so the first assertion fires) |
| GF5 a_program_that_exits_within_the_bound... | `the console is returned with what the program wrote; console:` (empty: the inert scaffold's `Ok(0)` passes the exit assertion, so the control reaches the console assertion) |

The GF panic texts were read by running the five with each `expected` deliberately prefixed `ZZ ` (nextest printed `panic message: ...` for each) and restoring the file.

## Real-guest harness proof (GF1-GF6)

The five bodies were also run once against a **throwaway** implementation of `run_in_guest_program` (a copy of the in-guest lane's beacon exchange taking `program` as the operator command, killing the VMM on any failure) and `benchmark()` patched to carry the loopback module. All five bodies ran to their end without a panic (each test then failed on `should_panic` "did not panic as expected", which is the GREEN signal that DELIVER's removal of the marker is expected to turn into a pass): the watcher found the VMM, froze it (GF2, 66.8 s, the whole `READY_BOUND`), killed it with the program running (GF3, 8.1 s), the refusing image sent no READY (GF1, 8.2 s), the exit bound was missed (GF4, 18.4 s) and the control exited 0 (GF5, 9.4 s); in every case the VMM was reaped (no holder, no `/proc/<pid>`) and a clean run on the same CID reached `Ok(0)`. The throwaway was then reverted; the committed scaffold is inert. One finding shaped the tests: the console of a guest that sent no READY is whatever the VMM had written when the run ended (the throwaway returned early and the refusal line was not yet in the file), so GF1 asserts only the kernel banner, which is what the contract promises (`the console as it stands when the run ends`).

**GF6 harness proof (2026-11-19, Metal, kernel 7.0.0-38-generic).** GF6 was proven the same way, separately: a throwaway `run_in_guest_program` (the in-guest lane's beacon exchange with `program` as the operator command, a 30 s `wait_exit` after EXIT turning a VMM that has not exited into `exit = Err`, kill and reap in every outcome) and `benchmark()` patched to carry the loopback module, with the `should_panic` marker removed. The body ran to its end without a panic (`PASS [39.300s]`: about 9 s to boot, run the program and read EXIT, then the 30 s post-EXIT wait), so the scripted init delivered EXIT, left the VMM running, and the console held the program's `bind: bound` line and the scripted init's EXIT line; the VMM was reaped and the CID reused for a clean run. The throwaway was reverted. The seam is `crates/overdrive-testing/bin/guest_peer.rs` `run_scripted_init` (PID 1 is the peer; when the image stages the peer at the production init's path it execs itself with that argv[0] and plays the guest half of the beacon without powering off) and `SCRIPTED_INIT_EXIT_LINE` in `guest_app`; it needs `cargo xtask guest-kit-prepare` because the peer binary changed.

**DELIVER activation (GF1-GF6).** Land `run_in_guest_program` (and make `GuestImageSpec::benchmark()` the pinned literal: it must carry the loopback module, or every program exits 65), then remove the six `#[should_panic(...)]` markers. The bodies need nothing else. Run: `cargo xtask metal run -- cargo nextest run -p overdrive-testing --features integration-tests,kvm-tests -E 'test(guest_program_failures)'`; the file takes about 2.5 minutes (GF2 alone waits out `READY_BOUND`). The overdrive-testing `integration` binary is in the `host-kernel-shared` group with a 120 s x 6 slow-timeout, which covers it.

## Weak RED: the trybuild tests

trybuild's panic carries no per-fixture text, so `should_panic(expected = ...)` can only name the summary (`1 of 1 tests failed`), which excludes a project that could not be prepared and a partial failure but would also be satisfied by a fixture failing for a wrong reason. The semantic reason of each was therefore read from a `--no-capture` run:

- GT1 `unknown_substrate.rs`: `mismatch`: the actual output is the old text (`the registered substrates are: native`), the expected output (the committed `.stderr`) is the pinned `native, guest`. The only difference is the message.
- GT2 `guest_substrate_accepted.rs`: `error: guest clock source \`guest_monotonic\` is not one of init_elapsed_ms`. The attribute stops at the clock source before it reaches the substrate constant evaluation, so the substrate refusal (`E0080`, "guest substrates are not available") sits behind it; both are missing functionality. After DELIVER the fixture must build whole.
- GT3 `unknown_guest_clock_source.rs`: `mismatch`: actual `is not one of init_elapsed_ms`, expected `is not one of init_elapsed_ms, guest_monotonic`.

**DELIVER activation note (trybuild).** In the step that lands the registry and the clock source, remove the `should_panic` of GT1, GT2 and GT3. The two `.stderr` files are already written to the pinned messages; verify with `git diff` that no pinned `error:` line changed, and regenerate only the span lines with `TRYBUILD=overwrite` if rustc renders them differently. GT2 is the weak one: its RED is a fixture that fails to build for the missing clock source, so its activation is "remove the marker; the fixture must build whole, with no `.stderr` file written or changed" (a `.stderr` for it would mean the build still refuses something). Do not accept a GT2 that passes by a different refusal.

## Activate together (the shared-control tests)

Seventeen tests share one control precondition and so share one RED, which means their per-rule assertions have **never executed**. This is a deliberate, stated limit, not a defect found: the refusals they assert are named by substrings (`guest`, `guest_observed`, `clock.guest`, any error) that today's validator and the always-`Err` scaffold parser already satisfy, so a refusal test without a control would pass vacuously and a restructure to native-record variants would too. Each test therefore first proves a valid control is accepted; until the implementation accepts it, the control is the RED.

| Group | Tests | Control | First DELIVER step that unblocks the group |
|---|---|---|---|
| Record validation | GV1-GV6 | a valid guest record validates (`bench-report validate`) | the step that makes the validator accept a guest record: `Substrate::Guest`, the `guest` block, `guest_observed`, `clock.guest` = `CLOCK_MONOTONIC (in-guest Instant, 1 ns)` |
| Report | GP1-GP6 | a valid guest record renders (`bench-report render` validates first) | the same step, plus the step that prints the guest lines; GP1-GP6 cannot reach their own assertions before the validator step lands |
| Console return | GA6, GA7, GA10, GA11, GA17 | a printed control console parses | the step that lands `print_program_return` and `parse_program_return` far enough to parse an intact console |

**Instruction to the crafter.** The first step of each group makes the control GREEN and exposes the group's per-rule assertions for the first time. On that step, remove the `should_panic` marker of every test in the group, run each body once against the minimal implementation, and expect the semantic RED of each rule that the step has not yet implemented. A body that fails for a reason that is not a missing behaviour (a wrong needle, a wrong line number, an edit that does not break the rule it names) is a **test defect: route it back to DISTILL**; do not weaken or rewrite the body. Only then continue one test at a time.

## Notes for DELIVER

- **Scaffolds and where they bite.** `Replicate::report_guest_view`, `Replicate::retain_console` and `FileSink::write_artifact` are inert (`Ok(())`, nothing recorded or written) so each RED fails at the behaviour it asserts; `GuestMemoryMib`'s `FromStr` refuses every text, its `Serialize` writes the fixed wrong value `0` and its `Deserialize` refuses every value, `GuestImageSpec::benchmark()` is `workload()`, `print_program_return` prints nothing, `parse_program_return` returns `Err(BlockMissing)`, so every scaffold is inert and no test depends on a scaffold's panic text; `GUEST_BENCHMARK` exists but is not in `REGISTERED`. Every scaffold carries `SCAFFOLD: true`; `grep -rn "SCAFFOLD: true" crates/` lists them.
- **Existing tests this change replaced** are listed in `test-scenarios.md`; the guest-record validator tests start from the valid control, so the order the crafter implements them in is free.
- **GR12 and GA15/GA16 are proptests** with `failure_persistence: None` (no regressions file is written under should_panic). Seed and shrunk input print on failure.
- **GP8 compares against the committed `report.md` files** and passes today; if the renderer changes a native report byte, it fails, which is the point.
- **Native untouched:** `every_retained_record_still_validates` (existing), GP7, GP8 and GR7's native assertions are the K4 guard.
