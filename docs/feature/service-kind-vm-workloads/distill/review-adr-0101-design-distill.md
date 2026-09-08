# ADR-0101 revision 3 — consolidated DESIGN + DISTILL review

**Review ID:** `consolidated_rev_2026-09-08_adr-0101_iter2`  
**Reviewer:** `nw-acceptance-designer-reviewer` + `nw-solution-architect-reviewer`  
**Model:** GPT5.6 Luna, maximum thinking  
**Iteration:** 2, focused re-review of CD-0101-01/02; iteration 1 retained below  
**Reviewed:** 2026-09-08  
**Iteration 1 verdict:** **CHANGES_REQUESTED**  
**Current verdict:** **APPROVED**

## Iteration 1 — initial consolidated review

## Scope and boundary

This review covers only the ADR-0101 revision-3 authoritative Service backend
projection amendment and its newly authored DISTILL package:

- `docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md`
- `docs/feature/service-kind-vm-workloads/design/backend-eligibility-convergence-ruling.md`
- `docs/feature/service-kind-vm-workloads/design/review-adr-0101.md`
- `docs/feature/service-kind-vm-workloads/distill/adr-0101-acceptance.md`
- `docs/feature/service-kind-vm-workloads/distill/red-classification.md`
- `crates/overdrive-sim/tests/integration/service_backend_projection.rs`
- its `crates/overdrive-sim/tests/integration.rs` registration
- the paired source-local property in
  `crates/overdrive-reconcilers/src/service_lifecycle.rs`
- the ADR-0101 DISTILL section of
  `docs/feature/service-kind-vm-workloads/feature-delta.md`

The accepted revision-3 architecture, existing production paths, the original
seed-257209 diagnostic, and the separate native E09 evidence were read as
context. No production implementation is expected at this gate, so the current
pre-correction ServiceLifecycle and BackendDiscoveryBridge implementation is not
itself a finding. No production, test, design, harness, or unrelated
documentation file was changed by this review; this artifact is the only owned
output.

The verdict is not a re-review of the broader phase-02 feature, its existing
harness defects, or unrelated roadmap steps.

## Decision summary

The DESIGN is sound and remains approved within its already completed
independent DESIGN review. ADR-0101 revision 3 has a precise owner, exact public
and private Rust shape, direct bridge retirement, complete all-listener row
projection, observed-row comparison, retained terminal veto, serial action
ordering, and asynchronous consumer boundary. The alternatives and
consequences are sufficient, and the selected change is the smallest mechanism
that closes the proven same-ID restart sequence.

The DISTILL package is substantially credible: the new fixture uses the real
runtime, redb View persistence, action shim, ProbeRunner, validated Service
intent and allocator ports, List/Watch consumers, and the supported local
hydrator path. Faults are injected through existing observation/prober ports.
The original diagnostic remains unchanged and the new suite's failures are
behavioral, not setup or compile failures. The package is also explicit that
suffix assertions behind a first failing assertion have not executed.

Two bounded acceptance-oracle gaps nevertheless allow a contract-violating
implementation to become green: the exact LWW stamp construction is not
asserted at the ServiceLifecycle call site, and the allocator-issued row VIP is
never compared with the published row. These are test/design alignment issues,
not requests for a new production API or persistence mechanism. The original
acceptance designer must close them before this consolidated gate can approve.

## Evidence and focused commands

| Evidence | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(service_backend_projection)' --no-capture --no-fail-fast` | Nextest `03f96a38-150c-44b4-abc5-bd24aac6c34a`; 12 run, 2 passed, 10 failed, 24 skipped; outer exit 1. Every failure reached a domain assertion after legitimate setup: missing listener rows, empty-row publication, write repair, startup withdrawal, action pairing, hydrator handoff, or lifecycle wake. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --lib -E 'test(backend_policy_preserves)' --no-capture` | Nextest `4f596916-02a5-45d2-8700-446d812bc3d0`; 1 passed, 38 skipped; exit 0. The fixed proptest seed 257222 traversed 128 cases and all 12 finite policy combinations per case. |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test e09_v2_failed_service_reachability_spike --no-capture --no-fail-fast` | Nextest `dec2f001-1fe5-4516-b466-0c34888c6a60`; seed 257209 reproduced the unchanged production-owner same-ID restart defect and failed at the retained terminal-veto assertion. |
| `crates/overdrive-sim/tests/integration.rs:20,25` | The new integration module inherits the existing `integration-tests` feature gate and is registered through the required inline module. |
| Contract-shape scan | 12 `#[tokio::test]` functions and 12 exact `/// CONTRACT_SHAPE: bounded-change.` declarations in `service_backend_projection.rs`; the paired source-local property carries the exact `/// CONTRACT_SHAPE: pure-function.` declaration. |
| Boundary scan | The new Sim fixture contains no `Command::spawn`, production-binary invocation, expectation-runner import, or `cargo test`/test-binary launch. It uses `run_convergence_tick` and existing adapters in-process. |

No full-workspace, native-100, mutation, or DES command was run, as required by
the bounded review scope.

## Strengths

- The user-selected contract is unambiguous: ServiceLifecycle is the sole
  complete `ServiceBackendRow` publisher and BackendDiscoveryBridge is retired;
  startup failure is a lifecycle fact, not an all-consumer acknowledgement.
- D2 pins the exact three-field `ServiceLifecycleState`, replacement
  `ServiceDataplaneIdentity`, `backend_ip`, View removal, unchanged reconciler
  signatures, and the prohibition on invented public surface
  (`adr-0101...md:91-117`).
- D3/D4 preserve the real authority boundaries: current Service intent and
  allocator listeners, Running membership, allocation-scoped readiness, the
  retained terminal veto, complete observed-row comparison, ordered
  write/hydrator pairs, and the existing serial error-isolation path
  (`adr-0101...md:119-200`).
- The architecture review's iteration 3 independently resolved the only prior
  owner contradiction. The corrected feature-delta driving-port row now also
  states sole ServiceLifecycle publication and direct bridge retirement
  (`review-adr-0101.md:607-682`, `feature-delta.md:505`).
- The composed Sim world lets real production owners author the observed
  allocation, probe, terminal, backend, View, action, and consumer state. It
  does not seed a disputed terminal row, health bit, fingerprint, or favorable
  final writer (`adr-0101-acceptance.md:30-54`).
- Coverage is not happy-path biased: BE-04/05/06/07/11/12 are reachable fault,
  withdrawal, restart, ordering, or owner-wake sequences; BE-08/09/10 cover
  readiness and asynchronous consumer recovery; BE-P1 covers generated policy
  edges.
- The red classification correctly distinguishes the fresh behavioral REDs from
  setup failures and explicitly preserves the original diagnostic as a fresh,
  unchanged RED witness (`red-classification.md:11-78`).

## Gate assessment

| Gate / dimension | Result | Evidence and rationale |
|---|---|---|
| ADR context and priority | PASS | Seed 257209 and the retained native E09 evidence identify the actual same-ID restart/LWW-authority bottleneck; the ruling explains why a shared health overlay is insufficient. |
| Alternatives and consequences | PASS | The ADR evaluates the shared overlay, one-publisher fact join, consumer-side joins, and selected sole publisher, including reliability, maintainability, work-per-tick, and testability consequences. |
| Feasibility and testability | PASS | No new technology, public port, persistence subsystem, acknowledgement protocol, or deployment unit is introduced; existing ports/adapters are sufficient for the bounded fixture. |
| Exact API and ownership contract | PASS for DESIGN; implementation gate pending | The exact contract is pinned. DELIVER must still compile-check direct retirement and every listed compiler-fallout site; this review does not treat the intentionally old implementation as a RED defect. |
| Happy-path bias | PASS | Ten of twelve composed scenarios are behavioral failure, edge, restart, ordering, or consumer cases; two are explicit controls. |
| GWT and domain narrative | PASS with Rust project exception | The acceptance table gives one driving action per scenario. Rust names necessarily identify ServiceLifecycle, ProbeRunner, rows, and actions because this amendment's contract is an owner/action boundary; those terms are not used to replace the stakeholder-visible consumer or lifecycle outcome. |
| Production composition | PASS | The fixture calls the production convergence entry point, runtime, View store, action shim, ProbeRunner, resolver/DNS consumers, and hydrator; Sim substitutes only external process/prober/clock/dataplane ports. |
| Fault causality | PASS | Probe outcomes and observation write refusals are driven through existing ports; production authors the resulting rows, terminal state, retained View, and consumer state. |
| Safety and convergence oracle | PASS with the two findings below | BE-05/06 retain the veto across same-ID restart and a rejected withdrawal; BE-09/10 exercise asynchronous consumers. The missing exact stamp/VIP assertions weaken the oracle for two specific D1/D4 values but do not turn it into a favorable-last-writer test. |
| All-listener/structural complements | PASS with bounded gaps below | BE-02 and BE-08 use one/three listeners and same-port TCP/UDP; BE-07 inspects ordered write/handoff structure and then dispatches it through the real shim. |
| Generated cases and anti-vacuity | PASS | BE-P1 uses generated counter/threshold values, saturation edges, all readiness/veto/status combinations, and unrelated View inputs; it is correctly described as a policy complement rather than reachability proof. |
| Contract Shape | PASS | Every live composed test has the required bounded-change declaration and the source-local pure property has the exact pure-function declaration. |
| Tier separation | PASS | Sim tests stay in-process; native E09 remains the VM/kernel/wire boundary; no Rust test launches the product binary and no expectation runner imports a crate or test harness. |
| RED honesty and suffix accounting | PASS | The focused run independently reproduced 2 green controls and 10 behavioral failures. The acceptance documents do not claim the unexecuted suffixes are green. |

## Findings

### CD-0101-01 — ServiceLifecycle stamp oracle does not prove D4's exact dominating construction

**Severity:** HIGH — blocking for consolidated approval  
**Dimensions:** exact contract alignment, observable structural oracle, LWW safety  
**Status:** Open; acceptance-designer remediation required

ADR-0101 D4 requires every changed row to use exactly
`LogicalTimestamp::dominating(tick.tick, identity.writer.clone(), observed_stamp)`
and makes observed-row readback, rather than an emit marker, the equality
reference (`adr-0101-service-backend-health-observed-convergence.md:167-175`).
The acceptance table correspondingly claims that BE-07 checks “dominating
stamps” (`distill/adr-0101-acceptance.md:69`).

The executable assertion is weaker. In
`service_backend_projection.rs:590-597`, BE-07 only checks that the new
counter is greater than the prior row's counter. It does not assert the writer,
the tick-floor relationship, or the prior-plus-one behavior when the observed
row is ahead of the current tick. The fixture's normal tick sequence climbs
monotonically, so a ServiceLifecycle implementation that derived the stamp from
the tick alone could satisfy the current assertion. The existing core
`LogicalTimestamp` tests prove the utility's semantics, but not that this
ServiceLifecycle call site supplies the observed Service row and the selected
identity writer.

This is a contract-coverage finding, not a claim that the current pre-correction
production code has a new bug. The production failure mode is nevertheless
reachable: `LogicalTimestamp` documents that convergence ticks reset on process
restart while durable row counters do not
(`overdrive-core/src/traits/observation_store.rs:265-275`). The new BE-06
runtime reload does not reset `World::tick`, so its repair suffix does not
exercise that cross-restart ordering boundary.

**Required disposition:** strengthen the existing BE-07 structural oracle (or a
bounded companion using only existing observation/action ports) to assert the
passed writer and exact dominating counter behavior, including an observed
prior ahead of the current tick. Do not add a public API, generation field,
persistence mechanism, or test-only production seam. If the existing ports
cannot reach the high-prior case, record that bounded testability gap for user
direction instead of inventing a mechanism.

### CD-0101-02 — Published rows never assert the allocator-issued VIP

**Severity:** MEDIUM — blocking for consolidated approval  
**Dimensions:** D1 source authority, complete-row projection, anti-vacuity  
**Status:** Open; acceptance-designer remediation required

ADR-0101 D1 says the current allocator assignment is authoritative for the
Service VIP and D4 requires that VIP in every complete row
(`adr-0101-service-backend-health-observed-convergence.md:67-75,154-168`).
BE-02 claims exact listener rows and the acceptance fixture does allocate a real
VIP (`distill/adr-0101-acceptance.md:64`;
`service_backend_projection.rs:199-239`).

However, `World::assert_shape` at
`service_backend_projection.rs:388-412` checks ServiceId, backend count,
allocation identity, address, health, and weight, but never compares
`ServiceBackendRow.vip` with the allocator-issued VIP. BE-07's fingerprint
calculation starts from the VIP already present in the row
(`service_backend_projection.rs:586-590`), so it also cannot detect a wrong
row VIP. BE-10 similarly reads its lookup VIP from the published row before
checking the local map. Consequently, an implementation that derives the
correct ServiceId keys but writes a different VIP could pass the new composed
suite while violating the selected source authority and changing the
consumer-facing dataplane key.

This is a directly reachable output mismatch, not a hypothetical concurrent
schedule and not a request for another writer. It is also independent of the
currently missing production implementation.

**Required disposition:** retain the allocator-issued VIP from the existing
fixture setup and assert it for every published listener row, including the
empty and withdrawal projections where the row remains keyed. Keep the
assertion at the existing row/output boundary; do not add a public method or
duplicate allocator state in production.

## Red suffix limitations

The focused run's result is correctly classified as partial RED evidence, not
full GREEN:

- BE-01 and BE-09 are the two complete green controls.
- BE-02 reaches its single-listener control, then stops at the three-listener
  row-cardinality assertion; the multi-listener repeat suffix is not executed.
- BE-03 reaches the missing-allocator control, then stops before the assigned
  no-allocation empty-row assertion can pass.
- BE-04 reaches the exact rejected write and bounded repair attempts, but the
  repair assertion remains red.
- BE-05 stops at failed-membership withdrawal, so the same-ID restart suffix is
  authored but not executed in that run.
- BE-06 verifies the injected error, terminal drain, and View reload before its
  first post-restart health assertion fails; later repair-history assertions are
  not evidence of execution.
- BE-07 stops at the complete pair-count assertion; its serial-dispatch history
  assertions are authored but not executed.
- BE-08's one-listener threshold/reset/recovery branch is a control; the
  three-listener branch stops at its first row-shape failure.
- BE-10 stops when the real hydrator handoff count is absent. Its local-map
  install/remove/restore suffix is explicitly unexecuted.
- BE-11 reaches Start/SVID wake evidence, then stops at the missing Stop wake;
  later stopped-state assertions are not green evidence.
- BE-12 reaches the normal five-restart and typed final liveness failure, then
  stops at the missing final ServiceLifecycle wake; its final empty-membership
  assertion is not executed.
- BE-P1 is independently green policy evidence only. It does not prove the
  complete-row projection, row stamp wiring, or consumer handoff.

The unchanged seed-257209 diagnostic is independently fresh RED evidence for the
before-correction competing-writer path. No assertion above converts a reported
terminal state into an all-consumer acknowledgement, and no native VM/kernel/wire
claim is made from the Sim suite.

## Non-blocking documentation note

The exact ADR and ruling say independent DESIGN review is approved and the
fresh consolidated gate is pending (`adr-0101...md:3-18`; ruling `:1-16`).
Two linked summary labels remain stale or ambiguous: `feature-delta.md:568-569`
still says “independent review pending,” and `brief.md:10521`/`:10546` retains
“review pending”/“Proposed ADR-0101” wording. The corrected driving-port row at
`feature-delta.md:505` is consistent with the selected sole publisher, so this
does not create an implementation-owner contradiction. It is a low-severity
documentation synchronization note for the owning design/documentation flow,
not a reason to expand this review or invent compatibility behavior.

## Disposition and next gate

| Finding | Disposition |
|---|---|
| CD-0101-01 | Return to the original acceptance designer for a bounded stamp-oracle correction; no production API or architecture change. |
| CD-0101-02 | Return to the original acceptance designer for a bounded VIP assertion correction; no production API or architecture change. |
| Red suffix accounting | Accepted as honest partial RED evidence; no request to claim or manufacture GREEN suffixes. |
| Design architecture | Accepted as revision-3 DESIGN; no design remediation required. |
| Stale summary labels | Non-blocking documentation note; no action taken in this review. |

**Consolidated verdict: CHANGES_REQUESTED.** Once the two bounded acceptance
oracles are corrected and their focused RED evidence is rerun/classified, the
same design remains eligible for re-review. The original seed-257209 diagnostic,
the Sim/native tier boundary, asynchronous consumer contract, and greenfield
no-compatibility boundary must remain unchanged.

## Iteration 2 — focused re-review of CD-0101-01/02

This iteration is limited to the two acceptance-oracle findings above after the
original acceptance designer's remediation. The accepted ADR-0101 revision-3
architecture, the prior DESIGN approval, and all other iteration-1 gate results
are carried forward; they were not re-opened. The remediation changed only the
composed Sim test and its scoped DISTILL documentation. No production, design,
diagnostic, native, mutation, DES, or commit work was performed by this review.

### CD-0101-01 disposition — resolved

The revised BE-07 oracle now computes the expected stamp at the existing action
boundary as
`LogicalTimestamp::dominating(tick, state.node_id.clone(), observed_same_key_prior)`
and compares every real planned `WriteServiceBackendRow` to it. The source
proof is bounded to the existing production composition: `World::new` obtains
the allocator and real intent/listener facts (`service_backend_projection.rs:
238-283`), `planned_service_actions` calls the production hydrate wrappers,
loads the registered ServiceLifecycle View, and invokes the production
reconciler (`:335-359`), and `assert_planned_stamps` uses the same-key observed
row rather than an emitted marker (`:379-399`).

The three non-vacuous cases execute before the unchanged pair-count RED:

| Case | Tick supplied to the existing reconciliation call | Observed same-key prior | Expected stamp |
|---|---:|---:|---|
| First backend publication | 12 | absent | counter 13, writer `local` |
| Deciding failure before reload | 15 | counter 13 | counter 16, writer `local` |
| Same hydrated failure after the existing process-local tick reset | 1 | counter 13 | counter 14, writer `local` |

The baseline and high-prior row are production-authored dispatch outputs. The
fixture re-registers the real persisted View and resets only the existing
process-local scheduling coordinate (`service_backend_projection.rs:619-628`);
it does not seed, rewrite, or fabricate a prior row. Explicit prior/tick
inequalities and the nonzero-write assertion prove that each branch was
reached. The focused run below fails only afterward at one legacy write versus
four required write/handoff actions, so this exact stamp oracle is executed and
does not falsely turn BE-07 green.

**Disposition:** Resolved. The test now proves the selected writer, tick floor,
and observed-prior dominance at the ServiceLifecycle action construction
boundary without adding API or a test-only production seam.

### CD-0101-02 disposition — resolved

The fixture retains the exact `ServiceVip` returned by the existing allocator,
independently of any observed or planned row (`service_backend_projection.rs:
238-260, 270-289`). Every row emitted through the lag-aware subscription is
checked at the event boundary, and every row returned by the observation-store
read is checked independently (`:312-377`). Planned writes also pass through
the same oracle (`:379-398`). BE-07's correlation fingerprint uses the retained
allocator VIP (`:639-654`), and BE-10's local-map lookup uses that same retained
VIP (`:1000-1029`); neither derives its expected key from the published row.

This includes the oracle for keyed empty and unhealthy withdrawal rows. The
current RED is recorded honestly: BE-03's assigned-listener empty-row
publication is still absent in the implementation, so its empty-row assertion
does not execute yet; BE-07's correlation/handoff and BE-10's local-map suffixes
are likewise behind their first behavioral REDs. That is execution-suffix
limitation, not a missing VIP assertion or green-result claim. Once those
production rows are reached, the event/read boundary checks cannot accept a VIP
different from the retained allocator assignment.

**Disposition:** Resolved. The source-authority oracle is independent of
published fields and is placed at the existing observation/action output
boundaries; no production state or allocator API was added.

### Iteration-2 verification and RED disposition

| Evidence | Result |
|---|---|
| Focused changed-oracle run: `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(deciding_tick_orders_complete_row_handoffs_before_failure)' --no-capture --no-fail-fast` | Nextest `75e5194a-3d2f-488c-9dbd-5ab9f1f67cd5`; 0 passed, 1 behavioral failure, 35 skipped; seed 257225. The exact stamp/VIP assertions completed; the first failure was the intended existing pair-count assertion (1 legacy write vs 4 required actions). Inner exit 100 / outer exit 1; no setup or compile failure. |
| Acceptance designer's final focused composition | Nextest `4bc1f645-5589-4531-96c9-45c68a2726c6`; 12 run, 2 green controls, 10 behavioral RED, 24 skipped; inner exit 100 / outer exit 1. The run does not claim unexecuted suffixes. |
| Paired pure policy property | Retained green 128-case property evidence; unchanged by this remediation. |
| Unchanged original diagnostic | Nextest `0b7ed041-1979-45ff-9b0a-0be4c50dc7a3`; seed 257209, 1 behavioral RED, retaining the same-ID terminal-veto failure. |
| Scope checks | Acceptance/red-classification/feature-delta DISTILL updates and the Sim oracle changes are documented; no production or design edits, native run, mutation run, DES event, or commit was made. |

The remaining BE-02/03/04/05/06/07/08/10/11/12 REDs and all suffixes behind
their first failures remain required implementation evidence, not reasons to
weaken this acceptance design or to claim full GREEN. The original diagnostic
remains a separate before-change reachability witness. Sim remains in-process;
native/kernel/wire expectations remain outside this bounded DISTILL review.

### Iteration-2 disposition and current verdict

| Item | Disposition |
|---|---|
| CD-0101-01 | **Resolved** — exact dominating stamp, owner writer, absent/tick-ahead/prior-ahead cases are reached before the existing behavioral RED. |
| CD-0101-02 | **Resolved** — allocator-issued VIP is retained independently and checked for event/read/planned-row outputs plus independent consumer-key expectations. |
| RED suffix accounting | **Accepted as honest partial RED** — empty-row, pair/correlation, hydrator, and later lifecycle suffixes remain unexecuted where their first production behavior is missing. |
| Design architecture and other iteration-1 gates | **Carried forward as previously approved** — no re-review or scope expansion. |

**Current consolidated verdict: APPROVED.** Both blocking acceptance-oracle
findings are closed by bounded, source-grounded test changes. The artifact does
not claim the absent production implementation is green; it approves the
ADR-0101 DESIGN+DISTILL package for the next implementation/review gate with
the stated behavioral RED and suffix limitations preserved.
