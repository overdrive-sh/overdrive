# Terminal-Race Cancellation DESIGN Remediation Review

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review type | DESIGN remediation review |
| Review iterations | 1–5 |
| Iteration 1 reviewed commit | `d2071336b71c5d9fd1328041e622fd944bef50c5` (`d5ce795b87b2c4da1abbf3ff13babc48b3e2e104..d2071336b71c5d9fd1328041e622fd944bef50c5`) |
| Iteration 2 reviewed commit | `f5ef1d30a7d6eb0eb31a1caf0171fa9527af4c18` (`c8578f0e0266eda55d7cb523583bbb5a3b790ee8..f5ef1d30a7d6eb0eb31a1caf0171fa9527af4c18`) |
| Iteration 3 reviewed commit | `59e0afc7dd8007f6a88260a2e0fda80bb138266d` (`00bb83482c964cc427db846cfc63de47363ed133..59e0afc7dd8007f6a88260a2e0fda80bb138266d`) |
| Iteration 4 reviewed commit | `623b9af157f14e9cc6a6d43a8504e64dd5eba9a8` (`5b1adfcc818b6e4e236a30a9174f472489127560..623b9af157f14e9cc6a6d43a8504e64dd5eba9a8`) |
| Iteration 5 reviewed commit | `91e819f0655d113c9b6be2c6e520394a280ff221` (`2dd7ba82d786a3810f2c8804888a79eb8ef0f566..91e819f0655d113c9b6be2c6e520394a280ff221`) |
| Latest subject | `docs(design): make attempt and netns recovery total` |
| Review basis | Accepted recovery DESIGN; `deliver/mutation/terminal-race-remediation-review.md` findings TRR-01 through TRR-04; iterations 1–4 findings TRC-ARCH-001 through TRC-ARCH-004; original-scope provision/restart unwind |
| Current verdict | **NEEDS_REVISION** |

## Executive assessment

Iteration 5 closes the time-identity and final-delete state-machine defects in
their runtime models. The accepted Running row's dominating logical timestamp
is a durable, wall-clock-independent attempt identity; the existing hook and
probe-runner path carries it into attempt-first latest-row ordering, exact
hydration, and View-before-dispatch counter reset. The netns path is now
observed as absent, mounted NSFS, or detached placeholder; live retry and boot
GC unlink a detached placeholder without repeating the failed detach, and slot
release waits for dependent resources plus path absence. The prior replay,
generation, unwind, terminal, async, and rejected-architecture decisions remain
intact.

One High implementation-feasibility defect keeps TRC-ARCH-003 open. The exact
rkyv enum append cannot also satisfy the mandated “V1 bytes remain fixed and
decodable” contract. rkyv sizes the archived enum's inline region to its largest
variant; adding the proposed optional logical timestamp grows a V1 archive from
72 to 96 bytes. An exact throwaway archive probe reproduced the current 72-byte
V1 fixture, then showed that the same V1 payload under the proposed two-variant
enum is 96 bytes and that the old bytes fail to decode. DELIVER cannot implement
both the specified V2 shape and the specified compatibility/fixture rule.

## Iteration 1 — initial terminal-race review

### Review boundary and evidence

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

### Contract validation

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

### Finding

#### TRC-ARCH-001 — the selected replay owner cannot re-emit Stop for either repair state

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

### Architecture and scope checks

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

### Independent verification

| Check | Result |
|---|---|
| `git diff --check d5ce795b87b2c4da1abbf3ff13babc48b3e2e104 d2071336b71c5d9fd1328041e622fd944bef50c5` | **PASS** |
| Reviewed-range file scope | **PASS** — exactly the seven requested DESIGN/architecture documents |
| Commit parent and subject | **PASS** |
| Existing reconciliation predicate audit | **FAIL for the design premise** — Stop/GC emit only for `Running`; `Terminated` rows do not produce `StopAllocation` |
| Runtime re-drive audit | **PASS as source fact** — self-enqueue, subscription routing, and periodic relist submit reevaluation rather than replaying a prior action |
| Public API/rejected-surface audit | **PASS** — no new surface or rejected mechanism in the reviewed range |
| Code/tests | **NOT RUN** — documentation-only DESIGN review; source inspection was sufficient to establish the executability counterexample |

### Iteration-1 remediation disposition

| Finding | Disposition |
|---|---|
| TRC-ARCH-001 — selected replay owner cannot re-emit Stop for either repair state | **OPEN** — DESIGN must supply a production-reachable convergence disposition before DELIVER resumes this remediation |

### Iteration-1 verdict

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

---

## Iteration 3 — complete emitter and resource-unwind remediation

### Assessment

The combined remediation correctly expands the production inventory to the
four actual `StopAllocation` constructors. The generation-replacement branch
now examines the numerically current predecessor before intentional-stop
filtering or placement: Running and `Terminated/None` emit Operator Stop,
Draining waits, terminal+routed forwards the exact current terminal for a
zero-durable tail, and only terminal+unrouted may mint and stamp the fresh
allocation. That closes the old-id route leak from iteration 2 without a new
action, receipt, durable route, or task owner.

The same-id restart sequence is also materially safer. Prior driver stop,
awaited old intercept teardown, and total old structural teardown now precede
replacement provision, identity, driver start, Running commit, new intercept,
D7, and EXEC release. The private supervision guard is bounded to the existing
idempotent release call, and the failure table preserves existing typed primary
errors and row semantics. R4a's two-proposal policy, cancellation partitions,
async `write_alloc_lifecycle`, local blocking-closure ownership, Job-only exit
fence, Service/Reclamation complements, and no-await accepted tail remain
consistent across the amended ADRs, brief, decision log, and DISTILL handoff.

The liveness and process-loss boundaries are not yet executable as claimed.
The liveness design assumes an exact+routed ServiceLifecycle repair is followed
by an exact+unrouted ServiceLifecycle pass that clears its persisted counter
before WorkloadLifecycle restarts. The runtime provides no such cross-reconciler
barrier. The same terminal subscription wakes both reconcilers; a drained batch
continues through WorkloadLifecycle after the Service repair removes the route,
while ServiceLifecycle's self-enqueued clear is deferred to the next batch.
The replacement Running row therefore encounters the still-reached counter and
the old latest Fail observation. Separately, post-netns teardown failures lose
their boot discovery key if the process exits: the allocator binding is
process-local and current boot recovery enumerates only surviving netns names.

### Iteration-3 contract validation

| Requirement | Result | Assessment |
|---|---|---|
| Complete production Stop-emitter inventory | **PASS** | Source and design agree on exactly four production constructors: explicit Operator, absent-intent SystemGc, generation Operator, and Service liveness. |
| Explicit-stop/GC loser and cancellation repair | **PASS** | R4a's target-aware Running, `Terminated/None`, exact+routed, exact+unrouted, mismatch, Pending, and Draining partitions remain production-reachable. |
| Generation replacement predecessor fence | **PASS** | The current predecessor is examined before filtering/placement; no id or generation stamp advances before terminal+route convergence. |
| Generation two-loser/cancellation-B trigger and no-spin path | **PASS** | Self-enqueue, watch/Lagged, and relist rehydrate the old id; exact+routed removes only the tail and exact+unrouted permits exactly one later placement. |
| Service terminal/route hydration surface | **PASS** | The two exact `ServiceAllocFact` fields reuse the existing read-only route port; no Service state/View field, sixth port, or route mutator is introduced. |
| Service liveness retained-then-cleared handoff | **FAIL** | Route repair and counter clearing occur in different ServiceLifecycle evaluations, while WorkloadLifecycle may restart between them; see TRC-ARCH-003. |
| Slot-assignment ownership cut and slot-exhausted complement | **PASS** | Only post-assignment provision errors own structural cleanup; exhaustion invokes no teardown. |
| Four-stage teardown attempt and diagnostic bound | **PASS** | Netns, typed-owned host TAP, host-veth/route, and resolver stages are ordered, all attempted, absence-tolerant, and return only the first existing typed error while logging the bounded set. |
| Failed-row primary-cause preservation | **PASS** | Provision class/text remains primary; one returned cleanup error is bounded secondary detail; accepted Failed still returns success and write failure remains `Observation`. |
| Live-process retry after teardown failure | **PASS** | Retaining the in-memory slot lets an ordinary same-id Restart/Finalize action derive the same resource names and retry. |
| Process-loss recovery after any teardown-stage failure | **FAIL** | A later-stage failure after successful netns deletion leaves no netns for the existing boot observer and loses the in-memory binding on process exit; see TRC-ARCH-004. |
| Same-id old-protection-first order | **PASS** | Driver quiescence, old `stop_alloc`, and old structural teardown precede all replacement work; no interception moves before Running. |
| Same-id early failure matrix | **PASS with blocked recovery premise** | Primary errors, rules/listeners, network/slot, route, supervision, and row dispositions are pinned; process-loss convergence of a later-stage retained structural residue remains blocked by TRC-ARCH-004. |
| Prior terminal-race/cancellation model | **PASS** | Two proposals, loser/error distinction, synchronous supervision guard, in-flight redb closure ownership, and contiguous accepted tail remain unchanged. |
| Job/Service/Reclamation exit complements | **PASS** | The fence remains exactly `Job && terminal.is_some()`; terminal Service and terminal-free Platform Reclamation remain Driver-writable and route-neutral. |
| Exact public API and topology disposition | **PASS** | Only the already-sanctioned route/hydration/fact fields are additive. Network, driver, action, store, error, wire, persistence, and C4 surfaces are not widened. |
| DISTILL handoff completeness | **FAIL** | The requested production-path scenarios are present, but the asserted liveness ordering and boot-GC convergence have no valid production mechanism. |

### TRC-ARCH-003 — liveness restart can overtake counter clearing and re-kill the replacement

- **Severity:** High
- **Dimensions:** correctness, liveness, inter-reconciler ordering, effect
  isolation, testability
- **Status:** Open
- **Design evidence:**
  `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1745-1798`;
  `docs/feature/guest-stack-transparent-mtls-intercept/design/upstream-changes.md:149-157`;
  `docs/product/architecture/adr-0087-single-restart-authority-workload-lifecycle-owns-crash-and-liveness.md:105-113,159-166`
- **Production evidence:**
  `crates/overdrive-reconcilers/src/service_lifecycle.rs:472,1014-1064`;
  `crates/overdrive-reconcilers/src/workload_lifecycle.rs:155,950-1066,1468-1475`;
  `crates/overdrive-control-plane/src/reconciler_runtime.rs:1633-1676,1767-1777`;
  `crates/overdrive-control-plane/src/lib.rs:3360-3379,3482-3507`;
  `crates/overdrive-core/src/eval_broker.rs:81-103`

R4b retains the `(alloc_id, ProbeIdx(0))` threshold counter when
ServiceLifecycle emits for `Terminated/None` or exact liveness+routed. It
clears that counter only on a later exact liveness+unrouted evaluation, then
states that WorkloadLifecycle consumes the terminal afterward. That “then” is
not an ordering guarantee in the production runtime.

The counter's next View is persisted before action dispatch. A repair tick
therefore durably retains the threshold, dispatches exact/routed Stop, and
removes the route. Its self-enqueued Service evaluation is only pending for a
future broker drain. Meanwhile, the AllocStatus row that created the repair
state already woke both ServiceLifecycle and WorkloadLifecycle. The convergence
loop freezes a drained batch and executes every evaluation in it sequentially;
submissions made by the Service tick do not alter that batch. WorkloadLifecycle
can consequently hydrate after route removal in the same batch, see the exact
liveness terminal as restartable, and emit same-id `RestartAllocation`
immediately. This trajectory is reachable after a two-loser repair and is also
reachable after cancellation B.

The following Service tick sees the same allocation Running, but its persisted
counter is still at threshold and the latest liveness probe observation is the
old Fail. Under the selected Running disposition it emits another liveness
Stop before the replacement has produced any new probe result. Broker
same-key collapse prevents simultaneous Service actions; it does not order the
independent WorkloadLifecycle restart behind the later counter-clear tick.

This violates the claimed “replacement Running attempt starts clean” property
and can consume restart budget or enter a repeated stop/restart loop from one
historical liveness decision. A pure partition test or a manually ordered
Service-then-Workload trajectory will miss the race.

**Required remediation:** DESIGN must pin a production-reachable handoff in
which same-id WorkloadLifecycle restart cannot overtake retirement of the old
liveness decision, while loss/cancellation of the route-tail repair still
remains replayable. The disposition must state which existing fact owns replay,
the exact View update relative to action dispatch, and the cross-reconciler
ordering-independent trajectory. It must not leave DELIVER to add a receipt,
cross-View read, action variant, or implicit broker priority. DISTILL must drive
the real broker with Service and Workload evaluations in the same drained batch
and prove the first replacement Running state does not inherit the old probe
decision.

### TRC-ARCH-004 — netns-first total teardown loses boot discoverability for later-stage residue

- **Severity:** High
- **Dimensions:** recovery, resource ownership, process-loss safety,
  executability, operability
- **Status:** Open
- **Design evidence:**
  `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1891-1916,1997-2005,2019-2026`;
  `docs/feature/guest-stack-transparent-mtls-intercept/design/upstream-changes.md:159-176,200-205`;
  `docs/product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md:93-119,167-175`
- **Production evidence:**
  `crates/overdrive-control-plane/src/action_shim/mod.rs:1317-1333`;
  `crates/overdrive-control-plane/src/veth_provisioner.rs:2171-2211,2453-2524,2527-2577`

R6a fixes the teardown order as netns first, then a host-stranded TAP, then
host veth/route, then resolver directory. It also says any failed stage retains
the allocator binding so either an ordinary lifecycle retry or boot netns GC
derives the same names, and promises that a slot is never made available while
any teardown stage is unproven.

That promise holds only while the process-local `NetSlotAllocator` survives.
Consider a successful stage 1 followed by a stage-2 host-TAP delete error (the
same counterexample applies to stage 3 or 4). The netns name has been removed,
the later resource remains, and the slot binding stays only in RAM. If the
process exits before the ordinary retry, the binding disappears. Existing boot
recovery enumerates `ovd-ns-<slot>` netns names, builds its slot set solely from
those survivors, and GCs only that set. It does not enumerate host TAPs,
host-veths, or `/etc/netns` directories. The residue therefore supplies no boot
input, and the allocator can treat its slot as free despite an unproven cleanup
stage.

The residual resource names do encode the slot, but no accepted design or
existing observer turns those other resource classes into the boot inventory.
Saying “boot netns GC derives the same names” does not specify an executable
path once the netns—the existing derivation anchor—has already been deleted.
DELIVER would have to invent a broader boot scanner, change teardown ordering,
or introduce another recovery marker, each an architectural choice.

**Required remediation:** DESIGN must choose and pin a process-loss-safe
ownership/discovery mechanism for failures after successful netns deletion.
It must preserve the typed non-deletion rule for foreign same-name links and
the no-new-public-cleanup-framework constraint, but it must make every claimed
remaining TAP/veth/resolver stage discoverable before slot reuse. The boot
failure-cut contract must include a process exit after each successful prefix
of the four-stage teardown, not merely a live-process retry with the allocator
map still present.

### Iteration-3 architecture and API checks

| Area | Result |
|---|---|
| One-node/one-process/one-data-directory boundary | PASS |
| ObservationStore current plus bounded occurrence remains the lifecycle SSOT | PASS |
| Route remains process-local and action-shim-mutated | PASS |
| Four production Stop constructors are classified | PASS |
| Generation predecessor identity/route fence | PASS |
| Service/Workload cross-reconciler handoff | **FAIL — TRC-ARCH-003** |
| Resource-specific live-process teardown retry | PASS |
| Resource-specific process-loss discovery | **FAIL — TRC-ARCH-004** |
| Same-id old-protection-first safety order | PASS |
| Public store/action/driver/network/error/task shapes | PASS |
| `write_alloc_lifecycle` remains async with local sync blocking closure only | PASS |
| C4 component/crate topology | PASS |
| Rejected outbox/receipt/durable-route/detached-task/retry systems remain absent | PASS |
| Contract Shape and effect isolation | PASS for generation, route hydration, teardown stages, and restart order; FAIL for liveness handoff and boot discovery |

### Iteration-3 independent verification

| Check | Result |
|---|---|
| `git diff --check 00bb83482c964cc427db846cfc63de47363ed133 59e0afc7dd8007f6a88260a2e0fda80bb138266d` | **PASS** |
| Remediation scope | **PASS** — exactly eleven DESIGN/architecture documents; no code, tests, roadmap, DES log, mutation configuration, or existing review artifact changed by the reviewed commit |
| Commit parent, subject, and Feature-Id trailer | **PASS** |
| Production Stop-emitter source inventory | **PASS** — exactly the four classified constructors |
| Generation replacement source-feasibility audit | **PASS** — the gate can run before active filtering and scheduler mint/stamp using the sanctioned current-row and route inputs |
| Service liveness runtime-order audit | **FAIL** — the counter-clear evaluation is deferred while WorkloadLifecycle can restart from the same drained batch |
| Four-stage teardown source-feasibility audit | **PASS for one live process** — existing resource-specific operations can be attempted without widening the port/error shape |
| Boot recovery source audit | **FAIL** — current observe/GC enumerates only surviving workload netns slots |
| Same-id restart API/ordering audit | **PASS** — existing async action, driver, worker, provisioner, allocator, and error surfaces suffice for the specified live failure cuts |
| Prior async store/redb/cancellation regression audit | **PASS** |
| Code/tests | **NOT RUN** — documentation-only DESIGN review; source inspection and deterministic runtime/resource counterexamples establish the open findings |

### Iteration-3 finding dispositions

| Finding | Disposition |
|---|---|
| TRC-ARCH-001 — selected replay owner cannot re-emit Stop for either repair state | **CLOSED** — R4a's target-aware explicit-stop/GC predicates and route hydration remain reachable and bounded |
| TRC-ARCH-002 — replay ownership omits the generation-replacement Stop emitter | **CLOSED as the inventory/generation defect** — all four emitters are now enumerated and generation replacement is fenced through old-id terminal+route convergence; the newly specified liveness owner has the distinct ordering defect TRC-ARCH-003 |
| TRC-ARCH-003 — liveness restart can overtake counter clearing and re-kill the replacement | **OPEN** — pin an ordering-independent, replayable Service-to-Workload handoff |
| TRC-ARCH-004 — netns-first total teardown loses boot discoverability for later-stage residue | **OPEN** — pin process-loss-safe discovery/ownership before slot reuse |

### Iteration-3 verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 2 High, 0 Medium, 0 Low**.

The generation fence, complete emitter inventory, exact hydration surface,
same-id restart order, primary-error preservation, terminal race/cancellation
model, async store boundary, and rejected-mechanism constraints are approved as
designed. DELIVER remains blocked on the liveness handoff race and the missing
boot discovery path for residue left after a successful netns deletion.

---

## Iteration 4 — attempt-fence and netns-last remediation

### Assessment

The remediation fixes the ordering assumptions identified in iteration 3. A
Running evaluation now compares the persisted attempt marker and clears an old
counter in `next_view` before dispatch, while actual hydration admits only a
probe whose completion timestamp is strictly later than the current
`started_at`. This makes an actually distinct, forward-moving attempt safe
across the adverse frozen-broker batch: the Workload restart may overtake the
Service clear tick without inheriting the old threshold. The non-Running
counter and marker remain available for `Terminated/None` and exact+routed
repair, and exact-unrouted/mismatched terminal clears both without spinning.

The resource remediation likewise repairs the dependency-order failure from
iteration 3. Typed host TAP, host veth/route, and resolver directory are all
attempted before the named netns; any dependent failure withholds final netns
deletion, retains the live slot binding, and leaves the existing boot inventory
anchor. Boot adoption precedes orphan GC and allocation, foreign same-name TAPs
remain untouched, and slot release requires the four-part absence proof.

Neither proof is total under the accepted production primitives. `started_at`
is an observed wall-clock snapshot, not an attempt identity: neither the Clock
trait nor SystemClock promises uniqueness or monotonicity, and SimClock repeats
the same value until explicitly advanced. The final netns operation is not an
atomic deletion either: the accepted rtnetlink implementation detaches the
mount and only then unlinks its pathname. Equal/rolled-back clocks and a
process-loss cut inside that final operation leave the two original findings
open with narrower counterexamples.

### Iteration-4 contract validation

| Requirement | Result | Assessment |
|---|---|---|
| TRC-ARCH-001 explicit-stop/GC production replay | **PASS** | Target-aware Running, `Terminated/None`, exact+routed, exact+unrouted, mismatch, Pending, and Draining partitions remain reachable and bounded. |
| TRC-ARCH-002 complete Stop inventory and generation fence | **PASS** | All four emitters remain classified; fresh-id placement still waits for predecessor terminal+route convergence. |
| Persisted liveness marker before action dispatch | **PASS** | The exact serde-defaulted View field and reset are pinned in `next_view`, with persistence before action dispatch. |
| Strict post-start probe hydration | **PASS for strictly advancing timestamps** | Equality is rejected and a probe later than a strictly newer start belongs to the new attempt. |
| Attempt identity across equal and rolled-back clocks | **FAIL** | `started_at` may equal the prior attempt or move behind its latest probe; the marker and timestamp filter then cannot distinguish attempts. See TRC-ARCH-003. |
| Frozen-batch ordering independence | **FAIL in the accepted Clock domain** | The revised ordering works only if same-id restart produces a distinct forward start, which the Clock contract does not guarantee. |
| Liveness replay after cancellation/process loss | **PASS with blocked replacement premise** | Non-Running marker+threshold retention keeps old terminal repair reachable, but replacement isolation still fails for equal/rollback starts. |
| Liveness no-spin and clean replacement | **FAIL** | An old post-start Fail can remain eligible against an equal/rolled-back replacement and immediately stop or seed that replacement. |
| Dependency-ordered teardown stages 1–3 | **PASS** | All dependent siblings are attempted; any failure withholds netns deletion and keeps the named boot anchor. |
| Foreign same-name TAP handling | **PASS** | Typed-incompatible links are not deleted and keep the slot/netns unavailable. |
| Slot release proof | **PASS before final-delete partial cuts** | Release is gated on TAP, veth/route, resolver directory, and netns absence. |
| Process-loss recovery after stage-1/2/3 cuts | **PASS** | The netns name survives, boot enumerates it before allocation, and adopt/orphan-GC derives the same plan. |
| Final netns deletion retry/idempotence | **FAIL** | `MNT_DETACH` can succeed before `unlink` fails or the process dies, leaving a pathname that is no longer a mounted namespace and is not removable by the same operation. See TRC-ARCH-004. |
| Provision failure primary cause and unwind | **PASS with final-delete caveat** | Slot ownership cut, bounded secondary cleanup detail, accepted Failed semantics, and pre-driver/pre-route boundary remain precise; total retry is blocked only at TRC-ARCH-004. |
| Same-id old-protection-first restart | **PASS with open liveness identity defect** | Awaited driver stop, intercept teardown, structural teardown, replacement provision/start/Running/intercept/D7/release order remains correct. |
| Async and terminal semantics | **PASS** | Compound store stays async; local synchronous closure ownership, Job fence, Service/reclamation complements, and contiguous accepted tail are unchanged. |
| Rejected architecture and exact public API | **PASS** | No outbox, receipt, durable route, second store, detached task, retry framework, aggregate error, new action, or unpinned public method/type is introduced. |
| DISTILL handoff completeness | **FAIL** | Equal probe/start is covered, but equal attempt starts, clock rollback, and the detach-before-unlink process cut are absent from the claimed proof. |

### TRC-ARCH-003 — wall-clock `started_at` is not a total attempt discriminator

- **Severity:** High
- **Dimensions:** correctness, attempt identity, time semantics, liveness,
  cancellation convergence, testability
- **Status:** Open
- **Design evidence:**
  `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1767-1783,1811-1854`;
  `docs/product/architecture/adr-0087-single-restart-authority-workload-lifecycle-owns-crash-and-liveness.md:122-133,483-490`;
  `docs/product/architecture/adr-0086-reconcilers-own-hydration-overdrive-reconcilers-crate.md:328-343`
- **Production evidence:**
  `crates/overdrive-core/src/traits/clock.rs:11-21`;
  `crates/overdrive-host/src/clock.rs:17-27`;
  `crates/overdrive-sim/src/adapters/clock.rs:133-151`;
  `crates/overdrive-control-plane/src/action_shim/mod.rs:2385-2403`;
  `crates/overdrive-core/src/traits/observation_store.rs:2433-2458`

The design declares that a same-id restart's “fresh `started_at`” makes this
wall-clock value an attempt discriminator. The accepted Clock contract says
only that `unix_now()` is wall-clock duration since the epoch. It gives
monotonicity only to the separate `now()` value, which is not persisted here.
SystemClock reads `SystemTime::now()` and maps pre-epoch rollback to zero;
SimClock returns an unchanged UNIX value until the harness explicitly calls
`tick`. Therefore two real transition observations may be equal, and a later
Running transition may observe a value earlier than its predecessor or the
predecessor's last probe.

Let the old Running attempt have marker `S`, a retained threshold, and latest
liveness Fail `P` with `P > S`. The exact+routed repair removes the route and
WorkloadLifecycle restarts the same id as the design's adverse batch permits.
If the restart also writes `started_at == S`, the marker compares equal, so
the new rule performs ordinary maintenance rather than clearing the retained
threshold. `P` still passes the strict post-start filter and the replacement
is stopped from the old attempt immediately. Probe-equals-start rejection does
not cover attempt-start-equals-attempt-start.

If the wall clock rolls back to a different `S' < P`, the marker does clear the
counter, but the same old row is falsely admitted as post-start and seeds the
replacement; threshold one stops immediately and the unchanged per-evaluation
counter policy can advance larger thresholds from that row. The probe store's
strict timestamp LWW also rejects new results that do not dominate `P`, so a
rollback can preserve the old decision and suppress genuinely new probe input
until wall time catches up. Hydration and View persistence cannot recover the
lost attempt ordering because neither fact contains a structural attempt
identity.

**Required remediation:** DESIGN must pin an attempt-boundary proof that is
valid for every value allowed by the existing Clock and probe-LWW contracts,
including equal same-id starts, equality at millisecond projection, and
wall-clock rollback. If that proof requires a different identity or ordering
primitive, its exact accepted surface must be designed rather than invented by
DELIVER. DISTILL must drive the real frozen batch with equal old/new starts and
with a restarted clock behind the retained probe, proving clean replacement,
fresh-probe progress, bounded replay, and no spin.

### TRC-ARCH-004 — final netns deletion has an uncovered detach-before-unlink cut

- **Severity:** High
- **Dimensions:** crash recovery, idempotence, liveness, resource ownership,
  executability, operability
- **Status:** Open
- **Design evidence:**
  `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1970-2030`;
  `docs/product/architecture/adr-0089-tap-in-netns-provisioning-boundary-and-ch-net-attach.md:103-128`
- **Production evidence:**
  `crates/overdrive-control-plane/src/veth_provisioner.rs:2364-2421,2453-2577,2844-2871`;
  `rtnetlink-0.23.0/src/ns.rs:81-109`

Netns-last correctly keeps the discovery anchor through every failure and
process cut in stages 1–3. Its final-stage proof, however, treats successful
deletion and a returned failure as if each had an atomic postcondition. The
existing `NetworkNamespace::del` first executes `umount2(MNT_DETACH)` and then
executes `unlink` on `/run/netns/<name>`. A process can die between those
syscalls, and `unlink` can independently fail after the detach succeeded.

That cut leaves `/var/run/netns/ovd-ns-<slot>` present as the underlying
ordinary file, but it no longer names the network namespace. Boot's existing
observer enumerates by pathname and accepts any successful `metadata()` inode;
no running PID's network-namespace inode matches the ordinary file, so the
slot is planned as an orphan. Orphan GC then sees the pathname as present and
calls the same delete. Its first `umount2` now fails because the path is not a
mount, GC reports a non-benign cleanup error, and boot refuses forever. The
name therefore remains discoverable but is not the “same anchor” asserted by
the design, while deletion has not completed according to its unlink
postcondition. The retry is neither idempotent nor convergent through existing
machinery; on an appliance without an operator shell this is a permanent boot
wedge.

**Required remediation:** DESIGN must include the process-loss cut between the
final delete's existing unmount and unlink effects and pin how boot/live retry
distinguishes and converges a slot-named path that is present but no longer a
mounted namespace. It must preserve the dependent-resource absence proof,
foreign-link non-deletion, pre-allocation fail-closed behavior, and exact public
API constraint. DELIVER must not infer a new marker, scanner, trait method, or
special error swallow. DISTILL must inject process loss after detach and an
unlink failure after successful detach, then prove bounded recovery and safe
slot disposition.

### Iteration-4 architecture and API checks

| Area | Result |
|---|---|
| One-node/one-process/one-data-directory boundary | PASS |
| ObservationStore current plus occurrence remains the lifecycle SSOT | PASS |
| Route remains process-local and action-shim-mutated | PASS |
| Four Stop constructors and generation predecessor fence | PASS |
| Attempt-scoped Service/Workload handoff | **FAIL — TRC-ARCH-003** |
| View persistence before action dispatch | PASS |
| Stage-1/2/3 resource dependency and boot discovery | PASS |
| Final netns operation process-loss convergence | **FAIL — TRC-ARCH-004** |
| Provision and restart ownership/failure matrices | PASS with the two open boundary defects |
| Public store/action/driver/network/error/task shapes | PASS |
| `write_alloc_lifecycle` remains async with local sync blocking closure only | PASS |
| C4 component/crate topology | PASS |
| Rejected outbox/receipt/durable-route/detached-task/retry systems remain absent | PASS |
| Contract Shape and effect isolation | PASS for the pinned View/hydration and reordered stages; FAIL for the unsupported time-identity and final-delete postconditions |

### Iteration-4 independent verification

| Check | Result |
|---|---|
| `git diff --check 5b1adfcc818b6e4e236a30a9174f472489127560 623b9af157f14e9cc6a6d43a8504e64dd5eba9a8` | **PASS** |
| Remediation scope | **PASS** — exactly nine DESIGN/architecture documents; no code, tests, roadmap, DES log, mutation configuration, or prior review artifact changed by the reviewed commit |
| Commit parent, subject, and Feature-Id trailer | **PASS** |
| Prior TRC-ARCH-001/002 regression audit | **PASS** |
| Liveness View/hydration source-feasibility audit | **PASS only for distinct forward timestamps** |
| Clock-domain audit | **FAIL** — `unix_now` has no uniqueness/monotonicity contract and production/simulation implementations permit equality or rollback |
| Probe LWW audit | **FAIL for rollback recovery premise** — equal/older new-attempt results cannot displace the old latest row |
| Netns dependency-order source-feasibility audit | **PASS** — existing typed per-resource operations can preserve the named anchor through stages 1–3 without public surface expansion |
| Final netns delete implementation audit | **FAIL** — detach and unlink are separately fallible/process-cuttable, and retry cannot remove a detached ordinary mountpoint file |
| Provision unwind, same-id restart, terminal, async, and rejected-mechanism regression audit | **PASS** |
| Code/tests | **NOT RUN** — documentation-only DESIGN review; source and dependency inspection establish both deterministic counterexamples |

### Iteration-4 finding dispositions

| Finding | Disposition |
|---|---|
| TRC-ARCH-001 — selected replay owner cannot re-emit Stop for either repair state | **CLOSED** — target-aware explicit-stop/GC predicates and route hydration remain reachable and bounded |
| TRC-ARCH-002 — replay ownership omits the generation-replacement Stop emitter | **CLOSED** — all four emitters remain enumerated and generation placement remains fenced through predecessor terminal+route convergence |
| TRC-ARCH-003 — liveness restart can overtake counter clearing and re-kill the replacement | **OPEN** — the marker removes the broker-order premise only for a distinct forward `started_at`; equal/rolled-back accepted Clock values still inherit or re-admit the old attempt decision |
| TRC-ARCH-004 — netns-first total teardown loses boot discoverability for later-stage residue | **OPEN** — netns-last preserves discovery through dependent stages, but the final detach-before-unlink cut leaves a false anchor that existing boot GC cannot converge |

### Iteration-4 verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 2 High, 0 Medium, 0 Low**.

The explicit-stop/GC replay owner, complete emitter inventory, generation
fence, View-before-dispatch sequencing, strict-forward probe filter,
dependency-ordered teardown, provision/restart unwind, terminal semantics,
async boundary, and rejected-mechanism constraints are approved as designed.
DELIVER remains blocked on total attempt identity under the accepted wall-clock
domain and retryable final netns deletion across the detach-before-unlink cut.

---

## Iteration 5 — logical-attempt and detached-path remediation

### Assessment

The logical-attempt model closes the semantic counterexample from iteration 4.
`AllocStatusRow.updated_at` is minted by the existing per-key
`LogicalTimestamp::dominating` boundary, so a same-id Running replacement
strictly dominates its terminal predecessor independently of wall clock. The
design carries that accepted identity through the changed existing Running
hook and ProbeRunner methods, stores it on the latest probe row, orders probe
replacement by attempt before diagnostic wall time, filters liveness hydration
by exact identity, and clears the old counter/marker in persisted `next_view`
before action dispatch. Old tasks can finish late without crossing attempts,
and the frozen Service-tail/Workload-restart orders converge without inheriting
the old threshold.

TRC-ARCH-004 is also closed as a runtime/resource design. The final path is no
longer treated as an atomic netns delete. Safe `statfs` distinguishes an NSFS
mount from its detached underlying placeholder; mounted deletion re-observes
after error, detached state performs unlink-only, and boot enumerates both but
owner-correlates only mounted namespaces. The deterministic pathname therefore
remains discovery input across every detach/unlink cut, while dependent
TAP/veth/route/resolver cleanup precedes final path convergence and slot release
requires path absence.

The liveness design is not implementable with its stated rkyv compatibility
contract. Appending the exact V2 payload grows the archived enum's inline
region. The old V1 archive is consequently too short for the new enum and
cannot reach `From<V1> for V2`; preserving its bytes and decoding it through
the new envelope are mutually impossible with the specified derive shape.
This is a persistence-boundary design defect, not mechanical fixture fallout.

### Iteration-5 contract validation

| Requirement | Result | Assessment |
|---|---|---|
| TRC-ARCH-001 explicit-stop/GC replay | **PASS** | Target-aware terminal/route partitions and fresh runtime triggers remain unchanged and bounded. |
| TRC-ARCH-002 complete Stop inventory and generation fence | **PASS** | Four emitters remain classified and fresh-id placement still waits for predecessor terminal+route convergence. |
| Authoritative attempt minting | **PASS** | The accepted Running row's existing `updated_at` is minted from the current same-key predecessor with `LogicalTimestamp::dominating`. |
| Same-id uniqueness without wall clock | **PASS** | The replacement Running identity dominates the intervening terminal identity even under equal starts, millisecond collision, clock rollback, and process-local tick reset. |
| Exact API propagation | **PASS** | Only existing `Driver::on_alloc_running`, `ProbeRunner::start_alloc`, and `ProbeRunner::probe_once_and_record` signatures change; the accepted row identity flows through each production task. |
| Probe latest-row key/cardinality | **PASS** | `(alloc_id, role, probe_idx)` and one-row cardinality remain unchanged; no history, receipt, or second key axis is introduced. |
| Attempt-first LWW semantics | **PASS** | Newer `Some` beats older/legacy regardless of wall time; equal attempt preserves strict wall-time LWW; older/legacy cannot overwrite newer attribution. |
| V1-to-V2 semantic projection | **PASS in the type model** | `From<V1>` honestly yields `alloc_attempt: None`; exact-attempt liveness hydration rejects legacy/unattributed rows. |
| V1 archived-byte compatibility | **FAIL** | The added V2 variant grows the enum inline region from 72 to 96 bytes, so the fixed historical V1 archive does not decode through the proposed enum. See TRC-ARCH-003. |
| V2 schema fixture/version matrix | **FAIL as specified** | A V2 fixture can be added, but the simultaneous prohibition on V1 fixture regeneration is incompatible with the exact rkyv derive layout. |
| Exact-attempt hydration and counter reset | **PASS** | Only matching attributed liveness results reach the fact; a changed logical marker resets old counter state in the View before dispatch. |
| Frozen-batch Service/Workload ordering | **PASS** | Both broker orders reject the old attempt and require a new-attempt result before another Stop; non-Running terminal repair remains replayable. |
| Stale/late probe rejection | **PASS across attempts** | A delayed old task cannot overwrite a newer attempt and does not pass exact-attempt hydration; the first new-attempt probe wins despite equal/rolled-back wall time. |
| Liveness no-spin and progress | **PASS for the remediated trajectory** | New Running with no exact-attempt row is action-free; fresh Fail starts at one, while exact-unrouted/mismatch clears retained repair state. |
| Mounted/detached/absent observation | **PASS** | The private canonical-path observer uses the existing safe `nix::sys::statfs` surface and maps non-absence observation errors to the existing typed error. |
| Detach/unlink live retry | **PASS** | Error re-observation distinguishes still-mounted, already-absent, and detached-placeholder outcomes; only detached state retries unlink. |
| Boot GC reachability | **PASS** | Both present path states are enumerated before allocation; mounted only is adoptable and detached always enters ordinary orphan GC. |
| Structural dependency and slot release | **PASS** | All three dependent siblings precede final path convergence, foreign TAPs remain untouched, and release requires dependents plus path `Absent`. |
| Provision/restart/terminal/async regressions | **PASS** | Primary error preservation, old-protection-first restart, Job/Service/reclamation complements, async compound store, and contiguous accepted tail remain intact. |
| Rejected architecture | **PASS** | No outbox, cleanup token, durable route, second store, quarantine, multi-instance protocol, detached completion, generic cleanup framework, or retry subsystem is introduced. |

### TRC-ARCH-003 — the exact ProbeResultRow V2 append cannot preserve or decode the fixed V1 archive

- **Severity:** High
- **Dimensions:** implementation feasibility, persistence compatibility,
  schema integrity, testability, exact API contract
- **Status:** Open
- **Design evidence:**
  `docs/feature/guest-stack-transparent-mtls-intercept/feature-delta.md:1792-1838,1961-1991,2337-2340`;
  `docs/product/architecture/adr-0048-rkyv-versioned-envelope.md:487-549`;
  `docs/product/architecture/adr-0054-probe-runner-subsystem.md:378-405,438-482`
- **Production/rule evidence:**
  `crates/overdrive-core/src/observation/probe_result_row.rs:164-226`;
  `crates/overdrive-core/tests/schema_evolution/probe_result_row.rs:32-100`;
  `crates/overdrive-core/tests/schema_evolution/alloc_status_row.rs:14-23,67-93,146-198`;
  `.claude/rules/development.md:1479-1540`

The design requires all of the following at once: append
`V2(ProbeResultRowV2)` to the current rkyv-derived enum; put the exact optional
`LogicalTimestamp` field on V2; leave the V1 fixture bytes untouched; and prove
those historical bytes decode through the V1-to-V2 conversion. The last two
requirements do not hold for rkyv's archived enum layout.

rkyv gives the archived enum an inline region sized for its largest variant.
This repository already records that mechanism in the evolved
`AllocStatusRowEnvelope`: each larger variant padded every historical variant,
shifted the discriminant offset, and required an explicitly authorised
greenfield regeneration of V1/V2 bytes. That production precedent directly
contradicts ADR-0048's older general claim that appending a variant cannot
change an existing variant's archived layout.

An independent throwaway archive probe used the exact current
`ProbeResultRowV1` payload and exact proposed V2 tail field:

| Archive shape | Result |
|---|---|
| one-variant `V1(current canonical V1)` | 72 bytes, byte-identical to the checked-in `FIXTURE_V1` |
| two-variant enum, archiving the same V1 payload | 96 bytes |
| two-variant enum, archiving proposed V2 | 96 bytes |
| decode the checked-in 72-byte V1 archive as the two-variant enum | rejected |

The failure occurs before `into_latest()` can match `V1` and call
`From<ProbeResultRowV1>`. Updating `known_discriminants` or adding a V2 fixture
cannot change the missing inline bytes. The local observation adapter may later
self-heal an undecodable probe row when a fresh probe write arrives, but that
is lossy replacement of a malformed predecessor, not V1 compatibility, and it
does not satisfy the mandated fixed-fixture proof.

**Required remediation:** DESIGN must reconcile the exact persisted
representation with rkyv's measured layout and explicitly satisfy the selected
V1 compatibility policy. It may not leave DELIVER to regenerate the immutable
fixture contrary to the design, claim that the existing bytes migrate through
`From<V1>`, or silently treat decode-and-replace as version compatibility. Any
changed persistence/API shape must be pinned at DESIGN before implementation;
the logical-attempt semantics themselves do not need to return to wall clock.

### Iteration-5 architecture and API checks

| Area | Result |
|---|---|
| One-node/one-process/one-data-directory boundary | PASS |
| ObservationStore remains the lifecycle/probe durable boundary | PASS |
| Existing Running logical identity is reused | PASS |
| Logical attempt propagation and exact hydration | PASS |
| Probe rkyv V1 compatibility | **FAIL — TRC-ARCH-003** |
| View persistence before action dispatch | PASS |
| Four Stop constructors and generation predecessor fence | PASS |
| Mounted/detached/absent netns state machine | PASS |
| Boot adopt-before-GC-before-allocation order | PASS |
| Final netns operation process-loss convergence | PASS |
| Provision and restart ownership/failure matrices | PASS |
| Public store/action/driver/network/error/task shapes | PASS except for the infeasible archive-compatibility claim on the exact sanctioned V2 shape |
| `write_alloc_lifecycle` remains async with local sync blocking closure only | PASS |
| C4 component/crate topology | PASS |
| Rejected recovery/persistence/task mechanisms remain absent | PASS |

### Iteration-5 independent verification

| Check | Result |
|---|---|
| `git diff --check 2dd7ba82d786a3810f2c8804888a79eb8ef0f566 91e819f0655d113c9b6be2c6e520394a280ff221` | **PASS** |
| Remediation scope | **PASS** — exactly thirteen DESIGN/architecture documents; no code, tests, roadmap, DES log, mutation configuration, or review artifact changed by the reviewed commit |
| Commit parent, subject, and Feature-Id trailer | **PASS** |
| LogicalTimestamp source audit | **PASS** — per-key dominating minting is durable and wall-clock independent under the accepted single-writer boundary |
| Running-hook/ProbeRunner API audit | **PASS** — accepted identity can flow through the three exact changed existing signatures without a new owner/port/method |
| Attempt-first local/sim LWW feasibility | **PASS** — both existing adapters have one bounded comparator site at the unchanged composite key |
| Exact-attempt hydration/View audit | **PASS** |
| Exact rkyv archive experiment | **FAIL for the design claim** — current V1 72 bytes; proposed enum's V1 96 bytes; old V1 cannot decode as the proposed enum |
| Existing multi-version precedent audit | **FAIL for the design claim** — AllocStatus documents the same max-variant padding and historical fixture regeneration |
| Netns statfs substrate/API audit | **PASS** — nix 0.30 exposes safe `statfs`, `FsType`, and `NSFS_MAGIC` behind the bounded existing dependency feature |
| Netns retry/boot path source audit | **PASS** — current enumeration, inode correlation, orphan GC, and resource-specific teardown can implement the three-state disposition privately |
| Prior TRC-ARCH-001/002, provision unwind, same-id restart, terminal, async, and rejected-mechanism regression audit | **PASS** |
| Code/tests | **NOT RUN** — documentation-only DESIGN review; the isolated archive probe and source/dependency inspection establish feasibility |

### Iteration-5 finding dispositions

| Finding | Disposition |
|---|---|
| TRC-ARCH-001 — selected replay owner cannot re-emit Stop for either repair state | **CLOSED** — target-aware explicit-stop/GC predicates and triggers remain reachable and bounded |
| TRC-ARCH-002 — replay ownership omits the generation-replacement Stop emitter | **CLOSED** — all four emitters remain classified and generation placement remains fenced |
| TRC-ARCH-003 — liveness replacement inherits an old attempt decision | **OPEN at the persistence boundary** — logical attempt minting, propagation, ordering, hydration, and counters are sound, but the exact V2 envelope cannot preserve/decode the mandated fixed V1 bytes |
| TRC-ARCH-004 — final netns deletion is not retryable across detach-before-unlink | **CLOSED** — mounted/detached/absent observation, unlink-only retry, boot GC, dependency ordering, and path-absence release cover every specified cut |

### Iteration-5 verdict

**NEEDS_REVISION**

Open findings: **0 Critical, 1 High, 0 Medium, 0 Low**.

The wall-clock-independent attempt identity, production propagation, exact
hydration, counter handoff, frozen-batch ordering, netns detach/unlink recovery,
boot reachability, slot proof, prior replay/generation closures, unwind,
terminal semantics, async boundary, and rejected-mechanism constraints are
approved as designed. DELIVER remains blocked solely on the impossible
simultaneous ProbeResultRow V2 shape and fixed-V1 archive compatibility claim.
