# ADR-0314 — The volume store is probed at boot and refuses startup when it is broken

## Status

**Proposed** (decision VD-5, GH #348). Not accepted until the user approves it.

## Context

Earned Trust: an adapter that depends on the kernel, device-mapper and an LVM binary must
show it can honour its contract on the node where it runs, before the node accepts work
(the same obligation as `Vmm::probe`, ADR-0082 D5). The appliance has no operator to repair a
half-working volume store, and a broken store loses user data silently. The kernel is
Overdrive's own pinned image, so the kernel's claim and removal behaviour is read in source
and proven by Tier-3 tests, not probed at every boot.

## Decision

**The volume store adapter has a `probe` that runs at boot, before any VM workload is
placed (wire, then probe, then use). A node with no partition labelled `overdrive-volumes`
composes no volume store (soft: event `volume_backing.not_composed`; a workload that
declares a volume then fails its start with a typed cause). A node that has the partition
and fails the probe refuses to start with the structured event `health.startup.refused`,
reason `volume_backing.probe`.**

The probe fails on: device-mapper or the LVM tools unusable; the volume group `overdrive`
missing from the labelled partition (or the label matched by more than one partition); a scratch logical volume that cannot be created, whose
device node does not appear, or that cannot be removed. It removes a scratch volume left by a crashed earlier probe first, and
skips the scratch step (with an event) when the pool has no free extent, so a full pool does
not refuse startup.

## Alternatives considered

- **Probe lazily at first volume creation.** Rejected: the failure would surface per
  workload long after boot, with no node-level refusal.
- **Always refuse when the partition is absent.** Rejected: a node with no volume workloads
  is legitimate.
- **Trust the exit status of the LVM commands.** Rejected: a created volume whose node never
  appears is exactly a case where the command succeeds and the substrate lied.
- **Probe kernel claim and removal semantics at every boot.** Rejected: the pinned kernel
  cannot differ at run time; those are Tier-3 tests.

## Consequences

- Positive: a node either honours the volume contract or does not start.
- Negative: boot runs a scratch create/remove cycle (about 95 ms measured in V-1).

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Probe
- [ADR-0082](adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md) D5, [ADR-0313](adr-0313-the-image-provides-device-mapper-lvm-tools-and-a-labelled-volume-partition.md)
