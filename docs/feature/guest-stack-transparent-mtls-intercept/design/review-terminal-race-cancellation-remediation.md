# Terminal-Race Cancellation DESIGN Remediation Review

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review type | DESIGN remediation review |
| Review iterations | 1–2 |
| Iteration 1 reviewed commit | `d2071336b71c5d9fd1328041e622fd944bef50c5` (`d5ce795b87b2c4da1abbf3ff13babc48b3e2e104..d2071336b71c5d9fd1328041e622fd944bef50c5`) |
| Iteration 2 reviewed commit | `f5ef1d30a7d6eb0eb31a1caf0171fa9527af4c18` (`c8578f0e0266eda55d7cb523583bbb5a3b790ee8..f5ef1d30a7d6eb0eb31a1caf0171fa9527af4c18`) |
| Latest subject | `docs(design): make terminal tail replay reachable` |
| Review basis | Accepted recovery DESIGN plus `deliver/mutation/terminal-race-remediation-review.md` findings TRR-01 through TRR-04 |
| Current verdict | **NEEDS_REVISION** |

## Executive assessment

The remediation closes most of the previously omitted shape. It pins exactly
two fresh-read/compound-write proposals, separates `Ok(None)` from store
failure, makes post-cleanup supervision release total with a private drop
guard, leaves an in-flight local redb transaction owned by its synchronous
closure, makes the accepted tail contiguous and non-awaiting, preserves the
Job-only exit fence, and adds the required observable complements without
inventing public API, persistence, topology, or task ownership.

One High-severity liveness defect remains. The selected exhaustion and
cancellation dispositions depend on a later “level-triggered”
`StopAllocation` dispatch, but the existing production reconciler does not
emit `StopAllocation` for either current state that needs that replay. The
exit-observer winner is `Terminated` with `terminal: None`, and the accepted
cancelled Stop winner is `Terminated` with the exact terminal claim. The
`WorkloadLifecycle` stop and GC branches emit Stop only for `Running` rows and
declare all other states complete. Consequently, the retained route cannot be
shown to converge after two losers or cancellation cut B, and DELIVER would
have to invent an unapproved retry/re-drive owner or alter reconciler behavior.

## Review boundary and evidence

The reviewed range changes exactly seven DESIGN/architecture documents:

- `docs/feature/guest-stack-transparent-mtls-intercept/design/upstream-changes.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/design/wave-decisions.md`;
- `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md`;
- `docs/product/architecture/adr-0037-reconciler-emits-typed-terminal-condition.md`;
- `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md`;
- `docs/product/architecture/adr-0083-driver-registry-and-per-driver-allocation-payload.md`;
- `docs/product/architecture/brief.md`.

The range contains 607 insertions and 66 deletions. The direct parent and
documentation-only scope are correct. The pre-existing dirty `AGENTS.md` is
outside this review and was not modified.

Source was inspected only to test whether the proposed control flow is
executable through the accepted production composition:

- `crates/overdrive-reconcilers/src/workload_lifecycle.rs:619-662,670-706`;
- `crates/overdrive-control-plane/src/reconciler_runtime.rs:1703-1748,1767-1783`;
- `crates/overdrive-control-plane/src/lib.rs:3562-3655`;
- `crates/overdrive-control-plane/src/action_shim/mod.rs:2567-2713`.

## Contract validation

| Requirement | Result | Assessment |
|---|---|---|
| Exactly two fresh authoritative read plus compound-write proposals | **PASS** | R4a fixes a two-proposal budget with no delay or configuration surface. |
| First `Ok(None)` retains supervision/route and has zero write/projection effects | **PASS** | The first loser immediately rebases inside the same dispatch and performs no tail effect. |
| Second `Ok(None)` releases once, retains route, returns `Ok(())`, and remains replayable | **FAIL** | The local effects are precise, but the asserted unchanged level-triggered replay is not reachable for the exit-observer `Terminated` winner; see TRC-ARCH-001. |
| Store read/write `Err` remains distinct from `Ok(None)` | **PASS** | Both error paths use the existing typed error, release once, retain the route, and preserve atomic zero-write semantics. |
| Post-cleanup cancellation releases supervision exactly once | **PASS** | A private synchronous armed guard owns release across every later await and never removes the route. |
| In-flight local compound write has one completion owner | **PASS** | The existing `spawn_blocking` redb closure is assigned transaction completion and the accepted-current subscription-send attempt; no detached lifecycle task or receipt is introduced. |
| Accepted Stop tail cannot be cooperatively split | **PASS** | After `Ok(Some)` returns, release, route removal, and direct best-effort send are specified as one synchronous no-await tail. |
| Exact-terminal replay repairs the process-local tail without duplicating durable/live facts | **FAIL** | The branch itself is idempotent and exact, but no accepted production wake can cause the reconciler to emit that replay after cancellation cut B; see TRC-ARCH-001. |
| Terminal Job late-exit fence | **PASS** | `Job && terminal.is_some()` remains the only no-write fence and releases supervision without route ownership. |
| Terminal Service and Job Platform-Reclamation complements | **PASS** | Both remain eligible for Driver-sourced exit current plus occurrence with `terminal: None`, projections, and supervision release. |
| TRR-03 downstream outcome matrix | **PASS with blocked premise** | Accepted, loser, error, cancellation, Service, reclamation, route, occurrence, subscription, and direct-event deltas are stated precisely; replay-dependent rows remain blocked by TRC-ARCH-001. |
| TRR-04 durable/live documentation distinction | **PASS** | Accepted current plus bounded occurrence is durable; ObservationStore subscription and direct `LifecycleEvent` remain distinct process-local projections. |
| No rejected architecture reintroduced | **PASS** | No outbox, receipt, second store/route, public retry/error, detached future, generic task owner, or multi-process protocol is introduced. |
| Exact API/topology disposition | **PASS** | The change is confined to private action-shim/store control flow and existing surfaces; R0-R8 and the one-ObservationStore boundary remain intact. |

## Finding

### TRC-ARCH-001 — the selected replay owner cannot re-emit Stop for either repair state

- **Severity:** High
- **Dimensions:** liveness, cancellation convergence, executability, ownership
- **Status:** Open
- **Design evidence:** `feature-delta.md:1441-1448,1476-1494,1505-1509`;
  `design/upstream-changes.md:20-62`;
  `adr-0083-driver-registry-and-per-driver-allocation-payload.md:2768-2811`
- **Production evidence:**
  `crates/overdrive-reconcilers/src/workload_lifecycle.rs:619-662,670-706`;
  `crates/overdrive-control-plane/src/reconciler_runtime.rs:1703-1748,1767-1783`;
  `crates/overdrive-control-plane/src/lib.rs:3570-3655`

R4a says a second `Ok(None)` may return success because the unchanged
desired/current mismatch re-emits `StopAllocation`. In the finite race this
remediation is meant to handle, however, the competing exit observer has
already accepted a `Terminated` current with `terminal: None`. Both the
explicit-stop and absent-intent GC branches filter allocations to
`state == Running`; their comments explicitly classify Pending, Draining, and
Terminated rows as requiring no action, and they clear the pending retry view
when the filtered action set is empty. The retained terminal mismatch is
therefore not part of the reconciler's action predicate.

The runtime's immediate self-enqueue does not replay the stale action. It
submits a new evaluation only after dispatch returns, which rehydrates current
state and recomputes the action list. ObservationStore subscription wakes and
the periodic relist do the same. Each path sees `Terminated`, emits no Stop,
and considers convergence complete. Thus two-loser exhaustion can release
supervision and retain the route indefinitely without another production
dispatch capable of accepting the target terminal claim.

Cancellation cut B has the complementary failure. If the redb closure commits
the exact target terminal after the caller is dropped, the guard releases
supervision but correctly retains the route. The cancelled convergence tick
never reaches its post-dispatch self-enqueue, while a subscription or periodic
re-evaluation again sees a `Terminated` row and emits no Stop. The proposed
exact-terminal tail-repair branch is sound only if invoked; the accepted
production composition supplies no invocation. A cancellation-A/non-accepted
cut has the same problem whenever the exit observer has already advanced the
current row to `Terminated` with no terminal claim.

This is an architecture gap, not a request for the DELIVER crafter to add a
loop, public retry surface, or detached task. A direct unit or acceptance test
that manually redispatches the old action would prove only that the repair
branch works when called, not that production convergence reaches it.

**Required remediation:** DESIGN must pin a production-reachable owner and
trigger for a fresh Stop dispatch in both states: (1) the non-target
`Terminated` current left after bounded loser exhaustion or cancellation, and
(2) the exact target terminal current committed during a cancelled compound
write whose process-local route tail is incomplete. The choice must remain
within the accepted recovery architecture, must state its cancellation and
deduplication semantics, and must be back-propagated consistently through the
feature delta, ADRs, brief, upstream summary, decision log, and downstream
contracts. DELIVER must not infer a new reconciler predicate, retry owner,
receipt, or task system from this finding.

## Architecture and scope checks

| Area | Result |
|---|---|
| Accepted one-node/one-process/one-data-directory boundary | PASS |
| One ObservationStore current-plus-occurrence system of record | PASS |
| Existing LWW and unreadable-predecessor semantics preserved | PASS |
| Public/cross-crate API unchanged by this remediation | PASS |
| C4 topology and dependency graph unchanged | PASS |
| Reuse/component classification and Contract Shape | PASS — existing action shim and local ObservationStore adapter only; bounded-change semantics are explicit and no CREATE-NEW component is introduced |
| Rejected outbox/replay/receipt/durable-route systems remain absent | PASS |
| Rejected public retry/error/task ownership remains absent | PASS |
| Route ownership remains action-shim-only | PASS |
| Exit observer never mutates `AllocDriverIndex` | PASS |
| Production-reachable terminal tail convergence | **FAIL — TRC-ARCH-001** |

## Independent verification

| Check | Result |
|---|---|
| `git diff --check d5ce795b87b2c4da1abbf3ff13babc48b3e2e104 d2071336b71c5d9fd1328041e622fd944bef50c5` | **PASS** |
| Reviewed-range file scope | **PASS** — exactly the seven requested DESIGN/architecture documents |
| Commit parent and subject | **PASS** |
| Existing reconciliation predicate audit | **FAIL for the design premise** — Stop/GC emit only for `Running`; `Terminated` rows do not produce `StopAllocation` |
| Runtime re-drive audit | **PASS as source fact** — self-enqueue, subscription routing, and periodic relist submit reevaluation rather than replaying a prior action |
| Public API/rejected-surface audit | **PASS** — no new surface or rejected mechanism in the reviewed range |
| Code/tests | **NOT RUN** — documentation-only DESIGN review; source inspection was sufficient to establish the executability counterexample |

## Iteration-1 remediation disposition

| Finding | Disposition |
|---|---|
| TRC-ARCH-001 — selected replay owner cannot re-emit Stop for either repair state | **OPEN** — DESIGN must supply a production-reachable convergence disposition before DELIVER resumes this remediation |

## Iteration-1 verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 1 High, 0 Medium, 0 Low**.

The bounded proposal, supervision guard, store-closure ownership, no-await
accepted tail, observable complements, and documentation corrections are
otherwise precise and architecture-consistent. DELIVER remains blocked only
on the missing production-reachable replay/convergence disposition identified
by TRC-ARCH-001.

---

## Iteration 2 — target-aware replay-owner remediation

### Assessment

The remediation makes the originally reviewed explicit-stop and absent-intent
GC partitions production-reachable. The exact read-only
`AllocDriverRouteView` surface is feasible over the existing
`AllocDriverIndex` map, actual hydration retains only the target-workload
intersection, and desired hydration remains neutral. For those two branches,
the Running, `Terminated/None`, exact+routed, exact+unrouted,
mismatched-terminal, Pending, and Draining predicates are mutually explicit.
The completed-action self-enqueue, AllocStatus watch, Lagged recovery, and
unconditional 30-second relist all submit fresh hydration/diff work rather than
replaying a stale action. Exact+routed removes the route through the
action-shim's no-cleanup/no-write tail; exact+unrouted then emits no action, so
duplicate wakes converge without spinning.

The local compound-store contract also remains correctly asynchronous at the
public port. Only the synchronous redb transaction closure continues on the
existing blocking pool, owns commit/rollback and the accepted-current
subscription-send attempt, and returns through the async method when its caller
survives. The remediation introduces no synchronous facade, Tokio runtime
lookup, detached async completion, outbox, receipt, durable route, second
store, retry service, multi-instance protocol, or public task/completion API.

TRC-ARCH-001 is therefore closed for the two newly enumerated branches.
However, the design still treats those branches as the complete
`StopAllocation` replay universe. Production has another
WorkloadLifecycle-owned Operator Stop emitter for generation replacement, and
its two-loser and cancellation-B states bypass the new predicates. That leaves
the old allocation's process-local route stranded when the run branch advances
to a fresh allocation id. One new High finding remains.

### Iteration-2 contract validation

| Requirement | Result | Assessment |
|---|---|---|
| Explicit Operator-stop target predicate | **PASS** | Running and `Terminated/None` author the target; exact+routed repairs only the tail; exact+unrouted and mismatched terminal are stable. |
| Absent-intent SystemGc target predicate | **PASS** | The same target-parametric predicate and complements are pinned; no Operator-only special case is permitted. |
| `AllocDriverRouteView` hydration shape | **PASS** | Exact read-only synchronous key snapshot, core impl for the existing map type, target intersection on actual, empty desired, and exhaustive context fallout are specified. |
| Completed-action self-enqueue | **PASS for the two covered branches** | A bounded-yield Stop returns to the runtime; `has_work` submits a fresh evaluation after dispatch. |
| Subscription, Lagged, and 30-second relist triggers | **PASS for the two covered branches** | Each trigger rehydrates current plus route membership; no captured action or receipt is replayed. |
| Exact-tail duplicate/no-spin behavior | **PASS for the two covered branches** | One exact+routed tail removes the route with zero cleanup/store/live-event delta; the next exact+unrouted diff is empty. |
| Two-loser exhaustion convergence | **FAIL as a complete `StopAllocation` contract** | Explicit-stop and GC converge, but generation-replacement Stop bypasses both predicates and can advance to a fresh id with the old route retained; see TRC-ARCH-002. |
| Cancellation-B accepted convergence | **FAIL as a complete `StopAllocation` contract** | Explicit-stop and GC reach exact+routed repair, but generation-replacement exact Operator terminal is filtered as historical before fresh placement; see TRC-ARCH-002. |
| Store API remains async | **PASS** | `write_alloc_lifecycle` retains its async signature; only the internal synchronous redb closure uses the existing blocking pool. |
| Cancellation/error/supervision semantics | **PASS for specified branches** | The private guard releases once after cleanup; read/write error retains the route and returns the existing typed error; the accepted no-await tail remains unsplittable. |
| Current/occurrence/subscription/direct-event semantics | **PASS for specified branches** | Accepted current+occurrence remains atomic; store subscription and direct event remain separate best-effort projections; exact repair duplicates neither. |
| Terminal Job / Service / Platform-Reclamation complements | **PASS** | The Job-only late-exit fence, Service Driver-sourced exit, reclamation `terminal=None`, and exit-observer route non-ownership remain unchanged. |
| Rejected architecture remains absent | **PASS** | No outbox, receipt, second store, durable route, new task owner, public retry, detached completion, or multi-process protocol is introduced. |
| Complete downstream verification contract | **FAIL** | Tests cover only Operator-stop/GC targets and omit the production generation-replacement Stop emitter and its fresh-id route disposition. |

### TRC-ARCH-002 — replay ownership omits the generation-replacement Stop emitter

- **Severity:** High
- **Dimensions:** completeness, liveness, route ownership, cancellation
  convergence, testability
- **Status:** Open
- **Design evidence:**
  `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1360-1377,1465-1490,1520-1589,1658-1685`;
  `docs/feature/guest-stack-transparent-mtls-intercept/design/upstream-changes.md:20-77,146-156`
- **Production evidence:**
  `crates/overdrive-reconcilers/src/workload_lifecycle.rs:716-721,749-792,848-875,1099-1144`;
  `crates/overdrive-reconcilers/src/service_lifecycle.rs:1040-1064`;
  `crates/overdrive-control-plane/src/action_shim/mod.rs:1701-1707,2047-2049,2501-2506,2567-2713`

R4a says it governs the `StopAllocation`/exit-observer race, but its replay
owner modifies only the explicit-stop and absent-intent GC branches. The
WorkloadLifecycle run branch has a third live Stop emitter: when
`restart_pending` is true and the current allocation is Running, it emits
`StopAllocation` with the same Operator target before placing the replacement.
ServiceLifecycle adds a fourth Stop target for liveness. Neither emitter is
included in the design's predicate or verification inventory.

The generation-replacement omission has a concrete stranded-route trajectory:

1. a generation advance causes the run branch to emit Operator Stop for the
   current Running allocation;
2. on two-loser exhaustion, the exit observer has left that allocation
   `Terminated` with `terminal: None` and an Operator-stop reason, while the
   Stop releases supervision and retains the old route; alternatively,
   cancellation B can commit the exact Operator terminal while retaining that
   route;
3. the next WorkloadLifecycle evaluation still has a desired job and no stop
   intent, so it enters neither new target-aware branch. Its run branch filters
   the old Operator-stop row from `active_allocs_vec`, and `restart_pending`
   deliberately overrides the normal Operator-stop veto; and
4. placement mints a distinct allocation id from the retained history and
   dispatches `StartAllocation`. That fresh start inserts only the new id's
   route. The old route is removed only by a terminal action-shim tail for the
   old id, which this trajectory no longer emits.

Self-enqueue, watch delivery, and periodic relist cannot repair an action the
pure diff never emits. After fresh placement stamps the desired generation,
the old row is historical rather than current, making the omission stable.
The two-loser case additionally lacks the Stop-authored terminal current and
occurrence promised by the generic bounded-yield contract. The
cancellation-B case has its durable facts but never completes the old
process-local route tail.

The liveness emitter confirms that the two-branch list is not an exhaustive
inventory even where its existing restart path may reuse or later remove the
same-id route. A complete design must state the disposition rather than leave
DELIVER to infer which non-explicit Stop targets need exact-tail repair.

**Required remediation:** enumerate every production
`Action::StopAllocation` emitter and pin the production-reachable bounded-loser
and cancellation-B disposition for each. In particular, the
generation-replacement Operator Stop must converge the old allocation's route
and terminal authorship before or as the fresh-id replacement proceeds, with
the same no-duplicate durable/live effects and no-spin guarantees. Extend the
downstream contracts to drive that real run-branch emitter and its existing
self-enqueue/watch/relist triggers. DESIGN must choose the in-architecture
predicate/ordering; DELIVER must not invent a new action, retry owner, receipt,
task, or persistence surface.

### Iteration-2 architecture and API checks

| Area | Result |
|---|---|
| One-node/one-process/one-data-directory boundary | PASS |
| ObservationStore remains the sole durable lifecycle boundary | PASS |
| Route remains process-local and action-shim-mutated | PASS |
| Additive public Rust surface exactly pinned | PASS — one read-only trait and two hydration/state fields |
| Read/write capability split | PASS — route port exposes keys only and no mutator |
| `write_alloc_lifecycle` remains async | PASS |
| Blocking-pool adaptation remains synchronous internally | PASS |
| No detached async completion or runtime lookup | PASS |
| C4 component/crate topology | PASS — existing HydrationContext edge only; no new component or dependency direction |
| Contract Shape and effect isolation | PASS for the new route view and covered pure predicates |
| Exhaustive Stop-tail ownership | **FAIL — TRC-ARCH-002** |

### Iteration-2 independent verification

| Check | Result |
|---|---|
| `git diff --check c8578f0e0266eda55d7cb523583bbb5a3b790ee8 f5ef1d30a7d6eb0eb31a1caf0171fa9527af4c18` | **PASS** |
| Remediation scope | **PASS** — exactly eight DESIGN/architecture documents; no code, tests, roadmap, DES log, mutation configuration, or prior review artifact changed |
| Commit parent and subject | **PASS** |
| Explicit-stop/GC production reachability audit | **PASS** — predicate, route hydration, self-enqueue, watch, Lagged, and periodic relist form fresh evaluations |
| Exact-tail boundedness/no-spin audit | **PASS for covered branches** — route removal makes the following diff empty; duplicate wakes are coalesced or idempotent |
| Async store boundary audit | **PASS** — public async signature retained; no sync facade or detached async future sanctioned |
| Rejected-mechanism audit | **PASS** |
| Production Stop-emitter inventory | **FAIL** — generation-replacement and liveness emitters are absent from the replay/verification disposition |
| Code/tests | **NOT RUN** — documentation-only DESIGN review; source inspection establishes the omitted production trajectory |

### Iteration-2 finding dispositions

| Finding | Disposition |
|---|---|
| TRC-ARCH-001 — selected replay owner cannot re-emit Stop for either repair state | **CLOSED for explicit-stop and absent-intent GC** — target-aware predicates plus route hydration and existing triggers make both reviewed states reachable and bounded |
| TRC-ARCH-002 — replay ownership omits the generation-replacement Stop emitter | **OPEN** — complete the production emitter inventory and pin each bounded-loser/cancellation-B disposition |

### Iteration-2 verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 1 High, 0 Medium, 0 Low**.

The new read port, hydration edge, explicit-stop/GC predicates, runtime
triggers, async store boundary, cancellation guard, and exact-tail no-spin
behavior are approved as designed. DELIVER remains blocked on the omitted
generation-replacement Stop trajectory and the resulting incomplete
`StopAllocation` replay contract.
