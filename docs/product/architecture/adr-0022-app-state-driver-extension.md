# ADR-0022 — `AppState::driver: Arc<dyn Driver>` extension

## Status

Accepted. 2026-04-27. Decision-makers: Morgan (proposing), user
ratification 2026-04-27. Tags: phase-1, first-workload,
application-arch.

## Context

The Phase 1 control-plane server (`crates/overdrive-control-plane`)
holds shared per-process state in `AppState`:

```rust
#[derive(Clone)]
pub struct AppState {
    pub store:   Arc<LocalIntentStore>,
    pub obs:     Arc<dyn ObservationStore>,
    pub runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
}
```

The `phase-1-first-workload` feature introduces an action shim
(ADR-0023) that consumes lifecycle Actions emitted by the
`JobLifecycle` reconciler and dispatches allocation-management
operations to a `Driver` implementation. The shim needs a handle to
that driver. The shim runs inside the runtime; its needs propagate
into `AppState` because:

1. The runtime is constructed inside `run_server_with_obs` and
   stored on `AppState`. The shim is owned by the runtime (see
   ADR-0023).
2. Test fixtures need to inject a `SimDriver` so the default-lane
   tests do not spawn real processes.
3. Future Phase 2+ multi-driver dispatch (Process / MicroVm / Wasm)
   will compose against the same field.

The DISCUSS wave (Key Decision 5) pre-decided that `AppState` is the
right home for the driver handle. This ADR records the decision and
the migration shape for the existing `run_server_with_obs` test
callers.

## Decision

### 1. New field: `driver: Arc<dyn Driver>` on `AppState`

```rust
#[derive(Clone)]
pub struct AppState {
    pub store:   Arc<LocalIntentStore>,
    pub obs:     Arc<dyn ObservationStore>,
    pub runtime: Arc<reconciler_runtime::ReconcilerRuntime>,
    /// Workload driver. Production: `ProcessDriver` (Phase 1 single
    /// driver). DST / unit tests: `SimDriver` from `overdrive-sim`.
    /// Phase 2+ may interpose a `DriverRegistry` selecting per-
    /// `DriverType` (see Forward-compat below).
    pub driver:  Arc<dyn Driver>,
}
```

`Driver` is the existing trait in
`crates/overdrive-core/src/traits/driver.rs` (`Send + Sync +
'static`, async methods `start`, `stop`, `status`, `resize`). The
`Arc<dyn Driver>` shape preserves the ADR-0012 / ADR-0013 pattern of
trait-object swap at construction time: production `run_server`
threads `ProcessDriver`; tests thread `SimDriver`; the action shim
sees only `&dyn Driver`.

### 2. Production wiring: `ProcessDriver` constructed inside `run_server`

`run_server` is the production entry point that wires the
single-node `LocalObservationStore`. Per the parallel pattern
established for `obs`, `run_server` constructs `ProcessDriver` and
delegates to `run_server_with_obs_and_driver` (the renamed /
extended version of `run_server_with_obs`).

```rust
pub async fn run_server(config: ServerConfig) -> Result<…, …> {
    let obs:    Arc<dyn ObservationStore> = … ;       // existing path
    let driver: Arc<dyn Driver> = Arc::new(
        ProcessDriver::new(/* cgroup root, etc. */)?
    );
    run_server_with_obs_and_driver(config, obs, driver).await
}
```

The driver constructor is fallible (the cgroup root may not be
delegated; see ADR-0028). Failures surface through
`ControlPlaneError::Internal` exactly as `LocalIntentStore::open`
failures do today.

### 3. Test fixture path: `run_server_with_obs_and_driver(config, obs, driver)`

The existing `pub async fn run_server_with_obs(config, obs)` is
renamed to `run_server_with_obs_and_driver(config, obs, driver)`,
extending the test-fixture contract symmetrically with the
production path. Every existing test caller of `run_server_with_obs`
is migrated to pass an `Arc<dyn Driver>` — typically a `SimDriver`
constructed via `overdrive-sim` (test-only dep).

The migration is mechanical: each test fixture gains one extra
constructor line and one extra positional argument. The DISCUSS
wave's "mechanical migration" framing for D2 acknowledges this is a
broad-but-shallow change rather than a deep restructuring.

A convenience wrapper `run_server_for_testing(config)` is NOT added
— the Phase 1 test-fixture pattern is "construct what you need
explicitly," matching the existing `run_server_with_obs` caller
shape. Adding a defaulted convenience wrapper would shadow the
explicit-injection contract that is load-bearing for the DST-shape
test discipline.

### 4. Action shim consumption — `shim.dispatch(actions, &state)`

The action shim (ADR-0023) is invoked from the reconciler runtime's
post-reconcile pipeline. The shim signature takes
`&dyn Driver` (or equivalent — the exact signature is ADR-0023's
remit), and the runtime's invocation path passes `state.driver.as_ref()`.
Concretely:

```rust
// inside reconciler_runtime::action_shim (ADR-0023)
pub async fn dispatch(
    actions: Vec<Action>,
    driver:  &dyn Driver,
    obs:     &dyn ObservationStore,
    tick:    &TickContext,
) -> Result<(), ShimError> { … }
```

The shim does not own the driver; `AppState` does. This keeps
shutdown semantics clean: `AppState` drops at server shutdown; the
`Arc<dyn Driver>` reaches refcount zero; the driver's `Drop` impl
runs.

### 5. `AppState: Clone` is preserved

The existing `#[derive(Clone)]` continues to apply — every field is
either `Arc<…>` or holds an `Arc<…>` internally. Axum's
`with_state(state)` shape requires this; the per-request handler
clones the state envelope (cheap — three Arc bumps + one new Arc
bump for the driver field).

## Alternatives considered

### Alternative A — DriverRegistry from day one

Introduce `DriverRegistry { process: Arc<ProcessDriver>, micro_vm:
Option<Arc<MicroVmDriver>>, wasm: Option<Arc<WasmDriver>>, … }` and
hold the registry on `AppState`. The action shim looks up the right
driver per `AllocationSpec::driver_type`.

**Rejected for Phase 1.** The Phase 1 first-workload feature ships
exactly one driver (`ProcessDriver`). Adding a registry now is
premature ceremony — every call site goes through the same
`Process` arm, the optional fields are always `None`, the lookup
function is a tautology. Phase 2+ adds the second driver class
(`MicroVm` per whitepaper §6) and the registry pattern earns its
keep at that point. The migration is additive: replace
`Arc<dyn Driver>` with `Arc<DriverRegistry>` and route the shim's
dispatch through the registry. No external surface changes.

The DISCUSS wave's `shared-artifacts-registry.md` `driver_handle`
entry already flags this as the Phase 2+ forward-compat shape.

### Alternative B — Action shim owns the driver

Have the action shim hold its own `Arc<dyn Driver>` field, with
`AppState` holding a single `Arc<ActionShim>` instead of separate
driver / runtime fields.

**Rejected.** It conflates two distinct concerns: the shim is
*post-reconcile dispatch logic*; the driver is *workload-execution
infrastructure*. They have different lifetimes (the driver outlives
the shim's invocation; the shim's signature changes per
reconciler-Action surface; the driver's signature is fixed by
whitepaper §6). Holding the driver on `AppState` makes it a
first-class peer of `store` / `obs` / `runtime` — exactly what it
is — and allows future non-shim consumers (a `DriverHealth`
sub-reconciler in §14, an admission-time capacity check in
admission control) to access the driver without going through the
shim API.

### Alternative C — Defer to Phase 2

Land the action shim as a stub that logs Actions but does not
dispatch them; defer the `AppState::driver` extension to Phase 2.

**Rejected.** This trivially defeats the convergence-loop closure
that Phase 1 first-workload exists to deliver. Without driver
dispatch, the lifecycle reconciler's `Action::StartAllocation`
emits to `/dev/null` and the §18 architectural commitment ("the
reconciler primitive is real, the convergence loop closes")
remains performative. The shim must be real now; the driver field
must be real now.

## Consequences

### Positive

- **Action shim has a coherent driver handle.** `state.driver`
  is the single seam through which the shim reaches workload
  execution. No separate registration step; no global mutable
  state.
- **Test injection is mechanical.** Every test fixture passes the
  driver it needs (`SimDriver` for default-lane,
  `ProcessDriver` for `integration-tests`-gated suites). The
  existing `Arc<dyn ObservationStore>` injection pattern
  (ADR-0012) extends symmetrically.
- **Forward-compat for Phase 2+ multi-driver.** Replacing
  `Arc<dyn Driver>` with `Arc<DriverRegistry>` is one type
  swap at the field declaration plus an updated shim call site;
  no handler or test signature changes outside the field's
  immediate consumers.
- **Production driver lifetime is owned.** The driver lives as
  long as `AppState` lives — i.e. for the server's lifetime —
  with `Drop` running cleanly at shutdown.

### Negative

- **`run_server_with_obs` becomes
  `run_server_with_obs_and_driver` in tests.** Every existing
  caller in `crates/overdrive-control-plane/tests/` and in the
  workspace's other test fixtures gets one extra positional
  argument. The migration is mechanical (well under a day's
  worth of edits, distributed across ~12 test files); the
  diff-noise cost is the price of preserving the explicit-
  injection discipline. Renaming was considered against keeping
  the old name and adding `run_server_with_obs_and_driver` as
  a parallel function, but the parallel-function approach
  produces two functions with the same prefix that differ only
  in trailing parameters — a known flaky-test source. Single
  function, single signature wins.
- **The `Driver` trait surface is now reachable from
  `overdrive-control-plane`'s public types.** The existing
  workspace dep graph already permits this
  (`overdrive-control-plane` → `overdrive-core`); no new edge
  is added. The driver implementations
  (`ProcessDriver` in `overdrive-host`, `SimDriver` in
  `overdrive-sim`) are constructed at the entry-point seam, so
  the *control-plane* crate does not gain a runtime dependency
  on either `overdrive-host` or `overdrive-sim` — those are
  caller responsibilities of `run_server` (production) and
  test fixtures (DST / integration).

### Quality-attribute impact

- **Maintainability — modifiability**: positive. The action
  shim's driver dependency is explicit at the type level;
  swapping drivers is a one-field change.
- **Maintainability — testability**: positive.
  `SimDriver` injection at the test-fixture seam gives every
  reconciler-runtime test a driver of the right shape without
  spawning real processes.
- **Reliability — fault tolerance**: neutral. Driver failures
  surface through `Driver`'s existing `DriverError` envelope;
  the shim catches them and writes `AllocStatusRow {
  state: Failed, reason }` per US-03 AC.
- **Performance — time behaviour**: neutral. `Arc::clone` on
  the driver field is O(1) atomic; the shim never allocates
  per-action.

## Compliance

- **`Send + Sync` on shared state** (`development.md` §
  Concurrency): `Arc<dyn Driver>` requires `dyn Driver:
  Send + Sync`, which is part of the trait declaration
  (`pub trait Driver: Send + Sync + 'static`).
- **Type-driven design** (`development.md`): `dyn Driver` is the
  port; the concrete driver is the adapter. Hexagonal split
  preserved.
- **No global mutable state**: all driver access goes through
  `AppState::driver`, which is per-server-instance. No
  `static`s, no `OnceLock` of driver state.
- **Test injection via constructor parameter**
  (`development.md` § Tests that mutate process-global state):
  the driver is constructor-injected; tests need no
  `serial_test` discipline for it.

## References

- ADR-0013 — Reconciler primitive runtime; the runtime is the
  shim's host environment.
- ADR-0023 — Action shim placement and signature (companion ADR).
- ADR-0012 — `LocalObservationStore` + the same trait-object
  swap pattern this ADR follows for `Driver`.
- Whitepaper §6 — Workload drivers; `Driver` trait surface.
- `crates/overdrive-core/src/traits/driver.rs` — current trait
  definition.
- `crates/overdrive-control-plane/src/lib.rs` — current
  `AppState` and `run_server_with_obs` shape.
- `docs/feature/phase-1-first-workload/discuss/wave-decisions.md`
  — Key Decision 5 pre-decides `AppState::driver`.
- `docs/feature/phase-1-first-workload/discuss/shared-artifacts-registry.md`
  — `driver_handle` artifact entry, Phase 2+ forward-compat
  notes.

## Amendment 2026-04-27 — Worker Crate Extraction

This ADR's body is unchanged in shape — `AppState::driver:
Arc<dyn Driver>` works exactly as written, the trait-object swap
pattern is preserved, and the production / test fixture wiring is
mechanically identical. The only change is **where the production
`ProcessDriver` impl is constructed from**: `overdrive-host` (as
originally written) → **`overdrive-worker`** (per ADR-0029).

The `Driver` trait itself stays in `overdrive-core`, exactly where
ADR-0016 placed it. The control-plane crate continues to see only
`&dyn Driver` and never imports the impl crate. The binary's `serve`
subcommand instantiates the worker subsystem (when `[node] role`
includes worker) and threads `Arc<ProcessDriver>` from the worker
into `AppState::driver` per ADR-0029's binary-composition pattern.
For Phase 2+ control-plane-only nodes, the same `AppState::driver`
field will hold a future `RemoteDriver` impl that proxies the same
trait surface over RPC.

See ADR-0029 for the extraction rationale, the new dependency
graph, and the binary-composition pattern.

