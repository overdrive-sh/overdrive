# ADR-0096 independent DESIGN review — startup-failure backend eligibility

## Review metadata

| Field | Value |
|---|---|
| ADR | ADR-0096, `startup-failure-withdraws-backend-eligibility` |
| Feature | `service-kind-vm-workloads` |
| Review type | Independent DESIGN review |
| Iteration | 1 |
| Verdict | **REJECTED** |
| Scope | The proposed `StartupProbeFailed` eligibility veto and its claimed backend-withdrawal-before-terminal-publication ordering only |

## Material reviewed

- `docs/product/architecture/adr-0096-startup-failure-withdraws-backend-eligibility.md`
- The amended feature delta, DESIGN decisions, DISTILL scenarios, architecture brief, and E09/E13 expectation runners.
- ADR-0055, ADR-0079, ADR-0080, ADR-0087, ADR-0090, ADR-0094, and ADR-0095 where they define the existing Service lifecycle, backend health, ordering, transport, and restart boundaries.
- The production path in `overdrive-reconcilers::ServiceLifecycleReconciler`, the control-plane action shim, and `BackendIndex`.

## Evidence and reachability

The reported E09 failure is reachable through the real product path, not a
test-only construction. The captured native-metal run starts the failing
Service through `overdrive deploy`, reports the startup refusal, then starts
the matching peer VM Job. The Service is shown with `startup ... last=fail`,
while the peer fails with exit `43` (`verification/expectations/E09-vm-service-tcp-truthfulness-100/evidence/product-run.out:22-58`). That is the E09 client's deliberate evidence that it reached the
guest reply when it should have been denied.

The current source explains that observation. A `Running` allocation reaching
the startup-failure branch emits `FinalizeFailed` and records the existing
non-Stable terminal marker (`crates/overdrive-reconcilers/src/service_lifecycle.rs:648-661`). The subsequent readiness/backend pass still considers every
`Running` allocation (`:1099-1111`), and a Service without readiness returns
`true` before consulting any other input (`:1150-1158`). The resolver selects
only a healthy backend and turns a known frontend with none into
`MeshUnreachable`, so an unhealthy row is the right existing data-plane
projection (`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:491-542`).

The narrow proposed mechanism is otherwise compatible with the established
owner boundary: it leaves the `Running` history to the action shim/driver,
keeps `ServiceLifecycle` as the health writer, and leaves liveness termination
and `WorkloadLifecycle` restart authority untouched. A non-terminal,
no-readiness allocation must retain the existing healthy default; the proposed
reconciler-level action test and E09/E13 built-product checks are the correct
evidence lanes for that normal-path behavior.

## Findings

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| R0096-1 | Blocking | The amendment changes a backend-health gate but neither ADR-0096 nor its updated SSOT records the required truthful Lifecycle Gate Ownership declaration. | Must remediate. |
| R0096-2 | Blocking | D2's claimed observable ordering is not established by vector order under the existing per-action error-isolation behavior that the ADR explicitly retains. | Must remediate. |

### R0096-1 — the changed health gate is declared “not applicable”

ADR-0096 D1 adds a priority veto to the existing `Backend.healthy` predicate:
after `StartupProbeFailed`, no readiness declaration must no longer imply a
healthy backend. That is a changed health gate, not merely a restatement of
the pre-existing target-projection boundary. Repository DESIGN rules require a
`Lifecycle Gate Ownership` section for every added, removed, or moved gate,
with owner, promise, affected state, failure projection, unaffected states,
ordering, counterexample, and evidence lane.

ADR-0096 contains no such section. More materially, the amended feature
`wave-decisions.md:437-449` labels the proposed gates “Not applicable” while
its own table says `Backend.healthy` is “vetoed by its existing terminal startup
failure.” `feature-delta.md:552-572` and `brief.md:10452-10473` retain the
same “Not applicable” framing while asserting the new veto. The label and the
contract conflict.

**Bounded correction:** add one ADR-0096 gate declaration for the eligibility
veto. It must name `ServiceLifecycle` as owner; define the affected result as
only the backend row's `healthy` bit; state that `Running`, `Stable`, liveness
termination, and `WorkloadLifecycle` restart authority are unaffected; and
list the no-readiness non-terminal, terminal/late-probe, liveness/restart, and
fresh-replacement-allocation cases as executable obligations. Update the three
SSOT references so they no longer call this change “Not applicable.” This
requires no new API, persisted field, owner, or mechanism.

### R0096-2 — serial dispatch does not prove withdrawal completed before terminal publication

D2 says that serial, awaited action-vector dispatch makes it true that once a
stream consumer observes `StartupProbeFailed`, the backend row has already
withdrawn eligibility. The cited loop does preserve *attempt order*, but it
does not make the preceding action a prerequisite for the following action:
`dispatch_with_network_provisioner` awaits each `dispatch_single` then records
the first error and continues with the rest of the vector
(`crates/overdrive-control-plane/src/action_shim/mod.rs:926-955`).

The existing backend-row action can fail at the real ObservationStore boundary:
it directly awaits `observation.write(...)` and propagates the underlying
`ObservationStoreError` (`action_shim/write_service_backend_row.rs:21-39,48-60`).
The later `FinalizeFailed` action independently writes its terminal
`AllocStatusRow` and only then emits the lifecycle broadcast
(`action_shim/mod.rs:1764-1771`). Consequently the reachable failure sequence
is:

1. `WriteServiceBackendRow { healthy: false }` is attempted and its store write
   returns an error;
2. the batch loop records that error but continues;
3. `FinalizeFailed { StartupProbeFailed }` succeeds and broadcasts the
   terminal; and
4. the old healthy backend row remains available to the resolver.

This trace follows current production ownership and error behavior; it is not a
forced cancellation or test-only abort. Vector-order assertions required by D4
would still pass and therefore cannot prove D2's stronger observable boundary.

**Bounded correction:** ADR-0096 must choose a statement its permitted
mechanism can actually establish. If per-action isolation remains unchanged,
it may claim only normal/successful-write ordering and must explicitly state
that a failed withdrawal write does not establish the terminal-before-withdrawal
operator guarantee; the new reconciler test must be described as an
action-construction/order proof, with E09/E13 as healthy-store product proof.
If the intended requirement is truly “a caller may never observe the terminal
until withdrawal is durable,” then the existing per-action error-isolation
contract must change or the two writes need an atomic completion boundary.
Either is outside the amendment's stated no-new-mechanism scope and needs a
separate, user-authorized DESIGN decision. The current ADR cannot retain both
claims.

## Compatibility assessment

- **No-readiness:** preserved only when no startup terminal was decided; this
  is the necessary compatibility boundary and is covered by the proposed
  reconciler test obligation.
- **Readiness:** a startup-failure veto must dominate a readiness pass in the
  same tick; a later pass must not re-enable the terminal allocation. A fresh
  allocation ID is correctly evaluated by the ordinary existing predicate.
- **Liveness and restart:** no change is justified. Liveness remains the
  existing `StopAllocation` detector, and `WorkloadLifecycle` stays the sole
  restart authority under ADR-0087.
- **Data plane:** a successfully persisted unhealthy full row causes the
  existing resolver to fail the known frontend closed. The unconditional
  fail-closed claim is not supportable while a backend-row write error can be
  followed by terminal publication.
- **Scope:** no new API/state/owner/persistence/retry/restart mechanism is
  warranted by the proven E09 normal-path failure. The two corrections above
  are documentation/contract corrections unless the user elects the stronger
  durable ordering guarantee.

## Verification

No code, test, runner, roadmap, or proposal was modified during this review.
The review used source-path tracing and the existing native-metal E09 capture.
No new test was run because ADR-0096 has not been implemented and the worktree
contains unrelated in-progress changes. The required post-remediation evidence
remains: a production-hydrated reconciler assertion of the emitted unhealthy
row plus constructed action order, the non-terminal no-readiness control, and
fresh black-box E09/E13 captures against the built default-feature binary.

## Re-review — iteration 2

### Remediation evidence

The architect corrected the first finding in ADR-0096 itself. The new
`Lifecycle Gate Ownership — changed backend eligibility gate` section names
`ServiceLifecycle` as owner, limits the affected result to the existing row's
`healthy` bit, identifies the failed-write projection, preserves the stated
unaffected owners and states, and records ordering, counterexamples, and test
lanes (`docs/product/architecture/adr-0096-startup-failure-withdraws-backend-eligibility.md`, Lifecycle Gate Ownership section). The amended DESIGN
SSOT now calls this a changed health gate rather than “Not applicable” and
records the same limited contract in `wave-decisions.md`.

The second finding is also corrected rather than papered over. ADR-0096 D2 now
claims only constructed-vector order and normal successful-write behavior. It
explicitly preserves the existing error path: a failed backend-row write does
not establish durable withdrawal before the terminal and a stronger guarantee
would require separately authorized design work. This matches the source:
the batch awaits each action but records an error and continues
(`crates/overdrive-control-plane/src/action_shim/mod.rs:926-955`); the backend
writer propagates its `ObservationStore` error
(`action_shim/write_service_backend_row.rs:21-60`); and a later successful
`FinalizeFailed` can independently write and broadcast its terminal
(`action_shim/mod.rs:1764-1771`). D4 correspondingly limits the future Rust
test to action construction/order and reserves E09/E13 for the healthy-store
built-product path.

### Finding dispositions

| ID | Iteration 1 disposition | Iteration 2 result |
|---|---|---|
| R0096-1 | Must remediate | **Resolved.** The ADR and amended SSOT now identify and bound the changed backend-health gate. |
| R0096-2 | Must remediate | **Resolved.** D2 no longer claims a durable terminal-before-withdrawal property that the continuing action shim cannot provide. |

No further reachable blocker was found. The retained normal-path scope is
appropriately narrow: it uses the existing `ServiceLifecycle` health writer
and existing backend row, does not revise `Running`, and leaves readiness,
liveness, restart ownership, persistence, API, and transport behavior alone.

## Iteration record

| Iteration | Result | Notes |
|---|---|---|
| 1 | **REJECTED** | R0096-1 and R0096-2 remain unresolved. |
| 2 | **APPROVED** | Both bounded findings are remediated; the durable-ordering claim is truthfully limited to the successful-write path. |

## Verdict

**APPROVED.** ADR-0096 now accurately scopes the normal successful-write
eligibility withdrawal, documents ownership and failure behavior, and avoids
claiming an atomic durability boundary that the existing action shim does not
provide.
