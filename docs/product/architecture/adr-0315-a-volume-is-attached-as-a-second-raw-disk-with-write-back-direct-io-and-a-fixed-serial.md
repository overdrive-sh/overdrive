# ADR-0315 — A volume is attached as a second raw disk with write-back, direct I/O and a fixed serial

## Status

**Proposed** (decision VD-6, GH #348). Not accepted until the user approves it.

## Context

Spike probes A, C, E and F (metal host, confined VMM uid 4200 with Landlock and seccomp)
measured the disk attributes. Write-back plus guest flush was durable on every backend,
across SIGKILL (0 violations, negative control 40 of 40). Buffered VMM writes are charged to
the allocation's `memory.max` and slowed volume writes 2.1x to 5.7x. A guest that wrote a
qcow2, VHDX or fixed-VHD marker into its own volume made the next boot fail, or silently
shrank the disk by 512 bytes when no `image_type` was given. The default serial changes with
the inode and the host. A guest can switch the virtio-blk cache mode itself.

## Decision

**The volume is the second `--disk` of the VMM: `image_type=raw` always, `direct=on`,
writable, the default cache mode (write-back with guest flush), the default `sparse=true`,
and `serial=ovd-volume`. Overdrive never formats it: the guest sees the volume as created (the image wipes the
partition, ADR-0313) and owns its filesystem. The guest finds it as
`/dev/disk/by-id/virtio-ovd-volume` where it runs udev, otherwise as the second virtio-blk
disk (`/dev/vdb`). Write-through is not offered. Before launch the platform makes the volume's
device node accessible to the confined VMM user (owner `root`, the VMM's group, mode `0660`); a
block device node is refused to the dropped uid otherwise (spike C3). The owner and mode are not
restored at teardown: each VMM's Landlock allow-list is derived from its own launch arguments, so
no VMM can open another workload's node.**

## Alternatives considered

- **Write-through.** Rejected: slower, and not needed for durability; the guest can still
  request it, which the fork changes keep honest (ADR-0316).
- **Leave the node's owner and mode alone and run the VMM with wider privilege.** Rejected: the
  confined uid drop is the platform's isolation boundary and is not weakened for one disk.
- **Buffered host I/O (`direct=off`).** Rejected: page cache charged to `memory.max`; the OOM
  boundary under heavy dirtying was not mapped.
- **The default serial.** Rejected: it changes with the inode and the host.
- **A per-volume unique serial.** Rejected: a serial identifies a disk only within one
  guest, and a guest has one volume.
- **The platform formats the volume.** Rejected: the platform cannot know the guest's
  filesystem choice.

## Consequences

- Positive: durable flush, bounded memory charge, a guest-stable device identity.
- Negative: a guest `IOERR` on flush is the platform's only fault signal and is the guest's
  to treat as a volume fault.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Disk attachment
- Spike findings A, C1 to C3, E3, F; [ADR-0316](adr-0316-a-failed-write-through-fsync-completes-the-request-with-an-io-error.md),
  [ADR-0319](adr-0319-a-writable-block-device-is-opened-exclusively-by-the-vmm.md),
  [ADR-0320](adr-0320-an-explicit-disk-image-type-skips-format-sniffing.md)
