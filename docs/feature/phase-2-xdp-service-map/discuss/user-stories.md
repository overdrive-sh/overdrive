<!-- markdownlint-disable MD024 -->

# User Stories — phase-2-xdp-service-map

Eight LeanUX stories, each delivering one carpaccio slice from
`story-map.md`. All stories share the persona (Ana, Overdrive platform
engineer) and the platform commitments from `docs/whitepaper.md` §7
(eBPF Dataplane), §15 (Zero Downtime Deployments — atomic backend
swaps), §19 (Security Model — DDoS baseline posture). Every story
traces to **J-PLAT-001** (DST trust under real failure conditions) and
**J-PLAT-004** (reconciler convergence against the simulated cluster
— activated by this feature; US-02 / US-08 cite `J-PLAT-004`
explicitly in their Who sections, with US-08 being the first
non-trivial use of the `Reconciler` trait against a real dataplane
port).

This feature **builds on** the eBPF scaffolding landed by
`phase-2-aya-rs-scaffolding` (#23). Every System Constraint from that
feature still applies; the additions here are the constraints that
come with shipping a non-trivial XDP program against real packets.

> **Phase 2.2 is single-kernel in-host.** Per #152, Tier 3 / Tier 4
> integration runs on the developer's local Lima VM and on CI's
> `ubuntu-latest` runner — no nested-VM kernel matrix. The kernel
> matrix landing is deferred to a later Phase 2 slice. The local-host
> kernel is a precondition shared by every story in this file.

> **Conntrack is OUT of scope.** This feature ships a stateless
> Maglev-style XDP forwarder. Maglev's ≤1% disruption property bounds
> flow misroute under backend churn until conntrack lands as a
> separate slice (#154). Every story below states this explicitly.

## System Constraints (cross-cutting)

These extend the constraints from the prior feature
(`phase-2-aya-rs-scaffolding`) — every constraint from
`docs/feature/phase-2-aya-rs-scaffolding/design/wave-decisions.md`
applies verbatim. The additional constraints specific to this feature:

- **The XDP program is `#![no_std]` and lives in `overdrive-bpf`.**
  Per ADR-0038. New SERVICE_MAP / BACKEND_MAP / REVERSE_NAT_MAP /
  MAGLEV_MAP declarations live in the kernel-side crate; the
  userspace loader and hydrator live in `overdrive-dataplane`. No
  kernel-side code crosses into `overdrive-host` or `overdrive-core`.
- **`Dataplane` port trait is the only consumer-facing surface.**
  Per ADR-0038 + `.claude/rules/development.md` § Port-trait
  dependencies. Reconcilers and the action shim see
  `Arc<dyn Dataplane>`; production wiring uses `EbpfDataplane`, DST
  uses `SimDataplane`. No code path imports `aya` directly outside
  `overdrive-dataplane`.
- **Hydrator reconciler is pure.** The new SERVICE_MAP hydrator
  reconciler MUST satisfy the existing `ReconcilerIsPure` DST
  invariant. No `.await`, no wall-clock reads, no `bpf_map_update_elem`
  calls inside `reconcile`. Wall-clock comes from `tick.now`. The
  action shim translates emitted typed Actions into syscalls.
- **Determinism in the hydrator is load-bearing.** Iteration order
  over `service_backends` rows MUST be `BTreeMap` per
  `.claude/rules/development.md` ordered-collection rule. The Maglev
  permutation generation (Slice 04) reads weighted-backend inputs in
  `BTreeMap` iteration order so the produced permutation table is
  bit-identical across runs and across nodes given identical inputs.
- **Newtypes are STRICT for new identifiers.** `ServiceVip`,
  `BackendId`, `MaglevTableSize`, `ServiceId` are NEW newtypes shipped
  by this feature (in `overdrive-core`). Each MUST have `FromStr`
  (validating, case-insensitive where appropriate per newtype
  completeness rule), `Display`, serde + rkyv derives, and full
  proptest round-trip coverage. Raw `String` / `u32` for any of
  these is a blocking violation.
- **Real infrastructure is gated behind `integration-tests`
  feature.** Per `.claude/rules/testing.md`. Default `cargo nextest
  run` exercises only the in-process `SimDataplane` envelope. Tier 2
  `BPF_PROG_TEST_RUN` tests, Tier 3 in-host integration tests, Tier
  4 perf gates all gate behind the feature flag.
- **Tier 3 / Tier 4 are SUDO'D IN-HOST single-kernel.** Per #152
  (deferred nested-VM kernel matrix). The developer runs the suite
  inside their Lima VM via `cargo xtask lima run --` per
  `.claude/rules/testing.md` § "Running tests on macOS — Lima VM";
  CI runs on `ubuntu-latest`. The `cargo xtask integration-test vm`
  LVH harness wired in #23 stays in place but is not exercised by
  Phase 2.2 — it lights up when #152's matrix lands.
- **Native XDP only; warn on generic fallback.** Per Cilium / Cloudflare
  precedent and the research recommendation. Lima virtio-net,
  `ubuntu-latest` virtio-net, and production mlx5 / ena all support
  native XDP. A failure to attach in native mode logs a structured
  warning (no silent fallback to `XDP_SKB`).
- **Conntrack is explicitly out of scope.** Stateless Maglev
  forwarder only; flow affinity is the ≤1% Maglev-disruption
  guarantee. Per-flow conntrack is #154. No `LRU_HASH` flow tables
  ship in this feature.
- **No new fields on existing aggregates.** `Job` and `Node` ship
  unchanged. Service / backend hydration reads the existing
  `service_backends` ObservationStore table; no schema migration.

---

## US-01: Real-iface XDP attach (veth, not `lo`)

### Problem

Phase 2.1 (#23) attaches the no-op `xdp_pass` to the loopback
interface `lo` because veth is not present in the dev VM by default.
But every production attach target — virtio-net (Lima, cloud guests),
mlx5 (bare metal), ena (AWS) — is a real driver, not the loopback
fallback. The lift from `lo` to veth is the smallest possible test of
the assumption that ifindex semantics are uniform across drivers.
Without it, the first time a SERVICE_MAP slice loads against a real
driver and fails, we do not know whether the failure is in the new
program, the new map, or the cross-driver attach path. Isolating the
attach-on-real-iface concern from the LB-logic concern collapses the
debugging surface area to one variable per slice.

### Who

- Overdrive platform engineer wiring the Phase 2.2 dataplane |
  motivated by the need for a known-good real-iface attach path
  before any LB logic lands so failures attribute cleanly to the
  slice that introduced them.

### Solution

Spin up a `veth0`/`veth1` pair inside the developer's Lima VM (and
on CI's `ubuntu-latest` runner) at integration-test setup. Re-target
Phase 2.1's `xdp_pass` program at `veth0`. Send a small batch of
crafted Ethernet frames from `veth1`. Assert the existing
`PACKET_COUNTER` (`LruHashMap<u32, u64>`) increments on the kernel
side. Tear the veth pair down at integration-test teardown.

### Domain Examples

#### 1: Happy Path — Attach to veth, send 100 frames, counter reads 100

`veth0`/`veth1` pair created via `ip link add veth0 type veth peer
name veth1`. `xdp_pass` attached to `veth0` via the Phase 2.1 loader
in native mode. 100 dummy Ethernet+IPv4 frames pushed from `veth1`
via `pnet`. `bpftool map dump name PACKET_COUNTER` reads `100`. veth
pair removed.

#### 2: Edge Case — Native attach fails, generic mode logs warning, integration test still passes

Driver does not support native XDP (synthetic test where the harness
forces generic mode). Loader logs structured warning
`xdp.attach.fallback_generic`. `xdp_pass` still attaches via
`XDP_SKB`. Counter still increments. Integration test passes — the
contract is "attach happens AND the warning surfaces", not "native
mode succeeds always."

#### 3: Error Boundary — veth0 doesn't exist when loader runs

Loader receives ifindex resolution error (ENODEV or similar). Returns
`DataplaneError::IfaceNotFound { iface: "veth0" }`. Integration test
asserts the structured error and does NOT silently fall through.

### UAT Scenarios (BDD)

#### Scenario: XDP program attaches to a real veth pair and counts a packet batch

Given a `veth0`/`veth1` pair exists on the host
And `xdp_pass` is attached to `veth0` in native mode
When 100 Ethernet+IPv4 frames are pushed from `veth1`
Then `bpftool map dump name PACKET_COUNTER` reads `100`
And the kernel-side counter is the only side effect observed

#### Scenario: Native attach failure logs structured fallback warning

Given the underlying driver lacks native XDP support
When the loader attempts `xdp.attach(iface, XdpFlags::default())`
Then a structured warning `xdp.attach.fallback_generic` is logged
And the program attaches in generic mode
And the integration test passes against the generic-mode attach

#### Scenario: Missing iface produces a structured error, not a silent fall-through

Given the named iface does not exist on the host
When the loader attempts to attach
Then it returns `DataplaneError::IfaceNotFound`
And no XDP program is loaded
And the integration test asserts the typed error variant

### Acceptance Criteria

- [ ] Tier 3 integration test `crates/overdrive-dataplane/tests/integration/veth_attach.rs` (gated `integration-tests`) creates a `veth0`/`veth1` pair, attaches `xdp_pass`, pushes 100 frames, asserts `PACKET_COUNTER == 100`, tears the pair down
- [ ] Loader resolves iface name to ifindex via `nix::net::if_::if_nametoindex` (or equivalent) and returns `DataplaneError::IfaceNotFound { iface }` on miss
- [ ] Native-mode attach is the default; fallback to `XDP_SKB` logs a `tracing::warn!` with structured `iface` field
- [ ] CI's `ubuntu-latest` runner has `iproute2` (which supplies `ip link`) — the integration test bails with a skip-message if not, but ubuntu-latest already includes it
- [ ] Test is sudo'd in-host per #152 deferral: developer's Lima VM is the canonical local environment via `cargo xtask lima run --`; nested-VM LVH path is NOT exercised
- [ ] Phase 2.1's existing loopback-iface integration test continues to pass (no regression)

### Outcome KPIs

- **Who**: Overdrive platform engineer + DST harness (the `EbpfDataplane` host adapter is exercised by every later slice's integration test)
- **Does what**: trusts the loader to attach against a real virtio-net-class driver and surface attach failures as typed errors
- **By how much**: 100% of integration test runs show `PACKET_COUNTER` incrementing on a real veth pair; 100% of native-attach failures surface as a `tracing::warn!` (not a silent regression to generic mode); 0 silent fall-throughs to generic mode without a structured warning
- **Measured by**: Tier 3 integration test gated `integration-tests`; structured-log assertion via `tracing-test` or equivalent
- **Baseline**: greenfield — Phase 2.1 attaches to `lo` only

### Technical Notes

- Direct `ip link` invocation via `std::process::Command` is acceptable per `.claude/rules/development.md` § Reconciler I/O — the integration test is the I/O boundary, not the reconciler.
- veth pair creation requires `CAP_NET_ADMIN`; `cargo xtask lima run --` defaults to root (per `.claude/rules/testing.md` § "Running tests on macOS — Lima VM"), so this is satisfied.
- This slice does NOT load any new BPF program — it re-targets Phase 2.1's `xdp_pass` at a real-iface ifindex. The structural change is in the loader / attach path, not in the kernel program.
- **Depends on**: phase-2-aya-rs-scaffolding (`EbpfDataplane`, `xdp_pass`, `PACKET_COUNTER`, `cargo xtask bpf-build`).

---

## US-02: SERVICE_MAP forward path with single hardcoded backend

### Problem

Phase 2.1's `xdp_pass` is `XDP_PASS` for every packet. Whitepaper §7
commits to "service load balancing — O(1) BPF map lookup for
VIP-to-backend resolution, replacing kube-proxy entirely." Without a
real SERVICE_MAP-shaped lookup, every later slice (Maglev,
HASH_OF_MAPS swap, REVERSE_NAT, packet-shape sanity, perf gates) is
adding to a non-existent foundation. The smallest possible useful
SERVICE_MAP slice — one VIP, one backend, hardcoded by the userspace
loader at attach time — proves the parsing + lookup + DNAT-rewrite
+ `XDP_TX` shape is verifier-clean and end-to-end testable, before
any of Maglev's algorithmic complexity lands.

This is the first slice where the **verifier complexity budget**
becomes a real measurement (not a theoretical concern from Phase
2.1's no-op program). It is also the first slice where the **typed
Rust map newtype API** the research recommended (#5 in
recommendations) starts paying off.

### Who

- Overdrive platform engineer wiring the SERVICE_MAP slice |
  motivated by the need for a verifier-clean, testably-correct,
  single-VIP forwarding path before any algorithmic LB logic lands
  on top.
- DST harness consuming `Arc<dyn Dataplane>` via the existing
  `Dataplane::update_service` port — this slice gives the trait its
  first non-no-op implementation behind `EbpfDataplane`, and the
  first non-no-op `SimDataplane` shape that the SERVICE_MAP hydrator
  reconciler that lands in this feature (Slice 08, US-08, serving
  `J-PLAT-004`) reads against.

### Solution

Land a new XDP program `xdp_service_map_lookup` in `overdrive-bpf`
that:
1. Parses Ethernet + IPv4 + TCP/UDP headers (returning `XDP_PASS`
   on any non-IPv4 / non-TCP/UDP frame for now — packet-shape
   sanity is Slice 06).
2. Looks up a 5-tuple-derived key in `SERVICE_MAP`
   (`BPF_MAP_TYPE_HASH`, key = `ServiceKey { vip, vport, proto }`,
   value = `Backend { ip, port }`).
3. On hit: rewrites the destination IP and port, recomputes the IP
   and TCP/UDP checksums, returns `XDP_TX`.
4. On miss: returns `XDP_PASS`.

Userspace: extend `EbpfDataplane::update_service` (currently a no-op
stub from Phase 2.1) to write a single `(ServiceKey, Backend)` entry
into `SERVICE_MAP` via aya. Add a typed `ServiceMapHandle` newtype
in `overdrive-dataplane` that wraps the aya `HashMap<_, _, _>` —
this is recommendation #5 from the research doc, paying off with the
first real call site.

Tier 2: PKTGEN/SETUP/CHECK triptych against
`xdp_service_map_lookup` — pktgen builds a TCP SYN to VIP, setup
populates SERVICE_MAP with one entry, check asserts `XDP_TX` returned
and the destination rewrite happened. Tier 3: integration test on
veth pair end-to-end (10 frames, all forwarded). Tier 4: veristat
records baseline instruction count.

### Domain Examples

#### 1: Happy Path — Single VIP rewrite

VIP `10.0.0.1:8080`, backend `10.1.0.5:9000`. Loader writes one
SERVICE_MAP entry. A TCP SYN arriving at `10.0.0.1:8080` on `veth0`
gets its dest rewritten to `10.1.0.5:9000`, checksum recomputed,
returned via `XDP_TX`. `tcpdump -i veth1` shows the frame leaving
with the rewritten destination.

#### 2: Edge Case — UDP packet to same VIP

Same VIP/backend pair. UDP packet arrives. ServiceKey carries
`proto=UDP`; the SERVICE_MAP entry was inserted with `proto=TCP`.
Lookup misses. `XDP_PASS` returned. The frame falls through to the
kernel networking stack as if the LB were not present.

#### 3: Error Boundary — Truncated frame

Frame with Ethernet header but truncated IPv4 (only 10 bytes of IP
header). Bounds check fails. `XDP_PASS` returned (not `XDP_DROP` —
packet-shape sanity is Slice 06; this slice's policy is "don't crash
on malformed input, let the kernel handle it").

### UAT Scenarios (BDD)

#### Scenario: Single-VIP TCP SYN is rewritten and forwarded

Given a SERVICE_MAP entry mapping VIP `10.0.0.1:8080` (TCP) to backend `10.1.0.5:9000`
And a TCP SYN to `10.0.0.1:8080` arrives on `veth0`
When the XDP program processes the frame
Then the dest IP becomes `10.1.0.5`
And the dest port becomes `9000`
And the IP and TCP checksums are valid
And the action returned is `XDP_TX`

#### Scenario: SERVICE_MAP miss returns XDP_PASS

Given an empty SERVICE_MAP
And a TCP SYN to `10.0.0.1:8080` arrives
When the XDP program processes the frame
Then the action returned is `XDP_PASS`
And no header rewrites happened

#### Scenario: Tier 2 PKTGEN/SETUP/CHECK triptych asserts the action and rewrite

Given a Tier 2 test harness with the program loaded
When PKTGEN builds a TCP SYN, SETUP populates SERVICE_MAP, CHECK invokes `BPF_PROG_TEST_RUN`
Then the returned action is `XDP_TX`
And the data-out buffer carries the rewritten dest IP / port
And the IP/TCP checksums are recomputed correctly

#### Scenario: Verifier accepts the program; instruction count baselined

Given the compiled `xdp_service_map_lookup.o`
When `veristat --emit insns` is run
Then the program is accepted by the verifier
And the instruction count is recorded under `perf-baseline/main/`
And the count is well below the 1M-privileged-instruction ceiling

### Acceptance Criteria

- [ ] `xdp_service_map_lookup` program lives in `crates/overdrive-bpf/src/programs/service_map.rs` and parses Eth + IPv4 + TCP/UDP
- [ ] `SERVICE_MAP` is a `BPF_MAP_TYPE_HASH` keyed on `ServiceKey { vip: u32, vport: u16, proto: u8 }`, value `Backend { ip: u32, port: u16 }`
- [ ] `ServiceVip` and `ServiceId` newtypes ship in `overdrive-core` per § System Constraints; `ServiceKey` and `Backend` are kernel-side `#[repr(C)]` POD that the userspace shape converts into via `From` impls
- [ ] `ServiceMapHandle` typed newtype in `overdrive-dataplane` wraps the aya `HashMap` and exposes `insert(ServiceVip, Backend)` / `remove(ServiceVip)` only — raw aya HashMap is not visible at the call site
- [ ] `EbpfDataplane::update_service` writes / removes single entries in `SERVICE_MAP` via `ServiceMapHandle`
- [ ] Tier 2 PKTGEN/SETUP/CHECK triptych passes via `BPF_PROG_TEST_RUN` (sudo'd in-host per #152)
- [ ] Tier 3 integration test sends 10 TCP SYNs through a veth pair, asserts all 10 are rewritten and forwarded, captured via `tcpdump`
- [ ] `veristat` baseline for `xdp_service_map_lookup` recorded under `perf-baseline/main/veristat-service-map.txt` — Slice 07 turns this into a CI gate
- [ ] `Dataplane` port trait surface is unchanged; only the implementation behind `EbpfDataplane::update_service` graduates from no-op to real
- [ ] Default lane (`cargo nextest run` without `integration-tests`) does NOT load any BPF program; `SimDataplane::update_service` continues to no-op behaviourally for now (Slice 03 evolves it)

### Outcome KPIs

- **Who**: Overdrive platform engineer + the SERVICE_MAP hydrator reconciler shipped in this feature (Slice 03's `HASH_OF_MAPS` swap reads against this map shape; Slice 08's hydrator is the first reconciler to drive the `Dataplane::update_service` port end-to-end)
- **Does what**: trusts a single-VIP forwarding path that is verifier-accepted, packet-correct, and end-to-end testable before any algorithmic LB logic lands
- **By how much**: 100% of Tier 2 SERVICE_MAP-hit packets return `XDP_TX` with valid checksums; 100% of SERVICE_MAP-miss packets return `XDP_PASS`; veristat instruction count baselined and < 50% of the 1M-privileged ceiling (research's "L1-cache fits" claim quantified)
- **Measured by**: Tier 2 PKTGEN/SETUP/CHECK; Tier 3 veth integration test; Tier 4 veristat output captured under `perf-baseline/main/`
- **Baseline**: greenfield — no SERVICE_MAP forwarding exists; Phase 2.1 ships `xdp_pass` only

### Technical Notes

- IP / TCP checksum recompute: aya provides helpers via the `aya-ebpf` `csum_diff` family, or the program can do `bpf_l3_csum_replace` / `bpf_l4_csum_replace` against the raw header bytes. DESIGN picks; both are well-documented and verifier-clean for IPv4 / TCP fast path.
- Endianness: SERVICE_MAP values are host-endian on the userspace side, network-endian on the kernel side. The `ServiceMapHandle` constructor explicitly takes host-endian inputs and converts; this is the seam that prevents the most common kind of LB-rewrite bug.
- IPv6 is OUT of scope for this slice — adding IPv6 doubles the program size and the verifier exploration surface; flagged as a future slice.
- ICMP is OUT of scope — same reason. The program returns `XDP_PASS` on non-TCP/UDP.
- This slice does NOT do REVERSE_NAT (return-path rewrite). Asymmetric routing is acceptable for this slice's test harness — the integration test asserts forward-path correctness only. REVERSE_NAT is Slice 05.
- **Conntrack is OUT of scope** — every flow looks like a fresh flow. The Maglev ≤1% disruption guarantee is not yet relevant because there is only one backend. Conntrack is #154.
- **Depends on**: US-01 (real-iface attach), Phase 2.1 scaffolding (`EbpfDataplane`, `Dataplane` port trait).

---

## US-03: HASH_OF_MAPS atomic per-service backend swap

### Problem

Whitepaper §15 commits to **zero-drop atomic backend swaps** for
canary, rolling, and blue/green deployments. Slice 02's
`SERVICE_MAP` (a `BPF_MAP_TYPE_HASH`) cannot deliver this — replacing
a service's backend set means N sequential `bpf_map_update_elem`
calls, with a transient window where readers see a mix of old and
new backends. The research finding 3.3 establishes that Cilium's
`HASH_OF_MAPS` shape — outer map keyed by service ID, inner map
holds the per-service backend table — is the only shape that
delivers the §15 zero-drop guarantee with off-the-shelf BPF
primitives. A single 64-bit pointer swap on the outer map is
atomic; readers always see either the old or the new full table.

This slice is the §15 zero-drop guarantee made concrete. It is
also the first slice where multiple backends per service exist —
the first time round-robin across backends becomes visible. (Random
selection is acceptable here; Maglev consistent hashing is Slice
04.)

### Who

- Overdrive platform engineer | motivated by the need to validate
  the §15 atomic-swap claim end-to-end before Maglev (which depends
  structurally on this shape).
- Operator who runs canary deployments via §15 weighted backends —
  this slice ships the substrate; weighted Maglev (Slice 04) is the
  algorithm that uses it.

### Solution

Restructure SERVICE_MAP from a flat `HashMap` into a **two-level
shape**:
- Outer map `SERVICE_MAP` is `BPF_MAP_TYPE_HASH_OF_MAPS`, keyed by
  `ServiceId` (a stable u32 service identifier).
- Inner map (one per service) is `BPF_MAP_TYPE_ARRAY` of size N
  (slot count), value `BackendId`. This is the first piece of the
  Cilium three-map split — backend lookup table.
- New flat map `BACKEND_MAP` is `BPF_MAP_TYPE_HASH` keyed by
  `BackendId` (stable u32), value `Backend { ip, port, weight,
  flags }`. This is the second piece — backend address resolution.

XDP program restructures the lookup: hash-derived slot index → inner
array → `BackendId` → BACKEND_MAP → backend address → DNAT-rewrite +
`XDP_TX`. Random hashing of the 5-tuple to a slot is acceptable
(Slice 04 replaces with Maglev).

`EbpfDataplane::update_service` now performs the two-level swap:
1. Insert / update relevant rows in `BACKEND_MAP`.
2. Allocate a fresh inner map populated with the new backend slot
   table.
3. `bpf_map_update_elem(SERVICE_MAP, &service_id, &new_inner_fd)`
   — single atomic pointer swap.
4. Release the old inner map (kernel refcounts; release happens
   automatically once no XDP invocation is referencing it).
5. Garbage-collect orphaned BACKEND_MAP entries.

Tier 3 integration test: under sustained `xdp-trafficgen` load
(100 kpps minimum), perform an atomic swap from backend-set
`{B1}` to `{B1, B2, B3}`. Assert **zero packet drops** across the
swap window; assert post-swap traffic distributes across all three
backends.

### Domain Examples

#### 1: Happy Path — Atomic swap during traffic, zero drops

Service `S1` has one backend `B1 = 10.1.0.5:9000`. xdp-trafficgen
sends 100 kpps of TCP SYNs to VIP `10.0.0.1:8080`. While traffic
flows, `EbpfDataplane::update_service` swaps `S1`'s inner map to
contain `{B1, B2, B3}`. Counter snapshot: every packet pre-swap
went to B1; every packet post-swap distributes across B1/B2/B3
(roughly evenly given hash distribution); ZERO packets are dropped
across the swap window.

#### 2: Edge Case — Removing a backend orphans BACKEND_MAP rows; sweeper reclaims

Service `S1` swaps from `{B1, B2, B3}` to `{B1, B2}`. Inner-map
swap atomic; no in-flight readers see `B3` after the swap. A
follow-up GC pass walks `BACKEND_MAP` and removes `B3` if no
service references it. The integration test asserts the
post-GC `BACKEND_MAP` size matches the live-backend count.

#### 3: Error Boundary — Inner-map allocation fails (kernel rejects, e.g. memlock cap)

Loader attempts `MapData::create` for the new inner map; kernel
returns ENOMEM (or EPERM if memlock cap exceeded). Loader returns
`DataplaneError::MapAllocFailed { source }`. The action shim
writes the failure as observation; the existing inner map is
NOT touched (atomicity preserved on failure path). The next
hydrator tick can retry.

### UAT Scenarios (BDD)

#### Scenario: Atomic backend swap drops zero packets under traffic

Given service `S1` has one backend and xdp-trafficgen pushes 100 kpps to its VIP
When the loader swaps `S1`'s inner map to a three-backend table
Then ZERO packets are dropped across the swap window
And post-swap packets distribute across the three backends within ±10% of even distribution

#### Scenario: Removing a backend leaves no orphans after the GC pass

Given service `S1` references backends `{B1, B2, B3}` in `BACKEND_MAP`
When `EbpfDataplane::update_service` swaps `S1` to `{B1, B2}` and runs the GC pass
Then `BACKEND_MAP` no longer contains `B3`
And no service references `B3`

#### Scenario: Inner-map allocation failure preserves the existing service mapping

Given service `S1` has a live inner map with backends `{B1, B2}`
When `EbpfDataplane::update_service` attempts to swap and the kernel rejects the inner-map allocation
Then `update_service` returns `DataplaneError::MapAllocFailed`
And the existing inner map is unchanged
And subsequent traffic continues to forward against `{B1, B2}`

#### Scenario: SimDataplane mirrors the atomic-swap semantics under DST

Given `SimDataplane` has a service mapping with backends `{B1}`
When `update_service` is called atomically with `{B1, B2, B3}`
Then any concurrent `lookup_service` call sees either `{B1}` or `{B1, B2, B3}`
And NEVER a mixed state (e.g. `{B1, B2}`)

### Acceptance Criteria

- [ ] `SERVICE_MAP` declared as `BPF_MAP_TYPE_HASH_OF_MAPS` in `overdrive-bpf`; outer key `ServiceId`, inner is `BPF_MAP_TYPE_ARRAY` of `BackendId` with operator-tunable size (default 256 — well below Maglev sizes; that comes Slice 04)
- [ ] `BACKEND_MAP` declared as `BPF_MAP_TYPE_HASH` in `overdrive-bpf`; key `BackendId`, value `Backend { ip, port, weight, flags }`
- [ ] `BackendId`, `ServiceId` STRICT newtypes in `overdrive-core` per System Constraints
- [ ] `EbpfDataplane::update_service` performs the five-step atomic swap: BACKEND_MAP upsert → fresh inner map → outer-map pointer swap → orphan GC → release
- [ ] `SimDataplane::update_service` updates its in-memory shape atomically (one mutex acquisition per swap; the mutation is a single BTreeMap reassignment); mirrors the EbpfDataplane semantics so DST replay matches production
- [ ] Tier 3 integration test under `xdp-trafficgen` 100 kpps load asserts ZERO packet drops across an atomic swap
- [ ] DST invariant `BackendSetSwapAtomic` (always invariant: at every observation, every service's backend set is either the pre-swap set or the post-swap set, never a mixed state) lands in `overdrive-sim::invariants` and passes
- [ ] BACKEND_MAP orphan-GC integration test asserts post-swap orphan count == 0

### Outcome KPIs

- **Who**: Overdrive platform engineer + DST harness + operators running canary deploys
- **Does what**: trusts the §15 zero-drop atomic backend swap end-to-end against a real kernel
- **By how much**: 0 dropped packets across an atomic backend-set swap under 100 kpps of `xdp-trafficgen` load (Tier 3); 100% of `BackendSetSwapAtomic` invariant evaluations pass (DST); 0 `BACKEND_MAP` orphans after the GC pass
- **Measured by**: Tier 3 integration test (zero-drop assertion); DST invariant on every PR; orphan-count assertion in integration test
- **Baseline**: greenfield — no two-level map exists; Slice 02 ships flat `HashMap` only

### Technical Notes

- Kernel-side `bpf_map_lookup_elem(&SERVICE_MAP, &service_id)` returns a pointer to the inner map; verifier requires a NULL check before the second-level lookup. This is a well-documented `HASH_OF_MAPS` pattern (Cilium reference); the program is verifier-clean by construction.
- Userspace inner-map allocation: aya 0.13 exposes `MapData::create` for the fixed `BPF_MAP_TYPE_ARRAY` shape with a known size; the loader allocates a fresh inner map by calling `MapData::create` per swap.
- Round-robin / random slot selection here is intentional simplicity; Slice 04 replaces with Maglev. The verifier complexity gap between random and Maglev is small (research §5.4); recording the random-mode baseline now lets Slice 04's veristat delta be a clean comparison.
- The orphan GC sweep is a userspace function called from `update_service`; it is NOT a separate reconciler. (A reconciler-driven GC sweep is a candidate for a future slice if the live-orphan signal becomes visible enough to warrant separate ownership.)
- **Conntrack is OUT of scope** — flow-affinity stickiness is the Maglev ≤1% guarantee (Slice 04) plus the future #154 conntrack table. This slice has no flow memory.
- **Depends on**: US-02 (single-VIP forward path).

---

## US-04: Maglev consistent hashing inside MAGLEV_MAP

### Problem

Slice 03's random-slot lookup gives even load distribution but is
not **consistency-preserving** under backend churn — every backend
add/remove redistributes 100% of flows to new backends. Whitepaper
§15 commits to canary deployments (95% v1 / 5% v2 weighted
distribution) without forcing existing flows to migrate. Maglev
consistent hashing — Eisenbud et al. NSDI 2016, in production at
Google, Katran, Cilium — is the algorithm that delivers ≤1% flow
disruption per backend change.

This slice is also where the **verifier-complexity** question
becomes a real engineering risk. Random slot selection is a few
instructions; Maglev's table-indexing pattern, while still O(1),
runs through a fixed-size permutation table that the verifier must
prove safe. The research finding 5.4 ("L1-cache-fits") says
production Maglev programs stay well under the 1M-privileged
ceiling; the slice produces the empirical veristat number that
either confirms or disproves this for an aya-rs-written program.

This slice ALSO ships the **weighted Maglev** variant — the
algorithm modification where backends contribute multiple slot
entries proportional to their weight — because §15's canary
shape requires it. Vanilla Maglev as a stepping-stone with a
weighted-Maglev follow-on is not credibly cheaper than landing
weighted Maglev directly given the userspace permutation generator
holds most of the weight-aware logic.

### Who

- Overdrive platform engineer | motivated by the §15 canary
  commitment — without weighted Maglev, the platform cannot
  honestly claim "weighted backends (e.g., 95% v1, 5% v2)."
- DST harness | the Maglev permutation generator is a pure function
  whose determinism the harness asserts (research recommendation
  #6); flakiness here would propagate to every later DST run.

### Solution

Add a third map `MAGLEV_MAP` of type `BPF_MAP_TYPE_HASH_OF_MAPS`,
outer keyed by `ServiceId`, inner is `BPF_MAP_TYPE_ARRAY` of size
`M = 16381` (default; operator-tunable per service from the Cilium
prime list per research finding 5.2), value `BackendId`. The XDP
program's lookup becomes: hash 5-tuple → modulo `M` → MAGLEV_MAP
lookup → BackendId → BACKEND_MAP → backend address → DNAT.

Userspace ships a `maglev::generate(backends: &BTreeMap<BackendId,
Weight>, m: MaglevTableSize) -> Vec<BackendId>` pure function
implementing the Eisenbud weighted-Maglev table generation. Inputs
iterated in `BTreeMap` order so the permutation is bit-identical
across runs and across nodes.

`EbpfDataplane::update_service` now generates a fresh Maglev table
on every backend-set change, prepares a fresh inner MAGLEV_MAP,
and atomically swaps it via the Slice 03 swap shape.

Tier 3: validate the ≤1% disruption property — under
`xdp-trafficgen` load with 100 backends and stable 5-tuple flows,
remove one backend and assert ≤1% of flows shift backend (above
the 1/N=1% bound is a regression). Tier 4: veristat baseline
update — the Maglev program's instruction count is the new
baseline.

### Domain Examples

#### 1: Happy Path — Even distribution across 100 backends

Service `S1` has 100 equally-weighted backends. xdp-trafficgen
sends 100k synthetic 5-tuple flows. Each backend receives 1000±50
flows (within ±5% of even distribution).

#### 2: Edge Case — Weighted (95% / 5%) canary distribution

Service `S1` has backends `B1` (weight 95) and `B2` (weight 5).
100k flows arrive. `B1` receives ~95,000±500; `B2` receives
~5,000±200. Within ±2% of declared weight (Maglev's variance
property holds at this scale).

#### 3: Error Boundary — Removing one of 100 backends shifts ≤2% of flows total

Service `S1` has 100 backends, 100k flows pinned by 5-tuple.
Backend `B50` is removed; `update_service` regenerates the
permutation and atomically swaps. The Maglev guarantee is "≤1%
additional disruption beyond the minimum forced shift" — equivalently,
≤2% total flow shift when one of 100 backends is removed (1% forced
from B50's evicted flows + ≤1% incidental shift on flows that were
not on B50 pre-removal). The remaining ≥98% stay pinned to their
pre-removal backend.

### UAT Scenarios (BDD)

#### Scenario: Maglev distribution is even across equally-weighted backends

Given service `S1` has 100 equally-weighted backends
And 100k synthetic 5-tuple flows are sent
When MAGLEV_MAP is consulted for each flow
Then each backend receives 1000±50 flows (±5% of even)

#### Scenario: Weighted Maglev honors declared weights

Given service `S1` has backends `B1` (weight 95) and `B2` (weight 5)
And 100k flows are sent
Then `B1` receives 95,000±2000 flows
And `B2` receives 5,000±2000 flows

#### Scenario: Removing one of 100 backends shifts ≤2% of flows total (Maglev's ≤1% incidental-disruption guarantee)

Given service `S1` has 100 backends with 100k flows pinned by 5-tuple
When backend `B50` is removed and the Maglev table is regenerated and swapped
Then every flow previously on `B50` is shifted to some other backend (the 1% forced shift)
And ≤1% of flows that were NOT on `B50` pre-removal land on a different backend (the ≤1% incidental shift)
And the total flow shift is ≤2% across the 100k-flow population

#### Scenario: Maglev permutation generation is deterministic

Given a fixed `(BTreeMap<BackendId, Weight>, MaglevTableSize)` input
When `maglev::generate(...)` is called twice in succession
Then both calls return the bit-identical permutation `Vec<BackendId>`

#### Scenario: Verifier accepts the Maglev program; instruction count under 50% of ceiling

Given the compiled XDP program with Maglev lookup
When `veristat` is run
Then the program is accepted by the verifier
And the instruction count is ≤ 50% of the 1M-privileged-instruction ceiling
And the count is recorded as the new `perf-baseline/main/veristat-service-map.txt` baseline

### Acceptance Criteria

- [ ] `MAGLEV_MAP` declared as `BPF_MAP_TYPE_HASH_OF_MAPS`; inner `BPF_MAP_TYPE_ARRAY` of size `M=16381` (default), value `BackendId`
- [ ] `MaglevTableSize` STRICT newtype in `overdrive-core` constrained to the Cilium prime list `{251, 509, 1021, 2039, 4093, 8191, 16381, 32749, 65521, 131071}`; `FromStr` rejects non-prime / out-of-list values with structured `ParseError`
- [ ] `maglev::generate(&BTreeMap<BackendId, Weight>, MaglevTableSize) -> Vec<BackendId>` lives in `overdrive-dataplane` (or a dedicated `overdrive-maglev` crate — DESIGN picks); is a pure synchronous function
- [ ] Proptest covers `maglev::generate` determinism: same `(backends, M)` input produces bit-identical output across calls
- [ ] Proptest covers ±5% even-distribution under equally-weighted backends and ±2% honoring of declared weights under skewed weights
- [ ] Tier 3 integration test asserts ≤2% total flow shift when one of 100 backends is removed (1% forced shift from `B50`'s evicted flows + ≤1% incidental shift per Maglev's published bound; equivalently ≤1% additional disruption beyond the minimum forced shift)
- [ ] Tier 3 integration test asserts ZERO packet drops under atomic Maglev table swap (composes with Slice 03's atomic-swap guarantee)
- [ ] `veristat` records the new baseline; instruction count ≤ 50% of 1M ceiling (the research's "fits in L1" claim quantified)
- [ ] DST invariant `MaglevDistributionEven` (eventual invariant: across N≥1024 simulated 5-tuples, each backend receives ≥M/N · 0.95 flows) lands in `overdrive-sim::invariants` and passes
- [ ] DST invariant `MaglevDeterministic` (always: identical inputs produce identical permutation) lands and passes

### Outcome KPIs

- **Who**: Overdrive platform engineer + operators running canary deploys
- **Does what**: trusts Maglev's ≤1% disruption property and the §15 weighted-canary commitment end-to-end against a real kernel
- **By how much**: ≤2% total flow shift on single-backend removal among 100 backends — 1% forced (B50's evicted flows) + ≤1% incidental, per Maglev's ≤1%-incidental-disruption bound (Tier 3); even distribution within ±5% across equal-weight backends (Tier 3 + DST); declared weights honored within ±2% (Tier 3); veristat instruction count ≤ 50% of ceiling (Tier 4)
- **Measured by**: Tier 3 integration test under `xdp-trafficgen` synthetic flows; DST invariants on every PR; veristat baseline update under `perf-baseline/main/`
- **Baseline**: greenfield — Slice 03's random hashing is the prior-step comparison

### Technical Notes

- Maglev permutation table generation is computationally non-trivial (O(M log N) for prime M and N backends); ship it pure in userspace — the kernel side just indexes the resulting array. Research finding 5.4.
- The `MaglevTableSize` newtype enforces M ∈ Cilium prime list at the API boundary; this is the type-system enforcement of the research's "M must be prime" invariant.
- Weighted Maglev: Eisenbud's `offset` and `skip` per-backend computation is multiplied by weight to produce repeated entries. Research finding 5.3.
- The flow-pinning property under removal (≤1% disruption for one backend among 100) requires M ≥ 100·N (research finding 5.2). The default M=16381 supports up to ~160 backends per service before disruption-bound degrades.
- The hash function feeding the modulo into MAGLEV_MAP indexing is the same 5-tuple hash as Slice 03's random slot selection. Same hash → same DST replay shape; the only delta is the lookup table.
- **Conntrack is OUT of scope** — Maglev's ≤1% disruption is the only flow-affinity guarantee until #154 lands. Acceptable per research §4.3 — Maglev's M ≥ 100·N means most flows survive backend churn naturally.
- **Depends on**: US-03 (HASH_OF_MAPS atomic swap shape).

---

## US-05: REVERSE_NAT_MAP for response-path rewrite

### Problem

Slices 02-04 deliver forward-path forwarding (client → backend).
Backends respond from their backend address; without REVERSE_NAT,
the response from `10.1.0.5:9000` to the client is leaving with the
backend's source address instead of the VIP `10.0.0.1:8080`. The
client's TCP stack will reject the response (out-of-flow), the
connection breaks, and the LB is functionally unusable on every
real traffic path that matters.

The research §2.1 says Cilium's `cilium_lb4_reverse_nat` is the
third map of the three-map split — keyed by reverse-NAT index,
value `lb4_reverse_nat { address, port }`. On the response path,
the backend's source IP is rewritten back to the VIP before the
client sees it.

This is the slice that makes the whole forward-path infrastructure
end-to-end useful. Without it, Slice 02-04 are correct in
isolation but practically inert — no real client/backend pair can
complete a TCP connection.

### Who

- Overdrive platform engineer | motivated by the need to close the
  forward + return paths so the LB is end-to-end functional against
  a real client/backend pair (not just synthetic forward-path
  traffic).
- Operator who runs services exposed via VIPs — without REVERSE_NAT,
  the operator-visible "the LB doesn't work" failure mode is
  guaranteed.

### Solution

Add `REVERSE_NAT_MAP` (third map of the Cilium three-map split):
`BPF_MAP_TYPE_HASH`, key `BackendKey { backend_ip, backend_port,
proto }`, value `Vip { ip, port }`. Add a second XDP program
`xdp_reverse_nat` attached on the egress side of the same iface
(or as a TC egress program — DESIGN picks based on what aya 0.13
supports cleanly) that:
1. Parses Eth + IPv4 + TCP/UDP.
2. Looks up the backend's source 5-tuple in `REVERSE_NAT_MAP`.
3. On hit: rewrites the source IP/port back to the VIP, recomputes
   checksums, returns `XDP_TX` (or `TC_ACT_OK` for the TC-egress
   variant).
4. On miss: returns `XDP_PASS` / `TC_ACT_OK` (not LB traffic;
   pass-through).

Userspace: `EbpfDataplane::update_service` now also writes
REVERSE_NAT_MAP entries — when service `S1` references backend
`B1 = 10.1.0.5:9000` for VIP `10.0.0.1:8080`, the reverse map gets
key `(10.1.0.5, 9000, TCP) → (10.0.0.1, 8080)` written. Removed
backends remove their reverse entries.

Tier 3 integration test: spin up a real `nc` listener as backend on
veth1; send a real TCP connection to the VIP from veth0's network
namespace via `nc`. Assert the connection completes (3-way
handshake + payload + close) without manual response routing.

### Domain Examples

#### 1: Happy Path — Real TCP connection completes through forward + reverse paths

Backend `nc -l 9000` running on `veth1`'s namespace; VIP
`10.0.0.1:8080` configured with backend `B1 = 10.1.0.5:9000`. Client
on `veth0`'s namespace runs `nc 10.0.0.1 8080`. Forward path: SYN
rewritten to backend; SYN-ACK rewritten back to VIP; ACK
re-rewritten to backend. Connection completes; payload echoes;
`nc` exits cleanly.

#### 2: Edge Case — Backend response from a non-served port falls through

Backend `B1 = 10.1.0.5:9000` is the registered backend for VIP
`10.0.0.1:8080`. The same backend host has another process listening
on `10.1.0.5:5555`. A response from port 5555 (unrelated to LB
traffic) hits the reverse-NAT lookup, misses (not in
REVERSE_NAT_MAP), and falls through (`XDP_PASS`). The unrelated
traffic continues to function untouched.

#### 3: Error Boundary — Removed backend's old REVERSE_NAT entry doesn't leak

Backend `B1` is removed from service `S1`. `update_service`
regenerates the Maglev table AND removes `B1`'s REVERSE_NAT_MAP
entry. A late response from `B1` (e.g. for a flow that was
in-flight when the backend was removed) hits the reverse-NAT
lookup and misses; falls through as if from a non-LB source.
Stale-rewrite-leak is impossible.

### UAT Scenarios (BDD)

#### Scenario: Real TCP connection completes through forward and reverse paths

Given a `nc -l 9000` listener on `veth1` and a VIP `10.0.0.1:8080` mapping to it
When a client on `veth0` runs `nc 10.0.0.1 8080` and writes a payload
Then the connection completes the 3-way handshake
And the payload echoes
And `nc` exits with code 0

#### Scenario: Non-LB backend traffic falls through reverse-NAT untouched

Given a backend host has both a registered LB backend and an unrelated listener
When the unrelated listener emits traffic
Then the reverse-NAT lookup misses
And the traffic falls through with action `XDP_PASS` / `TC_ACT_OK`
And no source-address rewrite is applied

#### Scenario: Removed backend's REVERSE_NAT entry is purged on service update

Given service `S1` references backend `B1` and `REVERSE_NAT_MAP` contains `B1`'s entry
When `update_service` removes `B1`
Then `REVERSE_NAT_MAP` no longer contains `B1`'s entry
And a late response from `B1` is treated as non-LB traffic

### Acceptance Criteria

- [ ] `REVERSE_NAT_MAP` declared as `BPF_MAP_TYPE_HASH` in `overdrive-bpf`; key `BackendKey { ip, port, proto }`, value `Vip { ip, port }`
- [ ] `xdp_reverse_nat` program ships in `crates/overdrive-bpf/src/programs/reverse_nat.rs`; loaded on the egress side of the same iface (or TC egress; DESIGN picks)
- [ ] `EbpfDataplane::update_service` writes / removes REVERSE_NAT_MAP entries in lockstep with service-backend changes; the existing atomic-swap discipline (Slice 03) extends to cover the third map
- [ ] Tier 2 PKTGEN/SETUP/CHECK triptych for `xdp_reverse_nat`: pktgen builds a backend response, setup populates REVERSE_NAT_MAP, check asserts source-address rewrite
- [ ] Tier 3 integration test runs a real `nc` server and client across a veth pair and asserts the connection completes end-to-end with payload echoed
- [ ] DST invariant `ReverseNatLockstep` (always: every forward-path SERVICE_MAP entry has a matching REVERSE_NAT_MAP entry; removing a backend purges both) lands and passes
- [ ] `SimDataplane` mirrors REVERSE_NAT_MAP semantics so DST invariants run unchanged

### Outcome KPIs

- **Who**: Overdrive platform engineer + future operators
- **Does what**: trusts the LB to handle real bidirectional TCP connections through both VIP→backend and backend→client paths
- **By how much**: 100% of Tier 3 `nc` connection-completion runs succeed; 100% of `ReverseNatLockstep` invariant evaluations pass; 0 stale REVERSE_NAT entries after backend removal
- **Measured by**: Tier 3 real-TCP integration test; DST invariant on every PR
- **Baseline**: greenfield — no return-path rewrite exists

### Technical Notes

- The forward-path `xdp_service_map_lookup` (US-02 / US-04) and the return-path `xdp_reverse_nat` are two separate XDP programs, not one bidirectional program. Splitting keeps each program simple, verifier-clean, and independently veristat-able.
- **Endianness lockstep with US-02**: `BackendKey` (the REVERSE_NAT_MAP key) and the REVERSE_NAT_MAP value shape follow the same host→network endian conversion ownership as `ServiceMapHandle` (US-02) — written by a sibling typed handle (`ReverseNatMapHandle`) that takes host-endian inputs and converts on write. This is the same seam that prevents the most common kind of LB-rewrite bug; the discipline is identical for forward and reverse maps.
- TC-egress vs XDP-egress: aya 0.13 has solid TC support; an XDP-egress hook (newer kernels) is also viable. DESIGN picks; either satisfies the AC. The Tier 3 test is hook-agnostic (it asserts behaviour, not the attach point).
- Without conntrack, every response packet pays the full reverse-NAT lookup cost. With conntrack (#154), most responses can skip the lookup. The performance delta is acknowledged and quantified by Slice 07's perf gates.
- This slice is where the asymmetric-routing question becomes real — backend nodes that aren't the same node that the forward path entered need separate REVERSE_NAT_MAP hydration. Phase 2.2 single-host integration test sidesteps this; cross-node REVERSE_NAT is a future Phase 2 slice (alongside #154).
- **Conntrack is OUT of scope** — per research §4. The reverse-NAT lookup is stateless; it does not remember which client originated a flow.
- **Depends on**: US-04 (Maglev forward path provides the SERVICE_MAP entries that REVERSE_NAT mirrors).

---

## US-06: Pre-SERVICE_MAP packet-shape sanity checks

### Problem

§7's "DDoS mitigation — drop attack traffic before it consumes
kernel resources" and §19's defense-in-depth layer 4 (XDP network
policy) both require **pre-lookup** drop of pathological traffic.
A packet that is not IPv4-or-IPv6, not TCP-or-UDP, or has nonsense
TCP flag combinations should NEVER reach the SERVICE_MAP lookup —
it consumes verifier-explored state and CPU cycles for nothing.
Cloudflare's published technique stack (research §7.2) does
exactly four checks before any map lookup: EtherType, IP
version+IHL, transport protocol, TCP flag sanity.

The whitepaper §7's stronger DDoS posture (operator-tunable rules
compiled into BPF bytecode) is OUT of this slice — that is
POLICY_MAP territory (#25). This slice ships the cheap, static,
hardcoded checks that every reasonable XDP LB has.

This is also the slice that establishes the **drop-rate baseline**
for synthetic-malformed traffic — a signal Slice 07's Tier 4 perf
gates can compare against.

### Who

- Overdrive platform engineer | motivated by the §7 / §19
  defense-in-depth claim and by the verifier-budget concern (every
  pathological packet that reaches SERVICE_MAP costs verifier
  budget).
- Future operator | this slice is the substrate for #25's
  operator-tunable POLICY_MAP rules.

### Solution

Insert a sanity-check prologue at the top of
`xdp_service_map_lookup` (and `xdp_reverse_nat`):
1. EtherType is IPv4 (`0x0800`) — non-IPv4 returns `XDP_PASS`
   (let the kernel handle ARP, IPv6, etc.).
2. IP version is 4 and IHL ≥ 5 (20 bytes) — invalid headers
   return `XDP_DROP`.
3. IP total_length sanity (≥ IHL\*4, ≤ packet length).
4. Transport protocol is TCP (`6`) or UDP (`17`) — others return
   `XDP_PASS`.
5. For TCP: flag combination is not nonsense (no SYN+RST, no
   SYN+FIN, no all-zero flags) — invalid combinations return
   `XDP_DROP`.

A new `DROP_COUNTER` (`PerCpuArray<u64>`) records the count of
DROPs by sanity-check class; this is the operator-visible signal
for "how much pathological traffic are we seeing." The hydrator
reconciler reads this counter as observation (Slice 07 wires this).

Tier 3: send a batch of synthetic pathological frames (non-IPv4,
truncated IPv4, SYN+RST TCP, ICMP) and assert `DROP_COUNTER`
increments correctly per class; assert legitimate forwarding
behavior unchanged for valid frames.

### Domain Examples

#### 1: Happy Path — Valid frame passes sanity checks unchanged

A normal TCP SYN to a registered VIP. EtherType IPv4, IP version 4,
IHL 5, total_length matches packet, protocol TCP, flags SYN-only.
All five checks pass; SERVICE_MAP lookup proceeds; forwarding
behaviour unchanged from US-02 / US-04.

#### 2: Edge Case — IPv6 frame falls through (not dropped)

An IPv6 frame to the same iface. EtherType `0x86DD` ≠ IPv4. Returns
`XDP_PASS`. Falls through to the kernel's networking stack
unchanged. Operator-observable: no DROP_COUNTER increment for this
frame; the host's IPv6 stack receives it normally.

#### 3: Error Boundary — Truncated IPv4 (IHL=4) is dropped

A frame whose IP IHL field is 4 (would imply 16 bytes of IP header,
invalid). Sanity check 2 fails. Returns `XDP_DROP`.
`DROP_COUNTER[invalid_ip_header]` increments by 1. The packet does
NOT reach SERVICE_MAP lookup.

### UAT Scenarios (BDD)

#### Scenario: Valid frame passes the sanity prologue and reaches SERVICE_MAP

Given a TCP SYN to a registered VIP with valid Ethernet/IP/TCP headers
When the XDP program processes the frame
Then all five sanity checks pass
And SERVICE_MAP lookup proceeds normally

#### Scenario: Non-IPv4 frame falls through (not dropped)

Given an IPv6 frame
When the XDP program processes the frame
Then the action returned is `XDP_PASS`
And `DROP_COUNTER` does NOT increment

#### Scenario: Truncated IPv4 header is dropped with structured counter increment

Given a frame with invalid IPv4 IHL
When the XDP program processes the frame
Then the action returned is `XDP_DROP`
And `DROP_COUNTER[invalid_ip_header]` increments by 1
And SERVICE_MAP lookup is NOT performed

#### Scenario: Pathological TCP flag combination (SYN+RST) is dropped

Given a TCP frame with SYN and RST flags both set
When the XDP program processes the frame
Then the action returned is `XDP_DROP`
And `DROP_COUNTER[invalid_tcp_flags]` increments by 1

#### Scenario: Verifier complexity stays within budget after sanity checks

Given the program with the sanity prologue compiled in
When `veristat` is run
Then the program is accepted
And the instruction count delta vs the pre-sanity baseline is < 20%
And the absolute count remains ≤ 60% of the 1M-privileged ceiling

### Acceptance Criteria

- [ ] Sanity prologue prepended to `xdp_service_map_lookup` and `xdp_reverse_nat`; five checks in the Cloudflare order (EtherType → IP version+IHL → IP total_length → protocol → TCP flags)
- [ ] `DROP_COUNTER` declared as `BPF_MAP_TYPE_PERCPU_ARRAY` with one slot per drop-class enum; DESIGN finalises the exact enum and slot count (4-6 slots inclusive — variants for invalid IP-header, invalid IP total_length, invalid TCP flags, plus DESIGN-chosen residual slots; "non-IPv4" and "non-TCP/UDP" are explicitly NOT drop classes — they are pass-through and do NOT increment any counter)
- [ ] Drop classes are typed via a `DropClass` enum that maps 1:1 to slot indices; raw u32 indexing is not the call-site shape
- [ ] Tier 2 PKTGEN/SETUP/CHECK triptych per drop class: build the pathological frame, assert `XDP_DROP`, assert correct counter slot
- [ ] Tier 3 integration test sends a mixed batch (legitimate + each pathological class) and asserts counters per class
- [ ] `veristat` re-baselines: instruction count delta vs Slice 04 ≤ 20%; absolute count ≤ 60% of 1M ceiling
- [ ] DST invariant `SanityChecksFireBeforeServiceMap` (always: in any DST run, every observed packet that violates a sanity rule produces `XDP_DROP` AND no SERVICE_MAP lookup) lands and passes — `SimDataplane` mirrors the same prologue logic in its packet-shape simulator

### Outcome KPIs

- **Who**: Overdrive platform engineer + future operators
- **Does what**: trusts the dataplane to drop pathological traffic before SERVICE_MAP lookup happens, with operator-visible counters per drop class
- **By how much**: 100% of synthetic pathological frames are dropped at the prologue (Tier 3); per-class drop counter is correct on every Tier 2 test; verifier instruction-count delta vs Slice 04 baseline < 20% (Tier 4)
- **Measured by**: Tier 2 triptych per drop class; Tier 3 mixed-batch integration test; veristat baseline update
- **Baseline**: greenfield — Slice 02-05 have no sanity prologue; this slice creates the baseline

### Technical Notes

- The "non-IPv4 / non-TCP-or-UDP returns XDP_PASS" decision is intentional — it preserves IPv6 / ICMP / ARP semantics for the host's other workloads. The program is an LB, not a firewall; the firewall layer is #25 POLICY_MAP.
- TCP flag sanity rules are intentionally narrow (SYN+RST, SYN+FIN, all-zero). Aggressive flag-based filtering is operator-tunable and belongs in POLICY_MAP.
- `DROP_COUNTER` per-CPU semantics: aggregate by summing across CPUs in userspace. Per-CPU avoids the cross-CPU contention that a global counter would face under DDoS load (research §2.3).
- DESIGN may decide to fold the sanity prologue into a shared parsing helper that both `xdp_service_map_lookup` and `xdp_reverse_nat` invoke, OR duplicate the prologue inline in each program. Both are verifier-clean; the trade-off is between a `bpf_tail_call` or shared-include vs duplication. Either is acceptable for this slice.
- **Conntrack is OUT of scope** — sanity checks are stateless and have no flow memory.
- **Depends on**: US-04 (Maglev forward path) and US-05 (REVERSE_NAT). Sanity checks compose into both programs.

---

## US-07: Tier 4 perf gates + veristat baseline land on `main`

### Problem

`.claude/rules/testing.md` § Tier 4 commits to per-PR gates on
veristat instruction count (≤5% growth), `xdp-bench` pps (≤5%
regression), and `xdp-bench` p99 latency (≤10% regression),
measured against a baseline stored under `perf-baseline/main/`.
Phase 2.1 (#23) deferred this work to a future slice ("no point
baselining a no-op program"; ADR-0038 D8). Slices 02-06 have been
recording veristat numbers under `perf-baseline/main/` as side
effects but no PR-blocking gate enforces them yet.

This slice closes that gap: it lands the actual CI gate
(`cargo xtask verifier-regress` and `cargo xtask xdp-perf` filled
in from the Phase 2.1 stubs at `xtask/src/main.rs:588-594`),
captures the baseline numbers from the SERVICE_MAP / Maglev
program as it stands at the end of Slice 06, and enforces the
Tier 4 deltas on every subsequent PR.

This slice is the one that closes #24. After it lands, every
future Phase 2 PR (POLICY_MAP, IDENTITY_MAP, FS_POLICY_MAP,
conntrack #154, sockops, kTLS) is measured against a real,
enforced perf-gate floor — not against a TODO comment.

### Who

- Overdrive platform engineer wiring Phase 2 follow-on slices |
  motivated by needing a real perf-regression signal before adding
  more eBPF program complexity.
- CI maintainers | motivated by needing the perf-gate to be a
  pass/fail signal, not an "advisory data point."

### Solution

Fill in the two existing xtask stubs:
- `cargo xtask verifier-regress` runs `veristat` against every
  compiled BPF program, compares against
  `perf-baseline/main/veristat-{program}.txt`, fails the build if
  any program's instruction count exceeds its baseline by >5% or
  if any approaches the per-program complexity ceiling by >10%.
- `cargo xtask xdp-perf` runs `xdp-trafficgen` + `xdp-bench` (DROP
  + TX + LB-forward modes) against the loaded program inside the
  Lima VM, measures pps and p99 latency, compares against
  `perf-baseline/main/xdp-perf-{mode}.txt`, fails the build if pps
  drops by >5% or p99 latency rises by >10%.

Land the baseline numbers under
`perf-baseline/main/`:
`veristat-service-map.txt`, `veristat-reverse-nat.txt`,
`xdp-perf-drop.txt`, `xdp-perf-tx.txt`, `xdp-perf-lb-forward.txt`.

Wire both xtask subcommands into the per-PR CI workflow per
`.claude/rules/testing.md` § "CI topology" Job E.

Single-kernel in-host per #152. The kernel-matrix variant of these
gates is deferred to Phase 2's nested-VM slice.

### Domain Examples

#### 1: Happy Path — Subsequent PR adds 100 instructions to a 5000-instruction program; gate passes (2% growth, under threshold)

PR adds a small feature (e.g. extra logging counter). veristat
shows program grew from 5000 to 5100 instructions (2.0% growth).
Gate compares against the 5%-threshold; passes. PR can merge.

#### 2: Edge Case — Subsequent PR adds 600 instructions (12% growth); gate fails the build

PR introduces a new lookup path. veristat shows program grew from
5000 to 5600 instructions (12% growth). Gate fails the build with
structured output: "verifier-regress: program `xdp_service_map_lookup`
grew 12% (5000→5600); ceiling is 5% growth. Investigate."

#### 3: Error Boundary — perf gate measures 6% pps regression; build fails

PR refactors the parser. xdp-bench measures 4.7 Mpps on the LB-
forward path vs the baseline 5.0 Mpps. 6% regression > 5%
threshold. Gate fails. Author either reverts the regression or
updates the baseline with explicit justification (the latter
requires a follow-up PR with `perf-baseline/main/` updated and
the regression explained in the commit message).

### UAT Scenarios (BDD)

#### Scenario: veristat regression gate accepts a 2% instruction-count growth

Given the baseline veristat for `xdp_service_map_lookup` is 5000 instructions
And a PR causes the count to grow to 5100
When `cargo xtask verifier-regress` runs in CI
Then the gate passes
And CI summarizes "verifier-regress: PASS (2.0% growth, threshold 5%)"

#### Scenario: veristat regression gate fails on 12% growth

Given the baseline is 5000 instructions
And a PR causes the count to grow to 5600
When `cargo xtask verifier-regress` runs in CI
Then the gate fails the build
And the failure message names the program, both counts, and the threshold

#### Scenario: xdp-bench gate fails on 6% pps regression

Given the baseline LB-forward pps is 5.0 Mpps on the runner
And a PR's measured pps is 4.7 Mpps
When `cargo xtask xdp-perf` runs in CI
Then the gate fails the build
And the failure message reports both numbers and the relative delta

#### Scenario: Baseline update requires explicit justification in commit

Given a PR intentionally regresses pps to support a new feature
When the author updates `perf-baseline/main/xdp-perf-lb-forward.txt`
Then the commit message MUST include the regression rationale
And reviewers can grep for `perf-baseline` updates in PR diffs

#### Scenario: Single-kernel in-host execution per #152

Given the developer's local Lima VM (or CI's `ubuntu-latest` runner)
When `cargo xtask xdp-perf` runs
Then it does NOT spawn a nested VM
And it does NOT iterate over a kernel matrix
And it produces results against the host kernel only
And the deferral to a future kernel-matrix slice is documented in `wave-decisions.md`

### Acceptance Criteria

- [ ] `cargo xtask verifier-regress` is implemented (replaces the Phase 2.1 stub at `xtask/src/main.rs:588`); runs veristat against every compiled BPF program; fails on >5% instruction-count growth vs `perf-baseline/main/veristat-{program}.txt` or >10% approach to per-program complexity ceiling
- [ ] `cargo xtask xdp-perf` is implemented (replaces the Phase 2.1 stub at `xtask/src/main.rs:594`); runs `xdp-trafficgen` + `xdp-bench` in DROP / TX / LB-forward modes against the loaded program; fails on >5% pps regression or >10% p99 latency regression
- [ ] Baseline files under `perf-baseline/main/` for: `veristat-service-map.txt`, `veristat-reverse-nat.txt`, `xdp-perf-drop.txt`, `xdp-perf-tx.txt`, `xdp-perf-lb-forward.txt`
- [ ] Both xtask subcommands wired into the per-PR CI workflow per `.claude/rules/testing.md` § "CI topology" Job E
- [ ] Single-kernel in-host: `cargo xtask xdp-perf` runs inside the calling host's Lima VM (developer) or `ubuntu-latest` runner (CI); does NOT spawn a nested VM (per #152)
- [ ] Regression failures emit structured output naming program / metric / baseline / measured / threshold
- [ ] Baseline-update PRs MUST update `perf-baseline/main/` files with explicit commit-message justification; this is a documented contributor convention, not a mechanical check
- [ ] DST invariant `PerfBaselineGatesEnforced` (light, asserted by an xtask self-test): the xtask subcommand returns non-zero on a synthetic >5% regression input and zero on a synthetic 2% input — proving the gate logic itself works

### Outcome KPIs

- **Who**: Overdrive platform engineer + every future Phase 2 PR author + CI maintainers
- **Does what**: trusts the perf-gate to catch instruction-count growth and pps regressions before merge, on every PR
- **By how much**: 100% of PRs that breach the 5% / 10% / 5% thresholds fail CI (no false negatives); 0 false positives on representative PRs that don't actually regress (hand-validated for the first three Phase 2.3+ PRs after this lands); both xtask subcommands return well-formed structured output on every run
- **Measured by**: Per-PR CI logs + xtask self-test (the gate-logic-correctness invariant); first three follow-on Phase 2.3+ PRs hand-validated by the author for false-positive rate
- **Baseline**: greenfield — Phase 2.1 left these xtask subcommands stubbed; this slice is the first time they actually run in CI

### Technical Notes

- `xdp-bench` is provided by `xdp-tools` (Linux Foundation, MIT/Apache-2.0). The Lima image at `infra/lima/overdrive-dev.yaml` extends to install `xdp-tools` via apt or upstream binary; CI runs the same install.
- The runner-class variance in absolute pps numbers is the reason the gate is on RELATIVE delta only, not absolute thresholds. Per `.claude/rules/testing.md` § Tier 4: "Never gate on absolute numbers — runner hardware varies enough to make absolute gates flaky. Deltas only."
- Baseline updates are a known operational concern: when a slice deliberately changes program shape (e.g. landing a new map), the baseline must be updated in the same PR. CI does not auto-update baselines; this is intentional friction.
- The kernel-matrix variant (running these gates against 5.10 LTS through current LTS per `.claude/rules/testing.md` § Tier 3 / Tier 4) is OUT of scope per #152. This slice ships single-kernel; the matrix lands when #152 lands.
- DESIGN may decide to ship a separate `cargo xtask perf-baseline-update` helper to reduce update friction; not required for this slice.
- **Conntrack is OUT of scope** — same as every other slice; this slice only measures the stateless forwarder.
- **Depends on**: US-02 (SERVICE_MAP, baseline source #1), US-04 (Maglev, baseline source #2), US-05 (REVERSE_NAT, baseline source #3), US-06 (sanity checks, final baseline state).

---

## US-08: SERVICE_MAP hydrator reconciler converges Dataplane port

### Problem

Slices 02-06 fill in the body of `Dataplane::update_service` —
SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP / REVERSE_NAT_MAP / sanity
prologue — but nothing in those slices is the *consumer* that drives
`update_service` against real cluster state. Without a reconciler,
the port plumbing is exercised only by hand-written integration tests
and DST-direct calls; the §18 reference shape (sync `reconcile`,
typed `Action` emission, runtime-owned redb View persistence) is
never closed against a real dataplane port. J-PLAT-004 (reconciler
convergence — activated by this feature) requires the loop to close.

The hydrator reconciler — the consumer that watches `service_backends`
ObservationStore rows and converges the dataplane to match — is the
missing piece. It is also the first non-trivial use of the
`Reconciler` trait against a real dataplane port: every later slice
in Phase 2 (POLICY_MAP / IDENTITY_MAP / FS_POLICY_MAP, conntrack #154,
sockops, kTLS) will mirror this shape, so getting it right here is
load-bearing for the whole Phase 2 reconciler family.

### Who

- Overdrive platform engineer | motivated by the J-PLAT-004
  commitment — without a real reconciler closing the loop, every
  earlier slice's `Dataplane` port plumbing is untested against an
  ESR-shaped consumer.
- DST harness | the hydrator is the §18 reference implementation
  every later dataplane reconciler will mirror; the ESR pair
  (`HydratorEventuallyConverges`, `HydratorIdempotentSteadyState`)
  defines the bar every later reconciler in this family must clear.

> **Job trace**: `J-PLAT-001` (DST trust under real failure
> conditions — SimDataplane mirrors the real port semantics, so the
> reconciler's invariants run identically against both) +
> **`J-PLAT-004` (reconciler convergence)** — this is the first
> non-trivial use of the `Reconciler` trait against a real dataplane
> port and is the direct realisation of `J-PLAT-004` in this feature.

### Solution

Land a `ServiceMapHydrator` reconciler in
`crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs`
following the §18 / ADR-0035 / ADR-0036 contract:

- **`type State = ServiceMapHydratorState`** — typed projection
  carrying the desired backend set per `ServiceId` (from IntentStore
  / spec-derived projection thereof) and the actual backend set per
  `ServiceId` (from `service_backends` rows in ObservationStore).
- **`type View = ServiceMapHydratorView`** — typed memory persisted
  by the runtime via the `ViewStore` redb adapter. Carries
  per-service convergence inputs (last-seen `service_backends`
  generation, attempts since last successful `update_service`,
  `last_failure_seen_at` for retry-budget gating) — never derived
  state per "Persist inputs, not derived state."
- **`fn reconcile(...)`** — sync, no `.await`, no wall-clock reads
  inside the function (`tick.now` is the single snapshot the runtime
  passes in). Diffs `desired` against `actual`; emits one
  `Action::DataplaneUpdateService { service_id, backends }` per
  service whose backend set has drifted; returns the updated `View`.

The action shim translates the emitted `Action` into a call against
`Arc<dyn Dataplane>::update_service` — `EbpfDataplane` in production
(driving the real BPF map updates Slice 03 / Slice 04 implement),
`SimDataplane` under DST. The exact Action variant name and shape is
flagged for DESIGN; if a new Action variant is required vs reusing an
existing one, that is a DESIGN-time concern. The reconciler
contract is fixed regardless.

DST: two new invariants in `overdrive-sim::invariants`.
`HydratorEventuallyConverges` (eventual: from any combination of
`service_backends` rows and starting BPF map state, repeated
reconcile ticks drive `actual == desired` and hold there) and
`HydratorIdempotentSteadyState` (always: once `actual == desired`,
no further `Action` is emitted on subsequent ticks given unchanged
inputs).

### Domain Examples

#### 1: Happy Path — Service gains a backend; hydrator emits one update

Service `S1` is at `actual = {B1}`. A new `service_backends` row
appears with `S1, B2`. Next reconcile tick: `desired = {B1, B2}`,
`actual = {B1}` — diff non-empty. Hydrator emits
`Action::DataplaneUpdateService { service_id: S1, backends: {B1, B2} }`.
Action shim calls `update_service`. Following tick: `actual = {B1, B2}`,
`desired = {B1, B2}` — converged; no further action emitted.

#### 2: Edge Case — Transient failure; retry-budget gate honors backoff

`update_service` fails (kernel returns ENOMEM, e.g. memlock cap).
`actual` does not advance. Hydrator's View bumps `attempts` to 1 and
records `last_failure_seen_at = tick.now_unix`. Next tick:
`tick.now_unix < last_failure_seen_at + backoff_for_attempt(1)` —
hydrator emits no action (retry-budget gate), View unchanged.
Tick after the backoff window elapses: hydrator re-emits the
`DataplaneUpdateService` action; if successful, View resets attempts
to 0.

#### 3: Error Boundary — Stale ObservationStore; no spurious convergence

Service `S1` is at `desired = {B1, B2}`, `actual = {B1, B2}` —
converged. The `service_backends` ObservationStore row for `S1, B2`
disappears momentarily due to a transient gossip miss but reappears
on the next tick. Reconciler reads `desired = {B1}` on the
intermediate tick, emits `DataplaneUpdateService { S1, {B1} }`.
Subsequent tick: `desired = {B1, B2}` returns; reconciler emits
`DataplaneUpdateService { S1, {B1, B2} }` and converges. No spurious
intermediate state — every tick acts on the snapshot it sees.

### UAT Scenarios (BDD)

#### Scenario: Hydrator converges to the desired backend set when service_backends rows change

Given service `S1` has `actual = {B1}` in the dataplane and `service_backends` rows for `S1, B1`
And a new `service_backends` row appears for `S1, B2`
When the hydrator reconciler runs a tick
Then it emits `Action::DataplaneUpdateService` with `backends = {B1, B2}`
And the action shim calls `Dataplane::update_service` with that backend set
And the next tick observes `actual = desired` and emits no further action

#### Scenario: Hydrator is idempotent in steady state

Given a service whose `actual = desired = {B1, B2}`
When the hydrator reconciler runs ten consecutive ticks with no new `service_backends` row changes
Then no `Action::DataplaneUpdateService` is emitted on any of the ten ticks
And the View is unchanged across the ten ticks

#### Scenario: Hydrator honors retry budget after a failed update

Given a service whose `update_service` call returned `DataplaneError::MapAllocFailed`
And the hydrator's View carries `attempts = 1` and `last_failure_seen_at = T`
When the hydrator reconciler runs a tick at `tick.now_unix = T + (backoff_for_attempt(1) / 2)`
Then no action is emitted (within backoff window)
And View unchanged
When the hydrator reconciler runs a tick at `tick.now_unix = T + backoff_for_attempt(1)`
Then the `DataplaneUpdateService` action is re-emitted

#### Scenario: ESR — hydrator eventually converges from any starting state

Given a `SimDataplane` with arbitrary `actual` map state for service `S1`
And `service_backends` rows declaring an arbitrary `desired` set
When the hydrator runs reconcile ticks until quiescence
Then `actual == desired` for `S1`
And `HydratorEventuallyConverges` invariant holds across every DST seed

#### Scenario: Reconciler purity — no .await, no wall-clock, no DB handle inside reconcile

Given the `ServiceMapHydrator::reconcile` function body
When `dst-lint` scans it on every PR
Then no `.await` appears inside `reconcile`
And no direct `Instant::now()` / `SystemTime::now()` call appears inside `reconcile`
And no `IntentStore` / `ObservationStore` / `ViewStore` handle is held by the reconciler
And `tick.now` is the only wall-clock source used inside `reconcile`

### Acceptance Criteria

- [ ] `ServiceMapHydrator` reconciler lands in `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs` with `type State = ServiceMapHydratorState` and `type View = ServiceMapHydratorView` per ADR-0035 / ADR-0036
- [ ] `reconcile` is sync (no `async fn`, no `.await` inside the body); no direct wall-clock reads; no IntentStore / ObservationStore / ViewStore handle held by the reconciler — runtime owns hydration and View persistence end-to-end
- [ ] `ServiceMapHydratorView` is `Serialize + DeserializeOwned + Default + Clone + Send + Sync` and carries inputs only (`attempts`, `last_failure_seen_at`, last-seen `service_backends` generation per service) — no derived deadlines or pre-computed backoff windows persisted
- [ ] Hydrator emits `Action::DataplaneUpdateService` (or DESIGN-chosen Action variant) per service whose backend set has drifted; the action shim dispatches against `Arc<dyn Dataplane>` so production wiring uses `EbpfDataplane`, DST uses `SimDataplane`
- [ ] DST invariant `HydratorEventuallyConverges` lands in `overdrive-sim::invariants` and passes — eventual: from any seeded combination of `service_backends` rows and starting `SimDataplane` state, repeated ticks drive `actual == desired`
- [ ] DST invariant `HydratorIdempotentSteadyState` lands in `overdrive-sim::invariants` and passes — always: once `actual == desired`, no further action is emitted on subsequent ticks given unchanged inputs
- [ ] `ReconcilerIsPure` invariant continues to pass with `ServiceMapHydrator` added to the catalogue
- [ ] Tier 1 (DST) is the primary test surface; Tier 2 / Tier 3 secondary — the hydrator is exercised end-to-end by Slice 02 / 03 / 04's existing integration tests because `EbpfDataplane::update_service` is the call site the action shim drives

### Outcome KPIs

- **Who**: Overdrive platform engineer + DST harness (§18 / J-PLAT-004 reference implementation that every later dataplane reconciler will mirror)
- **Does what**: trusts the reconciler runtime to converge `Dataplane::update_service` against `service_backends` ObservationStore rows under ESR — the first non-trivial use of the `Reconciler` trait against a real dataplane port
- **By how much**: 100% pass rate of `HydratorEventuallyConverges` and `HydratorIdempotentSteadyState` invariants across every DST seed on every PR; `ReconcilerIsPure` continues to pass with the hydrator added; 0 `.await` / wall-clock / DB-handle violations in `reconcile` (`dst-lint` clean)
- **Measured by**: DST invariant pass rate per PR (`cargo xtask dst`); `dst-lint` gate on `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs`
- **Baseline**: greenfield — no prior dataplane reconciler exists; this is the §18 reference implementation J-PLAT-004 activates on

### Technical Notes

- The hydrator does NOT need to know about HASH_OF_MAPS / Maglev / atomic-swap mechanics — it calls `Dataplane::update_service(service_id, backends)` and the `EbpfDataplane` impl owns the swap (Slice 03's 5-step shape). This is the structural reason the hydrator can land in parallel with Slices 03-06.
- Cross-node propagation (a hydrator on Node A reacting to `service_backends` rows written by Node B) is OUT of scope — Phase 2.2 is single-node per #156 (`[5.20]`); the hydrator runs against locally-readable observation rows only.
- The exact `Action` variant name and shape is flagged for DESIGN. If `Action::DataplaneUpdateService { service_id, backends }` is added as a new variant, the existing action-shim dispatch table extends; if an existing variant fits, no schema change. Per `wave-decisions.md` § "What is NOT being decided in this wave."
- **Conntrack is OUT of scope** — the hydrator drives the stateless Maglev forwarder; flow affinity is the ≤1% Maglev guarantee. Conntrack is #154.
- **Depends on**: US-02 (provides `Dataplane::update_service` body the hydrator drives). Can land in parallel with US-03 / US-04 / US-05 / US-06 because none of those slices are upstream of the hydrator's reconcile contract — they are downstream effects of the same `update_service` call.

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial seven user stories for `phase-2-xdp-service-map` DISCUSS wave (lean shape: no JTBD, no Journey design — feature is dataplane infrastructure consumed by reconciler runtime via `Dataplane` port). Trace J-PLAT-001 (DST trust) and J-PLAT-004 (newly activated). |
| 2026-05-05 | Eclipse-review remediation: added US-08 (SERVICE_MAP hydrator reconciler) restoring the user-ratified IN-scope decision; reconciled US-02 Who and KPI Who from "shipped later" to "shipped in this feature"; added explicit `J-PLAT-004` job-ID citation in US-02 Who and US-08 Who; corrected US-04 disruption-bound phrasing to ≤2% total (1% forced + ≤1% incidental) per Maglev's published guarantee; added REVERSE_NAT_MAP endianness-lockstep note in US-05 Tech Notes; dropped DropClass placeholder slot in US-06 AC. |
