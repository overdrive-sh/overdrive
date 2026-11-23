# ADR-0350 — Boot reclamation reserves the guest address of a stuck VM and lets the node boot

## Status

**Accepted** 2026-11-23 (decision RB-4 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-23, decision 14, option c: "the
node should not refuse to start just because of a dead vm"). Not implemented. Amends ADR-0156 (implemented; the amendment is explicit there). The
interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "Boot proceeds past a stuck VM".

## Context

After a `serve` crash, boot reclamation (`vm_reclamation_boot::converge`) kills every VMM the new process does not supervise and removes its scope
before the first launch. A VMM whose last task is stuck in exit (a held flush on a failing or suspended disk) keeps its scope populated, its
vhost-vsock device and so its guest CID (`GUEST_CID_BASE` plus the lease offset, ADR-0156). Any executor error, a busy scope included (ADR-0349), is
`ConvergeError::Reclamation` and `serve` refuses to start. There is no operator on the appliance, so the node restarts until the I/O drains, and
every other workload on the node is down meanwhile because of one dead VM.

The refusal protected the CID: after a restart no lease exists (ADR-0118), so a launch could be handed the CID the stuck VMM still holds and fail
`EADDRINUSE` (`vhost_vsock_set_cid`, `drivers/vhost/vsock.c:804-813 @ v7.0`). That reason is about the CID, not about whether the node may boot. A
reservation of the offset removes it.

## Decision

**Boot reclamation never refuses the boot because a leftover VM is stuck.** For each leftover scope it waits the bound (ADR-0349), in parallel
across scopes. For each scope still populated at the bound it reserves the guest-address offset the stuck VMM holds in `GuestAddressPool`, before the
first `assign`, and the boot continues. Cleanup of the scope retries on later passes as for any scope. The reservation is a Retiring lease with no
network; it is released by the one release site of every lease, once the scope has reported empty and been removed (ADR-0156, second amendment).

The offset comes from the allocation's row ([ADR-0353](adr-0353-a-vms-guest-address-is-recorded-on-its-pending-row-before-the-vmm-exists.md)),
never from the VMM. The row of a busy scope is written by
[ADR-0354](adr-0354-a-busy-boot-scope-writes-its-terminal-row.md).

## Alternatives considered

- **Keep the refusal.** Rejected by the user: one dead VM must not take the node's other workloads down, and with no operator the node does not
  recover until the I/O drains.
- **Boot proceeds and the CID is not protected.** Rejected: a launch whose next-fit offset is the stuck VMM's fails `EADDRINUSE` and is charged as a
  start failure, and ADR-0156's "no production path produces that state" becomes false.
- **Claim the CID on the kernel at boot.** Rejected (ADR-0156, CID-NOCLAIM): a descriptor `serve` holds past the VMM's exit keeps host flows
  established to a dead guest.
- **Reserve every leftover offset before the kill and release those that settle.** Rejected: it adds a second state and a boot-time release site for
  the scopes that settle, and nothing is lost by reserving only the scopes still busy, because no `assign` runs between the kill and the reservation.

## Consequences

- `serve` boots with a stuck VM. The stuck VM costs one attachment slot until it is reaped; reservations count toward the attachment cap, so the cap
  stays truthful, and enough stuck VMs stall launches node-wide, as for held leases (ADR-0348).
- The VM is visible through existing surfaces: its scope stays in `VmHostState::observe()`, its allocation's row is terminal and carries the
  address, and the attachment occupancy `describe` shows counts the reservation (ADR-0336). No new field, event or surface.
- ADR-0156's "boot reclamation completes before the first `assign`" becomes "boot reclamation, and the reservation of every VM it could not remove,
  complete before the first `assign`".
- A reservation is process-local, as a lease is (ADR-0118); a later boot derives it again from the rows.
- `VmHostState` gains no method.
