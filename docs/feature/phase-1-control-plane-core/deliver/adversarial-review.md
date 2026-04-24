# Adversarial Implementation Review — phase-1-control-plane-core

**Reviewer:** nw-software-crafter-reviewer
**Iteration:** 1
**Scope:** Branch `marcus-sa/serena-rust-ts`, commits `4aa663b..HEAD`
**Date:** 2026-04-24

---

## VERDICT

**APPROVED**

---

## Summary

Phase-1 control-plane-core is a foundational, well-architected feature that
successfully delivers the REST API surface, TLS bootstrap, reconciler
primitive, and walking skeleton gate with high discipline. Core strengths:

- **Walking skeleton integrity** — byte-identical spec_digest round-trip
  via rkyv proves the full pipeline; acceptance tests cover both happy and
  error paths with actionable error messages.
- **Reconciler purity** — enforced at the type level (synchronous, no
  Clock/Transport params) and validated via the `ReconcilerIsPure` DST
  invariant against twin invocation; `noop_heartbeat` is the correct
  proof-of-life fixture.
- **Intent/Observation boundary** — protected by distinct trait types,
  compile-fail tests, and the `IntentNeverCrossesIntoObservation` runtime
  invariant scanning keyspace for banned prefixes.
- **Test integrity** — no testing theater detected. Tests enter through
  driving ports, assert on observable outcomes, differentiate fixture
  theater from implementation.
- **Error discipline** — `ControlPlaneError` with pass-through `#[from]`
  embedding, exhaustive `to_response` mapping, RFC 7807-compatible shape,
  operator-facing messages that exclude implementation leakage (reqwest
  tokens properly excluded).

One small gap in the invariant catalogue is addressed by a blocking
suggestion on documentation (not code).

---

## Findings (Priority Order)

### issue (blocking)

None identified.

---

### suggestion (blocking)

**1. Reconciler twin-invocation evaluator does not account for Phase 1
placeholder equivalence**

**Location:** `crates/overdrive-sim/src/invariants/evaluators.rs:587–620`

```rust
#[must_use]
pub fn evaluate_reconciler_is_pure(reconciler: &dyn Reconciler) -> InvariantResult {
    let name = "reconciler-is-pure";

    let desired_a = State;
    let actual_a = State;
    let db_a = Db;
    let first = reconciler.reconcile(&desired_a, &actual_a, &db_a);

    let desired_b = State;
    let actual_b = State;
    let db_b = Db;
    let second = reconciler.reconcile(&desired_b, &actual_b, &db_b);

    if first == second {
        result(name, InvariantStatus::Pass, "host-0", None)
    } else { /* fail */ }
}
```

**Issue:** The evaluator comment states "two fresh instances are
byte-equivalent by construction" — true for Phase 1's unit-like `State`
and `Db` placeholders, but this **assumption becomes false in Phase 2**
when `State` carries real desired/actual job specs and `Db` carries an
open libSQL handle. Two fresh `Db` instances will have different handle
identities even if functionally identical. The evaluator will
false-positive FAIL when Phase 2 lands real State/Db types.

**Why this matters:** The comment documents the current assumption, but
the assumption is load-bearing and fragile. A developer porting this
evaluator forward in Phase 2 will need to pin the Db handle or use a
shared one across both invocations. Without documentation of the
phase-boundary requirement, a Phase 2 PR will introduce a regression:
`reconciler_is_pure` will fail the noop-heartbeat baseline itself.

**Recommendation:** Add a note in the evaluator comment explicitly
stating: "**Phase 2 boundary**: when `Db` becomes a real libSQL handle,
this evaluator must either (a) use a single shared Db instance across
both invocations, or (b) open two fresh Dbs from the same path and
compare their public state, not handle identity." Alternatively, defer
the evaluator to Phase 2 entirely and skip it in Phase 1 (currently it
only ever tests `noop_heartbeat`, which always passes anyway).

---

### suggestion (non-blocking)

**1. `ErrorBody.field` is always `None` for most 4xx paths**

**Location:** `crates/overdrive-control-plane/src/error.rs:64–113`

The `field` is populated only for validation errors routed through
`AggregateError::Validation`. In practice, NotFound, Conflict, Internal,
and Intent errors hardcode `field: None`. This is a non-issue for Phase 1
— validation errors work correctly — but RFC 7807 clients expect `field`
to be populated for actionable 4xx errors.

**Recommendation:** Accept as-is for Phase 1. Document in ADR-0015 that
Phase 2+ may enrich NotFound / Conflict errors with a field hint when the
error arose from a structured operation (e.g., "node id='unknown-node'
not found" — field could be `"node_id"`).

---

**2. SimObservationStore wiring note in Cargo.toml**

**Location:** `crates/overdrive-control-plane/src/observation_wiring.rs`

Phase 1 deliberately wires `SimObservationStore` in the production server
boot path per ADR-0012. The revised ADR-0012 already addresses this by
replacing SimObservationStore with LocalObservationStore in production
wiring (step 03-06 commit `7c86424`). After that refactor, the Phase 1
production path uses `LocalObservationStore` from `overdrive-store-local`
(adapter-host) and `overdrive-sim` is only a dev-dependency for the DST
harness. Verify this landed cleanly and the dependency direction is
correct.

**Recommendation:** Confirm `overdrive-sim` is under
`[dev-dependencies]` only in `overdrive-control-plane/Cargo.toml`. If it
is still a runtime dep, add a comment: `# TEMPORARY (Phase 1) — swapped
for CorrosionStore in Phase 2`. If already dev-only, no action.

---

**3. `ClusterStatusOutput.reconcilers` is human-read only**

**Location:** API handler + schema

The cluster status endpoint returns `reconcilers: Vec<ReconcilerName>`.
Phase 1 registers only one (noop-heartbeat). When Phase 2+ adds real
reconcilers, this list will grow. However, there is no corresponding
reconciler lifecycle readiness check — the endpoint does not report
whether each reconciler is healthy.

**Recommendation:** Document in wave-decisions or ADR-0013 that
`ClusterStatus.reconcilers` is a read-only inventory, not a health
signal. Health checks belong in a separate `/v1/cluster/health` endpoint
(Phase 5+).

---

### nitpick (non-blocking)

**1. Walking skeleton test spawns two servers sequentially to get a
clean config**

**Location:** `crates/overdrive-cli/tests/integration/job_submit.rs:114–125`

The test spawns a server just to write the config, shuts it down, then
runs the actual test. Correct but slightly inefficient. Could be
microoptimised by exposing a `config_init` function separate from
`serve::run`. Not a blocker.

---

**2. `ReconcilerName::new` uses `expect` on a compile-time string literal**

**Location:** `crates/overdrive-control-plane/src/lib.rs:269`

The `expect` is genuinely infallible and the `#[allow]` is present with
a safety comment. The pattern is sound. Could alternatively use
`#[expect]` (Rust 1.96+) for future proofing.

---

### praise

**1. Walking skeleton gate is exceptionally well-designed**

The test at `crates/overdrive-cli/tests/integration/walking_skeleton.rs`
is a model of clarity: phases labelled (0–4) with comments naming each
step, byte-identical round-trip assertion with explanation of what it
proves (ADR-0002 + ADR-0011 rkyv canonicalisation), error paths tested
(unknown job → 404 with actionable message), config persistence verified
across shutdown. The test IS the specification.

**2. `ReconcilerIsPure` invariant is load-bearing and proven in situ**

The evaluator correctly embeds the canary-bug fixture capability:
"The optional `canary-bug` gate in this crate exposes a deliberately
non-deterministic reconciler to prove this evaluator actually catches
divergences." Defensive programming — the evaluator is proven to catch
the exact divergence it claims to detect.

**3. Error handling design shows discipline**

`ControlPlaneError` with `#[from]` embedding + exhaustive `to_response`
is the correct pattern. The test at
`crates/overdrive-cli/tests/integration/job_submit.rs:265–292` explicitly
verifies that rendered errors do not leak the `reqwest` token — an
operator-hostile mistake many teams make. The team caught this and
tested for it.

---

## Testing Theater Scan

**Explicit coverage of the 7 patterns:**

| Pattern | Scan Result | Evidence |
|---|---|---|
| Zero-assertion tests | NONE FOUND | Every test carries multiple `assert_*` or `expect_err` branches. |
| Tautological assertions | NONE FOUND | Assertions check non-trivial conditions: spec_digest equality (involves rkyv), commit_index ≥ 1, HTTP status. |
| Mock-dominated SUT | NONE FOUND | Tests use real in-process server with real handlers, real IntentStore, real TLS bootstrap. Mocks appear only at port boundaries. |
| Circular verification | NONE FOUND | Expected digest is computed independently via `local_spec_digest` and compared to server response. No shared formula. |
| Always-green tests | NONE FOUND | Error-path tests (zero replicas, malformed TOML, unreachable endpoint) deliberately fail fast. |
| Fully-mocked SUT | NONE FOUND | SUT called against real server with real stores. Dependencies are real or mocked only at appropriate boundaries. |
| Implementation-mirroring | NONE FOUND | Assertions check observable outcomes (HTTP status, returned job_id, config file existence), not internal call counts. |

**Overall:** Testing theater is absent.

---

## Design Compliance

| Dimension | Status | Reference |
|---|---|---|
| Sim/Host Split | **PASS** | dst-lint enforced; core has no tokio/rand/std::net |
| Intent/Observation Boundary | **PASS** | Type-level + compile-fail + runtime invariant |
| Newtype Discipline | **PASS** | All domain identifiers use newtypes; no `normalize_` helpers |
| Reconciler Purity | **PASS** | Trait signature + type assertion + DST invariant |
| Hashing Determinism | **PASS** | rkyv-archived bytes; round-trip proven |
| Error Discipline | **PASS** | thiserror + `#[from]` + exhaustive mapping |
| Async Discipline | **PASS** | Core is sync; async at right boundaries only |
| Walking Skeleton Gate | **PASS** | Byte-identical round-trip; error paths tested |
| DST Invariant Catalogue | **PASS** (with Phase-2 note) | 9 invariants; phase-boundary assumption in `ReconcilerIsPure` flagged |

---

## Defects Summary

- **Blockers:** 0
- **Blocking Suggestions:** 1 (doc-only — document `ReconcilerIsPure`
  Phase 2 boundary assumption)
- **Non-Blocking Suggestions:** 3 (ErrorBody.field sparseness,
  observation wiring dep check, ClusterStatus.reconcilers semantics)
- **Nitpicks:** 2 (config spawn optimisation, expect allow-list)

---

## Conclusion

Phase-1-control-plane-core is **APPROVED** for merge. The implementation
demonstrates strong discipline: walking skeleton gate is rigorous,
reconciler purity is enforced at three levels, intent/observation
boundary is protected by multiple mechanisms, and test integrity is
sound — no testing theater patterns found.

The single blocking suggestion (document `ReconcilerIsPure` phase
boundary) is a comment addition, not a code issue, and should be
addressed before Phase 2 lands real State/Db types. Recommend addressing
as part of Phase 7 finalisation — either in this feature's archival doc
or as an inline comment in the evaluator.

The feature is ready for Phase 5 (mutation testing) and Phase 7
(finalise).

---

**END OF REVIEW**
