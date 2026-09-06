# Increment B: host-to-guest network probe

Throwaway, self-contained native-metal probe for H6 of the
`service-kind-vm-workloads` design evidence gate. The standalone fixture has no
dependency on an Overdrive workspace crate. `run.sh` drives the built product
binary externally because the hypothesis is specifically about the current
production VM and network composition.

## What it exercises

- a TCP listener fixture injected into a private clone of the production
  rootfs while retaining production `overdrive-init`;
- the built default-feature Overdrive binary, `VmDriver`, network provisioner,
  workload address assignment, deploy/describe/stop path, and resource cleanup;
- root-host TCP reachability to the VM `workload_addr` through the allocation
  host-veth, network namespace, TAP, and guest interface;
- independent TAP capture of the TCP SYN.

## Prerequisites

- Repository-root `.env` contains `OVERDRIVE_METAL_TARGET`.
- The target passes `cargo xtask metal`'s native `x86_64`/KVM preflight.
- `/srv/vm/overdrive-testing/kernel` and
  `/srv/vm/overdrive-testing/rootfs.ext4` exist and are readable on the target.
- Local `/opt/homebrew/bin/rsync`, `ssh`, Cargo, and Perl are available.
- The target has the repository's Rust toolchains/targets, Cloud Hypervisor,
  Python 3, `ip`, `tcpdump`, `losetup`, and required root privileges.

## Run and capture

From the repository root, choose a new zero-padded run ID. The capture script
refuses to overwrite an existing run.

```sh
bash spike-scratch/service-kind-vm-workloads/increment-b-host-network-probe/capture.sh 0003
```

The expanded sanitized command is recorded in the run metadata. It builds only
the standalone guest fixture from this increment, then `run.sh` builds the
normal product/BPF artifacts and drives the product as an external
system-under-test. `rsync-without-local-env.sh` prevents `.env` and build output
from being synchronized.

Expected result lines:

```text
H6 PASS source_namespace=host target=guest_workload_addr ... response=H6-GUEST-OK
H6_WIRE PASS tap_capture_syn=observed
H6_CLEANUP PASS owned_vmm=0 netns_delta=0 scope_delta=0 run_dir_delta=0
```

`runs/<id>.stdout` and `runs/<id>.stderr` preserve command output byte-for-byte
except that the configured metal target and host are replaced with explicit
redaction markers. `runs/<id>.meta` records timestamps, exit code, sanitized
command, and capture hashes.

## Cleanup

`run.sh` snapshots relevant resources, marker-binds private directories, stops
the deployed job through the product, and removes only the VMM, namespace,
links, scopes, key, mounts, and temporary directories attributable to its own
delta. A passing run reports zero owned/delta resources.
