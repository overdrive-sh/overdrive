# Benchmark report: guest-bench-return / guest_bench_return

- run id: 1728b190aa4eaf615bf534f1
- family: guest-bench-return
- benchmark: guest_bench_return
- evidence: spike evidence
- substrate: native
- commit: 5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b
- dirty: true
- methodology: 1
- seed: 2746882999444222559
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
- replicates ok: 70
- replicates failed: 0
- status: finished
## Profile: default

### Table: returned

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| samples_returned | console-1k | 1000 | 0 | 10 | 0 |
| samples_returned | vsock-1k | 1000 | 0 | 10 | 0 |
| samples_returned | console-100k | 100000 | 0 | 10 | 0 |
| samples_returned | vsock-100k | 100000 | 0 | 10 | 0 |
| samples_returned | console-1m | 1000000 | 0 | 10 | 0 |
| samples_returned | vsock-1m | 1000000 | 0 | 10 | 0 |
| bytes_on_wire | console-1k | 3146 | 1 | 10 | 0 |
| bytes_on_wire | vsock-1k | 8008 | 0 | 10 | 0 |
| bytes_on_wire | console-100k | 306750 | 11 | 10 | 0 |
| bytes_on_wire | vsock-100k | 800008 | 0 | 10 | 0 |
| bytes_on_wire | console-1m | 3066671 | 41.25 | 10 | 0 |
| bytes_on_wire | vsock-1m | 8000008 | 0 | 10 | 0 |
| integrity_ok | console-1k | 1 | 0 | 10 | 0 |
| integrity_ok | vsock-1k | 1 | 0 | 10 | 0 |
| integrity_ok | console-100k | 1 | 0 | 10 | 0 |
| integrity_ok | vsock-100k | 1 | 0 | 10 | 0 |
| integrity_ok | console-1m | 1 | 0 | 10 | 0 |
| integrity_ok | vsock-1m | 1 | 0 | 10 | 0 |
| exec_to_block_ns | console-1k | 61037275 | 2568635.5 | 10 | 0 |
| exec_to_block_ns | vsock-1k | 8167649.5 | 669961.5 | 10 | 0 |
| exec_to_block_ns | console-100k | 5152294063 | 225897247.5 | 10 | 0 |
| exec_to_block_ns | vsock-100k | 14916882.5 | 1058406.75 | 10 | 0 |
| exec_to_block_ns | console-1m | 52280674493.5 | 1504966062.25 | 10 | 0 |
| exec_to_block_ns | vsock-1m | 88118125.5 | 4379185 | 10 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_sample_ns | console-1k | 30 | 40 | 40 | 41 | 10000 | 10 | 0 |
| guest_sample_ns | vsock-1k | 30 | 40 | 40 | 41 | 10000 | 10 | 0 |
| guest_sample_ns | console-100k | 30.40066927796191 | 50.07803097260116 | 60.368992361700485 | 60.98622806882185 | 1000000 | 10 | 0 |
| guest_sample_ns | vsock-100k | 30.360438292964243 | 40.6804735907696 | 40.87643117766708 | 41.52100565875262 | 1000000 | 10 | 0 |
| guest_sample_ns | console-1m | absent: no stored value | absent: no stored value | absent: no stored value | absent: no stored value | 0 | 0 | 10 |
| guest_sample_ns | vsock-1m | absent: no stored value | absent: no stored value | absent: no stored value | absent: no stored value | 0 | 0 | 10 |

percentiles of guest_sample_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (arm minus console-1k) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| guest_sample_ns | vsock-1k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| guest_sample_ns | console-100k | 0.3603526068559688 | [0.36012938987088816, 0.36174402373748293] | [0, 0] | 100 | 0.00009999000099990002 | distinguishable |
| guest_sample_ns | vsock-100k | 0.36028248307433586 | [0.36007232285394863, 0.3604486865249896] | [0, 0] | 100 | 0.00009999000099990002 | distinguishable |
| samples_returned | vsock-1k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| samples_returned | console-100k | 99000 | [99000, 99000] | [0, 0] | 100 | 0.00019998000199980003 | distinguishable |
| samples_returned | vsock-100k | 99000 | [99000, 99000] | [0, 0] | 100 | 0.00009999000099990002 | distinguishable |
| samples_returned | console-1m | 999000 | [999000, 999000] | [0, 0] | 100 | 0.00009999000099990002 | distinguishable |
| samples_returned | vsock-1m | 999000 | [999000, 999000] | [0, 0] | 100 | 0.00009999000099990002 | distinguishable |
| bytes_on_wire | vsock-1k | 4862 | [4861, 4862] | [-1, 2.5] | 100 | 0.00009999000099990002 | distinguishable |
| bytes_on_wire | console-100k | 303604 | [303597, 303610.5] | [-1, 2.5] | 100 | 0.00009999000099990002 | distinguishable |
| bytes_on_wire | vsock-100k | 796862 | [796861, 796862] | [-1, 2.5] | 100 | 0.00009999000099990002 | distinguishable |
| bytes_on_wire | console-1m | 3063525 | [3063516, 3063574.5] | [-1, 2] | 100 | 0.00009999000099990002 | distinguishable |
| bytes_on_wire | vsock-1m | 7996862 | [7996861, 7996862] | [-1, 2.5] | 100 | 0.00009999000099990002 | distinguishable |
| integrity_ok | vsock-1k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| integrity_ok | console-100k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| integrity_ok | vsock-100k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| integrity_ok | console-1m | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| integrity_ok | vsock-1m | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| exec_to_block_ns | vsock-1k | -52869625.5 | [-54029823.5, -50665907] | [-2567237.9999999995, 2210102.5] | 0 | 0.00009999000099990002 | distinguishable |
| exec_to_block_ns | console-100k | 5091256788 | [4940295858.5, 5180682462.5] | [-2454329.3625, 2210573.5] | 100 | 0.00009999000099990002 | distinguishable |
| exec_to_block_ns | vsock-100k | -46120392.5 | [-47443815, -43777794.5] | [-2603029, 2210114.275] | 0 | 0.00009999000099990002 | distinguishable |
| exec_to_block_ns | console-1m | 52219637218.5 | [50281319883, 52269181346.5] | [-2559193.5, 2210102.5] | 100 | 0.00009999000099990002 | distinguishable |
| exec_to_block_ns | vsock-1m | 27080850.5 | [24743381, 31725294] | [-2509629, 2249667] | 100 | 0.00009999000099990002 | distinguishable |

## Failures

- profile default block 0 position 0 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 0 position 1 arm console-1m metric guest_sample_ns: not recorded
- profile default block 1 position 0 arm console-1m metric guest_sample_ns: not recorded
- profile default block 1 position 1 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 2 position 0 arm console-1m metric guest_sample_ns: not recorded
- profile default block 2 position 5 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 3 position 1 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 3 position 4 arm console-1m metric guest_sample_ns: not recorded
- profile default block 4 position 0 arm console-1m metric guest_sample_ns: not recorded
- profile default block 4 position 6 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 5 position 4 arm console-1m metric guest_sample_ns: not recorded
- profile default block 5 position 5 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 6 position 3 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 6 position 5 arm console-1m metric guest_sample_ns: not recorded
- profile default block 7 position 0 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 7 position 4 arm console-1m metric guest_sample_ns: not recorded
- profile default block 8 position 2 arm console-1m metric guest_sample_ns: not recorded
- profile default block 8 position 6 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 9 position 4 arm vsock-1m metric guest_sample_ns: not recorded
- profile default block 9 position 6 arm console-1m metric guest_sample_ns: not recorded
