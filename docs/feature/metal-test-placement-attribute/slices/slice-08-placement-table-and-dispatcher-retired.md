# Slice 08 — Placement table and dispatcher retired

Story: US-PLACE-08. Job: J-PLAT-007. Depends on: slices 01-07 and 09.

## Outcome

Placement has one source: the declarations in the test source. The JSON table, the wrapper's table lookup and the separate dispatcher are deleted (DDD-14), and `.claude/rules/testing.md` § "Test placement" describes the real model.

## In the slice

- Delete `infra/metal/test-placement.json` and the wrapper's lookup, parser and the table diagnostics that become unreachable (delete the code and its tests together).
- Delete the dispatcher binary (`fwd_guest_suite.rs`), the wrapper's `--guest-suite` mode, `OVERDRIVE_METAL_TEST_DISPATCHER`, the explicit batched command, `OVERDRIVE_FWD_GUEST_GROUP`, `OVERDRIVE_FWD_GUEST_EXACT`, the body-count constants and the nine dispatcher unit tests (deleted, not rewritten).
- Update references (including the `overdrive-fwd-guest-suite` command cited in `docs/product/journeys/verify-independent-worktrees.yaml`, `docs/feature/metal-run-isolation`, `infra/metal/README.md`, `website/content/docs/how-to/run-verification-on-metal.mdx` and `tests/conformance/CLAUDE.md`) in the same change.
- Update testing.md § "Test placement" to the attribute and the lint, through the normal rules-change process.

## Not in the slice

New features on the attribute; changing group semantics.

## Entry point and evidence

Rename a declared test (for example `cluster_status_with_no_server_returns_transport_error`) and run `cargo xtask metal run -- cargo nextest run -p overdrive-cli --features integration-tests,kvm-tests -E 'test(<new name>)'`: it passes with no other edit, and `git ls-files infra/metal/test-placement.json` prints nothing. Evidence: full Metal pass set equal to the pre-retirement set.

## Acceptance

- [ ] A renamed test and a brand-new declared test pass on Metal with no table or dispatcher edit.
- [ ] Full Metal pass set equals the pre-retirement set.
- [ ] The dispatcher is deleted entirely.
- [ ] These searches over the repository (excluding immutable history and recorded evidence) come back clean: `test-placement.json`, `fwd_guest_suite`, `overdrive-fwd-guest-suite`, `--guest-suite`, `OVERDRIVE_METAL_TEST_DISPATCHER`, `OVERDRIVE_FWD_GUEST_GROUP`, `OVERDRIVE_FWD_GUEST_EXACT`, `_COMPLETE_BODIES`.
- [ ] testing.md § "Test placement" matches the shipped model.

## Estimate

1 to 2 days.
