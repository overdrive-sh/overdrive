# Concurrent worktree verification

Feature: `metal-run-isolation`. Story: `US-METAL-ISOLATION`. Job: `J-PLAT-006`.
Status: requirements ready for DESIGN; implementation contracts pending.

## Goal

Marcus verifies two independent worktree changes concurrently on the shared Metal box and trusts each source-matched result without physical-host test-kernel contamination.

## IN

- Worktree remote identity with the user-selected branch fallback, including same-branch workspaces and sanitisation collisions.
- Remove ordinary box-wide test execution serialization while preserving per-checkout source/build exclusion and exclusive host maintenance.
- Existing fresh disposable guest per isolation group; bounded concurrent CID/listener/resource ownership and cleanup changes required for identical selections.
- Existing ordinary direct-command paths must meet the isolation outcome, not only the recommended explicit guest-suite command.
- Durable invocation records, logs, exact statuses, collection, source markers and native/nested evidence applicability.
- Bounded compiler, test and operator-documentation fallout required by the approved design.

## OUT

Product runtime changes; the stopped service feature; a new framework or outer guest per run; concurrent source mutation within one checkout; fleet scheduling; unlimited capacity; run reboot recovery; new dashboards, benchmarks or recurring verification jobs.

## Learning hypothesis

Concurrent execution of the existing guest-group suite from two real worktrees can preserve source fidelity, kernel isolation and invocation-owned resources without a box-wide test lease. A collision, host test-kernel delta, incorrect evidence attribution or interference with the surviving run disproves this hypothesis; mere lease removal or green isolated runs do not confirm it.

## Acceptance

AC-1..AC-7 in [the feature delta](../feature-delta.md) all apply. Dogfood in the same delivery session through existing `cargo xtask metal run -- ...` and `metal collect <run-id>`, using the worktrees' actual synced sources and the existing suite bodies. Retain overlapping execution, distinct run IDs, exact command results, source attribution and external ownership/cleanup observations. Include a same-branch controlled case, one existing failure path and repeat collection; do not manufacture success output.

## Dependencies and estimate

Existing qualified Metal launcher and prepared guest artifacts. DESIGN closes Q1..Q5, verifies contract-shaping facts, records the exact interfaces/error mappings and obtains required approvals before DISTILL/DELIVER. No dependency on resumed service-feature delivery.

Estimate: one working day / six crafter hours, provisional until DESIGN checks the bounded changes. This estimate neither caps the invariant-preserving work nor licenses an unsafe partial slice. Reference class: adaptations of the existing Metal scripts and guest suite, not construction of a new runner.

## Priority and taste tests

Only release slice: one value-bearing story covering choose source → prepare → invoke → observe → decide. No technical-layer slice can ship independently with the promised concurrency outcome. Uses actual worktree sources and existing guest tests; no synthetic-only demo, new component framework or scale-only variants. Any contract-shaping spike is DESIGN-owned and bounded to its specific unresolved premise.
