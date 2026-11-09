# Admission rerun — Nextest timeout expired during guest setup

This stable copy preserves the complete root capture from
`/tmp/metal-native-remaining-20261109T003244Z-admission`, including raw owner
snapshots and observer query errors. It is a failed admission-driver attempt;
no exclusive-maintenance or waiter invocation was started.

## Captured command and source

The ordinary durable run is `97fde96f2704e50754be8acd`, source commit
`401d902ed8af5ee2f883b2e39c6e3d8ba2523f02`, source digest
`e09f7c631cc2a6e4901e2b56363db7b976a61bf145a3f227d45c6d897ee575a0`, bound
to checkout key `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410`.
The host was `ubuntu@151.115.99.251`. The command selected the existing
standalone fixture manifest and one guest-group/core case:

```text
cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_attempt_waits_for_its_timeout)' --no-capture
```

Root reports the prescribed ordinary `guest-kit-prepare` run
`6002d0ff436a418ca5fcdcbe` completed with status 0 and prepared its declared
artifacts; that separate preparation log is not in this capture.

## Actual timeout state

Nextest started the selected case and reported `TERMINATING` / `TIMEOUT` at
10.001–10.002 seconds. The durable command returned aggregate status 100 with
`cause=command_returned`. There is no selected-body marker in any captured
guest console.

The owner samples narrow the cause to setup before VMM launch:

| Sample | Captured state |
|---|---|
| `ordinary-start-27.json` | Owner is still preparing; per-run roots have appeared. |
| `ordinary-start-28.json` | Owner is running; its process tree includes the dispatcher and `cargo build --package overdrive-init --target x86_64-unknown-linux-musl --release` plus active `rustc` children. No Cloud Hypervisor VMM or guest-console record is present. |
| `ordinary-start-29.json` through `ordinary-start-38.json` | The run remains active, with no VMM or guest-console record in the observed owner process set. |
| `ordinary-start-39.json` through `ordinary-start-41.json` | The unit is inactive with terminal command exit 100, complete receipt, zero processes, and all three roots absent. |

The observer's only query errors were early registration races while the
run-directory records were being created. Later owner samples were readable.
The final Nextest timeout happened while the dispatcher was still building
the static guest init; no VMM was present to execute the selected body. This
is not the prior source-root package-resolution failure and is not a missing
dispatcher or missing guest-console marker-format failure.

## Production owner path

The selected dispatcher resolves the `core` plan, then calls
`GuestKit::provision` before creating the VMM or executing the selected ELF
(`crates/overdrive-testing/bin/fwd_guest_suite.rs:3594-3635`). That method
unconditionally calls `vm_fixture::build_overdrive_init_static()` before it
returns (`crates/overdrive-testing/src/guest_kit.rs:452-469`). When
`/opt/overdrive-testing/prebuilt-overdrive-init` is absent, that function runs
the captured Cargo build (`crates/overdrive-testing/src/vm_fixture.rs:1051-1075`).
The `core` plan boots `GuestInit::ScriptedPeer` (`fwd_guest_suite.rs:3380-3385,
1394-1416`); it does not use the `overdrive-init` image binary that the
unconditional provision step builds.

The accepted fixture config gives Nextest a 10-second slow-timeout
(`infra/metal/tests/fixtures/attempt/.config/nextest.toml`). That timeout
covers the run-wrapper/dispatcher setup as well as the selected body. The
captured process tree shows the static init Cargo build still active during
that interval, before VMM launch. The selected fixture body sleeps for 180
seconds only after it reads its guest boot ID and prints/flushed
`METAL-ISOLATION-TIMEOUT body_started=true`; none of those body operations ran
in this capture.

## Disposition and owner route

The driver’s body-start observer is not the cause in this run: the owner
snapshots contain no VMM or guest console at all. The timeout expired while
`GuestKit::provision` was compiling a static artifact before guest creation.
The required remediation belongs to the existing GuestKit/preparation owner:
ensure this exact static `overdrive-init` target is available before the
per-attempt Nextest timeout starts, without changing the 10-second timeout or
body marker. The current `guest-kit-prepare` artifact list does not build that
static-musl binary; its `overdrive-init` lane entry builds the test target,
not the captured `--target x86_64-unknown-linux-musl --release` artifact.

This is not evidence of an admission-lock failure, dispatcher-path failure,
or marker buffering. The command reached START and failed as a Nextest timeout
while guest setup was still active. The driver stopped at its first
body-start guard, so it did not test exclusive maintenance waiting, the
unadmitted waiter, or cleanup complements. No source/test changes, native
commands, or DES phase events were made during this diagnosis.
