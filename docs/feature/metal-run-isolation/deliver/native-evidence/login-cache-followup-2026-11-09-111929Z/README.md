# Login-mode Cargo cache handoff — first native follow-up

## Scope and result

This capture reran only the two remaining login-mode tails from the existing
placement driver. Both used the exact existing acceptance selector
`placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain`:

```text
cargo xtask metal run --no-sudo -- cargo nextest run -p xtask --test acceptance --features integration-tests -E 'test(placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain)' --no-capture
cargo xtask metal run --no-sync --no-sudo -- cargo nextest run -p xtask --test acceptance --features integration-tests -E 'test(placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain)' --no-capture
```

Both outer commands exited 101 after the real durable owner emitted
`OVERDRIVE_METAL_RUN_STARTED`. The bootstrap temporary-root preparation
therefore passed the earlier login-mode creation failure. The selected
acceptance body did not start: Cargo could not open the bound checkout's
`target/debug/.cargo-lock` because the file was not writable by the login
user. Each owner recorded `cause=command_returned`; this is a failed native
attempt, not acceptance evidence.

The first run used ID `cce6d6ed6f5fe4faaf8555f2`; the no-sync run used
`04241420430e5cd67d0298cb`. The no-sync command passed the source identity
check. No checkout file changed between the two invocations. Their raw output,
commands, statuses and source metadata are retained byte-for-byte in this
directory. The first invocation records HEAD
`7d8da10c9a5fa5de15fdf45837b1393418176278` and the bootstrap content hash
`5224234003ec4142f700bac42320de5d4e500e567c2036bb5abb917ba4cb996a`; the
second command ran before any further source edit and had the same source
state.

The preexisting capture in
[`feature-native-closeout-2026-11-09-104319Z`](../feature-native-closeout-2026-11-09-104319Z/README.md)
records the earlier `target/metal-run-tmp` root-owned 0755 failure. The
private bootstrap change now gives that common parent the login primary group
write/search access with the sticky bit, preserving each invocation's
existing 0700 root and owner. This follow-up exposed the next access boundary:
root-owned Cargo artifacts under this same bound checkout's default
`target/debug` prevent the exact login-mode command from starting.

## Bounded implementation

The accompanying source change hands only root-owned entries under the
bound checkout's `target/debug` back to the remote login user before a
login-mode command. It leaves other target subtrees and all three invocation
roots untouched. The checkout lock is already held during this preparation.
The two failures above predate that cache handoff and do not validate it; the
final source must be checked by rerunning these same two selectors.

## Focused Lima verification

After the bounded source change, canonical Lima checks passed:

- `bash -n infra/metal/bootstrap.sh`
- `controller::sad_registered_missing_rsync_keeps_its_preparation_error_and_no_sync_needs_no_rsync`
- `placement::non_metal_and_explicit_exclusive_attempts_preserve_the_original_exec_chain`
- `finalization::finalization_removes_exact_owned_roots_and_repeating_it_preserves_the_complement`

The three existing acceptance cases passed. They are not native validation of
the remote Cargo cache handoff.
