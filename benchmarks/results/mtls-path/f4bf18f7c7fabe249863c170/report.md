# Benchmark report: mtls-path / mtls_hop

- run id: f4bf18f7c7fabe249863c170
- family: mtls-path
- benchmark: mtls_hop
- evidence: maintained
- substrate: native
- commit: f0d059a53320d8b331b3f98d9ee26db00598e67a
- dirty: true
- methodology: 1
- seed: 7116310952086180854
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

closed-loop (each request waits for its response before the next is sent; no scheduled arrival) over the frame protocol, plaintext workload dialler to a mesh peer on loopback; 10 warm-up then 100 sequential connections (connect, first byte, handshake), then one connection of 200 warm-up and 10000 timed 64-byte round trips (tail latency) and one connection carrying one 256 MiB frame (throughput). Baseline arm plaintext-baseline: the same dialler, bound the same way, connects straight to a plaintext TCP peer with no intercept worker, no nft rules and no TLS. Arm composed-mtls: the dialler's flow is registered with the real intercept worker and captured on leg F, which runs mTLS over kTLS to a rustls mesh peer that requires a client certificate; peer tickets disabled; handshake is the enforce duration; resource rows are whole-host scope (the aggregate cpu line of /proc/stat over every CPU and every process, no cgroup figure; each ratio is the class's tick delta over the delta of all CPU time, idle included) sampled across the 256 MiB throughput window, with memory (VmRSS) and open sockets of the benchmark process taken at its end; forward_userspace_cpu_ratio and return_splice_cpu_ratio are the mtls-out-fwd and mtls-out-ret threads' share of the CPU the process's threads outside the family's own ovbm- threads accrued in that window, and because the window carries the request one way the return pump is the light one; thread CPU is read in 10 ms ticks, so the heavy-to-light ratio (record note pump_heavy_to_light_lower_bound) is a lower bound

### Table: connection-establishment

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| connect_ns | plaintext-baseline | 18823.5 | 30576 | 36613.04999999995 | 46581.509999999944 | 3000 | 30 | 0 |
| connect_ns | composed-mtls | 29224 | 50537 | 53460.99999999999 | 61163.399999999965 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| connect_ns | composed-mtls | 10385.75 | [9587.175, 11038.5] | [-550.75, 498.5] | 867 | 0.00009999000099990002 | distinguishable |

### Table: handshake

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| handshake_ns | plaintext-baseline | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | 0 | 0 | 0 |
| handshake_ns | composed-mtls | 2846485 | 3804305 | 3841733 | 3884329.61 | 3000 | 30 | 0 |

### Table: first-byte

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| first_byte_ns | plaintext-baseline | 124496.5 | 150005 | 159633.05 | 182394.4 | 3000 | 30 | 0 |
| first_byte_ns | composed-mtls | 2962804.5 | 3944800 | 3970887 | 4004116.9299999997 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| first_byte_ns | composed-mtls | 2836256.75 | [2830265.25, 2849396.2625] | [-8542.95, 12594] | 900 | 0.00009999000099990002 | distinguishable |

### Table: throughput

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| throughput_bytes_per_second | plaintext-baseline | 2648166235.1359296 | 191779572.1933298 | 30 | 0 |
| throughput_bytes_per_second | composed-mtls | 863688432.8865058 | 238542374.48338866 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| throughput_bytes_per_second | composed-mtls | -1784477802.2494237 | [-2003309887.858879, -1659134066.0750115] | [-54638541.7842741, 143443733.0258932] | 0 | 0.00009999000099990002 | distinguishable |

### Table: tail-latency

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| tail_latency_ns | plaintext-baseline | 23034.430992736077 | 29445.03640312073 | 30365.98294898672 | 36241.11886201992 | 300000 | 30 | 0 |
| tail_latency_ns | composed-mtls | 64895.241843188436 | 86597.29333882709 | 91833.63368421052 | 98498.86678542513 | 300000 | 30 | 0 |

percentiles of tail_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| tail_latency_ns | composed-mtls | 44746.30270472639 | [36515.60494225647, 49754.178235538806] | [-5892.088966410025, 293.60517155428533] | 900 | 0.00009999000099990002 | distinguishable |

### Table: host-cpu

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| host_cpu_user_ratio | plaintext-baseline | 0 | 0 | 30 | 0 |
| host_cpu_user_ratio | composed-mtls | 0.014433235127815969 | 0.004326801647317746 | 30 | 0 |
| host_cpu_system_ratio | plaintext-baseline | 0.07164242942686055 | 0.009594515990835314 | 30 | 0 |
| host_cpu_system_ratio | composed-mtls | 0.08305881279372176 | 0.003834001061686479 | 30 | 0 |
| host_cpu_irq_ratio | plaintext-baseline | 0 | 0 | 30 | 0 |
| host_cpu_irq_ratio | composed-mtls | 0 | 0 | 30 | 0 |
| host_cpu_softirq_ratio | plaintext-baseline | 0.012658227848101266 | 0.003702774317642146 | 30 | 0 |
| host_cpu_softirq_ratio | composed-mtls | 0.018808962264150943 | 0.0018235103820272089 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| host_cpu_user_ratio | composed-mtls | 0.014433235127815969 | [0.012728565258080773, 0.01588891956924776] | [0, 0] | 900 | 0.00009999000099990002 | distinguishable |
| host_cpu_system_ratio | composed-mtls | 0.01141638336686121 | [0.006544044044044045, 0.013914417899895365] | [-0.003081031879313023, 0.005352131681203978] | 752 | 0.00009999000099990002 | distinguishable |
| host_cpu_irq_ratio | composed-mtls | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| host_cpu_softirq_ratio | composed-mtls | 0.006150734416049677 | [0.005117941913607548, 0.006738323876036666] | [-0.0006745984056403479, 0.00413790527795067] | 762.5 | 0.00009999000099990002 | distinguishable |

### Table: memory

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| memory_bytes | plaintext-baseline | 19634176 | 1597440 | 30 | 0 |
| memory_bytes | composed-mtls | 21121024 | 1412096 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| memory_bytes | composed-mtls | 1486848 | [849920, 2236416] | [-606208, 733184] | 746 | 0.00009999000099990002 | distinguishable |

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

closed-loop (each request waits for its response before the next is sent; no scheduled arrival) over the frame protocol, client to the node's inbound path on loopback with an in-memory guest end; 10 warm-up then 100 sequential connections (connect, first byte, handshake), then one connection of 200 warm-up and 10000 timed 64-byte round trips (tail latency) and one connection carrying one 256 MiB frame (throughput). Baseline arm plaintext-baseline: a plaintext client to a loopback listener that bridges each connection to the same in-memory guest sink, with no TLS and no enforcement. Arm composed-mtls: a rustls client presenting a node-trusted certificate to the real HostMtlsEnforcement serving Routed::Inbound over loopback leg C; the guest transport is excluded, and first byte includes the client side of the TLS handshake; resource rows are whole-host scope (the aggregate cpu line of /proc/stat over every CPU and every process, no cgroup figure; each ratio is the class's tick delta over the delta of all CPU time, idle included) sampled across the 256 MiB throughput window, with memory (VmRSS) and open sockets of the benchmark process taken at its end; forward_userspace_cpu_ratio and return_splice_cpu_ratio are absent because the relay is asynchronous on shared runtime threads

### Table: connection-establishment

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| connect_ns | plaintext-baseline | 21587.5 | 32869 | 35373.99999999999 | 39921.79999999996 | 3000 | 30 | 0 |
| connect_ns | composed-mtls | 25438 | 31178.899999999998 | 35553 | 44408.089999999975 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| connect_ns | composed-mtls | 3518.25 | [-207.80624999999995, 5743.75] | [-4296.75, 1565.062499999999] | 615 | 0.0165983401659834 | not distinguishable from noise |

### Table: handshake

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| handshake_ns | plaintext-baseline | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | absent: the plaintext baseline arm has no mTLS enforcement, so there is no handshake to time | 0 | 0 | 0 |
| handshake_ns | composed-mtls | 822503.5 | 866027.1 | 1107218 | 1374894.5999999985 | 3000 | 30 | 0 |

### Table: first-byte

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| first_byte_ns | plaintext-baseline | 73004.5 | 89380.09999999999 | 96389.99999999997 | 124887.4 | 3000 | 30 | 0 |
| first_byte_ns | composed-mtls | 950681 | 1011994.1 | 1250656.5 | 1501076.9499999993 | 3000 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| first_byte_ns | composed-mtls | 879464 | [870050, 885535.81875] | [-8274.5, 6953.25] | 900 | 0.00009999000099990002 | distinguishable |

### Table: throughput

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| throughput_bytes_per_second | plaintext-baseline | 1028006437.4649534 | 137467079.79745626 | 30 | 0 |
| throughput_bytes_per_second | composed-mtls | 1128261670.4112356 | 171720297.22310853 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| throughput_bytes_per_second | composed-mtls | 100255232.94628215 | [43871366.63545489, 159461851.4599912] | [-50908118.18857527, 55743562.69772303] | 643 | 0.0031996800319968005 | distinguishable |

### Table: tail-latency

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| tail_latency_ns | plaintext-baseline | 28904.393992315752 | 36061.12628846867 | 37671.248397204145 | 42318.002641744555 | 300000 | 30 | 0 |
| tail_latency_ns | composed-mtls | 37740.7086689203 | 52040.15414488427 | 65629.68640892785 | 72853.5523303835 | 300000 | 30 | 0 |

percentiles of tail_latency_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| tail_latency_ns | composed-mtls | 7807.282265877238 | [7246.067739193225, 13048.78362822107] | [-2636.225722279247, 1655.681123570047] | 883 | 0.00009999000099990002 | distinguishable |

### Table: host-cpu

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| host_cpu_user_ratio | plaintext-baseline | 0.0408823011963406 | 0.004380106873153232 | 30 | 0 |
| host_cpu_user_ratio | composed-mtls | 0.058300536672629695 | 0.010924113996407248 | 30 | 0 |
| host_cpu_system_ratio | plaintext-baseline | 0.08097898377227986 | 0.003968070026418996 | 30 | 0 |
| host_cpu_system_ratio | composed-mtls | 0.09661892120540273 | 0.008385047071725188 | 30 | 0 |
| host_cpu_irq_ratio | plaintext-baseline | 0 | 0 | 30 | 0 |
| host_cpu_irq_ratio | composed-mtls | 0 | 0 | 30 | 0 |
| host_cpu_softirq_ratio | plaintext-baseline | 0.011280348144754924 | 0.0019918481949024857 | 30 | 0 |
| host_cpu_softirq_ratio | composed-mtls | 0.00556337919187855 | 0.0010859245564454542 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| host_cpu_user_ratio | composed-mtls | 0.017418235476289096 | [0.014335784847787532, 0.022003333029843253] | [-0.002508145633514207, 0.0018197090984600206] | 893 | 0.00009999000099990002 | distinguishable |
| host_cpu_system_ratio | composed-mtls | 0.015639937433122877 | [0.011813754781162797, 0.01858928771648123] | [-0.005820336725084405, 0.001112858438420461] | 856 | 0.00009999000099990002 | distinguishable |
| host_cpu_irq_ratio | composed-mtls | 0 | [0, 0] | [0, 0] | 450 | 1 | not distinguishable from noise |
| host_cpu_softirq_ratio | composed-mtls | -0.005716968952876374 | [-0.006371468887113441, -0.004665534940016659] | [-0.0007024562003781952, 0.0014544409688289454] | 5 | 0.00009999000099990002 | distinguishable |

### Table: memory

| metric | arm | median | IQR | replicates | failures |
|---|---|---|---|---|---|
| memory_bytes | plaintext-baseline | 20492288 | 136192 | 30 | 0 |
| memory_bytes | composed-mtls | 20504576 | 139264 | 30 | 0 |

| metric | arm | delta (composed-mtls minus plaintext-baseline) | 95% bootstrap interval | A/A 95% interval | Mann-Whitney U | p | verdict |
|---|---|---|---|---|---|---|---|
| memory_bytes | composed-mtls | 12288 | [-61440, 81920] | [-71680, 69632] | 503 | 0.44085591440855915 | not distinguishable from noise |

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
