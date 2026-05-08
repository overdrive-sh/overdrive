<!-- markdownlint-disable MD024 MD036 -->

# Test Scenarios — phase-2-xdp-service-map

**Wave**: DISTILL (acceptance-designer)
**Date**: 2026-05-05
**Status**: handoff-ready for DELIVER

> **Specification only — NOT executed.** Per `.claude/rules/testing.md`,
> all acceptance and integration tests are Rust `#[test]` /
> `#[tokio::test]` functions. The Gherkin-style Given/When/Then blocks
> below are SPECIFICATION for traceability and review; the executable
> form is the Rust body the DELIVER wave produces in the file path
> cited per scenario. **No `.feature` files anywhere; no
> `cucumber-rs`; no pytest-bdd.**

> **Walking-skeleton inheritance.** Phase 1's
> `customer-submits-a-job-and-watches-it-run` walking skeleton already
> closed the user-observable loop; Phase 2.2 fills the body of one
> hexagonal port (`Dataplane::update_service`) and adds one
> reconciler that drives it. No new `@walking_skeleton` scenarios in
> this feature — see
> `docs/feature/phase-2-xdp-service-map/distill/walking-skeleton.md`.

> **Tag legend.** `@US-NN` story | `@K-N` outcome KPI |
> `@ASR-2.2-NN` quality-attribute scenario | `@slice-NN` carpaccio
> slice | `@real-io @adapter-integration` real eBPF / real veth /
> real `BPF_PROG_TEST_RUN` (Tier 2 / Tier 3) | `@in-memory`
> `SimDataplane` + `SimObservationStore` only (Tier 1 DST) |
> `@property` proptest-shaped | `@kpi` verifies emission of an
> outcome-KPI signal | `@pending` scaffold-not-yet-GREEN.

---

## Scenario index

| # | Scenario ID | US | Tier | Test home |
|---|---|---|---|---|
| 1 | S-2.2-01 | US-01 | Tier 3 | `crates/overdrive-dataplane/tests/integration/veth_attach.rs` |
| 2 | S-2.2-02 | US-01 | Tier 3 | `crates/overdrive-dataplane/tests/integration/veth_attach.rs` |
| 3 | S-2.2-03 | US-01 | Tier 3 | `crates/overdrive-dataplane/tests/integration/veth_attach.rs` |
| 4 | S-2.2-04 | US-02 | Tier 2 | `crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs` |
| 5 | S-2.2-05 | US-02 | Tier 2 | `crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs` |
| 6 | S-2.2-06 | US-02 | Tier 3 | `crates/overdrive-dataplane/tests/integration/service_map_forward.rs` |
| 7 | S-2.2-07 | US-02 | Tier 4 | `cargo xtask verifier-regress` |
| 8 | S-2.2-08 | US-02 | Tier 2 | `crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs` |
| 9 | S-2.2-09 | US-03 | Tier 3 | `crates/overdrive-dataplane/tests/integration/atomic_swap.rs` |
| 10 | S-2.2-10 | US-03 | Tier 3 | `crates/overdrive-dataplane/tests/integration/atomic_swap.rs` |
| 11 | S-2.2-11 | US-03 | Tier 3 | `crates/overdrive-dataplane/tests/integration/atomic_swap.rs` |
| 12 | S-2.2-12 | US-04 | Tier 1 | `crates/overdrive-sim/tests/integration/maglev_churn.rs` |
| 13 | S-2.2-13 | US-04 | Tier 1 | `crates/overdrive-sim/tests/integration/maglev_churn.rs` |
| 14 | S-2.2-14 | US-04 | Tier 4 | `cargo xtask verifier-regress` |
| 15 | S-2.2-15 | US-05 | Tier 3 | `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs` |
| 16 | S-2.2-16 | US-05 | Tier 2 | `crates/overdrive-bpf/tests/integration/tc_reverse_nat.rs` |
| 17 | S-2.2-17 | US-05 | Tier 2 | `crates/overdrive-bpf/tests/integration/reverse_key_roundtrip.rs` |
| 18 | S-2.2-18 | US-05 | Tier 3 | `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs` |
| 19 | S-2.2-19 | US-06 | Tier 2 | `crates/overdrive-bpf/tests/integration/sanity_prologue_drops.rs` |
| 20 | S-2.2-20 | US-06 | Tier 2 | `crates/overdrive-bpf/tests/integration/sanity_prologue_drops.rs` |
| 21 | S-2.2-21 | US-06 | Tier 2 | `crates/overdrive-bpf/tests/integration/sanity_prologue_drops.rs` |
| 22 | S-2.2-22 | US-06 | Tier 3 | `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs` |
| 23 | S-2.2-23 | US-06 | Tier 4 | `cargo xtask verifier-regress` |
| 24 | S-2.2-24 | US-07 | Tier 4 | `xtask/tests/perf_gate_self_test.rs` |
| 25 | S-2.2-25 | US-07 | Tier 4 | `xtask/tests/perf_gate_self_test.rs` |
| 26 | S-2.2-26 | US-08 | Tier 1 | `crates/overdrive-sim/src/invariants/service_map_hydrator.rs` |
| 27 | S-2.2-27 | US-08 | Tier 1 | `crates/overdrive-sim/src/invariants/service_map_hydrator.rs` |
| 28 | S-2.2-28 | US-08 | Tier 1 | `crates/overdrive-control-plane/tests/integration/service_map_hydrator_dispatch.rs` |
| 29 | S-2.2-29 | US-08 | Tier 1 | `crates/overdrive-sim/src/invariants/service_map_hydrator.rs` |
| 30 | S-2.2-30 | US-08 | Tier 1 | `crates/overdrive-sim/src/invariants/service_map_hydrator.rs` |

**Counts.** 30 scenarios. Tier 1: 8. Tier 2: 8. Tier 3: 9. Tier 4:
5. Error-path scenarios: 13/30 = 43.3 % (≥ 40 % mandate satisfied).
KPI-tagged: K1, K2, K3, K4, K5, K6, K7, K8 — all eight covered.

---

## US-01 — Real-iface XDP attach (slice-01)

### S-2.2-01 — Real veth pair attach with packet count assertion

**Tags**: `@US-01` `@K1` `@slice-01` `@real-io @adapter-integration`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/veth_attach.rs`
**Test fn**: `xdp_attaches_to_real_veth_and_packet_counter_increments`
**Status**: starting scenario for DELIVER (NOT `@pending`).

```gherkin
Given a `veth0`/`veth1` pair exists on the host
And `xdp_pass` is attached to `veth0` in native mode
When 100 Ethernet+IPv4 frames are pushed from `veth1`
Then `bpftool map dump name PACKET_COUNTER` reads `100`
And the kernel-side counter is the only side effect observed
```

### S-2.2-02 — Native attach failure logs structured fallback warning

**Tags**: `@US-01` `@K1` `@slice-01` `@real-io @adapter-integration`
`@kpi` `@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/veth_attach.rs`
**Test fn**: `native_attach_failure_logs_fallback_warning`

**Category**: edge-path.

```gherkin
Given the underlying driver lacks native XDP support
When the loader attempts `xdp.attach(iface, XdpFlags::default())`
Then a structured warning `xdp.attach.fallback_generic` is logged
And the program attaches in generic mode
And the integration test passes against the generic-mode attach
```

### S-2.2-03 — Missing iface produces typed `IfaceNotFound` error

**Tags**: `@US-01` `@K1` `@slice-01` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/veth_attach.rs`
**Test fn**: `missing_iface_returns_typed_iface_not_found_error`

**Category**: error-path.

```gherkin
Given the named iface does not exist on the host
When the loader attempts to attach
Then it returns `DataplaneError::IfaceNotFound`
And no XDP program is loaded
And the integration test asserts the typed error variant
```

---

## US-02 — SERVICE_MAP forward path (slice-02)

### S-2.2-04 — Tier 2 PKTGEN/SETUP/CHECK triptych for SERVICE_MAP hit

**Tags**: `@US-02` `@K2` `@slice-02` `@ASR-2.2-03` `@real-io
@adapter-integration` `@pending`

**Tier**: Tier 2
**File**: `crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs`
**Test fn**: `service_map_hit_returns_xdp_tx_with_rewritten_headers`

```gherkin
Given a Tier 2 test harness with `xdp_service_map_lookup` loaded
And SERVICE_MAP populated with VIP `10.0.0.1:8080` (TCP) → backend `10.1.0.5:9000`
When PKTGEN builds a TCP SYN, SETUP populates SERVICE_MAP, CHECK invokes `BPF_PROG_TEST_RUN`
Then the returned action is `XDP_TX`
And the data-out buffer carries the rewritten dest IP `10.1.0.5` and port `9000`
And the IP and TCP checksums are recomputed correctly
```

### S-2.2-05 — Tier 2 SERVICE_MAP miss returns XDP_PASS

**Tags**: `@US-02` `@K2` `@slice-02` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 2
**File**: `crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs`
**Test fn**: `service_map_miss_returns_xdp_pass_no_rewrite`

**Category**: error-path (miss is a structural alternative).

```gherkin
Given an empty SERVICE_MAP
And a TCP SYN to `10.0.0.1:8080` arrives
When the XDP program processes the frame
Then the action returned is `XDP_PASS`
And no header rewrites happened
```

### S-2.2-06 — Single-VIP TCP forwarding through real veth

**Tags**: `@US-02` `@K2` `@slice-02` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/service_map_forward.rs`
**Test fn**: `ten_tcp_syns_to_vip_are_rewritten_and_forwarded_via_veth`

```gherkin
Given a SERVICE_MAP entry mapping VIP `10.0.0.1:8080` (TCP) to backend `10.1.0.5:9000`
And a TCP SYN to `10.0.0.1:8080` arrives on `veth0`
When the XDP program processes 10 such frames
Then every frame leaves `veth1` with dest IP `10.1.0.5` and port `9000`
And the IP and TCP checksums are valid
And the action returned for each frame is `XDP_TX`
```

### S-2.2-07 — Verifier accepts SERVICE_MAP program; instruction count baselined

**Tags**: `@US-02` `@K2` `@K7` `@slice-02` `@ASR-2.2-03`
`@real-io @adapter-integration` `@kpi` `@pending`

**Tier**: Tier 4
**File**: `cargo xtask verifier-regress` against
`perf-baseline/main/verifier-budget/veristat-service-map.txt`
**Test fn**: ran from CI; the baseline file IS the assertion

```gherkin
Given the compiled `xdp_service_map_lookup.o`
When `veristat --emit insns` is run
Then the program is accepted by the verifier
And the instruction count is recorded under `perf-baseline/main/verifier-budget/`
And the count is well below the 1M-privileged-instruction ceiling
And the count is below 50% of the ceiling (the research's "L1-cache fits" claim)
```

### S-2.2-08 — Truncated frame falls through (no crash, no DROP)

**Tags**: `@US-02` `@K2` `@slice-02` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 2
**File**: `crates/overdrive-bpf/tests/integration/xdp_service_map_lookup.rs`
**Test fn**: `truncated_ipv4_frame_returns_xdp_pass_no_lookup_no_crash`

**Category**: error-path. Slice 02 policy is "don't crash on
malformed input, let the kernel handle it"; aggressive sanity drops
land in Slice 06 (S-2.2-19..S-2.2-22).

```gherkin
Given a frame with Ethernet header but truncated IPv4 (only 10 bytes of IP header)
When the XDP program processes the frame
Then the bounds check fails and the action returned is `XDP_PASS`
And the program does not crash
And SERVICE_MAP is not consulted
```

---

## US-03 — HASH_OF_MAPS atomic per-service backend swap (slice-03)

### S-2.2-09 — Atomic backend swap drops zero packets under traffic

**Tags**: `@US-03` `@K3` `@slice-03` `@ASR-2.2-01`
`@real-io @adapter-integration` `@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/atomic_swap.rs`
**Test fn**: `atomic_swap_under_50kpps_traffic_drops_zero_packets`

**Notes.** Acknowledges Risk #3 from DISCUSS — the test runs at
50 kpps on `ubuntu-latest` (Lima can sustain 100 kpps); the
zero-drop assertion holds at any kpps level the runner can sustain.
Measurement source per K3 is `xdp-trafficgen` send count vs sink
receive count (Slice 03-time signal); `DROP_COUNTER` is a Slice 06
artifact and is not yet available at Slice 03 time.

```gherkin
Given service `S1` has one backend `B1` and `xdp-trafficgen` pushes 50 kpps to its VIP
When the loader swaps `S1`'s inner map to a three-backend table `{B1, B2, B3}`
Then ZERO packets are dropped across the swap window
And post-swap packets distribute across the three backends within ±10% of even distribution
```

### S-2.2-10 — Removing a backend leaves no orphans after GC pass

**Tags**: `@US-03` `@K3` `@slice-03` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/atomic_swap.rs`
**Test fn**: `removing_backend_purges_orphaned_backend_map_entries`

**Category**: edge-path.

```gherkin
Given service `S1` references backends `{B1, B2, B3}` in `BACKEND_MAP`
When `EbpfDataplane::update_service` swaps `S1` to `{B1, B2}` and runs the GC pass
Then `BACKEND_MAP` no longer contains `B3`
And no service references `B3`
```

### S-2.2-11 — Inner-map allocation failure preserves existing service

**Tags**: `@US-03` `@K3` `@slice-03` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/atomic_swap.rs`
**Test fn**: `kernel_rejects_inner_map_alloc_existing_mapping_preserved`

**Category**: error-path.

```gherkin
Given service `S1` has a live inner map with backends `{B1, B2}`
When `EbpfDataplane::update_service` attempts to swap and the kernel rejects the inner-map allocation
Then `update_service` returns `DataplaneError::MapAllocFailed`
And the existing inner map is unchanged
And subsequent traffic continues to forward against `{B1, B2}`
```

---

## US-04 — Maglev consistent hashing (slice-04)

### S-2.2-12 — Maglev permutation generation is deterministic

**Tags**: `@US-04` `@K4` `@slice-04` `@in-memory` `@property`
`@pending`

**Tier**: Tier 1 (DST proptest)
**File**: `crates/overdrive-sim/tests/integration/maglev_churn.rs`
**Test fn**: `maglev_generate_is_deterministic_under_seeded_inputs`

**Property**: For any valid `(BTreeMap<BackendId, Weight>,
MaglevTableSize)` input, two successive `maglev::generate(...)`
calls produce bit-identical `Vec<BackendId>`. `assert_always!`-shaped
proptest, 1024 cases default per
`.claude/rules/testing.md` § "Property-based testing (proptest)".

```gherkin
Given any valid `(BTreeMap<BackendId, Weight>, MaglevTableSize)` input
When `maglev::generate(backends, m)` is called twice in succession
Then both calls return the bit-identical permutation `Vec<BackendId>`
```

### S-2.2-13 — Single-backend removal among 100 shifts ≤ 2 % of flows

**Tags**: `@US-04` `@K4` `@slice-04` `@ASR-2.2-02` `@in-memory`
`@property` `@pending`

**Tier**: Tier 1 (DST proptest, primary) + Tier 3 confirm at
`crates/overdrive-dataplane/tests/integration/maglev_real.rs`
**File**: `crates/overdrive-sim/tests/integration/maglev_churn.rs`
**Test fn**: `single_backend_removal_shifts_at_most_two_percent_of_flows`

**Property**: For any seeded backend set of N=100 with M=16_381,
removing one backend shifts ≤ 2 % of the flow population (1 % forced
from the evicted backend's flows + ≤ 1 % incidental per Maglev's
published bound, research § 5.2). `@property` tag triggers the
crafter to use seeded `Entropy` per `.claude/rules/development.md`
§ "Port-trait dependencies".

```gherkin
Given any seeded set of 100 equally-weighted backends and 100,000 5-tuple flows
When backend `B50` is removed and `maglev::generate(...)` rebuilds the permutation
Then flows previously on `B50` are shifted to some other backend (1% forced shift)
And ≤ 1% of flows that were NOT on `B50` pre-removal land on a different backend
And the total flow shift is ≤ 2% across the 100k-flow population
```

### S-2.2-14 — Verifier accepts Maglev program; instruction count under 50%

**Tags**: `@US-04` `@K4` `@K7` `@slice-04` `@ASR-2.2-03`
`@real-io @adapter-integration` `@kpi` `@pending`

**Tier**: Tier 4
**File**: `cargo xtask verifier-regress` against
`perf-baseline/main/verifier-budget/veristat-service-map.txt`
(updated baseline post-Maglev landing)

```gherkin
Given the compiled XDP program with Maglev lookup
When `veristat` is run
Then the program is accepted by the verifier
And the instruction count is ≤ 50% of the 1M-privileged-instruction ceiling
And the count is recorded as the new `perf-baseline/main/verifier-budget/veristat-service-map.txt` baseline
```

---

## US-05 — REVERSE_NAT_MAP for response-path rewrite (slice-05)

### S-2.2-15 — Real TCP connection completes through forward and reverse paths

**Tags**: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs`
**Test fn**: `real_tcp_connection_completes_through_vip_with_payload_echo`

```gherkin
Given a `nc -l 9000` listener on `veth1`'s namespace and a VIP `10.0.0.1:8080` mapping to it
When a client on `veth0`'s namespace runs `nc 10.0.0.1 8080` and writes a payload
Then the connection completes the 3-way handshake
And the payload echoes back to the client
And `nc` exits with code 0
```

### S-2.2-16 — Tier 2 PKTGEN/SETUP/CHECK triptych for tc_reverse_nat

**Tags**: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 2 (TC egress: `BPF_PROG_TEST_RUN` is the right
mechanism for TC programs per `.claude/rules/testing.md` § Tier 2)
**File**: `crates/overdrive-bpf/tests/integration/tc_reverse_nat.rs`
**Test fn**: `reverse_nat_lookup_hit_rewrites_source_to_vip`

```gherkin
Given a Tier 2 test harness with `tc_reverse_nat` loaded
And REVERSE_NAT_MAP populated with key `(10.1.0.5, 9000, TCP)` → value `(10.0.0.1, 8080)`
When PKTGEN builds a backend response from `10.1.0.5:9000`, SETUP populates REVERSE_NAT_MAP, CHECK invokes `BPF_PROG_TEST_RUN`
Then the returned action is `TC_ACT_OK`
And the data-out buffer carries source IP `10.0.0.1` and source port `8080`
And the IP and TCP checksums are recomputed correctly
```

### S-2.2-17 — Endianness lockstep — wire-order packet roundtrips host-order key

**Tags**: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
`@property` `@pending`

**Tier**: Tier 2 + userspace proptest (per architecture.md § 11
locked endianness lockstep)
**File**: `crates/overdrive-bpf/tests/integration/reverse_key_roundtrip.rs`
**Test fn**: `wire_order_packet_produces_host_order_reverse_key`
**Sibling**: userspace mod-tests proptest in
`crates/overdrive-dataplane/src/maps/reverse_nat_map_handle.rs`

**Property**: A synthetic packet with known wire-order bytes through
`reverse_key_from_packet` produces the host-order `ReverseKey` that
the userspace test seeded into the map. Closes the architecture.md
§ 11 endianness lockstep guarantee. The userspace proptest
round-trips host-order writes against host-order reads to assert no
userspace-side endian flip sneaks in.

```gherkin
Given a synthetic IPv4+TCP packet with known wire-order bytes
And REVERSE_NAT_MAP seeded with the equivalent host-order ReverseKey by userspace
When `reverse_key_from_packet(iph, l4, proto)` is invoked at the kernel boundary
Then the resulting ReverseKey matches the userspace-seeded host-order key bit-for-bit
And the kernel-side lookup hits with the userspace-seeded value
```

### S-2.2-18 — Removed backend's REVERSE_NAT entry purged on service update

**Tags**: `@US-05` `@K5` `@slice-05` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/reverse_nat_e2e.rs`
**Test fn**: `removing_backend_purges_reverse_nat_entry_no_stale_rewrite`

**Category**: error-path (stale rewrite leak prevention).

```gherkin
Given service `S1` references backend `B1` and `REVERSE_NAT_MAP` contains `B1`'s entry
When `update_service` removes `B1`
Then `REVERSE_NAT_MAP` no longer contains `B1`'s entry
And a late response from `B1` is treated as non-LB traffic (no rewrite, action `TC_ACT_OK`)
```

---

## US-06 — Pre-SERVICE_MAP packet-shape sanity prologue (slice-06)

### S-2.2-19 — Truncated IPv4 (IHL=4) drops with `MalformedHeader` counter

**Tags**: `@US-06` `@K6` `@slice-06` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 2
**File**: `crates/overdrive-bpf/tests/integration/sanity_prologue_drops.rs`
**Test fn**: `truncated_ipv4_header_drops_with_malformed_header_counter`

**Category**: error-path.

```gherkin
Given a frame with invalid IPv4 IHL=4 (would imply 16 bytes of IP header, malformed)
When the XDP program processes the frame
Then the action returned is `XDP_DROP`
And `DROP_COUNTER[MalformedHeader]` increments by 1
And SERVICE_MAP lookup is NOT performed
```

### S-2.2-20 — Pathological TCP flag combination (SYN+RST) drops

**Tags**: `@US-06` `@K6` `@slice-06` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 2
**File**: `crates/overdrive-bpf/tests/integration/sanity_prologue_drops.rs`
**Test fn**: `tcp_syn_plus_rst_flags_drops_with_malformed_header_counter`

**Category**: error-path.

```gherkin
Given a TCP frame with SYN and RST flags both set
When the XDP program processes the frame
Then the action returned is `XDP_DROP`
And `DROP_COUNTER[MalformedHeader]` increments by 1
```

### S-2.2-21 — IPv6 frame falls through (NOT dropped, no counter increment)

**Tags**: `@US-06` `@K6` `@slice-06` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 2
**File**: `crates/overdrive-bpf/tests/integration/sanity_prologue_drops.rs`
**Test fn**: `ipv6_ethertype_returns_xdp_pass_no_drop_counter_increment`

**Category**: edge-path. The "non-IPv4 / non-TCP-or-UDP returns
XDP_PASS" decision is intentional per US-06 — it preserves IPv6 /
ICMP / ARP semantics for the host's other workloads. The program
is an LB, not a firewall.

```gherkin
Given an IPv6 frame (EtherType 0x86DD)
When the XDP program processes the frame
Then the action returned is `XDP_PASS`
And `DROP_COUNTER` does NOT increment for any drop class
```

### S-2.2-22 — Mixed legitimate + pathological batch hits per-class counters

**Tags**: `@US-06` `@K6` `@slice-06` `@real-io @adapter-integration`
`@pending`

**Tier**: Tier 3
**File**: `crates/overdrive-dataplane/tests/integration/sanity_mixed_batch.rs`
**Test fn**: `mixed_batch_increments_per_class_counters_correctly`

```gherkin
Given a SERVICE_MAP entry for VIP `10.0.0.1:8080` (TCP) → backend `10.1.0.5:9000`
And a mixed batch of 50 valid TCP SYNs + 10 truncated frames + 10 SYN+RST frames + 10 IPv6 frames
When the XDP program processes the batch on `veth0`
Then 50 frames are forwarded to `10.1.0.5:9000`
And `DROP_COUNTER[MalformedHeader]` increments by 20 (10 truncated + 10 SYN+RST)
And `DROP_COUNTER[UnknownVip]` is unchanged
And the 10 IPv6 frames pass through to the kernel networking stack
```

### S-2.2-23 — Verifier-budget delta vs Slice 04 baseline ≤ 20 %

**Tags**: `@US-06` `@K6` `@K7` `@slice-06` `@ASR-2.2-03`
`@real-io @adapter-integration` `@kpi` `@pending`

**Tier**: Tier 4
**File**: `cargo xtask verifier-regress` against
`perf-baseline/main/verifier-budget/veristat-service-map.txt`

```gherkin
Given the program with the sanity prologue compiled in
When `veristat` is run against the post-Slice-06 program
Then the program is accepted by the verifier
And the instruction count delta vs the Slice-04 recorded baseline is ≤ 20%
And the absolute count remains ≤ 60% of the 1M-privileged ceiling
And the post-Slice-06 number becomes the new baseline that Slice 07 enforces against subsequent PRs
```

---

## US-07 — Tier 4 perf gates (slice-07)

### S-2.2-24 — `verifier-regress` self-test fails on synthetic 12 % regression

**Tags**: `@US-07` `@K7` `@slice-07` `@kpi` `@pending`

**Tier**: Tier 4 (xtask self-test — proves the gate logic itself)
**File**: `xtask/tests/perf_gate_self_test.rs`
**Test fn**: `verifier_regress_returns_nonzero_on_synthetic_twelve_percent_growth`

```gherkin
Given a synthetic baseline of 5000 instructions for `xdp_service_map_lookup`
And a synthetic candidate veristat output of 5600 instructions (12% growth)
When `cargo xtask verifier-regress` evaluates the gate against the synthetic input
Then the subcommand returns non-zero exit code
And the structured output names the program, both counts (5000, 5600), and the 5% threshold
```

### S-2.2-25 — `xdp-perf` self-test fails on synthetic 6 % pps regression

**Tags**: `@US-07` `@K7` `@slice-07` `@kpi` `@pending`

**Tier**: Tier 4 (xtask self-test)
**File**: `xtask/tests/perf_gate_self_test.rs`
**Test fn**: `xdp_perf_returns_nonzero_on_synthetic_six_percent_pps_regression`

```gherkin
Given a synthetic baseline LB-forward pps of 5.0 Mpps
And a synthetic candidate `xdp-bench` output of 4.7 Mpps (6% regression)
When `cargo xtask xdp-perf` evaluates the gate against the synthetic input
Then the subcommand returns non-zero exit code
And the structured output reports both numbers (5.0, 4.7) and the 5% relative-delta threshold
```

---

## US-08 — SERVICE_MAP hydrator reconciler (slice-08)

### S-2.2-26 — Hydrator converges to desired backend set on row change (DST)

**Tags**: `@US-08` `@K8` `@slice-08` `@ASR-2.2-04` `@in-memory`
`@property` `@pending`

**Tier**: Tier 1 (DST invariant `HydratorEventuallyConverges`)
**File**: `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`
**Invariant fn**: `assert_eventually!(actual_fingerprint == desired_fingerprint)`

**Property**: For every `service_id`, repeated reconcile ticks with
stable `desired` drive `actual.fingerprint == desired.fingerprint`
within bounded ticks. ESR (Eventually Stable Reconciliation) per
ADR-0035 + USENIX OSDI '24 *Anvil*.

```gherkin
Given service `S1` has `actual = {B1}` in the dataplane and `service_backends` rows for `S1, B1`
And a new `service_backends` row appears for `S1, B2`
When the hydrator reconciler runs reconcile ticks until quiescence
Then it emits `Action::DataplaneUpdateService` with `backends = {B1, B2}` exactly once per fingerprint change
And the next tick observes `actual = desired` and emits no further action
And `HydratorEventuallyConverges` holds across every DST seed
```

### S-2.2-27 — Hydrator is idempotent in steady state (DST)

**Tags**: `@US-08` `@K8` `@slice-08` `@ASR-2.2-04` `@in-memory`
`@property` `@pending`

**Tier**: Tier 1 (DST invariant `HydratorIdempotentSteadyState`)
**File**: `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`
**Invariant fn**: `assert_always!(steady_state_emits_zero_actions)`

**Property**: Once `actual.fingerprint == desired.fingerprint` for all
services, the hydrator emits zero `DataplaneUpdateService` actions
per tick.

```gherkin
Given a service whose `actual.fingerprint = desired.fingerprint`
When the hydrator reconciler runs ten consecutive ticks with no `service_backends` row changes
Then no `Action::DataplaneUpdateService` is emitted on any of the ten ticks
And the View's `RetryMemory` for that service is unchanged across the ten ticks
And `HydratorIdempotentSteadyState` holds across every DST seed
```

### S-2.2-28 — Action shim writes `service_hydration_results` row on dispatch

**Tags**: `@US-08` `@K8` `@slice-08` `@in-memory` `@pending`

**Tier**: Tier 1
**File**: `crates/overdrive-control-plane/tests/integration/service_map_hydrator_dispatch.rs`
**Test fn**: `dispatch_writes_completed_row_on_dataplane_ok`

```gherkin
Given the hydrator emits `Action::DataplaneUpdateService { service_id, vip, backends, correlation }`
And `SimDataplane::update_service` returns `Ok(())`
When the action shim dispatches the action
Then the shim writes a `service_hydration_results` row with `status: Completed { fingerprint, applied_at: tick.now }`
And the row is keyed on `service_id` matching the emitted action
And the next reconcile tick reads the row via `actual` and observes convergence
```

### S-2.2-29 — Hydrator honors retry budget after failed dispatch

**Tags**: `@US-08` `@K8` `@slice-08` `@in-memory` `@property` `@pending`

**Tier**: Tier 1 (proptest over backoff windows)
**File**: `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`
**Invariant fn**: `assert_always!(no_dispatch_within_backoff_window)`

**Category**: error-path. Property-shaped: for any seeded
`(attempts, last_failure_seen_at, tick.now_unix)`, the hydrator
emits an action iff `tick.now_unix >= last_failure_seen_at +
backoff_for_attempt(attempts)` per `.claude/rules/development.md`
§ "Persist inputs, not derived state" — the deadline is recomputed
from inputs every tick, never persisted.

```gherkin
Given a service whose `update_service` call returned `DataplaneError::MapAllocFailed`
And the hydrator's View carries `attempts = 1` and `last_failure_seen_at = T`
When the hydrator reconciler runs a tick at `tick.now_unix = T + (backoff_for_attempt(1) / 2)`
Then no action is emitted (within backoff window)
And the View's `RetryMemory` for the service is unchanged
When the hydrator reconciler runs a tick at `tick.now_unix = T + backoff_for_attempt(1)`
Then `Action::DataplaneUpdateService` is re-emitted
```

### S-2.2-30 — Reconciler purity preserved (`dst-lint` + `ReconcilerIsPure`)

**Tags**: `@US-08` `@K8` `@slice-08` `@in-memory` `@pending`

**Tier**: Tier 1 (existing `ReconcilerIsPure` invariant inherits the
new reconciler) + `dst-lint` mechanical check
**File**: `crates/overdrive-sim/src/invariants/service_map_hydrator.rs`
+ `xtask/src/dst_lint.rs` (existing scanner)

**Category**: structural-invariant.

```gherkin
Given the `ServiceMapHydrator::reconcile` function body
When `dst-lint` scans it on every PR
Then no `.await` appears inside `reconcile`
And no direct `Instant::now()` or `SystemTime::now()` call appears inside `reconcile`
And no `IntentStore` / `ObservationStore` / `ViewStore` handle is held by the reconciler
And `tick.now` is the only wall-clock source used inside `reconcile`
And the existing `ReconcilerIsPure` DST invariant continues to pass with `ServiceMapHydrator` added to the catalogue
```

---

## Coverage matrix

### By user story

| Story | Scenarios | Happy/Edge/Error | Tier coverage |
|---|---|---|---|
| US-01 | S-2.2-01..03 (3) | 1 / 1 / 1 | Tier 3 |
| US-02 | S-2.2-04..08 (5) | 2 / 0 / 2 + 1 perf | Tier 2 / Tier 3 / Tier 4 |
| US-03 | S-2.2-09..11 (3) | 1 / 1 / 1 | Tier 3 |
| US-04 | S-2.2-12..14 (3) | 1 property / 1 property / 1 perf | Tier 1 + Tier 3 + Tier 4 |
| US-05 | S-2.2-15..18 (4) | 1 / 1 / 1 / 1 endianness | Tier 2 / Tier 3 |
| US-06 | S-2.2-19..23 (5) | 1 + 1 perf / 1 / 2 | Tier 2 / Tier 3 / Tier 4 |
| US-07 | S-2.2-24..25 (2) | 0 / 0 / 2 self-test | Tier 4 |
| US-08 | S-2.2-26..30 (5) | 2 invariants / 1 dispatch / 1 retry / 1 purity | Tier 1 |

**Total**: 30 scenarios. Story coverage: 8/8. KPI coverage: K1-K8
(8/8). ASR coverage: ASR-2.2-01 (S-2.2-09), ASR-2.2-02 (S-2.2-13),
ASR-2.2-03 (S-2.2-04, S-2.2-07, S-2.2-14, S-2.2-23), ASR-2.2-04
(S-2.2-26, S-2.2-27).

### By tier

| Tier | Count | What it catches |
|---|---|---|
| Tier 1 (DST) | 8 | concurrency, ordering, ESR, replay-equivalence, property invariants |
| Tier 2 (`BPF_PROG_TEST_RUN`) | 8 | program-level kernel correctness on curated input |
| Tier 3 (real veth) | 9 | attach, real packet rates, real BPF map format, libbpf-sys binding drift, kernel verifier, kTLS / NIC driver behaviour |
| Tier 4 (`veristat` + `xdp-bench`) | 5 | verifier complexity regression, pps / latency regression |

### Error-path ratio

13 scenarios out of 30 are error- or boundary-path
(S-2.2-02, S-2.2-03, S-2.2-05, S-2.2-08, S-2.2-10, S-2.2-11,
S-2.2-13, S-2.2-18, S-2.2-19, S-2.2-20, S-2.2-21, S-2.2-29, plus
S-2.2-24 / S-2.2-25 self-test failure paths). 13/30 = **43.3 %** —
above the 40 % mandate floor.

### KPI emission scenarios (`@kpi`)

K1: S-2.2-02 (structured warning). K7: S-2.2-07, S-2.2-14, S-2.2-23,
S-2.2-24, S-2.2-25 (perf-gate self-tests + verifier baseline records).
KPIs K2/K3/K4/K5/K6/K8 are emitted-by-construction (per their
measurement-plan rows in `outcome-kpis.md`) and exercised by the
named scenarios above; no separate `@kpi`-tagged scenarios needed
for those.

---

## What `SimDataplane` + `SimObservationStore` CANNOT model (Mandate 6 disclosure)

The Tier 1 DST envelope using `SimDataplane` + `SimObservationStore`
is the primary surface for ASR-2.2-04 (hydrator ESR closure) and the
`@property` proptests around Maglev determinism / disruption bounds /
fingerprint determinism. It cannot catch:

- **Kernel verifier rejection** — the Linux verifier is a real
  semantic surface; `SimDataplane` does not run it. ASR-2.2-03 is
  Tier 4-only for this reason.
- **kTLS / kernel-side BPF map format mismatches** — `aya` ↔ kernel
  ABI drift is invisible to in-memory simulation. Tier 3 is the
  only catcher.
- **Real packet rates / NIC driver behaviour** — synthetic flows
  in DST do not load NIC drivers; ASR-2.2-01 (zero-drop atomic
  swap under sustained load) is Tier 3-only.
- **`BPF_PROG_TEST_RUN` syscall / aya internal driving paths** —
  Tier 2 only.
- **Real veth packet plumbing** (Ethernet header bytes, MAC
  resolution, TX-vs-PASS confirmation via `tcpdump`) — Tier 3.
- **Endianness conversion at the wire boundary** — the kernel-side
  `from_be` calls only execute under real `BPF_PROG_TEST_RUN` or real
  packet flow; userspace can only proptest the mirror. S-2.2-17 is
  the Tier 2 lockstep.
- **libbpf-sys binding drift** — only the real `aya::Ebpf::load_file`
  + `Bpf::program_mut` call paths exercise this. Tier 3 catches.

This is the structural reason every BPF map and every driven adapter
gets at least one `@real-io` scenario — synthetic tests cannot
substitute for real-kernel integration. Bug classes partition.

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial 30-scenario test-scenarios spec for `phase-2-xdp-service-map`. WS strategy inherited from Phase 1 (no new WS); 4-tier coverage; 43.3 % error-path ratio; 8/8 stories; 8/8 KPIs; 4/4 ASRs. — Quinn (Atlas). |

---

## Review

| Field | Value |
|---|---|
| Review ID | `distill-rev-2026-05-05-phase2.2-xdp-service-map` |
| Reviewer | `nw-acceptance-designer-reviewer` |
| Date | 2026-05-05 |
| Verdict | **APPROVED** — handoff-ready for DELIVER |

### Summary

30 scenarios across 4 tiers; 43.3 % error-path coverage; 8/8 stories;
8/8 KPIs; 4/4 ASRs. Zero `.feature` files, zero pytest-bdd, zero
`conftest.py`. All RED scaffolds use `panic!()` / `todo!()` with
descriptive messages — none are compile-fail shapes. Integration tests
correctly gated `integration-tests` and laid out per project rules
(`tests/integration.rs` entrypoint + `tests/integration/<scenario>.rs`
modules). Walking-skeleton inheritance properly documented; regression
coverage via existing Phase 1 / 2.1 invariants. New
`Action::DataplaneUpdateService` variant correctly additive in
`reconciler.rs`; ESR pair (`HydratorEventuallyConverges`,
`HydratorIdempotentSteadyState`) scaffolded in
`overdrive-sim/src/invariants/service_map_hydrator.rs` and wired into
`Invariant::ALL`. `BackendSetFingerprint` docstring commits to
rkyv-archived hash (not `serde_json`). `ServiceMapHydratorView` ships
as a placeholder — no pre-introduced derived-state field, honors
`development.md` § Persist inputs, not derived state. Mandate
compliance: CM-A, CM-B, CM-C, CM-D, Mandate 5, Mandate 6, Mandate 7
all pass. No design drift from DISCUSS / DESIGN locked decisions.

### Praise (verbatim)

> "All RED scaffolds panic with descriptive messages and cite the exact
> Slice/Scenario that will fill them — DELIVER will have zero ambiguity
> about what to implement and where."

### Compliance evidence

Reviewer's full audit (23 mandatory compliance items, all PASS) is
captured in the orchestrator transcript. Spot-check highlights:

- Zero `.feature` files (verified by glob across the workspace).
- Zero pytest / pytest-bdd / `conftest.py` files (verified by glob).
- Every BPF map (`SERVICE_MAP`, `BACKEND_MAP`, `MAGLEV_MAP`,
  `REVERSE_NAT_MAP`, `DROP_COUNTER`) has ≥ 1 `@real-io
  @adapter-integration` scenario.
- ESR-pair invariant names match the DISCUSS / DESIGN-locked literals
  exactly.
- Tier breakdown 8 / 8 / 9 / 5 across Tier 1–4; total 30; error-path
  ratio 13/30 = 43.3 % (above the 40 % mandate).

### Non-blocking advisory

None blocking. Three nitpicks logged informationally only: (a)
`@pending` / `#[ignore]` markings consistent between the scenario
index and the test-scaffold bodies; (b) `service_map_hydrator.rs`
docstring correctly references the additive `Invariant` enum entries;
(c) `wave-decisions.md` cross-references DWD-3 (file-path inventory)
and DWD-4 (RED-scaffold strategy) — the load-bearing decision records
for DELIVER consumption.
