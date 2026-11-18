# Benchmark report: cgroup-connect-hook / connect_hook_cost

- run id: 9e740a312f48cddc18c56ee3
- family: cgroup-connect-hook
- benchmark: connect_hook_cost
- evidence: maintained
- substrate: native
- commit: 8dabedfc35ee290cfb9b8993ade5a6641ce53ac0
- dirty: true
- methodology: 1
- seed: 14692921128210123342
- debug_assertions: false
- kernel: 7.0.0-38-generic
- cpu_model: AMD EPYC 8024P 8-Core Processor
- cpu_count: 16
- governor: schedutil
- smt: on
- boost: on
- bpf_stats_enabled: 0
- clocksource: tsc
- irq_time_accounting: n
- bpf stats: on (per process)
- clock: host CLOCK_MONOTONIC_RAW; service none; guest none
- canary: steady
- replicates ok: 90
- replicates failed: 0
- status: finished
## Profile: default

### Table: hook-time

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| hook_ns_per_connect | no-hook | absent: no connect program is attached on this arm | absent: no connect program is attached on this arm | 0 | 0 |
| hook_ns_per_connect | gateway-hook | 137.4357 | 4.161450000000002 | 30 | 0 |

reference: spike evidence, different run, gateway-hook median: 137.6249 ns (source: spike-scratch/pig-connect-cost/results/cgroup-connect-hook/112c3104a22101479a180f5c/report.md)

### Table: wall-time

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| wall_ns_per_connect | no-hook | 30108.7034 | 241.31454999999914 | 30 | 0 |
| wall_ns_per_connect | gateway-hook | 30354.5151 | 175.97695000000022 | 30 | 0 |

| metric | arm | delta (gateway-hook minus no-hook) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| wall_ns_per_connect | gateway-hook | 245.81170000000202 | [131.9191749999997, 332.91519999999946] | [-115.88379999999961, 89.28580000000147] | 730 | 0.00009999000099990002 | distinguishable |

reference: spike evidence, different run, gateway-hook minus no-hook delta: 185.6735 ns (source: spike-scratch/pig-connect-cost/results/cgroup-connect-hook/112c3104a22101479a180f5c/report.md)
