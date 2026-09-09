# DISTILL review — BE10 local direct-VIP withdrawal

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Review subject | Bounded BE10 local direct-VIP withdrawal acceptance amendment |
| Authority | ADR-0101 revision 4 D7; `design/amendment-be10-local-backend-withdrawal.md` |
| Review inputs | Current ADR/design, independent DESIGN review, `distill/be10-local-withdrawal-test-handoff.md`, changed property, supplied helper-rename patch |
| Review type | Fresh isolated `nw-acceptance-designer-reviewer` DISTILL review |
| Reviewer | Codex |
| Model | User-selected GPT5.6 Luna, maximum thinking |
| Iteration | 1 |
| Review date | 2026-09-08 |
| Verdict | **APPROVED** |

## Scope and review boundary

This review covers only the acceptance material for the approved BE10 local,
non-mesh, direct-VIP withdrawal amendment. The exact contract is the existing
`ServiceMapHydrator` reconcile/action surface: retain a classifier-accepted
local candidate in the full fingerprint, select
`RegisterLocalBackend` for materialized `healthy: true`, select
`DeregisterLocalBackend` for materialized `healthy: false`, and preserve the
remote action, View, ordering, listener identity, protocol, ports and
correlations. The private helper rename is the only named implementation
surface change; no public API is introduced.

The unchanged composed BE10 trajectory, the paired pure reconcile property and
the three existing private-helper controls were reviewed as one bounded
evidence package. BE02's separately approved singleton correction, the four
startup-fixture alignments, the original E09 mesh/VM mechanism, lifecycle
policy, replica arbitration, persistence, retry/readback architecture,
historical membership cleanup and BPF/kernel implementation are outside this
review and are not reopened.

I read the repository-mandated `AGENTS.md`, `CLAUDE.md` and seven
`.claude/rules` files, the `nw-acceptance-designer-reviewer` definition, and
its required acceptance critique, test-design-mandate and BDD skills. Existing
dirty work was preserved. This review wrote only this native Markdown review
artifact; it made no implementation, test, design, DES, roadmap or commit
change.

## Contract and production-path assessment

### Composed BE10 reachability

The BE10 test at
`crates/overdrive-sim/tests/integration/service_backend_projection.rs:967-1036`
is unchanged in the current diff apart from the unrelated BE02 fixture change
elsewhere in that file. Its `consumer_trajectory(true)` uses the existing
`World`, real `ProbeRunner` readiness stimulus, `ServiceLifecycle`, observation
publication, broker handoff and queued `ServiceMapHydrator`. The
`hydrate_published` helper at lines 413-429 drains the actual pending
`service-map-hydrator` evaluations and runs those owners; it does not author a
row or manufacture an evaluation. The trajectory reaches the healthy local
map, readiness `Fail`, the authoritative unhealthy row and mesh/DNS
withdrawal before asserting that the local map is removed.

The current production path explains the supplied RED without an imaginary
state: `ServiceMapHydrator` includes `Backend.healthy` in its local fingerprint
but the current helper at
`crates/overdrive-reconcilers/src/service_map_hydrator.rs:599-634` always emits
`RegisterLocalBackend`. A materialized health change therefore opens the
existing local-fingerprint gate while dispatching the wrong existing action.
The supplied BE10 run observes exactly that reachable mismatch. This is
adequate necessity evidence for the focused amendment and remains Sim
production-owner-to-adapter evidence, not a native unhealthy-routing claim.

### Paired pure-reconcile property

The new
`local_health_selects_existing_action_preserving_remote_and_view` property at
`crates/overdrive-core/tests/mesh_backend_lb_gate.rs:831-913` is a bounded pure
reconcile check. It calls the existing `ServiceMapHydrator::reconcile` entry
point with generated nonzero listener/backend ports and the finite contract
universe of local/remote/mesh addresses × TCP/UDP × healthy/unhealthy. It
asserts the whole ordered action vector, including the remote action first,
the selected existing local action, exact action fields, independent listener
and backend ports, protocol, backend tuple and purpose-derived correlation.
It also asserts both View maps, including the full local fingerprint and
remote retry inputs, then drives the same input and time through the existing
emission gate and requires no actions plus an identical View on repeat.

The property carries the exact required declaration
`/// CONTRACT_SHAPE: pure-function.` at line 838. It does not create
observation rows, seed a lifecycle state, bypass the runtime, add a production
seam or claim that its constructed state is a reachable schedule.

The expected value is independent of the implementation under test for the
load-bearing behavior. The property constructs the expected existing Action
variants and correlations from the input `healthy` bit and the specified
contract fields; it does not call the private helper, inspect the actual
variant and translate it into an expected value, or copy the emitted vector.
The finite address partition is an explicit contract universe, and the shared
canonical fingerprint function is the prescribed full-content identity needed
to assert the existing View/correlation contract. The current unconditional
registration already fails the property at the unhealthy local/TCP case, which
is direct evidence that the assertion is not tautological. Existing healthy
classification coverage remains unchanged and supplies the separate
address-disposition control.

### Helper-rename transfer

`.context/be10-hydrator-test-transfer.patch` is correctly an unapplied,
test-only `apply_patch` payload. It changes only the three existing
`cfg(test)` helper calls to `push_local_backend_actions` and adds the exact
pure-function Contract Shape declaration to each existing pure helper control.
It does not rename a test, alter an assertion, add a public surface or edit
production behavior. Applying it alongside the approved private production
helper rename preserves the healthy exact-field, unequal VIP/backend-port and
IPv6/guard-rejection assertions. Keeping the old test names is appropriate:
their inputs remain healthy and their assertions remain about registration.

The production implementation must still perform the separately owned rename
and health branch. This review does not apply the patch or claim GREEN.

## Evidence and commands actually run

The following read-only checks were run in the shared workspace while
preserving all pre-existing dirty files:

| Command/check | Evidence used |
|---|---|
| `git status --short --untracked-files=all` | Confirmed extensive pre-existing work and isolated the review artifact as the only permitted write. |
| `rg -n` searches over the design, handoff, source and test files | Located the approved D7 contract, owner path, test/property names, Contract Shape declarations and unexecuted suffixes. |
| `nl -ba` on the ADR, amendment, DESIGN review, handoff, production hydrator, paired property, helper patch and composed BE10 | Verified exact line-level API, state, ordering, assertion and evidence claims. |
| `git diff -- crates/overdrive-core/tests/mesh_backend_lb_gate.rs crates/overdrive-sim/tests/integration/service_backend_projection.rs` | Confirmed the new property and that the BE10 trajectory body is unchanged; identified the unrelated BE02 fixture diff. |
| `git diff --check` | Passed with no whitespace errors. |
| `test -e docs/feature/service-kind-vm-workloads/distill/review-be10-local-withdrawal.md` before writing | Confirmed this artifact did not already exist. |

No test, compile/check, native/kernel, mutation, expectation or whole-suite
command was run by this reviewer, as required by the bounded review request.

### Supplied execution evidence reviewed, not rerun

The handoff records these fresh commands and results. They are reported as
author-supplied evidence, not as executions by this review:

1. The unchanged composed BE10 command:

   ```text
   cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(existing_consumers_follow_withdrawal_and_recovery_asynchronously)' --no-capture
   ```

   Run `4d38274f-380a-45f1-a295-c61afc223a3b` recorded 0 passed, 1 failed and
   35 skipped. Seed 257221 reached the healthy registration, real readiness
   failure, unhealthy row and mesh/DNS withdrawal, then failed at
   `service_backend_projection.rs:1019` with the local map still
   `Some(192.0.2.10:18081)` instead of `None`. This is a behavioral RED, not
   a setup failure.

2. The paired property/classification command:

   ```text
   cargo xtask lima run -- cargo nextest run -p overdrive-core --features integration-tests --test mesh_backend_lb_gate -E 'test(local_health_selects_existing_action_preserving_remote_and_view) | test(three_way_split_routes_each_address_class_to_exactly_one_disposition)' --no-capture --no-fail-fast
   ```

   Run `90bab83e-71ab-4690-958b-cafb892e8c40` recorded the unchanged
   classification property passing and the new property failing at the
   unhealthy local/TCP comparison: actual `RegisterLocalBackend` versus
   expected `DeregisterLocalBackend`, with the wrong purpose. Ports shrank to
   listener 1/backend 1. Because the first counterexample stopped the property,
   zero complete generated cases were recorded; the handoff honestly does not
   claim subsequent unhealthy View, UDP, remote/mesh or all 128 configured
   cases.

3. The retained helper/removal-control command:

   ```text
   cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers -p overdrive-control-plane --features integration-tests --lib --test integration -E 'test(push_register_local_backend) | test(deregister_local_backend_dispatch_removes_entry_from_dataplane)' --no-capture --no-fail-fast
   ```

   Run `1d42dfc0-a765-492f-9349-d237a53e740e` recorded 4 passed and 468
   skipped. The handoff separately records the necessary unused-import cleanup
   discovered after that run and does not claim a warning-free recompilation.

## Assertions and suffixes not yet executed

The following are explicit downstream obligations, not missing evidence that
invalidates this pre-implementation DISTILL review:

- The BE10 recovery suffix at
  `service_backend_projection.rs:1021-1031` did not execute because the
  retained local-removal assertion is currently RED. After the authorized
  production correction, it must prove the existing registration path restores
  the local map and the real ProbeRunner/ServiceLifecycle path supplies the
  healthy fact again.
- The paired property stopped at its first unhealthy local/TCP counterexample;
  all remaining finite combinations and all configured generated cases must
  execute after GREEN. The retained helper controls must then run with the
  production private-helper rename applied.
- Native unhealthy direct-VIP routing, BPF/kernel behavior, black-box
  expectations, mutation testing and full-suite verification were not run and
  are not claimed. Sim local-map state is not proof of connection denial,
  established-flow revocation or native attached-cgroup routing. Any later
  native claim remains a separate authorized Tier-3 boundary.

These limits are explicitly stated in the accepted D7 amendment and handoff.
They do not authorize a new readback, persistence, retry, cleanup, membership,
replica or BPF mechanism.

## Findings and dispositions

No critical, high, medium or low finding remains within the bounded BE10
acceptance scope. The following review observations are deliberately
dispositions rather than findings:

| Observation | Disposition |
|---|---|
| The composed BE10 and paired property are RED before the production correction. | Retain both REDs as honest, reachable pre-implementation evidence. Do not require green tests against the not-yet-implemented D7 behavior. |
| The BE10 recovery suffix and native unhealthy-routing evidence are unexecuted. | Retain as clearly labeled downstream obligations. Do not convert them into a new test seam or a native claim. |
| The helper-transfer patch is unapplied and the current production helper still has its old name. | Correct handoff sequencing: the implementation crafter owns the private rename; the patch is only the necessary test fallout and preserves assertions. |
| BE02 and startup fixture changes appear in the dirty work. | Keep their separate approved dispositions; do not reopen or expand this review. |

The test package meets the applicable acceptance mandates: behavior is
asserted through the existing production composition and driving ports; the
pure property is bounded and output-focused; its expected actions are not
derived from observed output; complement/idempotence and exact contract fields
are asserted; and Sim/native, example/expectation and implementation/test
boundaries remain distinct.

## Iteration history

| Iteration | Reviewed material | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | BE10 local direct-VIP withdrawal DISTILL amendment and test handoff | **APPROVED** | None remaining in scope | Ready for the separately owned implementation GREEN and downstream acceptance gates. |

## Final verdict

**APPROVED.** The handoff is an honest, bounded acceptance package for the
approved ADR-0101 D7 contract. It demonstrates the reachable behavioral RED
through the real ServiceLifecycle → observation/broker → queued
ServiceMapHydrator → existing local action path, adds a proper pure reconcile
property with the required Contract Shape declaration and exact action/View
complements, and keeps the helper rename mechanical. It neither invents API or
architecture nor overclaims recovery or native routing. The original crafter
may proceed with the separately authorized implementation workflow; the
recovery suffix, complete property run, helper controls and any native evidence
remain required downstream gates.
