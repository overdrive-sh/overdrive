# Prioritization — phase-1-control-plane-core

## Release Priority

| Priority | Release | Target Outcome | KPI | Rationale |
|---|---|---|---|---|
| 1 | Walking Skeleton (Slices 1-5) | Submit → IntentStore commit → reconciler runtime alive → alloc status round-trip honestly | K1, K2, K3, K4, K5 (see outcome-kpis.md) | This feature IS one walking-skeleton slice per the phase-1-foundation precedent. All five internal slices ship together. |

There is no Release 2 for this feature. The follow-up feature `phase-1-first-workload` is a separate DISCUSS; it picks up the scheduler, process driver, and job-lifecycle reconciler.

## Backlog Suggestions

| Story | Release | Priority | Outcome Link | Dependencies |
|---|---|---|---|---|
| US-01 — Job / Node / Allocation aggregates + canonical intent keys | WS | P1 | K4 (aggregate round-trip); K5 (canonical key stability) | phase-1-foundation newtypes |
| US-02 — Control-plane HTTP/REST service surface | WS | P2 | K1 (round-trip), K2 (error shape) | US-01 |
| US-03 — API handlers + IntentStore commit + ObservationStore reads | WS | P3 | K1 (round-trip), K3 (commit_index monotonic) | US-01, US-02 |
| US-04 — Reconciler primitive: trait + runtime + evaluation broker | WS | P4 (parallel with US-03 after US-01) | K6 (duplicate-collapse invariant), K7 (reconciler registry visibility) | US-01 |
| US-05 — CLI handlers for job / alloc / cluster / node | WS | P5 | K1 (round-trip), K2 (error shape), K8 (honest empty states) | US-02, US-03, US-04 |

## Priority ordering within the walking skeleton

1. **US-01 first** — aggregate shape + canonical key function are consumed by every other slice. Without them, the server and CLI diverge on field definitions and key derivation.
2. **US-02 next** — the REST request / response types and OpenAPI schema are the contract between all subsequent work. The schema-lint gate must be green and the shared types must compile before anything else can depend on them.
3. **US-03 and US-04 in parallel** — they share US-01 but don't share code directly. Two engineers or two branches can proceed in parallel. If resource-constrained to one, US-03 comes first because the CLI acceptance test needs the commit path to exist.
4. **US-05 last** — the CLI is the driving port, and it fails fast if any of US-02, US-03, or US-04 is missing. This is the acceptance gate for the whole feature.

## Tie-break rules

Applied only if Slices 3 and 4 can't run in parallel:

1. Walking skeleton coverage first — both slices are on the skeleton, so this doesn't break the tie.
2. Riskiest assumption first — US-03 (intent commit) is lower risk than US-04 (reconciler primitive). US-04 introduces the whitepaper §18 primitive shape that every later reconciler will inherit; a subtle bug in the broker's cancelable-eval-set would ripple forward. **US-04 therefore jumps ahead of US-03** under resource contention.
3. Highest-value outcome — closing GH #17 (reconciler primitive) was called out in the decisions as one of the three issues this feature covers; not closing it delays Phase 2.

Under no contention, parallel remains the fastest.

## Changelog

| Date | Change |
|---|---|
| 2026-04-23 | Initial prioritization for phase-1-control-plane-core. |
| 2026-04-23 | Transport pivot: US-02 retitled "Control-plane HTTP/REST service surface"; intra-slice ordering unchanged (REST contract still must compile before anything else depends on it). Slice ordering and tie-break rules unchanged. |
