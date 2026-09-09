# Increment A: guest Exec control

Throwaway, self-contained native-metal probe for H1–H5 of the
`service-kind-vm-workloads` design evidence gate. Nothing under this directory
is production code or a test tier.

## What it exercises

- real Cloud Hypervisor and the production kernel/rootfs artifacts;
- the existing guest-initiated Beacon connection kept alive concurrently with
  a distinct guest-initiated control connection;
- correlated concurrent probe execution, timeout/process-tree cleanup,
  bounded admission, disconnect cleanup, no replay, and reconnect;
- an exploratory persistent PID 1 installed only into a private rootfs clone.

The ASCII framing, port 1235, capacity three, and process-group implementation
are provisional probe mechanisms, not accepted product contracts.

## Prerequisites

- Repository-root `.env` contains `OVERDRIVE_METAL_TARGET`.
- The target passes `cargo xtask metal`'s native `x86_64`/KVM preflight.
- `/srv/vm/overdrive-testing/kernel` and
  `/srv/vm/overdrive-testing/rootfs.ext4` exist and are readable on the target.
- Local `/opt/homebrew/bin/rsync`, `ssh`, Cargo, and Perl are available.
- The target has the Rust musl target, Cloud Hypervisor, Python 3, `losetup`,
  and mount privileges installed by the repository metal bootstrap.

## Run and capture

From the repository root, choose a new zero-padded run ID. The capture script
refuses to overwrite an existing run.

```sh
bash spike-scratch/service-kind-vm-workloads/increment-a-guest-exec-control/capture.sh 0004
```

The expanded sanitized command is recorded in the run metadata. It builds only
this standalone manifest for the guest fixture, then executes `run.sh` under
the canonical metal lease. `rsync-without-local-env.sh` excludes the local
`.env`, all feature spike `target/` and `out/` directories, and a stale ignored
pre-remediation target directory.

Expected result lines:

```text
H1 PASS beacon=ready distinct_control=session-1 vmm=alive
H2 PASS correlated=3 exit0=1 exit7=1 signal15=1 primary=alive
H3 PASS timeout=bounded ... process_group_residual=0 primary=alive vmm=alive
H4 PASS accepted=3 overloaded=1 ... active_residual=0 provisional_capacity=3
H5 PASS ambiguous_count=1 replay=0 reconnect=session-2 ... fresh_tick_count=2 cleanup=bounded
H1_H5_COMPLETE owned_vmm_residual=0
```

`runs/<id>.stdout` and `runs/<id>.stderr` preserve command output byte-for-byte
except that the configured metal target and host are replaced with explicit
redaction markers. `runs/<id>.meta` records timestamps, exit code, sanitized
command, and capture hashes.

## Cleanup

`run.sh` marker-binds cleanup to its private directory, terminates only VMMs
whose command line contains that directory, unmounts/detaches its rootfs, and
removes the directory. A passing run ends with `owned_vmm_residual=0`. If the
outer command is interrupted, rerun only after confirming no process command
line contains `/srv/vm/overdrive-testing/service-vm-h1-h5.`.
