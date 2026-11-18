# Benchmark report: vm-lifecycle / vm_lifecycle_stages

- run id: 1759869cfd254906cd263e60
- family: vm-lifecycle
- benchmark: vm_lifecycle_stages
- evidence: maintained
- substrate: native
- commit: 284f2a6596a9088578e0c39fc43c44f1c38d943e
- dirty: true
- methodology: 1
- seed: 15379119280374342581
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
- clock: host CLOCK_MONOTONIC_RAW; service none; guest CLOCK_MONOTONIC (overdrive-init Instant::elapsed, 1 ms)
- canary: steady
- replicates ok: 150
- replicates failed: 0
- status: finished
## Profile: healthy-concurrency-1

one workload is deployed and stopped per replicate

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2814582 | 3195571.5 | 3247104.05 | 3298809.39 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 14114086 | 15926688.8 | 16438506.049999999 | 16682796.67 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1279715205 | 1287479206.2 | 1287968573.8 | 1297984574.99 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1016730930.5 | 1020665740.4 | 1021541109 | 1026319208.88 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2795052.5 | 4258515.800000001 | 5679007.899999996 | 8123732.810000002 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 64531.5 | 75617.1 | 77316.2 | 83761.51000000001 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 61504750.5 | 82363956.80000001 | 86113106.75 | 86537486.27 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 611191.5 | 680631.7000000001 | 730849.4499999998 | 752664.38 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 18000000 | 25000000 | 25549999.999999996 | 30970000.000000007 | 30 | 30 | 0 |

## Profile: healthy-concurrency-4

four workloads are deployed together and stopped together per replicate

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 4215628 | 4852819.4 | 4970350.55 | 5657736.450000001 | 120 | 30 | 0 |
| vmm_launch_ns | lifecycle | 31509458 | 41835334.7 | 43321961.55 | 46702534.09 | 120 | 30 | 0 |
| guest_ready_ns | lifecycle | 1284269341 | 1308719835.9 | 1323972891.65 | 1432296938.7600002 | 120 | 30 | 0 |
| stable_ns | lifecycle | 1035840469.5 | 1058571945.2 | 1062619430.25 | 1066971948.95 | 120 | 30 | 0 |
| stop_submission_ns | lifecycle | 3761397 | 6874144 | 7904437.249999997 | 19157812.77000001 | 120 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 59224.5 | 89755.30000000002 | 108575.49999999999 | 251048.1600000002 | 120 | 30 | 0 |
| guest_exit_ns | lifecycle | 67697208.5 | 85297281.3 | 86822372.5 | 89615234.52 | 120 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 597721.5 | 787040.2000000001 | 844823.2999999999 | 907895.0800000001 | 120 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 11500000 | 16000000 | 18049999.999999996 | 20810000.000000004 | 120 | 30 | 0 |

## Profile: cold-cache

page cache dropped before each replicate

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2936855 | 3281544.9 | 3299458.15 | 3376967.5100000002 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 20791737 | 23224850.8 | 23666472.55 | 25145525.290000003 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1285371740 | 1291682934.6 | 1292346054.9 | 1297297655.97 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1015337580.5 | 1018570355.6 | 1019476078.55 | 1021068255.78 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2793244.5 | 3411804.9000000013 | 4874655.699999997 | 6721270.720000002 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 67596.5 | 75422.2 | 79304.79999999999 | 84226.88 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 58893539 | 72433387.7 | 73442161.25 | 77050460.38000001 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 636189.5 | 699234.8 | 740857.7499999999 | 754347.17 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 15000000 | 21000000 | 22099999.999999993 | 24420000 | 30 | 30 | 0 |

## Profile: warm-cache

image pre-read

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2966609.5 | 3256558.7 | 3329975.05 | 3993991.4500000007 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 18005753.5 | 21126372.400000002 | 21521170.05 | 22006392.810000002 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1280828500 | 1289749719 | 1291008588.75 | 1294385876.64 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1017025321.5 | 1021838957.1 | 1022277356.75 | 1022744104.99 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2816540 | 3381591.800000002 | 4997567.449999998 | 6007984.130000001 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 64827 | 72592.2 | 74788.09999999999 | 75958.81 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 64837609.5 | 79005797.8 | 82294207.74999999 | 85186312.42 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 647196 | 742241.9 | 748362.05 | 827698.8700000001 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 18000000 | 24000000 | 24549999.999999996 | 30680000.000000007 | 30 | 30 | 0 |

## Profile: stop-grace-deadline

workload ignores the stop signal and exits only when the guest stop grace ends

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2797111 | 3211466 | 3288836 | 3359777.1 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 18627106 | 20685258.9 | 20881800.45 | 21074306.01 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1279694894.5 | 1289076668.5 | 1290096684.4 | 1295347442.68 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1016211802 | 1020730797.7 | 1021770888.5 | 1024303953.83 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2859058.5 | 3246346.9 | 3968553.9499999965 | 4980225.9 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 66234.5 | 72993.5 | 77417.7 | 84336.52 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 5070497041.5 | 5083843346.7 | 5086799113.5 | 5103893727.55 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 670591.5 | 762736.9 | 835768.1999999997 | 1332489.7400000007 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 5024000000 | 5033100000 | 5037300000 | 5042840000 | 30 | 30 | 0 |
