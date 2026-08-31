# DESIGN → DISTILL upstream changes — terminal race and cancellation

**Feature:** `guest-stack-transparent-mtls-intercept`

**Date:** 2026-08-31

**Scope:** terminal race/cancellation, complete Stop-emitter replay, and
resource-specific pre-start/restart unwind (TRR-01–TRR-04,
TRC-ARCH-001/002/003/004)

## Why this back-propagation is required

DESIGN does not change the accepted product outcome or external product API,
but it makes previously open liveness, cancellation, and complement behavior
observable.
Those are material acceptance-contract additions, so DISTILL must incorporate
them before DELIVER remediates code. This file specifies behavior only; it does
not author tests, change a roadmap, or prescribe test names.

The authoritative mechanisms and rationale are in `../feature-delta.md`
R4a/R4b and R6a/R6b. This file is the concise downstream contract.

## Contract 1 — bounded LWW loser policy

For one `StopAllocation` dispatch after cleanup:

- exactly two fresh-read/compound-write proposals are permitted;
- the first `Ok(None)` changes no durable family, sends no notification,
  retains supervision and the `AllocDriverIndex` route, and immediately
  permits the second proposal;
- if the second proposal is accepted, the dispatch total is exactly one Stop
  current plus one Stop occurrence, followed by one supervision release, route
  removal, one best-effort current-subscription send attempt, and one
  best-effort Reconciler direct-event attempt; and
- a second `Ok(None)` is bounded exhaustion: exactly one supervision release,
  route retained, zero Stop current/occurrence/notification deltas, `Ok(())`,
  and the target-aware WorkloadLifecycle predicate remains level-triggered for
  the exit-observer `Terminated/None` winner.

An `Ok(None)` is not a store failure. A current read `Err` or compound write
`Err` instead releases supervision once, retains the route, produces no Stop
durable/live delta, and returns the existing typed error. No new error variant,
backoff, deadline, retry owner, or public knob is allowed.

## Contract 2 — cancellation partitions

DISTILL must preserve three distinct cancellation cuts:

| Cut | Required observable outcome |
|---|---|
| After cleanup while the authoritative read is pending / before write starts | No Stop current or occurrence; supervision released exactly once; route retained; no Stop subscription or direct event. Running or `Terminated/None` remains eligible for a fresh WorkloadLifecycle action. |
| While the local `spawn_blocking` compound transaction is in flight | The redb closure reaches rollback/`None`/commit despite caller drop, and supervision releases exactly once while the route remains. If not accepted: zero Stop durable/live delta and later Running/`Terminated/None` replay. If accepted: exactly one current+occurrence, the store-owned current subscription send is attempted, the direct event may be absent, and later exact-terminal-plus-route replay removes the route without another durable/live event. |
| After `Ok(Some)` reaches the shim | Supervision release, route removal, and direct send form one synchronous no-await tail. Cooperative cancellation cannot observe only a prefix. The direct send remains best effort under process death. |

A cancelled caller receives no synthetic result or receipt. The next
level-triggered dispatch reconciles from `AllocStatusRow` plus the process-local
route view: exact target + retained route means accepted tail debt; exact target
with no route is converged; terminal-free current is eligible for a fresh bounded
proposal. Occurrence history is not queried as a receipt.

## Contract 3 — explicit-stop/GC replay owner and triggers

For these two emitters the owner is the existing `WorkloadLifecycle`
reconciler. Both explicit Operator stop and absent-intent SystemGc use the same target-aware predicate.
For target `T`, one Stop is emitted per row exactly for Running,
`Terminated/terminal=None`, or `Terminated/terminal=Some(T)` while the route
snapshot contains the allocation. Exact+unrouted, mismatched `Some(..)`,
Pending, and Draining emit no Stop.

Actual hydration obtains route membership through the exact new core read port
`AllocDriverRouteView::routed_allocations() -> BTreeSet<AllocationId>`, exposed
as `HydrationContext.alloc_driver_routes`, and stores only the target-workload
intersection in `WorkloadLifecycleState.routed_allocations`; desired hydration
uses an empty set. This is process-local input, not durable lifecycle state.

Fresh evaluations come from the existing completed-action self-enqueue,
AllocStatus subscription interest routing, Lagged relist, and unconditional
30-second interest-router relist. No stale Action, receipt, or detached task is
replayed. One exact+routed tail action removes the route; the next hydration is
exact+unrouted and emits nothing, preventing a busy loop.

## Contract 4 — exact terminal replay

An exact target terminal current at initial preflight or a later re-read has
zero cleanup/current/occurrence/subscription/direct-event delta. It releases
supervision idempotently, removes the route idempotently, and returns success.
This is tail convergence, not lifecycle event replay.

## Contract 5 — exit-observer fence complements

The canonical fence remains exactly
`kind == WorkloadKind::Job && terminal.is_some()`:

- terminal Job + late exit: current byte/cardinality unchanged, occurrence
  cardinality unchanged, no direct event, and supervision release through the
  existing no-write outcome;
- terminal Service + late exit: the Driver-sourced exit proposal remains
  eligible. When accepted, it writes one current plus one occurrence with
  `terminal = None`, attempts its current/direct projections, and releases
  supervision; and
- Job Platform Reclamation (`terminal = None`) + late exit: same eligible
  Driver-sourced behavior, with accepted current/occurrence `terminal = None`.

The Service and reclamation complements prevent a widened “fence every
terminal state” interpretation. The exit observer never owns
`AllocDriverIndex`; all three outcomes leave the route unchanged. Only the
action-shim's accepted or route-gated exact-terminal Stop tail removes it.

## Contract 6 — intermediate rejection complements

When the first Stop write is rejected and the second write is held pending,
the exact intermediate state is:

- zero Stop occurrence and zero Stop direct event;
- the original driver route is still present; and
- the supervision claim is still held.

Only the accepted second result may release/remove/project. Exhaustion instead
releases but retains the route and projects nothing.

## Contract 7 — terminology and evidence boundary

- Durable: accepted `AllocStatusRow` current plus bounded
  `AllocLifecycleOccurrenceRow`.
- Best-effort store wake: `SubscriptionEvent::Row(AllocStatus)`.
- Best-effort authoring-component bus: `LifecycleEvent`.
- Deliberate zero-event: terminal Job late exit.

The direct bus is not a permanent record, receipt, or replay log. DELIVER must
align `worker/exit_observer.rs` module and `RetryOutcome` documentation with
these terms. No black-box expectation should inspect private route or
supervision state; those are in-process integration-contract surfaces.

## Contract 8 — complete production Stop-emitter disposition

The production inventory is exactly: WorkloadLifecycle explicit Operator,
WorkloadLifecycle absent-intent SystemGc, WorkloadLifecycle
desired-generation Operator replacement, and ServiceLifecycle
LivenessProbe. A source audit and acceptance inventory must fail if any
production constructor lacks a bounded-loser and cancellation-B owner.

For `restart_pending`, the numerically current predecessor is fenced before
intentional-stop filtering and before placement. Running emits Operator Stop;
Draining waits; Terminated/None emits Operator Stop; Terminated/Some(T) with a
route emits Stop carrying the exact current T for zero-durable tail repair;
only Terminated/Some with no route may mint a fresh id and stamp the desired
generation. Tests must assert no earlier id/stamp and exactly one later
placement under self-enqueue, watch, Lagged/lost wake, and relist.

Service actual hydration adds
`ServiceAllocFact.terminal: Option<TerminalCondition>` and
`ServiceAllocFact.driver_route_present: bool`, sourced from the row and one
existing route-view snapshot, plus
`ServiceAllocFact.status_updated_at: LogicalTimestamp` copied from the row.
The existing liveness failure counter is not reset on emission. Once
threshold-reached it is retained while non-Running: Running and
Terminated/None emit liveness Stop; Draining waits; exact+routed emits the
exact tail; exact+unrouted or mismatched Some emits none and clears the
counter. Pending/Failed emit none.

Ordering is made attempt-scoped rather than broker-scoped. Add exactly
`#[serde(default)] ServiceLifecycleView.liveness_attempt:
BTreeMap<AllocationId, LogicalTimestamp>` and add serde derives to the existing
logical timestamp. On Running, a missing/different logical marker clears the
old counter and stores the accepted Running row's `updated_at` before action
dispatch. Liveness hydration supplies a probe only when its exact
`alloc_attempt` equals that status identity; wall clock does not attribute an
attempt. Thus WorkloadLifecycle may restart in the same frozen batch as Service
exact-tail removal: the next Running Service evaluation retires the historical
decision before selecting an action, even across equal or rolled-back clocks.
Exact+unrouted/mismatch also clears marker+counter. Non-Running repair retains
both, so loss/cancellation remains replayable. No broker priority, cross-View
read, receipt, or second restart owner is added. The existing exact
`AllocStatusRow.terminal` plus `driver_route_present` facts remain the repair
owner; the marker only prevents that completed old-attempt decision from being
applied to a later Running attempt.

Append `ProbeResultRowEnvelope::V2` with exactly
`alloc_attempt: Option<LogicalTimestamp>` and move both public payload aliases
to V2; V1 migrates as `None`. The existing latest-row LWW compares attempt first: newer
logical attempt beats old/legacy regardless of wall clock, same attempt retains
the strict wall-clock comparison, and legacy never replaces attributed data.
The accepted Running row's identity flows through the changed existing
`Driver::on_alloc_running`, `ProbeRunner::start_alloc`, and
`ProbeRunner::probe_once_and_record` signatures pinned in R4b. No new row
family, key axis, task, method, or history is added. The existing wire-only
`ProbeResultRowJson`, OpenAPI schema, and CLI render remain unchanged; their
conversion omits the internal attempt identity.

## Contract 9 — provision-failure structural unwind

After `NetSlotAllocator::assign` succeeds, every later workload-netns or VM-TAP
provision error must invoke the existing allocation-keyed structural teardown
before writing Failed. Production teardown attempts typed-owned host-stranded
TAP, host veth/route, and the owned resolver directory in that order even when
an earlier sibling fails. Only when all three prove absence may it delete the
netns (which reaps the in-netns peer/TAP). Absence is success. The four-stage
set remains the diagnostic bound; the existing return type carries the first
typed cleanup error while structured logs preserve every attempted failure.

The slot releases only after all dependent resources and the netns are proven
absent; otherwise it stays bound to the same derived names for an existing
lifecycle retry. A dependent failure withholds netns deletion, leaving
`ovd-ns-<slot>` as the existing process-loss discovery anchor. Boot observes
that name before allocation, derives the same deterministic plan, and adopts a
live owner or runs ordinary orphan GC; a GC error refuses boot.

Final delete is a private three-state converge over the existing slot-named
path: `Absent`, `MountedNamespace` (`statfs == NSFS_MAGIC`), or
`DetachedPlaceholder` (path present, non-NSFS). Mounted calls the existing
detach+unlink operation; on error it re-observes and finishes unlink-only when
detach already succeeded. Detached always runs unlink-only. Boot enumerates
both path-present states, correlates owners only for Mounted, and sends
Detached directly to orphan GC. Observation/unlink failure refuses boot; slot
release requires path state Absent after all dependents are absent. The safe
`nix::sys::statfs` observer uses only the existing workspace dependency with
its `fs` feature; no marker/scanner/public method/error is added. Once final
path deletion succeeds, every dependent resource was already absent, so a cut
before in-memory `release` is safe and needs no anchor. The
Failed cause keeps the original `WorkloadNetnsProvisionFailed` class and
primary detail first, appending the returned cleanup error when present. An
accepted Failed write returns success; cleanup failure does not replace the
primary or skip the record. Slot exhaustion owns no resources and calls no
teardown. A fresh pre-driver failure owns no driver supervision, route,
intercept listener/rule, or SVID.

## Contract 10 — old-protection-first same-id restart

The observable order is prior driver stop → awaited old `stop_alloc` → total
old structural teardown and slot release → replacement provision → awaited
identity → prior supervision release → driver start → Running commit → new
`start_alloc` → D7 → awaited EXEC release. A private synchronous guard totals
only the existing prior `release_supervision` call after quiescence and before
the new `Driver::start` ownership cut.

A driver-stop error preserves all old protection. An old-stop error runs no
network/replacement work; listeners/rules are already dropped by the worker
and its failed handles remain privately retryable, while old network/slot and
route remain. Old network failure runs no replacement and retains the slot.
Provision failure executes Contract 9 and records Failed; identity/other
driver error executes the same structural unwind but preserves its existing
primary error and row semantics; typed StartRejected records the existing
Failed disposition. A Running-write failure stops the just-started driver,
releases supervision, and structurally unwinds before returning the store
error. Every cut after successful old `stop_alloc` has no old active redirect
or listener; every cut after successful old network teardown has no old
netns/veth/TAP/resolver state.

Reachability is existing level-triggered machinery: every emitted Restart sets
the runtime's pre-dispatch `has_work` bit, so both successful and failed
dispatch return paths self-enqueue the workload target. Accepted Failed writes
also send the ordinary AllocStatus subscription, while the existing backoff
gate and unconditional relist cover delayed/lost wakes; process loss falls to
boot netns GC through the netns-last anchor. No cleanup-specific timer, task,
receipt, or queue is added.

## Unchanged boundaries

No new store/action/driver/network/task method, error variant, external wire
schema, retention bound, C4 topology, persistence subsystem, or roadmap is
added. The internal cross-crate Rust surface is the
exact `AllocDriverRouteView` trait, `HydrationContext.alloc_driver_routes`,
`WorkloadLifecycleState.routed_allocations`, the three exact
`ServiceAllocFact` fields, `ServiceLifecycleView.liveness_attempt`, the V2
probe-attempt field/envelope, serde derives on `LogicalTimestamp`, and the
three changed existing Running/probe signatures in Contract 8.
`ServiceLifecycleState` and `ProbeResultRowJson` gain no field.
`teardown_workload_netns`,
`WorkloadNetworkProvisioner`,
`VethProvisionError`, and `ShimError` retain their signatures/variants.
No outbox, second store, durable route, event receipt, multi-process protocol,
public retry, detached completion future, `CompletionFence`, or `OwnedTaskSet`
is sanctioned.
`ObservationStore::write_alloc_lifecycle` remains async. The local adapter may
run only its existing synchronous redb closure on the blocking pool; a sync
public facade, Tokio runtime lookup, and detached async completion are all
forbidden.

## Verification additions

DISTILL must cover both Operator and SystemGc over Running,
Terminated/None, exact+routed, exact+unrouted, mismatched-Some, Pending, and
Draining; route-snapshot target filtering; two-loser self-enqueue convergence;
cancelled accepted-write convergence through subscription and through the
30-second relist when that wake is lost; exact-tail zero cleanup/store/event
deltas; duplicate wake/no-spin steady state; the generation and liveness
partitions in Contract 8 through their real emitters/triggers; every provision
and dependency-ordered four-stage unwind cut in Contract 9; process exit after
every successful prefix/failure, including detach-before-unlink and
post-detach unlink failure, with boot rediscovery before slot allocation; the
same-frozen-batch Service-tail→Workload-restart ordering under equal and
rolled-back wall clocks; attempt-first probe LWW and exact-attempt hydration;
and the complete action trace/failure matrix in Contract 10 for Exec and VM as
applicable. Source-local pure
properties carry the exact `/// CONTRACT_SHAPE: pure-function.` declaration.
A test that only manually redispatches a stale Stop or calls a private cleanup
helper is insufficient because it does not prove the production owner,
trigger, or resource boundary.
