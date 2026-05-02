# Slice 3 — Job-lifecycle reconciler + action shim (the convergence loop closes; includes `job stop`)

**Story**: US-03
**Walking skeleton row**: 3 (Watch convergence) + 4 (Stop or recover)
**Effort**: ~1-2 days (upper end; the slice touches new variants + a new runtime layer + new DST invariants in concentration AND the stop-and-drain end-to-end)
**Depends on**: Slices 1, 2. **HARD DESIGN DEPENDENCY**: see DoR.

## Outcome

`Action::StartAllocation { alloc_id, job_id, node_id, spec }`, `Action::StopAllocation { alloc_id }`, and `Action::RestartAllocation { alloc_id }` variants exist on the `Action` enum. `JobLifecycle` reconciler (`crates/overdrive-control-plane/src/reconciler/job_lifecycle.rs` proposed) implements `Reconciler` with a real `Self::View = JobLifecycleView` that carries restart counts and backoff timestamps. `AnyReconciler::JobLifecycle` and `AnyReconcilerView::JobLifecycle` variants land alongside `NoopHeartbeat` per the extension contract documented in `crates/overdrive-core/src/reconciler.rs`. The lifecycle reconciler reads `desired` (job spec from rkyv-hydrated IntentStore) and `actual` (current AllocStatusRow set), calls Slice 1's `schedule(...)` to decide placement, and emits `Action::StartAllocation`. The runtime's NEW **action shim** consumes allocation-management Actions and dispatches them to `Arc<dyn Driver>` (production: ProcessDriver from Slice 2; DST: SimDriver), then writes the resulting `AllocStatusRow` back to ObservationStore.

`AppState` extends with `driver: Arc<dyn Driver>`. The lifecycle reconciler is registered in `run_server_with_obs()` alongside `noop_heartbeat()`.

This slice ALSO ships **`overdrive job stop <id>`** end-to-end: the CLI subcommand, the corresponding handler (`POST /v1/jobs/{id}:stop` per DESIGN's pick — `DELETE /v1/jobs/{id}` is the alternative), the Job-aggregate desired-state update through IntentStore, and the lifecycle reconciler's read of stopped intent, which causes it to emit `Action::StopAllocation` for each running allocation. The action shim consumes the StopAllocation by calling `Driver::stop`, which drains the workload through Running → Draining → Terminated.

## Value hypothesis

*If* we can't close the convergence loop end-to-end via the §18 reconciler primitive (pure `reconcile` + async action shim) AND give the operator a clean stop-and-drain affordance, *then* the §18 architectural commitment is performative — every later reconciler ships untested or compromises purity, and the operator has no way to reverse a `job submit`. *Conversely*, if we can — and DST proves convergence + purity simultaneously, AND `job stop` drains cleanly — Phase 2+ reconciler authors have a reference implementation that satisfies the whitepaper's contract.

## Disproves (what's the named pre-commitment we're falsifying)

- **"The lifecycle reconciler must perform I/O — it can't really be pure."** No — the action shim is the I/O boundary; the reconciler stays pure. The codebase research confirms this matches Anvil's `reconcile_core` + shim pattern (USENIX OSDI '24).
- **"We need to invent a new abstraction to dispatch Actions."** No — the action shim is just a runtime function: take an Action, switch on the variant, call Driver if it's allocation-management, write AllocStatusRow.
- **"`State` can stay opaque forever."** No — this slice forces the structural blocker open: lifecycle needs Job + Allocation data to converge.
- **"`job stop` belongs in a separate slice from convergence."** No — stop is the inverse of start; both pass through the same lifecycle reconciler + action shim path. Splitting them would force two slices to land the same I/O machinery.

## Scope (in)

- `Action::{StartAllocation, StopAllocation, RestartAllocation}` variants (additive on the existing Action enum).
- `JobLifecycle` reconciler struct + `JobLifecycleView` (libSQL-hydrated; `restart_counts: BTreeMap<AllocationId, u32>`, `next_attempt_at: BTreeMap<AllocationId, Instant>`).
- `AnyReconciler::JobLifecycle(JobLifecycle)` + match arms in `name`, `hydrate`, `reconcile`.
- `AnyReconcilerView::JobLifecycle(JobLifecycleView)`.
- Action shim (NEW runtime layer in `overdrive-control-plane`): consumes `Vec<Action>` from the reconciler runtime, dispatches allocation actions to `Arc<dyn Driver>`, writes AllocStatusRow on completion.
- `AppState::driver: Arc<dyn Driver>` extension.
- Lifecycle reconciler registered at boot via `runtime.register(job_lifecycle())?`.
- Three new DST invariants: `JobScheduledAfterSubmission`, `DesiredReplicaCountConverges`, `NoDoubleScheduling`.
- **`overdrive job stop <id>` CLI subcommand**.
- **`POST /v1/jobs/{id}:stop` handler** (path shape DESIGN-owned). Handler updates Job aggregate's desired-state in IntentStore via the existing typed-action path.
- **Lifecycle reconciler reads stopped intent and emits `Action::StopAllocation`** for each running allocation belonging to that job. Action shim calls `Driver::stop`. Allocation transitions Running → Draining → Terminated.

## Scope (out)

- Server-bootstrap cgroup slice creation (Slice 4).
- Multi-node placement (Phase 2+; the scheduler from Slice 1 already returns the single-node placement).
- `MigrateAllocation` Action variant (Phase 3+ when `overdrive-fs` cross-region migration lands).
- Per-workload resource enforcement on cgroup scope beyond what Slice 2 wires (out unless DESIGN bundled into Slice 2).

## Target KPI

- Submitting a 1-replica job to the single-node cluster produces a Running allocation within N reconciler ticks under DST.
- DST `JobScheduledAfterSubmission` and `DesiredReplicaCountConverges` invariants pass.
- The lifecycle reconciler is added to the existing `ReconcilerIsPure` invariant — twin invocation produces identical outputs.
- `NoDoubleScheduling` holds: each allocation appears in `alloc_status_rows` under exactly one `node_id` (vacuous-pass shape under N=1; the invariant still has to hold).
- `overdrive job stop` drives a Running allocation to Terminated within N reconciler ticks (extension of the same convergence assertion above).

## Acceptance flavour

See US-03 scenarios. Focus: end-to-end submit→Running under DST (using SimDriver), backoff via libSQL, purity invariant, no-double-scheduling safety invariant, stop-and-drain (Running → Draining → Terminated, cgroup scope removed).

## Failure modes to defend

- DESIGN's State shape is wrong — discovered when `hydrate()` can't materialize what `reconcile()` needs. This is why the DoR HARD-flags State as a DESIGN dependency; if it's deferred, Slice 3 can't start.
- Action shim writes AllocStatusRow but ProcessDriver's actual cgroup creation fails — shim must catch and write `state: Failed` with reason.
- Backoff math allows infinite restart loop — capped in `JobLifecycleView`'s logic; tested via DST scenario `SimDriver::always_fail`.
- `overdrive job stop` on a non-existent job — handler returns 404 Not Found; no allocation state changes.
- Process ignores SIGTERM during stop — driver escalates to SIGKILL after grace; transition still completes Running → Draining → Terminated.

## Slice taste-test

| Test | Status |
|---|---|
| ≤4 new components | BORDERLINE — JobLifecycle reconciler, AnyReconciler/View extension (treat as one), action shim, AppState extension, three new Action variants (treat as one), three new DST invariants (treat as one), `job stop` CLI+handler. 6 conceptual components. **At the upper limit; acceptable because each is small and the shim wires them all; the stop side mirrors the start side through the same shim, so the marginal cost over a stop-less version is small** |
| No hypothetical abstractions landing later | PASS — every dep exists or lands in Slices 1-2 |
| Disproves a named pre-commitment | PASS — see above |
| Production-data-shaped AC | PASS — DST end-to-end + integration test against ProcessDriver under integration-tests gate |
| Demonstrable in single session | BORDERLINE — the slice is dense; an explicit demo is "submit a job under DST seeded run; observe converged Running state; run `overdrive job stop`; observe Terminated" |
| Same-day dogfood moment | PASS — `cargo xtask dst` green, AND a Linux integration test showing real Running → Terminated on `job stop` |
