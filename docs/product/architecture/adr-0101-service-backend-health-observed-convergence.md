# ADR-0101: One authoritative Service backend projection

## Status

**Accepted — revision 4 amendment; independent DESIGN review APPROVED**, 2026-09-08,
[review iteration 1](../../feature/service-kind-vm-workloads/design/review-amendment-be10-local-backend-withdrawal.md#iteration-history),
with no findings.
The user approved focused DESIGN work for BE10's local direct-VIP withdrawal
assertion. D7 below is the only approved exception to revision 3's unchanged
consumer behavior. Revision 3's approvals below remain its separate provenance.
No implementation completion, native unhealthy-routing reproduction or executed
BE10 recovery suffix is claimed.

**Proposed — revision 5 amendment; independent DESIGN review required**,
2026-09-09. The focused E11 run and seeded Sim reproduction prove that the
existing `ProbeResultRow` write does not wake `ServiceLifecycle`, while the
two-second readiness-withdrawal contract requires that wake. The proposed
amendment is recorded in
[`amendment-e11-readiness-wake.md`](../../feature/service-kind-vm-workloads/design/amendment-e11-readiness-wake.md).
It is not accepted and does not authorize DELIVER to resume until an
independent DESIGN review returns `APPROVED`.

**Accepted — revision 3; independent DESIGN review APPROVED**, 2026-09-08,
[review iteration 3](../../feature/service-kind-vm-workloads/design/review-adr-0101.md#iteration-3--focused-re-review-of-r0101-3).
The independent consolidated DESIGN+DISTILL review is also **APPROVED** on
2026-09-08 in [iteration 2](../../feature/service-kind-vm-workloads/distill/review-adr-0101-design-distill.md#iteration-2--focused-re-review-of-cd-0101-0102).
These are design/acceptance-package approvals, not production GREEN or
implementation completion; recorded behavioral REDs and unexecuted suffixes
remain implementation obligations.
The user selected one authoritative projection colocated with ServiceLifecycle
and resolved the consumer boundary: startup failure is a lifecycle fact,
**not** an acknowledgement that every routing consumer has applied withdrawal.
Authoritative publication is followed by asynchronous consumer convergence.

This is greenfield, with no users: replace the retired publisher directly.
No compatibility layer, dual publisher, rollout transition, upgrade-state
handling, or migration is part of this design. Normal runtime View persistence,
restart and reconciliation remain required.

Revision 1's health overlay was reconsidered; revision 2 compared authority
options. Neither earlier proposal's API list is executable. The
[independent review history](../../feature/service-kind-vm-workloads/design/review-adr-0101.md)
preserves the earlier CHANGES_REQUESTED verdicts and records revision 3's
APPROVED disposition in iteration 3 on 2026-09-08. Approval is independent,
not an author self-approval.

## Context and decision

Seed `257209` in
`crates/overdrive-sim/tests/e09_v2_failed_service_reachability_spike.rs`
reproduces the production-owner sequence: startup failure establishes the
ServiceLifecycle veto; Failed removes membership; WorkloadLifecycle restarts
the same allocation ID; BackendDiscoveryBridge republishes membership with
true health; the unchanged emit fingerprint suppresses correction through six
more ServiceLifecycle ticks. The retained native E09 v2 capture independently
shows replacement-VM reachability. Evidence and source boundaries are in the
[ruling](../../feature/service-kind-vm-workloads/design/backend-eligibility-convergence-ruling.md)
and [diagnosis](../../analysis/e09-v2-failed-service-reachability.md).

**Decision:** ServiceLifecycle alone derives and publishes every
`ServiceBackendRow` for the current Service's allocator-issued listeners.
It combines current Running membership with its existing allocation-scoped
eligibility policy before publication, and compares the complete desired
projection with the observed row. BackendDiscoveryBridge is removed, not
retained as a dormant or secondary publisher.

### Changed assumptions

- ADR-0096 D3's “bridge continues to converge membership” is replaced by the
  user-selected sole ServiceLifecycle projection. Its health predicate and
  WorkloadLifecycle restart authority remain unchanged.
- ADR-0079 D4's exclusion of ServiceLifecycle observed-row convergence and
  D9's unresolved shared-row ownership are superseded for this path: the
  competing publisher is removed before full-row convergence is used.
- ADR-0096 D2's earlier “resolver sees ... before” wording is corrected to
  successful row publication before terminal dispatch, with asynchronous
  consumer observation. The user explicitly selected this boundary.
- ADR-0096's distinct-ID replacement premise is corrected to the actual
  same-ID restart contract in ADR-0099; no restart policy is altered.

“Writer” here means the sole reconciler author of the complete backend
projection. The existing action shim still executes the row write through
ObservationStore; no direct store access moves into pure reconciliation.

### D1 — Guarantees and source authority

| Value / boundary | Authority and promise |
| --- | --- |
| Current listeners and VIP | Validated Service intent plus the existing allocator assignment for its spec digest. Retain every listener and `ServiceId::derive(vip, port, protocol, "service-map")`. |
| Membership | Current `AllocStatusRow` facts for this workload, filtered to `AllocState::Running`. Membership does not imply eligibility. |
| Backend identity and address | Existing `SpiffeId::for_allocation`; materialized `workload_addr` when present, otherwise the configured host IPv4; port from the particular listener. Weight remains 1. |
| Eligibility | Existing ServiceLifecycle terminal veto first, then existing readiness observations, counters and threshold. No readiness means true for a non-vetoed Running allocation; Stable is not a prerequisite. |
| Publication safety | From a deciding tick that establishes the existing veto, every backend row this owner subsequently publishes has false for that allocation whenever it is a member. Same-ID disappearance/reappearance cannot grant true. |
| Stored-state convergence | Under successful subsequent reads/writes and owner execution, the stored row converges to that complete projection and unchanged input/output produces no further row write. |
| Consumer convergence | Existing asynchronous mesh/DNS indexes and applicable ServiceMapHydrator effects consume the materialized row. Lifecycle failure reporting does not wait for those consumers. No zero-propagation-delay promise or existing-flow revocation is introduced. |

A missing observed row/member supplies **no health default**. Reconstruct
membership and evaluate the authoritative predicate, whether creating,
repairing or emptying a row. A same-ID restart does not reset
`terminal_announced`. A genuinely distinct allocation without that veto
follows ordinary eligibility policy. This design does not invent a permanent
failure policy, a new attempt identity or a rule for resetting existing policy.

The same allocation-level readiness result applies to each listener that
allocation serves. Service probe policy is allocation-scoped, not a separate
per-listener readiness policy. Evaluate/update it once per Running allocation
per Service tick with a nonempty listener projection, then reuse that result
across listeners. Adding listeners must not multiply readiness counter
increments.

### D2 — Exact public Rust surface

All items below are in
`crates/overdrive-reconcilers/src/service_lifecycle.rs` unless named otherwise.
These are complete replacement shapes for the named structs; unchanged
surrounding interfaces must not gain parameters or variants.

| Item | Exact final contract |
| --- | --- |
| `ServiceLifecycleState` | Retain `pub allocs: BTreeMap<AllocationId, ServiceAllocFact>`; replace `service_dataplane` with `pub service_dataplane: BTreeMap<ServiceId, ServiceDataplaneIdentity>`; replace `prior_backend_row_at` with `pub observed_backend_rows: BTreeMap<ServiceId, ServiceBackendRow>`. These are its only three fields. |
| `ServiceDataplaneIdentity` | Exactly `pub vip: ServiceVip`, `pub port: NonZeroU16`, `pub protocol: Proto`, `pub writer: NodeId`. Delete `service_id`: the enclosing map key supplies it. Preserve `Debug, Clone, PartialEq, Eq` derives. |
| `ServiceAllocFact` | Replace `pub backend_addr: std::net::SocketAddr` with `pub backend_ip: std::net::Ipv4Addr`. Retain `backend_spiffe` and every other field/type unchanged. The new field is the materialized workload/host fallback IP, without a first-listener port. |
| `ServiceLifecycleView` | Delete `pub last_emitted_backend_fingerprint: BTreeMap<ServiceId, BackendSetFingerprint>` and its field attribute/documentation. Retain all other fields, derives, persistence ownership, canonical reconciler name and `has_alloc_mid_startup_window(&self) -> bool` unchanged. |

`NonZeroU16` is `std::num::NonZeroU16`; `Proto` is
`overdrive_core::dataplane::backend_key::Proto`. State and listener identity
remain transient hydration values, without serde/archive persistence.

Keep `ServiceLifecycleReconciler::new() -> Self`, `Default`, its
`Reconciler` implementation signatures and associated types unchanged.
Keep `interests() = &[ObservationRowKind::AllocStatus]`; no new interest,
cadence, broker or observation API.

No new public method, trait, enum variant, action, observation type, consumer
API, configuration, dependency or data-store schema. This expressly permits
the listed State/Fact/Identity changes and retired-surface removals below;
it is not a claim of zero Rust public changes.

### D3 — Exact private interface changes and hydration

In `service_lifecycle.rs`:

| Existing function | Final signature / disposition |
| --- | --- |
| `service_dataplane_identity` | Replace with `async fn service_dataplane_identities(ctx: &HydrationContext<'_>, workload_id: &WorkloadId, svc: &ServiceV2) -> Result<BTreeMap<ServiceId, ServiceDataplaneIdentity>, HydrateError>`. |
| `hydrate_service_alloc_facts` | `async fn hydrate_service_alloc_facts(ctx: &HydrationContext<'_>, workload_id: &WorkloadId, spec_facts: &(u32, Duration, String, bool, bool), readiness_facts: &(bool, u32), liveness_facts: &(bool, u32)) -> Result<BTreeMap<AllocationId, ServiceAllocFact>, HydrateError>`. Delete only the `backend_port: u16` parameter. |
| `hydrate_service_lifecycle_actual` | Unchanged: `async fn hydrate_service_lifecycle_actual(ctx: &HydrationContext<'_>, workload_id: &WorkloadId) -> Result<ServiceLifecycleState, HydrateError>`. |
| `readiness_backend_row_action` | Replace with `fn service_backend_row_actions(actual: &ServiceLifecycleState, next_view: &mut ServiceLifecycleView, tick: &TickContext, startup_failed_this_tick: &BTreeSet<AllocationId>) -> Vec<Action>`. Returns row/hydrator-handoff pairs, not lifecycle actions. |
| `compute_backend_healthy` | Keep the existing signature: `fn compute_backend_healthy(alloc_id: &AllocationId, fact: &ServiceAllocFact, next_view: &mut ServiceLifecycleView, startup_failed_this_tick: bool) -> bool`. The final argument continues to receive the combined deciding-tick/retained terminal veto, not merely membership. |

All other production helper signatures remain unchanged except the removals
in D5. No additional private cross-component interface is required. Ordinary
local implementation structure is the crafter's responsibility, not permission
to add a public seam.

Hydration uses the current Service-only intent path. Absent/non-Service intent
returns empty State; missing allocator assignment or no listeners produces an
empty listener map and no backend projection actions. Do not synthesize a VIP.
The plural identity helper reuses the bridge's all-listener computation,
including the spec-digest lookup and exact ServiceId domain string, within
ServiceLifecycle; no parallel listener cache or fact store.

Actual hydration contains the allocation facts, all listener identities and
the complete current backend row from the existing keyed
`ObservationStore::service_backends_rows(&service_id)` read for every listener.
A missing result omits that map entry. Propagate existing `HydrateError`
failures, never treat failed reads as absence. Hydrate allocation/probe facts
once per workload, not once per listener. The desired hydration method retains
its existing trait signature and returns empty State after target validation;
the existing actual-side projection supplies the policy/intent inputs.

### D4 — Projection, deduplication and action ordering

For every current listener, construct the complete desired row with exactly
the Running allocation members in allocation-ID order. Construct each address
from `backend_ip` and that listener's port. Use the allocation's once-per-tick
computed eligibility; do not carry an observed health value into desired.
Keep the existing IPv4-only allocator contract; no new address-family policy.

Empty membership is a real desired value: write an empty row when the current
row is missing or nonempty; once the observed empty row matches, emit nothing.
Do not return early merely because there are no allocations or no Running
allocations. With no current listener identities, there is no managed row key
to write. This does not add historical-listener garbage collection or change
existing intent/VIP withdrawal ownership.

Compare `service_id`, VIP and the entire deterministic backend vector against
the observed row, excluding only `updated_at`. Skip only on that equality.
For every differing/absent row, use the existing
`LogicalTimestamp::dominating(tick.tick, identity.writer.clone(), observed_stamp)`.
Reuse `fingerprint(&identity.vip, &backends)` solely as correlation content:
target `service-lifecycle/backends/{service_id}`,
`ContentHash::of(fp.to_le_bytes().as_slice())`, purpose
`"write-service-backend-row"`. No emit/acknowledgement marker is persisted;
a dispatch result is not the next tick's equality oracle.

For each changed listener, in ServiceId order, return exactly the existing
`WriteServiceBackendRow { row, correlation }` followed by the existing
`EnqueueEvaluation` for `service-map-hydrator` at
`service/{service_id}`. Resolve its name from
`<crate::service_map_hydrator::ServiceMapHydrator as Reconciler>::NAME`;
no new public constant or constructor. Health-only changes require the same
handoff as membership/address changes.

Insert this whole ordered projection-action group before the first deciding
`FinalizeFailed { terminal: Some(ServiceFailed { reason: StartupProbeFailed }) }`
already in the Service lifecycle action vector. If there is no such action,
append the group after the existing startup/Stable actions. Then run the
existing liveness-action collection. This retains lifecycle ordering and
ensures every changed listener withdrawal is attempted before startup failure
publication. Unchanged already-false rows need no write.

The runtime still persists next View before serial awaited dispatch.
Successful row write precedes later terminal dispatch; consumers may observe
it afterward. On a row-write error, the existing shim continues draining and
may publish the lifecycle failure. No transaction, terminal barrier or new
error handling is added. Retained veto plus next observed-state comparison
keeps later projections false and allows readback repair. Runtime self-enqueue
after emitted work, AllocStatus interest routing, boot LIST and the existing
periodic interest-router relist remain the wakeup paths. No new resync schedule.

### D5 — Direct retirement and registration contract

| Location | Exact removal / replacement |
| --- | --- |
| `overdrive-reconcilers/src/backend_discovery_bridge.rs` | Delete the module after relocating the membership/listener computation into D3–D4. Retire `BackendDiscoveryBridge`, `BackendDiscoveryBridgeState`, `BackendDiscoveryBridgeView`, `ServiceListenerSet`, `ProjectedListener`, `RunningAllocSet`, their constructors and bridge-only helpers. No forwarding exports or no-op reconciler. |
| `overdrive-reconcilers/src/lib.rs` | Remove module declaration/re-exports and `BackendDiscoveryBridge` variants from `AnyState`, `AnyReconciler`, `AnyReconcilerView`, with their forwarding/match arms. Existing ServiceLifecycle variants are unchanged. |
| `overdrive-control-plane/src/reconciler_runtime.rs` | Remove bridge import, `AnyViewMap::BackendDiscoveryBridge`, registration/load/read/write/backoff match arms and bridge-only canonical-name helper. Remove `loaded_backend_discovery_bridge_views_for_test`, `apply_next_backend_discovery_bridge_view_for_test`, and `seed_backend_discovery_bridge_view_for_test`; do not replace these with new seams. Retain normal ServiceLifecycle View loading/persistence. |
| `overdrive-control-plane/src/lib.rs` | Delete public `backend_discovery_bridge(host_ipv4: Ipv4Addr, writer_node_id: NodeId) -> AnyReconciler` and its registration call. Keep `service_map_hydrator(host_ipv4)` and `service_lifecycle()` registered before runtime/loop use, in their existing relative order. Retain host-IP resolution: allocation hydration and the hydrator still need it. |
| `overdrive-reconcilers/src/workload_lifecycle.rs` | Remove bridge import/name constant and bridge enqueue. Within the existing alloc-mutating-action block, enqueue ServiceLifecycle once for Service-kind workloads on **all four** existing variants: Start, Restart, Stop, FinalizeFailed. Retain `is_alloc_mutating_action(&Action) -> bool` and `SERVICE_LIFECYCLE_NAME`; delete `is_service_alloc_starting_action`. Preserve the SVID enqueue and all restart/budget/VIP-release behavior. |
| `overdrive-control-plane/src/action_shim/reclamation.rs` | Delete only the bridge-name helper/import and its broker submission. The already-present WorkloadLifecycle, ServiceLifecycle and SvidLifecycle submissions remain. Reclamation guards, resource effects, terminal authorship and signatures remain unchanged. |

Remove live references to retired production symbols in compiler-required
call sites. Bridge-specific in-process evidence must be transferred or retired
under the acceptance designer's ownership, including the old Sim invariant
module's three bridge evaluators/wrappers; it must not keep a second production
publisher alive. No renamed shadow reconciler or additional Sim/public seam.

No code reads, converts, deletes, aliases or otherwise manages old bridge/View
formats for upgrade purposes. No legacy-blob tests or rollout controls.
Normal restarts of the **new** program still reload its ServiceLifecycle View;
greenfield does not authorize erasing the terminal-policy input.

### D6 — Consumers and failure/lifetime boundaries

Revision 3 contract follows; accepted revision 4 D7 overrides only the named
local direct-VIP action-selection behavior. All other D6 boundaries remain.

Keep `ServiceBackendRow`, `ObservationStore`, the write action/executor and
all consumer APIs unchanged. Mesh `ServiceBackendsResolve` and DNS
`NameIndex` retain their List/Watch/relist and existing fault behavior.
ServiceMapHydrator retains its listener-fact lookup, materialized healthy bit,
retry behavior and supported dataplane paths. No consumer reconstructs the
ServiceLifecycle predicate or guesses health from membership.

No new consumer acknowledgement or inter-consumer atomicity protocol. A
successful backend write does not mean a resolver index has processed it.
Existing DNS answers do not become revocable routing decisions, and no
existing connection is terminated by this change. Consumers converge when
their existing observation/effect paths make progress; no new latency SLO.

Normal ServiceLifecycle View persistence retains terminal inputs across
runtime restart; backend output is always read back. The real convergence
loop awaits each tick before checking shutdown. No detached publication task,
cross-View read, replay journal, policy reset, global writer fence or
generation/attempt identity is added. The single logical writer claim covers
the current production composition, not an invented HA topology.

### D7 — Revision 4 amendment: honor local backend health with existing removal

BE10 seed `257221`, focused run `21245211-7507-4f57-b5c1-48d450097fc5`,
reproduced a healthy local registration surviving an authoritative readiness
withdrawal and the actual queued ServiceMapHydrator dispatch. The local map
remained `Some(192.0.2.10:18081)` instead of `None`. The recovery suffix did
not execute. This is production-owner-to-Sim-adapter evidence, not native
unhealthy-routing evidence and not the original mesh-subnet VM E09 mechanism.

On the existing local-fingerprint change, ServiceMapHydrator must emit
`RegisterLocalBackend` for a classifier-accepted local backend with
`healthy: true`, and `DeregisterLocalBackend` for that same still-present
backend with `healthy: false`. Do not filter unhealthy members out before
fingerprinting or action construction: their address supplies the existing
removal port's required reverse key. Both actions already exist and have the
same fields. Recovery uses the existing registration path. Preserve the mesh
exclusion, local/remote partition, listener identity, remote actions, retry
memory and local-fingerprint emission gate; do not recompute health policy.

Only one private helper changes name, in `service_map_hydrator.rs`:
`fn push_register_local_backend_actions(actions: &mut Vec<Action>, local:
&[&Backend], ctx: &LocalBackendEmit<'_>)` becomes
`fn push_local_backend_actions(actions: &mut Vec<Action>, local: &[&Backend],
ctx: &LocalBackendEmit<'_>)`. Its context fields and caller arguments stay
unchanged. Healthy correlation purpose stays `"register-local-backend"`;
unhealthy purpose is `"deregister-local-backend"`, with the same existing
target and full desired-set content hash. No public type, field, action,
method, parameter, error variant, schema or adapter API changes.

Reuse the awaited deregistration shim and `Dataplane::deregister_local_backend`
unchanged, including idempotent forward-then-reverse removal and typed errors.
Successful action completion withdraws that local rewrite; a map miss allows
the original destination unchanged, not a guaranteed connection denial.
Lifecycle publication never waits for this later consumer tick. This amendment
adds neither established-flow revocation nor a local-map readback/retry protocol.
The existing local fingerprint records emission before dispatch, not successful
application; do not claim that a subsequent unchanged tick retries a failed
local effect. BE10's healthy/failing trajectory and required recovery suffix
assume successful local effects; recovery remains unexecuted. Normal
one-Running-per-workload convergence bounds
this amendment; replica arbitration and BE02 are separate and unchanged.

The [focused amendment](../../feature/service-kind-vm-workloads/design/amendment-be10-local-backend-withdrawal.md)
pins exact reused signatures, fields, production evidence, failure boundaries,
changed assumption, reuse analysis and acceptance-designer handoff. It is part
of this accepted contract, approved by the
[independent revision 4 review](../../feature/service-kind-vm-workloads/design/review-amendment-be10-local-backend-withdrawal.md).

### D8 — Proposed revision 5 amendment: wake ServiceLifecycle from accepted probe results

The E11 run at `2026-09-09T09:12:09Z` and the seeded Sim wake reproduction at
seed `25717` establish a reachable contradiction with the revision 4 surface:
`ProbeRunner` writes the durable `ProbeResultRow`, but neither production
observation adapter emits a subscription item for that write;
`ServiceLifecycle` therefore remains asleep until an unrelated
`AllocStatus` event or the 30-second interest-router relist. The result is a
stale `ServiceBackendRow` during a failed readiness window even though the VM
remains Running, which violates step 03-01's two-second E11 contract.

The proposed correction preserves `ServiceLifecycle` as the sole readiness
policy and complete `ServiceBackendRow` author. It appends the following
event-only projection and interest discriminant to the existing in-process
vocabulary:

```rust
ObservationRow::ProbeResult(ProbeResultRow)
ObservationRowKind::ProbeResult
```

`ObservationRow::kind` maps the new variant to `ProbeResult`, and
`ObservationRowKind::as_str` returns exactly `"probe-result"`. Both variants
are appended after the existing `Signal` variants; existing ordering is not
renumbered. The probe variant is emitted only by an accepted
`ObservationStore::write_probe_result` after its existing LWW mutation has
committed. A stale/equal write or failed commit emits no event. The existing
`SubscriptionEvent` shape and
`ObservationStore::subscribe_all_events() -> Result<LagAwareSubscription,
ObservationStoreError>` signature remain unchanged. `ProbeResultRow`, its
`(alloc_id, role, probe_idx)` key, its V1 envelope and its persistence table
remain unchanged; the event-only projection is not a generic
`ObservationWrite`, row-history entry, or new gossip payload.

The projection covers the complete probe-result family, not readiness alone,
because `ServiceLifecycle` already hydrates startup, readiness and liveness
from the same per-allocation snapshot and `ObservationRowKind` has no
role-level dimension. Its existing policy filters the role after hydration;
adding a readiness-only kind or worker callback would create more surface and
leave the same owner without an honest wake for its other probe roles.

The existing stream-completeness rule is extended only for this producer:
an accepted `write_probe_result` is delivered as this `Row` projection (or its
loss is surfaced as the existing `Lagged` signal). Generic
`ObservationStore::write` acceptance and emission remain unchanged.

`ServiceLifecycleReconciler::interests()` retains its exact existing signature
and returns exactly:

```rust
&[
    ObservationRowKind::AllocStatus,
    ObservationRowKind::ProbeResult,
]
```

This is the narrowly scoped revision-5 replacement for revision 4 D2's
`interests() = &[ObservationRowKind::AllocStatus]` / "no new interest"
clause. Every other D2 public-surface prohibition remains in force: no new
method, field, trait, action, lifecycle state, cadence, broker capability,
resync schedule, persistence schema or consumer API is authorized.

No other reconciler declares `ProbeResult`. The existing interest router
routes a probe event by reading the current allocation with the existing
`ObservationStore::alloc_status_row(&AllocationId)` method and deriving the
existing `workload/<workload_id>` target only for a current Service allocation.
Its existing allocation List, `Lagged` relist and 30-second periodic relist
remain the only target-enumeration path; because `AllocStatus` remains in the
same ServiceLifecycle interest declaration, those paths already wake the
Service target, whose hydration reads latest probes through the existing
`list_probe_results_for_alloc(&AllocationId)`. No new store method, warm cache,
wildcard target, broker channel or resync schedule is added. The shared broker
and 100 ms convergence cadence remain unchanged. The existing runtime persists
View before serial awaited dispatch, self-re-enqueues emitted work, and leaves
consumer acknowledgement asynchronous.

Ordering is durable probe row → event projection → existing broker evaluation
→ existing ServiceLifecycle backend-row action → existing mesh/DNS consumer
effects. Probe cancellation remains cooperative between ticks; a completed
in-flight write is valid and terminal/current-state hydration still dominates
late results. Store or point-read failures retain typed/log-and-continue
behavior and are retried only by the existing next probe event or relist; this
amendment adds no outage SLO or terminal barrier. Existing readiness policy,
terminal dominance, Running/Stable meanings, liveness, restart/finalization,
cleanup, consumer List/Watch behavior and public HTTP/CLI surfaces are
unchanged.

The exact production path, alternatives, compatibility boundary and proof
obligations are pinned in the
[proposed focused amendment](../../feature/service-kind-vm-workloads/design/amendment-e11-readiness-wake.md).
This D8 text is **proposed**, not accepted. Independent DESIGN review must
approve it before DELIVER step 03-01 resumes; the existing revision 3/4
approval records and the failed 03-01 execution evidence are not rewritten.

## Alternatives and consequences

| Alternative | Evaluation against the selected contract |
| --- | --- |
| Shared-row observed-health overlay | Repairs persistent drift but still lets the independent membership writer newly grant true while the veto applies. Fails publication safety; rejected, not an interim implementation. |
| Separate membership/health facts joined by one publisher | Can be coherent with explicit retained-veto and missing-fact rules. Adds a fact lifetime and inter-owner synchronization boundary that the existing ServiceLifecycle owner does not need to consult its own policy. Not selected. |
| Separate facts joined by every consumer | Multiplies policy/freshness interpretation across mesh, DNS and materialized dataplane paths; still needs a projection for kernel consumers. Does not itself provide a terminal/consumer barrier. Not selected. |
| Single projector located at ServiceLifecycle | Chosen: combines current membership and actual policy authority before publication, retaining existing materialized consumers. Costs all-listener projection work and removal of bridge registration/callers; no new deployed component or persistence. |

[Primary-source research](../../research/backend-eligibility-convergence.md)
informs explicit projection authority and comparison with observed output.
Kubernetes/Consul are qualified precedents, not proof of this local design;
their particular stores, trackers and protocols are not imported.

Reliability improves by removing competing full-row authors and emission-based
dedup. Maintainability improves by removing duplicated endpoint construction.
Work per tick is O(allocations × listeners), already inherent in the bridge's
projection; probe hydration and readiness updates remain once per allocation.
Changed output alone writes. Rust module/enum removal makes retired production
calls fail to compile; existing pure Reconciler contracts and bounded action
capabilities remain the effect boundary. No new external dependency, adapter,
security policy, deployment unit or Earned-Trust probe is introduced.

## Lifecycle Gate Ownership and handoff

The eligibility gate's predicate and owner stay ServiceLifecycle; its authority
now governs the sole complete backend projection. It may change only published
backend eligibility, not Running, Stable, restart decisions or consumer
acknowledgements. Unavailable observations remain hydration errors; failed row
effects remain shim errors with existing retry/convergence behavior. No new
deadline or timeout budget. A Running nonterminal allocation without readiness
is the counterexample to adding a Stable/startup-success admission gate.
A late Pass cannot override the existing terminal veto.

The [ruling](../../feature/service-kind-vm-workloads/design/backend-eligibility-convergence-ruling.md)
records the full gate matrix, grounded source path and behavioral handoff.
After DESIGN approval, the **acceptance designer owns all test design and
executable tests** in DISTILL, including composed publication safety and
consumer convergence, healthy controls and reachable failure/restart evidence.
This ADR supplies contracts, not detailed scenarios or test implementations.
Keep the seeded witness and independent persistent-CP native E09 v2 oracle
honest; no unfavorable intermediate state may be hidden by a final-writer
assertion. No production/test/harness edit or execution is claimed by DESIGN.

The proposed revision 5 E11 wake amendment is intentionally outstanding and
has its own independent DESIGN review gate; the revision 3/4 approval history
does not approve that amendment.
