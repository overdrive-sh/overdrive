# ADR-0150 — Ordinary sockets of a Linux guest reach the vsock transport through guest-kernel Aya capture and the in-kernel forwarder, controlled by `overdrive-init`

## Status

**Accepted.** Decision D6 approved by the user 2026-10-05; decision D25, the
guest intake model (reserved ports, slot identity and release, pool sizes,
exhaustion outcomes, deploy refusal of a reserved port) with its residual D15
difference D15-R2, approved 2026-10-06; the exact bind-refusal scope
(D25-BIND) and the end of an unpaired or aborted datagram association (U-2),
approved 2026-10-06; the guest side of INB-1, R2-END and R2-DRAIN approved
2026-10-11; the guest's forwarding by the in-kernel forwarder (ADR-0197,
EVMOD-ADOPT) approved 2026-10-16; attach at pairing (EVMOD-PAIR), datagram
associations as one framed stream (EVMOD-DGRAM, ADR-0201), end
classification in the forwarder (EVMOD-END, ADR-0180) and the Linux guest
kit (EVMOD-GUEST-KIT, ADR-0206) approved 2026-10-17; TCP-CORRELATION and its
bounded ADR-0159 staging-pin exception, and the bounded guest UDP-slot
attach-refusal carrier and regression cell approved 2026-10-23. GH #295.
Recorded in the #295 feature delta, § *[REF] Guest adaptation contract*, § *[REF] Core
vocabulary* for the intake constants and the deploy refusal's typed error,
and § *[REF] vsock data path — the in-kernel event-driven forwarder
(EVMOD)*. Not yet implemented.

**TCP-CORRELATION amendment (approved 2026-10-23).** Linux 7.0 assigns the application's local TCP port
after `connect4`, while the accepted guest-owner event requires that port's
accepted intake child to carry the application's socket cookie and original
destination. The approved correction adds one SockOps program that carries
that metadata in a guest-internal option on the handshake-completing ACK and
materializes it under the exact accepted child cookie, while preserving the
five accepted capture/lifecycle programs. Its exact interface and evidence
are in the #295 feature delta,
§ *[REF] TCP capture correlation*. Its one parameterized existing-lane Tier-3
cell remains a required STEP 11-06 validation.

This ADR is the guest side of a **Linux** guest. The host side is
guest-neutral (ADR-0207); a guest that runs no Linux kernel is
[GH #321](https://github.com/overdrive-sh/overdrive/issues/321).

Two user rulings of 2026-10-05 shape it:

- **D15 (APPROVED):** guests keep today's network behaviour wherever the
  kernel allows it — the workload address is present in the guest and is what
  `getsockname` returns; programs see the original peers of their outbound
  connections; UDP is general, not DNS-only; early data is never silently
  lost. Inbound connections appear from the guest gateway, as today
  (ADR-0174). Deferred: IPv6 (#308), UDP above 59,000 bytes (#309), inbound
  UDP service to VMs (#310), ICMP (#311).
- **Guest-side mechanism accepted at its real size** (APPROVED): the
  mechanism listed in the Decision below.

ADR-0082 §D4's READY meaning is unchanged (§ Consequences).

## Context

Overdrive workloads are unmodified: they open ordinary `AF_INET` sockets and
hold no platform material. ADR-0145 removes the guest NIC, so a guest
`connect()`, `sendto()` or `accept()` has no route of its own.

The guest-capture spike (`spike/guest-vsock-capture-findings.md`, WORKS on
7.0.0-29 host and guest, native metal, the vendored Cloud Hypervisor fork with
`backend=vhost-kernel`) ran unmodified `curl`, `sshd`/`ssh`/`scp`, `getent`,
`dig` and busybox `nslookup` in a booted guest with only `lo` and a dummy
device. It established that D15 holds: the workload address is on the dummy
device and in `getsockname`; `getpeername` and `recvfrom` return the original
peer; UDP of sizes 0 to 59,000 bytes worked connected and unconnected; and
only fixed-size control I/O crossed either owner's syscalls.

The forwarder of ADR-0197 joins two connected sockets at any time and reads
its source from the start of its queue, so bytes that arrive before a flow is
paired wait in the source socket's own receive queue (ADR-0212; V-EV-PAIR,
P-71: 961,995 of 962,000 trials byte-exact, early data waiting only in the
source's queue).

Closing the owner's application-side sockets normally made an in-flight
application see a clean EOF with 0 bytes, not an error, when no
`SO_LINGER{1,0}` was set (guest-capture spike).

The original spike needed a separate SockOps active-established callback to
join the `connect4` destination, looked up by socket cookie, to the ephemeral
local port visible on the accepted intake child. Linux 7.0 calls `connect4`
before allocating that port (`net/ipv4/tcp_ipv4.c:206-218`, `:301-308 @
v7.0`). The spike's callback and owner lookup are
`spike-scratch/netns-density-295-guest-vsock-capture/bpf/src/main.rs:300-313`
and `vfwd/src/owner.rs:1001-1017`. The accepted five-program wording omitted
that correlation attachment. A later design review rejected retaining that
tuple row because an abandoned handshake can outlive application release and
create a below-quota refusal. The replacement uses the TCP handshake itself:
the bounded Aya probe in
`spike/findings-tcp-correlation-option.md` carried the exact cookie and
destination in an experimental option on the handshake-completing ACK. It
recovered them for ordinary, forced-syncookie and data-in-SYN Fast Open
connections after immediate client close; post-handshake peer input deleted
active socket storage and a refused socket carried its storage through
`TCP_CLOSE` for automatic destruction.

## Decision

The guest adaptation stays in the guest kernel, loaded and controlled by
`overdrive-init`, which never reads or writes application payload:

- **Workload address.** `workload_addr/32` is configured on a guest dummy
  device, so binding it, `getsockname`, and kernel source selection all yield
  it. The guest has no other non-loopback device.
- **Capture.** Root-cgroup `connect4` and `sendmsg4` programs rewrite every
  non-local IPv4 destination to a guest intake on the workload address, and
  keep the original destination per socket. `getpeername4` and `recvmsg4`
  restore the original destination to the application. A `sock_release` hook
  reports released application UDP sockets.
- **TCP capture correlation (`TCP-CORRELATION`, approved 2026-10-23).**
  A guest-only SockOps program copies the exact origin at `TCP_CONNECT_CB`,
  after ephemeral-port allocation, and writes the cookie and destination in
  the captured socket's 18-byte option on the handshake-completing ACK.
  Ordinary and syncookie passive establishment create `Ready` under that
  exact accepted child cookie. A data-in-SYN Fast Open child is `Pending`
  until its final-ACK parse callback creates `Ready`. `overdrive-init` takes
  `Ready` once at accept; a pending Fast Open child is held and counted within
  the same 4,096-place bound, woken privately and scanned before/after every
  event wait, with the existing audit deadline as the idle fallback.
  `KernelGuestOwnerIo` keeps that combined count across internally held
  Pending children and emitted `CapturedConn` handles until attach or close,
  so no owner-port or model change is needed. The active copy is
  `BPF_MAP_TYPE_SK_STORAGE`: the first post-handshake peer packet deletes it,
  or it dies automatically with that exact socket. A child row is consumed by
  classification or removed by exact `sock_release`. Application file release
  does not erase the copy from a still-live orphaned TCP socket, so ACK
  retransmission remains exact. The active storage has no `max_entries`
  capacity; the child map is an ordinary preallocated 8,194-entry Aya hash
  map, derived from the 4,096 backlog plus the 4,096 accepted pending bound.
  Aya's BTF-backed active map uses a private create/pin/reuse/unlink staging
  path that is removed before program load, attach and READY; service and
  teardown have no bpffs pin or adoption. It has no sockmap install, parking
  cell, payload path, LRU, eviction or tuple conflict. The accepted
  original-peer fact remains per application socket until `sock_release`,
  unchanged by owner flow end.
  Load and attach failures use the existing typed `ProgramLoad` /
  `ProgramAttach` setup errors with `GuestProgram::TcpCorrelation`.
  Active-storage BTF/create/staging/reuse/unlink and child-map/ELF load
  failures are part of that program load and preserve the typed Aya map cause as
  `ProgramLoad { program: GuestProgram::TcpCorrelation, source:
  aya::EbpfError::MapError(..) }`.
  `bind_tcp_intake` uses `listen(4096)` and enables `TCP_FASTOPEN` with the
  same backlog; failure stays its existing typed `IntakeBind { addr, source }`
  result. The exact option ABI, universe and transitions are normative in the
  feature delta's accepted § *[REF] TCP capture correlation*.
- **Forwarder.** `overdrive-init` checks and loads the guest kit's `ovd_fwd`
  module before READY (ADR-0205, ADR-0206), opens one forwarder context and
  probes it, and attaches each flow's pair at pairing (ADR-0212): a TCP
  intake child to its flow vsock on `Paired`; the flow vsock it accepted to
  its connection to the application before it sends `Paired`; a UDP slot
  socket to its association's flow vsock on `Paired`. Until `Paired` an
  intake child's bytes wait in its own receive queue. A refused guest
  datagram attach returns only the consumed flow vsock in
  `GuestSlotAttachRefused`; the slot socket was borrowed and remains resident.
  The owner closes the flow vsock and ends the association by discarding and
  disconnecting that resident socket, retaining the slot owner (ADR-0210).
  One source-local cell in the existing guest-owner suite proves the ended-V_g
  refusal, exact guest/host end paths and later slot reassociation; no lane,
  Quint state or public test support is added.
- **Pending TCP bound.** At most `GUEST_TCP_PENDING_MAX` guest TcpConnect
  flows are held before `Paired`; an intake child accepted beyond it is reset
  and counted (`guest_tcp_pending_exhausted`).
- **Reserved intake ports (D25).** The guest's intakes live on the workload
  address at a fixed reserved port range above the kernel's default ephemeral
  range: one TCP intake port and one UDP port per datagram slot (values in the
  feature delta). `overdrive-init` binds them before READY without
  `SO_REUSEADDR` / `SO_REUSEPORT`, each slot socket to its own explicit port,
  and marks the range reserved (`net.ipv4.ip_local_reserved_ports`), so an
  application can never share an intake and the kernel never hands out a
  reserved port as an ephemeral port. An application bind on the workload
  address or `0.0.0.0` fails with `EADDRINUSE` every time only on a port that
  holds an intake of the same protocol: TCP on the TCP intake port, UDP on a
  slot port. A TCP bind on a slot port, or a UDP bind on the TCP intake port,
  succeeds (the reservation covers ephemeral assignment only). A workload spec
  that declares a listener port inside the reserved range, of either
  protocol, is refused at deploy, with a typed spec-validation error naming
  the port and the range.
- **UDP slot identity (D25).** A slot belongs to one (application socket,
  original destination) pair; one application socket that sends to N
  destinations holds N slots. `overdrive-init` holds the authoritative
  slot-ownership table, read from the kernel slot map.
- **UDP slot release (D25).** A slot is released when its application socket
  is released — every slot of that socket, not only the last — or when the
  slot's association has been idle for the pinned idle period. If a
  `sock_release` report cannot be queued, the level-triggered audit in
  `overdrive-init` finds slots whose socket no longer exists and releases
  them. Releasing a slot ends its association in one order: abort the pair,
  discard the slot socket's receive queue without reading payload,
  disconnect the slot socket, then return the slot (ADR-0201).
- **An ended association does not re-associate by itself (U-2, user ruling
  2026-10-06).** When an association whose slot is still owned ends without
  pairing or is aborted — `Refused`, `Abort`, or the pairing deadline — the
  guest discards the slot socket's queued datagrams in the order above and
  keeps the slot for its (application socket, destination). Only a datagram
  the application sends after the end starts a new association. A
  destination that refuses every attempt is therefore tried at most once per
  pairing round trip, driven by the application's own sends, never by a
  retry loop. A late `Paired` for an ended association is answered with
  `Abort`.
- **Pool exhaustion (D25).** With no free UDP slot, the capture program
  refuses the application's `connect()` or `sendmsg()` to a new destination
  (`EPERM`). It is counted.
- **Inbound.** For a flow the host opens (ADR-0173), `overdrive-init` accepts
  the flow vsock on its inbound listener, reads the request, and connects to
  the application's declared port with a socket bound by `IP_TRANSPARENT` to
  the address and port the request names — the guest gateway with port 0
  (ADR-0174) — carrying a platform `SO_MARK`. A fwmark rule to the local
  table returns the application's replies to it, and
  `net.ipv4.fwmark_reflect=1` makes the guest kernel's own replies (a reset
  for a port with no listener) carry the mark, so a refusal is delivered at
  once. The connect is bounded by the guest's inbound connect timeout, which
  is below the host's pairing deadline for a dialled flow (values in the
  feature delta); `overdrive-init` attaches the pair and answers on the
  control session with `Paired`, or answers `Refused` carrying the errno, or a
  timeout carrying the errno.
- **Flow ends.** The guest forwarder classifies every end of a flow vsock as
  a clean half-close or an abort (ADR-0180) and propagates a clean end with
  `shutdown(SHUT_WR)` on its target after the last accepted byte (ADR-0183).
  `overdrive-init` learns each pair's end from its end record (ADR-0202).
- **Abort visibility.** Owner-held TCP sockets on the application side carry
  `SO_LINGER{1,0}` from creation, so an abort resets the application's
  connection rather than ending it cleanly; a clean close clears linger first
  (ADR-0160). When the host owner dies, every flow vsock receives `0x3` while
  open, the guest forwarder aborts the pair and resets the application's
  connection (ADR-0180).
- **Loss of the owner fails closed.** Every guest program attachment is a BPF
  link held by `overdrive-init`, and every pair belongs to its forwarder
  context (ADR-0199); no attachment or pair outlives it, and its death ends
  the VM (below).
- **Control.** `overdrive-init` opens the per-VM control session (ADR-0166)
  and writes 32-byte flow requests carrying its launch token on flow vsock
  connections (ADR-0177).
- **Programs.** The guest capture programs are Aya Rust in `overdrive-bpf`,
  built by the one eBPF pipeline (ADR-0038) and embedded in the guest image.

## Alternatives considered

- **Derive the capture identity at `accept()` from the five accepted hooks.**
  `connect4` has the cookie and original destination before the local port
  exists; the intake accept has the local port after the application socket
  may have reset and disappeared. Socket diagnostics are therefore a timing
  guess, and `sock_release` cannot read `src_port` at this attach type
  (`net/core/filter.c:8960-9002 @ v7.0`). Rejected.
- **Carry a private token in `SO_MARK`.** This changes the workload socket's
  mark and introduces token collision, capacity and cleanup semantics that no
  accepted contract or spike proves. Rejected.
- **Active-established full-tuple row, re-keyed at passive establishment.** A
  connection can release after the active callback without ever reaching the
  passive callback. Cleaning then can erase reset-before-accept metadata;
  retaining forever adds a below-quota capacity refusal. The user required
  admission below the quota to remain correct. Rejected after independent
  review.
- **Write the correlation in the SYN and load its saved copy into
  `BPF_MAP_TYPE_SK_STORAGE`.** Fast Open consumes the remaining SYN option
  space and syncookie children have no saved SYN. Aya 0.13.1 also requires a
  create/pin/reuse lifecycle, which that proposal left incompatible with D19.
  The replacement uses the final ACK, keeps the child handoff in a supported
  unpinned HashMap and confines the capacity-free active storage's staging pin
  to pre-attach load before unlink. Rejected after native falsification and
  independent review.
- **Hook each intake child at establishment and park its bytes in a kernel
  cell until `Paired`.** Needed only by a sockmap redirect, which loses bytes
  that arrive before its socket is installed; the forwarder needs neither
  (ADR-0212). Rejected.
- **A guest kernel module hooking socket operations for capture** (as #303's
  module hooks `tcp_prot`). The cgroup socket hooks capture every ordinary
  socket without one. Rejected.
- **A guest userspace shim.** Payload would cross guest userspace. Rejected by
  constraint.
- **vsock-aware workloads.** Breaks the unmodified-workload model. Rejected.
- **Present inbound peers as the guest's own address.** The application would
  see itself as its client. Rejected (ADR-0174).

## Consequences

- **`TCP-CORRELATION` validation is required.**
  Existing five-program setup assertions receive neutral six-program
  corrections, and DISTILL adds one parameterized Tier-3 scenario in the
  existing disposable guest-kit owner lane. It invokes ordinary outbound TCP
  capture through the production guest programs and `KernelGuestOwnerIo`, and
  covers exact cookie/destination delivery, immediate close before accept, an
  uncorrelated direct intake connection, forced syncookie, actual Fast Open
  `Pending -> Ready`, one-time consumption and transient-map cleanup. It adds
  no lane, seeded correlation state, raw-packet fixture, model, public
  test-support API or production test branch.

- **READY keeps its meaning.** READY means guest platform init, including
  guest network configuration, completed (ADR-0082 §D4). Guest network
  configuration is: the dummy device carries the workload address, the fwmark
  rule and `fwmark_reflect` are set, the guest capture programs are loaded and
  attached, the forwarder module is loaded and its context opened and probed,
  and the inbound vsock listener is bound. READY waits for exactly that and
  adds no dependency on the host. The per-VM control session is **not** a
  READY precondition; host activation requires it instead (feature delta,
  G-V3).
- **`overdrive-init` is the forwarding owner and is PID 1.** If it dies the
  guest kernel panics before its files close (`kernel/exit.c:929-930`,
  `:971 @ v7.0`), so no guest socket closes and no linger fires; the VM ends
  (ADR-0182) and the host's flows of that guest are reset by the
  device-release orphan sweep (ADR-0146).
- **Guest kernel requirements:** BPF with BTF, cgroup-v2 socket hooks and
  `sock_release`, the forwarder module of the guest kit with module
  unloading off on the appliance kernel (ADR-0198, ADR-0068 § 7),
  `CONFIG_VIRTIO_VSOCKETS`, `CONFIG_DUMMY`, policy routing, and
  `IP_TRANSPARENT`. Observed on stock 7.0.0-29.
- **Residual D15 differences:**
  - **D15-R2 (approved with D25, 2026-10-06):** a reserved port range on the
    guest is unavailable to applications; a VM can hold at most the pinned
    number of concurrent UDP (socket, destination) pairs and pending outbound
    TCP flows; an idle UDP association ends after the idle period.

  D15-R1 and D15-R3 are recorded in the feature delta's D15 re-derivation.
  Every datagram of an association, an empty one included, is delivered once
  and in order (ADR-0201).
- **Both owners' `connect()` calls are non-blocking** (ADR-0151).
- **Guest kernel command line** gains a transport token carrying the workload
  address (feature delta, § *[REF] Guest adaptation contract*).
- **Inbound UDP service** would need classification of unconnected UDP by
  sender (#310).
