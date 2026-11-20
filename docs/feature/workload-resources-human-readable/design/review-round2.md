# Verification review (round 2) — DESIGN, workload-resources-human-readable (GH #372)

Reviewer: nw-solution-architect-reviewer (Sonnet), 2026-11-20. Persisted by the orchestrator.

**Verdict: APPROVED** (0 critical, 0 high, 1 medium, 4 low). All A1–A10 / S1–S17 dispositions verified applied; no new contradictions; no open contract-shaping item; no pending decision.

1. MEDIUM — grep gate (`feature-delta.md:233-236`) misses `examples/service-kind-vm-workloads-v2/run-example.sh:1249` log label `memory_bytes=$HOST_MEM_BYTES` (host capacity, not a spec field). Add as exception or rename.
2. LOW — "35 files under `examples/`" is 33 TOML specs + that script.
3. LOW — `docs/feature/node-local-persistent-volume/design/wave-decisions.md:15` still lists "a byte-granule rule" (dropped from ADR-0310 under S16).
4. LOW — AC-2.7 (`:109`) and the KPI (`:142`) say zero occurrences; the gate allows the `mtls_path.rs` metric label.
5. LOW — sub-MiB sentence (`:214-215`) omits proptest strategies drawing arbitrary `u64` memory (`describe_round_trip.proptest-regressions:9-10`); they must generate whole-MiB memory.
