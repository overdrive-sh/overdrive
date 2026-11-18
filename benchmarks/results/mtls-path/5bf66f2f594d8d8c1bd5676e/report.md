# Benchmark report: mtls-path / mtls_hop

- run id: 5bf66f2f594d8d8c1bd5676e
- family: mtls-path
- benchmark: mtls_hop
- evidence: maintained
- substrate: native
- commit: 20d93006b48313772b9a28a167f5bd711d9410f1
- dirty: true
- methodology: 1
- seed: 8118437145321509051
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
- replicates ok: 180
- replicates failed: 0
- status: finished
## Profile: egress

closed-loop (each request waits for its response before the next is sent; no scheduled arrival) over the frame protocol, plaintext workload dialler to a mesh peer on loopback; 10 warm-up then 100 sequential connections (connect, first byte, handshake), then one connection of 200 warm-up and 10000 timed 64-byte round trips (tail latency) and one connection carrying one 256 MiB frame (throughput). Baseline arm plaintext-baseline: the same dialler, bound the same way, connects straight to a plaintext TCP peer with no intercept worker, no nft rules and no TLS. Arm composed-mtls: the dialler's flow is registered with the real intercept worker and captured on leg F, which runs mTLS over kTLS to a rustls mesh peer that requires a client certificate; peer tickets disabled; handshake is the enforce duration; resource rows are whole-host scope (the aggregate cpu line of /proc/stat over every CPU and every process, no cgroup figure; each ratio is the class's tick delta over the delta of all CPU time, idle included, user includes nice) sampled across the 256 MiB throughput window; irq and softirq are tick-sampled on this kernel (irq_time_accounting is off, so irq reads 0 and softirq is coarse); memory (VmRSS) and open sockets of the benchmark process are taken at the window's end; connection_concurrency is the peak number of client sessions the family held open (opened and not yet dropped) during that window; forward_userspace_cpu_ratio and return_splice_cpu_ratio are the mtls-out-fwd and mtls-out-ret threads' share of the CPU the process's threads outside the family's own ovbm- threads accrued in that window; the window carries the request one way, so the return pump is the light one; thread CPU is read in 10 ms ticks, so the split is a lower bound at the tick (record note pump_heavy_to_light_lower_bound is the heavy-to-light ratio) and a return_splice_cpu_ratio of 0 means below one tick, because the sustained window carries data forward only; the TLS mesh peer runs on the same host as the agent and may co-limit throughput and tail latency, so the numbers are not pure agent cost

### Table: connection-establishment

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| connection_establishment_ns | plaintext-baseline | 18778 | 30045 | 31858.549999999992 | 44269.00999999993 | 3000 | 30 | 0 |
| connection_establishment_ns | composed-mtls | 28828 | 50496 | 54082.49999999999 | 61795.09999999995 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| connection_establishment_ns | composed-mtls | 10085 | [9128, 10663] | [-467.25, 1472.0124999999998] | 877.5 | 0.00009999000099990002 | distinguishable |

### Table: handshake

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| handshake_ns | plaintext-baseline | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | 0 | 0 | 0 |
| handshake_ns | composed-mtls | 2845528 | 3810798 | 3846882.55 | 3895272.91 | 3000 | 30 | 0 |

### Table: first-byte

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| first_byte_ns | plaintext-baseline | 118238 | 142215.9 | 150937.55 | 170167.80999999994 | 3000 | 30 | 0 |
| first_byte_ns | composed-mtls | 2962834 | 3943423 | 3970096 | 4010519.6299999994 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| first_byte_ns | composed-mtls | 2846570 | [2839801.75, 2849769.75] | [-4867.5, 12171] | 900 | 0.00009999000099990002 | distinguishable |

### Table: throughput

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| throughput_bytes_per_second | plaintext-baseline | 2672264472.3266754 | 201291023.23103523 | 30 | 0 |
| throughput_bytes_per_second | composed-mtls | 868124784.0403786 | 211316692.1009586 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| throughput_bytes_per_second | composed-mtls | -1804139688.2862968 | [-1976807496.1770353, -1756167218.5120645] | [-148034719.66704845, 34000735.75480598] | 0 | 0.00009999000099990002 | distinguishable |

### Table: tail-latency

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| tail_latency_ns | plaintext-baseline | 17383.59314442762 | 28277.85053045187 | 29329.80633365981 | 31892.501717921536 | 300000 | 30 | 0 |
| tail_latency_ns | composed-mtls | 64635.22551733228 | 84352.24860184861 | 88904.14083388484 | 97910.70957393486 | 300000 | 30 | 0 |

percentiles of tail_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| tail_latency_ns | composed-mtls | 48837.72746004097 | [40471.810631482585, 54337.097076181664] | [-2976.53749418654, 6050.87875442511] | 900 | 0.00009999000099990002 | distinguishable |

### Table: host-cpu

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| host_cpu_user_ratio | plaintext-baseline | 0 | 0.006163371960917973 | 30 | 0 |
| host_cpu_user_ratio | composed-mtls | 0.016276855569814133 | 0.00386789834403237 | 30 | 0 |
| host_cpu_system_ratio | plaintext-baseline | 0.07883916618856378 | 0.010116927237944454 | 30 | 0 |
| host_cpu_system_ratio | composed-mtls | 0.0835715955776746 | 0.004474210872033632 | 30 | 0 |
| host_cpu_irq_ratio | plaintext-baseline | 0 | 0 | 30 | 0 |
| host_cpu_irq_ratio | composed-mtls | 0 | 0 | 30 | 0 |
| host_cpu_softirq_ratio | plaintext-baseline | 0.013423423423423425 | 0.0049145385002477785 | 30 | 0 |
| host_cpu_softirq_ratio | composed-mtls | 0.018998890478163808 | 0.003771230138951958 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| host_cpu_user_ratio | composed-mtls | 0.016276855569814133 | [0.01006277389972177, 0.017094329284996802] | [-0.005774479131997604, 0.004311548666387376] | 870 | 0.00009999000099990002 | distinguishable |
| host_cpu_system_ratio | composed-mtls | 0.004732429389110823 | [0.0012427326432393748, 0.009495622148230368] | [-0.008064676793587503, 0.0018520662113670483] | 674.5 | 0.0010998900109989002 | not distinguishable from noise |
| host_cpu_irq_ratio | composed-mtls | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| host_cpu_softirq_ratio | composed-mtls | 0.005575467054740383 | [0.002147549464526592, 0.007392847894145314] | [-0.0036189539102102174, 0.0016281604008283016] | 773 | 0.00009999000099990002 | distinguishable |

### Table: memory

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| memory_bytes | plaintext-baseline | 19568640 | 2386944 | 30 | 0 |
| memory_bytes | composed-mtls | 21080064 | 2432000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| memory_bytes | composed-mtls | 1511424 | [-36864, 2967552] | [-1374208, 1562624] | 674 | 0.0004999500049995 | not distinguishable from noise |

### Table: sockets

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| socket_count | plaintext-baseline | 6 | 0 | 30 | 0 |
| socket_count | composed-mtls | 11 | 0 | 30 | 0 |
| connection_concurrency | plaintext-baseline | 1 | 0 | 30 | 0 |
| connection_concurrency | composed-mtls | 1 | 0 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| socket_count | composed-mtls | 5 | [5, 5] | [0, 0] | 900 | 0.00009999000099990002 | distinguishable |
| connection_concurrency | composed-mtls | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |

### Table: pump-cpu

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| forward_userspace_cpu_ratio | plaintext-baseline | absent: the plaintext baseline arm has no mTLS pump threads, so there is no forward or return work to split | absent: the plaintext baseline arm has no mTLS pump threads, so there is no forward or return work to split | 0 | 0 |
| forward_userspace_cpu_ratio | composed-mtls | 1 | 0 | 30 | 0 |
| return_splice_cpu_ratio | plaintext-baseline | absent: the plaintext baseline arm has no mTLS pump threads, so there is no forward or return work to split | absent: the plaintext baseline arm has no mTLS pump threads, so there is no forward or return work to split | 0 | 0 |
| return_splice_cpu_ratio | composed-mtls | 0 | 0 | 30 | 0 |

## Profile: ingress

closed-loop (each request waits for its response before the next is sent; no scheduled arrival) over the frame protocol, client to the node's inbound path on loopback with an in-memory guest end; 10 warm-up then 100 sequential connections (connect, first byte, handshake), then one connection of 200 warm-up and 10000 timed 64-byte round trips (tail latency) and one connection carrying one 256 MiB frame (throughput). Baseline arm plaintext-baseline: a plaintext client to a loopback listener that bridges each connection to the same in-memory guest sink, with no TLS and no enforcement. Arm composed-mtls: a rustls client presenting a node-trusted certificate to the real HostMtlsEnforcement serving Routed::Inbound over loopback leg C; the guest transport is excluded, and first byte includes the client side of the TLS handshake; resource rows are whole-host scope (the aggregate cpu line of /proc/stat over every CPU and every process, no cgroup figure; each ratio is the class's tick delta over the delta of all CPU time, idle included, user includes nice) sampled across the 256 MiB throughput window; irq and softirq are tick-sampled on this kernel (irq_time_accounting is off, so irq reads 0 and softirq is coarse); memory (VmRSS) and open sockets of the benchmark process are taken at the window's end; connection_concurrency is the peak number of client sessions the family held open (opened and not yet dropped) during that window; forward_userspace_cpu_ratio and return_splice_cpu_ratio are absent because the relay is asynchronous on shared runtime threads; handshake_ns starts when the enforcement accepts the connection, so it includes the wait for the client's ClientHello (client-side TLS work); the TLS client runs on the same host as the agent and may co-limit throughput and tail latency, so the numbers are not pure agent cost

### Table: connection-establishment

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| connection_establishment_ns | plaintext-baseline | 21667.5 | 33580.1 | 35564.99999999999 | 43475.69999999998 | 3000 | 30 | 0 |
| connection_establishment_ns | composed-mtls | 25147 | 31819 | 37237.49999999999 | 43717.18999999997 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| connection_establishment_ns | composed-mtls | 3618 | [1722.375, 4732.25] | [-3254.5, 263.25] | 633 | 0.007399260073992601 | distinguishable |

### Table: handshake

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| handshake_ns | plaintext-baseline | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | 0 | 0 | 0 |
| handshake_ns | composed-mtls | 820410 | 1043243.1 | 1098252.95 | 1402035.0999999999 | 3000 | 30 | 0 |

### Table: first-byte

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| first_byte_ns | plaintext-baseline | 71763 | 88714 | 97036.95 | 118563.69999999987 | 3000 | 30 | 0 |
| first_byte_ns | composed-mtls | 948412.5 | 1168053.0999999999 | 1245350.55 | 1540376.259999999 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| first_byte_ns | composed-mtls | 874333.75 | [870232.5, 880633.3125] | [-7100.75, 3149.75] | 900 | 0.00009999000099990002 | distinguishable |

### Table: throughput

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| throughput_bytes_per_second | plaintext-baseline | 1031304623.8565831 | 210535749.3109331 | 30 | 0 |
| throughput_bytes_per_second | composed-mtls | 1206124184.771479 | 146184286.56122303 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| throughput_bytes_per_second | composed-mtls | 174819560.91489577 | [10073473.935791492, 220459373.58450776] | [-100010353.61998019, 98686685.38843171] | 631 | 0.0065993400659934 | distinguishable |

### Table: tail-latency

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| tail_latency_ns | plaintext-baseline | 27645.944544619466 | 38448.813277529225 | 41636.75538545653 | 45481.785276664 | 300000 | 30 | 0 |
| tail_latency_ns | composed-mtls | 38168.94735680388 | 58277.40903225808 | 68160.91408964185 | 79897.73414847165 | 300000 | 30 | 0 |

percentiles of tail_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| tail_latency_ns | composed-mtls | 9154.87537238602 | [7886.583851394779, 13693.965103605673] | [-2642.3404769453555, 2435.0379356924336] | 837 | 0.00009999000099990002 | distinguishable |

### Table: host-cpu

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| host_cpu_user_ratio | plaintext-baseline | 0.03959962528510916 | 0.005530007999207352 | 30 | 0 |
| host_cpu_user_ratio | composed-mtls | 0.06069928524058654 | 0.010433927609361397 | 30 | 0 |
| host_cpu_system_ratio | plaintext-baseline | 0.08108108108108109 | 0.004196346480128163 | 30 | 0 |
| host_cpu_system_ratio | composed-mtls | 0.09803716376078331 | 0.013575132295435818 | 30 | 0 |
| host_cpu_irq_ratio | plaintext-baseline | 0 | 0 | 30 | 0 |
| host_cpu_irq_ratio | composed-mtls | 0 | 0 | 30 | 0 |
| host_cpu_softirq_ratio | plaintext-baseline | 0.011148386295157332 | 0.002044362377746571 | 30 | 0 |
| host_cpu_softirq_ratio | composed-mtls | 0.005517755757492732 | 0.003020588681649154 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| host_cpu_user_ratio | composed-mtls | 0.021099659955477384 | [0.016319631482194008, 0.02518690037301411] | [-0.001243879568159155, 0.0029359918389691335] | 886 | 0.00009999000099990002 | distinguishable |
| host_cpu_system_ratio | composed-mtls | 0.016956082679702222 | [0.012778256927573813, 0.022137432916392003] | [-0.002404610864724266, 0.0025218119352310026] | 890 | 0.00009999000099990002 | distinguishable |
| host_cpu_irq_ratio | composed-mtls | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| host_cpu_softirq_ratio | composed-mtls | -0.0056306305376646005 | [-0.007334903203932975, -0.004324121650663468] | [-0.001131126937578551, 0.0012092211360726392] | 15.5 | 0.00009999000099990002 | distinguishable |

### Table: memory

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| memory_bytes | plaintext-baseline | 20996096 | 352256 | 30 | 0 |
| memory_bytes | composed-mtls | 21008384 | 232448 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| memory_bytes | composed-mtls | 12288 | [-28672, 118784] | [-106496, 106496] | 543 | 0.1640835916408359 | not distinguishable from noise |

### Table: sockets

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| socket_count | plaintext-baseline | 7 | 0 | 30 | 0 |
| socket_count | composed-mtls | 8 | 0 | 30 | 0 |
| connection_concurrency | plaintext-baseline | 1 | 0 | 30 | 0 |
| connection_concurrency | composed-mtls | 1 | 0 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| socket_count | composed-mtls | 1 | [1, 1] | [0, 0] | 900 | 0.00009999000099990002 | distinguishable |
| connection_concurrency | composed-mtls | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |

### Table: pump-cpu

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| forward_userspace_cpu_ratio | plaintext-baseline | absent: asynchronous relay on shared runtime threads | absent: asynchronous relay on shared runtime threads | 0 | 0 |
| forward_userspace_cpu_ratio | composed-mtls | absent: asynchronous relay on shared runtime threads | absent: asynchronous relay on shared runtime threads | 0 | 0 |
| return_splice_cpu_ratio | plaintext-baseline | absent: asynchronous relay on shared runtime threads | absent: asynchronous relay on shared runtime threads | 0 | 0 |
| return_splice_cpu_ratio | composed-mtls | absent: asynchronous relay on shared runtime threads | absent: asynchronous relay on shared runtime threads | 0 | 0 |
