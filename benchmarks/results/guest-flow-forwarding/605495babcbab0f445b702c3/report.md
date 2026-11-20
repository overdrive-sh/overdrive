# Benchmark report: guest-flow-forwarding / guest_flow_forwarding

- run id: 605495babcbab0f445b702c3
- family: guest-flow-forwarding
- benchmark: guest_flow_forwarding
- evidence: maintained
- substrate: guest
- commit: 36618cba7a86588dce3608ed3f898890017b9ef4
- dirty: true
- methodology: 1
- seed: 3325604675173432802
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
- guest launch: metal host (not nested)
- guest boot: fresh guest per replicate
- guest vcpu scheduling: unpinned
- guest governor: absent: not visible from a guest
- guest smt: absent: not visible from a guest
- guest boost: absent: not visible from a guest
- guest vcpus: 2
- guest memory mib: 512
- guest kernel: 7.0.0-38-generic
- guest clocksource: tsc
- guest artifact overdrive_init: 010f5819f7876bf7a64db6f4d0f8ba2c02d8946d289acb05042ce33d60db63d0
- guest artifact overdrive_guest_peer: 537b159757b75fba3f716767772d4974f021af36cb13068908025178014dcfaa
- guest artifact ovd_fwd: 0e64db0873bffd666da6a98588b06ce2e0fa5fe88b49920d64a0a9a34615fca1
- bpf stats: off
- clock: host CLOCK_MONOTONIC_RAW; service none; guest CLOCK_MONOTONIC (in-guest Instant, 1 ns)
- canary: steady
- replicates ok: 30
- replicates failed: 0
- status: finished
## Profile: default

### Table: forwarded-path

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| forwarded_round_trip_ns | forwarding | 50548.19092595716 | 57239.5006833713 | 62337.90020618555 | 74186.20061538463 | 150000 | 30 | 0 |

percentiles of forwarded_round_trip_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)
