# DISCUSS Wave Decisions — phase-2-xdp-service-map

**Wave**: DISCUSS (product-owner)
**Owner**: Luna
**Date**: 2026-05-05
**Status**: COMPLETE — handoff-ready for DESIGN (solution-architect),
pending peer review.

---

## Lean shape — what ran, what was skipped, why

This DISCUSS ran in **lean shape**: Phase 1 (JTBD discovery) and Phase
2 (Journey design + Gherkin) of the `nw-product-owner` workflow were
deliberately skipped. Phase 2.5 (User Story Mapping + Carpaccio
slicing) was the highest-value part for this feature and was run in
full. Phase 3 (User stories + AC + DoR + outcome KPIs) was run lightly
— stories trace to existing jobs rather than re-derive new ones.

### Why lean

- **Motivation is locked by the whitepaper.** § 7 (eBPF Dataplane), §
  15 (Zero Downtime Deployments — atomic backend swaps), § 19
  (Security Model — DDoS baseline posture) collectively define WHY
  this feature exists. JTBD discovery would re-derive what is already
  the canonical SSOT.
- **No competing jobs.** `docs/product/jobs.yaml` already carries
  `J-PLAT-001` (DST trust under real failure conditions) and
  `J-PLAT-004` (reconciler convergence — activated by this feature
  from `deferred` → `active`). Both are explicit motivations the
  whitepaper grounds.
- **No end-user UX.** This feature is dataplane infrastructure. The
  consumer is the reconciler runtime via the `Dataplane` port trait
  (`crates/overdrive-core/src/traits/dataplane.rs`). There is no
  emotional arc, no `${variable}` flow between human-facing steps —
  producing journey-*.yaml / journey-*-visual.md / journey-*.feature
  / shared-artifacts-registry.md would be ceremony.
- **Phase 2.1 precedent.** `phase-2-aya-rs-scaffolding` (#23) skipped
  DISCUSS entirely — pure scaffolding feature, no DISCUSS artifacts
  exist for it. This feature is the next-step infrastructure layer
  with the same lack-of-UX shape; the lean-DISCUSS choice here is
  the middle ground between Phase 2.1's "no DISCUSS" and Phase 1's
  "full DISCUSS with journey artifacts."

### What was skipped

- **No `discover/` directory** — DISCOVER did not run. Motivation is
  whitepaper-locked.
- **No `diverge/` directory** — DIVERGE did not run. No design
  alternatives at the requirements level (algorithm + map-shape
  alternatives are research-locked: Cilium three-map split + weighted
  Maglev are the production-validated choices per
  `docs/research/networking/xdp-service-load-balancing-research.md`).
- **No `journey-*.yaml`, `journey-*-visual.md`, `journey-*.feature`,
  `shared-artifacts-registry.md`** — Phase 2 of the skill workflow
  is skipped. The consumer is the runtime, not a human persona; the
  data flowing through `Dataplane::update_service` is structured
  (typed `ServiceVip`, `BackendId`, `Backend`), not user-facing
  shared artifacts.
- **No JTBD analysis** — Phase 1 of the skill workflow is skipped.
  Existing jobs cover this feature.

### What ran

- **Phase 2.5 (Story Mapping + Carpaccio slicing) — full.** Eight
  slices identified (one per backbone activity plus the hydrator
  reconciler), dependency-ordered, taste-tested, sized.
- **Phase 3 (Stories + AC + DoR + KPIs) — light.** Eight LeanUX
  stories (one per slice), each with embedded BDD scenarios, AC
  derived from UAT, DoR validated 9/9, outcome KPIs at the feature
  level (one per story, K1-K8).

## Wizard decisions honoured (pre-filled, not asked)

- **Decision 1 — Feature type**: Backend / Cross-cutting (dataplane
  port consumed by reconciler runtime, gateway, scheduler).
- **Decision 2 — Walking skeleton**: NO new walking skeleton. Phase
  1's `submit-a-job.yaml` walking skeleton already shipped; Phase 2.1
  added the `Dataplane` port adapter behind it. This feature fills
  the empty body of `EbpfDataplane::update_service`.
- **Decision 3 — UX research depth**: Lightweight. No UX; consumer is
  the runtime via the `Dataplane` port trait. CLI / TUI patterns
  do not apply.
- **Decision 4 — JTBD analysis**: NO. Motivation locked by whitepaper
  § 7 / § 15 / § 19 + existing `J-PLAT-001` (DST trust) + newly
  activated `J-PLAT-004` (reconciler convergence).

## Pinned scope (from GitHub roadmap + research + #152 + #154)

The Phase 2.2 issue this feature delivers:

- **GH #24 [2.2]** XDP routing + service load balancing (`SERVICE_MAP`,
  O(1) lookup) — whitepaper § 7. **This is the issue this feature
  closes.**

Scope-deferral cross-references:

- **GH #154 [2.16]** Conntrack — explicitly OUT of this feature.
  Maglev's ≤ 1% disruption property is the interim flow-affinity
  guarantee. Every slice's Technical Notes lists this deferral.
- **GH #152 [2.7]** Kernel matrix — explicitly OUT of this feature.
  Tier 3 / Tier 4 run single-kernel in-host (developer Lima VM, CI
  `ubuntu-latest`). The `cargo xtask integration-test vm` LVH harness
  wired in #23 stays in place but is not exercised by Phase 2.2.
- **GH #25 [POLICY_MAP]** — Operator-tunable DDoS rules. The static
  packet-shape sanity prologue lands here (Slice 06); operator-tunable
  rules belong to #25 with materially different mechanics (compile-on-
  rule-change vs hardcoded prologue).
- **GH #23 [2.1]** — Phase 2.1 scaffolding, finalized and merged. The
  dependency this feature builds on. ADR-0038 captures the substrate
  this feature inherits.

## Constraints established

This feature is bounded by the following constraints; they are not
debatable within this wave and must be carried forward into DESIGN /
DISTILL / DELIVER:

- **Phase 2.2 is single-kernel in-host** — no nested-VM, no kernel
  matrix. Tier 3 / Tier 4 run on the developer's local Lima VM (via
  `cargo xtask lima run --`) and CI's `ubuntu-latest` runner. The
  `cargo xtask integration-test vm` LVH harness from #23 stays
  available for the kernel-matrix slice that lands when #152 lands.
- **Conntrack is OUT of scope** — Phase 2.2 ships a stateless
  Maglev-style forwarder. Maglev's ≤ 1% disruption property bounds
  flow misroute under backend churn; conntrack is #154.
- **The XDP program is `#![no_std]` and lives in `overdrive-bpf`**
  per ADR-0038. Userspace loader and hydration logic live in
  `overdrive-dataplane`. No kernel-side code crosses into
  `overdrive-host` or `overdrive-core`.
- **`Dataplane` port trait is the only consumer-facing surface.**
  Reconcilers and the action shim see `Arc<dyn Dataplane>`; production
  wiring uses `EbpfDataplane`, DST uses `SimDataplane`. No code path
  imports `aya` directly outside `overdrive-dataplane`.
- **Hydrator reconciler purity is non-negotiable.** The SERVICE_MAP
  hydrator reconciler — the consumer of the `Dataplane` port body
  — lands in this feature (Slice 08) and MUST satisfy the existing
  `ReconcilerIsPure` DST invariant. Sync `reconcile`, no `.await`,
  no wall-clock reads inside the function, View persistence via
  the runtime-owned redb `ViewStore`, all I/O expressed as typed
  `Action` values consumed by the action shim. The reconciler is
  the consumer that closes the ESR loop against `service_backends`
  rows; without it, every earlier slice's `Dataplane` port plumbing
  is untested against a real reconciler.
- **Determinism in the hydrator-side userspace logic is load-bearing.**
  Maglev table generation (Slice 04) reads weighted-backend inputs
  in `BTreeMap` iteration order so the produced permutation table is
  bit-identical across runs and across nodes given identical inputs.
- **STRICT newtypes for new identifiers.** `ServiceVip`, `ServiceId`,
  `BackendId`, `MaglevTableSize`, `DropClass` are NEW newtypes shipped
  by this feature in `overdrive-core`. Each MUST have FromStr / Display
  / serde / rkyv / proptest discipline per
  `.claude/rules/development.md` § Newtype completeness.
- **Real-infrastructure tests gated `integration-tests` feature.**
  Default lane uses `SimDataplane`. Tier 2 PROG_TEST_RUN, Tier 3
  in-host integration tests, Tier 4 perf gates all behind the
  feature flag.
- **Native XDP only; warn on generic fallback.** Lima virtio-net,
  `ubuntu-latest` virtio-net, mlx5, ena all support native mode. A
  failure to attach in native mode logs a structured warning, not a
  silent regression.
- **No new fields on existing aggregates.** `Job` and `Node` ship
  unchanged. Service / backend hydration reads the existing
  `service_backends` ObservationStore table; no schema migration
  required.

## Aggregate field changes do NOT emerge here

This feature ships zero schema changes on `Node`, `Job`, or any other
aggregate in `overdrive-core`. The existing aggregate-roundtrip
proptest in `crates/overdrive-core/tests/acceptance/aggregate_roundtrip.rs`
continues to pass byte-identical. New types (`ServiceVip`, `ServiceId`,
`BackendId`, `MaglevTableSize`, `DropClass`) are NEW types with their
own roundtrip proptests; they extend the testbed but do not modify
existing aggregate shapes.

## Artifacts produced

### Product SSOT (additive)

- `docs/product/jobs.yaml` — activated `J-PLAT-004` (status:
  `deferred` → `active`) tied to phase-2-xdp-service-map. Source
  string extended to whitepaper § 7 (BPF Map Architecture — hydration
  as reconciler) alongside § 18. Single new changelog row dated
  2026-05-05. **NO new job entries**; `J-PLAT-001` and `J-PLAT-004`
  together cover this feature.

### Feature artifacts (this directory)

- `docs/feature/phase-2-xdp-service-map/discuss/user-stories.md` —
  eight LeanUX stories (US-01 through US-08) with System Constraints
  header and embedded BDD per story.
- `docs/feature/phase-2-xdp-service-map/discuss/story-map.md` —
  7-activity backbone (the hydrator is cross-cutting against
  activities 2-6, captured as Slice 08 rather than a new backbone
  column), walking-skeleton inheritance from Phase 1 / 2.1
  documented, 8 carpaccio slices, priority rationale, scope assessment
  PASS, slice taste-tests against the 8-slice plan all green.
- `docs/feature/phase-2-xdp-service-map/discuss/outcome-kpis.md` —
  eight feature-level KPIs (K1-K8) + measurement plan + handoff to
  DEVOPS.
- `docs/feature/phase-2-xdp-service-map/discuss/dor-validation.md` —
  9-item DoR PASS on 8/8 stories. No HARD DESIGN dependencies flagged.
- `docs/feature/phase-2-xdp-service-map/discuss/wave-decisions.md`
  (this file).
- `docs/feature/phase-2-xdp-service-map/slices/slice-{01..08}-*.md` —
  one brief per carpaccio slice (≤ 100 lines each), with slice
  taste-tests applied.

### NOT produced (lean shape — explicit non-output)

- No `journey-*-visual.md` (no UX surface to mock up).
- No `journey-*.yaml` (no human-facing journey to structure).
- No `journey-*.feature` (no Gherkin to specify; per-story BDD is
  embedded in `user-stories.md` as specification only).
- No `shared-artifacts-registry.md` (no cross-step shared variables;
  data flowing through `Dataplane::update_service` is typed not
  vocabularied).
- No `prioritization.md` (priority rationale is in `story-map.md`'s
  "Priority Rationale" section per the lean methodology).
- No `discover/` or `diverge/` artifacts (waves did not run).

## Key decisions

### 1. Lean DISCUSS shape adopted

The methodology guidance says JTBD + Journey are the highest-leverage
parts of DISCUSS for user-facing features. For dataplane
infrastructure consumed by the runtime, both add ceremony cost
without proportionate signal. The lean shape ran Phase 2.5 (story
mapping) and Phase 3 (stories + AC + DoR + KPIs) only. Justification
is whitepaper § 7-anchored motivation + Phase 2.1 (#23) precedent +
no human persona to drive a journey.

### 2. No DIVERGE artifacts — grounded in research + whitepaper

The two key algorithmic / structural choices for this feature
(three-map split à la Cilium; weighted Maglev consistent hashing) are
locked by `docs/research/networking/xdp-service-load-balancing-research.md`,
which surveyed 37 sources from kernel docs, Cilium / Katran source,
the Maglev NSDI 2016 paper, and Cloudflare engineering. DIVERGE would
re-litigate questions the research already answered with high
confidence. **Risk**: design choices are inferred from research
literature, not interview-validated against an Overdrive-specific
load profile. **Mitigation**: every key decision lists a falsifiable
hypothesis (research § "Hypothesis", research § "Implication for
Overdrive Phase 2.2"); if any of them turns out wrong, the slice's
Tier 3 / Tier 4 evidence will surface it.

### 3. J-PLAT-004 activation

`docs/product/jobs.yaml` flips `J-PLAT-004` from `status: deferred`
to `status: active`. Phase 2.2 is the first slice where a non-trivial
reconciler — the SERVICE_MAP / BACKEND_MAP / MAGLEV_MAP /
REVERSE_NAT_MAP hydrator — needs ESR-shaped invariants against
SimDataplane in DST. The hydrator's pure-reconcile + action-shim
shape is the §18 reference implementation that every later dataplane
reconciler will mirror. **The hydrator reconciler — the first
non-trivial use of the `Reconciler` trait against a real dataplane
port — IS what serves J-PLAT-004 in this feature**, landing in
Slice 08 against the `Dataplane` port body Slices 02-06 fill in.
The ESR pair (`HydratorEventuallyConverges`,
`HydratorIdempotentSteadyState`) lives in `overdrive-sim::invariants`
and runs on every PR.

### 4. Eight slices, Slice 04 at 1.5 days, the rest ≤ 1 day, two parallel-able

Slices 05 and 06 can run in parallel; Slice 08 (hydrator reconciler)
can run in parallel with Slice 03 onward (it depends only on Slice 02's
`Dataplane::update_service` body and the new `service_backends`
ObservationStore subscription, not on HASH_OF_MAPS / Maglev /
REVERSE_NAT mechanics). The remaining slices are sequential.
Total estimated effort **8.5 days** (Slice 04 acknowledged as 1.5d
given the genuine component count: `MaglevTableSize` newtype with
FromStr/Display/Serialize/Deserialize/proptest + weighted multiplicity
expansion + Eisenbud permutation + lookup switch + veristat baseline +
2 DST invariants — see Risk #2 / Risk #5); wall-clock can compress to
~6.5 days with the parallel pairs. Story count (8) is at the upper
end of the right-sized band; bounded contexts (3) are within the ≤ 3
oversized signal (no new crate boundary for the hydrator — it lives
alongside the existing reconciler set in `overdrive-control-plane`);
integration points (4) are within the ≤ 5 oversized signal.
**Verdict: RIGHT-SIZED**, full taste-test results in `story-map.md` §
"Slice taste-tests against the 8-slice plan."

### 5. Single-kernel in-host per #152

Every Tier 3 / Tier 4 measurement runs single-kernel in-host. The
developer uses `cargo xtask lima run --` per
`.claude/rules/testing.md` § "Running tests on macOS — Lima VM"; CI
runs on `ubuntu-latest`. The `cargo xtask integration-test vm` LVH
harness from #23 stays in place but is not exercised by Phase 2.2;
it activates when #152 lands the kernel matrix. **Risk**: `ubuntu-latest`
kernel will drift over time; baselines under `perf-baseline/main/`
may need re-baselining when the runner image updates. **Mitigation**:
the relative-delta-only nature of Tier 4 gates means runner-class
variance is absorbed; #152 will eventually pin kernel versions.

### 6. Conntrack-free per #154

Phase 2.2 ships a stateless Maglev-style forwarder. Flow affinity is
the Maglev ≤ 1% disruption guarantee, not per-flow conntrack. Every
slice's Technical Notes states this explicitly. **Risk**: real
operator workloads with long-lived flows may notice the 1% disruption
under aggressive backend churn. **Mitigation**: M=16381 default with
M ≥ 100·N rule (research § 5.2) bounds the disruption tightly;
operator-tunable M for high-fanout services. Conntrack lands as #154.

### 7. ServiceMapHandle / typed map newtype API per research recommendation #5

Slice 02 introduces `ServiceMapHandle` — a typed Rust newtype in
`overdrive-dataplane` wrapping the aya `HashMap<_, _, _>`. Slice 03
extends with similar handles for the HASH_OF_MAPS shape; Slice 04
extends with MAGLEV_MAP handles; Slice 05 extends with REVERSE_NAT_MAP
handles. The pattern hides `BPF_MAP_TYPE_*` choice from call sites,
matching Overdrive's "make invalid states unrepresentable" discipline
(`development.md` § Type-driven design). Research recommendation #5.

### 8. Weighted Maglev shipped directly in Slice 04 (no vanilla-then-weighted progression)

Research § 5.3 establishes that the weighted variant's algorithmic
delta is in userspace permutation generation; the kernel-side lookup
shape is identical. Splitting "vanilla Maglev" and "weighted Maglev"
into two slices would force Slice 04's verifier baseline to be
re-baselined by a follow-on slice with no algorithmic gain. Decision:
ship weighted directly in Slice 04, fulfil § 15's "weighted backends
(e.g., 95% v1, 5% v2)" commitment in one slice.

### 9. Sanity prologue (Slice 06) ships static checks only

Operator-tunable DDoS rules are explicitly OUT of Slice 06 — they
belong to #25 POLICY_MAP, with materially different mechanics
(compile-on-rule-change). Slice 06 ships the five static
Cloudflare-shape checks (research § 7.2) at the top of both XDP
programs. The verifier-budget delta is explicitly bounded (< 20% vs
Slice 04 baseline; absolute ≤ 60% of 1M ceiling).

### 10. Tier 4 perf gates land in Slice 07, baseline only end-of-feature

Phase 2.1 deferred Tier 4 to "the first slice with real programs to
gate" (ADR-0038 D8). Slice 07 closes that gap by filling in the two
xtask stubs (`verifier-regress`, `xdp-perf`) and capturing baseline
files under `perf-baseline/main/`. Baselining only at end of feature
(after Slice 06 has finalised the program shape) avoids spurious
re-baselining mid-feature.

### 11. Scenario titles are business outcomes, not implementation

Every scenario title in the embedded BDD describes
operator-or-platform-observable behaviour ("Atomic backend swap drops
zero packets under traffic", "Real TCP connection completes through
forward and reverse paths") rather than internal mechanics. The
single semi-mechanical exception is verifier-acceptance scenarios
("Verifier accepts the program; instruction count baselined") —
"verifier accepts" is unavoidable phrasing because the kernel
verifier's accept/reject is the structural contract.

### 12. No regression on prior feature guardrails

Every guardrail from `phase-1-foundation`, `phase-1-control-plane-core`,
`phase-1-first-workload`, and `phase-2-aya-rs-scaffolding` applies
verbatim: DST wall-clock < 60s, lint-gate false-positive at 0,
snapshot round-trip byte-identical, CLI round-trip < 100ms, OpenAPI
schema-drift gate green, `cargo xtask bpf-build` produces the BPF
artifact at the canonical path, macOS `--no-run` gate stays green.
The new Phase 2.2 invariants compose with the existing catalogue;
they do not replace it.

## Scope assessment result

- **Stories**: 8 (at the upper end of the right-sized band; the
  hydrator reconciler is cross-cutting against the dataplane port
  Slices 02-06 fill in, but is its own deliverable with its own AC,
  taste-test, and DoR row).
- **Bounded contexts / crates touched**: 3 (`overdrive-bpf`,
  `overdrive-dataplane`, `overdrive-core` for additive newtypes; the
  hydrator lives in the existing `overdrive-control-plane` reconciler
  set, not a new crate). Within the ≤ 3-bounded-context oversized
  signal.
- **Walking-skeleton integration points**: 4 (Phase 1 walking
  skeleton via `Dataplane::update_service`; Phase 2.1 `EbpfDataplane`
  adapter; veth integration test harness; CI workflow Job E).
  Within the ≤ 5 oversized-signal threshold.
- **Estimated effort**: ~8.5 focused days (Slice 04 acknowledged as
  1.5d, the rest ≤ 1d each). Slices 05/06 and Slice 08 can run in
  parallel against Slice 04+ wall-clock; wall-clock can compress to
  ~6.5 days.
- **Multiple independent user outcomes worth shipping separately**:
  no — the eight slices are sequential / parallel on the same § 7 /
  § 15 / J-PLAT-004 commitment. The hydrator reconciler is the
  consumer that closes the ESR loop against the port body the other
  slices fill in; deferring it would have left the port plumbing
  untested against a real reconciler.
- **Verdict**: **RIGHT-SIZED** — 8 stories at the upper end of the
  band, 3 crates well-bounded, 4 integration points, every slice
  passes carpaccio taste tests.

## Risks surfaced

| # | Risk | Probability | Impact | Mitigation |
|---|---|---|---|---|
| 1 | DIVERGE was not run; algorithmic and map-shape choices are research-inferred, not Overdrive-load-profile-validated | Low | Medium | Research surveyed 37 sources at high confidence; every key choice carries a falsifiable hypothesis; Tier 3 / Tier 4 evidence will surface mismatches. Documented as "Risk #1 — DIVERGE not run" per the orchestrator brief. |
| 2 | Aya-rs program emits BPF bytecode that the verifier rejects on a corner case Cilium / Katran's C-written equivalents handle silently | Medium | Medium | Slice 02 establishes the verifier-baseline early. If the basic SERVICE_MAP shape doesn't fit, the slice surfaces it before Maglev's complexity lands. Research § 5.4 says C and aya-rs through LLVM produce equivalent bytecode; absent published benchmark of an aya-rs LB closing the loop, this is a known knowledge gap (research § Gap 2). Slice 04 (Maglev) is acknowledged as a 1.5-day slice rather than 1 day given its genuine component count (`MaglevTableSize` newtype with full FromStr/Display/serde/proptest discipline + weighted multiplicity expansion + Eisenbud permutation + lookup switch + veristat baseline + 2 DST invariants); splitting it would create two near-duplicate slices and fail the carpaccio taste test, so it stays as one slice with a 1.5-day budget. |
| 3 | `xdp-trafficgen` 100 kpps load is hard to sustain on `ubuntu-latest` runner hardware; Slice 03's zero-drop assertion may flake | Medium | Low | Drop the load to 50 kpps on CI (still high enough to trigger an atomic-swap crossing); developer Lima VM exercises 100 kpps. The zero-drop assertion holds at any kpps level the runner can sustain. |
| 4 | `ubuntu-latest` runner kernel drifts over time; veristat / xdp-bench baselines under `perf-baseline/main/` need re-baselining on runner image updates | Medium | Low | Relative-delta-only thresholds (5% / 10% / 5%) absorb most variance; baseline-update commits are an explicit contributor convention with rationale required. #152's kernel matrix will eventually pin versions. |
| 5 | Maglev permutation generator at M=16381 + 100 backends takes more userspace CPU than expected during atomic swap; bounds the maximum atomic-swap rate | Low | Low | Userspace permutation generation is a one-time cost per backend-set change; production rate is ops-per-minute scale. Acknowledged; not blocking. |
| 6 | Sanity prologue's verifier delta exceeds 20% of Slice 04 baseline despite being five trivial-looking checks | Low | Medium | If the actual delta exceeds 20%, Slice 06's design needs to fold the prologue into a `bpf_tail_call` shared helper (research § 8.2's Cilium pattern). The slice's AC explicitly budgets 20%; > 20% is a slice-redesign signal, not a feature-blocker. |
| 7 | Slice 07's xtask `verifier-regress` and `xdp-perf` self-test produce a false positive on a Phase 2.3+ PR that genuinely doesn't regress | Medium | Low | The first three follow-on PRs after Slice 07 lands are hand-validated for false-positive rate per K7's measurement plan. If the false-positive rate is > 0, the gate threshold gets tuned before the next batch of PRs. |
| 8 | Conntrack-free flow affinity is too lossy under realistic operator workloads (long-lived flows + aggressive canary cutover) | Low | Low | M=16381 + M ≥ 100·N rule bounds disruption to ≤ 1% per backend change. #154 lands conntrack when it materially matters. Acknowledged; not blocking. |

## What DESIGN wave should focus on

**Priority Zero (none flagged this feature)**:

(No HARD DESIGN dependencies. The substrate is complete from ADR-0038
+ existing port traits.)

**Priority One (architectural, gates downstream slices)**:

1. **Checksum-helper choice** (Slice 02): `bpf_l3_csum_replace` /
   `bpf_l4_csum_replace` (kernel helpers) vs `csum_diff` family (aya
   helpers) — both verifier-clean, picks at the implementation
   boundary.
2. **TC-egress vs XDP-egress for `xdp_reverse_nat`** (Slice 05): aya
   0.13 has solid TC support; XDP-egress hook (newer kernels) is
   also viable. DESIGN picks; Tier 3 test is hook-agnostic.
3. **Sanity-prologue duplication vs `bpf_tail_call` shared helper**
   (Slice 06): both verifier-clean; the trade-off is between code
   duplication and tail-call indirection cost.
4. **Optional `cargo xtask perf-baseline-update` helper** (Slice 07):
   nice-to-have for reducing baseline-update friction; not required.

**Priority Two (mechanical, wireable per slice)**:

5. **Inner-map sizing** (Slice 03): per-service inner-map size for
   the HASH_OF_MAPS shape (default 256 — well below Maglev sizes).
   Easy to reconfigure; not a structural choice.
6. **Maglev table size operator-tunability surface** (Slice 04): how
   per-service M overrides reach `update_service` from the spec.
   Trivial plumbing.
7. **DropClass enum slot count and exact semantics** (Slice 06): the
   exact set of drop classes (4-6 slots) — DESIGN finalises.

## What is NOT being decided in this wave (deferred to DESIGN or beyond)

- Exact Rust module layouts inside `overdrive-dataplane::*` and
  `overdrive-bpf::*`.
- Exact aya-rs map-declaration syntax and helper macro usage.
- Whether Slice 06's static prologue is shared via tail-call or
  duplicated inline.
- The exact Action variant name and shape the hydrator emits.
  Slice 08's brief flags `Action::DataplaneUpdateService` as a
  candidate; if a new Action variant is needed (vs reusing an
  existing one), that is a DESIGN-time concern. The structural
  contract — sync `reconcile`, View persistence via redb
  `ViewStore`, Actions consumed by the action shim — is fixed by
  the §18 reconciler discipline regardless.
- IPv6 forwarding — out of scope for Phase 2.2; tracked as a future
  Phase 2 slice in GH #155.
- Cross-node REVERSE_NAT — out of scope for Phase 2.2; intrinsically a
  multi-node concern (no observable behaviour in a single-node cluster),
  so deferred to Phase 5 alongside HA, Corrosion-driven map hydration,
  and multi-node consensus. Tracked as `[5.20]` in GH #156.

## Handoff package for DESIGN (solution-architect)

- `docs/product/jobs.yaml` — `J-PLAT-004` activated tied to phase-2-xdp-service-map.
- `docs/feature/phase-2-xdp-service-map/discuss/user-stories.md` —
  eight LeanUX stories with AC + per-story BDD.
- `docs/feature/phase-2-xdp-service-map/discuss/story-map.md` —
  8 carpaccio slices + scope assessment + slice taste-tests.
- `docs/feature/phase-2-xdp-service-map/discuss/outcome-kpis.md` —
  eight measurable KPIs (K1-K8) + measurement plan.
- `docs/feature/phase-2-xdp-service-map/discuss/dor-validation.md` —
  9-item DoR PASS on 8/8 stories; no HARD DESIGN dependencies.
- `docs/feature/phase-2-xdp-service-map/discuss/wave-decisions.md`
  (this file).
- `docs/feature/phase-2-xdp-service-map/slices/slice-{01..08}-*.md` —
  slice briefs.
- Reference: `docs/whitepaper.md` § 7 (eBPF Dataplane), § 15 (Zero
  Downtime Deployments), § 19 (Security Model).
- Reference: `docs/research/networking/xdp-service-load-balancing-research.md`
  — 37-source research doc, the slicing source for this feature.
- Reference: `docs/feature/phase-2-aya-rs-scaffolding/design/wave-decisions.md`
  — Phase 2.1 substrate; ADR-0038.
- Reference: `docs/product/architecture/adr-0038-ebpf-crate-layout-and-build-pipeline.md`
  — the substrate ADR for kernel-side / userspace crate split, build
  pipeline, loader stub.
- Reference: `.claude/rules/testing.md` § "Tier 2", § "Tier 3", §
  "Tier 4" — the test discipline this feature ships against.
- Reference: `.claude/rules/development.md` § Newtype completeness,
  § Port-trait dependencies, § Reconciler I/O, § Ordered-collection
  choice — every constraint in System Constraints traces to one of
  these sections.

## Open questions surfaced for user

None blocking handoff. All eight stories cleanly clear DoR.

**User-ratified scope decision (2026-05-05, Eclipse-review-remediated).**
The hydrator reconciler — the reconciler that drives
`Dataplane::update_service` against `service_backends` row events —
is IN scope for phase-2-xdp-service-map. It lands as Slice 08 / US-08;
its ESR pair (`HydratorEventuallyConverges`,
`HydratorIdempotentSteadyState`) lives in `overdrive-sim::invariants`
and runs on every PR. The hydrator is what makes this feature
observable from the control-plane side and is the first non-trivial
use of the `Reconciler` trait against a real dataplane port — the
direct realisation of J-PLAT-004 in this feature.

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial DISCUSS wave decisions for `phase-2-xdp-service-map` (lean shape: no JTBD, no Journey, no Gherkin standalone). 7 carpaccio slices; 7/7 DoR PASS; J-PLAT-004 activated; conntrack-free per #154; single-kernel in-host per #152. |
| 2026-05-05 | User-ratified scope override: SERVICE_MAP hydrator reconciler IN scope (Luna recommended deferral; user kept it in this feature). DESIGN sizes / slices the hydrator inside Phase 2.2. IPv6 forwarding tracked in #155 (Phase 2.17); cross-node REVERSE_NAT moved to Phase 5.20 (#156) since it has no observable behaviour before HA / multi-node lands. |
| 2026-05-05 | Eclipse review (NEEDS_REVISION). Blocking: hydrator scope contradicted across artifacts after user override — `wave-decisions.md:136-140` and `:248-255` say hydrator does NOT land in Phase 2.2; override at `:464-473` says it IS in scope. Three Whos / KPIs in `user-stories.md` and `outcome-kpis.md` echo the "shipped later" framing. DoR did not validate the hydrator dimension. Slice taste-tests sized for no-hydrator world. Suggestions (high): Slice 04 ≤1-day budget tight; US-04 disruption-bound scenario phrasing imprecise; verifier-acceptance scenario convention should be captured in dor-validation cross-cutting. Suggestions (medium): K6 baseline column ambiguous; K3 measurement names a Slice 06 artifact; DropClass placeholder name "...hold-on-this-is-pass-not-drop"; REVERSE_NAT_MAP key endianness lockstep not stated. Suggestion (low): slice-05 still says "future Phase 2 slice" for cross-node REVERSE_NAT; should now say "future Phase 5 slice — #156". |
| 2026-05-05 | Remediated all findings from Eclipse review. Hydrator reconciler IN scope: US-08 + Slice 08 added; `wave-decisions.md` Constraints, Decision 3, Decision 4, Risk register, and "What is NOT being decided" sections reconciled to 8-slice IN-scope frame. DoR re-validated 8/8 with new hydrator-dimension cross-cutting item. K8 added to outcome-kpis. Slice 04 acknowledged as 1.5d. US-04 disruption-bound phrasing corrected. K3/K6 measurement plans tightened. DropClass placeholder slot dropped. REVERSE_NAT_MAP endianness lockstep documented. slice-05 cross-node phrasing updated to Phase 5.20. Story–slice 1:1 mapping documented. Verdict pending re-review. |
