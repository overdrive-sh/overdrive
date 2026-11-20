# ADR-0333 — A replicate retains its guest console beside the record

## Status

**Accepted (2026-11-19).** Approved by the user as PD-9, option (a): retain every replicate's console. Extends [ADR-0301](adr-0301-a-benchmark-result-is-an-append-only-record-with-a-derived-report.md) (operative) and the `Replicate` contract of the native benchmark program (DDD-6). DDD-9 in the [feature delta](../../feature/guest-benchmark-substrate/feature-delta.md).

## Context

US-GBS-03 requires the guest console to be retained with the result directory, so a failed guest can be diagnosed. A benchmark body has no way to write a file next to the record: `Replicate` is opaque and the runner owns the result directory. A series metric keeps raw values only up to 1,000 per replicate and a histogram above that, so the console of a replicate that returned 5,000 values is the only raw copy of them.

## Decision

`Replicate::retain_console(text)` records the replicate's console (a second call by the same replicate is a `ReplicateFailure`). After the body returns, in every outcome, the runner writes it to `<result dir>/artifacts/<profile>/<block>-<position>-<arm>/console.log` through `RecordSink::write_artifact` with create-new semantics; a write failure fails the replicate, with the write error as the reason when the body returned Ok and appended to the body's reason when it already failed. Writing in place would make `Replicate` borrow the sink and so not `Send`. The path derives from the replicate's own coordinates, which every `replicate` line carries, so no record field and no note is added. Under `smoke` there is no result directory: nothing is written and `Ok` is returned. A guest family retains each replicate's console in every outcome (which replicates, PD-9). The `artifacts/` directory is pulled with the result directory and is not committed.

## Alternatives

- **A `console` field on the `replicate` line**: grows every record line by tens of kilobytes. Rejected.
- **The family writes into the result directory itself**: the directory path is not exposed to bodies, and a body writing beside the record could overwrite it. Rejected.
- **A general `retain(name, bytes)` with a name grammar and a `notes` entry**: one caller; the grammar and the note exist only to serve callers that do not exist. Rejected.

## Consequences

`RecordSink` gains one method. Native families do not call `retain_console`; their records are unchanged. The repository ignores `benchmarks/results/**/artifacts/` (a `.gitignore` line, approved with PD-9 on 2026-11-19).

## Links

- [feature delta, DDD-9, PD-9](../../feature/guest-benchmark-substrate/feature-delta.md)
