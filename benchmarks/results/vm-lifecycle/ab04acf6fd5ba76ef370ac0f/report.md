# Benchmark report: vm-lifecycle / vm_lifecycle_stages

- run id: ab04acf6fd5ba76ef370ac0f
- family: vm-lifecycle
- benchmark: vm_lifecycle_stages
- evidence: maintained
- substrate: native
- commit: 6ef7014f06b3f91c815092cac042decb93b99aac
- dirty: true
- methodology: 1
- seed: 457632121410316784
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
- replicates ok: 150
- replicates failed: 0
- status: finished
## Profile: healthy-concurrency-1

one workload is deployed and stopped per replicate

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2646029 | 3057382 | 3212265.9999999995 | 3468371.25 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 14425022.5 | 16682186 | 16721968.3 | 16940624.66 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1279495180 | 1289294155.2 | 1291859456.45 | 1301771038.21 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1016479328.5 | 1019104152.4 | 1022249040.75 | 1026266841.93 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2848943.5 | 3421668.700000003 | 5329456.399999999 | 6245837.610000001 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 66875 | 74415.1 | 79604.14999999998 | 83738.71 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 61085877.5 | 72051239.10000001 | 76914098.79999998 | 81729431.67 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 623820.5 | 691060.8 | 702573.9 | 709744.93 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 16000000 | 23000000 | 24099999.999999993 | 25710000 | 30 | 30 | 0 |

## Profile: healthy-concurrency-4

four workloads are deployed together and stopped together per replicate

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 4280625.5 | 5037768.4 | 5329034.1 | 5648692.9 | 120 | 30 | 0 |
| vmm_launch_ns | lifecycle | 31543274 | 41563342.400000006 | 43961367.99999999 | 48365446.05 | 120 | 30 | 0 |
| guest_ready_ns | lifecycle | 1288817107.5 | 1305792803.4 | 1312053092.1 | 1488410785.9100003 | 120 | 30 | 0 |
| stable_ns | lifecycle | 1039123178.5 | 1057855358.4 | 1061678848.55 | 1083195765.24 | 120 | 30 | 0 |
| stop_submission_ns | lifecycle | 3986544.5 | 7253764.9 | 8990577.6 | 11517834.21 | 120 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 62719.5 | 95123 | 111769.19999999997 | 213210.2300000001 | 120 | 30 | 0 |
| guest_exit_ns | lifecycle | 68454945 | 84320242.60000001 | 87183587.39999999 | 92455420.38000001 | 120 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 596114 | 734151.3 | 779252.6 | 890311.4700000002 | 120 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 12000000 | 17100000.000000007 | 20000000 | 22810000.000000004 | 120 | 30 | 0 |

## Profile: cold-cache

page cache dropped before each replicate

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2159774.5 | 2902555 | 2954957.75 | 3144078.1100000003 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 18248127 | 20887925.2 | 21408658.599999998 | 21750191.93 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1288359587 | 1296220874.5 | 1297954312.4 | 1298895046.65 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1017096333.5 | 1028264007.5 | 1030815443.35 | 1034255785.09 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2540045 | 3930180.400000002 | 5061969.1 | 6236684.420000002 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 58378 | 71544.1 | 73384.15 | 75751.04000000001 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 57169047.5 | 75984437.8 | 78827083.44999999 | 80161943.59 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 554742 | 676886.8 | 687660.85 | 739693.89 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 13000000 | 19100000 | 20549999.999999996 | 23840000.000000004 | 30 | 30 | 0 |

## Profile: warm-cache

image pre-read

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2000791 | 2707773.8000000003 | 2878624.3 | 3030890.22 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 15464912 | 17863253.900000002 | 18558381.349999998 | 19375354.3 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1282811349 | 1290376981.9 | 1292524941.15 | 1311407818.8 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1018260179 | 1022906191.2 | 1024226796.3 | 1026995416.95 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2244567 | 3113600.6000000015 | 4166649.4499999993 | 4802845.48 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 46529.5 | 65118.50000000001 | 68722.5 | 1036811.4500000011 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 51672873 | 73633998.10000001 | 79283672.95 | 85480130.44000001 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 517406 | 662203.8 | 694181.95 | 727796.36 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 9000000 | 17300000.000000004 | 21099999.999999993 | 24840000.000000004 | 30 | 30 | 0 |

## Profile: stop-grace-deadline

workload ignores the stop signal and exits only when the guest stop grace ends

### Table: host-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| admission_ns | lifecycle | 2081162 | 2540298.9 | 2596328.65 | 2798190.49 | 30 | 30 | 0 |
| vmm_launch_ns | lifecycle | 15351377 | 17555841.2 | 18498901.749999996 | 18901690.33 | 30 | 30 | 0 |
| guest_ready_ns | lifecycle | 1282473683 | 1291980706.3 | 1293472919.05 | 1294838338.64 | 30 | 30 | 0 |
| stable_ns | lifecycle | 1019687560.5 | 1028508267.2 | 1029452093.05 | 1032499123.3100001 | 30 | 30 | 0 |
| stop_submission_ns | lifecycle | 2370030.5 | 3899391.2000000007 | 4490551.899999999 | 4764083.42 | 30 | 30 | 0 |
| shutdown_delivery_ns | lifecycle | 48182 | 69637 | 71342.65 | 72264.8 | 30 | 30 | 0 |
| guest_exit_ns | lifecycle | 5065185609 | 5073445105.5 | 5079544309.5 | 5085618549.86 | 30 | 30 | 0 |
| cleanup_calls_ns | lifecycle | 506595 | 645924.3 | 674859.3999999999 | 1766904.0100000014 | 30 | 30 | 0 |

### Table: guest-stages

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| guest_shutdown_handling_ns | lifecycle | 5021000000 | 5026100000 | 5028650000 | 5036390000 | 30 | 30 | 0 |
