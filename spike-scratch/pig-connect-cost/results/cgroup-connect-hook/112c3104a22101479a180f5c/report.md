# Benchmark report: cgroup-connect-hook / connect_hook_cost

- run id: 112c3104a22101479a180f5c
- family: cgroup-connect-hook
- benchmark: connect_hook_cost
- evidence: spike evidence
- substrate: native
- commit: 5f78426f7c5c945dee9e4f4575a45b56e5437ad8
- dirty: true
- methodology: 1
- seed: 13360204042039750177
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
| hook_ns_per_connect | gateway-hook | 137.62490000000003 | 1.45799999999997 | 30 | 0 |

### Table: wall-time

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| wall_ns_per_connect | no-hook | 30105.213 | 362.7086500000005 | 30 | 0 |
| wall_ns_per_connect | gateway-hook | 30290.8865 | 221.76574999999866 | 30 | 0 |

| metric | arm | delta (gateway-hook minus no-hook) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| wall_ns_per_connect | gateway-hook | 185.6735000000008 | [30.98139999999694, 333.9830000000002] | [-107.76669999999649, 212.0059000000001] | 629 | 0.0081991800819918 | not distinguishable from noise |
