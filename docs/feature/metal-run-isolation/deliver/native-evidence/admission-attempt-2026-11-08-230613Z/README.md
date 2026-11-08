# Admission native attempt — selected body refused before start

This directory preserves the complete available output from
`/tmp/metal-native-full-20261108T230613Z-admission`. It contains the ordinary
controller stdout/stderr and the driver's stderr. The driver stopped at its
initial body-start guard; it did not create owner snapshots or begin its
exclusive-maintenance and waiter portions. The prepared worktrees were left
untouched.

## Captured invocation and failure

The driver attempted this ordinary Metal command on
`ubuntu@151.115.99.251`:

```text
cargo xtask metal run -- env RUST_LOG=metal_host_boot_id=<captured-host-boot-id> cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_attempt_waits_for_its_timeout)' --no-capture
```

Run ID: `11eae164e76918cb07451a86`.

The run reached START and Nextest began one selected case. The run-wrapper
reported `OVERDRIVE_METAL_TEST_ERROR error=lane_not_prepared`; Nextest then
reported one failed test, zero passed and two skipped. No
`METAL-ISOLATION-TIMEOUT body_started=true` line appears. The outer durable
command returned exit code 100 with `cause=command_returned`. The body did not
reach its own guest-boot-ID assertion or 180-second wait, so the driver's
body-start guard failed before it could exercise admission. No maintenance or
ordinary waiter invocation was started, and no `ordinary-running.json`,
`maintenance.*`, or waiter snapshot was produced.

This is a lane-preparation refusal before the selected acceptance body, not an
observed maintenance-lock or systemd-owner failure. The capture has no
`guest-kit-prepare` output, remote artifact listing, or owner snapshot; it
cannot establish whether preparation was omitted or which dispatcher path
check failed.

## Current producer and consumer paths

The fixture's Nextest configuration enables the existing wrapper. Its exact
test identity is classified as `guest-group` / `core` in
`infra/metal/test-placement.json`. For that placement,
`infra/metal/test-wrapper.sh:212-231` requires the configured dispatcher path
to resolve to an executable below the bound source checkout's `target/` tree
before it creates the guest-local namespace and execs the dispatcher. Any
failure in that gate emits the captured `lane_not_prepared` marker and exits
before the selected body.

The current preparation/launch paths do not agree:

| Owner | Path in current source |
|---|---|
| `cargo xtask guest-kit-prepare` | `xtask/src/main.rs:504-506` passes `cargo_target_dir(&root)` into `guest_kit_prepare::run`; `xtask/src/guest_kit_prepare.rs:213-217,283-297` builds and verifies `<target_dir>/debug/overdrive-fwd-guest-suite`. |
| Ordinary Metal bootstrap | `infra/metal/bootstrap.sh:671-675,793-796` sets `OVERDRIVE_METAL_TEST_DISPATCHER` to `<remote-checkout>/target/guest-kit/overdrive-fwd-guest-suite`. |
| Fail-closed consumer | `infra/metal/test-wrapper.sh:213-220` canonicalizes and checks the bootstrap-provided path, then returns `lane_not_prepared` if it is absent, outside `source/target/`, not a file, or not executable. |

The existing preparation code has no step that copies its verified
`<target_dir>/debug/...` output to the bootstrap's `<remote-checkout>/target/guest-kit/...`
path. The saved capture directly reproduces the consumer's fail-closed
`lane_not_prepared` result through the real ordinary Nextest run. The exact
failed subcheck is not emitted, but the source shows a producer/consumer path
mismatch that makes a successful preparation insufficient for the current
consumer path. `cargo xtask guest-kit-prepare`'s own module documentation
states that this existing step must run before each lane invocation; the
admission driver starts the ordinary lane at `infra/metal/tests/admission_journey.sh:23`
without invoking that preparation owner itself.

## Disposition

This capture does not show a production admission or cleanup defect. It shows
that the accepted preparation prerequisite was not demonstrated and that the
current preparation producer and wrapper consumer name different dispatcher
locations. The minimal implementation correction is to align those existing
paths while keeping the wrapper fail-closed and preparation in its existing
owner. No build was added to the wrapper and no acceptance assertion was
changed here.

The native admission and placement portions remain unproven. The capture does
not credit exclusive waiting, predecessor stop ordering, or ordinary waiter
behavior. No test, remote command, source change, or DES phase event was run
or recorded during this diagnosis.
