# Benchmark report: dns-responder / dns_query_latency

- run id: adbbc5b7e9e8994c72b9c47e
- family: dns-responder
- benchmark: dns_query_latency
- evidence: maintained
- substrate: native
- commit: 83bcc400a3177e4473f07f74166d1bb8755a71a5
- dirty: true
- methodology: 1
- seed: 16019628498247202406
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
| client_latency_ns | responder | 38818.1970383062 | 45280.11274611399 | 47761.65781357882 | 55365.65930666669 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 14069.763983208804 | 15794.144732824429 | 16661.94942020983 | 23227.0872874494 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: a-record-1000-names-4-clients-8000qps

scheduled-arrival load of 8000 qps for 5 seconds from 4 client sockets; query shape A on resolvable names; name table of 1000 names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 8000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 99847.6415904228 | 108399.68631322125 | 111799.99544002074 | 121586.88317190598 | 1200000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 77278.98108729137 | 85699.06694618198 | 90785.78775187673 | 97777.39085980973 | 1200000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: aaaa-no-data-100-names-1-client-2000qps

scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape AAAA on a resolvable name that has no AAAA record (no data); name table of 100 names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 2000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 47987.53243447481 | 55454.880447256146 | 58389.47154929577 | 69483.77982089555 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 23049.09973415659 | 25716.178757655292 | 26674.800132934528 | 36220.16000000001 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Profile: unknown-name-1-client-2000qps

scheduled-arrival load of 2000 qps for 5 seconds from 1 client socket; query shape A on a name that is not in the name table (name error); name table of 100 other names

### Table: client-observed

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| client_throughput_qps | responder | 2000 | 0 | 30 | 0 |

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| client_latency_ns | responder | 40569.52945908981 | 46396.389079878674 | 48983.459325533375 | 56651.72514672689 | 300000 | 30 | 0 |

percentiles of client_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

### Table: resolver-internal

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| resolver_service_ns | responder | 15816.50273764527 | 17700.313195201747 | 18586.386544068173 | 25076.879243856343 | 300000 | 30 | 0 |

percentiles of resolver_service_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)
