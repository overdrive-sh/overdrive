# ADR-0303 — BPF run statistics are enabled for the benchmark process only

## Status

**Accepted.** Approved by the user 2026-11-17; independent design review resolved 2026-11-17. DDD-8 in the feature delta.

## Context

Ruling 2: `kernel.bpf_stats_enabled` stays 0; a benchmark that needs `run_time_ns`
and `run_cnt` enables statistics for its own process with the file-descriptor
scoped `BPF_ENABLE_STATS` command. In `vendors/linux` at v7.0 the command returns
an anonymous-inode descriptor named `bpf-stats` and raises the global static key
(`kernel/bpf/syscall.c:6002-6020`); closing the descriptor, including at process
exit, lowers it (`:5990-5996`); it needs `CAP_SYS_ADMIN` (`:6030`). The sysctl
handler keeps its own saved value, so the sysctl reads 0 while a descriptor holds
the key (`:6533-6558`). The overhead spike measured +67 ns per program run.

## Decision

A benchmark that declares `bpf_stats` gets one descriptor from
`aya::sys::enable_stats` for the whole run, held by the runner and exposed to the
body as a read handle; every arm of a comparison therefore runs with the same
setting. The record states `bpf_stats` as `off` or `per_process`. For a declared
benchmark the runner checks that the programs it reads did accumulate runs, and
reports a replicate as failed when statistics were requested but a read shows none.
Nothing writes the sysctl. The smoke test does not hold the descriptor: statistics
metrics are `absent` under smoke and the smoke test asserts no timing.

## Alternatives

- **Write the sysctl and restore it**: a crash leaves it on; rejected by ruling 2.
- **A stats-off control arm per benchmark**: a cost the user has not asked for;
  the overhead is recorded in the spike (about 0.5% of a connect) and the record
  names the setting; not adopted.

## Consequences

A benchmark of a very short program (a roughly 70 ns XDP run roughly doubles with
statistics on) must not declare `bpf_stats`; the catalogue states the caveat.
V-1 confirms the descriptor behaviour on the Metal host.

## Links

- [Decision DDD-8](../../feature/native-benchmark-program/feature-delta.md#ddd-8--per-process-bpf-statistics)
