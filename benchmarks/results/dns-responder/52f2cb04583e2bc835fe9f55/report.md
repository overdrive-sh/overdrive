# Benchmark report: dns-responder / dns_query_latency

- run id: 52f2cb04583e2bc835fe9f55
- family: dns-responder
- benchmark: dns_query_latency
- evidence: maintained
- substrate: native
- commit: 96e4f23580b8e95b90dbba1eba222b54ed4cc831
- dirty: true
- methodology: 1
- seed: 14265126596616745040
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
- clock: host CLOCK_MONOTONIC_RAW; service Clock::now (monotonic); guest none
- canary: steady
- replicates ok: 120
- replicates failed: 0
- status: finished
## Profile: a-record-1-name-1-client-2000qps

scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape A on a resolvable name; name table of 1 name

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 2000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 38795.63696660362 | 44332.47795657727 | 46776.450010759625 | 53797.29815845827 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 13753.562986330178 | 15255.801006470167 | 16040.693106382978 | 20074.157694013316 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: a-record-1000-names-4-clients-8000qps

scheduled-arrival load of 8000 qps for 5 seconds from 4 client sockets; query shape A on resolvable names; name table of 1000 names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 8000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 99350.57588514479 | 108021.70777871022 | 111462.17766107857 | 120531.40664343788 | 1200000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 76675.90700417793 | 85294.38307068462 | 89887.28165967256 | 96631.91797833936 | 1200000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: aaaa-no-data-100-names-1-client-2000qps

scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape AAAA on a resolvable name that has no AAAA record (no data); name table of 100 names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 2000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 47842.54384662557 | 55045.21606450613 | 58127.6179237406 | 67729.34281167111 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 22750.895177984625 | 25472.965739022886 | 27016.228955098613 | 35844.05400630916 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: unknown-name-1-client-2000qps

scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape A on a name that is not in the name table (name error); name table of 100 other names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 2000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 40737.57414929145 | 46434.54623655915 | 49566.40573476702 | 54732.00754534463 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 15538.528066528066 | 17465.83539379935 | 21965.237493526667 | 25669.870211480367 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)
