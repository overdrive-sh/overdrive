# ADR-0353 — A VM's guest address is recorded on its `Pending` row before the VMM exists

## Status

**Accepted** 2026-11-23 (decision RB-6 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-23, N-2: "save the address when it
is handed out"). Not implemented. Amends ADR-0156 (implemented; the amendment is explicit there). The interface contract is in
[`feature-delta.md`](../../feature/node-local-persistent-volume/feature-delta.md) § "The guest address is on record before the VMM exists".

## Context

Boot reclamation reserves the guest-address offset of a VM it cannot remove ([ADR-0350](adr-0350-boot-reclamation-reserves-the-guest-address-of-a-stuck-vm-and-lets-the-node-boot.md)).
The offset is the VM's `workload_addr` within the node prefix, and the CID is derived from it (ADR-0156). Boot needs a source for that address for
every VMM that can hold a CID, including a launch parked in guest boot when `serve` crashes, a VMM already exiting at boot, and a VMM stopped before
the crash and killed-not-reaped.

## Decision

**When the pool hands out an offset for a VM allocation and the network is provisioned, the action shim writes the allocation's row in the existing
`Pending` state with `workload_addr` set, before `Vmm::create` is reachable.** Every later row of the allocation carries the address forward. Boot
reads the offset to reserve from the row and from nowhere else; the VMM's command line is never read and `VmHostState` gains no method. A launch
whose `Pending` write fails creates no VMM.

A `Pending` row is a launch record, not a placement: readers that act on a running allocation ignore it. No persisted format changes (the field and
the state exist).

## Alternatives considered

- **Read `--vsock cid=` from the VMM's command line for a scope with no address.** Rejected by the user. It needs a new port read, works only while
  the VMM is alive, and leaves a window: a VMM already exiting when boot reads it has neither source.
- **Forward-carry the address on terminal rows only.** Rejected: it leaves the launch parked in guest boot, which has a VMM and no row.
- **A new row state or field for the address.** Rejected: the existing `Pending` state and `workload_addr` field represent it, so no envelope bump or
  fixture is needed.

## Consequences

- Every launch costs one more observation write, before the VMM is created; it is one write on the launch path, which is not a hot path.
- A launch whose `Pending` write fails retires and tears down its lease and returns the typed error; no VMM that can hold a CID exists without a row
  that names its address.
- A `Pending` row must never outlive its launch: [ADR-0355](adr-0355-a-launch-that-ends-before-running-ends-its-pending-row-with-a-terminal-row-that-names-why.md).
- The stop, `FinalizeFailed`, failed-start and reclamation rows carry the prior row's address.
- ADR-0156's "boot reclamation precedes every `assign`" has its input: the address of every VMM that can hold a CID is on a row.
