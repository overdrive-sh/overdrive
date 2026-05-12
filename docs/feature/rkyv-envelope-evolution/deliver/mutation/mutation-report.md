---
step_id: phase-5-mutation-testing
phase: PHASE_5_MUTATION_TESTING
gate: PASS
kill_rate_final: 100.0%
kill_rate_initial: 54.4%
total_mutants_final: 58
caught_final: 56
missed_final: 0
unviable_final: 2
threshold: 80.0%
mutation_tool: cargo-mutants 27.0.0
mode: diff (--diff origin/main --features integration-tests)
---

# Phase 5 — Per-Feature Mutation Testing Report

`rkyv-envelope-evolution`

## Verdict

**PASS** — kill rate 100.0% (56 caught / 56 viable mutants), well above the 80% threshold per project CLAUDE.md § "Mutation Testing Strategy". 2 mutants reported as unviable (build failure during mutation — separate signal class, not counted in kill rate).

## Final numbers

| Metric | Value |
|---|---|
| Total mutants found | 58 |
| Caught | 56 |
| Missed | 0 |
| Timeout | 0 |
| Unviable | 2 |
| Kill rate | **100.0%** |
| Threshold | 80.0% |
| Status | PASS |

## Initial baseline (before test additions)

A first diff-scoped run against `origin/main` surfaced **31 missed mutations** out of 70 viable (kill rate 54.4%, gate FAIL). The mutation taxonomy was systematic — almost every miss landed on the `VersionedEnvelope` introspection surface used by the `probe_known_variant` pre-decode helper:

- **15 mutations on per-envelope `known_discriminants()`** overrides — 5 envelopes × 3 `Vec::leak(...)` body replacements (`Vec::new()`, `vec![0]`, `vec![1]`).
- **10 mutations on per-envelope `type_name()`** overrides — 5 envelopes × 2 string-body replacements (`""`, `"xyzzy"`).
- **2 mutations on the trait-default `discriminant_offset_from_end()`** — `Some(0)` and `Some(1)` replacing `None`.
- **3 mutations on the trait-default `known_discriminants()`** — three `Vec::leak(...)` replacements for the default `&[]`.
- **2 mutations on the trait-default `type_name()`** — `""` and `"xyzzy"` replacing the default `"<unknown>"`.
- **1 mutation on `probe_known_variant`** body — `Ok(())` replacing the entire UnknownVersion classification logic.

## Root cause

Two distinct gaps were present in the pre-existing test suite:

1. **Cross-package scope mismatch.** The existing intent-side integration test
   (`crates/overdrive-store-local/tests/integration/envelope_intent_refuse.rs`)
   exercised the `UnknownVersion` probe path end-to-end for `JobEnvelope`,
   asserting on `observed`, `type_name`, and `supported_max`. That test runs
   under `overdrive-store-local`'s test scope — not `overdrive-core`'s.
   `cargo-mutants --diff` runs mutations against a single package's test
   scope per mutant (the canonical per-mutant invocation is
   `--package <owner>`); mutations in `overdrive-core`'s source therefore
   reran only `overdrive-core`'s test suite, which had no equivalent
   `UnknownVersion`-probe assertion. The structural defense was in place
   but not visible from the mutation-runner's vantage point.

2. **Four observation-row envelopes had no probe test at all.** Even at the
   cross-package level, only `JobEnvelope`'s `UnknownVersion` path was
   exercised. `AllocStatusRowEnvelope`, `NodeHealthRowEnvelope`,
   `ServiceHydrationResultRowEnvelope`, and `ServiceBackendRowEnvelope`
   shipped the same envelope-introspection surface but had no test
   asserting on `type_name`, `known_discriminants`, or
   `discriminant_offset_from_end` flowing through the probe end-to-end.

## Resolution

### Test additions (kill 19 mutations across 5 envelopes)

Added a single new harness helper
[`assert_unknown_version_probe_surfaces`](../../crates/overdrive-core/tests/schema_evolution/harness.rs)
and one test per envelope calling it
(`crates/overdrive-core/tests/schema_evolution/{alloc_status_row,job,node_health_row,service_backend_row,service_hydration_result_row}.rs`).

The helper performs two assertions per envelope, both flowing through the
public `decode_envelope_bytes::<E>(...)` driving port:

1. **Valid-bytes round-trip.** Archives the canonical V1 payload, decodes
   through `decode_envelope_bytes`, asserts equality. Kills any mutation
   to `known_discriminants()` that excludes the V1 tag from the known
   set (`Vec::leak(Vec::new())`, `Vec::leak(vec![1])`) — those mutations
   cause the pre-decode probe to reject valid bytes as `UnknownVersion`,
   so the round-trip fails.

2. **Unknown-tag-bytes surface `UnknownVersion`.** Flips the discriminant
   byte at the empirically-pinned `discriminant_offset_from_end()` to
   `99` (outside every envelope's `known_discriminants()`), asserts
   `decode_envelope_bytes` returns `Err(EnvelopeError::UnknownVersion {
   observed: 99, type_name: <expected>, supported_max: 0 })`. Kills
   mutations to `type_name()` (assertion compares the literal expected
   string) and to `probe_known_variant`'s body (a mutant that replaces
   the body with `Ok(())` returns the `Ok` round-trip for the synthesised
   bytes, failing the `.err().unwrap_or_else(...)` assertion).

The helper takes `expected_type_name: &'static str` and
`expected_supported_max: u8` as explicit per-envelope pins, so adding a
future `V2` requires updating both pins in the same commit alongside the
schema-evolution fixture — the structural defense documented in
`development.md` § "Version-bump procedure".

The helper is cross-validated by 5 per-envelope call sites; an edit
that regresses the helper itself fails all 5 simultaneously. The
existing harness self-test pattern (`harness_self_test_*` modules)
does not extend to the new helper because the inline `MockEnvelope`
intentionally relies on the trait defaults (`None` /
`&[]` / `"<unknown>"`) — adding overrides for it would require
empirically pinning a sixth offset for test machinery that is never
shipped, and the cross-validation across 5 production envelopes
already provides equivalent regression coverage.

### Exclusions (filter 12 semantically-unreachable mutants)

Added 4 new `exclude_re` entries to `.cargo/mutants.toml`:

| Pattern | Mutants excluded | Rationale |
|---|---|---|
| `replace VersionedEnvelope::discriminant_offset_from_end` | 2 | Trait default returns `None`. Every production envelope overrides; default is never reached from production code. Per-envelope overrides ARE mutation-tested by the new harness. |
| `replace VersionedEnvelope::known_discriminants` | 3 | Trait default returns `&[]`. Same rationale — every production envelope overrides. |
| `replace VersionedEnvelope::type_name` | 2 | Trait default returns `"<unknown>"`. Same rationale. |
| `::known_discriminants -> &'static[u8] with Vec::leak(vec![0])` | 5 | Equivalent mutant — every per-envelope override returns `&[0]` (V1-only state). `&[0u8]` and `Vec::leak(vec![0u8])` produce byte-equivalent slices; no test can distinguish them while V1 is the only variant. When V2 lands, production `known_discriminants()` becomes `&[0, 1]` and this exclusion naturally narrows. |

Source-level `// mutants: skip` comments on each trait default in
`crates/overdrive-core/src/codec/envelope.rs` carry the same rationale
as load-bearing documentation for future readers, per the
`docs/evolution/2026-04-29-exec-driver-rename.md` Lesson 3 (skip
annotations remain valuable as DOCUMENTATION even when the toolchain
does not honour them mechanically).

## Files changed

Test additions (kill the assertion gap):

- `crates/overdrive-core/tests/schema_evolution/harness.rs` — new
  `assert_unknown_version_probe_surfaces<E>` helper + import expansion.
- `crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs`
  — new `alloc_status_row_unknown_version_probe_surfaces` test.
- `crates/overdrive-core/tests/schema_evolution/job.rs`
  — new `job_unknown_version_probe_surfaces` test.
- `crates/overdrive-core/tests/schema_evolution/node_health_row.rs`
  — new `node_health_row_unknown_version_probe_surfaces` test.
- `crates/overdrive-core/tests/schema_evolution/service_backend_row.rs`
  — new `service_backend_row_unknown_version_probe_surfaces` test.
- `crates/overdrive-core/tests/schema_evolution/service_hydration_result_row.rs`
  — new `service_hydration_result_row_unknown_version_probe_surfaces` test.

Documentation + structural exclusions:

- `crates/overdrive-core/src/codec/envelope.rs` — added `// mutants: skip`
  documentation comments on the three trait defaults with rationale
  pointers to the per-envelope override tests.
- `.cargo/mutants.toml` — added 4 `exclude_re` entries documented above.

## Cross-references

- `.claude/rules/testing.md` § "Mutation testing (cargo-mutants)" — the
  governing discipline.
- Project CLAUDE.md § "Mutation Testing Strategy" — per-feature gate,
  ≥80% kill rate.
- ADR-0048 — the rkyv versioned-envelope decision record that motivated
  the introspection surface this gate validates.
- `crates/overdrive-store-local/tests/integration/envelope_intent_refuse.rs`
  — the existing cross-package integration test whose assertion shape
  the new helper mirrors (same probe path, same `UnknownVersion` field
  shape).

## Validation timeline

| Stage | Total | Caught | Missed | Kill rate | Status |
|---|---|---|---|---|---|
| Initial run | 70 | 37 | 31 | 54.4% | FAIL |
| After tests | 70 | 56 | 12 | 82.4% | PASS |
| After exclusions | 58 | 56 | 0 | 100.0% | PASS |

The middle stage (82.4%) is reported separately because adding tests
alone (without the exclude_re entries) ALREADY crossed the 80%
threshold. The exclude_re entries finish the cleanup by filtering
mutations whose only possible kill is a tautological test against
an equivalent mutant or against a code path that is structurally
unreachable from production.

Both stages would have passed the gate; the final run is the cleanest
signal for future readers.
