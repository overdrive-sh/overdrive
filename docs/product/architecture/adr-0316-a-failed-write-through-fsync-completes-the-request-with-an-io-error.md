# ADR-0316 — A failed write-through fsync completes the guest's request with an I/O error on every engine

## Status

**Proposed** (decision VD-7a, GH #348). Not accepted until the user approves it.

## Context

Overdrive runs only its own Cloud Hypervisor fork (ADR-0161) and changes it when the platform needs
VMM behaviour the fork lacks. A guest can switch the virtio-blk cache mode itself, so a guest can reach
write-through whatever the platform exposes. In the stock build, when the host fsync behind a
write-through write fails, the error is discarded on the io_uring and AIO engines (20 of 20 failures
swallowed) and wedges the sync engine (spike A3). The fix is proven on branch
`spike/node-local-persistent-volume` (`docs/feature/node-local-persistent-volume/spike/fork-patches/`)
against a stock control (`run-030`).

## Decision

**The fork completes a guest request whose write-through fsync failed with an I/O error on every I/O
engine, instead of reporting success. The pinned fork revision and its checksums change (ADR-0161).**

## Alternatives considered

- **Upstream-only (no fork change).** Rejected: upstream has no such change and Overdrive runs only the
  fork.
- **Work around it in Overdrive.** Rejected: the failed fsync is not observable from outside the VMM.
- **Forbid write-through by not exposing a guest-writable cache mode.** Rejected: the guest selects the
  cache mode, so the platform cannot withhold write-through; and a failed flush reported as success is a
  silent data-loss path for a volume the user keeps.

## Consequences

- Positive: a failed flush is never reported to the guest as success.
- Negative: one patch to carry across fork rebases; the pin and checksums change in the same delivery
  step as the other block-path patches (ADR-0319, ADR-0320).

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Disk attachment
- Spike findings A3; [ADR-0161](adr-0161-vendored-cloud-hypervisor-fork-is-built-pinned-and-provisioned-as-a-checksummed-release.md),
  [ADR-0315](adr-0315-a-volume-is-attached-as-a-second-raw-disk-with-write-back-direct-io-and-a-fixed-serial.md)
