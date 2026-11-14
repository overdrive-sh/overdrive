# Slice 09 — Module tests share one guest

Story: US-PLACE-09. Job: J-PLAT-007. Decision: D8. Depends on: slice 01 (per-test guest path), slice 04 (per-group artifacts, so a shared guest stages the same things).

## Outcome

The tests of a module can declare that they share one guest of a group. An ordinary nextest run of that module boots one guest, and each test still reports its own result. This replaces the explicit batched `overdrive-fwd-guest-suite` command, so slice 08 can delete the dispatcher.

## Learning hypothesis

Disproves "a shared guest can keep nextest's per-test results, retries and filtering" if the only workable shape collapses a module into one nextest test. Confirms it if per-test reporting survives with one boot per module.

## In the slice

- A module-level declaration, `shared_guest!(guest = "<group>"[, profile = "<profile>"])`, read by the tests of that module that carry `#[overdrive_test]` with no arguments (shape approved by the user, 2026-11-13).
- A shared owner per declaration with an exec bound (DESIGN decision N-1, pending the user) and an idle exit (DESIGN decision N-2, approved 2026-11-14).
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

## Shape

A guest shared across test processes through a per-declaration owner process (ADR-0283, ADR-0290); tests in one guest share its kernel, cgroup tree, nft tables and `/tmp` (DDD-9).

## Estimate

2-3 days, after DESIGN pins the shape.
