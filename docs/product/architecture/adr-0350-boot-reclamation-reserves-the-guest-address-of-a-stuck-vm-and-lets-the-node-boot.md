# ADR-0350 — Boot reclamation reserves the guest address of a stuck VM and lets the node boot

## Status

**Accepted** 2026-11-23 (decision RB-4 of the node-local persistent volume feature, GH #348; the user's ruling of 2026-11-23, decision 14,
option c, in the user's words: "the node should not refuse to start just because of a dead vm"). The mechanism was worked out by three user
rulings of the same day: N-2 (the guest address is saved on the allocation's row when the lease hands it out, before the VMM is created, so
boot reads it from the row and never from the VMM), N-3 (a busy boot scope writes its terminal row once the kill is delivered) and N-1
(ADR-0349: one remover of a VM scope). Not implemented, so the text is in the present tense. Amends ADR-0156 (implemented; the amendment is
explicit there) and replaces the fail-closed boot clause of ADR-0349 as first written. The interface contract is pinned in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "Boot proceeds past a stuck VM".

One detail of the mechanism is proposed and awaits the user's ruling (feature-delta, N-4): how the workload lifecycle reconciler treats a
`Pending` row that no launch is completing.

## Context

After a `serve` crash, boot reclamation (`vm_reclamation_boot::converge`) kills every VMM the new process does not supervise and removes its
scope before the first launch. A VMM whose last task is stuck in exit (a held flush on a failing or suspended disk) keeps its scope populated,
its vhost-vsock device and so its guest CID (`GUEST_CID_BASE` plus the lease offset, ADR-0156). Today any executor error, a busy scope included
(ADR-0349), is `ConvergeError::Reclamation` and `serve` refuses to start. There is no operator on the appliance, so the node restarts until
the I/O drains, and every other workload on the node is down meanwhile because of one dead VM.

Refusing was kept because after a restart no lease exists (ADR-0118): a launch could be handed the CID the stuck VMM still holds and fail
`EADDRINUSE` (`vhost_vsock_set_cid`, `drivers/vhost/vsock.c:804-813 @ v7.0`). That reason is about the CID, not about whether the node may
boot. A reservation of the offset removes it. The reservation needs to know the offset, and a VMM that exists must have its offset on record.

## Decision

**Boot reclamation never refuses the boot because a leftover VM is stuck.** For each leftover scope it waits the bound (ADR-0349), in
parallel across scopes. For each scope still populated at the bound it reserves the guest-address offset the stuck VMM holds in
`GuestAddressPool` before the first `assign`, surfaces the VM as node-visible state, and the boot continues. Cleanup of the scope retries on
later passes as for any scope. The reservation is released when the scope has reported empty and was removed (ADR-0349), the same gate as a
lease (ADR-0348).

**The address is on record before the VMM can hold it.** When the pool hands out an offset for a VM allocation and the network is provisioned,
the action shim writes the allocation's row in the existing `Pending` state with `workload_addr` set, before `Vmm::create` is reachable. So
every VM that can hold a CID has its address on an `AllocStatusRow`, including a launch parked in guest boot when `serve` crashes. Every later
row of that allocation carries the address forward (the `Running` row sets it, and every terminal or failed row copies it from the prior row).
Boot reads the offset from the row: `workload_addr - GUEST_NODE_PREFIX.network()`, the CID being `GUEST_CID_BASE + offset`. A scope maps to its
row by name, `<alloc_id>.scope`. The row is readable at boot: the observation store opens in the composition root before `run_server`, and
the pool is built before reclamation runs. The VMM's command line is never read.

A `Pending` row is a launch record, not a placement: readers that act on a running allocation ignore it (ADR-0276's backend hydration gates on
`Running`), and the workload lifecycle reconciler does not treat it as the workload's current allocation (feature-delta § "Boot proceeds past a
stuck VM", the reader table).

**A busy scope writes its terminal row.** When the kill is delivered and the scope stays populated at the bound, the boot executor step writes
the allocation's terminal row, carrying the address, as ADR-0348 does for an unreaped stop. The VMM is killed and can run no guest code, so the
row is true, and the stuck VM's workload restarts at once, its replacement waiting on the volume claim (ADR-0317). The artifact discard is left
to `VmReclamation`. This is the one exemption from ADR-0349's "busy means no row write".

## Alternatives considered

- **Keep the refusal (the ADR-0349 clause as first written).** Rejected by the user: one dead VM must not take the node's other workloads
  down, and with no operator the node does not recover until the I/O drains.
- **Boot proceeds and the CID is not protected.** Rejected: a launch whose next-fit offset is the stuck VMM's fails `EADDRINUSE` and is charged
  as a start failure, ADR-0156's "no production path produces that state" becomes false.
- **Claim the CID on the kernel at boot.** Rejected (ADR-0156, CID-NOCLAIM): a descriptor `serve` holds past the VMM's exit keeps host flows
  established to a dead guest.
- **Read the offset from the VMM's command line (`--vsock cid=`) for a scope with no address on a row.** Rejected by the user (ruling N-2). It
  needs a new port read, works only while the VMM is alive, and leaves a window: a VMM that is already exiting when boot reads it has neither
  source, so it is not reserved. Recording the address first closes the window and needs no port read.
- **Read the offset from the VMM only, for every scope.** Rejected: a VMM stopped before the crash and killed-not-reaped has already lost its
  command line; only a row has the address then.
- **Reserve nothing and accept a clash at launch.** Same as "not protected" above.
- **Leave the row `Running` until the scope empties, for a busy boot scope (ADR-0349's first text).** Rejected by the user (ruling N-3): the row
  would say a VM that can run no guest code is running, and its workload would not restart until the reap.

## Consequences

- `serve` boots with a stuck VM. The stuck VM costs one attachment slot until it is reaped; reservations count toward the attachment cap, so
  the cap stays truthful. Enough stuck VMs stall launches node-wide, as for held leases (ADR-0348).
- The VM is visible as node-visible state: the scope stays in `VmHostState::observe()`, its allocation's row is terminal and carries the
  address, and the attachment occupancy `observe` reports counts the reservation (`describe` shows the held count, ADR-0336). No new field or
  surface.
- Every launch costs one more observation write, before the VMM is created (the `Pending` row). It is one write on the launch path, which is
  not a hot path.
- A launch whose `Pending` write fails does not create a VMM: its lease is retired and torn down and the dispatch returns the typed error. No
  VMM that can hold a CID exists without a row that names its address.
- ADR-0156's "boot reclamation completes before the first `assign`" becomes "boot reclamation, and the reservation of every VM it could not
  remove, complete before the first `assign`".
- A pool reservation is process-local, as a lease is (ADR-0118). A later boot derives it again from the rows.
- `VmHostState` gains no method. A `Pending` row left by a crash is ended by the existing paths: boot reclamation's reclaim executor for one
  whose VM left artifacts, and the lifecycle reconciler for one that did not (feature-delta, same section).
