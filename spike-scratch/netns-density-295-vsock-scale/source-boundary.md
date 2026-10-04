# Exact stock implementation boundary

The native dependency is pinned to [CH v53.0](https://github.com/cloud-hypervisor/cloud-hypervisor/tree/9ed824d6d08df3e96f7d5f50795d9449ac99f431).
No production Overdrive crate is imported and no upstream file is edited.

The stock VMM's [DeviceManager::make_virtio_vsock_device](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/9ed824d6d08df3e96f7d5f50795d9449ac99f431/vmm/src/device_manager.rs#L3525)
constructs `VsockUnixBackend::new(cid, path)` and passes it to `Vsock::new`.
The harness calls those same public constructors, then the actual stock
[VirtioDevice::activate](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/9ed824d6d08df3e96f7d5f50795d9449ac99f431/virtio-devices/src/vsock/device.rs#L461).
The resulting worker executes stock `VsockEpollHandler::run`, `process_rx`
and `process_tx`, stock packet decoding, the Unix muxer and stock connection
state machine. A synthetic peer writes/reads standards-format virtio descriptors
and `virtio_vsock_hdr`; it neither replaces nor patches those implementations.

The [muxer constructor](https://github.com/cloud-hypervisor/cloud-hypervisor/blob/9ed824d6d08df3e96f7d5f50795d9449ac99f431/virtio-devices/src/vsock/unix/muxer.rs#L370)
retains one CID, independent connection/listener maps, RX/kill queues,
base Unix listener and nested epoll file. The maps are preallocated for the
per-muxer tracked-connection budget; that fixed resource cost is included in
native measurements. The 1,023 tracked connections per muxer are not a global
VM/device count. This probe retains one established connection in each muxer.

Each stock worker additionally owns an outer epoll, RX/TX/event queue eventfds,
kill/pause eventfds and an exit eventfd clone. Its real lifecycle is exercised:
the stock reset drops the worker collection, signals it and joins its thread;
stock shutdown removes its exact base socket pathname. The observer sees
kernel object identities and FD aliasing independently of the Rust count.

The harness supplies 128KiB peer memory and a host interrupt callback. It does
not execute CH's `DeviceManager` composition, PCI transport, KVM memory/vCPU
creation, guest OS/AF_VSOCK socket implementation, guest scheduling or workload
memory. Seccomp is set to `Allow` through the existing stock constructor;
the production confinement profile is not exercised. There are zero host TAPs,
bridge ports, virtio-net devices, host AF_VSOCK CID registrations or kernel
vsock devices. The CID proof is the virtio config plus stock-produced packet
destination CID in each independently owned context.

This is a capacity result for active **stock virtio-vsock transport device and
Unix muxer owners** in a standalone process. It is not a 16,384-running-microVM
claim, a bulk-throughput/fairness/latency result, or proof of Overdrive's policy,
TLS, DNS, activation, capture, restart or allocation-generation contracts.
