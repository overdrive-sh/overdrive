# Slice 07 — DNS responder: client-observed apart from resolver-internal

Story: US-BENCH-07. Job: J-PLAT-008. Family: `dns-responder`, native substrate.

## Goal
Query latency and throughput per named profile, with client-observed results and resolver-internal timing in separate tables.

## In scope
- Profile matrix: query shape, cache hit or miss, record cardinality, transport, client concurrency, response size; each result set names its profile.
- Scheduled-arrival load for tail latency; the arrival model stated in the report.
- Failed or timed-out queries counted, never latency samples.
- Fixture conditions recorded; catalogue row.

## Out of scope
Changing DNS behaviour; wire-correctness (a Tier-3 test).

## Learning hypothesis
Disproves: that the production responder exposes resolver-internal timing separate from the client's view. If it does not, DESIGN defines the observable point (O3).

## Acceptance
- [ ] Client-observed and resolver-internal in separate tables.
- [ ] Profile named per result set.
- [ ] Arrival model stated.
- [ ] Failures counted separately.
- [ ] Catalogue row present.

## Dependencies
Slices 01 to 03; O3. This is the slice whose diff measures KPI K1.

## Estimate
2 to 3 days. Reference class: slice 01.
