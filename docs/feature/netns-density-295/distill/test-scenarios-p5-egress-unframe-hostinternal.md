# P5 — Egress unframe (hybrid), host-internal deny, mesh/non-mesh egress, VIP

Scope: the hybrid datagram-frame removal (host verdict strip + fragment-aware
TC egress unframe on `lo` and every root-namespace interface — D5a, D5a-IFACE,
FRAG-MAP-LIFECYCLE), the no-UDP-segmentation-offload source rule
(USO-NO-SEGMENT), the no-stacked-interface boot refusal (IFACE-NO-STACK), the
host-internal destination refusal enforced by a marked-socket output rule (D26,
D26-PINS), non-mesh kernel-forwarded egress (D14) and mesh egress via leg-F
(D9), and guest service-VIP access via the `connect4` rewrite + mesh resolution
(D24, D24a). No Quint module covers egress/host-internal (not modelled);
`shared_table` partial-convergence is P6. See
[shared-test-infrastructure.md](shared-test-infrastructure.md).

Owns: D5a, D5a-IFACE, D5a-IFACE-BOOT, D5a-LATE, IMG-IFACE-BOOT, USO-NO-SEGMENT,
IFACE-NO-STACK, FRAG-MAP-LIFECYCLE, D14, D9, D24, D24a, D26, D26-PINS. Gate
G-V1 (unframe-attach half) + G-V6 (egress legs). R5-6, R5-7, R5-8, R5-38, R5-46,
R5-47, R5-48. V-21 restated (done P-47). (D8-PREFIX-VIP / R5-40 is P4's — the
`StackedInterface`/`UnframeAttach` refusals in `probe_startup`/`converge_shared`
are P5's.)

## Load-bearing sequencing status (read first)

The P5 production surface **does not exist in the tree** (confirmed 2026-10-09):
`overdrive-dataplane::guest_flow::{GuestFlowForwarder, converge_unframe_interfaces,
UnframeConvergence, UnframeAttachError}`, `GUEST_UNFRAME_FRAGMENTS_MAX`,
`OWNER_EGRESS_MARK`, the reshaped `SharedGuestNetworkOwner` methods
(`probe_startup` VIP/stacked checks, the D26 per-flow policy), the D24a VIP
branch + `ServiceBackendsResolve::new(vip_ranges)`, and
`MtlsInterceptWorker::register_forwarded_outbound` are ABSENT. `MtlsResolve`,
`ServiceBackendsResolve`, `MtlsInterceptWorker`, `VipRange`,
`MTLS_LEG_S_DIAL_MARK` exist and are extended; `GuestNetworkError` exists and
gains variants. Per CLAUDE.md §"Implement to the design — never invent API
surface" and `.claude/rules/testing.md` §"RED acceptance scaffolds and
activation", the DELIVER roadmap reconciliation (the first DELIVER activity,
feature-delta L16554-16558) creates the production scaffolds; the compiling
Rust RED bodies in the five owned files are **co-sequenced** with those
scaffolds and stay **unregistered** in the crate test entrypoints until then
(shared-infra §6). The scenario specification below is the authorable-now
DISTILL product; the body files are authored RED input DELIVER wires. See
§"RED / activation / gaps".

## Files owned (disjoint)

| File | Lane | Entrypoint (NOT registered yet) | Contents |
|---|---|---|---|
| `crates/overdrive-dataplane/tests/integration/vsock_egress_unframe.rs` | Tier-3, `host-kernel-shared` | `tests/integration.rs` → `mod integration` | unframe byte-exactness, boot attach + errno map, stacked-iface refusal, USO |
| `crates/overdrive-dataplane/tests/integration/vsock_fragment_map.rs` | unit/proptest + Tier-3 | `tests/integration.rs` → `mod integration` | fragment-decision pure fn proptest + lifecycle S1/S2/S3 |
| `crates/overdrive-worker/tests/integration/vsock_mesh_egress_mtls.rs` | Tier-3, `host-kernel-shared` | `tests/integration.rs` → `mod integration` | non-mesh direct (D14), mesh leg-F mTLS wire (D9) |
| `crates/overdrive-control-plane/tests/acceptance/vsock_host_internal_deny.rs` | default lane (unit/proptest) + seeded sim | `tests/acceptance.rs` → `mod acceptance` | D26 policy order, D24a resolve branches, mark disjointness, seeded-sim owner check |
| `crates/overdrive-control-plane/tests/integration/vsock_host_internal_enforce.rs` | Tier-3, `host-kernel-shared` | `tests/integration.rs` → `mod integration` | D26-PINS rule construction F-1..F-6 |

`host-kernel-shared` nextest-group entries this piece adds (MODULE-matched,
`.config/nextest.toml`): `overdrive-dataplane::integration::vsock_egress_unframe`,
`overdrive-dataplane::integration::vsock_fragment_map` (Tier-3 cells only),
`overdrive-worker::integration::vsock_mesh_egress_mtls`,
`overdrive-control-plane::integration::vsock_host_internal_enforce`. **The author
does not edit `.config/nextest.toml` now** (shared file; reported in gaps — the
entry co-arrives with registration in DELIVER).

## Lanes & layered-PBT discipline (Mandate 9 / testing.md §"Integration vs unit gating")

- **Layer 1 (unit/proptest, default lane)** — pure decision functions: the
  unframe attach-errno classifier, the IFACE-NO-STACK stacked predicate, the
  FRAG-MAP-LIFECYCLE fragment-shift decision function, the D26 host-internal
  policy order, the D24a resolve classification, the `OWNER_EGRESS_MARK`
  pairwise-disjoint-bits check. PBT full (`proptest`) — unbounded/combinatorial
  errno and fragment-train input spaces. **Placement note:** the errno
  classifier and fragment-decision fn are pure fns in
  `overdrive-dataplane::guest_flow`; their proptests belong in that crate's
  DEFAULT unit lane. The owned `tests/integration/*.rs` files are
  `integration-tests`-gated, so the pure proptests authored there carry a
  `// LAYER-1` marker and DELIVER SHOULD relocate them into the crate unit lane
  when it extracts the pure fn (surfaced in gaps). The D26/D24a/mark-disjoint
  pure tests live in `vsock_host_internal_deny.rs` under `tests/acceptance/`,
  which is control-plane's **default** lane — correct placement.
- **Layer 2 (in-memory acceptance / seeded sim)** — the D26 owner-check flow
  (`SimMtlsResolve` + `SimSharedGuestNetworkOwner` through the owner's private
  seam). Example-pinned (sad paths enumerated), not PBT-generated (Mandate 11).
- **Layer 3+ (Tier-3, example-only, `host-kernel-shared`)** — every kernel
  effect: the real egress unframe attach + datagram byte-exactness, the
  fragment lifecycle with packet injection, the D26-PINS marked-socket output
  rule, mesh/non-mesh egress through `overdrive serve` + `overdrive deploy`.
  Example-based, sad paths enumerated (Mandate 11). State-delta/Universe is
  optional at layer 3+ (Mandate 8); the Tier-3 oracles are observable kernel
  state (`bpftool net/map`, `ss`, `ip`, `tcpdump`/`WireScan`, byte-exact
  round-trips, exit codes).

≥40% error/edge: S1 attach refusals (5 errno classes), S2 stacked refusal, S3
USO falsification guard, S4-S2 eviction/S4-S1 orphan, S5/S6/S8 refusals, S7
`MeshUnreachable`. Sad paths > happy paths in this piece.

## Scenarios (GWT, spec-only; crafter translates each to a Rust `#[test]`/`#[tokio::test]`)

Every scenario records: **decision/invariant · contract · production owner path
· fault stimulus (sad paths) · oracle · evidence lane · @contract-shape**. The
outcome elevator pitch (domain verb) propagates to the test name verbatim.

### `vsock_egress_unframe.rs` — Tier-3 + layer-1 (`host-kernel-shared`)

#### P5-S1 — every interface present at boot carries the egress unframe program `@contract-shape:unbounded-preservation`
- **Decision/invariant:** D5a, D5a-IFACE, D5a-IFACE-BOOT, D5a-SERVE-IFACE, R5-38,
  R5-39, G-V1. ADR-0165. Anchored on P-44 (V-17) and P-30/P-31 (V-11).
- **Contract:** at boot `GuestFlowForwarder::load` attaches the TCX egress
  unframe program to `lo`; `converge_unframe_interfaces(interfaces)` attaches it
  to every other root-namespace interface present, returning `UnframeConvergence
  { attached, failed }` with every requested interface in exactly one map. When
  boot opens, an empty guest datagram (8-byte frame carrier) leaves through any
  boot-present interface and reaches its peer as **0 bytes**; a frame-shaped
  application payload leaves and reaches its peer **byte-exact**; several
  independent interfaces under policy routing each hold the property. Nothing
  else on the system changes.
- **Production owner path:** `run_server_with_obs_and_drivers` → `probe_startup`
  → `converge_shared` (`GuestFlowForwarder::load` + `converge_unframe_interfaces`
  over the interface set from the host root namespace) → `open_after_boot`; then
  `overdrive deploy` a VM whose guest sends datagrams.
- **Fault stimulus (sad):** the kernel refuses an attach — one `UnframeAttachError`
  variant per errno class: `EEXIST` (same program twice), `ERANGE` (the 64th
  egress program on one interface), `ENODEV` (interface deleted between
  enumeration and attach), `EMFILE` (descriptor limit), `ENOMEM` (memory
  pressure, injected).
- **Oracle:** byte-exact round-trip at the peer (empty → 0 B; frame-shaped →
  unchanged); kernel link query (`bpftool net` / `GuestFlowInventory.unframe_interfaces`)
  shows the link on every interface; on an attach failure `converge_shared`
  returns `GuestNetworkError::UnframeAttach { source }` whose `source.ifindex()`
  names the interface and `run_server` emits `health.startup.refused` reason
  `guest_network.converge`, **before** `open_after_boot`.
- **Layer-1 property (errno classifier):** for every errno, `ENOMEM →
  UnframeAttachError::KernelMemory`; every other errno (`EEXIST`, `ENODEV`,
  `EMFILE`, `ERANGE`, any) → `UnframeAttachError::Attach`. Proptest over the
  errno domain. (The pure classifier is a crafter extraction — see gaps G-1.)
- **Evidence lane:** Tier-3 (Lima CI + metal for a real NIC) for attach +
  datagram property; seeded sim (scripted `failed` convergence) for the boot
  refusal; layer-1 proptest for the classifier.
- **Universe (sad, boot refusal, layer 2 sim):** `{ server.boot_reached_open,
  startup.refused_reason, error.variant, error.ifindex, side_effects.boot }` —
  `boot_reached_open set_to(false)`, `refused_reason set_to("guest_network.converge")`,
  `error.variant set_to(UnframeAttach)`, `side_effects.boot unchanged` (no route,
  no listener bound beyond the attach attempt).

#### P5-S2 — serve refuses to boot on a stacked root-namespace interface `@contract-shape:unbounded-preservation`
- **Decision/invariant:** IFACE-NO-STACK (ADR-0181), R5-38 (stacked half), G-V1.
  Anchored on P-44 (V-17 falsified stacked double-unframe).
- **Contract:** `probe_startup` refuses `GuestNetworkError::StackedInterface {
  interface, stacking }` **before any boot effect** (no attach, no route, no nft
  table) when any root-namespace interface has a master (`InterfaceStacking::Enslaved`)
  or a stacking link kind (`InterfaceStacking::Upper { kind, lower }`, kind ∈
  `{Vlan, Macvlan, Macvtap, Ipvlan, Ipvtap}`). A host whose interfaces are
  independent (veth pairs included — a veth peer is not stacked) boots.
- **Production owner path:** `SharedGuestNetworkOwner::probe_startup` (Composition
  step 1, before `compose_vm_driver` / the step-3 veth pair).
- **Fault stimulus:** boot on a host carrying a VLAN over an interface / a macvlan
  over one / a bridge with an enslaved port.
- **Oracle:** `probe_startup` returns `StackedInterface` naming the interface and
  its `InterfaceStacking`; `health.startup.refused` reason `guest_network.probe`;
  the system filesystem/kernel is unchanged (no attach, no route, no table).
- **Layer-1 property:** refusal **iff** some root-namespace interface has a
  master OR a stacking link kind; a veth peer, tunnel/encrypting kinds (`vxlan`,
  `geneve`, `gre`, `ipip`, `xfrm`, `macsec`, `wireguard`) are NOT stacked.
  Proptest over a generated interface inventory. (Pure predicate is a crafter
  extraction — gaps G-1.)
- **Evidence lane:** Tier-3 (build the stacked topology, assert refusal + no
  effect) + layer-1 proptest for the predicate.

#### P5-S3 — guest datagrams survive a UDP-segmentation-offload interface byte-exact `@contract-shape:unbounded-preservation`
- **Decision/invariant:** USO-NO-SEGMENT, R5-46, G-V6. ADR-0165. P-43 (V-16).
- **Contract:** guest datagrams of every size class (empty, 1 B, each MTU
  boundary, fragmented, frame-shaped, near-miss, 59,000 B) leaving through an
  interface with `tx-udp-segmentation: on` reach their peer byte-exact; no owner
  UDP socket has `UDP_SEGMENT` set (read back) and no send carries it; a source
  check over the owner + forwarder crates finds no `UDP_SEGMENT` reference.
- **Production owner path:** owner datagram socket H (post-`install_datagram`) →
  egress unframe program → USO-enabled interface → peer.
- **Fault stimulus (falsification guard):** toggling the interface feature
  changes nothing (no GSO skb reaches the program); a frame-kept datagram sent
  through a socket that *does* set `UDP_SEGMENT` is dropped (`bpf_skb_adjust_room`
  refuses a UDP GSO skb) — this guard confirms the prohibition is load-bearing.
- **Oracle:** byte-exact at peer; `getsockopt(UDP_SEGMENT)` read back == 0 on
  every owner UDP socket; the source-check assertion (grep-equivalent in-test
  over the crate source, or an architecture test).
- **Evidence lane:** Tier-3 (production `run_server`) + source check.

### `vsock_fragment_map.rs` — layer-1 proptest + Tier-3

#### P5-S4 — a fragmented guest datagram is unframed exactly once over its own train `@contract-shape:pure-function` (decision fn) / `@contract-shape:unbounded-preservation` (Tier-3)
- **Decision/invariant:** FRAG-MAP-LIFECYCLE, R5-47, G-V6. ADR-0165. P-45 (V-18).
- **Contract:** the egress program's `Fragments` map is a `BPF_MAP_TYPE_LRU_HASH`
  (`max_entries = GUEST_UNFRAME_FRAGMENTS_MAX = 4_096`) keyed by the IPv4
  reassembly key (source, destination, protocol, identification). A stripped
  first fragment inserts/overwrites; an unstripped first fragment removes; the
  last fragment (MF clear) removes. **Invariant:** a later fragment is shifted
  back by one 8-byte unit iff its own train's first fragment was stripped and
  its record was not evicted; nothing about another datagram changes it. An
  eviction loses at most the one datagram whose record it took (counted
  `unframe_fragment_orphan`), never corrupts another.
- **Production owner path:** `GuestFlowForwarder` egress program on a converged
  interface; records driven by `install_datagram`-registered tuples.
- **Layer-1 property (decision fn):** for any sequence of interleaved trains,
  `shifted(frag) == (own_train.first_fragment_stripped && !record_evicted)`.
  Proptest on the pure fragment-decision function modelling the map lifecycle.
  (Pure fn is a crafter extraction — gaps G-1; the sim forwarder model hosts it.)
- **Fault stimulus / Tier-3 cells (sad):**
  - **S1:** a stripped first fragment whose train is cut (no later fragments),
    then an unrelated bare fragmented datagram with the same reassembly key —
    arrives byte-exact and **unshifted** (`unframe_fragment_orphan`-free for the
    bare one).
  - **S2:** the map filled beyond capacity with cut trains from several CPUs —
    every later datagram arrives byte-exact or, where a live record was evicted,
    is lost whole (counted), never delivered corrupted; inserts never fail.
  - **S3:** a congested qdisc (`netem delay 20ms limit 1`) dropping fragments
    after the program for three runs — afterwards every datagram byte-exact.
- **Oracle:** byte-exact / lost-whole at peer; `GuestFlowCounters` entry counts
  (`unframe_fragment_shifted`, `unframe_fragment_orphan`) as the map-state oracle;
  wire capture reassembles.
- **Evidence lane:** layer-1 proptest (decision fn) + Tier-3 with packet
  injection.

### `vsock_host_internal_deny.rs` — default lane (unit/proptest) + seeded sim

#### P5-S5 — a guest flow to a host-internal destination is refused HostInternal `@contract-shape:pure-function`
- **Decision/invariant:** D26, R5-6, G-V6. ADR-0167. Anchored on the per-flow
  policy order (owner).
- **Contract:** the owner's per-flow policy evaluates in a fixed order, first
  match decides: (1) static `Policy` (`0.0.0.0/8`, `224.0.0.0/4`,
  `255.255.255.255`, `240.0.0.0/4`; datagram guest-prefix ≠ `gateway:53`); (2)
  static `HostInternal` (`127.0.0.0/8`, `169.254.0.0/16`); (3) mesh resolution
  (TCP, D24a); (4) local-delivery `HostInternal` (`dst` routed `local` /
  `broadcast` / `multicast` / `anycast`, except `gateway:53`); (5) datagram
  kernel-peer check. Guest TCP/UDP to `127.0.0.1:p`, `169.254.169.254:80`, the
  host's own address, `gateway:22` → `FlowRefusal::HostInternal`; `gateway:53`
  resolves; a mesh-resolved TCP destination is not host-internal.
- **Production owner path:** `SharedGuestNetworkOwner` per-flow policy classifier
  (pure decision over request, dst, route type, resolve result, VIP ranges).
- **Layer-1 property:** for any destination, the first-match order holds; a
  `HostInternal` classification is flow-local and counted, never advancing/revoking
  allocation state. Proptest over generated destinations × route types ×
  request kinds, pinned `@example` for each R5-6 literal.
- **Oracle:** the classifier returns `HostInternal` for the host-internal set and
  the resolve/connect branch for `gateway:53`; no allocation-state mutation.
- **Evidence lane:** layer-1 proptest (pure policy order) + seeded sim (the owner
  refuses the flow and counts it). Tier-3 enforcement is P5-S8/R5-6-Tier-3.
- **Universe (seeded sim):** `{ flow.refusal, flow.state, alloc.running,
  counters.host_internal }` — `flow.refusal set_to(HostInternal)`, `alloc.running
  unchanged`, `counters.host_internal appended_with(1)`.

#### P5-S6 — a late-added host address is still rejected by the kernel rule (TOCTOU) `@contract-shape:bounded-change`
- **Decision/invariant:** D26-PINS F-4, R5-7, G-V6. ADR-0167. P-47 (V-21).
- **Contract:** a host address added **between** the owner's check and the
  connect is still rejected by the constant marked-socket output rule, closing
  the check-then-connect window. A TCP flow → `Refused(DestinationUnreachable)`.
  A datagram association whose first forwarded datagram the rule rejects at
  transmit delivers **nothing** to the host service; the owner observes an error
  on its UDP socket H (`EPOLLERR`, non-zero `SO_ERROR`; errno recorded, not
  asserted — errno-agnostic per F-4) and ends the association
  `FlowCloseCause::DestinationUnreachable`; the guest's next datagram starts a
  new association, which step 4 refuses `HostInternal`.
- **Production owner path:** owner connects marked socket H → kernel output rule
  rejects; the verdict's redirect onto H is the transmit for a forwarded
  datagram (reported on H via `sk_psock_report_error`).
- **Fault stimulus:** add a loopback/local address to the host after the owner's
  policy check passed it (the race window).
- **Oracle:** host service receives 0 bytes; TCP flow close cause
  `DestinationUnreachable`; datagram association ends `DestinationUnreachable`;
  next guest datagram refused `HostInternal`.
- **Evidence lane:** Tier-3 on the **production forwarder redirect path** (P-47
  used userspace sends; R5-7 discharges the redirect transmit path). Seeded sim
  for the owner's association-end classification.

#### P5-S7 — guest access to a service VIP is mesh-resolved, unknown-VIP refused, datagram-VIP rewritten `@contract-shape:pure-function`
- **Decision/invariant:** D24, D24a, R5-8, G-V6. ADR-0164, ADR-0153. P-32 (VIP).
- **Contract:** `ServiceBackendsResolve` (the VIP branch, D24a) classifies a TCP
  `(vip, port, Tcp)` matching a service exactly as a frontend hit (`Mesh(first
  healthy backend)` or `MeshUnreachable`); a TCP destination in the configured
  VIP ranges matching no service → `MeshUnreachable` (fail closed, never
  `NonMesh`); a guest UDP datagram to a VIP is rewritten by the ADR-0053
  `connect4` program on the owner's connected host socket (the guest keeps
  reporting the VIP). `ServiceBackendsResolve::new` takes the configured
  `vip_ranges` as a mandatory parameter (one SSOT with the VIP allocator).
- **Production owner path:** owner policy step 3 → `MtlsResolve::resolve(dst)` →
  leg-F registration (`register_forwarded_outbound`) for `Mesh`; `connect4` on
  the owner cgroup for datagram VIP.
- **Layer-1 property:** the `mtls_resolve_rekey` equivalence test extends to the
  VIP branch with **per-arm mutation coverage** (frontend hit, unknown VIP in
  range → `MeshUnreachable`, non-VIP → unchanged classification). Each arm
  killed by a distinct mutant.
- **Oracle:** `MtlsResolution` per arm; `MeshUnreachable` for unknown-in-range;
  the datagram VIP socket's kernel peer (`getpeername`) is the backend after
  `connect4`, the guest's reported peer is the VIP.
- **Evidence lane:** unit (resolve branches, mutation-killing) + Tier-3 (V-6,
  inside the walking skeleton / P6) for the enforced mesh path.

### `vsock_host_internal_enforce.rs` — Tier-3 (`host-kernel-shared`)

#### P5-S8 — the host-internal output rule rejects marked owner sockets, structurally `@contract-shape:unbounded-preservation`
- **Decision/invariant:** D26-PINS (F-1, F-2, F-3, F-5, F-6), R5-48, G-V6.
  ADR-0167. P-47 (V-21).
- **Contract:** with the production shared firewall table —
  - **F-1 (position/reuse):** a marked UDP socket reusing the source port of an
    earlier **unmarked** flow to a local destination is still rejected (the D26
    rules precede every `accept`, PREFIX-REPLY's `ct established,related accept`
    included).
  - **F-2 (replacement):** a marked-connect storm during repeated table
    replacement reaches no host service (the table is replaced in **one** nft
    transaction — delete+add batched — never separate delete/add).
  - **F-3 (verdict):** TCP `ECONNREFUSED`, UDP `EPERM`, **never a hang** (plain
    `reject`, never `reject with tcp reset` or `drop`).
  - **F-5 (non-unicast):** marked UDP to a subnet-directed broadcast, the
    prefix's broadcast address and `224.0.0.1` reaches no host service — with and
    without `SO_BROADCAST` — while `gateway:53`, an off-host address and a
    neighbour stay reachable (`fib daddr type != unicast` plus loopback /
    link-local).
  - **F-6 (mark):** a packet carrying the owner bit with other bits set is
    rejected; a packet with only the leg-S or TPROXY mark is unaffected.
- **Production owner path:** `run_server` composes the shared table; the owner
  marks H with `OWNER_EGRESS_MARK` for every direct-connect flow (not leg-F).
- **Layer-1 property:** `OWNER_EGRESS_MARK`, `MTLS_LEG_S_DIAL_MARK` and the
  TPROXY mark are **pairwise-disjoint single bits** (unit, default lane — but
  this cell lives here with its Tier-3 siblings per file ownership; marked
  `// LAYER-1`, DELIVER may relocate).
- **Fault stimulus:** each F-case enumerated as a named Tier-3 cell (`Sad_*`).
- **Oracle:** host service byte count == 0 for rejected dials; `ECONNREFUSED` /
  `EPERM` observed; latency bounded (no hang); reachable destinations still
  reachable; the mark-disjoint unit assertion.
- **Evidence lane:** Tier-3 (production table) + unit (mark constants).

### `vsock_mesh_egress_mtls.rs` — Tier-3 (`host-kernel-shared`)

#### P5-S9 — non-mesh guest egress is kernel-forwarded directly to its destination `@contract-shape:unbounded-preservation`
- **Decision/invariant:** D14, G-V6. ADR-0162. Anchored on D14 (leg-F cleartext
  relay removed).
- **Contract:** a guest TCP flow to a non-mesh unicast destination is forwarded
  to that destination with **no leg-F cleartext relay** (the owner's host socket
  connects `dst` directly, marked, `SO_LINGER{1,0}`); a guest UDP datagram to a
  non-mesh destination likewise. `spawn_cleartext_passthrough` /
  `OutboundAction::PassThrough` no longer exist (deleted by D14).
- **Production owner path:** owner policy `NonMesh` → direct host socket →
  destination; no `MtlsInterceptWorker::register_forwarded_outbound`.
- **Oracle:** the destination receives the guest's bytes byte-exact; a wire scan
  of the host→destination leg shows **no leg-F loopback hop** and no cleartext
  relay socket; `MtlsInterceptWorker` records no forwarded-outbound registration
  for the flow.
- **Evidence lane:** Tier-3 (hand-built route-disjoint off-host destination via
  `overdrive_testing::cidr_lease::TestCidrLease::acquire` — NOT a CIDR literal).

#### P5-S10 — mesh guest egress is resolved once at pairing and enforced with mTLS on the inter-agent wire `@contract-shape:unbounded-preservation`
- **Decision/invariant:** D9, G-V6. ADR-0153. Inherits CLAUDE.md §"East-west
  mTLS tests — the egress DIALER speaks PLAINTEXT".
- **Contract:** a guest TCP flow to a mesh peer reaches leg-F only through a
  registered pair (`register_forwarded_outbound`); leg-F does **not** re-resolve
  (the owner's single resolution, ADR-0123); the inter-agent wire (leg-F →
  remote peer) carries mTLS (TLS 1.3 application_data records, `0x17`, zero
  cleartext). A duplicate registration for the same `forwarder_local` →
  `DuplicateForwardedRegistration`; a leg-F accept with no registration is closed.
- **Production owner path:** owner policy `Mesh(b)` → `register_forwarded_outbound(alloc,
  forwarder_local, original_destination, backend)` → the forwarder's socket to
  leg-F installed at `ACTIVE_ESTABLISHED` → leg-F claims by exact 4-tuple at
  accept.
- **Egress-dialer discipline (load-bearing):** the guest application is the
  dialer and speaks **plaintext** over an ordinary socket (identity-unaware); the
  test driving the guest MUST NOT present client TLS (it would open a peerless
  session leg-F never terminates → stall → RST). The mTLS proof is on the
  **leg-F ↔ remote** inter-agent segment (`WireScan { plaintext: 0 }` / `ss -K`
  kTLS / `tcpdump` of `0x17` records), NOT the client handshake. Keep a
  byte-distinct REQUEST/RESPONSE litmus so the assertion proves the real
  server→client reply pipe. The server side still holds SVIDs.
- **Fault stimulus (sad):** a second accept from the same peer finds nothing and
  is closed; a leg-F accept with no registration is closed; a mesh resolution
  miss → `MeshUnreachable` (shared with P5-S7).
- **Oracle:** byte-exact plaintext round-trip at the guest app; leg-F wire scan
  shows `0x17` TLS records with zero cleartext on the inter-agent leg;
  `register_forwarded_outbound` claimed exactly once.
- **Evidence lane:** Tier-3 through `overdrive serve` + `overdrive deploy`
  (mesh peer another node / HeldServerIdentity PKI).

## Required boundary scenarios — G-V1 (unframe-attach half) and G-V6 (egress legs)

Per feature-delta §"Required boundary scenarios" (rows 1-6). P5 owns the
unframe-attach half of G-V1 and the egress-legs half of G-V6.

| Row | G-V1 (unframe attach) | G-V6 (egress legs) | Covered by |
|---|---|---|---|
| 1 available | boot attaches every present interface; datagrams byte-exact | `Paired` mesh/non-mesh flow carries bytes | P5-S1, P5-S9, P5-S10 |
| 2 typed failure in owner domain | `UnframeAttach` / `StackedInterface` → `health.startup.refused` | `Refused(HostInternal)` / `Refused(MeshUnreachable)` / `Refused(DestinationUnreachable)` | P5-S1(sad), P5-S2, P5-S5, P5-S6, P5-S7 |
| 3 unrelated unchanged | boot refusal reclaims/adopts nothing; late-registered interfaces never gate | per-flow refusal never advances/revokes allocation `Running`/READY/Service `Stable` | P5-S2 (no effect), P5-S5 (alloc unchanged) |
| 4 late success can't resurrect | an interface registered after boot is never attached and carries no guest datagram (D5a-LATE) | — (flow-scoped; no terminal-state resurrection in P5 egress) | P5-S1 note, P5 doc §D5a-LATE |
| 5 disconnect/reconnect | — (boot-only) | association ended `DestinationUnreachable` does not re-associate by itself (U-2); next datagram opens a new one refused `HostInternal` | P5-S6 |
| 6 feature disabled | single-cut; substitute sim fixture `transport: None` behaves as `network: None` | ← | shared-infra; P5 asserts no owner effect under `transport: None` |

**D5a-LATE (row 4) note:** there is no late-interface attach operation, no
link-notification watch, relist or retry for unframe attachments. The appliance
image configures root-namespace interfaces only at boot (IMG-IFACE-BOOT,
ADR-0068 §5); an interface the kernel registers after boot is never configured
→ carries no guest datagram → needs no attach. The dev/metal hosts run a
catch-all systemd-networkd config and are **not** the appliance image (not
evidence for or against D5a-LATE). This is an image requirement under A-31, not
a runtime check — **no runtime test** asserts a late-interface attach; P5
asserts only that boot attaches every present interface and nothing after.

## State-delta / Universe (Mandate 8, layers 1-3) — Rust realization

This Rust codebase has **no `nwave_ai.state_delta` port** (that is the Python
pilot; no equivalent crate exists here, and inventing one is out of P5's
test-support scope). Mandate 8's universe-bound assertion is realized the
project way, matching the design's "Contract shapes, bounded-change universes"
tables (feature-delta §"Owner and provisioner"): a layer-2 seeded-sim mutating
step captures the **port-exposed observable surface** before and after, asserts
the declared delta on the named entries, and asserts **everything else in the
universe unchanged** (fail-closed) — explicit `before`/`after` comparisons, no
private owner fields. Universe entries are owner observables only (flow
refusals, flow close causes, counters, allocation state flags, boot outcome).
Layer-1 pure proptests assert their return directly (pure-function shape — no
mutation universe). Layer-3 Tier-3 uses observable-kernel-state oracles
(Mandate 8 permits traditional assertions at layers 4+). The per-scenario
universes (P5-S1 sad, P5-S5) name the entries the body compares.

## Outcome → OUT-N registry (DISTILL §"Register Outcomes")

P5 introduces these typed contract surfaces — register in
`docs/product/outcomes/registry.yaml` at reconciliation (deferred to the
first-needer / DELIVER step 1, since the production surface is gated):
`invariant` guest-egress-unframe-is-idempotent-per-train (FRAG-MAP-LIFECYCLE);
`specification` host-internal-destinations-refused (D26); `specification`
no-udp-segmentation-on-owner-sockets (USO-NO-SEGMENT);
`specification` serve-refuses-stacked-interface (IFACE-NO-STACK). (Not
registered now: the surface does not exist; registration co-arrives with the
scaffold per shared-infra §0.)

## Completeness audit (nw-at-completeness-check, P5 slice)

| Item | Verdict | Note |
|---|---|---|
| C1a happy path per gate | PASS | P5-S1/S9/S10 (available rows) |
| C1b observable outcome at port | PASS | byte-exact/round-trip/`FlowRefusal`/counters |
| C2a state machine (flow end) | PASS | association-end `DestinationUnreachable`, U-2 (P5-S6) |
| C2b transitions | PASS | boundary rows 2/4/5 |
| C3 error contract | PASS | `UnframeAttachError`, `GuestNetworkError::{StackedInterface,UnframeAttach,...}`, `FlowRefusal::HostInternal`, `FlowCloseCause::DestinationUnreachable` enumerated |
| C4a boundary/edge | PASS | errno classes, MTU boundaries, fragment trains, mark bits |
| C4b null/empty | PASS | empty datagram (8-B carrier → 0 B) |
| C5a mode flags | PASS | USO on/off; `SO_BROADCAST` on/off; marked/unmarked |
| C5b config | PASS | feature-disabled substitute (`transport: None`) |
| C6a failure injection | PASS | attach refusal, qdisc congestion, late host address, table replacement storm |
| C6b recovery | PASS | association re-association refusal (U-2) |
| C6c concurrency | PASS | several-CPU fragment inserts (S2), marked-connect storm (F-2) |
| C7a env matrix | PASS | Lima CI + metal; `host-kernel-shared` serialization |
| C7b resource | PARTIAL | `Fragments` map capacity (4,096) eviction behaviour covered; fd-budget is G-V1 owned by boot/P6 |
| C7c multi-actor | PASS | mesh (two agents) + non-mesh (off-host dest) |
**Verdict: COMPLETE (≥13/15).** No `SPECIFICATION_AMBIGUITY` in P5 scope — every
oracle traces to a design-pinned observable or a crafter-extractable pure fn.

## RED / activation / gaps

- **Activation markers.** Every body carries `#[ignore = "pending DELIVER …"]`
  naming the activating DELIVER work (roadmap reconciliation is the first
  DELIVER activity; exact step numbers are produced by it — the markers name the
  decision + activity):
  - P5-S1/S2/S3/S4 → the egress-unframe forwarder + `probe_startup`
    stacked/USO checks (forwarder-load step, ≈ step 4; D5a-IFACE / IFACE-NO-STACK
    / USO-NO-SEGMENT / FRAG-MAP-LIFECYCLE).
  - P5-S5/S6/S7/S8 → the owner D26 policy + `OWNER_EGRESS_MARK` rule +
    `connect4`/VIP resolve (owner + rule step, ≈ steps 9/11; D26 / D26-PINS /
    D24 / D24a).
  - P5-S9/S10 → non-mesh direct + mesh leg-F registration (egress step, ≈ step
    ≥ 9; D14 / D9).
- **Blocked on nothing** — no contract-shaping validation item is open (V-16/17/18
  = P-43/44/45; V-21 = P-47; all DONE). These are normal RED scaffolds, not
  blocked.
- **Gaps / shared-fixture dependencies (reported, NOT edited by this piece):**
  - **G-1 (pure-fn extraction, internal structure — CLAUDE.md §"interface
    contracts, not internal structure").** The attach-errno classifier, the
    IFACE-NO-STACK stacked predicate, the FRAG-MAP-LIFECYCLE fragment-shift
    decision fn, the D26 per-flow policy order, and the D24a resolve
    classification are PURE fns the design pins by *behaviour* (enum + map +
    table) but not by a public pure-fn signature. DELIVER extracts them (Mandate
    4 / CM-D) and SHOULD place the layer-1 proptest in the owning crate's
    **default** unit lane. The bodies reference recommended signatures marked
    `// CRAFTER-EXTRACTION`; the exact name/location is the crafter's HOW, not a
    design gap routed to DESIGN.
  - **G-2 (shared sim + fixtures; layer placement).** The seeded-sim
    P5-S1(sad)/S5 bodies need `SimSharedGuestNetworkOwner` reshaped to the new
    traits + `SimMtlsResolve` VIP-branch scripting + the owner's private
    forwarder seam (`SimGuestFlowForwarder` scripting
    `converge_unframe_interfaces` `failed` entries) — shared-layer, first-needer
    creates (shared-infra §3). **The crate-local Tier-3 bodies drive their own
    crate's real objects, NOT `boot_vsock_node`** (an `overdrive-dataplane` /
    `overdrive-worker` test depending on `tests/conformance` is a dependency
    cycle): `vsock_egress_unframe.rs` / `vsock_fragment_map.rs` drive
    `GuestFlowForwarder` directly over a RAII veth fixture (the existing
    `tests/integration/helpers` veth pair); `vsock_mesh_egress_mtls.rs` drives
    `MtlsInterceptWorker` leg-F directly + a route-disjoint netns peer
    (`TestCidrLease`); `vsock_host_internal_enforce.rs` composes the production
    shared table (control-plane + worker) + a marked dialer. The genuinely
    end-to-end cells that require full `overdrive serve` + `overdrive deploy`
    (R5-7's production redirect reporting on owner socket H; R5-46's full
    `run_server` USO path; P5-S9/S10 through `deploy`) are discharged inside
    **P6's walking skeleton / `tests/conformance`** (V-6); the crate-local tests
    cover the component-level kernel behaviour and cross-reference P6.
  - **G-3 (`.config/nextest.toml` host-kernel-shared entries).** Shared file; the
    four module→group entries co-arrive with registration in DELIVER.
  - **G-4 (placement).** The layer-1 pure proptests in the two
    `tests/integration/*.rs` files are `integration-tests`-gated by their
    entrypoint; DELIVER SHOULD relocate the pure-fn proptests (errno classifier,
    fragment-decision fn, mark-disjointness) into the crate default unit lane so
    they run fast and unserialized. Marked `// LAYER-1` in the bodies.
  - **G-5 (compile verification).** Because the production surface is gated and
    the files stay unregistered (shared-infra §6), `cargo check --all-targets`
    does NOT compile these bodies; they are authored RED input, compile-verified
    when DELIVER registers them against the step-1 scaffold. The exact
    design-pinned symbols each file depends on are listed in its file header.

## Retired (by DELIVER roadmap reconciliation, NOT by this piece)

P5 production code replaces the old egress path. The old-model GREEN tests
defending it — `crates/overdrive-dataplane/tests/integration/{guest_tcx_egress_lifecycle,
guest_tcx_inventory}`, `crates/overdrive-worker/tests/.../outbound_enforce_substrate_splice`
(revalidate-vs-replace per the Contract-impact matrix) — are retired in DELIVER
coupled with removal of the old production code (shared-infra §7). This piece
does not delete them (deleting now leaves an uncommittable tree).
