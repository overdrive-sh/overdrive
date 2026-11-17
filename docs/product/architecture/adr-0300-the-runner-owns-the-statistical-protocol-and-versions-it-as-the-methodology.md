# ADR-0300 — The runner owns the statistical protocol and versions it as the methodology

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-5 in the feature delta.

## Context

Ruling 5 accepts at least 9 replicates per arm, an A/A noise-floor control, a
canary, and a nonparametric A/B comparison with bootstrap confidence intervals.
The spike `bpf-stats-overhead` used seeded shuffled blocks of OFF1, OFF2 and ON,
percentile bootstrap of the difference of medians, and a rule that an effect
counts only when its interval excludes zero and its point estimate exceeds the
A/A bound. No tool in the research versions its methodology in the result.

## Decision

The runner, not the benchmark body, owns the protocol; a body cannot skip any
part. Methodology version 1 is:

- a replicate is one fresh fixture; the body runs once per replicate;
- per profile, replicates are grouped in blocks; each block runs every arm once
  and, when there are two or more arms, one A/A control (a second run of the
  reference arm, the first declared arm); the order inside a block is a seeded shuffle, and the seed (from
  OS entropy, with no override) is recorded;
- no value is dropped: a series metric of a replicate is stored raw up to 1,000
  values and otherwise as a log-linear histogram (32 sub-buckets per power of
  two, relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values) from which percentiles are computed;
- only `ok` replicates feed a statistic;
- a single-arm benchmark is a distribution benchmark: no comparison, no A/A;
- fewer than 9 replicates per arm is refused;
- the A/B statistic is the difference of per-replicate medians, with a 95%
  percentile bootstrap interval of 10,000 seeded resamples, a Mann-Whitney U
  statistic with a seeded 10,000-permutation p-value, and the same interval for
  the A/A pair;
- a difference is "distinguishable" only when its interval excludes zero and the
  absolute point estimate exceeds the larger absolute bound of the A/A interval;
  otherwise it is "not distinguishable from noise";
- a fixed canary (loopback connect-and-close, no program attached) is taken at the
  start and the end; a shift under the same rule flags the run unstable;
- failures are counted, never dropped; an arm with fewer than 9 successful
  replicates reports "insufficient replicates" in place of a comparison, and a
  metric with no stored value in either compared arm has no comparison row.

The version is a number in `overdrive-bench` and in `benchmarks/methodology.md`;
a change to any parameter bumps it.

## Alternatives

- **Strict alternation A,B / B,A**: cannot randomise setup order (research
  F1.2); the spike already moved to seeded blocks; rejected.
- **Welch t-test or Criterion's bootstrap t-test**: assumes shape the latency
  data does not have (research F1.5); rejected.
- **A fixed percentage noise threshold**: the A/A interval measures the floor per
  run instead; rejected.

## Consequences

The story text "replicate order alternates between arms" becomes "each block
holds every arm once in a recorded seeded order". The statistics are implemented
in the crate (no new statistics dependency) and are property-testable.

## Links

- [Decision DDD-5 and the parameter table](../../feature/native-benchmark-program/feature-delta.md#ddd-5--the-statistical-protocol-methodology-version-1)
