# Benchmark report: guest-init-return-lead / guest_init_return

- run id: cf2a4bea07be31301b460ec2
- family: guest-init-return-lead
- benchmark: guest_init_return
- evidence: spike evidence
- substrate: native
- commit: 5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b
- dirty: true
- methodology: 1
- seed: 444524685700768324
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

### Table: returned

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| rtt_intact | bare | 1 | 0 | 30 | 0 |
| rtt_intact | lead-newline | 1 | 0 | 30 | 0 |
| accept_intact | bare | 1 | 1 | 30 | 0 |
| accept_intact | lead-newline | 1 | 0 | 30 | 0 |
| both_directions_ok | bare | 1 | 0 | 30 | 0 |
| both_directions_ok | lead-newline | 1 | 0 | 30 | 0 |
| interleaved_lines_in_blocks | bare | 0 | 0 | 30 | 0 |
| interleaved_lines_in_blocks | lead-newline | 0 | 0 | 30 | 0 |
| kernel_lines_after_block | bare | 13 | 3 | 30 | 0 |
| kernel_lines_after_block | lead-newline | 12.5 | 3 | 30 | 0 |
| console_bytes | bare | 71814.5 | 935.75 | 30 | 0 |
| console_bytes | lead-newline | 71914.5 | 842.25 | 30 | 0 |
| block_bytes | bare | 41077 | 173.75 | 30 | 0 |
| block_bytes | lead-newline | 41039.5 | 72.75 | 30 | 0 |
| exit_status | bare | 0 | 0 | 30 | 0 |
| exit_status | lead-newline | 0 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| rtt_ns | bare | 40892.60117548279 | 55183.48194399743 | 60395.94240327953 | 71094.58365217395 | 150000 | 30 | 0 |
| rtt_ns | lead-newline | 40732.255048946114 | 53394.435494880556 | 59763.80659868026 | 70804.69235023046 | 150000 | 30 | 0 |
| accept_ns | bare | 953004.5 | 1952166.1 | 1975343.5 | 2859992.3000000003 | 9500 | 19 | 11 |
| accept_ns | lead-newline | 939058 | 1929369 | 1962703.9999999998 | 2856173.4500000007 | 15000 | 30 | 0 |

percentiles of rtt_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (lead-newline minus bare) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| rtt_ns | lead-newline | -14.228911686521315 | [-97.90841205939068, 41.97905448733218] | [-83.76408549048938, 112.16337532118632] | 376 | 0.27857214278572145 | not distinguishable from noise |
| accept_ns | lead-newline | -14784.5 | [-34216.5, 5418.5] | [-28578.5, 19699] | 209 | 0.11828817118288171 | not distinguishable from noise |
| rtt_intact | lead-newline | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| accept_intact | lead-newline | 0 | [0, 1] | [-1, 1] | 615 | 0.00029997000299970003 | not distinguishable from noise |
| both_directions_ok | lead-newline | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| interleaved_lines_in_blocks | lead-newline | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| kernel_lines_after_block | lead-newline | -0.5 | [-3, 3] | [-3, 3] | 411 | 0.601039896010399 | not distinguishable from noise |
| console_bytes | lead-newline | 100 | [-533.5, 411] | [-343, 516.0124999999998] | 423 | 0.6992300769923008 | not distinguishable from noise |
| block_bytes | lead-newline | -37.5 | [-106.5, 20] | [-55, 119] | 395 | 0.41645835416458354 | not distinguishable from noise |
| exit_status | lead-newline | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |

### Table: cycle

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| guest_workload_ns | bare | 901171354.5 | 202277443.75 | 30 | 0 |
| guest_workload_ns | lead-newline | 856543603.5 | 126230938.25 | 30 | 0 |
| guest_print_ns | bare | 649809057 | 37743159.5 | 27 | 3 |
| guest_print_ns | lead-newline | 650405882.5 | 25411048 | 30 | 0 |
| image_ns | bare | 63919314.5 | 1484201 | 30 | 0 |
| image_ns | lead-newline | 64844009 | 1437127.5 | 30 | 0 |
| launch_to_ready_ns | bare | 1094665826.5 | 48653685 | 30 | 0 |
| launch_to_ready_ns | lead-newline | 1090933589.5 | 44062618 | 30 | 0 |
| exec_to_exit_ns | bare | 1553511503 | 213040706.25 | 30 | 0 |
| exec_to_exit_ns | lead-newline | 1523719323.5 | 123929595 | 30 | 0 |
| exit_to_vmm_exit_ns | bare | 40135178 | 8515.75 | 30 | 0 |
| exit_to_vmm_exit_ns | lead-newline | 40136995.5 | 31539.5 | 30 | 0 |
| cycle_ns | bare | 2752882626.5 | 200584683.25 | 30 | 0 |
| cycle_ns | lead-newline | 2738437480.5 | 156385292 | 30 | 0 |

| metric | arm | delta (lead-newline minus bare) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| guest_workload_ns | lead-newline | -44627751 | [-152786756, 27140976.5] | [-65885501.5, 151090767] | 320 | 0.057894210578942104 | not distinguishable from noise |
| guest_print_ns | lead-newline | 596825.5 | [-16207743.037499996, 31821160.5] | [-29630125, 27751676] | 449 | 0.49025097490250974 | not distinguishable from noise |
| image_ns | lead-newline | 924694.5 | [198273.5, 1528493.875] | [-549497, 952880] | 638 | 0.005099490050994901 | not distinguishable from noise |
| launch_to_ready_ns | lead-newline | -3732237 | [-26404365.2875, 22731585.5] | [-26571654.25, 27781095.5] | 441 | 0.8994100589941005 | not distinguishable from noise |
| exec_to_exit_ns | lead-newline | -29792179.5 | [-134410830.5, 40388287.71249998] | [-66111239.5, 150795937] | 352 | 0.1506849315068493 | not distinguishable from noise |
| exit_to_vmm_exit_ns | lead-newline | 1817.5 | [-5368, 23146.5375] | [-3422.05, 6766.037499999978] | 465 | 0.8346165383461653 | not distinguishable from noise |
| cycle_ns | lead-newline | -14445146 | [-134217826, 52860142.5] | [-84546392.5, 165056775] | 377 | 0.2872712728727127 | not distinguishable from noise |

## Failures

- profile default block 0 position 1 arm bare metric accept_ns: not recorded
- profile default block 1 position 2 arm bare metric accept_ns: not recorded
- profile default block 3 position 0 arm bare metric accept_ns: not recorded
- profile default block 4 position 1 arm bare metric accept_ns: not recorded
- profile default block 4 position 2 arm bare metric accept_ns: not recorded
- profile default block 7 position 0 arm bare metric accept_ns: not recorded
- profile default block 7 position 2 arm bare metric accept_ns: not recorded
- profile default block 8 position 0 arm bare metric accept_ns: not recorded
- profile default block 8 position 1 arm bare metric guest_print_ns: not recorded
- profile default block 9 position 2 arm bare metric accept_ns: not recorded
- profile default block 11 position 0 arm bare metric accept_ns: not recorded
- profile default block 13 position 0 arm bare metric guest_print_ns: not recorded
- profile default block 15 position 2 arm bare metric accept_ns: not recorded
- profile default block 17 position 1 arm bare metric accept_ns: not recorded
- profile default block 17 position 2 arm bare metric accept_ns: not recorded
- profile default block 18 position 1 arm bare metric accept_ns: not recorded
- profile default block 20 position 0 arm bare metric guest_print_ns: not recorded
- profile default block 20 position 1 arm bare metric accept_ns: not recorded
- profile default block 21 position 1 arm bare metric guest_print_ns: not recorded
- profile default block 21 position 2 arm bare metric guest_print_ns: not recorded
- profile default block 23 position 1 arm bare metric accept_ns: not recorded
- profile default block 23 position 2 arm bare metric accept_ns: not recorded
- profile default block 25 position 0 arm bare metric accept_ns: not recorded
- profile default block 25 position 2 arm bare metric accept_ns: not recorded
- profile default block 26 position 1 arm bare metric accept_ns: not recorded
- profile default block 27 position 2 arm bare metric accept_ns: not recorded
- profile default block 28 position 0 arm bare metric accept_ns: not recorded
- profile default block 28 position 2 arm bare metric accept_ns: not recorded
- profile default block 29 position 1 arm bare metric accept_ns: not recorded
