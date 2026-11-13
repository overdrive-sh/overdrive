# Slice 09 — Module tests share one guest

Story: US-PLACE-09. Job: J-PLAT-007. Decision: D8. Depends on: slice 01 (per-test guest path), slice 04 (per-group artifacts, so a shared guest stages the same things).

## Outcome

The tests of a module can declare that they share one guest of a group. An ordinary nextest run of that module boots one guest, and each test still reports its own result. This replaces the explicit batched `overdrive-fwd-guest-suite` command, so slice 08 can delete the dispatcher.

## Learning hypothesis

Disproves "a shared guest can keep nextest's per-test results, retries and filtering" if the only workable shape collapses a module into one nextest test. Confirms it if per-test reporting survives with one boot per module.

## In the slice

- A module-level placement declaration (shape is DESIGN's: an attribute on the `mod`, or a `group = …` argument on each test).
- One guest per run of a declared module, counted from `OVERDRIVE_GUEST_KIT_TIMING`.
- Per-test PASS/FAIL, retries and `-E 'test(=…)'` filtering unchanged.
- The current users of the explicit batched command moved to the declaration.

## Not in the slice

Changing which tests share a guest today beyond what the batched command already groups; new guest groups; deleting the dispatcher (slice 08).

## Entry point and evidence

`cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(/^integration::cluster_and_node_commands::/)'` on a declared module. Evidence: one boot in the timing output, one result per test, a deliberately failing test reported alone.

## Acceptance

- [ ] A declared module boots one guest per run.
- [ ] Each test reports its own result; one failure does not hide the others.
- [ ] Filtering to a single test runs only that test.
- [ ] Guest-placed tests outside a declared module still boot one guest each.
- [ ] The explicit batched command has no remaining user.

## Open for DESIGN

How nextest's one-process-per-test model reaches a shared guest (one generated test per module, or a guest shared across test processes), who starts and stops it, and what state tests in one guest can leak to each other.

## Estimate

2-3 days, after DESIGN pins the shape.
