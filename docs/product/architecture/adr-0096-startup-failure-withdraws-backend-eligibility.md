# ADR-0096: A terminal startup failure withdraws backend eligibility without redefining `Running`

## Status

**Proposed** (2026-09-07). Focused, user-authorized DESIGN amendment for
`service-kind-vm-workloads`; independent DESIGN review is required before
DELIVER remediation.

**Review record:** [independent review iteration 2](../../feature/service-kind-vm-workloads/design/review-adr-0096.md)
approved the original bounded decision; the Proposed label above is retained as
historical metadata, not a claim that review never occurred. The 2026-09-08
same-ID premise correction below is factual. Proposed revision 3 of
[ADR-0101](adr-0101-service-backend-health-observed-convergence.md) separately
pins the user-selected sole backend projection and asynchronous consumer
convergence. Its exact design remains pending independent review.

## Context

Native-metal E09 reaches a real contradiction in the current production path.
The unbound VM Service listener returns `connection refused` to its startup TCP
probe and `ServiceLifecycle` reaches its existing
`ServiceFailed { StartupProbeFailed }` terminal. Before that terminal is
observed, however, the same allocation is still `Running`.

`ServiceLifecycle::reconcile` subsequently invokes
`readiness_backend_row_action` for every `Running` allocation
(`crates/overdrive-reconcilers/src/service_lifecycle.rs:1088-1117`). Its
`compute_backend_healthy` returns `true` before consulting a probe result when
the Service declares no readiness probe (`:1150-1157`). The full
`service_backends` row therefore continues to advertise the startup-failed
allocation as healthy. The bridge carries that observed value, while the mesh
resolver selects only `healthy` backends and otherwise fails a known frontend
closed (`BackendIndex::first_healthy_backend_for`,
`crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:491-539`). E09's
matching peer VM Job consequently receives the deliberately unavailable
guest's reply and exits 43.

This is not a hypothetical cancellation trace. The bounded production sequence
is: `overdrive serve` starts the Service; `overdrive deploy
tcp-startup-failure.toml` starts the VM and commits Running; the marked TCP
probe receives refusal; `ServiceLifecycle` emits the existing terminal; the
same tick emits a healthy backend row due to the no-readiness default; and the
peer Job dials the existing Service frontend. E09 reproduces that exact path.

The prior contract is still correct in its own domain: startup failure must not
reinterpret driver-start success as a failed start. `Running` records the
truthful Beacon/start history. It does not entail continued eligibility after
the Service lifecycle has declared the startup contract terminally failed.

## Decision

### D1 — Startup terminal failure is an eligibility veto

When `ServiceLifecycle` decides its existing
`ServiceFailed { StartupProbeFailed { .. } }` terminal for allocation `A`, `A`
is ineligible for Service traffic from that deciding tick onward. The existing
backend row represents this as `Backend { healthy: false }` for `A`; it does
not alter `A`'s driver-start/Running meaning, probe result, terminal reason,
or restart policy.

The eligibility predicate is ordered exactly as follows:

1. an allocation for which the current `ServiceLifecycle` tick has reached
   the existing non-Stable startup terminal is `healthy = false`;
2. otherwise, the existing readiness rule applies unchanged: no readiness
   declaration is `healthy = true`; a declared readiness probe uses its
   existing latest-result and consecutive-success threshold rule.

Thus the no-readiness compatibility default remains true for a non-terminal
Running allocation, including an ordinary startup-success/Stable allocation.
It is not permission to route to an allocation whose startup contract has
already terminally failed. A later readiness pass cannot override this veto,
because the allocation carries the existing terminal veto. A genuinely distinct
allocation without that veto is evaluated by the ordinary predicate. Automatic
WorkloadLifecycle restart, however, creates another attempt under the same
allocation ID; Running alone does not clear the existing veto.

### D2 — Construct the withdrawal action before the terminal action

The existing `WriteServiceBackendRow` carrying `healthy: false` for the
deciding allocation must precede the existing `FinalizeFailed` action in that
reconcile batch. `action_shim::dispatch_with_network_provisioner` attempts and
awaits those actions in vector order
(`crates/overdrive-control-plane/src/action_shim/mod.rs:926-950`). This is an
action-construction and normal-success-path ordering rule: a successful
backend-row publication precedes the following terminal action's lifecycle
event. The resolver observes it asynchronously, not necessarily before that
event. This corrects the earlier “resolver sees ... before” wording under the
user-selected ADR-0101 consumer boundary: failure is a lifecycle fact, not a
routing-consumer acknowledgement.

No new transactional store, action variant, terminal, event, or ordering
service is introduced. This is only the order of two existing actions produced
by the existing reconciler. The existing per-action error-isolation loop
continues after a failed backend-row write, so this amendment does **not**
claim that every observable terminal is preceded by a durable withdrawal. If
that stronger terminal-delivery guarantee is required, changing the shim's
continue-on-error contract or introducing an atomic completion boundary is a
separate user-authorized DESIGN decision, outside this amendment.

### D3 — Existing owners and durable inputs remain the contract

**Topology amendment:** Proposed ADR-0101 revision 3 replaces the bridge
membership publisher described below with sole ServiceLifecycle projection,
as selected by the user. The following records this ADR's original topology;
the eligibility predicate, same-ID premise correction and restart authority
remain inputs to that amendment. ADR-0101 is pending independent review.

`ServiceLifecycle` remains the sole owner of both the existing startup
terminal decision and the `Backend.healthy` value. `BackendDiscoveryBridge`
continues to converge membership from `Running` rows and carry the observed
health value; it gains no startup classifier. The action shim remains the sole
writer of the allocation terminal and of the backend row. `WorkloadLifecycle`
remains the sole restart authority and receives no startup-failure-specific
policy.

No persisted schema changes. The existing durable `AllocStatusRow.terminal`
records the typed `StartupProbeFailed` outcome; the existing persisted
`ServiceLifecycleView::terminal_announced` records the non-Stable terminal
publication/dedup input used during the deciding tick and across control-plane
restart. No `eligible` field, new health state, retry marker, generation,
owner, or recovery protocol is added.

After the terminal action's existing cleanup/finalization, the allocation no
longer contributes to the bridge's `Running` membership and the bridge
converges the row again, including its established empty-backend behavior when
there are no remaining allocations. A replacement allocation, if existing
`WorkloadLifecycle` policy creates one, is an attempt under the same allocation
ID (ADR-0099; `workload_lifecycle.rs::restart_allocation_action` copies
`row.alloc_id`). The existing ServiceLifecycle terminal veto survives that
restart. Membership disappearance/reappearance must not permanently defeat the
unchanged health predicate. The former distinct-ID assertion was a design
premise error, not authority to change restart identity or create new policy.

### Lifecycle Gate Ownership — changed backend eligibility gate

This amendment changes one existing health gate; it is not a new lifecycle
state machine.

| Contract field | Decision |
|---|---|
| Owner | `ServiceLifecycle` remains the sole author of `Backend.healthy` and of the existing startup terminal decision. |
| Gate promise | A `Running` allocation for which this tick decides existing `StartupProbeFailed` is ineligible (`healthy: false`); otherwise the existing readiness/no-readiness predicate applies. |
| Affected result | Only the `healthy` bit in the existing full `ServiceBackendRow` for that allocation. A successfully written false bit makes the existing resolver return `MeshUnreachable` for a known frontend with no healthy backend. |
| Failure projection | A failed backend-row write is returned through the existing action-shim error path. Because the shim continues to `FinalizeFailed`, it can leave a pre-existing healthy row until normal existing convergence repairs it; this ADR claims no durable withdrawal-before-terminal guarantee on that failure path. |
| Unaffected states and owners | `Running` retains Beacon/driver-start meaning; `Stable` remains startup success; liveness remains the existing `ServiceLifecycle` `StopAllocation`; `WorkloadLifecycle` remains sole restart authority. |
| Ordering | The reconciler constructs `WriteServiceBackendRow { healthy: false }` before `FinalizeFailed`. The shim attempts them serially in that order, but success of the first is not a prerequisite for the second under its existing continue-on-error contract. |
| Counterexamples retained | A non-terminal no-readiness allocation stays healthy; a late startup/readiness pass cannot override its unchanged terminal veto; liveness/restart semantics do not change; automatic replacement reuses the ID, while a genuinely distinct allocation uses ordinary eligibility evaluation. |
| Evidence lanes | Existing reconciler acceptance test proves constructed false-row/action order plus the no-readiness control and late-probe/replacement cases; built-product E09/E13 prove the healthy-store normal path, not a store-write-fault guarantee. |

### D4 — Scope and test obligations

This amendment changes only the relationship between the existing terminal
startup decision and the existing backend-health calculation/order. It does
not change:

- the Beacon/action-shim meaning of `Running`, driver start, or VM networking;
- Stable success, startup deadline/attempt policy, probe target projection,
  TCP result classification, readiness thresholds, or liveness termination;
- no-readiness behavior for allocations that have not terminally failed
  startup;
- the existing `Action`, `TerminalCondition`, `ServiceBackendRow`,
  `Backend`, store, wire, CLI, configuration, or restart API surface; or
- bridge membership ownership, stream-cap behavior, or UDP/HTTP scope.

DELIVER must add bounded evidence at the existing reconciler and built-product
boundaries:

1. an existing `ServiceLifecycle` acceptance test drives a Running,
   no-readiness allocation through the existing `StartupProbeFailed` branch
   and proves the emitted full backend row marks that allocation unhealthy;
2. that test proves only construction order — `WriteServiceBackendRow`
   precedes the corresponding `FinalizeFailed` in the one action vector — and
   that the non-terminal no-readiness, late-probe, and fresh-replacement cases
   retain the gate contract above;
3. E09's 100 unbound trials prove `StartupProbeFailed` and prove each matching
   peer VM Job cannot receive the guest reply; its healthy paired trials remain
   serving; and
4. E13's unbound inferred-startup control remains ineligible and refuses its
   peer Job after reporting `StartupProbeFailed`.

The Rust test must carry the repository-required per-test Contract Shape
declaration. The expectations remain black-box built-binary evidence and do
not recreate the lifecycle assertion in shell.

## Alternatives considered

1. **Leave no-readiness as unconditional health — rejected.** It contradicts
   E09's accepted failed-deployment/no-peer-service outcome and treats a
   terminal startup failure as less significant than absence of an optional
   readiness declaration.
2. **Rewrite `Running` to Failed earlier — rejected.** It conflates driver
   start truth with Service health and contradicts the established Running
   ownership contract.
3. **Make startup failure request restart or add a retry policy — rejected.**
   `WorkloadLifecycle` already owns restart decisions. No reachable evidence
   requires a second startup-specific policy.
4. **Add a persisted eligibility state or new backend-owner reconciler —
   rejected.** Existing terminal/view inputs and the existing health writer are
   sufficient; adding state or ownership would be architectural expansion.
5. **Remove the backend from the terminal deciding-tick row — rejected.** A
   full row with `healthy: false` preserves the existing known-service,
   fail-closed semantics while withdrawing traffic. Subsequent bridge
   convergence already owns membership removal after the allocation leaves
   Running.
6. **Publish the terminal before constructing the withdrawal — rejected.** It
   would make the normal successful-write path advertise the old healthy row
   before the reconciler has even proposed the existing withdrawal action.

## Consequences

- On the normal successful backend-row write path, a Service terminally failing
  startup becomes unavailable to peer traffic without lying about a successful
  VM start. The existing row-write-error path has only its current convergence
  behavior, not a new terminal-delivery guarantee.
- Existing no-readiness Services keep immediate eligibility until a terminal
  startup failure; no readiness policy or default changes.
- The change is an extension of `ServiceLifecycle` only and uses existing
  data/action paths, so no new component diagram node or persistence migration
  is required.

## References

- ADR-0055, ADR-0079, ADR-0080, ADR-0087, ADR-0090, ADR-0094, ADR-0095
- `docs/feature/service-kind-vm-workloads/distill/test-scenarios.md` S-SVM-18,
  S-SVM-25, and S-SVM-29
- `verification/expectations/E09-vm-service-tcp-truthfulness-100/runner.sh`
- `verification/expectations/E13-vm-service-inferred-tcp-startup/runner.sh`
