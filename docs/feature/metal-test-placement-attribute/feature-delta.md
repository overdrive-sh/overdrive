<!-- markdownlint-disable MD024 -->
# Feature delta — `metal-test-placement-attribute`

DISCUSS, 2026-11-13. Infrastructure (test tooling), brownfield, changes existing behaviour. Density `lean`, Tier-1 `[REF]` sections only. One job, thirteen stories in nine slices, walking skeleton promoted to production code. Source: [GitHub issue #354](https://github.com/overdrive-sh/overdrive/issues/354) (reference only; no issue number appears in names). Requirements only — DESIGN owns every shape not named below.

## Wave: DISCUSS / [REF] Persona ID

`marcus-platform-verification-engineer` — Marcus, the Overdrive engineer who verifies changes on the shared Metal box; coding agents run the same commands for him, so every pitch below is also what an agent sees. Extended for this feature with a trigger (adding or renaming an integration test). [Persona](../../product/personas/marcus-platform-verification-engineer.yaml).

## Wave: DISCUSS / [REF] JTBD one-liner

`J-PLAT-007`: When I add, rename or move an integration test that must run in a particular Metal isolation (a guest group, the host, a lane fixture), I want that placement declared once, on the test, and checked before the Metal box, so I can trust that the test runs where I meant and never find out at Metal-run time that it was unplaced or mis-placed. [Job register](../../product/jobs.yaml). Journey: [declare where a Metal test runs](../../product/journeys/declare-where-a-metal-test-runs.yaml) — a new journey, because `verify-independent-worktrees` is about concurrency across worktrees, not authoring placement.

## Wave: DISCUSS / [REF] Premise and evidence

Grounded in the repository, not in a reproduced defect rate.

| Fact | Source |
|---|---|
| Placement lives in three places: the test, `infra/metal/test-placement.json`, and the per-group lists in `fwd_guest_suite.rs` (a ~3,900-line dispatcher). | `.claude/rules/testing.md` § "Test placement"; `crates/overdrive-testing/bin/fwd_guest_suite.rs` |
| The table is keyed by binary id (about 80 ids) and full test name; the wrapper refuses an absent entry as `test_unclassified` (status 78) before the test body starts. The rules state "no gate checks that it is complete". | `infra/metal/test-wrapper.sh:59-64,138-143`; testing.md |
| Baseline count: 4,642 entries = 3,855 host-pure + 771 guest-group + 16 native-launcher or guest-lane. Host-pure and "other" counts are line counts of the pretty-printed file; the 771 is the issue's figure. Most guest entries are `cgroup-default` and `mtls-relay`, then `core`. DESIGN's migration tooling must recount by parsing the JSON, since a layout change breaks `grep -c`. | `infra/metal/test-placement.json` |
| The dispatcher already runs one named test in a fresh guest with `--exact <name>` (the `--nextest-wrapper` per-test path, `exact()` helper) and already stages `cgroup-default` host tools "batched or per-test". The skeleton reuses a demonstrated mechanism; no probe is needed. | `fwd_guest_suite.rs:137-161,2340,3932-3941` |
| For guest placements the wrapper currently wraps the dispatcher in the local-mode vsock network namespace; a self-dispatching test's host-side body needs an equivalent. | `test-wrapper.sh:416-418,466` |

Production entry point: this is test tooling, so there is no `overdrive serve` / `overdrive deploy` path. The vertical-slice bar is `cargo xtask metal run -- cargo nextest run …` on the real Metal box with no hand-installed stand-in for the wrapper or the guest kit. Missing DIVERGE artifacts are a noted risk: the attribute-macro approach comes from the issue, not from a DIVERGE comparison; DESIGN must record the alternatives it weighs (for example a generated table, or a lint-only fix).

## Wave: DISCUSS / [REF] Locked decisions

| ID | Decision | Status |
|---|---|---|
| D1 | Feature type: Infrastructure (test tooling), brownfield, changes existing behaviour. UX research depth: lightweight (happy path and error paths). | User, 2026-11-13 |
| D2 | JTBD: yes. Stories trace to `J-PLAT-007`; persona is `marcus-platform-verification-engineer`. New journey `declare-where-a-metal-test-runs`. | User, 2026-11-13 |
| D3 | A walking skeleton is built and PROMOTED: committed production code under `crates/`, not throwaway spike code. | User, 2026-11-13 |
| D4 | Skeleton contract, approved verbatim: (1) the attribute, guest case only: `#[overdrive_test(guest = "<group>")]` in a new proc-macro crate; `host_pure`, guest-lane fixtures and native-launcher are OUT; (2) a guest-kit library extracted out of `fwd_guest_suite.rs`, exposing `inside_guest() -> bool` and `run_in_guest(group, exe, test_name)`, where the host test fails if the in-guest run fails; (3) one existing `cgroup-default` test migrated to the attribute with its JSON entry deleted, and `test-wrapper.sh` runs it without a table lookup, the rest of the table and the dispatcher remaining until later slices; (4) the skeleton records per-test guest boot time (one guest per test) as evidence for DESIGN's batched-run decision. Production entry point: `cargo xtask metal run -- cargo nextest run …`. DESIGN may reshape items 1-2 for the general case. | User, approved 2026-11-13 |
| D5 | Sequencing: DISCUSS, then the walking skeleton (nw-spike walking-skeleton mode, no probe), then DESIGN answers the open questions using skeleton evidence, then DISTILL/DELIVER for the remaining slices. | User, 2026-11-13 |
| D6 | Attribute variants, the lint mechanism and every public shape beyond D4 are DESIGN's. This wave does not name them. | Scope rule |
| D7 | Slice and identifier names describe behaviour, never roadmap steps or issue numbers (`.claude/rules/domain-naming.md`). | Project rule |
| D8 | Module-level batching is IN scope: the tests of a module can declare that they share one guest of a group (US-PLACE-09). It replaces the explicit batched `overdrive-fwd-guest-suite` command, so the dispatcher can be retired entirely in US-PLACE-08. The attribute's shape (on the `mod`, or a `group = …` argument on each test) and how per-test results survive a shared guest are DESIGN's. | User, 2026-11-13 |

D4 does not authorise public surface beyond the named attribute, `inside_guest` and `run_in_guest`; the "inside the guest" marker and the boot-time record's format are DESIGN's or the spike's.

## Wave: DISCUSS / [REF] User stories with elevator pitches

All stories: `job_id: J-PLAT-007`. Persona Marcus. Test names are real names from the placement table. Where a pitch says "picked at slice start", the pick is one of the table's existing entries of that kind and is recorded in the slice's first commit message; no new test is invented.

### US-PLACE-01 — Declare a guest placement on the test (walking skeleton)

Slice: [guest placement on the test](slices/slice-01-guest-placement-on-the-test.md). Must-have.

Marcus adds a test that needs the `cgroup-default` guest. Today he also edits the JSON table and, for new group behaviour, the dispatcher; if he forgets the table the first signal is `test_unclassified` on the Metal box.

### Elevator Pitch
Before: placing a guest test means editing the test, `infra/metal/test-placement.json` and possibly `fwd_guest_suite.rs`; a miss shows up only on a Metal run.
After: run `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(cluster_status_with_no_server_returns_transport_error)'` (candidate, final pick at slice start) → sees `PASS [ …s] overdrive-cli::integration integration::cluster_and_node_commands::cluster_status_with_no_server_returns_transport_error` for a test carrying only `#[overdrive_test(guest = "cgroup-default")]` and no table entry.
Decision enabled: Marcus decides the placement by writing one attribute and trusts the run result without touching the table or the dispatcher.

Domain examples:
1. Marcus migrates `cluster_status_with_no_server_returns_transport_error` (`cgroup-default` today), deletes its JSON entry, and the Metal run passes with the body executing inside a fresh guest.
2. Marcus makes the in-guest body assert a false condition; the host test is FAIL with the guest's failure output, and the run exits nonzero.
3. Marcus runs the migrated test outside any Metal run (a laptop shell, Lima, CI); its body runs in the current process, as it does today (user ruling 2026-11-14, DDD-11).
4. Marcus typos the group (`"cgroup-defualt"`); the failure names the unknown group instead of silently picking one.

```gherkin
Scenario: A self-placed guest test passes on Metal
  Given Marcus's cluster_status test declares guest "cgroup-default" and has no table entry
  When he runs it through cargo xtask metal run with cargo nextest run
  Then the test body executes inside a fresh cgroup-default guest
  And nextest reports one ordinary PASS for it

Scenario: A failing in-guest body fails the host test
  Given the same test's body asserts a false condition
  When Marcus runs it on Metal
  Then the host-side test is FAIL and the guest's failure output is visible
  And the metal run exits nonzero with the command's own status

Scenario: A guest that cannot boot fails the test, never passes it
  Given the declared group's guest cannot start or exceeds its bound
  When Marcus runs the test on Metal
  Then the host test is FAIL and the failure names the group and the stage that failed

Scenario: An unknown group is rejected by name
  Given a test declares guest "cgroup-defualt"
  When Marcus builds or runs it
  Then the failure names the unknown group (where it surfaces is DESIGN's)

Scenario: A guest test outside a Metal run runs in place
  Given the migrated test is run with no Metal run context
  Then its body runs directly in the current process, as it does today (user ruling 2026-11-14, DDD-11)
  And on a Metal run its body never runs on the host

Scenario: Guest boot time is recorded per test
  Given the migrated test ran with one guest for that test
  When Marcus reads the retained run output
  Then the guest boot time for that test is recorded for later comparison

Scenario: Tests still in the table are unaffected
  Given the rest of the table and the dispatcher are untouched
  When Marcus runs any not-yet-migrated guest test
  Then it runs through the dispatcher exactly as before and passes
```

AC: (a) migrated test PASSes on Metal with its JSON entry deleted; (b) in-guest failure surfaces as host FAIL with the run exiting nonzero; (c) boot failure or timeout is a FAIL naming group and stage; (d) an unknown group is rejected by name; (e) outside a Metal run the test runs its body in place, and on a Metal run it never runs on the host; (f) per-test boot time retained; (g) unmigrated guest tests still pass through the dispatcher after the extraction. Technical notes: the host-side body needs the local-mode vsock namespace it has today; telling a self-declared test from an unplaced one while the table coexists is O1.

### US-PLACE-02 — Declare a host-pure placement on the test

Slice: [host-pure placement on the test](slices/slice-02-host-pure-placement-on-the-test.md). Must-have.

Marcus adds a pure integration test with no kernel effects. Today it still needs a table entry (3,855 of 4,642 entries are this case).

### Elevator Pitch
Before: a host-pure test needs a table entry or it is `test_unclassified` on Metal.
After: run `cargo xtask metal run -- cargo nextest run -p overdrive-store-local --features integration-tests -E 'test(malformed_intent_bytes_cause_refuse_to_start)'` → sees `PASS` for a test declared host-pure on itself, with no table entry and no guest boot.
Decision enabled: Marcus decides "no isolation needed" in the test, in one line, and gets the behaviour the table gave.

Domain examples: (1) `integration::envelope_intent_refuse::malformed_intent_bytes_cause_refuse_to_start` in `overdrive-store-local::integration` is declared host-pure, its entry deleted, and it passes without a guest; (2) the sibling `unknown_future_variant_tag_causes_refuse_to_start` stays in the table and still passes (coexistence); (3) Marcus declares host-pure on a test whose Metal context is invalid (stale source marker); the existing `invalid_run_context` diagnostic still fires, so the declaration cannot bypass the run-context checks.

```gherkin
Scenario: A host-pure declared test runs without a guest
  Given the malformed_intent_bytes test declares host-pure placement and has no table entry
  When Marcus runs it through metal run
  Then it passes without a guest being booted

Scenario: Host-pure tests keep the original runner chain
  Given the test declares host-pure placement
  When it runs on Metal
  Then it runs under the same chain as a table host-pure test today, with no namespace

Scenario: Run-context checks still apply to declared tests
  Given the Metal run context is invalid
  When a host-pure declared test is launched
  Then the existing invalid_run_context diagnostic fires before the body

Scenario: A host-pure declaration is visible in the test source
  Given Marcus reviews the diff of a new integration test
  Then the host-pure declaration appears on the test, not in a separate file
```

AC: declared host-pure test passes on Metal with no table entry and no guest; chain and isolation equal today's host-pure; existing run-context diagnostics remain; the declaration is on the test. Notes: depends on US-PLACE-01's resolution of O1.

### US-PLACE-03 — Declare lane-fixture and native-launcher placements on the test

Slice: [lane and launcher placement on the test](slices/slice-03-lane-and-launcher-placement-on-the-test.md). Must-have (16 table entries; they block table deletion).

Marcus maintains tests that exercise guest-lane operator/owner fixtures and the Metal runner itself (native-launcher). They carry extra data (a named in-guest fixture) the table holds today.

### Elevator Pitch
Before: lane-fixture and runner self-tests are the only placements that still need a hand-edited table row with a fixture name.
After: run `cargo xtask metal run -- cargo nextest run --features integration-tests,kvm-tests -E 'test(a_burst_of_host_aborts_resets_every_application_connection)'` → sees `PASS` for a lane-owner test whose fixture `integration::in_guest_lane::a_burst_of_host_aborts_resets_every_connection_in_a_real_guest` is declared on the test.
Decision enabled: Marcus can place any of the five placement kinds from the test, so the table has no remaining reason to exist.

Domain examples: (1) the owner test `guest_flow::real_kernel::a_burst_of_host_aborts_resets_every_application_connection` names its fixture on itself and passes; (2) its operator-side counterpart does the same (name from the table at slice start); (3) a native-launcher runner self-test (one of the table's native-launcher entries, picked at slice start) keeps the full node view (network, mount, cgroup namespaces, own per-run cgroup).

```gherkin
Scenario: A lane owner test declares its fixture
  Given the burst-of-host-aborts owner test declares its in-guest fixture on itself
  When Marcus runs it on Metal
  Then the same fixture runs in a fresh guest as the table-driven path did

Scenario: A lane operator test declares its fixture
  Given an operator-side lane test declares its fixture
  Then it passes on Metal with no table entry

Scenario: A native-launcher self-test keeps its node view
  Given a runner self-test declares native-launcher placement
  When it runs on Metal
  Then it still sees a private network, mount and cgroup namespace set and its own per-run cgroup

Scenario: A missing fixture is reported by name
  Given a lane test names a fixture that does not exist
  When Marcus runs it
  Then the failure names the missing fixture and the test
```

AC: all 16 non-guest-group, non-host-pure placements are expressible and pass with entries deleted; node-view isolation unchanged; a missing fixture is named. Notes: the mapping is DESIGN's open question; "not expressible by the attribute" is a valid DESIGN finding that returns to this wave.

### US-PLACE-04 — Declare guest artifacts per group, once

Slice: [guest artifacts declared per group](slices/slice-04-guest-artifacts-declared-per-group.md). Must-have.

Marcus needs a new tool inside `cgroup-default` guests. Today that is a hand-grown copy list inside the dispatcher (it already stages `rustc`, a C linker toolchain, `bpftool` and busybox).

### Elevator Pitch
Before: per-group guest staging lives as copy lists inside `fwd_guest_suite.rs`, reachable only by editing the dispatcher.
After: run `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(cluster_status_with_no_server_returns_transport_error)'` after adding a tool in the one declared `cgroup-default` location → sees `PASS` and a guest that contains the tool; removing it makes the run FAIL naming the group and artifact.
Decision enabled: Marcus adds or removes a guest dependency in one named place and knows which groups it affects.

Domain examples: (1) add `tcpdump` to `cgroup-default`; only that group's guests contain it; (2) remove a required tool and the dependent test fails naming `cgroup-default` and the tool; (3) `mtls-relay` and `cgroup-default` keep distinct artifact sets.

```gherkin
Scenario: A group's artifacts are declared in one place
  Given cgroup-default's artifact set is declared once
  When Marcus adds a tool to it
  Then every cgroup-default test sees the tool and other groups do not

Scenario: A missing artifact names the group
  Given a required artifact is absent from the group's declaration
  When a migrated test needing it runs
  Then the failure names the group and the artifact

Scenario: A declared artifact that does not exist on the box is reported
  Given a group declares an artifact path that is absent on the Metal box
  When a guest of that group starts
  Then the run fails naming the group and the path before the test body

Scenario: Existing staging is unchanged for unmigrated tests
  Given dispatcher paths still exist
  Then they stage the same artifacts as before
```

AC: one declaration per group (shape DESIGN's); the dispatcher's copy lists for declared groups read from it; missing and absent artifacts fail by name; existing tests' results unchanged.

### Migration stories (US-PLACE-05a to 06c)

Pre-split into six stories of one batch each, scripted and mechanical by hypothesis; each is 1-3 days and each has 3-4 scenarios. Rule: a batch that exceeds 3 days at DESIGN's estimate is split further by binary id with the same AC. Each story deletes its batch's JSON entries and moves the tests' declarations onto the tests.

| Story | Batch |
|---|---|
| US-PLACE-05a | Guest placements in groups `core`, `probe`, `unload`, `ulp`, `stress`, `nested-boundary` |
| US-PLACE-05b | Guest placements in group `mtls-relay` (including `overdrive-bpf::integration`) |
| US-PLACE-05c | Guest placements in group `cgroup-default` |
| US-PLACE-06a | Host-pure placements in `overdrive-core*` and `overdrive-sim*` binary ids |
| US-PLACE-06b | Host-pure placements in `overdrive-control-plane*`, `overdrive-worker*`, `overdrive-cli*`, `overdrive-host*`, `overdrive-gateway`, `overdrive-reconcilers*`, `overdrive-store-local*` |
| US-PLACE-06c | Host-pure placements in all remaining binary ids (`overdrive-dataplane*`, `overdrive-fwd*`, `overdrive-init*`, `overdrive-netlink`, `overdrive-system-conformance*`, `overdrive-testing*`, `xtask*`, fixtures) |

Slices: [guest placements migrated](slices/slice-05-guest-placements-migrated.md) (05a-c), [host-pure placements migrated](slices/slice-06-host-pure-placements-migrated.md) (06a-c). Must-have. Depends: 05a-c on 01, 04; 06a-c on 02; 05c and 06 batches also need 03 where a batch contains lane or native-launcher entries.

### Elevator Pitch (shared by each migration story, with its batch)
Before: renaming or adding a test in this batch needs a JSON edit, or the Metal run fails `test_unclassified`.
After: run `cargo xtask metal run -- cargo nextest run --features integration-tests,kvm-tests -E 'test(<a test of the batch>)'` after renaming that test (batch-specific names: 05b `tcp_syn_plus_rst_flags_drops_with_malformed_header_counter`; 05c `cluster_status_with_no_server_returns_transport_error`; 06b `malformed_intent_bytes_cause_refuse_to_start`; 05a/06a/06c pick at slice start from the batch) → sees `PASS` for the renamed test with zero JSON edits, and the batch's pass set equals the pre-migration pass set.
Decision enabled: Marcus can rename or add tests in this batch without a placement edit and trusts that no test changed where it runs.

Domain examples (05b, representative): (1) the `overdrive-bpf::integration` classifier tests move together and pass; (2) a batch that would change `tcp_syn_plus_rst_flags_drops_with_malformed_header_counter` from `mtls-relay` to another group is rejected by the count check; (3) a renamed test in the batch needs no JSON edit. 06 batches: (1) store-local tests migrate together; (2) a test found to mutate node-global kernel state while declared host-pure is reported as a finding and not carried over; (3) a renamed test needs no JSON edit.

```gherkin
Scenario: Every placement in the batch is preserved
  Given the table lists N tests with kind and group per test in the batch
  When the batch is migrated
  Then the same N tests carry the same kind and group on the tests, and none remains in the table

Scenario: A batch passes on Metal with the same pass set
  When Marcus runs the migrated binaries through metal run
  Then the pass set equals the pre-migration pass set

Scenario: A changed placement is rejected
  Given a migrated test would land in a different kind or group than its table entry
  Then the batch is rejected and the difference is reported

Scenario: Unmigrated batches keep working mid-migration
  Given other batches are still in the table
  Then they run as before
```

AC: batch entries removed from the table; kind and group preserved per test (count and identity check, recounted by parsing the JSON); pass-set parity on Metal; mis-placed tests surfaced; end state across the six stories: guest-group entries 771 to 0 and host-pure 3,855 to 0.

### US-PLACE-07 — An unplaced integration test fails before the Metal box

Slice: [unplaced test fails before Metal](slices/slice-07-unplaced-test-fails-in-the-default-lane.md). Must-have.

Marcus adds an integration test as a plain `#[test]`. Today it first fails on Metal as `test_unclassified`; with the attribute alone it could run on the host silently.

### Elevator Pitch
Before: a missing placement is discovered on the Metal box, or not at all.
After: run `cargo xtask metal run -- cargo check --all-targets --features integration-tests` (the compile-check the rules already prescribe for integration binaries; if DESIGN makes the lint a different command, that command replaces it here) → sees a failure naming the binary and the test with no placement declared; exact text is DESIGN's.
Decision enabled: Marcus fixes a missing placement in seconds, before pushing, instead of after a Metal run.

Domain examples: (1) a new plain `#[test]` `integration::new_mesh_case::dials_by_name` in an integration binary fails the check naming binary and test; (2) the same test with a declaration passes; (3) a test in the unit lane with no attribute, no module declaration and no binary default is flagged, and passes once its module declares host-pure; (4) a declared test inside a binary that still has a table entry for another test does not trip the check.

```gherkin
Scenario: A plain integration test is flagged
  Given an integration binary contains a test with no placement declaration
  When Marcus runs the check
  Then it fails naming the binary and the test, without a Metal guest boot

Scenario: A declared test passes the check
  Given every integration test in the binary declares a placement
  Then the check passes

Scenario: Unit-lane tests are placed by a module or binary declaration
  Given a unit-lane test has no attribute of its own
  When its module declares `placement!(host_pure)` or its binary declares `overdrive_test_default!(host_pure)`
  Then the check passes; with neither declaration it flags the test (DDD-4, user ruling 2026-11-14)

Scenario: The check is wired where Marcus already runs it
  Given Marcus pushes a branch that adds an unplaced integration test
  Then the check fails in the step the repository already runs for that surface
```

AC: unplaced integration test fails before Metal naming binary and test; declared tests pass; a unit-lane test passes when its module or binary declares host-pure and is flagged otherwise; check runs in a command already in Marcus's loop (adding a new lane or tool requires user approval first). Notes: mechanism and how it lands before full migration are O3.

### US-PLACE-08 — Retire the placement table, the lookup and the stale rule text

Slice: [placement table and dispatcher retired](slices/slice-08-placement-table-and-dispatcher-retired.md). Must-have.

With everything self-placed and linted, the three drift sources are removed and the rules describe the one model.

### Elevator Pitch
Before: three places define placement and `testing.md` documents the table.
After: run `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(cluster_status_with_no_server_returns_transport_error)'` after renaming that test to a new name and changing nothing else → sees `PASS`, and `git ls-files infra/metal/test-placement.json` prints nothing because there is no file left to edit.
Decision enabled: Marcus relies on the attribute as the only placement source and reads one rule section for it.

Domain examples: (1) the JSON file is gone and the renamed test passes; (2) a brand-new test with a declaration passes with no table and no dispatcher edit; (3) `.claude/rules/testing.md` § "Test placement" describes the attribute and the lint, and no longer the JSON.

```gherkin
Scenario: A renamed or new test needs no placement file
  Given the table, its lookup and the dispatcher's per-test table are removed
  When Marcus renames a declared test and runs it on Metal
  Then it passes with no other edit

Scenario: The full Metal pass set is unchanged
  When Marcus runs the full integration lane on Metal after retirement
  Then the pass set equals the pre-retirement pass set

Scenario: The dispatcher is retired or reduced as DESIGN decided
  Given DESIGN decided whether batched group runs remain
  Then the dispatcher binary is deleted, or only the retained batched path remains

Scenario: The rule text and references match the model
  Then testing.md § "Test placement" documents the attribute and the lint
  And a search for the removed names finds no stale reference
```

AC: with the table and lookup absent, a renamed test and a new test pass and the full pass set matches; dispatcher deleted or reduced exactly as DESIGN decided; no stale reference to removed names (`test-placement.json`, the removed diagnostics, a deleted binary); testing.md updated through the normal rules-change process. Technical notes (structural, not AC): unreachable wrapper code and its tests are deleted together; other documents cite `overdrive-fwd-guest-suite` (for example the `metal-run-isolation` journey) and are updated in the same slice.

### US-PLACE-09 — Run a module's tests in one shared guest

Slice: [module tests share one guest](slices/slice-09-module-tests-share-one-guest.md). Must-have (D8).

Today every guest-placed test boots its own guest on an ordinary run (about 0.45 s staging plus 2.4 s boot each); only the explicit batched dispatcher command shares a guest across a group, and it lives in the table-driven dispatcher this feature retires. A module-level declaration keeps the speed-up on the test, so the batched path survives the dispatcher's removal.

### Elevator Pitch
Before: sharing one guest across tests needs the separate `overdrive-fwd-guest-suite` command and its hand-kept group lists; ordinary nextest runs boot one guest per test.
After: run `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(/^integration::cluster_and_node_commands::/)'` on a module declared as one shared `cgroup-default` guest → sees one result per test and one guest boot for the module in the `OVERDRIVE_GUEST_KIT_TIMING` output.
Decision enabled: Marcus chooses per module between isolation (a guest per test) and speed (one guest per module) on the test code itself, and drops the separate batched command.

```gherkin
Scenario: A declared module boots one guest
  Given a module whose tests declare that they share one cgroup-default guest
  When Marcus runs the module's tests on Metal
  Then one guest boots for the module and every test reports its own result

Scenario: One failing test does not hide the others
  Given one test in a shared-guest module fails
  When the module runs on Metal
  Then that test is reported FAIL and the other tests report their own results

Scenario: Filtering to one test still works
  When Marcus runs a single test of a shared-guest module with -E 'test(=…)'
  Then only that test runs and it reports its result

Scenario: Undeclared tests keep one guest each
  Given a guest-placed test outside any shared-guest module
  Then it still boots its own guest
```

AC: a shared-guest module boots one guest per run (counted from the timing output); per-test results, failures and `-E` filtering behave as for per-test guests; tests outside a declared module are unchanged; the explicit batched dispatcher command's users can be moved to the declaration, so US-PLACE-08 deletes the dispatcher. Technical notes (DESIGN's): the attribute shape; how nextest's one-process-per-test model reaches a shared guest (one generated test per module versus a guest shared across test processes) and who starts and stops it; what state tests in a shared guest can leak to each other.

## Wave: DISCUSS / [REF] Story map and scope assessment

Backbone (Marcus's journey): Write the test, declare its placement, run it on Metal, provide guest needs, move existing tests, catch a miss, retire the old path.

| Declare placement | Run on Metal | Provide guest needs | Move existing tests | Catch a miss | Retire the old path |
|---|---|---|---|---|---|
| WS: guest on the test (US-01) | WS: wrapper runs it, no lookup (US-01) | | WS: one test (US-01) | | |
| host-pure (US-02) | | per-group artifacts (US-04) | guest batches (US-05a-c) | unplaced test fails before Metal (US-07) | table, lookup, dispatcher, rule text (US-08) |
| lane and launcher (US-03) | | | host-pure batches (US-06a-c) | | |

Walking skeleton = US-01 (guest case, one test, boot time recorded). Priority rationale: US-01 first because it retires the riskiest assumption (a self-dispatching test binary reproduces dispatcher results and the wrapper can run it without a lookup) and yields the boot-time evidence DESIGN needs. US-02 and US-03 follow: they complete the set of declarable placements, US-03 carrying the most mapping risk. US-04 follows because guest batches need declared artifacts. The migrations follow, guest batches before host-pure because guest placement is where the dispatcher and artifacts drift hurts. US-07 follows the migrations because a lint on a half-migrated tree is noise unless DESIGN scopes it per binary (O3). US-08 closes the loop.

Scope assessment: **PASS (sliced)** — 12 stories, 1 bounded context (Metal test tooling), 4 integration surfaces (proc-macro crate, guest-kit library, wrapper script, dispatcher and nextest config). Only the effort signal is borderline (migration volume, 4,642 entries, plausibly more than two weeks in total); it is mitigated by eight independently shippable slices, six scripted migration batches of 1-3 days, and the user-approved skeleton-first sequencing. Story count exceeds ten only through the pre-split migration batches, which share one acceptance shape.

## Wave: DISCUSS / [REF] Outcome KPIs

Objective: Marcus declares a Metal test's placement once and learns of a miss before the Metal box.

| KPI | Who does what | Target | Baseline | Measured by |
|---|---|---|---|---|
| K1 (primary) | Marcus places one new integration test | 1 edit site (the test), 0 table or dispatcher edits | 2-3 edit sites (test, JSON, dispatcher for new group behaviour) | Diff of the skeleton commit and of one later new-test commit |
| K2 | Marcus finds a missing placement | 100% of unplaced integration tests flagged before any Metal guest boots, within one run of the check | 0% flagged before Metal; first signal is `test_unclassified` on Metal | Negative-fixture test (one deliberately unplaced test) plus no `test_unclassified` in later Metal runs |
| K3 (guardrail) | Marcus's results stay correct through migration | 0 pass-set differences per migrated batch; 0 placements changing kind or group | Table-driven pass set | Per-batch before/after pass sets and per-test kind and group identity, recounted by parsing the JSON |
| K4 (guardrail) | Marcus pays a bounded guest cost | Per-test guest boot time unchanged from the table-driven path (ordinary runs already boot one guest per guest-placed test attempt); measured about 0.45 s staging plus 2.4 s boot on the skeleton test | One guest per test attempt on ordinary runs; only the explicit batched dispatcher command boots once per group | Skeleton evidence record |
| K5 | The repository sheds placement sources | Placement sources 3 to 1; table entries 4,642 to 0 | 3 sources, 4,642 entries | File presence and parsed entry count at US-08 |

Owner: implementer captures, independent reviewer evaluates; no new dashboard. K4 is one recorded measurement used as design evidence, not a performance claim or benchmark.

## Wave: DISCUSS / [REF] Definition of Done

- [ ] US-01 skeleton: attribute, guest-kit entry points and the wrapper's no-lookup path on the real Metal path; boot time recorded; promotion gate decided.
- [ ] US-02, US-03: every placement kind declarable on the test with a passing Metal run.
- [ ] US-04: one declaration location per group.
- [ ] US-05a-c, US-06a-c: zero table entries; pass parity per batch.
- [ ] US-07: unplaced test fails before Metal.
- [ ] US-08: table, lookup and dispatcher (or its reduced remainder) retired; testing.md updated.
- [ ] No GitHub issue number or roadmap-step id in any new name.

## Wave: DISCUSS / [REF] Out of scope

Changing which placement any existing test has; changing guest group semantics or the node-view isolation; the Metal run lease and worktree identity (see `metal-run-isolation`); new test tiers. Whether to keep batched group runs is not deferred: it is a DESIGN decision with skeleton evidence.

## Wave: DISCUSS / [REF] Walking skeleton strategy

Real, promoted, not a mock. Spike in walking-skeleton mode with no probe (the dispatcher already demonstrates in-guest `--exact`). Increment: US-01 as in D4. The spike records the promotion-gate decision in its own `wave-decisions.md`; DESIGN may reshape items 1-2. Run on the native Metal box per `.claude/rules/testing.md`; Lima is no signal for guest boots. The spike's gate decision is PROMOTE (user, 2026-11-13); the skeleton code is promoted.

## Wave: DISCUSS / [REF] Driving ports

Developer-facing only: (1) writing the placement attribute on a test; (2) `cargo xtask metal run -- cargo nextest run …`; (3) the placement lint (US-07). `inside_guest` and `run_in_guest` are skeleton-approved library entry points used by the attribute's expansion, not operator surfaces.

## Wave: DISCUSS / [REF] Pre-requisites

Metal box with KVM provisioned; guest kit prepared (`cargo xtask metal run -- cargo xtask guest-kit-prepare`) and module loaded (`--exclusive … kmod-load`); `OVERDRIVE_METAL_TARGET`, `OVERDRIVE_METAL_KERNEL`, `OVERDRIVE_METAL_ROOTFS` set; the existing per-test dispatcher path (present).

## Wave: DISCUSS / [REF] Open points for DESIGN and blockers for the user

DESIGN's questions from the issue stand (lane and native mapping, per-group artifacts, batched runs, local-mode vsock namespace). Added by this wave, none resolved here:

- O1 (blocks US-01's general shape): while the table coexists, the wrapper cannot skip the table lookup for every test without silently running unplaced plain tests on the host. DESIGN must pin how a self-declared test is told apart from an unplaced one, and which side supplies the local-mode vsock namespace to the host-side body. The skeleton uses the thinnest form that works; if the spike cannot find one, it returns to the user rather than weakening the `test_unclassified` guard.
- O2: whether the guest-kit extraction reuses `crates/overdrive-testing/src/guest_kit.rs`; D4 names a library, DESIGN picks its home.
- O3: how the lint can land before 100% migration (per-binary scope, or last). US-07's order depends on it.
- O4: resolved by D8: batching moves onto the test as a module-level declaration (US-09), so US-08 deletes the dispatcher. Per-test guest cost is unchanged by this feature (ordinary runs already boot one guest per test attempt). The explicit `overdrive-fwd-guest-suite` command is cited by `metal-run-isolation` and testing.md, which US-08 updates.
- O5: migration batch sizing (1-3 days each) is a hypothesis; the split rule above applies if DESIGN's estimate disagrees.
- Deferrals: no deferral language is used and no GitHub issue is created or proposed; every limitation above is an in-feature story. If the user wants any story removed from this feature, that becomes a deferral needing an approved issue.

## Wave: DISCUSS / [REF] DoR validation

Evidence is in each story's section. Columns: 01-04 as numbered; M = each of the six migration stories; 07; 08.

| Item | 01 | 02 | 03 | 04 | M (05a-06c) | 07 | 08 |
|---|---|---|---|---|---|---|---|
| 1 Problem in domain language | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 2 Persona specific (Marcus, extended) | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 3 3+ domain examples, real names | PASS (4) | PASS (3) | PASS (3) | PASS (3) | PASS (3 per batch family) | PASS (4) | PASS (3) |
| 4 UAT 3-7 scenarios incl. error path | PASS (7) | PASS (4) | PASS (4) | PASS (4) | PASS (4) | PASS (4) | PASS (4) |
| 5 AC derived from UAT, observable | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 6 Right-sized (1-3 days) | PASS | PASS | PASS (2-3 d) | PASS | PASS (pre-split, rule for further split) | PASS | PASS |
| 7 Technical notes | PASS | PASS | PASS | PASS | PASS | PASS | PASS |
| 8 Dependencies tracked | PASS | PASS (01) | PASS (01) | PASS (01) | PASS (01-04) | PASS (O3) | PASS (all) |
| 9 Outcome KPIs numeric | PASS (K1,K4) | PASS (K1) | PASS (K5) | PASS (K1) | PASS (K3,K5) | PASS (K2) | PASS (K5) |

US-01 has 7 scenarios, the upper bound of 3-7. Elevator Pitch is present on every story; every "After" names a runnable command with an observable result (US-07's exact failure text and lint command are DESIGN-pinned and marked as such). `job_id: J-PLAT-007` on all. Verdict: PASSED, with O1 as a DESIGN blocker on US-01's general shape, not a DoR failure. Peer review: iteration 1 returned `rejected_pending_revisions` (unsized migration stories, non-runnable lint command, non-observable pitches, missing error scenarios, a technical AC); this revision addresses each.

## Wave: DESIGN / [REF] Inputs read

PROPOSE mode, scope Application/components, OOP paradigm, all priorities (P1 and P2: O1-O5 and every question the dispatch listed). Every decision below is **APPROVED** by the user (`.claude/rules/design.md` § "Surface material design decisions"): the shape of DDD-1 and DDD-7 on 2026-11-13; U-1 to U-6 and N-1 to N-4 on 2026-11-14 (DDD-3, DDD-4, DDD-11, DDD-12, the 18-test template of DDD-1, no tracking issues, DDD-16, DDD-17, DDD-18, the conformance exemption); and all remaining decisions on 2026-11-14 (the DDD-1 details, DDD-2, DDD-5, DDD-6, DDD-8, DDD-9, DDD-10, DDD-13, DDD-14, DDD-15 and the Quint validation budget of about 2.5 minutes, 19 checks). Independent DESIGN review completed three rounds.

| Input | Read |
|---|---|
| This file (DISCUSS D1-D8, US-PLACE-01..09, O1-O5) | ✓ |
| `slices/slice-09-module-tests-share-one-guest.md` | ✓ |
| `slices/slice-01..08` | ⊘ not opened: their content is carried by the stories above |
| `spike/findings.md`, `spike/wave-decisions.md`, `spike/findings-module-shared-guest.md` | ✓ |
| Probe code `spike-scratch/metal-test-placement-module-guest/increment-a/` (`lib.rs`, `bin/broker.rs`) | ✓ (`tests/probe.rs` ⊘) |
| `crates/overdrive-testing-macros/` (`lib.rs`, `Cargo.toml`) | ✓ |
| `crates/overdrive-testing/src/guest_suite.rs` | ✓ partial: header, per-test path (`run_in_guest`, plans, attempt, lane dispatch, `run_dispatcher`); the batched `run_*` bodies and the 9 unit tests ⊘ |
| `guest_kit.rs` | ✓ header, errors, `runtime_staging_root`; `bin/fwd_guest_suite.rs` ⊘ (3-line `main` per findings) |
| `infra/metal/test-wrapper.sh` | ✓ whole |
| `infra/metal/test-placement.json` | ✓ binary-id keys and every lane/native entry shape; the 4,642 entries not read |
| `infra/metal/README.md`, `bootstrap.sh`, `xtask guest_kit_prepare.rs` | ⊘ grepped for references only |
| `xtask/tests/acceptance/metal_run_isolation/placement.rs` | ✓ outline |
| `.config/nextest.toml` | ✓ whole |
| GitHub issue #354 body and comments | ⊘ no shell in this dispatch; the DISCUSS summary stands in. |
| `brief.md`, ADR-0280 and the ADR index | ✓ headings, ADR-0280 whole; highest ADR number is 0281, new ones start at 0282 |
| `docs/product/journeys/declare-where-a-metal-test-runs.yaml` | ⊘ |
| `vendors/linux` | read for V-4 (`findings-owner-lifetime.md`, `028ef9c96e96`, "Linux 7.0"); DDD-8 cites `path:line` |
| Rules: CLAUDE.md, `design.md`, `testing.md`, `rust.md`, `development.md`, `domain-naming.md` | ✓ |

## Wave: DESIGN / [REF] Findings that reshape the DISCUSS premise

| ID | Finding | Evidence | Consequence |
|---|---|---|---|
| F-1 | **Placement has a fourth source: per-test guest requirements keyed by test name inside the dispatcher.** `nextest_guest_plan` and `cgroup_attempt_files`/`mtls_attempt_files`/`nested_resources` choose staged files, loaded modules, setup and cleanup scripts, memory and vCPUs from about a dozen hand-kept `Cell` tables and one literal test name (`DECLARED_LISTENER_TCP_CAPTURE_TEST`), matched by exact name or prefix. | `guest_suite.rs:3788-3794` (`selected_cell`), `:3829-3905`, `:4043-4074`, `:3961` | A rename of such a test silently drops its variant (the guest then lacks a tool or setup and the test fails by name, not silently, but a rename "with zero edits" no longer holds for them). DDD-5 moves each variant into a named profile on the attribute. The DISCUSS count of "three sources" is four. |
| F-2 | **Off Metal, guest-placed tests run their body on the host today** (no Metal context: the wrapper `exec "$@"`). The CI `integration` job (`cargo nextest run --workspace --features integration-tests`, profile `ci`, on Lima) and the pre-push `nextest` hook run these binaries. | `test-wrapper.sh:25-31`; `.github/workflows/ci.yml:442-464`; `lefthook.yml:267-268` | Declaring placement on the test changes nothing off Metal: those tests keep running in place in those lanes (DDD-11, user ruling 2026-11-14). |
| F-3 | **The table covers every test the Metal run executes, including default-lane unit and acceptance binaries** (`overdrive-core::acceptance`, proptest binaries, `compile_fail`, `xtask`). US-07's "unit-lane tests are not flagged" has no meaning once the table is gone unless something supplies the unit lane's placement. | binary-id keys in `test-placement.json` | DDD-4 (approved 2026-11-14): a per-binary host-pure default for host-only binaries and a per-module host-pure declaration for mixed ones; US-07 flags an unplaced unit-lane test. |
| F-4 | **An attribute proc-macro cannot be applied to a file module (`mod x;`) on stable Rust, nor to an inline module that contains one** ("non-inline modules in proc macro input are unstable"). The integration entrypoints use exactly `mod integration { mod a; mod b; }`. Confirmed by V-1 on rustc 1.95.0 (E0658 on `#[attr] mod x;`, `#[attr] mod outer { mod y; }` and an inner `#![attr]`). | `findings-module-declaration.md`, Rust issues #54727, #54726 | A module-level attribute is unusable for the repository's layout (V-1 confirms). The user approved (2026-11-13) a function-like declaration inside the module that the tests read by name resolution (DDD-1, DDD-7). |
| F-5 | **Lane placements are not "re-run this test in a guest".** The placed test (`guest_flow::real_kernel::…` in `overdrive-init`) is the in-guest body, run from a prepared in-guest binary (`target/guest-kit/in-guest/overdrive-init--overdrive-init`); an owner lane also runs a host-side script chosen by fixture name (six scripts in the dispatcher); the fixture (`integration::in_guest_lane::…`) is the host driver test. | `guest_suite.rs:4614-4747`, `INIT_GUEST_LANE_PLACEMENTS` | The lane scripts and the closed slot table move into `guest_lane` with the dispatcher's removal (DDD-13). |
| F-6 | **The dispatcher carries body-count assertions** (`CGROUP_DEFAULT_COMPLETE_BODIES`, `MTLS_RELAY_COMPLETE_BODIES`, …) that guard "every authored body was credited". They disappear with the batched path. | `guest_suite.rs:1462-1478` | The eager listing check (DDD-3) and the migration parity check (DDD-15) replace them. |
| F-7 | **`xtask` may not depend on an `overdrive-*` crate**, and the macro crate is one; `core` crates would also take a dev-dependency on an `adapter-host` proc-macro crate to declare host-pure tests. | `development.md` § "xtask is build / test / dev orchestration"; `Cargo.toml` of the macro crate | DDD-12 (approved 2026-11-14): the rule's command ignores dev edges. |
| F-8 | **`host-kernel-shared` already serialises whole guest-placed binaries** (`overdrive-cli`, `control-plane`, `worker`, `dataplane`, `init`, `fwd`, `netlink` integration binaries run with `max-threads = 1`). | `.config/nextest.toml` | A shared guest serving one test at a time (DDD-9) costs nothing in those binaries; queuing only arises in binaries outside a single-writer group. |

## Wave: DESIGN / [REF] Reuse Analysis

Contract shape per `.claude/rules/design.md` principle: **pure-function** = value in, value out, no effect; **bounded-change** = a declared mutation set (the delta) over a named universe; **unbounded-preservation** = must return a plan and never mutate (no component here has this shape: every read-only component is a pure-function core over a read-only shell). Every effect below is declared as a delta; every ambient read (environment variables, `NEXTTEST_BINARY_ID`) is a thin shell function over a pure core that tests feed directly (capability injection, not a god-object).

| Component | Existing | Verdict | Justification, contract shape, universe, assertion mechanism |
|---|---|---|---|
| `#[overdrive_test]` expansion | `overdrive-testing-macros` (skeleton, guest case only) | **EXTEND** | Grammar grows to five explicit forms plus the module form, and a function-like `shared_guest!` is added. pure-function: tokens in, tokens out; universe = the annotated fn (the declaration macro: the module's one const item). Assertion: trybuild-style fixtures if DISTILL chooses them (their call). |
| Group registry, profiles, per-group artifacts | `guest_suite.rs` plans, wrapper lists, dispatcher `main` | **EXTEND** into new module `placement::groups` | Replaces three group lists and about a dozen name-keyed tables with one closed registry. pure-function: closed data plus `const fn` validators, no effect. Assertion: constant evaluation at build time and one unit test per profile. |
| Per-test guest path (`run_in_guest`, attempt, `GuestRun`, plans) | `guest_suite.rs` lines 3423-4403 and helpers | **EXTEND** by moving to `placement::guest_run` | bounded-change: delta = one work directory under the run's runtime root and one VMM per boot; universe = that directory and that VMM. Every profile setup and cleanup script (including the `host-route`, `routed-route`, `legb-*` and `normal-gate` ones) runs inside the guest through the exec channel, never on the host (`guest.command`, `guest_suite.rs:3152-3154`). Same implementation the dispatcher wrapper path uses today. Assertion: the `OVERDRIVE_GUEST_KIT_TIMING` boot count and no run directory left after the run. |
| Guest kit (`GuestKit`, `GuestPeer`, `WorkDir`) | `guest_kit.rs`, `guest_peer.rs` | **EXTEND** | bounded-change, unchanged delta (one VMM and its work directory per boot). No new boot parameter; the exec channel sets the in-guest marker for a test's environment (DDD-10). `GuestPeer::exec(&mut self)` already enforces one exec at a time, which the owner inherits (ADR-0284). |
| Lane entry points and owner scripts | `guest_lane.rs`, dispatcher `guest_lane_dispatch` | **EXTEND** `guest_lane` | The six owner scripts and the closed slot table move in (DDD-13). bounded-change: one lane guest and its work directory per call; the host-side owner scripts keep the effect set they have today (this feature widens none). |
| Wrapper | `infra/metal/test-wrapper.sh` | **EXTEND** then shrink | Reads records; loses the table, group lists, `--guest-suite` mode and dispatcher branches. pure-function resolver (record bytes and test name to a placement or a typed refusal); bounded-change for the existing namespace effects, unchanged (delta = the run's namespaces and the final `exec`). |
| Shared-guest owner | none (ADR-0280 rejected a broker as out of its scope) | **CREATE NEW**: `OwnerCore` (pure) + owner binary (shell) | No existing alternative: nextest runs a process per test, so a guest shared by several tests needs a process that outlives any one of them. Probe evidence. `OwnerCore` is a pure-function state machine (events in, effects as data; it reads no clock and holds no handle); the shell is bounded-change (delta = one VMM, one socket, one lock file inside the run's runtime directory, plus the exec-bound and idle timers). Assertion: the Quint conformance driver over `OwnerCore` and the Metal scenarios over the shell. |
| Eager listing check | the wrapper's per-test refusal | **CREATE NEW** xtask subcommand `placement-lint` (approved by the user 2026-11-14, DDD-3) | The wrapper refuses only at run time on Metal, per test. pure-function over its inputs plus a read-only shell; universe = nextest's list output and the binaries' ELF sections; writes nothing (the migration-only `--emit` prints to stdout and is deleted with the table). |
| Module and binary host-pure declarations (`placement!`, `overdrive_test_default!`) | none | **CREATE NEW** (approved 2026-11-14, DDD-4) | Otherwise 3,855 per-test sites. pure-function: tokens in, one named item holding one record `static` out; universe = the declaration's own item; they reference no runtime crate. |
| Table, lookup, dispatcher binary, `--guest-suite` mode, `OVERDRIVE_METAL_TEST_DISPATCHER`, body-count constants | present | **DELETE** (US-08) | Replaced as above. |
| `overdrive-guest-peer`, `guest-kit-prepare`, `bootstrap.sh` grammar | present | **EXTEND** | Prepare builds the owner binary where it built the dispatcher; the ordinary grammar swaps the dispatcher bin for `placement-lint`. `guest-kit-prepare`: bounded-change, delta = the owner binary under `target/guest-kit/` beside the artifacts it already builds. |
| Ambient reads: `inside_guest()`, `in_metal_run()`, `NEXTTEST_BINARY_ID` | partly present (`inside_guest`) | **EXTEND** | pure-function cores `metal_run_context_present(run_id, execution_class, source_root)` and the scope-key function (§ Shared-guest scope); the zero-argument `inside_guest()` and `in_metal_run()` and the binary-id read are read-only shell edges feeding them, so tests inject the inputs to the cores and never mutate process environment. |

## Wave: DESIGN / [REF] Components and ports

Style: modular, functional core and imperative shell, ports and adapters where a port has two implementers. Paradigm OOP per CLAUDE.md. Technology: Rust, existing crates, `syn`/`quote`/`proc-macro2` (already workspace deps, MIT/Apache-2.0), `python3` in the wrapper (already required), `libc` for `pidfd_open`/`unshare`/`flock` (already a workspace dependency of the testing crates). No new third-party crate.

| Component | Responsibility | Crate or path | Driving port | Driven ports |
|---|---|---|---|---|
| Placement attribute and declarations | declare placement, emit record, gate guest forms; `placement!` and `overdrive_test_default!` emit host-pure records | `overdrive-testing-macros` | the attribute grammar (DDD-1) and the two declaration macros (DDD-4) | none |
| Placement registry | closed group and profile set, recipes | `overdrive-testing::placement::groups` | `GuestPlacement::declare` (const) | none |
| Guest run | one isolated guest per test | `overdrive-testing::placement::guest_run` | `run_in_guest`, `inside_guest` | `GuestKit` |
| Shared-guest client | elect or attach, send the attach request | `overdrive-testing::placement::shared` | `run_in_guest` with `Sharing::Module` | election lock file, socket |
| Owner core | queue, one in flight, discard after failure, generations, exec bound, idle exit | `overdrive-testing::placement::owner_core` | `OwnerCore::handle(Event)` | none (effects are data) |
| Owner shell | executes effects against the guest, serves the socket | `overdrive-shared-guest-owner` bin in `overdrive-testing` | process entry | `GuestKit`, socket, exec-bound and idle timers |
| Lane run | in-guest body plus owner scripts | `overdrive-testing::guest_lane` | `run_lane` | `GuestKit` |
| Wrapper | resolve placement, apply isolation | `infra/metal/test-wrapper.sh` | nextest per-test wrapper call | namespaces, exec |
| Placement check | list, resolve, refuse unplaced and conflicting, check lane fixtures exist | `xtask placement-lint` | CLI | `cargo nextest list`, ELF sections |

Principle 13 (Earned Trust). Each dependency and the lie it can tell: (a) the **linker** can drop the record section: the test then has no record and is refused (`test_unclassified`), and `placement-lint` names it before any guest boots; the macro keeps the record referenced, and an xtask acceptance fixture reads a real linked test binary with both readers. (b) The **host kernel's PID namespace teardown** is what makes the VMM die with the owner: V-4 proves it on the Metal kernel. The **owner binary** can be absent (an unprepared kit): the electing client checks its contract path before it takes the election lock and fails as `LaneNotPrepared` naming it, so no lock is held and nothing starts. `unshare(CLONE_NEWPID)` is refused only to a caller without the privilege; the run is root, so no refusal is designed for. (c) The **run runtime directory** can be absent or not owned: the client refuses to elect without a valid `OVERDRIVE_METAL_RUN_ID` (the existing `runtime_staging_root` rule). (d) The **guest** can lie about readiness: `GuestKit::provision` and the existing boot-bound checks are the probe; a guest that cannot run its profile's setup fails the attach with the stage named. (e) The **in-guest test** can hang: the owner bounds every exec (DDD-16) and discards the guest at the bound, so a hung test cannot hold a shared guest past it.

## Wave: DESIGN / [REF] Decisions

Every row is APPROVED by the user. The user approved DDD-3, DDD-4, DDD-11, DDD-12 and the 18-test template of DDD-1 on 2026-11-14 (U-1 to U-5), ruled no tracking issues (U-6), approved DDD-16 to DDD-18 and the conformance exemption (N-1 to N-4), and approved all remaining decisions on 2026-11-14.

| ID | Decision (one sentence) | Status | Needs |
|---|---|---|---|
| DDD-1 | Placement is declared by `#[overdrive_test]` with five explicit per-test forms plus a no-argument form that reads the module's `shared_guest!(guest = "<g>"[, profile = "<p>"])` declaration (grammar in § Interface contracts). **Shape APPROVED by the user 2026-11-13.** Details APPROVED 2026-11-14: the macro never inserts `#[test]`; the attribute sits below `#[quint_test]`/`#[quint_run]`; the `()`-return rule binds the guest forms only; the hidden const is named `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE`; the conflict rule between a module declaration and an explicit per-test placement is explicit-overrides. | Shape: APPROVED 2026-11-13; the 18-test `macro_rules!` template (one attribute inside the template, T1): APPROVED (user, 2026-11-14, U-5); the details: APPROVED (user, 2026-11-14) | none |
| DDD-2 | The placement record is a NUL-terminated field sequence in the `overdrive_test_placement` ELF section; the wrapper is its only run-time reader and `placement-lint` reads it through the wrapper; until the table is retired a record wins over a table entry (the migration commit deletes the entry with the attribute, and the parity check catches divergence). | APPROVED (user, 2026-11-14; ADR-0282) | none |
| DDD-3 | The unplaced-test guard stays in the wrapper and is also run eagerly over every listed test by a new `xtask placement-lint` (ADR-0288), wired into CI and pre-push beside `dst-lint` and `conformance-lint`, landing right after the lane and launcher forms (slice 03; its fixture-existence check reads `lane_operator` / `lane_owner` records, which that slice introduces). A test counts as placed when it has a record, an applicable module or binary declaration, or (until retirement) a table entry; the lint also checks that every lane fixture exists as a test in the binary that provides it. | **APPROVED (user, 2026-11-14, U-1, including the fixture-existence extension)** | none |
| DDD-4 | Host-pure placement of the 3,855 host-pure and unit-lane tests once the table is gone: both a per-binary default (`overdrive_test_default!(host_pure)`, only in a binary with no guest-placed, lane-placed or native-launcher test) and a per-module declaration (`placement!(host_pure)`, a module-scoped record applying to the tests of exactly that module); precedence is the test's own attribute, then its module's declaration, then the binary default; a child module does not inherit a parent's declaration; a binary with a default and a non-host-pure record is refused as `placement_conflict` (ADR-0287). | **APPROVED (user, 2026-11-14, U-4)** | none |
| DDD-5 | The guest group set lives once in the guest kit as a closed registry with named profiles (21 environments over 8 groups, names in § DDD-5), each profile carrying its exec bound; the attribute is checked at compile time by const evaluation against it; the wrapper holds no group list; test-name-keyed tables become profiles, among them one `quint` profile that stages the Quint runtime and the `specs/quint` tree (ADR-0294). | APPROVED (user, 2026-11-14) | none |
| DDD-6 | The guest kit is split into `placement::{groups, guest_run, shared, owner_core}` plus the existing `guest_kit`/`guest_lane`; the public surface is the contract in § Interface contracts. | APPROVED (user, 2026-11-14) | none |
| DDD-7 | Sharing is opted into once per module by `shared_guest!`; the sharing scope is the declaration (the module that holds it, recorded in the declaration itself), its group and profile within one test binary file. A child module reaches the declaration only by an explicit glob import and then shares the declaring module's guest. **Shape APPROVED by the user 2026-11-13.** | APPROVED 2026-11-13 (shape) | none |
| DDD-8 | One owner process per sharing scope per run, elected by an exclusive lock and reached through a socket under the run's runtime directory (ADR-0283); the owner runs as init of its own PID namespace so the VMM dies with it (ADR-0290). V-4, V-5 hold. | APPROVED (user, 2026-11-14; ADR-0283, ADR-0290) | none |
| DDD-9 | The owner serves one test at a time in arrival order and never cancels a running test (ADR-0284). A guest that ran a failed test, was lost, or hit its exec bound is discarded (ADR-0291). | APPROVED (user, 2026-11-14; ADR-0284, ADR-0291) | none |
| DDD-16 | **APPROVED (user decision N-1, 2026-11-14).** Every shared exec has a bound: the profile's cell bound. At the bound the owner discards the guest and the test gets a typed `GuestExecution` failure with stage `exec_bound`. | APPROVED (ADR-0292) | user |
| DDD-17 | **APPROVED (user decision N-2, 2026-11-14).** A shared owner exits when it has nothing queued, nothing running and no attached client for the idle period; its guest dies with it. | APPROVED (user, 2026-11-14; ADR-0293) | user |
| DDD-18 | **APPROVED (user decision N-3, 2026-11-14).** A client that goes away while its test runs taints the guest: when that exec ends, the guest is discarded whatever the verdict. | APPROVED (ADR-0291) | user |
| DDD-10 | `inside_guest()` is true exactly when the process environment carries the marker the guest exec channel sets (`OVERDRIVE_TEST_IN_GUEST`); a spoof cannot change an outcome (§ DDD-10). | APPROVED (user, 2026-11-14) | none |
| DDD-11 | Off a Metal run (no Metal run context) a guest-placed or lane-placed test runs its body directly in the current process, as today; on a Metal run its body never runs on the host. No Metal run context is the wrapper's own predicate: none of `OVERDRIVE_METAL_RUN_ID`, `OVERDRIVE_METAL_EXECUTION_CLASS`, `OVERDRIVE_METAL_SOURCE_ROOT` is set and non-empty. | **APPROVED (user, 2026-11-14, U-2, option B; ADR-0286)** | none |
| DDD-12 | Placement declarations may take a dev-dependency on `overdrive-testing-macros` from `xtask` and from `core`-class crates; the rule's command becomes `cargo tree -p xtask -e normal,build \| rg overdrive-` (dev-dependencies allowed); `xtask` also takes `overdrive-testing` as a dev-dependency for its guest-placed test. The orchestrator amends `.claude/rules/development.md` (ADR-0295). | **APPROVED (user, 2026-11-14, U-3)** | none |
| DDD-13 | Lane kinds self-dispatch through `guest_lane` the way guest placements do; the owner scripts and slot table move out of the dispatcher. | APPROVED (user, 2026-11-14) | none |
| DDD-14 | Retirement removes the table, wrapper lookup, `--guest-suite` mode, dispatcher binary, `OVERDRIVE_METAL_TEST_DISPATCHER`, the explicit batched command and the body-count constants, and rewrites the docs that cite them. | APPROVED (user, 2026-11-14) | none |
| DDD-15 | Migration is a scripted rewrite per batch with a parity check against the table snapshot; slice order puts the listing check before the migrations. | APPROVED (user, 2026-11-14) | none |

### DDD-1 — Attribute surface (question 1)

**Approved shape (user, 2026-11-13).** `#[overdrive_test]` carries per-test placement; sharing and the guest group and profile of a module are declared once, by a plain function-like macro call inside the module, which the per-test attribute reads:

```rust
// top of the module file, once
overdrive_testing::shared_guest!(guest = "cgroup-default");

#[overdrive_test]            // no arguments: the module's declaration
async fn a_test() { … }
```

`shared_guest!` expands to a module-private item (a `const` named `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE`, built in constant context, under `#[allow(non_upper_case_globals)]`). A no-argument `#[overdrive_test]` expands to code naming that item by plain path, so ordinary name resolution finds the declaration of the module the test sits in; with no declaration in scope the test does not compile. The ELF record is built by constant evaluation from the same const, so the wrapper sees the placement the macro declared (V-1: held, `findings-module-declaration.md`).

**Const name (APPROVED, 2026-11-14).** A proc macro cannot see the enclosing module's items, so it cannot emit a `compile_error!` for a missing declaration; absence surfaces only as rustc's E0425 "cannot find value `<name>` in this scope", and the const's name is the whole message (V-1 b2). The earlier name `__OVERDRIVE_MODULE_PLACEMENT` reads as an internal symbol and tells an author nothing; `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE` makes the compile error read as the instruction (V-1 b2 recorded exactly this text). A trait or `diagnostic::on_unimplemented` cannot improve the message because name resolution fails before a bound is evaluated. The name is part of the contract (authors see it in build output; a fixture asserts it). The mixed case is the price of the sentence and is covered by the `allow`.

**What the macro emits (APPROVED, 2026-11-14; V-2).** The macro never inserts `#[test]`. All 4,672 test-introducing sites in the workspace already carry a test attribute (`#[test]`, `#[tokio::test]`, a `proptest!` block's `#[test]`, `#[quint_test]`, `#[quint_run]`), and "insert when absent" ran four stackings twice (`#[test]` above ours, `#[test] #[should_panic]` above ours, `#[quint_run]` and `#[quint_test]` above ours, in V-2). With no insertion every stacking yields one libtest test and one record, and attribute order stops mattering, except for the two rules below. The existing "add `#[test]` when absent" logic in `overdrive-testing-macros` is removed with this decision.

- **Position.** `#[overdrive_test]` is written below `#[quint_test]` and `#[quint_run]` (and below `#[serial]`, `#[ignore]` and `#[should_panic]` by convention). Above `#[quint_*]` the guest forms cannot compile: the quint macros take `-> impl Driver` and replace the fn with a `()` fn, so a body-wrapping macro above them is rejected (`error: tests must return ()`, `quint-connect-macros-0.1.2/src/internal/test.rs:59-85`).
- **Return type.** The `()`-return requirement applies to the **guest forms** only (module form, `guest`, `lane_operator`, `lane_owner`), because they wrap the body. `host_pure` and `native_launcher` leave the fn untouched and emit the record as a sibling `static` in an anonymous const, so they accept a `Result`-returning test (6 host-pure tests in the workspace return `Result`: `overdrive-dataplane/tests/integration/build_rs_artifact_check.rs:29`, `overdrive-worker/src/probe_runner/tcp_prober.rs:210,219`, `http_prober.rs:528,537`, `xtask/src/metal_run_isolation_conformance.rs:1410`). V-2 ran this `record_only` form and the body-wrapping `strict` form in every order: 31/31 and 11/11 passed, `running 1` everywhere. The 352 `quint_*` tests are host-pure except 5 guest-placed ones (`overdrive-gateway` `application::quint_gateway_application::*`), which take the attribute below `#[quint_*]` as a guest form and so also need a `()` fn, which the quint macros already produce.
- **Nested modules** do not inherit a declaration (V-1 b3: E0425 in a child module). A child module that writes `use super::*;` (or names the const by path) does bring the parent's private const into scope, because a child can see its parent's private items; a proc macro cannot forbid that. The design therefore makes the outcome correct instead of rejecting it: the declaration const records the module path of the module that holds it (`module_path!()` captured in `shared_guest!`), and the sharing scope is built from that recorded path, not from the reading test's own module. A glob-importing child therefore shares its parent's declaration and its guest (one declaration, one guest); a child with no import is E0425 as before. A module that nests tests declares in each module that holds tests, or imports deliberately to share.

**The 18-test `macro_rules!` template (APPROVED by the user, 2026-11-14, U-5, option T1).** `equivalence_cases!` (`overdrive-dataplane` integration, `gateway_relay_equivalence.rs:616`) generates 18 guest-placed tests from one template. One attribute inside the `macro_rules!` template places all 18. DDD-15's stop rule ("stops on a macro-generated test") does not apply to this one template: the migration script special-cases it (it recognises the template, adds the attribute inside it, and deletes the 18 table entries), and the parity check proves the same 18 names carry the same placement. Rejected: **(T2)** expanding the template into 18 hand-written tests (churn for no placement gain); **(T3)** leaving the 18 in the table until the retirement slice, which blocks the table's deletion.

Options considered. **(A) one attribute with per-test forms, a per-test `shared` flag** (the earlier recommendation): rejected by the user; sharing is a property of the module, and a flag repeated on every test lets one test forget it or name a different profile, so the module quietly splits into several guests. **(C) an attribute on the `mod`**: rejected; on stable Rust a proc-macro attribute on `mod x;` (and on an inline `mod` containing `mod y;`, which is the repository's `mod integration { mod a; mod b; }` entrypoint layout) fails with E0658 (`proc_macro_hygiene`, nightly), and an inner custom attribute needs `custom_inner_attributes`; the workspace stays on stable (F-4, V-1). **(B) a separate attribute per kind** (`#[host_pure]`, `#[guest(...)]`): five macros and five error surfaces for one concept; rejected. Mapping of the five table kinds, per-test explicit forms: `host-pure` → `host_pure`; `native-launcher` → `native_launcher`; `guest-group` → `guest = "<group>"[, profile = "<profile>"]`; `guest-lane-operator` → `lane_operator = "<fixture>"`; `guest-lane-owner` → `lane_owner = "<fixture>"`. Module-level sharing (D8) → `shared_guest!` plus the no-argument form on each test of the module. Explicit `guest = …` is always an isolated guest (no `shared` argument exists).

**Conflict rule (APPROVED, 2026-11-14).** What an explicit per-test placement means inside a module that has `shared_guest!`: **(O, recommended) the explicit form overrides**: the test keeps exactly what it names (isolated guest, `host_pure`, a lane), and only tests with the no-argument form are shared. **(E) conflict is a compile error.** (E) cannot be built from name resolution: the explicit form has no way to observe that a sibling declaration exists, and having each explicit test define an item of the same name collides with other explicit tests in the module. It would need a module-level record for `shared_guest!` and a wrapper and `placement-lint` check that reports a conflict, which is reader logic beyond the precedence rule of DDD-4. (O) costs nothing, keeps every placement readable on the test that has it, and lets a `host_pure` or lane test live in a module of shared-guest tests. Its risk is an author who writes `guest = "<same group>"` in a shared module expecting sharing and gets an isolated guest: always safe (more isolation, never less than declared) and visible in the `OVERDRIVE_GUEST_KIT_TIMING` boot count.

**Module and binary host-pure declarations (DDD-4, APPROVED by the user, 2026-11-14, U-4).** Both forms exist; § DDD-4 holds the rules and § Interface contracts the grammar and record. `placement!(host_pure)` is the module-scoped record variant (M2): it emits a module-scoped record that the wrapper and `placement-lint` apply to every test of that module without a per-test attribute; unlike `shared_guest!` it does not make a bare `#[overdrive_test]` read it, so a plain `#[test]` is placed by it. A module with no declaration and tests with no attribute stays `test_unclassified`. The read-only variant (M1, tests still carry a bare attribute) is rejected: it is the every-test-declares option with a shorter spelling.

### DDD-2 and DDD-3 — Marker, ambiguity, guard (question 2, O1, O3)

O1 resolved: **keep the ELF-section record** (ratified). Alternatives in ADR-0282. A test with both a record and a table entry is placed by the record (the skeleton's rule, kept): the table clause is transitional, the migration commit adds the attribute and deletes the entry together, and the parity check (DDD-15) catches a divergence. There is no version field: the macro, the wrapper and the lint come from one checkout and nothing persists, so a version token would defend a skew that cannot occur.

Replacement for `test_unclassified` once the table is gone: the wrapper's refusal stays the exact, last line of defence at run time; it is the same function over the records (the test's own, its module's declaration and the binary default of DDD-4). The early check is `placement-lint`. Chosen (L-A): an xtask subcommand over `cargo nextest list --message-format json` plus the ELF records: exact (no guess about macro-generated tests) and runs in the lane that already compiles the test binaries. Rejected: **(L-B)** an xtask acceptance test that shells nextest (nested cargo inside a test, which the guest-kit rules forbid for lock contention); **(L-C)** a syntactic scan of test sources (misses tests generated by `proptest!`, `quint_test` and similar, and cannot see the linker dropping a record). The reader of the record exists once: the wrapper's Python reader, which `placement-lint` invokes through `test-wrapper.sh --check-placement` with the list on stdin, so there is one parser for the format. `--check-placement` is handled **before** the wrapper's no-Metal-context `exec "$@"` (`test-wrapper.sh:25-31`), because the lint runs on CI and developer hosts that have no Metal context; and the lint lists tests with `integration-tests` and `kvm-tests` enabled (without `kvm-tests` the 140 tests behind it, V-7, would not be listed and would escape the check). The invocation is pinned in § `placement-lint`: `kvm-tests` is declared by 5 of the 37 workspace members, so the feature set is formed per package from `cargo metadata`, and the lint runs on Linux through the routing of the nextest steps. No CI or pre-push lane builds with `kvm-tests` today, so the lint adds one test-binary compile to each lane that runs it (budget in G-4). The subcommand, its grammar entry in `bootstrap.sh`, and its wiring into CI and pre-push beside `dst-lint` and `conformance-lint` are **APPROVED by the user, 2026-11-14 (U-1)**. The approved extension: `placement-lint` also reads every listed binary's placement records and checks that the fixture named by each `lane_operator` / `lane_owner` record exists as a listed test of the binary that provides it, so a misspelled fixture fails before a Metal run; the run-time failure naming fixture and test remains as the lane run's own check.

O3 resolved: the check needs no per-binary scope. Its rule, "a listed test is placed when it has a record, an applicable module or binary declaration, or (until retirement) a table entry", is true at every migration stage, so it can land before any migration and the table clause is deleted in the retirement slice. Suggested order: 01 (done) → 02 → 03 → **07** → 04 → 09 → 05 and 06 → 08.

### DDD-4 — Host-pure and unit-lane scale (F-3)

**Decision (APPROVED by the user, 2026-11-14, U-4): both a per-binary default and a per-module declaration.** Counts for context: 66 host-only binaries (2,222 tests) and 21 mixed binaries (1,635 host tests).

- **Binary default.** `overdrive_test_default!(host_pure)`, written once at the root of a test binary; only `host_pure` is accepted. It applies to every test of the binary that has neither its own attribute nor an applicable module declaration. It is allowed only in a binary with no guest-placed, lane-placed or native-launcher test; a binary that has the default **and** any non-host-pure record is refused as `placement_conflict` by the wrapper and by `placement-lint`. It serves the host-only binaries with one line each.
- **Module declaration.** `placement!(host_pure)` (the module-scoped record variant, M2), written once in a module; only `host_pure` is accepted. It emits a module-scoped record that the wrapper and `placement-lint` apply to every test of that module without a per-test attribute. It serves the mixed binaries: the host-pure modules declare, the guest-placed tests carry their own attributes. A module with no declaration, whose tests carry no attribute and whose binary has no default, stays `test_unclassified`.
- **Precedence.** A test's own attribute (including a bare `#[overdrive_test]` reading `shared_guest!`) > its module's declaration > the binary default. A module may declare `placement!(host_pure)` and contain a test with an explicit guest, lane or `native_launcher` attribute: the attribute wins and no conflict arises. A module may carry both `shared_guest!` and `placement!(host_pure)`: bare `#[overdrive_test]` tests share the guest, tests with no attribute are host-pure.
- **Nesting.** A module declaration applies to the tests whose module path equals the declaring module's, exactly. A child module does not inherit a parent's declaration and declares its own, as with `shared_guest!` (DDD-7): one rule for every module-scoped declaration, resolved by the test's name minus its last segment, with no prefix-and-nesting logic and no ambiguity about which module's record wins. The cost is one declaration per module that holds tests, which for a file-module layout (`mod integration { mod a; mod b; }`) means one in `a` and one in `b`. The declaration count per module is not measured here; the migration script counts it.
- **Duplicates.** Each declaration macro emits one **named** item (`placement!` a module-level const, `overdrive_test_default!` a crate-root const), so a second `placement!` in one module or a second default in one binary is rustc's E0428 at build time; no reader refusal exists for it. A test resolved by a module declaration or the binary default that also has a table entry is placed by the declaration (as for a record: the table clause is transitional, deleted at retirement, and the migration deletes the entry with the declaration). A `placement!` or `overdrive_test_default!` argument other than `host_pure` is a build error.
- **Rejected.** Every test declares (about 4,640 sites; every future unit test needs the attribute). A binary default that allows mixing (a forgotten declaration in a mixed binary silently runs on the host, the failure the guard exists to prevent). A read-only module declaration (M1).

US-07's "unit-lane tests are not flagged" scenario becomes "a unit-lane test is placed by a module or binary declaration and flagged otherwise".

### DDD-5 — Group registry, profiles, per-group artifacts (questions 1 and 4, F-1)

SSOT: `overdrive_testing::placement::groups::GuestGroup` (closed enum of the eight groups) and, per group, a table of **profiles**. Compile-time validation without a second list: the macro (an explicit guest form, or `shared_guest!`) emits a `const` that calls `GuestPlacement::declare(group, profile, sharing)`, a `const fn` that panics at constant evaluation on an unknown group or profile, so a typo is a build error and the macro crate holds no group list. **Diagnostic text (V-11, `spike/findings-const-diagnostic.md`):** a `panic!` in constant context cannot format a runtime `&str` (E0015), but the proc macro sees the user's literal, so it emits a const that panics with a message built at expansion time by `concat!`. Pinned text: `unknown guest group `<value>`` and `unknown guest profile `<value>` for group `<group>``; the message names the offending value and lists no known set (US-01 AC d requires only the value), so the macro crate needs nothing from the registry. The macro emits that const with `quote_spanned!` on the user's literal, so rustc's E0080 error points at the offending argument. `GuestPlacement::declare` keeps fixed-text panics (`unknown guest group`, `unknown guest profile for this group`) as the floor for a caller that does not go through the macro. The profile message is the same pattern, unprobed. (A proc-macro crate cannot export types, and `overdrive-testing` depends on it, so the registry cannot live there.) The wrapper does not validate groups. Per-group artifacts (US-04): each (group, profile) has one recipe in the registry, the single place a copy list is declared. The recipe's content (tools, modules, boot-time setup, per-test setup and cleanup, memory, vCPUs, CID slot) is the crafter's, derived from the V-3 inventory (`findings-validation.md`); the contract pins only the group and profile names, the selection semantics, and that every profile carries an **exec bound** (DDD-16). F-1's name-keyed variants become profiles selected on the attribute (`profile = "<name>"`; omitted means the group's base profile). Alternatives: **(a) keep test-name-keyed tables as library data**: a renamed test silently loses its variant; **(b) make each variant a new group**: changes the group vocabulary the out-of-scope list protects and multiplies CID slots. The profile names are derived from the inventory in V-3 (`findings-validation.md`), which re-implements the dispatcher's per-test selection over all 818 guest-group table entries; every test lands in exactly one environment. A shared guest's key includes the profile, because profiles differ in loaded modules and setup.

**Closed registry names (APPROVED, 2026-11-14): 21 environments over 8 groups** (the 20 of the V-3 inventory plus `quint`). The contract is the group and profile names and what each selects; recipes (tools, modules, scripts, memory, vCPUs, CID slots) are the crafter's, taken from the V-3 inventory (`findings-validation.md`, which holds the per-profile settings and the group constants). Each profile's **exec bound** is its existing cell bound: `CELL_BOUND` (300 s, `guest_lane.rs:134`) for every profile except the nested ones, which keep the bound they already carry (`NestedVmGuestResources::STANDARD` 600 s, `nested-cohort` 1,200 s, `guest_suite.rs:298, 316, 1617-1628`).

| Group | Profile | Tests | What it selects (one phrase) |
|---|---|---|---|
| probe | `base` | 6 | probe tools, no modules |
| core | `base` | 85 | nf_tables and tls modules |
| unload | `base` | 4 | forwarder module only |
| ulp | `base` | 1 | tls module |
| stress | `base` | 1 | 16 vCPU |
| nested-boundary | `base` | 7 | nested-KVM substrate |
| mtls-relay | `native` (default) | 124 | dataplane staging, native modules and native setup |
| mtls-relay | `relay` | 8 | the same staging with only tls loaded (`integration::mtls_inbound_guest_relay::`) |
| cgroup-default | `base` | 479 | the cgroup-default environment |
| cgroup-default | `quint` | 5 | `base` plus the Quint runtime and the `specs/quint` tree (the five `overdrive-gateway` `application::quint_gateway_application::` tests) |
| cgroup-default | `dummy-link` | 16 | `base` plus the dummy module |
| cgroup-default | `host-route` | 2 | `dummy-link` plus route setup and cleanup (cleanup failure fails a passing test) |
| cgroup-default | `legb-tools` | 2 | `base` plus sysctl |
| cgroup-default | `legb-audit` | 3 | `legb-tools` plus LEGB setup and audit |
| cgroup-default | `routed-route` | 3 | `base` plus routed-route setup and cleanup |
| cgroup-default | `normal-gate` | 16 | `base` plus the control-plane native setup and cleanup |
| cgroup-default | `nested` | 12 | nested-KVM substrate |
| cgroup-default | `nested-cohort` | 2 | `nested` with 12,821 MiB |
| cgroup-default | `nested-launch` | 16 | `nested` plus the launched VMM and node setup |
| cgroup-default | `nested-launch-rustc` | 13 | `nested-launch` plus the Rust sysroot |
| cgroup-default | `nested-launch-lane` | 12 | `nested-launch` plus the prepared `overdrive-init` test binary (`integration::in_guest_lane::`) |

Count checks. `normal-gate` is 16 because `CONTROL_PLANE_NORMAL_NATIVE` (`guest_suite.rs:1332-1403`) lists 18 rows, two of which are the dead rows below; the constant `CONTROL_PLANE_NORMAL_NATIVE_BODIES = 20` (`:1524`) is the batched path's expected-passing-body total, not a row count, and is deleted with the batched path (F-6), so it constrains no profile. The table above sums to 817 tests against V-3's 818 table entries: one entry is unaccounted for in the V-3 record; the migration script's per-entry parity check (DDD-15) reconciles it, and DISTILL confirms it before any migration batch.

The 1-test `declared-listener-diagnostic` variant (the literal `DECLARED_LISTENER_TCP_CAPTURE_TEST`) only prints `nft --version; uname -r`, and nft is staged in every cgroup-default guest, so it folds into `cgroup-default` / `base` (counted there); no profile is minted for it. The lane kinds keep their own placement data (`INIT_GUEST_LANE_PLACEMENTS`, DDD-13). The profile is selected by `profile = "<name>"` on the explicit form or the module declaration; omitted means `base` (`native` for `mtls-relay`).

**Quint profile (APPROVED, 2026-11-14).** `QUINT_GUEST_MODULES` (`fwd_guest_suite.rs:15`, keyed by binary and module prefix) stages the Quint runtime and one model file for 5 gateway tests. Decision: one `quint` profile of `cgroup-default` stages the Quint runtime plus the `specs/quint` tree, and the five tests select it (`profile = "quint"`). The tree is measured: 3.9 MB in total, of which the `.qnt` files are 856 KB, so staging it whole costs less than the per-test model-file argument and its record field, build error and staging step would add. Rejected: keeping `QUINT_GUEST_MODULES` as library data (the same name-keyed drift as F-1); a model-file argument on the test (a second declaration surface to avoid staging 3.9 MB); a profile per spec (an environment per model file).

**Migration findings from the inventory (not design decisions).** (1) Two `CONTROL_PLANE_NORMAL_NATIVE` rows match no placed test and are dead rows: `workload_netns_provision::provision_creates_and_idempotently_converges_per_workload_netns` and `…::provision_injects_node_local_responder_into_netns_resolv_conf`; they are deleted with the dispatcher, not migrated. (2) 9 non-host-pure table entries match no `fn <name>` anywhere in the repository (6 in `overdrive-control-plane::integration` look stale, 3 belong to a fixture crate, V-2); the migration script reports them and the user rules on each (delete or find the renamed test), because a test that is named in the table but does not exist cannot carry an attribute.

### DDD-6 — Guest-kit library structure (question 5, O2)

O2 resolved: keep `overdrive-testing` as the home (same crate class, already holds `guest_kit`, `guest_lane`, the peer binary); split `guest_suite.rs`, delete its batched half, and re-home the per-test path under `placement`. A new crate for the owner would force a third crate to share `GuestKit`; rejected. The macro re-export stays `overdrive_testing::overdrive_test`. Public surface is pinned below.

### DDD-7 — Sharing opt-in (D8)

Sharing is declared once per module by `shared_guest!` (user-approved shape, 2026-11-13; DDD-1 holds the options and the rejections: a per-test `shared` flag, and an attribute on `mod`, which stable Rust refuses). Only a test that carries `#[overdrive_test]` with no arguments inherits the declaration; a plain `#[test]` does not, so the explicitness of "this test is placed" stays on the test. The declaration is a module-private item found by plain path, so a child module does not inherit its parent's; a module whose tests are in `mod integration { mod a; mod b; }` declares in `a` and in `b` (a child that glob-imports its parent's declaration shares the parent's guest; DDD-1 § Nested modules). Scope key = `(binary id, declaring module path recorded in the declaration, group, profile)` (§ Shared-guest scope). Nothing is shared across declarations, binaries or builds of a binary.

### DDD-8 — The owner (question 3)

Who owns the guest: the **owner process**, one per scope per run, started by the first arrival. Election: an exclusive `flock` on `<runtime>/shared/<scope>.lock`, released by the kernel when the holder dies; after acquiring the lock the elector tries to connect once more and, if the socket is connectable, attaches and releases the lock (an owner already exists); otherwise it starts the owner, waits until the socket is connectable (bounded by the existing `SCRIPTED_BOOT_BOUND`), then releases the lock. A later arrival finds the socket and attaches; if the connect is refused it takes the lock and elects. The owner calls `setsid` before it serves, so a group signal to the elector's process group (nextest's slow-timeout kill) does not reach the owner. Lifetime: the owner calls `unshare(CLONE_NEWPID)` and forks; the child is the owner proper and so is init of its namespace, and the kernel kills every process of a PID namespace when its init dies. That makes "the VMM never outlives its owner" structural and keeps the production launcher unchanged. Kernel basis (read in `vendors/linux` at `028ef9c96e96`, "Linux 7.0"; V-4, `findings-owner-lifetime.md`): `copy_pid_ns` applies the new namespace to children only (`kernel/pid_namespace.c:175-182`); the first child becomes the namespace's init, marked `SIGNAL_UNKILLABLE` (`kernel/fork.c:2239, 2425-2428`); when the init's last thread exits, `exit_notify` → `forget_original_parent` → `find_child_reaper` → `zap_pid_ns_processes` runs (`kernel/exit.c:988, 743, 706, 597-624`); `zap_pid_ns_processes` SIGKILLs every other task in the namespace (`kernel/pid_namespace.c:225-234`), reaps them and waits until all are freed (`:270-275`), so the init becomes a zombie only after its VMM is dead; a SIGKILL from an ancestor namespace is delivered through `SIGNAL_UNKILLABLE` (`kernel/signal.c:1187-1191, 94-96`). Measured: SIGKILL of the owner during a 40 s guest exec left no VMM, owner and VMM gone in at most 38.6 ms over 13 cycles; the production launcher worked inside the namespace (election to first exec 1.5-2.5 s, unchanged). End of life: an owner ends by itself when it is idle (DDD-17); it holds no handle on the nextest process, and no wrapper-exported variable or pidfd opened before the namespace exists is needed. The run's transient unit (ADR-0281) ends any leftover descendant at the end of the command.

Implications of V-4 and V-5 (informative notes for the crafter; the contract is the behaviour stated in § Shared-guest scope and client protocol): (1) the owner reads its host pid from `/proc/self`, never `/proc/<getpid()>`, because `getpid()` is 1 inside the namespace (a first attempt did this and sent `kill(1, SIGKILL)`, which the kernel dropped, `kernel/signal.c:91-92`); (2) the owner never calls `waitpid(-1)`, which would steal the launcher's VMM wait status; (3) a client attached to a killed owner sees end of stream (an empty reply), not an error, and the client protocol maps EOF to the owner-loss failure; (4) election and rendezvous need no change: 8 simultaneous first arrivals under `flock` gave exactly 1 owner (without the lock, 8), clients in 7 other per-test network namespaces connected, the socket path was 52-58 bytes (limit 108), and `/run/ovd-gk/<run>` was gone after the run. Untested: an owner with its own `/proc` mount, a kernel other than 7.0.0-38. Rendezvous (informative): `/run/ovd-gk/<run id>/shared/`, the run-owned VMM runtime root the guest kit already uses (`runtime_staging_root`), mode 0700; the socket path is about 70 bytes, under the 108-byte `sun_path` limit that broke the run `TMPDIR` path in the probe. Owner crash: attached tests get a typed failure naming the owner stage; the lock is free and the next arrival elects a new owner, which unlinks the stale socket path before it binds; the old VMM died with it (V-4). The owner detaches its inherited stdout and stderr when it starts, so a nextest leak check does not see it as a leaked child of the test that elected it. Nothing is left in `/tmp`. A missing owner binary is an unprepared kit: the elector checks the owner binary's contract path before it takes the election lock and fails the test with `LaneNotPrepared` naming it (`unshare(CLONE_NEWPID)` is refused only to an unprivileged caller and the run is root). The elector explicitly releases the election lock with `flock(LOCK_UN)` once the socket is connectable, and the owner child does not keep it (the forked child inherits the open file description); the kernel releases it on process exit in every other case.

### DDD-9 — One at a time, no cancel, discard after failure (question 3)

Details in ADR-0284, ADR-0291 and the model. Options considered for retries: reuse the guest (cheap, residue) versus a fresh guest after a failure (about 3 s). Recommended: fresh after failure, because a retry exists precisely because a test failed, and the failed test is the one most likely to have left state. Observable consequences the conformance driver and the docs rely on: after a discard with tests waiting, the owner starts the replacement boot in the same event, so the observation is `Booting` with generation + 1 (`NoGuest` is observable only with an empty queue); a boot failure answers only the test at the head of the queue (`boot_failed`) and, if tests still wait, starts a new boot for the next one at once (the failed test is not retried automatically); a boot that fails with an empty queue (every waiter disconnected during the boot) leaves `NoGuest` and notifies nobody; a guest lost while idle is replaced at the next attach. Guest kernel crash mid-test: the test gets a typed failure, the guest is discarded. Isolation statement for the docs: tests that share a guest share its kernel, cgroup tree, nft tables and `/tmp`; the owner runs each profile's per-test setup and cleanup before and after every test and resets nothing else. One `StartExec` covers the profile's per-test setup, the test body and the profile's cleanup under one exec bound (DDD-16); a setup or cleanup failure is a nonzero verdict (`exit:<n>`), which discards the guest like any failed test.

### DDD-16 — Every shared exec has a bound (APPROVED, user decision N-1, 2026-11-14)

Reachable failure: the only existing attempt bound is `NEXTTEST_ATTEMPT_RESPONSE_BOUND` = 86,400 s (`guest_suite.rs:301`; nextest owns the real timeout). When nextest kills a test process at its slow-timeout, an isolated guest dies with that process, but a shared guest outlives it and the owner never cancels a running test (ADR-0284). A hung in-guest test would hold the shared slot for up to 24 h, and every queued test of the module would wait out its own nextest timeout. `.claude/rules/reconcilers.md` § "Superseded in-flight work runs to its bound" requires the bound this adds.

Contract: each shared exec (profile per-test setup, test body and cleanup together, DDD-9) runs under its profile's **exec bound** (DDD-5: the cell bound the same bodies already ran under in the batched dispatcher: 300 s, 600 s for `nested*`, 1,200 s for `nested-cohort`). The owner shell arms a timer per exec (no test-only bound override exists) and delivers `Event::ExecBoundElapsed(AttachId)` to `OwnerCore`; the core answers with `Effect::DiscardGuest` and, if the client is still attached, `Reply(.., ExecBound)`; the client maps it to `GuestExecution` with stage `exec_bound`. Discarding the guest is the existing cleanup path, so no second cancel mechanism exists. An isolated guest keeps the existing attempt bound (nextest owns that timeout and the guest dies with the process).

Consequence the user should know: a test that legitimately runs longer than its profile's bound cannot live in a shared module; it takes an explicit `guest = "<group>"` form (isolated, 24 h bound). Nextest overrides in `.config/nextest.toml` allow some tests up to 180 periods; migration lists any test in a shared module whose nextest timeout exceeds its profile bound.

Alternatives: (a) rely on nextest alone: unbounded for the slot (rejected, above); (b) kill only the in-guest test process: needs a guest-side kill channel, more mechanism than discarding the guest; (c) a per-test configurable bound: a new surface for no reachable need. Cost: one timer per exec, one `OwnerCore` event, one model action, one hazard and one run; no hot-path cost beyond the timer. Model: `D-BOUND` (hazard `noBound` leaves `EveryClientTerminates` violated).

### DDD-17 — A shared owner exits when it is idle (APPROVED, user decision N-2, 2026-11-14)

Reachable failure: a shared guest otherwise lives until the end of the Metal run, so every module that ran keeps its VMM for the rest of the command. A `cgroup-default` guest is configured with 8 vCPU and 4,096 MiB, `nested-cohort` with 12,821 MiB (V-3); these are the guest's configured limits, not its measured consumption. The number of declared modules is not measured (it grows with each migration batch) and no budget or cap exists.

Contract: `OwnerCore` answers `Event::IdleElapsed` with `Effect::Exit` if and only if no test is queued and no exec is in flight (a detached exec still counts as in flight); otherwise it does nothing. The shell raises `IdleElapsed` after the core has been continuously idle for `SHARED_GUEST_IDLE` (one constant in `placement::shared`; 30 s; no test-only idle override exists). On `Exit` the owner unlinks the socket path, then closes its listener and exits, so a successor that binds after the close never has its socket unlinked by the exiting owner; its guest dies with it (ADR-0290). The period affects the boot count only, never a result: too short costs one more boot (about 3 s), too long holds a guest longer.

An attach that races the exit: closing a listening unix socket resets every connection still queued for `accept` (`net/unix/af_unix.c:721-723` calls `unix_release_sock(skb->sk, 1)`, which sets `ECONNRESET` on the peer, `:708-709`, @ v7.0), so a client can see its connection cut before the owner has read its request. Rule: a client whose connection ends before the owner has acknowledged the request has run nothing and re-attaches once through the election (a refused connect elects); end of stream or `ECONNRESET` after the acknowledgement is owner loss (the same `unix_release_sock` path sets `ECONNRESET`, `net/unix/af_unix.c:708-709` @ v7.0). Discharged by a deterministic client-side test through `attach_shared` (§ Guest kit public surface): a unix listener at the endpoint's socket that accepts and closes before it acknowledges produces one re-attach through the election, and a second cut is `GuestExecution` at stage `owner`. The model treats attach and idle exit as atomic (assumption A-6), and a Tier-3 scenario is not used because the race is not reproducible on demand.

Evaluated first: **no timer** (exit as soon as the queue is empty and no client is attached). Rejected: the tests of a module are separate nextest processes, so the owner is empty between any two of them; it would exit after every test and the module would boot a guest per test, which removes the feature's reason to exist. Alternatives: (a) no idle exit and a stated maximum: no measured maximum exists; (b) an anchor on the nextest process (the owner opens a pidfd on the wrapper's parent before it creates its namespace and exits when nextest exits): ends owners at the end of a nextest process but not during a long one, and needs a wrapper-exported variable and an ordering constraint (`pidfd_open` resolves in the caller's namespace and returned `ESRCH` from inside the new one, V-4); (c) a cap on live shared guests: a new admission mechanism with its own queue. Cost: one timer, one event and effect, one model action, one hazard and one run.

### DDD-18 — A client that goes away while its test runs taints the guest (APPROVED, user decision N-3, 2026-11-14)

Reachable case: nextest kills a test process at its slow-timeout while the in-guest test still runs (the case the no-cancel rule exists for). The in-guest test may then finish with a pass; without this rule the guest stays `Ready` for the next test, although the harness has already judged the test timed out and killed it. It tightens the rule that a passing test leaves the guest for the next one. Contract: a client that goes away while its test runs marks that generation; when the exec ends, whatever the verdict, the owner discards the guest (ADR-0291). The exec still runs to its bound, never cancelled. Cost: one extra boot (about 3 s) after each nextest kill of a shared test. Alternatives: keep the guest (residue of a timed-out test reaches the next test unobserved); discard at the disconnect (cancels mid-effect, rejected by ADR-0284). Model: part of `D-TAINT`.

### DDD-10 — `inside_guest()` (question 6)

`inside_guest()` is true exactly when the process environment carries the in-guest marker, the existing environment variable `OVERDRIVE_TEST_IN_GUEST`, which the guest exec channel sets for every test it runs. The shared-owner request carries the attaching test's environment (including the forwarded `OVERDRIVE_METAL_*` variables, so `in_metal_run()` is true inside the guest), and the exec channel adds the marker to it. The marker's name is the skeleton's; the function keeps its name.

Why a spoof cannot change an outcome. The marker only makes a guest-placed test run its body in place. On Metal, the ordinary `metal run` grammar accepts a closed list of environment keys and the local environment does not cross ssh, so the marker cannot be set on the host test process of a Metal run. Off a Metal run the body runs in place anyway (DDD-11). A host that exports the marker therefore changes no result that its own absence would not.

Rejected: a kernel command line parameter read from `/proc/cmdline` (it defends a host process that sets the marker, which neither case above allows, and adds a boot parameter, a core and a probe).

### DDD-11 — Off Metal (question 7, F-2)

**Decision (APPROVED by the user, 2026-11-14, U-2, option B; ADR-0286).** This feature changes where placement is declared, not where tests run. Guest placement exists to isolate tests from each other on the shared Metal box; off a Metal run the machine is private, so a guest-placed or lane-placed test runs its body directly in the current process, exactly as tests behave today when the table is ignored off Metal. Tests that need bare-metal KVM stay gated by the `kvm-tests` feature as today. On a Metal run a guest-placed test never runs its body on the host: the wrapper places it, and the test hands itself to a guest or fails with a typed error.

**Detecting a Metal run (pinned, no heuristic).** A Metal run context is present when any of `OVERDRIVE_METAL_RUN_ID`, `OVERDRIVE_METAL_EXECUTION_CLASS`, `OVERDRIVE_METAL_SOURCE_ROOT` is set and non-empty in the process environment: the predicate `test-wrapper.sh` already uses (`:26-31`) to decide it has nothing to do. Until the dispatcher is retired the wrapper's predicate also tests a fourth variable, `OVERDRIVE_METAL_TEST_DISPATCHER` (`:29`), which only the dispatcher path sets for its own children; the library never reads it, and the retirement slice deletes it from the wrapper, leaving the two predicates identical. The library exposes it as `in_metal_run()` (§ Interface contracts), and the wrapper and the library read the same three variables. The three variables are set by `metal run` and by nothing else; the in-guest marker (DDD-10) is the separate fact `inside_guest()` reads.

**Expansion off Metal.** The guest forms expand to: if `inside_guest()` or not `in_metal_run()`, run the original body in the current process; otherwise hand the test to `run_in_guest` / `run_lane` and panic with the typed error. The lane kinds' in-guest body runs in place off Metal the same way. `host_pure` and `native_launcher` are unaffected (their body always runs where the wrapper puts it). With no Metal run context the wrapper `exec`s the original command and ignores records, as today; `placement-lint` still checks every record.

**Rejected: fail closed off Metal** (a typed `NoMetalContext` failure, the body never runs). From the lane inventory below it would turn red the CI default nextest job (13 tests), the CI integration job (≈614), the CI `bpf-unit` job (23), `cargo xtask ci` (13) and the nightly workspace mutation run (≈685): about 685 tests that pass in CI today would fail, and CI would need a Metal runner path nobody asked for, to guard against cross-test interference that cannot happen on a private machine. **Rejected: skip with a visible reason**: nextest has no runtime skip, so a skip is a PASS.

**Lane inventory (V-7, `findings-validation.md`).** Feature gating of the 834 guest-placed tests (807 matched to a source fn): 13 compile with no feature (`overdrive-dataplane` lib 2, `overdrive-gateway` lib 5, `overdrive-sim::acceptance` 6); 654 need `integration-tests` (+18 macro-generated in the dataplane integration binary); 140 need `integration-tests` and `kvm-tests` (cli 78, fwd 13, host 19, control-plane 7, init 23). No off-Metal lane enables `kvm-tests`.

| # | Lane | Runs on | Guest-placed tests run today | Behaviour under this decision |
|---|---|---|---|---|
| 1 | CI `nextest + doctests` (`ci.yml:290-311`) | runner → Lima | 13 | unchanged: body in place |
| 2 | CI `nextest (integration-tests)` (`ci.yml:442-466`) | runner → Lima | ≈614 | unchanged: body in place |
| 3 | CI `bpf-unit` (`ci.yml:1171-1241`) | runner → Lima | 23 | unchanged: body in place |
| 4 | pre-commit `nextest-affected` (`lefthook.yml:194-196`) | Metal if `.env` sets `OVERDRIVE_METAL_TARGET`, else Lima | ≤13 | Metal: in a guest; Lima: in place |
| 5 | pre-push `nextest` (`lefthook.yml:267-268`) | same routing | ≈685 | Metal: in a guest; Lima: in place |
| 6 | `cargo xtask ci` (`xtask main.rs:1692-1693`) | invoking host directly | 13 | unchanged: body in place |
| 7 | nightly `mutants-workspace` (`nightly.yml:79-83`) | runner → Lima | ≈685 | unchanged: body in place |
| 8 | per-PR `mutants` | commented out (`ci.yml:836-982`) | 0 | not applicable |
| 9 | developer Lima inner loop | Lima | ≤≈685 | unchanged: body in place |
| 10 | Metal ordinary run | native host | 834 | in a guest |

No off-Metal lane changes and no lane moves to Metal. No test needs a one-time ruling for being guest-placed off Metal.

### DDD-12 — Dev-dependencies and the xtask rule (F-7)

Hypothesis tested by V-9 in a throwaway worktree: `overdrive-testing-macros` added as a dev-dependency of `xtask` and `overdrive-core`. Result: `dst-lint` and `yaml-free-cli` exit 0; `cargo tree -p xtask | rg overdrive-` (the rule's literal command, which includes dev edges) finds one hit, `overdrive-testing-macros v0.0.0 (proc-macro)`; `cargo tree -p xtask -e normal,build | rg overdrive-` is empty, and so is the same check for `overdrive-core`. No automated graph gate reads dev edges. The macro crate has no `build.rs`, depends only on `proc-macro2`, `quote` and `syn`, and creates no cycle. `cargo deny check bans sources` runs in no lane and already fails at baseline (17 wildcard errors, `anyhow` banned); the new edges raise the wildcard count (core 2→4, xtask 0→1) without changing the failure class.

**Rule text amendment (APPROVED by the user, 2026-11-14, U-3; the orchestrator edits `.claude/rules/development.md`).** The `development.md` § "xtask is build / test / dev orchestration" command becomes `cargo tree -p xtask -e normal,build | rg overdrive-` (must return empty). The rule's reason is the build-time compile chain: only normal and build edges enter the chain that must succeed before an xtask subcommand can run, so a dev-dependency does not recreate the bootstrap deadlock. The sentence "`xtask/Cargo.toml [dependencies]` MUST NOT contain any `overdrive-*` crate" is unchanged; `[dev-dependencies]` is the new, narrower allowance.

**`xtask` takes `overdrive-testing` as a dev-dependency (APPROVED by the user, 2026-11-14, U-3, option X).** The guest forms of the macro expand to `::overdrive_testing::…` (`run_in_guest`), so xtask's 1 guest-group test needs `overdrive-testing` in scope; V-9 counted the 5 native-launcher tests too, but under this design `native_launcher` references no runtime crate (§ Interface contracts), so they need only the macro crate, like the other 392 xtask tests. `overdrive-testing` pulls `overdrive-core`, `overdrive-fwd` and `overdrive-host`; none has a `build.rs` and none reaches `overdrive-dataplane`, so the bootstrap deadlock does not recur. Chosen: a dev-dependency, with the cargo-tree command above as the enforcement. Rejected **(Y)**: leave that one test with its table entry, which delays the table's retirement; the widened dev graph is the accepted cost. Not run: `cargo check` of the `overdrive-core` and `xtask` test targets with the new edges.

### DDD-13 to DDD-15

DDD-13: `lane_operator` and `lane_owner` tests are the in-guest body; their host half calls `guest_lane::run_lane`, which boots the lane guest from the prepared in-guest binary and, for the owner kind, runs the host script chosen by the fixture; an unknown fixture fails naming the fixture and the test. The six scripts and the placement slot table move into `guest_lane` and are matched by exact fixture name. The slot per placement is internal; a guest in its own local-mode vsock namespace does not collide with another run's. DDD-14: the retirement list is the Reuse Analysis DELETE row plus `OVERDRIVE_FWD_GUEST_GROUP`, `OVERDRIVE_FWD_GUEST_EXACT`, the `bootstrap.sh` grammar entry for the dispatcher, `guest_kit_prepare`'s dispatcher build, and the nine dispatcher unit tests (deleted, not rewritten, per the deletion rule); documents to update in the same slice: `.claude/rules/testing.md` § "Test placement" (through the rules process), `infra/metal/README.md`, `website/content/docs/how-to/run-verification-on-metal.mdx`, `tests/conformance/CLAUDE.md` and `src/lib.rs` comments, the `metal-run-isolation` and `declare-where-a-metal-test-runs` journeys, and the xtask placement acceptance tests. DDD-15: per batch, a script under `docs/feature/metal-test-placement-attribute/deliver/` (temporary, deleted at retirement) reads the table by parsing the JSON, finds each test's function by module path and name, adds the attribute (for host-pure tests, adds `overdrive_test_default!(host_pure)` to a host-only binary, or `placement!(host_pure)` to each host-pure module of a mixed one, instead of a per-test attribute), deletes the entry, and **stops on any ambiguity** (two functions, a macro-generated test other than the approved template, a missing function); parity = `placement-lint --emit` over the migrated binaries, diffed against the table snapshot at the batch's start commit, equal on (binary, test, kind, group, fixture, profile); pass-set parity is the Metal run. Where a test cannot take an attribute, the batch reports it and the user rules; V-2 found none under the pinned grammar (no test is unable to take the attribute once the macro inserts no `#[test]` and the host forms leave the fn untouched), apart from the 18-test template, which the script special-cases (U-5, T1, approved 2026-11-14). The workspace uses no `rstest`, `test_case`, `libtest-mimic` or `harness = false`.

## Wave: DESIGN / [REF] Interface contracts

These are the pinned cross-crate, cross-process contracts. Private structure is the crafter's.

### Attribute grammar (`overdrive_testing::overdrive_test`, re-exported from `overdrive-testing-macros`)

```text
shared_guest!(guest = "<group>" [, profile = "<profile>"] [,])      // function-like, once per module

#[overdrive_test]                       // module form; `#[overdrive_test()]` is identical
#[overdrive_test(host_pure)]
#[overdrive_test(native_launcher)]
#[overdrive_test(guest = "<group>" [, profile = "<profile>"])]   // isolated guest, one per test
#[overdrive_test(lane_operator = "<fixture>")]
#[overdrive_test(lane_owner = "<fixture>")]

placement!(host_pure)                   // function-like, once per module; host-pure for that module's tests
overdrive_test_default!(host_pure)      // function-like, once at the root of a test binary
```

User-approved shape (2026-11-13); every clause is approved (2026-11-14). `placement!` and `overdrive_test_default!` are approved (2026-11-14, U-4).

`overdrive_testing::placement!` and `overdrive_testing::overdrive_test_default!` (both re-exported from `overdrive-testing-macros`):

- Each expands to one record `static` inside one **named** item (`placement!` a module-level const, `overdrive_test_default!` a crate-root const, so a duplicate is E0428) and nothing else, so each references no runtime crate and a `core`-class crate needs only the macro crate as a dev-dependency (DDD-12). Neither is read by `#[overdrive_test]`; a test with no attribute of its own (a plain `#[test]`, a `proptest!` or `quint_*` test) is placed by them.
- The only accepted argument is `host_pure`. Any other argument, no argument, or more than one is a build error.
- `placement!` records the declaring module's path (`module_path!()`, crate segment included) and applies to the tests whose module path equals it exactly; a child module does not inherit it. `overdrive_test_default!` records no module and applies to every test of its binary not placed by an attribute or a module declaration; it is written once at the binary's root (for a unit-test binary, in the crate root under `#[cfg(test)]`).
- Precedence: a test's own attribute, then its module's `placement!`, then the binary's default. A second `placement!` in one module or a second default in one binary does not compile (E0428).
- A binary that carries `overdrive_test_default!` and any record of kind `guest`, `native_launcher`, `lane_operator` or `lane_owner` is refused (`placement_conflict`, wrapper and `placement-lint`).

`overdrive_testing::shared_guest!`:

- A function-like macro exported by `overdrive-testing`, used as a module-level item. It expands to one module-private, non-`pub` item `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE` (approved name; V-1) of type `GuestPlacement` with `Sharing::Module`, built by `GuestPlacement::declare` in constant context with the declaring module's `module_path!()` as its fourth argument, with `#[allow(dead_code, non_upper_case_globals)]`. The recorded module path, not the reading test's own module, is the sharing scope (DDD-1 § Nested modules). The name is pinned because rustc's E0425 for a missing declaration prints it. Whether the macro is declarative or procedural is the crafter's, provided the item is visible by plain path to the module's own tests and to nothing else.
- `guest` is required; `profile` is optional (omitted means the group's base profile); a trailing comma is accepted.
- Build errors: an unknown group or profile (E0080 at the offending literal, naming the value; DDD-5), an unknown or repeated argument, a missing `guest`, a second `shared_guest!` in one module (duplicate item), and any `shared` argument (not a form).
- Scope: the declaring module; a child module reaches it only by importing it explicitly (`use super::*;` or a path) and then shares the declaring module's guest; without an import a child is E0425 and declares its own.

`#[overdrive_test]` (no arguments):

- Expands to code that names `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE` by plain path. Ordinary name resolution binds the module's declaration; with none in scope the test fails to compile with rustc's E0425 "cannot find value `OVERDRIVE_TEST_NEEDS_shared_guest_DECLARATION_IN_THIS_MODULE` in this scope" (a proc macro cannot emit a `compile_error!` for an absent sibling item). A declaration the test cannot see (a different module, including a parent the test did not import it from) is not found; two declarations in one module are an E0428.
- The placement is the declaration's group and profile with sharing `Module`. The ELF record is emitted from the same const by constant evaluation (kind `guest`, group, profile, sharing `1`), so the wrapper reads exactly what the declaration holds.
- Any other form is a per-test placement and never reads the module declaration. An explicit `guest = …` is an isolated guest (record sharing `0`).

Shared clauses:

- **The macro never inserts `#[test]`** (approved; V-2). Every test carries its own test attribute (`#[test]`, `#[tokio::test]`, the `#[test]` inside `proptest!`, `#[quint_test]`, `#[quint_run]`); the macro leaves them and any `#[ignore]`, `#[serial]`, `#[should_panic]` intact, and a fn with no test attribute is not a test (the macro does not make it one). Each test has exactly one record regardless of order. `#[overdrive_test]` is written below `#[quint_test]`/`#[quint_run]`.
- The guest forms (module form, `guest`, `lane_operator`, `lane_owner`) apply to a `fn` returning `()` (sync or `async`; V-1(c) fixes async bodies under `#[tokio::test]`). `host_pure` and `native_launcher` apply to any test fn, including one returning `Result`: they emit the record as a sibling `static` inside a named item (derived from the fn name, so two attributes on one fn are E0428) and leave the fn untouched.
- `host_pure` and `native_launcher` emit the record and leave the body untouched, and reference no runtime crate.
- The module form, `guest` and the lane forms replace the body with: run the original body in the current process where `inside_guest()` holds or `in_metal_run()` does not (off a Metal run, DDD-11, ADR-0286); otherwise hand the test to `run_in_guest` / `run_lane` on a plain OS thread and panic with the typed error. On a Metal run the body never runs on the host.
- Build errors of the attribute: an unknown `group` or `profile` (text as for `shared_guest!`), an unknown argument (including `shared`, with the message "sharing is declared by `shared_guest!`"), a repeated argument, a lane fixture not of the form `integration::in_guest_lane::<ident>`, and (by name resolution, not a macro diagnostic) the no-argument form without a declaration. A lane fixture's existence is checked at run time (the failure names fixture and test).
- **Conflict rule (approved):** an explicit per-test form inside a module that has `shared_guest!` overrides it for that test; it is not an error. Alternative (compile error) and why it is not recommended are in DDD-1.
- **Tests of the Metal runner's own placement machinery (user ruling, 2026-11-14, D-10):** a test that must observe the host from outside any guest to prove the machinery (the guest kit booting a guest, a host marker file, the owner and VMM processes, process-group signals, the timing lines of the host half) cannot run inside a guest, where the kit, its artifacts and the run's namespaces are absent and `inside_guest()` holds. Such a test is a test of the runner itself and is placed `#[overdrive_test(native_launcher)]` through the attribute, never by a table entry. This is the one case the placement rule allows for `native_launcher`; a product test is never placed this way. The form is available from the lane-and-launcher slice, so these tests activate there or later. A test of such a driver that observes nothing outside its own process (it spawns a child with a chosen environment and reads an exit status and a marker file, no guest and no node-global effect) is `host_pure`.

### Placement record (ELF section `overdrive_test_placement`)

A sequence of records; each record is a sequence of NUL-terminated fields: `<kind>`, `<test>`, `<argc>`, then `argc` arguments. There is no version field: the macro, the wrapper and the lint come from one checkout and nothing persists. `<kind>` is one of `host_pure`, `native_launcher`, `guest`, `lane_operator`, `lane_owner`; `<test>` is `module_path!()` of the fn plus `::<fn name>` (crate segment included; the reader drops it to compare with nextest's test name). Arguments: `guest`: `<group>`, `<profile or empty>`, `<"1" when the placement came from the module's `shared_guest!`, "0" for an explicit `guest` form>`; lane kinds: `<fixture>`; the others: none. Two further kinds carry no arguments (`<argc>` is `0`): `module_host_pure`, whose `<test>` field is the declaring module's `module_path!()` (crate segment included; the reader drops it) and applies to the tests whose module path equals it; and `default_host_pure`, whose `<test>` is empty and applies to the binary. Resolution for a test, first match wins: a per-test record (`host_pure`, `native_launcher`, `guest`, `lane_operator`, `lane_owner`); a `module_host_pure` record for the test's module; the binary's `default_host_pure`; until retirement, a table entry; otherwise `test_unclassified`. A test with a record or a declaration record that also has a table entry is placed by the record (the table clause is transitional). Every record emission is a named item, so a duplicate (two attributes on one fn, two `placement!` in one module, two defaults in one binary) is rustc's E0428 at build time and no reader rule for it exists. A reader that cannot parse the section refuses with `placement_invalid` (one refusal, no sub-cases).

### Wrapper diagnostics (stderr line `OVERDRIVE_METAL_TEST_ERROR error=<name> [detail]`)

| Name | Status | Cause |
|---|---|---|
| `test_unclassified` | 78 | no per-test record, no `module_host_pure` record for its module, no `default_host_pure` (and, until retirement, no table entry) |
| `placement_invalid` | 78 | unreadable section or unreadable table |
| `placement_conflict` | 78 | a `default_host_pure` and a `guest`, `native_launcher`, `lane_operator` or `lane_owner` record in one binary |
| `invalid_run_context`, `namespace_setup_failed` | 78, 70 | unchanged |

`lane_not_prepared` and `guest_execution_failed` leave the wrapper with the dispatcher; the same conditions are typed errors of the test's own host half. The wrapper maps kinds to isolation as today: `host_pure`, `module_host_pure`, `default_host_pure` → original chain; `native_launcher` → node view; `guest`, `lane_operator`, `lane_owner` → local-mode vsock network namespace. `test-wrapper.sh --check-placement` is handled before the wrapper's no-Metal-context `exec "$@"`, reads nextest list JSON on stdin and prints one `unplaced <binary-id> <test>` line per unplaced test and one `conflict <binary-id>` line per conflicting binary, exiting non-zero if any.

### `placement-lint` (xtask)

`cargo xtask placement-lint`: approved by the user, 2026-11-14 (U-1); wired into CI and pre-push beside `dst-lint` and `conformance-lint`, with its grammar entry in `bootstrap.sh`. It lists tests with `cargo nextest list --message-format json --workspace`, enabling `integration-tests` and, per package, `<pkg>/kvm-tests` for every workspace member whose manifest declares `kvm-tests` (today `overdrive-fwd`, `overdrive-host`, `overdrive-control-plane`, `overdrive-cli`, `overdrive-init`, found through `cargo metadata`, so a member added later is covered without an edit; a bare `--workspace --features kvm-tests` is not used because 32 of 37 members do not declare it). Without `kvm-tests` the 140 tests behind it would escape. It reads ELF sections of the binaries it lists, so it runs on Linux: in pre-push through `infra/hooks/linux-run.sh` (Metal when `.env` sets `OVERDRIVE_METAL_TARGET`, else Lima), the routing of the `nextest` step, and in CI in the `integration` job beside its nextest step, with that step's routing. Run directly on a macOS host it would read Mach-O binaries, which is why the pre-push lane routes it. It prints `placement-lint: unplaced <binary-id> <test>` per unplaced test, `placement-lint: conflict <binary-id>` per binary with a default and a non-host-pure record, and `placement-lint: missing-fixture <binary-id> <test> <fixture>` per `lane_operator` / `lane_owner` record whose fixture is not a listed test of the binary that provides it (it reads every listed binary's records; the provider is the single binary `overdrive-init::bin/overdrive-init`, see § Lane contract), and exits 1 if it printed any. The run-time failure naming fixture and test remains as the lane run's own check. It writes nothing. A migration-only `--emit` mode (prints the resolved placements as JSON for DDD-15's parity diff) exists until retirement and is deleted with the table; its output format belongs to the migration script and is not a contract.

### Guest kit public surface (`overdrive_testing::placement`)

```rust
// Informative: no caller outside the registry names this type; the contract is the group name strings in § DDD-5.
pub enum GuestGroup { Probe, Core, Unload, Ulp, MtlsRelay, CgroupDefault, Stress, NestedBoundary }

pub enum Sharing { Isolated, Module }

/// Built in constant context by the macro; panics at constant evaluation on
/// an unknown group or profile (message text: DDD-5, V-11). `declaring_module`
/// is the `module_path!()` of the module holding the declaration (empty for
/// `Sharing::Isolated`); it is part of the sharing scope. The three string
/// parameters are `&'static str`: the placement is stored in a `const`, and the
/// macros pass string literals and `module_path!()`.
pub struct GuestPlacement { /* group, profile, sharing, declaring module */ }
impl GuestPlacement {
    pub const fn declare(group: &'static str, profile: &'static str, sharing: Sharing,
                         declaring_module: &'static str) -> GuestPlacement;
}

pub struct LanePlacement { /* kind, fixture */ }
impl LanePlacement {
    pub const fn operator(fixture: &str) -> LanePlacement;
    pub const fn owner(fixture: &str) -> LanePlacement;
}

pub fn inside_guest() -> bool;
/// True when any of OVERDRIVE_METAL_RUN_ID, OVERDRIVE_METAL_EXECUTION_CLASS,
/// OVERDRIVE_METAL_SOURCE_ROOT is set and non-empty: the wrapper's own
/// predicate for "a Metal run context is present" (DDD-11). The function reads
/// the process environment; the shape of any pure core it delegates to (its
/// parameter types) is the crafter's, not contract. Tests drive `in_metal_run()`.
pub fn in_metal_run() -> bool;
pub fn run_in_guest(placement: &GuestPlacement, exe: &Path, test_name: &str) -> Result<(), RunInGuestError>;
pub fn run_lane(placement: &LanePlacement, exe: &Path, test_name: &str) -> Result<(), RunInGuestError>;

pub enum RunInGuestError {
    LaneNotPrepared  { group: String, test: String, detail: String },
    GuestExecution   { group: String, test: String, detail: String },
    BodyFailed       { group: String, test: String, status: i32 },
}

/// The rendezvous and owner binary of one shared scope. On a `Sharing::Module`
/// placement `run_in_guest` computes it (scope key, the run's runtime directory,
/// the kit's owner-binary contract path) and passes it to `attach_shared`; that
/// is its only production producer.
pub struct SharedEndpoint { pub socket: PathBuf, pub lock: PathBuf, pub owner_binary: PathBuf }

/// The client half of the shared-guest protocol (§ Shared-guest scope and client protocol).
pub fn attach_shared(endpoint: &SharedEndpoint, placement: &GuestPlacement, test_name: &str)
    -> Result<(), RunInGuestError>;
```

`attach_shared` is the shell/core split of the client: `run_in_guest` reads the environment and computes the paths, `attach_shared` takes them as parameters and runs the protocol. Its pinned behaviour: (1) `endpoint.owner_binary` absent: `LaneNotPrepared` naming that path, returned before the election lock is taken (`endpoint.lock` is not created); (2) a connection that ends before the acknowledgement: one re-attach through the election, and a second cut is `GuestExecution` at stage `owner` (DDD-17); (3) every other outcome maps as in § Shared-guest scope and client protocol. The lock and socket file names inside the endpoint, the request's wire encoding and the owner's command line are the crafter's. The per-profile exec bounds and `SHARED_GUEST_IDLE` are read only by the owner shell: their values are behaviour (DDD-5, DDD-16, DDD-17), their names and location are internal, and no test reads them.

`inside_guest`, `run_in_guest` and `RunInGuestError` keep the names the skeleton approved; `run_in_guest`'s first parameter changes from `&str` to the typed placement, and `in_metal_run` is new (DDD-11); there is no `NoMetalContext` variant, because off a Metal run the macro expansion never calls `run_in_guest`. Per-test mode prints one `OVERDRIVE_GUEST_KIT_TIMING group=<g> test=<t> staging_ms=<n> boot_ms=<n> total_ms=<n>` line per guest booted (unchanged); a shared attach prints `OVERDRIVE_GUEST_KIT_TIMING group=<g> test=<t> shared=1 generation=<n>`, and the owner prints the boot's staging and boot times with the same `generation`. One boot per module is counted as the distinct `generation` values per scope. The guest's console output of the test is returned to the host test's output. Preconditions of `run_in_guest`: the caller has found `in_metal_run()` true and `inside_guest()` false; the Metal run is an ordinary one with a valid `OVERDRIVE_METAL_RUN_ID`, and the guest kit is prepared. `run_in_guest`'s existing run-runtime rule (`runtime_staging_root`, `guest_kit.rs:423-438`) refuses a run id that is present but malformed as `GuestExecution`; it does not refuse an absent run id (it uses the kit's non-run runtime root), and no separate context check exists. A Metal context with an absent run id (for example only an `exclusive` class, which the wrapper never places) still hands the test to `run_in_guest`, never to the body on the host. With none of the three variables set, `in_metal_run()` is false and the body runs in place (DDD-11). Failure maps to the variants above with the stage in `detail`.

**Lane contract.** The lane forms are bound to one test binary: `overdrive-init::bin/overdrive-init`, which holds both the placed in-guest body and its `integration::in_guest_lane::<fixture>` driver (`guest_suite.rs:4735`, `INIT_GUEST_LANE_PLACEMENTS`). `run_lane` takes the same `(placement, exe, test_name)` as `run_in_guest` because the dispatcher it replaces derived the exe, binary id and slot (internal) from its environment (`:4747-4771`); the binary id comes from `NEXTTEST_BINARY_ID` at the shell edge. The fixture tests of that binary need their own placement: a `placement!(host_pure)` in the module that holds them or a per-test attribute, because a binary that holds lane records cannot take the binary default (`placement_conflict`).

### Shared-guest scope and client protocol

**Scope key** (a pure function of its inputs; the shell reads the inputs): the test binary's nextest binary id (`NEXTTEST_BINARY_ID`, read once at the shell edge), the declaring module path recorded in the `shared_guest!` declaration, the group, and the profile. Rendezvous: one lock file and one socket per key inside the run's own runtime directory (the location, file names and mode are the crafter's; § DDD-8 records the probed layout). A new owner unlinks a stale socket path before it binds; an exiting owner unlinks its path before it closes its listener (DDD-17). After acquiring the election lock a client connects once more and attaches if the socket is connectable (DDD-8).

**Owner binary:** `overdrive-shared-guest-owner`, prepared by `guest-kit-prepare` at a documented contract path and refused as `LaneNotPrepared` naming it when absent. Its command line, file names, key hashing and wire encoding are the crafter's; the contract is the following behaviour.

- A request carries the test name, working directory and environment of the attaching test. The owner **acknowledges** a request before it runs anything for it.
- A client whose connection ends before the acknowledgement has run nothing: it re-attaches once through the election (DDD-17). End of stream or `ECONNRESET` after the acknowledgement is owner loss.
- The owner calls `setsid` before it serves, so a group signal sent to the electing client's process group does not reach it (DDD-8).
- A reply carries the verdict (`exit:<i32>`, `boot_failed`, `guest_lost`, `exec_bound`), the generation and the console bytes.
- Client mapping: `exit:0` → `Ok`; other exit → `BodyFailed`; `boot_failed`, `guest_lost`, `exec_bound` and owner loss → `GuestExecution` with the stage named (`boot`, `guest_lost`, `exec_bound`, `owner`).

### Owner core (conformance accessors)

```rust
pub struct OwnerCore { /* private */ }
/// One attached client connection; the shell numbers its accepted connections.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct AttachId(u32);
impl AttachId { pub const fn new(n: u32) -> AttachId; }
#[derive(Clone, Debug, PartialEq, Eq)]   // Event, Effect, Verdict, GuestView, OwnerObservation
pub enum Event { Attached(AttachId), ClientGone(AttachId),
                 BootFinished(bool),            // true: the guest is ready; false: the boot failed
                 ExecFinished(AttachId, i32),   // the exec's exit status
                 GuestLost, ExecBoundElapsed(AttachId), IdleElapsed }
pub enum Effect { StartBoot, StartExec(AttachId), Reply(AttachId, Verdict), DiscardGuest, Exit }
pub enum Verdict { Exit(i32), BootFailed, GuestLost, ExecBound }
pub enum GuestView { NoGuest, Booting, Ready }
pub struct OwnerObservation { pub guest: GuestView, pub generation: u32,
                              pub in_flight: Option<AttachId>, pub queue: Vec<AttachId>,
                              pub stopped: bool }
impl OwnerCore { pub fn new() -> OwnerCore; pub fn handle(&mut self, event: Event) -> Vec<Effect>; pub fn observe(&self) -> OwnerObservation; }
```

The five types `Event`, `Effect`, `Verdict`, `GuestView` and `OwnerObservation` derive exactly `Clone, Debug, PartialEq, Eq` (the derive line above applies to each). `Debug` prints a failing seeded schedule and a failed assertion; `Clone` lets the driver and tests keep a snapshot across steps; `PartialEq` and `Eq` compare snapshots and returned effects. Nothing else is pinned (no `Copy`, `Hash` or `Ord`).

`OwnerCore::new()` observes `guest = NoGuest`, `generation = 0`, `in_flight = None`, an empty `queue` and `stopped = false`. After `Effect::Exit` it observes `stopped = true`, `guest = NoGuest`, `in_flight = None` and an empty `queue` (`Exit` is returned only when nothing is queued or in flight).

**Events the core does not enable.** An event that is not enabled in the current state, or that names an `AttachId` that is not queued or in flight, returns no effects and changes nothing; it never taints the guest. This covers `ClientGone` for a client whose reply was already delivered (every client closes its connection after its reply), `ExecBoundElapsed` racing `ExecFinished`, an `ExecFinished` after the guest was discarded, an `ExecBoundElapsed` naming another id, `GuestLost` when no guest is `Ready`, `BootFinished` when no boot is in progress, `IdleElapsed` while a test is queued or running, an `Attached` naming an id already queued or in flight, and any event after `Effect::Exit`. The seeded `OwnerCore` run injects these events in every state.

There is no election port: the election lock is the kernel's, the model's `owners` is model-only for election, and "two test processes arrive at once, one owner" is proved by the Tier-3 scenario (V-5), not by a conformance accessor. `OwnerCore` is reactive: every event that frees the slot or adds work also decides what starts next, so `StartBoot` and `StartExec` are returned by the event that causes them (a discard with tests waiting returns `DiscardGuest` and `StartBoot` together; the observation after it is `Booting` with generation + 1).

**Conformance mapping (every action of the conformance instance `guest_conf` is one `OwnerCore` call; the driver holds an `OwnerCore`, never a Sim election):**

| Spec action (`guest_conf`) | Production call | Projected after the step |
|---|---|---|
| `Attached{c}` | `handle(Event::Attached(id_c))` | `observe()` |
| `ClientGone{c}` | `handle(Event::ClientGone(id_c))` | `observe()` |
| `BootFinished{ok}` | `handle(Event::BootFinished(ok))` | `observe()` |
| `ExecFinished{pass}` | `handle(Event::ExecFinished(id, status))`, `status = 0` when `pass`, otherwise `1` | `observe()` |
| `GuestLost` | `handle(Event::GuestLost)` | `observe()` |
| `ExecBound` | `handle(Event::ExecBoundElapsed(id))` | `observe()` |
| `IdleElapsed` | `handle(Event::IdleElapsed)` | `observe()` |

State fields and their accessor: `guest` → `OwnerObservation::guest`; `gen` → `generation`; `inflight` → `in_flight`; `queue` → `queue`; `owners` (0 or 1 in this instance) → `not stopped`. A client's outcome is read from the `Reply` effect the call returns. Model-only: `lock`, `bad`, `execgen`, `execs`, `faults`, `idleLoss`, `conf_step`, and the per-client outcome phases. The model's client number is the `AttachId` the driver passes in. The queue is a list appended at the tail and consumed from the head, so arrival order holds by construction; the spec has no ordering ghost.

**Environment actions outside conformance replay (APPROVED exemption, user decision N-4, 2026-11-14).** `Arrive`, `ElectBegin`, `OwnerStarted` and `OwnerCrash` are performed by the operating system or the client shell, not by `OwnerCore` (an owner crash leaves no process to receive an event). They live in the full instance `guest_ok` (exhaustive checks, hazards and the environment named runs, which the evaluator checks and `#[quint_test]` does not pin). This departs from `.claude/rules/testing.md` § "Per-driver checklist" items 3 and 6 for those actions; the per-invariant discharge is:

| Invariant | Not exercised by `guest_conf` replay because | Proved by |
|---|---|---|
| `I_OWNER_UNIQUE` | the election lock is the kernel's, and the conformance instance starts with the owner present | Tier-3: eight simultaneous first arrivals elect exactly one owner (V-5), on the production lock path |
| `I_NO_ORPHAN` | an owner crash leaves no process to receive an event | Tier-3: SIGKILL of the owner during a guest exec leaves no VMM (V-4) |

Every other invariant (`I_SERIAL`, `I_NO_REUSE`, `I_DETACH_NO_CANCEL`, `I_DELIVER_ONLY_ATTACHED`, `I_IDLE_NO_LOSS`, `EveryClientTerminates`) is constrained by actions `guest_conf` replays. DISTILL records one exemption row per invariant in the table, naming the Tier-3 scenario.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

| Signal or state | Owning component | Promise | Inputs that may gate it | Must not gate |
|---|---|---|---|---|
| nextest test result (PASS/FAIL/TIMEOUT) | nextest | the attempt's process exited with that status | the wrapper's refusal, the test's own panic | guest readiness of other tests |
| a test's placement | the test (record) | which isolation its body needs | the record (and DDD-4 default) | the test's result |
| guest generation `Ready` | owner core | a booted guest of the scope's profile with its setup done, idle, never used by a failed test | boot result, previous exec verdict | any test outside the scope, any `Isolated` test |

Gates:

- **G-1 Placement guard.** Existing evidence: `test-wrapper.sh:58-247` (table lookup then `test_unclassified`), skeleton marker read `:71-161`; no state-write path beyond exit status. Owner: the wrapper. Promise: the test has exactly one resolvable placement. Affected state: whether the body is launched. Failure projection: status 78 with `test_unclassified`, `placement_invalid` or `placement_conflict`. Explicitly unaffected: the test's kind semantics, run-context checks (still first). Ordering: after run-context validation, before the isolation profile; no timeout. Counterexample: a plain `#[test]` that mutates nft would run on the host if absent placement defaulted to host-pure in a mixed binary. Evidence lane: xtask acceptance with fixture ELFs, plus the Metal ordinary run.
- **G-2 Metal context for a guest placement.** Existing evidence: `test-wrapper.sh:25-31` (the no-Metal-context predicate and the `exec "$@"` path), `run_in_guest` and `GuestKit::provision` refuse when unprepared. Owner: the guest-form macro expansion (`inside_guest()`, `in_metal_run()`) and `run_in_guest`. Promise: on a Metal run a guest-placed body runs only inside a guest; off a Metal run it runs in the current process, as today. Affected state: where the body runs, and the test's result when the guest path fails. Failure projection: on a Metal run, `GuestExecution` with the stage named (FAIL), including a Metal run context whose run id is present but malformed (refused by `run_in_guest`'s existing run-runtime rule, `guest_kit.rs:423-438`). With none of the three variables set there is no Metal run context: `in_metal_run()` is false and the body runs in place (DDD-11, U-2), not a failure. A context whose run id is absent but another variable is set is a Metal run: the test is handed to `run_in_guest`, never run in place. Unaffected: `host_pure` and `native_launcher` tests, and every off-Metal lane's tests. Ordering: the predicate is read before any boot. Counterexample: running the body in place when only `OVERDRIVE_METAL_EXECUTION_CLASS=exclusive` is set would put a guest-placed body on the shared Metal host, so a Metal context of any class counts as a Metal run. Evidence lane: unit tests that drive `in_metal_run()` over the three variables (each alone, none), a Metal run showing the body in a guest, the malformed-run-id scenario (fails naming the group, body never on the host), and the lane inventory V-7 showing no off-Metal lane changes.
- **G-3 Shared-guest admission.** Existing evidence: today the batched dispatcher boots one guest per group and runs its bodies serially in it under `CELL_BOUND` (`guest_suite.rs` batched `run_*`, `guest.exec(.., CELL_BOUND)`), and `GuestPeer::exec(&mut self)` allows one exec at a time; the per-test path boots one guest per attempt. No state-write path beyond the attempt's exit status. Owner: owner core. Promise: a test is served only on a `Ready` generation that never ran a failed or detached test, one at a time, in arrival order. Affected state: that attach's reply. Failure projection: `exit:<n>`, `boot_failed`, `guest_lost`, `exec_bound`, owner loss (each a typed host failure). Unaffected: nextest's retry policy, `Isolated` tests, other scopes. Ordering: queue wait plus exec consume the one nextest slow-timeout budget; the owner adds the exec bound of G-5 and the idle period of G-6 (a wait that never delays a test) and no other deadline (election uses `SCRIPTED_BOOT_BOUND`). Counterexample: a test that fails and leaves a nft rule must not be followed by a pass or fail that depends on it. Evidence lane: the Quint model, a seeded run of `OwnerCore` (including the stale events of § Owner core), and the Tier-3 module scenarios.
- **G-4 Eager placement check.** Existing evidence: the wrapper's per-test refusal `test-wrapper.sh:58-247` (run time, on Metal only), the CI `integration` job (`ci.yml:442-466`, `--features integration-tests` only) and the pre-push `nextest` step (`lefthook.yml:267-268`, through `infra/hooks/linux-run.sh`), neither of which builds with `kvm-tests`; `dst-lint` and `conformance-lint` are syntactic and run on the runner and the invoking host, so they are not the precedent for routing; no state-write path beyond the command's exit status. Owner: `placement-lint`. Promise: every test listed with `integration-tests` and the per-package `kvm-tests` set resolves a placement, no binary mixes a default with a non-host-pure record, and every lane fixture exists, before any guest boots. Affected state: the command's exit status. Failure projection: one `unplaced`, `conflict` or `missing-fixture` line per finding and exit 1; a nextest list failure or an unreadable section exits non-zero with the cause, never as "no findings". Explicitly unaffected: Metal runs, test results, and what any test does. Ordering: after the test binaries build, before any Metal command in the pre-push and CI lanes; no timeout of its own. Budget (estimate, unmeasured): the `kvm-tests` set rebuilds the library and test targets of the five declaring crates and their dependents (`overdrive-host` and `overdrive-fwd` feed `overdrive-testing`, `overdrive-control-plane` and `overdrive-cli`) that the `integration-tests` build does not rebuild; about 2 to 5 minutes of extra compile in a warm cache and up to about 10 in a cold one, once per lane, plus a few seconds to list and read the sections. The CI `integration` job and the pre-push step each pay it; growth past that needs user approval. Counterexample: a test generated by `proptest!` or `quint_test` has no source-level marker, so a source scan would call it placed; only the listed test plus its ELF record decides. Evidence lane: negative fixture binary (one unplaced test, one conflicting binary, one misspelled fixture) plus no `test_unclassified` in later Metal runs.
- **G-5 Shared exec bound (APPROVED, N-1, 2026-11-14).** Existing evidence: `NEXTTEST_ATTEMPT_RESPONSE_BOUND` 86,400 s on the per-test path (`guest_suite.rs:301`); `CELL_BOUND` and the nested bounds on the batched path. Owner: owner core (timer armed by the shell). Promise: no shared exec holds the slot past its profile's bound. Affected state: the in-flight exec's slot and the guest generation. Failure projection: `exec_bound` to an attached client; a detached exec's result is simply discarded; either way the guest is discarded. Explicitly unaffected: `Isolated` tests, queued tests (they wait for the replacement boot), other scopes. Ordering: the bound runs from `StartExec`; after it fires, the discard precedes the next boot. Counterexample: a test killed by nextest at its slow-timeout whose in-guest body hangs would otherwise hold the slot for 24 h. Evidence lane: the Quint model (`D-BOUND`), a seeded `OwnerCore` run that delivers the bound event to a hung exec (the bound's duration is not exercised; no test-only bound override exists).
- **G-6 Owner idle exit (APPROVED, N-2).** Existing evidence: shared guests are new; the nearest precedent is the run's transient unit (ADR-0281), which ends a run's leftovers at the end of the whole command. Owner: owner core. Promise: an owner with no queued test, no exec in flight and no attached client for `SHARED_GUEST_IDLE` exits and its guest dies with it; an owner with any such work never exits. Affected state: the owner's existence and its guest. Failure projection: an attach racing the exit sees its connection end before acknowledgement and re-attaches once; no client with work in flight is ever told "owner lost" by an idle exit. Explicitly unaffected: results of any test, other scopes, nextest's retry. Ordering: the idle clock restarts at any event that adds work or ends an exec. Counterexample: exiting as soon as the queue is empty would exit between every two tests of a module (they are separate processes) and boot a guest per test. Evidence lane: the Quint model (`D-IDLE`, hazard `idleAnyTime`), a seeded `OwnerCore` run, and the deterministic client-side re-attach test of DDD-17 (no Tier-3 race scenario, no test-only idle override).

Boundary scenarios for DISTILL: (1) gate available: a declared test passes in a guest; (2) unavailable or timed out: guest cannot boot, the owner binary is missing (`LaneNotPrepared`, refused before the lock is taken, driven through `attach_shared` with an endpoint naming an absent owner binary), a hung exec reaches its bound (`exec_bound`, guest discarded, in the seeded `OwnerCore` run) → typed failure naming group and stage; (3) unrelated state: a `host_pure` test in the same binary and a test with an explicit per-test placement in the same module (approved override rule) are unchanged; (4) late success: an exec whose client was killed returns after the next client attached, and its result reaches nobody; (5) disconnect and reconnect: owner killed mid-module, a new owner is elected and boots a new generation; a client killed while its test runs taints the guest (replacement boots after the exec ends); a client whose connection is cut before the acknowledgement re-attaches once (deterministic client-side test: `attach_shared` against an endpoint whose socket is a listener that accepts and closes), and a second cut is `GuestExecution` at stage `owner`; the electing client killed by a group signal while the owner serves other tests leaves the owner serving (the owner called `setsid`); (6) feature disabled: off a Metal run (none of the three Metal variables set), a guest-placed and a lane-placed test run their bodies in place and pass as before; (7) declarations: a test with its own attribute in a module with `placement!(host_pure)` keeps its attribute's placement, a child module of a declaring module without its own declaration is `test_unclassified`, and a binary with `overdrive_test_default!` and a guest-placed test is `placement_conflict`; (8) lint: a misspelled lane fixture is reported by `placement-lint` naming binary, test and fixture. Seeded invariant: `OwnerCore` under `overdrive-sim`-style seeded schedules of attach, gone, failure, exec-bound and idle events (the model's invariants as `assert_always!`); real-kernel proof: the two-process election (`I_OWNER_UNIQUE`), owner death (`I_NO_ORPHAN`), the group-signal survival of the owner, and per-test results on Metal. The seeded `OwnerCore` run covers the exec bound and the idle period; no test-only bound or idle override exists.

## Wave: DESIGN / [REF] Formal protocol model

**Applicable.** The shared-guest owner protocol is concurrent (several test processes), ordered (FIFO, one in flight) and crash-sensitive (owner crash, startup failure). Spec: `specs/quint/shared-guest-owner/` (`shared_guest.qnt`, `hazard/guest_hazards.qnt`, `checks.toml`, `README.md`, `evidence/`), produced by the `quint-modeler` and run on Metal (Quint 0.32.0, Apalache 0.56.1, Java 21).

**Instances.** `guest_ok` (full protocol: exhaustive checks, hazards and the environment named runs, the last checked by the evaluator only) and `guest_conf` (the conformance instance: environment actions disabled, owner already started). Every action of `guest_conf` is exactly one `OwnerCore` event (§ Interface contracts, Conformance mapping); the environment actions are outside conformance replay under the exemption approved there (N-4, 2026-11-14).

**Validation budget (design.md cost budget).** 19 checks: 2 exhaustive, 7 hazards and 10 named runs. Wall time of a full `--record` run on Metal: 128.7 s (checks run in parallel); the slowest check, `guest-hazard-bound`, takes 80.5 s. All 19 checks are `ci = true`. The budget of 19 checks and about 2.5 minutes of total wall time is approved (user, 2026-11-14); growth beyond it needs user approval.

Verdicts: **no counterexample against any decision.** D-OWNER (`I_OWNER_UNIQUE`), D-LIFE (`I_NO_ORPHAN`), D-SERIAL (`I_SERIAL`), D-DETACH (`I_DETACH_NO_CANCEL`, `I_DELIVER_ONLY_ATTACHED`), D-TAINT (`I_NO_REUSE`: a Ready guest with no exec in flight is never a bad generation; failure, loss, exec bound and a client leaving while its test runs all taint), D-BOUND (`EveryClientTerminates` with a hung exec possible) and D-IDLE (`I_IDLE_NO_LOSS`) hold; `EveryClientTerminates` holds under weak fairness of the progress actions (not including exec completion), and at most two faults. Seven hazard variants, one per decision, each produce the predicted counterexample: no election lock gives two owners, no in-flight guard gives two concurrent execs, no taint reuses a bad generation, an owner crash that leaves the guest gives an orphan, a disconnect that frees the slot lets the next exec start while the first runs, no exec bound lets a hung exec hold the slot for ever, and an idle exit without its guard drops a waiting client. `I_DELIVER_ONLY_ATTACHED` is structural and has no hazard of its own.

Named runs. `guest_conf` (the `#[quint_test]` pins, all `main = guest_conf`): `happy_path_three_clients_share_one_guest`, `failure_then_replacement_guest`, `guest_lost_mid_exec_then_replacement`, `disconnect_while_running_next_waits`, `disconnect_while_queued_is_removed`, `boot_failure_then_retry_boot_on_next_attach`, `exec_bound_discards_guest_and_replies_bound`, `idle_exit_only_when_empty`. `guest_ok` environment runs (checked by the evaluator, proved against the kernel by Tier 3, not replayed by conformance): `concurrent_first_arrival_one_owner`, `owner_crash_then_new_owner_elected`.
Underspecified points the modeler raised, and how this design pins them: the owner is reactive, so a discard with tests waiting starts the replacement boot in the same event and the observation is `Booting` with generation + 1; a boot that fails with an empty queue leaves `NoGuest` and notifies nobody; a boot failure with tests waiting answers the head and starts a new boot for the next (DDD-9); an attach racing an idle exit sees its connection cut before acknowledgement and re-attaches once (DDD-17; model assumption A-6, discharged by the deterministic client-side re-attach test); owner reported present only once it serves (A-5, a design pin on `OwnerCore::observe()`); a missing owner binary is refused as `LaneNotPrepared` before the lock is taken, so no start-failure state exists; owner death gives attached queued and running tests an owner-loss failure, and a detached test is simply gone; a retry is a new client. Assumptions with a discharging item, discharged on Metal kernel `7.0.0-38-generic` (`spike/findings-owner-lifetime.md`): A-1 (owner is init of its PID namespace, so the VMM dies with it) → **V-4 held**; kernel `flock` as the election arbiter and the socket surviving across network namespaces (A-7) → **V-5 held**. The spec files are modeler output; evidence is recorded and `verify-evidence --subsystem shared-guest-owner` passes. The unscoped `verify-evidence` reports a different subsystem, `guest-flow-owner`, as stale; it is outside this feature.

## Wave: DESIGN / [REF] Validation items

**Contract-shaping items: all run.** Findings: `spike/findings-module-declaration.md` (V-1), `spike/findings-validation.md` (V-2, V-3, V-6, V-7, V-9), `spike/findings-owner-lifetime.md` (V-4, V-5), `spike/findings-const-diagnostic.md` (V-11).

| ID | Verdict | Findings | Effect on the design |
|---|---|---|---|
| V-1 | (a) attribute on a module: DOESN'T-WORK (E0658); (b) function-like macro plus const plus argument-free attribute: WORKS; (c) file-module entrypoint layout: held | `findings-module-declaration.md` | DDD-1 and DDD-7 hold as approved; the const name is pinned (DDD-1); nested modules do not inherit |
| V-2 | WORKS, with three rules | `findings-validation.md` | no `#[test]` insertion; attribute below `#[quint_*]`; `()`-return for guest forms only; 18-test template is U-5 |
| V-3 | WORKS: 20 environments over 8 groups, every one of the 818 tests in exactly one | `findings-validation.md` | profile table adopted (DDD-5), with a `quint` profile for the five gateway tests |
| V-4 | HOLDS: no VMM survives the owner (≤38.6 ms over 13 cycles); production launcher works in the namespace | `findings-owner-lifetime.md` | DDD-8: host pid from `/proc/self`; no `waitpid(-1)`; EOF is owner loss; Quint A-1 discharged |
| V-5 | HOLDS: one owner among 8 simultaneous arrivals; socket reachable from 7 other netns; run directory reclaimed | `findings-owner-lifetime.md` | no change |
| V-6 | not used: the in-guest marker is an environment variable (DDD-10), so the command-line probe has no design role | `findings-validation.md` | none |
| V-7 | inventory complete: 10 lanes | `findings-validation.md` | DDD-11 options A and B for the user (U-2) |
| V-9 | no gate rejects the dev-dependency edges; the rule's literal command does, the graph gates do not | `findings-validation.md` | DDD-12 rule text and the `overdrive-testing` question (U-3); two pre-existing lint failures are out-of-feature findings |
| V-11 | **HOLDS with a different mechanism**: runtime formatting is E0015, but the macro embeds the literal at expansion time and spans the error on it | `spike/findings-const-diagnostic.md` | the pinned build-error text of an unknown group or profile (US-01 AC d, DDD-5) |

Original questions:

| ID | Question | Falsification changes | Where |
|---|---|---|---|
| V-1 | On the pinned stable toolchain: (a) is an attribute proc-macro on `mod x;`, and on an inline `mod` containing `mod y;`, rejected (E0658, `proc_macro_hygiene`; an inner custom attribute needing `custom_inner_attributes`)? (b) does `shared_guest!` plus a no-argument `#[overdrive_test]` compile, bind the module's own declaration by plain path (not a parent's or sibling's), fail to compile in a module with no declaration, and yield an ELF record built by constant evaluation (read back from the linked test binary: kind `guest`, group, profile, sharing `1`)? (c) does it stack with `#[tokio::test]` and `async fn` bodies, leaving one `#[test]` and one record per test, in the `mod integration { mod a; mod b; }` layout with the declaration in a file module? | DDD-1/DDD-7 (the approved module-declaration shape; a failure of (b) or (c) returns it to the user) | host compile probe in `spike-scratch/`, no Metal |
| V-2 | Does `#[overdrive_test(...)]` stack correctly over `#[tokio::test]`, a `proptest!` block's tests, `#[quint_test]`, `#[quint_run]` and trybuild-style tests, leaving one record per test and no duplicate `#[test]`? Count the tests in the workspace that cannot take an attribute. | the grammar and the migration stop rule | host compile probe plus reading the records |
| V-3 | Inventory the dispatcher's per-test variants (the `Cell` tables, `NESTED_GUEST_LAUNCH`, `focused_selection`, the mtls native/relay split, the literal test name): how many distinct (group, variant) environments, and do they reduce to a closed named set? | DDD-5 (profiles versus groups; the `profile` argument) | parse `guest_suite.rs`, no Metal |
| V-4 | With the owner as init of a fresh PID namespace, does `SIGKILL` of the owner leave no VMM within a bound (and does the production launcher work inside the namespace)? Read `kernel/pid_namespace.c` (`zap_pid_ns_processes`) and `kernel/exit.c` in `vendors/linux` at `v7.0` first (`git submodule update --init vendors/linux`), cite `path:line`, then run it. | DDD-8 / ADR-0283 | Metal ordinary run |
| V-5 | Does a socket under `/run/ovd-gk/<run id>/shared/` connect from a test process in a different local-mode vsock network namespace, is the directory reclaimed at run end, and do two simultaneous arrivals elect exactly one owner through `flock`? | the rendezvous location and election | Metal ordinary run |
| V-7 | Which lanes run guest-placed tests off Metal today (CI `integration` job, pre-push hook, `infra/hooks/linux-run.sh`, others) and which of the guest-placed tests compile without `kvm-tests`? | DDD-11 (the user's choice needs the list) | grep and reading |
| V-9 | Do `dst-lint`, `cargo tree -p xtask`, `cargo deny` or crate-class rules reject a dev-dependency from a `core` crate or `xtask` onto `overdrive-testing-macros`? | DDD-12 | repository gates, no Metal |
| V-11 | On rustc 1.95.0 (the pinned stable): (a) does a `const fn` called in constant context accept `panic!("... {}", s)` with a runtime `&str` argument (expected: E0015, no); (b) with a literal-only message, does the rustc error for `const _: GuestPlacement = GuestPlacement::declare("<unknown>", ..)` generated by a macro that re-quotes the user's own string literal point its span at that literal, and print the panic message; (c) does the same hold through `#[overdrive_test(guest = "<unknown>")]` and through `shared_guest!`? Record the exact error text for a fixed message per failure kind. | the pinned diagnostic text and whether the offending value can be named in it | host compile probe in `spike-scratch/`, no Metal |

**Not contract-shaping (may stay open; each blocks only its DELIVER step):** console completeness for a failing shared test (the probe used a 300 ms flush sleep, which is an upper bound and not a design); boot-time distribution per profile (the skeleton has two samples); whether a profile's per-test setup scripts are idempotent across consecutive tests in one guest (checked per migrated module at DELIVER).

## Wave: DESIGN / [REF] Scope limitations

None. The design cuts nothing the feature was asked to deliver, so no tracking issue is proposed (user ruling, 2026-11-14). Properties that follow from the chosen mechanism are not scope limitations:

- a guest is shared within one module of one test binary (DDD-7: that is the sharing contract, not a cap on it);
- the set of guest groups is closed and grows by adding a registry entry (DDD-5);
- a lane fixture's name is validated in form at build time, and its existence when the test runs, because one crate's build cannot see another crate's tests (DDD-1);
- execs in a shared guest are serialised (DDD-9).

## Wave: DESIGN / [REF] Open questions and blockers

No contract-shaping validation item is Open: V-1 to V-5, V-7, V-9 and V-11 are run and recorded, and V-6 has no design role (§ Validation items). DESIGN is complete: every decision is approved by the user and independent DESIGN review (three rounds) is finished. The feature is ready for DISTILL.

**New material decisions for the user (N-1, N-2 and N-3 APPROVED, 2026-11-14).**

| # | Decision | Contract | Alternatives | Cost |
|---|---|---|---|---|
| N-1 | DDD-16: a shared exec has a bound | each profile's exec bound is its existing cell bound (300 s; nested 600 s; `nested-cohort` 1,200 s); at the bound the owner discards the guest and the test fails with `GuestExecution` stage `exec_bound`; isolated guests keep the existing attempt bound | rely on nextest alone (24 h slot hold); kill only the in-guest test (guest-side kill channel); per-test bound (new surface) | one timer per exec; one `OwnerCore` event; a test that legitimately outlasts its profile bound cannot live in a shared module and takes an explicit isolated `guest = ..` form; model: one action, one hazard, one run |
| N-2 | **APPROVED (user, 2026-11-14).** DDD-17: a shared owner exits when idle; no anchor on the nextest process | `IdleElapsed` after `SHARED_GUEST_IDLE` (30 s) with nothing queued, running or attached; exit ends the guest; an attach racing the exit re-attaches once; owners hold no handle on nextest and the wrapper exports no anchor variable | no idle exit and a stated maximum; anchor on the nextest process (pidfd opened before the PID namespace); a cap on live shared guests; a timer-free exit (rejected: boots a guest per test) | one timer; one event and effect; the period costs only boots, never results; model: one action, one hazard, one run |
| N-3 | DDD-18: a client that goes away while its test runs taints the guest | when that exec ends (any verdict or the bound) the guest is discarded | keep the guest after a timed-out test passes; discard at the disconnect (cancels mid-effect) | one extra boot (about 3 s) after each nextest kill of a shared test; no new mechanism (extends the existing discard rule) |

**Conformance exemption (user decision N-4, APPROVED 2026-11-14).**

| # | Decision | Contract | Alternatives | Cost |
|---|---|---|---|---|
| N-4 | **APPROVED (2026-11-14).** The environment actions of the shared-guest owner model are exempt from conformance replay | `Arrive`, `ElectBegin`, `OwnerStarted` and `OwnerCrash` stay in the model (`guest_ok`) but are not driven through `OwnerCore` by a `quint-connect` driver, because the operating system or the client shell performs them and an owner crash leaves no process to receive an event; this departs from `.claude/rules/testing.md` § "Per-driver checklist" items 3 and 6 for those actions. Per invariant: `I_OWNER_UNIQUE` is proved by the Tier-3 scenario in which eight simultaneous first arrivals elect exactly one owner on the production lock path (V-5); `I_NO_ORPHAN` by the Tier-3 scenario in which SIGKILL of the owner during a guest exec leaves no VMM (V-4); every other invariant (`I_SERIAL`, `I_NO_REUSE`, `I_DETACH_NO_CANCEL`, `I_DELIVER_ONLY_ATTACHED`, `I_IDLE_NO_LOSS`, `EveryClientTerminates`) is constrained by actions the conformance instance replays. DISTILL records one exemption row per invariant, naming its Tier-3 scenario | model the election and the owner start as `OwnerCore` events with a `Sim*` election (a driver that checks the model against a copy of the lock, which `testing.md` rejects); drive the real lock from the conformance driver (a multi-process fixture inside a conformance test, for an invariant the kernel's lock gives) | none beyond the two Tier-3 scenarios, which are required anyway; the exemption removes no check |

**User decisions (numbered; all APPROVED by the user, 2026-11-14).**

| # | Decision | Status | Recorded in |
|---|---|---|---|
| U-1 | DDD-3: add `cargo xtask placement-lint`, wired into CI and pre-push beside `dst-lint` and `conformance-lint`, with the `bootstrap.sh` grammar entry; extension: it also checks that every lane fixture named by `lane_operator` / `lane_owner` exists as a test in the binary that provides it (it reads every binary's placement records) | APPROVED (user, 2026-11-14) | § DDD-2 and DDD-3; § Interface contracts, `placement-lint` |
| U-2 | DDD-11: off a Metal run (no Metal run context: CI, Lima, pre-commit and pre-push on Lima, `cargo xtask ci`, mutants) a guest-placed or lane-placed test runs its body directly in the current process, as today; not fail-closed; `kvm-tests` gating unchanged; on a Metal run a guest-placed body never runs on the host | APPROVED (user, 2026-11-14), option B | § DDD-11; ADR-0286 |
| U-3 | DDD-12: the `development.md` xtask rule's command becomes `cargo tree -p xtask -e normal,build \| rg overdrive-` (dev-dependencies allowed; the orchestrator edits the rule); `xtask` takes `overdrive-testing` as a dev-dependency for its guest-placed test | APPROVED (user, 2026-11-14) | § DDD-12 |
| U-4 | DDD-4: both a per-binary default (`overdrive_test_default!(host_pure)`, only in a binary with no guest-placed, lane-placed or native-launcher test) and a per-module declaration (`placement!(host_pure)`, module-scoped record M2); precedence own attribute > module declaration > binary default; a child module does not inherit; `placement_conflict` for a default beside a non-host-pure record | APPROVED (user, 2026-11-14) | § DDD-4; § Interface contracts |
| U-5 | DDD-1 and DDD-15: one attribute inside the `equivalence_cases!` `macro_rules!` template (`gateway_relay_equivalence.rs:616`); the migration script special-cases it, checked by the parity diff (T1) | APPROVED (user, 2026-11-14) | § DDD-1; § DDD-13 to DDD-15 |
| U-6 | No tracking issues: the design imposes no scope limitation | APPROVED (user, 2026-11-14) | § Scope limitations |

**No decision remains PROPOSED.** The user approved on 2026-11-14: the DDD-1 details (no `#[test]` insertion, position below `#[quint_*]`, guest-forms-only `()` rule, the hidden const's name, explicit per-test form overrides `shared_guest!`), DDD-2, DDD-5, DDD-6, DDD-7, DDD-8, DDD-9, DDD-10, DDD-13, DDD-14, DDD-15, and the Quint validation budget of about 2.5 minutes (19 checks). The conformance exemption N-4 and DDD-16 to DDD-18 (N-1 to N-3) were approved the same day. The shapes of DDD-1 and DDD-7 were approved 2026-11-13.

**Out-of-feature findings for the user (not design scope).** V-9 found two lints failing on this branch's base before any edge was added: `cloexec-lint` (`overdrive-dataplane/src/gateway_relay/sys.rs:87:31`) and `conformance-lint` (`overdrive-testing/src/guest_suite.rs:183:5 [spec-without-quint-connect]`). `cargo deny check bans sources` also fails at baseline (17 wildcard errors, `anyhow` banned) and runs in no lane. The `conformance-lint` hit is in a file this feature rewrites (`guest_suite.rs` splits and its batched half is deleted, DDD-6, DDD-14); whether it disappears with the deletion is for the crafter to observe, not a design claim.

Handoff notes. External integrations: none (no third-party API); no consumer-driven contract tests apply. The record format is a three-reader contract (macro writes, wrapper and `placement-lint` read): one fixture test binary read by both readers is the drift guard. Enforcement tooling: `placement-lint` (new, DDD-3), the wrapper refusal, the owner Quint conformance driver under `conformance-lint`, and the existing dst-lint, which does not scan these `adapter-host` crates. Recommended DISTILL scenarios per Lifecycle Gate Ownership above and per `.claude/rules/testing.md` § "DISTILL": each gate with a healthy control and one deliberate fault. No roadmap is created in DESIGN.

## Wave: DISTILL / [REF] Inputs read and reconciliation

Run 2026-11-14 by the acceptance designer, on the branch that carries the promoted walking skeleton. Mode: DISTILL for the whole feature, with project rules overriding the generic nWave skill (`.claude/rules/testing.md`: Rust `#[test]`, no `.feature` files, pending-marked complete bodies; `design.md`: implement to the design; `domain-naming.md`).

| Input | Read |
|---|---|
| This file (DISCUSS, DESIGN, interface contracts, G-1..G-6, boundary scenarios, formal model, N-4, validation items, profile registry) | ✓ |
| `slices/slice-01..09` | ✓ 01, 02, 03, 04, 07, 09 in full; 05, 06, 08 by their stories here |
| `spike/` (`findings*.md`, `wave-decisions.md`) | ✓ `wave-decisions.md`, `findings-const-diagnostic.md`, `findings-module-declaration.md` (pinned diagnostics); the others by their summaries here |
| Walking skeleton (`crates/overdrive-testing-macros/`, `guest_suite.rs`, `test-wrapper.sh`, the migrated `overdrive-cli` test) | ✓ built on, not rewritten; the migrated test still passes on Metal |
| ADRs 0282-0295, `brief.md`, `c4-diagrams.md`, journey, `jobs.yaml` | ⊘ not re-read: the feature delta carries the contracts they record (DESIGN review approved the set) |
| `specs/quint/shared-guest-owner/` (`shared_guest.qnt`, `checks.toml`, README) | ✓ |
| DEVOPS delta | - missing: WARN, project defaults used (the Metal box, `integration-tests,kvm-tests`, the Metal run grammar) |
| `docs/architecture/atdd-infrastructure-policy.md` | ✓ exists; rows appended for the ports this feature adds (`--policy=inherit`) |

Wave-decision reconciliation: DISCUSS and DESIGN decisions read (no separate `wave-decisions.md` exists for either; the decision tables above are the record); **0 contradictions**. The one place DESIGN's text and the code disagree is a premise, not a decision (G-2's run-id claim), and is corrected in G-2 (D-1, "Design gaps and proposals").

## Wave: DISTILL / [REF] Scenario list with tags

The scenario specification is `distill/test-scenarios.md` (Given/When/Then, Contract Shape, owner path, fault stimulus, oracle, evidence tier, the Quint spec/run/invariant each conformance test replays, and the N-4 exemption rows). Every scenario is a Rust test with a complete executable body.

| Crate (binary) | Tests | Active | Pending | Tags |
|---|---|---|---|---|
| `overdrive-testing` (lib) `placement::owner_core::tests` | 18 | 0 | 18 | `@in-memory @property @error @boundary` |
| `overdrive-testing` (lib, `integration-tests`) `placement::owner_core::conformance` | 9 | 0 | 9 | `@conformance @in-memory` |
| `overdrive-testing` (lib) `placement::tests` | 6 | 0 | 6 | `@in-memory @error` |
| `overdrive-testing::compile_fail` | 1 (5 fixtures) | 0 | 1 | `@error` |
| `overdrive-testing::integration` (`integration-tests,kvm-tests`) | 17 scenarios (+15 fixtures) | 0 | 17 (+15 fixtures) | `@real-io @error @boundary` |
| `xtask::acceptance` `placement_attribute` | 28 (22 scenarios + 6 fixtures) | 0 | 28 | `@real-io @error @boundary` |
| **Total** | **79** (+15 integration fixtures) | **0** | **79** (+15 fixtures) | |

Per story: US-PLACE-01 (A skeleton delivered; typed registry, in-place and run-id scenarios here), 02 (E), 03 (E, F), 04 (C; see gaps), 05a-c and 06a-c (no acceptance test: the migration parity check of DDD-15 is the batch's procedure, see gaps), 07 (E), 08 (E, two repository-state tests), 09 (A, B, F). Most pending scenarios are error, fault or boundary paths (tagged `@error` and `@boundary` in `test-scenarios.md`); the happy paths are the skeleton's active tests and the first example of each area.

## Wave: DISTILL / [REF] WS strategy

The walking skeleton (slice 01) is promoted production code and green; DISTILL adds scenarios on top of it and does not rewrite it. Port classes (Architecture of Reference, project policy file): driving = the wrapper (`--check-placement`, an ordinary run), `xtask placement-lint`, `run_in_guest`, `OwnerCore::handle`; driven internal (real) = the guest kit and Cloud Hypervisor guests, the election `flock`, the owner's socket, the wrapper's ELF reader; driven external (faked) = `cargo nextest list` only. No Strategy A to D selection applies (retired in the skill); `@walking_skeleton` scenarios exist as the delivered migrated test and the active timing and isolation tests of section F.

## Wave: DISTILL / [REF] Adapter coverage

| Adapter | Real-I/O scenario | Covered by |
|---|---|---|
| Guest kit and Cloud Hypervisor guest | YES | `guest_placed_tests_outside_a_shared_module_boot_one_guest_each`, `the_boot_time_of_a_guest_per_test_is_recorded_in_the_run_output` (active); the shared-guest scenarios (pending) |
| Election `flock` and owner socket | YES (pending) | `eight_simultaneous_first_arrivals_elect_exactly_one_owner` and the four owner-fault scenarios; the client protocol through `attach_shared` (missing owner binary, one re-attach) |
| Owner PID namespace (VMM dies with the owner) | YES (pending) | `killing_the_owner_during_a_guest_exec_leaves_no_vmm` |
| Wrapper placement reader (ELF section) | YES (pending) | section E, hand-built binaries and this binary's own records |
| Wrapper isolation (namespaces) | YES | existing `metal_run_isolation` tests; `a_test_that_declares_itself_host_pure_runs_without_a_namespace` and the native-launcher view test (pending) |
| `cargo nextest list` (lint input) | stub only | `placement_lint_*` fake it, because a nested `cargo` inside a test is forbidden; the real path is the CI and pre-push step itself (see gaps) |
| `quint` CLI (conformance replay) | YES (pending) | section B, driven by `quint-connect` under `integration-tests` |

## Wave: DISTILL / [REF] Driving Adapter coverage

| Driving adapter (DESIGN) | Scenario that invokes it by its protocol |
|---|---|
| `#[overdrive_test(...)]` and the declaration macros on a test | the fixtures of `xtask::acceptance` and `overdrive-testing::integration`, read back through the wrapper and the checker; `compile_fail` for the diagnostics |
| `cargo xtask metal run -- cargo nextest run ...` | every Metal run of the suite; the drivers of section F run the fixture binary as nextest does |
| `test-wrapper.sh --check-placement` and an ordinary wrapper run | section E (`std::process::Command`, exit status, stderr diagnostics) |
| `cargo xtask placement-lint` | section E lint tests (subprocess of the built `xtask`, exit status, output lines) |
| `OwnerCore::handle` / `observe` | sections A and B |

## Wave: DISTILL / [REF] Scaffolds

Production scaffolds, only where the design pins the exact shape; every body is marked `SCAFFOLD: true` and built by the named slice.

| File | Scaffold | Built by |
|---|---|---|
| `crates/overdrive-testing-macros/src/lib.rs` | the grammar's other forms are accepted and return the item unchanged; `placement!` and `overdrive_test_default!` expand to nothing; the existing `guest = "<group>"` expansion is untouched | `host-pure-placement-on-the-test`, `lane-and-launcher-placement-on-the-test`, `guest-placement-on-the-test` |
| `crates/overdrive-testing/src/lib.rs` | re-exports of the two macros; `shared_guest!` (`macro_rules!`, the pinned const name, stores its arguments) | `module-tests-share-one-guest` |
| `src/placement/mod.rs` | `in_metal_run`, `metal_run_context_present`, `run_in_guest` (typed placement), `run_lane`: `todo!()`; `inside_guest` and `RunInGuestError` are the skeleton's | `guest-placement-on-the-test`, `lane-and-launcher-placement-on-the-test` |
| `src/placement/groups.rs` | `GuestGroup`, `Sharing`, `GuestPlacement::declare` (stores, validates nothing), `LanePlacement` | `guest-placement-on-the-test`, `guest-artifacts-declared-per-group` |
| `src/placement/owner_core.rs` | `AttachId` (real), `Event`, `Effect`, `Verdict`, `GuestView`, `OwnerObservation`, `OwnerCore::{new, handle, observe}`: `todo!()` | `module-tests-share-one-guest` |
| `crates/overdrive-testing/Cargo.toml` | dev-dependencies for the tests; the `kvm-tests` feature (the convention of the five guest-booting crates) | - |
| `xtask/Cargo.toml` | dev-dependency on `overdrive-testing-macros` (DDD-12, approved) | - |

## Wave: DISTILL / [REF] Test placement

Tests sit with their owning crate (`testing.md`): the owner core and registry in `overdrive-testing/src/placement/**` (default lane, fast); the conformance driver in the owning crate's `src/` behind `#[cfg(all(test, feature = "integration-tests"))]`; Metal scenarios in `overdrive-testing/tests/integration/` behind `integration-tests` and `kvm-tests` (no off-Metal lane enables `kvm-tests`, so none of them runs in CI on Lima); the wrapper and lint tests in `xtask/tests/acceptance/` beside the existing `metal_run_isolation` tests. Precedent: `crates/overdrive-cli/tests/integration.rs` (inline `mod integration`), `xtask/tests/acceptance.rs`.

Placement of the new tests: **DISTILL adds no `infra/metal/test-placement.json` entry** (the feature exists to retire that table). Every test of this feature is pending (`#[ignore = "pending DELIVER: <slice>"]`), so none reaches the Metal wrapper on an ordinary run and none needs a placement. **Activating a test includes giving it its attribute placement** in the slice that delivers that form (`#[overdrive_test(guest = "<group>")]`, `host_pure`, `native_launcher`, the lane forms, `placement!`, `overdrive_test_default!`); a test whose form is not built yet stays pending until then, and no test is ever placed by a table entry. `native_launcher` is never a product test's placement (`testing.md` § "Test placement"); the ruling for tests of the runner's own machinery (D-10) is in § Interface contracts, Shared clauses. Placement forms on activation: the Metal scenarios of `shared_guest_scenarios`, `shared_owner_lifetime`, `placement_off_metal` (the Metal-run and invalid-run-id scenarios) and `lane_placement` are `#[overdrive_test(native_launcher)]` (they observe the host from outside any guest); `a_guest_placed_test_runs_its_body_in_place_when_no_metal_run_is_present` and the two `attach_shared` scenarios observe nothing outside their own process and are `#[overdrive_test(host_pure)]`. Fixtures that exist only to be driven by a pending scenario are pending with it and activated in the same slice; the fixtures that already carry `#[overdrive_test(guest = "cgroup-default")]` (`isolated_guest`) keep it. The shared-module fixtures keep the argument-free `#[overdrive_test]` form beside `shared_guest!`: an explicit `guest = ..` would give each its own guest and defeat the sharing they exist to show. `.config/nextest.toml` gains one override putting the `overdrive-testing` integration binary in `host-kernel-shared` with a wider slow-timeout (the scenarios boot guests and kill owner processes).

## Wave: DISTILL / [REF] Pending tests and the slice that activates each

The `#[ignore]` reason is `pending DELIVER: <slice>` (the slice file stem; no roadmap exists yet, so no step identifier is used).

| Slice | Pending tests | Test names (see `red-classification.md` for the per-test evidence) |
|---|---|---|
| `host-pure-placement-on-the-test` | 19 | the wrapper and checker tests of section E that read `host_pure`, `module_host_pure`, `default_host_pure` and invalid or unplaced sections; the two registry tests of `placement::tests`; `a_declared_test_with_an_invalid_run_context_is_refused_by_the_run_context_check`; the five host-pure fixtures of `placement_attribute` (activated with them) |
| `guest-placement-on-the-test` | 5 (+3 fixtures) | `an_unknown_group_is_rejected_by_name_at_declaration`, the two `in_metal_run` predicate tests (driven through `in_metal_run()`), the diagnostics test, `a_guest_placed_test_runs_its_body_in_place_when_no_metal_run_is_present` (`host_pure`); the three `isolated_guest` fixtures |
| `lane-and-launcher-placement-on-the-test` | 8 (+1 fixture) | `every_placement_a_test_declares_on_itself_is_placed`, the native-launcher node-view test, the lint missing-fixture test, `a_lane_test_naming_a_missing_fixture_fails_naming_the_fixture_and_the_test`, and the four runner-machinery scenarios placed `native_launcher` (D-10): `guest_placed_tests_outside_a_shared_module_boot_one_guest_each`, `the_boot_time_of_a_guest_per_test_is_recorded_in_the_run_output`, `a_guest_placed_test_never_runs_its_body_on_the_host_on_a_metal_run`, `a_guest_placed_test_with_an_invalid_run_id_fails_naming_its_group_and_never_runs_on_the_host` |
| `unplaced-test-fails-in-the-default-lane` | 5 | the `placement_lint_*` tests except the missing-fixture one |
| `guest-artifacts-declared-per-group` | 1 | `an_unknown_profile_or_one_of_another_group_is_rejected` |
| `module-tests-share-one-guest` | 38 (+12 fixtures) | 18 owner-core tests, 9 conformance tests, 9 Metal scenarios of section F (`native_launcher`), 2 `attach_shared` scenarios (`host_pure`, § Design gaps D-6); the `shared_module` and `scenario_clients` fixtures |
| `placement-table-and-dispatcher-retired` | 2 | the two repository-state tests |
| `guest-placements-migrated`, `host-pure-placements-migrated` | 0 | the batch parity check of DDD-15 |

## Wave: DISTILL / [REF] Pre-requisites

The Metal box with KVM, the prepared guest kit and the loaded module (`OVERDRIVE_METAL_TARGET`, `OVERDRIVE_METAL_KERNEL`, `OVERDRIVE_METAL_ROOTFS`); `python3` and `bash` for the wrapper tests; the pinned Quint toolchain on the box for the conformance replay. Run commands: `cargo xtask metal run -- cargo nextest run -p overdrive-testing -p xtask --features overdrive-testing/integration-tests,overdrive-testing/kvm-tests,xtask/integration-tests -E 'package(overdrive-testing) | test(placement_attribute)'` (add `--run-ignored only` to see the pending set).

## Wave: DISTILL / [REF] Design gaps and proposals

Every gap is closed in the DESIGN contract (§ Interface contracts, G-2 and the sections named below); none blocks DELIVER. The table records the resolution and the change to the tests. Nothing below was added to production code.

| # | Gap | Resolution | Change to the tests |
|---|---|---|---|
| D-1 | G-2 claimed that a Metal run context without a valid run id is refused by `run_in_guest`'s run-runtime rule; `runtime_staging_root()` refuses only a set but malformed id (`guest_kit.rs:423-438`). | G-2 and the `run_in_guest` preconditions state it exactly: no variable set means no Metal run and the body runs in place; a malformed id is refused as `GuestExecution`; an absent id with another variable set is still handed to `run_in_guest`, never run in place. | none: the malformed-id scenario stands; its comment no longer points at a gap |
| D-2 | `declare` and a `const` need `'static` strings. | `GuestPlacement::declare` takes `&'static str` for `group`, `profile` and `declaring_module` (§ Guest kit public surface). | none |
| D-3 | The derives on the owner-core types were unpinned. | Pinned: `Clone, Debug, PartialEq, Eq` on `Event`, `Effect`, `Verdict`, `GuestView`, `OwnerObservation` (§ Owner core). | the mirror types in `owner_core/tests.rs` and `owner_core/conformance.rs` are removed; assertions compare the core's own values |
| D-4 | The pure predicate core's parameter types were unpinned. | They are the crafter's, not contract; `in_metal_run()` is the contract (§ Guest kit public surface). | the two predicate tests drive `in_metal_run()` under `#[serial(env)]` with an environment guard, not `metal_run_context_present` |
| D-5 | The skeleton inserts `#[test]` when absent, so `#[test]` above `#[overdrive_test(guest = ..)]` registers the test twice. | DDD-1 / V-2: the macro never inserts `#[test]`; both attribute orders yield one test. | none until the guest-placement slice; the fixtures' order is the crafter's to keep valid |
| D-6 | The missing-owner-binary refusal, the DDD-17 re-attach after a cut before acknowledgement, a guest that cannot boot, and the exec-bound and idle durations had no pinned seam. | The shell computes the rendezvous paths and owner binary path and passes them to the client protocol, so that entry point is the natural production shape and is pinned: `SharedEndpoint` and `attach_shared` (§ Guest kit public surface; `run_in_guest` is its only production caller). It unlocks two scenarios: the missing owner binary (`LaneNotPrepared` naming the path, lock not created) and the re-attach (one re-attach, a second cut is `GuestExecution` at stage `owner`), which also discharges model assumption A-6. A guest that cannot boot has no client-side seam and is not added: the owner side is covered by `a_boot_failure_answers_only_the_head_and_starts_a_new_boot_for_the_next`, `a_boot_that_fails_with_nobody_waiting_leaves_no_guest_and_notifies_nobody` and the conformance run `boot_failure_then_retry_boot_on_next_attach`; the client's mapping of `boot_failed`, `guest_lost` and `exec_bound` to `GuestExecution` is a total `match` on `Verdict` with no test (the wire encoding is the crafter's). The exec bounds and `SHARED_GUEST_IDLE` are read only by the owner shell; their values are behaviour, their names are internal, and the seeded `OwnerCore` run covers the bound and idle events, not their durations. | two new scenarios in `tests/integration/shared_client_protocol.rs` (`host_pure`, slice `module-tests-share-one-guest`); US-01 AC c (a guest that cannot start) is covered by the malformed-run-id scenario for the per-test path |
| D-7 | The per-test path forwards no pinned environment, so a per-test failing-body scenario (US-01 AC b) has no in-guest toggle. | No seam is added (it would exist only for a test). US-01 AC b is narrowed to the shared path: `one_failing_test_in_a_shared_module_does_not_hide_the_others` and `a_failed_shared_test_is_reported_with_its_status_and_its_guest_is_replaced` cover propagation of a failing in-guest body; the per-test path's `BodyFailed` mapping has no scenario of its own. | none; no scenario is added |
| D-8 | US-PLACE-04's scenarios (a tool added to one group only, a missing artifact naming group and artifact, an absent artifact path) need a way to alter a group's declaration under test. | No seam is added. The scenarios are dropped. Coverage that remains: the registry's names (`the_registry_accepts_exactly_the_twenty_one_named_environments`, `an_omitted_profile_selects_each_groups_base_profile`, `an_unknown_profile_or_one_of_another_group_is_rejected`) and the existing `guest_suite` staging tests; migration parity (DDD-15) checks the artifacts per batch. | none; no scenario is added |
| D-10 | Metal scenarios that observe the host from outside any guest cannot be placed in a guest group (inside a guest the kit, its artifacts and the run's namespaces are absent and `inside_guest()` holds). | **User ruling 2026-11-14:** they are tests of the Metal runner's own placement machinery and are placed `#[overdrive_test(native_launcher)]` through the attribute once that form lands (lane-and-launcher slice), never by a table entry (§ Interface contracts, Shared clauses). Applies to `guest_placed_tests_outside_a_shared_module_boot_one_guest_each`, `the_boot_time_of_a_guest_per_test_is_recorded_in_the_run_output`, `a_guest_placed_test_never_runs_its_body_on_the_host_on_a_metal_run`, `a_guest_placed_test_with_an_invalid_run_id_fails_naming_its_group_and_never_runs_on_the_host` and the scenarios of `shared_owner_lifetime`. The same property holds for the three scenarios of `shared_guest_scenarios` (they read the host half's timing lines) and for `a_lane_test_naming_a_missing_fixture_fails_naming_the_fixture_and_the_test` (it calls `run_lane` on the host); they take the same placement. | the four scenarios move to the lane-and-launcher slice; every Metal scenario above carries `#[overdrive_test(native_launcher)]` when activated; `a_guest_placed_test_runs_its_body_in_place_when_no_metal_run_is_present` observes nothing outside its own process and takes `host_pure` |
| D-9 | The real `cargo nextest list` path and the CI and pre-push wiring of `placement-lint` have no acceptance test (a nested `cargo` inside a test is forbidden; a test that reads `lefthook.yml` for a command name would be theatre). | The lint's own CI step (and its pre-push step) runs the real `cargo nextest list` path on every change; that is its coverage (G-4). The acceptance tests stub only `nextest list`. | none |

Items added to the tree that the user may want to rule on: the `kvm-tests` feature on `overdrive-testing` and the `host-kernel-shared` override (without them the guest scenarios would run in the Lima CI lane and fail there).

## Wave: DISTILL / [REF] AT completeness audit and mandate evidence

15-item checklist (`nw-at-completeness-check`): **12 of 15 pass**, verdict ACCEPTABLE_WITH_DOCUMENTED_GAPS. Passing: C1a (empty queue, empty profile, no variables), C1b (no numeric partition; not applicable), C2a (state machine in the module and spec), C2b (`events_that_are_not_enabled_change_nothing`), C3 (zero, one, three, eight clients; zero, one, two findings), C4a, C4b, C5a (placement forms by sharing), C5b (the explicit form overriding a module declaration), C6a (unknown group, profile, truncated section), C7b (killed client, killed owner, killed elector), C7c (eight simultaneous arrivals). Failing, each documented: C6b (the client's mapping of `boot_failed` has no scenario, by the D-6 resolution; the owner-binary-missing and re-attach errors are covered through `attach_shared`), C6c (the closed error set of the wrapper is not asserted as a set), C7a (no degraded-resource scenario; no DEVOPS environment matrix exists, so this is a specification gap routed to DEVOPS, not a delivery gap). No `SPECIFICATION_AMBIGUITY` blocker.

Mandate evidence for a Rust project: CM-A (every test enters a driving port: wrapper, lint, `run_in_guest`, `OwnerCore::handle`; no internal component is imported; the one in-crate predicate core is the design-named pure function, D-4); CM-B (names are domain statements, no step identifiers or issue numbers in any name, helper or message); CM-D (pure cores are tested directly, adapters through real subprocesses); CM-E (universe-bound assertions are observation-level, see the note in `test-scenarios.md`); CM-F (property machinery only at layer 1-2: the owner core; layer 3 scenarios are example-only); CM-G (no Tier B state-machine file: the conformance replay and the seeded schedules already explore the state space); CM-H (every Tier-3 sad path is a named example test). The Python-specific Mandate-12 criteria (`domain_types.py`, step decorators) do not apply: there are no step files.

Conformance checklist for the `OwnerCore` driver (`testing.md` § "Per-driver checklist"): (1) in `src/` behind `#[cfg(all(test, feature = "integration-tests"))]`, only gate `integration-tests`, no always-false cfg, pending-marked; (2) the driver struct holds the owner core; (3) every `switch!` arm calls `handle`, no `_` arm, the four environment actions are exempt under N-4 with one row per invariant; (4) every `State` field maps to a spec variable through `observe()`; (5) no `Sim*` and no driver bookkeeping in the projection; (6) all eight named `guest_conf` runs have a `#[quint_test]` and a `#[quint_run]` exists; (7) the target module has a `step` relation and nothing spawns the `quint` CLI. `cargo xtask conformance-lint` exits 0 with the driver present.

## Wave: DISTILL / [REF] Hand-off

DISTILL adds no `test-placement.json` entries; table entries are only ever deleted by this feature. Every test of this feature is pending with a reasoned `#[ignore]`; activating a test includes giving it its attribute placement, and a placement form that does not exist yet keeps its tests pending. RED classification of the tests that were active before this correction (the registry tests, the host-pure and native-launcher fixtures, the run-context test, the shared-guest and off-Metal scenarios and the fixtures they drive) is taken when their slice activates them.
