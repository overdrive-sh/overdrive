# Admission rerun — guest fixture CWD missing from disposable image

This stable copy preserves the completed current capture from
`/tmp/metal-native-remaining-20261109T012325Z-admission`. Its raw source/owner
snapshots and controller output are retained unchanged. This is the current
admission failure, separate from earlier attempts.

## Captured invocation and source

The ordinary invocation is run `e6f13d369350e848d51c0192`, source commit
`500126cddbd759b46f1f47201bf18e6b4d8bad5b`, source digest
`1b9ead3ea028bca3386d5b16fe9e52a50d2dc392a531394cf0469d8097b8b56d`, bound
to checkout key `e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410`.
The command selected the standalone fixture manifest and the existing
timeout body on `ubuntu@151.115.99.251`:

```text
cargo nextest run --manifest-path infra/metal/tests/fixtures/attempt/Cargo.toml --target-dir target/metal-run-isolation-fixture --run-ignored only -E 'test(selected_attempt_waits_for_its_timeout)' --no-capture
```

The selected test is explicitly ignored for ordinary suites and is enabled
only by this existing external admission driver. It asserts the guest boot
ID, then emits `METAL-ISOLATION-TIMEOUT body_started=true` and sleeps 180
seconds.

## Exact observed failure

The source-owned VMM serial capture in `ordinary.stdout` reports:

```text
overdrive-guest-peer: control connection to the host on 251709352
env: cannot change directory to '/home/ubuntu/overdrive-worktrees/e3d831d7dafe621226ae6047ebcac3536f37470cafcf5dbb64eb9598886af410/infra/metal/tests/fixtures/attempt': No such file or directory (os error 2)
```

Nextest marks the selected case `FAIL [1.830s]`, with zero passed. The outer
durable command returns aggregate exit 100 with `cause=command_returned`.
This is not a timeout or console-marker-format failure: the guest `/usr/bin/env
--chdir` fails before executing the selected ELF, so the body never reads its
boot ID or prints its marker.

`ordinary-start-32.json` records an active Cloud Hypervisor VMM (PID `1914862`)
with serial file
`/run/ovd-gk/e6f13d369350e848d51c0192/ovd-gk-1914549-2/kit-16785416/console.log`.
That console snapshot contains kernel boot output but no body marker. The
subsequent controller log includes the later serial-console `env --chdir`
error above. The final owner samples show terminal command 100, complete
receipt, no processes, and the three owned roots absent. The driver did not
write `ordinary-running.json`; exclusive maintenance and waiter stages did
not execute.

## Production caller and ownership path

The standalone fixture's `Cargo.toml` declares its own `[workspace]`; its
Nextest workspace/current directory is therefore
`.../infra/metal/tests/fixtures/attempt`. The ordinary run-wrapper validates
the selected source owner and routes this identity to guest-group `core`.
`run_nextest_group_attempt` obtains the selected Nextest current directory
(`fwd_guest_suite.rs:3094-3104`) and passes it to
`exec_nextest_guest_attempt` (`fwd_guest_suite.rs:3532-3560`). That path
constructs `/usr/bin/env --chdir <selected-current-dir>` without changing the
selected path (`fwd_guest_suite.rs:1419-1447`).

`nextest_attempt_files` stages the actual selected ELF and Nextest helpers at
their reported guest paths (`fwd_guest_suite.rs:3090-3127`). The GuestKit
image installs those files and their parent directories
(`guest_kit.rs:487-552`); it does not stage the separate standalone fixture
workspace directory. Consequently `/usr/bin/env` cannot chdir to the exact
selected directory, and the selected ELF never starts.

## Bounded remediation

The dispatcher now stages the existing `/usr/bin/mkdir` guest utility and,
after booting the selected guest, materializes **that exact Nextest working
directory** inside the disposable guest before `env --chdir` executes. It does
not copy the host checkout, substitute a fallback directory, or change the
selected path, argv, helpers, environment, or body assertion. The directory is
inside the invocation-owned guest image and is destroyed with that guest.

Focused canonical Lima checks after the fix:

| Check | Result |
|---|---|
| `cargo check -p overdrive-testing --all-targets --features integration-tests` | Passed |
| `cargo clippy -p overdrive-testing --all-targets --features integration-tests -- -D warnings` | Passed |
| `cargo nextest run -p overdrive-testing --lib --features integration-tests` | 24 passed |
| Existing `sad_invalid_context_missing_identity_and_missing_guest_dispatcher_never_run_the_body` placement acceptance | 1 passed, 377 filtered |
| Existing `non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain` placement acceptance | 1 passed, 377 filtered |
| `rustfmt --edition 2024 --check crates/overdrive-testing/bin/fwd_guest_suite.rs` | Passed |

This raw native capture remains the reproduced failing evidence; no native
rerun has occurred after the remediation. Independent review and the
prescribed preparation/admission/placement reruns remain outstanding. No
acceptance body, test identity, or DES phase event was changed or added.
