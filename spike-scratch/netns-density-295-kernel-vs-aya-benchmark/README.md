# Common kernel-module/Aya benchmark

Standalone SPIKE. Experimental code runs only inside owned private stock
`7.0.0-29-generic` QEMU/KVM host-role VMs on the qualified metal fixture, under
its canonical exclusive lease. No production crate is linked or changed.
Synthetic guest endpoint drivers use actual independent started vhost devices,
CIDs, memory tables and virtqueues. These are not guest OS boots.

## Operator entry points

Use a **fresh increment/output identity**. Existing increments and receipts are
immutable. Copy a source increment while omitting evidence, build outputs and
Python caches, then execute its selected matrix:

```sh
python3 spike-scratch/netns-density-295-kernel-vs-aya-benchmark/launch.py increment-new --matrix run-matrix.json
uv run --with matplotlib python spike-scratch/netns-density-295-kernel-vs-aya-benchmark/analyze.py increment-new
```

`launch.py` acquires the canonical lease, syncs sources, executes `run.py` on the
qualified host and retrieves original evidence before another sync. Immutable
native evidence directories are excluded from rsync. The private raw tar archive
is ignored; its SHA-256 and sanitized byte-identical captures are retained.
`retrieve-phase.py` can retrieve a completed phase while later phases continue.
`compress-evidence.py` preserves exact native bytes in deterministic gzip files,
with source/archive hashes. Uncompressed local views are ignored.

`analyze.py` creates CSV/JSON tables and standalone SVG plots from original data.
`compare.py` selects the compatible c/e TCP and original UDP/idle datasets;
`compare-corrected.py` separately selects the matched increment-r corrected UDP
dataset, including candidate labels. Selection and exclusions are recorded
explicitly. No pilot or failed window is
substituted for a valid comparison point.

## Variants and shared configuration

`upstream-pins.json` pins the reviewed commits and exact original source bytes in
`upstream/`. Cargo locks, stock kernel/module/BTF hashes, compiled ELF hashes,
executed source manifest, init script, QEMU command and run matrix are retained.

A is the reviewed C module. Its receive idle fixture deadline is extended from
300 to 3600s to keep earlier population holders alive. The UDP scratch socket
uses the same IPv4 no-check transmit policy as the Aya codec. Its existing relay
worker architecture is preserved. B uses the reviewed Rust Aya TCP stream
SockHash program and the reviewed zero-capable UDP prefix SK_SKB + TCX program;
UDP map capacity/owned port range are extended and timed witness writes disabled.
There is no C eBPF, libbpf, kernel patch, new worker pool or production API.

Each private VM has 16GiB RAM, four online/64 possible CPUs, stock PID/thread
limits, queue size 8, and 256KiB guest memory per peer. Application sockets use
the same buffer requests and TCP options; proxy TCP buffers/NODELAY retain native
defaults. TCP initiation is outgoing. UDP compares one registered IPv4 flow per
transport peer, using 127.0.0.2/.3 peers and 127.0.0.1 proxy sockets. Full IPv6,
UDP maximum length, guest ordinary socket transparency, SSH and mTLS are unproved.

Controllers configure, map and signal lifecycle. They never read/write a proxy
socket's application payload. Endpoint actors exclusively own their messages and
private codecs; the guest response is independently generated and verified.

## Timers, activity and CPU

Populations are 1, 64, 1024, 4096 and 16384 actual transport owners. Every pool is
held and checked by CID-tagged two-direction traffic, distinct live vhost FDs,
unique mappings, occupied BPF entries, kernel UDP bindings and task inventories.
Sources keep the whole pool alive through condition windows; idle expiry is not
capacity evidence.

TCP request/reply: 64/1024/65536 application bytes, paced 100/1000 and unrestricted
four-shard closed loop, three repeats. At most four requests are outstanding.
Pool eligibility is not 16k concurrent activity. These are request/reply results,
not sustained-stream peaks.

UDP: 0/64/1431/59000 application bytes. Nonempty messages include nonce/CID inside
the stated payload; zero messages remain genuinely empty and have count/loss
load measurements plus separate closed-loop latency. Open-loop send pacing is
independent of reply completion. Actual offered/sent/delivered counts, eligible
and participating peers, and concurrent outstanding requests are separate fields.
Late messages are validated and consumed outside measurement using tagged
endpoint fences. A late prior epoch during a window invalidates its timing;
a same-epoch duplicate or byte/boundary mismatch fails the fixture. Worker failure
propagates while retaining owners for normal retirement.

Primary traffic CPU: raw whole-VM `/proc/stat` immediately around all endpoint
work, including all user/kernel/IRQ/softIRQ/vhost/module/BPF work on all four CPUs.
Normalize non-idle user/nice/system/irq/softirq ticks by recorded CLK_TCK, excluding
idle/iowait/steal and avoiding guest-field double-counting. Full stat data and
`perf stat -a` software/PMU output are retained. System-wide cpu/task-clock includes
online CPU time and is not busy CPU. Generator user/system CPU is diagnostic;
inline BPF CPU is never excluded.

Exact idle CPU uses stat/sleep/stat with inventory and serialization outside.
The original c/e idle snapshot intervals are excluded because inventory scans
contaminated them. Kernel Slab/SUnreclaim/KernelStack/PageTables/Vmalloc, socket
statistics and process PSS/RSS are separate observations. Physical QEMU/process,
shared-cgroup and whole-host/steal scopes are secondary and never silently merged
with private-kernel CPU. No unmatched baseline is subtracted as proxy CPU.

Retirement starts before owned routes/maps, socket close/reset, vhost stop,
eventfd close and memory unmap. Private module unload is normal and timed where
instrumented. Stock transport reference zero must be observed in the live private
kernel. Poweroff or process disappearance is only VM-lifetime cleanup if no live
zero receipt exists.

## Attempt index and original failure

- a: initial completed 1/64 integration pilot; excluded (pacing/preflight).
- b: canonical sync failure, no native probe.
- c: valid module TCP through16k; Aya negative test used a half-closed peer and
  failed before timed work; no UDP execution.
- d: JSON macro recursion compile fallout, no VM.
- e: corrected matching Aya TCP through16k; UDP accounting panic. Owned failed
  QEMU cleanup was explicitly authorized, with PID/start/argv/signal receipt.
  No live-reference-zero claim for that failed phase.
- f: prepared measurement repair, not executed.
- g: compile fallout at a changed UDP caller, no VM.
- h: exact idle and intermediate UDP. Module reached16k; Aya pressure boundary
  validation failed at1024. Both retired normally with live references zero.
- i: bounded failure diagnostic. A 781-byte partial prefix is followed by the
  complete same1439-byte frame (same nonce/CID); normal retirement, live zero.
- j: environment-interrupted first idle cohort, partial original evidence archived.
  Recovery audit found no owned process/lease and unchanged physical inventory.
- k: fresh safe capacity/idle/low-load continuation after j recovery.

The original Aya UDP pressure failure is a boundary/byte correctness violation,
not harmless best-effort loss. All original failing sources and bytes remain
available. Corrected results never replace the original candidate silently.

## Corrected Aya candidate in this same benchmark

- l directly reproduces hidden partial SEQPACKET send progress with no BPF:
  1439-byte nonblocking send returns EAGAIN after the real queue receives 781 bytes;
  retrying emits the entire same frame. Normal cleanup proved live references 0.
- m preserves the first corrected fixture's borrow-check failure; no VM launched.
- n implements `aya-duplex-stream-seqpacket-v1` and passes 36 open-loop windows at 64
  actual peers, sizes 0/64/1431/59000, loads 1000/10000/unrestricted, three repeats.
  All 726277 datagrams reached each endpoint direction with no counted loss,
  corruption or boundary violation; normal retirement proved live stock usage 0.
- o retained valid module full-population data and a corrected Aya outside-window
  fence timeout; p proved intact retained backlog needed10.53s, then exposed a
  missing final-window settle before audit. Both original failures remain.
- q passed the corrected population-one pressure/final-audit path, with separate
  settling CPU and live-reference-zero cleanup.
- r completed the matched corrected comparison: three independently provisioned pairs
  in AB/BA/AB order, all five actual held populations, one repeat per independent
  VM cohort. Original TCP c/e rows remain unchanged.

The corrected candidate uses two actual vsock connections on each actual CID,
vhost FD, memory context and virtqueue pair. Host-to-guest STREAM port 29500 uses
unchanged 8-byte prefix/length encoding and exact endpoint-owned message parsing;
partial positive send progress now advances the stock sockmap byte offset.
Guest-to-host SEQPACKET port 29501 retains the original Rust SK_SKB/TCX decoder.
Guest ports and credit/fwd counters are distinct per direction. Setup owns three
SockHash keys and two source-cookie routes per peer. BPF source is byte-identical
to the original increment-k candidate. The extra sockets/maps and endpoint codec
work remain inside the same measured CPU/memory budget. No kernel patches,
module assistance or userspace application-payload relay are used.

The common pressure preflight queues 70×1431 or 3×59000 application bytes before
actual guest consumption, exceeding the 65536-byte transport credit window, then
checks all exact requests/replies. It also queues 70 genuine empty datagrams and
checks subsequent ordinary messages. This gate is identical for both variants.

At 1024/4096/16384 eligible-all activity, one datagram is offered per actual peer
before a shared endpoint-service barrier, creating measured simultaneous pending
requests across the population. Subsequent independent paced/unrestricted sends
and receives record loss and delivered-subset latency without waiting for each
reply. Maximum pending peers is an application request count; it does not claim
all kernel queues are simultaneously full. Small-subset activity uses 64 peers.

Low populations cover all four sizes and target 1000/10000/unrestricted load;
intermediate populations cover 64/1431 bytes at 10000/unrestricted; maximum population
covers all sizes at 10000/100000/unrestricted, small-subset and whole-population.
Missing intermediate 0/59000 traffic cells are explicitly unmeasured. Idle and
memory/audits cover every population. Common epoll_wait0 polling is retained for
both, so paced CPU cost is generator/control dominated; unrestricted goodput is
a complete-pipeline measurement, not an isolated proxy peak.

A steady window uses the original offering deadline plus at most two-second
delivery drain. Loss is not-delivered-by-cutoff, with actual late validated
messages retained separately. The common endpoint-owned settlement precedes
profiling, audits and population transitions, and has a separate whole-VM CPU
interval. Whole-harness cohort CPU also captures control/serialization/idle/
retirement work; it is not isolated forwarding CPU.

After a completed native matrix and original evidence retrieval:

```sh
uv run --with matplotlib python spike-scratch/netns-density-295-kernel-vs-aya-benchmark/analyze.py increment-r
uv run --with matplotlib python spike-scratch/netns-density-295-kernel-vs-aya-benchmark/compare-corrected.py
python3 spike-scratch/netns-density-295-kernel-vs-aya-benchmark/compress-evidence.py
python3 spike-scratch/netns-density-295-kernel-vs-aya-benchmark/post-release-audit.py
```

Scale-to-zero is a documentary implication only: compare allocated idle cost,
retirement to live references 0 and eventual re-provision cost. No additional
scale-to-zero implementation or experiment is part of this benchmark.

Final increment-r receipt audit: six passed private VM cohorts, 384 valid open
windows, 30 empty-message request/reply windows, 90 idle intervals; all five real
populations audited at interval start/end; all normal live reference-zero receipts.
No owned QEMU or canonical lease owner remains. Physical modules/links unchanged.

`result.json` maximum/order fields are dispatcher CLI defaults when `--matrix` is
used. The authoritative executed matrix, per-cohort begin/native-result records
and `comparison/corrected-udp/matrix-completion-receipt.json` identify the actual
six 16,384-owner cohorts. Original raw metadata is preserved unchanged.

Final verification and report derivation:

```sh
python3 spike-scratch/netns-density-295-kernel-vs-aya-benchmark/audit-corrected.py
python3 spike-scratch/netns-density-295-kernel-vs-aya-benchmark/report-corrected.py
```
