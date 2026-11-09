# Nested-boundary device-module ordering remediation

This note records the bounded test-support fix for the captured nested
placement failure in
[`placement-rerun-2026-11-09-051107Z`](../placement-rerun-2026-11-09-051107Z/README.md).
That foreground run failed with `guest_execution_failed` before the selected
body because `chmod /dev/kvm /dev/vhost-vsock` ran before the already-staged
KVM and vhost modules were loaded.

## Change

The first focused fix removed XFS from the shared `nested_vm_test_support`
module list. Review found that the existing batched
`run_nested_vm_test_binary_with_resources` caller also consumes that list and
needs XFS loaded before its `losetup`/reflink setup. The final correction keeps
XFS in the shared list. In the nextest per-test path, `prepare_nested_attempt`
loads XFS explicitly for `losetup`, then removes only that known path from its
local bulk list to avoid a duplicate load; it loads the remaining staged
modules before the device-node chmod. This matches the existing batch's
module-before-chmod order while preserving its shared XFS precondition. All
module load failures still propagate through the existing
`guest_execution_failed` path. No test body, kernel configuration, VMM
identity, confinement, guest input, or oracle changed.

## Checks

| Check | Result |
|---|---|
| `cargo xtask lima run -- cargo check -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests` | Passed. |
| `cargo xtask lima run -- cargo clippy -p overdrive-testing --bin overdrive-fwd-guest-suite --features integration-tests -- -D warnings` | Passed. |
| `cargo xtask lima run -- cargo nextest run -p xtask --test acceptance --features integration-tests -E 'test(acceptance::metal_run_isolation::placement::)' --no-capture` | 8 passed; 40 skipped by the filter. This includes the existing placement partition, non-Metal/exclusive, malformed-context, and missing-dispatcher refusal cases. |
| `cargo xtask lima run -- rustfmt --check --edition 2024 crates/overdrive-testing/bin/fwd_guest_suite.rs` | Passed for the changed file. |

`cargo xtask lima run -- cargo fmt --package overdrive-testing -- --check`
reported an import-order diff in the untouched
`crates/overdrive-testing/bin/guest_peer.rs`; that file was not changed. The
scoped check for `fwd_guest_suite.rs` passes.

The native placement journey has not been rerun after this change. The
051107Z capture remains the actual failure evidence; it is not retroactively
credited as a pass. No DES phase event was recorded for these remediation
checks.
