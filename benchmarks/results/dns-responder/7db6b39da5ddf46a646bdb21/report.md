# Benchmark report: dns-responder / dns_query_latency

- run id: 7db6b39da5ddf46a646bdb21
- family: dns-responder
- benchmark: dns_query_latency
- evidence: maintained
- substrate: native
- commit: c5edf53b2bc6c9534de11b51928729863049c197
- dirty: true
- methodology: 1
- seed: 9163226831717898970
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
| client_latency_ns | responder | 38752.5339873114 | 44589.40745762712 | 46952.80237791932 | 55746.140818713466 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 13959.038930153205 | 15602.631944131563 | 16499.45985299265 | 22338.183245283028 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: a-record-1000-names-4-clients-8000qps

scheduled-arrival load of 8000 qps for 5 seconds from 4 client sockets; query shape A on resolvable names; name table of 1000 names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 8000 | 0.04513934245369455 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 99863.69110639372 | 108776.33298108063 | 111876.0885979269 | 120553.1751884058 | 1200000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 77002.25817439842 | 85757.33395521948 | 90620.89634602243 | 97270.1463251048 | 1200000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: aaaa-no-data-100-names-1-client-2000qps

scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape AAAA on a resolvable name that has no AAAA record (no data); name table of 100 names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 2000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 47946.775001853355 | 52378.12330335242 | 54603.82867328749 | 59087.80455089821 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 22947.26905445194 | 25227.640392383153 | 26179.142320385883 | 31608.041089494174 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: unknown-name-1-client-2000qps

scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape A on a name that is not in the name table (name error); name table of 100 other names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 2000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 40044.141268872554 | 45656.31532344724 | 48187.04766097963 | 55759.53827298053 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 15633.543466747371 | 17285.238022059737 | 18096.363268257413 | 24269.4531687243 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)
