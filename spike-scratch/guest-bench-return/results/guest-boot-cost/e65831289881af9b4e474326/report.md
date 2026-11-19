# Benchmark report: guest-boot-cost / guest_boot_cost

- run id: e65831289881af9b4e474326
- family: guest-boot-cost
- benchmark: guest_boot_cost
- evidence: spike evidence
- substrate: native
- commit: 5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b
- dirty: true
- methodology: 1
- seed: 13125700775602893332
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
- bpf stats: off
- clock: host CLOCK_MONOTONIC_RAW; service none; guest none
- canary: steady
- replicates ok: 90
- replicates failed: 0
- status: finished
## Profile: default

### Table: lifecycle

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| image_ns | fresh-guest | 74678905.5 | 2229874.25 | 30 | 0 |
| image_ns | fresh-guest-control | 75312536 | 2462386 | 30 | 0 |
| boot_to_peer_ns | fresh-guest | 947918975.5 | 50227471.5 | 30 | 0 |
| boot_to_peer_ns | fresh-guest-control | 943325742.5 | 39954057 | 30 | 0 |
| first_exec_ns | fresh-guest | 8039232 | 859065.25 | 30 | 0 |
| first_exec_ns | fresh-guest-control | 8739052 | 1242482.5 | 30 | 0 |
| shutdown_ns | fresh-guest | 40219887 | 20052023.75 | 30 | 0 |
| shutdown_ns | fresh-guest-control | 40210949 | 17584.25 | 30 | 0 |
| total_ns | fresh-guest | 1125189564.5 | 45321807 | 30 | 0 |
| total_ns | fresh-guest-control | 1112605317 | 56858667 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| warm_exec_ns | fresh-guest | 2201212.5 | 2298277.2 | 2353788.05 | 2642762.1799999997 | 600 | 30 | 0 |
| warm_exec_ns | fresh-guest-control | 2220141.5 | 2309844.1 | 2364668.849999999 | 4062637.87 | 600 | 30 | 0 |

| metric | arm | delta (fresh-guest-control minus fresh-guest) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| image_ns | fresh-guest-control | 633630.5 | [-645458.1749999998, 1704486.474999999] | [-1928790.6874999998, -48108.68750000058] | 487 | 0.5927407259274072 | not distinguishable from noise |
| boot_to_peer_ns | fresh-guest-control | -4593233 | [-34561954, 12865184.074999977] | [-24665121.5, 26137345.062499955] | 426 | 0.732026797320268 | not distinguishable from noise |
| first_exec_ns | fresh-guest-control | 699820 | [-71549.96249999994, 961740] | [281469.975, 1316254] | 561 | 0.10068993100689931 | not distinguishable from noise |
| warm_exec_ns | fresh-guest-control | 23177.75 | [-21657.5, 97138.75] | [-57917, 84486.75] | 507 | 0.4056594340565943 | not distinguishable from noise |
| shutdown_ns | fresh-guest-control | -8938 | [-46159, 4266] | [-46169.5, 11808] | 332 | 0.07939206079392061 | not distinguishable from noise |
| total_ns | fresh-guest-control | -12584247.5 | [-41604100.6, 15710363.462499999] | [-25668117.5, 28123863.14999997] | 414 | 0.5974402559744025 | not distinguishable from noise |
