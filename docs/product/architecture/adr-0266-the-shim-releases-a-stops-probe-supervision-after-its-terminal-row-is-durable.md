# ADR-0266: The shim releases a stop's probe supervision after its terminal row is durable

## Status

Accepted, 2026-11-03 (approval: feature delta § Approval record, B2; accepted as
proposed). Not implemented. GH #318, GH #334. The exact ordering, the lifecycle
gate (G-5) and the model changes live in the
[feature delta](../../feature/service-allocation-readiness-and-ending/feature-delta.md).

## Context

The shim's `StopAllocation` arm stops the guest, runs the mTLS and network cleanup,
fires `on_alloc_terminal` (which releases the allocation's probe supervisor), and
only then writes the terminal row (`action_shim/mod.rs:3250-3272`, `:3356`). If the
row write fails, the row stays `Running`, the guest is dead, and the supervisor is
gone. `VmDriver::stop` emits no exit event, so the exit observer never sees the
allocation, and no further probe row can be written.

Every retry the design relies on is keyed on a new probe row: the startup stop
(ADR-0260) and the liveness stop (ADR-0191) both re-emit on a row newer than the one
they acted on. `WorkloadLifecycle` does not restart a `Running` row, and the VM
reclamation sweep acts only on allocations with host residue, which the stop already
removed (`vm_reclamation.rs`, `plan_reclamation` iterates the host observation).
Nothing converges the allocation: a dead guest behind a `Running` row.

The liveness stop has carried this gap since ADR-0087; ADR-0260 mirrors the liveness
retry and inherits it.

## Decision

**In `StopAllocation` the shim fires `on_alloc_terminal` after the terminal row
write succeeds, not before.** If the write fails the probe supervisor stays
registered, probe rows keep arriving, and the detector's next row re-emits the stop;
the retry then finds the guest already stopped (not-found is tolerated) and writes
the row.

- The hook contract (ADR-0255) already states that the hook has no ordering relative
  to the row write, so no contract text changes.
- The VM slot release and the alloc-to-driver index removal keep their current
  position.
- `FinalizeFailed`'s genuine-terminal arm keeps its order: the ceiling terminal is
  re-emitted by the restart authority while the committed row lacks it, which is a
  row-keyed convergence, not a probe-row-keyed one.

## Alternatives considered

- **Retry the row write inside the shim.** Hides a store failure behind a bounded
  loop in the executor, and a persistent failure still ends with the supervisor
  gone.
- **Re-emit the stop whenever the row is `Running` and the gates hold, ignoring the
  probe row sequence.** Convergent, but removes the pacing the retry design chose:
  the runtime re-enqueues a target after a failed dispatch, so a persistent failure
  repeats a driver call at evaluation cadence.
- **Teach the reclamation sweep to converge a `Running` row whose VM and host
  surface are gone.** Correct but a new class of reclamation (row against slot),
  larger than the defect.
- **Accept the exposure.** A store write error at this one step permanently strands
  one allocation, silently.

## Consequences

- Positive: a failed terminal row write for a stop no longer strands the allocation;
  the retry trigger outlives the failure.
- Negative: after a failed write the dead guest keeps being probed until the retry
  lands, one probe interval at a time; probe rows for a dead guest are harmless (the
  feature delta, Q3).
- Negative: the shim's `StopAllocation` ordering changes, so the accepted ADR-0255
  scenario that checks the stop hook now also checks its position after the write.
- Neutral: no archived, wire or persisted change.
