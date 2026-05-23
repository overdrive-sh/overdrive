# RCA — Service-arm convergence gap (Service workloads don't reach Running)

**Date**: 2026-05-21
**Triggered by**: step 02-04 walking-skeleton S-BDB-01 blocker
**Investigator**: nw-troubleshooter (Rex)
**Status**: RESOLVED (commit `66935193`)

---

## Symptom

Submitting a `WorkloadIntent::Service` through the real HTTPS driving
port produces **zero `alloc_status` rows** in `ObservationStore` 10 s
after admission. The same shape works end-to-end for Job-kind
workloads. Downstream of this, the `backend-discovery-bridge`
reconciler's `actual.running` set is always empty for Service
workloads, no `ServiceBackendRow` is ever written, the
`ServiceMapHydrator` never dispatches `DataplaneUpdateService`, and
the kernel-side `SERVICE_MAP` / `BACKEND_MAP` never populate.

This is the upstream blocker that makes S-BDB-01 (the
walking-skeleton TCP round-trip through the VIP) structurally
impossible.

---

## Investigation altitudes

Per `.claude/rules/debugging.md` § 5 ("compare populations") — every
altitude below is a side-by-side check of the working Job-kind path
vs the broken Service-kind path at the same call site.

### Altitude 1 — Submit-handler enqueue

**Hypothesis**: submit handler does not enqueue `WorkloadLifecycle`
evaluation for Service kind (the orchestrator's first plausible
cause).

**Prediction (if true)**: `enqueue_workload_lifecycle_eval` is gated
on `WorkloadKind::Job` somewhere on the submit path, or the Service
arm short-circuits before reaching the enqueue.

**Falsification**: a call to `enqueue_workload_lifecycle_eval` on
both the Inserted and Unchanged paths regardless of `WorkloadKind`.

**Finding**: **falsified**. The enqueue fires uniformly for both
kinds.

- `crates/overdrive-control-plane/src/handlers.rs:378` (Inserted
  branch): `enqueue_workload_lifecycle_eval(&state, &workload_id)?;`
  is unconditional after `put_if_absent` returns `Inserted`. The
  branch above it (steps 4a / 5a) wraps Service in an `if matches!`
  for VIP allocation only — the enqueue itself is not behind that
  branch.
- `crates/overdrive-control-plane/src/handlers.rs:409` (Unchanged
  branch): same enqueue, same lack of `kind` gate.
- The kind discriminator IS persisted by the same handler at
  `handlers.rs:371` (`state.store.put(kind_key.as_bytes(),
  &[workload_kind.discriminator_byte()])`), so downstream code can
  observe `WorkloadKind::Service` correctly.

The handler is doing its job. Move down-altitude.

### Altitude 2 — Runtime hydrate_desired

**Hypothesis**: `hydrate_desired` for `WorkloadLifecycle` does not
project the Service spec onto the `WorkloadLifecycleState` shape the
reconciler expects.

**Prediction (if true)**: `WorkloadLifecycleState.job` would be
populated for Service kind from `WorkloadIntent::Service(svc).driver`
+ `.resources` etc., OR `WorkloadLifecycleState` would carry an
auxiliary Service-shape field that the reconciler reads on the
Service branch.

**Falsification**: `WorkloadLifecycleState.job` is `Option<Job>` and
populated only for `WorkloadIntent::Job` variants — Service workloads
arrive at `reconcile()` with `desired.job = None`.

**Finding**: **confirmed**. This is the defect.

- `crates/overdrive-control-plane/src/reconciler_runtime.rs:1267-1275`
  — `read_job` returns `(None, Some(digest))` for `WorkloadIntent::
  Service(_)`:

  ```rust
  match &intent {
      overdrive_core::aggregate::WorkloadIntent::Job(job) => Ok((Some(job.clone()), None)),
      overdrive_core::aggregate::WorkloadIntent::Service(_) => {
          let digest = intent.spec_digest().map_err(...)?;
          Ok((None, Some(digest)))
      }
      overdrive_core::aggregate::WorkloadIntent::Schedule(_) => Ok((None, None)),
  }
  ```

- `crates/overdrive-control-plane/src/reconciler_runtime.rs:1047-1056`
  — the hydrate site folds that `None` into the state struct
  directly:

  ```rust
  let s = WorkloadLifecycleState {
      workload_id: workload_id.clone(),
      job,                       // <-- None for Service workloads
      desired_to_stop,
      nodes,
      allocations: BTreeMap::new(),
      workload_kind,             // <-- WorkloadKind::Service, but downstream never branches on this for emission
      service_spec_digest,       // <-- Some(digest), but only used by service_vip_release_emission
  };
  ```

- The hydrate-side comment at `reconciler_runtime.rs:1241-1244`
  explicitly states the current intent: "For Service and Schedule
  kinds, `read_job` returns `Ok(None)` — the reconciler's
  `desired.job` field is `None` for those variants, which is the
  correct 'no Job allocation target' shape for Phase 1's Service-arm
  (allocations are not yet spawned for Services)."

  That comment is the candid admission that the Service-arm
  *allocation-emission* gap is intentional in the hydrate layer,
  pending implementation in the reconciler — which never landed.

The `desired.job: Option<Job>` shape is the wrong type for a
kind-agnostic reconciler. `ServiceV1` carries an identical
`driver: WorkloadDriver` (see `crates/overdrive-core/src/aggregate/mod.rs:391-399`)
and could feed the same allocation path — but the hydrate-to-state
plumbing throws it away.

### Altitude 3 — Reconciler emission branch

**Hypothesis**: `WorkloadLifecycle::reconcile` Service-arm takes a
degenerate branch (the orchestrator's third plausible cause).

**Prediction (if true)**: the reconcile body has an early-return on
`workload_kind == Service` (no StartAllocation emitted), OR the
emission branch is gated on `desired.job.is_some()` and falls through
to a no-op when `job` is `None`.

**Falsification**: the reconcile body matches on
`desired.job.as_ref()` and the `Some(job)` arm is the only path that
emits `Action::StartAllocation`. The `None` arm runs the
"GC absent" branch which only stops Running allocs and otherwise
emits nothing.

**Finding**: **confirmed**. The downstream consequence of altitude
2's hydrate gap.

- `crates/overdrive-core/src/reconciler.rs:1411` —
  `match desired.job.as_ref()`. Two arms total:
  - `None => ...` at `reconciler.rs:1441-1464` — emits
    `StopAllocation` for any Running alloc (a Service workload at
    initial submit has no Running allocs, so this arm returns
    `(Vec::new(), view.clone())`).
  - `Some(job) => ...` at `reconciler.rs:1466-1755` — contains the
    `Action::StartAllocation` emission at `reconciler.rs:1739-1751`,
    routed through `first_fit_place`.

- Because `desired.job` is unconditionally `None` for Service
  workloads (per altitude 2), the `None` arm runs every tick:
  - No Running allocs exist → `stop_actions.is_empty()` → returns
    empty actions and clears backoff state.
  - The reconciler is "converged" from its own perspective: nothing
    is desired (no `Some(job)`), nothing is running. Quiescent.

- The presence of `desired.workload_kind == WorkloadKind::Service`
  in scope is consulted *only* by `service_vip_release_emission`
  (`reconciler.rs:1845-1875`), which is a release-path helper — it
  emits `Action::ReleaseServiceVip` when a Service alloc reaches
  terminal state. It plays no role in the start path.

- Spot-check: `Action::StartAllocation` emission in the codebase
  occurs at exactly one site — `crates/overdrive-core/src/reconciler.rs:1739`.
  There is no Service-arm equivalent.

### Altitude 4 — Action shim / driver

Not investigated. The chain is already broken upstream at altitudes
2 and 3 — no `StartAllocation` is emitted, so no
`action_shim::dispatch` arm fires, so no `driver.start(&spec)` call,
so no `alloc_status` row. The downstream layers (action shim,
ExecDriver, exit observer) are operating correctly given an empty
actions vector.

---

## Root cause

**Two-layer compound defect, both layers in the control-plane crate**
(no defect in `overdrive-core`'s reconciler shape — that's
kind-agnostic enough that a single hydrate fix would fix
the whole chain).

### Primary defect

`crates/overdrive-control-plane/src/reconciler_runtime.rs:1267-1275`
— `read_job` discards the `ServiceV1.driver` / `.resources` / `.id`
fields and returns `(None, Some(digest))` for Service workloads.
The hydrate-time intent says "Service has no allocation target,"
which is contradicted by:

1. `architecture.md:154-156` (this feature's design): step 3 is
   *"`WorkloadLifecycle.reconcile` emits `StartAllocation`
   (existing behaviour)"* and step 4 is *"Action shim dispatches
   `StartAllocation` → `ExecDriver` spawns the process; alloc
   transitions Pending → Running."*
2. The `ServiceV1` struct shape itself
   (`crates/overdrive-core/src/aggregate/mod.rs:391-399`), which
   carries `driver: WorkloadDriver`, `resources: Resources`,
   `replicas: NonZeroU32` — every field the allocation path needs.

### Secondary defect (downstream consequence)

`crates/overdrive-core/src/reconciler.rs:1411` matches on
`desired.job.as_ref()`. The `None` arm is the GC branch, not the
"converge a Service" branch. The reconciler has no path to emit
`StartAllocation` when `job` is `None` even though `workload_kind ==
Service` and `service_spec_digest` is `Some(_)` — both signals are
in scope but unused by the start path.

A correctly-shaped fix lives at the hydrate boundary (project
`ServiceV1` into a kind-agnostic `WorkloadLifecycleState` shape so
the reconciler's `Some(job)` arm fires), but a fix at the reconciler
itself (a new Service branch reading `workload_kind` +
`service_spec_digest` + a yet-to-exist `service_driver` field) is
also possible. The hydrate fix is structurally smaller and preserves
the kind-agnostic reconciler invariant.

---

## Fix shape estimate

The hydrate-side fix is the natural shape. Two options:

### Option H1 — kind-agnostic `Job`-shaped projection (smallest diff)

Treat `WorkloadIntent::Service(svc)` and `WorkloadIntent::Job(job)`
identically at the hydrate boundary by building a `Job` value from
`ServiceV1`'s fields and feeding it into the existing
`WorkloadLifecycleState.job: Option<Job>` slot.

- `crates/overdrive-control-plane/src/reconciler_runtime.rs:1267-1275`
  (~10 LOC): in the `WorkloadIntent::Service(svc)` arm of `read_job`,
  construct a `Job { id: svc.id.clone(), replicas: svc.replicas,
  resources: svc.resources, driver: svc.driver.clone() }` and
  return `(Some(job), Some(digest))`. The digest stays
  Service-specific.
- `crates/overdrive-control-plane/src/reconciler_runtime.rs:1241-1244`
  (~5 LOC): update the docstring on `read_job` to match the new
  shape ("returns a kind-agnostic `Job` projection for both Job and
  Service variants; Service workloads pick up their driver + resource
  envelope identically").
- No change required at `crates/overdrive-core/src/reconciler.rs`.
  The existing `Some(job) => ...` arm at `reconciler.rs:1466`
  handles emission, and `desired.workload_kind` continues to be
  threaded onto every emitted action via the `kind:
  desired.workload_kind` field already present at
  `reconciler.rs:1682` (RestartAllocation) and `reconciler.rs:1750`
  (StartAllocation).
- **Estimated diff**: ~15–20 LOC in one file. Plus tests below.

**Trade-off**: introduces an internal mutation of `WorkloadIntent::
Service` into a `Job` shape inside the control-plane crate. The
shape is structurally identical (`Job` and `ServiceV1` are
field-for-field equivalent excluding `listeners`, which the
reconciler doesn't read), so the projection is lossless from the
reconciler's perspective. Future Service-only fields (e.g.
per-listener health probes, draining strategy) cannot live on this
projection — they'd need a separate hydrate field.

### Option H2 — typed Service-arm field on `WorkloadLifecycleState`

Add `pub service_driver: Option<(WorkloadDriver, Resources)>` (or a
typed `ServiceAllocTarget` newtype) to `WorkloadLifecycleState` and
have the reconciler's match consult it in addition to
`desired.job`.

- `crates/overdrive-core/src/reconciler.rs:406-470` (~5 LOC):
  add the field.
- `crates/overdrive-core/src/reconciler.rs:1411` (~50 LOC):
  reshape the match to `match (desired.job.as_ref(),
  desired.service_driver.as_ref())` with a fold or a new
  `match` arm; refactor the Run-branch body to take a
  `&WorkloadDriver` + `&Resources` instead of `&Job`.
- `crates/overdrive-control-plane/src/reconciler_runtime.rs:1267-1275`
  (~10 LOC): populate `service_driver` from `ServiceV1`.
- All emit sites at `reconciler.rs:1672-1751`: factor the
  `WorkloadDriver::Exec(Exec { command, args }) = &job.driver`
  destructure to consume the new field.
- **Estimated diff**: ~80–120 LOC across two files. Plus tests
  below.

**Trade-off**: cleaner type-level distinction (no synthesised
`Job` for a Service workload), but ~5–8x the diff and the
reconciler signature surface area grows. Option H1 is the right
shape for Phase 1.

### New tests (either option)

The existing acceptance test
`crates/overdrive-control-plane/tests/acceptance/service_workload_convergence_no_panic.rs`
asserts *liveness preservation only* — its bar is "convergence tick
must not panic." A Service-arm convergence test that asserts on
emission shape is missing. Add:

- `crates/overdrive-control-plane/tests/acceptance/service_workload_emits_start_allocation.rs`
  (~80 LOC, mirror of `service_workload_convergence_no_panic.rs`
  structure): submit Service, tick once, assert
  `state.obs.alloc_status_rows()` is non-empty AND the first row
  carries `WorkloadKind::Service` AND the row's spec carries the
  Service's `driver.command`. Roughly mirrors
  `tests/integration/workload_lifecycle/submit_to_running.rs` but
  for the Service kind.
- Tier 1 DST counterpart: extend the existing
  `crates/overdrive-sim/src/dst/invariants/` Service-kind invariant
  set with a `ServiceArmEmitsStartAllocation` invariant whose
  property is "for any seed where a Service workload is submitted,
  within N ticks an `Action::StartAllocation` carrying
  `kind: WorkloadKind::Service` is observed in the dispatch log."

---

## Test coverage gap

The existing test landscape covers the path *up to* the convergence
gap but not *through* it.

| Test | What it covers | What it does NOT cover (and why this defect slipped) |
|------|----------------|---------------------------------------------------|
| `tests/acceptance/service_workload_convergence_no_panic.rs:65` | Submit Service → eval enqueued → tick completes without panic | Does not assert that `StartAllocation` was emitted. The bar is "no panic" (regression-shaped — it was added to defend against an `unreachable!()` panic in `read_job`). |
| `tests/acceptance/service_vip_submit_acceptance.rs` | Submit Service → VIP allocator memoises + returns a VIP | Does not exercise the convergence loop. Tests the allocator boundary only. |
| `tests/integration/workload_lifecycle/submit_to_running.rs` | Submit Job → reconcile → ExecDriver → alloc reaches Running | Job-kind only. The handler's enqueue + reconciler's `Some(job)` arm are both covered for Job kind. |
| `tests/integration/backend_discovery_bridge/walking_skeleton.rs` | (RED scaffold) Submit Service → Running → bridge → hydrator → dataplane → TCP round-trip | Currently `#[should_panic]` — the *intended* gate; this is the test that surfaced the defect. |
| `tests/acceptance/service_workload_convergence_tick_does_not_panic` (= the no_panic test above) | "Doesn't panic" only | The hydrate-side comment at `reconciler_runtime.rs:1241-1244` ("Phase 1 Service-arm allocations are not yet spawned") was treated as the contract, not as a gap. The regression test pinned the panic absence without questioning whether the *post-panic* behavior was correct. |

The structural test-coverage gap: **there is no test that asserts a
Service workload's convergence trajectory produces a non-empty
`alloc_status` row stream.** This defect is exactly the shape that
mutation testing or a property-based "every WorkloadKind that has a
driver field reaches Running" invariant would have caught.

---

## Recommendation

**Option A: fix-in-scope, before walking-skeleton lands.** Take
Option H1 above (~15–20 LOC in `reconciler_runtime.rs`) plus the
Service-arm emission acceptance test, land it as a prerequisite
slice of `backend-discovery-bridge-service-reachability` step
02-04. Then the walking-skeleton RED scaffold can transition to
GREEN inside the same DELIVER wave.

Justification:

1. **The fix shape is small and structurally clean** — H1 is ~15
   LOC at one site, with no signature changes to the core
   reconciler. The risk surface is bounded.
2. **The defect blocks S-BDB-01 absolutely** — there is no
   workaround at the bridge / hydrator / dataplane layer; without
   Running allocs for Services, every downstream stage operates on
   an empty actual-set.
3. **The defect contradicts this feature's own design** —
   `architecture.md:154-156` explicitly names step 3 as "existing
   behaviour" that the design relies on. The crafter who wrote that
   line was operating against the same hydrate-comment-as-contract
   that masked the gap.
4. **Test coverage gap is in scope** — adding a Service-arm
   emission acceptance test alongside the fix closes the structural
   hole that let this defect slip past Job-arm coverage. The cost
   is bounded (one ~80 LOC test file mirroring an existing one).

**Option B (rejected): defer via GH issue.** Tracking the gap as a
separate issue and shipping the walking-skeleton as a known-broken
RED scaffold would leave the entire backend-discovery-bridge
feature with no end-to-end evidence. The bridge, the hydrator, the
EbpfDataplane Earned-Trust probe, and the DST invariants would all
be passing while the joint promise (TCP traffic through a VIP) is
structurally unreachable. The feature's value is unobservable until
this defect is fixed; deferring it defers the whole feature's gate.

**Next probe if Option A is approved and the fix lands but the
walking-skeleton still fails**: re-run this RCA at altitudes 4–5
(action shim arm, ExecDriver dispatch for Service kind, exit
observer behaviour for long-running Service-kind allocs). The
chain is currently broken so far upstream that downstream
diagnostics produce no signal.

---

## Resolution

**Date resolved**: 2026-05-21
**Fix commit**: `66935193` — *fix(overdrive-control-plane): Service-arm hydrate projects driver + resources into Job-shape*

**Approach taken**: Option A (fix-in-scope) + Option H1 (kind-agnostic
`Job`-shaped projection at the hydrate boundary) per the recommendation
above. ~15 LOC change at `crates/overdrive-control-plane/src/
reconciler_runtime.rs:1267-1275`; no reconciler-signature change
required.

**Verification**:

- New acceptance test `crates/overdrive-control-plane/tests/acceptance/
  service_workload_emits_start_allocation.rs::service_workload_convergence_emits_start_allocation_and_running_row`
  was written first; confirmed FAIL on parent `27e340b4` with the
  documented failure message *"Service workload convergence must
  produce a Running alloc within 10 ticks; pre-fix value: zero rows
  because read_job returned (None, _) for Service intents"*; then
  PASS after the H1 fix landed. RED → GREEN flip verified inside
  the same fix commit.
- Existing `service_workload_convergence_no_panic.rs` continues to
  pass — no regression on liveness-preservation contract.
- Full `overdrive-control-plane` crate suite (312 tests, all 312 pass,
  1 skipped) green via `cargo xtask lima run -- cargo nextest run -p
  overdrive-control-plane --features integration-tests`.
- Workspace suite (1410 tests, all 1410 pass, 13 skipped) green via
  `cargo xtask lima run -- cargo nextest run --workspace --features
  integration-tests`.
- `cargo xtask lima run -- cargo check --workspace --features
  integration-tests --all-targets` clean.
- `cargo xtask lima run -- cargo clippy --workspace --all-targets
  --features integration-tests -- -D warnings` clean.
- `cargo xtask dst-lint` zero violations.

**Coverage gap closed**: the new acceptance test asserts on
`alloc_status_rows()` non-emptiness, the row's `kind ==
WorkloadKind::Service`, and the broker's `dispatched` counter as a
structural witness that `Action::StartAllocation` actually fired. The
prior coverage shape — Job-arm `submit_to_running.rs` + Service-arm
`service_workload_convergence_no_panic.rs` — left the Service-arm
convergence *trajectory* unobserved; this test closes that hole at
the same altitude as `submit_to_running.rs` covers it for Job kind.

**Logged in**: `docs/feature/backend-discovery-bridge-service-reachability/deliver/upstream-issues.md` § UI-04.

**Downstream unblocking**: step 02-04's `walking_skeleton.rs` (the
`#[should_panic(expected = "RED scaffold")]` scaffold that surfaced
this defect) can now transition to GREEN — the bridge, hydrator,
dataplane, and TCP round-trip operate against a non-empty actual-set
once a Service submit propagates through the corrected hydrate path.
