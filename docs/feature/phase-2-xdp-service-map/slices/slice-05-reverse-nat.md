# Slice 05 — REVERSE_NAT_MAP for response-path rewrite

**Story**: US-05
**Backbone activity**: 5 (Close the return path)
**Effort**: 1 day
**Depends on**: Slice 04 (Maglev forward path provides the SERVICE_MAP entries that REVERSE_NAT mirrors).
**Parallel-able with**: Slice 06.

## Outcome

A new `REVERSE_NAT_MAP` (`BPF_MAP_TYPE_HASH`, key `BackendKey {
backend_ip, backend_port, proto }`, value `Vip { ip, port }`) lands
in `overdrive-bpf` — the third map of the Cilium three-map split
(research § 2.1). A new XDP program `xdp_reverse_nat` (or TC-egress
equivalent — DESIGN picks; aya 0.13 supports both cleanly) ships in
`crates/overdrive-bpf/src/programs/reverse_nat.rs`. The egress
program parses Eth + IPv4 + TCP/UDP, looks up the backend's source
5-tuple in REVERSE_NAT_MAP, on hit rewrites the source IP/port back
to the VIP and recomputes checksums, returns `XDP_TX` (or
`TC_ACT_OK`). On miss returns `XDP_PASS` / `TC_ACT_OK` (not LB
traffic; pass-through).

`EbpfDataplane::update_service` writes / removes REVERSE_NAT_MAP
entries in lockstep with service-backend changes — when service S1
references backend B1 = 10.1.0.5:9000 for VIP 10.0.0.1:8080, the
reverse map gets `(10.1.0.5, 9000, TCP) → (10.0.0.1, 8080)`. Removed
backends remove their reverse entries. The Slice 03 5-step swap
extends to the third map.

Tier 3 integration test runs a real `nc -l 9000` listener as backend
on `veth1`'s namespace and a `nc 10.0.0.1 8080` client on `veth0`'s
namespace; asserts the connection completes 3-way handshake +
payload + close cleanly. New DST invariant `ReverseNatLockstep`
(always: every forward-path SERVICE_MAP entry has a matching
REVERSE_NAT_MAP entry; removing a backend purges both).

## Value hypothesis

*If* REVERSE_NAT can't be added without entangling the existing
forward-path program, the program shape is wrong and every later
slice carries that confusion. *Conversely*, if the egress program is
a clean independent module that shares only the parsing helpers,
every later slice has a known-good template for additional egress
logic (sockops integration, conntrack lookups, etc.).

## Disproves (named pre-commitment)

- **"Forward and return paths can share one program."** No, per
  Cilium reference; splitting keeps each program small, verifier-
  clean, independently veristat-able.
- **"REVERSE_NAT is a Phase 3 concern."** No — without it, the LB is
  functionally inert against any real client/backend pair.

## Scope (in)

- `REVERSE_NAT_MAP` declared as `BPF_MAP_TYPE_HASH` in `overdrive-bpf`; key `BackendKey { ip, port, proto }`, value `Vip { ip, port }`.
- `xdp_reverse_nat` program (XDP egress or TC egress — DESIGN picks).
- `EbpfDataplane::update_service` writes / removes REVERSE_NAT entries in lockstep with service-backend changes.
- DST invariant `ReverseNatLockstep` in `overdrive-sim::invariants`.
- Tier 2 PKTGEN/SETUP/CHECK triptych for `xdp_reverse_nat`.
- Tier 3 real `nc` end-to-end TCP integration test across veth.
- `SimDataplane` REVERSE_NAT_MAP mirror.

## Scope (out)

- Sanity prologue (Slice 06; can run in parallel with this slice).
- Perf gates (Slice 07).
- Cross-node REVERSE_NAT (deferred to Phase 5.20 — see #156; intrinsically a multi-node concern with no observable behaviour in a single-node cluster, deferred alongside HA + Corrosion-driven map hydration + multi-node consensus).
- Conntrack-based reverse-NAT optimisation (#154).
- Kernel matrix (#152).

## Target KPI

- 100% of Tier 3 `nc` connection-completion runs succeed.
- 100% `ReverseNatLockstep` invariant pass rate on every PR.
- 0 stale REVERSE_NAT entries after backend removal.

## Acceptance flavour

See US-05 scenarios. Focus: real bidirectional TCP via `nc`; lockstep
invariant; non-LB-source fall-through.

## Failure modes to defend

- Asymmetric routing: test setup runs forward + reverse on the same
  veth pair / same node so the rewrite is observable. Cross-node
  asymmetric routing is a future slice.
- Stale REVERSE_NAT entry from a removed backend: lockstep invariant
  catches this in DST; integration test asserts post-update
  REVERSE_NAT_MAP size matches live-backend count.
- Non-LB traffic from the same backend host: lookup misses; falls
  through with `XDP_PASS` / `TC_ACT_OK`. No source-address rewrite
  applied.

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — REVERSE_NAT_MAP + `xdp_reverse_nat` + lockstep update path + lockstep DST invariant (4) |
| No hypothetical abstractions landing later | PASS — extends Slice 04's three-map split with the third Cilium-reference map |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — real `nc` end-to-end TCP across veth |
| Demonstrable in single session | PASS — `cargo xtask lima run -- cargo nextest run --features integration-tests` plus manual `nc` curl-style verification |
| Same-day dogfood moment | PASS — Linux developer watches a real TCP connection complete |
