<!-- markdownlint-disable MD024 -->
# Feature delta — `metal-test-placement-attribute`

DISCUSS, 2026-11-13. Infrastructure (test tooling), brownfield, changes existing behaviour. Density `lean`, Tier-1 `[REF]` sections only. One job, twelve stories in eight slices, walking skeleton promoted to production code. Source: [GitHub issue #354](https://github.com/overdrive-sh/overdrive/issues/354) (reference only; no issue number appears in names). Requirements only — DESIGN owns every shape not named below.

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
3. Marcus runs the migrated test outside any Metal context (a laptop shell); it must not report PASS without its body having run in a guest.
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

Scenario: A guest test outside Metal never reports a false PASS
  Given the migrated test is run with no Metal context
  Then it does not report PASS unless its body ran in a guest (fail closed or skipped with a visible reason; DESIGN picks)

Scenario: Guest boot time is recorded per test
  Given the migrated test ran with one guest for that test
  When Marcus reads the retained run output
  Then the guest boot time for that test is recorded for later comparison

Scenario: Tests still in the table are unaffected
  Given the rest of the table and the dispatcher are untouched
  When Marcus runs any not-yet-migrated guest test
  Then it runs through the dispatcher exactly as before and passes
```

AC: (a) migrated test PASSes on Metal with its JSON entry deleted; (b) in-guest failure surfaces as host FAIL with the run exiting nonzero; (c) boot failure or timeout is a FAIL naming group and stage; (d) an unknown group is rejected by name; (e) outside Metal the test never reports a body-less PASS; (f) per-test boot time retained; (g) unmigrated guest tests still pass through the dispatcher after the extraction. Technical notes: the host-side body needs the local-mode vsock namespace it has today; telling a self-declared test from an unplaced one while the table coexists is O1.

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

Domain examples: (1) a new plain `#[test]` `integration::new_mesh_case::dials_by_name` in an integration binary fails the check naming binary and test; (2) the same test with a declaration passes; (3) a test in the unit lane (not an integration binary) with no declaration is not flagged; (4) a declared test inside a binary that still has a table entry for another test does not trip the check.

```gherkin
Scenario: A plain integration test is flagged
  Given an integration binary contains a test with no placement declaration
  When Marcus runs the check
  Then it fails naming the binary and the test, without a Metal guest boot

Scenario: A declared test passes the check
  Given every integration test in the binary declares a placement
  Then the check passes

Scenario: Unit-lane tests are not flagged
  Given a test outside the integration binaries has no declaration
  Then the check ignores it

Scenario: The check is wired where Marcus already runs it
  Given Marcus pushes a branch that adds an unplaced integration test
  Then the check fails in the step the repository already runs for that surface
```

AC: unplaced integration test fails before Metal naming binary and test; declared tests pass; unit-lane tests ignored; check runs in a command already in Marcus's loop (adding a new lane or tool requires user approval first). Notes: mechanism and how it lands before full migration are O3.

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

Real, promoted, not a mock. Spike in walking-skeleton mode with no probe (the dispatcher already demonstrates in-guest `--exact`). Increment: US-01 as in D4. The spike records the promotion-gate decision in its own `wave-decisions.md`; DESIGN may reshape items 1-2. Run on the native Metal box per `.claude/rules/testing.md`; Lima is no signal for guest boots. Until the spike's gate decision, the skeleton code is "pending promotion", not "promoted".

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
