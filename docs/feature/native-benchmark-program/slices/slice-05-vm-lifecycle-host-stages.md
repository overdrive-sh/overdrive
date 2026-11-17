# Slice 05 — VM lifecycle: host-observable stages reported separately

Story: US-BENCH-05. Job: J-PLAT-008. Family: `vm-lifecycle`, native substrate.

## Goal
One report row per host-observable stage (admission/queue, VMM launch, guest READY as received, Stable, stop submission, shutdown delivery, guest exit, cleanup), none merged.

## In scope
- Stage timestamps taken by one host process on one host clock, driving the production composition in-process.
- p50, p90, p95, p99, sample and failure counts per stage; failure/grace profile kept apart from healthy stops.
- Header: image cache state, VM resources, concurrency, Cloud Hypervisor and kernel versions.
- Catalogue row.

## Out of scope
Guest-measured durations and cold/warm profiles (slice 06); changing lifecycle semantics.

## Learning hypothesis
Disproves: that each listed stage has an observable boundary on a production surface. A missing boundary sends a DESIGN decision back before DISTILL (O3).

## Acceptance
- [ ] One row per stage with the four percentiles, counts, failures.
- [ ] No row stands for READY, writer success or Stable as a broader completion.
- [ ] A guest that never reports READY is a counted failure in that row.
- [ ] Grace-deadline stops in a separate profile.
- [ ] Header lists the run's conditions.

## Dependencies
Slices 01 to 03; O3. Closed #283 holds the stage list.

## Estimate
3 days; split by stage group if DESIGN's estimate is higher. Reference class: `spike-scratch/boot-cost`.
