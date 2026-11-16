# Slice 02 — A/B comparison under the fixed statistical protocol (walking skeleton)

Story: US-BENCH-02. Job: J-PLAT-008. Family: `cgroup-connect-hook`.

## Goal
The report states the B-minus-A delta with a bootstrap interval beside an A/A noise floor, and shows the spike's 117 ns point as a labelled reference.

## In scope
- Interleaved arm order across replicates; an A/A control in the same run.
- At least 9 replicates per arm (a declaration below that is refused).
- Canary measurement at start and end; a shift flags the run.
- Nonparametric comparison, bootstrap interval of the difference, noise-floor interval; "not distinguishable from noise" labelling.
- Spike reference (117.0 ns hook delta; wall delta about 168 ns inside noise) shown, labelled spike evidence from a different run.
- Methodology document at a named version (runner isolation, host eligibility, clock boundaries, cold/warm handling, sample and percentile method, record format, failure accounting).

## Out of scope
Host-settings refusal and per-process statistics (slice 03); regression thresholds or baselines.

## Learning hypothesis
Disproves: that the shared core adds no bias of its own. If the new hook-time delta falls outside the spike's per-run spread (115.5 to 126.5 ns for the gateway program), the core or the method is suspect.

## Acceptance
- [ ] Interleaving and A/A in one run.
- [ ] Delta, interval, floor and nonparametric result in the report.
- [ ] Inside-the-floor deltas labelled noise.
- [ ] Fewer than 9 replicates refused.
- [ ] Canary shift flags the run, samples kept.
- [ ] Spike reference shown and labelled.
- [ ] Methodology document present with a version.

## Dependencies
Slice 01.

## Estimate
2 days. Reference class: the spike's own analysis code (30 interleaved runs, bootstrap of 10,000 resamples).
