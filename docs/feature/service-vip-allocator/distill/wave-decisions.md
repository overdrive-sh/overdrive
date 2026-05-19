# Wave Decisions — service-vip-allocator (DISTILL)

**Wave**: DISTILL
**Feature**: service-vip-allocator
**Date**: 2026-05-14
**Author**: Acceptance Designer (nw-acceptance-designer)

## Reading Confirmation Checklist

| # | Artifact | Status | Path |
|---|----------|--------|------|
| 1 | Journeys | + read | `docs/product/journeys/submit-a-job.yaml` |
| 2 | Architecture Brief | + read | `docs/product/architecture/brief.md` |
| 3 | KPI Contracts | - not found | `docs/product/kpi-contracts.yaml` does not exist |
| 4a | DISCUSS user-stories | + read | `docs/feature/service-vip-allocator/discuss/user-stories.md` |
| 4b | DISCUSS story-map | + read | `docs/feature/service-vip-allocator/discuss/story-map.md` |
| 4c | DISCUSS wave-decisions | + read | `docs/feature/service-vip-allocator/discuss/wave-decisions.md` |
| 4d | DISCUSS outcome-kpis | + read | `docs/feature/service-vip-allocator/discuss/outcome-kpis.md` |
| 5 | SPIKE findings | - not found | `docs/feature/service-vip-allocator/spike/` absent (no spike wave) |
| 6 | DEVOPS wave-decisions | - not found | `docs/feature/service-vip-allocator/devops/` absent (no devops wave) |
| 7a | DESIGN wave-decisions | + read | `docs/feature/service-vip-allocator/design/wave-decisions.md` |
| 7b | DESIGN upstream-changes | + read | `docs/feature/service-vip-allocator/design/upstream-changes.md` |
| 7c | ADR-0049 | + read | `docs/product/architecture/adr-0049-platform-issued-service-vip-allocator.md` |
| 8 | Existing allocator code | + read | `crates/overdrive-dataplane/src/allocator.rs` |
| 9 | Existing handler code | + read | `crates/overdrive-control-plane/src/handlers.rs` (`submit_workload`) |
| 10 | Existing test patterns | + read | `crates/overdrive-control-plane/tests/acceptance/`, `crates/overdrive-dataplane/tests/` |

## Wave-Decision Reconciliation

Reconciliation passed — 0 contradictions.

| DISCUSS Decision | DESIGN Resolution | Contradiction? |
|---|---|---|
| D1: Single user story | Confirmed | No |
| D2: ACs from #167 minus pinned-VIP | Confirmed + strengthened (K8: field removal vs admission rejection) | No — DESIGN strengthens DISCUSS direction |
| D3: Allocator in `overdrive-dataplane/` | Confirmed (ADR-0049 § 1) | No |
| D4: Shared primitive shape deferred | Resolved: pure-core + persistence shim (K1) | No — resolution, not contradiction |
| D5-D7: Skip journey/JTBD/WS | Confirmed | No |
| D8: Phase 1 single-node | Confirmed (C4) | No |
| Changed Assumption: platform-issued only | Strengthened: parser-level field removal (K8 amended) | No — same direction, stronger enforcement |

All five DISCUSS open questions resolved by DESIGN with no contradictions
against DISCUSS's direction. The K8 amendment (parser-level removal vs
admission-level rejection) is a strengthening of the same intent.

## Graceful Degradation

| Artifact | Status | Action |
|---|---|---|
| DEVOPS | Missing | Default environment matrix applied: clean install. No infrastructure constraints to propagate. |
| SPIKE | Missing (intentional) | Brownfield refactor; no new mechanism to validate. Proceed. |
| KPI Contracts | Missing (`kpi-contracts.yaml` does not exist) | KPI targets taken from `discuss/outcome-kpis.md` (K1–K4). Proceed. |

## Decisions

### DWD-01: Walking Skeleton — SKIP

Per DISCUSS D7: brownfield refactor of an existing isolated primitive
(`BackendIdAllocator`). The Phase 1 walking skeleton already ships at the
workspace level; this feature is one primitive deepening within it.

The acceptance test suite exercises the full admission-through-reclamation
path via the driving ports (submit handler + reconciler tick), which is
functionally equivalent to a walking-skeleton end-to-end test for this
feature's scope.

### DWD-02: Adapter Strategy — Real local (Strategy C)

All resources are local and cheap:

| Resource | Adapter | Strategy |
|---|---|---|
| IntentStore (redb) | `LocalStore` with `TempDir` | Real (same as existing acceptance tests) |
| ObservationStore | `SimObservationStore` | Sim (in-memory LWW, per existing pattern) |
| Driver | `SimDriver` (or none — allocator tests don't exercise drivers) | Sim |
| Clock | `SimClock` (for reclamation timing) or `SystemClock` (for admission-only) | Per-test |
| Dataplane | `SimDataplane` (allocator tests don't exercise kernel maps) | Sim |
| TOML parser | Real serde/toml deserializer | Real |

No costly external dependencies. No containers needed. All tests
run inside Lima VM per `.claude/rules/testing.md` § "Running tests —
Lima VM".

### DWD-03: Test Crate Placement

| Scenario Scope | Crate | Path | Rationale |
|---|---|---|---|
| AC-01/02/04 (admission-driven) | `overdrive-control-plane` | `tests/acceptance/service_vip_*.rs` | Exercises `submit_workload` handler (the driving port) |
| AC-03 (reclamation) | `overdrive-control-plane` | `tests/acceptance/service_vip_reclamation.rs` | Exercises `WorkloadLifecycle` reconciler tick |
| AC-05 (shared primitive) | `overdrive-dataplane` | `tests/` (unit-level, default lane) | Exercises `PoolAllocator<T>` directly |
| AC-06 (parser rejection) | `overdrive-core` or `overdrive-control-plane` | Per crafter (parser lives in core; handler exercises it) | Exercises TOML deserializer |
| Config/boot validation | `overdrive-control-plane` | `tests/acceptance/` or `tests/integration/` | Exercises boot path with config |
| Property tests | Per-crate (`overdrive-core`, `overdrive-dataplane`) | `tests/` (proptest, default lane) | Per-type roundtrip + allocator invariants |
| Schema evolution | `overdrive-dataplane` | `tests/schema_evolution/allocator_entry.rs` | rkyv golden-bytes fixture |

### DWD-04: Scenario Coverage Shape

| Category | Count | % |
|---|---|---|
| Happy path (AC-01, AC-02, AC-03, AC-05) | 7 | 33% |
| Error / edge / boundary (AC-04, AC-06, config, boot) | 10 | 48% |
| Property-based (mandated by `testing.md`) | 4 | 19% |
| **Total** | **21** | — |

Error/edge path coverage: 48% (above 40% target).

### DWD-05: Mandate 7 Scaffolding — Deferred to DELIVER

Per project convention: DISTILL writes specification-level GIVEN/WHEN/THEN
in `test-scenarios.md`. The crafter translates scenarios into Rust
`#[test]` / `#[tokio::test]` functions during DELIVER, using
`#[should_panic(expected = "RED scaffold")]` per
`.claude/rules/testing.md` § "RED scaffolds". No Rust scaffold files are
created by this DISTILL wave.

## Upstream Issues

None discovered. All DISCUSS acceptance criteria are testable as written
after DESIGN's resolutions. No back-propagation from DISTILL required.

## Changelog

- 2026-05-14 — Initial DISTILL wave decisions captured. 0
  contradictions in reconciliation. 21 scenarios across 6 ACs +
  config/boot + property tests. Walking skeleton skipped (brownfield).
  Adapter strategy: real local (Strategy C). Mandate 7 scaffolding
  deferred to DELIVER per Rust project convention.
