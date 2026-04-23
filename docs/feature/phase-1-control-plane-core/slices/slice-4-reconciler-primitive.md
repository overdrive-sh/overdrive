# Slice 4 — Reconciler primitive: trait + runtime + evaluation broker

**Story**: US-04
**Walking skeleton row**: 4 (Host reconcilers)
**Effort**: ~1-2 days (the largest slice by risk)
**Depends on**: Slice 1 (Action enum references aggregate IDs); otherwise independent of Slices 2-3 and can run in parallel with Slice 3.

## Outcome

`Reconciler` trait in `overdrive-core` matches whitepaper §18 exactly: pure function over `(desired, actual, db) -> Vec<Action>`, no `.await`, no I/O. `ReconcilerRuntime` registers reconcilers at boot and exposes a read surface (`registered()`, broker counters). `EvaluationBroker` collapses duplicate `(reconciler, target)` evaluations into a cancelable set, drains the survivors, and reaps the cancelled ones. Per-primitive private libSQL DBs are provisioned and passed to `reconcile(...)`. DST invariants assert these contracts hold.

## Value hypothesis

*If* the reconciler primitive isn't shipped with storm mitigation from day one, *then* Phase 2+ reconciler scale becomes a Nomad-shaped incident. The whitepaper §18 *Evaluation Broker — Storm-Proof Ingress* section explicitly calls out HashiCorp's retrofit; Overdrive's differentiator is shipping this native, not retrofitted. Anything less kills the whitepaper claim.

## Scope (in)

- `Reconciler` trait in `overdrive-core::reconciler::Reconciler`:
  - `fn reconcile(&self, desired: &State, actual: &State, db: &Db) -> Vec<Action>`
  - No `async fn`, no `.await`, no I/O — purity is load-bearing per whitepaper §18 + development.md
- `Action` enum in `overdrive-core::reconciler::Action` with at minimum:
  - `Noop` (so the noop-heartbeat reconciler has something to return)
  - `HttpCall { request_id, correlation, target, method, body, timeout, idempotency_key }` per development.md (shim runtime is Phase 3+, but the variant is part of the primitive contract)
  - `StartWorkflow { spec, correlation }` (lightweight placeholder; workflow primitive lands Phase 3)
- `ReconcilerRuntime` in the control-plane crate:
  - Registers reconcilers at boot through a typed registration surface
  - Exposes `registered() -> Vec<&dyn ReconcilerHandle>` (or a read-only snapshot type)
  - Owns an `EvaluationBroker`
- `EvaluationBroker`:
  - Keyed on `(reconciler_name, target_resource)` per whitepaper §18
  - Cancelable-eval-set: a new evaluation for an existing key moves the prior one to the cancelable set
  - Reaps cancelled evaluations in bulk (reaper is a reconciler itself — `evaluation-broker-reaper` — per whitepaper §18 Built-in Primitives; in Phase 1 it can be an in-runtime loop, not a user-facing reconciler)
  - Surfaces counters (`queued`, `cancelled`, `dispatched`) visible to `cluster status`
- Per-primitive private libSQL DB provisioning:
  - Each registered reconciler gets a dedicated libSQL file (path includes the reconciler name)
  - `Db` handle is passed to `reconcile(...)` — reads and writes stay private to the primitive per development.md state-layer hygiene
- `noop-heartbeat` reconciler registered at boot as living proof the contract holds
- New DST invariants in `overdrive-sim::invariants`:
  - `at_least_one_reconciler_registered` (always-true-at-boot)
  - `duplicate_evaluations_collapse` (fire N evaluations at the same key in one tick → 1 dispatched, N-1 cancelled)
  - `reconciler_is_pure` — property-shaped: a reconciler invoked twice with identical inputs produces identical outputs (no hidden state via Instant::now())

## Scope (out)

- Job-lifecycle reconciler — phase-1-first-workload
- `Action::HttpCall` runtime shim — Phase 3 (#3.11 on the roadmap)
- Workflow primitive (the durable async peer to reconcilers) — Phase 3 (#3.2)
- Real reconcilers beyond noop-heartbeat (drain, right-sizing, rolling deploy, …) — their home phases per roadmap
- ESR formal verification — acknowledged as a future target; Phase 1 ships the contract that ESR proofs will hold against

## Target KPI

- DST passes `at_least_one_reconciler_registered` on every run
- DST passes `duplicate_evaluations_collapse` under N concurrent evaluations on the same key (N ≥ 3)
- DST passes `reconciler_is_pure` — twin invocation under the same inputs produces identical outputs
- Per-reconciler libSQL files are filesystem-isolated (no reconciler can accidentally open another's DB by path)
- `cluster status` output surfaces the reconciler registry and broker counters (operator-visible proof the runtime is alive)

## Acceptance flavour

See US-04 scenarios. Focus: purity of `reconcile(...)`, cancelable-eval-set, private libSQL isolation, DST invariant coverage.

## Failure modes to defend

- A reconciler sneakily uses `Instant::now()` in its `reconcile(...)` body — caught by the dst-lint gate (phase-1-foundation) AND by the new `reconciler_is_pure` DST invariant
- Evaluation broker fails to collapse duplicates under tight timing windows
- Two reconcilers share a libSQL file by accident (path derivation collision)
- `ReconcilerRuntime::registered()` returns the empty set after a successful boot — silent regression
- The broker's reaper never runs, so `cancelled` grows unboundedly
