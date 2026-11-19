# Benchmark report: guest-bench-return / guest_bench_return

- run id: fbd007f8944b07177409d285
- family: guest-bench-return
- benchmark: guest_bench_return
- evidence: spike evidence
- substrate: native
- commit: 5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b
- dirty: true
- methodology: 1
- seed: 1625066820745844186
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
- replicates ok: 40
- replicates failed: 10
- status: finished
## Profile: default

### Table: returned

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| samples_returned | console-1k | 1000 | 0 | 10 | 0 |
| samples_returned | vsock-1k | 1000 | 0 | 10 | 0 |
| samples_returned | console-100k | 100000 | 0 | 10 | 0 |
| samples_returned | vsock-100k | absent: no stored value | absent: no stored value | 0 | 10 |
| bytes_on_wire | console-1k | 3146 | 1 | 10 | 0 |
| bytes_on_wire | vsock-1k | 8008 | 0 | 10 | 0 |
| bytes_on_wire | console-100k | 306745.5 | 3.5 | 10 | 0 |
| bytes_on_wire | vsock-100k | absent: no stored value | absent: no stored value | 0 | 10 |
| integrity_ok | console-1k | 1 | 0 | 10 | 0 |
| integrity_ok | vsock-1k | 1 | 0 | 10 | 0 |
| integrity_ok | console-100k | 1 | 0 | 10 | 0 |
| integrity_ok | vsock-100k | absent: no stored value | absent: no stored value | 0 | 10 |
| exec_to_block_ns | console-1k | 61290949.5 | 1367093.75 | 10 | 0 |
| exec_to_block_ns | vsock-1k | 8291578.5 | 794299.75 | 10 | 0 |
| exec_to_block_ns | console-100k | 5244930429 | 48002919 | 10 | 0 |
| exec_to_block_ns | vsock-100k | absent: no stored value | absent: no stored value | 0 | 10 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_sample_ns | console-1k | 30 | 60 | 70 | 70 | 10000 | 10 | 0 |
| guest_sample_ns | vsock-1k | 30 | 40 | 40 | 41 | 10000 | 10 | 0 |
| guest_sample_ns | console-100k | 30.360257021769613 | 40.68019498104351 | 40.876431744860554 | 41.52181567498009 | 1000000 | 10 | 0 |
| guest_sample_ns | vsock-100k | absent: no stored value | absent: no stored value | absent: no stored value | absent: no stored value | 0 | 0 | 10 |

percentiles of guest_sample_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (arm minus console-1k) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| guest_sample_ns | vsock-1k | 0 | [0, 0] | [0, 0] | 45 | 1 | not distinguishable from noise |
| guest_sample_ns | console-100k | 0.3601267649200608 | [0.3600230414746548, 0.36052411866970857] | [0, 0] | 90 | 0.0008999100089991 | distinguishable |
| samples_returned | vsock-1k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| samples_returned | console-100k | 99000 | [99000, 99000] | [0, 0] | 100 | 0.00009999000099990002 | distinguishable |
| bytes_on_wire | vsock-1k | 4862 | [4861, 4862] | [-1, 1] | 100 | 0.00009999000099990002 | distinguishable |
| bytes_on_wire | console-100k | 303599.5 | [303596.5, 303601] | [-1, 1] | 100 | 0.00009999000099990002 | distinguishable |
| integrity_ok | vsock-1k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| integrity_ok | console-100k | 0 | [0, 0] | [0, 0] | 50 | 1 | not distinguishable from noise |
| exec_to_block_ns | vsock-1k | -52999371 | [-53727825.85, -51871706] | [-2980205, 386289.5] | 0 | 0.00009999000099990002 | distinguishable |
| exec_to_block_ns | console-100k | 5183639479.5 | [5125138837.5, 5187647470.775] | [-2980205, 362209] | 100 | 0.00009999000099990002 | distinguishable |

## Failures

- profile default block 0 position 4 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363138 within 130s
- profile default block 1 position 4 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363235 within 130s
- profile default block 2 position 0 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363237 within 130s
- profile default block 3 position 4 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363339 within 130s
- profile default block 4 position 4 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363436 within 130s
- profile default block 5 position 3 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363532 within 130s
- profile default block 6 position 2 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363538 within 130s
- profile default block 7 position 4 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363637 within 130s
- profile default block 8 position 0 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363639 within 130s
- profile default block 9 position 1 arm vsock-100k: exec: no reply to EXEC 2f6f70742f7370696b652f67756573742d73616d706c6572 313030303030 76736f636b 31303533313039363737 within 130s
