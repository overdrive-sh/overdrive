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

## Wave: DESIGN / [REF] Inputs read

PROPOSE mode, scope Application/components, OOP paradigm, all priorities (P1 and P2: O1-O5 and every question the dispatch listed). Every decision below is **PROPOSED**; none is accepted until the user approves it by number (`.claude/rules/design.md` § "Surface material design decisions").

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
| GitHub issue #354 body and comments | ⊘ no shell in this dispatch; the DISCUSS summary stands in. The orchestrator must read `gh issue view 354 --comments` before relaying the proposed issues below |
| `brief.md`, ADR-0280 and the ADR index | ✓ headings, ADR-0280 whole; highest ADR number is 0281, new ones start at 0282 |
| `docs/product/journeys/declare-where-a-metal-test-runs.yaml` | ⊘ |
| `vendors/linux` | ⊘ the submodule is not checked out in this worktree, so no kernel claim below carries a `path:line @ v7.0` citation; each is a hypothesis, listed in validation items V-4 and V-5 |
| Rules: CLAUDE.md, `design.md`, `testing.md`, `rust.md`, `development.md`, `domain-naming.md` | ✓ |

## Wave: DESIGN / [REF] Findings that reshape the DISCUSS premise

| ID | Finding | Evidence | Consequence |
|---|---|---|---|
| F-1 | **Placement has a fourth source: per-test guest requirements keyed by test name inside the dispatcher.** `nextest_guest_plan` and `cgroup_attempt_files`/`mtls_attempt_files`/`nested_resources` choose staged files, loaded modules, setup and cleanup scripts, memory and vCPUs from about a dozen hand-kept `Cell` tables and one literal test name (`DECLARED_LISTENER_TCP_CAPTURE_TEST`), matched by exact name or prefix. | `guest_suite.rs:3788-3794` (`selected_cell`), `:3829-3905`, `:4043-4074`, `:3961` | A rename of such a test silently drops its variant (the guest then lacks a tool or setup and the test fails by name, not silently, but a rename "with zero edits" no longer holds for them). DDD-5 moves each variant into a named profile on the attribute. The DISCUSS count of "three sources" is four. |
| F-2 | **Off Metal, guest-placed tests run their body on the host today** (no Metal context: the wrapper `exec "$@"`). The CI `integration` job (`cargo nextest run --workspace --features integration-tests`, profile `ci`, on Lima) and the pre-push `nextest` hook run these binaries. | `test-wrapper.sh:25-31`; `.github/workflows/ci.yml:442-464`; `lefthook.yml:267-268` | Declaring placement on the test makes those tests fail closed in those lanes (DDD-11). User decision. |
| F-3 | **The table covers every test the Metal run executes, including default-lane unit and acceptance binaries** (`overdrive-core::acceptance`, proptest binaries, `compile_fail`, `xtask`). US-07's "unit-lane tests are not flagged" has no meaning once the table is gone unless something supplies the unit lane's placement. | binary-id keys in `test-placement.json` | DDD-4: user decision between declaring every test and a per-binary host-pure default. |
| F-4 | **An attribute proc-macro cannot be applied to a file module (`mod x;`) on stable Rust, nor to an inline module that contains one** ("non-inline modules in proc macro input are unstable"). The integration entrypoints use exactly `mod integration { mod a; mod b; }`. Recalled, not run here. | Rust `proc_macro_hygiene` tracking; validation V-1 | A module-level attribute is unusable for the repository's layout (V-1 confirms). The user approved (2026-11-13) a function-like declaration inside the module that the tests read by name resolution (DDD-1, DDD-7). |
| F-5 | **Lane placements are not "re-run this test in a guest".** The placed test (`guest_flow::real_kernel::…` in `overdrive-init`) is the in-guest body, run from a prepared in-guest binary (`target/guest-kit/in-guest/overdrive-init--overdrive-init`); an owner lane also runs a host-side script chosen by fixture name (six scripts in the dispatcher); the fixture (`integration::in_guest_lane::…`) is the host driver test. | `guest_suite.rs:4614-4747`, `INIT_GUEST_LANE_PLACEMENTS` | The lane scripts and the closed slot table move into `guest_lane` with the dispatcher's removal (DDD-13). |
| F-6 | **The dispatcher carries body-count assertions** (`CGROUP_DEFAULT_COMPLETE_BODIES`, `MTLS_RELAY_COMPLETE_BODIES`, …) that guard "every authored body was credited". They disappear with the batched path. | `guest_suite.rs:1462-1478` | The eager listing check (DDD-3) and the migration parity check (DDD-15) replace them. |
| F-7 | **`xtask` may not depend on an `overdrive-*` crate**, and the macro crate is one; `core` crates would also take a dev-dependency on an `adapter-host` proc-macro crate to declare host-pure tests. | `development.md` § "xtask is build / test / dev orchestration"; `Cargo.toml` of the macro crate | DDD-12: user decision on the rule's scope. |
| F-8 | **`host-kernel-shared` already serialises whole guest-placed binaries** (`overdrive-cli`, `control-plane`, `worker`, `dataplane`, `init`, `fwd`, `netlink` integration binaries run with `max-threads = 1`). | `.config/nextest.toml` | A shared guest serving one test at a time (DDD-9) costs nothing in those binaries; queuing only arises in binaries outside a single-writer group. |

## Wave: DESIGN / [REF] Reuse Analysis

Contract shape per `.claude/rules/design.md` principle: **pure** = value in, value out; **bounded** = declared effect set; **plan-only** = returns a plan, never mutates.

| Component | Existing | Verdict | Justification, contract shape, universe, assertion mechanism |
|---|---|---|---|
| `#[overdrive_test]` expansion | `overdrive-testing-macros` (skeleton, guest case only) | **EXTEND** | Grammar grows to five explicit forms plus the module form, and a function-like `shared_guest!` is added. Pure: tokens in, tokens out; universe = the annotated fn (the declaration macro: the module's one const item). Checked by trybuild-style fixtures if DISTILL chooses them (their call). |
| Group registry, profiles, per-group artifacts | `guest_suite.rs` plans, wrapper lists, dispatcher `main` | **EXTEND** into new module `placement::groups` | Replaces three group lists and about a dozen name-keyed tables with one closed registry. Pure data plus one `const fn` validator. |
| Per-test guest path (`run_in_guest`, attempt, `GuestRun`, plans) | `guest_suite.rs` lines 3423-4403 and helpers | **EXTEND** by moving to `placement::guest_run` | Bounded: boots one guest, copies in, execs, tears down; universe = its work dir and one VMM. Same implementation the dispatcher wrapper path uses today. |
| Guest kit (`GuestKit`, `GuestPeer`, `WorkDir`) | `guest_kit.rs`, `guest_peer.rs` | **EXTEND** | One new boot parameter (DDD-10); `GuestPeer::exec(&mut self)` already enforces one exec at a time. |
| Lane entry points and owner scripts | `guest_lane.rs`, dispatcher `guest_lane_dispatch` | **EXTEND** `guest_lane` | The six owner scripts and the closed slot table move in (DDD-13). |
| Wrapper | `infra/metal/test-wrapper.sh` | **EXTEND** then shrink | Reads records; loses the table, group lists, `--guest-suite` mode and dispatcher branches. Pure resolver (record bytes and test name to a placement or a typed refusal) plus the existing namespace effects, unchanged. |
| Shared-guest owner | none (ADR-0280 rejected a broker as out of its scope) | **CREATE NEW**: `OwnerCore` (pure) + owner binary (shell) | No existing alternative: nextest runs a process per test, so a guest shared by several tests needs a process that outlives any one of them. Probe evidence. `OwnerCore` is plan-only (events in, effects as data); the shell is bounded (one VMM, one socket, one lock, inside the run's runtime dir). |
| Eager listing check | the wrapper's per-test refusal | **CREATE NEW** xtask subcommand `placement-lint` (user approval needed, DDD-3) | The wrapper refuses only at run time on Metal, per test. Read-only: plan-only, never writes; universe = nextest's list output and the binaries' ELF sections. |
| Binary default declaration | none | **CREATE NEW only if DDD-4 option B is chosen** | Otherwise 3,855 per-test sites. |
| Table, lookup, dispatcher binary, `--guest-suite` mode, `OVERDRIVE_METAL_TEST_DISPATCHER`, body-count constants | present | **DELETE** (US-08) | Replaced as above. |
| `overdrive-guest-peer`, `guest-kit-prepare`, `bootstrap.sh` grammar | present | **EXTEND** | Prepare builds the owner binary where it built the dispatcher; the ordinary grammar swaps the dispatcher bin for `placement-lint`. |

## Wave: DESIGN / [REF] Components and ports

Style: modular, functional core and imperative shell, ports and adapters where a port has two implementers. Paradigm OOP per CLAUDE.md. Technology: Rust, existing crates, `syn`/`quote`/`proc-macro2` (already workspace deps, MIT/Apache-2.0), `python3` in the wrapper (already required), `libc` for `pidfd_open`/`unshare`/`flock` (already a workspace dependency of the testing crates). No new third-party crate.

| Component | Responsibility | Crate or path | Driving port | Driven ports |
|---|---|---|---|---|
| Placement attribute | declare placement, emit record, gate guest forms | `overdrive-testing-macros` | the attribute grammar (DDD-1) | none |
| Placement registry | closed group and profile set, recipes | `overdrive-testing::placement::groups` | `GuestPlacement::declare` (const) | none |
| Guest run | one isolated guest per test | `overdrive-testing::placement::guest_run` | `run_in_guest`, `inside_guest` | `GuestKit` |
| Shared-guest client | elect or attach, send the attach request | `overdrive-testing::placement::shared` | `run_in_guest` with `Sharing::Module` | `OwnerElection`, socket |
| Owner core | queue, one in flight, discard after failure, generations | `overdrive-testing::placement::owner_core` | `OwnerCore::handle(Event)` | none (effects are data) |
| Owner shell | executes effects against the guest, serves the socket | `overdrive-shared-guest-owner` bin in `overdrive-testing` | process entry | `GuestKit`, anchor `pidfd`, socket |
| Lane run | in-guest body plus owner scripts | `overdrive-testing::guest_lane` | `run_lane` | `GuestKit` |
| Wrapper | resolve placement, apply isolation | `infra/metal/test-wrapper.sh` | nextest per-test wrapper call | namespaces, exec |
| Placement check | list, resolve, refuse unplaced, emit resolved table | `xtask placement-lint` | CLI | `cargo nextest list`, ELF sections |

Principle 13 (Earned Trust). Each dependency and the lie it can tell: (a) the **linker** can drop the record section: the test then has no record and is refused (`test_unclassified`), and `placement-lint` names it before any guest boots; the macro keeps the record referenced, and an xtask acceptance fixture reads a real linked test binary with both readers. (b) The **host kernel's PID namespace teardown** is what makes the VMM die with the owner: V-4 proves it on the Metal kernel, and the owner's startup fails closed if `unshare(CLONE_NEWPID)` is refused. (c) The **run runtime directory** can be absent or not owned: the client refuses to elect without a valid `OVERDRIVE_METAL_RUN_ID` (the existing `runtime_staging_root` rule). (d) The **guest** can lie about readiness: `GuestKit::provision` and the existing boot-bound checks are the probe; a guest that cannot run its profile's setup fails the attach with the stage named.

## Wave: DESIGN / [REF] Decisions

All rows PROPOSED. "User" means the decision needs an explicit answer; "relay" means the recommendation is within the user-approved DISCUSS contract, but design.md still requires approval before DISTILL.

| ID | Decision (one sentence) | Status | Needs |
|---|---|---|---|
| DDD-1 | Placement is declared by `#[overdrive_test]` with five explicit per-test forms plus a no-argument form that reads the module's `shared_guest!(guest = "<g>"[, profile = "<p>"])` declaration (grammar in § Interface contracts). **Shape APPROVED by the user 2026-11-13.** The conflict rule between a module declaration and an explicit per-test placement is PROPOSED (explicit overrides). | Shape: APPROVED 2026-11-13; conflict rule: PROPOSED | conflict rule: relay |
| DDD-2 | The placement record is a versioned, NUL-terminated field sequence in the `overdrive_test_placement` ELF section; the wrapper is its only run-time reader; a test with a record and a table entry is refused as ambiguous. | PROPOSED (ADR-0282) | relay |
| DDD-3 | The unplaced-test guard stays in the wrapper and is also run eagerly over every listed test by a new `xtask placement-lint`, landing right after the host-pure form and counting a test as placed when it has a record, an accepted default or a table entry. | PROPOSED | **user** (new subcommand and its wiring) |
| DDD-4 | How the 3,855 host-pure and unit-lane tests are placed once the table is gone: A every test declares; B1 a per-binary host-pure default that is refused in any binary that also declares a non-host-pure test; B2 the same default, mixing allowed. Recommendation: B1. | PROPOSED | **user** |
| DDD-5 | The guest group set lives once in the guest kit as a closed registry with named profiles; the attribute is checked at compile time by const evaluation against it; the wrapper holds no group list; test-name-keyed tables become profiles. | PROPOSED | relay |
| DDD-6 | The guest kit is split into `placement::{groups, guest_run, shared, owner_core}` plus the existing `guest_kit`/`guest_lane`; the public surface is the contract in § Interface contracts. | PROPOSED | relay |
| DDD-7 | Sharing is opted into once per module by `shared_guest!`; the sharing scope is the declaring module, its group and profile within one test binary. The declaration is not inherited by child modules. **Shape APPROVED by the user 2026-11-13.** | APPROVED 2026-11-13 (shape) | none |
| DDD-8 | One owner process per sharing scope per run, elected by an exclusive lock, run as init of its own PID namespace, anchored to the nextest process by a pidfd, reached through a socket under the run's runtime directory. | PROPOSED (ADR-0283) | relay; V-4, V-5, V-10 first |
| DDD-9 | The owner serves one test at a time in arrival order, never cancels a running test, and discards a guest after a failed test or a loss. | PROPOSED (ADR-0284) | relay |
| DDD-10 | `inside_guest()` is true exactly when the process's kernel command line carries `overdrive.test_guest=1`. | PROPOSED (ADR-0285) | relay; V-6 first |
| DDD-11 | A guest-placed test fails with a typed error when there is no Metal run context. | PROPOSED (ADR-0286) | **user** (CI integration job and pre-push hook affected) |
| DDD-12 | Placement declarations may take a dev-dependency on `overdrive-testing-macros` from `xtask` and from `core`-class crates; the xtask lint reads normal and build edges only. | PROPOSED | **user** (amends a `development.md` rule) |
| DDD-13 | Lane kinds self-dispatch through `guest_lane` the way guest placements do; the owner scripts and slot table move out of the dispatcher. | PROPOSED | relay |
| DDD-14 | Retirement removes the table, wrapper lookup, `--guest-suite` mode, dispatcher binary, `OVERDRIVE_METAL_TEST_DISPATCHER`, the explicit batched command and the body-count constants, and rewrites the docs that cite them. | PROPOSED | relay |
| DDD-15 | Migration is a scripted rewrite per batch with a parity check against the table snapshot; slice order puts the listing check before the migrations. | PROPOSED | relay |

### DDD-1 — Attribute surface (question 1)

**Approved shape (user, 2026-11-13).** `#[overdrive_test]` carries per-test placement; sharing and the guest group and profile of a module are declared once, by a plain function-like macro call inside the module, which the per-test attribute reads:

```rust
// top of the module file, once
overdrive_testing::shared_guest!(guest = "cgroup-default");

#[overdrive_test]            // no arguments: the module's declaration
async fn a_test() { … }
```

`shared_guest!` expands to a module-private item (a `const` named `__OVERDRIVE_MODULE_PLACEMENT`, built in constant context). A no-argument `#[overdrive_test]` expands to code naming that item by plain path, so ordinary name resolution finds the declaration of the module the test sits in; with no declaration in scope the test does not compile. The ELF record is built by constant evaluation from the same const, so the wrapper sees the placement the macro declared.

Options considered. **(A) one attribute with per-test forms, a per-test `shared` flag** (the earlier recommendation): rejected by the user; sharing is a property of the module, and a flag repeated on every test lets one test forget it or name a different profile, so the module quietly splits into several guests. **(C) an attribute on the `mod`**: rejected; on stable Rust a proc-macro attribute on `mod x;` (and on an inline `mod` containing `mod y;`, which is the repository's `mod integration { mod a; mod b; }` entrypoint layout) fails with E0658 (`proc_macro_hygiene`, nightly), and an inner custom attribute needs `custom_inner_attributes`; the workspace stays on stable (F-4, V-1). **(B) a separate attribute per kind** (`#[host_pure]`, `#[guest(...)]`): five macros and five error surfaces for one concept; rejected. Mapping of the five table kinds, per-test explicit forms: `host-pure` → `host_pure`; `native-launcher` → `native_launcher`; `guest-group` → `guest = "<group>"[, profile = "<profile>"]`; `guest-lane-operator` → `lane_operator = "<fixture>"`; `guest-lane-owner` → `lane_owner = "<fixture>"`. Module-level sharing (D8) → `shared_guest!` plus the no-argument form on each test of the module. Explicit `guest = …` is always an isolated guest (no `shared` argument exists).

**Conflict rule (PROPOSED, relay).** What an explicit per-test placement means inside a module that has `shared_guest!`: **(O, recommended) the explicit form overrides**: the test keeps exactly what it names (isolated guest, `host_pure`, a lane), and only tests with the no-argument form are shared. **(E) conflict is a compile error.** (E) cannot be built from name resolution: the explicit form has no way to observe that a sibling declaration exists, and having each explicit test define an item of the same name collides with other explicit tests in the module. It would need a module-level record plus a wrapper and `placement-lint` check that reports `placement_conflict`, a new diagnostic and reader logic. (O) costs nothing, keeps every placement readable on the test that has it, and lets a `host_pure` or lane test live in a module of shared-guest tests. Its risk is an author who writes `guest = "<same group>"` in a shared module expecting sharing and gets an isolated guest: always safe (more isolation, never less than declared) and visible in the `OVERDRIVE_GUEST_KIT_TIMING` boot count.

**Module-declaration option for DDD-4 (PROPOSED for the user; not decided).** The same mechanism could carry the host-pure default: a module-level `overdrive_testing::placement!(host_pure)` instead of the per-binary `overdrive_test_default!(host_pure)` of B1. Two variants. **(M1) read-only**, like `shared_guest!`: tests still carry a bare `#[overdrive_test]`; it shortens the 3,855 sites to the no-argument form but does not remove them, so it is A with a shorter spelling, and plain `#[test]` functions stay unplaced. **(M2) the declaration also emits a module-scoped record** (kind `module_default`, the module path) that the wrapper applies to every test under that module path without a per-test attribute: one site per module containing tests instead of one per binary (B1, about 80) or per test (A); finer than B1, so a mixed binary is allowed without B2's silent failure (a module with no declaration and no per-test record is still `test_unclassified`); cost: more declaration sites than B1 (count not measured), a new record kind and a prefix-and-nesting rule in the wrapper reader (which module's record wins for a test in a child module; ambiguity with a per-test record), and the dev-dependency on the macro crate for every `#[cfg(test)] mod` in `core` crates (DDD-12) at more places than B1. Versus B1: M2 trades B1's all-or-nothing binary rule (`placement_conflict`) for a per-module rule and more churn; versus A: far fewer sites. Not recommended over B1 without the user asking for per-module granularity.

### DDD-2 and DDD-3 — Marker, ambiguity, guard (question 2, O1, O3)

O1 resolved: **keep the ELF-section record** (ratified, with a version field and an explicit ambiguity rule). Alternatives in ADR-0282. Ambiguity: a test with both a record and a table entry is `placement_ambiguous` (status 78); the skeleton's "record wins" is removed. The migration commit adds the attribute and deletes the entry together, so a state with both never lands.

Replacement for `test_unclassified` once the table is gone: the wrapper's refusal stays the exact, last line of defence at run time; it is the same function over the record (and the accepted default of DDD-4). The early check is `placement-lint`. Options: **(L-A, recommended) xtask subcommand over `cargo nextest list --message-format json` plus the ELF records**: exact (no guess about macro-generated tests) and runs in the lane that already compiles the test binaries; **(L-B) an xtask acceptance test that shells nextest**: nested cargo inside a test, which the guest-kit rules forbid for lock contention; **(L-C) a syntactic scan of test sources**: misses tests generated by `proptest!`, `quint_test` and similar, and cannot see the linker dropping a record. The reader of the record exists once: the wrapper's Python reader, which `placement-lint` invokes through `test-wrapper.sh --check-placement` with the list on stdin, so there is one parser for the format. The subcommand is new tooling: **it needs the user's approval**, the grammar addition in `bootstrap.sh`, and wiring beside `dst-lint` and `conformance-lint` in CI and pre-push (the repository's precedent for lints).

O3 resolved: the check needs no per-binary scope. Its rule, "a listed test is placed when it has a record, an accepted default, or (until retirement) a table entry", is true at every migration stage, so it can land before any migration and the table clause is deleted in the retirement slice. Suggested order: 01 (done) → 02 → 03 → **07** → 04 → 09 → 05 and 06 → 08.

### DDD-4 — Host-pure and unit-lane scale (F-3)

Options: **A. every test declares** (about 4,640 sites: 3,855 host-pure including every unit test in every crate, plus the guest and lane placements). Safest and most uniform; most churn; every new unit test needs the attribute; US-07's "unit-lane tests are not flagged" scenario is dropped. **B1. a per-binary default**, written once in a test binary's root (`overdrive_test_default!(host_pure)`, only `host_pure` is allowed as a default), applying to every test without its own record; a binary that has the default **and** any non-host-pure record is refused by the wrapper and the check as `placement_conflict`. About 80 one-line declarations replace 3,855 per-test ones; adding the first guest test to a pure binary forces the author to drop the default and declare every test, which is the intended friction. **B2.** as B1 but mixing allowed: a forgotten declaration in a mixed binary silently runs on the host, which is the failure the guard exists to prevent. **Recommendation: B1.** It prevents no failure that A does not, but A's cost is a rewrite of every test and every future unit test for no reachable gain; B1 keeps the exact refusal for the only binaries where a misplaced test is plausible (mixed ones). This is a scope-shaping choice, so it is the user's.

### DDD-5 — Group registry, profiles, per-group artifacts (questions 1 and 4, F-1)

SSOT: `overdrive_testing::placement::groups::GuestGroup` (closed enum of the eight groups) and, per group, a table of **profiles**. Compile-time validation without a second list: the macro (an explicit guest form, or `shared_guest!`) emits a `const` that calls `GuestPlacement::declare(group, profile, sharing)`, a `const fn` that panics at constant evaluation with a message naming the unknown value and the known set, so a typo is a build error and the macro crate holds no group list (a proc-macro crate cannot export types, and `overdrive-testing` depends on it, so the registry cannot live there). The wrapper does not validate groups. Per-group artifacts (US-04): each (group, profile) has one `GuestRecipe` (tools, modules, boot-time setup, per-test setup and cleanup, memory, vCPUs, CID slot); that table is the single place a copy list is declared. F-1's name-keyed variants become profiles selected on the attribute (`profile = "<name>"`; omitted means the group's base profile). Alternatives: **(a) keep test-name-keyed tables as library data**: a renamed test silently loses its variant; **(b) make each variant a new group**: changes the group vocabulary the out-of-scope list protects and multiplies CID slots. The profile names are derived from the inventory in V-3, not invented here. A shared guest's key includes the profile, because profiles differ in loaded modules and setup.

### DDD-6 — Guest-kit library structure (question 5, O2)

O2 resolved: keep `overdrive-testing` as the home (same crate class, already holds `guest_kit`, `guest_lane`, the peer binary); split `guest_suite.rs`, delete its batched half, and re-home the per-test path under `placement`. A new crate for the owner would force a third crate to share `GuestKit`; rejected. The macro re-export stays `overdrive_testing::overdrive_test`. Public surface is pinned below.

### DDD-7 — Sharing opt-in (D8)

Sharing is declared once per module by `shared_guest!` (user-approved shape, 2026-11-13; DDD-1 holds the options and the rejections: a per-test `shared` flag, and an attribute on `mod`, which stable Rust refuses). Only a test that carries `#[overdrive_test]` with no arguments inherits the declaration; a plain `#[test]` does not, so the explicitness of "this test is placed" stays on the test. The declaration is a module-private item found by plain path, so a child module does not inherit its parent's; a module whose tests are in `mod integration { mod a; mod b; }` declares in `a` and in `b`. Scope key = `(binary id, module path of the test, group, profile)`; the module path is the test's name without its last segment, which equals the module of the declaration. Nothing is shared across modules or binaries.

### DDD-8 — The owner (question 3)

Who owns the guest: the **owner process**, one per scope per run, started by the first arrival. Election: an exclusive `flock` on `<runtime>/shared/<scope>.lock`, released by the kernel when the holder dies; the elector starts the owner, waits until the socket is connectable (bounded by the existing `SCRIPTED_BOOT_BOUND`), then releases the lock. A later arrival finds the socket and attaches; if the connect is refused it takes the lock and elects. Lifetime: the owner calls `unshare(CLONE_NEWPID)` and forks; the child is the owner proper and so is init of its namespace, and the kernel kills every process of a PID namespace when its init dies. That makes "the VMM never outlives its owner" structural and keeps the production launcher unchanged. Anchor: the wrapper exports `OVERDRIVE_TEST_RUN_ANCHOR_PID` (its own parent, which nextest spawns directly: V-10); the owner opens `pidfd_open` on it at start and exits when the fd becomes readable; if `pidfd_open` fails (the anchor is already gone) the owner refuses to start. Teardown: run teardown (ADR-0279) remains the backstop. Rendezvous: `/run/ovd-gk/<run id>/shared/`, the run-owned VMM runtime root the guest kit already uses (`runtime_staging_root`), mode 0700; the socket path is about 70 bytes, under the 108-byte `sun_path` limit that broke the run `TMPDIR` path in the probe. Owner crash: attached tests get a typed failure naming the owner stage; the lock is free and the next arrival elects a new owner; the old VMM died with it (V-4). Interrupt: nextest dies, the anchor pidfd fires, the owner exits. Nothing is left in `/tmp`. Exposure if V-4 fails: an orphan VMM until run teardown, and a replacement boot would collide on the CID; the decision then returns to DESIGN, no heuristic recovery is designed. A pid-reuse race between the wrapper reading its parent and the owner opening the pidfd is bounded by the run teardown.

### DDD-9 — One at a time, no cancel, discard after failure (question 3)

Details in ADR-0284 and the model. Options considered for retries: reuse the guest (cheap, residue) versus a fresh guest after a failure (about 3 s). Recommended: fresh after failure, because a retry exists precisely because a test failed, and the failed test is the one most likely to have left state. A guest lost while idle is replaced at the next attach. A boot failure answers only the test at the head of the queue and the next attach boots again (no automatic retry). Guest kernel crash mid-test: the test gets a typed failure, the guest is discarded. Isolation statement for the docs: tests that share a guest share its kernel, cgroup tree, nft tables and `/tmp`; the owner runs each profile's per-test setup and cleanup before and after every test and resets nothing else.

### DDD-10 — `inside_guest()` (question 6)

Options: keep the environment variable (spoofable, DISCUSS finding); strip it in the wrapper (Metal only); **kernel command line parameter `overdrive.test_guest=1` (recommended)**: the guest kit's boot already accepts command-line parameters (`ScriptedBootExtra.cmdline`), and a host's command line cannot be changed by a process. Cost: one `/proc/cmdline` read per call. Falsified if the parameter does not reach the guest's `/proc/cmdline` (V-6).

### DDD-11 — Off Metal (question 7, F-2)

Today's behaviour off Metal is "run the body on the host". Options: **(A, recommended) fail closed** with `RunInGuestError::NoMetalContext`: the DISCUSS scenario ("never reports a body-less PASS") and `GuestKit::provision`'s own refusal point there, and the user's standing rule is that guest-kit surfaces are non-signal off Metal. Effect: the CI `integration` job and the pre-push `nextest` hook, which run `--features integration-tests` over these binaries, will fail the guest-placed tests that compile without `kvm-tests`; those lanes must either run on Metal (matches the standing directive to run everything on Metal) or exclude those binaries. **(B)** run in place off Metal: restores today's lanes but reports PASS for placed tests that never ran in a guest, so it cannot satisfy the approved scenario. **(C)** skip with a visible reason: nextest has no runtime skip, so a skip is a PASS. The lane inventory is validation V-7; relay it to the user with the decision.

### DDD-13 to DDD-15

DDD-13: `lane_operator` and `lane_owner` tests are the in-guest body; their host half calls `guest_lane::run_lane`, which boots the lane guest from the prepared in-guest binary and, for the owner kind, runs the host script chosen by the fixture; an unknown fixture fails naming the fixture and the test. The six scripts and the placement slot table move into `guest_lane` and are matched by exact fixture name. The slot per placement is internal; a guest in its own local-mode vsock namespace does not collide with another run's. DDD-14: the retirement list is the Reuse Analysis DELETE row plus `OVERDRIVE_FWD_GUEST_GROUP`, `OVERDRIVE_FWD_GUEST_EXACT`, the `bootstrap.sh` grammar entry for the dispatcher, `guest_kit_prepare`'s dispatcher build, and the nine dispatcher unit tests (deleted, not rewritten, per the deletion rule); documents to update in the same slice: `.claude/rules/testing.md` § "Test placement" (through the rules process), `infra/metal/README.md`, `website/content/docs/how-to/run-verification-on-metal.mdx`, `tests/conformance/CLAUDE.md` and `src/lib.rs` comments, the `metal-run-isolation` and `declare-where-a-metal-test-runs` journeys, and the xtask placement acceptance tests. DDD-15: per batch, a script under `docs/feature/metal-test-placement-attribute/deliver/` (temporary, deleted at retirement) reads the table by parsing the JSON, finds each test's function by module path and name, adds the attribute, deletes the entry, and **stops on any ambiguity** (two functions, a macro-generated test, a missing function); parity = `placement-lint --emit` over the migrated binaries, diffed against the table snapshot at the batch's start commit, equal on (binary, test, kind, group, fixture, profile); pass-set parity is the Metal run. Where a test cannot take an attribute (V-2), the batch reports it and the user rules.

## Wave: DESIGN / [REF] Interface contracts

These are the pinned cross-crate, cross-process contracts. Private structure is the crafter's.

### Attribute grammar (`overdrive_testing::overdrive_test`, re-exported from `overdrive-testing-macros`)

```text
shared_guest!(guest = "<group>" [, profile = "<profile>"] [,])      // function-like, once per module

#[overdrive_test]                       // module form; `#[overdrive_test()]` is identical
#[overdrive_test(host_pure)]
#[overdrive_test(native_launcher)]
#[overdrive_test(guest = "<group>" [, profile = "<profile>"])]      // isolated guest, one per test
#[overdrive_test(lane_operator = "<fixture>")]
#[overdrive_test(lane_owner = "<fixture>")]
```

User-approved shape (2026-11-13); the clauses marked PROPOSED are not approved.

`overdrive_testing::shared_guest!`:

- A function-like macro exported by `overdrive-testing`, used as a module-level item. It expands to one module-private, non-`pub` item `__OVERDRIVE_MODULE_PLACEMENT` of type `GuestPlacement` with `Sharing::Module`, built by `GuestPlacement::declare` in constant context, with `#[allow(dead_code)]`. Whether the macro is declarative or procedural is the crafter's, provided the item is visible by plain path to the module's own tests and to nothing else.
- `guest` is required; `profile` is optional (omitted means the group's base profile); a trailing comma is accepted.
- Build errors: an unknown group or profile (message names the value and the known set), an unknown or repeated argument, a missing `guest`, a second `shared_guest!` in one module (duplicate item), and any `shared` argument (not a form).
- Scope: the declaring module only; child modules do not inherit it and declare their own.

`#[overdrive_test]` (no arguments):

- Expands to code that names `__OVERDRIVE_MODULE_PLACEMENT` by plain path. Ordinary name resolution binds the module's declaration; with none in scope the test fails to compile (the error names the missing `shared_guest!`). A declaration the test cannot see (a different module) is not found.
- The placement is the declaration's group and profile with sharing `Module`. The ELF record is emitted from the same const by constant evaluation (kind `guest`, group, profile, sharing `1`), so the wrapper reads exactly what the declaration holds.
- Any other form is a per-test placement and never reads the module declaration. An explicit `guest = …` is an isolated guest (record sharing `0`).

Shared clauses:

- Applies to a `fn` returning `()` (sync or `async`). If the fn carries no test-introducing attribute the macro adds `#[test]`; stacked above `#[tokio::test]` and the other test attributes it leaves them intact (V-2 fixes the exact set; V-1(c) fixes `#[tokio::test]` and async bodies).
- `host_pure` and `native_launcher` emit the record and leave the body untouched, and reference no runtime crate.
- The module form, `guest` and the lane forms replace the body with: run the original body where `inside_guest()` holds; elsewhere hand the test to `run_in_guest` / `run_lane` on a plain OS thread and panic with the typed error. The body never runs on the host.
- Build errors of the attribute: an unknown `group` or `profile`, an unknown argument (including `shared`, with the message "sharing is declared by `shared_guest!`"), a repeated argument, a lane fixture not of the form `integration::in_guest_lane::<ident>`, and the no-argument form without a declaration. A lane fixture's existence is checked at run time (the failure names fixture and test).
- **Conflict rule (PROPOSED):** an explicit per-test form inside a module that has `shared_guest!` overrides it for that test; it is not an error. Alternative (compile error) and why it is not recommended are in DDD-1.

### Placement record (ELF section `overdrive_test_placement`, version 1)

A sequence of records; each record is a sequence of NUL-terminated fields: `"1"`, `<kind>`, `<test>`, `<argc>`, then `argc` arguments. `<kind>` is one of `host_pure`, `native_launcher`, `guest`, `lane_operator`, `lane_owner`; `<test>` is `module_path!()` of the fn plus `::<fn name>` (crate segment included; the reader drops it to compare with nextest's test name). Arguments: `guest`: `<group>`, `<profile or empty>`, `<"1" when the placement came from the module's `shared_guest!`, "0" for an explicit `guest` form>`; lane kinds: `<fixture>`; the others: none. Under DDD-4 B a default record has `<kind>` `default_host_pure` and an empty `<test>`. Readers reject an unknown version, an unknown kind, a wrong `argc`, or a non-UTF-8 field as `placement_invalid`; two records for one test as `placement_ambiguous`.

### Wrapper diagnostics (stderr line `OVERDRIVE_METAL_TEST_ERROR error=<name> [detail]`)

| Name | Status | Cause |
|---|---|---|
| `test_unclassified` | 78 | no record, no accepted default (and, until retirement, no table entry) |
| `placement_ambiguous` | 78 | two records, or a record and a table entry, for one test |
| `placement_invalid` | 78 | unreadable section, bad version, unknown kind, wrong `argc`, unreadable table |
| `placement_conflict` | 78 | only under DDD-4 B1: a default and a non-host-pure record in one binary |
| `invalid_run_context`, `namespace_setup_failed` | 78, 70 | unchanged |

`lane_not_prepared` and `guest_execution_failed` leave the wrapper with the dispatcher; the same conditions are typed errors of the test's own host half. The wrapper maps kinds to isolation as today: `host_pure` → original chain; `native_launcher` → node view; `guest`, `lane_operator`, `lane_owner` → local-mode vsock network namespace, with `OVERDRIVE_TEST_RUN_ANCHOR_PID` exported. `test-wrapper.sh --check-placement` reads nextest list JSON on stdin and prints one `unplaced <binary-id> <test>` line per unplaced test, exiting non-zero if any.

### `placement-lint` (xtask)

`cargo xtask placement-lint [--emit]`: lists tests with the lane's features, prints `placement-lint: unplaced <binary-id> <test>` per unplaced test and exits 1; with `--emit` prints `{"format":"overdrive-metal-placement-resolved-v1","placements":{"<binary id>":{"<test>":{"kind":"…","args":[…]}}}}` for migration parity. Writes nothing.

### Guest kit public surface (`overdrive_testing::placement`)

```rust
pub enum GuestGroup { Probe, Core, Unload, Ulp, MtlsRelay, CgroupDefault, Stress, NestedBoundary }
impl GuestGroup { pub const ALL: [GuestGroup; 8]; pub const fn as_str(self) -> &'static str; }

pub enum Sharing { Isolated, Module }

/// Built in constant context by the macro; panics at constant evaluation
/// naming the unknown group or profile and the known set.
pub struct GuestPlacement { /* group, profile, sharing */ }
impl GuestPlacement { pub const fn declare(group: &str, profile: &str, sharing: Sharing) -> GuestPlacement; }

pub struct LanePlacement { /* kind, fixture */ }
impl LanePlacement {
    pub const fn operator(fixture: &str) -> LanePlacement;
    pub const fn owner(fixture: &str) -> LanePlacement;
}

pub fn inside_guest() -> bool;
pub fn run_in_guest(placement: &GuestPlacement, exe: &Path, test_name: &str) -> Result<(), RunInGuestError>;
pub fn run_lane(placement: &LanePlacement, test_name: &str) -> Result<(), RunInGuestError>;

pub enum RunInGuestError {
    NoMetalContext   { group: String, test: String },
    LaneNotPrepared  { group: String, test: String, detail: String },
    GuestExecution   { group: String, test: String, detail: String },
    BodyFailed       { group: String, test: String, status: i32 },
}
```

`inside_guest`, `run_in_guest` and `RunInGuestError` keep the names the skeleton approved; `run_in_guest`'s first parameter changes from `&str` to the typed placement, and `NoMetalContext` is new (DDD-11). Per-test mode prints one `OVERDRIVE_GUEST_KIT_TIMING group=<g> test=<t> staging_ms=<n> boot_ms=<n> total_ms=<n>` line per guest booted (unchanged); a shared attach prints `OVERDRIVE_GUEST_KIT_TIMING group=<g> test=<t> shared=1 generation=<n> queue_ms=<n> exec_ms=<n>`, and the owner prints the boot's staging and boot times with the same `generation`. One boot per module is counted as the distinct `generation` values per scope. The guest's console output of the test is returned to the host test's output. Preconditions of `run_in_guest`: a Metal run context (`OVERDRIVE_METAL_RUN_ID`), the guest kit prepared; failure maps to the variants above with the stage in `detail`.

### Shared-guest scope, socket and wire (version 1)

Scope key: SHA-256 of `"1" NUL binary-id NUL module NUL group NUL profile`, first 16 hex characters. Files: `/run/ovd-gk/<run id>/shared/<key>.lock` and `.sock`. Owner command: `overdrive-shared-guest-owner --socket <path> --lock-fd <n> --anchor-pid <pid> --exe <path> --binary-id <id> --group <g> --profile <p>`, prepared by `guest-kit-prepare` at a documented contract path and refused as `LaneNotPrepared` naming it when absent. Request, line oriented, fields hex-encoded: `ATTACH 1`, `TEST <hex>`, `CWD <hex>`, `ENV <n>` then `n` lines `<hex key> <hex value>`, `GO`. Reply: `RESULT 1 <verdict> <generation> <queue_ms> <exec_ms> <console_len>` then `console_len` bytes, verdict `exit:<i32>`, `boot_failed` or `guest_lost`. End of stream before a reply is the owner's loss. Client mapping: `exit:0` → `Ok`; other exit → `BodyFailed`; `boot_failed`, `guest_lost` and owner loss → `GuestExecution` with the stage named.

### Owner core (conformance accessors)

```rust
pub struct OwnerCore { /* private */ }
pub struct AttachId(/* opaque */);
pub enum Event { Attached(AttachId), ClientGone(AttachId), BootFinished(Result<(), BootFailure>),
                 ExecFinished(AttachId, ExecVerdict), GuestLost }
pub enum Effect { StartBoot, StartExec(AttachId), Reply(AttachId, Verdict), DiscardGuest }
pub enum GuestView { NoGuest, Booting, Ready }
pub struct OwnerObservation { pub guest: GuestView, pub generation: u32,
                              pub in_flight: Option<AttachId>, pub queue: Vec<AttachId> }
impl OwnerCore { pub fn handle(&mut self, event: Event) -> Vec<Effect>; pub fn observe(&self) -> OwnerObservation; }

pub trait OwnerElection { fn try_elect(&self, scope: &ScopeKey) -> Elected; fn live_owners(&self, scope: &ScopeKey) -> usize; }
```

Model variable to accessor: `guest`, `gen`, `inflight`, `queue` → `OwnerObservation`; `owners` → `OwnerElection::live_owners` (the kernel lock is the authentic arbiter; a `Sim` election may feed the projection only with the Tier-3 scenario "two test processes arrive at once, one owner" paired with it); `lock`, `anchor`, `torn` and the ghost variables are model-only; a client's outcome is read from the `Reply` effect or the client call that returns it. Actions map: `arrive`/`elect_and_start_owner`/`connect` → `OwnerElection::try_elect` plus `Event::Attached`; `begin_boot`/`boot_ok`/`boot_fail` → `Effect::StartBoot`, `Event::BootFinished`; `begin_exec`/`exec_done_*` → `Effect::StartExec`, `Event::ExecFinished`; `guest_lost` → `Event::GuestLost`; `client_disconnect` → `Event::ClientGone`; owner crash, anchor gone and run teardown → the driver drops the core and clears the election.

## Wave: DESIGN / [REF] Lifecycle Gate Ownership

| Signal or state | Owning component | Promise | Inputs that may gate it | Must not gate |
|---|---|---|---|---|
| nextest test result (PASS/FAIL/TIMEOUT) | nextest | the attempt's process exited with that status | the wrapper's refusal, the test's own panic | guest readiness of other tests |
| a test's placement | the test (record) | which isolation its body needs | the record (and DDD-4 default) | the test's result |
| guest generation `Ready` | owner core | a booted guest of the scope's profile with its setup done, idle, never used by a failed test | boot result, previous exec verdict | any test outside the scope, any `Isolated` test |

Gates:

- **G-1 Placement guard.** Existing evidence: `test-wrapper.sh:58-247` (table lookup then `test_unclassified`), skeleton marker read `:71-161`; no state-write path beyond exit status. Owner: the wrapper. Promise: the test has exactly one resolvable placement. Affected state: whether the body is launched. Failure projection: status 78 with `test_unclassified`, `placement_ambiguous`, `placement_invalid` or `placement_conflict`. Explicitly unaffected: the test's kind semantics, run-context checks (still first). Ordering: after run-context validation, before the isolation profile; no timeout. Counterexample: a plain `#[test]` that mutates nft would run on the host if absent placement defaulted to host-pure in a mixed binary. Evidence lane: xtask acceptance with fixture ELFs, plus the Metal ordinary run.
- **G-2 Metal context for a guest placement.** Existing evidence: `run_in_guest` and `GuestKit::provision` refuse when unprepared. Owner: `run_in_guest`. Promise: a guest-placed body runs only inside a guest booted under a Metal run. Affected state: the test's result only. Failure projection: `NoMetalContext` (FAIL). Unaffected: `host_pure` and `native_launcher` tests, other lanes' unrelated tests. Ordering: before any boot. Counterexample: pre-push on Lima, where today the body runs on the host and passes. Evidence lane: unit test with the context unset; the lane inventory V-7.
- **G-3 Shared-guest admission.** Owner: owner core. Promise: a test is served only on a `Ready` generation that never ran a failed test, one at a time, in arrival order. Affected state: that attach's reply. Failure projection: `exit:<n>`, `boot_failed`, `guest_lost`, owner loss (each a typed host failure). Unaffected: nextest's retry policy, `Isolated` tests, other scopes. Ordering: queue wait plus exec consume the one existing nextest slow-timeout budget; the owner adds no deadline of its own (exec uses the existing attempt bound, election uses `SCRIPTED_BOOT_BOUND`). Counterexample: a test that fails and leaves a nft rule must not be followed by a pass or fail that depends on it. Evidence lane: the Quint model, a seeded run of `OwnerCore`, and the Tier-3 module scenarios.
- **G-4 Eager placement check.** Owner: `placement-lint`. Promise: every listed test resolves a placement before any guest boots. Affected state: the command's exit status. Unaffected: Metal runs. Late success/disconnect do not apply. Evidence lane: negative fixture binary (one unplaced test) plus no `test_unclassified` in later Metal runs.

Boundary scenarios for DISTILL: (1) gate available: a declared test passes in a guest; (2) unavailable or timed out: guest cannot boot, owner cannot start, anchor gone → typed failure naming group and stage; (3) unrelated state: a `host_pure` test in the same binary and a test with an explicit per-test placement in the same module (PROPOSED override rule) are unchanged; (4) late success: an exec whose client was killed returns after the next client attached, and its result reaches nobody; (5) disconnect and reconnect: owner killed mid-module, a new owner is elected and boots a new generation; (6) feature disabled: an undeclared binary with a default behaves as before. Seeded invariant: `OwnerCore` under `overdrive-sim`-style seeded schedules of attach, gone and failure events (the model's invariants as `assert_always!`); real-kernel proof: the two-process election, owner death, interrupt and per-test results on Metal.

## Wave: DESIGN / [REF] Formal protocol model

**Applicable.** The shared-guest owner protocol is concurrent (several test processes), ordered (FIFO, one in flight) and crash-sensitive (owner crash, anchor loss). Spec: `specs/quint/shared-guest-owner/` (`shared_guest.qnt`, `hazard/guest_hazards.qnt`, `checks.toml`, `README.md`, `evidence/`), produced by the `quint-modeler` and run on Metal (Quint 0.32.0, Apalache 0.56.1, Java 21; 19 checks).

Verdicts: **no counterexample against any decision.** D-OWNER (`I_OWNER_UNIQUE`), D-LIFE (`I_NO_ORPHAN`, `I_TEARDOWN_CLEAN`), D-SERIAL (`I_SERIAL`, `I_FIFO`), D-DETACH (`I_DETACH_NO_CANCEL`, `I_DELIVER_ONLY_ATTACHED`) and D-TAINT (`I_NO_REUSE`) hold; the progress property `EveryClientTerminates` holds under weak fairness of non-fault actions, an anchor that stays alive, and at most two faults. Five hazard variants (one per decision) each produce the predicted counterexample: no election lock gives two owners, no in-flight guard gives two concurrent execs, reuse after failure runs on a failed generation, an owner crash that leaves the guest gives an orphan, and a disconnect that frees the slot lets the next exec start while the first still runs. `I_FIFO` and `I_DELIVER_ONLY_ATTACHED` are largely structural in the model and have no hazard of their own.

Named runs for `#[quint_test]` (all `main = guest_conf`, module `shared_guest.qnt`): `happy_path_three_clients_share_one_guest`, `concurrent_first_arrival_one_owner`, `failure_then_replacement_guest`, `guest_lost_mid_exec_then_replacement`, `owner_crash_then_new_owner_elected`, `disconnect_while_running_next_waits`, `disconnect_while_queued_is_removed`, `boot_failure_then_retry_boot_on_next_attach`, `anchor_gone_ends_owner_and_guest`, `run_teardown_removes_leftovers`, `retry_of_failed_client_gets_fresh_guest`, `elector_killed_before_owner_starts`. The spec reaches the pinned accessors as mapped in § Interface contracts.

Underspecified points the modeler raised, and how this design pins them: owner reported present only once it serves (A-6); an elector killed holding the lock frees the lock and leaves no owner (the run `elector_killed_before_owner_starts`); no owner exits on its own while the anchor lives; owner death gives attached queued and running tests an owner-loss failure, and a detached test is simply gone; a boot failure fails only the head of the queue and nothing retries it automatically; a guest lost while idle is replaced at the next boot; a retry is a new client; the model's "clients die with the anchor" assumption (A-2) is replaced by a structural rule (an owner whose `pidfd_open` on the anchor fails refuses to start), so no client assumption remains. Assumptions with a discharging item: A-1 (owner is init of its PID namespace, so the VMM dies with it) → **V-4**; pidfd on the wrapper's parent being nextest → **V-10**; the lock being the election arbiter and the socket surviving across network namespaces → **V-5**. The spec files are modeler output; evidence is recorded and `verify-evidence` passed.

## Wave: DESIGN / [REF] Validation items

**Contract-shaping (run before the design is approved; none needs more than one Metal ordinary run or a host compile):**

| ID | Question | Falsification changes | Where |
|---|---|---|---|
| V-1 | On the pinned stable toolchain: (a) is an attribute proc-macro on `mod x;`, and on an inline `mod` containing `mod y;`, rejected (E0658, `proc_macro_hygiene`; an inner custom attribute needing `custom_inner_attributes`)? (b) does `shared_guest!` plus a no-argument `#[overdrive_test]` compile, bind the module's own declaration by plain path (not a parent's or sibling's), fail to compile in a module with no declaration, and yield an ELF record built by constant evaluation (read back from the linked test binary: kind `guest`, group, profile, sharing `1`)? (c) does it stack with `#[tokio::test]` and `async fn` bodies, leaving one `#[test]` and one record per test, in the `mod integration { mod a; mod b; }` layout with the declaration in a file module? | DDD-1/DDD-7 (the approved module-declaration shape; a failure of (b) or (c) returns it to the user) | host compile probe in `spike-scratch/`, no Metal |
| V-2 | Does `#[overdrive_test(...)]` stack correctly over `#[tokio::test]`, a `proptest!` block's tests, `#[quint_test]`, `#[quint_run]` and trybuild-style tests, leaving one record per test and no duplicate `#[test]`? Count the tests in the workspace that cannot take an attribute. | the grammar and the migration stop rule | host compile probe plus reading the records |
| V-3 | Inventory the dispatcher's per-test variants (the `Cell` tables, `NESTED_GUEST_LAUNCH`, `focused_selection`, the mtls native/relay split, the literal test name): how many distinct (group, variant) environments, and do they reduce to a closed named set? | DDD-5 (profiles versus groups; the `profile` argument) | parse `guest_suite.rs`, no Metal |
| V-4 | With the owner as init of a fresh PID namespace, does `SIGKILL` of the owner leave no VMM within a bound (and does the production launcher work inside the namespace)? Read `kernel/pid_namespace.c` (`zap_pid_ns_processes`) and `kernel/exit.c` in `vendors/linux` at `v7.0` first (`git submodule update --init vendors/linux`), cite `path:line`, then run it. | DDD-8 / ADR-0283 | Metal ordinary run |
| V-5 | Does a socket under `/run/ovd-gk/<run id>/shared/` connect from a test process in a different local-mode vsock network namespace, is the directory reclaimed at run end, and do two simultaneous arrivals elect exactly one owner through `flock`? | the rendezvous location and election | Metal ordinary run |
| V-6 | Does `ScriptedBootExtra.cmdline` put `overdrive.test_guest=1` in the guest's `/proc/cmdline`, and is it absent on the host? | DDD-10 | Metal ordinary run |
| V-7 | Which lanes run guest-placed tests off Metal today (CI `integration` job, pre-push hook, `infra/hooks/linux-run.sh`, others) and which of the guest-placed tests compile without `kvm-tests`? | DDD-11 (the user's choice needs the list) | grep and reading |
| V-9 | Do `dst-lint`, `cargo tree -p xtask`, `cargo deny` or crate-class rules reject a dev-dependency from a `core` crate or `xtask` onto `overdrive-testing-macros`? | DDD-12 | repository gates, no Metal |
| V-10 | Is the wrapper's parent the nextest process (not an intermediate shell), and is `pidfd_open` on it permitted for root in the wrapper's namespace? | the anchor contract | Metal ordinary run |

**Not contract-shaping (may stay open; each blocks only its DELIVER step):** console completeness for a failing shared test (the probe used a 300 ms flush sleep, which is an upper bound and not a design); boot-time distribution per profile (the skeleton has two samples); whether a profile's per-test setup scripts are idempotent across consecutive tests in one guest (checked per migrated module at DELIVER).

## Wave: DESIGN / [REF] Scope limitations proposed for tracking

No issue is created. Each is proposed to the user, with the file that pins it. I could not run `gh` in this dispatch, so none of these was searched against open issues or the ADRs for a planned mechanism; the orchestrator must run `gh issue list --state all --search …` and `gh issue view <N> --comments` first (CLAUDE.md § "Deferrals require GitHub issues").

| Proposal | Where pinned | What lifting needs |
|---|---|---|
| Sharing is per module of one test binary; no sharing across modules or binaries | DDD-7 | a wider scope key; not requested, probably no issue wanted |
| A default declaration can only be `host_pure` | DDD-4 B1 | only relevant if the user picks B |
| A lane fixture's existence is checked at run time, not at build time (the fixture lives in another crate's test binary) | DDD-1 | a cross-crate registry; not requested |
| Guest groups are a closed set of eight; adding one is a code change in the registry | DDD-5 | already the case today |

Not a limitation, so no issue proposed: serialising execs within a shared guest (DDD-9). Concurrency inside one guest would reintroduce the node-global races the guest isolates.

## Wave: DESIGN / [REF] Open questions and blockers

Blocking before DISTILL: user answers to DDD-3, DDD-4, DDD-11, DDD-12; V-1 to V-7, V-9 and V-10 run and recorded (V-4 and V-6 first: they decide DDD-8 and DDD-10); `vendors/linux` initialised so V-4's kernel claim can be cited. Not blocking: the exact names of the profiles (V-3's output), the final wording of diagnostics.

Handoff notes. External integrations: none (no third-party API); no consumer-driven contract tests apply. The record format is a three-reader contract (macro writes, wrapper and `placement-lint` read): one fixture test binary read by both readers is the drift guard. Enforcement tooling: `placement-lint` (new, DDD-3), the wrapper refusal, the owner Quint conformance driver under `conformance-lint`, and the existing dst-lint, which does not scan these `adapter-host` crates. Recommended DISTILL scenarios per Lifecycle Gate Ownership above and per `.claude/rules/testing.md` § "DISTILL": each gate with a healthy control and one deliberate fault. No roadmap is created in DESIGN.
