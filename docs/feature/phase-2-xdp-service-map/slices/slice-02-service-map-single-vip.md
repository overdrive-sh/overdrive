# Slice 02 — SERVICE_MAP forward path with single hardcoded backend

**Story**: US-02
**Backbone activity**: 2 (Forward to a backend)
**Effort**: 1 day
**Depends on**: Slice 01 (real-iface attach), Phase 2.1 scaffolding.

## Outcome

A new XDP program `xdp_service_map_lookup` lands in
`crates/overdrive-bpf/src/programs/service_map.rs`. It parses
Ethernet + IPv4 + TCP/UDP headers, looks up a VIP-keyed entry in
`SERVICE_MAP` (`BPF_MAP_TYPE_HASH`, key `ServiceKey { vip, vport,
proto }`, value `Backend { ip, port }`), rewrites the destination IP
and port on a hit, recomputes IP + TCP/UDP checksums, returns
`XDP_TX`. On miss returns `XDP_PASS`. `EbpfDataplane::update_service`
graduates from no-op stub to real single-entry insert/remove via a
typed `ServiceMapHandle` newtype in `overdrive-dataplane` that wraps
the aya `HashMap`. Tier 2 PKTGEN/SETUP/CHECK triptych asserts
verifier acceptance + correct rewrite. Tier 3 veth integration test
forwards 10 TCP SYNs end-to-end with `tcpdump` capture. Tier 4
veristat baseline recorded under
`perf-baseline/main/veristat-service-map.txt` (Slice 07 enforces it).

## Value hypothesis

*If* the basic SERVICE_MAP shape (parse + lookup + rewrite + checksum
+ XDP_TX) doesn't fit the verifier budget cleanly when written in
aya-rs, every later slice building on top is uncertain. *Conversely*,
if the baseline veristat is well under 50% of the 1M-privileged
ceiling, Maglev (Slice 04) can land without verifier-budget anxiety.

## Disproves (named pre-commitment)

- **"We need to write the program in C to fit the verifier budget."**
  No — research § 5.4: Cilium and Katran fit equivalent in their
  datapath; aya-rs through LLVM produces equivalent bytecode.
- **"The single-VIP slice is too thin."** No — it's the smallest
  possible verifier-clean lookup-and-rewrite, the precondition for
  every later slice.

## Scope (in)

- `xdp_service_map_lookup` XDP program (Eth + IPv4 + TCP/UDP parse + lookup + rewrite + checksum + `XDP_TX`).
- `SERVICE_MAP` declared as `BPF_MAP_TYPE_HASH` keyed by `ServiceKey`, value `Backend`.
- STRICT newtypes `ServiceVip`, `ServiceId` in `overdrive-core` (FromStr / Display / serde / rkyv / proptest).
- `ServiceMapHandle` typed newtype in `overdrive-dataplane` wrapping aya `HashMap`; exposes `insert(ServiceVip, Backend)` / `remove(ServiceVip)` only — raw aya HashMap not visible at call site.
- `EbpfDataplane::update_service` real implementation for single-entry insert/remove.
- Tier 2 PKTGEN/SETUP/CHECK triptych in `crates/overdrive-bpf/tests/integration/service_map_test_run.rs`.
- Tier 3 veth integration test sending 10 TCP SYNs end-to-end with tcpdump capture.
- Tier 4 baseline `perf-baseline/main/veristat-service-map.txt` (recorded; not yet gated — Slice 07 lights up the gate).

## Scope (out)

- Multiple backends per service (Slice 03).
- Maglev consistent hashing (Slice 04).
- REVERSE_NAT / return-path rewrite (Slice 05).
- Sanity prologue / packet-shape sanity (Slice 06).
- Tier 4 perf-gate enforcement (Slice 07 wires the gate).
- IPv6, ICMP (future Phase 2 slice).
- Conntrack (#154).

## Target KPI

- 100% of Tier 2 SERVICE_MAP-hit packets return `XDP_TX` with valid checksums.
- 100% of misses return `XDP_PASS`.
- veristat instruction count ≤ 50% of 1M-privileged ceiling.
- Tier 3 veth test forwards all 10 frames end-to-end (tcpdump capture confirms rewrite).

## Acceptance flavour

See US-02 scenarios. Focus: Tier 2 triptych asserts action + rewrite;
Tier 3 veth test forwards real TCP SYNs; Tier 4 veristat captured.

## Failure modes to defend

- Truncated frame: returns `XDP_PASS` (sanity is Slice 06; this slice's policy is "don't crash on malformed input").
- UDP packet to TCP-keyed service: SERVICE_MAP miss → `XDP_PASS`.
- Missing SERVICE_MAP entry: miss → `XDP_PASS`.
- Endianness mismatch at userspace boundary: `ServiceMapHandle` constructor takes host-endian inputs, converts to network-endian when writing to map. Proptest covers roundtrip.

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — `xdp_service_map_lookup` + `SERVICE_MAP` decl + `ServiceMapHandle` newtype + `ServiceVip` newtype (4) |
| No hypothetical abstractions landing later | PASS — extends Phase 2.1's `EbpfDataplane` and existing `Dataplane` port trait |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — Tier 2 PROG_TEST_RUN + Tier 3 veth integration with real TCP SYNs |
| Demonstrable in single session | PASS — `cargo xtask lima run -- cargo xtask bpf-unit && cargo xtask lima run -- cargo nextest run -p overdrive-dataplane --features integration-tests` |
| Same-day dogfood moment | PASS — Linux developer iterates Tier 2 / Tier 3 in Lima VM |
