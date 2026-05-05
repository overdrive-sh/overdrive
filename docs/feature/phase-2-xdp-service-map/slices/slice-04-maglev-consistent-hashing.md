# Slice 04 — Maglev consistent hashing inside MAGLEV_MAP

**Story**: US-04
**Backbone activity**: 4 (Distribute consistently)
**Effort**: 1.5 days (acknowledged at this size given component count: full `MaglevTableSize` newtype discipline + weighted multiplicity expansion + Eisenbud permutation + lookup switch + veristat baseline + 2 DST invariants. Splitting would create two near-duplicate slices that fail the carpaccio taste test, so this stays as one slice with a 1.5-day budget per `wave-decisions.md` Risk #2 / Decision 4.)
**Depends on**: Slice 03 (HASH_OF_MAPS atomic-swap shape).

## Outcome

A new `MAGLEV_MAP` (`BPF_MAP_TYPE_HASH_OF_MAPS`, inner
`BPF_MAP_TYPE_ARRAY` of size `M=16381` — Cilium default per research
§ 5.2) lands in `overdrive-bpf`. STRICT `MaglevTableSize` newtype in
`overdrive-core` is constrained to the Cilium prime list `{251, 509,
1021, 2039, 4093, 8191, 16381, 32749, 65521, 131071}`; `FromStr`
rejects non-prime / out-of-list values with structured `ParseError`.
A pure synchronous function `maglev::generate(&BTreeMap<BackendId,
Weight>, MaglevTableSize) -> Vec<BackendId>` ships in
`overdrive-dataplane` (or a dedicated `overdrive-maglev` crate —
DESIGN picks); inputs iterated in BTreeMap order so the produced
permutation is bit-identical across runs and across nodes.

The XDP program lookup chain switches from Slice 03's random slot
selection to MAGLEV_MAP-indexed: hash 5-tuple → modulo `M` →
MAGLEV_MAP lookup → BackendId → BACKEND_MAP → backend address →
DNAT-rewrite + `XDP_TX`. Weighted-Maglev variant (Eisenbud per-
backend slot multiplicity proportional to weight) ships in this
slice — research § 5.3 establishes there is no engineering-time
saving in landing vanilla Maglev first, then weighted later.

`EbpfDataplane::update_service` regenerates the Maglev table on
every backend-set change, prepares a fresh inner MAGLEV_MAP, and
atomically swaps it via the Slice 03 5-step swap shape.

Tier 3 integration test validates Maglev's incidental-disruption
property: 100 backends + 100k synthetic 5-tuple flows; remove one
backend and assert ≤2% total flow shift — 1% forced (B50's evicted
flows must land somewhere) + ≤1% incidental (Maglev's published bound
on flows that were NOT on B50 pre-removal). Tier 4 veristat baseline
updates
— the post-Maglev instruction count is the new
`perf-baseline/main/veristat-service-map.txt`. Two new DST
invariants: `MaglevDistributionEven` (eventual: even distribution
within ± 5%) and `MaglevDeterministic` (always: identical inputs
produce bit-identical permutation).

## Value hypothesis

*If* Maglev's lookup-table-driven indexing pattern doesn't fit
comfortably under the verifier complexity ceiling when written in
aya-rs, the § 15 weighted-canary claim either ships without
consistent-hashing affinity or pushes verifier budget into red.
*Conversely*, if it does — and the disruption proptest proves ≤2%
total flow shift (1% forced + ≤1% incidental) — every later slice
(POLICY_MAP, IDENTITY_MAP, conntrack #154) inherits a known-clean
veristat budget headroom.

## Disproves (named pre-commitment)

- **"Vanilla Maglev is enough; weighted Maglev is a follow-on."**
  No — research § 5.3: weighted-Maglev's algorithmic delta is in
  userspace permutation generation; verifier delta is negligible.
  Splitting saves no engineering time and ships an incomplete § 15
  commitment.
- **"Maglev is too verifier-expensive for aya-rs."** No, per
  research § 5.4; the slice produces the empirical disproof.

## Scope (in)

- `MAGLEV_MAP` declared as `BPF_MAP_TYPE_HASH_OF_MAPS`; inner `BPF_MAP_TYPE_ARRAY` of size `M=16381` (default).
- STRICT `MaglevTableSize` newtype in `overdrive-core` constrained to Cilium prime list.
- Pure `maglev::generate(&BTreeMap<BackendId, Weight>, MaglevTableSize) -> Vec<BackendId>` in `overdrive-dataplane`.
- Weighted-Maglev variant (per-backend slot multiplicity proportional to weight).
- XDP program lookup switches from Slice 03's random slot to MAGLEV_MAP-indexed.
- `EbpfDataplane::update_service` regenerates Maglev table on every backend-set change, swaps via Slice 03 shape.
- Proptest: `maglev::generate` determinism (same inputs → bit-identical output).
- Proptest: ± 5% even distribution under equal weights; ± 2% honoring of declared weights under skewed weights.
- DST invariants `MaglevDistributionEven`, `MaglevDeterministic`.
- Tier 3 disruption test (100 backends, 100k flows, remove one, assert ≤2% total flow shift = 1% forced + ≤1% incidental per Maglev's published bound).
- Tier 4 veristat baseline update.

## Scope (out)

- REVERSE_NAT (Slice 05).
- Sanity prologue (Slice 06).
- Perf gates (Slice 07 enforces the baseline).
- Conntrack-based flow pinning (#154).
- Kernel matrix (#152).

## Target KPI

- ≤2% total flow shift on single-backend removal among 100 backends — 1% forced (B50's evicted flows) + ≤1% incidental per Maglev's published bound.
- ± 5% even-distribution under equally-weighted backends.
- ± 2% honoring of declared weights under 95/5 canary distribution.
- veristat instruction count ≤ 50% of 1M-privileged ceiling.

## Acceptance flavour

See US-04 scenarios. Focus: disruption-bound proptest; weighted-
distribution proptest; veristat baseline update; deterministic
generation invariant.

## Failure modes to defend

- M not in Cilium prime list: `MaglevTableSize::from_str` rejects
  with structured `ParseError`; type system prevents construction
  via constructor.
- Weight overflow during multiplicity calculation: saturating
  arithmetic; proptest covers boundary inputs.
- Verifier rejects MAGLEV_MAP lookup pattern: slice's Tier 2 test
  catches this at PR time; `veristat` produces structured failure
  output.

## Slice taste-test

| Test | Status |
|---|---|
| ≤ 4 new components | PASS — MAGLEV_MAP + `maglev::generate` + `MaglevTableSize` newtype + XDP-side lookup switch (4) |
| No hypothetical abstractions landing later | PASS — extends Slice 03's HASH_OF_MAPS shape; Eisenbud NSDI 2016 algorithm is published |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — Tier 3 disruption test + Tier 4 veristat against real verifier |
| Demonstrable in single session | PASS — proptest + Tier 3 test on developer's Lima VM |
| Same-day dogfood moment | PASS — Linux developer iterates Maglev table generation + XDP-side lookup |
