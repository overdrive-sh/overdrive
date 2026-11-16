# Slice 03 — Production host settings, drift refusal and per-process BPF statistics (walking skeleton)

Story: US-BENCH-03. Job: J-PLAT-008. Family: `cgroup-connect-hook`.

## Goal
A run records the host's settings, refuses a host that differs from the production settings before any sample, and takes BPF run time without ever changing `kernel.bpf_stats_enabled`.

## In scope
- Host block in record and report: kernel version, CPU model and count, governor, SMT, boost, `bpf_stats_enabled`.
- Refusal before the first sample, naming the setting with found and production values.
- BPF statistics enabled for the benchmark process only; the record states on or off.
- A killed run leaves nothing enabled and the host unchanged.

## Out of scope
Changing host settings (no turbo, SMT or ASLR changes); canary (slice 02); the reference set's source is open point O1.

## Learning hypothesis
Disproves: that per-process statistics are enough to avoid toggling the node. If hook times differ from the spike's sysctl-on figures by more than the measured +67 ns per run, the scoping or the settings check is wrong.

## Acceptance
- [ ] Drifted host refused before any sample, setting named.
- [ ] Host block present in record and report.
- [ ] `bpf_stats_enabled` reads 0 before, during and after.
- [ ] Killed run leaves no statistics enabled.
- [ ] Statistics-off benchmarks record "off".

## Dependencies
Slice 01 (record). O1 settled by DESIGN.

## Estimate
1 to 2 days. Reference class: the overhead spike (one day).
