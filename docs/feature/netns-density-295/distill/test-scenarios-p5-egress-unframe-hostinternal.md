# P5 — Egress unframe (hybrid), host-internal deny, mesh/non-mesh egress, VIP

> **DISTILL status (2026-10-10, second increment after DG-1..DG-16).** Every
> round-1 `UI-*` item is closed by a DG decision. One round-2 item bears on
> P5: the authoring time of P5-S7's resolve-adapter bodies (DG-2 row 14,
> new-step 11) is not pinned (UI-R2-16). Source of truth: feature-delta § *vsock Reconciliation and Status
> — 2026-10-10* (scenario status table, *Attachment-cut DISTILL increment*).
>
> - **Authored now (compiled, registered, ignored):**
>   `overdrive-dataplane` `integration::vsock_egress_unframe` (P5-S1, P5-S3),
>   `integration::vsock_fragment_map` (P5-S4) — `pending DELIVER vsock
>   new-step 4 (…)`; `overdrive-control-plane`
>   `guest_network::prefix_boot_acceptance` (P5-S1 boot `UnframeAttach`
>   refusal, P5-S2) — `pending DELIVER vsock attachment cut (new-step 4, …)`;
>   `overdrive-worker` `integration::vsock_host_internal_enforce` (P5-S8) —
>   `pending DELIVER vsock attachment cut (new-step 14, host-internal deny set,
>   D26 output rules)`; `overdrive-worker` `integration::vsock_mesh_egress_mtls`
>   and the `mtls_intercept_worker` unit test (P5-S10, DG-10) — `pending
>   DELIVER vsock attachment cut (new-step 8, mesh registration, D9)`; `overdrive-core`
>   `dataplane::mtls_mark` mark-disjointness tests (P5-S8 layer-1, DG-9) —
>   default lane, not ignored.
> - **Attachment cut (DG-2 rows 2, 3, 7, 9, 14):** P5-S5..S7 and the P5-S9
>   owner `NonMesh` decision — source-local in `overdrive-control-plane`
>   beside the owner, over `SimGuestFlowIo`. The 91f9436ff draft
>   `overdrive-control-plane/tests/acceptance/vsock_host_internal_deny.rs` was
>   removed and is the starting draft. The P5-S9 end-to-end cell and the P5-S8
>   `gateway:53` accept / guest-broadcast reject cells are P6-S13's (DG-16,
>   UI-R2-14).
>
> Classes: `red-classification.md`.

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

DG-2 splits P5 into two authoring times (feature-delta § *vsock
Reconciliation and Status — 2026-10-10*):

- **Authored now.** Bodies that name only additive surface —
  `overdrive-dataplane::guest_flow::{GuestFlowForwarder, GuestFlowLinks,
  UnframeAttachError, GuestFlowCounters}`, `GUEST_UNFRAME_FRAGMENTS_MAX`,
  `OWNER_EGRESS_MARK` / `MTLS_TPROXY_MARK` / `MTLS_SHARED_NFT_TABLE` (DG-9),
  `SharedGuestNetworkOwner::{probe_startup, converge_shared}` over
  `with_flow_io`, `MtlsInterceptWorker::register_forwarded_outbound` — are
  complete, compiled bodies registered in their crate's test entrypoint and
  marked `#[ignore = "pending DELIVER vsock …"]` with the activating step.
  The crafter removes only the marker (testing.md § "RED acceptance
  scaffolds and activation").
- **Attachment cut.** Bodies that must call a DG-2 row-1..18 replacement
  (here rows 2, 3, 7, 9 and 14 — `GuestAddressPool::assign`, the
  `GuestNetworkPlan` reshape, `ForwardingActivation`,
  `HostSharedGuestNetworkOwner::new(ports)`,
  `ServiceBackendsResolve::new(store, frontend, vip_ranges)`) cannot compile
  before the cut, so they are not in the tree; their plan is the
  *Attachment-cut DISTILL increment* row "Host policy P5-S5..S7, S9 owner
  `NonMesh` decision", written in the cut's working tree before the crafter
  lands it.

## Files owned (disjoint)

| File | Lane | Entrypoint | Contents | Status |
|---|---|---|---|---|
| `crates/overdrive-dataplane/tests/integration/vsock_egress_unframe.rs` | Tier-3, `host-kernel-shared` | registered in `tests/integration.rs` → `mod integration` | `load` / attach, boot-interface unframe byte-exactness, `EEXIST` / `ENODEV` / `EMFILE` attach classes, USO, forwarder-crate `UDP_SEGMENT` source check (P5-S1, P5-S3) | authored, `pending DELIVER vsock new-step 4 (…)` |
| `crates/overdrive-dataplane/tests/integration/vsock_fragment_map.rs` | Tier-3, `host-kernel-shared` | registered in `tests/integration.rs` → `mod integration` | fragment lifecycle cells S1/S2/S3 (P5-S4) | authored, `pending DELIVER vsock new-step 4 (host forwarder fragment map)` |
| `crates/overdrive-control-plane/src/guest_network/prefix_boot_acceptance.rs` (source-local `#[cfg(test)]`, shared with P4) | default lane over `SimGuestFlowIo` | in-crate module | boot `UnframeAttach` refusal (P5-S1), stacked-interface refusal and its property (P5-S2) | authored, `pending DELIVER vsock attachment cut (new-step 4, …)` |
| `crates/overdrive-worker/tests/integration/vsock_host_internal_enforce.rs` | Tier-3, root, `host-kernel-shared` | registered in `tests/integration.rs` → `mod integration` | D26-PINS F-1..F-6 against the production shared table written by `MtlsInterceptWorker::start_shared_owner` (P5-S8) | authored, `pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)` |
| `crates/overdrive-worker/tests/integration/vsock_mesh_egress_mtls.rs` | Tier-3, root, `host-kernel-shared` | registered in `tests/integration.rs` → `mod integration` | forwarded-outbound port on the real `MtlsInterceptWorker`, inter-agent mTLS wire, DG-10 drop readings and refusal precedence (P5-S10) | authored, `pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)` |
| `crates/overdrive-worker/src/mtls_intercept_worker.rs` (in-crate unit test) | default lane | in-crate | DG-10 reading 1 stage labels (P5-S10) | authored, `pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)` |
| `crates/overdrive-core/src/dataplane/mtls_mark.rs` (in-crate unit tests) | default lane | in-crate | DG-9 mark values and pairwise-disjoint single bits (P5-S8 layer-1) | authored, not ignored (GUARD) |
| owner host-policy tests, source-local in `overdrive-control-plane` beside `HostSharedGuestNetworkOwner` | default lane over `SimGuestFlowIo` | in-crate | P5-S5..S7, P5-S9 owner `NonMesh` decision | attachment cut (DG-2 rows 2, 3, 7, 9, 14); starting draft `overdrive-control-plane/tests/acceptance/vsock_host_internal_deny.rs` @91f9436ff (removed) |

The 2026-10-09 placement `overdrive-control-plane/tests/integration/vsock_host_internal_enforce.rs`
moved to `overdrive-worker`: the worker owns the shared nft table, and its
integration binary is already in `host-kernel-shared` (feature-delta
§ *vsock Test Placement*). Both `overdrive-dataplane` and `overdrive-worker`
`integration` binaries are assigned whole to `host-kernel-shared` in
`.config/nextest.toml`, so no per-module entry is needed.

## Lanes & layered-PBT discipline (Mandate 9 / testing.md §"Integration vs unit gating")

- **Layer 1 (unit/proptest, default lane)** — pure decision functions: the
  unframe attach-errno classifier, the IFACE-NO-STACK stacked predicate, the
  FRAG-MAP-LIFECYCLE fragment-shift decision function, the D26 host-internal
  policy order, the D24a resolve classification, the `OWNER_EGRESS_MARK`
  pairwise-disjoint-bits check. PBT full (`proptest`) — unbounded/combinatorial
  errno and fragment-train input spaces. **Placement (2026-10-10):** the
  IFACE-NO-STACK property is authored as
  `probe_startup_refuses_iff_some_root_namespace_interface_is_stacked` in
  `overdrive-control-plane` `guest_network::prefix_boot_acceptance` (default
  lane). The mark-disjointness check is the DG-9 default-lane unit test in
  `overdrive-core` `dataplane::mtls_mark`. No layer-1 proptest is authored for
  the attach-errno classifier or the fragment-decision function: the design
  pins those by behaviour, not by a public pure-function signature, so the
  authored P5-S1 / P5-S4 bodies are Tier-3 example cells only; a crafter
  extraction may add a default-lane proptest in `overdrive-dataplane`. The
  D26 policy-order and D24a resolve tests are authored at the attachment cut,
  source-local in `overdrive-control-plane` (default lane).
- **Layer 2 (in-memory acceptance / seeded sim)** — the D26 owner-check flow
  over the owner seam sim `SimGuestFlowIo` (`overdrive-control-plane`
  `#[cfg(test)] guest_network::flow_io::sim`, scripted `route_type`; read
  surfaces `socket_observation`, `steering_observation`, `steering_effects`)
  plus the boot refusals over `SimGuestFlowIo` in `prefix_boot_acceptance`.
  Example-pinned (sad paths enumerated), not PBT-generated (Mandate 11).
- **Layer 3+ (Tier-3, example-only, `host-kernel-shared`)** — every kernel
  effect: the real egress unframe attach + datagram byte-exactness, the
  fragment lifecycle with packet injection, the D26-PINS marked-socket output
  rule, the forwarded-outbound mesh port on the real `MtlsInterceptWorker`;
  mesh/non-mesh egress through `overdrive serve` + `overdrive deploy` is
  P6-S13's walking skeleton (attachment cut).
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
- **Status (2026-10-10):** authored: `overdrive-dataplane`
  `integration::vsock_egress_unframe`::`load_attaches_every_link_and_the_unframe_program_to_lo_only`,
  `load_refuses_the_root_cgroup`,
  `every_boot_interface_carries_unframe_and_datagrams_arrive_byte_exact`,
  `on_a_registered_tuple_only_a_valid_frame_is_removed`,
  `an_unregistered_datagram_passes_a_converged_interface_unchanged`,
  `oversize_and_malformed_guest_datagrams_are_dropped_and_counted`,
  `a_second_attach_to_an_attached_interface_fails_naming_it` (`EEXIST`),
  `an_interface_gone_before_its_attach_fails_naming_it_and_the_others_attach`
  (`ENODEV`), `a_descriptor_limit_at_attach_fails_as_attach_naming_the_interface`
  (`EMFILE`) — ignored `pending DELIVER vsock new-step 4 (host forwarder)`;
  boot refusal: `overdrive-control-plane`
  `guest_network::prefix_boot_acceptance`::`a_failed_boot_unframe_attach_refuses_startup_and_writes_nothing_else`
  (scripted `KernelMemory` and `Attach` failures over `SimGuestFlowIo`) —
  ignored `pending DELIVER vsock attachment cut (new-step 4, host forwarder: egress unframe attach at boot, D5a-IFACE-BOOT)`. Not authored at Tier-3: the `ENOMEM` and
  `ERANGE` classes (they need a fault seam or 63 foreign TCX programs; the
  boot refusal covers `KernelMemory` through the sim). The layer-1 errno
  classifier proptest below is not authored (no pinned pure-function
  signature).
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
  names the interface, and the seam records no route write and no later boot
  effect (`steering_effects()`). The authored boot-refusal body observes only
  the owner's `GuestNetworkError` and the sim seam's effects; it does not boot
  `run_server`. The `run_server` projection — `health.startup.refused` reason
  `guest_network.converge`, `open_after_boot` never reached, no veth pair or
  nft table created — is the attachment-cut cell **"`run_server` boot
  refusal"** (DG-2 row 9) planned under P4-S13.
- **Layer-1 property (errno classifier):** for every errno, `ENOMEM →
  UnframeAttachError::KernelMemory`; every other errno (`EEXIST`, `ENODEV`,
  `EMFILE`, `ERANGE`, any) → `UnframeAttachError::Attach`. Proptest over the
  errno domain. (The pure classifier is a crafter extraction — see gaps G-1.)
- **Evidence lane:** Tier-3 (Lima CI + metal for a real NIC) for attach +
  datagram property; seeded sim (scripted `failed` convergence) for the boot
  refusal; layer-1 proptest for the classifier.
- **Universe (sad, boot refusal, layer 2 sim, as authored):** `{ error.variant,
  error.ifindex, seam boot effects }` — `error.variant set_to(UnframeAttach)`,
  the failing interface named, the seam's effects unchanged after the attach
  attempt (no route write, no steering effect). `boot_reached_open` and
  `startup.refused_reason` belong to the "`run_server` boot refusal" cell at
  the cut (P4-S13).

#### P5-S2 — serve refuses to boot on a stacked root-namespace interface `@contract-shape:unbounded-preservation`
- **Status (2026-10-10):** authored: `overdrive-control-plane`
  `guest_network::prefix_boot_acceptance`::`serve_refuses_to_boot_on_a_stacked_root_namespace_interface`,
  `probe_startup_refuses_iff_some_root_namespace_interface_is_stacked`
  (the layer-1 property) — ignored `pending DELIVER vsock attachment cut (new-step 4, probe_startup stacked-interface refusal, IFACE-NO-STACK)`. Both drive
  `HostSharedGuestNetworkOwner::with_flow_io(ports, SimGuestFlowIo)` with a
  scripted interface inventory; the "no effect" oracle reads the sim's
  `steering_effects` / `socket_observation`. No Tier-3 stacked-topology body
  is authored; the refusal is decided in `probe_startup` from interface
  facts the seam supplies.
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
- **Oracle (as authored):** `probe_startup` returns `StackedInterface` naming
  the interface and its `InterfaceStacking`; the sim seam records no boot
  effect (`steering_effects()` empty, `socket_observation()` unchanged: no
  attach, no route, no steering). The `run_server` projection —
  `health.startup.refused` reason `guest_network.probe`, `open_after_boot`
  never reached, no veth pair or nft table created — is the attachment-cut
  cell **"`run_server` boot refusal"** (DG-2 row 9) planned under P4-S13.
- **Layer-1 property:** refusal **iff** some root-namespace interface has a
  master OR a stacking link kind; a veth peer, tunnel/encrypting kinds (`vxlan`,
  `geneve`, `gre`, `ipip`, `xfrm`, `macsec`, `wireguard`) are NOT stacked.
  Proptest over a generated interface inventory. (Pure predicate is a crafter
  extraction — gaps G-1.)
- **Evidence lane:** seeded sim over `SimGuestFlowIo` (refusal + no seam
  effect) + layer-1 proptest for the predicate, both authored; the
  `run_server` projection at the cut (P4-S13). A Tier-3 stacked-topology body
  is not authored (see Status).

#### P5-S3 — guest datagrams survive a UDP-segmentation-offload interface byte-exact `@contract-shape:unbounded-preservation`
- **Status (2026-10-10):** authored: `overdrive-dataplane`
  `integration::vsock_egress_unframe`::`guest_datagrams_survive_a_uso_interface_byte_exact`
  — ignored `pending DELIVER vsock new-step 4 (host forwarder)`;
  `the_forwarder_crate_never_references_udp_segment` — ignored `pending
  DELIVER vsock new-step 4 (USO-NO-SEGMENT source check)`; a GUARD that
  already holds today (forwarder-crate half only). The owner crate's
  `UDP_SEGMENT` half and the `getsockopt(UDP_SEGMENT)` read-back on owner
  sockets belong to `overdrive-control-plane` and are not in the authored
  inventory; the full `run_server` USO path is P6-S13's.
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

### `vsock_fragment_map.rs` — Tier-3

#### P5-S4 — a fragmented guest datagram is unframed exactly once over its own train `@contract-shape:pure-function` (decision fn) / `@contract-shape:unbounded-preservation` (Tier-3)
- **Status (2026-10-10):** authored: `overdrive-dataplane`
  `integration::vsock_fragment_map`::`a_fragmented_guest_datagram_is_unframed_once_over_its_own_train`,
  `cut_train_then_unrelated_bare_fragmented_datagram_arrives_unshifted` (S1),
  `map_filled_beyond_capacity_loses_whole_datagrams_never_corrupts` (S2),
  `congested_qdisc_dropping_fragments_leaves_later_datagrams_byte_exact`
  (S3) — ignored `pending DELIVER vsock new-step 4 (host forwarder fragment
  map)`. The layer-1 decision-function proptest below is not authored (no
  pinned pure-function signature).
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

### Owner host policy — source-local in `overdrive-control-plane`, default lane over `SimGuestFlowIo` (attachment cut)

The 2026-10-09 file `overdrive-control-plane/tests/acceptance/vsock_host_internal_deny.rs`
no longer exists; it is the starting draft (@91f9436ff) for the
attachment-cut item "Host policy P5-S5..S7, S9 owner `NonMesh` decision"
(DG-2 rows 2, 3, 7, 9, 14): flows over `SimGuestFlowIo` with scripted
`route_type`, asserting `Refused(HostInternal | Policy |
DestinationUnreachable)`, the owner's O3 `flow_observation()` and the sim's
`outbox`.

#### P5-S5 — a guest flow to a host-internal destination is refused HostInternal `@contract-shape:pure-function`
- **Status (2026-10-10):** attachment cut (DG-2 rows 2, 3, 7, 9): flows over
  `SimGuestFlowIo` with scripted `route_type` drive the owner's per-flow
  policy; asserts `Refused(HostInternal)` / `Refused(Policy)` per the order
  below, O3 `flow_observation()` and S `outbox`, with allocation state
  unchanged. No authored test yet.
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
- **Status (2026-10-10):** attachment cut (DG-2 rows 2, 3, 7, 9): the owner's
  association-end classification over `SimGuestFlowIo` (scripted transmit
  error on H) asserts `FlowCloseCause::DestinationUnreachable`, then the next
  datagram `Refused(HostInternal)`, via O3 and S `outbox`. The kernel-rule
  half (a marked socket rejected regardless of when the address was added)
  is authored in P5-S8's `vsock_host_internal_enforce`; the forwarder-redirect
  transmit on H (R5-7, F-4) is P6-S13's walking skeleton.
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
- **Status (2026-10-10):** attachment cut (DG-2 rows 2, 3, 7, 9, 14; row 14 is
  `ServiceBackendsResolve::new(store, frontend, vip_ranges)`, landed in its
  own new-step 11): the D24a VIP branch arms and the owner's `Mesh` /
  `MeshUnreachable` decision over `SimGuestFlowIo`. The enforced mesh path is
  P6-S13's walking skeleton. No authored test yet.
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

### `overdrive-worker/tests/integration/vsock_host_internal_enforce.rs` — Tier-3 (`host-kernel-shared`)

#### P5-S8 — the host-internal output rule rejects marked owner sockets, structurally `@contract-shape:unbounded-preservation`
- **Status (2026-10-10):** authored: `overdrive-worker`
  `integration::vsock_host_internal_enforce`::`a_marked_tcp_connect_to_a_loopback_service_is_refused_econnrefused_without_hang`,
  `a_marked_tcp_connect_to_a_host_interface_address_is_refused`,
  `a_marked_udp_first_transmit_to_a_local_service_fails_eperm` (F-3),
  `a_marked_udp_socket_reusing_an_unmarked_flows_source_port_is_still_rejected`
  (F-1), `a_marked_udp_transmit_to_a_subnet_directed_broadcast_reaches_no_service`,
  `a_marked_udp_transmit_to_the_limited_broadcast_reaches_no_service`,
  `a_marked_udp_transmit_to_the_all_hosts_multicast_group_is_rejected_eperm`
  (F-5), `a_marked_tcp_connect_to_a_link_local_service_is_refused`,
  `a_packet_carrying_the_owner_bit_among_other_bits_is_refused` (F-6),
  `a_marked_connect_storm_across_repeated_table_replacements_reaches_no_local_service`
  (F-2), and three controls (ignored with their step; pass against today's table) `a_marked_tcp_connect_to_a_neighbour_stays_reachable`,
  `a_marked_udp_transmit_to_an_off_host_address_is_not_rejected` (F-5
  reachable half), `a_packet_carrying_only_the_leg_s_or_tproxy_mark_reaches_a_local_service`
  (F-6 unaffected half) — all ignored `pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)`. The controls hold
  against today's table; the rejection cells are RED until the D26 output
  rules land. The `gateway:53` accept cell and the guest prefix's broadcast
  reject cell need the shared prefix `local` route that only
  `overdrive-control-plane`'s `converge_shared` writes, so they move to
  P6-S13 at the attachment cut (UI-R2-14); a worker-crate body would have to
  hand-install that route. Layer-1: `overdrive-core`
  `dataplane::mtls_mark`::`overdrive_marks_are_single_bits_and_pairwise_disjoint`,
  `overdrive_marks_and_table_carry_their_pinned_values` (DG-9) — default
  lane, not ignored (GUARD).
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
    link-local). (The prefix-broadcast and `gateway:53` cells are P6-S13's,
    UI-R2-14.)
  - **F-6 (mark):** a packet carrying the owner bit with other bits set is
    rejected; a packet with only the leg-S or TPROXY mark is unaffected.
- **Production owner path:** `run_server` composes the shared table, whose
  owner is the mTLS worker (`MtlsInterceptWorker::start_shared_owner` over
  the production `HostMtlsIntercept`; a successor worker's call replaces the
  table). The owner marks H with `OWNER_EGRESS_MARK` for every direct-connect
  flow (not leg-F). In the authored bodies a marked userspace socket stands in
  for H; no body writes an nft rule, set or chain itself.
- **Layer-1 property:** `OWNER_EGRESS_MARK`, `MTLS_LEG_S_DIAL_MARK` and
  `MTLS_TPROXY_MARK` are **pairwise-disjoint single bits** — the DG-9
  default-lane unit test in `overdrive-core` `dataplane::mtls_mark`.
- **Fault stimulus:** each F-case enumerated as a named Tier-3 cell (`Sad_*`).
- **Oracle:** host service byte count == 0 for rejected dials; `ECONNREFUSED` /
  `EPERM` observed; latency bounded (no hang); reachable destinations still
  reachable; the mark-disjoint unit assertion.
- **Evidence lane:** Tier-3 (production table) + unit (mark constants).

### Mesh and non-mesh egress — `overdrive-worker` Tier-3 (P5-S10), owner decision at the cut (P5-S9)

#### P5-S9 — non-mesh guest egress is kernel-forwarded directly to its destination `@contract-shape:unbounded-preservation`
- **Status (2026-10-10):** attachment cut (DG-2 rows 2, 3, 7, 9; DG-16): the
  owner-level `NonMesh` decision — route-type check, marked direct connect,
  `DestinationUnreachable` projection — is a seeded-sim scenario over
  `SimGuestFlowIo`, source-local in `overdrive-control-plane`; asserts O3 and
  S `outbox`, and that no `register_forwarded_outbound` is made. The
  end-to-end non-mesh egress cell (destination receives bytes, no leg-F hop
  on the wire) is P6-S13's walking skeleton. Not in
  `vsock_mesh_egress_mtls.rs`. No authored test yet.
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
- **Status (2026-10-10):** authored: `overdrive-worker`
  `integration::vsock_mesh_egress_mtls`::`registered_mesh_flow_reaches_its_registered_backend_over_mtls_without_leg_f_resolving_again`,
  `a_replayed_peer_and_an_unregistered_peer_are_closed_by_leg_f_without_resolution`,
  `a_second_registration_for_the_same_forwarder_address_is_refused`,
  `an_allocation_without_a_live_intercept_cannot_register_a_forwarded_flow`,
  `dropping_an_unclaimed_registration_closes_the_flow_before_it_reaches_the_backend`,
  `dropping_a_claimed_registration_with_the_loopback_leg_open_resets_both_sides`,
  `dropping_a_claimed_registration_after_a_clean_close_keeps_the_remote_fin`,
  `forwarded_registration_refusals_follow_the_pinned_precedence`,
  `a_clean_close_of_both_directions_ends_the_remote_stream_with_fin`,
  `a_serve_exit_mid_flow_resets_the_remote_peer`; and the in-crate unit test
  `overdrive-worker` `mtls_intercept_worker`::`forwarded_outbound_refusals_have_pinned_stage_labels`
  — all ignored `pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)`.
  The DG-10 readings pin the bodies: reading 1 stage labels
  (`not_intercept_live`, `duplicate_forwarded_registration`), reading 2/3
  claimed vs unclaimed drop and clean-close FIN vs reset, reading 4 refusal
  precedence. The bodies drive the real `MtlsInterceptWorker`
  (`HostMtlsIntercept`, `HostMtlsEnforcement`, `start_alloc`) and play the
  owner only at the port boundary (`register_forwarded_outbound` plus the
  forwarder-side loopback socket); the `MtlsResolve` double counts calls and
  answers `MeshUnreachable`. The full `overdrive serve` + `overdrive deploy`
  mesh cell is P6-S13's.
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
- **Evidence lane:** Tier-3 adapter integration on the real
  `MtlsInterceptWorker` (remote peer holding an SVID chain from the test CA);
  the `overdrive serve` + `overdrive deploy` cell is P6-S13's.

## Required boundary scenarios — G-V1 (unframe-attach half) and G-V6 (egress legs)

Per feature-delta §"Required boundary scenarios" (rows 1-6). P5 owns the
unframe-attach half of G-V1 and the egress-legs half of G-V6.

| Row | G-V1 (unframe attach) | G-V6 (egress legs) | Covered by |
|---|---|---|---|
| 1 available | boot attaches every present interface; datagrams byte-exact | `Paired` mesh/non-mesh flow carries bytes | P5-S1, P5-S9, P5-S10 |
| 2 typed failure in owner domain | `UnframeAttach` / `StackedInterface` from the owner (authored); `health.startup.refused` through `run_server` (cut cell "`run_server` boot refusal", P4-S13) | `Refused(HostInternal)` / `Refused(MeshUnreachable)` / `Refused(DestinationUnreachable)` | P5-S1(sad), P5-S2, P5-S5, P5-S6, P5-S7; P4-S13 cut cell |
| 3 unrelated unchanged | boot refusal reclaims/adopts nothing; late-registered interfaces never gate | per-flow refusal never advances/revokes allocation `Running`/READY/Service `Stable` | P5-S2 (no effect), P5-S5 (alloc unchanged) |
| 4 late success can't resurrect | an interface registered after boot is never attached and carries no guest datagram (D5a-LATE) | — (flow-scoped; no terminal-state resurrection in P5 egress) | P5-S1 note, P5 doc §D5a-LATE |
| 5 disconnect/reconnect | — (boot-only) | association ended `DestinationUnreachable` does not re-associate by itself (U-2); next datagram opens a new one refused `HostInternal` | P5-S6 |
| 6 feature disabled | single-cut; substitute sim fixture `transport: None` behaves as `network: None` | ← | the planned attachment-cut cell P6-S12b (feature-disabled path, C5a / C5b) |

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

- **Activation markers (exact, 2026-10-10).** Every authored body carries a
  reasoned `#[ignore]` naming its activating DELIVER step:
  - `pending DELIVER vsock new-step 4 (host forwarder)` —
    `vsock_egress_unframe` P5-S1 / P5-S3 cells;
    `pending DELIVER vsock new-step 4 (USO-NO-SEGMENT source check)` —
    `the_forwarder_crate_never_references_udp_segment` (GUARD, holds today);
    `pending DELIVER vsock new-step 4 (host forwarder fragment map)` —
    `vsock_fragment_map` (P5-S4).
  - `pending DELIVER vsock attachment cut (new-step 4, probe_startup stacked-interface refusal, IFACE-NO-STACK)` and `pending DELIVER vsock attachment cut (new-step 4, host forwarder: egress unframe attach at boot, D5a-IFACE-BOOT)` —
    `prefix_boot_acceptance` (P5-S2, P5-S1 boot refusal).
  - `pending DELIVER vsock attachment cut (new-step 14, host-internal deny set, D26 output rules)` — `vsock_host_internal_enforce` (P5-S8, including
    its three controls, ignored with their step; they pass against today's
    table).
  - `pending DELIVER vsock attachment cut (new-step 8, mesh registration, D9)` —
    `vsock_mesh_egress_mtls` and `forwarded_outbound_refusals_have_pinned_stage_labels`
    (P5-S10).
  - The DG-9 `mtls_mark` unit tests are not ignored (default lane).
- **Attachment cut.** P5-S5, P5-S6 (owner half), P5-S7, P5-S9 (owner
  `NonMesh` half) are written at the cut (DG-2 rows 2, 3, 7, 9, 14), source-local
  in `overdrive-control-plane`, from the removed 91f9436ff draft
  `vsock_host_internal_deny.rs`. The P5-S9 end-to-end cell, the P5-S8
  `gateway:53` / guest-broadcast cells and every `overdrive serve` +
  `overdrive deploy` cell are P6-S13's (DG-16, UI-R2-14).
- **Blocked on nothing.** No contract-shaping validation item is open
  (V-16/17/18 = P-43/44/45; V-21 = P-47; all DONE) and every round-1 design
  gap is closed by a DG decision.
- **Gaps (2026-10-09 list, current disposition):**
  - **G-1 (pure-function extraction).** Unchanged as a crafter note: the
    attach-errno classifier and the fragment-decision function have no
    pinned public signature, so no layer-1 proptest is authored for them;
    a crafter extraction may add one in `overdrive-dataplane`'s default
    lane. The IFACE-NO-STACK property is authored
    (`probe_startup_refuses_iff_some_root_namespace_interface_is_stacked`).
    The D26 policy-order and D24a classification tests are at the cut.
  - **G-2 (shared sim + fixtures).** Closed. The owner seam sim is
    `SimGuestFlowIo` / `SimGuestNode` (`overdrive-control-plane`
    `#[cfg(test)] guest_network::flow_io::sim`), already used by
    `prefix_boot_acceptance`; the 2026-10-09 `SimSharedGuestNetworkOwner` /
    `SimGuestFlowForwarder` names are not used. The crate-local Tier-3 bodies
    drive their own crate's real objects (`GuestFlowForwarder` over the
    `overdrive-dataplane` `helpers::guest_net` veth fixtures;
    `MtlsInterceptWorker` over a `TestCidrLease` netns topology).
  - **G-3 (`.config/nextest.toml`).** Closed. The `overdrive-dataplane` and
    `overdrive-worker` `integration` binaries are assigned whole to
    `host-kernel-shared`.
  - **G-4 (placement).** Closed. No layer-1 proptest sits in an
    `integration-tests`-gated file; the mark check is the DG-9
    `overdrive-core` unit test.
  - **G-5 (compile verification).** Closed. Every authored body is
    registered and compiled (`cargo check --workspace --all-targets
    --features integration-tests` clean on Lima, per the feature-delta
    status); attachment-cut bodies are not in the tree (DG-2).

## Retired (by DELIVER roadmap reconciliation, NOT by this piece)

P5 production code replaces the old egress path. The old-model GREEN tests
defending it — `crates/overdrive-dataplane/tests/integration/{guest_tcx_egress_lifecycle,
guest_tcx_inventory}`, `crates/overdrive-worker/tests/.../outbound_enforce_substrate_splice`
(revalidate-vs-replace per the Contract-impact matrix) — are retired in DELIVER
coupled with removal of the old production code (shared-infra §7). This piece
does not delete them (deleting now leaves an uncommittable tree).
