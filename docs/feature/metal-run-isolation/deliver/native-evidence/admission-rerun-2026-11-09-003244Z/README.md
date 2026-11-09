# Admission rerun — guest setup exceeded the selected test timeout

This stable copy preserves the completed root capture from
`/tmp/metal-native-remaining-20261109T003244Z-admission`, including owner
snapshots, console observations and query errors. It is a failed admission
journey; the exclusive-maintenance and waiter stages did not run.

## Source and command

The ordinary invocation is run `97fde96f2704e50754be8acd`, source commit
`401d902ed8af5ee2f883b2e39c6e3d8ba2523f02`, source digest
`e09f7c631cc2a6e4901e2b56363db7b976a61bf145a3f227d45c6d897ee575a0`, bound
to checkout key `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410`.
The target host is `ubuntu@151.115.99.251`.

The command selects the fixture manifest with a separate target directory for
its Nextest test executable:

```text
cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_attempt_waits_for_its_timeout)' --no-capture
```

Root reports that the preceding approved ordinary `guest-kit-prepare` run
`6002d0ff436a418ca5fcdcbe` completed with status 0. Its output is not in this
admission capture.

## Actual timeout and owner state

Nextest started the selected fixture case and timed it out at 10.001–10.002
seconds. The durable ordinary command completed with aggregate exit code 100
and `cause=command_returned`. The admission driver never observed
`METAL-ISOLATION-TIMEOUT body_started=true` and did not write
`ordinary-running.json`.

The retained owner snapshots establish that setup had not reached guest
execution before the timeout:

| Snapshot | Captured state |
|---|---|
| `ordinary-start-27.json` | The ordinary owner is active and preparing; its invocation roots exist. |
| `ordinary-start-28.json` | The owner is running. Its process tree includes the dispatcher, `cargo build --package overdrive-init --target x86_64-unknown-linux-musl --release`, and Rust compiler children. No Cloud Hypervisor process or guest-console record is present. |
| `ordinary-start-29.json` through `ordinary-start-38.json` | The owner remains active; the observation set contains no VMM or guest console. |
| `ordinary-start-39.json` through `ordinary-start-41.json` | The owner is inactive with command terminal 100, complete receipt, no processes, and all three roots absent. |

The corrected driver checks the body marker in the run-owned VMM serial
console. The matching test source validates the guest boot ID, prints and
flushes the marker, then sleeps for 180 seconds. No console or VMM exists in
these observed samples, so this timeout occurred before the body could run;
this is not a buffered marker or format mismatch.

## Production setup path

The selected Nextest placement is the existing `guest-group/core` path. Its
dispatcher calls `GuestKit::provision` before it creates the VMM and executes
the selected ELF (`crates/overdrive-testing/bin/fwd_guest_suite.rs:3594-3635`).
`GuestKit::provision` unconditionally calls
`vm_fixture::build_overdrive_init_static()` (`crates/overdrive-testing/src/guest_kit.rs:452-469`).
Unless `/opt/overdrive-testing/prebuilt-overdrive-init` is present, that owner
runs the exact Cargo build captured in `ordinary-start-28.json`
(`crates/overdrive-testing/src/vm_fixture.rs:1051-1075`). The `core` plan
creates `GuestInit::ScriptedPeer` images
(`fwd_guest_suite.rs:3380-3385,1394-1416`), so this selected attempt does not
consume the `overdrive-init` image binary that the unconditional provisioning
step builds.

The fixture's configured 10-second Nextest timeout covers the run-wrapper and
dispatcher setup as well as the selected body. The active process snapshot
shows the static-init Cargo build still compiling when the attempt timed out,
before any VMM was launched. The root's guest-kit preparation completed, but
the existing preparation artifact set does not prebuild this exact static
musl `overdrive-init` artifact: its overdrive-init lane builds the test target,
while the runtime build is `--target x86_64-unknown-linux-musl --release`.

The required remediation belongs to the existing GuestKit/preparation owner:
make the static-init artifact needed by this GuestKit path available before
Nextest's per-attempt timeout starts, without increasing the timeout or
changing the body marker. No new API, build variant, preparation command,
timeout rule or test fixture is authorized by this evidence.

No admission owner snapshot was captured before the body-start guard failed,
so this attempt does not establish exclusive maintenance wait, predecessor
stop ordering, an unadmitted waiter, or a successful admission journey. The
raw capture is not a native acceptance pass. No production/test change, new
native command, or DES phase event was made during this diagnosis.
