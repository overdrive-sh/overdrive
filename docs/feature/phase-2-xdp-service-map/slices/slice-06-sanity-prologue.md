# Slice 06 — Pre-SERVICE_MAP packet-shape sanity checks

**Story**: US-06
**Backbone activity**: 6 (Drop pathological traffic)
**Effort**: 1 day
**Depends on**: Slice 04 (Maglev forward path) and Slice 05 (REVERSE_NAT). Sanity checks compose into both programs.
**Parallel-able with**: Slice 05.

## Outcome

A sanity-check prologue is prepended to both `xdp_service_map_lookup`
and `xdp_reverse_nat`. Five Cloudflare-order checks (research § 7.2):

1. EtherType is IPv4 (`0x0800`) — non-IPv4 returns `XDP_PASS` (let
   the kernel handle ARP, IPv6, etc.).
2. IP version is 4 and IHL ≥ 5 (20 bytes) — invalid headers return
   `XDP_DROP`.
3. IP `total_length` sanity (≥ IHL\*4, ≤ packet length).
4. Transport protocol is TCP (`6`) or UDP (`17`) — others return
   `XDP_PASS`.
5. For TCP: flag combination is not nonsense (no SYN+RST, no SYN+FIN,
   no all-zero) — invalid combinations return `XDP_DROP`.

A new `DROP_COUNTER` (`BPF_MAP_TYPE_PERCPU_ARRAY`, one slot per
drop class) records drops per class — operator-visible signal for
"how much pathological traffic are we seeing." A typed `DropClass`
enum in `overdrive-core` maps 1:1 to slot indices; raw u32 indexing
is not the call-site shape. Tier 2 PKTGEN/SETUP/CHECK triptych per
drop class. Tier 3 mixed-batch test sends legitimate + each
pathological class; asserts per-class counter. Tier 4 veristat
re-baseline (instruction-count delta vs Slice 04 < 20%; absolute ≤
60% of 1M ceiling).

New DST invariant `SanityChecksFireBeforeServiceMap` (always: in any
DST run, every observed packet that violates a sanity rule produces
`XDP_DROP` AND no SERVICE_MAP lookup) — `SimDataplane` mirrors the
prologue logic in its packet-shape simulator.

## Value hypothesis

*If* the static sanity checks don't quantify their verifier cost
(delta vs Slice 04 baseline), we don't know whether to budget for
them in future slices or fold them into POLICY_MAP / #25's compile-on-
rule-change shape. *Conversely*, if delta < 20% and absolute remains
comfortable, the Phase 2 verifier-budget plan stays on track and
operator-tunable POLICY_MAP rules can land later without revisiting
the static prologue.

## Disproves (named pre-commitment)

- **"Sanity checks are free (verifier-wise)."** No — every check
  costs branches the verifier walks; the slice quantifies how much.
- **"Operator-tunable rules belong in this slice."** No — that's
  POLICY_MAP / #25, with materially different mechanics.

## Scope (in)

- 5-check sanity prologue (EtherType → IP version+IHL → IP total_length → protocol → TCP flags).
- Sanity prologue prepended to BOTH `xdp_service_map_lookup` and `xdp_reverse_nat`.
- `DROP_COUNTER` (`BPF_MAP_TYPE_PERCPU_ARRAY`) keyed by drop-class slot.
- `DropClass` typed enum in `overdrive-core` mapping 1:1 to slot indices.
- DST invariant `SanityChecksFireBeforeServiceMap`.
- Tier 2 PKTGEN/SETUP/CHECK per drop class.
- Tier 3 mixed-batch integration test.
- Tier 4 veristat re-baseline.

## Scope (out)

- Operator-tunable DDoS rules (POLICY_MAP / #25 — different mechanics).
- Perf gates (Slice 07).
- Conntrack (#154).
- IPv6 sanity rules (future Phase 2 slice).
- Kernel matrix (#152).

## Target KPI

- 100% of synthetic pathological frames are dropped at the prologue (Tier 3).
- Per-class drop counter is correct on every Tier 2 test.
- Verifier instruction-count delta vs Slice 04 baseline < 20%.
- Absolute instruction count ≤ 60% of 1M-privileged ceiling.

## Acceptance flavour

See US-06 scenarios. Focus: per-class drop assertion; verifier delta
budget; lockstep insertion in both forward and reverse XDP programs.

## Failure modes to defend

- Verifier delta exceeds 20% budget: slice-redesign signal. Fold the
  prologue into a `bpf_tail_call` shared helper (research § 8.2's
  Cilium pattern) instead of inline duplication.
- Cross-CPU contention on global counter: prevented by per-CPU
  array; aggregate by summing across CPUs in userspace. Research §
  2.3.
- Drop class slot index drift between kernel and userspace: typed
  `DropClass` enum + 1:1 slot mapping prevents this.

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — sanity prologue + DROP_COUNTER + DropClass enum + sanity DST invariant (4) |
| No hypothetical abstractions landing later | PASS — ships static prologue both XDP programs invoke; operator-tunable layer is explicitly OUT (POLICY_MAP / #25) |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — Tier 2 per drop class + Tier 3 mixed-batch + Tier 4 veristat delta budget |
| Demonstrable in single session | PASS — Tier 2 + Tier 3 + veristat run on developer's Lima VM in single session |
| Same-day dogfood moment | PASS — Linux developer iterates the prologue, watches the per-class counter increment |
