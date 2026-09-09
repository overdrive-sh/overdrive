# DESIGN review — ADR-0099: Require accepted Running publication before completing restart release

| Field | Value |
| --- | --- |
| Feature | `service-kind-vm-workloads` (GH #257) |
| Design under review | Proposed ADR-0099 — Require accepted Running publication before completing restart release |
| Related ruling | `design/allocation-restart-write-ruling.md` |
| Roadmap boundary | Phase 02, step `02-03`, E10; design amendment before implementation |
| Review type | Independent DESIGN review |
| Review history | Iteration 1 — CHANGES_REQUESTED |
| Remediation history | R099-1 documentation corrected on 2026-09-07; user explicitly waived re-review |
| Last independent review verdict | **CHANGES_REQUESTED** — iteration 1, preserved below |
| Current design disposition | **Accepted by user direction after documentation correction**, not by a subsequent reviewer approval |

## Scope and verdict basis

This review is limited to the production-reachable same-allocation restart
publication defect established by seed `257203`: an old Exec attempt's
intentional exit is observed as `Terminated(36)` while `RestartAllocation` is
starting its replacement, the replacement proposes `Running(36)` from the old
`Failed(35)` snapshot, the compound observation write correctly returns
`Ok(None)`, and the action shim nevertheless releases Running-confirmed hooks.
The replacement driver is then live while the allocation-current observation is
`Terminated`, so the observation-driven operator stop path does not select it.

The proposed correction is technically necessary and bounded for that witness:
the action shim must treat compound-write acceptance as the prerequisite for
Running-confirmed effects, refresh the current row and re-propose once on the
first rejection, and use the existing awaited restart unwind after a second
rejection. It does not need a new public method, type, field, variant,
parameter, writer queue, persistence subsystem, allocation-attempt identity, or
recovery protocol.

The correction itself is acceptable in principle. Approval is withheld for one
blocking DESIGN-document defect: ADR-0099's `Lifecycle Gate Ownership` table
does not contain the complete gate declaration required by
`.claude/rules/design.md`. In particular, it omits an explicit affected
state/result, failure projection, ordering/deadline statement, counterexample,
and evidence lane for the changed acceptance/effect boundary, and it does not
explicitly discharge the required unavailable, unrelated, late-success,
disconnect/reconnect, and feature-disabled scenarios. This is a bounded
documentation correction; it does not authorize an architectural or API
change.

No implementation, production code, roadmap, mutation exclusion, native host
state, or DES event was changed by this review. The only new file is this
review artifact.

## Inputs and accepted-contract evidence

I read the mandatory project instructions and all seven repository rule files
named by `AGENTS.md`, the proposed ADR, the ruling, the architecture brief and
feature delta, the selected roadmap, and the relevant accepted contracts and
their prior review. The roadmap is already approved (`roadmap.json:299-303`);
its step `02-03` remains an E09/E10/E13 black-box expectation step with no Rust
test implementation scope (`roadmap.json:205-229`). ADR-0099 is a bounded
design amendment, not a new roadmap step.

The contract checks that matter to this amendment are:

| Contract | Review consequence |
| --- | --- |
| [ADR-0077 — prior-derived LWW counter](../../../../docs/product/architecture/adr-0077-lww-counter-derives-from-the-prior-row-not-the-tick.md) | `LogicalTimestamp::dominating` derives a successor stamp from the current row and uses the tick only as a floor. Equal `(counter, writer)` does not dominate. The existing single-writer limitation is stated honestly; this amendment does not replace it with a new publisher. |
| [ADR-0078 — `last_terminated` and `restart_count`](../../../../docs/product/architecture/adr-0078-crash-and-recover-is-durably-observable-last-terminated-plus-restart-count.md) | `CrashFacts::advance` snapshots only a terminal predecessor when the accepted successor is `Running`, and increments only for that terminal-to-Running transition. A re-proposal must therefore use the freshly accepted predecessor, not the rejected candidate. |
| [ADR-0096 — startup failure and backend eligibility](../../../../docs/product/architecture/adr-0096-startup-failure-withdraws-backend-eligibility.md) | `Running` retains driver-start/Beacon meaning; `ServiceLifecycle` owns startup, Stable, health, and liveness decisions; `WorkloadLifecycle` remains restart authority. The restart write correction must not reinterpret those states. |
| [ADR-0097 — probe target and startup attempt observation](../../../../docs/product/architecture/adr-0097-exec-network-probe-target-and-startup-attempt-observation.md) | Probe result projection and once-per-LWW startup counting remain independent of the restart publication acknowledgement. |
| [ADR-0098 — terminal netns cleanup](../../../../docs/product/architecture/adr-0098-terminal-netns-cleanup-from-observed-network-address.md) and [its approved review](review-adr-0098.md) | The address fallback is a separate terminal cleanup correction. The demonstrated equal-version rejection happens at replacement Running publication, so ADR-0098 is not a prerequisite and its dirty implementation remains untouched. |
| [Feature delta DDD-13 and lifecycle matrix](../feature-delta.md) | The named reuse boundary is one authorized same-ID restart, at most two proposals, with effects only after accepted Running. Existing ownership rows remain the source of truth for Stable, health, liveness, and restart authority. |

## Primary-source research verification

The ruling reports research before local mechanism selection. I checked the
specified upstream documentation and pinned source pages and verified the
important differences rather than treating the systems as interchangeable.

| System and source | Verified behavior | Difference that remains material here |
| --- | --- | --- |
| [Kubernetes API resource versions](https://kubernetes.io/docs/reference/using-api/api-concepts/#resource-versions) and [client-go `RetryOnConflict`](https://github.com/kubernetes/client-go/blob/v0.34.0/util/retry/util.go#L68) | A stale `resourceVersion` update is rejected with HTTP 409. The client helper bounds retries and requires the caller to fetch the current object, modify it, and retry each attempt. | Kubernetes uses server-controlled optimistic concurrency. Overdrive uses caller-stamped `(counter, writer)` LWW and an `Option` acceptance result. The relevant shared principle is that a rejected write is not an acknowledgement; Kubernetes does not authorize importing a server-version or publisher architecture. |
| [Kubelet v1.34 status manager](https://github.com/kubernetes/kubernetes/blob/v1.34.0/pkg/kubelet/status/status_manager.go) | The manager keeps a local status version distinct from the API-acknowledged version. A failed patch returns before advancing the acknowledged version, so the unsynced status is retried; Pod UID checks prevent applying status to another incarnation. | Kubelet separates local progress from API reporting and does not universally kill a workload after a failed status patch. Overdrive's particular gate is narrower: action-shim Running hooks and exit-emission release are lifecycle effects that require an accepted allocation-current row. |
| [Nomad v1.10 task runner](https://github.com/hashicorp/nomad/blob/v1.10.0/client/allocrunner/taskrunner/task_runner.go), [client reporting](https://github.com/hashicorp/nomad/blob/v1.10.0/client/client.go), and [restart policy](https://developer.hashicorp.com/nomad/docs/job-specification/restart) | Nomad updates local task state and restart events separately from client-to-server allocation reporting. Failed update RPCs remain pending and `AcknowledgeState` occurs only after successful reporting; a task restart remains distinct from rescheduling a replacement allocation. | Nomad's local task authority and asynchronous server reporting are not the allocation-current LWW gate in this defect. The sources support distinguishing runtime truth, write acceptance, and policy; they do not support adding a second persistence owner or rolling back this replacement through a new subsystem. |

The research therefore supports the proposed minimum: use the existing current
row as the retry predecessor, bound the retry, and do not treat a rejected
publication as accepted. It does not supply a reason to copy Kubernetes'
resource-version service, kubelet's publisher, or Nomad's reporting queue.

## Revalidated production owner and caller path

The defect is reachable through the real production convergence composition,
not a test-only state or forced task abort. The current dirty source was
re-read at the following owner boundaries:

| Order | Production owner path and exact evidence |
| --- | --- |
| 1 | `spawn_convergence_loop` in [`lib.rs:3332`](../../../../crates/overdrive-control-plane/src/lib.rs:3332) drains broker evaluations and awaits each [`run_convergence_tick`](../../../../crates/overdrive-control-plane/src/lib.rs:3377) serially. Shutdown is checked only between completed ticks (`:3400-3403`); production does not cancel a half-finished restart future at this boundary. |
| 2 | [`run_convergence_tick`](../../../../crates/overdrive-control-plane/src/reconciler_runtime.rs:1527) selects a registered reconciler, hydrates through the production runtime, persists its view, validates its action vector, and dispatches through the action shim (`:1588-1783`). The spike calls this production function for both registered `workload-lifecycle` and `service-lifecycle`. |
| 3 | `ServiceLifecycle` consumes the injected startup [`ProbeResultRow`](../../../../crates/overdrive-reconcilers/src/service_lifecycle.rs:664) and calls its `StartupProbeFailed` action builder (`:1271-1319`), authoring the existing `FinalizeFailed { ServiceFailed::StartupProbeFailed }` action. This is a Service startup-probe terminal decision, not a failed `Driver::start`. |
| 4 | The [`FinalizeFailed` action arm](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:1564) performs its existing terminal cleanup and publishes `Failed(35)` only after that ordering. It does not stop the Exec process; the old process remains owned by the driver until the restart stop-half. |
| 5 | [`WorkloadLifecycle`](../../../../crates/overdrive-reconcilers/src/workload_lifecycle.rs:919) alone selects the restart under the existing unified budget and builds the same-ID [`Action::RestartAllocation`](../../../../crates/overdrive-reconcilers/src/workload_lifecycle.rs:1426). Its intentional-stop predicate excludes an Operator-terminated row from later restart selection (`:1468-1475`). |
| 6 | The [`RestartAllocation` arm](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:2259) reads `prior_row` before stopping the old driver (`:2263-2302`), retains that Failed snapshot through provision and replacement driver start, and constructs the Running candidate with `LogicalTimestamp::dominating(..., Some(&prior_row.updated_at))` (`:2562-2591`). |
| 7 | The existing driver stop future can deliver the old attempt's intentional exit while it is awaited. [`exit_observer`](../../../../crates/overdrive-control-plane/src/worker/exit_observer.rs:198) selects cancellation before a new event and then completes the consumed event's existing bounded retry call (`:206`, `:342-375`). No test-only abort is needed. |
| 8 | [`handle_exit_event`](../../../../crates/overdrive-control-plane/src/worker/exit_observer.rs:452) fresh-reads Failed(35), classifies the intentional old exit as `Terminated` with the current `Stopped { by: Operator }` reason, derives counter 36 (`:474-480`), and writes through the same compound store route (`:547-549`). Its Job terminal fence is not applied to this Service row; the Service observation is accepted. |
| 9 | The replacement candidate is still stamped `(36, local)` from the earlier Failed snapshot. The compound write at [`action_shim/mod.rs:2612`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:2612) returns `Ok(None)`, but the current arm only treats `Err` as failed publication. It then inserts the routing index, installs mTLS when present, releases exit emission, and calls `on_alloc_running` (`:2660-2705`). |
| 10 | The winning Terminated/Operator observation is not restartable (`workload_lifecycle.rs:1468-1475`), and the operator desired-stop branch selects only observed Running rows (`workload_lifecycle.rs:639-655`). Thus the replacement driver's live state and the allocation-current observation disagree in a production-reachable way. |

The `build_alloc_status_row` diagnostic that reports recovery is emitted before
store acceptance ([`action_shim/mod.rs:344-398`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:344)); it cannot be treated as proof that a Running occurrence committed.

## Store, predecessor, and shutdown semantics

The public contract shape is exact and remains unchanged:

```rust
async fn ObservationStore::write_alloc_lifecycle(
    &self,
    current: AllocStatusRow,
    source: TransitionSource,
) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError>;
```

The trait documents `Some` as an accepted current-row winner plus its
occurrence and `None` as a rejected/equal write with no mutation or emitted
occurrence ([`observation_store.rs:2062-2076`](../../../../crates/overdrive-core/src/traits/observation_store.rs:2062)).
`LogicalTimestamp::dominating` is the shared comparator constructor
([`observation_store.rs:283-330`](../../../../crates/overdrive-core/src/traits/observation_store.rs:283));
`CrashFacts::advance` snapshots only a terminal predecessor for an accepted
Running successor and increments at most once ([`observation_store.rs:1299-1394`](../../../../crates/overdrive-core/src/traits/observation_store.rs:1299)).

Local and Sim are different adapters with the same contract, not two different
LWW models:

| Adapter | Evidence and boundary |
| --- | --- |
| `LocalObservationStore` | The redb transaction reads the current row and returns `Ok(None)` before inserting either current or occurrence when the candidate does not dominate ([`observation_backend.rs:460-530`](../../../../crates/overdrive-store-local/src/observation_backend.rs:460)). The async adapter emits the observation only when acceptance is `Some` (`:688-704`). |
| `SimObservationStore` | The in-memory current/occurrence lock applies the same `LogicalTimestamp::dominates` comparator and returns `None` for a losing/equal candidate ([`observation_store.rs:308-348`](../../../../crates/overdrive-sim/src/adapters/observation_store.rs:308)). Its trait method returns that acceptance result and routes the attempted row without changing the local winner (`:710-726`). |
| Production/test boundary | The spike uses Sim only for deterministic scheduling and its existing `SimDriver` port. It exercises production `run_convergence_tick`, registered reconcilers, action shim, and exit observer; it does not claim to observe netns, veth, TAP, nftables, mTLS, or kernel effects. Local redb and Sim conformance share the core comparator, so the seeded witness establishes the ordering defect while E10 remains the required native black-box evidence for host effects. |

The existing StopAllocation arm already demonstrates the relevant finite
discipline: it fresh-reads, rebuilds from the winner, and permits at most two
terminal proposals before releasing supervision without fabricating an
occurrence ([`action_shim/mod.rs:2777-2863`](../../../../crates/overdrive-control-plane/src/action_shim/mod.rs:2777)). ADR-0099's reuse of that shape is therefore feasible without a new port or
public retry method.

## Reproduction and necessity evidence

The checked-in spike uses a valid Service intent, the real registered
reconcilers, the real action shim and exit observer, and no pre-seeded
allocation-status row. `ScheduledStop` wraps the existing Driver port and uses
`SimDriver::inject_exit_after` plus `SimClock` to make the old attempt's real
exit event arrive while the stop future is pending ([spike lines 39-109](../../../../crates/overdrive-sim/tests/allocation_restart_write_spike.rs:39)).
Both live tests carry the required `/// CONTRACT_SHAPE: bounded-change.`
declaration ([lines 222-232](../../../../crates/overdrive-sim/tests/allocation_restart_write_spike.rs:222)).

The bounded command was rerun during this review:

```sh
cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests,overdrive-control-plane/integration-tests --test allocation_restart_write_spike --no-capture
```

Current nextest run ID: `2a3b67ab-4209-4549-97b3-8173ffe481c1`. Result: one
control passed and the seeded safety test failed, so `xtask` exited 1. The
failure output reports:

| Scenario | Observed result |
| --- | --- |
| No contender, seed `257203` | **PASS** — `Running(34) → Failed(35) → Running(36)`; one accepted replacement occurrence, `restart_count = 1`, driver `Running`, two Running hooks. |
| Old exit during awaited stop, seed `257203` | **FAIL** — `Running(34) → Failed(35) → Terminated(36)`; no replacement Running occurrence, `restart_count = 0`, replacement driver `Running`, two Running hooks, and convergence returns `Ok(())`. The assertion at [`allocation_restart_write_spike.rs:214-219`](../../../../crates/overdrive-sim/tests/allocation_restart_write_spike.rs:214) reports “replacement Running hook released with no accepted Running observation.” |

The ruling recorded two earlier complete runs of the same command, with IDs
`fa654829-2f9c-4993-8bfc-d4d470675e2d` and
`09c4440a-824d-49d0-90a8-4f000d8971c8`, each retaining the same pass/fail
control split. This review's rerun preserves that seed and failure shape.

This is sufficient necessity evidence for an acceptance-aware publication
correction. The first rejection could be made safe by immediate unwind, but
that would discard the already-authorized replacement when the old attempt's
expected exit wins. One fresh read/rebuild/re-proposal retains that restart
without changing restart policy. A second rejection must use the existing
awaited unwind; eventual publication, a third proposal, or a detached task is
not required by the witness.

No native E10 rerun was performed during DESIGN. Existing native captures are
historical corroboration only and are not represented as a current passing
result. Native E10 remains a post-implementation, default-feature,
built-binary black-box obligation over all eight checked-in driver/status
cells; it must not be replaced by this in-process seeded simulation.

## Design assessment

### D1 — Exact public and persistence contract

**Approved.** ADR-0099 explicitly retains the existing
`write_alloc_lifecycle` signature and the `Some`/`None` meaning. It correctly
forbids a retry method, error variant, action variant, schema field, writer
identity, or persistence subsystem. The proposal stays inside the existing
private `RestartAllocation` action arm and uses existing point lookup,
`build_alloc_status_row`, `LogicalTimestamp::dominating`, and
`cleanup_restart_abort` helpers.

### D2 — Acknowledgement correction and finite retry

**Approved in principle.** The proposed ordering is the minimum correction to
the reproduced defect:

1. Keep the existing restart decision, old-driver stop, cleanup, replacement
   provision, identity, and driver-start order.
2. On the first `Ok(Some(occurrence))`, continue existing routing, intercept,
   exit-emission release, Running hook, and occurrence emission.
3. On the first `Ok(None)`, point-read the current winner, rebuild the same
   replacement Running row from that fresh predecessor, and re-propose exactly
   once. Do not restart or reprovision the replacement and do not increment the
   `WorkloadLifecycle` decision budget a second time.
4. On acceptance, the fresh terminal predecessor is what
   `CrashFacts::advance` snapshots. The demonstrated trajectory becomes
   `Running(34) → Failed(35) → Terminated(36) → Running(37)`, with one accepted
   replacement occurrence and one durable restart increment.
5. On a second `Ok(None)`, do not install interception, release exit emission,
   call `on_alloc_running`, or synthesize an occurrence. Await the existing
   restart failed-publication unwind in the same action frame: stop the new
   driver, release supervision, stop mTLS where applicable, tear down the new
   network assignment, and release its binding only after teardown. Return the
   existing no-op disposition after that unwind; preserve the store winner.
6. Keep typed read/write errors and existing cleanup-error propagation. Do not
   reinterpret `None` as an I/O failure or add a third attempt.

The wording that a fresh read cannot normally return no current row is an
explicit current-path invariant, not a license to invent missing-row recovery.
The existing point lookup has no allocation-current delete operation. A
malformed-storage `Ok(None)` policy belongs to the existing observation
decode/recovery contract and is not part of this reproduced ordering witness;
the amended gate declaration should state that boundary without adding a new
recovery mechanism.

### D3 — Lifecycle gate meaning and unaffected owners

**Conceptually approved; declaration incomplete.** The proposed owner intent is
correct: `WorkloadLifecycle` authorizes the restart, the selected driver owns
driver start, the observation store authoritatively reports whether the
current-row proposal was accepted, and the action shim owns the ordering of
Running-confirmed effects. `ServiceLifecycle` remains the owner of startup,
Stable, backend eligibility, and liveness. The proposal does not move any of
those gates.

However, the current ADR table at `ADR-0099:102-119` combines “Accepted
Running observation” and “Intercept install and Running-confirmed release” in
a `Signal / effect` table. It does not declare the required separate gate
fields, and “Compound ObservationStore write, coordinated by action shim” is
ambiguous about the distinction between the store's acceptance decision and
the shim's effect gate. This is the blocking finding below; it is a contract
clarification, not a request for a second owner or a new component.

### D4 — Evidence boundary and E10 independence

**Approved in principle.** The seeded `overdrive-sim` invariant is the right
evidence lane for ordering and convergence. It uses a legal delay at an
existing Driver port and production composition; it does not manufacture a
row or assert kernel effects. The required post-implementation tests should
cover first acceptance, rejection-then-acceptance, exhausted rejection,
fresh-predecessor crash facts, Job fences, start-failure classification, typed
I/O, no hook/occurrence on refusal, and no third proposal. The E10 expectation
must continue to drive the built default-feature product externally and verify
only stakeholder-visible HTTP status and zero owned host cleanup deltas.

## Finding R099-1 — incomplete Lifecycle Gate Ownership declaration

| Field | Value |
| --- | --- |
| Severity | **Blocking DESIGN** |
| Reachability | The seed-257203 production path above is reproduced through `run_convergence_tick`, registered reconcilers, the action shim, and the exit observer. This is a real changed ordering boundary, not a hypothetical schedule. |
| Evidence | `.claude/rules/design.md:28-61` requires an existing state matrix and a separate gate declaration with existing evidence, owner, promise, affected state, failure projection, unaffected states, ordering/deadline, counterexample, and evidence lane. `.claude/rules/design.md:82-101` additionally requires the six boundary scenarios. |
| Current defect | ADR-0099's section at `:102-119` has owner/promise-like prose and an “unchanged boundaries” column, but no explicit affected state/result, failure projection, ordering/deadline, counterexample, evidence lane, or executable scenario mapping. |
| Disposition | **Open — changes requested in ADR-0099 only.** No production/API/architecture expansion is authorized. |

Before ADR-0099 can be accepted, amend its existing `Lifecycle Gate Ownership`
section with the following bounded material, using the already-named owners and
ports:

1. Retain an explicit existing state-ownership matrix for allocation
   `Running`, Service `Stable`, `Backend.healthy`, liveness termination, and
   the `WorkloadLifecycle` restart decision. For each row, name the owner, the
   exact promise, inputs that may gate it, and states it must not gate.
2. Declare a separate Gate G-99 for the changed boundary. State that the
   `ObservationStore`'s returned `Some` is the authoritative acceptance result,
   while the action shim is the owner of the post-start effect gate that
   requires both successful driver start and accepted Running publication;
   `WorkloadLifecycle` remains the sole restart-decision owner. This makes the
   existing roles precise without creating a new ownership model.
3. Fill every required gate field:

   - **Existing evidence:** the complete path from `run_convergence_tick` to
     `WorkloadLifecycle`, `Action::RestartAllocation`, the action shim, the
     compound store write, and the existing effects, with the current source
     references cited above.
   - **Owner and promise:** accepted current-row `Running` is the only proof
     that permits the existing release/intercept/hook effects; driver start
     alone is insufficient for this effect gate.
   - **Affected state/result:** identify precisely whether the declaration is
     the accepted allocation-current `Running` observation, the
     Running-confirmed effect release, or both, and say that `AllocState::Running`
     itself is not redefined.
   - **Failure projection:** first `Ok(None)` is one fresh-read/rebuild/re-
     proposal; second `Ok(None)` is existing awaited unwind with no hook or
     occurrence; typed read/write or cleanup errors retain the existing typed
     error path; cancellation remains owned by the existing action/convergence
     and exit-observer boundaries. Classify a fresh-read no-row result as the
     stated current-row invariant/out-of-scope boundary rather than inventing
     recovery.
   - **Explicitly unaffected:** Service `Stable`, startup/readiness and
     backend eligibility, liveness `StopAllocation`, Job terminal fences,
     probe result counting, non-conflicting restart, all other observation
     writers, and native host cleanup ownership.
   - **Ordering and deadline:** state that acceptance precedes routing-index
     insertion, intercept installation, exit-emission release, Running hook,
     and occurrence emission; the one fresh read and one re-proposal consume
     the existing action/tick deadline and do not multiply or create a new
     timeout budget.
   - **Counterexample:** retain at least the seed's no-contender control and
     old-exit winner, plus a plausible Stable/probe/liveness or Job-terminal
     case showing why the acceptance gate must not be moved to Service health,
     Stable, driver start, or a broader publisher.
   - **Evidence lane:** map the seeded simulation to ordering, in-process
     integration evidence to private lifecycle and predecessor facts, and
     default-feature native E10 to host-network/HTTP outcomes. Mark external
     disconnect/reconnect as not an added gate and state the existing
     cancellation/replay behavior.

4. Add executable obligations for the required boundary scenarios: available
   gate, unavailable/timeout, unrelated state, late success, disconnect/
   reconnect, and feature disabled. The existing mechanisms can satisfy these
   without new behavior: acceptance releases effects; rejection/error unwinds;
   unrelated Service and Job states remain under their current owners; a late
   stale write cannot overwrite a newer LWW winner; no remote reconnect gate is
   introduced; and the pre-feature/non-restart path remains unchanged.

This finding does not require a new retry API, a store-assigned version,
publisher, durable pending row, allocation identity, or ADR-0098 behavior. It
only makes the approved ownership, failure, ordering, and evidence contract
auditable before implementation.

## Necessity and explicitly rejected scope expansion

The reproduction proves that some acceptance-aware action-shim behavior is
necessary: treating `Ok(None)` as success is the direct cause of the live
driver/Terminated-row split. The selected one-refresh correction is preferable
within the stated scope because it preserves the authorized replacement. An
immediate unwind is a valid safety alternative but not the selected behavior;
“read later” cannot repair the already-released effects.

ADR-0098 is not necessary for this path. Its accepted fixture concerns terminal
teardown after an allocation binding is absent and an observed address can
prove the slot. In the reproduced restart path, the old terminal cleanup
releases the old binding after teardown; the replacement is then provisioned
under the same allocation ID, and its rejected Running row never persists the
replacement address. No production entry point or seeded invariant shows that
ADR-0098's address fallback gates the rejected publication. Its current dirty
implementation and approved review are preserved.

The external Kubernetes, kubelet, and Nomad mechanisms likewise do not justify
generalized stale-exit fencing, distributed-writer serialization, crash/
recovery architecture, a global status publisher, or a new persistence system.
Those remain outside this bounded amendment and require separate user scope and
independent DESIGN review if ever proposed.

## Verification record

| Check | Result |
| --- | --- |
| Seeded reproducer command | One current rerun: control passed, safety invariant failed as expected; nextest/xtask exit 1. Two earlier complete runs are recorded in the ruling with the same split. |
| Contract Shape declarations | Both live spike tests declare `/// CONTRACT_SHAPE: bounded-change.`; no new test surface was added by this review. |
| Local/Sim semantics | Core comparator, Local redb transaction, and Sim acceptance/rejection paths were inspected. Both return no accepted occurrence for a losing/equal candidate. |
| Shutdown/cancellation/retry | Production loop drains ticks serially and checks shutdown between ticks; exit observer cancellation is selected before receive and retries errors only. No forced abort or hypothetical detached owner was used. |
| Native E10 | Not rerun during DESIGN. Historical native captures remain corroboration, not current passing evidence; all eight black-box cells remain post-implementation obligations. |
| Review artifact | This Markdown artifact was checked for whitespace errors with `git diff --no-index --check` after creation; no whitespace diagnostics were produced. |

## Iteration 1 disposition and verdict

| Area | Disposition |
| --- | --- |
| Seeded production-path reachability and necessity | Confirmed. |
| Exact existing public API and `Ok(Some)`/`Ok(None)` meaning | Approved. |
| Fresh-predecessor, bounded two-proposal correction | Approved in principle. |
| Error, unwind, shutdown, and cancellation boundary | Approved in principle, subject to explicit gate failure projection. |
| Lifecycle Gate Ownership declaration | **Changes requested** — R099-1 is blocking under the repository DESIGN rule. |
| E10 independent native-black-box boundary | Preserved as a post-implementation obligation. |

**CHANGES_REQUESTED.** Amend ADR-0099's existing `Lifecycle Gate Ownership`
section with the complete required fields and boundary-scenario obligations
above, then obtain a fresh DESIGN re-review. No implementation claim is made,
and the original crafter must not resume against the proposed ADR until that
design review accepts the corrected artifact.

## Post-review documentation disposition — 2026-09-07

The designer corrected R099-1 in ADR-0099's `Lifecycle Gate Ownership` section:
an explicit existing state-ownership matrix, separate Gate G-99 with every
required field, and all six boundary-scenario mappings. No production behavior,
API, test, DES event, or ADR-0098 implementation was changed in this correction.

Two clarifications are grounded in the current production path rather than
read as new requirements from the review wording:

- The runtime supplies `TickContext.deadline`, but neither convergence dispatch
  nor the restart arm enforces an action/tick timeout. The correction records
  the existing two-proposal work bound; it does not claim that the extra read
  and proposal consume an enforced deadline or introduce a new timeout budget.
- LWW rejects a non-dominating candidate, while an authorized Service restart's
  fresh proposal can supersede an old attempt's accepted Terminated observation.
  This is not a blanket late-old-attempt-event fence or immutable Service
  terminal-state guarantee. Existing Job terminal-attempt fences remain intact.

The user explicitly directed **“no re-review”** after the bounded clarification
was requested. Accordingly, R099-1 is recorded as documentation-remediated and
ADR-0099 as **Accepted by user direction**, with re-review waived. This is not a
second independent review iteration or an `APPROVED` verdict by the reviewer.
The original iteration-1 findings and CHANGES_REQUESTED verdict above remain
unaltered historical evidence. Implementation and its normal step review remain
separate obligations; no implementation success is implied.
