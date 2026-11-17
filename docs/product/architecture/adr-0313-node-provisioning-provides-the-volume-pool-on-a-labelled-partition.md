# ADR-0313 — Node provisioning provides the volume pool: built-in device-mapper, LVM tools and a provisioned volume group on a labelled partition

## Status

**Accepted** (decision VD-4, GH #348). Approved by the user on 2026-11-17.
Validation item V-1 passed (feature delta § Validation).

## Context

The **volume pool** is the node-level LVM setup every VM volume is carved from: the
`overdrive-volumes` partition, the physical volume and the `overdrive` volume group on it, plus
the `lvm` tools and device-mapper. One VM volume is a single logical volume in the pool.

ADR-0068 pins only the kernel version and a few options. The repository holds no appliance image
definition, kernel configuration or disk-layout file, and no appliance image exists yet; GH #75
owns the image build and records the requirement below for it. Today Overdrive runs on the metal
test host, whose layout is `infra/metal/partitions.json` plus `infra/metal/provision.sh`, and in
guest-kit test guests. The appliance has no operator shell. ADR-0068 § 7 turns kernel module
unloading off, so anything the node needs from device-mapper must be built in.

## Decision

**Node provisioning provides the volume pool: (a) a kernel with `CONFIG_BLK_DEV_DM=y` built in
(dm-linear is part of dm core), `CONFIG_EFI_PARTITION=y` (without it the kernel does not
export `PARTNAME`, which is how the partition is found) and `CONFIG_DEVTMPFS=y` mounted on
`/dev`, plus a writable tmpfs on `/run`; (b) the `lvm` command-line tools from LVM2 with their
library closure (or a static build), run one-shot with no daemon, no udev, no `lvm.conf` and no
`dmeventd`; the node carries no `/etc/lvm`, because the adapter supplies LVM's system directory
on every spawn (feature delta § Port); (c) a GPT partition with partition label `overdrive-volumes` on the VM data
disk, separate from the reflink data partition and wiped when the disk is provisioned (LVM
zeroes at most the first 4 KiB of a new volume, so its contents are those of its extents; the
wipe is what the design relies on, not that default); (d) on that partition, one physical volume
and a volume group named `overdrive` with a 4 MiB extent size, created when the disk is
provisioned, and re-provisioned together with the data directory (deleting the data directory
alone leaves volumes whose workloads a redeploy of the same id would re-attach). Overdrive only verifies this at boot (ADR-0314); it never creates, formats or
wipes the partition, the physical volume or the volume group.**

Who provisions it, per environment where Overdrive runs:

- **The metal test host:** `infra/metal/partitions.json` and `infra/metal/provision.sh` are extended in
  DELIVER to create the partition, the physical volume and the group.
- **Guest-kit test guests** (Tier-3 tests run inside guests, not on the host's own disk): the guest kit
  gives each guest its own volume pool. The guest's kernel provides the three kernel features of (a)
  (the guest kit's kernel is the metal host's own Linux 7.0 kernel); the guest's staged rootfs carries
  the `lvm` binary with its library closure and a writable `/run`; and the guest is attached a second
  block device, a backing disk holding a wiped GPT partition labelled `overdrive-volumes` with the
  physical volume and the `overdrive` group (4 MiB extents), created by the preparation step and never
  by Overdrive, and fresh for each guest so no test sees another guest's volumes. The obligation sits
  in `xtask/src/guest_kit_prepare.rs` (prepares the backing disk at a target-scoped path, like its other
  artefacts), `crates/overdrive-testing/src/guest_kit.rs` (stages the `lvm` closure into the guest
  rootfs and attaches the disk) and `crates/overdrive-testing/src/placement/groups.rs` (the closed
  registry of guest groups and profiles; the volume Tier-3 tests run in `cgroup-default`). Which
  profile or profiles of that group carry the pool is DISTILL's to pin; a new profile is a change to
  the closed registry, which DISTILL surfaces. A guest of a group or profile without the pool composes
  no volume pool (the soft case of ADR-0314).
- **The appliance image (future):** GH #75 records the same four items as requirements for the image build.
  The image does not exist yet and is not a precondition of this feature.

## Alternatives considered

- **Overdrive creates the physical volume and group on a blank partition at boot.**
  Rejected: a destructive code path with its own crash window (a physical volume with no
  group), and provisioning already carries LVM and already writes the partition table, so the
  provisioner is not a new place.
- **Put the existing XFS data filesystem on a logical volume in the same group.** Rejected:
  changes the layout every VM launch depends on (reflink clones, redb stores) to save a
  partition boundary.
- **Hand-written device-mapper tables with an Overdrive-owned extent allocator.** Rejected:
  it would re-implement LVM's crash-safe metadata and allocation, and LVM runs correctly on a
  read-only, udev-less node (V-1).
- **An empty `/etc/lvm` baked into the node.** Rejected: it works (V-1) but adds a provisioning
  detail the adapter's per-spawn `LVM_SYSTEM_DIR=/run/lvm` makes unnecessary.

## Consequences

- Positive: no operator step and no destructive Overdrive path; the provisioning contract is
  four items.
- Negative: LVM2 (GPL-2.0, libdevmapper LGPL-2.1) joins what a node carries; the node's provisioner
  sizes, labels and provisions the partition (the future image's share is tracked under GH #75), and
  the guest kit's provisioning is extended in DELIVER alongside the metal host's.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Validation V-1
- [ADR-0068](adr-0068-pinned-appliance-kernel-floor.md) (kernel pin; module-unload amendment), GH #75
- `PARTNAME` is exported by the kernel for a named GPT partition (`block/partitions/core.c:259 @ v7.0`).
