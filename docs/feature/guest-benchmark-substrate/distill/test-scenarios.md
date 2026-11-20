# Test scenarios — guest-benchmark-substrate (DISTILL)

Specification only; nothing here is parsed. Executable bodies are Rust tests (`.claude/rules/testing.md`: no `.feature` files). Each row: scenario | traces | driving port | production owner path | fault stimulus | outcome oracle | contract shape | lane | state. Every test body carries its contract shape as a `CONTRACT_SHAPE:` doc line (precedent: `overdrive-bench` and `overdrive-testing`). Lanes: **unit** (in-crate `#[cfg(test)]`, in-memory sink and canary, layers 1-2), **int-host** (integration binary of `overdrive-bench`, subprocess of `bench-report` or document read, host-pure), **trybuild** (compile-fail, host-pure). "RED" = pending as `#[should_panic(expected = ...)]` with a full executable body; "GREEN" = passes now and guards a contract that must stay. All 67 tests are host-pure by the crate defaults (`overdrive_test_default!(host_pure)` in both lib test configurations and in the `overdrive-bench` integration binary; trybuild tests carry `#[overdrive_test(host_pure)]`); none boots a guest, none is `native_launcher`, none spawns the `overdrive` binary or the benchmark binary.

**What is not a test.** The design places three things in retained Metal evidence, not nextest (`feature-delta.md` DDD-6, DDD-8, G-2 "Evidence lane", V-C, V-D; `testing.md` § "Classify external execution"): real-guest boot, the in-guest program, and the family's host half over a real guest. `run_in_guest_program`'s failure handling is the exception: it is a nextest regression file in real guests (GF1-GF6). The rest are listed in [Benchmark-run evidence](#benchmark-run-evidence-deliver-retained-not-tests). The guest family's own smoke test is macro-generated from its declaration, so it is not authored here (see "Smoke variant" below).

**Existing tests this change replaces.** `registry::tests::native_is_the_one_registered_substrate` and `the_refusal_names_the_substrate_and_the_registered_ones` (now `native_and_guest_are_the_registered_substrates` and the two-entry message), `registry::tests::guest_gets_its_own_refusal` (deleted: the refusal is removed by ADR-0325), `compile_fail::an_unregistered_or_guest_substrate_fails_the_build_naming_it` (split into the unregistered-substrate refusal and the acceptance of a guest declaration; `compile_fail/guest_substrate.rs` and its `.stderr` deleted), `compile_fail::an_unknown_clock_source_fails_the_build_naming_the_set` (split: service clock unchanged, guest clock set now `init_elapsed_ms, guest_monotonic`), and the `("substrate", "guest")` case of `a_value_outside_a_closed_vocabulary_is_refused` (now an unregistered `hypervisor`). Each was written for the behaviour this feature removes; leaving them would make DELIVER weaken or rewrite an acceptance body.

## Walking skeleton (slices 01 and 02)

The skeleton's end-to-end run (the benchmark form booting a production-`overdrive-init` guest on the exclusive Metal host and returning a record) is evidence runs E-G1 and E-G2 below. Its nextest side is the runner (GR1-GR12), the record and report contracts (GV, GP) and the parser (GA): every contract the host half and the runner rely on, driven without a guest.

## Runner over a guest benchmark (`crates/overdrive-bench/src/runner.rs`, `mod tests`)

The runner is driven with a declaration whose `substrate` is `"guest"` and a scripted body (the family's host half replaced by a per-replicate script: report a view, fail before or after reporting, return `Ok` without a view). Production owner path: `Runner::run` (the same wiring order as a native run). Driving port: `run_main`'s `Runner`, through its ports (in-memory `RecordSink`, canary and host probe). Lane: unit. Shape: bounded-change (the record the runner appends).

| # | Test | Traces | Fault | Oracle | State |
|---|---|---|---|---|---|
| GR1 | the_first_reported_guest_view_is_written_once_before_that_replicates_line | US-GBS-01 AC (a), (b); US-GBS-02 AC (b); boundary 1; DDD-2 | none | `run_started` says substrate `guest` and carries the static guest block; line 3 is the one `guest_observed` line holding the reported view, line 4 the first replicate; the record validates | RED (`run_started must state substrate guest`: the runner still writes `native`) |
| GR2 | a_later_guest_view_writes_no_second_line_and_changes_nothing | DDD-4; ADR-0328 | two later replicates report a different kernel | exactly one line, holding the first view; no replicate fails; validates | RED (0 lines) |
| GR3 | a_guest_replicate_that_returns_ok_without_a_view_is_failed_and_the_record_still_validates | boundary 8 (M-1); G-2 | body returns `Ok`, never reports | every replicate `failed` with `the replicate reported no guest view`; no `guest_observed`; `run_finished` 0 ok / 9 failed; validates | RED (replicates are `ok`) |
| GR4 | a_view_reported_by_a_replicate_that_then_fails_stays_the_runs_view | DDD-4 | replicate 1 reports then fails with a program exit | the failed replicate's view is the run's one line, before its line; it keeps its own failure | RED (0 lines) |
| GR5 | a_replicate_that_fails_before_reporting_is_counted_and_later_replicates_still_run | US-GBS-03 AC (a)-(c) as CA-1 and CA-4 restate them; boundaries 2 and 5 | replicate 1 fails `guest boot: ...`, replicate 6 fails `guest run: ...` | lines: failed replicate, then `guest_observed`, then the first `ok` replicate; 7 ok / 2 failed; validates | RED (the line is missing, so the third line is a replicate) |
| GR6 | a_drifted_host_never_calls_a_guest_benchmarks_body_and_writes_no_view | US-GBS-02 AC (a); G-1; boundary 6 | smt off on the host; body panics if called | `Err(Refused smt)`; exactly `run_started` (with the guest block) and `run_refused`; no `guest_observed`; no canary sample; validates | RED (no guest block on `run_started`) |
| GR7 | a_native_benchmark_writes_no_guest_block_and_refuses_a_reported_view | boundary 6 (feature disabled); US-GBS-01 AC (d) | a native body calls `report_guest_view` | no `guest` key, substrate `native`, no `guest_observed`; every replicate failed; validates | RED (replicates are `ok`: the scaffold accepts the view) |
| GR8 | a_replicates_console_is_retained_beside_the_record_in_every_outcome | US-GBS-03 AC (d); DDD-9 | one in three replicates fails after retaining | one artifact per replicate at `artifacts/<profile>/<block>-<position>-<arm>/console.log` under the result directory, bytes equal what the replicate retained, failed replicates included | RED (0 artifacts) |
| GR9 | retaining_a_console_twice_in_one_replicate_fails_it_and_keeps_the_first | DDD-9 (create-new) | second `retain_console` | every replicate fails; only the first console is written (9 artifacts, `first`) | RED (replicates `ok`) |
| GR10 | a_console_write_that_fails_fails_the_replicate_and_the_run_continues | DDD-9 | the sink's artifact write fails (full disk) | every replicate is `failed`, all 9 ran, the run returns `Ok` | RED (replicates `ok`) |
| GR11 | the_file_sink_writes_an_artifact_create_new_and_creates_its_parents | DDD-9 (`write_artifact`) | write the same path twice | bytes on disk, parents created; second write `AlreadyExists`, first bytes kept | RED (nothing written) |
| GR12 | the_guest_record_ordering_holds_for_every_script_of_replicates (proptest, 64 cases, Mandate 9) | G-2 evidence lane: the seeded ordering invariant | every script of 9 steps over {report, report-then-fail, no-view, fail-before-report} x two kernels | the run finishes; the record validates; a `guest_observed` line exists exactly when a replicate reported; it holds the first reported view and sits right before that replicate's line; every no-view replicate failed with the pinned reason; the terminal line is last | RED (`guest record ordering invariant violated`; the property collects every breach into one message so the minimal failing script cannot change the reason) |
| GR13 | a_guest_benchmarks_smoke_run_reports_and_retains_without_effect (`replicate.rs`) | DDD-9, DDD-10; US-GBS-04 AC (a) | none | under `smoke`, `report_guest_view` and `retain_console` return `Ok` and nothing is written; the smoke run passes | GREEN guard (the scaffold is inert; stays green once implemented; it is the control of GR14-GR16: under `smoke` only the effect is skipped) |
| GR14 | smoke_fails_a_guest_body_that_returns_ok_without_reporting_a_view (`replicate.rs`) | U6 (pinned); DDD-9; M-1 | a guest body returns `Ok` and never reports a view | the well-formed guest body passes `smoke` (control); the broken one panics naming the benchmark and `the replicate reported no guest view`, as in a real run | RED (`a guest body that returns Ok without a view must fail its smoke run`: `smoke` passes today) |
| GR15 | smoke_fails_a_guest_body_that_retains_its_console_twice (`replicate.rs`) | U6 (pinned); DDD-9 | a second `retain_console` in one replicate | control passes; the second call fails the replicate, so `smoke` panics naming the benchmark | RED (`a second retain_console must fail its smoke run`) |
| GR16 | smoke_fails_a_native_body_that_reports_a_guest_view (`replicate.rs`) | U6 (pinned); DDD-4 (the substrate precondition) | a native benchmark calls `report_guest_view` | control passes; the native body's smoke run panics naming the benchmark | RED (`a view reported by a native benchmark must fail its smoke run`) |

## `GuestMemoryMib` (`crates/overdrive-bench/src/record.rs`, `mod tests`)

Pure newtype contract (DDD-4): `Display` is the decimal digits; `FromStr` accepts decimal digits only, refusing zero, signs, spaces, hex, units and overflow; serde is the JSON integer the record carries; deserialising 0 is refused naming `memory_mib`. Lane: unit. Shape: pure-function. Layers 1-2: PBT full.

| # | Test | Oracle | State |
|---|---|---|---|
| GM1 | a_guest_memory_size_displays_as_its_decimal_digits | `512` displays `512` | GREEN guard |
| GM2 | a_guest_memory_size_round_trips_through_its_text (proptest over every positive `u32`) | `Display` then `FromStr` returns the size | RED (`the text of a size must parse back to the size`: the inert `FromStr` refuses every text) |
| GM3 | a_guest_memory_size_round_trips_through_the_json_integer_the_record_carries (proptest) | JSON equals `Display`; reads back | RED (`the record carries the size as the JSON integer of its digits`: the inert `Serialize` writes `0`) |
| GM4 | text_that_is_not_a_positive_decimal_count_is_refused | `""`, `0`, `00`, `-1`, `+5`, `1.5`, padded, `0x10`, `512mib`, `4294967296` refused; `4294967295` parses (boundary) | RED (`the largest count parses`: the control comes first, so the refusal table is never vacuous) |
| GM5 | a_json_value_that_is_not_a_positive_integer_is_refused_naming_the_field | `0` refused with an error naming `memory_mib`; `-1`, `1.5`, a string, `null`, `4294967296` refused | RED (`a positive JSON integer deserialises to its size`: the control comes first; the inert `Deserialize` refuses every value) |

## Registry and attribute (`crates/overdrive-bench/src/registry.rs`, `tests/compile_fail.rs`)

| # | Test | Traces | Oracle | Lane | State |
|---|---|---|---|---|---|
| GS1 | native_and_guest_are_the_registered_substrates | ADR-0325, DDD-1; US-GBS-04 AC (d) | `native` and `guest` registered; `guest_xyz`, empty, `Native`, `native `, `Guest`, `guest ` not | unit | RED (`guest must be a registered substrate`) |
| GS2 | the_refusal_names_the_substrate_and_the_registered_ones | DDD-1 table | `substrate \`guest_xyz\` is not registered; the registered substrates are: native, guest` | unit | RED |
| GT1 | an_unregistered_substrate_fails_the_build_naming_it_and_the_registered_ones | DDD-1; US-GBS-04 AC (d) | the pinned message with `native, guest` (the fixture's `.stderr` carries it) | trybuild | RED (weak, see `red-classification.md`) |
| GT2 | a_guest_benchmark_declaration_builds | ADR-0325; DDD-1 | `substrate = guest`, a series metric and `clock(guest = guest_monotonic)` build | trybuild | RED (weak: fails at `guest_monotonic` is not one of `init_elapsed_ms`) |
| GT3 | an_unknown_guest_clock_source_fails_the_build_naming_the_set | DDD-1 | `guest clock source \`tsc\` is not one of init_elapsed_ms, guest_monotonic` | trybuild | RED (weak; mismatch on the message text only) |

## Record validation (`bench-report validate`; `tests/integration/report_validation.rs`)

Driving port: the `bench-report` binary (port 4). Production owner path: `record::validate`. Lane: int-host. Shape: pure-function (record in, verdict and diagnostic out). Mandate 11: examples only, one named sad path per failure mode; each case is the valid guest record with one edit, and every test first proves the valid control validates, so until the validator accepts the guest substrate that precondition is the RED (the first refusal today is `clock.guest` for the pinned guest string). The control is shared and unavoidable: every refusal needle (`guest`, `guest_observed`, `clock.guest`) is also a substring of what today's validator says about a guest record, so a refusal test that skipped the control would pass vacuously. GV1-GV6 therefore **activate together** (`red-classification.md` "Activate together").

| # | Test | Traces | Oracle | State |
|---|---|---|---|---|
| GV1 | a_wellformed_guest_record_validates_in_every_terminal_state | US-GBS-01 AC (b); K1 | validates silently: with view; every replicate failed and no view; a failure before the view; a failure between ok replicates; refused with the guest block; incomplete; absent/unreadable view values | RED |
| GV2 | a_guest_block_is_present_exactly_when_the_substrate_is_guest | DDD-2 | a native record carrying `guest`, and a guest record lacking it, refused naming `guest` on line 1 | RED |
| GV3 | a_guest_block_outside_its_pinned_vocabulary_is_refused_naming_the_field | DDD-2; US-GBS-02 AC (c), (d) | 11 cases: `launch`, `boot`, `vcpu_scheduling`, each setting as a value / another reason / unreadable / `0`, a missing setting, an extra setting, an unknown key | RED |
| GV4 | a_guest_observed_line_outside_its_place_is_refused | DDD-2 | on a native record; before the start canary; twice; after the first ok replicate; absent while an ok replicate exists; each names `guest_observed` | RED |
| GV5 | a_malformed_guest_view_is_refused_naming_the_field | DDD-2 | `vcpus` 0 / string / fraction / null; `memory_mib` 0 / string / negative; `kernel` a number / two forms; `clocksource` null; digest of 63 / 65 / upper-case / non-hex / empty; missing, extra and unknown keys; no artifacts | RED |
| GV6 | the_guest_monotonic_clock_string_validates_and_any_other_is_refused | DDD-1 | `clock.guest` = `CLOCK_MONOTONIC (in-guest Instant, 1 ns)` validates; the 1 ms variant, a bare name, `tsc`, empty refused naming `clock.guest` | RED |
| GV7 | no_command_takes_two_records | D8; US-GBS-02 AC (e); K6 | `validate` and `render` with two paths exit 64 | GREEN guard |

The existing GREEN guard `every_retained_record_still_validates` (every record under `benchmarks/results` and the pig spike) is the K4 validation half and stays as it is.

## Report (`bench-report render`; `tests/integration/report_rendering.rs`)

`bench-report render` validates before it renders, so every guest report test (GP1-GP6) starts from a guest record the validator refuses today; the shared RED is that refusal, and GP1-GP6 **activate together** with GV1-GV6 (`red-classification.md` "Activate together"). GP7 and GP8 are native and run today.

| # | Test | Traces | Oracle | Shape | State |
|---|---|---|---|---|---|
| GP1 | the_report_states_the_guests_conditions_between_the_host_lines_and_the_statistics_line | US-GBS-02 AC (b)-(d); K2; DDD-2 | `substrate` is `guest`; the 13 lines between `irq_time_accounting` and `bpf stats` are exactly: launch `metal host (not nested)`, boot `fresh guest per replicate`, vcpu scheduling `unpinned`, governor/smt/boost `absent: not visible from a guest`, vcpus 2, memory mib 512, kernel, clocksource, three artifact digests; the clock line names the guest clock | pure-function | RED |
| GP2 | a_guest_record_without_a_view_prints_the_observed_values_as_absent | CA-4 | with no `guest_observed`, each of the seven observed lines (`vcpus`, `memory mib`, `kernel`, `clocksource`, and one `artifact <key>` line per artifact) prints `absent: no replicate returned a guest view` (U3, pinned); launch and scheduling still print; 0 ok / 9 failed; each failure listed with arm and reason | pure-function | RED |
| GP3 | a_guest_run_refused_on_a_drifted_host_reports_the_refusal_and_the_static_conditions | US-GBS-02 AC (a) | status `refused` and `refused` header `bpf_stats_enabled found 1 pinned 0`; static guest lines present; kernel absent; not incomplete | pure-function | RED |
| GP4 | a_killed_guest_run_is_incomplete_and_a_failed_guest_replicate_is_counted | US-GBS-03 AC (b), (c) as CA-1 restates; K5 | no terminal line: `incomplete (no run_finished or run_refused)`; 1 failed replicate listed with arm and reason | pure-function | RED |
| GP5 | a_one_arm_guest_report_shows_the_distribution_of_the_retained_samples | US-GBS-01 AC (a); parent R18 | series row values 27 / replicates 9 / failures 0; no comparison, no A/A | pure-function | RED |
| GP6 | rendering_a_guest_record_twice_is_byte_identical_and_leaves_the_record_untouched | ADR-0301; C4a | identical bytes; record unchanged | unbounded-preservation | RED |
| GP7 | a_native_report_prints_no_guest_line_and_states_substrate_native | US-GBS-01 AC (d); US-GBS-02 AC (e) | no `- guest ` line; substrate `native` | pure-function | GREEN guard |
| GP8 | every_committed_native_report_re_renders_byte_identical | K4; DDD-2 "a native report is byte-identical" | every committed `record.jsonl` under a native family directory (cgroup-connect-hook, dns-responder, mtls-path, vm-lifecycle) states substrate `native`; every native record has no `guest` block and no `guest_observed`; guest records (guest-flow-forwarding) are admitted; every record with a `report.md` re-renders to exactly those bytes (at least one native report compared) | unbounded-preservation | GREEN guard (observed passing on the Metal host; 12 s) |

## Documents (`tests/integration/methodology_documents.rs`)

| # | Test | Traces | Oracle | State |
|---|---|---|---|---|
| GD1 | the_methodology_has_a_guest_substrate_section_stating_its_rules | DoD; DDD-3 | a `## Guest substrate` section stating `never nested`, `fresh`, `unpinned`, `not visible from a guest`, `digest`, `cargo xtask guest-kit-prepare`, `never compared`, `bpf_stats`, `replicates_ok` | RED |
| GD2 | the_methodology_keeps_version_1_and_says_a_new_substrate_does_not_bump_it | ADR-0327 | `Methodology version 1` stays; the policy says `an addition for a new substrate does not` bump; the isolation rule is substrate-conditional (`a native benchmark runs on the native substrate`) | RED |
| GD3 | the_catalogue_has_a_row_for_the_guest_family_with_its_workload_clock_owner_and_caveats | US-GBS-01 AC (g); DDD-8 | the `guest-flow-forwarding` / `guest_flow_forwarding` row states the arm, metric, `2 vCPUs`, `512 MiB`, `fresh guest per replicate`, guest `CLOCK_MONOTONIC`, owners `overdrive-init` and `overdrive-fwd`, and the caveats citing #368, #370, #371 | RED |
| GD4 | the_catalogue_states_that_a_family_that_launches_vms_is_not_a_guest_benchmark | US-GBS-01 AC (e); D9 | the catalogue states the rule and cites #347 | RED |

The wording each of these pins is the wording DDD-3 and DDD-8 pin; DELIVER writes the documents to it.

## Guest program return (`crates/overdrive-testing/src/guest_app/program_return_tests.rs`)

Driving port: `parse_program_return`, the host half's parse of one replicate's console (DDD-6 pins the signature). Lane: unit. Shape: pure-function. Layers 1-2: proptest for the input space, plus a named example per sad path. The grammar's printer, `print_program_return` (scaffolded in `guest_app` beside the parser so one owner holds both halves, `upstream-issues.md` U1), prints every console below and the parser reads it back; the checksum definition is pinned independently by two fixed vectors computed outside both (GA3). The scaffold printer prints nothing and the scaffold parser finds no block, so every console is `BlockMissing`. Five refusal tests assert only `Err` or `BlockMissing` (GA6, GA7, GA10, GA11 and GA17), which the always-`Err` scaffold satisfies for the wrong reason, so each first proves a control console parses; those five **activate together** with the parser (`red-classification.md` "Activate together"). Every other refusal test pins a variant the scaffold does not return and reaches its own assertion today.

| # | Test | Traces | Oracle | State |
|---|---|---|---|---|
| GA1 | an_intact_console_parses_to_its_readings_and_values_in_order | DDD-6; boundary 1 | readings and values in order, through kernel lines and an unterminated `overdrive-init` fragment | RED |
| GA2 | a_reading_the_guest_could_not_provide_parses_as_absent_or_unreadable | US-GBS-02 AC (c) | `absent:<reason>` and `unreadable:<error>` parse to their forms, never text or a default | RED |
| GA3 | the_checksum_is_the_sha256_of_the_values_as_little_endian_bytes | DDD-6 | `[1,2,3]` and `[42, u64::MAX]` with literal digests parse | RED |
| GA4 | a_block_parses_at_every_value_line_boundary | C1b | 1, 2, 15, 16, 17, 31, 32, 33 and 5,000 values parse exactly | RED |
| GA5 | a_return_after_an_unterminated_fragment_parses_only_with_the_leading_newline | V-B2; ADR-0330 | with the program's `\n` the return parses; glued to the fragment the first reading is `ReadingMissing { kernel }` | RED |
| GA6 | a_begin_line_glued_to_foreign_text_is_refused | strict grammar | a begin line mid-line is not matched; the return fails | RED |
| GA7 | a_console_with_no_block_for_the_metric_is_refused_as_missing | G-2 | empty console, readings only, another metric's block: `BlockMissing` | RED |
| GA8 | a_block_printed_twice_is_refused_as_repeated | G-2 | `BlockRepeated` | RED |
| GA9 | a_reading_that_is_missing_or_repeated_is_refused_naming_its_key | G-2 | `ReadingMissing` / `ReadingRepeated` naming `kernel` and `clocksource` | RED |
| GA10 | a_cut_or_malformed_block_is_refused_and_never_read_as_a_shorter_series | B2 | no end line, cut mid-line, non-digit, negative, above `u64::MAX`, a leading empty value: `Err` | RED |
| GA11 | foreign_text_spliced_into_a_block_line_is_refused_never_shortened | B2 | a kernel message spliced into a value line, a value and the end line: `Err` | RED |
| GA12 | a_block_of_a_different_count_is_refused_naming_both_counts | B2; C1b | 4,999, 5,001, 1, 2 values against 5,000: `CountMismatch { expected, found }` and a `Display` naming both numbers | RED |
| GA13 | a_block_whose_values_do_not_match_its_checksum_is_refused | B2 | `ChecksumMismatch` | RED |
| GA14 | every_error_names_its_cause | DDD-6 | each `Display` names its key or is distinct | RED |
| GA15 | a_printed_return_parses_back_to_exactly_what_was_printed (proptest) | DDD-6 round trip | for every non-empty `u64` series, kernel and clocksource text, foreign lines before and after and an unterminated fragment before the `\n`, `print_program_return` prints and `parse_program_return` returns what was printed | RED |
| GA16 | a_damaged_block_never_parses_to_a_different_series (proptest) | B2 | a flipped value is `ChecksumMismatch`; a dropped or added value is `CountMismatch` (or `ChecksumMismatch`); never `Ok` | RED |
| GA17 | a_reading_line_without_a_value_is_refused | C6b audit gap; DDD-6 | `guest-reading: kernel` with no `=` is `Err` (the variant, missing or malformed, is the crafter's, U2) | RED |

## Guest kit and workspace resolution (`crates/overdrive-testing`)

| # | Test | Traces | Oracle | Lane | State |
|---|---|---|---|---|---|
| GK1 | the_benchmark_guest_band_is_registered_disjoint_and_sixteen_wide | DDD-5 (CID per replicate); V-C partly | `GUEST_BENCHMARK` registered, `REGISTERED` has six bands and is sound, offset `0x2050`, width 16, 16 distinct CIDs | unit | RED (`the benchmark guest band must be registered`) |
| GK2 | a_slot_past_the_benchmark_guest_band_is_refused | DDD-5 (the family's counter wraps at the width) | slot 16 panics `past the end of the CID band` | unit | GREEN guard |
| GK3 | the_benchmark_image_boots_production_init_with_both_modules_and_no_extra_files | DDD-5, DDD-7 | `GuestImageSpec::benchmark()` equals the pinned literal and differs from `workload()` | unit | RED (`the benchmark image must be production init with both modules and no extra files`: the inert `benchmark()` is `workload()`) |
| GW1 | the_workspace_root_is_the_exported_source_root_in_the_exclusive_class | DDD-11; CA-3; boundary of O8 | `OVERDRIVE_METAL_EXECUTION_CLASS=exclusive` with a source root returns it, whatever the cwd | unit (`serial(env)`) | RED (returns the cargo-located root) |
| GW2 | the_workspace_root_keeps_the_ordinary_class_and_every_other_context_unchanged | DDD-11 | `ordinary` uses the root; unset class, unknown class, `exclusive` or `ordinary` with no root resolve through cargo | unit (`serial(env)`) | GREEN guard |

## Smoke variant (US-GBS-04)

The guest family's smoke test is emitted by the macro from the family's declaration (`smoke(guest = "cgroup-default", profile = "nested-launch")`, DDD-10), exactly as the four native families' are. It cannot be written before DELIVER: the declaration does not build while `guest` is refused, and a hand-authored copy would test a different function. What DISTILL covers: the smoke contract for a guest declaration (GR13: report and retain have no effect under `smoke`; GR14-GR16: under `smoke` only the effect is skipped, so the M-1 failure, a second `retain_console` and a guest view reported by a native benchmark fail the replicate and panic naming the benchmark, exactly as in a real run), the generic broken-fixture behaviour the guest family inherits (`smoke_variant.rs`: the failure names the benchmark; a missing block is a fixture failure by GA7-GA13 and the G-2 mapping, which `smoke` turns into a panic naming the benchmark), the placement (`placement-lint` is run, and the declaration's placement is checked by it when DELIVER lands the family), and GT2 (the declaration builds). The nested-launch staging needs nothing beyond what the profile already stages (DDD-10).

### DELIVER handoff notes (smoke and placement)

- **The family declaration and its generated smoke test land in one step.** The declaration (`#[benchmark(substrate = guest, ..., smoke(guest = "cgroup-default", profile = "nested-launch"))]`) does not build until the registry accepts `guest`, and the macro emits its smoke test, so nothing in nextest proves the family before that step. That step is gated by `cargo xtask metal run -- cargo xtask placement-lint` (the generated test must be placed) and by a Metal nextest run of `overdrive-bench-families` (`cargo xtask metal run -- cargo nextest run -p overdrive-bench-families --features integration-tests`) in which the guest family's smoke test and the four native smoke tests pass.
- **GT2 is a weak RED, so its activation is explicit.** In the step that lands the registry entry and the guest clock source, remove GT2's `should_panic` marker and expect the fixture `guest_substrate_accepted.rs` to build whole with no `.stderr` change (it has none: it must compile). GT1 and GT3 keep their `.stderr` files (already pinned); run `git diff` on them and expect no `error:` line to change.

## `run_in_guest_program` failure handling in real guests (`crates/overdrive-testing/tests/integration/guest_program_failures.rs`)

User ruling 2026-11-19 on review finding F3 (option (a)): the failure handling of `run_in_guest_program` (DDD-5, gate G-2, boundary scenarios 2, 4 and 5 at the guest level) is a nextest regression file, not only retained evidence. Driving port: `run_in_guest_program` itself, called on a production-init guest (`GuestImageSpec::benchmark()`) at a benchmark-band CID. Production owner path: the beacon exchange and the VMM life of the function under test. Lane: **int-guest** (Metal, `integration-tests,kvm-tests`), each case in its own `#[overdrive_test(guest = "cgroup-default", profile = "nested-launch")]` guest, so the guest it boots is a nested microVM and the only `/dev/vhost-vsock` holder in that kernel is that VMM. Shape: bounded-change (the run's outcome, console and the VMM).

Faults are test support only, made from outside the function with the kit's own surface: an image that refuses before READY (`forwarder_module: false`), the guest program `overdrive-guest-peer bind 0 0 tcp:0.0.0.0:18080` (prints `bind: bound ...` once running and never ends), `bind 1 0 ...` (exits 0 after a second), and a watcher thread that finds the VMM through `vhost_vsock_holders()` and applies `SIGSTOP` or `SIGKILL` to it when the guest console file (`console.log` under `/run`) shows the kernel booting or the program running. No production code and no new API.

Every case asserts the three outcomes: `ProgramRun.exit` (an `Err`, or `Ok(0)` for the control), the retained console (the kernel banner, and `bind: bound ...` where the program ran or must not have), and the postconditions (no `/dev/vhost-vsock` holder and no `/proc/<pid>` of any VMM the watcher saw, then a clean run on the same CID reaches `Ok(0)`: the CID is free).

| # | Test | Traces | Fault | Oracle | State |
|---|---|---|---|---|---|
| GF1 | a_guest_that_sends_no_ready_is_a_failed_run_that_keeps_its_console_and_leaves_no_vmm | boundary 2 (no READY); G-2 | the image lacks the forwarder module, so the production init refuses before READY and powers the guest off | `exit` `Err`; console holds the kernel banner and not `bind: bound`; VMM reaped; CID reusable | RED (`a guest that never sent READY must not report an exit status`) |
| GF2 | a_guest_stalled_past_the_ready_bound_is_ended_and_its_program_never_runs | boundary 2 and 4 (READY not within `READY_BOUND`; a READY after the bound is never used) | the watcher `SIGSTOP`s the VMM once the kernel is booting; the guest is healthy but frozen | `exit` `Err` after at least `READY_BOUND`; console retained (kernel banner), no `bind: bound`; the live VMM is killed and reaped; CID reusable | RED (`a guest stalled past the READY bound must not report an exit status`) |
| GF3 | a_guest_that_dies_after_ready_is_a_failed_run_that_keeps_its_console_and_leaves_no_vmm | boundary 5 (guest death after READY) | the watcher `SIGKILL`s the VMM once the console shows the program running | `exit` `Err`; console holds `bind: bound ...`; VMM reaped; CID reusable | RED (`a guest that died after READY must not report an exit status`) |
| GF4 | a_program_that_misses_the_exit_bound_is_a_failed_run_that_keeps_its_console_and_ends_the_guest | G-2 (EXIT within `bound`) | `bound` is 10 s and the program never ends | `exit` `Err` after at least the bound, the bound counted from the EXEC write as in `run_in_guest_cell`; console holds `bind: bound ...`; the live VMM is ended and reaped; CID reusable | RED (`a program that misses the exit bound must not report an exit status`) |
| GF5 | a_program_that_exits_within_the_bound_reports_its_exit_keeps_its_console_and_frees_the_cid | G-2 promise (the control); DDD-5 (CID reuse after the VMM exits) | none | `exit` `Ok(0)`; console holds `bind: bound ...`; the VMM exited by itself and is reaped; the same CID boots the next guest | RED (`the console is returned with what the program wrote`: the inert scaffold reports `Ok(0)` and an empty console) |
| GF6 | a_vmm_that_outlives_its_guests_exit_is_a_failed_run_that_keeps_its_console_and_is_ended | G-2 (the post-EXIT wait result is not discarded); DDD-5 (VMM reaped, CID reusable) | made inside the guest: the image stages the kit's scripted peer in place of the production init (`GuestImageSpec::extra` at `OVERDRIVE_INIT_GUEST_PATH`); it plays READY, EXEC and EXIT, runs the program (bound 1 s, exit 0), prints `SCRIPTED_INIT_EXIT_LINE` and idles instead of powering off, so the VMM outlives EXIT | `exit` `Err` (not the program's 0); console holds `bind: bound ...` and the scripted init's EXIT line; the live VMM is ended and reaped; CID reusable. The post-EXIT bound is not pinned by the contract; the case takes the wait whole (30 s in the in-guest lane), so the test runs about 40 s | RED (`a VMM that outlives its guest's EXIT must not report an exit status`) |

CIDs: benchmark guest band slots 11 to 15, one per case, each in its own outer guest (no case shares a kernel with another; the CID band registry is unchanged, so `REGISTERED` keeps the six bands GK1 pins).

**Late READY and a VMM that does not exit.** See "No-obligation rulings".

## Benchmark-run evidence (DELIVER, retained, not tests)

Run through `cargo xtask metal run --benchmark guest-flow-forwarding` on the exclusive Metal host and retained under `benchmarks/results/guest-flow-forwarding/` (consoles under `artifacts/`, not committed). A different agent evaluates each (`verification.md`).

| Run | Scenario | Evidence |
|---|---|---|
| E-G1 | the walking skeleton: a conforming host, a prepared kit, 30 fresh guests | `record.jsonl` validates with `bench-report validate`; `report.md` states `substrate: guest`, the guest lines, per-arm p50/p90/p95/p99; `replicates ok`; the command log contains no `overdrive` binary launch (US-GBS-01 AC (h)); K1, K2; V-D figures |
| E-G2 | K3: one deliberate drift (`kernel.bpf_stats_enabled` set to 1) | a retained refusal naming `bpf_stats_enabled` found 1 pinned 0, exit 78, no `guest_observed`, no guest booted (the console directory is empty and no VMM ran) |
| E-G3 | CA-4: an unprepared kit | a run of 30 failed replicates each `guest boot: ...` naming `cargo xtask guest-kit-prepare`, no `guest_observed`, exit 0, `replicates_ok` 0 |
| E-G4 | V-C: the benchmark process killed during a replicate | after the unit ends `vhost_vsock_holders()` is empty and the CID is reusable; the record has no terminal line and reports incomplete |
| E-G5 | V-C: replicate 17 reuses replicate 1's CID | no launch failure in a 30-replicate run |
| E-G6 | O8: a `spike-scratch/` crate declaring `guest` through `--spike` | a record with the spike evidence marker and a guest view |
| E-G7 | boundary 2 and 4 through the family: a guest that never reaches READY, and one stalled past the bound | a failed replicate each, console retained, no sample used (the function-level behaviour is GF1 and GF2; this is the family's mapping of it) |

## Crafter-owned unit coverage (family-private, not pinned by the design)

The family's host half is private structure (`CLAUDE.md` "Implement to the design": architecture owns WHAT, the crafter owns HOW). These stay crafter-owned unit tests beside the code: the replicate counter modulo 16 that picks the CID, the artifact digests computed once per run, the mapping of each `ProgramReturnError` to its pinned failure text (`the guest returned no intact readings: <cause>` / `the guest returned no intact forwarded_round_trip_ns block: <cause>`), the argv of `forwarding-bench`, the host-pure mapping of `GuestKitError::LaneNotPrepared` to the replicate failure `guest boot: <cause>` with the cause naming `cargo xtask guest-kit-prepare` (US-GBS-01 AC (f), CA-4), and of a non-zero exit to `the in-guest program exited <n>`. The printer and parser are one owner in `guest_app`, and GA1-GA16 already round-trip them (U1 resolved).

## No-obligation rulings (reasons)

- **A READY that arrives after the bound (boundary 4), as a separate case from GF2.** It cannot reach the host half: at `READY_BOUND` the function ends the guest and kills the VMM, so a late READY has nothing to arrive at. The reachable form of the scenario is a healthy guest that is frozen past the bound (GF2), and what the scenario asks (the guest is ended, its program never runs, nothing it would send is used) is asserted there. A second case would exercise the same code path with the same observable.
- **`LANE_GUEST_VCPUS`, `LANE_GUEST_MEMORY_MIB`, `ProgramRun`, `run_in_guest_program`.** Now scaffolded exactly as the Interface contracts pin them (GF1-GF6 drive them); the scaffold's body is inert.
