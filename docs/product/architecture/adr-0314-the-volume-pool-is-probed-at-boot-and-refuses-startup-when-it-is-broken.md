# ADR-0314 — The volume pool is probed at boot and refuses startup when it is broken

## Status

**Accepted** (decision VD-5, GH #348). Approved by the user on 2026-11-17, including that a
volume pool that is present but broken refuses startup.

## Context

Earned Trust: an adapter that depends on the kernel, device-mapper and an LVM binary must
show it can honour its contract on the node where it runs, before the node accepts work
(the same obligation as `Vmm::probe`, ADR-0082 D5). The appliance has no operator to repair a
half-working volume pool, and a broken pool loses user data silently. The kernel is
Overdrive's own pinned image, so the kernel's claim and removal behaviour is read in source
and proven by Tier-3 tests, not probed at every boot.

## Decision

**The volume pool adapter has a `probe` that runs at boot, before any VM workload is
placed (wire, then probe, then use). A node with no partition labelled `overdrive-volumes`
composes no volume pool (soft: event `volume_backing.not_composed`; a workload that
declares a volume then fails its start with a typed cause). A node that has the partition
and fails the probe refuses to start with the structured event `health.startup.refused`,
reason `volume_backing.probe`.**

The probe runs under one deadline owned by the adapter, the same 30 s as the `ensure` deadline (ADR-0318):
one bound over the whole probe, not per command. A probe that outlives it returns a typed probe error
(the failing stage's scenario, source kind `TimedOut`) and startup is refused as for a broken pool; a
stalled `lvm` is never killed. Without the bound, a hung `lvm` at boot (for example on a lock left by a
crash) would hang `overdrive serve` forever on an appliance with no operator shell (decision P-4,
approved by the user on 2026-11-21).

The probe fails on: the partition discovery read of `/sys/class/block` failing (scenario
`SysfsUnreadable`, decision P-5, approved 2026-11-21); device-mapper or the LVM tools unusable; the volume group `overdrive`
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
- [ADR-0082](adr-0082-vmm-port-trait-and-vmconfig-anti-corruption-value.md) D5, [ADR-0313](adr-0313-node-provisioning-provides-the-volume-pool-on-a-labelled-partition.md)
