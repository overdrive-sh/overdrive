# ADR-0145 — Guest application traffic uses virtio-vsock with kernel forwarding on both sides, replacing the shared bridge and per-VM TAP

## Status

**Proposed — decision D1 approved by user 2026-10-05; pending independent
DESIGN review.** GH #295. Recorded in the #295 feature delta, § *[REF] vsock
Attachment Replacement DESIGN — PROPOSED 2026-10-05*.

It builds on three user rulings of 2026-10-05, recorded in that section:

- **D13 (a):** the vendored Cloud Hypervisor fork supplies the kernel
  vhost-vsock backend (ADR-0146, ADR-0161).
- **D14:** non-mesh guest egress is forwarded in the kernel too; no userspace
  payload relay exists anywhere (ADR-0162).
- **D15:** guests keep today's network behaviour wherever the kernel allows it
  (ADR-0150).

On acceptance this ADR **supersedes ADR-0114** (node-local shared bridge and
guest-prefix network). Because their only subject is the bridge, the TAP or
the guest's L2 identity, it also supersedes, as moot:

- ADR-0126 (fixed bridge MAC)
- ADR-0127 (inherited TAP queue fd)
- ADR-0130 (uid-0 TAP owner)
- ADR-0142 (TAP egress guest-MAC delivery)
- ADR-0144 (managed link identity)

ADR-0128 is superseded by ADR-0146. ADR-0115 and ADR-0139 are superseded by
ADR-0153. ADR-0117 is superseded by ADR-0155. Superseded ADR bodies are not
edited.

## Context

#295 removes the 4,096 `NetSlot` ceiling so that one node can run more VMs.
ADR-0114 replaced per-VM network namespaces with one node-local Linux bridge
and one host TAP per VM. On qualified metal (kernel `7.0.0-29-generic`, CH
v53.0) the production attach path received `EXFULL` at `set_link_master`
(`docs/feature/netns-density-295/deliver/native-e18-bridge-capacity-falsification.md`,
commit `9460d899`). Upstream Linux v7.0 gives a bridge a 10-bit port number,
from which the record infers 1,023 usable ports; the failing index was not
counted. That is a topology ceiling below the 16,384 measurement target
(ADR-0117), not the machine running out of resources.

The user selected shared-memory virtio queues, virtio-vsock, and Aya Rust
eBPF forwarding in the kernel, with the Rust control plane managing sockets,
maps, activation and lifecycle and never relaying payload. The evidence:

- 16,384 activated CH vsock devices (`spike/shared-memory-vsock-scale-findings.md`).
- Aya SK_SKB/SockHash forwarding between host `AF_VSOCK` and `AF_INET` sockets
  over stock kernel vhost-vsock, 16,384 owners
  (`spike/aya-vsock-proxy-findings.md`), empty UDP via a carrier
  (`spike/aya-zero-udp-findings.md`), and the reviewed benchmark at populations
  up to 16,384 (`spike/kernel-vs-aya-benchmark.md`).
- The vendored CH fork booting unmodified guests on kernel vhost-vsock, with
  zero payload bytes through CH (`spike/ch-vhost-vsock-findings.md`, WORKS).
- Unmodified guest programs (`curl`, `sshd`/`scp`, `getent`, `dig`, busybox
  `nslookup`) in a NIC-less booted guest, carried by guest-kernel and
  host-kernel Aya, with only 8- and 16-byte control I/O in either owner
  (`spike/guest-vsock-capture-findings.md`, WORKS).

All of it ran on stock `7.0.0-29-generic`, host and guest. None ran on the
pinned 6.18 appliance kernel, and none ran through `overdrive serve` +
`overdrive deploy`.

## Decision

A VM allocation has no guest network device, no host TAP, no bridge port and
no per-allocation host netdevice. Its application traffic crosses the VM's one
virtio-vsock device, whose backend is kernel vhost-vsock (ADR-0146).

Payload is forwarded only by Aya Rust eBPF programs:

- in the guest kernel, between the application's ordinary socket's peer and
  the flow's guest vsock socket (ADR-0150);
- in the host kernel, between the flow's host vsock socket and its
  host-facing socket (ADR-0151).

The Rust control planes on each side (`overdrive-init` in the guest, the
guest-flow owner on the host) create sockets, exchange fixed-size control
messages, install and remove pairs, and activate and retire allocations. They
never read or write application payload.

The parts that could be decided differently are separate ADRs: VMM backend
(0146), fork provisioning (0161), CID (0156), beacon (0157), pair model
(0147), pairing order (0158), socket types per direction (0148), datagram
frame (0149), guest adaptation (0150), ownership (0151), unpinned lifetime
(0159), stop semantics (0160), inbound intake (0152), host-intake listen
state (0163), mTLS boundary (0153), non-mesh egress (0162), DNS (0154) and
the density target (0155).

## Alternatives considered

- **More bridges (multi-bridge fanout).** Every bridge keeps the 1,023-port
  ceiling and the per-TAP netdevice, neighbour and lock costs
  (`spike/multi-bridge-findings.md`). Rejected: it keeps the per-VM netdevice
  cost #295 removes.
- **Open vSwitch kernel datapath.** 16,384 TAP/OVS ports verified
  (`spike/ovs-kernel-findings.md`). Rejected by constraint: TAP-based.
- **A loadable kernel module over the same vhost queues.** Verified and
  benchmarked (`spike/shm-vsock-kernel-proxy-findings.md`). At 16,384 owners
  it used about twice the unreclaimable slab and three times the kernel tasks.
  Rejected: the user selected Aya. Kept as the comparator.
- **A userspace TCP↔vsock relay.** Rejected by constraint.
- **Multiplex flows over one vsock connection per VM.** Needs a userspace
  payload demultiplexer. Rejected (ADR-0147).

## Consequences

**Positive:**

- The bridge port ceiling and every per-VM host netdevice disappear, with the
  host ARP, neighbour and FDB state per VM.
- An attachment costs one vhost-vsock device, its CID and its sockets.
- The guest keeps its workload address, original peer addresses, real inbound
  client addresses, general UDP and early data (D15, proven bounded).

**Negative:**

- The production VMM is a vendored Cloud Hypervisor fork (ADR-0146,
  ADR-0161).
- The guest adaptation is large: capture hooks, establishment-time install,
  kernel parking cells, strparser reassembly, a drain signal and a guest
  `lo` frame stripper (ADR-0150). It is a new guest artifact with its own
  verifier and kernel-version exposure.
- Host footprint: forwarding programs run in the physical host kernel,
  including a `sock_ops` program on the owner's cgroup and an
  `fexit(skb_send_sock)` counter (ADR-0151).
- Inbound UDP service to VMs (#310), IPv6 (#308), ICMP (#311) and UDP
  datagrams above 59,000 bytes (#309) are not carried by this feature.
- **#303 / PR #306 guest kTLS** assumes a guest NIC and is not compatible with
  this design as spiked (feature delta, § *Guest-mTLS boundary*).
- Implemented #295 bridge and TAP code becomes dead on the production path. It
  is deleted, with its tests, during DELIVER.
