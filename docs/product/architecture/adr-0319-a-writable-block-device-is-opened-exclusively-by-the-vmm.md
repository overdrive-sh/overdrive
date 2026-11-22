# ADR-0319 — The VMM opens a writable block device exclusively (`O_EXCL`)

## Status

**Accepted** (decision VD-7b, GH #348). Approved by the user on 2026-11-17.

## Context

Single writer on a volume rests on Cloud Hypervisor's OFD write lock, which is keyed on the device
node's inode (`posix_lock_inode` takes the lock context from the inode it is given, `fs/locks.c:1205-1220 @ v7.0`; spike B3). A second device node for the same block device (same
major:minor) is a different inode: in the spike a second VMM booted on it and both ran (probe B3).
The kernel's exclusive claim arbitrates per device: a second `O_EXCL` open through the alias gets `EBUSY`
(`block/bdev.c:598-611, 949-957 @ v7.0`). With the fix the same attempt fails `EBUSY` (`run-031`, closed
against a stock control). `O_EXCL` excludes only other claimants, so a stock opener is not stopped;
Overdrive runs no stock VMM. The fix is on branch `spike/node-local-persistent-volume`.

## Decision

**The fork opens a writable block device with `O_EXCL`, and exits with status 111 when that open fails with
`EBUSY`. The pinned fork revision and its checksums change (ADR-0161).**

The exit status is the typed signal for "another holder has the device". Cloud Hypervisor reads it from the
error chain: a `BlockError` of kind `Io`, operation `Open`, whose source is an `io::Error` with
`raw_os_error() == Some(EBUSY)`. It
does not match text. Every other start failure keeps exit status 1.
Metal evidence: a loop device held by an `O_EXCL` opener makes the fork exit 111; a nonexistent disk exits
1; a free device opens (spike `spike-scratch/node-local-persistent-volume-ch-exit/`, kernel 7.0.0-38).

## Alternatives considered

- **Upstream-only (no fork change).** Rejected: upstream does not open `O_EXCL` and Overdrive runs only
  the fork.
- **Rely on the OFD lock alone.** Rejected: the alias hole above is real. It is reachable only through
  an Overdrive path bug, but the kernel claim removes the exposure with one patch, and the same open is
  what `claim_state` (ADR-0317) arbitrates on.
- **A second Overdrive claim path on every device node.** Rejected: duplicates the kernel's claim in
  Overdrive code.

## Consequences

- Positive: single writer holds across any device node. For a block device the refusal is `EBUSY` from
  `open`, raised before the lock; it also means any other claimant of the device, and the message does
  not say which.
- Positive: the driver tells a held volume from every other start failure by the VMM's exit status, not by
  its stderr text.
- Negative: two small patches to carry across fork rebases (the open, and the exit status).

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Disk attachment
- Spike findings B; [ADR-0161](adr-0161-vendored-cloud-hypervisor-fork-is-built-pinned-and-provisioned-as-a-checksummed-release.md),
  [ADR-0317](adr-0317-a-replacement-waits-for-the-volume-claim-not-for-the-vmm-process.md)
