# Benchmark report: cgroup-connect-hook / connect_hook_cost

- run id: c591ccdbe61baae25a58fead
- family: cgroup-connect-hook
- benchmark: connect_hook_cost
- evidence: maintained
- substrate: native
- commit: d15bab5f7dd262884d6078f0c985ace4b693f921
- dirty: true
- methodology: 1
- seed: 9551137753598380525
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
| hook_ns_per_connect | service-only | 102.8535 | 3.148849999999996 | 30 | 0 |
| hook_ns_per_connect | service+gateway | 222.8653 | 4.548249999999996 | 30 | 0 |

| metric | arm | delta (service+gateway minus service-only) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| hook_ns_per_connect | service+gateway | 120.0118 | [117.48530000000001, 121.36907] | [-1.2214000000000027, 0.6663650000000059] | 900 | 0.00009999000099990002 | distinguishable |

reference: spike evidence, different run: 117 ns (source: docs/feature/public-ingress-gateway/spike/findings-connect-hook-cost.md)

### Table: wall-time

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| wall_ns_per_connect | service-only | 27525.5415 | 239.05759999999646 | 30 | 0 |
| wall_ns_per_connect | service+gateway | 27726.6363 | 364.7221500000014 | 30 | 0 |

| metric | arm | delta (service+gateway minus service-only) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| wall_ns_per_connect | service+gateway | 201.09479999999894 | [79.88243499999936, 336.30049999999756] | [-49.50119999999879, 207.72950000000128] | 662 | 0.0018998100189981002 | not distinguishable from noise |
