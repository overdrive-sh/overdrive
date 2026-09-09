# DESIGN review — E11 readiness wake, ADR-0101 revision 5

| Field | Value |
|---|---|
| Feature | `service-kind-vm-workloads` |
| Amendment under review | `docs/feature/service-kind-vm-workloads/design/amendment-e11-readiness-wake.md` |
| ADR under review | `docs/product/architecture/adr-0101-service-backend-health-observed-convergence.md`, proposed revision 5 D8 |
| Related architecture | ADR-0084, as amended by the adjacent E11 observation-vocabulary amendment |
| Roadmap boundary | DELIVER step `03-01`, E11 readiness recovery |
| Review type | Fresh isolated DESIGN review |
| Reviewer | Codex independent reviewer |
| Model | User-selected GPT 5.6 Luna, maximum thinking |
| Review date | 2026-09-09 |
| Iteration | 1 |
| Verdict | **APPROVED** |

## Scope and review boundary

This is a focused DESIGN review of the E11 readiness-wake amendment. It
verifies that the proposed event-only `ProbeResult` projection is necessary for
the reproduced production failure, that its public API shape is closed and
implementable, and that it preserves the existing ServiceLifecycle ownership,
terminal dominance, broker, consumer, cancellation, and relist boundaries.

This review does not approve implementation, the failed historical 03-01
execution, the native E11 expectation, mutation testing, or any adjacent
restart, persistence, consumer-acknowledgement, replica, or VM-lifecycle
architecture. It does not reopen the independently approved ADR-0101 revision
3/4 decisions. No production code, test, expectation, roadmap, DES log, or
unrelated dirty file was changed; no test, native run, mutation run, commit, or
additional agent was started. The required review artifact is the only file
written by this review.

I read `CLAUDE.md`, all seven mandatory `.claude/rules` files, and the
`nw-sa-critique-dimensions`, `nw-review-workflow`, `nw-security-by-design`, and
`nw-source-verification` skills. I also read the complete amendment, ADR-0101,
ADR-0084 as amended, the feature-delta, wave decisions, roadmap step 03-01,
and the 03-01 execution evidence, then traced the current production source
from `ProbeRunner` through the observation adapters, interest router, broker,
ServiceLifecycle, action shim, and consumers.

## Reviewed artifact pins

These hashes identify the design and execution artifacts reviewed in this
iteration.

| Artifact | SHA-256 |
|---|---|
| `design/amendment-e11-readiness-wake.md` | `e2d0400acdf0f23ee42b2498d4115175c4cafd898632f6f5491144673e0f6644` |
| `adr-0101-service-backend-health-observed-convergence.md` | `382bcbca3fbaa5b81b1119efd249123261cab545e89779f522b7e7e8b69b8440` |
| `adr-0084-reconciler-cadence-and-interest-declarations.md` | `7ed0bc7ee9ec807ec6b87dee5844ee33f595308a60567cbf78004aa2907d226d` |
| `feature-delta.md` | `1ad3dc66b91cceb08674e6be11d8989d46c2ac5ecdabacf261ddd02d3d91ea3b` |
| `design/wave-decisions.md` | `0a8433b1d208acd6a112033d327323ed7ba684e32346e1a8317db602433912a3` |
| `deliver/roadmap.json` | `56e279e133c15309de4248e727040e69481ac40af730c2f2852a57c20067e3d1` |
| `deliver/execution-log.json` | `96d6fd1df464550b772c8c7bf712f15a3d1b80518bcde5c8ac47ea42234ce48d` |

## Necessity and revalidated production path

The amendment addresses a concrete production failure. The native E11 run
recorded in `verification/expectations/E11-vm-service-readiness-traffic-recovery/evidence/product-run.out:69-88`
shows the same VM Service allocation still `Running` with restart count zero
while its readiness row is `Fail`. The during-window peer Job then received the
guest reply that the E11 negative assertion requires to be withheld
(`product-run.out:96-105`). The run recorded zero teardown deltas
(`product-run.out:115-120`) and exited with the expected failure status. The
03-01 execution log records this as the native GREEN failure at
`2026-09-09T09:12:09Z` (`docs/feature/service-kind-vm-workloads/deliver/execution-log.json`,
03-01 GREEN event). The readiness interval and timeout are each one second
(`examples/service-kind-vm-workloads/readiness-recovery.toml:27-32`), while the
expectation measures observation timestamp to detection and requires at most
2,000 ms (`examples/service-kind-vm-workloads/run-example.sh:345-377`).

The complete current owner path independently explains the failure:

1. The action shim awaits the successful `Running` allocation-row write and
   calls `driver.on_alloc_running` (`crates/overdrive-control-plane/src/action_shim/mod.rs:2103-2231`).
   The VM driver delegates that hook to `ProbeRunner::start_alloc`, and its
   terminal hook remains the existing `stop_alloc`
   (`crates/overdrive-worker/src/vm_driver.rs:1894-1903`).
2. `ProbeRunner::start_alloc` creates one supervised task per validated probe
   descriptor and is idempotent for an already-started allocation
   (`crates/overdrive-worker/src/probe_runner/mod.rs:298-350`). The supervised
   loop continues readiness and liveness after the startup role is retired
   (`crates/overdrive-worker/src/probe_runner/mod.rs:623-679`).
3. A real tick invokes the adapter, constructs the current `ProbeResultRow`,
   and awaits `ObservationStore::write_probe_result`
   (`crates/overdrive-worker/src/probe_runner/mod.rs:526-621`). The local
   adapter commits the probe LWW mutation but discards the accepted result and
   never emits through its subscription sender
   (`crates/overdrive-store-local/src/observation_backend.rs:968-982`). The Sim
   adapter has the same absence of fan-out
   (`crates/overdrive-sim/src/adapters/observation_store.rs:866-879`).
4. `ServiceLifecycle` hydrates the durable probe snapshot, including readiness,
   but its current declaration contains only `AllocStatus`
   (`crates/overdrive-reconcilers/src/service_lifecycle.rs:858-922,464-474`).
   The current router routes only `AllocStatus`, and its boot, lag, and
   periodic list legs enumerate only allocation rows
   (`crates/overdrive-control-plane/src/lib.rs:3463-3532,3555-3652`).
   The only level-triggered fallback for a quiet probe stream is the 30-second
   relist (`crates/overdrive-control-plane/src/lib.rs:3406-3414`).
5. Consequently the durable readiness fact exists but no broker evaluation is
   submitted for it. The stale `ServiceBackendRow` remains visible to the
   existing mesh/DNS consumers. The production convergence loop could service
   a broker evaluation at its existing 100 ms cadence
   (`crates/overdrive-control-plane/src/reconciler_runtime.rs:1203-1205`),
   but it cannot service an evaluation that the absent event edge never
   submits.

This is a reachable path through the default `serve` composition. It does not
depend on a fabricated observation, a forced task abort, a second allocation,
or an unreachable internal state. The native failure proves the stakeholder
outcome; the source path proves that the missing edge is the causal gap.

The new shape is indispensable within the accepted architecture. Reusing an
`AllocStatus` event would mislabel a probe observation and wake allocation
lifecycle consumers for a false allocation transition. A worker callback would
couple `ProbeRunner` to the control-plane broker and bypass the store's durable
LWW boundary. A new channel or durable wake journal would add an unapproved
ownership and recovery architecture. The selected event-only row projection
uses the already-owned write, stream, interest table, target lookup, broker,
and hydration path, with only the two new vocabulary variants required to
describe the observed row family.

### Evidence qualification

The execution log describes a seeded Sim wake reproduction at seed `25717`.
The present checked-in `crates/overdrive-sim/tests/acceptance/service_kind_vm_terminal_invariant.rs`
is a direct terminal/readiness reconciler test and does not itself exercise
`ProbeRunner`, the subscription, the interest router, or the broker. No separate
full wake-spike source file is present in the current tree. I therefore do not
count that log sentence as independently verified Sim evidence in this review.
This is not a design finding: the native E11 run independently proves the
production contradiction, and the amendment explicitly makes the full
production-owner-path Sim invariant a post-approval proof obligation
(`amendment-e11-readiness-wake.md:401-429`). DELIVER must not claim the Sim
obligation complete until that invariant exists and runs through the required
composition.

## Exact API and contract-shape verification

**Result: PASS.** The amendment closes the public shape and does not grant an
implementation latitude that would require inventing API.

### Observation vocabulary

The exact additions are:

```rust
ObservationRow::ProbeResult(ProbeResultRow)
ObservationRowKind::ProbeResult
```

Both are appended after the existing `Signal` variants. The existing
`ObservationRow::kind(&self) -> ObservationRowKind` gains exactly the total arm
`Self::ProbeResult(_) => ObservationRowKind::ProbeResult`; the existing
`ObservationRowKind::as_str(self) -> &'static str` returns exactly
`"probe-result"`. No existing enum order or discriminant changes. Exhaustive
matches and kind-coverage tests are compiler-required fallout, not new surface.

The event projection is explicitly not an `ObservationWrite`, generic
`ObservationStore::write` input, history entry, rkyv envelope, persistence
table, or generic observation-gossip payload. The durable source remains the
existing `ProbeResultRowV1`/`ProbeResultRow` at
`(alloc_id, role, probe_idx)`. This preserves the existing persistence and
cross-peer boundaries.

### Existing method postconditions

The public signatures remain unchanged, including
`write_probe_result(ProbeResultRow) -> Result<(), ObservationStoreError>`,
`subscribe_all_events() -> Result<LagAwareSubscription, ObservationStoreError>`,
`alloc_status_row(&AllocationId)`, and
`list_probe_results_for_alloc(&AllocationId)`.

The amended producer-specific postcondition is exact:

| Probe write outcome | Required stream effect |
|---|---|
| Strictly newer LWW winner, or local unreadable-predecessor repair | Exactly one `SubscriptionEvent::Row(ObservationRow::ProbeResult(row))`, synchronously after commit and before `Ok(())` |
| Stale or equal timestamp | Existing `Ok(())` no-op and no event |
| Failed transaction or commit | Existing typed error and no event |
| No subscribers | Existing benign broadcast-send condition; boot List and relist remain recovery paths |

The accepted boolean remains private. The amendment does not add an `accepted`
return value, a second subscription, a second `SubscriptionEvent` variant, or
a broker-side side channel. The current generic stream contract is extended in
the design only for this producer; the generic `ObservationStore::write`
acceptance/emission behavior remains unchanged. Updating the trait rustdoc and
adapter comments to state that producer-specific extension is required
implementation documentation fallout, not an API ambiguity.

### Interest and router shape

`ServiceLifecycleReconciler::interests()` retains its exact signature and
returns exactly:

```rust
&[
    ObservationRowKind::AllocStatus,
    ObservationRowKind::ProbeResult,
]
```

No other reconciler declares `ProbeResult`. The amendment pins the private
router helpers to the existing store capability, interest table, and restricted
broker capability:

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

For `ProbeResult`, the point read uses the existing
`alloc_status_row(&probe_row.alloc_id)`, derives only
`workload/<workload_id>` for an existing current `Service` allocation, and
submits the existing evaluation. Orphan, read-error, and non-Service rows are
not given synthetic, wildcard, or `alloc/<id>` targets. The List, `Lagged`
relist, and 30-second periodic relist continue to enumerate `AllocStatus`; the
Service hydration path reads probe winners through the existing
`list_probe_results_for_alloc` method. No public method, type, field, trait,
action, target type, cadence, broker capability, configuration value, or
consumer protocol is added.

The proposed router helper becomes asynchronous because the existing point read
must be awaited. This is a private implementation change and is fully pinned;
it does not preserve a stale synchronous public interface or introduce a
detached task.

## Reachability, ordering, and failure analysis

The proposed ordering is coherent with the current owners and effects:

| Boundary | Current evidence and proposed behavior | Review result |
|---|---|---|
| Probe attempt → durable observation | `probe_tick` awaits the adapter, creates the unchanged row, and awaits `write_probe_result` (`probe_runner/mod.rs:526-621`) | Existing typed adapter/store errors remain per-tick errors; the supervisor logs and continues (`:623-679`). |
| LWW mutation → event | Local `apply_probe_result_lww` already uses strict timestamp dominance and returns an accepted boolean (`observation_backend.rs:1363-1386`); Sim uses the same strict winner rule (`observation_store.rs:512-533`) | Emit only after commit and only for the accepted winner. A loser cannot produce a wake or regress durable state. |
| Event → target | Current production boot subscribes before List and gives the router a restricted submit-only broker (`control-plane/src/lib.rs:3073-3105,3416-3461`) | `ProbeResult` does a current allocation point read, then uses the same Service workload target. No callback or broker bypass. |
| Target → evaluation | Existing `EvaluationBroker` collapses a pending key `(ReconcilerName, TargetResource)` (`core/src/eval_broker.rs:57-103`) | Multiple concurrent role/index/list edges collapse while pending; the design makes no stronger exactly-once-evaluation claim. |
| Evaluation → policy | Runtime hydrates actual state, runs pure reconciliation, persists View before awaited dispatch, and self-re-enqueues existing emitted work (`reconciler_runtime.rs:1399-1501,1592-1608`) | The new edge changes only when the existing policy runs; it does not move policy ownership or action ordering. |
| Policy → backend row | ServiceLifecycle computes readiness and terminal veto in `service_backend_row_actions` (`service_lifecycle.rs:1081-1176`) and `compute_backend_healthy` (`:1178-1214`) | Readiness still changes only `Backend.healthy`; no readiness-induced restart or terminal action is authorized. |
| Backend row → consumers | The existing action shim awaits `ObservationStore::write(ServiceBackend)` (`action_shim/write_service_backend_row.rs:46-58`); mTLS/DNS consumers ignore non-`ServiceBackend` rows and retain their watch/relist paths (`mtls_resolve_adapter.rs:771-807`; `dns_responder/name_index.rs:399-424`) | Probe events wake the authoritative publisher; they do not become a consumer protocol or acknowledgement. |

### Duplicate and lost-event behavior

The exact event contract distinguishes event duplication from evaluation
coalescing. Each accepted LWW mutation has one event; stale/equal writes have
none. The sanctioned production supervisor creates one task per descriptor and
does not re-spawn on an idempotent `start_alloc` call
(`probe_runner/mod.rs:298-350`). Stable cancels only the startup role while
readiness/liveness continue (`driver.rs:846-860`), and terminal cleanup uses the
existing cooperative `stop_alloc` (`probe_runner/mod.rs:358-370`). Thus the
amendment does not introduce a second sanctioned writer for one live
`(alloc_id, role, probe_idx)` key.

Concurrent adapter commits can deliver accepted events in commit order rather
than probe-call initiation order. Hydration always reads the durable LWW winner,
so an event cannot make an older row authoritative. A broadcast lag is surfaced
as the existing `SubscriptionEvent::Lagged`; the router relists allocation
targets, and hydration reads the latest durable probe rows. A closed stream
stops the router under its existing shutdown/watch-failure behavior. These are
level-triggered recovery semantics, not an invented event journal.

The existing readiness counter increments on each ServiceLifecycle evaluation
(`service_lifecycle.rs:1196-1213`), while the broker's collapse guarantee is
limited to evaluations still pending (`eval_broker.rs:81-103`). I audited the
possible repeated-evaluation path and do not raise it as a finding here: it is
pre-existing evaluation-based policy, E11 uses `success_threshold = 1`, and the
amendment neither claims one event equals one counter increment nor changes the
threshold/counter contract. A fix would require a separate, approved policy
design and a seeded current production-owner-path failure; it is not authorized
as E11 wake hardening. The required implementation invariant must not overclaim
exactly-once reconciliation and should retain this distinction.

### Terminal dominance and lifecycle boundaries

ServiceLifecycle's existing reconcile guards terminal and Stable allocations
before readiness projection (`crates/overdrive-reconcilers/src/service_lifecycle.rs:494-510`).
Backend projection evaluates only current `Running` facts and passes the retained
terminal veto into `compute_backend_healthy`
(`service_lifecycle.rs:1108-1119`). A late Pass event therefore cannot make a
terminal allocation eligible, and the event itself cannot emit
`StopAllocation` or `RestartAllocation`. Liveness remains the existing
ServiceLifecycle detector and WorkloadLifecycle remains the sole restart
authority. The amendment's counterexample is supported by the existing
terminal seed path and is required again in the E11 Sim invariant.

An in-flight probe/store future remains allowed to finish when cancellation is
requested. The existing supervised loop observes cancellation between ticks;
it does not force-abort an in-flight adapter or store operation
(`probe_runner/mod.rs:623-679`). If its accepted row commits, the post-commit
event is valid; the next hydration's current allocation and terminal checks
remain authoritative. Router and convergence shutdown remain cooperative and
do not introduce drain, release, or cleanup effects
(`control-plane/src/lib.rs:3603-3657`; `reconciler_runtime.rs:1575-1590`).

## Lifecycle Gate Ownership

No lifecycle gate moves. The amendment adds a trigger for the existing
ServiceLifecycle owner; a probe row, event, broker evaluation, or consumer
effect is not an additional state authority.

| Signal / state | Owner and promise | Affected result | Unaffected owner/state | Ordering and counterexample | Evidence |
|---|---|---|---|---|---|
| Allocation `Running` | Existing action shim and selected VM driver; a successful `Running` row still means driver/Beacon success | None; a probe event only wakes observation reconciliation | VM process, Beacon, Stable, readiness, liveness, restart | Readiness `Fail` after `Running` does not stop or restart the allocation | Native E11 same allocation, restart count zero; action-shim/driver source |
| Service `Stable` | Existing ServiceLifecycle startup branch | None; probe event does not create or revoke Stable | Running, readiness, liveness, terminal ownership | A late readiness event cannot make the allocation Stable or un-Stable | `service_lifecycle.rs:494-595`; native/Sim terminal controls |
| `Backend.healthy` | ServiceLifecycle remains the sole complete `ServiceBackendRow` author | Existing readiness status/threshold and retained terminal veto change only `healthy` | Running, Stable, startup terminal, liveness stop, WorkloadLifecycle restart | Durable row → event → broker → hydration/policy; late Pass after terminal remains ineligible | `service_lifecycle.rs:1081-1214`; required seed 25717/native E11 |
| Probe observation | ProbeRunner remains the sanctioned writer; ObservationStore remains durable LWW owner | Accepted row adds one local post-commit stream projection | Probe schema/key, AllocStatus, terminal occurrences, View, gossip, consumers | Equal/stale write and failed commit produce no event | `probe_runner/mod.rs:526-621`; both adapter LWW paths |
| Broker evaluation | Existing interest router and shared `EvaluationBroker` | One routed ServiceLifecycle evaluation may be pending for the accepted event | Key collapse, 100 ms cadence, dispatch, other interests | Orphan/non-Service point read has no target; no wildcard wake | `control-plane/src/lib.rs:3416-3657`; `core/src/eval_broker.rs:81-103` |
| Mesh/DNS/backend consumers | Existing consumers apply materialized `ServiceBackendRow` asynchronously | They receive the same complete false/true row sooner | No acknowledgement, connection revocation, or consumer-owned health predicate | E11 must measure real peer Job and consumer effects, not infer them from event arrival | Existing consumer source; native E11 expectation |
| Liveness/restart | ServiceLifecycle detects liveness; WorkloadLifecycle alone restarts/finalizes | None; readiness event does not create liveness/restart action | Restart budget, allocation ID, exit watcher, cleanup | Readiness flap preserves Running/Stable and zero restart delta | `service_lifecycle.rs` liveness path; roadmap 03-01 controls |

The precise changed gate is therefore a wake edge only:

- **Owner:** ServiceLifecycle still owns readiness policy, terminal veto, and
  complete `ServiceBackendRow` projection.
- **Promise:** an accepted local probe-result write for a current Service
  allocation submits an existing broker evaluation through the existing stream
  and interest table.
- **Failure projection:** probe-store failure remains a typed ProbeRunner
  error; point-read failure is logged and retried only by existing newer events
  or relist; neither becomes an allocation terminal or restart.
- **Ordering:** durable probe row, event projection, broker evaluation,
  ServiceLifecycle backend-row action, then existing consumer effects.
- **Counterexample retained:** a terminal allocation with a later Pass still
  produces no eligible backend, and a readiness flap does not change Running,
  Stable, or restart count.

## Alternatives and architectural scope

The amendment's alternatives are correctly bounded:

| Alternative | Disposition verified |
|---|---|
| Event-only `ObservationRow::ProbeResult` plus `ObservationRowKind::ProbeResult` on the existing stream | Selected; it is the smallest store-owned edge that preserves durable LWW, target lookup, interest routing, broker collapse, and hydration. |
| Second `SubscriptionEvent` variant or `subscribe_probe_results()` | Rejected; it creates a second stream/API and breaks the single existing interest vocabulary. |
| `workload_id` or generation metadata in `ProbeResultRow` | Rejected; the existing allocation point read owns alloc-to-workload mapping, and persisted V1 shape is out of scope. |
| ProbeRunner callback, broker handle, or ServiceLifecycle handle | Rejected; it couples worker and control-plane owners and bypasses the observation boundary. |
| Synthetic `AllocStatus` event | Rejected; it misstates probe truth as allocation lifecycle truth and could wake the wrong consumers. |
| One-second resync or shorter global relist | Rejected; it changes cadence/load and remains polling rather than fixing the missing accepted-write edge. |
| Durable wake journal, acknowledgement, or consumer barrier | Rejected; no such dependency is in the reproduced path and it would expand persistence/ownership architecture. |

The compatibility statement is accurate for this greenfield in-process enum:
there is no serde, wire, or rkyv persistence for `ObservationRow` or
`ObservationRowKind`. All adapters and control-plane code must cut over
together; no mixed-version protocol, rollout flag, or migration is introduced.
The explicit exclusions for readiness thresholds, terminal/liveness/restart
policy, VM cleanup, cross-peer probe gossip, consumer acknowledgement, and
replica/store-outage SLOs prevent scope drift.

## Verification sufficiency and evidence disposition

### Seeded Sim invariant

The required seed is `25717`, and the required invariant is sufficient if
implemented exactly as specified. It must drive the production-owner
composition—not a private reconciler helper or synthetic event—through:

1. a `Running`/`Stable` Service baseline;
2. `ProbeRunner` writing an accepted readiness `Fail` to the Sim observation
   store;
3. one `SubscriptionEvent::Row(ObservationRow::ProbeResult(_))` after the
   durable LWW mutation;
4. the existing interest router and restricted broker submitting and draining a
   ServiceLifecycle evaluation;
5. a false `ServiceBackendRow` with unchanged allocation, Stable state,
   allocation ID, restart count, and terminal history;
6. an accepted readiness `Pass`, a true backend row, and the same unchanged VM
   allocation; and
7. a terminal late-Pass control that remains ineligible.

The invariant must print seed `25717` on failure, distinguish durable row state
from event projection, assert no event for stale/equal writes, and fail against
the current implementation because the event edge is absent. This is the
correct evidence layer for control-plane ordering. The current direct terminal
test and the execution-log narrative do not satisfy that composition by
themselves; they remain an explicit post-approval implementation obligation,
not evidence to be silently substituted.

The adapter/router tests must separately prove local and Sim accepted-write
event cardinality, LWW loser suppression, exact kind/label totality, Service
target derivation, orphan/non-Service rejection, subscribe-first boot ordering,
Lagged relist, periodic relist, typed point-read failure, and unchanged generic
write/gossip behavior. Any source-local pure property must carry the exact
rustdoc declaration `/// CONTRACT_SHAPE: pure-function.` on every live
property. These requirements are narrow compiler/test fallout from the closed
contract, not new API.

### Native E11 rerun

After implementation, the unchanged checked-in example and expectation must be
rerun through the existing native-metal harness. The black-box expectation must
verify, on one replica and one unchanged VM allocation:

- Pass → Fail and Fail → Pass detection each within 2,000 ms from the row's
  `last_observed_at` timestamp;
- no guest reply during the failed window, with exact positive replies before
  and after;
- `Running`, `Stable`, restart count zero, and no liveness action;
- terminal seed `25717` behavior in the independent Sim lane; and
- zero teardown delta for VM, probe, network, cgroup, run directory, mount,
  loop, and preparation resources.

The expectation must drive the built default-feature binary and external peer
Job only. It must not import an `overdrive-*` crate, invoke a Rust test binary,
recreate probe specs/workloads inline, or infer the public result from internal
rows. Rust/Sim tests remain in-process evidence for the internal composition;
the native expectation is evidence for VM, network, wire, consumer, and
cleanup surfaces. The failed current run has no ledger because it exits before
ledger extraction; the rerun must produce independent transition evidence.

No mutation run belongs to step 03-01; mutation remains the final DELIVER-wave
gate.

## Findings and remediation dispositions

No critical, high, medium, or low DESIGN finding remains within this bounded
amendment. The following reviewed concerns are explicitly dispositions, not
open findings:

| Concern | Severity if misrepresented | Disposition |
|---|---|---|
| The execution log claims a seeded wake reproduction but the current tree has no full owner-path wake-spike source | Evidence risk | Native E11 and source reachability independently prove the contradiction. The full seed-25717 composition is an explicit post-approval proof obligation; DELIVER may not claim it complete until implemented and run. No design expansion is authorized. |
| Broadcast events can lag or the watch can close | Existing failure mode | The existing `Lagged` signal and allocation relist recover the durable level-triggered state; closed watch remains a cooperative terminal watch failure. No new journal or acknowledgement is needed for healthy E11. |
| Broker collapse is only for pending evaluations; runtime self-reenqueue can evaluate again | Existing policy boundary | The amendment makes no exactly-once-evaluation or one-event-one-counter claim and does not change readiness threshold semantics. A separate seeded, reachable policy defect would require separate DESIGN approval. |
| A late in-flight probe write can emit after cancellation or terminal intent | Existing cooperative cancellation behavior | Durable row/event ordering plus current-state and terminal hydration remain authoritative; no forced abort or new fence is introduced. |
| Global stream traffic may cause lag under load | Existing stream capacity concern | E11 is a healthy single-replica bound and the design retains existing lag/relist behavior; scale or outage SLO expansion is explicitly out of scope. |

None of these dispositions authorizes a new public method/type/field/variant
beyond the exact appended pair, a persistence or recovery subsystem, a broker
capability, a consumer acknowledgement, or a lifecycle-owner change.

## Iteration history

| Iteration | Reviewed revision | Verdict | Findings | Disposition |
|---:|---|---|---|---|
| 1 | ADR-0101 revision 5 D8 / E11 readiness-wake amendment | **APPROVED** | None remaining in scope | The exact amendment is ready for the original 03-01 crafter's independent RED → GREEN → COMMIT and subsequent implementation review. |

## Final verdict

**APPROVED.** The amendment is necessary for the actually reproduced E11
failure: the production ProbeRunner already writes the readiness LWW row, but
the current local and Sim adapters emit no subscription item, the current
ServiceLifecycle declares no probe-result interest, and the existing 30-second
relist cannot satisfy the two-second withdrawal contract. The selected
`ObservationRow::ProbeResult(ProbeResultRow)` plus
`ObservationRowKind::ProbeResult` pair is the smallest closed vocabulary that
repairs that edge without moving readiness ownership or inventing persistence,
broker, cadence, target, consumer, retry, or lifecycle API.

Approval is for the DESIGN contract only. It does not mark E11 green or rewrite
the historical 03-01 failure. Step 03-01 may resume only with the original
crafter's production implementation, a real seed-25717 Sim invariant through
`ProbeRunner`/store/router/broker/convergence, adapter/router evidence, and an
unchanged native E11 rerun satisfying the stated timing, traffic, lifecycle,
and cleanup assertions. The existing dirty work and execution history remain
preserved.
