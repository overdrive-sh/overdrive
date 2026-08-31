# Terminal-Race Cancellation DESIGN Remediation Review

## Metadata

| Field | Value |
|---|---|
| Feature | `guest-stack-transparent-mtls-intercept` |
| Review type | DESIGN remediation review |
| Review iteration | 1 |
| Reviewed commit | `d2071336b71c5d9fd1328041e622fd944bef50c5` |
| Parent | `d5ce795b87b2c4da1abbf3ff13babc48b3e2e104` |
| Range | `d5ce795b87b2c4da1abbf3ff13babc48b3e2e104..d2071336b71c5d9fd1328041e622fd944bef50c5` |
| Subject | `docs(design): bound terminal lifecycle authorship` |
| Review basis | Accepted recovery DESIGN plus `deliver/mutation/terminal-race-remediation-review.md` findings TRR-01 through TRR-04 |
| Verdict | **NEEDS_REVISION** |

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
