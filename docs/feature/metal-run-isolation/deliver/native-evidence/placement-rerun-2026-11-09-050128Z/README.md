# Placement rerun — repeat-collection stderr comparison failed

This directory preserves 19 raw files copied byte-for-byte from
`/tmp/metal-native-remaining-20261109T045906Z-placement`. The foreground
invocation record reports driver exit 1, source HEAD
`6fbb68fafc48a88057d2edf123bc5579bf236a80`, and the captured dirty source
status. The driver stopped at the repeat-collection comparison. It did not
reach the direct-native case.

## Exact captured result

The selected `attempts` command was:

```text
cargo xtask metal run -- env PROPTEST_CASES=7 PROPTEST_RNG_SEED=123456 RUST_BACKTRACE=1 RUST_LOG=metal_host_boot_id=<captured-host-boot-id> cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_environment_and_status)' --retries 1 --no-capture
```

Run `8c1b67ba311626eec8afbadc` completed with command status 100. Both
`attempts-collect.status` and `attempts-repeat.status` are also 100.
`attempts-collect.stdout` and `attempts-repeat.stdout` are byte-identical.
Their stderr differs only on line 1: Cargo reports `Finished dev profile` in
0.07s for the first collection and 0.06s for the repeat. All remaining stderr
bytes match, including the same retained completion result. The driver fails
at the exact stderr `cmp`; the foreground log records that mismatch.

The foreign fixture setup and cleanup both returned 0. The raw capture has no
`direct-native.*` files, so no direct-native test or later placement case was
run. This attempt establishes neither native placement success nor a full
placement journey result. It preserves the first collection mismatch as
captured; it does not assert a collector-result inconsistency.
