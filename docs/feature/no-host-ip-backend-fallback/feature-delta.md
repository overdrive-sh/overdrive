# Feature delta — no host-IP fallback for Service backends

Decision record: [ADR-0276](../../product/architecture/adr-0276-a-service-backend-address-is-the-running-allocations-workload-address-never-the-node-address.md).
Not a roadmap step. Wave: DESIGN. Paradigm: object-oriented (CLAUDE.md).

## User rulings

Dated 2026-11-06.

1. Two Services never share a backend endpoint (`BackendKey` = ip, port, proto).
   Already enforced fail-closed by the dataplane (R-59).
2. Remove the host-IP fallback `row.workload_addr.unwrap_or(ctx.host_ipv4)`
   (`crates/overdrive-reconcilers/src/service_lifecycle.rs:923`). Skipping an
   allocation without an address is the wrong fix. An allocation that can be a
   Service backend must be unable to lack a workload address. Test boots that
   ran without one supply it through the same production path.

## Premise (verified in code, 2026-11-06)

Production producers of a Running row's `workload_addr`:

| Producer | Location | Source of the address |
|---|---|---|
| `StartAllocation` Running write | `crates/overdrive-control-plane/src/action_shim/mod.rs:2469-2473` | `spec.transport.workload_addr()` |
| `RestartAllocation` successor Running write | `action_shim/mod.rs:2977-2981` | `spec.transport.workload_addr()` |
| `FinalizeFailed` Stable (still Running) arm | `action_shim/mod.rs:3263` | forward-carry of `prior_row.workload_addr` |
| Exit observer | `crates/overdrive-control-plane/src/worker/exit_observer.rs:546` | forward-carry of `prior.workload_addr` (non-Running successor rows) |

`spec.transport` is set only in `provision_and_inject_netns`
(`action_shim/mod.rs:1473-1519`): the `guest_network = Some(..)` arm calls
`guest_pool.assign` and `guest_provisioner.provision` and sets
`spec.transport = Some(plan.assignment().clone())` (`:1483-1487`). Production
dispatch (`dispatch_with_workflow_intent` → `dispatch_with_network_owner`,
`:1311-1366`) always passes that arm (shared guest-network owner,
`state.guest_pool`). In a non-test build the `None` arm returns
`ShimError::GuestNetwork` (`:1489-1498`).

Paths that leave a Running row's `workload_addr` `None`: only the
`#[cfg(any(test, feature = "integration-tests"))]` legacy exec/process arm
(`:1499-1518`, returns `Ok(None)`), reached through `dispatch` (`:1019`),
`dispatch_with_network_provisioner` (`:1078`),
`dispatch_with_workflow_intent_and_network_provisioner_for_test` (`:1377`) and
`run_convergence_tick_with_network_provisioner_for_test`
(`crates/overdrive-control-plane/src/reconciler_runtime.rs:1512`), plus tests that
write rows directly. No production path leaves it `None`; the stop condition in
the task does not apply. (`dataplane_override` no longer exists as a config
field; only comments and a source-scan denylist name it.)

Non-Running rows legitimately carry `None` (`action_shim/mod.rs:2469-2473`
failed start; `:454-458` comment), so the persisted field stays optional.

Uses of `host_ipv4`:

| Use | Location | Disposition |
|---|---|---|
| Service backend fallback | `overdrive-reconcilers/src/service_lifecycle.rs:923` (reads `HydrationContext.host_ipv4`, `overdrive-core/src/reconcilers/hydration.rs:86`) | removed; the only reader of `HydrationContext.host_ipv4` |
| `ServiceMapHydrator.host_ipv4` | `overdrive-reconcilers/src/service_map_hydrator.rs:266,294,303,347` | unchanged (local-vs-remote backend partition) |
| `AppState.host_ipv4` | `overdrive-control-plane/src/lib.rs:373,663,707,766,804,3931,4250` | unchanged (hydrator registration, listener facts `listener_facts.rs:953`) |
| `ComposedDataplane.host_ipv4` and sim composer | `node_adapters.rs:468,594,604`, `overdrive-sim/src/adapters/dataplane_composer.rs:31-62`, `shared_network_test_ports.rs:629-665` | unchanged |

## Design (the mechanism)

Persisted format: **unchanged.** No rkyv envelope bump, no golden-bytes fixture,
no wire change. `AllocStatusRowEnvelope` has only `V1`
(`overdrive-core/src/traits/observation_store.rs:1025-1027`) and is not touched.
The larger-variant `FIXTURE_V1` caveat does not apply because no variant is
appended.

### Interface contracts (overdrive-reconciers, `service_lifecycle.rs`)

- `ServiceBackendIdentity { pub spiffe: SpiffeId, pub ip: Ipv4Addr }` —
  `Debug, Clone, PartialEq, Eq`. A Service backend's identity (SPIFFE id and
  address); the listener port stays with the dataplane identity.
- `NotRunningState` — newtype over `AllocState` with a **private** field.
  `NotRunningState::new(AllocState) -> Option<NotRunningState>` returns `None`
  iff the state is `Running`. `NotRunningState::get(self) -> AllocState`.
  `Debug, Clone, Copy, PartialEq, Eq`.
- `ServiceAllocPhase` — `Debug, Clone, PartialEq, Eq`:
  - `Running(ServiceBackendIdentity)`
  - `NotRunning(NotRunningState)`
  - `ServiceAllocPhase::from_row(row: &AllocStatusRow, spiffe: SpiffeId) -> Result<ServiceAllocPhase, RunningAllocationWithoutWorkloadAddr>` — total over
    `AllocState × Option<Ipv4Addr>`: `Running ∧ Some(ip)` → `Running(identity)`;
    `Running ∧ None` → `Err`; every other state → `NotRunning`, whatever the
    address. This is the only constructor path hydration uses.
  - `ServiceAllocPhase::state(&self) -> AllocState`.
- `RunningAllocationWithoutWorkloadAddr { pub alloc_id: AllocationId }` —
  `Debug, thiserror::Error`, `Display` names the allocation and states that a
  Running Service allocation must carry its workload address.
- `ServiceAllocFact` (`:70`): **remove** `state: AllocState`, `backend_spiffe`,
  `backend_ip`; **add** `pub phase: ServiceAllocPhase`; add
  `ServiceAllocFact::state(&self) -> AllocState` (delegates to the phase) and
  `ServiceAllocFact::backend(&self) -> Option<&ServiceBackendIdentity>` (`Some`
  iff Running). All other fields unchanged. Every `fact.state == X` read in
  `service_lifecycle.rs` (`:528, :550, :581, :712, :1046, :1090, :1104, :1227`)
  becomes `fact.state()`.
- `service_backend_row_actions` (`:1075`) builds each `Backend` from
  `ServiceAllocPhase::Running(identity)` only: `alloc: identity.spiffe.clone()`,
  `addr: SocketAddr::new(IpAddr::V4(identity.ip), identity.port)`; the
  `state == Running` filter and healthy computation key on the same variant.
- Hydration (`hydrate_service_alloc_facts`, `:888-957`): builds the phase with
  `from_row`; an `Err` returns the new `HydrateError` variant below. The
  `ctx.host_ipv4` read and the `backend_ip` local are deleted.

### Interface contracts (overdrive-core, `reconcilers/hydration.rs`)

- `HydrateError::RunningAllocationWithoutWorkloadAddr { alloc_id: AllocationId }` —
  new variant beside `ObservationRead` (`:109-`); `#[error(...)]` names the
  allocation. It converts through the existing `ConvergenceError: From<HydrateError>`
  variant; no call-site flattening to a string.
  Semantics: the observation row violates the Running ⇒ address contract; the
  tick for that Service target fails with this typed error and no
  `ServiceBackendRow` is written until the row is corrected.
- `HydrationContext` (`:58-90`): **remove** `pub host_ipv4: Ipv4Addr` and the
  `host_ipv4` mention in the struct doc (`:50`).
- `AllocStatusRow.workload_addr` doc (`observation_store.rs:1099-1101`): state the
  contract — `Some` is required on every Running row a Service backend can be
  derived from; `None` means no address was observed (Pending / failed / terminal).

### Lifecycle Gate Ownership

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
|---|---|---|---|---|
| Service backend membership | `ServiceLifecycle` (ADR-0101) | A published backend is a Running allocation with its own workload address | Running state, workload address, readiness policy | Non-Running lifecycle facts (startup, failure, termination gates) |

#### Gate G-1 — Running ⇒ address at Service hydration

- **Existing evidence:** ADR-0101 D1; producers above (`action_shim/mod.rs:2469`,
  `:2977`, `:3263`) → observation row → `hydrate_service_alloc_facts`.
- **Owner:** `ServiceLifecycle` hydration.
- **Promise:** a `Running` fact carries a workload address.
- **Affected state:** the hydration result for one Service target.
- **Failure projection:** `HydrateError::RunningAllocationWithoutWorkloadAddr`;
  read failure, timeout, malformed and duplicate rows keep their existing
  variants.
- **Explicitly unaffected:** Pending, Draining, Suspended, Terminated and Failed
  facts (they hydrate with or without an address); probe supervision; restart
  authority; the dataplane `BackendKey` conflict check.
- **Ordering:** evaluated per row during hydration, before `reconcile`; consumes
  no timeout budget.
- **Counterexample (why not at row write or persisted-row level):** a failed
  start or terminal row has no address; requiring it earlier would invent one.
- **Evidence lane:** pure/unit and the runtime hydrate test lane; no seeded
  simulation or kernel evidence is needed (no ordering, timing or kernel claim).

### Formal protocol model

Not applicable: a total pure projection of one row; no concurrent, ordered or
crash-sensitive protocol changes.

### Rejected mechanisms

Required persisted field via envelope bump; skip-the-allocation; `Option`
backend inside the fact; log-only fallback. Reasons in the ADR.

## Every affected site

### Production code

- `overdrive-reconcilers/src/service_lifecycle.rs`: `:70-174` (fact), `:528, :550,
  :581, :712, :923, :929-953, :1046, :1090, :1104-1107, :1227` and the in-file
  fact literals `:1326-1337, :1383-1403`.
- `overdrive-core/src/reconcilers/hydration.rs:50, :86, :109-`.
- `overdrive-control-plane/src/reconciler_runtime.rs:1820-1832`
  (`build_hydration_context`, the single production construction site).
- Stale comments made false: `action_shim/mod.rs:369` and `:393` ("residual
  host_ipv4-fallback masking is tracked in #248"), `:455-458`, `:2006-2009`;
  `reconciler_runtime.rs:2593-2596` docstring. Replace with a pointer to ADR-0276
  or delete the sentence; name no issue number that was not verified.

### `HydrationContext { .. }` literals losing `host_ipv4`

- `overdrive-control-plane/src/gateway_identity_lifecycle/quint_gateway_identity.rs:275, :287`
- `overdrive-control-plane/tests/acceptance/listener_fact_hydrate_equivalence.rs:176, :188`
- `overdrive-control-plane/tests/acceptance/public_ingress_gateway_hydration.rs:131, :143`
- `overdrive-sim/tests/acceptance/vm_reclamation_hydrate_actual_equivalence.rs:113, :128`
- `overdrive-control-plane/tests/acceptance/hydration_context_read_surface_audit.rs:86` (read), `:95/:107` (destructure), doc `:27`, surface list `:140`, count assertion `:153` (11 → 10).

### `ServiceAllocFact { .. }` literals (change to `phase`)

- `overdrive-reconcilers/tests/acceptance/service_kind_vm_workloads.rs:56, :76`
- `overdrive-core/tests/capture_advertise_port_set_equality.rs:104, :121`
- `overdrive-core/tests/acceptance/service_lifecycle_startup_stop.rs:64, :84`
- `overdrive-core/tests/acceptance/vm_reclamation_plan_purity.rs:664, :683` (Terminated)
- `overdrive-core/tests/acceptance/service_lifecycle_reconcile_branches.rs:60/:90, :339, :892, :1099/:1123, :1341/:1365, :1733, :1810, :1873`
- `overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs:161/:180, :299`
- `overdrive-control-plane/tests/acceptance/single_restart_authority_liveness_trajectory.rs:84/:101`; `service_lifecycle_liveness.rs:52/:72`; `service_submit_event_taxonomy.rs:378/:400, :409/:431`; `service_lifecycle_probe_to_stable.rs:146/:172`; `service_lifecycle_hydrate.rs:563/:583 (Failed), :656/:678`; `service_lifecycle_stable.rs:57/:81, :94/:116 (Failed)`; `service_lifecycle_readiness.rs:88/:108, :119/:138`

Fixtures that wrote Running with `NotRunning`-only data must choose the phase
honestly: Running facts get a `ServiceBackendIdentity`; Failed/Terminated facts
get `NotRunning(NotRunningState::new(..))`.

### Fixtures that seed a Running Service row with `workload_addr: None` and are hydrated by ServiceLifecycle (must supply `Some(addr)`)

- `reconciler_runtime.rs` test module: helper `write_alloc_status` `:2237` →
  `write_alloc_status_with_addr(.., None)` `:2243/:2249/:2256-2279`; uses `:2550`;
  `write_running_alloc` `:2856-2878` (`:2872`) used at `:2987, :3074`.
- `service_lifecycle_advertises_workload_addr_not_host_ipv4` (`:2597-2646`): the
  `payments-host` half (`:2612-2614, :2637-2645`) asserts the fallback and is
  rewritten (see obligation O2); the mesh half (`:2602-2635`) stays as the
  positive case, renamed to describe behavior (domain-naming rule).
- `overdrive-control-plane/tests/acceptance/service_lifecycle_hydrate.rs`:
  `make_alloc_status_row` `:209` ("Host-netns fixture"), `:221`, `:234`; Running
  writes `:358, :400, :490, :829`. Failed rows `:529, :784` may keep `None`.
- `hydration_characterization_golden.rs:235-251`, called `:417-418`; golden
  `tests/acceptance/fixtures/hydration_golden/service_lifecycle.txt:53`
  (`backend_ip: 127.0.0.1`) is regenerated from the new fact shape.
- `service_lifecycle_startup_stop_gate.rs:213`; `service_lifecycle_runtime_reenqueue.rs:142/:157`;
  `service_stream_recorded_verdict.rs:112/:133` (rows whose state parameter is
  Running; the inventory did not check each caller); `single_restart_authority_liveness_trajectory.rs:169-178`
  (Terminated, may keep `None`).
- Already safe (`Some(addr)` today): `probe_count_fixture.rs:439`,
  `liveness_gate_fixture.rs:469/:596`, `quint_restart_authority.rs` (rows from
  the shim, `:269/:457`), `service_stream_support.rs:328`.

### Legacy-provisioning compositions that tick a Service through ServiceLifecycle

These get `spec.transport = None` today and would hydrate a Running row with no
address. Move them to the guest-network composition that production uses —
`run_convergence_tick_with_guest_network_provisioner_for_test` /
`dispatch_with_guest_network_provisioner_for_test`
(`action_shim/mod.rs:1422`, with the existing sim guest-network provisioner in
`shared_network_test_ports.rs`):

- `overdrive-sim/tests/allocation_restart_write_spike.rs:246, :276, :291`
- `overdrive-control-plane/tests/acceptance/restart_authority_world.rs:204, :309, :578`

Legacy-form callers that do not hydrate ServiceLifecycle are not affected by this
change and are left alone: `service_workload_emits_start_allocation.rs:183`,
`crash_observability_two_cycles.rs:384`, `driver_neutral_allocation_replacement.rs`,
`lifecycle_broadcast.rs`, `terminal_hook_ownership.rs:251`,
`terminal_netns_cleanup_from_observed_addr.rs:118`,
`action_shim_running_write_failure_stops_alloc.rs:167`,
`action_shim_restart_uses_spec_from_action.rs:221`, `mtls_install_fail_closed.rs:971`,
`guest_activation_acceptance.rs:643`, `finalize_failed_forward_carries_workload_addr.rs:215`,
`stable_does_not_stop_probe_supervision.rs:282`, `alloc_netns_lifecycle.rs:200`,
`lww_counter_survives_restart.rs:142`, `action_shim_crash_observability.rs`.
This change does not touch the legacy branch itself.

## How test boots obtain a workload address

- A test that drives the shim (tick or dispatch) composes the guest-network owner
  path, so `guest_pool.assign` produces the address and the Running write copies
  it. No test sets an address the shim did not derive.
- A hydration test that seeds an observation row writes it through the
  `ObservationStore` port (the driven port the real row arrives through) with a
  `Some(addr)` inside the guest prefix, taken from one shared fixture constant or
  from a `guest_pool.assign` of the test's own `AppState` when it has one. A seeded
  Running row never carries `None`.
- The node address is no longer observable in Service hydration: the sim composer's
  `host_ipv4` (127.0.0.1) cannot leak into a backend.

## Test obligations for the acceptance designer

Each carries the contract/invariant, production owner path, fault stimulus,
oracle and evidence tier.

- **O1 Running with address is the backend.** Seed Running + `Some(addr)`; hydrate
  and reconcile through the runtime path; the published `ServiceBackendRow` has
  `addr == addr:listener_port`. Default lane.
- **O2 Running without address is refused, not defaulted or skipped.** Replaces
  the `payments-host` half of the test at `reconciler_runtime.rs:2597`. Seed Running +
  `None` beside a valid Running + `Some` row of the same Service; assert
  `hydrate_actual` returns `HydrateError::RunningAllocationWithoutWorkloadAddr`
  naming the right allocation, no `ServiceBackendRow` is written for that
  Service, and no row carries any host address. Default lane.
- **O3 Non-Running rows without an address still hydrate.** For each of Pending,
  Draining, Suspended, Terminated, Failed with `None`: `Ok`, phase `NotRunning`,
  no backend published, startup/failure/terminated gates unchanged. Default lane.
- **O4 Totality property.** Proptest over `AllocState × Option<Ipv4Addr>` for
  `ServiceAllocPhase::from_row`, plus `NotRunningState::new(Running) == None` and
  `new(s).map(get) == Some(s)` for every other state. Default lane; Contract
  Shape: pure-function. The generators live beside the types.
- **O5 Context surface.** `hydration_context_read_surface_audit.rs` updated
  (11 → 10 surfaces, `host_ipv4` absent). A source-scan or compile check that no
  `unwrap_or(..host_ipv4..)` backend expression remains is optional and not required.
- **O6 Composed shim → hydrate → row.** One seeded `overdrive-sim` composition on
  the guest-network path: shim Start writes Running with the pool-assigned
  address, ServiceLifecycle publishes exactly that address, across a
  `RestartAllocation` (successor row `action_shim/mod.rs:2977`). Replaces the
  legacy-form ticks in the two moved fixtures. Tier 1.
- **O7 Golden.** Regenerate `hydration_golden/service_lifecycle.txt` (print the
  new fact shape; the Running Service row carries an address).
- **O8 Mutation.** `from_row`'s three-way branch and the removed `unwrap_or` are
  mutation targets; the ≥80% gate applies to the diff.
- No envelope golden bytes, no `schema_evolution` change, no Quint work.
- Tier 3: no new real-kernel claim; the production composition is unchanged.

## Hot path and validation cost

No hot-path change (one enum match per row per Service tick). Validation adds
O1-O6 in the default lane plus one Tier-1 composition; no new lane, tool or xtask.

## Decisions for the user

None blocking. For awareness: the change alters no persisted or wire format; it
does change test compositions (two fixtures move to the guest-network
composition) and the `ServiceAllocFact` and `HydrationContext` public shapes.
