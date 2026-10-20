# Research: Unikraft as the guest side of the vsock data path (EVMOD guest forwarder)

**Date**: 2026-10-17 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium (code-read, not run; no spike executed) | **Sources**: 3 local codebases, the Linux 7.0 vendored source, the feature docs, 6 web pages

Labels: **FACT** (read in code or fetched from a page), **MEASURED** (taken from an Overdrive spike record), **INFERENCE** (my reasoning from facts), **UNVERIFIED** (not confirmed). No claim here was run on a Unikraft guest.

Paths: `UK` = `/Users/marcus/git/unikraft/unikraft`, `LWIP` = `/Users/marcus/git/unikraft/lib-lwip`, `ELF` = `/Users/marcus/git/unikraft/app-elfloader`, `LNX` = `vendors/linux` (v7.0). Checkouts are fresh clones of `staging` (clone timestamps 1790421572 / 1790422351, i.e. about 2026-09-27) (FACT: `UK/.git/logs/HEAD:1`, `LWIP/.git/logs/HEAD:1`).

## Executive Summary

1. **Unikraft now has a virtio-vsock driver and an AF_VSOCK posix-socket family, but it is days-old code.** The driver (`UK/drivers/virtio/vsock/virtio_vsock.c`) and the socket library (`UK/lib/ukvsockdev/`) landed as upstream PR #1800, merged 2026-06-05 (FACT, web). It does STREAM with credit flow control, shutdown, RST, an accept queue and poll events through `posix-socket`, so unmodified ELF binaries under app-elfloader reach it through the ordinary `socket(AF_VSOCK, ...)` syscall. It has gaps that matter here: no socket options, no `MSG_PEEK`, no `recvmsg`/`sendmsg`, no SEQPACKET record semantics, a fixed 64 KiB buffer, one device, and, most important, **the Unikraft virtio-pci transport is legacy-only and rejects the modern device IDs Cloud Hypervisor exposes**, so the driver cannot attach to Overdrive's CH fork today (INFERENCE from two FACTs, below).
2. **A guest forwarder is a micro-library, not a module.** Unikraft is one address space linked at build time, so there is no vermagic and no per-kernel module build. The guest-kit coupling (EVMOD-GUEST-KIT) disappears for Unikraft guests because the "kit" is the image build itself. The peek-send-consume pattern is not needed: a read-into-scratch loop bounded to one chunk per direction gives the same memory bound (INFERENCE), and lwIP does support `MSG_PEEK` without opening the window (FACT, upstream).
3. **The simplest correct guest hop is not a forwarder at all: a patched AF_INET socket family that terminates the workload's TCP and UDP sockets directly onto vsock** (Option B below). posix-socket dispatches by address family through a link-time table with a one-driver-per-family assertion, and lwIP currently owns AF_INET. A replacement family driver removes the lwIP hop, the second queue, the forwarder's end-classification and the per-pair state. It costs a new socket layer (connect handshake, getsockname/getpeername synthesis, UDP framing, loopback delegation) and a transport patch. It composes naturally with #303 because that work already patches the same layer.
4. **Ranking for a Unikraft guest**: B (AF_INET family shim onto vsock) > A (in-image forwarder library between lwIP and vsock) > C (Linux-style module, not applicable). For Linux guests, EVMOD-GUEST-KIT and `ovd_fwd` stay as designed; nothing here weakens them.
5. **Two blockers need a spike before any Unikraft-guest decision**: the legacy-only virtio-pci (CH compatibility), and the missing proactive `CREDIT_UPDATE` (a host-to-guest bulk stall hazard).

## Research Methodology

**Search Strategy**: read the three local Unikraft checkouts directly (grep then read); read the Overdrive feature delta and spike findings; read the vendored Linux 7.0 and Cloud Hypervisor sources for the peer behaviour; web-fetch the upstream PR, lwIP source, Firecracker docs and the Prisma write-up for context. **Source Selection**: primary source code over prose; web pages are official project pages or the vendor blog. **Verification**: each behaviour claim cites a file and line; where I could only infer, the label says so. **Limits**: Bash was unavailable, so nothing was built or run; web summaries come from a fetch tool that paraphrases pages, so quoted lines from PR #1800 are second-hand.

## Findings

### Q1. Unikraft's vsock driver and AF_VSOCK API

**F1.1 (FACT, High)** The stack is three layers. `UK/drivers/virtio/vsock/virtio_vsock.c` is the virtio transport; `UK/lib/ukvsockdev/` holds the device interface and the socket logic (`vsock.c`, 1680 lines); the socket family registers with `POSIX_SOCKET_FAMILY_REGISTER(AF_VSOCK, &socket_vsock_ops)` (`UK/lib/ukvsockdev/vsock.c:1680`). `ukvsockdev` selects `LIBPOSIX_SOCKET` (`UK/lib/ukvsockdev/Config.uk:5`). Nothing mentions vsock in lib-lwip (only two unrelated `if_arp.h` hits) or in app-elfloader (no hits).

**F1.2 (FACT, High, web)** Upstream PR #1800 "drivers/virtio: Add `virtio-vsock` support" is merged, 2026-06-05; older PRs #945 (2023) and #249 (2021) are open. Source: [github.com/unikraft/unikraft/pulls?q=vsock](https://github.com/unikraft/unikraft/pulls?q=vsock); [PR #1800](https://github.com/unikraft/unikraft/pull/1800) (fetched summary: developed and tested on Firecracker virtio-mmio; review concerns were integer overflow and peer-controlled credit values, addressed before merge). The v0.21.0 release notes (2026-04-20) do not mention vsock ([unikraft.org](https://unikraft.org/blog/2026-04-20-unikraft-releases-v0.21.0)), consistent with the merge landing after that release.

**F1.3 Feature table (FACT, read in code)**

| Feature | State | Evidence |
|---|---|---|
| STREAM | Works: REQUEST/RESPONSE handshake, accept queue (backlog max 128), RW, SHUTDOWN, RST | `virtio_vsock.c:1074-1120`; `UK/lib/ukvsockdev/include/uk/vsock.h:59` |
| SEQPACKET | Negotiated and created at driver level, but the socket library has no record logic (no `SEQPACKET`, `MSG_TRUNC`, `MSG_EOR` in `lib/ukvsockdev/*.c`; read and write treat the socket as a byte stream) | `virtio_vsock.c:1289-1295`, `:1982-1988`; grep over `UK/lib/ukvsockdev` returned no `SEQPACKET` hits in `.c` files |
| Credit flow control | Peer credit honoured on TX (`virtio_vsock.c:175-188`, `:1485-1494`); our `buf_alloc` and `fwd_cnt` ride on every TX header (`:1518-1519`) | |
| Proactive credit update | **Absent.** `CREDIT_UPDATE` is built in exactly one place, the reply to a peer's `CREDIT_REQUEST` (`virtio_vsock.c:814-862`, op at `:836`); `vsock_read` consumes with no send (`vsock.c:1555`) | grep of `OP_CREDIT_UPDATE` over `UK`: lines 836, 1109 only |
| Shutdown | `SHUT_RD/WR/RDWR` sent as `SHUTDOWN` flags (`vsock.c:1070-1121`, `virtio_vsock.c:1603-1652`); peer SHUTDOWN and RST handled (`:1089-1099`) | |
| Socket options | `setsockopt` returns `-ENOPROTOOPT` for everything; `getsockopt` supports only `SO_ERROR` (`vsock.c:1276-1323`). So `SO_VM_SOCKETS_BUFFER_SIZE` cannot be set | |
| Buffer | Per-socket receive ring, default and max 65,536 B (`UK/lib/ukvsockdev/Config.uk:10-26`); TX packet cap 64 KiB (`virtio_vsock.c:36`); RX descriptors are one page each (`:246`) | |
| `MSG_PEEK` and flags | `recvfrom`/`sendto` ignore flags ("FIXME: Add support for flags", `vsock.c:1574`, `:1463`); `recvmsg`/`sendmsg` are `NULL` (`:1669-1671`); `ioctl` returns `-ENOTTY` (`:1633`) | |
| Devices | One: handlers call `uk_vsockdev_get()` (`vsock.c:772`, `:1414`) | |
| Events | `EPOLLIN/OUT/ERR/HUP/RDHUP` set through `posix_sock_event_*` (`vsock.c:1563`, `:1625`, `:1445`); `poll_setup` links the posix socket (`:1640-1650`) | |
| Threads | RX and event queue work runs in dispatcher threads by default (`Config.uk:28-38`; `virtio_vsock.c:1857-1880`) | |

**F1.4 (FACT) Transport: legacy virtio-PCI only.** `virtio_pci_legacy_add_dev` rejects any PCI device ID outside `0x1000-0x103f` (`UK/drivers/virtio/pci/virtio_pci.c:375`); the only config ops table is `vpci_legacy_ops` (`:102`). The same PR thread is reported to say "Unikraft currently only supports legacy virtio PCI, while the Linux vhost-vsock driver requires modern virtio PCI" ([PR #1800](https://github.com/unikraft/unikraft/pull/1800), second-hand). The virtio-mmio transport does handle `VIRTIO_F_VERSION_1` (`UK/drivers/virtio/mmio/virtio_mmio.c:120`), which is why the PR was tested on Firecracker.

**F1.5 (FACT) Cloud Hypervisor uses modern device IDs.** `VIRTIO_PCI_DEVICE_ID_BASE: u16 = 0x1040` (`vendors/cloud-hypervisor/virtio-devices/src/transport/pci_device.rs:293`), so a CH vsock device is `0x1040 + 19`. **INFERENCE (High)**: stock Unikraft will not probe any virtio-PCI device that Overdrive's CH fork exposes, vsock included, because its legacy-only probe rejects the ID. Whether CH offers a virtio-mmio transport on x86 is UNVERIFIED (not checked). The options are a modern virtio-PCI transport patch in Unikraft (new work, the larger one) or a different VMM.

**F1.6 (FACT) Backend compatibility.** Cloud Hypervisor's `vhost-kernel` vsock backend supports stream and seqpacket and keeps the data path in the host kernel (`vendors/cloud-hypervisor/docs/vsock.md:5-6`, `:88-93`); its default `unix` backend is stream-only. Firecracker mediates AF_VSOCK to host AF_UNIX sockets and documents no credit or seqpacket behaviour ([firecracker docs/vsock.md](https://github.com/firecracker-microvm/firecracker/blob/main/docs/vsock.md)). So Unikraft's tested path (Firecracker mmio, userspace muxer) is not Overdrive's path (CH, vhost-kernel, CID leases, peer-CID attribution, orphan sweep) (INFERENCE).

**F1.7 (INFERENCE, Medium) Host-to-guest bulk stall.** Linux's receiver sends a `CREDIT_UPDATE` after a dequeue once free space is low (`LNX/net/vmw_vsock/virtio_transport_common.c:644-655`), and a Linux sender never sends `CREDIT_REQUEST` (the only hits in `net/vmw_vsock` are the receive case at `:1437-1438` and the packet-type switch at `:209`; `notify_send_pre_block` is a no-op, `:1026-1030`). Unikraft sends credit only piggybacked on its own outgoing packets (F1.3). Therefore a host sender that has used the guest's 64 KiB window and receives no guest packet carrying a newer `fwd_cnt` would block. Request-response traffic hides this; a long one-way upload to the guest would hit it. Needs a spike (host sends 1 MiB to a Unikraft guest that reads and does not reply). The fix is a small driver patch: send `CREDIT_UPDATE` from `uk_vsock_buffer_consume` when free space crosses a threshold, as Linux does.

**F1.8 (FACT)** ELF binaries use it through the syscall shim: `UK_SYSCALL_R_DEFINE(int, socket, ...)` dispatches to the family driver (`UK/lib/posix-socket/socket.c:317`, `:265-289`). app-elfloader ships defconfigs for QEMU and Firecracker only (`ELF/defconfigs/`) and says "Firecracker networking is not yet upstream" (`ELF/README.md:273`); there is no Cloud Hypervisor defconfig (FACT). Prisma Postgres runs on Unikraft Cloud with "a modified version of Firecracker VMM" ([Prisma blog](https://blog.prisma.io/blog/cloudflare-unikernels-and-bare-metal-life-of-a-prisma-postgres-query)); the page says nothing on vsock or elfloader. That is one production data point for Unikraft plus Firecracker, none for Unikraft plus CH.

**Maturity verdict (INFERENCE, High)**: usable for a guest-agent control channel on Firecracker; not a proven data path at 16k-flow density, not proven on CH, no throughput numbers published. Treat as alpha for our purposes.

### Q2. A forwarder as a Unikraft micro-library

**F2.1 (FACT)** No module problem. A library is linked at build time into one image, and its symbols resolve at link time. Drivers and libraries register through linker sections (`posix_socket_driver_list_start/_end`, `UK/lib/posix-socket/driver.c:45-46`) and init classes (`uk_initcall_class_prio`, `driver.c:121`). There is no vermagic equivalent, no `finit_module`, no `.ko` path, no `ForwarderModuleRelease`. **INFERENCE (High)**: EVMOD-GUEST-KIT (one artifact per guest kernel release, checked by `overdrive-init` at `GUEST_FORWARDER_MODULE_PATH`) has nothing to guard for a Unikraft guest. The image build is the kit, and the version pin is the Unikraft commit.

**F2.2 (FACT) Hook points.** Two usable surfaces:
- *posix-socket (file descriptors).* Both sides are `uk_file` objects with poll events. lwIP signals them from its event callback (`LWIP/sockets.c:467-484`, `posix_sock_event_assign`), vsock sets them directly. A forwarder thread can `epoll` both. lwIP socket support requires `LWIP_THREADS` (`LWIP/Config.uk:324`), so the tcpip thread exists, and `LWIP_NETCONN` is 1 whenever threads are on (`LWIP/include/lwipopts.h:332-335`).
- *Internal vsock API.* `uk_vsock_buffer_read` is documented non-consuming and `uk_vsock_buffer_consume` separate (`UK/lib/ukvsockdev/include/uk/vsock.h:170-203`). An in-image forwarder can peek and consume natively, which the posix wrapper does not expose (F1.3). Whether these symbols are linkable from another library is UNVERIFIED (`exportsyms.uk` exists, `UK/lib/ukvsockdev/exportsyms.uk`; I did not read it, and in a static link it may not matter).
- The `posix-socket` event receivers (`UK_SOCKET_EVENT_RECEIVER`) are observe-only: the generated handler always returns `UK_EVENT_HANDLED_CONT` (`UK/lib/posix-socket/include/uk/socket_event.h:68-82`). They can track CREATE/BIND/LISTEN/ACCEPT/CONNECT but cannot redirect a connect. They are not a capture mechanism (the analogue of our cgroup `connect4`).

**F2.3 (FACT, upstream) lwIP peek.** `lwip_recv_tcp` skips `netconn_tcp_recvd` when `MSG_PEEK` is set and keeps `lastdata`, so a peek does not reopen the TCP window ([lwIP sockets.c](https://raw.githubusercontent.com/lwip-tcpip/lwip/master/src/api/sockets.c), fetched summary; Unikraft uses its own fork, `fork-lwip`, fetched at build by `LWIP/Makefile.uk:46-60` plus 14 patches; the fork's source is not local, so the claim is for upstream and UNVERIFIED for the fork). `MSG_TRUNC` is not an input flag for UDP; it is only set on output when the buffer is short (same page).

**F2.4 (INFERENCE, Medium) Backpressure without peek.** Because the vsock side's peek is unavailable through posix, use the bounded-scratch form: read at most one chunk per direction into a per-pair buffer (this consumes it and returns that much credit or window), write it to the target, and if the target accepts less, keep the remainder and stop reading that source until the remainder is flushed on the target's writable event. Memory per stalled direction is then source window + target send buffer + one chunk, the same shape as the Linux bound (`feature-delta.md` § "Memory and backpressure bounds", MEASURED 0.25-8 MiB per pipeline there). The exact Linux pattern (peek, send, consume after accept) is better by one chunk and is possible only with the internal API above.

**F2.5 (INFERENCE)** Threading: one forwarder thread on `uk_sched` with epoll (no per-pair thread; Unikraft's cooperative scheduler would make per-pair threads expensive at density). Event-driven wakeups exist on both sides. The per-pair cost is unmeasured; Linux's is MEASURED at about 248 B plus a scratch pool.

**F2.6 (FACT, a real cost)** The forwarder keeps the lwIP TCP stack (tcpip thread, pbuf pools, `TCP_WND 32766` default or 262142, `lwipopts.h:186-195`) plus a no-NIC netif problem: lwIP's init iterates `uk_netdev_count()` (`LWIP/init.c:202`) and a loopback interface is optional (`LWIP/Config.uk:49-54`). Whether lwIP initializes cleanly with zero netdevs and only loopback is UNVERIFIED. The workload's `connect()` also needs a route for the frontend address `F`: lwIP would need a synthetic interface or a loopback-routed capture. That capture is exactly what the Linux guest does with cgroup hooks and nft, and Unikraft has no equivalent (F2.2). **INFERENCE (High)**: Option A therefore needs its own capture patch anyway, which erodes its simplicity advantage.

### Q3. Terminate the workload's connect/accept directly onto vsock

**F3.1 (FACT) The architecture allows it.** `socket()` looks up one driver by family (`posix_socket_driver_get`, `UK/lib/posix-socket/driver.c:57-70`, called from `socket.c:267`) and every later syscall goes through that driver's `posix_socket_ops` table (`socket_driver.h:471-500`). `driver.c:106` asserts only one driver per family. lwIP registers AF_INET and AF_INET6 (`LWIP/sockets.c:535-538`) with a **static** ops table (`:511`). So the shim is a second driver for AF_INET, built with lwIP's registration removed or patched out, and with `lwip_posix_socket_ops` made non-static if it needs to delegate (INFERENCE from the facts).

**F3.2 (INFERENCE, High) What the shim must do**, per syscall:

| Syscall | Shim behaviour | Cost |
|---|---|---|
| `socket(AF_INET, SOCK_STREAM)` | Return an unbound shim object. The destination is unknown, so the vsock or lwIP socket is created lazily | State machine |
| `connect(F:port)` | Open a vsock STREAM to CID 2 on the flow port, send the request frame (destination, as the host acceptor reads before attach, "H-1" in the delta), complete as vsock connect completes | Request framing, ECONNREFUSED mapping from host refusal |
| `bind/listen/accept` | Map the listening port to a vsock listen port, so a host `GuestStreamDial` reaches the workload directly, with no guest accept-then-connect-local hop | Port mapping, inbound peer address carried in a header |
| `getsockname/getpeername` | Synthesize AF_INET from the request frame (workload address, peer) because vsock gives only CID and port (`vsock.c:1325-1400`) | Must carry the peer IP in the frame |
| `shutdown/close` | Pass through (`vsock.c:1070-1134`); FIN maps to SHUTDOWN_SEND; reset fidelity is the host-side R2-END rule | Already specified host-side |
| `setsockopt/getsockopt` | `TCP_NODELAY`, `SO_REUSEADDR`, `SO_KEEPALIVE` etc. must be accepted (return 0) or workloads like Postgres fail on setup; vsock itself rejects all (F1.3) | Table of accepted no-ops |
| `recvmsg/sendmsg/MSG_PEEK` | vsock has none (F1.3); the shim needs them for any app using `readv`-style I/O | Patch the vsock family |
| `poll/epoll` | Reuse vsock events | Free |
| 127.0.0.0/8 and in-guest traffic | The workload may talk to itself or a sibling process in the same image. Needs delegation to lwIP loopback or a shim-internal loopback | Shim must hold both a vsock and an lwIP socket per fd, decided at connect/bind |

**F3.3 DNS (INFERENCE).** `getaddrinfo` in musl or glibc sends UDP to the resolver address; the platform's dial-by-name (ADR-0072 per project notes) returns a frontend `F`. In the shim the resolver address is just another UDP destination, handled by the UDP path (Q4). If the platform's DNS responder stays in the host, the DNS datagram rides the same datagram flow as any other.

**F3.4 Connect semantics (INFERENCE, Medium).** vsock connect completes when the host kernel answers `RESPONSE`, before the host dials the real target, as for the guest's TCP intake today. A later host failure surfaces as a reset or EOF on an already-connected socket. Same behaviour as the Linux design, so no regression, but it is not the real ECONNREFUSED of a direct connect (the current design has the same property, `feature-delta.md` rows 1 and 21 of the disposition table keep "request read before attach").

**F3.5 Net effect (INFERENCE, High).** Option B removes: the lwIP TCP hop and its tcpip thread for mesh traffic, a second queue per flow, forwarder pair state, forwarder end-classification (the shim sees the vsock's own SHUTDOWN/RST), and the capture problem (F2.6), because the family driver *is* the capture. It adds: the shim itself and a transport patch. It makes the guest memory per flow one vsock socket (a 64 KiB ring at default, `vsock.h:100`) instead of two sockets plus a pair.

### Q4. UDP datagrams over vsock in Unikraft

**F4.1 (FACT)** Unikraft vsock has no datagram socket type; `vsock_create` accepts STREAM and (driver-level) SEQPACKET only (`virtio_vsock.c:1285-1299`), and the library has no record boundaries (F1.3). lwIP has UDP (`LWIP/Config.uk:231`).

**F4.2 (INFERENCE, High)** EVMOD-DGRAM's framing is the right shape for Unikraft because it needs only STREAM: one vsock STREAM per association carrying the 8-byte-header frame of ADR-0149 (`feature-delta.md:3633-3634`). A shim `SOCK_DGRAM` object does: `sendto` builds a frame and writes it to the association's stream (all-or-nothing, using the per-direction partial-frame count the Linux forwarder uses); `recvfrom` reads the header, then waits for the whole payload, then returns one datagram with the peer address from the frame. That removes both the SEQPACKET dependence (absent in Unikraft) and the `EMSGSIZE`/short-record machinery the old design carried (disposition row 9).

**F4.3 (FACT, tight fit)** The datagram design needs the vsock half's buffer to hold the largest frame, 59,008 B (`feature-delta.md:3650-3657`). Unikraft's default and maximum are 65,536 B and cannot be changed at runtime (`Config.uk:10-26`, `vsock.c:1316`), so it fits with 6.5 KB spare; a larger `MAX_GUEST_DATAGRAM_FRAME` would need a build-time config change on every guest. The Linux host half defaults to 256 KiB (MEASURED context in the delta), so the host side is not the constraint.

**F4.4 (INFERENCE)** `MSG_TRUNC`-style peek of the datagram length is not available on lwIP for the Option A UDP half (F2.3), so Option A's UDP-to-vsock leg would use `recvmsg` with a max-size buffer and a flag check instead of the Linux peek-with-trunc. Another reason for B, where the shim owns the datagram queue.

### Q5. Composition with the #303 in-guest kernel mTLS patching

**F5.1 (FACT, project context)** #303: the guest has no NIC; a kernel module owns mesh sockets; the vsock relay moves into the kernel design; a guest kTLS socket reaches the host through a loopback TCP peer carrying ciphertext; `overdrive-init` owns non-mesh sockets (`feature-delta.md:11385-11395`). The forwarder refuses any socket with a ULP (EVMOD-KTLS, `feature-delta.md:3718-3731`).

**F5.2 (INFERENCE, Medium)** On Unikraft the "kernel module" is a library inside the same address space and the mTLS state machine would sit where the family shim sits: it already sees every `connect`/`accept` and every byte. Option B puts TLS (mbedTLS-class library, handshake plus record layer) between the workload socket and the vsock STREAM in one place: plaintext application API on top, TLS records on the vsock below, no loopback peer and no "ciphertext forwarded by a second component". The two patches are therefore one patch to one layer. Option A would need the loopback-peer arrangement that Linux uses because kTLS lives on a kernel socket the forwarder cannot touch. Whether Unikraft has anything like kTLS is UNVERIFIED (no evidence found in the files searched); in-library TLS is the natural fit since there is no kernel/user boundary to offload across.

**F5.3 (INFERENCE)** Constraint carried over: #303's handshake/RESOLVE channel is guest-initiated to CID 2 port 7100 (`feature-delta.md:11403-11406`). The shim must keep ports 1234 and 1240-1243 free of workload mappings; the port-mapping in F3.2 (workload port to vsock port) collides with them if done by identity, so the mapping needs its own namespace (for example a port range or a request-frame discriminator), not an identity map.

### Q6. Bottom line

**Options, ranked for a Unikraft guest**

1. **Option B: AF_INET family shim terminating TCP and UDP onto vsock (recommended, INFERENCE).**
   - Pros: no forwarder, no lwIP on the mesh path, no capture patch, no module or kit, one layer shared with #303, EVMOD-DGRAM framing reused as is, per-flow memory is one vsock socket.
   - Cons: biggest new code; must also delegate loopback to lwIP (or implement it); must patch vsock for options, `recvmsg/sendmsg`, `MSG_PEEK`, credit update; must add modern virtio-PCI or change VMM (F1.4-F1.5). Non-mesh and in-guest traffic is out of the shim's reach if some workload needs a real NIC-less lwIP stack for it.
2. **Option A: in-image forwarder library between lwIP and vsock (INFERENCE).**
   - Pros: workloads use unmodified lwIP semantics, including loopback; closest to the Linux design, so the host-side contracts (`FlowForwarder`, end classification) carry over conceptually.
   - Cons: still needs a capture mechanism Unikraft lacks (F2.6); two queues per flow; pair-state and end-classification code to rewrite for a different stack; UDP leg lacks `MSG_TRUNC` peek (F4.4). The simplicity that a library gives over a module (F2.1) is real, but it removes only the build-coupling, not the design cost.
3. **Option C: port `ovd_fwd` to Unikraft (not viable).** It is GPL-2.0 kernel code built on `sock_recvmsg`, `sk_state_change` overrides and `tcp_abort` (`feature-delta.md:3175-3178`); none of those exist in Unikraft. Listed to close the question.

**What must be patched or written (Option B)**
1. Modern virtio-PCI transport in `UK/drivers/virtio/pci` (or move the guest to a VMM with virtio-mmio). **Contract-shaping**: it decides the VMM. Spike first.
2. vsock library: proactive `CREDIT_UPDATE` on consume; `recvmsg/sendmsg`; honour `MSG_PEEK`; accept socket options; optional runtime buffer size. Spike the stall (F1.7) first.
3. The AF_INET family driver: request framing, port mapping namespace, getsockname/getpeername synthesis, option no-ops, UDP association over STREAM, loopback delegation to a patched-visible lwIP ops table.
4. Remove lwIP's AF_INET registration from the mesh build (or make it the delegate behind the shim).
5. A host-side check that a Unikraft guest's vsock behaves as `GuestStreamDial` and the host forwarder assume (peer CID, launch token, orphan-sweep reset, R2-END classes), as the Linux guest's was measured.

**Does it remove EVMOD-GUEST-KIT coupling?** For Unikraft guests, yes (F2.1): there is no loadable guest module, so `ForwarderModuleMissing/Release/Load`, the `vermagic` read and `kmod-build` per guest kernel do not apply; coupling moves to "this Unikraft commit plus these patches", pinned by the image factory (#259). It does not remove the host-side `ovd_fwd` (the host forwarder still joins vsock to TCP) or the need to publish a Unikraft kit per supported VMM transport (PCI-modern vs mmio).

**Linux guests (if both types ship)**: everything in EVMOD stays for Linux guests: `ovd_fwd` per kernel release, EVMOD-GUEST-KIT, consume-after-accept, EVMOD-DGRAM framing, EVMOD-KTLS. The wire contract (flow ports, request frame, datagram frame, end/abort classes on vsock) must be a **guest-neutral protocol** so the host forwarder and `GuestStreamDial` do not branch on guest type. That is the main design consequence of supporting both: the Unikraft shim speaks the same frames the Linux guest owner speaks (INFERENCE).

## Source Analysis

| Source | Domain | Reputation | Type | Accessed | Cross-verified |
|---|---|---|---|---|---|
| unikraft/unikraft (clone) | local, github.com/unikraft | High | primary code | 2026-10-17 | Y (PR page) |
| unikraft/lib-lwip (clone) | local | High | primary code | 2026-10-17 | N |
| unikraft/app-elfloader (clone) | local | High | primary code | 2026-10-17 | N |
| Linux v7.0 vendored | `vendors/linux` | High | primary code | 2026-10-17 | Y (spike records) |
| Cloud Hypervisor fork | `vendors/cloud-hypervisor` | High | primary code | 2026-10-17 | N |
| PR #1800 and PR list | github.com | High | official project | 2026-10-17 | Partly (fetch paraphrase) |
| lwIP sockets.c | github.com/lwip-tcpip | High | primary code | 2026-10-17 | N (fork not local) |
| Firecracker vsock.md | github.com/firecracker-microvm | High | official docs | 2026-10-17 | Y (CH doc says same base) |
| Prisma blog | blog.prisma.io | Medium-high | vendor | 2026-10-17 | N |
| Unikraft v0.21.0 release | unikraft.org | High | official | 2026-10-17 | N |
| Overdrive feature-delta and spike findings | repo | High (own) | internal | 2026-10-17 | Y |

Reputation: High 10 of 11, medium-high 1 of 11.

## Knowledge Gaps

### Gap 1: Cloud Hypervisor plus Unikraft boot and vsock
**Issue**: no evidence Unikraft boots on CH or attaches virtio-PCI there. **Attempted**: code read of both PCI paths; searched the PR and web. **Recommendation**: spike S-U1: boot a Unikraft vsock echo guest on the CH fork, record `uname -r` of the host (Linux 7.0).

### Gap 2: Host-to-guest credit stall (F1.7)
**Issue**: inferred from code, not run. **Recommendation**: S-U2: host sends 1 MiB to a Unikraft reader that does not reply; compare with and without a credit-update patch.

### Gap 3: lwIP with no NIC, and the fork's `MSG_PEEK`
**Issue**: `fork-lwip` source is not local; zero-netdev init unconfirmed. **Recommendation**: read the fetched fork or run an image with only loopif.

### Gap 4: Scale and throughput
**Issue**: no published numbers for Unikraft vsock; page-sized RX descriptors (`virtio_vsock.c:246`) and a copy per side suggest low throughput. **Recommendation**: S-U3: bulk and 1 KiB-message throughput at 1, 1k, 16k flows against the Linux numbers (P-67, P-68).

### Gap 5: Unikraft TLS offload and kTLS
**Issue**: not found; #303 on Unikraft is a library design question. **Recommendation**: include in the #303 design before choosing a layer.

### Gap 6: `exportsyms.uk` for ukvsockdev and in-image linkage of `uk_vsock_buffer_*`
**Recommendation**: read it when an Option A or B design starts.

## Conflicting Information

### Conflict 1: "vhost-vsock compatible?"
**Position A**: the driver "supports" vsock, tested on Firecracker mmio (PR #1800 summary). **Position B**: Linux vhost-vsock needs modern virtio-PCI, which Unikraft lacks (same PR thread, second-hand) and `virtio_pci.c:375` rejects modern IDs. **Assessment**: both hold; they are about different transports. The code read favours B for CH.

## Recommendations for Further Research
1. Run S-U1 and S-U2 above before any commitment; they are contract-shaping for a Unikraft guest (`.claude/rules/design.md` § "Run contract-shaping validation").
2. Decide guest-neutral wire frames now, so Linux and Unikraft guests share the host forwarder.
3. Read `fork-lwip` and Unikraft's TLS libraries for the #303 layer choice.

## Full Citations
[1] Unikraft. `unikraft/unikraft` staging, files cited above. Local clone, 2026-09-27.
[2] Unikraft. `unikraft/lib-lwip` staging. Local clone.
[3] Unikraft. `unikraft/app-elfloader` staging. Local clone.
[4] Unikraft contributors. "drivers/virtio: Add virtio-vsock support" PR #1800. https://github.com/unikraft/unikraft/pull/1800. Accessed 2026-10-17.
[5] Unikraft. Pull requests mentioning vsock. https://github.com/unikraft/unikraft/pulls?q=vsock. Accessed 2026-10-17.
[6] Unikraft. "Unikraft releases v0.21.0". 2026-04-20. https://unikraft.org/blog/2026-04-20-unikraft-releases-v0.21.0. Accessed 2026-10-17.
[7] lwIP. `src/api/sockets.c`. https://raw.githubusercontent.com/lwip-tcpip/lwip/master/src/api/sockets.c. Accessed 2026-10-17.
[8] Firecracker. "Vsock device". https://github.com/firecracker-microvm/firecracker/blob/main/docs/vsock.md. Accessed 2026-10-17.
[9] Prisma. "Cloudflare, Unikernels & Bare Metal: Life of a Prisma Postgres Query". 2025-02-05. https://blog.prisma.io/blog/cloudflare-unikernels-and-bare-metal-life-of-a-prisma-postgres-query. Accessed 2026-10-17.
[10] Overdrive. `docs/feature/netns-density-295/feature-delta.md` (§ EVMOD, § Guest-mTLS boundary) and `spike/findings-evmod.md`, `spike/findings-v3-guest-slot.md`.
[11] Linux v7.0 (vendored). `net/vmw_vsock/virtio_transport_common.c`. Cloud Hypervisor fork `docs/vsock.md`, `virtio-devices/src/transport/pci_device.rs`.

## Research Metadata
Duration: one session | Examined: about 40 files and 9 web pages | Cited: 11 | Confidence: High 40 %, Medium 45 %, Low 15 % | Output: `docs/research/networking/unikraft-guest-forwarder-research.md`
