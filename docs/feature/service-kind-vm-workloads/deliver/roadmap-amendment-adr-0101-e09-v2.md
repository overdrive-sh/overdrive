# Roadmap amendment — ADR-0101 and E09-v2

Date: 2026-09-08
Status: approved by independent roadmap review

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

Independent review iteration 4 approved this amendment on 2026-09-08:
`roadmap_rev_20260908_amendment_adr0101_e09v2_iteration_1` in
`deliver/review-roadmap.md`. The approval is limited to this amendment and
does not claim implementation, native-100, or mutation completion.
