# Slice 06 — VM lifecycle: guest-measured durations and cold versus warm cache

Story: US-BENCH-06. Job: J-PLAT-008. Family: `vm-lifecycle`.

## Goal
Guest receipt, workload exit and poweroff are reported as guest-measured durations, and cold and warm image cache are separate, named profiles.

## In scope
- Guest-internal stages as durations of one guest clock; never a guest instant subtracted from a host instant.
- Cold and warm profiles, each recording its preparation step; never merged.
- Absent (not zero) for a missing guest duration.
- Record of host and guest clock sources.

## Out of scope
Estimating an absolute guest-to-host clock offset (only if a stage truly needs it; not in this slice).

## Learning hypothesis
Disproves: that guest durations can be reported without a cross-clock assumption. If a stage cannot be expressed as a duration, DESIGN must decide how it is bounded.

## Acceptance
- [ ] Guest stages appear as durations only.
- [ ] Cold and warm sets separate, preparation named.
- [ ] Missing guest duration reported absent.
- [ ] Clock provenance recorded.

## Dependencies
Slice 05.

## Estimate
2 days. Reference class: slice 05.
