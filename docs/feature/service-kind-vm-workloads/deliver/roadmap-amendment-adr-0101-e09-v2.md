# Roadmap amendment — ADR-0101 and E09-v2

Date: 2026-09-08
Status: approved by independent focused revision-4 roadmap review

This amendment repurposes the uncompleted `02-03` verification-only step into
the ADR-0101 implementation prerequisite and adds `02-04` for the remaining
native verification. The completed steps and their identifiers are unchanged;
`03-01` now depends on `02-04`.

The existing `deliver/execution-log.json` was audited and not edited. Its
pre-amendment `02-03` RED events are historical verification-step evidence,
not execution of the new implementation contract. A crafter executing the
amended `02-03` must independently perform its own RED/GREEN/COMMIT phases;
no phase is inherited. The prior roadmap review remains provenance for the
pre-amendment eight-step roadmap only.

`02-03` maps BE-01 through BE-12 and BE-P1 exactly to the approved acceptance
artifacts and records seed 257209 plus bridge-specific evaluator/wrapper
transfer or retirement as acceptance-designer-owned controls. Direct
BackendDiscoveryBridge retirement and the sole ServiceLifecycle publisher
must land together; no dual-writer transition is permitted.

`02-04` maps S-SVM-25/26/29 to E09-v2, E10, and E13. E09-v2 is explicitly one
build/prepare, one persistent control plane, controlled concurrent Service
cycles, and exactly 100 honest healthy/failure pairs with no retries or
discarded trials. The retained diagnosis specifically identifies the v2
`wait_for_job_succeeded`/`stop_workload` terminal-state mismatch (failed peer
Jobs can exhaust the worker deadline before cleanup); any necessary correction
is limited to that runner/scheduler observation and cleanup path and remains
separate from ADR-0101 production ownership.

Independent review iteration 4 approved the preceding ADR-0101 revision-3 /
E09-v2 amendment on 2026-09-08:
`roadmap_rev_20260908_amendment_adr0101_e09v2_iteration_1` in
`deliver/review-roadmap.md`. That approval remains provenance only and does
not cover the focused revision-4 delta below.

## Focused revision-4 delta — approved by independent roadmap review

ADR-0101 revision 4 D7 is approved by
`design/review-amendment-be10-local-backend-withdrawal.md`. The existing
`02-03` implementation prerequisite therefore includes only the narrow
`ServiceMapHydrator` private action-selection change: retain the complete local
candidate vector and select the existing `RegisterLocalBackend` or
`DeregisterLocalBackend` action from materialized health. Revision-3's sole
ServiceLifecycle publisher, ownership, public API and all other consumer,
lifecycle, persistence, retry and acknowledgement boundaries remain unchanged.

The approved BE-02 correction is the singleton test input with one genuinely
production-scheduled allocation and all three listeners. Multi-allocation
membership/order coverage remains deferred to issue #282; no replica
scheduling or related mechanism is added to this roadmap.

At amendment creation, this focused delta changed
`roadmap.validation.status` back to `pending` until the original roadmap
reviewer recorded a fresh independent review. That review is now recorded
below. No implementation, native verification, DES event or mutation
completion is claimed.

## Independent roadmap review disposition

Iteration 5 of the focused roadmap review approved this revision-4 D7 and
BE-02 singleton delta on 2026-09-08:
`roadmap_rev_20260908_amendment_adr0101_e09v2_revision4_iteration_1` in
`deliver/review-roadmap.md`. The approval covers only this narrow alignment;
the preceding revision-3/E09-v2 approval and its review ID remain preserved as
provenance above. Validation is approved for roadmap readiness only and does
not claim implementation, native verification, DES, or mutation completion.
