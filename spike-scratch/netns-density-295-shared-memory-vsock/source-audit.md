# Stock Cloud Hypervisor source / ABI audit

Inspected read-only local clone: `/Users/marcus/git/cloud-hypervisor`, clean at `ae14eb5d8a4612efd8e1ed389da94a5c401c1d93` (561 commits after `v53.0`). The native executable reports `v53.0`; therefore the release tag was independently read with `git show v53.0:PATH`, without changing the clone. `v53.0` resolves to `9ed824d6d08df3e96f7d5f50795d9449ac99f431`. The tested executable SHA-256 is recorded in every native attempt. This does not attest a reproducible source-to-binary build of that executable.

## Three distinct layers

1. Linux guests use the stock `AF_VSOCK` stream ABI (`sockaddr_vm` has 32-bit CID and port fields). The actual local CID is read from `/dev/vsock` using upstream `IOCTL_VM_SOCKETS_GET_LOCAL_CID` (`_IO(7, 0xb9)`), independently of guest cmdline intent.
2. CH supplies a **virtio-vsock device** and three guest-memory virtqueues. At both inspected pins, each queue has 256 descriptors. This normal virtio shared-memory transport is not an attested custom shared-memory IP stack, a cross-VM memory pool or a zero-copy performance claim.
3. CH's userspace **Unix-vsock muxer** converts those connections into allocation-specific host `AF_UNIX` streams. Host→guest uses the base socket plus `CONNECT PORT\n` / `OK LOCAL_PORT\n`; guest→host targets CID 2 and CH connects to the base pathname suffixed `_PORT`. The experiment used no `/dev/vhost-vsock` FD in either VMM. Host `CONFIG_VHOST_VSOCK` in the project documentation is not a demonstrated prerequisite for this userspace backend. Guest virtio-vsock support **is** required; the existing rootfs has three stock Ubuntu modules under `/modules`, loaded only inside each newly created guest.

## Exact source bounds, not VM capacity promises

Both inspected CH pins define 1,023 connection-map entries **per Unix-vsock muxer / VMM**. `add_connection` first sweeps the kill queue and rejects when `conn_map.len() >= MAX_CONNECTIONS`. It counts initializing and other retained tracked states; established streams alone are not the admission predicate. The muxer RX queue has 256 entries and kill queue 128 entries. These queues differ from the virtqueue descriptor count.

Both pins reject guest packets whose destination CID is not the host CID, so stock CH supplies guest↔host transport rather than automatic direct VM↔VM CID routing. Missing host `_PORT` paths cause reset; an absent guest listener fails a host `CONNECT`. The native receipts reproduce these different behaviors, including timed-out attempts to the sibling/unknown CID.

The source has one muxer object per VMM. There is no 1,023-entry host-wide VM ceiling in this code. Equally, the per-muxer constant does not promise 1,023 simultaneously usable connections under the native process's 1,024 soft FD limit, buffering, guest limits or memory pressure. No saturation test was run.

## Primary sources

- [v53.0 Unix muxer constants](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/9ed824d6d08df3e96f7d5f50795d9449ac99f431/virtio-devices/src/vsock/unix/mod.rs).
- [v53.0 admission and destination-CID handling](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/9ed824d6d08df3e96f7d5f50795d9449ac99f431/virtio-devices/src/vsock/unix/muxer.rs).
- [v53.0 virtqueue sizes](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/9ed824d6d08df3e96f7d5f50795d9449ac99f431/virtio-devices/src/vsock/device.rs).
- [Research pin transport documentation](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/ae14eb5d8a4612efd8e1ed389da94a5c401c1d93/docs/vsock.md).
- [Linux v7.0 guest ABI](https://github.com/torvalds/linux/blob/v7.0/include/uapi/linux/vm_sockets.h).

The Unikraft precedent is kept at its published boundary: earlier 100,000-instance work used per-VM TAPs and multiple bridges; later work describes shared-memory/vsock communication, a substantial private Firecracker fork and scaled-to-zero lifecycles. The unpublished host runtime is not this stock CH probe. Current first-party TAP-pool documentation still conflicts with a universal TAP-elimination interpretation. The existing research report pins the public guest/library/docs trees; none was changed or executed by this probe. [May 2026 engineering account](https://unikraft.com/blog/1m-vms-single-box), [pinned current TAP-pool documentation](https://github.com/unikraft-cloud/docs/blob/0564725fe346f86b253ab5421497cae795f8a4e5/pages/features/custom-network-configuration.mdx).
