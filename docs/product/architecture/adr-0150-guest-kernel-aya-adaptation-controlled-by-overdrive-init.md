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

**GUEST-TRANSPARENT-ROUTE accepted (2026-10-24).** Native STEP 11-06 evidence
invalidated only this ADR's `fwmark -> local table` premise. The production
owner issued its transparent application connect and never completed; enabling
accepted-child mark inheritance alone still failed. The exact dedicated guest
mark/table/local-default configuration made the same native cell pass.
Independent DESIGN review iteration 2 approved the bounded correction with no
open finding and found that no further user decision is required because it
restores already-approved D15, INB-1 and D8-PEER outcomes without changing
them. The exact configuration, typed failures, ownership and cleanup contract
live in the #295 feature delta, the
[native record](../../feature/netns-density-295/deliver/native-11-06-guest-transparent-route-falsification.md),
and its [independent review](../../feature/netns-density-295/deliver/design-review-11-06-guest-transparent-route.md).

**TCP-CORRELATION amendment accepted 2026-10-23.** Native STEP 11-06
evidence falsified the Fast Open SockOps final-ACK consumer. Independent scope
validation and DESIGN review selected one handshake-only
`fentry/tcp_check_req` candidate plus the existing SockOps state callback. The
user approved the exact seventh program, C/CO-RE build/loader setup and the
bounded ACTIVE/CHILD pre-program-load staging-pin extension by answering
**“Approve and continue”** to the reviewed decision. Aya remains `0.13.1`;
there is no steady established-data hook, runtime pin, new lane/model or
outcome weakening. The exact interface and evidence are normative in the #295
feature delta, § *[REF] TCP capture correlation*.

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

The accepted prose collapsed the guest return-routing configuration to a rule
that looked up the kernel's reserved local table. STEP 11-06 reproduced the
real consequence on Linux `7.0.0-38-generic`: the transparent socket's connect
remained in progress and the owner observed no completion. The isolated
contrast retained that rule and added `tcp_fwmark_accept=1`; it still failed.
The same production cell passed with the guest-capture lineage's dedicated
`0x295` mark, table 295 local-default route and both mark sysctls. Linux carries
a socket's mark into IPv4 route lookup (`include/net/route.h:151-156 @ v7.0`),
matches it in FIB rules (`net/core/fib_rules.c:278-310 @ v7.0`), and copies an
incoming mark to the accepted child when `tcp_fwmark_accept` is enabled
(`include/net/inet_sock.h:105-112`, `net/ipv4/tcp_input.c:7515-7536`,
`net/ipv4/inet_connection_sock.c:1211-1233 @ v7.0`). The native contrast, not
those source facts alone, establishes that the dedicated route is required.

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
and `vfwd/src/owner.rs:1001-1017`. A later review rejected a global tuple row:
an abandoned handshake can outlive application release and either erase
reset-before-accept metadata or retain capacity below the accepted quota.

The first option probe proved the active producer and ordinary/syncookie
consumers. Its Fast Open result was false because it closed the client before
waiting for `Ready`; Linux skips SockOps header parsing while the child is
`TCP_SYN_RECV`. The production guest-kit path reproduced an open established
child stuck `Pending`.

The user-authorized scope validation found the smaller exact boundary. Linux
enters `tcp_check_req(..., fastopen=true, ...)` with the full child socket lock
held and before validating the ACK; only the accepted path reaches
`tcp_set_state(TCP_ESTABLISHED)`, whose existing SockOps state callback runs
under the same lock. One C/CO-RE fentry program records a whole-row candidate
with `BPF_EXIST`; the existing callback promotes it after validation. A fresh
disposable x86_64 Cloud Hypervisor guest on Metal, Linux
`7.0.0-38-generic`, Aya `0.13.1`, proved ordinary, syncookie, live-open Fast
Open exact identity, direct rejection, terminal/delete monotonicity and
preterminal release. Established data invoked the candidate zero times
(`spike/findings-tcp-correlation-candidate.md`).


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
- **TCP capture correlation (`TCP-CORRELATION`).** A guest-only SockOps program
  copies the exact application cookie/original destination at
  `TCP_CONNECT_CB`, after ephemeral-port allocation, into capacity-free
  `ACTIVE_CORRELATION` socket storage and writes the exact 18-byte option on
  eligible non-SYN ACKs. Ordinary and syncookie passive establishment create
  `Ready` under the exact full child cookie. Passive rows are gated to the
  configured IPv4 workload-address/TCP-intake child; a direct ordinary child
  becomes `Rejected`. Fast Open passive establishment creates `Pending` on the
  SYN and arms only the existing SockOps state callback.

  The exact seventh program is crate-private
  `GuestProgram::TcpCorrelationCandidate`, label and ELF symbol
  `tcp_correlation_candidate`, section `fentry/tcp_check_req`. The READY order
  is `TcpCorrelation`, `TcpCorrelationCandidate`, `Connect4`, `Sendmsg4`,
  `Getpeername4`, `Recvmsg4`, `SockRelease`. Aya loads it as `FEntry` against
  the running guest BTF and holds one global process-owned link. Ordinary
  `tcp_check_req` calls return at `fastopen == false`; unrelated Fast Open
  calls return after cookie/map lookup; only an exact nonterminal child row
  causes bounded TCP-header reads. Established data never invokes the hook.

  Linux calls the candidate with the exact `struct sock *` and
  `struct sk_buff *` under the child socket lock and before Fast Open ACK
  validation. `BPF_CORE_READ(skb, data)` plus `bpf_probe_read_kernel` reads
  only the already-linear TCP header/options, never application payload. The
  candidate whole-row replaces the original lock-free 24-byte value with
  `CandidateReady` or `CandidateRejected` using `BPF_EXIST`. Only a validated
  ACK reaches the existing SockOps `SYN_RECV -> ESTABLISHED` state callback;
  under the same child lock it whole-row replaces the candidate with terminal
  `Ready` or `Rejected` and sends the private wake. Linux's hash bucket lock
  publishes each complete replacement; deletion first makes `BPF_EXIST`
  return `ENOENT`, and replacement first is removed by the later deletion.
  No post-passive program inserts, so terminal/delete is final without an
  in-value lock.

  `KernelGuestOwnerIo` holds Pending/candidates and emits `Ready` only after
  the fd reports `TCP_ESTABLISHED`; it consumes `Rejected` once by linger-reset
  close with no event/flow/control. Its combined 4,096 count covers held
  children and emitted `CapturedConn` handles. `CHILD_CORRELATION` remains one
  preallocated 8,194-entry hash (4,097 accept-queued + 4,096 held + one
  classification boundary), with exact one-time classification or child
  `sock_release` cleanup. The active copy is deleted by the first later
  non-SYN peer packet or exact socket destruction; application file release
  cannot erase a still-live orphan's retransmission metadata. There is no
  additional tuple table, private cgroup, timer, LRU, eviction, retry,
  sockmap, parking or payload path.

  Production adds exactly
  `crates/overdrive-bpf/src/tcp_correlation_candidate.bpf.c`. `cargo xtask
  bpf-build` generates `vmlinux.h` from `OVERDRIVE_BPF_TARGET_BTF` (Linux
  default `/sys/kernel/btf/vmlinux`) with `bpftool`, compiles the one C/CO-RE
  program with Clang/LLVM major 21, `-g -O2 -target bpf` and the x86/arm64
  target macro, and writes
  `target/bpf/overdrive_guest_tcp_correlation_candidate_bpf.o`. Clang 21,
  `bpftool`, libbpf headers and target-kernel BTF are exact build-time
  requirements in dev setup, Lima and CI. `overdrive-init/build.rs` requires
  and embeds that object beside the existing Rust object; no guest runtime
  compiler/object lookup or Aya upgrade is added.

  Before any program load/attach, crate-private
  `prepare_guest_programs() -> Result<(), GuestTransportSetupError>` creates
  and temporarily pins `ACTIVE_CORRELATION`, loads the Rust object so it
  creates/pins ordinary lock-free `CHILD_CORRELATION`, loads the C object
  through the same Aya pin path so it reuses that exact child map, then
  unlinks both pins and the directory. Only then are the seven programs
  loaded/attached. Both objects and links stay process-owned; runtime and
  teardown have no pin/adoption. READY waits for pin absence, successful
  running-BTF load/attach and all existing setup.

  Active/staging/Rust-object failures map to
  `ProgramLoad { program: TcpCorrelation, source: EbpfError }`. Candidate
  object/child reuse/CO-RE/running-BTF/symbol/prototype/FEntry-load failures
  map to `ProgramLoad { program: TcpCorrelationCandidate, source: EbpfError }`;
  candidate attach maps to
  `ProgramAttach { program: TcpCorrelationCandidate, source: ProgramError }`.
  Missing/malformed validated metadata becomes private `Rejected`, reset-close
  and no event/flow/control. The exact option, state, capacity, build, staging
  and failure contracts are normative in the feature delta's accepted
  § *[REF] TCP capture correlation*.

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
- **Inbound (GUEST-TRANSPARENT-ROUTE).** For a flow the host opens (ADR-0173), `overdrive-init` accepts
  the flow vsock on its inbound listener, reads the request, and connects to
  the application's declared port with a socket bound by `IP_TRANSPARENT` to
  the address and port the request names — the guest gateway with port 0
  (ADR-0174) — carrying the dedicated guest `SO_MARK=0x295`. A full-mask rule
  sends that mark to dedicated table 295, whose host-scope local-default route
  on `lo` returns the application's replies to it;
  `net.ipv4.tcp_fwmark_accept=1` gives the accepted application child that
  mark, and `net.ipv4.fwmark_reflect=1` gives the guest kernel's own reply (a
  reset for a port with no listener) the mark, so a refusal is delivered at
  once. Exact order, typed failures and lifecycle/cleanup remain normative in
  the feature delta. The connect is bounded by the guest's inbound connect timeout, which
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
- **Programs.** Six guest capture programs remain Aya Rust in `overdrive-bpf`;
  the handshake-only candidate is the one approved C/CO-RE source. The single
  `cargo xtask bpf-build` pipeline produces both embedded guest objects under
  the exact compiler/BTF contract above (bounded amendment to ADR-0038).

## Alternatives considered

- **Derive the capture identity at `accept()` from the five original hooks.**
  `connect4` has the cookie and destination before the local port exists; the
  intake child has the port after the application socket may disappear.
  Diagnostics are a timing guess and `sock_release` cannot read `src_port` at
  this attach type. Rejected.
- **Carry a private token in `SO_MARK`.** This changes the workload socket's
  mark and adds token collision/capacity/cleanup semantics. Rejected.
- **Active-established full-tuple row re-keyed at passive establishment.** A
  released connection may never reach passive establishment. Cleanup can erase
  required reset metadata; retention creates a below-quota refusal. Rejected.
- **Write the correlation in the SYN.** Fast Open can consume the remaining
  option space and syncookie children have no saved SYN. Rejected.
- **Use the Fast Open child's SockOps parser for the final ACK.** Linux returns
  before parsing in `TCP_SYN_RECV`; production remained Pending. Later
  payload/FIN/close is not a valid trigger. Rejected by native falsification.
- **Root/private-cgroup `CgroupSkb` ingress.** Both forms passed native
  correctness, but both dispatch on established packets; the private form also
  adds a cgroup lifecycle and ingress-only in-value lock. The handshake-only
  fentry proved the same invariants without either cost. Superseded.
- **Listener-inherited socket filter.** It runs before the child lock and needs
  a separate synchronization/detachment contract. The locked ingress value is
  also verifier-incompatible with socket filters. Rejected.
- **Two tracing programs.** A separate `tcp_set_state` fentry is unnecessary:
  the existing SockOps state callback is already the post-validation boundary.
  Rejected.
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
- **Keep the rule pointed at the reserved local table and add only
  `tcp_fwmark_accept`.** The bounded production-owner native contrast still
  timed out. Rejected by reproduced evidence.
- **Put the catch-all local route in table 255 or `main`.** This would affect
  general local or unmarked guest routing instead of only the owner's marked
  sockets. Rejected.
- **Reuse the host leg-C/leg-F `0x1`/table-100 pair.** A v28 probe showed it is
  feasible, but it couples guest-internal return routing to the host
  transparent-mTLS configuration. The guest-capture lineage's dedicated
  `0x295`/295 pair has the exact successful native production-owner contrast.
  Not selected.

## Consequences

- **`TCP-CORRELATION` validation is required.** Existing six-program and build
  assertions receive neutral seven-program/candidate-object corrections. The
  already-authorized single parameterized Tier-3 cell remains in the existing
  disposable guest-kit lane: ordinary exact capture, immediate application
  close, direct uncorrelated intake, forced syncookie, live-open Fast Open
  `Pending -> CandidateReady -> Ready`, direct
  `CandidateRejected -> Rejected`, one-time consume, combined quota and both
  staging pins absent. Its real Fast Open producer remains open and sends no
  post-handshake payload before capture. PRIO/NETEM delayed final ACK,
  established sends and preterminal reset-close prove terminal/delete
  finality. Counts remain 8/12; no new cell, lane, seeded state, raw packet,
  model, public test API or production test branch is added.

- **The seventh-program practical limit is explicit and accepted.** The global
  fentry dispatches for each system `tcp_check_req`; ordinary calls return at
  the Fast Open gate, unrelated Fast Open calls return after cookie/map lookup,
  and established data never invokes it. Correctness evidence measures no CPU,
  throughput or tail latency. Native proof is x86_64 Linux
  `7.0.0-38-generic`; the build supports aarch64 structurally without a native
  aarch64 receipt.

- **READY keeps its meaning.** READY means guest platform init, including
  guest network configuration, completed (ADR-0082 §D4). Guest network
  configuration is: the dummy device carries the workload address; the
  GUEST-TRANSPARENT-ROUTE mark rule, local-default route,
  `tcp_fwmark_accept` and `fwmark_reflect` are set; the guest capture programs are loaded and
  attached, the forwarder module is loaded and its context opened and probed,
  and the inbound vsock listener is bound. READY waits for exactly that and
  adds no dependency on the host. The per-VM control session is **not** a
  READY precondition; host activation requires it instead (feature delta,
  G-V3).
- **The route adds no cleanup subsystem or lifecycle gate.** `overdrive-init`
  owns it before READY in the VM's network namespace. Any setup refusal exits
  before READY; later PID-1 loss ends the VM. Namespace destruction removes
  the rule and route, so no live teardown, repair, adoption, persistence or
  cross-boot cleanup is introduced.
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
