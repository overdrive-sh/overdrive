# DELIVER implementation review — step 02-03

## Metadata

- Feature: `service-kind-vm-workloads`
- Roadmap step: `02-03` — authoritative Service backend projection and local backend withdrawal
- Reviewer: Codex, `nw-software-crafter-reviewer`
- Review iteration: 1 (fresh implementation review)
- Review date: 2026-09-08
- Reviewed range: `cbcf9a6d205b187735c5a080be9711ed32a2116d..1b8445f5dcebe59bea3e3fd99d5262e2223e21ca`
- Production/test implementation commit: `3d8af54cfe1a380146e686554da119194256a297`
- Design/evidence revision commit: `1b8445f5dcebe59bea3e3fd99d5262e2223e21ca`
- Working-tree condition: the pre-existing `AGENTS.md` modification was preserved and is not part of this review or artifact change.
- DELIVER evidence observed: step `02-03` RED, GREEN, and COMMIT events are present in `deliver/execution-log.json`; historical failed attempts were not treated as inherited evidence.

## Review boundary and authority

This review covers only the implementation and transferred tests for roadmap step
`02-03`. The authoritative contract is ADR-0101 revision 4,
`design/backend-eligibility-convergence-ruling.md`,
`design/amendment-be10-local-backend-withdrawal.md`, and the selected step in
`deliver/roadmap.json`. The acceptance/test boundary is additionally defined by
`deliver/adr-0101-test-transfer.md` and the approved DISTILL and DESIGN reviews.

The review checked the following bounded outcomes:

- `ServiceLifecycle` is the sole complete all-listener `ServiceBackendRow`
  publisher, comparing actual rows while excluding only the LWW timestamp,
  applying the existing startup/readiness veto, and ordering row writes and
  hydrator enqueues as specified.
- The exact ADR-0101 `ServiceLifecycleState`,
  `ServiceDataplaneIdentity`, `ServiceAllocFact`, and `ServiceLifecycleView`
  shapes are implemented without new public API.
- The backend-discovery bridge is retired from the direct module, dispatch,
  runtime registration, wake, and reclamation paths while the existing
  `ServiceMapHydrator` remains the consumer.
- The existing local backend vector/fingerprint and remote, mesh, lifecycle,
  and asynchronous ports are preserved; only health-selected existing
  `RegisterLocalBackend`/`DeregisterLocalBackend` actions and the specified
  private helper rename are changed.
- The approved BE02 singleton/all-three-listener behavior, the distinct
  ADR-0097 startup-observation fixtures, and the adapted retired-owner seed
  `257209` remain asserted.

Native unhealthy-routing gates for E09-v2/E10/E13, mutation testing, and
roadmap step `02-04` are outside this review and are not claimed here.

## Design compliance assessment

| Contract | Evidence reviewed | Result |
| --- | --- | --- |
| ADR-0101 D1, D2 | `crates/overdrive-reconcilers/src/service_lifecycle.rs:70-256` defines `ServiceAllocFact` with `backend_ip`, the three-field `ServiceLifecycleState`, and the exact four-field `ServiceDataplaneIdentity`; `:258-340` retains the input-only `ServiceLifecycleView` shape without `last_emitted_backend_fingerprint`. | Pass |
| ADR-0101 D2 | `service_lifecycle.rs:454-474` keeps the existing reconciler type aliases and `interests() == [AllocStatus]`; `crates/overdrive-reconcilers/src/lib.rs:107-425` contains only the remaining typed dispatch variants. No bridge variant, new action, method, trait, parameter, or store was introduced. | Pass |
| ADR-0101 D3 | `service_lifecycle.rs:792-837` uses the specified private `service_dataplane_identities` shape and derives every listener identity from the assigned allocator VIP; `:854-916` hydrates allocation facts once per workload with `backend_ip`; `:921-958` reads the complete observed row set and propagates read failures. Empty/no-listener cases do not synthesize a managed identity. | Pass |
| ADR-0101 D4 | `service_lifecycle.rs:1092-1169` computes readiness once per Running allocation, applies the same-ID terminal veto, emits every listener in deterministic map order, compares `service_id`/VIP/full backend vector while excluding only `updated_at`, uses `LogicalTimestamp::dominating`, and emits `WriteServiceBackendRow` before the existing `ServiceMapHydrator` enqueue. `:476-683` places projection actions before the first startup `FinalizeFailed` action. | Pass |
| ADR-0101 D5 | `crates/overdrive-reconcilers/src/backend_discovery_bridge.rs` is deleted; the direct dispatch and runtime registration/read/write/backoff paths contain no bridge type or factory. `crates/overdrive-reconcilers/src/workload_lifecycle.rs:222-271` enqueues `ServiceLifecycle` once for Service allocation mutations and preserves the kind-independent SVID wake. `crates/overdrive-control-plane/src/action_shim/reclamation.rs:100-128,246-264` retains workload/service/SVID submissions and removes only the bridge submission. | Pass |
| ADR-0101 D6 | `service_map_hydrator.rs:326-490` preserves the existing local vector/fingerprint, remote/mesh partitioning, lifecycle and asynchronous dispatch behavior. No acknowledgement barrier, revocation protocol, retry/recovery subsystem, or consumer API was added. | Pass |
| ADR-0101 D7 / BE10 amendment | `service_map_hydrator.rs:580-647` is the only private helper rename. It retains classifier/address handling and local vector/fingerprint inputs, selecting existing `RegisterLocalBackend` for healthy and existing `DeregisterLocalBackend` for unhealthy backends with the exact purposes and existing action fields. No pre-fingerprint filtering or readback/retry was added. | Pass |

The implementation also preserves the approved scope boundary: BE02 asserts one
allocation with all three listeners and does not claim multi-allocation ordering;
the four stale startup-counter fixtures now pair the latest probe status with a
distinct observed timestamp while retaining their terminal/counter assertions.

## Test and implementation honesty

The transferred `service_backend_projection` suite drives the production
reconciler composition in-process through the Sim driver, real reconcilers,
runtime, action shim, and observation stores. It does not spawn the built
Overdrive binary and does not act as a black-box expectation runner. The native
integration fixtures that remain under the historical bridge-named directory
are pending Tier-3 gates and do not restore a production bridge owner.

The acceptance changes inspected retain the substantive assertions while
updating them to the authoritative ServiceLifecycle owner and observed-row
model. New or transformed properties carry Contract Shape declarations; the
source-local pure property uses the exact `/// CONTRACT_SHAPE: pure-function.`
declaration. No test was found to invoke `cargo test`, a Rust test binary, or an
Overdrive production binary from an expectation runner in this step's evidence.

## Verification performed

The following bounded checks were run independently by this reviewer through
the repository's Lima execution path:

1. `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test integration -E 'test(service_backend_projection)' --no-capture --no-fail-fast` — 12/12 passed (nextest run `d1337edd-68cf-49ed-bc9d-230f25874cc8`). This includes complete-listener idempotence, write/enqueue ordering, empty membership, asynchronous withdrawal/recovery, failed-withdrawal repair, liveness handoffs, readiness threshold behavior, same-ID terminal veto, and workload wake behavior.
2. `cargo xtask lima run -- cargo nextest run -p overdrive-reconcilers --lib -E 'test(backend_policy_preserves_veto_threshold_and_unrelated_view_inputs) | test(push_register_local_backend)' --no-capture --no-fail-fast` — 4/4 passed (run `d1cbe289-179b-451a-866c-7531e1565842`), covering D7 action selection/classification and declared listener-port behavior.
3. `cargo xtask lima run -- cargo nextest run -p overdrive-core --features integration-tests --test mesh_backend_lb_gate -E 'test(local_health_selects_existing_action_preserving_remote_and_view) | test(three_way_split_routes_each_address_class_to_exactly_one_disposition)' --no-capture --no-fail-fast` — 2/2 passed (run `fc3d3171-af98-4c70-8d83-daf9e0c86b89`).
4. `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test e09_v2_failed_service_reachability_spike -E 'test(same_id)' --no-capture --no-fail-fast` — seed `257209` passed (run `0c195b68-b6ff-444a-aed9-2ecf97f4955c`), preserving the terminal startup veto while adapting the owner to ServiceLifecycle.
5. `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` — passed.
6. `git diff --check cbcf9a6d205b187735c5a080be9711ed32a2116d..HEAD` — passed.

These checks complement the crafter's recorded GREEN evidence (BE suite 12/12,
D7/helper suites, BE10 recovery, BE-P1, seed 257209, transferred controls,
workspace check, and formatting). No per-step mutation or native expectation
run was performed, as required by the roadmap and repository rules.

## Findings, reachability, and evidence

### Finding disposition

No proven in-scope defect was found. No remediation is required for this
iteration.

During source review, the row projection's
`identity.vip.try_as_ipv4().unwrap_or(Ipv4Addr::UNSPECIFIED)` fallback at
`service_lifecycle.rs:1130` was checked against its production owner path rather
than treated as a hypothetical input defect. The production persistent VIP
allocator rejects persisted non-IPv4 values and its allocation path constructs
only IPv4 values (`crates/overdrive-dataplane/src/allocators/persistent_service_vip.rs:130-143,192-211` and `service_vip.rs:126-137`). The production hydration context supplies that allocator view
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:1743-1760`). No
reachable production entry path can therefore supply an IPv6 identity to this
projection, and no seeded production-owner invariant fails. This remains a
review limit/non-finding, not a basis for inventing a new API or remediation.

The `ServiceDataplaneIdentity` comment at `service_lifecycle.rs:240-243` says
“one-per-Service” even though the accepted map is keyed per listener
`ServiceId`; it is stale explanatory prose but does not alter the exact type,
owner, state, or behavior required by ADR-0101. It is not a reachable defect or
a required step expansion.

No other suspicion met the repository's required threshold of a concrete
production entry point, complete caller/owner path, triggering state and order,
and a reproducible failure through that path. In particular, no finding was
based on a forced test abort, an invented state, a hypothetical cancellation,
or a pending native gate.

## Remediation dispositions

- No findings were accepted for remediation.
- The original crafter was not dispatched because there is no proven
  implementation defect within the approved step scope.
- No design gap was identified; no persistence, scheduling, recovery,
  acknowledgement, revocation, compatibility, or generalized cleanup
  mechanism is proposed.
- The pre-existing dirty `AGENTS.md` was neither modified nor included.

## Verdict

**APPROVED**

The reviewed implementation conforms to the accepted ADR-0101 revision 4 and
BE10 amendment API and ownership contracts for step `02-03`, the transferred
tests are honest and sufficiently exercised for this bounded step, and the
review found no proven reachable in-scope defect requiring remediation. This
verdict does not approve the pending native E09-v2/E10/E13 gates or roadmap step
`02-04`.
