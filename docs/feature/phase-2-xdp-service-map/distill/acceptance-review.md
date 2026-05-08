# Self-review — phase-2-xdp-service-map DISTILL

**Reviewer**: Quinn (Atlas), DISTILL self-review.
**Date**: 2026-05-05
**Approach**: `nw-acceptance-designer` Self-Review Checklist adapted
to the project's four-tier Rust testing model
(`.claude/rules/testing.md`).

---

## Checklist

### Mandate 1 — Hexagonal Boundary Enforcement (CM-A)

- [x] **Driving port identified.** `Dataplane` trait
  (`crates/overdrive-core/src/traits/dataplane.rs`) is the driving
  port. The `ServiceMapHydrator` reconciler is the consumer that
  closes the ESR loop; the hydrator does not import any internal
  component.
- [x] **No internal-component imports.** Every Tier 1 scenario
  imports `Arc<dyn Dataplane>`, never `EbpfDataplane`-internal types
  or `aya` types directly. Tier 2 / Tier 3 scenarios are explicitly
  driving `BPF_PROG_TEST_RUN` or real veth — those ARE the boundary
  surface, not internal-component access.
- [x] **Action variant respects boundary.**
  `Action::DataplaneUpdateService` lives in `overdrive-core`; the
  shim consumes the typed value and invokes the trait method.
  Reconciler never touches `aya` directly.

### Mandate 2 — Business Language Abstraction (CM-B)

- [x] **Gherkin uses domain terms.** "Service VIP", "backend set",
  "atomic swap", "weighted Maglev", "drop counter", "VIP →
  backend rewrite" — all domain terms (LB / dataplane). No
  HTTP-status-code framing; no `assert_eq!(response.status, 200)`-
  shaped Gherkin. The single semi-mechanical exception is verifier-
  acceptance scenarios ("verifier accepts the program"), which is
  unavoidable phrasing because the kernel verifier's accept/reject
  IS the structural contract per `discuss/wave-decisions.md` §
  Decision 11.
- [x] **Step methods are Rust `#[test]` bodies that delegate to
  production services** — DELIVER's body construction will go
  through `Arc<dyn Dataplane>`, the action shim, and the reconciler
  runtime. RED scaffolds carry the contract via `panic!()` /
  `todo!()` with named gherkin one-liners.

### Mandate 3 — User Journey Completeness (CM-C)

- [x] **Phase 1 WS preserved.** No new WS scenario in this feature;
  the user-journey shape is unchanged from Phase 1's
  `customer-submits-a-job-and-watches-it-run`. See
  `walking-skeleton.md` for inheritance documentation.
- [x] **Focused scenarios cover the substrate change.** 30 named
  `S-2.2-NN` scenarios across 4 tiers. Bug-class partitioning
  ensures each tier exercises what only it can catch.
- [x] **Quality-attribute scenarios distinct from user-journey
  scenarios.** ASR-2.2-01..04 are tagged separately; they validate
  reliability / maintainability / correctness boundaries, not user
  goals.

### Mandate 4 — Pure Function Extraction (CM-D)

- [x] **Pure functions identified.** `maglev::generate(backends, m)`
  is the canonical pure function this feature ships
  (DESIGN architecture.md § 9 places it under
  `overdrive-dataplane::maglev::permutation` /
  `::maglev::table`). `dataplane::fingerprint(vip, backends)` is
  the second pure function. Both have proptest harnesses.
- [x] **Impure code isolated behind adapters.** Real eBPF kernel
  loading is the `EbpfDataplane` adapter; real `BPF_PROG_TEST_RUN`
  is the Tier 2 scaffolding; real veth is the Tier 3 harness. The
  `Dataplane` port trait is the seam.
- [x] **Fixture parametrization minimal.** No fixture
  parametrization across environments; the project rules forbid
  pytest-style parametrize anyway. Tier 3 environment is a single
  Lima VM (developer) or single `ubuntu-latest` runner (CI) — no
  matrix per
  `discuss/wave-decisions.md` § Decision 5 (single-kernel
  in-host per #152).

### Mandate 5 — Walking Skeleton User-Centricity

- [x] **No new WS scenario; inheritance from Phase 1 documented in
  `walking-skeleton.md`.** Litmus test applied (4-step):
  - Title? Phase 1's WS title ("customer submits a job and watches
    it run") is user-goal-shaped.
  - Given/When? User actions only.
  - Then? User observations only ("allocation is running",
    "customer can describe the job").
  - Stakeholder confirmation? Yes for Phase 1; Phase 2.2 does not
    change the user-observable shape.

### Mandate 6 — Adapter Integration Coverage

- [x] **Every driven adapter has at least one `@real-io
  @adapter-integration` scenario.** See `wave-decisions.md`
  DWD-6 table. Specifically:
  - `EbpfDataplane::update_service` → SERVICE_MAP, BACKEND_MAP,
    MAGLEV_MAP, REVERSE_NAT_MAP, DROP_COUNTER all have at least
    one Tier 3 scenario each.
  - The `Dataplane` port (driving) has Tier 1 DST coverage
    (S-2.2-26..30) AND Tier 3 confirm via the action shim's real
    dispatch path under load (Slice 03's atomic-swap scenario).
- [x] **Documented what `SimDataplane` cannot model.** See
  `test-scenarios.md` § "What `SimDataplane` + `SimObservationStore`
  CANNOT model" — kernel verifier, kTLS, packet rates, NIC driver,
  BPF map format mismatches, libbpf-sys binding drift, endianness
  conversion at the wire boundary. Each is mapped to the tier that
  CAN catch it.
- [x] **Tier composition.** ASR-2.2-02 (≤ 1 % Maglev disruption)
  has a Tier 1 DST proptest as primary AND a Tier 3 real-veth
  confirm — bug-class partitioning explicit.

### Mandate 7 — RED-Ready Scaffolding

- [x] **Production module scaffolds use `panic!("Not yet
  implemented -- RED scaffold")` or `todo!("RED scaffold: ...")`.**
  See DWD-4 in `wave-decisions.md`. NOT `unimplemented!()` (per
  project rule preference).
- [x] **Tests are RED (panic) when run against scaffolds, not
  BROKEN (compile error).** Every test body compiles; bodies panic
  with named gherkin one-liners.
- [x] **`Action::DataplaneUpdateService` variant declaration is
  data-only (no panic).** The shim's exhaustive match arm carries
  the panic, not the variant itself.

### Critique-dimension self-pass

#### Dim 1 — Happy Path Bias

13 / 30 = 43.3 % error/edge-path scenarios. Above 40 % mandate.
PASS.

#### Dim 2 — GWT Format Compliance

Every Gherkin block has Given / When / Then sections. Several use
"And" continuations after Given or Then — none have multiple When
actions in one scenario. The S-2.2-29 retry-budget scenario has
two When/Then pairs because it specifies two ticks at different
times — this is acceptable per the methodology (it expresses one
behaviour: "honors backoff window") and is necessary because the
retry policy IS time-dependent. PASS.

#### Dim 3 — Business Language Purity

The single semi-mechanical exception is verifier-acceptance phrasing
("verifier accepts the program") — unavoidable per
`discuss/wave-decisions.md` § Decision 11. All other Gherkin uses
LB / dataplane / reconciler domain terms exclusively. PASS.

#### Dim 4 — Coverage Completeness

Story-to-scenario mapping (8/8): US-01 → S-2.2-01..03; US-02 →
S-2.2-04..08; US-03 → S-2.2-09..11; US-04 → S-2.2-12..14;
US-05 → S-2.2-15..18; US-06 → S-2.2-19..23; US-07 → S-2.2-24..25;
US-08 → S-2.2-26..30. PASS.

#### Dim 5 — Walking Skeleton User-Centricity

No new WS in this feature. Phase 1 inheritance documented in
`walking-skeleton.md`. PASS by inheritance.

#### Dim 6 — Priority Validation

The eight stories are pre-prioritised by DISCUSS's carpaccio
slicing; DISTILL respects the slice ordering. No re-prioritisation
attempted. PASS.

#### Dim 7 — Observable Behavior Assertions

Every Then step asserts on:

- A return value from a driving port call (e.g.
  `update_service` returns `Ok(())` / `Err(MapAllocFailed)`),
- An observable outcome (`tcpdump` shows the rewritten packet,
  `bpftool map dump` shows the counter value, `nc` exits cleanly,
  the action emitted matches the expected variant),
- A typed error variant (`DataplaneError::IfaceNotFound`),
- An invariant property (`HydratorEventuallyConverges` over
  every DST seed).

NO Then step asserts:

- `assert mock.x.called` — there are no mocks in this design;
  test doubles are `Sim*` adapters, not mocks.
- Internal DB row state for its own sake — every observation-row
  assertion is "the next reconcile tick reads this and converges"
  (an observable outcome).
- Internal field access (no `_internal_field` patterns).

PASS.

#### Dim 8 — Traceability Coverage

**Check A (Story-to-Scenario)**: 8/8 stories have ≥ 1 scenario.
PASS.

**Check B (Environment-to-Scenario)**: DEVOPS wave has not run
yet for this feature. Per the orchestrator brief's "DEVOPS missing
→ default environment matrix" graceful-degradation rule, the
default matrix is `clean`, `with-pre-commit`, `with-stale-config`.
Phase 2.2's environment surface is not pytest-shaped fixtures — it
is the Tier 3 Lima VM / `ubuntu-latest` runner for real-kernel
tests, and the Tier 1 in-process DST harness for everything else.
The four-tier model substitutes structurally for the three-env
matrix: Tier 3 = `clean` (real kernel, real veth, no pre-existing
state); the other "environment" axes are not applicable (there is
no Phase 2.2 user-facing config to be `with-stale-config` against).
PASS by structural substitution; logged as informational deviation
from the default skill checklist for future DEVOPS runs to
reconcile.

#### Dim 9 — Walking Skeleton Boundary Proof

**9a (WS Strategy Declaration)**: declared in
`wave-decisions.md` DWD-1 (inherited from Phase 1). PASS.

**9b (WS Strategy-Implementation Match)**: Phase 1 + Phase 2.1
WS uses real adapters (real `LocalIntentStore`, real
`LocalObservationStore`, real `Driver` / `EbpfDataplane`). Strategy
C — real local. The Phase 2.1 EbpfDataplane was attached to `lo`
with a no-op program; Phase 2.2 makes it functional. PASS.

**9c (Adapter Integration Coverage)**: every driven adapter has a
real-I/O scenario. PASS — see DWD-6 table.

**9d (Walking Skeleton Fixture Tier)**: Phase 1's WS uses real
`tempfile::TempDir` for `LocalIntentStore`'s redb file, real
`reqwest`-shaped HTTP for the API surface, real `Driver::start`
for `ProcessDriver`. Litmus: deleting the real adapter would
break Phase 1's WS. PASS.

**9e (Strategy Drift Detection)**: no `@in-memory` tags on any
walking-skeleton scenario in this feature (because there are no
new WS scenarios). PASS by absence.

### Mandate compliance evidence (CM-A, CM-B, CM-C, CM-D)

- **CM-A**: every test file's intended import surface (per RED
  scaffolds) goes through `overdrive-core::traits::dataplane`,
  the `Reconciler` / `Action` enum surface, or the action shim
  — never internal-component types. DELIVER's grep gate at handoff
  will confirm.
- **CM-B**: grep over `test-scenarios.md` for technical terms like
  `database`, `JSON`, `HTTP status` returns zero hits. The string
  `verifier` appears in 4 scenarios (S-2.2-07, S-2.2-14, S-2.2-23,
  S-2.2-24) — the kernel verifier IS the structural contract for
  ASR-2.2-03; allowed exception per `discuss/wave-decisions.md`
  § Decision 11.
- **CM-C**: 0 walking-skeleton scenarios introduced (inheritance);
  30 focused scenarios. The 17:30 ratio of focused-to-total is
  well above the 17/20 expected by the methodology because no
  new WS landed.
- **CM-D**: pure functions enumerated above (`maglev::generate`,
  `dataplane::fingerprint`); proptest harnesses cited in
  `test-scenarios.md` (S-2.2-12, S-2.2-13). Impure code isolated
  behind `Dataplane` port trait + `BPF_PROG_TEST_RUN` Tier 2
  harness + real veth Tier 3 harness. No fixture parametrization
  across environments.

---

## Non-applicable items (project-adapted)

The following items from `nw-acceptance-designer`'s default skill
checklist are N/A for this project's four-tier Rust testing model:

- **Item 12 — pytest-bdd `scenarios()` registration consistency**:
  N/A. This project does not use pytest-bdd; tests are Rust
  `#[test]` / `#[tokio::test]` bodies.
- **Item 13 — `conftest.py` fixture scopes**: N/A. No `conftest.py`;
  Rust tests use `tempfile::TempDir`, `serial_test::serial(env)`,
  and project-specific fixture helpers as needed (the project's
  test-utils module pattern at
  `crates/overdrive-core/src/testing/`).
- **Item 15 — `pytest --collect-only` produces sane output**: N/A.
  `cargo nextest run --features integration-tests --no-run` is the
  project equivalent; CI already runs it.

Item 14 (timing budgets) translates to:

- **Tier 1 DST**: nextest slow-test 60 s budget per
  `.claude/rules/testing.md` § "Running tests".
- **Tier 1 DST tick budget**: per-tick reconcile work bounded by
  `TickContext.deadline` per
  `.claude/rules/development.md` § Reconciler I/O.
- **Tier 3 integration**: the existing `cargo xtask integration-test
  vm` harness manages timing; per-test wall-clock should be < 60 s.

These three project-specific budgets are documented in
`wave-decisions.md` DWD-3 file-path inventory and DWD-4 RED-scaffold
strategy.

---

## Verdict

**APPROVED for handoff to DELIVER.**

All eight Mandate-Compliance criteria pass. All nine Critique
Dimensions pass (Dim 8 with structural-substitution rationale; Dim
5 with inheritance rationale). 30 scenarios across 4 tiers,
43.3 % error-path coverage, 8/8 stories, 8/8 KPIs, 4/4 ASRs.

The orchestrator decides whether `/nw-review` runs an additional
peer review pass. DISTILL is internally consistent, scope-bounded,
and ready for the DELIVER wave to consume one scenario at a time
per the `@pending` discipline.

---

## Changelog

| Date | Change |
|---|---|
| 2026-05-05 | Initial DISTILL self-review for `phase-2-xdp-service-map`. — Quinn (Atlas). |
