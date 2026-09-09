# ADR-0099: Require accepted Running publication before completing restart release

## Status

**Accepted**, 2026-09-07. Bounded DESIGN ruling for `service-kind-vm-workloads`
step `02-03`, E10. The independent [iteration-1 review](../../feature/service-kind-vm-workloads/design/review-adr-0099.md)
supported the mechanism and requested only the R099-1 Lifecycle Gate Ownership
documentation clarification. That clarification is recorded below. The user
explicitly directed **“no re-review”** and waived independent re-review of this
documentation correction; acceptance follows that user direction, not a reviewer
approval of the revised text. The original CHANGES_REQUESTED review is preserved.
Acceptance does not claim implementation or passing post-implementation evidence.

This corrects the existing successful `RestartAllocation` publication boundary.
It does not replace the observation store's LWW contract, alter restart policy,
or authorize a persistence/recovery architecture. ADR-0098 is not a prerequisite.

## Context

Seed `257203` in
`crates/overdrive-sim/tests/allocation_restart_write_spike.rs` reproduces the
current production `run_convergence_tick` / registered reconcilers / action shim /
exit observer composition. Service startup failure publishes Failed(35).
WorkloadLifecycle authorizes a same-allocation-ID restart. During the shim's
awaited stop-half, the old Exec attempt's exit observer publishes Terminated(36).
The replacement driver starts; the shim proposes Running(36) using its prior
Failed(35) snapshot. The store correctly returns `Ok(None)`. The shim currently
treats that as sufficient to install interception and release Running hooks.
The driver can therefore be Running while the allocation-current row is
Terminated and observation-driven operator stop does not select it.

The accompanying [ruling](../../feature/service-kind-vm-workloads/design/allocation-restart-write-ruling.md)
records primary-source Kubernetes/Nomad research, the corrected owner trace,
the failing safety invariant and passing no-contender control, native evidence,
and limitations. A rejected compound write is not an accepted Running transition.

## Decision

### D1 — Preserve the existing public contract exactly

No new method, trait, type, field, variant, parameter, action, schema, route,
configuration, or dependency is introduced. In particular, retain:

```rust
async fn ObservationStore::write_alloc_lifecycle(
    &self,
    current: AllocStatusRow,
    source: TransitionSource,
) -> Result<Option<AllocLifecycleOccurrenceRow>, ObservationStoreError>;
```

`Some` means accepted current row plus occurrence; `None` means no accepted
transition. `None` must not be recast as a fabricated I/O failure. Existing
`Action::RestartAllocation { alloc_id, spec, kind }`, `Driver` hooks, and shim
dispatch signatures stay unchanged. Implement within the existing restart arm;
no new public retry abstraction is permitted.

### D2 — Bound successful restart publication to two proposals

Retain the existing restart decision, old-attempt stop, awaited old network
cleanup, replacement provision, identity acquisition, and driver start order.
This decision applies at that arm's successful-driver-start / Running publication
branch, not to unrelated observation writers or failed-start classification.

1. Propose the current Running row through `write_alloc_lifecycle` as today.
2. On `Ok(Some(occurrence))`, continue the existing routing-index insertion,
   mTLS installation, exit-emission release, Running hook, and occurrence emission.
   Emit only the returned accepted occurrence.
3. On the first `Ok(None)`, fresh-read the current row for the same allocation.
   Rebuild the Running proposal using the existing `build_alloc_status_row` and
   `LogicalTimestamp::dominating(tick.tick, node_id, Some(&fresh.updated_at))`.
   Preserve the replacement driver's result, spec-derived workload address, and
   already-captured replacement start time. Derive `last_terminated` and
   `restart_count` from this **fresh accepted predecessor**, never from the
   rejected proposal. Re-propose exactly once, without restarting the driver,
   reprovisioning its network, reissuing identity, or incrementing the
   WorkloadLifecycle decision budget again.
4. Only acceptance of that second proposal permits the existing Running-confirmed
   effects. The successful reproduced trajectory is therefore
   Running(34) → Failed(35) → Terminated(36) → Running(37); one accepted replacement
   Running occurrence and one durable restart increment. ADR-0078's depth-one
   snapshot records the actual predecessor Terminated(36); the earlier startup
   terminal remains in the existing bounded occurrence history. Do not invent a
   merged terminal history or rewrite the old exit's cause.
5. On a second `Ok(None)`, the restart is refused at its publication boundary:
   execute the existing restart failed-publication unwind in the same call frame
   (await driver stop, release supervision, await mTLS stop when applicable,
   teardown the newly assigned netns, release its binding only after teardown).
   Do not install interception, invoke Running hooks, or synthesize an occurrence.
   Once this existing unwind completes, return `Ok(())` as a completed no-op
   disposition, not as proof of an accepted Running transition. The current
   store winner remains authoritative. There is no third proposal, detached
   continuation, new error variant, or new observation written just to record
   this refusal.
6. Read/write errors retain the existing typed error and failed-publication
   unwind behavior. A fresh read cannot legitimately return no current row:
   the arm already read it and ObservationStore exposes no current-row delete.
   Preserve that existing invariant rather than adding a speculative missing-row
   recovery protocol. Existing cleanup errors retain their existing propagation.

The two-proposal bound reuses the current StopAllocation contention discipline;
it is not a general eventual-publication guarantee. The source of the retry
proposal remains the selected driver's existing `TransitionSource`; no writer
identity or LWW comparator changes. Existing Job terminal-attempt guards remain
before restart effects and are not bypassed by this change.

### Lifecycle Gate Ownership

The existing acceptance/effect gate is **enforced**, not moved or redefined.
The existing state-ownership matrix follows the architecture brief's
`Service-kind VM workloads / Lifecycle Gate Ownership` and ADRs 0096–0097:

| Signal or state | Owning component | Promise it makes | Inputs that may gate it | States it must not gate |
| --- | --- | --- | --- | --- |
| Allocation `Running` | Action shim + selected driver; VM uses Beacon | Driver start succeeded and the Running observation committed | Selected driver's start result and compound-write acceptance; probe success neither gates nor revokes it | Running alone must not establish Service Stable or backend eligibility, or authorize a restart |
| Service `Stable` | `ServiceLifecycle` startup branch | Existing startup contract passed or was explicitly disabled | Existing startup observations, threshold/deadline, or explicit startup opt-out | Must not define driver-start success or authorize a restart |
| `Backend.healthy` | `ServiceLifecycle` readiness branch with terminal startup veto | A non-terminal allocation meets declared readiness; terminal startup failure is ineligible | Existing readiness threshold/default and terminal startup decision | Must not redefine allocation Running, Service Stable, or restart authority |
| Liveness termination | `ServiceLifecycle` liveness detector | Existing liveness threshold emits the existing liveness `StopAllocation` | Existing liveness observations and threshold | Must not itself restart the allocation or reinterpret startup failure as driver-start failure |
| Restart decision/action | `WorkloadLifecycle` alone | Existing observed lifecycle, intent, and unified budget authorize a replacement allocation attempt under the same allocation ID | Existing restartability predicates, budget/backoff, and Job terminal-attempt guard | Must not redefine Service Stable or backend eligibility; publication retry must not spend a second restart decision |

#### Gate G-99 — accepted restart Running publication permits post-start effects

- **Existing evidence:** the accepted compound-write contract returns `Some` only
  for an accepted current row plus occurrence (ObservationStore trait,
  `crates/overdrive-core/src/traits/observation_store.rs:2062`); ADR-0077 owns
  prior-derived stamps and ADR-0078 owns fresh-predecessor crash facts. The
  production path is `spawn_convergence_loop` (`crates/overdrive-control-plane/src/lib.rs:3332`)
  → awaited `run_convergence_tick` (`src/reconciler_runtime.rs:1527`)
  → registered `WorkloadLifecycle` restart decision
  (`crates/overdrive-reconcilers/src/workload_lifecycle.rs:919`, action builder `:1426`)
  → serial shim `RestartAllocation` (`crates/overdrive-control-plane/src/action_shim/mod.rs:2259`)
  → prior-row read, old-driver stop, old network cleanup, replacement provisioning
  and driver start → compound write (`:2612`) → current unconditional-on-`Ok`
  Running effects (`:2660–2705`). The seed and full owner trace in the ruling
  prove the mismatch: `Ok(None)` currently passes this last boundary.
- **Owner:** the **action shim** owns this post-start effect gate.
  ObservationStore is authoritative about write acceptance, not about releasing
  driver hooks. `WorkloadLifecycle` remains the sole restart-decision owner.
- **Promise:** a successful replacement driver start **and** a returned
  `Some(occurrence)` for that replacement Running proposal permit the existing
  Running-confirmed effects. Driver start, a diagnostic log, or `Ok(None)` alone
  establishes no such permission. Acceptance is a committed transition, not a
  guarantee that no subsequent observation can supersede it.
- **Affected state:** the result controlled here is **release of the replacement's
  Running-confirmed effects**. It requires an accepted allocation-current Running
  observation; it does not redefine `AllocState::Running`, Service lifecycle,
  or the operator deploy result. A refused publication does not retrospectively
  turn the successful driver start into a driver-start failure.
- **Failure projection:** first `Ok(None)` (including an equal/duplicate candidate)
  takes D2's one fresh-read/rebuild/re-proposal; second `Ok(None)` takes the existing
  awaited failed-publication unwind and returns the existing completed-no-op
  `Ok(())`, without Running hooks or a fabricated occurrence. Read/write errors
  retain the existing typed error/unwind path; cleanup errors retain their
  existing propagation. Malformed storage remains the existing adapter decode
  contract: an exposed typed error takes that error path; a fresh-read no-row
  result is D2's current-row invariant boundary, not a new recovery policy.
  There is no new timeout result. Cancellation remains with the current owners:
  the convergence loop awaits dispatch and checks shutdown between completed
  ticks (`lib.rs:3377–3403`); the exit observer selects cancellation before
  receiving a new event and awaits its consumed event's existing retry procedure
  (`src/worker/exit_observer.rs:198–206`, `:342–375`). No detached continuation,
  durable pending proposal, or crash/replay guarantee is introduced.
- **Explicitly unaffected:** Service Stable, startup/readiness, backend eligibility,
  liveness `StopAllocation`, Job terminal-attempt fences, probe-result counting,
  non-conflicting restart, failed-driver-start classification, operator stop
  policy, operator deploy terminal projection, and all other observation writers.
  Existing network provisioner teardown and in-memory allocation slot binding
  ownership remain unchanged. A persisted workload address does not become the
  owner of this gate; ADR-0098's separate cleanup fallback is unchanged.
- **Ordering:** keep D2's pre-publication order; acceptance precedes routing-index
  insertion, intercept installation, exit-emission release, Running hook, and
  returned-occurrence emission, preserving their existing order and existing
  intercept-install failure handling. A fresh read and second proposal occur in
  this same awaited action frame, before those effects. **No enforced action/tick
  timeout exists on this path:** `spawn_convergence_loop` supplies
  `deadline = now + cadence` (`lib.rs:3356`), and the runtime carries it in
  `TickContext` (`src/reconciler_runtime.rs:1606`), but runtime dispatch awaits the
  shim (`:1725–1748`) and this restart arm neither reads that deadline nor wraps
  its work in a timeout. Thus the bound is two proposals, not an elapsed-time
  guarantee. No timeout budget is created or multiplied; existing driver-specific
  budgets (for example Exec stop grace, `crates/overdrive-worker/src/driver.rs:706`)
  remain independent and unchanged.
- **Counterexample:** seed `257203`'s no-contender control must retain accepted
  Running(36) and one restart increment. Its old-exit contender must not release
  replacement hooks after rejected Running(36); one fresh proposal may instead
  accept Running(37). Conversely, a driver that starts while its Service startup
  probe fails must retain truthful Running history while ServiceLifecycle owns
  startup failure and backend withdrawal. Moving G-99 to driver start, Service
  Stable, probe success, or a global publisher would change those promises rather
  than correct this acceptance boundary.
- **Evidence lane:** the existing seed-257203 `overdrive-sim` safety invariant and
  no-contender control exercise production convergence, registered reconcilers,
  shim, and exit observer. In-process integration tests through existing driven
  ports establish rejection/error unwind, fresh-predecessor facts, and unchanged
  lifecycle owners. Native default-feature E10 independently drives the built
  product for all eight HTTP/driver cells and owned host-network cleanup deltas.
  Sim does not prove redb or kernel effects; E10 must not absorb private lifecycle
  assertions. The obligations below specify required implementation evidence,
  not new tests claimed to have passed during this documentation correction.

#### Gate G-99 boundary-scenario obligations

| Scenario | Executable obligation and boundary |
| --- | --- |
| Available | Given an authorized replacement whose first Running proposal is accepted, when the production restart arm completes, then the existing effects follow acceptance exactly once, with one replacement occurrence and restart increment. Retain the seeded no-contender control and existing in-process effect assertions. |
| Unavailable / timeout | Given first rejection, when the existing arm refreshes and re-proposes, then second acceptance releases effects once; second rejection instead awaits existing unwind, releases no Running hook/occurrence, and makes no third proposal. Cover existing typed read/write and cleanup failures through existing driven ports. A hung-port timeout scenario is **not applicable** to this gate: no action/tick timeout is enforced here, so do not assert a new deadline or typed timeout. |
| Unrelated state | Given successful driver start with a failed Service startup probe, then Running history is not erased or renamed, while existing ServiceLifecycle startup failure/backend withdrawal remains authoritative. Preserve existing Stable, readiness, liveness-to-StopAllocation, Job terminal-attempt, and failed-driver-start regressions at their in-process owner boundaries. |
| Late success | Given a candidate whose LWW stamp does not dominate the current winner, then the store rejects it without changing current row or occurrence. In the seeded authorized Service restart, a **fresh** proposal built after observing the old attempt's Terminated(36) may validly accept Running(37); this is not acceptance of the stale Running(36) candidate. Assert fresh-predecessor `last_terminated`, one restart increment, and one replacement occurrence. Retain existing Job terminal-attempt fences. No new attempt fence is introduced: an old-attempt exit processed late can itself fresh-stamp an observation; this ADR does not promise that every late old-attempt event is rejected, or make a Service terminal observation immutable across authorized restart. |
| Disconnect / reconnect | A remote reconnect gate is **not applicable**: this local allocation-current write uses LocalObservationStore/redb, not a remote session prerequisite. Port errors retain the typed failure/unwind obligation above. The production convergence owner awaits the action; the exit observer retains its existing cancellation/retry boundaries. No new reconnect-triggered replay, pending-publication recovery, or state revocation is claimed; existing re-enqueue/reconciliation policy remains authoritative (`src/reconciler_runtime.rs:1767–1783`). |
| Feature disabled | A product switch is **not applicable**: ADR-0099 adds none. Given an initial StartAllocation, failed-driver-start branch, or another observation writer, this amendment is not activated and existing behavior remains unchanged. Preserve those in-process controls. Absence of optional mTLS skips its existing intercept work, not G-99's acceptance requirement; the existing seeded fixture without mTLS must still enforce the gate. |

## Alternatives

- **Unwind on the first rejection:** closes the safety gap but discards an
  authorized replacement because the old attempt's one exit observation won.
  One bounded fresh proposal is sufficient for the demonstrated contention.
- **Only read later:** narrows the race but does not enforce the already-returned
  acceptance verdict. It cannot justify Running hooks after an actual rejection.
- **Atomic store-assigned versions / serialized status publisher:** materially
  changes API, consistency or owner boundaries. Not necessary for this bounded
  witness; Kubernetes/Nomad reporting designs do not authorize copying them.
- **Recover an old slot from a persisted address:** ADR-0098 does not gate the
  replacement publication or intercept installation; it is not this defect's remedy.

## Consequences and verification

The no-conflict path adds no I/O. A rejection adds at most one current-row read
and one compound proposal. Repeated rejection can abandon the replacement after
cleanup; no eventual retry guarantee is added. This deliberately bounds work and
keeps existing convergence policy authoritative.

Required implementation evidence:

1. Keep seed `257203` and the no-contender control through the existing production
   convergence entry point. The reproduced contention must reach accepted
   Running(37) with one replacement occurrence and one restart increment before
   Running hooks. No fabricated allocation row, new Sim seam, or mutation test.
2. Using existing driven ports, verify first acceptance, one rejection followed
   by acceptance, and bounded exhausted rejection. On exhausted rejection, exact
   replacement ownership is cleaned, no Running hook/occurrence is released, and
   there is no third proposal. Preserve the current typed I/O failure behavior.
3. Pin the fresh-predecessor crash-fact rule and preserve existing Job terminal
   guards and driver-start failure classifications. Re-proposing a row must not
   count as a second restart decision or second driver start.
4. Native default-feature E10 remains independent black-box evidence: all eight
   checked-in driver/status journeys, no redirect following, no failure-body
   sentinel, and zero owned cleanup deltas. Do not add internal counters or
   lifecycle assertions to its runner. Re-capture through the canonical shared
   metal lease; the historical failure evidence is not a new passing capture.
5. ADR-0098 removal is not authorized here. If separately directed, verify the
   bounded correction without that fallback; retain missing-binding hypotheses
   as unproven unless independently reproduced through their real owner path.

This is not a mandate to generalize lifecycle publication across the product.
The ruling and its seed bound the defect. Any new persistence, ownership,
recovery, allocation identity, or broker mechanism requires separate user scope
approval and independent design review.
