# ADR-0145 — Guest application traffic uses virtio-vsock with kernel forwarding on both sides, replacing the shared bridge and per-VM TAP

## Status

**Accepted — D1 approved by user 2026-10-05; END-STATE (the guest stays
NIC-less and vsock-only, and #303 adapts to it) and the temporary leg-S relay
exception of INB-1 approved by user 2026-10-11.** GH #295. Recorded in the
#295 feature delta, § *[REF] vsock Attachment Replacement DESIGN*.

It builds on these user rulings, recorded in that section:

- **D13 (a), 2026-10-05:** the vendored Cloud Hypervisor fork supplies the
  kernel vhost-vsock backend (ADR-0146, ADR-0161).
- **D14, 2026-10-05:** non-mesh guest egress is forwarded in the kernel
  (ADR-0162).
- **D15, 2026-10-05:** guests keep today's network behaviour wherever the
  kernel allows it (ADR-0150).
- **INB-1, 2026-10-11:** host platform clients reach a VM port over an
  owner-opened vsock flow; leg-S's kTLS↔vsock relay is the one userspace
  payload relay, explicitly temporary until #303 (ADR-0173, ADR-0186).
- **END-STATE, 2026-10-11:** the guest is NIC-less and vsock-only; in-guest
  kernel mTLS (#303) is adapted to this transport.

This ADR **supersedes ADR-0114** (node-local shared bridge and guest-prefix
network). Because their only subject is the bridge, the TAP or the guest's L2
identity, it also supersedes, as moot:

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
commit `9460d899`). Linux v7.0 gives a bridge a 10-bit port number
(`net/bridge/br_private.h:28-29 @ v7.0`), from which the record infers 1,023
usable ports; the failing index was not counted. That
is a topology ceiling below the 16,384 measurement target (ADR-0155), not the
machine running out of resources.

The user selected shared-memory virtio queues, virtio-vsock, and Aya Rust
eBPF forwarding in the kernel, with the Rust control plane managing sockets,
maps, activation and lifecycle. The evidence:

- 16,384 activated CH vsock devices (`spike/shared-memory-vsock-scale-findings.md`).
- Aya SK_SKB/SockHash forwarding between host `AF_VSOCK` and `AF_INET` sockets
  over stock kernel vhost-vsock, 16,384 owners
  (`spike/aya-vsock-proxy-findings.md`), empty UDP
  (`spike/aya-zero-udp-findings.md`, `spike/findings-v29-udp-empty.md`), and
  the benchmark at populations up to 16,384
  (`spike/kernel-vs-aya-benchmark.md`).
- The vendored CH fork booting unmodified guests on kernel vhost-vsock, with
  zero payload bytes through CH (`spike/ch-vhost-vsock-findings.md`, WORKS).
- Unmodified guest programs (`curl`, `sshd`/`scp`, `getent`, `dig`, busybox
  `nslookup`) in a NIC-less booted guest, carried by guest-kernel and
  host-kernel Aya, with only fixed-size control I/O in either owner
  (`spike/guest-vsock-capture-findings.md`, WORKS).
- Host platform clients over an owner-opened vsock flow, half-close, abort
  classification, guest reset and drain completion on real microVMs
  (`spike/findings-v28-legs-vsock.md`, `findings-v30-guest-reset.md`,
  `findings-v31-r2-halfclose.md`, `findings-v32-udp-seqpacket.md`,
  `findings-drain-t1-t6.md`).

Every spike ran on Linux 7.0 (`7.0.0-29-generic`) on host and guest, the
kernel Overdrive pins (ADR-0068). None ran through `overdrive serve` +
`overdrive deploy`.

## Decision

A VM allocation has no guest network device, no host TAP, no bridge port and
no per-allocation host netdevice. Its application traffic crosses the VM's one
virtio-vsock device, whose backend is kernel vhost-vsock (ADR-0146).

Payload is forwarded by Aya Rust eBPF programs:

- in the guest kernel, between the application's ordinary socket's peer and
  the flow's guest vsock socket (ADR-0150);
- in the host kernel, between the flow's host vsock socket and its
  host-facing socket (ADR-0151).

The Rust control planes on each side (`overdrive-init` in the guest, the
guest-flow owner on the host) create sockets, exchange fixed-size control
messages, install and remove pairs, and activate and retire allocations. They
never read or write application payload, with one exception: a host platform
client that dials a VM port — leg-S delivering mesh plaintext, or the VM probe
runner — holds the host end of an owner-opened vsock flow and reads and writes
it itself. For leg-S that is a userspace kTLS↔vsock relay. It is explicitly
temporary and is deleted when #303 lands, after which the host forwards
ciphertext over the existing kernel redirect (ADR-0186).

The parts that could be decided differently are separate ADRs:

- VMM and device: kernel vhost backend opened by the VMM (0146), a guest reset
  ends the VMM (0182), fork provisioning (0161), CID from the lease (0156).
- Attribution and sessions: beacon (0157), peer CID plus launch token (0177),
  per-VM control session (0166).
- Flows: one pair per flow (0147), pairing order (0158), socket types per
  direction (0148), datagram frame (0149), where the frame is removed (0165),
  how a flow's end crosses the VM boundary (0180), drain completion (0183).
- Guest and host owners: guest adaptation (0150), host ownership (0151),
  unpinned lifetime (0159), stop and restore (0160), counted quiescence
  (0169), boot refusal on a failed shared-table write (0184).
- Addressing and reachability: the guest prefix holds no node address and
  no VIP (0185), only the gateway would be host-local in it (0152, GW-LOCAL
  proposed), constant guest-prefix rules (0172), host platform clients over
  an owner-opened flow (0173), leg-S to VM backends through a temporary relay
  until #303 (0186), the address the guest application sees (0174).
- Destinations: mTLS boundary (0153), non-mesh egress (0162), service VIPs
  (0164), host-internal destinations (0167), DNS (0154).
- Density: the measurement target (0155).

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
- **A userspace TCP↔vsock relay for every flow.** Rejected by constraint. The
  one relay this design admits is leg-S's, temporary until #303.
- **Multiplex flows over one vsock connection per VM.** Needs a userspace
  payload demultiplexer. Rejected (ADR-0147).

## Consequences

**Positive:**

- The bridge port ceiling and every per-VM host netdevice disappear, with the
  host ARP, neighbour and FDB state per VM.
- An attachment costs one vhost-vsock device, its CID and its sockets.
- The guest keeps its workload address, original peer addresses, general UDP
  and early data (D15). The residual differences are recorded as D15-R1 to
  D15-R4 (ADR-0150, ADR-0173); host-opened connections appear to the guest
  application from the guest gateway (ADR-0174).

**Negative:**

- The production VMM is the vendored Cloud Hypervisor fork, which must end the
  VMM on a guest reset (ADR-0146, ADR-0161, ADR-0182).
- The guest adaptation is large: capture hooks, establishment-time install,
  kernel parking cells, strparser reassembly, a flow-end classifier and a
  guest `lo` frame stripper (ADR-0150). It is a new guest artifact with its
  own verifier and kernel-version exposure.
- Host footprint: forwarding programs run in the physical host kernel — a
  `sock_ops` program on the owner's cgroup, a raw tracepoint on vsock packet
  receipt, two `fexit(skb_send_sock)` programs and an `fentry(sock_poll)`
  backstop (ADR-0151, ADR-0180, ADR-0183).
- Until #303 lands, mesh traffic into a VM passes leg-S's userspace
  kTLS↔vsock relay: about +0.5 ms per connection and 12–37 % less bulk
  client-to-guest throughput than loopback TCP (`spike/findings-v28-legs-vsock.md`).
- Inbound UDP service to VMs (#310), IPv6 (#308), ICMP (#311) and UDP
  datagrams above 59,000 bytes (#309) are not carried by this feature.
- VMs do not survive a `serve` restart: boot reclamation ends them (#312).
- The #295 bridge and TAP code on this branch is dead on the production path.
  It is deleted, with its tests, during DELIVER.
