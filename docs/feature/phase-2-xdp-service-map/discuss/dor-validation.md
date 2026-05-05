# Definition of Ready Validation — phase-2-xdp-service-map

9-item DoR checklist applied to each of the eight user stories in
`user-stories.md`. Per the product-owner skill's hard-gate rule,
DESIGN wave does not start until every item passes with evidence.

> **Phase 2.2 is single-kernel in-host** per #152; **conntrack is
> OUT** per #154. Both are explicit non-goals captured in System
> Constraints; no DoR item is gated on either.

---

## Story: US-01 — Real-iface XDP attach (veth, not `lo`)

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the slice-isolation argument: `lo` to veth lift collapses debug surface area for every later slice. No technical solution prescribed. |
| 2. User/persona with specific characteristics | PASS | Ana wiring Phase 2.2 dataplane; motivation explicit — clean attribution of attach-side failures. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) veth attach + 100 frames + counter == 100; (b) generic-mode fallback with structured warn; (c) IfaceNotFound on missing veth. Real iface names, real frame counts, real `bpftool` invocation. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 3 scenarios: veth attach + counter, native-attach fallback warning, missing iface error. Within band. |
| 5. AC derived from UAT | PASS | 6 AC bullets each trace to a scenario or to System Constraints (sudo'd in-host per #152). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ ½–1 day; 3 scenarios; single concern (attach path against a real iface). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address `ip link` invocation context, CAP_NET_ADMIN via `cargo xtask lima run --` defaulting to root, dependence on Phase 2.1 scaffolding. |
| 8. Dependencies resolved or tracked | PASS | Phase 2.1 (#23) is finalized and merged. No upstream dependencies pending. |
| 9. Outcome KPIs with measurable targets | PASS | K1 row targets 100% counter increment + 100% structured warning surfacing + 0 silent fall-throughs. |

### DoR Status: **PASSED**

---

## Story: US-02 — SERVICE_MAP forward path with single hardcoded backend

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the § 7 commitment; explicit "smallest verifier-clean lookup-and-rewrite slice" rationale. |
| 2. User/persona with specific characteristics | PASS | Ana wiring SERVICE_MAP slice + DST harness consuming `Arc<dyn Dataplane>`; both motivations explicit. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) VIP `10.0.0.1:8080` → backend `10.1.0.5:9000`; (b) UDP miss falls through; (c) truncated frame returns `XDP_PASS` (sanity is Slice 06). Real IPs, real ports. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 4 scenarios: single-VIP rewrite + forward, miss → PASS, Tier 2 triptych asserting action + rewrite, verifier baseline established. Within band. |
| 5. AC derived from UAT | PASS | 9 AC bullets each trace to a scenario or System Constraint (typed `ServiceMapHandle` per research recommendation #5). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ 1 day; 4 scenarios; single concern (one new program, one new map, one typed handle, one typed VIP newtype). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address checksum-helper choice (DESIGN owns), endianness boundary, IPv6 / ICMP scope deferral, conntrack scope deferral, hard dependency on US-01. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-01 (this feature), Phase 2.1 (merged). |
| 9. Outcome KPIs with measurable targets | PASS | K2 row targets 100% Tier 2 hit/miss correctness + veristat ≤ 50% of 1M ceiling. |

### DoR Status: **PASSED**

---

## Story: US-03 — HASH_OF_MAPS atomic per-service backend swap

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with § 15 zero-drop commitment + the structural argument that flat HashMap can't deliver atomic backend-set replacement. |
| 2. User/persona with specific characteristics | PASS | Ana + operator running canary deploys; motivations explicit. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) atomic swap from `{B1}` to `{B1, B2, B3}` under 100 kpps; (b) orphan GC reclaims B3; (c) inner-map allocation failure preserves existing mapping. Real backend identifiers, real load. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 4 scenarios: zero-drop swap, orphan GC, allocation-failure preservation, SimDataplane atomic-swap mirror. Within band. |
| 5. AC derived from UAT | PASS | 8 AC bullets each trace to a scenario or System Constraint. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ 1 day; 4 scenarios; single concern (HASH_OF_MAPS restructure + 5-step swap + DST invariant). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address verifier NULL-check requirement on inner-map lookup, aya 0.13 `MapData::create` for inner-map allocation, intentional simplicity of random-slot lookup, future-slice GC ownership question, conntrack scope deferral. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-02. |
| 9. Outcome KPIs with measurable targets | PASS | K3 row targets 0 drops at 100 kpps + 100% invariant pass + 0 orphans. |

### DoR Status: **PASSED**

---

## Story: US-04 — Maglev consistent hashing inside MAGLEV_MAP

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the § 15 canary commitment + explicit verifier-budget concern (research § 5.4); ships weighted variant directly per research § 5.3. |
| 2. User/persona with specific characteristics | PASS | Ana + DST harness; motivations explicit. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) 100-backend even distribution within ± 5%; (b) 95/5 weighted canary within ± 2%; (c) ≤ 1% disruption on single-backend removal. Real backend counts, real load. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: even distribution, weighted distribution, ≤ 1% disruption, deterministic generation, verifier acceptance + baseline. Within band. |
| 5. AC derived from UAT | PASS | 9 AC bullets each trace to a scenario or System Constraint. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ 1 day; 5 scenarios; weighted-and-vanilla landed together per research recommendation. The userspace permutation generator carries most of the algorithmic complexity; the kernel-side lookup is one indirection. |
| 7. Technical notes: constraints/dependencies | PASS | Notes address userspace-only generator, type-system enforcement of M-prime invariant via `MaglevTableSize`, weighted-Maglev slot-multiplicity, M ≥ 100·N rule, hash function sharing with Slice 03's random shape, conntrack scope deferral. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-03. |
| 9. Outcome KPIs with measurable targets | PASS | K4 row targets ≤ 1% disruption + ± 5% / ± 2% distribution + veristat ≤ 50% ceiling. |

### DoR Status: **PASSED**

---

## Story: US-05 — REVERSE_NAT_MAP for response-path rewrite

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the operator-observable failure mode ("the LB doesn't work" without REVERSE_NAT) and the research § 2.1 three-map split. |
| 2. User/persona with specific characteristics | PASS | Ana + future operator running services; motivations explicit. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) real `nc` end-to-end TCP completing with payload; (b) non-LB backend traffic falls through; (c) removed-backend REVERSE_NAT entry purged on update. Real `nc` setup, real ports. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 3 scenarios: end-to-end TCP, non-LB fall-through, purge-on-removal. Within band. |
| 5. AC derived from UAT | PASS | 7 AC bullets each trace to a scenario or System Constraint. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ 1 day; 3 scenarios; single concern (egress program + REVERSE_NAT_MAP + lockstep update + lockstep invariant). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address split-program rationale (small + verifier-clean + independently veristat-able), TC-egress vs XDP-egress (DESIGN picks), perf cost without conntrack acknowledged, cross-node REVERSE_NAT deferred to future Phase 2 slice, conntrack scope deferral. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-04. |
| 9. Outcome KPIs with measurable targets | PASS | K5 row targets 100% nc connection-completion + 100% lockstep invariant + 0 stale entries. |

### DoR Status: **PASSED**

---

## Story: US-06 — Pre-SERVICE_MAP packet-shape sanity checks

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with § 7 / § 19 defense-in-depth + verifier-budget concern. Explicit positioning: static checks here, operator-tunable rules in #25 POLICY_MAP. |
| 2. User/persona with specific characteristics | PASS | Ana + future operator; motivations explicit. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) valid frame passes; (b) IPv6 falls through (not dropped); (c) truncated IPv4 dropped with counter increment. Real EtherType / IHL / TCP-flag values. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: valid passes, IPv6 fall-through, truncated drop with counter, pathological TCP flags drop, verifier delta budget. Within band. |
| 5. AC derived from UAT | PASS | 7 AC bullets each trace to a scenario or System Constraint. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ 1 day; 5 scenarios; single concern (sanity prologue + DROP_COUNTER + DropClass enum + DST invariant). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address intentional non-IPv4 / non-TCP-or-UDP "PASS not DROP" semantics (LB not firewall), narrow flag-sanity scope, per-CPU counter rationale, shared-helper-vs-duplication trade-off (DESIGN picks), conntrack scope deferral. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-04 and US-05 (sanity prologue inserts into both forward and reverse programs). |
| 9. Outcome KPIs with measurable targets | PASS | K6 row targets 100% pathological drop + correct per-class counter + veristat delta < 20% / absolute ≤ 60% of ceiling. |

### DoR Status: **PASSED**

---

## Story: US-07 — Tier 4 perf gates + veristat baseline land on `main`

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with `.claude/rules/testing.md` § Tier 4 commitment + explicit positioning relative to Phase 2.1's deferred-stub state. The "without a real PR-blocking gate, every later Phase 2 slice has no signal" argument is the engineering case. |
| 2. User/persona with specific characteristics | PASS | Ana + every future Phase 2 PR author + CI maintainers; motivations explicit. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) 2% growth passes the 5% threshold; (b) 12% growth fails the build with structured output; (c) 6% pps regression fails the build. Real numeric thresholds, real metric names. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: veristat 2% pass, veristat 12% fail, xdp-bench 6% fail, baseline-update commit-message convention, single-kernel in-host per #152. Within band. |
| 5. AC derived from UAT | PASS | 8 AC bullets each trace to a scenario or System Constraint. |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ 1 day; 5 scenarios; single concern (two xtask subcommand bodies + baseline files + CI workflow wiring + xtask self-test). |
| 7. Technical notes: constraints/dependencies | PASS | Notes address `xdp-tools` Lima image extension, runner-class variance rationale (relative-only thresholds), baseline-update friction as intentional, kernel-matrix deferral to #152, optional `perf-baseline-update` helper as DESIGN choice, conntrack scope deferral. |
| 8. Dependencies resolved or tracked | PASS | Depends on US-02 (baseline source #1), US-04 (baseline source #2), US-05 (baseline source #3), US-06 (final baseline state). |
| 9. Outcome KPIs with measurable targets | PASS | K7 row targets 100% true-positive trip rate + 0 false positives on first three follow-on PRs (hand-validated bootstrap window). |

### DoR Status: **PASSED**

---

## Story: US-08 — SERVICE_MAP hydrator reconciler converges Dataplane port

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Opens with the J-PLAT-004 commitment + the structural argument that without a real reconciler closing the loop, every earlier slice's `Dataplane` port plumbing is untested against an ESR-shaped consumer. The §18 reference shape obligation is explicit. |
| 2. User/persona with specific characteristics | PASS | Ana + DST harness; both motivations explicit. Job trace cites `J-PLAT-001` (DST trust) and `J-PLAT-004` (reconciler convergence) by ID — the first non-trivial use of the `Reconciler` trait against a real dataplane port. |
| 3. 3+ domain examples with real data | PASS | Three examples: (a) service gains a backend → hydrator emits one update → next tick converges; (b) transient `MapAllocFailed`; retry-budget gate honors backoff per persisted inputs (`attempts`, `last_failure_seen_at`); (c) stale ObservationStore intermediate state — every tick acts on the snapshot it sees, no spurious convergence. Real `ServiceId` / `BackendId` references, real `ENOMEM` failure shape. |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 5 scenarios: convergence on backend-set change, idempotent steady state, retry-budget honoring backoff, ESR convergence from arbitrary state, reconciler purity (no `.await` / wall-clock / DB-handle in `reconcile`). Within band. |
| 5. AC derived from UAT | PASS | 8 AC bullets each trace to a scenario or to System Constraints (ADR-0035 / ADR-0036, runtime-owned redb ViewStore, sync `reconcile` discipline). |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | ~ 1 day; 5 scenarios; single concern (one new reconciler with State + View types + 2 DST invariants). The hydrator does NOT need to know HASH_OF_MAPS / Maglev / atomic-swap mechanics — it calls `update_service` and `EbpfDataplane` owns the swap. Component count is at the cap (4) but each component is naturally tiny. |
| 7. Technical notes: constraints/dependencies | PASS | Notes address: hydrator does NOT need to know dataplane internals (calls `update_service` only); cross-node propagation is OUT (#156 / Phase 5.20); exact `Action` variant name flagged for DESIGN; conntrack OUT (#154); depends on Slice 02 only (parallel-able with Slices 03-07). |
| 8. Dependencies resolved or tracked | PASS | Depends on US-02 (`Dataplane::update_service` body). No HARD DESIGN dependencies — the §18 reconciler discipline (ADR-0035 / ADR-0036) is the published shape. |
| 9. Outcome KPIs with measurable targets | PASS | K8 row targets 100% pass rate of `HydratorEventuallyConverges` + `HydratorIdempotentSteadyState` across every DST seed; `ReconcilerIsPure` continues to pass; 0 `dst-lint` violations on the hydrator file. |

### DoR Status: **PASSED**

---

## Summary

| Story | Status | Note |
|---|---|---|
| US-01 | PASSED | — |
| US-02 | PASSED | — |
| US-03 | PASSED | — |
| US-04 | PASSED | — |
| US-05 | PASSED | — |
| US-06 | PASSED | — |
| US-07 | PASSED | — |
| US-08 | PASSED | — |

**Net status**: 8/8 stories cleanly READY. No HARD DESIGN dependencies
flagged. Phase 2.1's structural questions (crate split, build pipeline,
loader stub shape) were resolved by ADR-0038, and ADR-0035 / ADR-0036
fix the §18 reconciler discipline US-08 inherits. The only
DESIGN-owned choices flagged are the mechanical ones each story's
Technical Notes lists (checksum helper, TC-egress vs XDP-egress,
sanity-prologue duplication-vs-shared-helper, optional
`perf-baseline-update` helper, exact `Action` variant name for the
hydrator).

## Cross-cutting

- All 8 stories use real data (real IPs, real ports, real frame
  counts, real verifier-instruction-count thresholds, real `ServiceId`
  / `BackendId` references). No `user123`, no `test@test.com`, no
  `Foo`/`Bar`.
- All 8 stories' acceptance criteria are testable through Tier 2
  PROG_TEST_RUN, Tier 3 sudo'd-in-host integration tests, Tier 4
  veristat / xdp-bench gates, or DST invariants — no AC requires
  manual inspection.
- **Story–slice 1:1 mapping**: US-01 → Slice 01, US-02 → Slice 02,
  US-03 → Slice 03, US-04 → Slice 04, US-05 → Slice 05, US-06 →
  Slice 06, US-07 → Slice 07, US-08 → Slice 08.
- **Hydrator reconciler dimension (J-PLAT-004 reference)**: US-08
  ships the §18 reference reconciler against a real dataplane port.
  Two new DST invariants — `HydratorEventuallyConverges` (ESR-shaped
  eventual: from any seeded `service_backends` rows + starting
  `SimDataplane` state, repeated ticks drive `actual == desired`)
  and `HydratorIdempotentSteadyState` (always: no action emitted
  in steady state given unchanged inputs) — pair with the existing
  `ReconcilerIsPure` invariant to lock the hydrator's correctness
  contract. The `dst-lint` gate enforces the sync-`reconcile`
  discipline (no `.await` / no wall-clock / no DB handle inside
  `reconcile`) on every PR. This is the dimension Eclipse's review
  flagged as initially missing; the cross-cutting commitment now
  rides through US-02 → US-08 traceability + K2 / K8 KPI pair.
- Scenario titles describe operator-or-platform-observable outcomes
  ("Atomic backend swap drops zero packets under traffic", "Real TCP
  connection completes through forward and reverse paths", "veristat
  regression gate fails on 12% growth", "Hydrator converges to the
  desired backend set when service_backends rows change") not
  internal mechanics ("HASH_OF_MAPS pointer atomic write happens",
  "BPF_MAP_TYPE_ARRAY inner map gets allocated").
- **Verifier-acceptance scenario convention**: scenarios of the form
  "Verifier accepts the program" or "Instruction count under N% of
  ceiling" name an implementation mechanism (the kernel BPF verifier)
  by design — the verifier IS the structural contract for any XDP
  feature, and "verifier accepts" is unavoidable phrasing because
  the kernel verifier's accept/reject is THE structural contract.
  This is convention, not a smell; reference `wave-decisions.md`
  Decision 11. Scenarios that name `dst-lint` or `cargo xtask dst`
  fall under the same convention — the gate IS the contract.
- No banned anti-pattern detected: no "Implement X" titles, no
  generic data, no technical AC beyond what is structurally required
  for an infrastructure feature (e.g. naming `BPF_MAP_TYPE_HASH_OF_MAPS`
  in AC is unavoidable because the kernel-side type IS the contract).
- **Phase 2.2 single-kernel in-host** per #152 — every story
  acknowledges the precondition; no story assumes the kernel matrix.
- **Conntrack OUT of scope** per #154 — every story's Technical Notes
  states this explicitly; Maglev's ≤ 1% incidental-disruption (≤2%
  total flow shift on single-backend removal among 100 backends, 1%
  forced + ≤1% incidental) is the interim flow-affinity guarantee.

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial DoR validation for `phase-2-xdp-service-map`. 7/7 stories PASS; no HARD DESIGN dependencies; Phase 2.1's ADR-0038 substrate is complete. |
| 2026-05-05 | Eclipse-review remediation: added US-08 (SERVICE_MAP hydrator reconciler) DoR row passing 9/9; net status 8/8. New cross-cutting items: hydrator-reconciler dimension (the J-PLAT-004 reference reconciler with paired DST invariants `HydratorEventuallyConverges` + `HydratorIdempotentSteadyState`); story–slice 1:1 mapping documented; verifier-acceptance scenario convention captured (verifier IS the structural contract — `dst-lint` and `cargo xtask dst` follow the same convention). |
