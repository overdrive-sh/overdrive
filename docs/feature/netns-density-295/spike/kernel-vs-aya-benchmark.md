# Kernel module versus Aya: common vhost-vsock benchmark

**Status: corrected Aya UDP works for the measured contract; native comparison complete. Not independently reviewed.**

The matching original TCP runs remain complete and unchanged. The original Aya
UDP pressure failures and direct no-BPF reproduction remain immutable evidence.
The user authorized correction within this same benchmark. A stock-kernel Aya
candidate with two directional vsock connections per actual CID/device has now
passed 36 open-loop windows and 726,277 datagrams in each direction at 64 peers,
including genuine empty messages, without observed loss or boundary corruption.
No kernel patch, module assistance, or userspace application-payload relay was
used. The affected measurements were rerun in six independently provisioned private
VMs with separate source provenance. Original-candidate results remain intact.

## Scope and topology

One common standalone Rust endpoint harness exchanges application messages
through either the reviewed C kernel module or the reviewed Aya Rust programs.
The experimental module and BPF programs run only inside private stock Ubuntu
`7.0.0-29-generic` QEMU/KVM **host-role VMs**. The qualified physical metal host
runs QEMU and the build tools. Each private VM has 16GiB RAM, four online/64
possible CPUs, stock kernel PID/thread limits and the same vhost queue layout.

Each held peer is a real independent `/dev/vhost-vsock` owner, started CID,
registered guest-memory context and pair of shared virtqueues. These are
synthetic endpoint actors; no guest OS is booted. TCP uses outgoing host
connection initiation. Guest ordinary socket transparency, incoming SSH, mTLS,
kTLS, full UDP maximum size, IPv6 and offloads remain outside this measurement.
There is no userspace application-payload relay between proxy legs.

The TCP workload has four endpoint actor shards and at most four outstanding
request/reply operations. The pool is eligible for traffic; it does not represent
16,384 simultaneous TCP requests. UDP reports actual participating peers and
concurrent outstanding application requests, separately from the eligible pool.

## Corrected UDP comparison: completed native evidence

**WORKS for the measured shared contract:** stock-kernel Aya forwarding of genuine empty and 0..59,000-byte IPv4 UDP messages, intact nonzero boundaries and identities, at 16,384 actual transport owners. No kernel patches, module assistance or userspace application-payload relay. The corrected directional candidate remains distinct from the frozen failing SEQPACKET candidate.

Increment-r completed **six independent private VM cohorts**, AB/BA/AB: **384 valid open-loop windows**, 30 real-empty-message request/reply windows, 90 exact idle intervals and start/end live audits at all five populations. All six normal teardowns observed stock references **0 inside the live private kernel**. The separate executable receipt audit has zero failed checks. The native launcher took **72.3 minutes**, including build/bootstrap/retrieval. No production, design or DES files were changed by this benchmark; no commits were made.

### All 16,384 peers participating and simultaneously pending

These unrestricted UDP conditions include the initial all-peer burst and four endpoint actor shards. Every window observed 16,384 participating and 16,384 maximum pending peers. Values are medians [min–max] across three independently provisioned cohorts. Throughput counts verified application bytes/datagrams in both directions. Tail latency describes delivered request/reply messages; the burst is part of this workload.

| Application bytes | Module delivered datagrams/s | Corrected Aya delivered datagrams/s | Module CPU µs/delivered datagram | Corrected Aya CPU µs/delivered datagram | Module p99 ms | Corrected Aya p99 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 65,063 [64,618–65,328] | 54,487 [53,843–54,593] | 51.6 [51.5–51.9] | 56.0 [55.3–56.0] | — | — |
| 64 | 50,768 [49,784–52,055] | 68,979 [68,653–71,660] | 67.2 [65.9–68.1] | 48.3 [47.5–48.8] | 815.4 [779.0–846.4] | 366.2 [361.5–366.4] |
| 1,431 | 47,299 [45,704–47,356] | 68,170 [67,589–70,644] | 72.8 [72.3–74.0] | 51.8 [51.4–52.1] | 855.1 [829.1–952.9] | 389.4 [388.4–411.2] |
| 59,000 | 10,611 [9,937–10,973] | 16,069 [15,918–16,111] | 320.2 [312.4–327.3] | 248.5 [247.9–250.9] | 2,604.8 [2,487.0–2,879.6] | 1,567.9 [1,558.6–1,586.0] |

At 59,000 bytes: module **0.583 [0.546–0.603] GiB/s**, **3.40 [3.25–3.43] busy cores**, **5.83 [5.69–5.96] core-s/GiB**; corrected Aya **0.883 [0.875–0.885] GiB/s**, **3.99 [3.99–3.99] busy cores**, **4.52 [4.51–4.57] core-s/GiB**. These are complete measured endpoint/kernel paths under the same four-vCPU budget, including codec and generator cost; they are not isolated proxy efficiencies or technology peaks.

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

## Preliminary matching TCP observations

Application goodput counts the successfully verified bytes in both directions.
CPU is whole-private-VM busy CPU from `/proc/stat`: user, nice, system, IRQ and
softIRQ across all four virtual CPUs, including vhost, module/BPF work and the
endpoint actors. It excludes idle/iowait/steal and avoids counting guest fields
twice. System-wide perf software and PMU counters are retained separately.

The C increment-c and Aya increment-e TCP `window()` functions are byte-identical,
with the same pacing, socket options, timer/stat boundaries, actor shards,
completion joins and workload. The original source/artifact manifests and the
compatibility receipt preserve that evidence. The corrected independent
preflight passed for both before this pair is used.

At **16,384 held peers**, unrestricted **65,536-byte request/reply** operations:

| Observed metric | C module | Aya Rust |
| --- | ---: | ---: |
| Application GiB/s | 0.195 [0.193–0.202] | 0.970 [0.947–1.004] |
| Whole-VM busy CPU cores | 0.796 [0.738–0.807] | 3.04 [3.03–3.11] |
| CPU core-seconds / delivered GiB | 3.94 [3.83–4.15] | 3.12 [3.09–3.21] |
| Observed roundtrip p99 | 6.20ms [5.42–8.15] | 0.979ms [0.973–1.013] |

Values are medians and min–max across three measured windows in the held cohort.
This is the measured request/reply workload, not a sustained streaming peak.
At 64 held peers, 65,536-byte p99 is 43.6ms for the module and 41.8ms for Aya;
the CPU-per-GiB ranges overlap (module 6.68–6.96, Aya 5.98–7.13). The workload's
request/reply and TCP ACK/credit behavior affects those observations.

Allocated TCP memory and tasks at 16,384 peers, immediately before traffic:

| Native component | C module | Aya Rust |
| --- | ---: | ---: |
| Kernel SUnreclaim increase from loaded baseline | 5.39GiB | 2.79GiB |
| KernelStack increase from loaded baseline | 768MiB | 256MiB |
| Process PSS increase from loaded baseline | 648MiB | 645MiB |
| Process FDs | 98,308 | 131,084 |
| Process tasks, including stock vhost workers | 16,385 | 16,385 |
| All private-kernel tasks | 49,231 | 16,462 |

These components are separate observations; they are not summed into an invented
memory total. In particular, kernel memory is not process PSS. The module's two
additional relay workers per peer are a property of this prototype. Aya still
has stock vhost workers.

The module TCP native phase retired its owned peers in **258.491s** and observed
stock `vhost_vsock` usage **0 inside the live private kernel** before normal module
unload and VM shutdown. Aya's corresponding owned retirement took **249.350s**; its live-reference-zero
receipt completed at **250.298s** from retirement start. VM poweroff is not a reference-drain
proof.

## Original candidate: reproduced UDP pressure failure

The module's corrected UDP cohort reached all five populations through 16,384,
completed tagged live audits, exact idle CPU and 1024/4096 load conditions, and
retired normally to stock transport usage zero. The Aya cohort passed low-load
conditions through 1024, then unrestricted 1431-byte messages failed whole-frame
validation. The failed window is excluded from throughput/CPU/latency rankings.

A separate native diagnostic retained the assembled bytes. For CID 300044 and
nonce 244493, the buffer contains a **781-byte initial portion of a 1439-byte
encoded message**, followed immediately at offset 781 by the **entire same
1439-byte encoded message**. The second copy matches the independently generated
1431-byte application pattern. The 781-byte first portion equals `65536 % 1439`.

Increment-l independently reproduced the same behavior **without loading any BPF**:
45 direct nonblocking SEQPACKET sends accepted 64,755 bytes, leaving 781 bytes of
advertised peer credit. The next 1439-byte `send()` returned `-1/EAGAIN` while the
real guest queue received 781 bytes with no EOM. Once credits recovered, retrying
the same call sent the entire same 1439-byte message. This isolates the partial
progress to stock AF_VSOCK nonblocking SEQPACKET send behavior, not the UDP load
generator or its EOM decoder.

The upstream Linux v7.0 source corroborates the path: sockmap's egress backlog
uses a nonblocking socket send and retains its current offset on EAGAIN; vsock's
SEQPACKET send suppresses a partial positive return unless the whole message was
sent. These are reference-source facts; the actual Ubuntu binary behavior is
established by the independent native run. See
[vsock send implementation](https://github.com/torvalds/linux/blob/v7.0/net/vmw_vsock/af_vsock.c),
[sockmap backlog](https://github.com/torvalds/linux/blob/v7.0/net/core/skmsg.c) and
[generic skb socket send](https://github.com/torvalds/linux/blob/v7.0/net/core/skbuff.c).

Parsing only the approved explicit length would combine the partial first frame
with the beginning of its repeated copy and fail byte identity. Ignoring EOM,
discarding a partial prefix or adding resynchronization would not establish the
required unchanged message contract. The corrected candidate below changes the directional transport arrangement; it
does not conceal, resynchronize, or discard the original failure.

The diagnostic and failing cohort both returned their owners, retired normally
and observed stock `vhost_vsock` usage 0 inside the still-live private kernel.
The direct no-BPF diagnostic also retired to live stock reference usage zero.
These results establish a pressure correctness blocker for the tested prototype;
they do not show that all Aya or kernel-module approaches have this behavior.

- [Original boundary diagnostic](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-i/evidence/00-aya-udp/native-serial.log.gz)
- [Independent byte-pattern derivation](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-i/derived/udp-boundary-diagnostic.json)

## Corrected candidate: directional STREAM and SEQPACKET

The user explicitly authorized bounded framing/transport choices inside this
benchmark, with no kernel patches and no userspace application-payload proxy.
Increment-n implements `aya-duplex-stream-seqpacket-v1`: each real CID, vhost FD,
guest-memory context and virtqueue pair owns two actual vsock connections.
Host-to-guest delivery uses STREAM on host port29500 and the existing eight-byte
`ZUD1`/length prefix. Stock STREAM reports partial positive progress, allowing
sockmap's backlog to advance its byte offset. The owning synthetic guest endpoint
parses exact lengths and independently verifies every logical message.
Guest-to-host delivery uses a separate SEQPACKET connection on host port29501;
the existing Rust SK_SKB and TCX prefix decoder preserve ordinary UDP boundaries,
including zero-length messages. No controller reads or writes either proxy leg.

The Rust BPF source is byte-identical to increment-k. Setup/map ownership and the
owning guest codec change: three occupied SockHash keys, two source-cookie routes
and two vsock connection contexts per actual device replace the original two
keys/one vsock connection. Each direction has its own real port/type/credit state.
The corrected candidate's extra connection/map/memory/setup cost remains inside
the measured budget and is reported separately from the frozen original.

Increment-m's borrow-check failure is preserved. Increment-n compiled and ran
on the unchanged stock7.0.0-29 private kernel. Its 36 open-loop windows at64 held
peers cover sizes0/64/1431/59000 and target1000/10000/unrestricted load, three
repeats each. All726,277 sent datagrams were delivered to the guest and all726,277
independently generated replies reached the ordinary app, with zero counted
loss/corruption/boundary violations. The ordinary zero receive uses readiness and
a65536-byte buffer. Normal retirement observed live `vhost_vsock` usage 0.

Increment-r reran both module and corrected Aya across 1/64/1024/4096/16384 with
three independently provisioned AB/BA/AB pairs. A new matched preflight holds
actual guest consumption while70×1431-byte or3×59000-byte messages exceed the
65536-byte transport credit window, then checks every complete message/reply;
70 actual empty messages and subsequent normal traffic are also checked.
At large eligible-all activity, one request is offered per actual peer before a
shared endpoint-service barrier. Actual sent/delivered/loss, participating peers
and maximum pending peers are retained; the four-request TCP loop is unchanged.
Increment-o's first module cohort completed all planned traffic and live-zero
retirement. The corrected Aya population-one phase exposed an outside-window
five-second fence timeout after its fixed delivery cutoff. Increment-p proved
that 5,843 retained, byte-valid old messages required10.53 seconds to consume;
it then exposed missing final-window settling before the tagged audit.
Increment-q adds the same bounded endpoint-owned settling before audits,
profiling and population transitions. Its twelve population-one open-loop
conditions passed, the final audit passed and normal retirement proved live
references zero. The final settling interval took10.94 seconds and retains its
own whole-VM CPU evidence. These are common harness corrections; the Aya
transport/BPF framing source is unchanged from the working increment-n route.

Steady delivery stops at the original offering interval plus at most two seconds
of drain. “Loss” columns count messages not delivered by that cutoff; they must
be read with the retained late-message counts. Stream queues can retain intact
messages past the cutoff. Outside-window fences validate and consume those old
endpoint messages before the next phase. Their CPU/control/late-delivery cost is
reported separately; it is not silently included in steady goodput or discarded
as zero work. Complete harness-cohort CPU also retains loading, inventories,
serialization, control, idle and retirement costs, with a distinct scope.

The full increment-r corrected matrix completed with frozen executable source
from q, three independent AB/BA/AB pairs, and unchanged steady timer boundaries.

- [Direct no-BPF reproduction](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-l/evidence/00-aya-udp/native-serial.log.gz)
- [Correction source provenance](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-n/correction-provenance.json)
- [Bounded native validation](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-n/correction-validation.json)

## Exact allocated idle CPU and capacity

Three exact stat/sleep/stat intervals at 16,384 held peers give:

| Private-kernel condition | Median busy CPU cores | Min–max |
| --- | ---: | ---: |
| Module TCP | 0.380 | 0.375–0.380 |
| Module UDP | 0.370 | 0.365–0.385 |
| Aya TCP | 0.000 observed | 0.000–0.000 |
| Aya UDP | 0.000 observed | 0.000–0.000 |

Zero is the observed busy-tick delta at the recorded 100Hz accounting resolution
and approximately two-second interval, not proof of literally zero execution.
Inventories, FD/task scans and serialization occur outside these intervals.
All five populations have exact idle and memory/audit evidence for both protocols.
Both candidates also demonstrated 16,384 actual held UDP owners with tagged
64-byte two-direction traffic, unique contexts/FDs and kernel binding/map counts.

The corrected module UDP 16k cohort reached live reference usage zero at 256.869s
from retirement start. The safe Aya UDP 16k cohort's owned retirement took
230.329s. Other successful and failed-but-normally-retired cohorts retain exact
timers and live-reference receipts in the coverage artifacts.

## Frozen original UDP measurements and unavailable cells

Population64, target1000 ordinary sends/s, three independent AB/BA/AB VM pairs:
all nine windows per variant/payload delivered approximately2000 datagrams/s
across both directions with zero counted loss, corruption or boundary violations.

| Application bytes | Module observed roundtrip p99 | Aya observed roundtrip p99 |
| --- | ---: | ---: |
| 64 | 4.15ms | 1.96ms |
| 1431 | 4.25ms | 0.351ms |
| 59000 | 5.31ms | 1.29ms |

These open-loop runs include a common receive-service polling loop that consumes
most of the four-core budget even at the paced load. Their whole-VM CPU is
recorded but is generator/control dominated, and does not estimate isolated
adapter CPU efficiency. Zero-message open-loop latency is undefined; separate
real empty-message request/reply samples provide its latency evidence.

Matched 1024-peer small-active 64-byte conditions completed at target10k and
unrestricted load. The module also completed 1024/4096 1431-byte conditions;
Aya's unrestricted1431-byte1024 condition failed. The corresponding remaining
pressure, high-population load and large-payload population1 cells are unavailable
for a correctness-valid comparison. A later population1 59000-byte/1000 offered
Aya run reproduced the failure; it was retired normally and excluded. These
missing cells are not filled using earlier TCP receipts, idle snapshots or pilot
values, and the report does not claim the full all-five UDP load matrix passed.

## Correctness and bounded adaptations

The independent preflight uses two actual transport peers; tagged TCP bytes,
262,144 bytes in each direction under a 300ms RX-descriptor hold, half-close and
reverse bytes after guest FIN. UDP uses genuine ordinary receives into a
65,536-byte buffer with readiness and real address checks for messages including
0, 64, 1431 and 59,000 bytes, repeated empty messages and normal messages after
empty messages. Ownership removal has a separate still-live test peer.

The shared UDP comparison covers one registered IPv4 flow per transport peer.
The module can address arbitrary remote peers in its private frame; Aya's
measured codec uses a registered connected flow. Full UDP semantics beyond the
shared 0..59,000-byte workload are not waived or claimed.

The module's scratch idle deadline extends 300 to 3600 seconds so earlier owners
remain alive through the held-population windows. Aya UDP map capacity expands
for the population and disables per-packet witness writes during traffic. The
UDP fixture uses matching IPv4 no-check transmission policy, including the
module's scratch socket configuration, because the Aya codec reconstructs a
zero-checksum IPv4 UDP packet. These are recorded benchmark configuration and
instrumentation changes. The module worker structure stays unchanged. The
corrected Aya directional connection arrangement above is a separately pinned
experimental candidate authorized by the user.

## Measurement definitions and implementation overhead

The common budget and application workload are equal. The implementation codecs
retain their actual wire costs: module data has a 12-byte private frame header;
corrected Aya UDP has an 8-byte prefix. The synthetic guest module codec sends
encoded bytes in 32KiB STREAM chunks; the Aya return codec sends one complete
SEQPACKET record (at most 59008 bytes here). Both use the same 65536-byte virtqueue
packet buffers and queue size 8. Those codec/descriptor differences remain in the
whole-VM CPU measurements. These figures describe the tested complete endpoint
and kernel paths; they do not isolate adapter CPU or establish a technology peak.

| Measurement | Actual timer/scope |
| --- | --- |
| Population setup/activation | `Instant` before adding new owners through each new owner's first verified two-direction 64-byte message; reports new owners per second. Boot and module/BPF loader latency are not separately timed. |
| Allocated idle CPU | Three exact raw-stat / two-second sleep / raw-stat intervals per population and cohort; inventories and formatting outside. |
| UDP steady CPU/goodput | Prepared actor readiness, shared start and all-actor finish; includes the fixed delivery drain and batch service. Ending raw CPU read precedes formatting/inventories. Application bytes/counts use actual verified deliveries. |
| UDP load rates | Raw pacing duration is the declared offering interval; guaranteed all-peer bursts can extend it under generator saturation. Additional send/offered rates over the entire actual measured traffic+delivery duration are derived and used for load plots. Counts and target rates remain available. |
| Settling/late work | Endpoint-owned fences before the next phase; a separate whole-VM CPU interval for post-population settling. Complete harness-cohort CPU also captures unassigned control, inventory, serialization, idle and retirement work; initial module load before the harness baseline is excluded. |
| Per-peer retirement | Exact control timer through map/route removal, resets, socket/FD closure, vhost stop and memory unmap, stopped before its inventory/JSON capture. |
| Module unload | Separate exact `rmmod` completion timer. Global BPF/map/link drop is not independently timed. |
| Live reference-zero receipt | Actual stock transport usage 0 in the still-live private kernel after backend release. Its elapsed-from-retire timer includes the intermediate retirement inventory/receipt serialization, so it is not presented as a pure control-operation latency. |
| Physical QEMU/host/cgroup | Original 0.5-second QEMU CPU/resource and physical/cgroup samples. Window interpolation is approximate and UART marker arrival can lag; shared-cgroup CPU is not attributed solely to QEMU. These are separate from primary private-kernel busy CPU. |

Kernel memory remains separate from userspace PSS/RSS. Allocated, idle,
traffic-before-settling, settled traffic and retired snapshots are retained.
Retired slab pages may remain cached after owned references disappear; reference
usage 0 does not promise that `/proc/meminfo` returns byte-for-byte to baseline.
Raw perf software and available PMU counters are retained. Short profiling
reports locate cost; the temporary `perf record` binary is not retained.

Scale-to-zero is a documentary consideration: allocated idle cost, per-peer
retirement and subsequent activation all matter. No additional scale-to-zero
implementation or experiment was started.

## Exclusions and attempt provenance

| Increment | Disposition |
| --- | --- |
| a | Completed initial four-variant 1/64 pilot; low-population pacing incorrectly divided by four and full independent preflight was absent. Excluded from accepted comparison. |
| b | Canonical sync failed on root-owned prior evidence; no native execution. Retained original launcher receipt. |
| c | Valid module TCP traffic retained. Aya passed bytes/backpressure/half-close, then the negative fixture reused the half-closed peer and got EPIPE; no Aya timed rows and no UDP execution. Sources/raw evidence remain immutable. |
| d | Expanded UDP diagnostic JSON hit Rust's macro recursion limit at compile; no private VM launched. |
| e | Matching Aya TCP completed. Aya UDP actor failed on an unoffered/duplicate nonce and blocked its finish barrier. Explicitly authorized cleanup sent ordinary SIGTERM only to the verified owned QEMU PID/start identity; failed phase has VM-lifetime cleanup, no live-zero proof. |
| f | Prepared exact idle CPU/intermediate UDP correction; not executed. |
| g | Caller fallout compile failure after failure-propagation change; no private VM execution. |
| h | Corrected queue fences/failure propagation. Module UDP reached 16k; Aya failed whole-frame pressure validation at 1024 and retired normally to live references zero. |
| i | Bounded diagnostic independently reproduced the partial/repeated frame bytes; normal retirement and live references zero. |
| j | Environment interrupted the first Aya UDP idle cohort at 1024. Original partial evidence recovered; no completed native-result or live-zero claim. Read-only recovery found no owned process/lease and unchanged physical module/link inventory. |
| k | Completed safe16k Aya UDP, both exact TCP-idle cohorts and three64-peer low-load pairs; final population1 Aya59000 cell failed the preserved contract and retired normally. |
| l | Direct stock SEQPACKET no-BPF reproduction isolates partial/EAGAIN/retry behavior; normal live-reference-zero cleanup. |
| m | Corrected directional connection fixture reached borrow-check failure before native VM execution; original source and build receipt retained. |
| n | Corrected Aya passed all36 bounded64-peer open-loop windows and retired to live references zero. |
| o | Module completed all-five traffic/idle; corrected Aya population-one next-window fence timed out after59000 pressure, normal live-zero cleanup. Original valid/failed rows retained separately. |
| p | Longer fence succeeded: 5,843 intact old messages needed10.53s; missing last-window settling then caused final audit to see old traffic. Raw failure preserved, post-harness live stock usage 0 observed before shutdown. |
| q | Common final/population settling correction passed all12 population-one open-loop conditions, tagged final audit and normal live-zero retirement. |
| r | Frozen sources from q; completed six full-population AB/BA/AB cohorts,384 valid open windows, all live reference-zero receipts and unchanged physical inventory. |

All original c/e idle CPU intervals include the ending inventory scan and are
excluded from quiescent idle CPU comparison. The raw 16k c receipt spans 2.65s
for a declared 2s sleep and includes 0.62s controller system CPU. Its memory and
live-population evidence remain usable. The correction reads raw `/proc/stat`
immediately around the sleep, with inventories and serialization outside.

The planned c UDP timing path would have timestamped after `send_to` and included
per-window epoll setup inside the CPU interval. c stopped before reaching UDP;
no such UDP rows are accepted. Corrected e/f UDP timers include inline BPF send
cost and use all-actor barriers. Zero-size UDP load is count/loss only; its
latency comes from the separate real zero-message request/reply condition.

## Reproduction and retained evidence

The operator entry point and protocol are in the standalone benchmark tree:

- [Harness and protocol](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/README.md)
- [Reviewed upstream pins](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-e/upstream-pins.json)
- [TCP measurement compatibility](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-e/tcp-measurement-compatibility.json)
- [Module native phase and original counters](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-c/evidence/00-kernel-tcp/native-serial.log.gz)
- [Aya native phase and original counters](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-e/evidence/01-aya-tcp/native-serial.log.gz)
- [Module derived samples](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-c/derived/samples.csv)
- [Aya derived samples](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-e/derived/samples.csv)

Each completed phase's original archive is retrieved and hashed before any
subsequent source sync. Exact source/build pins, raw whole-VM perf/stat data,
physical QEMU CPU/status/FD samples, kernel warnings and cleanup receipts are
retained. Physical and private-kernel CPU scopes remain separate. Final coverage and measured/unsupported cells are derived from the completed
native datasets. Final read-only audit confirms no owned QEMU or canonical lease
owner and unchanged physical module/link inventories. Sources, raw byte hashes,
failed attempts and all retained original counters remain available.

- [Compatible combined samples](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/samples.csv)
- [Summary and variation](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/summary.csv)
- [Exact idle CPU](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/idle-summary.csv)
- [TCP64KiB goodput/CPU/tail plot](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/tcp-rr-65536-population.svg)
- [Memory components by population](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/memory-population.svg)
- [Idle CPU by population](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/comparison/idle-cpu-population.svg)
- [Final physical/lease audit](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/post-release-audit.json)
- [Lossless capture hashes](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/compressed-evidence-index.json)

## Documentary consideration: scale to zero

These measurements concern held, allocated transport peers. A demand-driven
allocation policy could change the fraction held idle and amortize setup or
retirement differently. No scale-to-zero policy, cold-start latency, production
lifecycle or new spike has been implemented or measured here. Its consequences
need a separately pinned design and workload assumptions; they are not inferred
from this allocated-population dataset.
