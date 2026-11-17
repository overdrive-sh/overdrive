# Benchmark report: cgroup-connect-hook / connect_hook_cost

- run id: afda96dd5b779f7516937a4f
- family: cgroup-connect-hook
- benchmark: connect_hook_cost
- evidence: spike evidence
- substrate: native
- commit: dcce79fe853b1568e95c9786612b26c60f072acb
- dirty: true
- methodology: 1
- seed: 9824083295649266315
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
| hook_ns_per_connect | service-only | 102.46629999999999 | 1.4901499999999999 | 30 | 0 |
| hook_ns_per_connect | service+gateway | 220.8551 | 3.288550000000015 | 30 | 0 |

| metric | arm | delta (service+gateway minus service-only) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| hook_ns_per_connect | service+gateway | 118.3888 | [116.98160000000001, 119.47339999999998] | [-0.860342500000009, 1.6213999999999942] | 900 | 0.00009999000099990002 | distinguishable |

reference: spike evidence, different run: 117 ns (source: docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md)

### Table: wall-time

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| wall_ns_per_connect | service-only | 27619.7319 | 332.6495000000032 | 30 | 0 |
| wall_ns_per_connect | service+gateway | 27787.4775 | 231.9379000000008 | 30 | 0 |

| metric | arm | delta (service+gateway minus service-only) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| wall_ns_per_connect | service+gateway | 167.745600000002 | [54.78199999999924, 277.0514000000003] | [-2.1071999999985565, 256.17529999999897] | 665 | 0.0012998700129987 | not distinguishable from noise |
