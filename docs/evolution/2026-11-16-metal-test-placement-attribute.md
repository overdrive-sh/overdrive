# Evolution — Declaring where a Metal test runs, on the test

**Feature:** `metal-test-placement-attribute`. **Finalized:** 2026-11-16.
**Scope:** developer verification tooling; one job (`J-PLAT-007`), nine slices, fifteen implementation steps.
The [feature contract and wave history](../feature/metal-test-placement-attribute/feature-delta.md),
[roadmap](../feature/metal-test-placement-attribute/deliver/roadmap.json) and
[execution log](../feature/metal-test-placement-attribute/deliver/execution-log.json)
remain the canonical delivery records. The feature closes
[GH #354](https://github.com/overdrive-sh/overdrive/issues/354) on merge; no issue was closed during finalization.

## Summary

Where an integration test runs on the shared Metal box (host-pure, native launcher,
a guest group and profile, or a lane fixture) is now declared on the test:
`#[overdrive_test(...)]`, a per-module `placement!(host_pure)`, or a per-binary
`overdrive_test_default!(host_pure)`. A module whose tests share one guest declares
`shared_guest!(guest = "<group>"[, profile = "<profile>"])` and its tests carry a bare
`#[overdrive_test]`. The declaration is written as a record into the
`overdrive_test_placement` ELF section; the Metal wrapper reads it, and
`cargo xtask placement-lint` runs the wrapper's reader over the workspace listing before
any guest boots (wired into pre-push and CI). The 4,680-entry `infra/metal/test-placement.json`
table, the wrapper's name-keyed lookup, the ~3,900-line fwd guest-suite dispatcher, its
environment keys and the name-keyed lane slot table are deleted.

## Business context

Placement lived in three places: the test, a name-keyed JSON table, and per-group lists in
the dispatcher. The wrapper refused an absent entry only at Metal-run time and the rules
stated that no gate checked the table's completeness. The persona is the engineer (or the
coding agent acting for them) who adds, renames or moves an integration test and should
learn at lint time, not on the Metal box, that it is unplaced or mis-placed.
Source: GH #354; job `J-PLAT-007`; journey `declare-where-a-metal-test-runs`.

## Key decisions

All DESIGN decisions DDD-1 to DDD-18 were approved by the user (2026-11-13 and 2026-11-14).
Decision records: ADR-0282 to ADR-0295 (ADR-0285 is intentionally absent; the number was
never used).

| Decision | Outcome | Decided | Authority |
|---|---|---|---|
| Attribute surface (DDD-1) | `#[overdrive_test]` with five per-test forms plus a no-argument form reading the module's `shared_guest!`; the macro never inserts `#[test]` | user | [ADR-0282](../product/architecture/adr-0282-a-metal-test-placement-is-declared-on-the-test-and-recorded-in-its-binary.md), [ADR-0289](../product/architecture/adr-0289-a-module-opts-into-a-shared-guest-with-a-function-like-declaration-its-tests-read.md) |
| Record and reader (DDD-2) | ELF-section record; the wrapper is the only run-time reader | user | ADR-0282 |
| Eager lint (DDD-3) | `placement-lint` checks every listed test before any guest boots | user (U-1) | [ADR-0288](../product/architecture/adr-0288-an-eager-placement-lint-checks-every-listed-test-before-any-guest-boots.md) |
| Host-pure scale (DDD-4) | per-binary default and per-module declaration, with precedence and `placement_conflict` | user (U-4) | [ADR-0287](../product/architecture/adr-0287-host-pure-placement-is-declared-once-per-module-or-per-binary.md) |
| Closed registry (DDD-5) | groups and profiles are a closed registry checked at compile time | user | [ADR-0294](../product/architecture/adr-0294-guest-groups-and-environments-are-a-closed-registry-of-named-profiles.md) |
| Shared guest (DDD-7 to 9, 16 to 18) | one owner process per sharing scope; one test at a time, never cancelled; guest discarded after failure, loss, bound or detach; every exec bounded; owner exits when idle | user (incl. N-1 to N-3) | [ADR-0283](../product/architecture/adr-0283-the-tests-of-a-module-share-one-guest-through-a-per-module-owner-process.md), [0284](../product/architecture/adr-0284-a-shared-guest-serves-one-test-at-a-time-and-never-cancels-a-running-test.md), [0290](../product/architecture/adr-0290-the-shared-guest-owner-is-init-of-its-own-pid-namespace-so-its-guest-dies-with-it.md), [0291](../product/architecture/adr-0291-a-shared-guest-is-discarded-after-a-failed-lost-bounded-or-detached-test.md), [0292](../product/architecture/adr-0292-every-shared-exec-has-a-bound.md), [0293](../product/architecture/adr-0293-a-shared-guest-owner-exits-when-it-is-idle.md) |
| Off Metal (DDD-11) | a guest-placed test runs in place outside a Metal run | user (U-2) | [ADR-0286](../product/architecture/adr-0286-a-guest-placed-test-runs-in-place-outside-a-metal-run.md) |
| Dev-dependency rule (DDD-12) | `xtask` and `core` crates may dev-depend on the placement macros | user (U-3) | [ADR-0295](../product/architecture/adr-0295-xtask-and-core-crates-may-take-a-dev-dependency-on-the-placement-macros.md) |

Decisions taken during DELIVER (feature-delta, Upstream Issues table; each incorporated
into the design sections in present tense):

| # | Change | Decided |
|---|---|---|
| 1 | Registry is 22 environments over 8 groups: `mtls-relay` gains profile `fib-hit` | user, 2026-11-15 |
| 2 | Placement of `probe_gold` peer-reset test, the three attempt fixtures and an xtask guest-body test as `core`; `#[ignore]`d bodies selected with `--run-ignored` run in their guest | user, 2026-11-15 |
| 3 | `#[should_panic]` composes with guest, lane and shared placements; attribute ordering rule | user, 2026-11-15 |
| 4 | Stale table entries naming nonexistent tests are deleted | user, 2026-11-15 |
| 5 | Lane slot table removed; `lane_owner` takes a required `host = <path::to::fn>` | orchestrator (delegated), 2026-11-15 |
| 6 | `placement-lint` resolves a lane fixture against the lane test's package | orchestrator, 2026-11-15 |
| 7 | Six tests the table called host-pure that boot `serve` or a microVM became `guest = "cgroup-default"` | orchestrator, 2026-11-15 |
| 8 | Guest console dump on SIGTERM restored on the declared path; dispatcher's Quint staging check dropped | orchestrator, 2026-11-15 |
| 9 | Short attribute form with `use overdrive_testing::overdrive_test;` is the convention | user, 2026-11-15 |
| 10 | `#[doc(hidden)]` macro-support items are part of the contract | orchestrator, 2026-11-15 |
| 11 | Rendezvous directory scoped per cgroup namespace | crafter-internal |
| 12 | Fixture existence checked by `placement-lint` only; run-time lane check refuses a lane outside its package (review finding H1) | orchestrator, 2026-11-15 |

"Orchestrator" decisions were delegated by the user on 2026-11-15. No change adds a
deferral or scope limitation; no tracking issue was proposed.

## Work completed per roadmap phase

The branch was rebased during delivery; steps 01-01 to 06-02 are already on `main`.
Commits below are those recorded for each phase (`git log origin/main..HEAD` lists the
rest). The walking skeleton is `28b337491`.

| Phase | Steps | Commits |
|---|---|---|
| 01 Guest placement on the test | 01-01 typed guest-kit surface; 01-02 ELF record read by the wrapper | `b5e28855f` (typed hand-off, macro stops inserting `#[test]`), `06cdffd6d` |
| 02 Host-pure placement | 02-01 test, module and binary forms with precedence and conflict detection | `07adcb11c` |
| 03 Lane and launcher placement | 03-01 `lane_operator`, `lane_owner`, `native_launcher` | `d3c78981c` |
| 04 Unplaced test fails in the default lane | 04-01 `placement-lint` | `b81d5a19b` |
| 05 Guest artifacts per group | 05-01 closed registry with build-time profile rejection | `e60b52fff` |
| 06 Module tests share one guest | 06-01 owner core and Quint conformance replay; 06-02 `shared_guest!`, owner process, client protocol | `8a11f4685`, `d3309bd34` |
| 07 Guest placements migrated | 07-01 core, probe, unload, ulp, stress, nested-boundary; 07-02 mtls-relay; 07-03 cgroup-default | `6c9d9dbbb`, `246c86ce2`, `d12989db1`, `09e5f56b3` |
| 08 Host-pure placements migrated | 08-01 core and sim; 08-02 control-plane, worker, cli, host, gateway, reconcilers, store-local; 08-03 remainder | `e4ce696c7`, `815d56b2c`, `182483716`, `ab448287d`, `bfaa4a2c6`, `39f23a9ca`, `3b350f944`, `d1f855034`, `ba760a340`, `6f140ec27`, `96368c3a3`, `deda9e154`, `bd7be2dce` |
| 09 Table and dispatcher retired | 09-01 | `fe00896c9` |

DELIVER-wave additions on the branch: `e653ca8a7` (guest-placed `should_panic` reports the
in-guest outcome), `956553584` (rules: pending tests are `should_panic`), `2d2ea25fa` (owner
lane names its host script; no slot table), `8ccc05d14` (serve-booting tests run in a guest;
a driver ahead of its owner API names what it waits for), `33ce91de5` (decisions recorded),
`f017560ae` (roadmap and log).
Review resolution: `22f2a2bcf`, `05cd76d70`, `186ac8394`, `b54586acf` (placement-lint runs the
wrapper's record reader), `3a235aa9b` (wrapper check mode reports missing fixtures).
Closeout: `aa84fc75e` (delivery summary), `a63e4aa34` (how-to page and blog).

The table's 4,680 entries were moved onto their tests in scripted batches with per-test
parity checks against the table snapshot; entries naming tests that no longer existed were deleted.

## Quality gates

| Gate | Outcome |
|---|---|
| DES integrity (`des-verify-integrity`) | all 15 steps have complete traces |
| Implementation review | first pass rejected (2 HIGH, 3 MEDIUM, 6 LOW, plus subtractive findings); all resolved in `c19201c49`, `9519e3e37`, `80fb2fa6c`, `0bac89aa7`; the re-review's remaining finding (two record readers) resolved by `0bac89aa7` |
| Mutation testing | not run, by user ruling (not a design driver for this feature, skipped in delivery); no score is claimed |
| `placement-lint`, `conformance-lint`, workspace clippy `-D warnings` | clean on the rebased branch |
| Full-workspace Metal comparison against `main` | no regressions (below) |

Full-workspace comparison (2026-11-16): `cargo xtask metal run -- cargo nextest run --workspace
--features integration-tests,kvm-tests --no-fail-fast` on the branch at `06e50caf1` (Metal run
`c389d468c90cab793d1591e5`) and on its merge base with `main` at `52268fd5c`, plus
`-p overdrive-bpf --features overdrive-bpf/integration-tests` on both, compared by test name.
Taken before the last review fix; the xtask suite then passed 428 of 428.

| Side | Tests | Passed | Failed | Skipped |
|---|---|---|---|---|
| branch `06e50caf1` | 4,726 | 4,654 | 72 | 27 |
| main `52268fd5c` | 4,733 | 4,662 | 71 | 29 |

- 69 tests fail on both sides; 63 are `overdrive-cli` VM integration tests, and 59 of those carry
  `SystemdDetectVirtUnavailable { NotFound }` on both sides.
- 3 failed only on the branch; each passed on rerun in isolation on the branch and on `main`
  (`vsock_fault_matrix::a_guest_reset_ends_the_vmm_and_resets_every_far_end`,
  `guest_flow_through_serve::a_dropped_probe_stream_is_an_abort_at_the_guest_through_serve`,
  `vsock_walking_skeleton::a_vm_workload_round_trips_tcp_and_udp_through_serve_and_deploy`, which
  passes about half the time on both sides).
- 2 failed only on `main` and passed on the branch.
- Tests present on one side only are the retired dispatcher and table tests (main) and their replacements (branch).
- `overdrive-bpf` integration: 20 of 20 on both sides.

## Lessons learned

- **A design that kept a name-keyed table contradicted its own purpose.** The DESIGN retained a
  name-keyed lane slot table (`INIT_GUEST_LANE_PLACEMENTS`) in a feature whose point was to remove
  name-keyed placement. Only the user caught it; the fix was `lane_owner ... host = <path::to::fn>`
  (change 5). Test a design against the feature's stated purpose, not only against its own contracts.
- **An oracle defect in a pre-authored test.** In step 06-01 the owner-core test oracle never dequeued
  a `BootFailed` head, so three of eighteen tests could not pass against a correct owner (log: 06-01
  `BLOCKED_BY_DEPENDENCY`). The acceptance designer owns the body; the fix routed back to it rather
  than weakening the implementation.
- **`should_panic` and guest placement did not compose.** A pending test marked `should_panic` ran its
  body in a guest while libtest watched the host half. Fixed in `e653ca8a7` and recorded as change 3;
  the rule now prefers `should_panic` to `ignore` so a pending test is proven RED on every run.
- **Two record readers.** `placement-lint` first carried its own reader of the ELF record beside the
  wrapper's, a drift risk caught in review. It now runs the wrapper's reader (`b54586acf`).
- **JSON re-serialisation churn.** Rewriting the 4,680-entry table with a JSON round trip changes
  layout; counts were recomputed by parsing and each batch checked per test against the snapshot,
  since line-count based counts break on layout change.
- **Premise check on "host-pure".** Six tests the table called host-pure boot `serve` or a microVM
  (change 7); migration must read the body, not copy the old classification.

## Issues encountered

- The Metal box ran out of disk during skeleton timing runs (spike findings).
- `#[used]` alone did not keep the section record through linking; the test body references it.
- `systemd-detect-virt` is unavailable on the Metal guests, producing the 59 shared failures in the comparison.
- `vsock_walking_skeleton` is flaky on both `main` and the branch.
- The comparison was taken before the last review fix; the fix's own suite (xtask 428 of 428) is the
  evidence for that change, not a repeat of the full comparison.

## Permanent artifacts and rule changes

- ADRs: [adr-0282](../product/architecture/adr-0282-a-metal-test-placement-is-declared-on-the-test-and-recorded-in-its-binary.md)
  to adr-0295 in `docs/product/architecture/` (0282, 0283, 0284, 0286 to 0295; 0285 intentionally absent).
- Blog post: [Declaring where a test runs](../../website/content/blog/declaring-where-a-test-runs.mdx).
- How-to page: [Run verification on Metal](../../website/content/docs/how-to/run-verification-on-metal.mdx),
  updated for declaring placement.
- Rules: `.claude/rules/testing.md` (new "Test placement" section; pending tests are marked
  `#[should_panic(expected = ...)]` and proven RED, with `#[ignore]` only when a needed resource is
  missing; the `// conformance-lint: pending-owner-api <...>` marker for a driver ahead of its
  owner API) and a new `CLAUDE.md` section "Declare where a Metal test runs on the test".
- Formal model: `specs/quint/shared-guest-owner/` spec replayed by the conformance driver (step 06-01).

## Migration of lasting artifacts

This feature uses the lean feature-delta layout. None of the destination-map sources exist:
no `design/{architecture-design,component-boundaries,technology-stack,data-models}.md`,
no `design/adrs/ADR-*.md`, no `distill/walking-skeleton.md`, no `discuss/journey-*`. The ADRs
already live in `docs/product/architecture/` and were not copied. Nothing was migrated; all wave
history is preserved under `docs/feature/metal-test-placement-attribute/`.
