# ADR-0150 — Ordinary guest sockets reach the vsock transport through guest-kernel Aya capture, establishment-time install and kernel parking, controlled by `overdrive-init`

## Status

**Proposed — decision D6 approved by user 2026-10-05; decision D25, the guest
intake model (independent DESIGN review finding H-2: reserved ports, slot
identity and release, pool sizes, exhaustion outcomes, deploy refusal of a
reserved port) with its residual D15 differences D15-R2, approved by user
2026-10-06; the exact bind-refusal scope (D25-BIND) and the end of an
unpaired or aborted datagram association (U-2), approved by user 2026-10-06;
pending independent DESIGN review.** GH #295. Recorded in the #295
feature delta, § *[REF] vsock Attachment Replacement DESIGN — PROPOSED
2026-10-05*, § *[REF] Guest adaptation contract* and § *[REF] Core
vocabulary* (intake constants and the deploy refusal's typed error).

Two user rulings of 2026-10-05 shape it and are recorded there:

- **D15 (APPROVED):** guests keep today's network behaviour wherever the
  kernel allows it — the workload address is present in the guest and is what
  `getsockname` returns; programs see original peer addresses; inbound shows
  the real client address; UDP is general, not DNS-only; early data is never
  silently lost. Deferred: IPv6 (#308), UDP above 59,000 bytes (#309),
  inbound UDP service to VMs (#310), ICMP (#311).
- **Guest-side mechanism accepted at its real size** (APPROVED): the
  mechanism listed in the Decision below.

ADR-0082 §D4's READY meaning is **unchanged** (§ Consequences).

## Context

Overdrive workloads are unmodified: they open ordinary `AF_INET` sockets and
hold no platform material. ADR-0145 removes the guest NIC, so a guest
`connect()`, `sendto()` or `accept()` has no route of its own.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`, WORKS on
stock 7.0.0-29 host and guest, qualified metal, the CH fork with
`backend=vhost-kernel`) ran unmodified `curl`, `sshd`/`ssh`/`scp`, `getent`,
`dig` and busybox `nslookup` in a booted guest with only `lo` and a dummy
device. It established:

- D15 holds: the workload address is on the dummy device and in
  `getsockname`; `getpeername` and `recvfrom` return the original peer; `sshd`
  logged and exported the host's real client address; UDP of sizes 0 to
  59,000 bytes worked connected and unconnected.
- Early data is never lost: 10,000/10,000 in each stress (client writes then
  FINs, serial and 8-way; server speaks first, outbound and inbound; client
  writes immediately inbound; first UDP datagram, unconnected and connected).
- Only 8- and 16-byte control I/O crossed either owner's syscalls during
  64 MiB TCP, 8 MiB scp, DNS and 59,000-byte UDP.

It also established kernel facts that force the mechanism's shape (v7.0
source, reproduced on 7.0.0-29):

1. A TCP socket can be inserted into a SockHash only while `ESTABLISHED`; a
   client that writes and then FINs before the owner acts cannot be installed
   later (100/100 refused).
2. UDP and vsock deliver one skb per `data_ready` and have no re-arm, so bytes
   queued before install stay stranded (30/30 UDP; 18/100 vsock).
3. A TCP child installed before `accept()` ignores `data_ready` until it has a
   `struct socket`; `SO_RCVLOWAT` after `accept()` re-arms it without reading
   payload.
4. A FIN-only skb redirected to egress disables the target's TX with `EPIPE`.
5. Guest virtio-vsock RX splits host data into ≤4 KiB skbs.
6. `FIONREAD` on a psock TCP socket reports the psock queue, not the TCP queue.
7. Killing the guest owner made an in-flight application see a clean EOF with
   0 bytes, not an error (no `SO_LINGER{1,0}` was set).

## Decision

The guest adaptation stays in the guest kernel, loaded and controlled by
`overdrive-init`, which never reads or writes application payload:

- **Workload address.** `workload_addr/32` is configured on a guest dummy
  device, so binding it, `getsockname`, and kernel source selection all yield
  it. The guest has no other non-loopback device.
- **Capture.** Root-cgroup `connect4` and `sendmsg4` programs rewrite every
  non-local IPv4 destination to a guest intake on the workload address, and
  keep the original destination per socket. `getpeername4` and `recvmsg4`
  restore the original destination to the application.
- **Install at establishment.** A guest `sock_ops` program installs each TCP
  intake child into the SockHash at `PASSIVE_ESTABLISHED`, and each
  acceptor-side active socket (armed before `connect()`) at
  `ACTIVE_ESTABLISHED`. `overdrive-init` re-arms an intake child with
  `SO_RCVLOWAT` immediately after `accept()`.
- **Kernel parking.** Until a flow is paired, an intake child's bytes are
  redirected into a pooled loopback TCP parking cell, never left in a queue
  that cannot be re-armed.
- **Reserved intake ports (D25).** The guest's intakes live on the workload
  address at a fixed reserved port range above the kernel's default ephemeral
  range: one TCP intake port and one UDP port per datagram slot (values in the
  feature delta). `overdrive-init` binds them before READY without
  `SO_REUSEADDR` / `SO_REUSEPORT` and marks the range reserved
  (`net.ipv4.ip_local_reserved_ports`), so an application can never share an
  intake and the kernel never hands out a reserved port as an ephemeral port.
  An application bind on the workload address or `0.0.0.0` fails with
  `EADDRINUSE` every time only on a port that holds an intake of the same
  protocol: TCP on the TCP intake port, UDP on a slot port. A TCP bind on a
  slot port, or a UDP bind on the TCP intake port, succeeds (the reservation
  covers ephemeral assignment only). The TCP intake is never
  reported as an application listener (ADR-0163). A workload spec that
  declares a listener port inside the reserved range, of either protocol, is
  refused at deploy, with a typed spec-validation error naming the port and
  the range.
- **UDP.** Each guest UDP slot owns an intake that is SockHash-installed from
  boot. Its verdict frames every datagram (ADR-0149) into a strparser cell
  that feeds exactly one frame per verdict to the association's SEQPACKET.
  Host→guest frames are reassembled by a second strparser cell; a TCX program
  on guest `lo` removes the frame before the application receives it.
- **UDP slot identity (D25).** A slot belongs to one (application socket,
  original destination) pair; one application socket that sends to N
  destinations holds N slots. `overdrive-init` holds the authoritative
  slot-ownership table, read from the kernel slot map.
- **UDP slot release (D25).** A slot is released when its application socket
  is released — every slot of that socket, not only the last — or when the
  slot's association has been idle for the pinned idle period. A
  `sock_release` hook reports the released socket; if its report cannot be
  queued, the level-triggered audit in `overdrive-init` finds slots whose
  socket no longer exists and releases them. Release ends the association,
  discards every frame parked in the slot's cells (the cells are emptied or
  recreated), and only then returns the slot to the pool. A released slot is
  never re-associated with the departed application's destination; a late
  `Paired` for its flow is answered with `Abort` (user ruling 2026-10-06).
  (The off-host spike showed the hazard: released slots returned to the pool
  still holding frames, were re-associated to the old destination, and later
  first datagrams on those slots failed, 12/200; the spike's release path
  also returned only the last slot of a multi-destination socket.)
- **An ended association does not re-associate by itself (U-2, user ruling
  2026-10-06).** When an association whose slot is still owned ends without
  pairing or is aborted — `Refused`, `Abort`, or the pairing deadline — the
  guest discards every frame parked in the slot's cells and keeps the slot
  for its (application socket, destination). The discarded frames never start
  a new association; only a datagram the application sends after the end
  does. A destination that refuses every attempt is therefore tried at most
  once per pairing round trip, driven by the application's own sends, never
  by a retry loop.
- **Pool exhaustion (D25).** With no free parking cell, a new outbound TCP
  connection is not installed and the application sees a reset. With no free
  UDP slot, the capture program refuses the application's `connect()` or
  `sendmsg()` to a new destination (`EPERM`). Both are counted.
- **Inbound.** The guest socket that connects to the application binds the
  real client address with `IP_TRANSPARENT` and a platform `SO_MARK`; a
  fwmark rule to the local table returns the application's replies to it, and
  `net.ipv4.fwmark_reflect=1` makes the guest kernel's own replies (a reset
  for a port with no listener) carry the mark, so a refusal is delivered at
  once instead of after the connect timeout.
- **FIN and half-close.** Every TCP verdict drops zero-length skbs. FIN is
  propagated by the owner with `shutdown(SHUT_WR)` only after the drain signal
  shows the forwarding backlog has drained (ADR-0160).
- **Owner-crash visibility.** Owner-held TCP sockets on the application side
  carry `SO_LINGER{1,0}` from creation, so an owner exit resets the
  application's connection rather than ending it cleanly. A clean close clears
  linger first (ADR-0160). Untested (V-13).
- **Loss of the owner fails closed.** Every guest program attachment is a BPF
  link held by `overdrive-init`; its exit detaches the hooks, so new
  connections fail with `ENETUNREACH` instead of reaching anything.
- **Control.** `overdrive-init` opens the per-VM control session (ADR-0166),
  writes 16-byte flow requests on flow vsock connections, and installs and
  removes guest pairs.
- **Programs.** The guest programs are Aya Rust in `overdrive-bpf`, built by
  the one eBPF pipeline (ADR-0038) and embedded in the guest image.

## Alternatives considered

- **Install after `Paired` (late install).** Falsified by facts 1 and 2.
  Rejected.
- **A guest kernel module hooking socket operations** (as #303's module hooks
  `tcp_prot`). C, out-of-tree, with open teardown hazards. Rejected.
- **A guest userspace shim.** Payload would cross guest userspace. Rejected by
  constraint.
- **vsock-aware workloads.** Breaks the unmodified-workload model. Rejected.
- **Present inbound peers as loopback.** The kernel does not force it
  (`IP_TRANSPARENT` works), so it would break D15. Rejected.

## Consequences

- **READY keeps its meaning.** Today READY means guest platform init,
  including guest network configuration, completed (ADR-0082 §D4). Guest
  network configuration is now: the dummy device carries the workload address,
  the fwmark rule and `fwmark_reflect` are set, the guest programs are loaded
  and attached, the listener map is seeded (ADR-0163), and the inbound vsock
  listener is bound. READY waits for exactly that and adds no
  dependency on the host. The per-VM control session is **not** a READY
  precondition; host activation requires it instead (feature delta, G-V3).
- **`overdrive-init` is the forwarding owner and is PID 1.** If it dies the
  guest kernel panics and the VM ends. Its own exit paths and the window
  before a panic are covered by the link-detach and `SO_LINGER` rules above.
- **Guest kernel requirements:** BPF with BTF, cgroup-v2 socket hooks,
  `sock_ops`, SK_SKB with strparser, sockmap for TCP, UDP and vsock STREAM and
  SEQPACKET, TCX, `fexit`, `CONFIG_VIRTIO_VSOCKETS`, `CONFIG_DUMMY`,
  `CONFIG_INET_DIAG` and `CONFIG_INET_TCP_DIAG` (present in the guest image,
  built in or as modules the image carries), policy routing, and
  `IP_TRANSPARENT`. Observed on stock 7.0.0-29 only.
- **Residual D15 differences (D15-R2, approved with D25):** a reserved port
  range on the guest is
  unavailable to applications; a VM can hold at most the pinned number of
  concurrent UDP (socket, destination) pairs and outbound TCP flows; an idle
  UDP association ends after the idle period.
- **Both owners' `connect()` calls are non-blocking** (ADR-0158).
- **Guest kernel command line** gains a transport token carrying the workload
  address (feature delta, § *Guest adaptation contract*).
- **Inbound UDP service** would need TC-level classification because an
  SK_SKB verdict on an unconnected UDP socket sees payload only (#310).
