# DISTILL wave decisions — rkyv-envelope-evolution

**Feature**: `rkyv-envelope-evolution`
**Wave**: DISTILL (Acceptance Test Design)
**Designer**: Quinn (nw-acceptance-designer)
**Status**: CONFIRMED — handoff-ready to DELIVER
**Date**: 2026-05-12

---

## 1. Pre-wave artifact checklist

This feature originates from a bug-fix RCA, not a feature pipeline. The
DESIGN wave produced the authoritative artifacts; DISCUSS / SPIKE /
DEVOPS / journey SSOT / KPI contracts do not exist and that is the
correct state per the orchestrator confirmation.

| Path | Status |
|---|---|
| `docs/feature/rkyv-envelope-evolution/discuss/brief.md` | ⊘ not found (RCA-driven; correct) |
| `docs/feature/rkyv-envelope-evolution/discuss/user-stories.md` | ⊘ not found (RCA-driven; correct) |
| `docs/feature/rkyv-envelope-evolution/discuss/story-map.md` | ⊘ not found |
| `docs/feature/rkyv-envelope-evolution/discuss/wave-decisions.md` | ⊘ not found |
| `docs/feature/rkyv-envelope-evolution/devops/environments.yaml` | ⊘ not found |
| `docs/product/journeys/*.yaml` | ⊘ not found (RCA-driven; no journey applies) |
| `docs/product/kpi-contracts.yaml` | ⊘ not found |
| `docs/feature/rkyv-envelope-evolution/design/wave-decisions.md` | ✓ read (authoritative — 7 handoff scenarios in § 10) |
| `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md` | ✓ read (Accepted 2026-05-12; design SSOT) |
| `.claude/rules/development.md` § "rkyv schema evolution" | ✓ read (envelope shape canon, version-bump procedure) |
| `.claude/rules/testing.md` § "Archive schema-evolution roundtrip" | ✓ read (golden-bytes fixture rule) |
| `crates/overdrive-core/src/traits/observation_store.rs` | ✓ read (lines 270-506: four observation row types) |
| `crates/overdrive-core/src/aggregate/mod.rs` | ✓ read (lines 96-172: `Job` + `WorkloadDriver` + `Exec`) |

**Graceful degradation applied** — the "DISCUSS missing" path from the
skill: criteria derived from DESIGN; story-to-scenario traceability
(Dim 8 Check A) is `⊘ not applicable` (no `US-*` IDs exist); the seven
handoff scenarios from DESIGN § 10 are the AC corpus.

---

## 2. Scope reconciliation against ADR-0048

| Decision | ADR-0048 § | Reflected in DISTILL |
|---|---|---|
| Envelope shape — per-type rkyv enum (Option A1) | § 1 | All S-EV-01 scenarios assert against per-type envelopes, not a generic `Envelope<T>` |
| Outer-envelope-only on `Job` aggregate (Option H) | § 4 | No sub-envelope scenarios for `WorkloadDriver` / `Exec`; only one envelope per `Job` |
| Asymmetric read policy (intent fail-fast, observation log+skip) | § 3 | S-EV-03 (intent refuse-to-start) ↔ S-EV-04 (observation degrade-gracefully) explicitly distinct |
| Write invariant — two layers (visibility + dst-lint) | § 2 | S-EV-02 split into S-EV-02a (Layer 1 trybuild) + S-EV-02b (Layer 2 dst-lint scanner test) |
| Greenfield single-cut migration | § 5 | No in-place migration scenarios; S-EV-03 names "delete `<data_dir>/intent.redb` and restart" as the remediation in the `Display` assertion |
| Documented remediation in `Display` form | § 6 | S-EV-03 asserts the `Display` form contains the literal `"delete"` + redb-path substring |

No DISTILL decision contradicts ADR-0048.

---

## 3. DWD-01 — Test layout

Per `.claude/rules/testing.md` § "Archive schema-evolution roundtrip"
the canonical layout is **one test file per envelope** under
`crates/<crate>/tests/schema_evolution/<envelope_snake>.rs`, with an
entrypoint at `crates/<crate>/tests/schema_evolution.rs`. All five
envelopes live in `overdrive-core`, so all five test files live under
`crates/overdrive-core/tests/schema_evolution/`.

**Default-lane gating** — schema-evolution tests are pure-Rust
in-memory rkyv roundtrips. They run in the default `cargo nextest run`
lane (no `integration-tests` feature). Per testing.md § "What stays in
the default lane" they satisfy every criterion (well under 60 s, no
real I/O, no subprocess, no consensus).

**S-EV-03 / S-EV-04 placement** — those scenarios touch real `redb`
files via `tempfile::TempDir`, so they live under
`crates/overdrive-store-local/tests/integration/envelope_*.rs` gated
behind `integration-tests` per testing.md § "Integration vs unit
gating" / "Layout". The entrypoint at
`crates/overdrive-store-local/tests/integration.rs` carries the
`#![cfg(feature = "integration-tests")]` gate.

**S-EV-02a** — trybuild fixture at
`crates/overdrive-store-local/tests/compile_fail/<envelope>_payload_unreachable.rs`
driven from a `tests/compile_fail.rs` entrypoint that calls
`trybuild::TestCases::compile_fail()`.

**S-EV-02b / S-EV-06** — unit tests inside `xtask/src/dst_lint.rs` (or
a sibling `xtask/src/dst_lint/envelope.rs` module); no
`overdrive-*` crate import per development.md § "xtask is build / test
/ dev orchestration".

| File | Crate | Lane |
|---|---|---|
| `tests/schema_evolution.rs` | `overdrive-core` | default |
| `tests/schema_evolution/alloc_status_row.rs` | `overdrive-core` | default |
| `tests/schema_evolution/node_health_row.rs` | `overdrive-core` | default |
| `tests/schema_evolution/service_hydration_result_row.rs` | `overdrive-core` | default |
| `tests/schema_evolution/service_backend_row.rs` | `overdrive-core` | default |
| `tests/schema_evolution/job.rs` | `overdrive-core` | default |
| `tests/integration.rs` (entrypoint) | `overdrive-store-local` | `integration-tests` |
| `tests/integration/envelope_observation_skip.rs` | `overdrive-store-local` | `integration-tests` |
| `tests/integration/envelope_intent_refuse.rs` | `overdrive-store-local` | `integration-tests` |
| `tests/compile_fail.rs` (entrypoint) | `overdrive-store-local` | default (trybuild gates internally) |
| `tests/compile_fail/alloc_status_row_payload_unreachable.rs` | `overdrive-store-local` | trybuild fixture |
| `tests/dst_lint/envelope_variant_construction.rs` (or inline `#[cfg(test)] mod tests`) | `xtask` | default |

---

## 4. DWD-02 — Walking-skeleton strategy

**Strategy declaration**: **Strategy A — Full real-adapter path.**
"InMemory" in this design means in-process rkyv roundtrip with no doubles
needed — rkyv is a pure-Rust archival codec, and the redb store IS the
real adapter (tempfile-managed); no port/adapter substitution is
required to exercise the envelope mechanism end-to-end.

Adapter coverage decision: every driven adapter scenario writes
through `LocalStore` / `LocalObservationStore` against a real
`tempfile::TempDir`-managed redb file. No `Sim*` substitution. This
satisfies Mandate 1 directly — the driving "port" for this design is
the Rust API surface of the host adapters (`LocalStore::open`,
`LocalObservationStore::write_alloc_status`, etc.), which IS the entry
point production code calls.

**Walking skeleton scope**: `AllocStatusRowEnvelope` V1 roundtrip (one
of five envelopes) + Layer-1 trybuild + Layer-2 dst-lint clause +
observation log+skip + intent refuse-to-start (the latter two against
the `AllocStatusRow` shape for observation and the `Job` shape for
intent). This is the minimum E2E shape that proves the envelope
mechanism works; the other four envelopes follow the same pattern in
DELIVER. See `walking-skeleton.md` for full detail.

**Litmus test verdict** (Dimension 5):
- Title: "Operator restarts a node and observes an in-flight allocation's status without corruption" — describes operator goal, not technical flow. ✓
- Then steps describe operator observations (`overdrive ps` shows the alloc; control-plane logs do NOT contain `subtree pointer overran range`). ✓
- Non-technical stakeholder can confirm: "an operator with rolling-deploy intent expects yesterday's saved status to still be readable today" — yes. ✓

---

## 5. DWD-03 — Mapping to four-tier model

| Scenario | Tier | Rationale |
|---|---|---|
| S-EV-01 (×5 envelopes) | Tier 1 (default-lane) | In-process rkyv roundtrip; no concurrency / timing / partition |
| S-EV-02a (Layer 1) | trybuild compile-fail | Type-system property — visibility blocks payload construction cross-crate |
| S-EV-02b (Layer 2) | xtask unit test | AST scanner is pure-Rust syntactic; no platform crate import |
| S-EV-03 (intent refuse) | Tier 1 (integration-tests gated) | Real `redb` via `tempfile`; in-process boot; no DST needed (no concurrency under test) |
| S-EV-04 (observation skip) | Tier 1 (integration-tests gated) | Real `redb` via `tempfile`; in-process read; assert structured log event emitted |
| S-EV-05 (golden-bytes pinning) | Tier 1 (default-lane) | Subsumed by S-EV-01 — every fixture file IS the assertion |
| S-EV-06 (dst-lint coverage gate) | xtask unit test | AST scanner walks `crates/<crate>/src/` for `enum *Envelope` defs |

No scenario lands in Tier 2 (BPF), Tier 3 (real-kernel), or Tier 4
(verifier/perf). No DST coverage required — the envelope mechanism is
not concurrency- or timing-sensitive; the schema-evolution invariant
is a per-byte-pattern property, not a per-schedule property.

---

## 6. DWD-04 — RED scaffold strategy

Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
failing commits", every type / trait / scanner the acceptance tests
reference must exist as a compilable RED scaffold before DELIVER
starts. Quinn creates the production-side scaffolds in this DISTILL
wave:

| Production type | Path | Scaffold shape |
|---|---|---|
| `VersionedEnvelope` trait | `crates/overdrive-core/src/codec/envelope.rs` | trait methods bodied with `todo!("RED scaffold: ...")` |
| `EnvelopeError` enum | `crates/overdrive-core/src/codec/envelope.rs` | full enum with `UnknownVersion` + `Malformed` variants, `Display` impl bodied with `todo!`; thiserror derive |
| `AllocStatusRowEnvelope` + `V1`/`V2` payloads | `crates/overdrive-core/src/traits/observation_store.rs` | enum + `pub` payload structs (not re-exported from `overdrive-core::lib.rs` per ADR-0048 § 2 Layer 1 as amended UI-01 — rustc E0446 forbids literal `pub(crate)`) + `From<V1> for V2` bodied with `todo!` |
| `NodeHealthRowEnvelope` + `V1` payload | same file | enum + payload struct |
| `ServiceHydrationResultRowEnvelope` + `V1` payload | same file | enum + payload struct |
| `ServiceBackendRowEnvelope` + `V1` payload | same file | enum + payload struct |
| `JobEnvelope` + `V1` payload | `crates/overdrive-core/src/aggregate/mod.rs` | enum + payload struct |
| `xtask::dst_lint::scan_for_envelope_variant_construction` | `xtask/src/dst_lint.rs` | fn signature with `todo!("RED scaffold: ...")` body |
| `xtask::dst_lint::scan_for_envelope_fixture_coverage` | `xtask/src/dst_lint.rs` | fn signature with `todo!("RED scaffold: ...")` body |
| `IntentStoreError::Envelope` variant | `crates/overdrive-core/src/traits/intent_store.rs` | new variant with `#[from] source: EnvelopeError` |
| `ObservationStoreError::Envelope` variant | `crates/overdrive-core/src/traits/observation_store.rs` | new variant with `#[from] source: EnvelopeError` |

Each scaffold:
- Carries `// SCAFFOLD: true` marker comment immediately above the item.
- Production-side `todo!()` bodies are gated by `#[expect(clippy::todo, reason = "RED scaffold; lands GREEN in DELIVER step rkyv-envelope-02")]`.
- Test-side scaffolds (when the acceptance test cannot compile until
  GREEN) use `#[should_panic(expected = "RED scaffold")]` + a
  `panic!("Not yet implemented -- RED scaffold (<scenario-id>)")` body.

DELIVER step IDs are advisory only (no roadmap exists yet for this
feature); the crafter chooses the slice sequence.

---

## 7. Adapter Coverage Table

Per Dimension 9 (Walking Skeleton Boundary Proof) and Mandate 1, every
driven adapter must have a real-I/O integration test.

| Driven adapter | Crate | Real-I/O scenario | Tag |
|---|---|---|---|
| `LocalObservationStore` (redb-backed observation store; production) | `overdrive-store-local` | S-EV-04 (×4 row types: alloc_status, node_health, service_hydration_results, service_backends) | `@real-io @adapter-integration` |
| `LocalStore` (redb-backed intent store; production) | `overdrive-store-local` | S-EV-03 (`Job` aggregate) | `@real-io @adapter-integration` |

S-EV-01 scenarios (×5) exercise the rkyv codec in-process (no redb;
direct `rkyv::to_bytes` / `rkyv::from_bytes`) — those are pure-codec
tests and do NOT count toward adapter coverage. S-EV-04 / S-EV-03 are
the load-bearing adapter-integration scenarios; both adapters are
covered.

---

## 8. Critique-Dimensions Self-Review (Phase 4 dry-run)

| Dimension | Verdict | Notes |
|---|---|---|
| 1. Happy Path Bias | ✓ pass | 7 scenarios: 5 happy roundtrip + 2 error (refuse / skip) + 2 compile-fail enforcement = ~50% error/enforcement path. Above 40% threshold. |
| 2. GWT Format Compliance | ✓ pass | Every scenario in `test-scenarios.md` has Given / When / Then; no multi-When scenarios |
| 3. Business Language Purity | ⚠ partial | The "user" for this feature is an operator/developer; "redb", "rkyv", "envelope" are domain terms in the persistence-layer bounded context. Per acceptance for infrastructure features: low-level technical terms are permitted in Gherkin when they are the bounded-context's domain vocabulary. Documented as a Quinn-level exception, NOT a violation. |
| 4. Coverage Completeness | ✓ pass | All 7 handoff scenarios from DESIGN § 10 mapped to executable Gherkin |
| 5. Walking Skeleton User-Centricity | ✓ pass | WS title and Then steps phrased from operator perspective (see DWD-02 litmus test) |
| 6. Priority Validation | ✓ pass | This is bug-fix RCA scope; the seven scenarios collectively close the failure mode that surfaced 2026-05-12. No simpler alternatives exist (ADR-0048 § Alternatives) |
| 7. Observable Behavior Assertions | ✓ pass | All Then steps assert on (a) function return values, (b) emitted structured log events, (c) error `Display` strings, or (d) rustc/lint output. No internal-state assertions. |
| 8. Traceability Coverage | ⊘ not applicable for Check A; ✓ pass for Check B | No `US-*` IDs (Check A `⊘`); environments default to single dev-machine real-redb (Check B covered by S-EV-03/04 using `tempfile::TempDir` real-I/O) |
| 9. Walking Skeleton Boundary Proof | ✓ pass | Strategy A declared in DWD-02; every driven adapter has a real-I/O scenario (S-EV-03 + S-EV-04); no `@in-memory` markers on WS scenarios — both adapter scenarios are real redb |

**Dimension 3 exception rationale**: this feature is purely a
persistence-layer refactor. The "user" who observes the outcome is an
operator who runs `overdrive` commands or a developer who reads
control-plane logs. Bounded-context vocabulary ("rkyv envelope", "redb
row", "schema evolution") is the operator's vocabulary here. The
business-language rule applies most strongly when the feature has
end-user-facing semantics; this feature does not.

**Verdict**: handoff-ready. No blockers.

---

## 9. Handoff to DELIVER (software-crafter)

**Inputs the crafter receives**:
- `docs/feature/rkyv-envelope-evolution/distill/test-scenarios.md` — 7 scenarios in G/W/T form with file paths, type signatures, error variants
- `docs/feature/rkyv-envelope-evolution/distill/walking-skeleton.md` — minimum E2E shape
- This file — wave decisions
- RED scaffolds landed in `overdrive-core` (`codec::envelope`, per-type envelope enums) + `xtask::dst_lint` skeleton functions

**Sequencing recommendation** (advisory; crafter owns the slice plan):
1. WS: `AllocStatusRowEnvelope` V1 roundtrip (S-EV-01 for alloc_status only) — proves the envelope shape compiles and roundtrips
2. WS: Layer-1 trybuild (S-EV-02a) — proves non-re-export of the inner payload from `overdrive-core::lib.rs` causes E0432 on the short-path import (per ADR-0048 § 2 Layer 1 as amended UI-01; rustc E0446 forbids literal `pub(crate)`)
3. WS: Layer-2 dst-lint clause + its unit test (S-EV-02b) — proves the scanner catches in-crate violations
4. WS: Observation log+skip for `AllocStatusRow` only (S-EV-04 first row) — proves the read-side error path
5. WS: Intent refuse-to-start for `JobEnvelope` (S-EV-03) — proves the asymmetric policy
6. Then replicate S-EV-01 + S-EV-04 across the four remaining envelopes (node_health, service_hydration_results, service_backends, job) — pattern-matching expansion
7. S-EV-06 (xtask coverage gate) — closes the loop: any future envelope without a fixture file fails CI

**Out of scope for DELIVER** (per ADR-0048 § Migration):
- Pre-envelope on-disk redb migration tooling (greenfield single-cut)
- Phase-2+ sub-envelopes on `WorkloadDriver` / `Exec` (rejected per ADR-0048 § 4 / Option 5)

**Definition of Done — DISTILL side**:
- [x] All 7 scenarios written in `test-scenarios.md`
- [x] Walking-skeleton document declares Strategy A and identifies WS scope
- [x] RED scaffold strategy documented
- [x] Critique-dimensions self-review run (Phase 4 dry-run); all dimensions ✓ or rationalised
- [x] No deferrals introduced; no GitHub issues created
- [x] Reconciliation against ADR-0048 verified (no contradictions)
