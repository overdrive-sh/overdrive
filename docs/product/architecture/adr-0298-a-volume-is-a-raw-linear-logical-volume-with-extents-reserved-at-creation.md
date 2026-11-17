# ADR-0298 — A volume is a raw linear logical volume with extents reserved at creation

## Status

**Proposed** (decision VD-3, GH #348). Not accepted until the user approves it.
Validation item V-1 passed (feature delta § Validation).

## Context

The spike (`docs/feature/node-local-persistent-volume/spike/findings.md`, probes A,
C, D, E, run on the native metal host) compared a file on the reflink XFS data
filesystem with a `dm-linear` logical volume under the same confined VMM. Durability
was equal. For fsync-bound work the block device was 1.2x to 2.5x faster (median,
n = 10 per cell, two substrates). A file volume larger than the derived
`RLIMIT_FSIZE` (128 MiB in the spike) kills the VMM with SIGXFSZ or fails guest writes,
while a block device is exempt (`block/fops.c:746-783 @ v7.0`). A sparse file gives
the guest EIO partway through writes when the filesystem fills (probe E1). dm refuses
to remove an open device (`drivers/md/dm.c:357-372 @ v7.0`). A `WorkloadId` is up to
253 characters, while a logical-volume name is at most 128. On a read-only, udev-less node,
LVM creates the device node itself and a SIGKILL at any point of `lvcreate` or `lvremove`
leaves a volume either absent or present, never half-built, with no leaked extent (V-1).

## Decision

**A volume is one linear logical volume (a `dm-linear` device) in the volume group
`overdrive`, on a dedicated partition. Its extents are allocated and reserved when it
is created; there is no sparse or over-committed volume. Its name is `ovd-` followed by
the first 32 hexadecimal characters of the SHA-256 of the workload id. The volume lives
outside the reflink data filesystem and outside every clone and run-directory surface.**

A guest discard becomes a TRIM to the device and does not release the volume's
extents, so Cloud Hypervisor's default `sparse=true` stays on and there is no separate
sparse-versus-reserved choice.

## Alternatives considered

- **A file on the XFS data filesystem (preallocated or sparse).** Rejected: slower on
  fsync-bound work, needs a derived `RLIMIT_FSIZE` at least the largest volume, and
  ENOSPC arrives mid-flight when sparse.
- **A thin-provisioned logical volume.** Rejected: needs `dm-thin` (a module in the spike
  kernel; the image is built with module unload off), a pool and metadata-device
  lifecycle, and over-commit reintroduces ENOSPC-mid-flight.
- **A loop device over a file.** Rejected: an extra layer that keeps the file's
  `RLIMIT_FSIZE` and sparse behaviour, and direct I/O can silently fall back.
- **One static partition per volume.** Rejected: partitions cannot be created and removed
  at run time per workload.
- **The workload id embedded in the logical-volume name.** Rejected: 253 > 128 characters.
- **A registry of volume names.** Rejected: a second source of truth beside the volume
  group's own atomic metadata; the name is a pure function of the workload id.

## Consequences

- Positive: space is reserved by construction; no ENOSPC mid-flight; no `RLIMIT_FSIZE`
  interaction; removal of an in-use volume is refused by the kernel.
- Negative: the image must provide device-mapper, LVM tooling and a provisioned volume
  group (ADR-0299); a volume needs a node made accessible to the confined VMM uid.
  A 128-bit name collision would alias two workloads; at 2^-64 per pair this is accepted.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md)
- Spike findings A, C1, C2, D, E1, E2
