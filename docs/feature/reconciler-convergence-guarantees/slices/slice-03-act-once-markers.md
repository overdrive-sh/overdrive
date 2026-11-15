# Slice 03 — act-once-markers

Stories: US-04 (assessment) and the executor doc fix from US-03 AC4. Persona: Ana. Job: J-OPS-003. Order: 3.

US-03 (VIP release retried until the address is free) moved out of this feature to GH #211 (user ruling D-5, 2026-11-13). This feature does not wait on #211; #211 is not a prerequisite.

## Goal

Correct the `release_service_vip` executor doc comment so it states what the executor does. Keep the recorded assessment of the `startup_last_acted_run` / `liveness_last_acted_run` markers (feature delta D-6): compliant retry pacing unless the slice 04 property finds a reachable lost stop.

## IN

- Correct the `release_service_vip.rs` doc comment that says errors are absorbed; the executor returns `ShimError::AllocatorRelease`.
- US-04: the recorded assessment of whether a dropped `StopAllocation` can be lost when no newer probe row arrives (`service_lifecycle.rs:1057-1069`, `:1190-1208`, `:632-654`). The assessment is D-6 in the feature delta; the stimulus lives in the slice 04 property. A fix is opened only if that property goes red.

## OUT

- Replacing the `released_for_deletion` gate (`workload_lifecycle.rs:1531`, stamped `:247`) with a decision from allocator state through `service_vip_view` (D8), and the D14 seeded reproduction (same spec re-submitted, then deleted again): both move to GH #211 ("Implement workload deletion (intent withdrawal) + service/dataplane teardown", open). #211 requires: release decided from allocator state via `service_vip_view`, retried until the address is free, `released_for_deletion` deleted, designed together with the post-withdrawal digest source (the deletion verb supplies the spec_digest), and the same retry rule for the dial-by-name frontend `F`. Until #211 lands, `released_for_deletion` is listed as a known violation in the new View rule text.
- `observed_generation` (#325).
- `stable_announced` / `terminal_announced` (refuted, D5).
- `last_emitted_backend_fingerprint` ownership (O7).
- Editing stale test fixtures unless this slice touches them (O6).

## Learning hypothesis

US-04: the assessment either reproduces a lost stop on the production path (through the slice 04 property) or shows the markers are pure retry pacing.

## Acceptance criteria

- AC1 The executor doc comment matches behaviour (it propagates `ShimError::AllocatorRelease`; it does not absorb the error).
- AC2 Assessment recorded for the startup/liveness markers (D-6) with the resulting action: compliant pacing, or a fix only if the slice 04 property finds a reachable lost stop.

## Dependencies

None. Not blocked by and not waiting on #211.

## Effort

About 1 day (doc fix minutes; assessment recording; more only if the slice 04 property reproduces a lost stop).

## Spike

No. The US-04 assessment is part of the slice, not a pre-slice spike.

## Production-drivable fault

The doc fix has no runtime behaviour. The US-04 stimulus is a dropped `StopAllocation` for a Service allocation through `serve` + `deploy`, injected at the `SimDriver` stop seam (slice 04 property, D-6).
