# Slice 04 — Same declaration from a spike probe, and a smoke variant (walking skeleton)

Story: US-BENCH-04. Job: J-PLAT-008. Family: `cgroup-connect-hook`.

## Goal
A `spike-scratch/` crate produces a schema-identical record marked spike evidence from the same attribute, and the maintained benchmark has a smoke test in the ordinary lane.

## In scope
- `pig-connect-cost` re-expressed as a spike crate depending on the benchmark crate by path; no change under `crates/` for the probe.
- Record marked spike evidence; the report renders it.
- Every record and report states its substrate; no command compares records.
- Smoke variant: one replicate per arm, no timing assertion, gated on `integration-tests`, `#[overdrive_test]`-placed in a guest group and accepted by `placement-lint`; it runs on a Metal integration run, `cargo xtask metal run -- cargo nextest run -p <crate> --features integration-tests`.

## Out of scope
Deleting the existing probes; any gate on the smoke result's timing.

## Learning hypothesis
Disproves: that one declaration can serve both a maintained lane and a throwaway probe without a schema fork. A fork or a probe needing a `crates/` edit falsifies it.

## Acceptance
- [ ] Spike record validates against the same schema, marked spike evidence.
- [ ] Every record and report states its substrate; no command compares records.
- [ ] Smoke variant passes with one replicate per arm and asserts no timing, on a Metal integration run.
- [ ] A broken fixture fails the smoke test.
- [ ] `placement-lint` accepts the smoke test.

## Dependencies
Slices 01 and 02.

## Estimate
1 to 2 days. Reference class: slice 01 (macro expansion work already done).
