# Findings: node-local persistent volume (GH #348) DESIGN-time spikes

Probes A to E, plus a serial-derivation probe (F), run for real on the native Metal host through `cargo xtask metal run --` (ordinary grammar only; never `--exclusive`, never Lima). Every result below is pasted from a retained run log under `evidence/`. Probe code: `spike-scratch/node-local-persistent-volume-a/`.

## 0. Setup, and what was and was not reproduced

### Substrate (recorded, not assumed)

| Item | Value | Source |
|---|---|---|
| Overdrive commit | `d1953f3f7797d7e69c5a932b55305e3415888018` (branch `marcus-sa/overdrive-shared-volumes-sccache`) | `git rev-parse HEAD` |
| Fork under test ("stock") | `vendors/cloud-hypervisor` at `f252bb20b0d141d59e4c15a9ac09c5c2dd7c350d`; the installed `/usr/local/bin/cloud-hypervisor`, sha256 `6d54fbd2611e692fda00e476fc0723e9502bfd8acd53c8bc46edd96e84f3e842`, equal to `CLOUD_HYPERVISOR_SHA256_X86_64` in `infra/provision/versions.env`; its panic paths read `/var/tmp/overdrive/cloud-hypervisor/f252bb20b0d.../source/...` | `run-026`, `run-017` |
| Fork spike branch | `spike/node-local-persistent-volume`: `4f45f59b3` (F1), `56299d314` (F3), `3c8b8eb6f` (F2), all from `f252bb20b`. Typechecked with `cargo check`, never built or run (section 7) | `run-015`, `run-020`, `run-021` |
| Metal host | AMD EPYC 8024P, 16 threads, 64 GB, Ubuntu kernel `7.0.0-38-generic`; `/srv/vm` = XFS (`reflink=1`, `noatime`) on a Dell DC NVMe PM9A3; `systemd-detect-virt` = `none`; the NVMe reports `write_cache = write through` | `run-001`, `run-026` |
| Guest kernel | The same `7.0.0-38-generic` (the guest kit boots the host's own kernel). `CONFIG_BLK_DEV_LOOP=y`, `CONFIG_BLK_DEV_DM=y` (dm-linear and the `error` target are core), `CONFIG_DM_LOG_WRITES=m`, `CONFIG_DM_FLAKEY=m`, `CONFIG_DM_THIN_PROVISIONING=m`, `CONFIG_XFS_FS=m`, `CONFIG_EXT4_FS=y`, `CONFIG_BTRFS_FS=m`, `CONFIG_VIRTIO_BLK=y`, `CONFIG_BLK_DEV_WRITE_MOUNTED=y`, `CONFIG_IO_URING=y`, `kernel.io_uring_disabled=0`, `CONFIG_FAULT_INJECTION` unset | `run-001` |
| Kernel citations | `vendors/linux` is tag `v7.0`; the running kernel is Ubuntu's 7.0.0-38 build of it. Citations are to v7.0 source; a distro delta is not excluded | |
| Outer guest | `cgroup-default` / `nested-launch`: 4 GiB RAM, 8 vCPU, 2 GiB ext4 root; the nested CH runs inside it (nested virtualization) | `run-005` |
| Box load | Other worktrees ran jobs during these runs (a 7 GB CH VM at ~400% CPU from another session was visible in `ps`); the benchmark spread includes that noise | session `ps` capture |

### How a probe gets a CH binary and files inside a guest

The `nested-launch` guest stages `cloud-hypervisor`, `prlimit`, `setpriv`, `losetup`, `mkfs.ext4`, `mount`, a static `busybox`, the XFS module and the kernel (`guest_suite.rs`, `nested_vm_test_support`). It does not stage `dmsetup`, `mkfs.xfs`, `xfs_io`, `strace`, the `dm-log-writes`/`dm-flakey`/`dm-delay`/`brd` modules, or any workload. The probe binary carries them: `build.rs`, run on the box, builds a static-musl inner-guest PID 1/workload, decompresses the four modules from `/lib/modules/7.0.0-38-generic`, and bundles the tools with their `ldd` closure. The probe unpacks them in the guest and loads modules with `init_module(2)`. **No guest-kernel config change was made or needed.**

### Confinement actually reproduced

Reproduced (from `crates/overdrive-core/src/vm/config.rs` and `crates/overdrive-host/src/vmm.rs`): `prlimit --fsize=<derived> --nofile=1024 -- setpriv --reuid=4200 --regid=4200 --groups=<gid of /dev/kvm> --no-new-privs -- cloud-hypervisor --seccomp true --landlock --landlock-rules path=<run_dir>,access=rw`; rootfs, kernel copy and run dir chowned to 4200 before launch; `MemoryPlan::derive` (guest 128 MiB, `memory.max` 136 MiB) in a cgroup under `overdrive.slice/workloads.slice`; `RLIMIT_FSIZE = max(rootfs, RAM)` for probe C. Evidence: `Uid:4200 4200 4200 4200`, worker threads `Seccomp:2`, and the full argv in `run-017` (C3).

Not reproduced: the launch seccomp filter denying 13 TUN ioctls (ADR-0143), the ADR-0129 child hook, the `--vsock` device, netns/TAP, the rootfs as a reflink clone. None touches the block path.

### Method for durability (probe A)

`dm-log-writes` sits under the data disk. It records every write that completed before a flush, then the flush entry (`drivers/md/dm-log-writes.c:56-60, 318-341, 670-790 @ v7.0`). Per acknowledged guest write `i`, the runner places a log mark `ACK<i>` when it sees `ACK i` on the serial console; the guest paces 30 ms between writes so no flush can fall between ack and mark. The checker asserts: the last FLUSH/FUA entry before mark `ACK<i>` is preceded by a log write of block `i` with the expected payload. That is replay-to-the-flush-mark. For file-on-XFS the runner replays the log onto a copy of the pre-formatted image up to that flush, mounts it (XFS journal recovery) and reads the volume file. The check has teeth: with the guest acking without any flush it reports 40 of 40 violations.

A harness defect was found and fixed, and is recorded rather than hidden: the first write-through cells (`run-009`, `a1_wt_*`) showed 60/60 violations because the guest wrote through its own page cache and printed ACK before any I/O existed. `strace` showed one coalesced `pwritev` of blocks 1 to 10 and one `fsync` (`run-013`). Write-through cells now use guest `O_DIRECT` (`run-014`); the `run-009` write-through rows are superseded.

---

## A. Durability

**Hypothesis.** A guest FLUSH becomes host `fsync` on every raw backend, so every flushed write is below a flush bio; SIGKILL does not lose an acked write.
**Predicted.** 0 violations in write-back cells (any `direct`, any backend); the negative control reports violations; write-through holds once the guest really writes through.
**Falsified by.** Any `VIOLATION` in a write-back cell, or a negative control with none.

### A1. WORKS (block device under dm-log-writes)

io_uring on the data disk is confirmed per cell from `/proc/<pid>/fd` (2 `anon_inode:[io_uring]` fds with it, 1 with `_disable_io_uring=on`). AIO vs sync is inferred from the flags, not observed.

| Cell | host `direct` | backend | guest cache | acks | log writes | log flushes | violations |
|---|---|---|---|---|---|---|---|
| `a1_wb_buffered_uring` | off | io_uring | write-back | 60 | 60 | 60 | **0** |
| `a1_wb_direct_uring` | on | io_uring | write-back | 60 | 60 | 60 | **0** |
| `a1_wb_buffered_aio` | off | AIO | write-back | 60 | 60 | 60 | **0** |
| `a1_wb_buffered_sync` | off | sync | write-back | 60 | 60 | 60 | **0** |
| `a1_wb_direct_sync` | on | sync | write-back | 60 | 60 | 60 | **0** |
| `a1_guest_odirect_wb_uring` | off | io_uring | write-back, guest `O_DIRECT` | 60 | 60 | 60 | **0** |
| `a1_wt_buffered_uring` | off | io_uring | write-through | 60 | 60 | 60 | **0** |
| `a1_wt_buffered_sync` | off | sync | write-through | 60 | 60 | 60 | **0** |
| `a1_wt_direct_uring` | on | io_uring | write-through | 60 | 60 | 60 | **0** |
| `a1_kill9_wb_buffered_uring` | off | io_uring | SIGKILL after ACK 120 | 120 | 120 | 120 | **0** |
| `a1_kill9_wb_direct_uring` | on | io_uring | SIGKILL after ACK 120 | 120 | 121 | 121 | **0** |

Sources: `run-009` (write-back, kill) and `run-014` (write-through).

```
RESULT cell=a1_wb_buffered_uring ... acks=60 violations=0 missing_marks=0
log-writes: entries=182 writes=60 flushes=60 marks=62 discards=0 sectorsize=512
RESULT cell=a1_kill9_wb_direct_uring ... acks=120 violations=0 ... extra_unacked_writes_logged=1 exit=ExitStatus(unix_wait_status(9))
RESULT cell=a1_negctl_no_flush_must_violate ... sync=none acks=40 violations=40
log-writes: entries=82 writes=40 flushes=0 marks=42
VIOLATION ACK1: block 1 NOT in the log before the last flush preceding its mark (mark entry 0)
```

Write-through shows one flush per write (`Changing cache mode to writethrough` is in the CH log, `run-013`). The extra logged write in the direct kill cell is the 121st, in flight at kill, written without an ack, which is permitted.

### A2. WORKS (file on XFS under dm-log-writes)

XFS (`reflink=1`) on the log device, a 64 MiB volume file, CH opens the file; ACKs 1, n/4, n/2, 3n/4, n sampled and replayed to their commit entry.

| Cell | file | host `direct` | cache | acks | violations |
|---|---|---|---|---|---|
| `a2_xfs_prealloc_wb_buffered_uring` | `fallocate` mode 0 | off | write-back | 40 | **0** |
| `a2_xfs_prealloc_wb_direct_uring` | `fallocate` mode 0 | on | write-back | 40 | **0** |
| `a2_xfs_sparse_wb_buffered_uring` | sparse | off | write-back | 40 | **0** |
| `a2_xfs_sparse_wt_buffered_uring` | sparse | off | write-through | 40 | **0** (`run-014`) |
| `a2_xfs_sparse_kill9_wb_buffered` | sparse | off | SIGKILL after ACK 100 | 100 | **0** |

```
ACK40: replayed to entry 181: block 40 present and correct
RESULT cell=a2_xfs_sparse_kill9_wb_buffered backing=xfs-file ... acks=100 sampled=[1, 26, 51, 76, 100] violations=0 exit=ExitStatus(unix_wait_status(9))
```

Scope: the flush reaches the block layer below the filesystem before the guest is acked. What the device below does with it (here loop on tmpfs or brd; on the box an NVMe reporting `write through`) is not shown. This is not a power-loss test.

### A3. Fsync-error propagation: gap proven on unmodified `f252bb20b`

**Hypothesis.** Write-through's post-write `fsync(None)` result is discarded on io_uring and AIO (`engine_uring.rs:81`, `engine_aio.rs:83`); the sync engine returns it and `process_queue_complete` forwards it with `?` (`virtio-devices/src/block.rs:555`).
**Injection.** Data disk is dm-linear; after ACK 10 the table is swapped to `error`. The buffered host write still lands in page cache; every flush fails EIO.
**Predicted.** Explicit flush errors reach the guest on all engines; write-through on io_uring/AIO returns success; sync errors or hangs.
**Falsified by.** Any write-through io_uring/AIO write reporting an error.

Result (`run-022`):

```
FAULTRESULT cell=a3_fault_wb_explicit_flush_sync    wt=false no_uring=true  no_aio=true  acks_after_injection=0  errs_after_injection=20 exit=ExitStatus(0)
FAULTRESULT cell=a3_fault_wb_explicit_flush_uring   wt=false no_uring=false no_aio=false acks_after_injection=0  errs_after_injection=20 exit=ExitStatus(0)
FAULTRESULT cell=a3_fault_wt_post_write_fsync_uring wt=true  no_uring=false no_aio=false acks_after_injection=20 errs_after_injection=0  exit=ExitStatus(0)
FAULTRESULT cell=a3_fault_wt_post_write_fsync_aio   wt=true  no_uring=true  no_aio=false acks_after_injection=20 errs_after_injection=0  exit=ExitStatus(0)
FAULTRESULT cell=a3_fault_wt_post_write_fsync_sync  wt=true  no_uring=true  no_aio=true  acks_after_injection=0  errs_after_injection=0  exit=killed
```

CH log, explicit flush (control): `WARN:virtio-devices/src/block.rs:384 -- Request failed: Request { request_type: Flush ...` (sync) and `...block.rs:604 ...` (io_uring); the guest saw `sync=5` for all 20.
CH log, write-through on sync: `<_disk1_q0> WARN:virtio-devices/src/block.rs:725 -- Failed to process queue (complete): Fsync(Fsync(Os { code: 5, kind: Uncategorized, message: "I/O error" }))`; then the guest gets neither ack nor error for 120 s until the runner killed the VMM.

**Verdict.** DOESN'T-WORK for write-through on the stock fork: io_uring/AIO tell the guest success for a write whose flush failed (20 of 20); sync wedges the guest. Explicit-flush error propagation WORKS on every engine. The `EOPNOTSUPP`-swallowed finding is a kernel statement (`block/fops.c:613-633 @ v7.0`) and was **not** run: no available dm target returns `BLK_STS_NOTSUPP` for a flush.

**F1** (commit `4f45f59b3`, `fork-patches/0001-...`): the io_uring/AIO engines return `AsyncIoError::Fsync` from `fsync(None)`; `process_queue_complete` completes the request with `VIRTIO_BLK_S_IOERR` instead of `?`. `cargo check` passes (`run-015`). The re-run after the change is the "A3 after F1" block below.

#### A3 after F1: CLOSES THE GAP (`run-030`, metal run `fbe0159412be36c2f06fa6f1`)

**Hypothesis.** With F1 (`4f45f59b3`, built into the fixed binary at `3c8b8eb6f`) a failed post-write fsync completes the request with `VIRTIO_BLK_S_IOERR` on every engine. **Predicted.** Write-through cells: `errs_after_injection > 0`, `acks_after_injection = 0`, no wedge on sync. Explicit-flush cells unchanged. `a1_wb_buffered_uring` and `a1_wt_buffered_uring`: 0 violations. **Falsified by.** Any write-through cell with `acks_after_injection > 0`, a sync-engine wedge, or any violation.

Stock controls and the fixed binary, same run, same cells:

```
stock a3_fault_wb_explicit_flush_sync    acks_after=0  errs_after=20 exit=0
stock a3_fault_wb_explicit_flush_uring   acks_after=0  errs_after=20 exit=0
stock a3_fault_wt_post_write_fsync_aio   acks_after=20 errs_after=0  exit=0       <- gap
stock a3_fault_wt_post_write_fsync_sync  acks_after=0  errs_after=0  exit=killed   <- wedge (139.6 s)
stock a3_fault_wt_post_write_fsync_uring acks_after=20 errs_after=0  exit=0       <- gap
spike a3_fault_wb_explicit_flush_sync    acks_after=0  errs_after=20 exit=0
spike a3_fault_wb_explicit_flush_uring   acks_after=0  errs_after=20 exit=0
spike a3_fault_wt_post_write_fsync_aio   acks_after=0  errs_after=20 exit=0
spike a3_fault_wt_post_write_fsync_sync  acks_after=0  errs_after=20 exit=0       (7.8 s, no wedge)
spike a3_fault_wt_post_write_fsync_uring acks_after=0  errs_after=20 exit=0
spike a1_wb_buffered_uring               acks=60 violations=0 missing_marks=0
spike a1_wt_buffered_uring               acks=60 violations=0 missing_marks=0
```

The stock rows reproduce `run-022` exactly (20/20 swallowed on AIO and io_uring, sync killed after the 120 s runner bound). **Verdict.** F1 closes the gap on all three engines; the explicit-flush control and both happy-path cells are unchanged. This is injection by a dm `error` table over a buffered write, i.e. the same fault class as before; it does not show behaviour on a real media error.

### A. Design implications
- Rely on write-back plus guest FLUSH; durable on every backend tested, file or block device, any `direct`, across SIGKILL.
- Do not use write-through on the stock fork (silent loss on io_uring/AIO, wedge on sync). With F1 in the fork it propagates the error on all engines (proven, `run-030`), so write-through becomes offerable once F1 is upstreamed into the fork's mainline branch.
- A guest `IOERR` on flush is the only fault signal the platform gets; treat it as a volume fault.

---

## B. Single writer

**Hypothesis.** The OFD lock stops a second CH on one inode, is released on the holder's last close, and stops nobody else.
**Falsified by.** A second VMM booting on the same path; a refused plain writer; a lock outliving the process after its I/O is done.

### B1. Second VMM while the first runs: WORKS (same inode)

```
first VMM pid 194 holds lock: Some("type=WRITE start=0 len=67108864 pid=-1")
B1 RESULT b1_bdev_default_lock ... second_booted=false :: exited_early=Some(1) stderr=[... block.rs:1032 -- Can't get Write lock for /dev/mapper/... as there is already a ExclusiveWrite lock
B1 RESULT b1_bdev_full_lock / b1_file_default_lock / b1_file_full_lock ... second_booted=false
first VMM still running: true
```

`run-016`. The lock is absent before any VMM, present while one runs, absent after SIGKILL (`b1b_lock_is_taken_at_boot`). Hardlink and symlink aliases of a file are refused (same inode).

### B2. When is the lock released?

`F_OFD_GETLK` and `waitpid` polled every 0.5 ms (`LOCKTIMELINE`, `run-016`):

| Case | lock gone | process exit observed | replacement boots |
|---|---|---|---|
| clean exit, bdev | 23 ms after DONE | 44 ms | yes |
| clean exit, file | 23 ms | 36 ms | yes |
| SIGKILL idle, bdev | 9 ms | 22 ms | yes |
| SIGKILL idle, file | 1 ms | 21 ms | yes |
| SIGKILL, write in flight, sync engine | 2.46 s | 2.47 s (stuck in the I/O) | n/a |
| SIGKILL, write in flight, io_uring | **2.46 s** | **28 ms** | n/a |

`dm-delay` (3000 ms) under the disk, host `direct=on`, VMM killed 600 ms into the write:

```
B2b RESULT b2_inflight_kill_uring uring=true  delay_ms=3000 process_exit_after_kill_us=Some(28613)   lock_released_after_kill_us=Some(2461732)
B2b RESULT b2_inflight_kill_sync  uring=false delay_ms=3000 process_exit_after_kill_us=Some(2472555) lock_released_after_kill_us=Some(2460010)
B2b the dead VMM's in-flight write landed on the device after the kill: true
```

With io_uring the process is gone 28 ms after the kill but the lock lives until the in-flight write completes, because the in-flight request holds the open file description (`locks_remove_file` on last `fput`, `fs/locks.c:2764-2776 @ v7.0`). A replacement's `vm.boot` fails `AlreadyLocked` in that window, which is the safe outcome. A replacement launched at the guest's own DONE booted in 6 of 6 rounds (`B2c`, `run-025`).

### B3. Bypasses

```
B3 plain_writer bdev=false/true: lock_while_vmm_runs=Some("type=WRITE ...") open_rw=Ok("ok") write=Ok(Ok(8))
B3 ALIAS bdev second node (same major:minor): second_booted=true :: ready=true exited_early=None stderr=[]
both VMMs alive: first=true
B3 O_EXCL: first O_EXCL open Ok; second O_EXCL via ALIAS node Err("Device or resource busy (os error 16)"); plain rw open via alias Ok
B3 dmsetup remove while an fd is open: exit 1
```

**Verdict.** WORKS between CH instances on one inode. DOESN'T-WORK against any non-CH opener (advisory) and against a second device node for the same block device: two VMMs ran on one disk (the lock is keyed on the node's inode, `fs/locks.c:2771 @ v7.0`). The kernel claim CH never takes arbitrates per device: a second `O_EXCL` open through the alias gets `EBUSY` (`block/bdev.c:598-611, 949-957 @ v7.0`), measured with a probe-side `open`, not through CH. `O_EXCL` excludes only other claimants; a plain rw open through the alias still succeeds (`CONFIG_BLK_DEV_WRITE_MOUNTED=y`, `block/bdev.c:878-888`).

**F2** (commit `3c8b8eb6f`, `fork-patches/0003-...`): open writable block devices with `O_EXCL`. Typechecked (`run-021`); the after-fix run is below.

#### B after F2: CLOSES THE GAP, and changes the error a replacement sees (`run-031`, metal run `e95a1c5d3d3671df7feb5141`)

**Hypothesis.** With F2 (`3c8b8eb6f`) CH opens a writable block device `O_EXCL`, so the kernel refuses a second claimant whatever node names the device. **Predicted.** `b3_alias_second_vmm`: the second VMM fails to boot; B1 on a block device: still refused; B2 clean-exit and SIGKILL-idle: replacement boots; B2b: a replacement inside the in-flight window is refused, one after the drain boots. **Falsified by.** The alias VMM booting, or any replacement failing after release. Both the first and second VMM were the fixed binary (`O_EXCL` only excludes other claimants, so a stock opener is not stopped; Overdrive runs only the fork).

```
stock b3_alias_second_vmm:  B3 ALIAS bdev second node ... second_booted=true :: ready=true exited_early=None stderr=[]        <- gap
spike b3_alias_second_vmm:  second_booted=false :: Fatal error: VmBoot(VmBoot(DeviceManager(Disk(BlockError { kind: Io,
       source: Some(Os { code: 16, kind: ResourceBusy, message: "Resource busy" }),
       ctx: Some(ErrorContext { path: Some("/dev/mapper/alias-of-vol"), offset: None, op: Some(Open) }) }))))
spike b1_bdev_default_lock:  second_booted=false :: ... Os { code: 16, kind: ResourceBusy ... }
spike b1_bdev_full_lock:     second_booted=false :: ... Os { code: 16, kind: ResourceBusy ... }
spike b1_file_default_lock:  second_booted=false :: Can't get Write lock for /nlpv/spike_b1_file_default_lock.vol as there is already a ExclusiveWrite lock
spike b2_clean_exit_bdev / _file, b2_sigkill_bdev / _file: replacement_booted=true (4 of 4)
```

Two consequences for the design. (1) For a block device the refusal is now `EBUSY` from `open` (`VmBoot(DeviceManager(Disk(BlockError{kind: Io, ... code 16 ...})))`), raised before the lock, even for the same node; a file volume still fails with the lock text (`AlreadyLocked`). A replacement that retries "while `AlreadyLocked`" would stop retrying on a block device. (2) `EBUSY` also means any other claimant of that device (a mounted filesystem, a dm holder), and the message does not say which.

**B2b: does the replacement hit `EBUSY` while the dead VMM's I/O drains? Yes.** `dm-delay` 3000 ms under the disk, `direct=on`, VMM killed 600 ms into the write; replacement #1 started at once, #2 after the lock cleared:

```
uring=true : #1 (attempt ended 100423us after kill) booted=false :: ... Os { code: 16, kind: ResourceBusy ... path: "/dev/mapper/slow0" ... op: Open
             lock after attempt 1: Some("type=WRITE ...") ; process exited=Some(ExitStatus(unix_wait_status(9)))   <- process already dead
             lock released 2413932us after kill ; #2 (after the drain) booted=true
uring=false: #1 (attempt ended 97480us after kill) booted=false :: ... ResourceBusy ... ; process exited=None
             lock released 2418851us after kill ; #2 booted=true
```

With io_uring the process is gone but the in-flight request still holds the open file, so the claim (like the lock, 2.41 s here against 2.46 s on stock) outlives the process; CH reports `ResourceBusy` on `Open`, the replacement exits 1 at about 6 ms into its own start, and a retry after the drain boots. This is the safe outcome. Not measured: the order in which the claim and the lock are released inside the drain; the replacement booted at the first attempt after the lock cleared.

### B4. Hot-remove releases the lock

```
bdev=false: before remove: lock=Some("type=WRITE ...") fds_on_disk=2
remove-device response: HTTP/1.1 204
bdev=false: after remove-device: lock=None fds_on_disk=0
B4 RESULT bdev=false: replacement booted while first VMM still alive after remove-device: true   (same for bdev=true)
```

`run-017`. Overdrive restarts rather than hot-removes, so this is background.

### B. Design implications
- The lock is real and cheap between CH instances; both `lock_granularity` values refuse a second boot.
- A block-device volume must be reachable through one node only, or CH needs F2. Overdrive creates the node, so one node per volume plus F2 closes the hole. File aliases (hardlink, symlink) are covered.
- A replacement must not read VMM process exit as "volume free". Treat `AlreadyLocked` at `vm.boot` as "still held, retry within a bound"; the bound is the worst in-flight I/O on the backing device.
- The lock is advisory: Overdrive's own tools (format, grow, delete, GC) must hold it or be sequenced by the owner.

---

## C. Confinement

### C1. `RLIMIT_FSIZE`

Overdrive's derivation `max(rootfs 24 MiB, RAM 128 MiB)` = 128 MiB; 256 MiB volume; guest writes 1 MiB `O_DIRECT` blocks from offset 0 (`run-017`).

| Volume | backend | Result |
|---|---|---|
| file | sync | VMM killed by SIGXFSZ (25) at guest progress 128 MiB |
| file | AIO | VMM killed by SIGXFSZ (25) at 128 MiB |
| file | io_uring | VMM survives; guest sees `errno=5` at `off=134217728` (exactly the limit), 6 errors, exit 0 |
| block device | io_uring | 256 of 256 MiB, `errors=0`, no signal |
| file, limit 512 MiB (control) | io_uring | 256 of 256 MiB, `errors=0` |

```
C1 RESULT c1_file_aio  ... killed_by_signal=Some(25) guest_last_progress_MiB=128 guest_reports=[]
C1 RESULT c1_file_sync ... killed_by_signal=Some(25) guest_last_progress_MiB=128
guest: FIRSTERR write at off=134217728 i=128: errno=5 I/O error (os error 5)     (c1_file_uring)
guest: FILLEND blocks_attempted=256 errors=0                                     (c1_bdev_uring)
```

(`c1_file_uring` printed `Max file size unlimited` because the launcher sampled the process before `prlimit` had exec'd; the other rows show `134217728`.) **Verdict.** A file volume larger than the limit DOESN'T-WORK, and how it fails depends on a backend CH chooses by runtime probe. A block device is unaffected (`block/fops.c:746-783 @ v7.0`).

### C2. Page cache and `memory.max`

Guest RAM 128 MiB, `memory.max` 136 MiB (`MemoryPlan::derive`: 128 + 8 MiB reserve), 256 MiB volume on a real-disk path, guest `O_DIRECT` writes, flush every 8 MiB.

| Volume | host `direct` | elapsed | `memory.events max` | peak cgroup page cache | OOM kill |
|---|---|---|---|---|---|
| file | off | 15.8 s | 1367 | 23 MiB | 0 |
| file | on | 2.76 s | 494 | 20 MiB | 0 |
| block device | off | 5.78 s | 1835 | 23 MiB | 0 |
| block device | on | 2.77 s | 1415 | 21 MiB | 0 |

```
C2 t=15708ms PROGRESS 256: cg memory.current=136 MiB file=23 MiB file_dirty=8 MiB file_writeback=0 MiB anon=110 MiB
low 0 / high 0 / max 1367 / oom 0 / oom_kill 0
```

**Verdict.** The VMM's buffered volume writes are charged to the allocation cgroup. Near the limit the effect is reclaim stalls, not an OOM kill: buffered is 5.7x slower for the file and 2.1x for the block device than `direct=on`. `max` events are also high with `direct=on` because anonymous guest memory alone reaches the limit with an 8 MiB reserve. Scope: one sequential workload; a dirty rate above writeback was not run, so the OOM boundary is unmapped.

### C3. Second writable disk under uid drop, landlock, seccomp: WORKS

Every probe A cell wrote and flushed through the data disk as a second `--disk` under uid 4200, `--seccomp true`, `--landlock` with the run-dir rule; CH derives the disk's landlock grant itself. Negative cases (`run-017`):

```
C3 file volume root:root 0600 -> VMM(uid 4200) boots=false; ... PermissionDenied ... path: Some("/nlpv/c3_perms.vol") ... op: Some(Open)
C3 block device node root:root 0600 -> ... PermissionDenied ... path: Some("/dev/mapper/...")
```

The platform must chown the file (or set the node mode) to the confined uid before launch, as it already does for the rootfs clone.

---

## D. Benchmark (backing format)

Required by `design.md` before choosing a backing. Every number is measured **inside a guest under nested virtualization** (metal host, outer guest, inner guest, loop and `dm`) with other jobs on the box. Absolute values are distorted and say nothing about production speed. The ranking, not the magnitude, is the result. Nesting could change a ranking on bare hardware; it does not change within these two substrates.

Method: inner guest 512 MiB RAM, 2 vCPU, confined VMM; ext4 on the 256 MiB data disk; four workloads, six iterations per boot, iteration 1 discarded; two independent runs (`run-019`, `run-023`) pooled, **n = 10 per cell**. Substrates: `brd` (RAM disk) and `disk` (loop over a file on the outer guest's ext4 root, itself a virtio-blk file on the host NVMe). Backings: file on XFS (`fallocate` mode 0 "prealloc", or sparse) vs `dm-linear` (what a plain LV is). Workloads: `smallfiles_nofsync` (1500 x 8 KiB files then one `sync`), `smallfiles_fsync` (fsync per file), `randsync` (1500 random 4 KiB overwrites each followed by `fdatasync`, QD1), `tree` (600 files 2 KiB to 512 KiB, 4 writer threads, fsync every 10th, `sync`, cold read). A real `cargo build` in the guest was **not** run: the inner guest holds only the probe's static init, and `nested-launch-rustc` stages `rustc` and musl libs but not `cargo`.

Medians in ms (min..max and n in `evidence/bench-table.md`; spread is up to 2x within a cell); `randsync` also gives median p50/p99 per op in microseconds.

| substrate | backing | direct | smallfiles_nofsync | smallfiles_fsync | randsync (ms; p50/p99 us) | tree |
|---|---|---|---|---|---|---|
| brd | **dm-linear** | off | 95 | **1412** | **531**; 198/2261 | 450 |
| brd | **dm-linear** | on | 80 | **1510** | **302**; 138/1100 | 406 |
| brd | XFS prealloc | off | 98 | 3537 | 954; 489/2484 | 464 |
| brd | XFS prealloc | on | 94 | 3528 | 975; 513/2696 | 484 |
| brd | XFS sparse | off | 90 | 3179 | 1360; 786/3236 | 572 |
| brd | XFS sparse | on | 88 | 3941 | 1120; 623/2476 | 562 |
| disk | **dm-linear** | off | 120 | **7708** | **2971**; 1270/8106 | 646 |
| disk | **dm-linear** | on | 112 | **7450** | **2095**; 874/7086 | 808 |
| disk | XFS prealloc | off | 110 | 12002 | 3932; 1566/11304 | 808 |
| disk | XFS prealloc | on | 118 | 16280 | 6958; 3681/13444 | 1261 |
| disk | XFS sparse | off | 110 | 10872 | 2666; 1300/7288 | 638 |
| disk | XFS sparse | on | 114 | 9743 | 4522; 1830/12010 | 842 |

What the data supports:
- **fsync-bound workloads.** The block device beat a file in every comparison on both substrates, by 1.2x to 2.5x on medians (brd `smallfiles_fsync` 1412 vs 3179–3941 ms; `randsync` p50 198 vs 489–786 us). Likely mechanism, not measured here: a guest flush on a file costs an XFS journal force plus a device flush; on a block device it is one flush bio.
- **Bulk and metadata workloads** (`smallfiles_nofsync`, `tree`): no ranking beyond noise.
- **Prealloc vs sparse.** No consistent difference; prealloc is not faster and is often slower on `disk`. The first-touch unwritten-extent cost the research predicted is not visible at this resolution.
- **`direct=on` vs off.** Mixed and within spread, except a consistent gain for the block device on `randsync` (531 to 302 ms on brd). It does not change durability (A) and avoids the page-cache charge (C2).

---

## E. Edge cases

### E1. ENOSPC on a sparse file: guest gets EIO mid-flight, VMM survives

512 MiB sparse volume on a 320 MiB XFS; guest writes sequentially (`run-017`):

```
guest: FIRSTERR write at off=245366784 i=234: errno=5 I/O error (os error 5)
guest: FILLEND blocks_attempted=239 errors=6
E1 RESULT e1_enospc_{buffered_sync,buffered_uring,direct_uring}: ... READBACK_OK ... vmm_exit=Some(ExitStatus(0))
E1b fallocate(512MiB) on a 320MiB XFS: fallocate ...vol.img: No space left on device (os error 28)
```

Identical on sync, io_uring buffered and io_uring direct. The volume was "created" at 512 MiB and the failure arrived 234 MiB in, as EIO on a guest write. The VMM did not die and the guest could still read. Mode-0 `fallocate` fails at creation instead.

### E2. Guest discard with `sparse=true` (CH default): hole punched, reservation returned

```
E2 e2_discard_file_sparse_default: host allocated before=67108864 after=50335744 (delta 16773120); guest: discard_max_bytes="2199023255040", BLKDISCARD ret=0, READBACK_ZERO=Ok(true)
E2 e2_discard_bdev_sparse_default: before=67108864 after=50331648 (delta 16777216)     (dm-linear over loop over a file)
E2 e2_discard_file_sparse_off:     before=67108864 after=67112960; discard_max_bytes="0", BLKDISCARD ret=-1 err=Not supported (os error 95)
```

A 16 MiB guest discard of a 64 MiB preallocated file freed 16 MiB on the host. `sparse=off` does not offer discard and keeps the space. The `sparse=off` open-time `fallocate` cost was not measured.

### E3. Guest-written magic bricks the next boot (explicit `image_type=raw`): DOESN'T-WORK

```
e3_qcow2_magic_file: second boot reached the guest=false; BlockError { kind: InvalidFormat, source: Some(InvalidClusterSize) ... op: Some(Open) }
e3_qcow2_magic_bdev: ... InvalidFormat ... InvalidClusterSize
e3_vhdx_magic_file : ... ParseVhdxHeader(InvalidHeaderSign)
e3_vhd_footer_file : ... DiskImageTypeMismatch { specified: Raw, detected: FixedVhd }
```

A guest with write access to its volume can make it unbootable, for a file or a block device. Only the fixed-VHD footer takes the `DiskImageTypeMismatch` path the research named; qcow2 and VHDX markers fail inside the format parser because `open_disk` sniffs before the type check. **F3** (commit `56299d314`, `0002-...`): `DiskOpenOptions.image_type`; an explicit type skips the sniff. Typechecked (`run-020`); the after-fix run is below.

#### E3 after F3: CLOSES THE GAP (`run-032`, metal run `542652b3176f2229f81ed266`)

**Hypothesis.** With F3 (`56299d314`) an explicit `image_type=raw` skips the sniff. **Predicted.** The second boot reaches the guest and the guest reads back its own marker for qcow2 (file, bdev), VHDX and fixed VHD (file, bdev); with no `image_type`, detection is unchanged. **Falsified by.** Any explicit-type second boot failing, or the untyped boot behaving differently from stock. Boot 1 (explicit raw) writes the marker; boot 2 reads the head and tail 512 bytes back, once per binary on the same volume (`MAGICREAD` is a new guest mode that reports which marker the guest sees).

```
cell                 image_type=raw stock                                  image_type=raw spike
qcow2 file           boot failed: InvalidFormat/InvalidClusterSize          guest reached; MAGICREAD ... qcow2_magic=true
qcow2 bdev           boot failed: InvalidFormat/InvalidClusterSize          guest reached; qcow2_magic=true
vhdx file            boot failed: ParseVhdxHeader(InvalidHeaderSign)        guest reached; vhdx_magic=true
vhd footer file      boot failed: DiskImageTypeMismatch{Raw, FixedVhd}     guest reached; vhd_footer=true  size=67108864
vhd footer bdev      boot failed: DiskImageTypeMismatch{Raw, FixedVhd}     guest reached; vhd_footer=true  size=67108864
```

**No `image_type` (F3 must not change this).** Stock and spike behave identically in every cell:

```
qcow2 magic, file and bdev, NO image_type: stock and spike both fail: BlockError{kind: InvalidFormat, source: InvalidClusterSize}
vhdx magic, file, NO image_type:           stock and spike both fail: ParseVhdxHeader(InvalidHeaderSign)
clean raw file, NO image_type:             stock and spike both boot, both print "DEPRECATION: auto-detection of disk image type is deprecated"
fixed-VHD footer, NO image_type (file and bdev): stock and spike both boot, but the guest sees size=67108352 (512 bytes less than the 67108864 it had), vhd_footer=false
```

The last line is a new finding, unrelated to F3: a volume whose last 512 bytes look like a fixed-VHD footer is silently reopened as a VHD 512 bytes smaller when `image_type` is omitted. Another reason to always pass `image_type=raw`. **Verdict.** F3 closes the gap for qcow2, VHDX and fixed VHD, on a file and on a block device, and leaves detection unchanged where no type is given.

### F. Serial derivation (`run-024`)

```
explicit serial=nlpv-vol-0001            : guest /sys/block/vdb/serial = "nlpv-vol-0001"
default, file (st_dev=29 st_ino=4)       : "2904" (twice, identical)
default, copy of the file (st_ino=56)    : "29056"   (changes when the volume is re-created)
default, block device (7 / 64512 / 156)  : "764512156"
serial=0123456789012345678901234567890 (31 bytes): VMM refused at launch
```

The default is the decimal concatenation of `st_dev`, `st_rdev`, `st_ino`: stable only while one inode lives on one host boot. An explicit `serial=` of at most 20 bytes is read back by the guest.

---

## 6. Design implications by DESIGN decision

| Decision | What the spikes show | Evidence |
|---|---|---|
| Backing (file vs LV) | Durability is equal. The block device is faster for fsync-bound loads (1.2x to 2.5x median, both substrates), is exempt from `RLIMIT_FSIZE`, cannot ENOSPC mid-flight, and dm refuses to remove an open device. Costs: dm in the image (`CONFIG_BLK_DEV_DM=y` in this kernel; the appliance kernel config is not in the repo), a disk region outside the reflink filesystem, a node to create and chown, and the alias hole (closed by F2, proven on the fixed binary). A file keeps the existing filesystem and its aliases are covered by the lock, but needs a derived `RLIMIT_FSIZE`, and has mid-flight ENOSPC | A, B3, C1, D, E1 |
| Space model | Sparse: no up-front cost; ENOSPC arrives as guest EIO mid-flight. Reserved (`fallocate`): fails at creation, but `sparse=true` (CH default) lets a guest discard return the reservation, so holding it needs `sparse=off`. Preallocation bought no throughput | D, E1, E2 |
| Cache mode and `direct` | Write-back plus guest FLUSH is durable everywhere. Write-through is unsafe on the stock fork; with F1 it propagates the error on all engines (proven). `direct=on` removes the page-cache charge that throttles a buffered volume 2.1x to 5.7x under `memory.max`, does not change durability, and is neutral to mixed on throughput | A, C2, D |
| Replacement vs a held volume | The OFD lock is the arbiter, released on last close, not at process exit; with io_uring it outlived the process by 2.4 s. Treat `AlreadyLocked` (file) and, with F2, `EBUSY` on the disk `Open` (block device) at `vm.boot` as "still held, retry within a bound"; the `EBUSY` text does not distinguish another claimant from a draining predecessor. The lock is advisory and keyed on the node inode; F2's `O_EXCL` closes the alias hole for CH openers (all openers must use it), and Overdrive's own tools must take the same lock | B1 to B4 |
| `RLIMIT_FSIZE` derivation | Must be at least the largest file volume the VMM writes. Below it: SIGXFSZ kills the VMM (sync, AIO) or the guest gets EIO (io_uring), chosen by a runtime probe the platform does not control. Block-device volumes ignore it | C1 |
| Image-type sniffing | Guest-controlled bytes at the start (end for VHD) of the data disk make the next boot fail, for files and block devices. F3 removes the sniff when the type is explicit (proven); without it there is no recovery from inside the guest's own data. Omitting `image_type` keeps the sniff, and a fixed-VHD footer then shrinks the disk by 512 bytes | E3 |
| Serial derivation | Always pass an explicit `serial=` of at most 20 bytes; the default changes with the inode and the host | F |

## 7. What remains unverified

1. **F1, F2 and F3 were built and run** (fixed binary `3c8b8eb6f`, `run-030` to `run-032`): each closes its gap. Still unproven: F1 on a real media error rather than a dm `error` table; F2 against a non-fork opener (a plain `open` through an alias still succeeds, by design) and a stock VMM sharing the host with fork VMMs (a stock opener takes no claim and is not stopped); the order in which the claim and the lock are released during an in-flight drain; the fixed binary is one static-musl release build while the stock control is a different build.
2. Host power-loss durability (`dm-log-writes` shows block-layer ordering only).
3. `EOPNOTSUPP` swallowing in `blkdev_fsync` (source-read only).
4. `vm.resize-disk` on a file above `RLIMIT_FSIZE`; growth versus the byte-range lock range.
5. The page-cache OOM boundary under a dirty rate above writeback.
6. AIO vs sync is inferred from the disable flags.
7. A real `cargo build` in the guest; the `target/` workload is synthetic.
8. Ranking on bare metal without co-tenants.
9. Cost of `sparse=off`; a btrfs or ext4 data filesystem (only XFS used).
10. The launch seccomp filter and child hook were not reproduced.

## 8. Evidence index

`evidence/`, each log starts with `OVERDRIVE_METAL_RUN_ID=`:

| Log | Run id | Content |
|---|---|---|
| `run-001-recon-host.log` | `293835f9bc51...` | host facts |
| `run-005-recon-guest.log` | `7c52ea9e588b...` | outer-guest facts |
| `run-006` to `run-008` | `29989eec...`, `ba3c77ad...`, `8bbda05e...` | first A1 runs (device-node bug, then pass) |
| `run-009-a-matrix1.log` | `c4e6db73ce88...` | A1/A2 write-back matrix, kill9, negative control (write-through rows superseded) |
| `run-013-wt-diag.log` | `1de6696f83c8...` | strace and CH log that exposed the write-through harness defect |
| `run-014-wt-rerun.log` | `972f7a0571ff...` | corrected write-through cells |
| `run-016-b-first.collect.log` | `33bf351dea7a...` | B1 to B3 (b4 hung on an API read with no timeout, was stopped, fixed) |
| `run-017-b4-c-e.log` | `17bad9dcc527...` | B4, C1, C2, C3, E1, E2, E3 |
| `run-019-d-all.log`, `run-023-d-all-2.log` | `1015c7e2d32d...`, `757e91844b6f...` | D benchmark, two runs |
| `run-022-a3-faults.log` | `9cac2341d1b6...` | A3 fault injection with CH logs |
| `run-024-b2c-serial.log`, `run-025-b2c.log` | `7d48e51ce91c...`, `2200e0540a4f...` | F, B2c |
| `run-015`, `run-020`, `run-021` | | `cargo check` of the three fork patches |
| `run-030-f1-a3-a1.log` | `fbe0159412be...` | F1: A3 fault cells stock vs spike, spike A1 wb/wt cells |
| `run-031-f2-b.log` | `e95a1c5d3d3671...` | F2: B1, B2, B2b, B3 on the spike binary (B3 alias with stock control) |
| `run-032-f3-e3.log` | `542652b3176f...` | F3: E3 cells stock vs spike, with and without `image_type` |
| `run-028`, `run-029` | `54788cd25ce0...`, `10b17fe38c8a...` | `bpf-build`, `guest-kit-prepare` for the `nlpv-spike` run directory |
| `run-027-f1-a3-a1.log` | `9378e42303e5...` | failed attempt kept: guest kit not prepared in the new run directory (`OverdriveInitBuildFailed`) |
| `run-003`, `run-004`, `run-011`, `run-012` | | `bpf-build` and `guest-kit-prepare` prerequisites (two checkouts) |
| `run-002`, `run-007`, `run-010` | | failed attempts kept (missing guest-kit prerequisite; checkout lock timeout) |
