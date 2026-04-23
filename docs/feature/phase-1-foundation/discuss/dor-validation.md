# Definition of Ready — Validation

**Feature**: `phase-1-foundation`
**Validator**: Luna (product-owner, DISCUSS wave)
**Date**: 2026-04-21

---

## Story: US-01 — Core Identifier Newtypes

| DoR Item | Status | Evidence |
|---|---|---|
| Problem statement clear | PASS | Problem names the engineer, the `String`-as-identifier pain, and why typed identity is load-bearing for whitepaper §8. |
| User/persona identified | PASS | Overdrive platform engineer, primary author of control-plane logic, working inside `crates/overdrive-core`. |
| 3+ domain examples | PASS | Three examples: Ana parsing a job ID from a config, a SPIFFE round-trip, a garbage-input error boundary. Real data. |
| UAT scenarios (3–7) | PASS | Five scenarios: Display/FromStr round-trip, malformed rejection, empty rejection, canonical Display, rkyv stability. |
| AC derived from UAT | PASS | Nine AC items, each traceable to a scenario or a System Constraint. |
| Right-sized | PASS | ≤ 1 day; 5 scenarios; single demonstrable slice (three newtypes). |
| Technical notes | PASS | Case-insensitive rule, structured ParseError, serde routed through Display/FromStr; dependency = none. |
| Dependencies tracked | PASS | None. Slice 1 is the foundation. |
| Outcome KPIs defined | PASS | Who/Does-what/By-how-much/Measured-by/Baseline all specified. |

**Verdict: PASSED (9/9)**

---

## Story: US-02 — Extended Identifier Newtypes

| DoR Item | Status | Evidence |
|---|---|---|
| Problem statement clear | PASS | Explicitly names the re-parsing pain in subsystems and the structural fix. |
| User/persona identified | PASS | Overdrive platform engineer working across `overdrive-core` and future subsystem crates. |
| 3+ domain examples | PASS | SPIFFE URI happy path, ContentHash edge, Region case-insensitivity error boundary — each with real data. |
| UAT scenarios (3–7) | PASS | Five scenarios covering SpiffeId round-trip, rejection, ContentHash length, Region normalisation, static API inspection. |
| AC derived from UAT | PASS | Seven AC items, each traceable. |
| Right-sized | PASS | ≤ 1 day; 5 scenarios; parallelisable with Slice 3. |
| Technical notes | PASS | `SchematicId` canonicalisation documented; `CorrelationKey` derivation noted; dependency = US-01 pattern. |
| Dependencies tracked | PASS | US-01 (pattern). |
| Outcome KPIs defined | PASS | 11 newtypes total; 100% coverage target; static inspection as measurement. |

**Verdict: PASSED (9/9)**

---

## Story: US-03 — IntentStore Trait + LocalStore on Real redb

| DoR Item | Status | Evidence |
|---|---|---|
| Problem statement clear | PASS | Ties directly to whitepaper §4 and `commercial.md` density claim. |
| User/persona identified | PASS | Overdrive platform engineer + single-mode operator (indirect). |
| 3+ domain examples | PASS | Store/get happy path, snapshot migration edge case, corrupted snapshot error boundary. |
| UAT scenarios (3–7) | PASS | Five scenarios: put/get, snapshot round-trip, watch prefix, corrupted snapshot, cold-start envelope. |
| AC derived from UAT | PASS | Eight AC items, each traceable. |
| Right-sized | PASS | ≤ 1 day; 5 scenarios; ships a trait + concrete impl together. |
| Technical notes | PASS | Backing store (real redb), snapshot format (rkyv + versioned header), future-compat note for `RaftStore`. |
| Dependencies tracked | PASS | US-01 (typed keys), US-02 (typed keys for SPIFFE-addressed workloads). |
| Outcome KPIs defined | PASS | Cold start + RSS + round-trip byte-identity all measurable. |

**Verdict: PASSED (9/9)**

---

## Story: US-04 — ObservationStore Trait + SimObservationStore LWW

| DoR Item | Status | Evidence |
|---|---|---|
| Problem statement clear | PASS | Names the Fly-learned lesson and the compiler-enforcement fix. |
| User/persona identified | PASS | Overdrive platform engineer + DST harness (primary consumer in Phase 1). |
| 3+ domain examples | PASS | Cross-peer gossip happy path, concurrent update edge case, type-level rejection error boundary. |
| UAT scenarios (3–7) | PASS | Five scenarios: type-level distinction, LWW convergence, delay determinism, `intent_never_crosses_into_observation`, full-row writes. |
| AC derived from UAT | PASS | Seven AC items, each traceable. |
| Right-sized | PASS | ≤ 1 day; 5 scenarios; parallelisable with US-03. |
| Technical notes | PASS | Sim-only scope, LWW `(clock, writer_id)` tuple model, gossip via SimClock. |
| Dependencies tracked | PASS | US-01, US-02 (typed row keys). |
| Outcome KPIs defined | PASS | 100% compile-time confusion rejection; `intent_never_crosses_into_observation` assert_always. |

**Verdict: PASSED (9/9)**

---

## Story: US-05 — Nondeterminism Traits + CI Lint Gate

| DoR Item | Status | Evidence |
|---|---|---|
| Problem statement clear | PASS | Quotes the whitepaper §21 claim and makes the enforcement requirement explicit. |
| User/persona identified | PASS | Overdrive platform engineer + CI. |
| 3+ domain examples | PASS | Happy path (reconciler uses `&dyn Clock`), edge case (trait extension), error boundary (copy-pasted `std::thread::sleep`). |
| UAT scenarios (3–7) | PASS | Six scenarios: trait existence, Instant-block, rand-block, tokio-net-block, wiring-crate exemption, silent-when-clean. |
| AC derived from UAT | PASS | Nine AC items, each traceable to a scenario. |
| Right-sized | PASS | ≤ 1 day (trait scaffolding exists; this slice completes and wires the gate). |
| Technical notes | PASS | Labelling mechanism suggested, scanner choice left to crafter, fast-feedback requirement stated. |
| Dependencies tracked | PASS | US-03 (IntentStore trait is one of the six), US-04 (ObservationStore trait). |
| Outcome KPIs defined | PASS | 100% smuggling blocked, 0% false positives, weekly regression PR as measurement. |

**Verdict: PASSED (9/9)**

---

## Story: US-06 — turmoil DST Harness + Core Invariants

| DoR Item | Status | Evidence |
|---|---|---|
| Problem statement clear | PASS | Names this slice as the acceptance gate for the whole feature. |
| User/persona identified | PASS | Overdrive platform engineer running DST locally + CI. |
| 3+ domain examples | PASS | Clean clone green run, partition scenario, seed-reproduces-failure. |
| UAT scenarios (3–7) | PASS | Seven scenarios: clean-clone green, same-seed identity, failure format, reproduction identity, CI-fails-on-red, real-LocalStore composition, `intent_never_crosses_into_observation` assert_always. |
| AC derived from UAT | PASS | Twelve AC items. **Intentional exception to the 3–7 right-sizing heuristic**: this slice is the acceptance gate for all prior slices; its AC count reflects integration-check coverage, not scope creep. All twelve are testable. |
| Right-sized | PASS (with note) | ≤ 1 day of integration work on top of Slices 1–5. No net-new logic; the logic lives in prior slices. |
| Technical notes | PASS | Turmoil foundation, `single_leader` stub note (retired in Phase 2), CLI UX rules. |
| Dependencies tracked | PASS | US-01, US-02, US-03, US-04, US-05. |
| Outcome KPIs defined | PASS | Wall-clock < 60s, 100% reproduction rate, self-test identity. |

**Verdict: PASSED (9/9, with the AC-count note documented above)**

---

## Feature-level summary

| Story | DoR | Blockers |
|---|---|---|
| US-01 Core Identifier Newtypes | PASSED | — |
| US-02 Extended Identifier Newtypes | PASSED | — |
| US-03 IntentStore Trait + LocalStore | PASSED | — |
| US-04 ObservationStore + SimObservationStore | PASSED | — |
| US-05 Nondeterminism Traits + Lint Gate | PASSED | — |
| US-06 turmoil DST Harness + Invariants | PASSED | — |

**Overall: PASSED (6/6 stories)**

No DoR blockers. Feature is ready for DESIGN wave handoff.

## Changelog

| Date | Change |
|---|---|
| 2026-04-21 | Initial DoR validation for phase-1-foundation. |
