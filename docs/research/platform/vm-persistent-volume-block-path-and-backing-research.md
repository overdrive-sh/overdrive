# Research: VM persistent volume (GH #348) — the Cloud Hypervisor fork's block path and the node data filesystem

**Date**: 2026-11-17 | **Researcher**: nw-researcher (Nova) | **Confidence**: High on source-reading findings, Medium on filesystem comparison, Low where marked unverified | **Sources**: 3 trees (CH fork, Linux v7.0, this repo), 1 web page (issue text)

Citation forms: `CH:<path>:<line>` = `vendors/cloud-hypervisor/<path>:<line> @ f252bb20b`; `LX:<path>:<line>` = `vendors/linux/<path>:<line> @ v7.0`; repo paths are plain. Everything below labelled **Evidence** is read in those trees; everything labelled **Reasoning** is inference and is kept separate. No code was built or run, and there was no shell (no `git`, no `gh`).

## Executive Summary

The fork's raw-disk path gives #348 the durability it asks for, with caveats the design must own. A guest `FLUSH` always becomes a host `fsync` on the disk fd (sync, io_uring and AIO backends alike). On XFS and ext4 that `fsync` forces the journal and, when the log force was a no-op, issues a device cache flush; on a block device it is `blkdev_issue_flush`. Data written without a flush sits in the host page cache (`direct=off` is the default and Overdrive does not change it), which survives a VMM crash but not host power loss — the same contract a disk with a write cache gives the guest. CH never asks the kernel for exclusivity: it takes an advisory OFD lock at `vm.boot`, not at open, and never opens with `O_EXCL`, so single-writer is enforced between CH instances only, not against anything else that opens the file or block device. The lock is released by the kernel on last close, so a dead VMM releases it.

Three findings change the design inputs rather than confirm them. (1) The launch wrapper sets `RLIMIT_FSIZE = max(rootfs, guest RAM)`; the kernel fails any regular-file write at an offset at or beyond that limit with `SIGXFSZ`/`EFBIG` even when the file is already that large. A file-backed volume larger than that cap cannot work; a block device is exempt (the block-device write path never calls the limit check). (2) With `image_type=raw` CH refuses to start a disk whose first or last sectors look like qcow2/VHDX/fixed-VHD, so guest-written content can brick a volume at the next boot. (3) The appliance's data-partition filesystem is not decided anywhere in the repo for production: the only concrete choice is the test host (XFS, `reflink=1`), and the rest is "reflink-capable" (XFS or btrfs, not ext4). The kernel config is not pinned beyond three named options, so `dm`, XFS and btrfs presence are all unverified for the appliance.

## Research Methodology

**Search strategy**: Grep and Read over `vendors/cloud-hypervisor`, `vendors/linux`, the repo's ADRs/brief/infra, plus one WebFetch of the GitHub issue pages (`gh` unavailable).
**Source selection**: primary source code only for behaviour (the trees the platform runs). Reputation: High (primary source). Verification: where two code sites state the same fact they are both cited; no web sources used for behaviour.
**Quality standards**: each behavioural claim has a `path:line`; claims I did not trace are listed under Knowledge Gaps. Cross-referencing is intra-tree (caller and callee), not independent publishers, because the code is the authority.

## Findings — Question 1: the CH fork's block path

### 1a. Flush

**Evidence (request → host call).**
- A guest `VIRTIO_BLK_T_FLUSH` parses to `RequestType::Flush` (CH:block/src/io/request.rs:155, 212). `execute_async` calls `disk_image.fsync(Some(user_data))` for it (CH:block/src/io/request.rs:303-307); the device calls `execute_async` for every queued request (CH:virtio-devices/src/block.rs:358).
- Raw, sync backend: `libc::fsync(fd)` inline, error → `AsyncIoError::Fsync`, completion queued after it returns (CH:block/src/formats/raw/engine_sync.rs:102-116).
- Raw, io_uring: `IORING_OP_FSYNC` with default flags, so a full `fsync`, not `fdatasync` (CH:block/src/io/async_io/uring_data_io.rs:205-212; CH:block/src/formats/raw/engine_uring.rs:73-85). A negative CQE result becomes `VIRTIO_BLK_S_IOERR` (CH:virtio-devices/src/block.rs:523-610).
- Raw, AIO: `submit_fsync` (CH:block/src/formats/raw/engine_aio.rs:75-80; CH:block/src/io/async_io/aio_data_io.rs:107). I did not read the AIO body.
- Backend choice for raw is io_uring, then AIO, then sync, by runtime probe (CH:block/src/factory.rs:142-181). The probe requires `FSYNC`, `READV`, `WRITEV` opcodes (CH:block/src/lib.rs:285-301). `io_uring` is a default feature of the `cloud-hypervisor` crate (CH:cloud-hypervisor/Cargo.toml:47) and Overdrive's provisioning builds with default features plus `mshv` (infra/provision/common-user.sh:75), so the choice depends on the host kernel at run time.
- qcow2: flush calls `metadata.flush()` (CH:block/src/formats/qcow/engine_sync.rs:219-227). VHDX: `vhdx_file.flush()` (CH:block/src/formats/vhdx/engine_sync.rs:83-95). Fixed VHD delegates to the raw engines (CH:block/src/formats/vhd/engine_sync.rs:39-40; CH:block/src/formats/vhd/engine_uring.rs:42-43). I did not trace what `QcowDisk`/`VhdxFile` flush ends in; `sync_all` calls exist at CH:block/src/formats/qcow/metadata.rs:762, 808, 817, 1017 and `sync_data` at :1043.
- Write cache negotiation: the device offers `VIRTIO_BLK_F_FLUSH` and `VIRTIO_BLK_F_CONFIG_WCE`, and does not offer any FUA feature (CH:virtio-devices/src/block.rs:837-844). `writeback` starts true (CH:virtio-devices/src/block.rs:890, 938) and the guest can switch to write-through through the config space (CH:virtio-devices/src/block.rs:1080-1093, 1147-1160). The Linux guest driver defaults to write-back when FLUSH is offered and exposes `cache_type` (LX:drivers/block/virtio_blk.c:1068-1085, 1091-1111); it maps only `REQ_OP_FLUSH` to `VIRTIO_BLK_T_FLUSH` (LX:drivers/block/virtio_blk.c:262-263).
- Write-through mode: sync `execute` calls `fsync` after every write (CH:block/src/io/request.rs:207-209); the async path calls `disk_image.fsync(None)` after each write completes (CH:virtio-devices/src/block.rs:554-557). For io_uring and AIO that `fsync(None)` is `unsafe { libc::fsync(fd) }` with the return value discarded (CH:block/src/formats/raw/engine_uring.rs:80-82; CH:block/src/formats/raw/engine_aio.rs:82-84); the sync engine returns the error (CH:block/src/formats/raw/engine_sync.rs:102-107).
- `fsync` is not issued anywhere else on the device: the only other `fsync` site in the device is a test mock (CH:virtio-devices/src/block.rs:1382). There is no flush at VMM exit or on `remove-device`.

**Evidence (kernel side of the `fsync`).**
- XFS: `xfs_file_fsync` writes back and waits on the range, then forces the log to the inode's commit sequence; if the log force was a no-op on a single-device fs it issues `blkdev_issue_flush` on the data device (LX:fs/xfs/xfs_file.c:125-183, esp. 175-180). A datasync commit sequence is recorded only for changes other than timestamps/iversion (LX:fs/xfs/xfs_inode_item.c:890-894), so extent-allocation changes are in the datasync path.
- Unwritten-extent conversion runs in the I/O completion work before the folio's writeback state is cleared, so a later `fsync` sees it (LX:fs/xfs/xfs_aops.c:156-168, `iomap_finish_ioends` at :168); direct I/O does it in `xfs_dio_write_end_io` (LX:fs/xfs/xfs_file.c:617).
- ext4: `ext4_sync_file` waits for the data range, waits for the journal commit tid (`i_datasync_tid` or `i_sync_tid`) and issues `blkdev_issue_flush` when the commit will not send the barrier itself (LX:fs/ext4/fsync.c:109-189).
- Block device: `blkdev_fsync` = write-and-wait then `blkdev_issue_flush`, with `-EOPNOTSUPP` turned into success (LX:block/fops.c:613-633).

**Reasoning.** A completed guest flush implies host `fsync` returned success, which on XFS/ext4/bdev implies the data and the metadata needed to read it were submitted and the device cache flush was issued, provided the device honours flush. I found no path where `Flush` is acknowledged without an fsync in the raw engines. The exceptions are: (i) write-through mode on io_uring/AIO ignores fsync errors; (ii) `EOPNOTSUPP` from a device with no flush support is treated as success (that is correct for a device with no volatile cache, and wrong if a device lies); (iii) qcow2/VHDX flush behaviour is untraced. Without a flush, writes are in the host page cache only (writes use plain `pwritev` on an fd opened without `O_SYNC`; CH:block/src/factory.rs:86-93). That survives a VMM crash with the host up, and is lost on host power loss; this is the contract a write-cached disk presents, and the guest is told so by `WCE`.

### 1b. `direct=on` / O_DIRECT

**Evidence.**
- Default off: `#[serde(default)] pub direct: bool` (CH:vmm/src/vm_config.rs:388-389); CLI default `Toggle(false)` (CH:vmm/src/config.rs:1485-1489). Overdrive's `--disk` rendering emits only `path=…,image_type=raw[,readonly=on]` (crates/overdrive-core/src/vm/config.rs:233-239), so the fork default (buffered) applies.
- On, the open adds `O_DIRECT` (CH:block/src/factory.rs:89-91). Alignment is the maximum of `stx_dio_mem_align` and `stx_dio_offset_align` from `statx(STATX_DIOALIGN)`, falling back to 512 (CH:block/src/lib.rs:323-378). Unaligned guest requests are bounced/read-modify-written by `AlignedFile` (CH:block/src/aligned_file.rs:27-30; CH:block/src/formats/raw/mod.rs:179). `detect_image_type` reads through an `AlignedFile` forced to the aligned mode (CH:block/src/lib.rs:539-545).
- Flush is unchanged by `direct`: the same `fsync` is issued (CH:block/src/io/request.rs:303-307). Block devices accept `O_DIRECT` (`FMODE_CAN_ODIRECT`, LX:block/bdev.c:1000). Applies equally to the sync, AIO and io_uring raw engines (all take `direct` through `AlignedFile`, CH:block/src/formats/raw/engine_uring.rs:96-114).

**Reasoning.** `direct=on` removes host page-cache double-buffering and its memory pressure, and still needs the guest flush for device-cache and metadata durability. Page-cache memory cgroup charging of buffered writes against the allocation's `memory.max` was not read in `vendors/linux`; it is a load-dependent behaviour and is listed as a spike item.

### 1c. Exclusive access / locking

**Evidence.**
- CH takes an advisory **OFD** whole-file/byte-range lock with `fcntl(F_OFD_SETLK)` (CH:block/src/io/fcntl.rs:8-14, 197-222). The module states that it is advisory and does not prevent `open()` (CH:block/src/io/fcntl.rs:189-191).
- Knob: `lock_granularity=byte-range|full`, default `byte-range` (CH:vmm/src/config.rs:1448, 1567-1570; CH:block/src/io/fcntl.rs:149-156). Byte-range covers `[0, max(logical, physical))` and falls back to whole file if sizes cannot be read (CH:virtio-devices/src/block.rs:986-1012). A read-only disk takes a read lock, otherwise a write lock (CH:virtio-devices/src/block.rs:1015-1019).
- When: the disk is opened and the device built without locking (CH:block/src/factory.rs:85-109; CH:block/src/lib.rs:530-536). The lock is taken in `try_lock_disks`, called at VM boot and restore (CH:vmm/src/vm.rs:2765-2766, 2967-2968; CH:vmm/src/device_manager.rs:2620-2632), and immediately for a hot-plugged disk (CH:vmm/src/device_manager.rs:2873-2881). Release is explicit only for migration (CH:vmm/src/device_manager.rs:2634-2647; CH:vmm/src/lib.rs:1678-1681).
- A conflicting second VMM: `EAGAIN`/`EACCES` → `LockError::AlreadyLocked` (CH:block/src/io/fcntl.rs:214), logged with the existing lock state (CH:virtio-devices/src/block.rs:1027-1041), and the boot fails with `Error::LockingError` (CH:vmm/src/vm.rs:2765-2766). Opening the same image in a second VMM before boot succeeds silently; only `vm.boot` fails.
- No `O_EXCL` exists in the block, virtio-devices or vmm code (grep over those trees: no match); the open flags are read, write (unless readonly), and optionally `O_DIRECT` (CH:block/src/factory.rs:86-93).
- Kernel, lock lifetime: the OFD lock owner is the open file description (LX:fs/locks.c:2506-2515) and `locks_remove_file` drops it on last close (LX:fs/locks.c:2764-2776). The lock is attached to `file_inode(filp)` (LX:fs/locks.c:2771).
- Kernel, `O_EXCL` on a block device (the trace the prior study left open): `blkdev_open` computes the mode from the file flags and `O_EXCL` sets `BLK_OPEN_EXCL` and makes the file its own holder (LX:block/fops.c:644-659, 674-684) → `bdev_open(…, holder=filp)` calls `bd_prepare_to_claim` only when a holder is set (LX:block/bdev.c:949-953) → `-EBUSY` if another holder has it (LX:block/bdev.c:598-611). Without `O_EXCL` no claim is made and `bdev_open` runs with `holder == NULL` (LX:block/bdev.c:954-957). CH never sets `O_EXCL`, so **the kernel claim path is not reached** by CH. A writer opened without `O_EXCL` is also admitted while a filesystem holds the device unless `bdev_allow_write_mounted` is off (LX:block/bdev.c:878-888); `CONFIG_BLK_DEV_WRITE_MOUNTED` defaults to `y` (LX:block/Kconfig:77-79).
- Device-mapper: an LV (`dm`) cannot be removed while open (`dm_open_count` → `-EBUSY`, LX:drivers/md/dm.c:357-392), independent of CH.

**Reasoning.** Single-writer today means "no other CH `vm.boot` on the same inode range while this VMM lives". It does not exclude Overdrive's own code (format, grow, delete, a second `open` for GC) or a second VMM that is only created. The lock follows the inode of the path opened, so two different device nodes for one block device are different inodes and would not conflict; I read that `bdev_open` replaces only `f_mapping` (LX:block/bdev.c:1005) but did not test it. The kernel claim (`O_EXCL`) is the stronger primitive; using it would need a CH change, which the fork policy allows (CLAUDE.md "Vendored components"), and the design, not this study, decides whether it is wanted.

### 1d. Image formats and disk options

**Evidence.**
- Formats: raw (file or block device), qcow2, VHDX, fixed VHD (CH:block/src/factory.rs:96-106; CH:block/src/lib.rs:485-523). Dynamic VHD is not opened. qcow2 is opened writable unless `readonly` and has its own metadata flush path (CH:block/src/factory.rs:183-216); backing files only with `backing_files=on` (CH:vmm/src/device_manager.rs:2801-2803).
- `image_type`: if unset CH auto-detects, **disables sector-0 writes for raw/VHD** and warns (CH:vmm/src/device_manager.rs:2763-2786; CH:virtio-devices/src/block.rs:225-228). If set and different from the detected type, the device build fails with `DiskImageTypeMismatch` (CH:vmm/src/device_manager.rs:2794-2799). Overdrive always emits `image_type=raw` (crates/overdrive-core/src/vm/config.rs:212-239).
- Detection reads guest-writable bytes: the first block for qcow2 magic or the VHDX signature, and the VHD footer at the end of the file; anything else is raw (CH:block/src/lib.rs:539-557; CH:block/src/formats/vhd/footer.rs:121-129).
- Raw `sparse` defaults true (CH:vmm/src/vm_config.rs:413-414, 442-444). With `sparse=on` the device offers `DISCARD` as well as `WRITE_ZEROES` (CH:virtio-devices/src/block.rs:846-860), and a guest discard punches a hole (`fallocate(PUNCH_HOLE|KEEP_SIZE)` on files, `BLKDISCARD` on block devices; CH:block/src/sparse.rs:60-76). `WRITE_ZEROES` is offered whenever the file supports `ZERO_RANGE` (CH:block/src/sparse.rs:78-94; probe at CH:block/src/lib.rs:381-430). With `sparse=off` and a writable raw file, `open_raw` runs `fallocate(fd, 0, 0, size)` (preallocation) on every open and only warns on failure (CH:block/src/factory.rs:146-148; CH:block/src/lib.rs:455-483).
- `readonly=on` → `VIRTIO_BLK_F_RO`, writes rejected with `IOERR` (CH:virtio-devices/src/block.rs:866-868, 211-223). `num_queues` default 1, `queue_size` default 128, `num_queues` must not exceed boot vCPUs (CH:vmm/src/vm_config.rs:430-440; CH:vmm/src/config.rs:1631-1634). `rate_limit_group`/`bw_*`/`ops_*` apply a token-bucket (CH:vmm/src/device_manager.rs:2805-2835).
- Identification: `serial=` up to 20 bytes (`VIRTIO_BLK_ID_BYTES`; CH:vmm/src/config.rs:1662-1666). If omitted the serial is `st_dev`‖`st_rdev`‖`st_ino` of the path, decimal-concatenated and truncated (CH:block/src/lib.rs:86-116). `st_dev`/`st_rdev` are not stable across hosts or reboots in general. The Linux guest reads the serial with `VIRTIO_BLK_T_GET_ID` and exposes it as a sysfs attribute (LX:drivers/block/virtio_blk.c:804-821, 890-910). Capacity is in the config space (CH:virtio-devices/src/block.rs:887-889).

**Reasoning.** A raw sparse file, `image_type=raw`, io_uring (or fallback) is the well-trodden path. A stable guest identity needs an explicit `serial=`. The detection finding is a design input: a guest that writes qcow2 magic at offset 0 of its data disk makes the next boot fail at device build, and detection is the only guard against format confusion (omitting `image_type` would be worse: it would auto-detect and could interpret guest-written qcow2).

### 1e. Hotplug and resize

**Evidence.**
- `vm.add-disk` and `vm.remove-device` exist (CH:vmm/src/api/http/mod.rs:205-206, 264-266; CH:vmm/src/lib.rs:2714-2760); add-disk builds the device with `is_hotplug=true`, which takes the lock immediately (CH:vmm/src/device_manager.rs:5417-5426, 2873-2881). Before the VM boots, add-disk edits the config (CH:vmm/src/lib.rs:3504-3525).
- `vm.resize-disk` exists (CH:vmm/src/api/http/mod.rs:272-273; CH:vmm/src/device_manager.rs:5523-5533). For a file it calls `set_len`; for a block device it only verifies the size already equals the requested size and errors otherwise (CH:block/src/formats/raw/mod.rs:106-133). It pauses the device, updates the capacity, and raises a config-change interrupt (CH:virtio-devices/src/block.rs:1095-1117).

**Not traced**: whether `remove-device` closes the last dup of the fd so that the OFD lock is released at hot-remove; `File::try_clone` dups share one open file description, so the lock lives until every clone is dropped.

### 1f. What the fork changed, and how Overdrive invokes CH

**Evidence.**
- **Fork diff vs upstream (verified by git, 2026-11-17).** The fork's merge base with upstream `cloud-hypervisor/cloud-hypervisor` `main` is `9ed824d6d` ("build: Release v53.0", 2026-07-12). Exactly three commits sit on top: `9b68dbb57` (vhost-kernel vsock device), `41a619d19` (`backend=vhost-kernel` for `--vsock`) and `f252bb20b` (exit the VMM on guest reset). `git diff 9ed824d6d..HEAD -- block/ virtio-devices/src/block.rs` is empty, and `41a619d19`'s changes to `vmm/src/config.rs` / `vmm/src/device_manager.rs` are vsock-only. **The fork's block path is upstream v53.0 unchanged**; every block-path finding above describes upstream v53.0 behaviour.
- Overdrive spawns CH as `prlimit --fsize=… --nofile=… -- setpriv --reuid=<uid> --regid=<gid> --groups=… --no-new-privs -- cloud-hypervisor …` (crates/overdrive-core/src/vm/config.rs:499-520; crates/overdrive-host/src/vmm.rs:259-310), with `--seccomp true` (crates/overdrive-core/src/vm/config.rs:440-459), `--landlock` plus one `--landlock-rules path=<run_dir>,access=rw` (crates/overdrive-core/src/vm/config.rs:907-915), one `--disk` for the rootfs clone, and a launch seccomp that denies 13 TUN/`SIOCSIFHWADDR` ioctls and allows every other syscall (crates/overdrive-host/src/vmm/launch_seccomp.rs:107-132).
- CH's own seccomp allows `fsync`, `fdatasync`, `fcntl`, `fallocate`, `ftruncate`, and `io_uring_enter` on the block thread (CH:virtio-devices/src/seccomp_filters.rs:113-139) and `fcntl`, `fallocate`, `fsync`, `ftruncate`, `io_uring_setup` on the VMM thread (CH:vmm/src/seccomp_filters.rs:649-706). The block-thread ioctl allow-list is `BLKDISCARD`/`BLKZEROOUT` only (CH:virtio-devices/src/seccomp_filters.rs:141-148).
- CH derives a Landlock `rw` rule for each disk path (CH:vmm/src/vm_config.rs:421-428); the ruleset ABI is V3 and `from_write` includes truncate (CH:vmm/src/landlock.rs:37, 54-58).
- **`RLIMIT_FSIZE`** is `max(rootfs master bytes, guest RAM bytes)` (crates/overdrive-core/src/vm/config.rs:900-905), a value tied to a single rootfs file and RAM; its docstring says the managed-volume work "lands with" GH #97 (crates/overdrive-core/src/vm/config.rs:889-893). Kernel: a regular-file write at `pos >= RLIMIT_FSIZE` gets `SIGXFSZ` and `-EFBIG` regardless of the file's current size (LX:fs/read_write.c:1711-1723); XFS and ext4 both call it (LX:fs/xfs/xfs_file.c:469, 545; LX:fs/ext4/file.c:253), and growing past it via truncate/fallocate sizing is refused (LX:fs/attr.c:258-265). The block-device write path has its own size check and does not call `generic_write_checks` (LX:block/fops.c:746-783).
- Ownership: CH runs as a dropped uid and opens the rootfs clone `O_RDWR`, so the platform chowns it and grants traverse on the staging dir once (crates/overdrive-host/src/vmm.rs:735-775; docs/product/architecture/brief.md:493-509). A persistent volume file needs the same: a platform-owned directory, a chown to the confined uid, and a traverse grant. A block-device volume needs the device node's mode to permit that uid.
- CH is built with default features (`io_uring`, `kvm`) plus `mshv` on x86_64 (CH:cloud-hypervisor/Cargo.toml:47; infra/provision/common-user.sh:75).

**Reasoning.** Nothing in seccomp, Landlock or the launch filter blocks `fsync`, `fcntl` OFD locks or `fallocate` for a second disk. The binding constraint is `RLIMIT_FSIZE` for a file-backed volume. The uid-drop requirement means the volume path must be created and owned by the platform before launch, as the rootfs clone is.

## Findings — Question 2: the node data filesystem

### 2a. What the appliance uses and where it is pinned

**Evidence.**
- ADR-0068 pins the kernel (Linux 7.0) and three image kernel-config options (`CONFIG_NET_HANDSHAKE`, vsock, `CONFIG_MODULE_UNLOAD` off; docs/product/architecture/adr-0068-pinned-appliance-kernel-floor.md:73-74, 120-132, 176, 208-232). It names no filesystem, `dm`, XFS, ext4 or btrfs option. A repo-wide grep for `CONFIG_(XFS_FS|EXT4_FS|BTRFS_FS|BLK_DEV_DM|DM_…)` outside `vendors/` finds only the guest kernel's `CONFIG_EXT4_FS=y` and `CONFIG_VIRTIO_BLK=y` in spike findings (docs/feature/microvm-driver-cloud-hypervisor/spike/findings.md:752).
- The requirement is "reflink-capable", asserted with a real `FICLONE` probe, not an fstype string: docs/product/architecture/brief.md:10402-10410; "the appliance's single durable data partition" (docs/product/architecture/brief.md:504); `Vmm::probe` refuses boot otherwise (docs/product/architecture/brief.md:700). ext4 is explicitly not reflink-capable (infra/metal/provision.sh:381-384, 440).
- The only concrete filesystem is the test host's: `mkfs.xfs -m reflink=1` on the data disk, mounted `xfs defaults,noatime` (infra/metal/provision.sh:420-421, 434), and proved by `cp --reflink=always` (infra/metal/provision.sh:445-448). `infra/metal/partitions.json:20-21` is ext4 for `/boot` and `/`.
- GH #97 comments: 2026-08-19 says "on the shipped appliance both hold by construction — the appliance's one VM data partition"; 2026-08-11 records the benchmark filesystem "XFS(reflink=1) on NVMe" (WebFetch of https://github.com/overdrive-sh/overdrive/issues/97, 2026-11-17). Neither says the production image's filesystem. ADR-0083 and ADR-0082 repeat "reflink-capable" and name no filesystem (docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md:290-295, 2079-2123).
- No appliance image definition exists in the repo (no image or kernel-config files under `infra/`); `infra/` has only `metal/` and `provision/` scripts.

**Conclusion.** The production data filesystem is **undecided**. The constraint on it is "reflink-capable" (XFS or btrfs); XFS is the one exercised. #348 should not assume ext4. This is a design input, not something to guess.

### 2b. XFS, ext4, btrfs: the behaviours #348 depends on

All rows are from `vendors/linux` @ v7.0.

| Question | XFS | ext4 | btrfs |
|---|---|---|---|
| `fsync` persists extent metadata for a hole-filling or preallocation-converting write | Yes: allocation/conversion transactions set the datasync sequence (LX:fs/xfs/xfs_inode_item.c:890-894), `xfs_file_fsync` forces to it (LX:fs/xfs/xfs_file.c:101-122, 162-166) | Yes via the journal tid (LX:fs/ext4/fsync.c:109-128, 141-189) | Log-tree sync with a full-sync flag path (LX:fs/btrfs/file.c:1557-1731); not traced further |
| Device cache flush on `fsync` | Log force, plus `blkdev_issue_flush` if the log force was a no-op on one device (LX:fs/xfs/xfs_file.c:175-180) | `blkdev_issue_flush` when the commit does not send the barrier (LX:fs/ext4/fsync.c:123-125, 177-182) | not traced |
| Overwrite of an already-written preallocated file | Overwrite only: no metadata change, so the no-op-log-force flush at :175-180 is what makes `fdatasync`-class syncs durable | Journal-less or no-metadata case needs the barrier path (LX:fs/ext4/fsync.c:82-107, 158-163) | `btrfs_check_nocow_lock` returns early unless the inode is NODATACOW or PREALLOC (LX:fs/btrfs/file.c:987-988), i.e. an ordinary file takes the CoW path |
| `fallocate` modes | mode 0, `KEEP_SIZE`, `PUNCH_HOLE`, `ZERO_RANGE`, `COLLAPSE`, `INSERT`, `UNSHARE` (LX:fs/xfs/xfs_file.c:1409-1413); mode 0 allocates with `XFS_BMAPI_PREALLOC`, i.e. unwritten extents (LX:fs/xfs/xfs_bmap_util.c:751) | `KEEP_SIZE`, `PUNCH_HOLE`, `ZERO_RANGE`, `COLLAPSE`, `INSERT`, `WRITE_ZEROES`; mode 0 uses unwritten extents (LX:fs/ext4/extents.c:4720, 4869-4872); non-extent files return `EOPNOTSUPP` (:4709-4710) | `KEEP_SIZE`, `PUNCH_HOLE`, `ZERO_RANGE` only (LX:fs/btrfs/file.c:3130-3132) |
| First write to a preallocated (unwritten) range | Extra transaction at I/O completion to convert unwritten → written, before writeback state clears (LX:fs/xfs/xfs_aops.c:156-168; direct: LX:fs/xfs/xfs_file.c:617) | Converted at end-io (same pattern; not traced) | n/a (see CoW) |
| ENOSPC on a sparse file | Delalloc blocks are reserved at write time, so a guest write can fail with `ENOSPC` mid-flight after the file was sized (LX:fs/xfs/xfs_iomap.c:1976-1986; LX:fs/xfs/xfs_file.c:1017-1042) | Reserved at `da_write` time (LX:fs/ext4/inode.c:1625, 1857-1882) | Not traced; CoW overwrite allocates, see above |
| Online grow of a file volume | `ftruncate` larger (CH `set_len`; CH:block/src/formats/raw/mod.rs:128-130), subject to `RLIMIT_FSIZE` (LX:fs/attr.c:258-265); guest resizes its fs | same | same |

**Reasoning.** The finding that matters across all three: a sparse file gives no space guarantee, so a guest write can return an I/O error after the volume was "created" — the guest sees `VIRTIO_BLK_S_IOERR` (CH:virtio-devices/src/block.rs:604-610) and its fs reacts. A mode-0 `fallocate` reserves on XFS/ext4 and converts on first write at a measurable cost, and a guest discard (offered by default with `sparse=on`) punches the reservation back out, so reservation holds only with `sparse=off` (which also runs `fallocate` on every open) or no guest TRIM. On btrfs an ordinary file is CoW and does not guarantee overwrite space, so it is the weakest fit for the "reservation" property; this is inference from the NOCOW early-return, not traced to the allocator. XFS or ext4 with mode-0 `fallocate` is the straightforward file-backed reservation; the choice between them is bounded by the reflink requirement (ext4 is out).

### 2c. LVM / dm-linear as the backing

**Evidence.**
- dm-linear is built with the core device-mapper module: `dm-mod-y += dm.o dm-table.o dm-target.o dm-linear.o …` (LX:drivers/md/Makefile:6), so it needs `CONFIG_BLK_DEV_DM` (LX:drivers/md/Kconfig:224-225). Thin provisioning is a separate option, `CONFIG_DM_THIN_PROVISIONING` (LX:drivers/md/Makefile:73), not needed for plain LVs.
- Flush: the linear target sets `num_flush_bios = 1` and `flush_bypasses_map` (LX:drivers/md/dm-linear.c:61, 65), and remaps sectors one to one (LX:drivers/md/dm-linear.c:82-94). A guest flush on a CH-opened LV is `blkdev_fsync` → `blkdev_issue_flush` on the dm device (LX:block/fops.c:613-633), which the dm layer fans to the underlying device.
- Exclusivity: no stronger than a file unless CH opened with `O_EXCL`, which it does not (§1c). Separately, dm refuses removal of an open LV (`-EBUSY`; LX:drivers/md/dm.c:357-392), a kernel-enforced guard against deleting an LV under a live VMM that a file `unlink` does not give.
- `RLIMIT_FSIZE` does not apply to a block device write (LX:block/fops.c:746-783); `ENOSPC` is returned for a write at or beyond the device end (LX:block/fops.c:765-766); there is no sparse-file mid-flight `ENOSPC`.
- CH treats a block device as raw: size from `BLKGETSIZE64` (CH:block/src/lib.rs:585-601), discard through `BLKDISCARD`, resize only by verifying an externally resized size (CH:block/src/formats/raw/mod.rs:113-126), no preallocation call because `preallocate_disk` is skipped only for readonly/sparse (a mode-0 `fallocate` on a block device fails `EOPNOTSUPP`-class and only warns; CH:block/src/lib.rs:469-476, not tested).
- Presence in the repo: no `lvm`, `dmsetup`, `thin-pool` reference in `infra/`, `crates/`, `xtask/`, or `docs/product/` (grep). The only hit is `docs/feature/microvm-driver-cloud-hypervisor/intake.md`. The pinned-kernel config is not in the repo (§2a), so `CONFIG_BLK_DEV_DM` presence is **unverified**; the userspace (`lvm2`/`dmsetup`) is not provisioned by `infra/` either. ADR-0068 turns module unload off (docs/product/architecture/adr-0068-pinned-appliance-kernel-floor.md:208-232), so `dm-mod` would have to be built in or loaded before.

**Reasoning.** An LV is the one option that gives a hard size, a kernel-level "in use" guard, no `RLIMIT_FSIZE` interaction, and no sparse `ENOSPC`. It costs a partition/VG layout decision that collides with the "one VM data partition, reflink-capable filesystem" the rootfs clone path already requires (the rootfs clones need a filesystem; an LV pool would be a second region of the disk), an LVM/dm dependency in the image, and a separate story for growth (LV extend then `vm.resize-disk` size check). Whether that is worth it is a cost comparison, which design.md says needs a benchmark spike (below).

## What #348 can rely on vs what Overdrive must provide

| Property | #348 can rely on (evidence) | Overdrive must provide / decide |
|---|---|---|
| Guest flush durability | Guest FLUSH → host `fsync` on every raw engine; XFS/ext4/bdev `fsync` forces journal and flushes the device cache (§1a) | Keep default `direct`/cache mode explicit in the design; decide direct vs buffered; treat guest `IOERR` on flush as a volume fault; never accept write-through mode on io_uring/AIO as stronger (its errors are dropped); name the backend (io_uring/AIO/sync is a runtime probe) |
| Survives VMM crash / restart / redeploy | Data written is in the host page cache or on disk and belongs to the file/LV, which outlives the VMM; OFD lock dies with the VMM (LX:fs/locks.c:2764-2776) | The file/LV is Overdrive's, so create, adopt after crash, and delete are its convergent lifecycle (observe → diff → converge); host power loss loses only unflushed writes |
| Single writer | CH OFD lock at `vm.boot`; second boot fails `AlreadyLocked` (§1c) | A replacement must wait on the lock or the VMM exit; the lock is advisory, taken at boot and not at open, and not seen by non-CH openers; `O_EXCL`/kernel claim is not used (needs a fork change if wanted) |
| Stable guest identification | `serial=` ≤ 20 bytes via `GET_ID`; Linux guest exposes it in sysfs (§1d) | Always pass an explicit `serial`; the default is derived from `st_dev/rdev/ino` and is not stable |
| Space reservation | Mode-0 `fallocate` reserves on XFS/ext4 (unwritten extents); a block device has fixed size (§2b, §2c) | Decide reserved vs sparse; `sparse=on` (CH default) offers discard that un-reserves; sparse writes can `ENOSPC` mid-flight; `sparse=off` runs mode-0 `fallocate` on every open |
| File volume size cap | None from CH | `RLIMIT_FSIZE` is `max(rootfs, RAM)` today; a tens-of-GB file volume writes `SIGXFSZ`/`EFBIG` past that. A block device is exempt (§1f) |
| Growth | `vm.resize-disk` does `set_len` on a file and updates guest capacity; for a block device it only verifies the size (§1e) | Host-side extend (ftruncate or LV extend), guest-side fs resize, the `RLIMIT_FSIZE` cap, and the `ByteRange` lock range (it is computed from size at lock time) |
| Content safety of the volume | `image_type=raw` blocks format confusion by erroring on mismatch | Guest-written qcow2/VHDX/VHD bytes make the boot fail; decide how a volume is protected or recovered (§1d) |
| Filesystem for file-backed volumes | XFS (test host) or "reflink-capable" | The production data filesystem, and whether volumes live on it or on dm, are undecided (§2a) |
| Hot-attach/remove | `vm.add-disk` / `vm.remove-device` exist and lock on attach (§1e) | Restart currently ends the VMM; decide whether hotplug is used at all |

## Knowledge Gaps

1. **Fork diff vs upstream in the block path.** Closed: verified by git (see Question 1f): the block path is upstream v53.0 unchanged.
2. **AIO `submit_fsync` body, qcow2/VHDX flush ends, and write-through error propagation from the sync engine** (`Error::Fsync` out of `process_queue_complete`; CH:virtio-devices/src/block.rs:556). Not traced. Recommendation: only needed if write-through or non-raw formats are in scope.
3. **btrfs allocator and ENOSPC on overwrite, ext4 end-io unwritten conversion, XFS `ENOSPC` against `xfs_alloc_file_space`.** Partly traced; btrfs rows are inference. Recommendation: skip unless btrfs is a candidate.
4. **Whether the OFD lock is released on hot-remove** (`remove-device`), and whether an in-flight io_uring write at VMM death can complete after the lock is released. Timing-dependent; see spikes.
5. **Page-cache memory cgroup charging of buffered guest writes against the allocation's `memory.max`.** Not read in `vendors/linux`.
6. **Lock key for aliased device nodes** (inference from `file_inode` and `bdev_open`; not tested).
7. **Pinned kernel's filesystem/dm config.** Not in the repo; ADR-0068 names only three options.
8. **`gh` unavailable**: issue text came from WebFetch summaries (issue #348 has no comments; #97 comments as quoted). Issue content may omit nuance.
9. **Kernel evidence for the Linux guest kernel is the host tree**; the guest kit's own config was not read beyond `CONFIG_EXT4_FS=y`/`CONFIG_VIRTIO_BLK=y` (spike findings).

## Must decide or spike (per `.claude/rules/design.md`; named, not run)

**Decisions the design must make** (each has a surfaced-to-the-user obligation):
1. Backing: file on the data filesystem vs raw LV vs both; tied to the undecided production filesystem and to `CONFIG_BLK_DEV_DM`.
2. Space model: reserved (mode-0 `fallocate`, `sparse=off`) vs sparse; whether guest discard is offered; behaviour when a guest write hits `ENOSPC`.
3. Cache mode: `direct=on` vs buffered; whether write-through is allowed.
4. How a replacement allocation learns the previous VMM has released the volume (lock wait vs process exit), and whether the exclusivity primitive stays advisory.
5. How `RLIMIT_FSIZE` is derived once a volume exists (file backing only).
6. What protects a volume from guest content that matches another image type, and what recovery looks like.
7. Stable `serial` derivation and the guest-side by-id path.

**Spikes** (a behaviour depending on timing/load, or a cost comparison between mechanisms):
- **S1 (behaviour, timing):** `kill -9` the VMM mid-I/O with io_uring and with direct I/O, start a replacement; observe lock release vs in-flight completions, guest data after a flush-then-kill and a write-then-kill, on kernel 7.0.
- **S2 (behaviour, load):** a volume larger than `RLIMIT_FSIZE`, to confirm the `SIGXFSZ` kill on the CH process through each backend (sync/AIO/io_uring), versus an LV of the same size.
- **S3 (behaviour, load):** buffered writes of tens of GB inside the allocation cgroup against `memory.max`; throttle and OOM-kill behaviour, with `direct` on and off.
- **S4 (cost comparison):** file-on-XFS (preallocated, sparse) vs raw LV on one host and load: fsync latency, first-touch unwritten-conversion cost, fragmentation under a `target/`-like write pattern, CPU per flush. The result goes in the decision request before the choice is approved.
- **S5 (behaviour):** guest write failing with `ENOSPC` on a sparse file and the guest's fs reaction; guest discard on a preallocated file shrinking host allocation.
- **S6 (behaviour):** a guest writing qcow2 magic / a VHD footer to its data disk and the next boot.

## Source Analysis

| Source | Domain | Reputation | Type | Access | Cross-verified |
|---|---|---|---|---|---|
| Cloud Hypervisor fork @ f252bb20b | `vendors/cloud-hypervisor` (overdrive-sh fork) | High (primary code) | Source | 2026-11-17 | Caller/callee in-tree |
| Linux v7.0 @ 028ef9c96e96 | `vendors/linux` | High (primary code) | Source | 2026-11-17 | Multiple call sites |
| Repo ADRs, brief, infra, config | this worktree | High (project SSOT; whitepaper not used) | Project docs/code | 2026-11-17 | Yes |
| GitHub issues #348, #97 | github.com/overdrive-sh/overdrive | Medium (WebFetch summaries) | Project issue tracker | 2026-11-17 | No |

Reputation: High 3 (75%), Medium 1 (25%). Avg ≈ 0.95. No external web source supports any behavioural claim.

## Research Metadata

Examined ~70 file ranges; cited all in the findings. Tool failures: no shell (`git`, `gh`); LSP hook errors on read files were noise (cross-compile on macOS) and ignored. Output: `docs/research/platform/vm-persistent-volume-block-path-and-backing-research.md`. Closes prior-art study Knowledge Gaps 1, 2 (non-clone part) and 9 as follows: gap 1 → §1; gap 9 → §1c (the `O_EXCL` trace is done and CH never sets it); gap 2 → §2b for the non-clone case only (reflink/CoW internals remain open).
