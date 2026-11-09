# Placement rerun — nested device modules load after the device check

This directory preserves 98 raw files copied byte-for-byte from
`/tmp/metal-native-remaining-20261109T051107Z-placement`. The foreground
invocation record reports placement-driver exit 1 and source HEAD
`841354ae5541b588073a2ec171c34a7364c9a367`; the captured dirty source status
is retained in that record.

## Captured boundary and status

The direct-native case passed: `direct-native.status` is 0 and run
`dca66678b632b1a68122ddd2` completed with command exit 0. The explicit nested
case then failed: `named-nested.status` is 100, run
`9c28953bbc4e4e93803cb8f8` completed with command exit 100, and the foreground
placement driver stopped with exit 1. Foreign-fixture setup and cleanup both
returned 0. Earlier selected attempt, timeout, guest-execution-failure, and
default/CI profile cases returned their expected status 100.

Nextest selected one test and skipped 99:

```text
overdrive-fwd::integration integration::guest_boundary::a_guest_application_reset_reaches_the_host_application_as_a_reset
```

The guest dispatcher returned the existing typed
`OVERDRIVE_METAL_TEST_ERROR error=guest_execution_failed` while preparing the
outer guest, before executing the selected test body. Its raw guest console
shows a successful boot and the `ovd_fwd`, `netem`, and XFS setup, followed by:

```text
chmod: cannot access '/dev/kvm': No such file or directory
chmod: cannot access '/dev/vhost-vsock': No such file or directory
```

The captured failure is absence of both device nodes at the preparation step;
it is not a permission-denied result. The placement driver stopped before its
later cases and final `after.json`, so this attempt does not complete placement
acceptance or establish the selected reset body’s behavior.

## Current nested-group preparation path

`infra/metal/test-placement.json` assigns this test identity to
`guest-group` / `nested-boundary`. The existing nextest guest plan calls
`nested_vm_test_support`, which stages `NESTED_DEVICE_MODULES` and records
their guest paths (`crates/overdrive-testing/bin/fwd_guest_suite.rs:1994-2000,
3426-3430`). That list includes the guest KVM and vhost-vsock modules.

The selected attempt then calls `prepare_nested_attempt` before running the
selected ELF (`fwd_guest_suite.rs:3519-3520, 3635-3636`). That function loads
XFS for the reflink setup, mounts the filesystem and prepares cgroup mounts,
then runs `chmod /dev/kvm /dev/vhost-vsock` at line 3293. It does not load
`guest.module_paths` until lines 3301-3302, after the failing chmod. The raw
boot log’s missing KVM/vhost module messages matches this ordering.

The existing batched `run_nested_boundary` path has the needed sequence: after
the XFS setup, it loads the staged nested device modules with
`guest.command_with_modules` at lines 2980-2981, then changes the two device
modes at lines 2982-2986. The minimal owner route is to make the existing
per-test `prepare_nested_attempt` follow that same staged-module-before-device
setup sequence. No kernel configuration, body relocation, skipped module
failure, or new VMM interface is indicated by this evidence.

The direct-native result remains valid at its own boundary. The nested
preparation failure is separate; this capture makes no full placement or
complete native-feature claim.
