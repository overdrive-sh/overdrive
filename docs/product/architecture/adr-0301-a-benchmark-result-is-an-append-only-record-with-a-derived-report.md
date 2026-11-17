# ADR-0301 — A benchmark result is an append-only record with a derived report, retained under `benchmarks/results`

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-6 in the feature delta.

## Context

The research found raw samples plus a typed context block to be the dominant
result shape, and no tool records the methodology version or the dirty flag.
`testing.md` requires diagnostic history to be append-only within a run, so a
crashed run keeps its first failure. `development.md` says to persist inputs, not
derived state. `metal pull` rejects paths under `target/`.

## Decision

A run writes one JSON Lines file, `record.jsonl`: a `run_started` line (identity,
provenance, host block, declaration echo, seed), canary lines, one line per
replicate as it completes, and one terminal line: `run_finished`, or `run_refused`
for a run the runner refuses (a refused run is `run_started` then `run_refused`). A
run killed before its terminal line has none and the reader reports it incomplete;
an error before `run_started` (an unreadable checkout marker, a usage error) writes
no record. The runner prints `OVERDRIVE_BENCH_RESULT_DIR=<repo-relative path>` once
the result directory exists, so a refused or killed run can be pulled too. The record
holds only inputs: per-replicate values, failures and absences (a series of more than
1,000 values is stored as a histogram of all of them). Intervals, p-values
and labels are not persisted; `report.md` is a pure function of the record and is
written next to it. Maintained results live at
`benchmarks/results/<family>/<run-id>/`; a spike crate writes under its own
`results/`. The catalogue is `benchmarks/catalogue.md` and the methodology is
`benchmarks/methodology.md`. A reader validates a record against the typed
schema (unknown fields refused), and the `bench-report` tool validates and
renders any record.

## Alternatives

- **One JSON document written at the end**: a crash loses everything; rejected.
- **Persist the computed intervals in the record**: a stale cache of the analysis
  code; rejected.
- **Results under `target/`**: `metal pull` cannot retrieve them; rejected.
- **A histogram for every metric**: loses exact per-replicate values the A/B
  protocol needs; rejected for scalar metrics, which stay raw.
- **A per-replicate cap on stored values (the first N)**: a long profile then
  reports tail percentiles from the start of the run only, and N is a hardcoded
  ceiling; rejected. A series stays raw up to 1,000 values per replicate and is
  otherwise a log-linear histogram (DDD-5) of every value.

## Consequences

Both `record.jsonl` and `report.md` are committed under
`benchmarks/results/<family>/<run-id>/`: the raw record is the evidence and the
report is derived from it. The sizes are small (the connect-hook record is about 30
replicates per arm of two scalars). The largest
series are the DNS per-query latencies. A series metric stores at most 2,049 sparse
histogram buckets per replicate, so a committed record stays small whatever the
load rate and no value is dropped; the report states that histogram-derived
percentiles are within 3.125% relative error. The build
`debug_assertions` flag is recorded; the closed `cargo bench` command prevents a
debug build, so no other build field is recorded.

## Links

- [Decision DDD-6 and the schema](../../feature/native-benchmark-program/feature-delta.md#ddd-6--record-report-and-retention)
