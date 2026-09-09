# E11 readiness wake — proposed ADR-0101 revision 5 amendment

**Status:** Proposed; independent DESIGN review is required before DELIVER
step 03-01 resumes. This document is not an implementation approval and does
not claim that E11 is green.

**Author:** Codex acting as solution architect.

**Authority:** User-authorized focused DESIGN amendment for the proven E11
readiness-wake contradiction. ADR-0101 revisions 3 and 4 retain their
independent approval provenance. This amendment changes only the wake path
needed by the existing `ServiceLifecycle` backend-health owner.

**Implementation boundary:** No production code, tests, examples, expectation
evidence, DES event, roadmap approval, or commit is authored by this DESIGN
amendment.

The canonical roadmap remains unchanged: its 03-01 acceptance criteria already
state the observable E11 withdrawal/recovery outcome, and its
`implementation_scope` is guidance rather than a restrictive production-file
allowlist. The feature-delta and DESIGN wave-decision entries below record
this proposed prerequisite without changing roadmap validation metadata or
historical execution outcomes.

## Scope and revalidated evidence

DELIVER step 03-01 is blocked by a reachable production path, not by a
hypothetical scheduler interleaving. The required native E11 run observed a
readiness `Fail` while the VM Service remained `Running` with zero restarts,
then the during-window VM-client Job received the guest reply that the failed
readiness window is required to withhold. The same run records the exact
failure and its native cleanup in
`docs/feature/service-kind-vm-workloads/deliver/execution-log.json` (03-01
GREEN failure at `2026-09-09T09:12:09Z`). The bounded control-plane
reproduction also fails independently at seeded Sim seed `25717`: the
`ProbeResultRow` is written, but no subscription event reaches the
`ServiceLifecycle` evaluation broker.

The E11 fixture deliberately uses a one-second readiness interval and a
one-second timeout (`examples/service-kind-vm-workloads/readiness-recovery.toml:27-32`).
The public expectation measures the observed readiness timestamp to detection
timestamp and requires at most 2,000 ms
(`examples/service-kind-vm-workloads/run-example.sh:345-377`). The missing
edge wake leaves the only existing row-backed fallback at the interest
router's 30-second relist, so the two-second result is not achievable under
the current accepted ADR surface.

### Current production facts

| Production entry point / owner path | Verified source evidence | Fact established |
|---|---|---|
| Successful allocation publication reaches the driver hook | `crates/overdrive-control-plane/src/action_shim/mod.rs:2103-2231` | The action shim awaits the `Running` allocation-row write and then calls `driver.on_alloc_running(&spec)`. |
| VM probe supervision begins at that hook and ends only at the existing terminal hook | `crates/overdrive-worker/src/vm_driver.rs:1894-1903`; `crates/overdrive-worker/src/driver.rs:819-840` | `VmDriver` delegates `on_alloc_running` to `ProbeRunner::start_alloc` and terminal cleanup to `stop_alloc`; readiness remains continuous after `Stable`. |
| A real probe attempt produces the current observation | `crates/overdrive-worker/src/probe_runner/mod.rs:526-614` | `probe_tick` invokes the adapter, constructs the unchanged `ProbeResultRow`, and awaits `ObservationStore::write_probe_result`. The supervised loop logs an error and continues on a per-tick failure. |
| The production store commits the probe row but does not publish a watch item | `crates/overdrive-store-local/src/observation_backend.rs:968-982` | The redb transaction calls `apply_probe_result_lww`, commits, discards its accepted boolean, and returns; it never calls the subscription emitter. |
| The Sim store reproduces the same missing event | `crates/overdrive-sim/src/adapters/observation_store.rs:866-879` | The LWW map is updated and the accepted result is discarded; no fan-out item is sent. |
| `ServiceLifecycle` can read the result but declares no result wake | `crates/overdrive-reconcilers/src/service_lifecycle.rs:858-922`; `:464-474` | Hydration reads `list_probe_results_for_alloc`, while `interests()` contains only `ObservationRowKind::AllocStatus`. |
| The existing router only routes allocation rows and lists only allocation rows | `crates/overdrive-control-plane/src/lib.rs:3463-3532`; `:3555-3652` | `route_observation_row` has an `AllocStatus` target arm only; boot, lag and periodic relists call `alloc_status_rows`; periodic relist is 30 seconds (`:3414`). |
| A broker wake would be serviced promptly | `crates/overdrive-control-plane/src/reconciler_runtime.rs:1203-1205`; `:1592-1608` | The production convergence cadence defaults to 100 ms, and an evaluation that emits backend work self-re-enqueues through the existing broker. |
| Readiness policy already changes only backend eligibility | `crates/overdrive-reconcilers/src/service_lifecycle.rs:1098-1160`; `:1182-1220` | The existing owner computes health from readiness and terminal veto, constructs `WriteServiceBackendRow`, and does not emit a restart action for readiness. |
| Existing consumers already watch the materialized backend row | `crates/overdrive-control-plane/src/action_shim/write_service_backend_row.rs:46-58`; `crates/overdrive-control-plane/src/mtls_resolve_adapter.rs:771-785`; `crates/overdrive-control-plane/src/dns_responder/name_index.rs:401-407` | A successful existing row write is the asynchronous handoff to the mesh/DNS consumers; no lifecycle acknowledgement is present. |

The path is therefore:

```text
VM Driver on_alloc_running
  -> ProbeRunner probe_tick
  -> ObservationStore::write_probe_result (durable LWW row only)
  -X no SubscriptionEvent / no broker Evaluation
  -> ServiceLifecycle cannot re-hydrate until AllocStatus or 30s relist
  -> stale ServiceBackendRow
  -> existing mesh/DNS consumer still selects the backend
```

This is the complete caller/owner path in the default `serve` composition.
It does not require a test-only abort, fabricated row, second allocation, or
unreachable state. The readiness row is already written by the production
probe runner; only the wake edge is absent.

## Decision

Amend ADR-0101 with one event-only projection through the existing
`ObservationStore::subscribe_all_events` and interest-router vocabulary:

1. Append `ObservationRow::ProbeResult(ProbeResultRow)` to the existing
   in-process subscription row enum. It is an event projection, not a second
   persistence row: it is emitted only by an accepted
   `write_probe_result`, is never accepted by `ObservationStore::write`, and
   is never added to the generic observation gossip payload.
2. Append `ObservationRowKind::ProbeResult` to the existing interest
   discriminant and map the new row projection to it. Append the variant so
   existing kind ordering is retained.
3. Change only `ServiceLifecycleReconciler::interests()` to declare both
   existing allocation status and probe-result interests. No other reconciler
   declares the new interest.
4. Make each production observation adapter emit the event projection only
   after its probe-result LWW mutation has committed and only when that
   mutation accepted the existing LWW winner. A decodable prior row requires
   a strictly newer `last_observed_at_unix_ms`; the local adapter's existing
   unreadable-predecessor self-healing rule remains accepted as well.
5. Extend the existing interest router's target derivation and its existing
   List-then-Watch behavior to use the already-persisted allocation row. No
   new store method, broker channel, resync schedule, cadence, persistence
   table, or consumer protocol is introduced.

The event is intentionally the complete `ProbeResult` row family rather than
a readiness-only discriminant. `ServiceLifecycle` hydrates startup,
readiness and liveness from the same per-allocation probe snapshot, while the
existing interest vocabulary has no role-level dimension. Filtering the
event at readiness would leave the same owner unable to observe its startup
and liveness rows and would require another public kind or private wake path;
the existing reconciler policy already selects the role after hydration, and
the broker collapses the resulting evaluations.

This is the smallest change that uses the already-owned observation write as
the wake source while retaining the existing router, broker and reconciler
boundaries. The only new public vocabulary is the appended pair of existing
enum variants; no new public method, type, field, trait, action, route,
configuration value, or lifecycle state is introduced.

## Exact contract surface

The following shapes are closed. A crafter must not rename, re-box, add
metadata to, or otherwise extend them.

### Core observation vocabulary

In `crates/overdrive-core/src/traits/observation_store.rs`:

```rust
pub enum ObservationRow {
    // Existing variants, unchanged ...
    Signal {
        key: crate::workflow::SignalKey,
        value: crate::workflow::SignalValue,
    },
    ProbeResult(ProbeResultRow),
}

pub enum ObservationRowKind {
    // Existing variants, unchanged ...
    Signal,
    ProbeResult,
}
```

`ObservationRow::kind(&self) -> ObservationRowKind` gains exactly the total
arm `Self::ProbeResult(_) => ObservationRowKind::ProbeResult`. The existing
`ObservationRowKind::as_str(self) -> &'static str` returns exactly
`"probe-result"` for the new variant. The new variant is appended after the
existing `Signal` variants; no existing discriminant/order is renumbered.

`ObservationRow::ProbeResult` is deliberately **event-only**. It is not added
to `ObservationWrite`, is not accepted by the generic `ObservationStore::write`
path, is not stored in the `ObservationRow` history, and is not a new rkyv
envelope or table. The durable payload remains the unchanged
`ProbeResultRowV1`/`ProbeResultRow` at the existing
`(alloc_id, role, probe_idx)` key.

### Existing store methods with amended postconditions

The signatures remain exactly:

```rust
async fn write_probe_result(
    &self,
    row: ProbeResultRow,
) -> Result<(), ObservationStoreError>;

async fn subscribe_all_events(
    &self,
) -> Result<LagAwareSubscription, ObservationStoreError>;

async fn alloc_status_row(
    &self,
    alloc_id: &AllocationId,
) -> Result<Option<AllocStatusRow>, ObservationStoreError>;

async fn list_probe_results_for_alloc(
    &self,
    alloc_id: &AllocationId,
) -> Result<Vec<ProbeResultRow>, ObservationStoreError>;
```

`write_probe_result` keeps its existing LWW contract. After the probe row's
mutation commits:

- An accepted LWW-winning write emits exactly one
  `SubscriptionEvent::Row(ObservationRow::ProbeResult(row))` to the existing
  `subscribe_all_events` stream, synchronously after commit and before the
  method returns `Ok(())`. For a decodable prior row, "winning" means a
  strictly greater `last_observed_at_unix_ms`; the local adapter retains its
  existing rule that an unreadable prior envelope is repaired by the incoming
  typed row.
- A stale or equal write returns the existing `Ok(())` no-op and emits no
  event, because no current observation changed.
- A failed transaction/commit returns the existing typed
  `ObservationStoreError` and emits no event.
- No new `accepted` return value is added to the method. The adapter's
  accepted boolean remains private implementation state.

The existing `SubscriptionEvent` shape is unchanged:
`Row(ObservationRow)` and `Lagged { missed: u64 }` remain its only variants.
The event is delivered through that one existing lag-aware stream, not a
second subscription or a broker-side side channel.

The existing stream-completeness rule therefore gains one producer-specific
postcondition: an accepted `write_probe_result` is delivered as the new
`Row` projection (or its loss is surfaced as the existing `Lagged` signal).
The generic `ObservationStore::write` acceptance/emission contract is
unchanged; no subscriber is required to understand a second event shape.

### ServiceLifecycle declaration

In `crates/overdrive-reconcilers/src/service_lifecycle.rs`, the existing
trait method keeps its exact signature and returns exactly this static slice:

```rust
fn interests(&self) -> &'static [ObservationRowKind] {
    &[
        ObservationRowKind::AllocStatus,
        ObservationRowKind::ProbeResult,
    ]
}
```

`ServiceLifecycleReconciler::new()`, `Default`, all associated types, the
`Reconciler` signatures, `ServiceLifecycleState`, `ServiceLifecycleView`,
readiness policy, terminal veto, and action variants remain unchanged.

This two-kind slice is the narrowly scoped revision-5 replacement for
ADR-0101 revision 4 D2's `AllocStatus`-only interest and its "no new
interest" clause. Every other D2 prohibition remains active; the pair is not
permission to add a second interest API, cadence, broker capability, resync
schedule, state authority or consumer protocol.

## Router target and relist contract

The existing private router helpers are extended only as follows. These
private signatures pin the implementation boundary so the crafter cannot
invent a callback, cache, wildcard target or new broker API:

```rust
async fn route_observation_row(
    obs: &Arc<dyn ObservationStore>,
    row: &ObservationRow,
    interest_table: &BTreeMap<ObservationRowKind, Vec<ReconcilerName>>,
    broker: &InterestRouterBroker,
);

async fn list_and_route(
    obs: &Arc<dyn ObservationStore>,
    interest_table: &BTreeMap<ObservationRowKind, Vec<ReconcilerName>>,
    broker: &InterestRouterBroker,
);
```

`route_observation_row` retains the current `AllocStatus` behavior:
`AllocStatus(row)` derives `workload/<row.workload_id>` and submits one
existing `Evaluation` per interested reconciler. For
`ProbeResult(probe_row)`, it calls the existing
`alloc_status_row(&probe_row.alloc_id)` point read. It submits the same
`workload/<current_alloc_status.workload_id>` target only when:

- the point read succeeds;
- the allocation row exists; and
- `current_alloc_status.kind == WorkloadKind::Service`.

It then submits one existing `Evaluation` for each reconciler listed under
`ObservationRowKind::ProbeResult`. It does not derive an `alloc/<id>` target,
search intent, add a workload id to `ProbeResultRow`, maintain a warm cache,
or wake a wildcard target. A malformed derived `TargetResource` is dropped
using the current non-panicking router posture.

If the point read returns `Ok(None)`, the orphan probe event is ignored. If it
returns an error, the router logs a structured warning and ignores that edge;
the next newer probe write and the existing periodic/relist path retry the
level-triggered read. This is not a new retry queue or a claim that an
observation-store outage satisfies E11's two-second bound.

`list_and_route` retains the existing `alloc_status_rows()` List leg. When the
interest table contains `ObservationRowKind::ProbeResult`, it does **not** add
a second List family: `ServiceLifecycle` still declares
`ObservationRowKind::AllocStatus`, so the existing allocation snapshot List,
`Lagged` relist and 30-second periodic relist already submit the same Service
target. Hydration on that evaluation reads the latest probe rows through the
existing `list_probe_results_for_alloc(&AllocationId)` method. This preserves
one target-enumeration path, avoids a redundant per-allocation probe scan and
keeps the broker's existing `(reconciler, target)` collapse semantics.

The existing subscribe-first-before-list boot ordering remains mandatory.
The existing `Lagged` branch and 30-second periodic relist call this expanded
`list_and_route`, so a dropped probe event is recovered by re-waking
`ServiceLifecycle`, whose hydration reads the durable probe-row snapshot. No
new resync schedule or shorter global relist cadence is permitted. The direct
event path, rather than the 30-second backstop, is what makes the healthy-store
E11 bound possible.

## Ordering, retry and cancellation

| Boundary | Required ordering and failure behavior |
|---|---|
| Probe attempt → observation | `ProbeRunner::probe_tick` awaits the adapter, builds the unchanged row, and awaits `write_probe_result`. Adapter or store errors retain the current typed error/log-and-continue behavior; no event is emitted for a failed write. The next configured probe interval is the existing retry. |
| Durable probe row → wake | Each adapter commits the accepted LWW row first, then synchronously sends the event-only `ObservationRow::ProbeResult`. There is no event before commit, no event for an LWW loser, and no second task. A zero-subscriber broadcast send remains the existing benign condition; boot List catches already-persisted rows. |
| Probe event → evaluation | The router performs the existing allocation point read, verifies Service kind, and submits through the existing `InterestRouterBroker::submit`. There is no direct `ServiceLifecycle` call, callback, detached task, or broker bypass. Broker key collapse deduplicates concurrent role/index/list events. |
| Event loss | `SubscriptionEvent::Lagged` invokes the existing allocation List leg, which already wakes `ServiceLifecycle`; its hydration reads latest probe rows through `list_probe_results_for_alloc`. Read failure is logged and retried by the existing periodic relist or a later probe event. No event history, acknowledgement row, or new recovery subsystem is added. |
| Evaluation → backend projection | The existing 100 ms convergence loop drains the shared broker. `ServiceLifecycle` re-hydrates the latest probe row, applies its existing threshold and terminal veto, and emits only the existing backend-row/hydrator actions for readiness. Runtime View persistence and the existing self-reenqueue remain in their current order. |
| Backend row → consumers | The existing action shim awaits `ObservationStore::write(ObservationWrite::ServiceBackend(row))`; mesh, DNS and other consumers retain their current asynchronous List/Watch/relist paths. The lifecycle owner does not wait for consumer acknowledgement, and no established connection is actively revoked by this amendment. |
| Backend/action failure | Existing serial action-shim error isolation and runtime self-reenqueue remain unchanged. A backend-row write error is not converted into a new terminal state or wake error. This amendment adds no terminal barrier. |
| Probe cancellation | `ProbeRunner` continues to observe its per-allocation `CancellationToken` between ticks. An in-flight adapter/store future is not detached or force-aborted; if its accepted row commits, its event is valid and the next Service hydration's terminal/current-state checks dominate it. Terminal cleanup still calls the existing `stop_alloc`. |
| Router/convergence shutdown | The router's existing biased `shutdown.cancelled()` arm stops new routing cooperatively. The convergence loop still completes its in-flight tick before its shutdown selection. No `JoinHandle::abort`, drain, release, or cleanup operation is introduced. |

For the healthy E11 path, the existing one-second probe interval/timeout is
followed by a synchronous post-commit event, a broker submission without a
scheduled wait, and at most the existing 100 ms convergence drain. This
removes the current unbounded-to-E11 30-second quiet period. The two-second
end-to-end traffic assertion remains an empirical native expectation: it must
measure the existing backend-row consumers and the peer Job, not be inferred
from the event timestamp alone. Store outage, closed watch, and failed
downstream consumer effects remain outside the E11 healthy-store bound and
retain their current typed/fail-closed behavior.

## Lifecycle Gate Ownership

No lifecycle gate moves. The amendment adds a trigger for an existing owner;
it does not make a probe row, event, broker evaluation, or consumer effect an
additional state authority.

| Signal / state | Owner and promise | Affected state or failure projection | Unaffected states / owners | Ordering and counterexample | Evidence lane |
|---|---|---|---|---|---|
| Allocation `Running` | Existing action shim and selected driver; accepted `Running` row still means driver/Beacon success | None. A readiness result is still only an observation row and may wake reconciliation. | VM process, Beacon, Stable, readiness predicate, liveness and restart authority | A readiness `Fail` after `Running` must not emit `StopAllocation` or `RestartAllocation`. | Native E11 ledger: same Running allocation, restarts `0`; existing action-shim/driver tests. |
| Service `Stable` | Existing `ServiceLifecycle` startup branch | None. Probe-result event does not alter startup terminal or Stable transition. | Running, readiness, liveness and terminal ownership | A late readiness event cannot make an allocation Stable or un-Stable. | Existing stable/startup acceptance and terminal seed `25717`. |
| `Backend.healthy` | `ServiceLifecycle` remains sole policy and complete-row author | Existing readiness pass/fail and retained terminal veto change only the `healthy` bit in `ServiceBackendRow`. | Running, Stable, startup terminal, liveness stop, WorkloadLifecycle restart/finalization | ProbeResult event reaches hydration before the old 30-second relist; terminal veto still dominates a late Pass. | Seeded Sim safety/liveness invariant and native E11 before/during/after traffic. |
| Probe-result observation | `ProbeRunner` remains the sanctioned writer; `ObservationStore` remains durable LWW owner | Accepted row adds one local subscription projection after commit; no persisted schema or generic write family changes. | AllocStatus, terminal occurrences, service intent, ServiceLifecycle View and consumer stores | Equal/stale rows produce no event; a commit failure produces no event. | Local/Sim adapter event contract and LWW tests. |
| Broker evaluation | Existing interest router and shared `EvaluationBroker` | One ServiceLifecycle evaluation is submitted for a Service allocation's accepted probe-row event. | Broker key collapse, tick cadence, action dispatch and all other reconciler interests | An absent/non-Service allocation has no derived target; no wildcard wake exists. | Router source-local routing/relist tests and seeded Sim route. |
| Mesh/DNS/backend consumers | Existing consumers own asynchronous application of `ServiceBackendRow` | They receive the same existing false/true complete rows sooner because ServiceLifecycle is woken. | No acknowledgement, connection revocation, or consumer-owned health predicate | A stale consumer remains governed by its existing List/Watch/relist contract; E11 must prove the real healthy-store path. | Native E11 peer Job plus existing consumer integration tests. |
| Liveness/restart | `ServiceLifecycle` detects liveness; `WorkloadLifecycle` alone restarts/finalizes | None. Readiness event never creates a liveness terminal or restart action. | Restart budget, allocation id, VM exit watcher, cleanup | A readiness flap is the counterexample: same VM remains Running/Stable with zero restart delta. | E11 and existing S-SVM-21/E12 controls. |

The changed gate contract is therefore a wake edge only:

- **Owner:** `ServiceLifecycle` still owns the readiness predicate, terminal
  veto and complete `ServiceBackendRow` projection.
- **Promise:** an accepted local probe-result write for a current Service
  allocation causes an existing broker evaluation through the existing
  subscription and interest table.
- **Failure projection:** an observation-store write failure remains a typed
  probe-runner error; a dropped/lagged event uses the existing relist; no
  failure becomes an allocation terminal or restart.
- **Ordering:** durable probe row, then event, then broker evaluation, then
  existing `ServiceBackendRow` action, then existing consumer effects.
- **Counterexample retained:** a terminal allocation with a later Pass still
  produces no eligible backend, and a readiness flap never changes Running,
  Stable, or restart count.

## Alternatives and disposition

| Approach | Disposition |
|---|---|
| Add `ObservationRowKind::ProbeResult` plus event-only `ObservationRow::ProbeResult` to the existing stream (selected) | Reuses the current store-owned write, lag signal, interest declaration, target broker and List-then-Watch machinery. It adds only the two unavoidable vocabulary variants and leaves the durable probe payload and all consumer APIs unchanged. |
| Add a second `SubscriptionEvent::ProbeResult` variant or a dedicated `subscribe_probe_results()` method | Rejected: creates a second event shape or public observation API, forces every stream adapter/consumer to branch on a new channel type, and breaks the existing total `ObservationRow::kind()`/interest vocabulary. The event-only existing-row projection carries the same data through the one established stream. |
| Add a `workload_id` field or generation metadata to `ProbeResultRow` | Rejected: changes the public persisted V1 payload and rkyv schema solely to solve router target lookup. The existing `alloc_status_row(&AllocationId)` point read already owns the alloc→workload mapping. |
| Give `ProbeRunner` an `EvaluationBroker`, callback, or `ServiceLifecycle` handle | Rejected: couples the worker to the control-plane broker, bypasses observation commit/LWW ordering, adds a constructor/public capability surface, and creates a second wake owner. The store is already the producer boundary. |
| Emit a synthetic `AllocStatus` event or rewrite AllocStatus on probe change | Rejected: mislabels a probe result as allocation lifecycle truth and would wake `WorkloadLifecycle`/other allocation consumers with a false lifecycle transition. Precise row-kind routing is required. |
| Return `Some` from `ServiceLifecycle::resync_schedule()` at one second | Rejected: current `ResyncScope` is a host/local-node cadence mechanism, not a per-workload probe event target; it adds periodic work and directly contradicts ADR-0101 D2's explicit no-cadence decision. It also leaves a larger timing window than a post-commit edge. |
| Reduce the global 30-second interest-router relist | Rejected: broadens load and latency for every row-backed reconciler, is still polling rather than event-driven, and cannot recover the current non-emitted probe row without expanding the List family. |
| Add a new durable wake/recovery journal, acknowledgement, or consumer barrier | Rejected: no such dependency is present in the reproduced path. It would change persistence, ownership, consistency and consumer architecture beyond E11. |

## Compatibility, migration and scope boundaries

This project is greenfield with no deployed compatibility cohort. The new
variants are in-process Rust vocabulary only; `ObservationRow` and
`ObservationRowKind` do not carry serde, wire, or rkyv persistence in the
current API. Appending them requires compiler-required updates to exhaustive
matches and kind-coverage tests, but it does not alter any existing redb
table, `ProbeResultRow` envelope, public HTTP/CLI shape, or observation wire
protocol. Existing mTLS/DNS consumers already ignore non-`ServiceBackend`
rows and retain their boundaries. All production adapters and the control
plane must cut over together; no mixed-version protocol, dual publisher,
rollout flag, or legacy migration is designed.

The following are explicitly out of scope:

- changing `ProbeResultRow` fields, role/index identity, timestamp semantics,
  persistence retention, or cross-peer probe-result gossip;
- adding a generic `ObservationStore::write` route for probe rows or a new
  observation table/envelope;
- changing readiness thresholds, consecutive counters, terminal dominance,
  startup/Stable policy, liveness, restart/finalization, VM lifecycle or
  cleanup;
- adding an acknowledgement/atomicity contract between ServiceLifecycle and
  mesh, DNS or dataplane consumers, revoking established connections, or
  changing consumer retry/fail-closed behavior;
- adding broker capabilities, a new target type, wildcard evaluations,
  ServiceLifecycle cadence/resync, global relist tuning, a warm router cache,
  new task ownership, or a recovery journal;
- expanding E11 to replicas, a new allocation identity, store-outage SLOs,
  cross-node watch guarantees, or any VM Exec/UDP probe design.

## Verification and proof obligations

No implementation may advance from this amendment on static reasoning alone.
After independent DESIGN approval, the original step-03-01 crafter must first
run the required seeded regression through the real production composition
boundary and then implement only this contract.

### Seeded control-plane invariant

Add or transition the existing `overdrive-sim` wake spike at seed `25717` into
a real production-owner-path invariant. It must fail before the correction
because an accepted readiness `ProbeResultRow` has no wake, and pass after the
correction because it observes, in order:

1. a Running/Stable Service allocation and healthy backend baseline;
2. an accepted readiness Fail write through `ProbeRunner`/`SimObservationStore`;
3. `SubscriptionEvent::Row(ObservationRow::ProbeResult(_))`;
4. a `ServiceLifecycle` broker evaluation and a false `ServiceBackendRow`;
5. unchanged allocation state, Stable state, allocation id, restart count and
   terminal history;
6. an accepted readiness Pass event, a true backend row, and the same
   unchanged VM allocation;
7. no terminal allocation can be made eligible by a late Pass.

The invariant must use a deterministic seed and print that seed on failure.
It must exercise the production `ProbeRunner`, store adapter, interest router,
broker and convergence composition, not inject a synthetic subscription event
or call a private reconciler helper directly. It must distinguish the durable
probe-row state from the event projection and assert no event for an equal/LWW
losing write. This is the required seeded Sim safety/liveness/convergence
evidence for the ordering defect; no mutation run belongs in step 03-01.

### Adapter/router contract evidence

The implementation review must retain independent evidence for:

- Local and Sim accepted probe writes emitting exactly one event after the
  current-row mutation, with stale/equal writes emitting none;
- existing generic observation writes and gossip behavior remaining unchanged;
- `ObservationRow::kind()` and `ObservationRowKind::as_str()` totality and
  exact labels, with any source-local pure property carrying the exact rustdoc
  line `/// CONTRACT_SHAPE: pure-function.`;
- `ServiceLifecycle`'s interest table containing `AllocStatus` and
  `ProbeResult`, while all other reconciler declarations remain unchanged;
- event target derivation using the existing allocation point read and
  rejecting orphan/non-Service rows without synthetic targets;
- existing boot List, `Lagged` relist and periodic relist retaining their
  allocation-row target enumeration, with ServiceLifecycle hydration reading
  latest probe rows through the existing method, broker key collapse and
  typed read failures;
- cooperative cancellation: no detached probe/event task and no forced abort,
  and no event-induced terminal/restart action.

Rust tests remain in-process and must not spawn the built production binary.
The black-box expectation remains responsible for the built default-feature
binary and external peer Job only.

### Native E11 re-run

Re-run the unchanged E11 example/expectation after implementation through the
existing native-metal harness. The run must independently record:

- readiness Pass → Fail and Fail → Pass detection each within 2,000 ms from
  the row's `last_observed_at` timestamp;
- no guest reply from the during-window peer Job, with exact replies before
  and after;
- the same VM allocation id remains `Running` and `Stable`, with restart
  count zero and no liveness action;
- unchanged terminal-dominance behavior for seed `25717` in the Sim lane; and
- zero teardown delta for VM, probe tasks, network, cgroup, run directory,
  mount, loop and preparation resources.

The expectation must continue to drive the checked-in example through the
built product. It must not recreate probe specs, invoke a Rust test binary,
import an `overdrive-*` crate, or infer backend state from internal rows.
The native run is the evidence for the real VM, network, wire and cleanup
surfaces; the seeded Sim invariant is the evidence for control-plane ordering
and wake convergence.

## Independent review gate

This amendment is complete only as a proposed decision. An independent DESIGN
review must verify the revalidated production reachability, the exact enum and
method shapes, the absence of invented persistence/broker/cadence/consumer
architecture, the ordering/retry/cancellation boundaries, and the seeded Sim
plus native E11 proof obligations, including the narrowly scoped adjacent
ADR-0084 observation-vocabulary note and the feature-delta/wave-decision
alignment. The review must be recorded in a dedicated artifact (expected path:
`docs/feature/service-kind-vm-workloads/design/review-amendment-e11-readiness-wake.md`).
DELIVER step 03-01 must remain blocked until that review returns `APPROVED`.
After approval, the original step crafter—not this DESIGN agent—owns RED →
GREEN → COMMIT and its implementation review; this amendment does not alter
the historical failed execution-log entries.
