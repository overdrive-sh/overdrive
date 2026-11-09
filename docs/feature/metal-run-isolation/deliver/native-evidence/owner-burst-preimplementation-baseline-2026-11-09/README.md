# Preimplementation owner-burst fixture baseline

This isolated-worktree run used commit
`0e8ae265c2d80d98c345b342a396057940837e3a`, before metal-run-isolation
implementation. The exact existing owner fixture was run through canonical
`cargo xtask metal run --`:

```text
cargo nextest run -p overdrive-init --test integration
  --features integration-tests,kvm-tests
  -E 'test(=integration::in_guest_lane::a_burst_of_host_aborts_resets_every_connection_in_a_real_guest)'
  --no-capture --no-fail-fast
```

The source files containing the owner fixture and selected cell are byte
identical at this baseline and current HEAD:

- `crates/overdrive-init/tests/integration/in_guest_lane.rs`:
  `a7881f6cb85d60ccf0aa994dbec0c5670076704c`
- `crates/overdrive-init/src/guest_flow/real_kernel.rs`:
  `0792cdbadec5d20769eb196eecfd59be0166e230`

Canonical guest-kit preparation completed with exit 0 before the fixture
run. The fixture then failed with outer status 100 at the same
`real_kernel.rs:1623` assertion seen in the current selected-cell route:
the 1s `ABORT_SETTLE` expired with 78 of 1,000 flow ends consumed, and the
original host script received EOF waiting for `DONE`. The baseline source
status records the uninitialized `vendors/linux` submodule (`D`); both
preparation and the exact fixture run completed with that worktree state.

This establishes that the burst-cell failure predates the metal-run-isolation
implementation. It does not establish a pass for the current feature's owner
dispatch path. Raw output and source receipts are preserved under
`guest-kit-prepare/` and `original-fixture/`.
