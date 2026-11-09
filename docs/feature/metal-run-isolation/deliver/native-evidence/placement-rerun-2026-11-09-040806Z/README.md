# Placement journey — direct native VMM peer did not connect

This directory preserves 91 raw files copied byte-for-byte from
`/tmp/metal-native-remaining-20261109T040806Z-placement`. The foreground
invocation record reports placement-driver exit 1, source HEAD
`be7d50659adc8b4c6808b782c2207d87647cc002`, and the dirty source status in
`foreground-invocation.json`. The direct native run ID is
`57bd9829da0b29fefbb3ea19`; its retained status is 100 although the driver
expected 0. The driver stopped at this case. It did not run later native
journeys or write the final `after.json` physical snapshot.

## Earlier placement cases

The driver reached the direct native case after its existing selected attempts
and failure cases returned their expected statuses: `attempts`, timeout,
guest-execution-failure, and both `default` and `ci` profiles returned 100;
the two repeat collections also returned 100. The external foreign-fixture
setup and cleanup both returned 0. Their raw stdout, stderr, and status files
are preserved. These results do not substitute for the failed native case.

## Direct native failure

The exact selected command was:

```text
cargo nextest run -p overdrive-host --test integration --features integration-tests,kvm-tests -E 'test(integration::vsock_vmm_device::the_fork_boots_and_passes_data_under_the_production_launch_identity)' --no-capture
```

Nextest failed `integration::vsock_vmm_device::the_fork_boots_and_passes_data_under_the_production_launch_identity`
after 60.806 seconds. The panic is at
`crates/overdrive-host/tests/integration/vsock_vmm_device.rs:437`:

```text
the guest dials the host over the fork-opened device: NeverConnected { port: 3740592829, bound: 60s }
```

The test reached its `CloudHypervisorVmm::create` return and then waited for
the guest’s vsock connection. It failed before the identity, supplementary
group, open-device, filter, and data-round-trip receipts later in that test.
The current raw capture does not contain a direct-native owner snapshot, VMM
argv, `VmmExit.stderr_tail`, serial console, or `lstat` results for the run
directory ancestors. The durable owner did clean its invocation roots, so
those runtime artifacts are no longer available. The current capture therefore
establishes the failed peer connection but does not itself record the child’s
exact exit or errno.

## Reachable fixture path and retained parent evidence

The failing test calls `default_staging_root()` and `WorkDir::create()` at
`vsock_vmm_device.rs:409-411`; `WorkDir::create` prepares its returned work
directory with GID 4200 and mode 0710 (`crates/overdrive-testing/src/guest_kit.rs:1207-1225`).
The test then sets `run_root` to `work.path()/run` and
`launch_config_on` calls `create_dir_all(config.run_dir.path())`
(`vsock_vmm_device.rs:113-148`). `VmRunDir::for_alloc` adds the allocation
directory under that parent (`crates/overdrive-core/src/vm/config.rs:798-804`).
`CloudHypervisorVmm::create` chowns the per-allocation run directory and its
entries, but does not prepare the intermediate `run_root`
(`crates/overdrive-host/src/vmm.rs:486-509, 734-777`). The current fixture
caller contains no explicit GID 4200/mode 0710 preparation for that
intermediate parent.

This is the precise test-support path implicated by the accepted design’s
bounded native-run-parent fallout, which names
`vsock_vmm_device.rs` and `vmm_equivalence.rs` and requires GID 4200/mode 0710
parent traversal (`feature-delta.md:238`). The current run did not preserve the
parent stat, so it cannot prove which mode or group the direct run actually
used.

The retained earlier native evidence corroborates this path, but is separate
from this current capture. `spike/native-increment-l.log:61-68` records the
same test’s `NeverConnected` result, Cloud Hypervisor exit 1 with
`CreateApiServerSocket` permission denied, and an intermediate `run` parent
owned by `root:root` at mode 0750 while the per-allocation child was GID 4200.
`spike/native-increment-n.log:34, 50-54` records a qualified run parent at
`root:4200` mode 0710 and a passing run with UID/GID 4200, device group 991,
one open vhost-vsock descriptor, and a successful data round trip. Those
earlier logs support the parent-traversal diagnosis; they do not supply the
missing current-run `lstat` or child stderr.

The minimal owner route is the existing native test-support parent constructor
in `vsock_vmm_device.rs` (and its designed sibling in `vmm_equivalence.rs`):
prepare the relevant `run_root` parent for the already-approved VMM identity
before launching, without changing the VMM identity, backend, guest size,
confinement, or data oracle. If a follow-up must distinguish another cause,
the acceptance author needs to retain only this invocation’s run-parent
ancestor stats and the existing VMM stderr tail before cleanup. No production
lifecycle hook or networking redesign is indicated by this evidence.

The direct run’s owner metadata was not retained; its run ID and result come
from `direct-native.stderr` and `direct-native.status`. The foreground source
commit and dirty status come from the invocation record. The timeout owner’s
separately retained source digest must not be attributed to this direct native
run.
