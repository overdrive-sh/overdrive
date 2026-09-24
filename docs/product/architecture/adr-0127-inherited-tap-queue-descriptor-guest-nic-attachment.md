# ADR-0127 — Attach each guest NIC through an inherited TAP queue descriptor so the TAP stays down until intercept-live

## Status

**Accepted (2026-09-24).** GH #295 correctness-recovery replacement DESIGN,
decision D-295-R1. Proposed 2026-09-23; reviewed by independent DESIGN review
rounds 3, 4, and 5 and the round-5 verification
(`arch_rev_20260924_netns295_r5_verify`); accepted by the user on 2026-09-24
with the replacement DESIGN. It amends ADR-0088's #295 realization and
supersedes ADR-0089's named-TAP attachment for the shared bridge (each states
this explicitly). Exact implementation-facing contracts live only in the #295
feature delta (§ *Correctness-Recovery Replacement DESIGN*).

## Context

The zero-frame invariant is an accepted security outcome: no guest-originated
L2 frame may reach the host or the shared bridge before the allocation's
transparent intercept is live and read back. It is operative for the shipped
#222 guest-stack path (ADR-0088) and is required by the #295 born-captured
outcome. The recovery charter forbids weakening it to preserve the current
VMM surface.

Three native facts bound the choice:

- With the provisioned TAP administratively up, a configured guest kernel
  answers traffic it receives. Native run `c4d36190` captured guest-source ARP
  replies and a TCP reset before the exact `mtls.intercept.install.success`
  event, even though guest IPv6 and gratuitous ARP were suppressed.
- Cloud Hypervisor v53's named `--net tap=` path always raises the TAP. Its
  named open calls `Tap::enable`, whose `SIOCSIFFLAGS` requires
  `CAP_NET_ADMIN`. Under the accepted confinement it fails before READY with
  `EPERM` (native run `f1a15668`).
- Cloud Hypervisor v53's `--net fd=[N]` path imports queue descriptors and never
  calls the named enable. Two native spikes reached the real guest READY beacon
  with the TAP down, zero captured frames and zero interface counters. The first
  used a non-persistent TAP; the second used the production persistent-TAP
  lifecycle. The owner then activated the TAP after interception was live, and
  every captured frame followed that barrier. Teardown restored the exact empty
  complement.

Handing a VMM a TAP queue descriptor opened by a privileged parent is
established practice: QEMU's `-netdev tap,fd=`, libvirt's
`virCommandPassFD`, Kata Containers sending descriptors to Cloud Hypervisor
over `SCM_RIGHTS`, and Android AVF's `virtmgr` handing crosvm a `tap-fd=`.
Cloud Hypervisor's own source keeps the two paths apart: `from_tap_fds`
duplicates the descriptor and never raises the link, while only the named
`open_tap` path calls `tap.enable()`. Firecracker is the exception; it opens the
TAP by name inside a per-VM network namespace. (Research:
`docs/research/networking/netns-density-295-replacement-design-prior-art-comprehensive-research.md`,
Findings 1.1 and 1.2.) Holding the host TAP administratively down across guest
boot as the enforcement gate has no external precedent (Finding 3.3); it rests
on the native evidence above and on the kernel's separation of carrier from
administrative state (Finding 2.1).

ADR-0089 §A2 rejected fd-passing for the #222 per-workload namespace topology.
Its three reasons do not carry over to #295:

- **Isolation.** A2 kept the VMM inside the workload namespace. #295's accepted
  design already runs the VMM in the host namespace.
- **Reboot fragility.** Cloud Hypervisor v53 duplicates every inherited
  descriptor so the queue survives guest reboot.
- **Complexity.** A2 would have needed a cross-namespace `setns` helper and
  SCM_RIGHTS plumbing. The #295 launcher inherits the descriptor at spawn and
  needs neither.

Under the recovery charter, an alternative rejected for reasons the new
topology and evidence invalidate is reopened.

The persistent-TAP spike adds two conditions. The attach must request virtio
network headers (`IFF_VNET_HDR`). Without them the guest boots and exits
cleanly, but every frame carries a corrupt 12-byte prefix. The TAP must also be
administratively down at every attach, because the kernel raises carrier on
attach. Kernel source confirms both: a single-queue attach overwrites the
feature flags, including `IFF_VNET_HDR`, and `tun_set_iff` turns carrier on
unless the attacher asks for `IFF_NO_CARRIER` (Finding 2.1).

## Decision

Cloud Hypervisor receives the guest NIC as exactly one inherited TAP queue
descriptor through its supported `--net fd=` path. It never receives the TAP
name. The TAP stays administratively down through VMM start, guest READY, and
the accepted Running row. Only the shared guest-network owner raises it, after
intercept-live (ADR-0131).

The queue is attached only while the TAP is administratively down, and always
with virtio network headers. The attachment is one queue pair; multiqueue is
outside this decision. Guest network configuration, READY, Running, and EXEC
keep their existing meanings.

## Alternatives considered

### Keep the named TAP up and admit a closed set of pre-event control frames

Rejected. This was the uncommitted D-295-DELIVER-04-01 revision. It replaces
the zero-frame invariant with an allowlist of correlated ARP replies and
zero-payload TCP resets. That weakens an accepted security outcome to fit the
current VMM surface, which the charter forbids. Its stated reason, that fd
handoff was unnecessary, no longer holds once the handoff is proven.

### Grant Cloud Hypervisor `CAP_NET_ADMIN`

Rejected. Cloud Hypervisor would raise the TAP itself at device open, before
READY and before interception, so the grant does not restore the invariant. It
also widens VMM confinement.

### Hot-plug the NIC after intercept-live

Rejected. The guest applies its static network configuration before READY, so a
NIC added after READY would change READY's meaning and require a guest-init and
beacon redesign.

### Keep today's order and accept the observed frames

Rejected. Native evidence shows the invariant is violated.

## Consequences

Positive: the barrier becomes structural. A down TAP drops any guest write and
Cloud Hypervisor never mutates host link state. The zero-frame invariant is
restored without changing READY, Running, or EXEC semantics.

Negative: VMM launch gains a descriptor-inheritance channel, decided in
ADR-0128 and ADR-0129. The vnet-header condition is invisible to launch success,
because Cloud Hypervisor boots and exits 0 without it, so only native L2 or
traffic evidence can prove it. The decision covers exactly one queue pair per
guest.
