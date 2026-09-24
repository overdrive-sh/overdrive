# ADR-0128 — The VMM adapter opens, hands off, and closes the per-launch TAP queue descriptor

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R2. Proposed 2026-09-23 and revised 2026-09-24; reviewed by
independent DESIGN review rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. Depends on ADR-0127. Exact signatures live only in
the #295 feature delta.

## Context

ADR-0127 hands Cloud Hypervisor one inherited TAP queue descriptor. Some
component must open that descriptor, keep it for the shortest possible span,
and close its own copy. The kernel and the current types constrain that choice:

- A single-queue TAP admits one attached queue at a time. Any other attach
  returns `EBUSY`, including one made while the creator's descriptor is still
  open.
- The last attacher sets `IFF_VNET_HDR`, and the flag persists after detach. Any
  component that reattaches by name can therefore silently clear it.
- The kernel raises carrier at attach, so an attach onto a TAP that is still
  administratively up admits host frames before any VMM runs.
- `AllocationSpec`, `VmConfig`, and `VmNetworkAttachment` are cloneable,
  equality-comparable transient values. An owned descriptor is neither.
- Cloud Hypervisor duplicates the inherited descriptor several times and holds
  the queue until it exits. The launcher's copy may close as soon as the child
  has execed.

The first three facts are kernel semantics in `drivers/net/tun.c`, observed
natively by the persistent-TAP spike. Closing the parent's copy once the spawn
returns matches libvirt's `VIR_COMMAND_PASS_FD_CLOSE_PARENT` ("closed in the
parent no later than Run/RunAsync/Free") and the ownership model of the
`command-fds` mapping, which holds the descriptor until the command value is
dropped. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 1.2, 1.3 and 2.1.)

## Decision

`CloudHypervisorVmm::create` owns the descriptor for exactly one launch:

1. Immediately before spawning Cloud Hypervisor, it attaches one queue by name
   to the already-provisioned persistent TAP. The name comes from the existing
   `VmNetworkAttachment`, and the attach requests exactly TAP, no packet
   information, and virtio network headers.
2. It reads back the exact queue flags. They must show a persistent,
   single-queue TAP with virtio headers, which proves the attach joined the
   owner's TAP rather than creating a new device.
3. It reads back the TAP's administrative state and refuses unless the TAP is
   down.
4. It maps the descriptor to one fixed child descriptor number. The child
   inherits nothing else above standard I/O (ADR-0129).
5. It closes its own copy as soon as the spawn returns, before any further
   await, on every branch: success, a spawn error, and a spawned child that
   reports no pid. Because the mapping holds the descriptor, the spawn command
   value is dropped at that point. The failure branches' cleanup awaits run
   after the drop, never while the command value is alive.

The descriptor never crosses a crate or type boundary. `VmNetworkAttachment`,
`VmConfig`, and the `Vmm` trait keep their current shapes. The queue has one
holder from spawn to exit: the Cloud Hypervisor process.

The shared guest-network owner remains the sole owner of TAP creation,
persistence, bridge membership, administrative state, and deletion. No other
component attaches to a guest TAP by name. Each provisioned TAP receives at
most one launcher attach in its lifetime, because a VMM replacement always uses
a fresh allocation and therefore a fresh TAP.

## Alternatives considered

### `VmDriver` opens the queue and passes it into `Vmm::create`

Rejected. It widens the core `Vmm` contract, the `SimVmm` adapter, and the
equivalence harness. It also keeps an owned descriptor alive across the
driver's awaits and cancellation points, for no safety gain.

### The network owner opens the queue at the end of provision

Rejected. The descriptor would travel through `AllocationSpec`, which breaks
that value's clone and equality derives. The descriptor would also stay open
across the provision-to-start interval and its cancellation paths, giving one
queue two owners.

### Keep Cloud Hypervisor opening the TAP by name

Rejected by ADR-0127, because the named path raises the TAP.

## Consequences

Positive: the descriptor's lifetime is one short span inside one adapter. No
core or public type carries a descriptor, and the network owner's TAP authority
is unchanged.

Negative:

- `overdrive-host` gains a dependency on the existing TUN helpers in
  `overdrive-netlink`. This is a new adapter-to-adapter edge, and it is acyclic.
- The attach is by name, so an actor with host `CAP_NET_ADMIN` could replace
  the TAP between provision read-back and launch. The persistence and flag
  read-back rejects an accidental recreation. A malicious root actor is outside
  the threat model. Unprivileged by-name attaches are refused by the kernel
  because the TAP is owned by uid 0 (ADR-0130). The queue holder cannot change
  that owner: `TUNSETOWNER` returns `EPERM` to every Cloud Hypervisor thread
  (ADR-0143), and ADR-0130's audit read-back detects an owner change made
  anyway.
- `VmmError` gains typed queue-attach variants.
