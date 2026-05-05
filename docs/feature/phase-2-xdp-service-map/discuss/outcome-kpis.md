# Outcome KPIs — phase-2-xdp-service-map

## Feature: phase-2-xdp-service-map

### Objective

Overdrive platform engineers can land a real XDP-driven service load
balancer end-to-end against a real Linux kernel — proving § 7's
"O(1) BPF map lookup for VIP-to-backend resolution, replacing
kube-proxy entirely" with veristat-baselined verifier budget and
xdp-bench-baselined throughput, and proving § 15's "weighted backends
(e.g., 95% v1, 5% v2)" zero-drop atomic-swap commitment via Tier 3
integration tests under sustained `xdp-trafficgen` load — by the end
of Phase 2.2.

> **Phase 2.2 is single-kernel in-host.** Per #152, every Tier 3
> and Tier 4 measurement runs against the developer's local Lima VM
> kernel (or CI's `ubuntu-latest` runner). Kernel-matrix variants of
> these KPIs are deferred to a later Phase 2 slice. The KPIs below
> establish single-kernel baselines that future kernel-matrix work
> can extend rather than re-derive.

> **Conntrack is OUT of scope** per #154. Every KPI below measures
> the stateless Maglev-style forwarder. KPIs that would only make
> sense with conntrack (per-flow latency variance, conntrack-table
> overflow rate, etc.) are explicitly out of this feature.

### Outcome KPIs (feature level)

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | Overdrive platform engineer | Trusts the loader to attach `xdp_pass` against a real virtio-net-class driver and surfaces native-attach failures as typed errors | 100% of integration test runs show `PACKET_COUNTER` incrementing on a real veth pair; 100% of native-attach failures surface as a `tracing::warn!`; 0 silent fall-throughs to generic mode | N/A (greenfield — Phase 2.1 attached to `lo` only) | Tier 3 integration test gated `integration-tests` (`crates/overdrive-dataplane/tests/integration/veth_attach.rs`) | Leading — primary |
| K2 | Overdrive platform engineer + the SERVICE_MAP hydrator reconciler shipped in this feature | Trusts a verifier-clean, Tier-2-correct, Tier-3-end-to-end SERVICE_MAP forward path before any algorithmic LB logic lands | 100% of Tier 2 SERVICE_MAP-hit packets return `XDP_TX` with valid checksums; 100% of misses return `XDP_PASS`; veristat instruction count ≤ 50% of 1M-privileged ceiling on the post-Slice-02 program | N/A (greenfield — Phase 2.1 ships `xdp_pass` only) | Tier 2 PKTGEN/SETUP/CHECK + Tier 3 veth integration test + Tier 4 veristat baseline under `perf-baseline/main/veristat-service-map.txt` | Leading — primary |
| K3 | Overdrive platform engineer + DST harness + operators running canary deploys | Trusts the § 15 zero-drop atomic backend-set swap end-to-end against a real kernel | 0 dropped packets across an atomic backend-set swap under 100 kpps `xdp-trafficgen` load (Tier 3); 100% `BackendSetSwapAtomic` invariant pass rate (DST); 0 BACKEND_MAP orphans after GC pass | N/A (greenfield) | Tier 3 atomic-swap zero-drop integration test (zero-drop assertion: `xdp-trafficgen` send count vs trafficgen-sink receive count across the swap window — `DROP_COUNTER` is a Slice 06 artifact and not yet available at Slice 03 time, so the test asserts on the pre-/post-swap traffic accounting Slice 03 already produces); DST invariant on every PR; orphan-count assertion in integration test | Leading — primary |
| K4 | Overdrive platform engineer + operators running canary deploys | Trusts Maglev's ≤ 1% incidental-disruption property and § 15 weighted-canary commitment end-to-end | ≤2% total flow shift on single-backend removal among 100 backends — 1% forced from B50's evicted flows + ≤1% incidental, per Maglev's ≤1%-incidental-disruption bound (Tier 3); even distribution within ± 5% across equal-weight backends (Tier 3 + DST); declared weights honored within ± 2% (Tier 3); veristat instruction count ≤ 50% of 1M ceiling | N/A (greenfield) | Tier 3 disruption test under `xdp-trafficgen` synthetic flows; DST invariants `MaglevDistributionEven` + `MaglevDeterministic`; Tier 4 veristat baseline update | Leading — primary |
| K5 | Overdrive platform engineer + operators | Trusts the LB to handle real bidirectional TCP connections through both VIP→backend and backend→client paths | 100% of Tier 3 `nc` connection-completion runs succeed; 100% `ReverseNatLockstep` invariant pass rate; 0 stale REVERSE_NAT entries after backend removal | N/A (greenfield) | Tier 3 real-TCP `nc` integration test; DST invariant on every PR | Leading — primary |
| K6 | Overdrive platform engineer + future operators | Trusts the dataplane to drop pathological traffic before SERVICE_MAP lookup, with operator-visible per-class drop counters | 100% of synthetic pathological frames are dropped at the prologue (Tier 3); per-class drop counter is correct on every Tier 2 test; verifier instruction-count delta vs Slice 04's recorded baseline at the time Slice 06 lands < 20% (Tier 4 — Slice 06 then becomes the new baseline that Slice 07 enforces against subsequent PRs); absolute count ≤ 60% of 1M ceiling | N/A (greenfield — pre-prologue program is Slice 04's baseline) | Tier 2 triptych per drop class; Tier 3 mixed-batch integration test; Tier 4 veristat re-baseline | Leading — primary |
| K7 | Overdrive platform engineer + every future Phase 2 PR author + CI maintainers | Trusts the perf-gate to catch instruction-count growth and pps regressions before merge, on every PR | 100% of PRs that breach the 5% / 10% / 5% thresholds fail CI (no false negatives); 0 false positives on the first three Phase 2.3+ follow-on PRs (hand-validated); both xtask subcommands return well-formed structured output on every run | N/A (greenfield — Phase 2.1 left both xtask subcommands stubbed) | Per-PR CI logs; xtask self-test invariant `PerfBaselineGatesEnforced`; first three follow-on PRs hand-validated for false-positive rate | Leading — primary, gating |
| K8 | Overdrive platform engineer + DST harness (J-PLAT-004 reference reconciler) | Trusts the `ServiceMapHydrator` reconciler to converge `Dataplane::update_service` against `service_backends` ObservationStore rows under ESR — the first non-trivial use of the `Reconciler` trait against a real dataplane port | 100% pass rate of `HydratorEventuallyConverges` invariant across every DST seed on every PR; 100% pass rate of `HydratorIdempotentSteadyState`; `ReconcilerIsPure` continues to pass with the hydrator added; 0 `.await` / wall-clock / DB-handle violations in `reconcile` (`dst-lint` clean) | N/A (greenfield — no prior dataplane reconciler exists; this is the §18 reference J-PLAT-004 activates on) | `cargo xtask dst` against the new invariant pair on every PR; `dst-lint` gate on `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs` | Leading — primary, J-PLAT-004 |

### Metric Hierarchy

- **North Star**: an end-to-end run where Ana lands a Phase 2.3+ slice on a branch and watches the perf gate (K7) catch a verifier or pps regression that the unit suite missed AND the `ServiceMapHydrator` (K8) closes the J-PLAT-004 ESR loop end-to-end — proving that K7 is the structural backstop for every later eBPF slice and K8 is the §18 reference reconciler every later dataplane reconciler will mirror. Equivalently: K1 ∧ K2 ∧ K3 ∧ K4 ∧ K5 ∧ K6 ∧ K7 ∧ K8 all green simultaneously by the end of Slice 08.
- **Leading Indicators**:
  - K1 (real-iface attach) is the precondition for K2-K6 (every later slice's Tier 3 test depends on it).
  - K2 (SERVICE_MAP forward) is the precondition for K3-K6 AND for K8 (the hydrator drives `Dataplane::update_service`, which Slice 02 fills in).
  - K7 is the dataplane gating metric — without it, K2/K4/K6 baselines die of attrition on follow-on PRs.
  - K8 is the J-PLAT-004 gating metric — without it, every later dataplane reconciler in Phase 2 / Phase 3 will lack a published §18 reference shape.
- **Guardrail Metrics**:
  - The Phase 1 (`phase-1-foundation`, `phase-1-control-plane-core`, `phase-1-first-workload`) guardrails remain in force verbatim.
  - The Phase 2.1 (`phase-2-aya-rs-scaffolding`) guardrails remain in force: `cargo xtask bpf-build` produces the BPF artifact at the canonical path; the macOS `--no-run` gate stays green.
  - **NEW guardrail — reconciler purity preserved**. The existing `ReconcilerIsPure` invariant must continue to pass with the SERVICE_MAP hydrator reconciler shipped in this feature added to the catalogue. A regression here is a platform-team alert.
  - **NEW guardrail — `dst-lint` clean on `overdrive-dataplane` and `overdrive-bpf`**. Both crates are non-`core` (per ADR-0038), so the gate is advisory — but the team commitment is that no `core` crate gains an `aya` or kernel-side dependency as a side effect of any Phase 2.2 slice.
  - **NEW guardrail — verifier ceiling headroom**. No XDP program shipped by Phase 2.2 exceeds 60% of the 1M-privileged-instruction ceiling at the end of Slice 06. Slice 07's gate enforces ≤ 5% growth on every subsequent PR.
  - **NEW guardrail — zero-drop atomic-swap invariant**. `BackendSetSwapAtomic` must remain green on every PR, not just the PR that introduces it.

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|---|---|---|---|---|
| K1 | Tier 3 integration test in `crates/overdrive-dataplane/tests/integration/veth_attach.rs` (gated `integration-tests`) | `ip link add veth0 type veth peer name veth1`; attach `xdp_pass`; push 100 frames via `pnet`; assert `bpftool map dump name PACKET_COUNTER` reads 100 | Every PR touching `overdrive-dataplane` loader code | CI (single-kernel in-host per #152) |
| K2 | Tier 2 PKTGEN/SETUP/CHECK + Tier 3 veth integration test + Tier 4 veristat | Synthesise TCP SYN; populate SERVICE_MAP; `BPF_PROG_TEST_RUN`; assert `XDP_TX` + rewrite. Tier 3: 10 frames forwarded, captured via `tcpdump`. Tier 4: `veristat --emit insns` against compiled `xdp_service_map_lookup.o`, recorded under `perf-baseline/main/veristat-service-map.txt` | Every PR touching `overdrive-bpf` or the SERVICE_MAP shape | CI |
| K3 | Tier 3 atomic-swap test under `xdp-trafficgen` 100 kpps; DST invariant `BackendSetSwapAtomic` | `xdp-trafficgen` pushes 100 kpps to VIP; `update_service` swaps inner map; assert ZERO drops by comparing `xdp-trafficgen` send count against the trafficgen sink's receive count across the swap window — `DROP_COUNTER` is a Slice 06 artifact and is not yet available at Slice 03 time, so this Tier 3 test uses the trafficgen accounting Slice 03 already produces. (Slice 06 onward, `bpftool prog show` drop counters become available as a corroborating signal but the trafficgen accounting remains the authoritative measurement.) DST invariant evaluated on every simulated tick of every Phase 2.2+ DST run | Every PR touching `update_service` or atomic-swap mechanics | CI |
| K4 | Tier 3 disruption test (100 backends, remove one, assert ≤ 1% shift); Tier 4 veristat update | `xdp-trafficgen` 100k synthetic 5-tuple flows; pre/post snapshot of which flow lands on which backend; assert ≤ 1000 (1%) shifted. Tier 4: veristat baseline update | Every PR touching Maglev or the LB lookup path | CI |
| K5 | Tier 3 real-TCP `nc` integration test; DST invariant `ReverseNatLockstep` | `nc -l 9000` listener on `veth1`'s namespace; `nc 10.0.0.1 8080` from `veth0`'s namespace; assert payload echoes and connection closes cleanly. Lockstep invariant evaluated every DST tick | Every PR touching REVERSE_NAT_MAP or `xdp_reverse_nat` | CI |
| K6 | Tier 2 triptych per drop class; Tier 3 mixed-batch test; Tier 4 veristat re-baseline | Per-class PKTGEN/SETUP/CHECK; mixed-batch sends legitimate + each pathological class, asserts per-class counter; veristat delta < 20% vs Slice 04's recorded baseline at the time Slice 06 lands. Slice 06 then becomes the new baseline that Slice 07 enforces against subsequent PRs (the relative-delta semantics chain forward; the K6 gate is one-shot at Slice 06 land, K7's gate is continuous after that) | Every PR touching the sanity prologue | CI |
| K7 | Per-PR CI logs (Job E per `.claude/rules/testing.md`); xtask self-test | `cargo xtask verifier-regress` and `cargo xtask xdp-perf` run on every PR; structured output captured. Self-test verifies the gate logic itself returns non-zero on synthetic >5% regression input. First three Phase 2.3+ follow-on PRs hand-validated for false-positive rate | Every PR | CI + author hand-validation for the bootstrap window |
| K8 | DST invariant pair `HydratorEventuallyConverges` + `HydratorIdempotentSteadyState`; `dst-lint` gate; `ReconcilerIsPure` | `cargo xtask dst` runs the new invariant pair against seeded `SimDataplane` + seeded `service_backends` row generators on every PR; `dst-lint` walks `crates/overdrive-control-plane/src/reconcilers/service_map_hydrator.rs` for `.await` / direct wall-clock / DB-handle violations; `ReconcilerIsPure` continues to pass with the hydrator added | Every PR touching the hydrator reconciler or the `Dataplane` port surface | CI |

### Hypothesis

We believe that landing the real-iface XDP attach + SERVICE_MAP +
HASH_OF_MAPS atomic swap + Maglev consistent hashing + REVERSE_NAT
+ sanity prologue + Tier 4 perf gates + SERVICE_MAP hydrator
reconciler as an 8-slice carpaccio plan will produce the
structurally-complete dataplane substrate that every subsequent
Phase 2.3+ slice (POLICY_MAP / IDENTITY_MAP / FS_POLICY_MAP /
conntrack #154 / sockops / kTLS) can build on with verifier-budget
headroom, CI-enforced regression protection, and a published §18
reference reconciler shape.

We will know this is true when **a Overdrive platform engineer can
attach a real XDP program against a real virtio-net interface
(K1), forward TCP traffic through SERVICE_MAP / Maglev (K2 + K4),
swap backend sets atomically with zero drops (K3), close the
return path through REVERSE_NAT (K5), drop pathological traffic
before SERVICE_MAP lookup (K6), trust the perf gate to catch
verifier or pps regressions on every subsequent PR (K7), and
watch the `ServiceMapHydrator` reconciler converge the dataplane
to match `service_backends` ObservationStore rows under ESR (K8)
— all on the developer's local Lima VM and on CI's
`ubuntu-latest` runner, single-kernel in-host, conntrack-free**.

### Smell Tests

| Check | Status | Note |
|---|---|---|
| Measurable today? | Yes | Every KPI has an automated measurement path in CI; K1-K6 ride on Tier 2 / Tier 3 integration tests + DST invariants; K7 is the gate itself + an xtask self-test; K8 is two new DST invariants + `ReconcilerIsPure` + `dst-lint`. |
| Rate not total? | K1, K2, K5, K7 are rate-shaped (% of runs); K3 is a count-shaped invariant (0 drops); K4 and K6 are ratio-shaped against synthetic flow / instruction-count bounds. Acceptable mix for a greenfield infra feature. |
| Outcome not output? | K1-K7 all target observed engineer-or-platform behaviour against real packets / real verifier / real CI gates. None are feature-delivery checkboxes. |
| Has baseline? | Greenfield — every KPI's baseline row is explicit (N/A from prior phase). The veristat / xdp-bench baselines that K2/K4/K6/K7 depend on are CREATED by Slices 02/04/06/07 themselves and lock in at end-of-feature. |
| Team can influence? | Yes — every KPI is a direct consequence of code the platform team writes in this feature. |
| Has guardrails? | The Phase 1 + Phase 2.1 guardrails remain. New guardrails: reconciler purity, `dst-lint` clean on dataplane crates, verifier ceiling headroom, atomic-swap invariant remains green on every later PR. |

## Handoff to DEVOPS

The platform-architect needs these from this document to plan
instrumentation:

1. **Data collection requirements**:
   - CI job logs capturing veristat instruction counts per program (K2, K4, K6, K7).
   - CI job logs capturing `xdp-bench` pps + p99 latency per mode (K7).
   - Tier 3 integration test wall-clock for atomic-swap zero-drop assertions (K3).
   - DST invariant pass/fail per tick for `BackendSetSwapAtomic`, `MaglevDistributionEven`, `MaglevDeterministic`, `ReverseNatLockstep`, `SanityChecksFireBeforeServiceMap`, `PerfBaselineGatesEnforced`, `HydratorEventuallyConverges`, `HydratorIdempotentSteadyState` (K3, K4, K5, K6, K7, K8).
   - Structured `tracing::warn!` events for native-attach fallback (K1) — captured via `tracing-test` or the runner's log artifact.
2. **Dashboard/monitoring needs**:
   - CI dashboard tracking the eight new DST invariants over time + flakiness rate.
   - Trend visualisation for `perf-baseline/main/` numbers (research recommendation: dogfood the Overdrive DuckLake telemetry pipeline for CI metrics — see whitepaper § 22, but this is a stretch goal not in feature scope).
   - Alert if any of the eight new DST invariants regresses on `main` (not just on PRs).
3. **Alerting thresholds**:
   - Any regression on a new DST invariant on `main` is a platform-team alert.
   - Any `cargo xtask verifier-regress` or `cargo xtask xdp-perf` false-positive in the first three Phase 2.3+ follow-on PRs is a platform-team alert (signal that the gate logic itself needs tuning).
   - veristat instruction count growth > 5% per PR is a build-fail per K7 (already gated).
4. **Baseline measurement**:
   - The `perf-baseline/main/` directory is created by this feature (Slice 02 onwards) and locks in at end of Slice 07.
   - First three Phase 2.3+ PRs after this feature merge: hand-validate K7 for false-positive rate, then flip the alert threshold from "manual review" to "automated alert."

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial KPIs for `phase-2-xdp-service-map` DISCUSS wave (lean shape: 7 carpaccio slices → 7 feature-level KPIs). Single-kernel in-host per #152; conntrack-free per #154. |
| 2026-05-05 | Eclipse-review remediation: added K8 (SERVICE_MAP hydrator reconciler — J-PLAT-004 DST-ESR pair `HydratorEventuallyConverges` + `HydratorIdempotentSteadyState`); K3 measurement plan tightened (named-Slice-06-artifact `DROP_COUNTER` replaced with `xdp-trafficgen` send vs sink receive — the accounting Slice 03 already produces); K6 baseline column tightened (delta vs Slice 04's recorded baseline at the time Slice 06 lands; Slice 06 becomes the new baseline Slice 07 enforces); K4 corrected to ≤2% total flow shift on single-backend-removal (1% forced + ≤1% incidental per Maglev's published bound). |
