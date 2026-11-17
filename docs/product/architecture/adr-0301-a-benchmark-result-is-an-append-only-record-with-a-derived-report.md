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
replicate as it completes, and a `run_finished` line. A run killed before the last
line has no `run_finished` and the reader reports it incomplete. The record holds
only inputs: raw per-replicate values, failures and absences. Intervals, p-values
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
- **A mergeable histogram instead of raw values**: loses per-replicate structure
  the A/B protocol needs; rejected for scalar metrics, and not needed for the
  series the families produce, which a per-replicate cap bounds instead.

## Consequences

Both `record.jsonl` and `report.md` are committed under
`benchmarks/results/<family>/<run-id>/`: the raw record is the evidence and the
report is derived from it. The sizes are small (the connect-hook record is about 30
replicates per arm of two scalars). The largest
series are the DNS per-query latencies. A record stores at most 10,000 values per
series metric per replicate (the first in arrival order) and records the dropped
count as `truncated`, so a committed record stays in the low megabytes per profile
whatever the load rate; the report prints the truncated count. The build
`debug_assertions` flag is recorded; the closed `cargo bench` command prevents a
debug build, so no other build field is recorded.

## Links

- [Decision DDD-6 and the schema](../../feature/native-benchmark-program/feature-delta.md#ddd-6--record-report-and-retention)
