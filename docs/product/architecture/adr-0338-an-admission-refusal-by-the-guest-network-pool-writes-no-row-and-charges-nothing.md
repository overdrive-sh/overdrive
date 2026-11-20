# ADR-0338 — An admission refusal by the guest-network pool writes no row and charges nothing

## Status

**Accepted** 2026-11-18 (decision D-1 of the node-local persistent volume feature, GH #348; user ruling of 2026-11-18, option A). This ADR is
not implemented and is stated in the present tense. It is separate from [ADR-0336](adr-0336-a-workload-waiting-for-guest-network-capacity-is-derived-at-read-time.md),
which decides how a wait is shown: the refusal rule could be reversed (a charged `Failed` row) while a view of a placement withheld at the cap
would still be useful, and the view could be dropped without changing what a refusal does.

## Context

The guest-network pool caps the node at 16,384 attached guests. The reconciler refuses to emit a placement when the observed occupancy is at
the cap, but occupancy is read at hydration and other targets' ticks run concurrently, so a placement can reach the pool just after the last
slot is taken and be refused (`AdmissionCapReached`). `LeaseRetiring` is not reachable for a freshly issued id and `PoolExhausted` is not
reachable in production (the guest prefix is a /16 with 65,533 usable offsets, so the cap is reached first). A refusal is a capacity race, not
a fault of the workload.

## Decision

**An admission refusal writes no allocation row and charges no restart attempt. The reconciler's occupancy check paces it: while observed
occupancy is at the cap the reconciler emits no placement, and when a slot frees it emits again.**

1. A first placement refused by the pool returns the refusal with no row, no lease and no charge.
2. A restart refused by the pool still runs the predecessor's cleanup (`finish_restart`), which retires and tears down the predecessor's lease
   and releases its slot. That is the progress that lets a later placement succeed.
3. **The bound.** A refusal requires another target to have taken the last slot after this evaluation hydrated. The next hydration reads the
   occupancy: at the cap it emits nothing; below it (the restart arm's cleanup just released the predecessor's slot, or another workload
   stopped) it emits again and may succeed. The ids this workload issues are therefore bounded by the number of lost races, each of which is
   another target's admission, and never grow per tick while the pool stays at the cap. For a restart the next evaluation after a refusal is
   expected to emit once more, because its own predecessor released a slot.
4. The operator-visible wait is ADR-0336.

## Alternatives considered

- **A charged `Failed` row `WorkloadNetnsProvisionFailed { stage: "guest_network" }`.** Rejected: a capacity race would advance the restart
  count, and a persistently full node would exhaust the budget and finalise `BackoffExhausted` for a workload whose spec is sound.
- **A View counter that paces refusals.** Rejected: a second pacing input beside the occupancy the reconciler already reads, for a state the
  occupancy check already represents.

## Consequences

- Positive: a capacity race costs the workload no budget and leaves no failed record.
- Negative: a refusal leaves no durable occurrence; only the `guest_network.admission_refusal` log event records it. This is intended: nothing
  depends on its occurrence (a convergent record cannot answer "did it happen" applies to crashes, which write rows; a refusal charges and
  loses nothing).
- Negative: a restart-arm refusal can issue one more id when its own predecessor's release lowers the occupancy.

## Links

- [Feature delta](../../feature/node-local-persistent-volume/feature-delta.md) § Launch outcomes (N-ADM) and § Decisions on the waiting state
- Formal model: `specs/quint/restart-generation/` (action `LaunchRefused`; hazard `refusalCharged`, invariant
  `I_CAPACITY_WAIT_NEVER_EXHAUSTS`; run `admission_refusal_leaves_no_row_and_waits_for_capacity`)
- [ADR-0335](adr-0335-a-restart-generation-is-handled-when-its-successor-is-observed.md),
  [ADR-0336](adr-0336-a-workload-waiting-for-guest-network-capacity-is-derived-at-read-time.md),
  [ADR-0134](adr-0134-placement-reads-held-attachment-count-through-read-port.md), `.claude/rules/reconcilers.md`
