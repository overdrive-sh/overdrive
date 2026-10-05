# Independent review: kernel module versus Aya vhost-vsock benchmark

## Review metadata

- Feature: `netns-density-295`.
- Date: 2026-10-05.
- Reviewer: independent Codex reviewer, GPT 6.1 Sol with extra-high reasoning, as explicitly requested for this review.
- Iteration: 1.
- Authoritative report: [kernel-vs-aya-benchmark.md](kernel-vs-aya-benchmark.md).
- Evidence and executable boundary: [standalone benchmark](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/README.md), principally the completed `increment-r` corrected UDP cohorts and the retained matching `increment-c`/`increment-e` TCP cohorts.
- Verdict: **APPROVED for the documented measured benchmark boundary.**
- Blocking findings: **none**.

Loaded the local `nw-researcher-reviewer` definition and its `nw-rr-critique-dimensions` skill, and applied the repository's spike, evidence, and review ownership rules. The user-authorized metal/private-KVM substrate and C loadable-module comparator supersede the generic spike rules' Lima and language defaults. The review's only written file is this native Markdown artifact. No source, report, production, DESIGN, roadmap, DES, or original evidence was changed; no commit, kernel execution, matrix rerun, mutation test, or additional agent was started.

## Decision and exact scope

The corrected Aya candidate **works under the measured pressure conditions** on the unchanged stock `7.0.0-29-generic` private host-role kernel. It forwards the sampled IPv4 UDP application sizes **0, 64, 1,431, and 59,000 bytes** through real vhost-vsock owners, preserves the checked application bytes and message boundaries, and uses Aya Rust BPF for the proxy payload path. Its two directional connections belong to the same actual CID, vhost FD, memory context, and virtqueue pair. Userspace performs socket/map/lifecycle control and owns the workload endpoints; it does not relay application payload between proxy socket legs.

The six completed corrected UDP cohorts establish three independently provisioned AB/BA/AB pairs with the explicitly reported partial matrix: all five held populations, 384 valid open-loop windows, 30 empty-message request/reply windows, and 90 exact idle intervals. The unrestricted whole-population 16,384-peer windows have **16,384 actual participating peers and 16,384 simultaneously pending application requests**. This is stronger than the four outstanding requests in the separately qualified TCP workload, and is correctly distinguished from simultaneous kernel queue occupancy.

Approval does not convert this synthetic endpoint benchmark into ordinary guest networking, a production attachment implementation, or a general technology peak. The report explicitly records unmeasured intermediate 0/59,000-byte open-loop cells, full UDP maximum, IPv6, arbitrary multi-peer server semantics behind one CID, guest stack composition, incoming TCP/SSH, TLS/offloads, and separately timed loader/global-BPF-teardown latency. Those are accurate coverage limits, not new work required for this benchmark. Scale to zero remains a documentary consideration; no lifecycle policy or cold-start claim is approved here.

## Findings and disposition

### No blocking correctness, method, or provenance finding

The current bounded sources and original native receipts support the reported corrected result. The original Aya SEQPACKET pressure failure remains a negative result, separately pinned and excluded from valid performance rankings. The correction does not reinterpret that failure as harmless UDP loss, overwrite its captured bytes, discard its partial prefix, or claim success for the original transport arrangement.

No hypothetical production failure, architecture extension, public API change, or expanded matrix is prescribed by this review. The numeric zero corruption/boundary fields alone were not accepted as proof: the reviewer inspected the actual complete-byte assertions, framing decoder, pending-nonce removal, failure propagation, and valid-window selection, and reconciled the completed raw events independently.

### Interpretation bounds retained with approval

- The report's `0..59,000` shorthand is read as the **enumerated sampled sizes**, not every integer payload size. Its coverage table states those samples explicitly. The mechanism has not established the full 65,507-byte IPv4 UDP contract.
- Nonempty timed deliveries have exact CID/nonce/pattern checks and same-window reply duplicate rejection. Genuine empty messages have counts and pending-queue bounds, without fabricated nonce bytes or open-loop latency. Old messages consumed during settling have endpoint/framing/pattern validation and separate telemetry; their individual nonce inventories are not retained as an exhaustive historical delivery ledger. Do not infer an eventual loss-free or exhaustive exactly-once claim from that telemetry.
- Tail latency concerns the delivered subset. The reported cutoff losses remain losses at that cutoff even when a stream subsequently drains intact old messages. Steady CPU efficiency excludes the explicitly separate settling phase; complete harness-cohort CPU supplies the broader scope.
- The initial all-peer burst deliberately delays endpoint receive service until every shard has offered its first message to every eligible owner. Thus the pending-peer result is actual application concurrency under that declared burst workload, rather than a claim about an ordinary guest schedule or every socket being full concurrently.
- The common generator and codec are inside the CPU budget. Busy polling, byte generation, framing, descriptor service, and implementation-specific wire overhead prevent these measurements from identifying isolated proxy CPU efficiency or streaming peaks.
- Live stock module usage zero establishes the observed transport-reference drain before poweroff. It is not a promise that cached slab pages return exactly to baseline or a complete kernel heap leak proof.

These qualifications agree with the report and source-selection receipts. They do not authorize contract weakening or additional implementation.

## Source and payload-path review

### Frozen originals and bounded changes

Independently read each of the 11 frozen upstream files from the stated Git commit and compared it with both its retained copy and SHA256 pin: **11 matches, zero mismatches**. The pins identify:

| Comparator | Frozen commit |
| --- | --- |
| C module | `99e1b28e1822bb37e55c5fa2143f6c7c8f7c9bdb` |
| Aya TCP | `42f5d3689e7f038019b2d41c2d8c092898294bec` |
| Original Aya zero-capable UDP | `3ee2a6efa2b394942bf0a48cade1bc8cc543a74c` |

The module's actual bounded adaptations are the 300→3,600-second receive deadline and matching IPv4 no-check UDP transmission policy. Its original per-session relay workers remain. The BPF changes from the frozen small-population UDP program expand maps and the owned port range, constrain counters to their declared indices, and remove timed witness writes. The final corrected UDP BPF bytes are identical to `increment-k`; its new arrangement is in socket/map control and the owning synthetic endpoint codec.

All six executable source files checked between `increment-q` and `increment-r` are byte-identical: `src/main.rs`, `src/peer.rs`, `src/open_udp.rs`, `module/shmproxy.c`, `tcp-bpf/src/main.rs`, and `udp-bpf/src/main.rs`. All **31** final executed-manifest files independently match their retained hashes, including the final source, locks, matrix, and provenance. The corrected UDP path is explicitly labeled `aya-duplex-stream-seqpacket-v1`, rather than silently merged with original-candidate rows.

### One actual owner and two directional connections

[Peer::new](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/src/peer.rs) at line 74 opens `/dev/vhost-vsock`, sets owner/features, registers its actual mmap as guest memory, installs both queues and their eventfds, sets its actual CID, and starts the device. The same `Peer` owns both direction-specific flow entries. `connect_reverse` at line 261 adds type 2/port 29501 while the original type 1 connection uses port 29500; receive headers are checked against the owned CID, ports, and type at lines 219–236. This is not 16,384 aliases of one device or multiple fictitious labels on four owners.

[Owner::new](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/src/main.rs) at lines 262–319 creates the ordinary app/proxy UDP sockets, establishes and accepts both real vsock connections, and registers three SockHash keys and two source-cookie routes via `register_duplex` at line 182. `audit` at line 485 exchanges two-direction bytes with every owner, counts distinct live vhost FDs and memory bases, checks SockHash occupancy, and observes the complete expected IP/port tuples in `/proc/net/udp`. The six 16,384-owner end audits observe 49,152 occupied Aya keys and 32,768 source-cookie routes; the extra connection and map cost is included in the measured resource results.

### Complete payload path, including empty UDP

[Aya Rust BPF](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/udp-bpf/src/main.rs) at lines 14–29 looks up the source cookie, prefixes ordinary app-to-proxy payloads with `ZUD1` plus their length, and redirects them through SockHash to the accepted STREAM socket. It validates reverse SEQPACKET prefixes before redirecting to the ordinary UDP proxy socket. Lines 31–57 own the complete registered IPv4 tuple transform and remove the prefix before ordinary app delivery, updating lengths and the IPv4 checksum while retaining the declared zero UDP checksum policy. All BPF is Aya Rust; the `.c` source is the expressly permitted loadable comparator, not C BPF.

The ordinary app uses its own `send_to`/`recv_from`. The synthetic guest actor reads/writes its own shared virtqueue endpoint memory. The controller never performs a payload receive from an accepted proxy vsock socket or proxy UDP socket followed by a payload send to its other leg. Inspecting all of `main.rs`, `peer.rs`, and `open_udp.rs` establishes this boundary independently of the literal diagnostic `controller_proxy_payload_reads_writes: 0`.

The guest-side [STREAM decoder](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/src/open_udp.rs) at lines 25–57 accumulates segmentation and consumes exact magic/declared-length frames. It does not search for a later magic marker, discard an incomplete prefix, or resynchronize through the original failure. Full nonzero patterns, CID, nonce, expected size, and source tuple are checked in `identity` and `drain_ready` at lines 18–24 and 66–157. At line 142, an unoffered or duplicate same-window reply fails pending-nonce removal. Ordinary UDP receives allocate 65,536 bytes; a short or wrong-sized timed receive fails. Zero messages remain empty on both application endpoints, with private wire framing supplied by the forwarding mechanism.

The [common pressure preflight](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/src/main.rs) at lines 634–670 queues 70 real empty messages, 70×1,431-byte messages, and 3×59,000-byte messages before guest consumption, then checks every request and reply. The latter two exceed the 65,536-byte advertised transport-credit window including framing. Each of the six native cohorts records all three passed pressure cases. Two actual peers, actual zero/nonzero receives, normal traffic after zero, and a separate live ownership-removal negative are also exercised. Mapping removal is precisely the removed source-cookie route for Aya or closed module session for the module; target-SockHash removal is not claimed as that negative.

### Original pressure failure and corrected mechanism

The [original diagnostic](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-i/evidence/00-aya-udp/native-serial.log.gz), original serial line 1012, retains 2,220 assembled bytes with `ZUD1` prefixes at offsets 0 and 781. The second frame has the same CID/nonce/application pattern. Its line 1026 is a failed fixture window and is excluded; line 1028 observes live stock references zero.

The independent [direct no-BPF native reproduction](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-l/evidence/00-aya-udp/native-serial.log.gz), lines 930–934, records 45 successful 1,439-byte direct nonblocking sends, 64,755 bytes consumed from the 65,536-byte credit allowance, then `-1/EAGAIN` while the actual guest queue receives a 781-byte non-EOM prefix. Retrying emits the entire 1,439-byte message with EOM. Its [executed source](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-l/src/direct_seq.rs) creates only the actual test endpoints, loads no BPF, verifies the received prefix and full retry, and normally drains the owner. This preserves the qualification: reproduced stock nonblocking AF_VSOCK SEQPACKET behavior, not proof that every Aya arrangement is invalid.

The report's primary-source explanation is corroborated: upstream [v7.0 AF_VSOCK send](https://github.com/torvalds/linux/blob/v7.0/net/vmw_vsock/af_vsock.c#L2023) returns partial progress for STREAM but suppresses a partial positive SEQPACKET return unless the whole message is sent; [sockmap backlog](https://github.com/torvalds/linux/blob/v7.0/net/core/skmsg.c#L629) retains its offset on EAGAIN and advances it on positive progress; [generic skb send](https://github.com/torvalds/linux/blob/v7.0/net/core/skbuff.c#L3102) uses nonblocking sends. These references explain the correction. The Ubuntu native receipts, rather than an assumption that its binary equals upstream source, establish the observed behavior.

The corrected host-to-guest STREAM permits stock partial byte progress and explicit guest-owned framing; the reverse connection retains SEQPACKET into the atomic UDP socket path. The [bounded corrected increment-n receipt](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-n/evidence/00-aya-udp/native-serial.log.gz) independently reconciles to 36 open windows and **726,277 sent, guest-delivered, and app-delivered messages**, followed by live reference zero at line 1107. Increment-r then measures the separately pinned candidate under the final common harness. Neither candidate uses a kernel patch, rebuilt/forked kernel, or kernel-module assistance in its Aya cohorts.

## Method and independent numeric reconstruction

### Common timing and load

The final [UDP harness](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/src/open_udp.rs) prepares epoll state and settles old endpoint traffic before signaling readiness. At lines 343–365 the controller reads raw whole-VM CPU, starts all actors from a shared `Instant`, waits for all completion messages, and reads ending CPU before joins, latency sorting, full window-result formatting/UART serialization, or inventories. Actor completion metadata and the completion handoff remain inside the common CPU interval. Line 311 starts each nonempty latency sample **before** the actual `send_to`. All four shards are inside the same interval.

Send scheduling is independent of reply completion, with actual offered, sent, delivered, send backpressure, pending, late, and cutoff-loss counts. Large all-peer conditions at lines 310–314 include a real first offer per actual peer and a shared service barrier. The report explains that a guaranteed burst or the codec/generator can constrain the achieved load. Corrected load plots use actual sends over the full measured traffic-plus-delivery interval, while keeping the declared offering duration and target rate. No target rate is substituted for achieved throughput.

Same-window failures are caught, cancel peer work, return owners, mark the window invalid, and prevent advancement to another successful comparison window. `settle_population` at line 410 separately retains whole-VM late/control CPU before audit, profiling, and population transitions. Late previous-window messages in a timed interval invalidate timing. The failed o/p fixture attempts and their completed/failed rows remain outside the final r comparison.

### All completed raw counters and quantiles

Re-parsed all six original final serial captures independently of the author's audit and CSVs. Three module-drain printk insertions required removing exactly the known interleaved console line before strict JSON parsing; all JSON characters and numeric values were preserved. There were no other malformed event exclusions in the final six captures.

| Original native observation | Independent total |
| --- | ---: |
| Valid open-loop windows | 384 |
| Empty-message request/reply windows | 30 |
| Exact idle intervals | 90 |
| Start/end population audits | 60 |
| Offered / sent messages | 11,375,085 / 11,375,085 |
| Guest deliveries by cutoff | 11,126,720 |
| Guest replies submitted | 11,126,720 |
| Ordinary app replies by cutoff | 11,126,713 |
| Host→guest / guest→host cutoff losses | 248,365 / 7 |

Every accepted open window has valid timing, no actor failure, no old-epoch message inside timing, and consistent sent/delivered/pending counts. All large all-peer burst windows have the reported whole-population participating and maximum-pending counts. Nonempty latency vectors reconstructed from the retained exact base/deltas have one sample per actual app reply; every p50, p95, and p99 matches the reported sorted-array index `floor((N−1)×p)`. Empty open-loop windows have no fabricated samples or defined p99. The cutoff losses are retained rather than erased or treated as byte corruption.

### Recomputed 16,384-peer / 59,000-byte unrestricted UDP results

These are medians across three independent private kernels per variant. Goodput counts verified application bytes in both directions, using actual delivered counts and measured duration. CPU comes from recorded whole-VM busy ticks divided by its recorded 100Hz clock, not controller process time or system-wide task-clock.

| Metric | C module | Corrected Aya |
| --- | ---: | ---: |
| Delivered application GiB/s | 0.583038430 | 0.882939470 |
| Mean busy cores | 3.398018492 | 3.993447014 |
| Core-seconds / delivered GiB | 5.828120956 | 4.522288105 |
| Delivered datagrams/s, both directions | 10,610.724531 | 16,068.627749 |
| Delivered request/reply p99, ms | 2,604.842302 | 1,567.941388 |
| SUnreclaim delta from loaded baseline, GiB | 5.347141266 | 2.794113159 |
| KernelStack delta, MiB | 768.015625 | 256.046875 |
| Process PSS delta, MiB | 411.0546875 | 418.90234375 |
| Owned peer retirement, seconds | 256.464969400 | 334.343576556 |
| Elapsed to live-zero receipt, seconds | 257.502410537 | 335.332130728 |

These independently reproduce the report's rounded figures and min/max envelopes. The original native unrestricted-window serial lines are 1228/1228/1227 in the module's 00/03/04 cohorts and 1231/1227/1228 in Aya's 01/02/05 cohorts. No load, byte, or VM-budget normalization was changed to obtain the result.

### TCP retained independently

The original c module and e Aya captures each contain 216 accepted TCP windows. Their extracted `window()` source, including whitespace through the next function boundary, is byte-identical and matches the recorded digest `f099c14827328c859311fe9c94d09bbd197c81efe87176c3a7db092d34e6ffad`. Source comparison confirms the same TCP options, pacing, actor shards, timer boundaries, and workload; e's repairs concern the preflight owner choice, UDP bookkeeping, and post-traffic teardown. Both accepted TCP captures have successful byte/backpressure/half-close and mapping-removal gates.

Independently reconstructing the three 16,384-held-peer, 65,536-byte unrestricted TCP windows gives module/Aya median GiB/s **0.194646715 / 0.969992253**, busy cores **0.796493896 / 3.038145574**, core-seconds/GiB **3.937994065 / 3.121841278**, and p99 milliseconds **6.202813 / 0.978722**. The report correctly labels three measured windows within each held cohort, four maximum outstanding request/reply operations, and outgoing initiation. It does not claim three independent TCP VM pairs or 16,384 simultaneous TCP requests.

## CPU, memory, lifecycle, and native-substrate evidence

Primary private-kernel busy CPU sums user, nice, system, IRQ, and softIRQ once across all four CPUs. This includes BPF execution on application threads, module relay workers, stock vhost work, other kernel workers, endpoint actors, and control executed in the interval. Idle/iowait/steal are excluded from busy time, with raw fields retained; guest fields are not added twice. The native software and PMU outputs are present, including measured cycles/instructions in the inspected final windows, but are not substituted for busy CPU. System-wide task-clock covers a different scope and can include idle CPU time.

Exact idle at [main.rs line 820](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/src/main.rs) reads raw stat immediately around sleep. Inventory scans and serialization occur outside the interval. The report excludes contaminated original c/e idle snapshots from idle CPU comparison. Zero observed Aya busy ticks remains explicitly bounded by 100Hz accounting; it is not literal zero execution.

Memory tables derive separate native Slab/SUnreclaim, KernelStack, process PSS/RSS, page-table/vmalloc and socket observations from their declared baselines/stages. Slab and SUnreclaim overlap; socket memory can overlap kernel allocations; process PSS is not kernel memory. The author does not sum these into a fictitious total. Allocation, pre-settle traffic, settled traffic, idle, and retirement snapshots remain distinct. Additional Aya connections/maps and module workers are included in their own measured implementation costs.

Setup measures the actual new-owner batch through its first checked two-direction application operation. Retirement removes owned mappings/routes, sends resets, closes sockets/eventfds, stops vhost, and unmaps memory at [main.rs lines 198–230 and 943–951](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/src/main.rs). The exact owned-operation timer stops before inventory formatting. `rmmod` completion is separately timed where present; global BPF release is not independently timed. The later live-zero timer includes intermediate capture work and is correctly labeled accordingly.

All six final native kernels report `vhost_vsock` usage 0 **before** ordinary poweroff. Original serial line pairs for owned-retirement/live-zero are:

| Cohort | Owned retirement | Live zero |
| --- | ---: | ---: |
| 00 kernel | 1235 | 1238 |
| 01 Aya | 1238 | 1239 |
| 02 Aya | 1234 | 1235 |
| 03 kernel | 1235 | 1238 |
| 04 kernel | 1234 | 1237 |
| 05 Aya | 1235 | 1236 |

The [executed runner](../../../../spike-scratch/netns-density-295-kernel-vs-aya-benchmark/increment-r/run.py) uses `-accel kvm`, 16GiB, and `4,maxcpus=64` at line 85; each phase creates and boots its own private kernel. Original begin records verify `0-3` online and `0-63` possible CPUs. Stock images/modules are copied and hashed; only the comparator module is built against stock headers. The init script loads the experimental comparator inside module cohorts and no comparator in Aya cohorts. No experimental load or kernel/sysctl modification on the physical host appears in the execution path.

Physical QEMU process CPU, host CPU/steal/resource samples, and shared cgroup samples remain separate secondary scopes. The 0.5-second sampling and UART marker interpolation are correctly qualified as approximate; shared cgroup CPU is not attributed entirely to QEMU. Independently compared the captured physical before/after/post-release module and link inventories: **all identical**. The post-release receipt records no owned QEMU and no canonical lease owner.

## Archive provenance, failed attempts, and artifact boundary

Independently decompressed and hashed every record in `compressed-evidence-index.json`: **107 captures, zero byte-count or compressed/uncompressed SHA256 mismatches**. The retained final private archive is present, matches its original retrieval digest `67a79287abf7579e01b9eaf520981f3718a888f9df5c5514c2d3715412d538ac`, and its six native serial members plus executed source/artifact pin manifests match the retained captures byte-for-byte. The final sources match their executed manifest. Original init scripts, QEMU commands, matrix, locks, build output receipts, kernel/BTF/ELF pins, and phase retrieval receipts establish the source/build/native chain.

The final raw `result.json` still contains dispatcher defaults (`maximum: 64` and the two-variant default order). This is not used as the executed matrix: the retained `run-matrix.executed.json`, all six actual begin/native-result records, and `matrix-completion-receipt.json` establish the six 16,384-owner cohorts. The discrepancy is explicitly documented instead of altering historical metadata.

The source/data selectors keep original and corrected candidates distinct and exclude pilots, compile failures, invalid windows, and original contaminated idle CPU. The original interrupted j capture independently has no retirement or live-zero event; recovery is correctly labeled process/lease/physical-inventory cleanup rather than a live native drain. Failed original UDP, compile attempts, diagnostic i/l, and corrected fixture attempts m/o/p remain preserved. Final r results do not silently replace them.

Examined the 1,341 currently non-ignored candidate files under the benchmark tree for ELF/MZ signatures and generated binary/image/object/bytecode extensions: **none**. Build products, images, private raw tar archives, Python caches, and local uncompressed capture views remain outside candidate tracked scope. Sources, scripts, original compressed captures, provenance, derived tables, and SVG plots are the retained artifact boundary.

## Research-review dimensions and final verdict

| Dimension | Assessment |
| --- | --- |
| Source selection | Both bounded comparators are explicitly frozen and measured under a common harness; candidate changes and negative evidence are retained. Generic external-citation diversity quotas do not replace paired native measurement. |
| Evidence quality | Original native receipts, runtime/build/source pins, complete counters, stored latency samples, and original archive readback support the claims; no narrated success substitutes for execution. |
| Replicability | Standalone sources, locks, fresh-increment instructions, matrix, command/init captures, substrate/configuration, raw counters, and derivation scripts are retained. The qualified metal fixture is required. |
| Priority validation | The measured work addresses the requested kernel-module/Aya comparison and working stock-kernel Aya UDP under pressure. Generator limits and comparator-specific costs are identified rather than assigned to an imagined production mechanism. |
| Completeness | The authorized measured matrix is complete; remaining cells and unsupported product/transport semantics are explicitly unmeasured. No broader matrix or architecture is inferred as required. |

**Iteration 1 verdict: APPROVED.** No remediation is required for the bounded benchmark. Its corrected Aya UDP proof, qualified original negative, matching TCP comparison, resource/lifecycle observations, and final cleanup are supported by independently reconciled source and native evidence. This approves the reported empirical boundary and does not approve production DESIGN, full UDP semantics, guest composition, or a scale-to-zero implementation.
