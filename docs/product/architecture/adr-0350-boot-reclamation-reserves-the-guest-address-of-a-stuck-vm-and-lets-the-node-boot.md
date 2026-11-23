# ADR-0350 — Boot reclamation reserves the guest address of a stuck VM and lets the node boot

## Status

**Accepted** 2026-11-23 (decision RB-4 of the node-local persistent volume feature, GH #348; the user's ruling of 2026-11-23, decision 14,
option c, in the user's words: "the node should not refuse to start just because of a dead vm"). Not implemented, so the text is in the
present tense. Amends ADR-0156 (implemented; the amendment is explicit there) and replaces the fail-closed boot clause of ADR-0349 as first
written. The interface contract is pinned in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "Boot proceeds past a stuck VM".

Three details of the mechanism are proposed and await the user's ruling (feature-delta, N-1 to N-3): who removes a VM scope at a stop, the
exposure for a launch with no row whose VMM is already exiting, and the terminal row written for a busy boot scope.

## Context

After a `serve` crash, boot reclamation (`vm_reclamation_boot::converge`) kills every VMM the new process does not supervise and removes its
scope before the first launch. A VMM whose last task is stuck in exit (a held flush on a failing or suspended disk) keeps its scope populated,
its vhost-vsock device and so its guest CID (`GUEST_CID_BASE` plus the lease offset, ADR-0156). Today any executor error, a busy scope included
(ADR-0349), is `ConvergeError::Reclamation` and `serve` refuses to start. There is no operator on the appliance, so the node restarts until
the I/O drains, and every other workload on the node is down meanwhile because of one dead VM.

Refusing was kept because after a restart no lease exists (ADR-0118): a launch could be handed the CID the stuck VMM still holds and fail
`EADDRINUSE` (`vhost_vsock_set_cid`, `drivers/vhost/vsock.c:804-813 @ v7.0`). That reason is about the CID, not about whether the node may
boot. A reservation of the offset removes it.

## Decision

**Boot reclamation never refuses the boot because a leftover VM is stuck.** For each leftover scope it waits the bound (ADR-0349), in
parallel across scopes. For each scope still populated at the bound it determines which guest-address offset the stuck VMM holds, reserves
that offset in `GuestAddressPool` before the first `assign`, surfaces the VM as node-visible state, and the boot continues. Cleanup of the
scope retries on later passes as for any scope. The reservation is released when the scope has reported empty and was removed (ADR-0349), the
same gate as a lease (ADR-0348).

**Where the offset comes from.** The offset is `workload_addr - GUEST_NODE_PREFIX.network()` and the CID is `GUEST_CID_BASE + offset`.
`workload_addr` is read from the allocation's observation row. The row is readable at boot: the observation store opens in the composition
root before `run_server`, and the pool is built before reclamation runs. The scope name is `<alloc_id>.scope`, so a scope maps to its row.
Two cases leave a row with no address, and each is closed at its source:

- **Terminal rows.** A terminal row keeps the address the allocation held: the stop, `FinalizeFailed` and the reclamation executor
  forward-carry it instead of writing `None`. Service backend hydration already gates on `state == Running` (ADR-0276), so a carried address
  on a non-Running row is never a backend.
- **No row.** A launch parked in guest boot has a VMM and no row. The offset is read from the VMM itself: its command line carries
  `--vsock cid=<cid>,backend=vhost-kernel`, and the VMM is alive when boot reclamation starts, before it writes `cgroup.kill`. The read is
  taken before the kill, for scopes whose row gives no address.

## Alternatives considered

- **Keep the refusal (the ADR-0349 clause as first written).** Rejected by the user: one dead VM must not take the node's other workloads
  down, and with no operator the node does not recover until the I/O drains.
- **Boot proceeds and the CID is not protected.** Rejected: a launch whose next-fit offset is the stuck VMM's fails `EADDRINUSE` and is charged
  as a start failure, ADR-0156's "no production path produces that state" becomes false.
- **Claim the CID on the kernel at boot.** Rejected (ADR-0156, CID-NOCLAIM): a descriptor `serve` holds past the VMM's exit keeps host flows
  established to a dead guest.
- **Persist the address before the VMM is created, for the no-row case.** Rejected. A row before the launch means "an allocation was observed"
  to the restart-generation logic (ADR-0335) and a superseded launch writes no row (ADR-0334); an earlier row changes both. A separate record
  is a second source for a fact the lease and the VMM already carry, and a write on every launch for a crash window.
- **Read the offset from the VMM only, for every scope.** Rejected: a VMM stopped before the crash and killed-not-reaped has already lost its
  command line (its address space is gone); only the row has the address then.
- **Reserve nothing and accept a clash at launch.** Same as "not protected" above.

## Consequences

- `serve` boots with a stuck VM. The stuck VM costs one attachment slot until it is reaped; reservations count toward the attachment cap, so
  the cap stays truthful. Enough stuck VMs stall launches node-wide, as for held leases (ADR-0348).
- The VM is visible as node-visible state: the scope stays in `VmHostState::observe()`, its allocation's row is terminal (the kill is
  delivered; proposed, ruling N-3), and the attachment occupancy `observe` reports counts the reservation (`describe` shows the held count,
  ADR-0336). No new row, field or surface.
- ADR-0156's "boot reclamation completes before the first `assign`" becomes "boot reclamation, and the reservation of every VM it could not
  remove, complete before the first `assign`".
- A pool reservation is process-local, as a lease is (ADR-0118). A later boot derives it again.
- `VmHostState` gains one read, the guest CID of a scope's VMM from its command line, used only by boot reclamation.
