# ADR-0320 — An explicit disk image type skips format sniffing

## Status

**Accepted** (decision VD-7c, GH #348). Approved by the user on 2026-11-17.

**Amended 2026-11-20:** the fix (F3, an explicit `image_type` skips sniffing) is now upstream Cloud
Hypervisor's own behaviour (upstream commit `f2cedec65`), carried by the fork's move to upstream `main`
(ADR-0161 § Amendment). It is no longer a fork patch; the decision still holds. Where the text below says
the fork carries a patch, read it as upstream's behaviour at the pinned revision.

## Context

A volume is raw, and the guest owns its contents. A guest that writes a qcow2, VHDX or fixed-VHD marker
into its own volume makes the next boot fail even when `image_type=raw` is given, and shrinks the disk by
512 bytes when no `image_type` is given (spike E3). The fix is proven on branch
`spike/node-local-persistent-volume` against a stock control (`run-032`).

## Decision

**The fork does not sniff the format of a disk whose image type is given explicitly; it trusts the
declared type. The pinned fork revision and its checksums change (ADR-0161).**

## Alternatives considered

- **Upstream-only (no fork change).** Rejected: upstream sniffs regardless and Overdrive runs only the
  fork.
- **Work around it in Overdrive.** Rejected: the sniff is inside the VMM; a guest-written marker cannot
  be scrubbed by the platform, which does not own the guest's data.

## Consequences

- Positive: a guest cannot brick its own volume with a format marker.
- Negative: one patch to carry across fork rebases.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Disk attachment
- Spike findings E3; [ADR-0161](adr-0161-vendored-cloud-hypervisor-fork-is-built-pinned-and-provisioned-as-a-checksummed-release.md),
  [ADR-0315](adr-0315-a-volume-is-attached-as-a-second-raw-disk-with-write-back-direct-io-and-a-fixed-serial.md)
