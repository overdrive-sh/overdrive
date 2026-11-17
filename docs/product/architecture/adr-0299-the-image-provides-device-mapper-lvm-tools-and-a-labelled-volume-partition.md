# ADR-0299 — The image provides the volume store: built-in device-mapper, LVM tools and a provisioned volume group on a labelled partition

## Status

**Proposed** (decision VD-4, GH #348). Not accepted until the user approves it.
Validation item V-1 passed (feature delta § Validation).

## Context

ADR-0068 pins only the kernel version and a few options. The repository holds no image
definition, kernel configuration or disk-layout file; the appliance is described in prose
(GH #75 is an empty placeholder). The metal test host's layout is
`infra/metal/partitions.json` plus `infra/metal/provision.sh`. The appliance has no
operator shell. ADR-0068 § 7 turns kernel module unloading off, so anything the node needs
from device-mapper must be built in.

## Decision

**The appliance image provides: (a) a kernel with `CONFIG_BLK_DEV_DM=y` built in
(dm-linear is part of dm core), `CONFIG_EFI_PARTITION=y` (without it the kernel does not
export `PARTNAME`, which is how the partition is found) and `CONFIG_DEVTMPFS=y` mounted on
`/dev`, plus a writable tmpfs on `/run`; (b) the `lvm` command-line tools from LVM2 with their
library closure (or a static build), run one-shot with no daemon, no udev, no `lvm.conf` and no
`dmeventd`; the image carries no `/etc/lvm`, because the adapter supplies LVM's system directory
on every spawn (feature delta § Port); (c) a GPT partition with partition label `overdrive-volumes` on the VM data
disk, separate from the reflink data partition and wiped when the disk is provisioned (LVM
zeroes at most the first 4 KiB of a new volume, so its contents are those of its extents; the
wipe is what the design relies on, not that default); (d) on that partition, one physical volume
and a volume group named `overdrive` with a 4 MiB extent size, created when the disk is
provisioned, and re-provisioned together with the data directory (deleting the data directory
alone leaves volumes whose workloads a redeploy of the same id would re-attach). Overdrive only verifies this at boot (ADR-0300); it never creates, formats or
wipes the partition, the physical volume or the volume group.**

## Alternatives considered

- **Overdrive creates the physical volume and group on a blank partition at boot.**
  Rejected: a destructive code path with its own crash window (a physical volume with no
  group), and the image already carries LVM and already writes the partition table, so the
  installer is not a new place.
- **Put the existing XFS data filesystem on a logical volume in the same group.** Rejected:
  changes the layout every VM launch depends on (reflink clones, redb stores) to save a
  partition boundary.
- **Hand-written device-mapper tables with an Overdrive-owned extent allocator.** Rejected:
  it would re-implement LVM's crash-safe metadata and allocation, and LVM runs correctly on a
  read-only, udev-less node (V-1).
- **An empty `/etc/lvm` baked into the image.** Rejected: it works (V-1) but adds an image
  detail the adapter's per-spawn `LVM_SYSTEM_DIR=/run/lvm` makes unnecessary.

## Consequences

- Positive: no operator step and no destructive Overdrive path; the image contract is
  four items.
- Negative: LVM2 (GPL-2.0, libdevmapper LGPL-2.1) joins the image; the image owner sizes,
  labels and provisions the partition (tracked under GH #75). The metal test host's
  provisioning is extended in DELIVER.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Validation V-1
- [ADR-0068](adr-0068-pinned-appliance-kernel-floor.md) (kernel pin; module-unload amendment), GH #75
- `PARTNAME` is exported by the kernel for a named GPT partition (`block/partitions/core.c:259 @ v7.0`).
