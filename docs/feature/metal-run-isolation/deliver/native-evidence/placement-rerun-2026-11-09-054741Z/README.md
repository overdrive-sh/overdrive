# Placement retry — direct native passes; named nested image staging fails

This directory preserves **92 raw placement files** copied byte-for-byte
from `/tmp/metal-native-remaining-20261109T054741Z-placement`, plus the two preparation siblings
`/tmp/metal-native-remaining-20261109T054741Z-prepare.foreground.log` and
`/tmp/metal-native-remaining-20261109T054741Z-prepare.invocation.json`, retained as
`prepare.foreground.log` and `prepare.invocation.json`. Only this README is
new narrative; the capture bytes and original invocation receipts are unchanged.

## Invocation and source attribution

Both invocation receipts name source HEAD `eb210bb204e6d93f860faa8de20b1cc30ca35462` and retain the actual dirty
source state, including the existing service-feature edits and in-progress
review artifact. This was not a clean-source run. Commands below come directly
from those receipts.

Preparation (`prepare.invocation.json`), 2026-11-09T05:47:41.410056+00:00 through
2026-11-09T05:50:02.608016+00:00, exit **0**:

```sh
cargo xtask metal run -- cargo xtask guest-kit-prepare
```

The preparation log reports run `d9781bfe77dc6d66c32c0587`, prepared kernel
`7.0.0-38-generic` and target `x86_64-unknown-linux-musl`, then a retained
command result 0 with `cause=command_returned`.

Placement (`foreground-invocation.json`), 2026-11-09T05:50:02.608958+00:00 through
2026-11-09T05:55:02.627912+00:00, driver exit **1**:

```sh
bash infra/metal/tests/placement_journey.sh /Users/marcus/conductor/workspaces/helios/ouagadougou /tmp/metal-native-remaining-20261109T054741Z-placement
```

Per-run IDs and results below come from their own stderr/status files. No
unretained per-run source digest or owner metadata is reconstructed from the
separate attempt/timeout observations.

## Actual results

| Captured selection | Actual status | Evidence boundary |
|---|---:|---|
| Prepared guest lane | 0 | Preparation command completed; not a complete placement result |
| Selected attempts, repeat collections, timeout, guest-execution failure and default/ci profile cases | 100 each | Deliberate expected outcomes; raw streams and status files retained |
| Direct native production-launch-identity case | 0 | One selected test passed with its native identity/device/filter/data receipt |
| Named nested guest-boundary case | 100, expected 0 | Selected body failed image staging; placement driver stopped |
| Driver-owned foreign-fixture setup and cleanup | 0 each | Explicit fixture operations completed; not a full final kernel/root complement |

The direct native run `e0d720ab1d2a2919487e7424` executes
`integration::vsock_vmm_device::the_fork_boots_and_passes_data_under_the_production_launch_identity`.
`direct-native.stdout` reports one test passed and its V1C receipt records
128MiB (`size=134217728`), UID/GID 4200, supplementary group 991, one
vhost-vsock descriptor, seccomp/Landlock arguments, and a successful data
round trip. This is current native success for that exact existing case;
it does not claim all other native cases ran or infer a parent stat that the
success path did not emit.

The named nested run `a52bdbc1f4a0ecbd65be1cb3` selects
`integration::guest_boundary::a_guest_application_reset_reaches_the_host_application_as_a_reset`.
Its retained command result is 100 with `cause=command_returned`.
`named-nested.stdout` shows the outer disposable guest completing its XFS
mount, loading nested KVM support, and reporting `/dev/kvm` and
`/dev/vhost-vsock` at UID/GID 0 and mode 0660 before entering the selected body.
The body then panics at `guest_boundary.rs:91:71` while staging its scripted
image: the running guest kernel `7.0.0-38-generic` has no staged
`net/sched/sch_prio` module at either the named `.ko.zst` or `.ko` path.
This is the actual recorded staging failure, not a completed nested reset or
a direct-native result. The full error, console, Nextest summary and outer
collector marker remain in the raw files; no production cause beyond them is
asserted here.

## Completion limits

The placement driver exited at `named-nested: expected 0, got 100`. Later
selections were not reached, and **no `after.json` final physical-kernel
snapshot exists**. This capture therefore does not establish complete placement
acceptance, the final physical-kernel comparison or all final residue
complements. Foreign-fixture cleanup returning 0 must not be substituted for
those missing observations.

No test, preparation, native command, model check, source/design change or DES
phase event was performed while preserving this evidence. The original `/tmp`
capture and preparation siblings were left unchanged.
