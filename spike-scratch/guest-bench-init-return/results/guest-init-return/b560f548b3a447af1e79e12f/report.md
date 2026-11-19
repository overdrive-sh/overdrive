# Benchmark report: guest-init-return / guest_init_return

- run id: b560f548b3a447af1e79e12f
- family: guest-init-return
- benchmark: guest_init_return
- evidence: spike evidence
- substrate: native
- commit: 5eaa4f7e4ff93a08fb14480bbc1ef979be5e8e1b
- dirty: true
- methodology: 1
- seed: 12824241256093683403
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
| rtt_intact | init-return | 1 | 0 | 30 | 0 |
| rtt_intact | init-return-control | 1 | 0 | 30 | 0 |
| accept_intact | init-return | 1 | 0 | 30 | 0 |
| accept_intact | init-return-control | 1 | 0 | 30 | 0 |
| both_directions_ok | init-return | 1 | 0 | 30 | 0 |
| both_directions_ok | init-return-control | 1 | 0 | 30 | 0 |
| interleaved_lines_in_blocks | init-return | 0 | 0 | 30 | 0 |
| interleaved_lines_in_blocks | init-return-control | 0 | 0 | 30 | 0 |
| kernel_lines_after_block | init-return | 6.5 | 3 | 30 | 0 |
| kernel_lines_after_block | init-return-control | 8 | 3 | 30 | 0 |
| console_bytes | init-return | 71896 | 636.5 | 30 | 0 |
| console_bytes | init-return-control | 71818 | 1014.75 | 30 | 0 |
| block_bytes | init-return | 41102 | 139.5 | 30 | 0 |
| block_bytes | init-return-control | 41085.5 | 173 | 30 | 0 |
| exit_status | init-return | 0 | 0 | 30 | 0 |
| exit_status | init-return-control | 0 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| rtt_ns | init-return | 51348.97580357597 | 59139.504642014166 | 63667.307472712 | 75309.30677551024 | 150000 | 30 | 0 |
| rtt_ns | init-return-control | 40912.43639209548 | 57726.19884659747 | 60923.71643564356 | 76492.41539906112 | 150000 | 30 | 0 |
| accept_ns | init-return | 942208.5 | 1937105.1 | 1967040.55 | 2871891 | 15000 | 30 | 0 |
| accept_ns | init-return-control | 950601 | 1950908.9 | 1971961 | 2328377.8200000003 | 15000 | 30 | 0 |

percentiles of rtt_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (init-return-control minus init-return) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| rtt_ns | init-return-control | -160.23810059819516 | [-11859.237120266953, 36.464323310690816] | [-11924.058284234125, -29.259334829330328] | 385 | 0.34596540345965404 | not distinguishable from noise |
| accept_ns | init-return-control | -6049.5 | [-18588.25, 24860.25] | [-19536.5, 22684.5] | 527 | 0.26217378262173785 | not distinguishable from noise |
| rtt_intact | init-return-control | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| accept_intact | init-return-control | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| both_directions_ok | init-return-control | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| interleaved_lines_in_blocks | init-return-control | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| kernel_lines_after_block | init-return-control | 1.5 | [-3, 3] | [-3, 3] | 480 | 0.7973202679732027 | not distinguishable from noise |
| console_bytes | init-return-control | -78 | [-455.5, 519.5] | [-445.5, 727.0624999999991] | 479.5 | 0.6671332866713329 | not distinguishable from noise |
| block_bytes | init-return-control | -16.5 | [-92, 96.51249999999982] | [-106.5, 73.5] | 493 | 0.5375462453754625 | not distinguishable from noise |
| exit_status | init-return-control | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |

### Table: cycle

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| image_ns | init-return | 64141116 | 2848686.75 | 30 | 0 |
| image_ns | init-return-control | 64147048 | 2296323.75 | 30 | 0 |
| launch_to_ready_ns | init-return | 1108149441.5 | 58715781 | 30 | 0 |
| launch_to_ready_ns | init-return-control | 1090357670 | 45321668 | 30 | 0 |
| exec_to_exit_ns | init-return | 1578124076 | 179583560.75 | 30 | 0 |
| exec_to_exit_ns | init-return-control | 1582346724.5 | 238819364.25 | 30 | 0 |
| exit_to_vmm_exit_ns | init-return | 40140566 | 26192.5 | 30 | 0 |
| exit_to_vmm_exit_ns | init-return-control | 40143555 | 32093.25 | 30 | 0 |
| cycle_ns | init-return | 2799906605 | 195498099.75 | 30 | 0 |
| cycle_ns | init-return-control | 2803139666 | 199071621.25 | 30 | 0 |

| metric | arm | delta (init-return-control minus init-return) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| image_ns | init-return-control | 5932 | [-1485370.0999999999, 1027251.349999999] | [-1757576.15, 229778] | 386 | 0.3474652534746525 | not distinguishable from noise |
| launch_to_ready_ns | init-return-control | -17791771.5 | [-38439716.162499994, 14974534.5] | [-35663599.574999996, 21733722] | 386 | 0.3514648535146485 | not distinguishable from noise |
| exec_to_exit_ns | init-return-control | 4222648.5 | [-85191531.6875, 96429072.5] | [-118341804.975, 53163341.56249988] | 465 | 0.8355164483551645 | not distinguishable from noise |
| exit_to_vmm_exit_ns | init-return-control | 2989 | [-15130.012499999999, 24662.5] | [-20701, 15825] | 501 | 0.46515348465153483 | not distinguishable from noise |
| cycle_ns | init-return-control | 3233061 | [-68186265, 105317933.22499993] | [-113734803, 34260259.82499998] | 477 | 0.6923307669233076 | not distinguishable from noise |
