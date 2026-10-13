# ADR-0146 — Every VM's vsock device uses the kernel vhost-vsock backend of the vendored Cloud Hypervisor, and the VMM opens the device itself

## Status

**Accepted — decision D2 approved by user 2026-10-05; CID-NOCLAIM (the VMM
opens its own `/dev/vhost-vsock` instance and is its only holder) approved by
user 2026-10-11.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN*.

The VMM path inside this decision is ruling **D13 = (a), approved by user
2026-10-05**: the vendored Cloud Hypervisor fork
(`overdrive-sh/cloud-hypervisor`, branch `overdrive/vhost-kernel-vsock`,
commits `9b68dbb57` and `41a619d19` on v53.0) is the production VMM path. How
that fork is built, pinned and provisioned is ADR-0161; the fork change that
ends the VMM on a guest reset is ADR-0182.

This ADR **supersedes ADR-0128** (the VMM adapter owns the TAP queue fd). CID
derivation is ADR-0156; the beacon listener is ADR-0157.

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
- the CID was free and immediately reusable after poweroff, `kill -9`,
  in-guest reboot and hot-unplug (20/20 immediate relaunches);
- snapshot is refused with a typed CH error; while paused, host connects time
  out instead of queueing;
- the default Unix backend is behaviourally unchanged.

The fork's device opens `/dev/vhost-vsock`, becomes its owner and sets the
guest CID when the device is created
(`virtio-devices/src/vsock/vhost_kernel.rs:209-213` in
`vendors/cloud-hypervisor`). Under `--landlock`, the fork grants the VMM
read-write access to `/dev/vhost-vsock` for a `backend=vhost-kernel` vsock
device and for no other path of that device (`vmm/src/vm_config.rs:898-914`;
the path constant is `vmm/src/vm_config.rs:883`). Its seccomp filters allow
the vhost-vsock ioctls the device issues (`vmm/src/seccomp_filters.rs:387-402`).

The kernel ties the CID and the guest's connections to the open file, not to
the process. `VHOST_VSOCK_SET_GUEST_CID` refuses a CID another instance holds
with `EADDRINUSE` (`drivers/vhost/vsock.c:808-813 @ v7.0`). The file's
release, which runs at its last close, unhashes the CID and resets every
connected host socket whose peer CID no longer has a device
(`drivers/vhost/vsock.c:753-768 @ v7.0`, sweep `:721-751`, reset at `:746-750`: `SOCK_DONE`,
`peer_shutdown`, `sk_err = ECONNRESET`). V-30
(`spike/findings-v30-guest-reset.md`, Q2c/Q2d, 7.0.0-29) measured both
sides: with the VMM as the only holder, host flows reset at VMM exit with
`ECONNRESET`; while another process kept a copy of the VMM's fd, host flows
stayed ESTABLISHED to the dead guest and `send` kept succeeding until that
copy closed.

## Decision

- **Backend.** Each VM has exactly one virtio-vsock device, launched with
  `backend=vhost-kernel` and the CID the admission lease derives
  (ADR-0156). Both the beacon and every application flow cross it.
- **The VMM opens the device.** The fork opens `/dev/vhost-vsock` itself,
  under its one Landlock rule for the device, and sets the guest CID on that
  instance. Overdrive passes no device descriptor to the VMM and opens no
  `/dev/vhost-vsock` instance for a VM.
- **The VMM is the only holder.** No Overdrive process holds a copy of a
  VMM's vhost-vsock descriptor. When the VMM exits, its device is released
  and the kernel resets every host flow of that guest.
- **No fallback.** The VMM adapter never launches a VM with the Unix backend,
  with no vsock device or with a second vsock device.
- **The fork is the only VMM.** Overdrive runs only with the vendored fork
  (ADR-0161). No startup probe inspects the installed binary, and none opens
  `/dev/vhost-vsock`: the pinned appliance kernel provides the vhost-vsock
  backend built in (ADR-0068 §4), and the image, which ADR-0068 governs,
  sets the device node's mode.
- **Per-launch failure.** A launch whose device the fork cannot open or set
  up makes CH exit before READY. The driver projects it through the existing
  pre-READY VMM-exit start failure; the lease is released after the VMM has
  exited, and placement may retry with a fresh lease. No owner branches on
  CH's diagnostic text. The failure is non-terminal for the node.
- **No snapshot, migration or pause.** Overdrive does not snapshot, migrate or
  pause a vhost-kernel VM. The fork refuses snapshot and migration; pause is
  not issued.

The exact launch argument, error projections and probe stages are pinned in
the feature delta (§ *VMM backend contract*, § *Core vocabulary*).

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
- **`serve` claims the CID on its own `/dev/vhost-vsock` instance and hands
  the claimed descriptor to the VMM.** It defends against a CID held by a VMM
  the pool does not lease, which boot reclamation already removes before any
  lease is assigned (ADR-0156). A copy of the descriptor kept by `serve` past
  VMM exit keeps the dead guest's host flows ESTABLISHED (V-30 Q2c), and a
  once-only handoff guard ends the VMM on a guest reset only as a failed
  reboot. Rejected (CID-NOCLAIM).
- **A startup probe that opens `/dev/vhost-vsock` and sets a CID.** It checks
  a property of the pinned kernel and image that no Overdrive node lacks; a
  launch that cannot open the device already fails pre-READY through a typed
  start failure. Rejected.

## Consequences

- Every VM launch passes the derived CID and `backend=vhost-kernel`; the
  hardcoded `cid=3` and the per-VM vsock Unix socket paths are removed.
- The VMM process holds its `/dev/vhost-vsock` file and owns one kernel vhost
  worker task per VM.
- Host flow cleanup after a VM ends needs no Overdrive action: the release of
  the VMM's device resets them. This rests on the VMM being the only holder;
  an Overdrive descriptor copy would defeat it.
- **ADR-0143** (launch seccomp deny-list of TAP-mutating ioctls) must be
  re-validated against the fork under the production launch hook. The deny
  list does not include any `VHOST_*` request.
- **ADR-0068 §4** states that CH `--vsock` uses `/dev/vhost-vsock`; that holds
  only for the fork's `backend=vhost-kernel`. ADR-0068 §4 carries the
  correction.
- The host kernel must provide `CONFIG_VHOST_VSOCK` and the guest kernel
  `CONFIG_VIRTIO_VSOCKETS`; the pinned appliance kernel builds both in
  (ADR-0068 §4). Observed on stock 7.0.0-29.
- Running the fork under the production launch identity (uid drop, launch
  seccomp filter, Landlock) is validation item V-1(c) (DELIVER).
