# Admission rerun — dispatcher reached a fixture-workspace build failure

This stable evidence copy preserves the completed root run from
`/tmp/metal-native-remaining-20261108T233719Z-admission`. It contains the
ordinary controller stdout/stderr and the driver's stderr. The run reached
the dispatcher after the prior dispatcher-path correction, then failed before
the selected body began. No admission owner snapshot or exclusive/waiter run
was produced.

## Captured execution

Run ID: `f5612f4192bdb6a36bebec93`. The ordinary Nextest command used the
standalone manifest at `infra/metal/tests/fixtures/attempt/Cargo.toml` and
selected `metal-run-isolation-attempt-fixture::attempt::selected_attempt_waits_for_its_timeout`.
The ordinary durable runner emitted START and retained command exit code 100
with `cause=command_returned`.

The selected executable started its guest-group dispatcher, which emitted
`OVERDRIVE_METAL_TEST_ERROR error=guest_execution_failed`. The detail was
`OverdriveInitBuildFailed`: `cargo build --package overdrive-init --target
x86_64-unknown-linux-musl --release` exited 101 with Cargo reporting that
`overdrive-init` did not match any packages. Nextest reported one failed test
in 0.215 seconds. No `METAL-ISOLATION-TIMEOUT body_started=true` marker was
present; the body did not reach its guest boot-ID assertion, flush, or wait.
The admission driver's line 27/30 body-start guard therefore failed before it
could observe the ordinary owner or start the exclusive-maintenance and
waiter portions.

This is not a guest-stdout buffering case: the captured dispatcher error
identifies a build failure before VMM launch and before test-body execution.
The captured command does not produce an `ordinary-running.json`, owner
snapshot, maintenance command, or waiter record.

## Production owner path

The approved ordinary Metal context includes the canonical bound checkout in
`OVERDRIVE_METAL_SOURCE_ROOT` (`infra/metal/bootstrap.sh:793-796`). The
run-wrapper validates that source owner before invoking the prepared
dispatcher (`infra/metal/test-wrapper.sh:38-56,212-231`). It passes the
selected binary and test identity into `overdrive-fwd-guest-suite`, which
retains `OVERDRIVE_METAL_SOURCE_ROOT` for the selected executable and helper
files (`crates/overdrive-testing/bin/fwd_guest_suite.rs:3601-3623`).

The guest fixture itself is a separate Cargo workspace: its
`infra/metal/tests/fixtures/attempt/Cargo.toml` declares a standalone
`[workspace]` and contains no `overdrive-init` package. The dispatcher calls
`GuestKit::provision` at
`crates/overdrive-testing/bin/fwd_guest_suite.rs:3605-3612`; provisioning calls
`VmFixture::provision` and then builds the static init binary at
`crates/overdrive-testing/src/guest_kit.rs:452-469` and
`crates/overdrive-testing/src/vm_fixture.rs:1051-1075`. Before remediation,
`vm_fixture::workspace_root()` always ran
`cargo locate-project --workspace` from the dispatcher's inherited working
directory (`vm_fixture.rs:1078-1108`). Under this standalone manifest, that
resolves the fixture workspace. The ensuing `cargo build --package
overdrive-init` therefore fails exactly as captured.

## Bounded remediation

`vm_fixture::workspace_root()` now uses the existing `OVERDRIVE_METAL_SOURCE_ROOT`
only when the existing execution class is `ordinary`. The wrapper has already
validated that canonical source owner before dispatch. Non-Metal and explicit
exclusive invocations retain their prior runtime Cargo workspace discovery;
the dispatcher does not change the selected test's working directory,
arguments, environment, or result. No public API or environment name, new
fallback, on-demand build, or acceptance body changed. The wrapper continues
to refuse an actual missing source context before dispatcher execution.

Focused canonical Lima checks after the fix:

| Check | Result |
|---|---|
| `cargo nextest run -p overdrive-testing --lib --features integration-tests` | 24 passed |
| `cargo check -p overdrive-testing --all-targets --features integration-tests` | Passed |
| `cargo nextest run -p xtask --features integration-tests -E 'test(acceptance::metal_run_isolation::placement::sad_invalid_context_missing_identity_and_missing_guest_dispatcher_never_run_the_body)'` | 1 passed, 377 filtered; fail-closed absence behavior remains intact |
| `rustfmt --edition 2024 --check crates/overdrive-testing/src/vm_fixture.rs` | Passed |

The broader `cargo fmt -p overdrive-testing -- --check` reported existing
formatting drift in untouched `crates/overdrive-testing/bin/guest_peer.rs`.
The root separately ran the approved ordinary `guest-kit-prepare` before this
admission attempt (run `4f4761a1bec8adeec7adea6d`, reported terminal 0 with
prepared artifacts); that prep output is not part of this three-file
admission capture. This admission capture predates the source-root resolver
remediation documented here. No admission or placement native driver was run
after that resolver fix; the captured admission result remains pending focused
independent review and the prescribed later native rerun. No DES phase event
is claimed for the root-executed failure capture.
