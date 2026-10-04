# Vocabulary

Plain-language definitions for terms used in Overdrive's architecture, research,
spikes and measurements.

## Memory

| Term | Meaning |
| --- | --- |
| **PSS — Proportional Set Size** | Resident memory attributed to a process, dividing shared pages among the processes using them. Useful when comparing or adding process memory footprints without repeatedly counting the same shared pages. |
| **RSS — Resident Set Size** | Memory mapped by a process that is currently resident in RAM. Shared pages count in each process's RSS, so adding multiple processes' RSS can count them repeatedly. |
| **Guest RAM** | Memory configured for a VM. This is a different measurement from the VMM process's PSS or RSS. |
| **MemAvailable** | The kernel's estimate of RAM available for new applications without swapping, including memory it expects to reclaim. |
| **Shared memory** | Memory accessible to more than one participant. Virtio uses shared buffers between a device and its driver; this does not make all their memory shared. |
| **KiB / MiB / GiB** | Binary units: 1 KiB = 1,024 bytes; 1 MiB = 1,024 KiB; 1 GiB = 1,024 MiB. MB and GB normally denote decimal units. |

**PSS example:** a process has 8 MiB of private resident memory and shares 4 MiB
equally with one other process. Its RSS is 12 MiB; its PSS is 10 MiB.

PSS and RSS are process memory metrics, not complete measurements of every kernel
resource or the whole machine's memory use.

Sources: [Linux process memory accounting](https://docs.kernel.org/filesystems/proc.html),
[virtio shared buffers](https://docs.kernel.org/driver-api/virtio/virtio.html).

## CPUs and virtual machines

| Term | Meaning |
| --- | --- |
| **Host** | The machine and operating system running the virtual machines. |
| **Guest** | The operating system or unikernel running inside a VM. |
| **VMM — Virtual Machine Monitor** | The software that creates and manages virtual machines and their devices, such as Cloud Hypervisor or Firecracker. |
| **KVM — Kernel-based Virtual Machine** | Linux's virtualization facility. A VMM uses its API to create VMs and run guest CPUs. |
| **vCPU — Virtual CPU** | A CPU presented to the guest. Its count describes guest CPU topology; it does not reserve that many host CPUs or imply continuous CPU use. |
| **Logical CPU** | A CPU execution context visible to the host scheduler. With simultaneous multithreading, a physical core can expose multiple logical CPUs. |
| **CPU quota** | A cap on scheduled CPU time over a period. A quota of 0.125 CPU allows one eighth of one logical CPU's time; it is not a reservation or a minimum allocation. Idle workloads can use less. |
| **cgroup — Control group** | A Linux grouping of processes used to account for and control resources, including CPU and memory. |
| **`cpu.max`** | The cgroup v2 CPU quota and period, in microseconds. `12500 100000` means up to 12.5 ms of CPU time per 100 ms period across the group's applicable tasks: 0.125 CPU. |

**Example:** a VM can expose one vCPU and have a host CPU quota of 0.125. These
describe different things: the CPU the guest sees and the CPU time the host allows.

Sources: [KVM API](https://docs.kernel.org/virt/kvm/api.html),
[Linux CPU quotas and cgroups](https://docs.kernel.org/admin-guide/cgroup-v2.html#cpu).

## Virtual devices and transport

| Term | Meaning |
| --- | --- |
| **virtio** | A standard interface for virtual devices, including network, disk and vsock devices. A driver exchanges buffers with a device implementation through virtqueues. |
| **Virtqueue** | A queue of buffer descriptors used by a virtio driver and device to exchange data and notifications. |
| **virtio-vsock** | A virtio device implementation of the vsock transport. Instantiating and activating this device does not itself boot a guest OS. |
| **vsock / `AF_VSOCK`** | A socket family for communication across a virtualization boundary, commonly between guest and host. It uses context IDs and ports rather than IP addresses and does not require an Ethernet bridge. |
| **CID — Context ID** | A vsock address identifying a communication context. A CID alone is an address, not proof that a device, VM or connection exists. |
| **Muxer — Multiplexer** | A component that manages multiple communication streams. The stock Cloud Hypervisor vsock muxer connects virtio-vsock traffic with Unix sockets. |
| **Unix socket / UDS** | A socket addressed locally on a host, often through a filesystem path, rather than through an IP address. |
| **Attachment** | The actual object counted in a capacity test. The report must name it: for example, a bridge port or an activated virtio-vsock device and its muxer. An attachment count is not automatically a running-VM count. |
| **Device activation** | Starting a device's queue and event handling. Device activation and guest boot are separate operations. |

Sources: [Virtio on Linux](https://docs.kernel.org/driver-api/virtio/virtio.html),
[Linux vsock interface](https://man7.org/linux/man-pages/man7/vsock.7.html).
For the measured Cloud Hypervisor objects, see the
[vsock capacity spike](feature/netns-density-295/spike/shared-memory-vsock-scale-findings.md).

## Host resources and networking

| Term | Meaning |
| --- | --- |
| **PID — Process ID** | The operating system's identifier for a process. |
| **Thread** | An execution task inside a process. A process can have many threads; thread count is separate from CPU quota or vCPU count. |
| **FD — File descriptor** | A process-local handle for a resource, such as a file, socket, event counter or epoll instance. FD count is not limited to ordinary files. |
| **epoll** | Linux's facility for waiting for events on many file descriptors. |
| **eventfd** | A Linux file descriptor backed by a counter, used for event notifications between components. |
| **TAP** | A virtual network interface that exchanges Ethernet frames with a userspace process through an open device handle. |
| **veth** | A pair of linked virtual Ethernet interfaces. Frames transmitted through one arrive at the other. |
| **Bridge / bridge port** | A virtual Ethernet switch / an interface attached to that switch. Bridge-port capacity is separate from the machine's RAM or CPU capacity. |
| **Network namespace / netns** | A Linux namespace containing its own network interfaces, routes and related network state. |
| **ARP / NDP** | IPv4 / IPv6 mechanisms for discovering a neighbor's link-layer address. Neighbor-table pressure can affect traffic even when interface creation succeeds. |
| **Kernel module** | Code loaded into a running kernel. A module and a patch to the kernel source are different delivery mechanisms. |
| **EXFULL** | Linux's “Exchange full” error name. Its concrete cause depends on the operation; the native single-bridge experiment encountered it while requesting another bridge port. |
| **mTLS — Mutual TLS** | TLS in which both peers authenticate with certificates. |
| **kTLS — Kernel TLS** | Kernel processing of TLS records on a socket. The handshake can remain in userspace. |
| **TPROXY — Transparent proxying** | Redirecting traffic to a proxy while preserving addressing information needed for transparent handling. |
| **TCX** | Linux's newer attachment interface for BPF programs at traffic-control ingress and egress hooks. |

Sources: [epoll](https://man7.org/linux/man-pages/man7/epoll.7.html),
[eventfd](https://man7.org/linux/man-pages/man2/eventfd.2.html),
[TUN/TAP](https://docs.kernel.org/networking/tuntap.html),
[Linux error names](https://man7.org/linux/man-pages/man3/errno.3.html),
[kernel TLS](https://docs.kernel.org/networking/tls.html),
[transparent proxying](https://docs.kernel.org/networking/tproxy.html),
[BPF attachment types](https://docs.kernel.org/bpf/libbpf/program_types.html).

## Reading experiment results

| Term | Meaning |
| --- | --- |
| **Spike** | A bounded experiment to test an assumption before committing to a design. Its code and receipts are evidence, not production implementation. |
| **Native run** | Execution against the measured host's real kernel and resources. A native device test can still use synthetic drivers instead of a booted guest. |
| **Synthetic driver** | A harness component supplying device requests in place of an OS driver. The report must distinguish synthetic requests from the real device implementation being exercised. |
| **PSS of a device harness** | The harness process's measured memory footprint. It does not include hypothetical full guests that the experiment did not boot. |
| **Capacity** | The actual population held and exercised by an experiment, under its stated resource limits and conditions. A smaller successful run does not establish a larger capacity. |
