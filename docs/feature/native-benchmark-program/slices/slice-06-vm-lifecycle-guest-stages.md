# Slice 06 — VM lifecycle: guest-measured durations and cold versus warm cache

Story: US-BENCH-06. Job: J-PLAT-008. Family: `vm-lifecycle`.

## Goal
One guest-measured duration (`guest.shutdown-handling`) is reported next to the host-stamped shutdown-delivery and guest-exit stage intervals, and cold and warm image cache are separate, named profiles.

## In scope
- One guest-clock duration, `guest.shutdown-handling`; guest receipt and poweroff appear only as host-stamped stage intervals; never a guest instant subtracted from a host instant.
- Cold and warm profiles, each recording its preparation step; never merged.
- Absent (not zero) for a missing guest duration.
- Record of host and guest clock sources.

## Out of scope
A guest-to-host clock-offset estimate: not part of this feature.

## Learning hypothesis
Disproves: that guest durations can be reported without a cross-clock assumption. If a stage cannot be expressed as a duration, DESIGN must decide how it is bounded.

## Acceptance
- [ ] One guest duration (`guest.shutdown-handling`); receipt and poweroff only as host-stamped stage intervals.
- [ ] Cold and warm sets separate, preparation named.
- [ ] Missing guest duration reported absent.
- [ ] Clock provenance recorded.

## Dependencies
Slice 05.

## Estimate
2 days. Reference class: slice 05.
