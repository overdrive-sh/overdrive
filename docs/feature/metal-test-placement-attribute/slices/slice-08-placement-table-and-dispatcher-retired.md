# Slice 08 — Placement table and dispatcher retired

Story: US-PLACE-08. Job: J-PLAT-007. Depends on: slices 01-07.

## Outcome

Placement has one source: the attribute on the test. The JSON table, the wrapper's table lookup and the separate dispatcher (or everything of it that DESIGN decided not to keep) are deleted, and `.claude/rules/testing.md` § "Test placement" describes the real model.

## In the slice

- Delete `infra/metal/test-placement.json` and the wrapper's lookup, parser and its `test_unclassified` / `placement_*` diagnostics that become unreachable (delete the code and its tests together).
- Delete `fwd_guest_suite.rs`, or reduce it to the batched path if DESIGN kept batched group runs.
- Update references (including the `overdrive-fwd-guest-suite` command cited in `docs/product/journeys/verify-independent-worktrees.yaml` and `docs/feature/metal-run-isolation`) in the same change.
- Update testing.md § "Test placement" to the attribute and the lint, through the normal rules-change process.

## Not in the slice

New features on the attribute; changing group semantics.

## Entry point and evidence

Rename a declared test (for example `cluster_status_with_no_server_returns_transport_error`) and run `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(<new name>)'`: it passes with no other edit, and `git ls-files infra/metal/test-placement.json` prints nothing. Evidence: full Metal pass set equal to the pre-retirement set.

## Acceptance

- [ ] A renamed test and a brand-new declared test pass on Metal with no table or dispatcher edit.
- [ ] Full Metal pass set equals the pre-retirement set.
- [ ] Dispatcher deleted or reduced exactly as DESIGN decided.
- [ ] No stale references remain (search clean for the removed names).
- [ ] testing.md § "Test placement" matches the shipped model.

## Estimate

1 to 2 days.
