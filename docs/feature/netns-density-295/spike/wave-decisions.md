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
