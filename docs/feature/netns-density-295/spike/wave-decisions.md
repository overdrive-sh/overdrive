# SPIKE Decisions — netns-density-295

## Assumption Tested

- After Exec removal, two real Cloud Hypervisor microVMs on a node-local
  shared bridge can communicate through the actual production zero-copy mTLS
  enforcement core without depending on a per-workload network namespace,
  veth pair, `/30` carve, `NetSlot`, or `host_veth`.

## Probe Verdict

- **WORKS.** Part A proved the shared-bridge/TAP kernel mechanism. Part B
  reused the production `HostMtlsEnforcement`, workload-identity,
  mesh-resolution, and per-VM cgroup components on native metal and proved
  TLS 1.3 kTLS TX/RX plus zero-copy `splice(2)` with no direct shared-L2
  bypass. See `findings.md`.

## Promotion Decision

- **DISCARD from promotion; hand off to DESIGN.** Approved by the user on
  2026-09-16. The scratch implementation is not promoted directly into
  production. By explicit user instruction, the throwaway sources and raw
  evidence are retained and committed instead of deleted so DESIGN can audit
  the executed proof.

## Design Implications

- The microVM dataplane does not require the existing per-workload
  netns/veth/`/30`/`NetSlot`/`host_veth` mechanism.
- Preserve the topology-neutral production core: platform-held workload
  identities, mesh resolution, TLS 1.3, kTLS TX/RX, zero-copy splice, and the
  per-VM cgroup resource/lifecycle boundary.
- Replace the current netns-shaped VMM network attachment and
  `NetSlotAllocator`-backed DNS construction with a shared bridge/TAP design.
- Do not infer that per-allocation leg-F/leg-C listeners can be consolidated;
  the spike did not test that architectural choice.

## Constraints Discovered

- The proof is same-node. Cross-host routing, target density, throughput, and
  final production API shape remain DESIGN questions.
- The working interception path uses per-TAP bridge classification and local
  delivery before IP TPROXY; bare bridge `broute` was insufficient in the
  observed substrate.
- Per-VM cgroups remain necessary for host CPU weighting, reserve-padded
  memory limits, VMM ownership, OOM attribution, and total cleanup.

## Increment-y — persistent-TAP fd handoff (2026-09-23)

### Assumption Tested

- The Cloud Hypervisor `--net fd=[...]` handoff proven for a non-persistent,
  fd-owned TAP also holds for the production-shaped persistent named TAP:
  network owner creates it persistent and down, a separate VMM launcher
  attaches and hands its queue fd to CH, CH reaches READY while down, the
  launcher's copy closes, the owner activates after interception is live, CH
  exit leaves the TAP persistent and unheld, and explicit teardown removes it
  with no residue.

### Probe Verdict

- **WORKS**, with two conditions DESIGN must pin: the launcher's `TUNSETIFF`
  must request `IFF_VNET_HDR`, and the network owner must set the TAP down
  after VMM exit and before any relaunch attach. See
  `findings-persistent-fd-tap.md`.

### Promotion Decision

- **DISCARD from promotion; hand off to the replacement DESIGN.** Approved by
  the user on 2026-09-23. The recovery plan forbids production API before
  DESIGN pins the fd-ownership contract. By explicit user instruction the probe
  and its raw evidence in
  `spike-scratch/increment-y-netns-density-295-persistent-fd-tap/` are
  retained, not deleted.

## Increment-z — host-side TAP MAC change and bridge FDB delivery (2026-09-24)

### Assumption Tested

- An unprivileged VMM process (uid 4200, no `CAP_NET_ADMIN`) holding only its
  own guest TAP's queue fd can redirect another guest's host-to-guest traffic
  to itself by setting its TAP's host-side MAC via `SIOCSIFHWADDR` on the
  shared bridge (final design review finding R5-H1), and a destination-MAC
  egress control on each TAP (ADR-0142) stops it.

### Probe Verdict

- **Steal reproduced: yes. Control blocks: yes.** On native metal the
  unprivileged fd holder moved the victim's bridge FDB entry to its own port
  and read the victim's frames; the unknown-unicast flood variant also leaked.
  A destination-MAC egress gate blocked both, and `flood off` closed the flood
  variant on its own. See `findings-mac-fdb-isolation.md`.

### Promotion Decision

- **DISCARD from promotion.** Approved by the user on 2026-09-24. The control
  is specified by ADR-0142 and lands through DISTILL/DELIVER. The probe and its
  evidence in
  `spike-scratch/increment-z-netns-density-295-mac-fdb-20260924T021346Z/` are
  retained, not deleted.

## Increment-aa — launcher seccomp filter for TAP-mutating ioctls (2026-09-24)

### Assumption Tested

- A narrow seccomp deny-list, installed by the launcher before `exec` and
  inherited by Cloud Hypervisor, is compatible with CH v53 on the `--net fd=`
  path and binds every CH thread.

### Probe Verdict

- **WORKS.** From a CH v53.0 source audit, the `fd=` path's ioctl set is
  disjoint from the 13-request deny-list. Under the filter, CH booted to READY
  and passed bidirectional traffic. All 11 CH threads, including the main
  thread CH's own filters leave unfiltered, carried the filter. Every denied
  request returned `EPERM`, and the no-filter control returned none. See
  `findings-tap-ioctl-seccomp.md`.

### Promotion Decision

- **DISCARD from promotion; hand off to DESIGN.** Approved by the user on
  2026-09-24. The filter is adopted into the accepted design and lands through
  DISTILL/DELIVER. The probe and its evidence in
  `spike-scratch/increment-aa-netns-density-295-tap-ioctl-seccomp-20260924T120003Z/`
  are retained, not deleted.

## Increment-ch-vhost-vsock — kernel vhost-vsock backend in the Cloud Hypervisor fork (2026-10-05)

### Assumption Tested

- Cloud Hypervisor v53.0 can gain a kernel vhost-vsock device mode (host
  `/dev/vhost-vsock`, guest CID assigned through the kernel, data virtqueues
  owned by the kernel vhost worker) so that guest vsock traffic terminates in
  the host kernel's AF_VSOCK stack instead of CH's userspace Unix-socket
  muxer, and a real booted Linux guest works over it. Stock v53.0 ships only
  the userspace muxer, so the selected vsock + Aya kernel-forwarding transport
  had no production VMM backend.

### Probe Verdict

- **WORKS (bounded).** On the native metal host (AMD EPYC 8024P, host and
  guest kernel 7.0.0-29-generic, no nesting, seccomp on), an unmodified guest
  booted with `--vsock cid=N,backend=vhost-kernel`. STREAM and SEQPACKET
  carried byte-exact traffic in both directions with zero payload bytes
  through CH syscalls, against 8,413,773 bytes through CH for the same
  transfer on the unix backend. CIDs stayed isolated across two VMs, a
  duplicate CID was refused with `EADDRINUSE`, forged guest binds were
  rejected, and CIDs were released and reusable after shutdown, `kill -9` and
  reboot. Not proven: Aya forwarding on this backend, composition through
  `overdrive serve`, the pinned 6.18 kernel, density. See
  `ch-vhost-vsock-findings.md`.

### Promotion Decision

- **PROMOTE.** Approved by the user on 2026-10-05. The fork
  `overdrive-sh/cloud-hypervisor` branch `overdrive/vhost-kernel-vsock`
  (`9b68dbb57`, `41a619d19` on v53.0), vendored as the submodule
  `vendors/cloud-hypervisor`, becomes the production VMM path, which also
  rules D13 as option (a). How the fork is built, pinned and provisioned
  replaces the current Cloud Hypervisor download, and lands through DESIGN
  (ADR-0146) and DISTILL/DELIVER. The probe and its evidence in
  `spike-scratch/netns-density-295-ch-vhost-vsock/` are retained.

## V-PREFIX-ROUTE — non-local route on lo covering the guest prefix (2026-10-13)

### Assumption Tested

- A main-table route `<guest prefix> dev lo scope link src 127.0.0.1`,
  tagged with Overdrive's route protocol, gives an unmarked host `connect()`
  to a guest-prefix address a route and the fixed source `127.0.0.1`, so the
  intercept's output divert can take the connection to leg-C on a node with
  no default route; and every other host-originated packet to the prefix is
  refused by ADR-0172's constant rules.

### Probe Verdict

- **WORKS.** Linux `7.0.0-38-generic` (Lima, run as root, network namespace
  `ovd-prt-probe2`). Without the route `connect()` fails `ENETUNREACH`; with
  it the diverted tuple reaches the leg-C stand-in, which sees peer
  `127.0.0.1`, and the reject counters stay 0 (cases 1–2). Every other prefix
  address takes a local-delivery route: with the constant rules a TCP connect
  fails `ECONNREFUSED` and a UDP send fails `EPERM` (case 3); without them a
  UDP datagram and TCP SYNs reach host sockets (case 5). The gateway's DNS
  path needs GW-LOCAL (cases 4a, 4b). The leg-C listener is a Python
  `IP_TRANSPARENT` stand-in; the production leg-C end to end is a Tier-3
  scenario of the feature delta. See `findings-prefix-route-lo.md` (the run
  transcript with the probe script inline).

### Promotion Decision

- **DISCARD — superseded by V-LEGB-MARK.** The user's ruling of 2026-10-13
  ("Leg-B dial contract replaces PREFIX-ROUTE-LO / INB-1-READ") withdrew
  PREFIX-ROUTE-LO and INB-1-READ before either was implemented; their draft
  ADRs were deleted and the numbers 0188 and 0189 stay unused. No route
  covers the guest prefix except the gateway `/32` (GW-LOCAL, ADR-0152).
  `findings-prefix-route-lo.md` is retained as the record of this probe; it is
  not a design dependency.

## V-LEGB-MARK — leg-B marks and source-binds same-node VM dials (2026-10-13)

### Assumption Tested

- For a dial whose destination is inside `GUEST_NODE_PREFIX`, leg-B setting
  `SO_MARK = MTLS_TPROXY_MARK` and `bind(127.0.0.1:0)` before `connect()`
  gives the connect a route through the existing `fwmark 0x1 -> table 100`
  rule, so the existing output divert and TPROXY reach leg-C on a node with
  no default route and no route covering the guest prefix; unmarked and
  half-set dials fail closed. This replaces PREFIX-ROUTE-LO.

### Probe Verdict

- **WORKS.** Probe `spike-scratch/netns-density-295-v33-legb-mark/`, Linux
  `7.0.0-38-generic` aarch64 (Lima) and `7.0.0-29-generic` x86_64 (native
  metal), both with `route_localnet=0` pinned. S1 (no default route) and S2
  (default route present, zero IPv4 on the uplink) reach leg-C with
  `peer=127.0.0.1`, `local=100.95.0.7:8080`, reject counters 0. S3: marked
  dials to unregistered prefix tuples are refused (TCP `ECONNREFUSED`, UDP
  `EPERM`). S4: both halves are required; mark-only fails `ENETUNREACH`
  (source = destination is not a local address), bind-only fails
  `ENETUNREACH`/`EINVAL`. S5: non-prefix dials stay unmarked. S6: bind to
  `lo` works only together with `bind(127.0.0.1)`. S7: the production
  netlink client installs the rule at 32765, ahead of main. S8 as specified
  (real `serve` + `deploy`) could not run: the tree has no production leg-B
  mark and the vsock owner and guest transport are DESIGN scaffolds. The
  closest equivalent (marked+bound dial -> TPROXY leg-C stand-in -> vsock ->
  real Cloud Hypervisor microVM, no default route) answered from inside the
  guest. Open questions: `GUEST_NODE_PREFIX` is one constant identical on
  every node; the table-100 route accepts `src 127.0.0.1` (and then the bind
  is redundant); the pre-marked connect and the diverted packet take the
  same table-100 route to `lo`. See `findings-v33-legb-mark.md`.

### Promotion Decision

- **PROMOTE — approved by the user on 2026-10-13** as LEGB-DIAL
  ([ADR-0190](../../../product/architecture/adr-0190-leg-b-marks-and-loopback-binds-same-node-vm-dials.md)):
  leg-B sets `SO_MARK = MTLS_TPROXY_MARK` and binds `127.0.0.1:0` for a peer
  inside the guest prefix and neither otherwise; it replaces PREFIX-ROUTE-LO
  and INB-1-READ. Open question 1 (the prefix is identical on every node) is
  resolved as a single-node limit: "a peer inside `GUEST_NODE_PREFIX` is a
  same-node VM" holds while Overdrive is single-node, and cross-node VM
  addressing is [#317](https://github.com/overdrive-sh/overdrive/issues/317).
  The bind is chosen over `src 127.0.0.1` on the table-100 route (open
  question 2). The production-composition run (V-LEGB-SERVE) remains a
  DELIVER validation item.
