# Test scenarios — `metal-test-placement-attribute` (DISTILL)

Specification text only: nothing here is parsed or executed (`.claude/rules/testing.md`, "No `.feature` files anywhere"). Every scenario is a Rust test in the owning crate, with a complete executable body. A test marked **pending** carries `#[ignore = "pending DELIVER: <slice>"]`, where `<slice>` is the slice file stem under `../slices/`; the crafter removes the marker, records a semantic RED and builds the behaviour. A test marked **active** passes today and keeps an implemented behaviour true.

Contract Shape follows the design's definitions: **pure-function** (value in, value out), **bounded-change** (a declared mutation set over a named universe), **unbounded-preservation** (reads, never mutates). The universe-bound state-delta assertion of the generic nWave skill has no Rust port in this project; the equivalent here is the observation-level oracle each scenario names (whole `OwnerObservation` snapshots, the full output of the checker, a marker file, the timing lines).

Tags: `@ws` walking skeleton (the promoted slice 01, already delivered), `@error` error or fault path, `@boundary`, `@property`, `@real-io` real kernel or real subprocess, `@in-memory` no I/O, `@driving_port`, `@conformance`.

## Tests by area

### A. Shared-guest owner core (`overdrive-testing`, `src/placement/owner_core/tests.rs`; layer 1-2, in memory)

Owner path: `OwnerCore::handle(Event) -> Vec<Effect>` and `OwnerCore::observe()`. Evidence tier: unit/property (default lane). Oracle: `observe()` snapshots plus the returned effects, compared directly: the core's types derive `Clone, Debug, PartialEq, Eq` (design gap D-3). Activates: `module-tests-share-one-guest`.

| Test | Shape | Scenario | Gate / story |
|---|---|---|---|
| `a_new_owner_has_no_guest_no_clients_and_has_not_stopped` | pure-function | Given a new core / Then it observes `NoGuest`, generation 0, nothing in flight, empty queue, not stopped | G-3 |
| `three_clients_share_one_guest_and_are_served_in_arrival_order` `@driving_port` | bounded-change | Given clients 1, 2, 3 attach to a fresh core / When the boot finishes and each exec finishes with status 0 / Then one `StartBoot` is returned in total, the execs start 1, 2, 3 in order, each is answered `Exit(0)`, and the guest stays `Ready` at generation 1 | US-PLACE-09 s1, G-3 |
| `a_failed_test_discards_its_guest_and_the_next_test_gets_a_replacement` `@error` | bounded-change | Given clients 1-3 and client 1 running / When its exec finishes with status 1 / Then it is answered `Exit(1)`, `DiscardGuest` and `StartBoot` are returned in the same event, and the observation is `Booting` at generation 2 with 2 and 3 queued | G-3, D-TAINT |
| `a_guest_lost_during_a_test_answers_it_and_is_replaced_for_the_next` `@error` | bounded-change | When the guest is lost under a running test / Then it is answered `GuestLost`, a replacement boots (generation + 1) for the queued test | G-3 |
| `a_guest_lost_while_idle_is_replaced_at_the_next_attach` `@error` | bounded-change | When a `Ready` idle guest is lost / Then nobody is answered, the observation is `NoGuest`, and the next attach starts a boot at generation + 1 | DDD-9 |
| `a_client_that_goes_away_while_its_test_runs_taints_the_guest_and_is_never_answered` `@error` | bounded-change | When the running client goes away, then another attaches, then the exec finishes with status 0 / Then going away returns no effect (never cancelled), the next client waits, the finished exec reaches nobody, and the guest is discarded and replaced | G-3, DDD-18, I_DETACH_NO_CANCEL |
| `a_queued_client_that_goes_away_is_removed_and_never_served` `@error` | bounded-change | When a queued client goes away / Then it is removed and neither started nor answered | D-DETACH |
| `a_boot_failure_answers_only_the_head_and_starts_a_new_boot_for_the_next` `@error` | bounded-change | When a boot fails with two clients waiting / Then only the head is answered `BootFailed` and a new boot starts for the next | DDD-9 |
| `a_boot_that_fails_with_nobody_waiting_leaves_no_guest_and_notifies_nobody` `@error @boundary` | bounded-change | Given every waiter went away during the boot / When it fails / Then no effect, `NoGuest`, and the next attach boots generation + 1 | DDD-9 |
| `an_exec_that_reaches_its_bound_is_answered_exec_bound_and_its_guest_discarded` `@error` | bounded-change | When the bound elapses for the running exec / Then `Reply(ExecBound)`, `DiscardGuest` and the replacement `StartBoot` for the waiting test | G-5, D-BOUND |
| `the_late_result_of_an_exec_that_reached_its_bound_changes_nothing` `@error` | bounded-change | When that exec finishes after the discard / Then no effect and the observation is unchanged | G-5, boundary 4 |
| `a_late_result_from_a_detached_exec_reaches_nobody_not_even_the_next_client` `@error` | bounded-change | Given the next client attached after a detach / When the detached exec finishes / Then no `Reply` is returned to anyone | boundary 4 |
| `an_owner_exits_when_idle_and_never_while_work_is_queued_or_running` | bounded-change | When the idle period elapses with tests queued, then running, then with nothing / Then `Exit` is returned only in the last case and the core observes `stopped` | G-6, D-IDLE |
| `a_detached_exec_still_counts_as_running_for_the_idle_exit` `@boundary` | bounded-change | When the idle period elapses while a detached exec runs / Then no `Exit` | G-6 |
| `after_the_owner_exits_every_event_is_ignored` `@boundary` | bounded-change | Given an exited owner / When each of the seven events arrives / Then no effects and no change | G-6 |
| `seeded_event_schedules_keep_the_owner_invariants` `@property` | unbounded-preservation | Given any schedule of up to 60 attach, gone, boot, exec, loss, bound and idle events (256 cases, seed printed by proptest on failure) / Then per event: at most one exec in flight and `in_flight` equals the exec the effects started and no event ended (I_SERIAL, I_DETACH_NO_CANCEL); execs start in arrival order; no reply to a client that went away, none twice (I_DELIVER_ONLY_ATTACHED); a `Ready` idle guest is never a generation a failed, lost, bounded or detached-from exec tainted (I_NO_REUSE); `Exit` only when idle (I_IDLE_NO_LOSS); generation grows exactly by the `StartBoot` count; the queue is the arrival order minus the removed and the started | G-3, G-5, G-6 |
| `every_attached_client_that_stays_is_answered_once_when_the_environment_answers_everything` `@property` | unbounded-preservation | Given any such schedule, then an environment that finishes every boot and exec / Then every client that did not go away was answered exactly once (`EveryClientTerminates`) | G-3 |
| `events_that_are_not_enabled_change_nothing` `@property @boundary` | unbounded-preservation | Given any reached state / When each event that is not enabled in it is delivered (client gone for an unknown or already answered id; exec finished or bound elapsed for an id not in flight; guest lost with no `Ready` guest; boot finished with no boot; idle while work remains; attach of an id already queued or running; every event after exit) / Then no effects and an unchanged observation | § Owner core, "Events the core does not enable" |

Fault stimulus for the seeded runs is the event schedule itself; the bound and idle durations are not exercised (the design pins no test-only override).

### B. Quint conformance of `OwnerCore` (`overdrive-testing`, `src/placement/owner_core/conformance.rs`, `#[cfg(all(test, feature = "integration-tests"))]`)

Evidence tier: Tier 1 conformance (`quint-connect` replay of `specs/quint/shared-guest-owner/shared_guest.qnt`, instance `guest_conf`, referenced in place). The driver struct holds an `OwnerCore` and nothing else; every handled action is one `handle` call; the state is projected only from `observe()`. Activates: `module-tests-share-one-guest`. All pending.

| Test | Spec run (main `guest_conf`) | Invariants constrained by the replayed actions |
|---|---|---|
| `quint_shared_guest_happy_path_three_clients_share_one_guest` | `happy_path_three_clients_share_one_guest` | I_SERIAL, I_NO_REUSE, EveryClientTerminates |
| `quint_shared_guest_failure_then_replacement_guest` | `failure_then_replacement_guest` | I_NO_REUSE, I_SERIAL |
| `quint_shared_guest_guest_lost_mid_exec_then_replacement` | `guest_lost_mid_exec_then_replacement` | I_NO_REUSE |
| `quint_shared_guest_disconnect_while_running_next_waits` | `disconnect_while_running_next_waits` | I_DETACH_NO_CANCEL, I_DELIVER_ONLY_ATTACHED, I_NO_REUSE |
| `quint_shared_guest_disconnect_while_queued_is_removed` | `disconnect_while_queued_is_removed` | I_DELIVER_ONLY_ATTACHED, I_SERIAL |
| `quint_shared_guest_boot_failure_then_retry_boot_on_next_attach` | `boot_failure_then_retry_boot_on_next_attach` | EveryClientTerminates |
| `quint_shared_guest_exec_bound_discards_guest_and_replies_bound` | `exec_bound_discards_guest_and_replies_bound` | EveryClientTerminates (D-BOUND), I_NO_REUSE |
| `quint_shared_guest_idle_exit_only_when_empty` | `idle_exit_only_when_empty` | I_IDLE_NO_LOSS |
| `quint_shared_guest_randomized_traces` (`#[quint_run]`, 64 samples, seed printed) | random walks of `guest_conf` | every invariant of `Safety` over the replayed actions |

Projection: `guest` (NoGuest 0, Booting 1, Ready 2), `gen`, `inflight` (0 when none), `queue`, `owners` (1 unless `stopped`), each through `OwnerObservation`. Model-only and not projected: `lock`, `bad`, `execgen`, `execs`, `faults`, `idleLoss`, `conf_step`, the per-client phases. The `conf_step` pick is the `nondet` source. `ExecFinished{pass}` and `ExecBound` read the in-flight client from `observe()` to build the call, as the action map pins.

**Conformance exemption (user decision N-4, 2026-11-14): one row per invariant, each naming its Tier-3 scenario.**

| Invariant | Not exercised by `guest_conf` replay because | Discharged by (Tier 3, this feature) |
|---|---|---|
| `I_OWNER_UNIQUE` | the election lock is the kernel's; the instance starts with the owner present | `eight_simultaneous_first_arrivals_elect_exactly_one_owner` |
| `I_NO_ORPHAN` | an owner crash leaves no process to receive an event | `killing_the_owner_during_a_guest_exec_leaves_no_vmm` |
| `I_SERIAL` | constrained by replayed actions | covered by Section B replay and Section A property |
| `I_NO_REUSE` | constrained by replayed actions | Section B replay, Section A property, and `a_client_killed_while_its_test_runs_taints_the_guest_for_the_next_client` |
| `I_DETACH_NO_CANCEL` | constrained by replayed actions | Section B replay and Section A example |
| `I_DELIVER_ONLY_ATTACHED` | constrained by replayed actions | Section B replay and Section A property |
| `I_IDLE_NO_LOSS` | constrained by replayed actions | Section B replay and Section A example |
| `EveryClientTerminates` | constrained by replayed actions | Section B replay and Section A liveness property |

### C. Registry and Metal-run predicate (`overdrive-testing`, `src/placement/tests.rs`; layer 1, in memory)

Owner path: `GuestPlacement::declare` (a `const fn`; its string parameters are `&'static str`, D-2) and `in_metal_run()` (the predicate tests set the three variables under `#[serial(env)]` with an environment guard; the pure core's parameter types are the crafter's, D-4).

| Test | State | Shape | Scenario | Story / gate |
|---|---|---|---|---|
| `the_registry_accepts_exactly_the_twenty_one_named_environments` | pending `host-pure-placement-on-the-test` | pure-function | Given each of the 21 named (group, profile) pairs, isolated and shared / Then each is declarable (guards over-rejection; the rejection side is pending below) | US-PLACE-04 |
| `an_omitted_profile_selects_each_groups_base_profile` | pending `host-pure-placement-on-the-test` | pure-function | Given each of the 8 groups with an empty profile / Then it is declarable | US-PLACE-04 |
| `an_unknown_group_is_rejected_by_name_at_declaration` `@error` | pending `guest-placement-on-the-test` | pure-function | Given `cgroup-defualt`, an empty string, `CGROUP-DEFAULT`, `host` / Then declaration panics and the message contains `unknown guest group` | US-PLACE-01 AC d |
| `an_unknown_profile_or_one_of_another_group_is_rejected` `@error` | pending `guest-artifacts-declared-per-group` | pure-function | Given `(core, quint)`, `(probe, native)`, `(mtls-relay, base)`, `(cgroup-default, native)`, an unknown name / Then it panics with `unknown guest profile` | US-PLACE-04 |
| `a_metal_run_is_present_when_any_one_of_the_three_variables_is_set_and_non_empty` | pending `guest-placement-on-the-test` | pure-function | Each of run id, class (`ordinary` or `exclusive`), source root alone, or all, or empty beside `exclusive` / Then a Metal run is present | G-2 |
| `no_metal_run_is_present_when_no_variable_is_set_or_every_set_one_is_empty` `@error` | pending `guest-placement-on-the-test` | pure-function | None set, or only empty values / Then no Metal run | G-2 |

### D. Placement diagnostics (`overdrive-testing`, `tests/compile_fail.rs` + `tests/compile_fail/*`; trybuild)

| Test | State | Shape | Scenario |
|---|---|---|---|
| `placement_diagnostics_name_the_offending_value_and_the_missing_declaration` `@error` | pending `guest-placement-on-the-test` | pure-function | Five fixtures must fail to compile with the pinned message: `unknown guest group` on the attribute and in `shared_guest!` (E0080, text `unknown guest group \`no-such-group\``, span on the literal); `unknown guest profile \`no-such-profile\` for group \`cgroup-default\`` (E0080); the argument-free form without a declaration (E0425 naming `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE`); a `shared` argument (`sharing is declared by \`shared_guest!\``) |

The message line of each `.stderr` is the pinned contract (DDD-1, DDD-5, `spike/findings-const-diagnostic.md`). The span, label and `$DIR` lines are rustc's rendering, derived from the spike's recorded output; a refresh of those lines at activation, reviewed against the pinned message, is not a weakening.

### E. Wrapper and eager check (`xtask`, `tests/acceptance/placement_attribute.rs`; layer 3, real subprocess)

Owner paths: `bash infra/metal/test-wrapper.sh --check-placement` (list JSON on stdin, no Metal context), an ordinary wrapper run (the wrapper's own refusals and chain), and `xtask placement-lint` with a stub `cargo` for `nextest list` only (`cargo metadata` is real). Fixtures: hand-built ELF files that carry exactly the records a scenario needs and run nothing, and this binary's own `fixtures` module, whose records the real attribute wrote. Evidence tier: real subprocess, example-only (Mandate 11); every sad path is a named test.

| Test | State | Shape | Scenario | Story / gate |
|---|---|---|---|---|
| `every_placement_a_test_declares_on_itself_is_placed` | pending `lane-and-launcher-placement-on-the-test` | pure-function | Given records of all five per-test kinds (guest isolated and shared, both lane kinds, native, host-pure) / Then the checker reports nothing, with no table and no Metal context | US-02, US-03 |
| `a_listed_test_with_no_placement_is_reported_naming_its_binary_and_itself` `@error` | pending `host-pure-placement-on-the-test` | pure-function | Given a plain test with no record / Then exactly `unplaced <binary-id> <test>` and a non-zero exit | US-07, G-1 |
| `a_module_declaration_places_exactly_the_tests_of_that_module` `@boundary` | pending `host-pure-placement-on-the-test` | pure-function | Given `module_host_pure` for `crate::declared` / Then `declared::first` is placed, `declared::child::inherits_nothing` and `sibling::third` are unplaced | boundary 7, DDD-4 |
| `a_binary_default_places_every_test_of_a_host_only_binary` | pending `host-pure-placement-on-the-test` | pure-function | Given `default_host_pure` and one per-test host-pure record / Then every test is placed | DDD-4 |
| `a_binary_default_beside_a_non_host_pure_record_is_a_conflict_and_a_module_declaration_is_not` `@error` | pending `host-pure-placement-on-the-test` | pure-function | Default beside each of guest, native, lane operator, lane owner / Then `conflict <binary-id>`; a module declaration beside a guest record / Then no conflict | boundary 7 |
| `a_section_that_cannot_be_read_is_refused_as_an_invalid_placement` `@error` | pending `host-pure-placement-on-the-test` | pure-function | A truncated record / Then non-zero and `placement_invalid`, never "no findings" | G-1 |
| `the_records_the_attribute_wrote_into_this_binary_are_read_by_the_checker` `@real-io` | pending `host-pure-placement-on-the-test` | pure-function | This binary's own fixtures, one of each form (host-pure, `Result`-returning host-pure, native launcher, two module-declared) / Then all are placed: the writer-reader drift guard | DDD-2 |
| `an_unplaced_test_is_refused_on_a_metal_run_before_its_body_runs` `@error` | pending `host-pure-placement-on-the-test` | bounded-change | Ordinary run of a test with no record and no table entry / Then status 78, `test_unclassified` | G-1 |
| `a_binary_that_mixes_a_default_with_a_guest_record_is_refused_on_a_metal_run` `@error` | pending `host-pure-placement-on-the-test` | bounded-change | Then status 78, `placement_conflict` | G-1 |
| `an_unreadable_section_is_refused_on_a_metal_run_as_an_invalid_placement` `@error` | pending `host-pure-placement-on-the-test` | bounded-change | Then status 78, `placement_invalid` | G-1 |
| `a_declared_test_with_an_invalid_run_context_is_refused_by_the_run_context_check` `@error` | pending `host-pure-placement-on-the-test` | bounded-change | Given a declared test and a malformed run id / Then status 78, `invalid_run_context` (the declaration cannot bypass the run-context checks; true today) | US-02 |
| `a_test_that_declares_itself_host_pure_runs_without_a_namespace` `@real-io` | pending `host-pure-placement-on-the-test` | bounded-change | Ordinary run of the fixture / Then exit 0 and the fixture reports the namespace the runner started in | US-02 |
| `a_test_that_declares_itself_a_native_launcher_runs_in_the_full_node_view` `@real-io` | pending `lane-and-launcher-placement-on-the-test` | bounded-change | Then the fixture sees a private network (vsock local mode), mount and cgroup namespace | US-03 |
| `a_record_wins_over_a_table_entry_for_the_same_test` `@boundary` | pending `host-pure-placement-on-the-test` | bounded-change | Given a host-pure record and a `guest-group` table entry (which needs the absent dispatcher) / Then the run passes: only the record can place it | DDD-2 |
| `placement_lint_passes_when_every_listed_test_is_placed` | pending `unplaced-test-fails-in-the-default-lane` | unbounded-preservation | Then exit 0 | US-07, G-4 |
| `placement_lint_names_the_binary_and_the_test_of_an_unplaced_integration_test` `@error` | pending `unplaced-test-fails-in-the-default-lane` | unbounded-preservation | Then `placement-lint: unplaced <binary-id> <test>` and exit 1 | US-07 |
| `placement_lint_reports_a_binary_that_mixes_a_default_with_a_guest_test` `@error` | pending `unplaced-test-fails-in-the-default-lane` | unbounded-preservation | Then `placement-lint: conflict <binary-id>` and exit 1 | boundary 7 |
| `placement_lint_reports_a_lane_test_whose_fixture_does_not_exist` `@error` | pending `lane-and-launcher-placement-on-the-test` | unbounded-preservation | Then `placement-lint: missing-fixture <binary-id> <test> <fixture>` for the misspelled one and nothing for the existing one | boundary 8 |
| `placement_lint_fails_with_the_cause_when_the_test_listing_fails` `@error` | pending `unplaced-test-fails-in-the-default-lane` | unbounded-preservation | Given `nextest list` fails / Then non-zero and the cause in the output, never "no findings" | G-4 |
| `placement_lint_lists_with_integration_tests_and_each_members_own_kvm_feature` | pending `unplaced-test-fails-in-the-default-lane` | unbounded-preservation | Then the listing enables `integration-tests` and `<member>/kvm-tests` for every member whose manifest declares it (read from the manifests, not from a constant) | G-4 |
| `the_placement_table_the_lookup_and_the_dispatcher_are_retired` | pending `placement-table-and-dispatcher-retired` | unbounded-preservation | Then the table file, the dispatcher binary and the wrapper's references to them are gone | US-08 |
| `the_maintained_documents_describe_the_attribute_and_name_no_removed_path` | pending `placement-table-and-dispatcher-retired` | unbounded-preservation | Then `testing.md` documents the attribute and the lint, and no maintained document names a removed path or variable | US-08 |

Fixtures (not scenarios; pending with the scenarios that drive them, and activated in the same slice; `native_launcher_fixture` with `lane-and-launcher-placement-on-the-test`, the others with `host-pure-placement-on-the-test`): `fixtures::per_test_host_pure_fixture`, `host_pure_fixture_may_return_a_result`, `host_pure_fixture_reports_its_namespace`, `native_launcher_fixture`, `module_declared::first_test_of_the_declared_module`, `module_declared::second_test_of_the_declared_module`.

### F. Metal scenarios (`overdrive-testing`, `tests/integration/`; layer 3, real guests, gated `integration-tests` + `kvm-tests`)

Driving port: a fixture test of this binary run as a child process the way nextest runs it (`--exact <name>`), on a Metal run; oracle: the `OVERDRIVE_GUEST_KIT_TIMING` lines, exit status and output, a host marker file, and the owner binary's processes in the run's own cgroup. Every driver observes the host from outside any guest, so it tests the Metal runner's own placement machinery and is placed `#[overdrive_test(native_launcher)]` through the attribute when it is activated (user ruling D-10, 2026-11-14); no table entry exists. The form lands in the lane-and-launcher slice, so the drivers activate there or later. The whole binary joins `host-kernel-shared` in `.config/nextest.toml`.

| Test | State | Shape | Scenario | Story / gate / invariant |
|---|---|---|---|---|
| `a_declared_module_boots_one_guest_and_every_test_reports_its_own_result` `@ws-extension @real-io` | pending `module-tests-share-one-guest` | bounded-change | Given three tests of a module that declares `shared_guest!(guest = "cgroup-default")` / When each runs as its own process / Then each passes alone and the three attach to one guest generation | US-PLACE-09 s1 |
| `one_failing_test_in_a_shared_module_does_not_hide_the_others` `@error @real-io` | pending `module-tests-share-one-guest` | bounded-change | When one test fails inside the guest / Then it alone is FAIL with the guest's output visible, the next test passes on a later generation | US-PLACE-09 s2, D-TAINT |
| `filtering_to_one_test_of_a_shared_module_runs_only_that_test` | pending `module-tests-share-one-guest` | bounded-change | Then `running 1 test` and exactly one attach | US-PLACE-09 s3 |
| `guest_placed_tests_outside_a_shared_module_boot_one_guest_each` | pending `lane-and-launcher-placement-on-the-test`; `native_launcher` (D-10) | bounded-change | Two isolated guest tests and an explicit `guest = ..` test inside a shared module / Then each boots exactly one unshared guest (the approved override rule) | US-PLACE-09 s4, boundary 3 |
| `the_boot_time_of_a_guest_per_test_is_recorded_in_the_run_output` | pending `lane-and-launcher-placement-on-the-test`; `native_launcher` (D-10) | bounded-change | Then one timing line with the group, the test and numeric `staging_ms`, `boot_ms`, `total_ms` | US-PLACE-01 AC f |
| `a_guest_placed_test_runs_its_body_in_place_when_no_metal_run_is_present` `@boundary` | pending `guest-placement-on-the-test`; `host_pure` (it observes nothing outside its own process) | bounded-change | Given the three Metal variables removed / Then the body ran in this process (marker written, exit 0) | US-PLACE-01 AC e, DDD-11, boundary 6 |
| `a_guest_placed_test_never_runs_its_body_on_the_host_on_a_metal_run` | pending `lane-and-launcher-placement-on-the-test`; `native_launcher` (D-10) | bounded-change | Then the marker is absent | US-PLACE-01 AC e, G-2 |
| `a_guest_placed_test_with_an_invalid_run_id_fails_naming_its_group_and_never_runs_on_the_host` `@error` | pending `lane-and-launcher-placement-on-the-test`; `native_launcher` (D-10) | bounded-change | Given the maintenance class and a run id that is present but malformed (refused by the run-runtime rule; an absent id is not this case, D-1) / Then the test fails naming `cgroup-default`, no marker | US-PLACE-01 AC c, G-2 |
| `a_lane_test_naming_a_missing_fixture_fails_naming_the_fixture_and_the_test` `@error` | pending `lane-and-launcher-placement-on-the-test`; `native_launcher` (it calls `run_lane` on the host, D-10) | pure-function | `run_lane` with a fixture nobody wrote / Then the error names fixture and test and is not a body failure | US-PLACE-03 |
| `eight_simultaneous_first_arrivals_elect_exactly_one_owner` `@error @real-io` | pending `module-tests-share-one-guest` | bounded-change | Fault: eight clients start together at a scope with no owner / Then exactly one owner process starts, all pass, one generation | I_OWNER_UNIQUE (N-4 row), G-3 |
| `killing_the_owner_during_a_guest_exec_leaves_no_vmm` `@error @real-io` | pending `module-tests-share-one-guest` | bounded-change | Fault: SIGKILL of the owner during a 40 s exec / Then no process of its tree (the VMM included) outlives it within 5 s | I_NO_ORPHAN (N-4 row), G-3 |
| `the_electing_client_killed_by_a_group_signal_leaves_the_owner_serving` `@error @real-io` | pending `module-tests-share-one-guest` | bounded-change | Fault: SIGKILL to the elector's process group / Then the owner survives and serves the next client, no second owner | boundary 5, DDD-8 |
| `an_owner_killed_midway_fails_its_attached_test_and_the_next_client_elects_a_new_owner` `@error @real-io` | pending `module-tests-share-one-guest` | bounded-change | Fault: SIGKILL of the owner under an attached client / Then the client fails naming the owner stage and the next client elects a new owner | boundary 5 |
| `a_client_killed_while_its_test_runs_taints_the_guest_for_the_next_client` `@error @real-io` | pending `module-tests-share-one-guest` | bounded-change | Fault: SIGKILL of the client during its exec / Then the next client is served on a later generation | D-TAINT, boundary 4 and 5 |
| `a_failed_shared_test_is_reported_with_its_status_and_its_guest_is_replaced` `@error @real-io` | pending `module-tests-share-one-guest` | bounded-change | Fault: the in-guest body fails / Then the client fails, the next passes on a later generation | G-3 |

Fixtures (not scenarios; pending with the scenarios that drive them, activated in the same slice: `shared_module` and `scenario_clients` with `module-tests-share-one-guest`, `isolated_guest` with `guest-placement-on-the-test`): `shared_guest_fixtures::{shared_module, isolated_guest, scenario_clients}`. The scenario clients do nothing under nextest; a scenario names a body through `OVERDRIVE_PLACEMENT_SCENARIO_BODY` and runs one as a child, so each owner scenario has a scope (a distinct declaring-module string) that no other test uses.

### G. Shared-guest client protocol (`overdrive-testing`, `tests/integration/shared_client_protocol.rs`; real filesystem and unix socket in a temporary directory, no guest)

Owner path: `attach_shared(&SharedEndpoint, &GuestPlacement, test_name)` (design gap D-6): the test builds the endpoint itself (socket, lock and owner-binary paths in a temporary directory) and a `GuestPlacement` for `cgroup-default` with `Sharing::Module`. Placement on activation: `host_pure` (no guest, no node-global effect). Activates: `module-tests-share-one-guest`. Both pending. Evidence tier: Tier-3-lane integration (gated `integration-tests`), example-only.

| Test | Shape | Scenario | Gate / story |
|---|---|---|---|
| `a_missing_owner_binary_is_refused_as_an_unprepared_kit_before_the_election_lock_is_taken` `@error` | bounded-change | Given an endpoint whose owner binary path does not exist / When the test attaches / Then `LaneNotPrepared` naming that path, and the lock file was not created | DDD-8, boundary 2 |
| `a_connection_cut_before_the_acknowledgement_re_attaches_once_and_a_second_cut_is_owner_loss` `@error @boundary` | bounded-change | Given a unix listener at the endpoint's socket that accepts every connection and closes it unread, and an existing executable as owner binary / When the test attaches / Then the listener accepted exactly two connections and the failure is `GuestExecution` naming stage `owner` | DDD-17, boundary 5, model assumption A-6 |

## Gate coverage

| Gate | Healthy control | Deliberate fault | Where |
|---|---|---|---|
| G-1 placement guard | `every_placement_a_test_declares_on_itself_is_placed`, `a_record_wins_over_a_table_entry_for_the_same_test` | unplaced, conflict, invalid section, invalid run context | E |
| G-2 Metal context | `a_guest_placed_test_never_runs_its_body_on_the_host_on_a_metal_run`, the `in_metal_run` predicate pair | no Metal variables (runs in place), invalid run id | C, F |
| G-3 shared-guest admission | three-client example, seeded schedules, conformance, module scenario | failure, loss, detach, killed client, killed owner | A, B, F |
| G-4 eager check | `placement_lint_passes_when_every_listed_test_is_placed` | unplaced, conflict, missing fixture, listing failure | E |
| G-5 exec bound | (none: the bound's duration is not exercised) | `ExecBoundElapsed` to a running exec; late result | A, B |
| G-6 idle exit | `an_owner_exits_when_idle_and_never_while_work_is_queued_or_running` | work queued, running, detached; events after exit | A, B |

## Not covered, and why (see the feature delta, "Design gaps and proposals")

The exec bound's duration and the idle period's duration (read only by the owner shell; no test-only override); the client's mapping of `boot_failed`, `guest_lost` and `exec_bound` to `GuestExecution` (a total `match`; the wire encoding is the crafter's; the owner side is covered by sections A and B); a failing body on the per-test path (US-01 AC b is narrowed to the shared path, covered by `one_failing_test_in_a_shared_module_does_not_hide_the_others` and `a_failed_shared_test_is_reported_with_its_status_and_its_guest_is_replaced`); US-PLACE-04's missing-artifact scenarios (dropped; the registry names and the existing `guest_suite` staging tests remain); the real `cargo nextest list` path of `placement-lint` (covered by its own CI and pre-push step); migration parity (a DELIVER procedure). No seam exists for any of these and none is added (design gaps D-6 to D-9).

## Placement of these tests

DISTILL adds no `infra/metal/test-placement.json` entry. A pending test never reaches the Metal wrapper on an ordinary run, so it needs no placement. **Activating a test includes giving it its attribute placement** (`#[overdrive_test(...)]`, `placement!`, `overdrive_test_default!`) in the slice that delivers that form; a test whose form does not exist yet stays pending until then. `native-launcher` is never used for a product test.

Design gap D-10, resolved by the user's ruling of 2026-11-14: tests that must observe the host from outside any guest to prove the placement machinery are tests of the Metal runner itself and are placed `#[overdrive_test(native_launcher)]` through the attribute once that form lands (lane-and-launcher slice), never by a table entry. They are the four scenarios named below, the scenarios of `shared_owner_lifetime`, the three of `shared_guest_scenarios` and `a_lane_test_naming_a_missing_fixture_fails_naming_the_fixture_and_the_test`. The reasoning behind the ruling: the four scenarios that were active (`guest_placed_tests_outside_a_shared_module_boot_one_guest_each`, `the_boot_time_of_a_guest_per_test_is_recorded_in_the_run_output`, `a_guest_placed_test_never_runs_its_body_on_the_host_on_a_metal_run`, `a_guest_placed_test_with_an_invalid_run_id_fails_naming_its_group_and_never_runs_on_the_host`) run a guest-placed fixture as a child process and observe the host (a host marker file, the guest kit's own boot of a guest). No guest group can run them: the guest kit, the guest artifacts and the run's namespaces exist only on the host, and inside a guest `inside_guest()` holds, so the fixture would run its body in place and never take the host path. They are tests of the placement machinery, which is Metal-runner tooling, and the rule allows `native-launcher` only for tests of the runner itself. The owner scenarios have the same property and take the same placement.
