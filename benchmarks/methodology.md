# Benchmark methodology

Methodology version 1. The runner (`crates/overdrive-bench`) owns this protocol and stamps the
version into every record's header; a record states which version produced it. A change to or
removal of a rule below bumps the version; an addition for a new substrate does not. Normative definitions live in ADR-0300 (statistics), ADR-0301
(record and report), ADR-0302 (host eligibility), ADR-0303 (BPF statistics), ADR-0305 (clocks).

A benchmark is not a test. It runs only through `cargo xtask metal run --benchmark <family>`
under exclusive admission on the test host, and its result is a retained record, never a
nextest outcome. Each benchmark's declaration also generates a smoke test that runs one
replicate per arm in a guest and asserts no timing; for a guest benchmark that test guest boots the
family's measuring guest and still takes no measurement.

## Runner isolation

- The benchmark form holds exclusive admission for the whole run: no other Metal run, test or
  maintenance command shares the host while samples are taken.
- A native benchmark runs on the native substrate (the host itself, no namespace, no wrapper); a
  guest benchmark runs in a guest the benchmark process launches directly on that host (see
  Guest substrate). The record carries the substrate; records of different substrates are never compared, and the
  report tool compares only the arms of one record.
- Every replicate builds a fresh fixture and tears it down on exit; no state survives from one
  replicate to the next.
- The runner never writes `kernel.bpf_stats_enabled`. Per-program run statistics are enabled
  with a per-process `BPF_ENABLE_STATS` descriptor the runner holds for the run; the kernel
  lowers the statistics when the process exits, including when it is killed.
- Replicate order is seeded and shuffled inside each block; the seed comes from OS entropy and
  is always recorded. There is no seed override.

## Host eligibility

Before any sample the runner reads the host against a pinned list and refuses on the first
difference, in this order: probe self-check, kernel series (`7.0`), `kernel.bpf_stats_enabled`
(`0`), CPU governor (`schedutil`), SMT (`on`), CPU boost (on), any holder of a `bpf-stats`
descriptor (none), and a release build (no debug assertions). A pinned setting with no readable
source cannot be verified and refuses. CPU model, CPU count, clocksource and
`irq_time_accounting` are recorded and never refuse. A refused run writes `run_started` and
`run_refused` naming the setting with the found and pinned values, takes no sample and exits 78.
The scan for `bpf-stats` holders exists because the sysctl alone cannot see a descriptor-scoped
enable held by another process.

## Clock boundaries

Every instant that crosses a boundary is stamped on the host with `CLOCK_MONOTONIC_RAW`. A
stage that happens inside a guest is a duration the guest measured; it is never added to or
subtracted from a host instant, and a stage the guest cannot report is recorded as absent.
A family declares its service and guest clock sources, and the record names them in its `clock`
block (`null` for a source the family does not use).
Each family's catalogue row names its clock boundary: what the first and last stamp of a
measured interval are. BPF program time is the kernel's per-program `run_time_ns` over
`run_count`, reported as a separate metric from wall time and never mixed with it.

## Cold and warm handling

A replicate is one fresh fixture, so its first operations run cold. A family states in its
declaration and catalogue row whether it measures cold or warm behaviour and how it gets
there: a warm-up phase of unrecorded operations before the timed phase (warm), or profiles
whose preparation text names the cache state (cold, warm). Warm-up operations never enter a
metric. The record's replicate notes carry the cache state and concurrency of the replicate.

## Samples and percentiles

- Minimum replicates per arm: 9; fewer is refused.
- A block holds every arm once, plus one A/A control (a second run of the reference arm, which
  is the first declared arm) when there are two or more arms.
- Only `ok` replicates feed any statistic; a failed replicate's partial metrics are recorded and
  never used.
- Quartiles and percentiles use linear interpolation between closest ranks (R-7). A series
  metric keeps every value: raw up to 1,000 values per replicate, else a log-linear histogram of
  at most 2,049 buckets whose percentile is within one bucket width (3.125% relative at 1 and
  above) of the exact one.
- The A/B statistic is the difference of per-replicate medians, non-reference arm minus reference
  arm, with a 95% percentile bootstrap interval over 10,000 seeded resamples, the same interval
  for the A/A pair, and a seeded 10,000-permutation Mann-Whitney U p-value.
- Verdict: `distinguishable` when the delta interval excludes 0 and the absolute point estimate
  exceeds the larger absolute bound of the A/A interval; otherwise `not distinguishable from
  noise`; `insufficient replicates` below 9 `ok` replicates.
- A canary of loopback connect-and-close samples runs at the start and the end of every run; a
  shift between them marks the run `unstable`. A reference value quoted from earlier evidence
  is printed under its table with its label and is never a comparison.

## Record format

A run writes `benchmarks/results/<family>/<run-id>/record.jsonl`, append-only JSON Lines, and
derives `report.md` from it. Line kinds: `run_started` (methodology version, substrate, host
block, declaration echo, seed), `canary`, `replicate` (profile, arm, block, outcome, metrics,
notes, command log), `run_finished` or `run_refused`. A killed run leaves a record with no
terminal line and no report; `bench-report render <record>` reports it as incomplete.
`bench-report validate <record>` checks a record against the schema. Both the record and the
report are committed; the record is the evidence and the report is derived from it.

## Failure accounting

A failed replicate, a failed metric and an absent metric are recorded and counted; none is
dropped or zero-filled. A declared metric a replicate never recorded is one failure
`not recorded`. A series metric that was partly measured keeps both: its measured values feed
the statistics and each failure is recorded and counted. A failed replicate does not stop the run; failures and the count of absent
entries appear in each report table. A run that finishes with failed replicates still exits 0;
the failures are in the report, not the status. Refusals exit 78, usage errors 64 and internal
errors (including a failed statistics enable) 70.

## Guest substrate

A guest benchmark measures code that runs inside a guest. Every rule above applies to it unless
this section says otherwise.

- **Isolation.** The benchmark process launches the guest directly on the exclusively held host;
  a guest is never nested inside another guest. Every replicate boots a fresh guest and the guest
  is gone when the replicate ends.
- **Eligibility.** The full host list above is read on the real host before any guest boots, and
  a drifted host is refused with no guest booted. No guest setting is pinned. The CPU governor,
  SMT and boost are recorded absent with the reason "not visible from a guest". The guest's
  kernel and clocksource, its vCPU count and memory, and the digests of the staged guest
  artifacts are recorded from the first replicate that reports them; later replicates are not
  compared with that reading.
- **Clock boundaries.** The guest clock is the guest's own `CLOCK_MONOTONIC`. A guest duration is
  never combined with a host stamp.
- **vCPU marking.** The record states the vCPU count and that vCPU scheduling is unpinned.
- **Staged artifacts.** The record names the guest artifacts that ran by digest. The benchmark
  form does not rebuild them: run `cargo xtask guest-kit-prepare` at the commit being measured.
- **Canary.** The loopback canary is a host measurement and says nothing about the guest.
- **Comparability.** Records of different substrates are never compared. Arms are compared only
  inside one record.
- **BPF statistics.** A guest benchmark declares no `bpf_stats`; its record states `off`.
- **Failure accounting.** A guest failure, including a guest kit that was not prepared, is a
  failed replicate; its console is retained beside the record and the run continues. A run whose
  replicates all failed still exits 0, so exit status 0 does not imply that any replicate was ok:
  read `replicates_ok`.
