# ADR-0335 — A restart generation is handled when its successor is observed

## Status

**Accepted** 2026-11-18 (decision SD-6 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-18, option A; item 4 by the user ruling of 2026-11-24, RB-13). The
finished-Job rule (Decision, item 3) is **Accepted** 2026-11-19 by user ruling (H1, option A); its definition of a finished Job row, which
includes an operator-stopped row whichever writer recorded the stop, is **Accepted** 2026-11-28 by user ruling (H3). ADR-0073 is implemented, so its amendment
section links here; this ADR is not implemented and is stated in the present tense. Two neighbouring rules are their own decisions: the
recording of a failure before the driver runs ([ADR-0337](adr-0337-a-launch-failure-before-the-driver-runs-is-a-recorded-charged-failed-row.md))
and the handling of an admission refusal ([ADR-0338](adr-0338-an-admission-refusal-by-the-guest-network-pool-writes-no-row-and-charges-nothing.md)).

## Context

`WorkloadLifecycle` places a restart's or a resume's replacement allocation, and it must decide on every evaluation whether the generation the
operator committed (`workloads/<id>/generation`) has been handled. ADR-0073 answered with a marker the reconciler stamped on the tick that
emitted the placement, and the runtime persists the View before it dispatches. A launch can end without writing an allocation row: it is
superseded by a stop, a restart or node shutdown (ADR-0334), or it dies with the control plane (reachable on `main` today). On `main` the next
evaluation then read the generation as handled beside the old operator-stopped or replaced row, the veto and the restartable filter emitted nothing,
and the workload stayed down while intent said run. The marker records what the reconciler emitted, not a fact about the resource it
manages (`.claude/rules/reconcilers.md` § "A View decides when to act, never whether an effect is in place").

Deriving "handled" from observation has one more consequence this decision must own. The action shim treats a Job row that carries a terminal
claim as immutable (`allocation_attempt_transition` returns `NoChange`), so a `RestartAllocation` for such a row ends with no successor and no
row. A generation that stays pending against that row is never handled: every evaluation would issue an id, grow the View and persist it,
invisibly. On `main` the stamp-on-emit marker hides this by marking the generation handled at emit, and the operator-stopped Job simply stays
down.

## Decision

**The reconciler records what it issued, never that a launch happened. A restart's generation is handled once the allocation issued for it
has an allocation row that ran, and a generation that the shim cannot honour is not pending.**

1. **The View records issuance.** For each allocation id the reconciler issues, the View holds the generation that id serves. Nothing is
   stamped when the placement is emitted.
2. **"Restart pending" is derived from observation.** A generation is pending while the newest allocation that ran (the highest id that has a
   row that is not an abandoned launch, ADR-0355) serves a generation below the desired one. With no such row there is nothing to replace.
3. **A finished Job is never replaced.** A Job row is finished when it carries a terminal claim (it completed or failed), or when the
   operator stopped it, whichever writer recorded the stop. An operator stop is recorded two ways: the stop path writes
   `terminal: Some(Stopped { by: Operator })`, and the exit observer writes a `Terminated` row of reason `Stopped { by: Operator }` with
   `terminal: None`. When both of the stop path's write proposals lose under contention, the exit observer's row stands as the authoritative
   row, and the reconciler emits a stop only for a `Running` row, so nothing would ever add the terminal. The existing classifier
   `is_operator_stopped` recognises both shapes. When the newest row that ran is a finished Job row, the generation is not pending. `workload restart` of it advances the generation and nothing is
   issued: the same outcome as today, now decided by the reconciler, which emits nothing, rather than by a shim fence that returns after the
   id was issued. The rule matches the action shim's immutability rule for a Job row, so the two cannot disagree: both use the one definition
   above. Without the operator-stopped clause, `workload restart` (which bumps the generation and clears the stop sentinel) of a Job whose
   stop left only the exit observer's terminal-less row would give the stopped Job a new allocation. `workload restart` of a running Job therefore ends it and leaves it down (the stop writes the terminal),
   as on `main`. This is a scope limitation tracked by **#301** ("`overdrive workload restart` on a stopped Job is silently ignored"): this
   decision keeps today's behaviour, and #301 changes it so that an operator restart of a stopped Job starts a new attempt while a Job that
   finished on its own is not re-run by crash replacement. The handler adds no answer for the no-op; the `restarted` answer it gives is #301's
   symptom.
4. **A launch that never ran does not count.** An abandoned launch (superseded, or lost with the control plane) ends its row with a
   `Terminated` row of reason `LaunchSuperseded` or `LaunchLostWithControlPlane` and no `terminal` (ADR-0355). The row stays in history and is
   the allocation's row, but it handles no generation and gives no count. Every derivation reads the newest row that ran, the **count basis**:
   the restart count, the `last_failure_seen_at` and the ceiling decision of a new launch, and whether a generation is pending. The new launch's
   count is the basis row's count plus one for a crash restart (no generation pending); the basis row's count for a generation replacement, which
   forgives a `Failed` basis row; and 0 when no row ran (a fresh placement). A generation whose issued id ended abandoned is therefore still
   pending, and its relaunch is a generation replacement. Deleting the abandoned rows from a chain changes no later launch's count, `charged` or
   `exhausted`. An abandoned row is not a natural exit. A relaunch of a platform-reclaimed basis row is unchanged (no ceiling check, no charge). The
   lane exclusivity of ADR-0102 and the dispatch awaited inside the tick mean no launch is in flight when the next evaluation hydrates, so one
   generation is never launched twice.
5. **Bounds.** The View's per-id entries are never pruned (true on `main` for `restart_counts`). An id is issued only when the observed state
   changed since the last issue: a launch that wrote a row, a supersession (one per operator commit or shutdown), an admission refusal
   (ADR-0338), a stale-dispatch fence (one per observed state change). No outcome issues an id on every tick while the observed state is
   unchanged.

Exact shapes and the exhaustive table of launch outcomes with their code citations: feature delta § "Restart generation is handled when its
successor is observed". The test obligations: `distill/upstream-issues.md` of the feature.

## Alternatives considered

- **Keep the stamp on emit and re-arm it from the shim (clear the stamp when a launch is superseded).** Rejected: a second writer of the
  reconciler's View, nothing for a launch lost with the control plane (the shim never runs), and a marker of what was emitted standing in
  for a fact about the resource.
- **Carry the generation on the allocation row (an `AllocStatusRow` field and an envelope bump).** Rejected: the allocation id and the View's
  issuance record already identify "the successor for generation N" without changing a persisted, gossiped observation row.
- **Leave the defect and correct only the text.** Rejected: a stop, a restart and a shutdown leave a workload down while intent says run.
- **Let an abandoned row handle the generation and carry its own count.** Rejected by the user (2026-11-24): an operator restart that supersedes a
  crash restart's launch would inherit the crash restart's charge, and the crash would never be forgiven.
- **Charge every issued id that never gets a row.** Rejected: a supersession is indistinguishable from a fault by the View alone, so an
  operator's stop or a shutdown would spend the restart budget.
- **Leave a finished Job's generation pending and let the shim's fence return.** Rejected: the generation would stay pending forever, the
  reconciler would issue an id on every evaluation and nothing would show it.
- **Make `workload restart` re-run a finished Job (replace the Job row).** Rejected for this decision by the user's ruling (H1, option A):
  re-running a Job is a change to the Job's run-once contract (ADR-0037, ADR-0047) that is not part of this feature.
- **Make the stop path always leave a terminal row, and keep the finished-Job test as `terminal.is_some()`.** Rejected by the user (H3): it
  changes the two-writer arbitration of the stop to fix what the existing classifier `is_operator_stopped` already handles.

## Consequences

- Positive: a stop, a restart or a shutdown during a replacement launch, and a control-plane crash during one, leave one fresh allocation
  to be placed for the generation, charged nothing.
- Positive: `workload restart` of a finished Job issues no id and grows no state.
- Negative: `WorkloadLifecycleView` changes its persisted CBOR shape (a single cut on a greenfield node), and the tests that seed the old
  marker are rewritten.
- Negative: `workload restart` of a finished Job (and of a running Job, which it ends) leaves the Job down and answers as any restart does;
  re-running a Job is not delivered here (#301).
- Exposure, unchanged from `main` and stated once in the feature delta § Scope limitations and exposures: a persistent failure of the
  observation write of a launch's own row leaves the generation pending and re-issues one id per tick.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Restart generation is handled when its successor is observed
- Formal model: `specs/quint/restart-generation/` (one hazard, `stampOnEmit`); feature delta § Formal protocol model
- [ADR-0073](adr-0073-backend-instance-replacement-workload-restart-generation-precursor.md) (amended),
  [ADR-0102](adr-0102-bounded-convergence-evaluation-ownership.md),
  [ADR-0334](adr-0334-a-superseding-stop-or-restart-interrupts-an-in-flight-launchs-waits.md),
  [ADR-0087](adr-0087-single-restart-authority-workload-lifecycle-owns-crash-and-liveness.md),
  [ADR-0337](adr-0337-a-launch-failure-before-the-driver-runs-is-a-recorded-charged-failed-row.md),
  [ADR-0338](adr-0338-an-admission-refusal-by-the-guest-network-pool-writes-no-row-and-charges-nothing.md),
  [ADR-0336](adr-0336-a-workload-waiting-for-guest-network-capacity-is-derived-at-read-time.md), `.claude/rules/reconcilers.md`
