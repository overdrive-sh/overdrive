# ADR-0146 — Every VM's vsock device uses the kernel vhost-vsock backend of the vendored Cloud Hypervisor; any other backend is a typed refusal

## Status

**Proposed — decision D2 approved by user 2026-10-05; the launch on a device
the admission lease already claimed (D16-CLAIM, ADR-0170) approved by user
2026-10-07; pending independent DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*.

The VMM path inside this decision is already ruled: **D13 = (a), APPROVED
2026-10-05** — the vendored Cloud Hypervisor fork
(`overdrive-sh/cloud-hypervisor`, branch `overdrive/vhost-kernel-vsock`,
commits `9b68dbb57` and `41a619d19` on v53.0) is the production VMM path.
How that fork is built, pinned and provisioned is ADR-0161.

When accepted, this ADR **supersedes ADR-0128** (the VMM adapter owns the TAP
queue fd). CID derivation is ADR-0156; the kernel claim of the CID and its
handoff to the VMM is ADR-0170; the beacon listener is ADR-0157.

## Context

The forwarding evidence (ADR-0145) exists only for kernel vhost-vsock: the
host kernel services the guest's virtqueues, and host endpoints are
`AF_VSOCK` sockets a SockHash can hold.

Stock Cloud Hypervisor v53.0 cannot provide that backend. Its `--vsock`
always constructs the userspace Unix-socket muxer `VsockUnixBackend`, accepts
only `cid, socket, iommu, id, pci_segment, pci_device_id`, and documents
stream-only support. Host `AF_VSOCK` sockets do not join the muxer's
contexts, and the muxer admits at most 1,023 connections per VMM.

The fork adds `--vsock cid=N,backend=vhost-kernel`
(`spike/ch-vhost-vsock-findings.md`, WORKS, on native metal with seccomp on,
kernel 7.0.0-29 host and guest):

- an unmodified guest bound CID 42; a `vhost-<pid>` kernel worker serviced
  its queues; CH held `/dev/vhost-vsock` and no vsock Unix socket;
- STREAM and SEQPACKET were byte-exact both ways; CH made zero payload
  syscalls, against 8,413,773 bytes on the Unix backend for the same
  transfer;
- two VMs were isolated; a duplicate CID exited non-zero with `Failed to
  assign guest CID 101 to vhost-vsock` caused by `EADDRINUSE`, and the first
  VM kept working;
- forged guest binds to CIDs 142, 2 and 1 failed with `EADDRNOTAVAIL`; the
  host saw the assigned CID on every accepted connection;
- the CID was released and immediately reusable after poweroff, `kill -9`,
  in-guest reboot and hot-unplug (20/20 immediate relaunches);
- snapshot is refused with a typed CH error; while paused, host connects time
  out instead of queueing;
- the default Unix backend is behaviourally unchanged.

The CH run was a spike harness, not `overdrive serve`. It ran CH without the
production launch hook, uid drop, launch seccomp filter or Landlock rules.

## Decision

- **Backend.** Each VM has exactly one virtio-vsock device, launched with
  `backend=vhost-kernel` on the `/dev/vhost-vsock` instance whose CID the
  admission lease already claimed (ADR-0156, ADR-0170), passed to the fork as
  an inherited descriptor. Both the beacon and every application flow cross
  it.
- **No fallback.** The VMM adapter never launches a VM with the Unix backend,
  with no vsock device, with a second vsock device, or without a claimed
  device.
- **Typed refusal at startup.** `Vmm::probe` refuses the node when the
  installed CH does not advertise `backend=vhost-kernel` with the `fd=`
  handoff. Access to `/dev/vhost-vsock` is checked by the claim probe
  (ADR-0170) under the `serve` identity; the VMM never opens the device.
- **Per-launch failure.** A launch whose device the fork cannot set up makes
  CH exit before READY. The driver projects it through the existing pre-READY
  VMM-exit start failure; the lease is retired and released, and placement
  may retry with a fresh lease and claim. No owner branches on CH's diagnostic
  text. A CID still held by another VMM cannot cause it: the claim was taken
  before launch. It is non-terminal for the node.
- **No snapshot, migration or pause.** Overdrive does not snapshot, migrate or
  pause a vhost-kernel VM. The fork refuses snapshot and migration; pause is
  not issued.

The exact error variants, probe stages and their projections are pinned in
the feature delta (§ *Core vocabulary*, § *VMM backend contract*).

## Alternatives considered

- **Keep CH's Unix-socket muxer.** The VMM would copy every application byte,
  CH is stream-only (no SEQPACKET leg for ADR-0148), and each VM is capped at
  1,023 connections. Rejected by constraint.
- **Swap to QEMU (`vhost-vsock-device`).** Supports the kernel backend
  upstream, but replaces the VMM, its launch confinement (ADR-0129/0143) and
  the adapter. Rejected by ruling D13.
- **Firecracker.** Same hybrid Unix model as CH. Rejected for the same reasons.
- **Fall back to the Unix backend when vhost is unavailable.** A booted VM
  whose flows never reach host `AF_VSOCK` is a silent black hole. Rejected.

## Consequences

- Every VM launch takes the claimed device as a required argument; the
  hardcoded `cid=3` and the per-VM vsock Unix socket paths are removed.
- The VMM process holds the claimed `/dev/vhost-vsock` file (inherited) and
  owns one kernel vhost worker task per VM. The fork adds `VHOST_SET_OWNER`,
  `VHOST_SET_MEM_TABLE` and `VHOST_VSOCK_SET_RUNNING` to its own seccomp
  rules; in `fd=` mode it needs no Landlock access to `/dev/vhost-vsock`
  (ADR-0170).
- **ADR-0143** (launch seccomp deny-list of TAP-mutating ioctls) must be
  re-validated against the fork under the production launch hook. The deny
  list does not include any `VHOST_*` request.
- **ADR-0068 §4** states that CH `--vsock` uses `/dev/vhost-vsock`. That is
  false for stock CH; only the fork's `backend=vhost-kernel` does. ADR-0068
  §4 is listed for correction in the feature delta.
- Access to `/dev/vhost-vsock` depends on the device node's mode on the host.
  The claim probe (ADR-0170) turns that dependence into a startup refusal, not
  a launch surprise (the ADR-0144 principle: no silent correctness dependence
  on host configuration).
- The host kernel must provide `CONFIG_VHOST_VSOCK` and the guest kernel
  `CONFIG_VIRTIO_VSOCKETS`; the startup probe and the guest's setup checks
  refuse when they are absent. Observed on stock 7.0.0-29.
