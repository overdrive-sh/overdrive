## Corrected UDP comparison: completed native evidence

**WORKS for the measured shared contract:** stock-kernel Aya forwarding of genuine empty and 0..59,000-byte IPv4 UDP messages, intact nonzero boundaries and identities, at 16,384 actual transport owners. No kernel patches, module assistance or userspace application-payload relay. The corrected directional candidate remains distinct from the frozen failing SEQPACKET candidate.

Increment-r completed **six independent private VM cohorts**, AB/BA/AB: **384 valid open-loop windows**, 30 real-empty-message request/reply windows, 90 exact idle intervals and start/end live audits at all five populations. All six normal teardowns observed stock references **0 inside the live private kernel**. The independent receipt audit has zero failed checks. The native launcher took **72.3 minutes**, including build/bootstrap/retrieval. No production, design or DES files were changed by this benchmark; no commits were made.

### All 16,384 peers participating and simultaneously pending

These unrestricted UDP conditions include the initial all-peer burst and four endpoint actor shards. Every window observed 16,384 participating and 16,384 maximum pending peers. Values are medians [min–max] across three independently provisioned cohorts. Throughput counts verified application bytes/datagrams in both directions. Tail latency describes delivered request/reply messages; the burst is part of this workload.

| Application bytes | Module delivered datagrams/s | Corrected Aya delivered datagrams/s | Module CPU µs/delivered datagram | Corrected Aya CPU µs/delivered datagram | Module p99 ms | Corrected Aya p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 65,063 [64,618–65,328] | 54,487 [53,843–54,593] | 51.6 [51.5–51.9] | 56.0 [55.3–56.0] | — | — |
| 64 | 50,768 [49,784–52,055] | 68,979 [68,653–71,660] | 67.2 [65.9–68.1] | 48.3 [47.5–48.8] | 815.4 [779.0–846.4] | 366.2 [361.5–366.4] |
| 1,431 | 47,299 [45,704–47,356] | 68,170 [67,589–70,644] | 72.8 [72.3–74.0] | 51.8 [51.4–52.1] | 855.1 [829.1–952.9] | 389.4 [388.4–411.2] |
| 59,000 | 10,611 [9,937–10,973] | 16,069 [15,918–16,111] | 320.2 [312.4–327.3] | 248.5 [247.9–250.9] | 2,604.8 [2,487.0–2,879.6] | 1,567.9 [1,558.6–1,586.0] |

At 59,000 bytes: module **0.583 [0.546–0.603]GiB/s**, **3.40 [3.25–3.43] busy cores**, **5.83 [5.69–5.96]core-s/GiB**; corrected Aya **0.883 [0.875–0.885]GiB/s**, **3.99 [3.99–3.99] busy cores**, **4.52 [4.51–4.57]core-s/GiB**. These are complete measured endpoint/kernel paths under the same four-vCPU budget, including codec and generator cost; they are not isolated proxy efficiencies or technology peaks.

All the tabled unrestricted all-peer conditions had zero counted deadline loss, corruption and boundary violations. Other saturation conditions did have missing deliveries at the cutoff, retained below. Empty open-loop messages have count/loss evidence and undefined latency; the separate ordinary empty-message request/reply windows retain actual latency samples.

### Allocated memory, idle and lifecycle at 16,384 UDP owners

| Component / operation | Module | Corrected Aya |
| --- | ---: | ---: |
| Kernel SUnreclaim increase from loaded baseline, GiB | 5.347 [5.347–5.347] | 2.794 [2.794–2.794] |
| KernelStack increase, MiB | 768.0 [768.0–768.0] | 256.0 [256.0–256.0] |
| Process PSS increase, MiB | 411.1 [411.0–411.2] | 418.9 [418.2–419.3] |
| Process FDs | 98,308 [98,308–98,308] | 147,474 [147,474–147,474] |
| Process tasks, including stock vhost workers | 16,385 [16,385–16,385] | 16,385 [16,385–16,385] |
| All private-kernel tasks | 49,233 [49,233–49,233] | 16,466 [16,466–16,467] |
| Idle busy cores, nine exact intervals | 0.385 [0.315–0.410] | 0.000 observed [0.000–0.005] |
| Activation of new 12,288 owners while growing 4,096→16,384, seconds | 27.47 [27.30–28.88] | 5.11 [5.05–5.32] |
| New endpoints/s in that activation batch | 447 [426–450] | 2,404 [2,309–2,435] |
| Per-peer owned retirement, seconds | 256.46 [256.39–257.54] | 334.34 [333.51–341.02] |
| Live-zero proof elapsed from retirement start, seconds | 257.50 [257.43–258.59] | 335.33 [334.46–342.06] |

Memory components are separate native observations, never an invented combined total. Zero idle CPU is an observed 100Hz tick delta, not literal zero execution. The live-zero receipt timer includes intermediate inventory/serialization; the per-peer control timer stops before that capture. Corrected Aya owns three occupied SockHash keys and two actual vsock connections per CID, **49,152 occupied keys at 16,384**, while retaining one actual vhost FD/memory context/queue pair per CID. Its additional connection/map cost is included.

### Saturation loss, long queues and generator limits

| 59,000B, one active/held peer, target 10,000 sends/s | Module | Corrected Aya |
| --- | ---: | ---: |
| Actual sends/s over total measured traffic+delivery duration | 2,675 [2,671–2,878] | 2,501 [2,455–2,729] |
| Not delivered host→guest by cutoff, datagrams | 7,480 [7,446–8,119] | 5,863 [5,727–6,543] |
| Delivered-subset p99, ms | 12.7 [10.7–12.8] | 2,728.3 [2,721.6–2,778.8] |

Across all 384 open windows: **11,375,085 sent**, **11,126,720 delivered to guest consumers by cutoff**, **11,126,713 delivered replies by cutoff**. Not-delivered-by-cutoff counts are **248,365 host→guest** and **7 guest→host**. These counts are not relabelled as corruption or erased. Late messages were validated during inter-window and post-population settling; its counts and separate whole-VM CPU are retained.

The common polling/service loop and byte-pattern generation consume much of the budget at some loads. In particular, a 59,000-byte target can exceed actual generator rate. Goodput is actual verified delivery over the actual measured interval; load plots use actual sends over the full measured traffic+delivery duration, with target/declared pacing duration preserved separately. No unmatched baseline is subtracted as proxy CPU.

### Coverage and remaining limits

| Population | UDP open-loop payloads | Activity | Load axes | Independent pairs |
| ---: | --- | --- | --- | ---: |
| 1 / 64 | 0 / 64 / 1431 / 59000 | Whole eligible pool | 1000 / 10000 / unrestricted | 3 |
| 1024 / 4096 | 64 / 1431 | 64-peer subset and all-peer burst | 10000 / unrestricted | 3 |
| 16384 | 0 / 64 / 1431 / 59000 | 64-peer subset and all-peer burst | 10000 / 100000 / unrestricted | 3 |

Allocated idle, memory, real binding/map/context counts, tagged start/end audits and empty-message request/reply samples cover all five populations. Intermediate 0/59000 open-loop cells were not run. This does not claim the entire Cartesian matrix, full 65507-byte UDP, IPv6, arbitrary multi-peer UDP-server behavior behind one CID, ordinary-guest-stack composition, incoming TCP/SSH, TLS or offloads. Module/BPF loader latency and global BPF teardown latency were not separately timed. Original c/e TCP three-window cohorts remain a separately scoped comparison, not three independent TCP VM pairs.

### Evidence and plots

- [Native receipt audit](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/native-validation.json)
- [Actual samples and all median/range tables](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/summary.csv)
- [Goodput versus whole-VM CPU](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/goodput-cpu.svg)
- [59,000-byte population/goodput/CPU/p99 plot](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/udp-59000-population.svg)
- [p95/p99 versus measured load at 16,384](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/udp-59000-load-16384.svg)
- [Separate memory-component curves](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/memory-population.svg)
- [Late/control settling CPU](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/posttraffic-settle.csv)
- [Complete harness-cohort CPU scope](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/cohort-accounting.csv)
- [Traffic/idle/retired memory snapshots](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/corrected-udp/memory-stages.csv)

The final physical audit found no owned QEMU process or canonical lease owner and unchanged physical module/link inventories. Three retirement JSON events had an inserted module printk line on the shared UART: exact known console bytes were deinterleaved with raw/recovered hashes, strict JSON parsing and no numeric edits; originals remain intact. No unrecoverable UART events or failed comparison checks remain.
