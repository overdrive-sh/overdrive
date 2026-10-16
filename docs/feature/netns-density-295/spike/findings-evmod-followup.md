# evmod resume: findings for items 0–4 (netns-density-295)

Overall verdict: items 0, 1 and 2 are complete. Item 3 is complete. Item 4 shows two defects in the unmodified module (a close_notify abort and a kTLS-then-rmmod panic). The gate recommendation is at the end.

## Inventory of the interrupted run (nothing redone)

| Run | Evidence | State |
|---|---|---|
| Host-role V-3 (`bench-v3`) | `evidence/bench-v3-…T161003Z` (complete). `…T160503Z` is a partial earlier run. | Complete (item 1) |
| Lifecycle, module `1eecbe01` (source `d5710d7c`) | `out/life-a/b/c.txt`: cut, rst, kill, kills, sclose, tight, unload, unloadf, unload-stress 30×3 | Complete, with one caveat (ev-cut-T below) |
| Leak audit with the `module-audit` build | `out/audit.txt`, `out/leak.txt` | Complete |
| Core battery A and T | `out/core-A.txt`, `out/core-T.txt` | Complete |
| Core battery G | `out/core-G.txt` | Stopped after ev-tiny-G, the last scenario to finish; fault1/2/3-G never ran. Finished by me. |
| Unbound at scale, kTLS | no evidence. `module-ktls/evmod.c` is byte-identical to `module/evmod.c` (`d5710d7c`). | Never run |
| Step 0 journal | not captured | Collected now |

Substrate: physical host `uname -r` is `7.0.0-38-generic`. The guest and QEMU host-role VMs run `7.0.0-29-generic`. `systemd-detect-virt` is `none` and `/dev/kvm` is API 12; the preflight failed closed on both every run. The module is loaded only inside the Cloud Hypervisor guest or the private QEMU VM. After my runs, `lsmod` on the physical host shows no `evmod`, `tls` or `shmproxy`, and no spike processes were left over (job `final-check`).

**Harness correction.** Every inc2 `result.json` says `physical_modules_unchanged=false`. The cause is a byte comparison of `/proc/modules` that includes field 3, the use count. Use counts move while a VM holds vhost_vsock (vsock 0–2, vhost 1). `driver/vmrun.py` now compares names and sizes and records any `evmod`, `tls` or `shmproxy` entry. Rerun result: `unchanged: True diff=[] evmod/tls/shmproxy=[]`. The preflight snapshots in all 11 earlier runs contain none of the three modules.

## Step 0: why the host went down

**Hypothesis:** the 14:45 CEST outage was caused by the spike (OOM, oops or load).
**Prediction:** a kernel message at the end of boot -1, and high load or memory pressure in the sysstat data.
**Falsified if:** the log ends silently on an idle host.

The previous agent captured nothing, so I collected it now (`out/step0-journal.txt`, `out/step0-host-inv.txt`).

```
IDX BOOT ID                          FIRST ENTRY                 LAST ENTRY
 -1 f99aa8af20f547b8bd6509d74b1e1698 Fri 2026-08-14 13:30:42 UTC Fri 2026-10-16 12:41:09 UTC
  0 acdbd1e5824c49469aa7e3b67c05d9a5 Fri 2026-10-16 15:39:32 UTC ...
```

- **Boot -1 ends at 12:41:09 UTC (14:41 CEST).** Boot 0 starts at 15:38:18 UTC. Boot 0's journald logs `system.journal corrupted or uncleanly shut down`. There is no shutdown sequence.
- **Last lines of boot -1:** sshd preauth noise from an external IP, `sysstat-collect`, and an AppArmor audit line at 12:40:53.
- **No fault messages.** A grep over 1,474,146 lines for Oops, BUG, panic, oom-kill, Machine check, watchdog, hung task, soft lockup, RCU stall, thermal and EDAC found nothing. The only warnings are old: workqueue-hog notices on Oct 10–11 and `loop0` I/O errors on Oct 3.
- **The host was idle.**

  ```
  sar 12:20-12:40 UTC  %idle 97.00 / 98.02   ldavg-1 0.66 / 0.00   kbavail 59.1 GB
  ```

  The last spike lease (`tp-percpu256`, 12:36:17) never started its payload: no `sudo … env` line follows it.
- **No `evmod`/`shmproxy` in boot -1.** The grep matched only sudo command paths. `/sys/fs/pstore` is empty and `ipmitool` is absent.

**Verdict:** the host lost power or was reset abruptly, with no software trace, while idle. It was not caused by load or by the spike. I cannot prove the hardware cause without BMC/SEL access.

## Item 1: host-role V-3 (bench-v3), WORKS

**Hypothesis:** on the vhost transport, memory stays flat, the sender stalls through credit, resumes, and the stream is byte-exact.
**Prediction:** pipeline of about 4.7 MiB (v2t) or 0.65 MiB (t2v), `SUnreclaim` change under 1 MiB, resume under 10 ms.
**Falsified if:** the pipeline grows with stall time, or any byte is wrong.

Run `bench-v3-…T161003Z`: source `d5710d7c`, private QEMU VM (`7.0.0-29-generic`, 16 GiB), and `all_pass: true`.

| label | trials | stream ok | peak pipeline MiB | SUnreclaim rise max MiB | longest sender block ms | resume ms (min/p50/max) |
|---|---:|---:|---|---:|---|---|
| v2t-stall | 10 | 10/10 | 4.69/4.69/4.69 | 0.2 | 2956/2957/2959 | 0.98/2.50/2.88 |
| v2t-long | 2 | 2/2 | 4.69/4.69/4.69 | 0.2 | 9958/9958/9959 | 2.96/3.11/3.26 |
| v2t-slow | 3 | 3/3 | 3.21/4.62/4.66 | 0.2 | 0/72/75 | n/a (sender 19.9–20.0 MiB/s against a 20 MiB/s reader) |
| v2t-cyc | 5 | 5/5 | 4.69 | 0.2 | 993/995/998 | n/a (the 1.3 s figure is the harness's longest block, not a resume) |
| t2v-stall | 10 | 10/10 | 0.63/0.65/0.66 | 0.5 | 3000/3001/3002 | 0.50/0.72/0.90 |
| t2v-long | 2 | 2/2 | 0.64/0.65/0.65 | 0.2 | 10000 | 0.79/0.87/0.94 |
| t2v-cyc | 5 | 5/5 | 0.64/0.65/0.73 | 0.2 | 1009/1011/1013 | n/a |

v2t is the vsock-source-on-vhost direction, t2v the TCP-source-to-vsock mirror. `SUnreclaim` range inside a stall window was at most 128–132 KiB. The plateau was static (trial 0: `a=233033 b=4165312 c=516855` bytes from t=252 ms to t=2768 ms, then drained to 0 at t=3019 ms). The first run (`…T160503Z`) was partial and the second is the authoritative one.

Kernel citations (vendors/linux @ v7.0):
- Peek returns no credit: `net/vmw_vsock/virtio_transport_common.c:806-809` (peek dispatch), `:536` (`do_peek`), `:583` (`do_dequeue`), `:654-655` (credit update only on dequeue).
- Credit update wakes the writer: `:1440-1441`.
- Non-blocking vsock send returns `-EAGAIN` without credit: `net/vmw_vsock/af_vsock.c:2215-2218`.

## Item 2: final-build lifecycle

Final build: `evmod.ko` `1eecbe01…`, source `d5710d7c…`.

### tcp_abort propagation

**Hypothesis:** an abort sends RST to the TCP half and `SHUT_RDWR` to the vsock half.
**Prediction:** sender error within ~10 ms and `live=0` in every case.
**Falsified if:** a pair stays live or a sender hangs.

| scenario | trials | outcome | sender/reader errno | latency ms (min/p50/max) | live at end |
|---|---:|---|---|---|---|
| cut-A (target app `shutdown(RDWR)`) | 5 | 5 aborted | host sender EPIPE (err_32) 5/5 | 0.22/0.37/202.3 | 0 |
| cut-G (target app cut) | 5 | 5 aborted | guest TCP writer err_104 ×2, err_32 ×3 | 0.04/0.16/0.21 | 0 |
| rst-T (source RST) | 5 | 5 aborted | target reader err_104 | 0.08/0.23/0.37 | 0 |
| rst-G (source RST) | 5 | 5 aborted | vsock target reader EOF | 0.11/0.17/0.66 | 0 |
| kill-A (target process dies, RST) | 10 | 10 aborted | err_32 ×10 | n/a | 0 |
| kill-T / kills-T (stalled-then-killed) | 10 / 10 | 20 aborted | err_104 ×20 | n/a | 0 |
| sclose-A (sender closes at 8 MiB) | 5 | 5 completed | target read exactly 8,388,608 B | n/a | 0 |
| **cut-T** (target app `shutdown(RDWR)`, fd kept open) | 5 | **3 aborted, 2 stuck** | err_104 ×3; err_32 ×2 only after the harness force-unblocked at 20 s | 0.15/1.84/**19,960** | 0,**1**,0,0,**1** |

The cut-T fix works for close, kill and RST (23/23). The two stuck pairs held a bounded 2.6 MB in the target send queue and 12–98 KB in the source queue; no leak was reported.

My explanation is an inference from source reading, not a measurement:
- `tcp_shutdown` with `SHUT_RD` sends nothing (`net/ipv4/tcp.c:3113`).
- The peer's socket goes FIN_WAIT2 with `RCV_SHUTDOWN`. Data would trigger a reset (`net/ipv4/tcp_input.c:7403-7416`).
- With a full receive buffer the window is zero, so a zero-window probe is dropped as out of window before that state code runs (`tcp_sequence`, `tcp_input.c:4860-4872`).

That is plain TCP backpressure, not a module defect. Only a peer that closes or resets sends an abort signal.

### rmmod with a live pair

**Hypothesis:** rmmod with live pairs gives no oops, restores callbacks, and leaks nothing.
**Prediction:** `rmmod` rc 0, a surviving userspace pump continues byte-exact, audit counters balanced.
**Falsified if:** an oops, a nonzero rc, or unbalanced counters.

| scenario | trials | rmmod rc | rmmod ms (p50/max) | pairs live at rmmod | pump after rmmod |
|---|---:|---|---|---|---|
| unload-A / T / G (target stalled) | 3 each | all 0 | 21–26 / 22–36 | 3/3 | EOF 3/3 each |
| unloadf-A / T / G (flowing) | 5 each | all 0 | 21–22 / 30–33 | 5/5 | EOF 5/5 each |
| unload-stress-A / T / G (rmmod 2–62 ms into a flowing pair) | 30 each | 90/90 rc 0 | 22–23 / 37–41 | 30/30 | EOF 30/30 each, `lost_wakeups` 0, no stderr |

All 90 stress cycles left `stream_ok` 30/30 per direction and no oops, BUG or warning in the guest logs. The `module-audit` build prints counters at every rmmod:

```
evmod: AUDIT pair alloc=1 free=1 sock get=2 put=2 buf alloc=8 free=8 hook=2 unhook=2   (rmmod_lines 90, unbalanced [])
```

Callbacks are restored: hook equals unhook, and the sockets keep working under the userspace pump afterwards. This evidence is for sockets without kTLS. See item 4 for the kTLS panic.

A 300-cycle leak audit (`leak-mod` against a no-module control) is within noise. `SUnreclaim` slope was 1.20 vs 0.60 KiB/cycle in the audit run, and 1.20 vs 2.17 in the earlier run. The counters balance.

### Tight buffers

Target and app buffers were set to 64 KiB, with a 2 s stall. Byte-exact in all cases.

| scenario | trials | ok | peak pipeline MiB p50/max | SUnreclaim rise max | longest block |
|---|---:|---|---|---:|---|
| ev-tight-A | 5 | 5/5 | 0.51/0.51 | 0.2 | 2033 ms |
| ev-tight-T | 5 | 5/5 | 0.36/0.37 | 0.0 | 1999 ms |
| ev-tight-G | 5 | 5/5 | 0.12/0.15 | 0.1 | 2000 ms |

The default-buffer pipeline plateau was 2.7 MiB, so pinning socket buffers tightens the bound as predicted.

### Core battery on the final build, A, T and G

Scenarios: stall, long, slow, cyc, burst, bidir, eof, tiny and fault. `module_err=0` and `short_consume=0` everywhere.

| group | A | T | G |
|---|---|---|---|
| stall (3 s), 20 trials | 20/20 ok, pipeline 2.75 MiB, rise ≤0.4 MiB | 20/20, 2.71 (max 10.2, autotune), ≤0.5 | 20/20, 0.51, ≤0.4 |
| long (10 s) | 3/3 | 3/3 | 3/3 |
| slow, cyc, burst | 25/25 | 25/25 | 25/25 |
| bidir (both directions, FIN both) | 10/10 | 10/10 | 10/10 |
| eof (1 MiB + FIN) | 40/40, FIN 40/40 | 40/40 | 40/40 |
| tiny (8 B + FIN) | 60/60 | 60/60 | 60/60 |

Longest sender blocks matched the stalls (3027, 3038 and 3000 ms for A, T and G). Resume p50 was 2.77, 3.08 and 1.56 ms.

### Fault-injection controls (A, T, G)

**Hypothesis:** the verifier detects a dropped byte, duplicated bytes, and swapped words.
**Falsified if:** a corrupted stream passes.

| fault | A | T | G |
|---|---|---|---|
| 1 (drop 1 byte) | 3/3 detected | 3/3 | 2/3 detected, 1 no-op |
| 2 (duplicate 8 B) | 3/3 | 3/3 | 3/3 |
| 3 (swap two 8 B words) | 3/3 | 3/3 | 3/3 |

26 of 27 injections took effect and were detected. The remaining G trial (fault1 trial 1) passed with a byte-exact stream. Module totals show only two bytes dropped:

```
module stats after: ... bytes_t2v=50331646 ... fault_hits=3   (3 x 16777216 - 2)
```

The injector counts a hit and sets `fault_done` before the send is accepted. When the send returned `-EAGAIN`, the peek was retried without the fault. That is an injector bug, not a verifier miss. `first_bad` for detected faults was about 1.05–1.11 MB, just past `fault_after` = 1 MiB.

Bytes verified in the final-build battery: A+T+G together verify well over 10 GiB. The earlier increment's 64.6 GiB total stands.

## Item 3: unbound mode at scale

**Hypothesis:** `unbound=1` helps single-flow streaming without hurting the many-flow rows.
**Prediction:** at 16,384 peers it is within ±10% of per-CPU; single-flow T is higher.
**Falsified if:** it is clearly worse at scale or on small messages.

Harness: the kernel-vs-aya-benchmark harness (`bench-v3`, the frozen perf harness plus the `v3` subcommand), private QEMU VM, 4 online vCPUs. The runs are `scale1` (unbound:20, per-CPU:20) and `scale2` (aya:20, unbound:21). The per-CPU and Aya rows were rerun in this session, so n=1 each. Unbound has n=2.

Median [min–max], unrestricted closed loop, 4 outstanding:

| population, size | proxy | GiB/s | busy cores | core-s/GiB | p50 ms | p99 ms |
|---|---|---:|---:|---:|---:|---:|
| 16,384, 64 KiB | Aya | 1.056 | 3.23 | 3.06 | 0.274 | 0.969 |
| | evmod per-CPU | **1.193** | 3.22 | **2.70** | 0.279 | **0.805** |
| | evmod `unbound=1` | 1.020 [0.963–1.077] | 2.90 | 2.85 | 0.390 | 0.865 |
| 64, 64 KiB | Aya | 0.148 | 0.86 | 5.83 | 0.787 | 41.85 |
| | evmod per-CPU | 0.148 | 0.80 | 5.40 | 0.492 | 41.75 |
| | evmod `unbound=1` | 0.113 [0.110–0.115] | 0.71 | 6.27 | 0.640 | 41.74 |
| 64, 1 KiB | Aya | 0.107 | 3.97 | 37.1 | 0.067 | 0.115 |
| | evmod per-CPU | 0.106 | 3.97 | 37.6 | 0.068 | 0.147 |
| | evmod `unbound=1` | **0.041** | 2.73 | 66.5 | **0.178** | 0.346 |
| 16,384 (64 active), 1 KiB | Aya | 0.097 | 3.96 | 40.8 | 0.075 | 0.179 |
| | evmod per-CPU | 0.096 | 3.95 | 41.2 | 0.074 | 0.185 |
| | evmod `unbound=1` | 0.044 | 2.71 | 61.7 | 0.165 | 0.331 |

The earlier per-CPU and Aya cohorts (3 cohorts each, same harness, from the first increment) were 16K/64 KiB: evmod 1.198 and Aya 0.955 GiB/s. At 64 peers they were at parity (0.146 and 0.143). This session reproduces them: 1.193 and 1.056.

Memory and tasks at 16,384 peers are the same in all three modes: 16,466 tasks, `SUnreclaim` +2,822 MiB (unbound 2,821), 256 MiB kernel stack, no proxy-owned kernel threads.

Single-flow streaming, count only, MiB/s p50 (min–max). `ev-tp-*` is 1 GiB per trial, 10 trials. Each mode was run twice, in AB then BA order:

| dir | per-CPU run 1 | per-CPU run 2 | unbound run 1 | unbound run 2 |
|---|---|---|---|---|
| A | 662 (537–761) | 552 | 583 | 573 |
| T | 1093 (875–1205) | 890 (648–1716) | **1242 (1040–1582)** | **1258 (1087–1401)** |
| G | 1166 (1015–1267) | 861 (789–1178) | **1355 (1091–1599)** | **1394 (1312–1630)** |

All 10/10 byte-exact, with `SUnreclaim` rise ≤0.4 MiB. Per-CPU single-flow throughput varies a lot between boots (earlier increment: 569 and 667 MiB/s for T; here 890 and 1093). Unbound is stable at 1.24–1.26 GiB/s for T and 1.36–1.39 for G. So unbound helps the large single flow by 1.1–1.4×, but the earlier "per-CPU is far behind" gap is only partly reproduced.

**Verdict and recommendation:** `unbound=1` is **worse** wherever there are many flows or small messages (0.041 vs 0.106 GiB/s at 64 peers and 1 KiB; −14% at 16K and 64 KiB). Its single big flow gain is real but smaller than the first increment suggested. Recommend **per-CPU as the default**. Offer unbound only as a per-pair choice at attach time, since the module parameter is global.

## Item 4: kTLS coexistence

Setup: TLS 1.3 AES-128-GCM with fixed test keys installed by `setsockopt(SOL_TLS)`, TX and RX, on the module-owned TCP legs. Mask: A = target leg, G = source leg, T = both legs. `TlsTxSw` and `TlsRxSw` counters confirm software kTLS was active. The module is unmodified (`d5710d7c`).

**Hypotheses:** peek/send/consume works through the kTLS record layer; the module's callbacks chain with TLS's in both install orders.

### Result when kTLS is installed before the module attaches (production order)

| scenario | A | T | G |
|---|---|---|---|
| ver (512 MiB, 5 trials) | 5/5 | 5/5 | 5/5 |
| stall 3 s (10 trials) | 10/10 | 10/10 | 10/10 |
| bidir + FIN | 5/5 | 5/5 | 5/5 |
| eof (1 MiB + FIN) | 20/20 | 20/20 | 20/20 |
| unload (rmmod, live) | 3/3, rc 0 | 3/3 | 3/3 |
| **alert** (close_notify record before FIN) | 5/5 | **0/5, `module_err`=5** | **`module_err`=5, no EOF** |
| throughput p50 (`ek-tp`, 10/10 byte-exact) | 572 MiB/s | 622 | 686 |

Peek and consume work byte-exact through the kTLS record layer, and the bound holds. Stall pipeline p50: A 2.69 MiB, T 2.7 (max 2.9), G 0.47. `SUnreclaim` rise was ≤0.7 MiB.

**Throughput.** `ev-tp` without kTLS ran 552–662 MiB/s (A), 890–1093 (T) and 861–1166 (G) in the same session (per-CPU mode). Software kTLS therefore costs about 30–45% on T and G, and A is about equal (kTLS on one leg only). The cost is mostly AES-GCM, with `core-s/GiB` of 2.55–3.46 against roughly 1.45–1.7 untouched.

**close_notify defect.** `tls_sw_recvmsg` handles `MSG_PEEK` (`net/tls/tls_sw.c:2062`). When the next record is not application data and the caller passes no control buffer, it returns `-EIO` (`tls_record_content_type`, `tls_sw.c:1844-1860`). The module peeks without a cmsg, so any alert aborts the pair:
- In T, the target reader got `err_104` 3/5 and lost tail bytes.
- In G, all bytes arrived but EOF never did (0 EOF, timeout 5/5).

So the module is not kTLS-correct without a cmsg-aware peek and an alert-aware EOF path.

### Callback chaining with the TLS ULP

- **sk_user_data:** TLS keeps its state in `icsk_ulp_data` (`net/tls/tls_main.c:933`), not `sk_user_data`, so the module's `-EBUSY` guard does not collide with it.
- **kTLS first, then the module:**
  - TLS RX arms by saving `sk_data_ready` and installing `tls_data_ready` (`tls_sw.c:2693-2694`), and TX swaps `sk_write_space` (`tls_main.c:767-768`).
  - The module saves those as its `orig_*` and overrides all four callbacks. Its `ev_data_ready` never calls the original, so `tls_strp_data_ready` never runs.
  - It still works because `tls_sw_recvmsg` drives the strparser from the recv path itself.
- **Module first, then kTLS (ek-after-T/A/G, 5/5 byte-exact each):**
  - TLS saves `ev_data_ready` as `saved_data_ready` and the chain is `tls_data_ready` → `tls_rx_msg_ready` → `saved_data_ready` (`tls_sw.c:2511-2516`).
  - TX: `tls_write_space` calls `ctx->sk_write_space`, which is the module's `ev_write_space` (`tls_main.c:303,314`).
- **Hazard.** `tls_sw_release_resources_rx` restores `sk->sk_data_ready = ctx->saved_data_ready` (`tls_sw.c:2603-2606`), and `tls_rx_msg_ready` calls it (`:2515`). In the after-order, that saved pointer is module text.

### Finding: kTLS-after-module plus rmmod panics the guest

Scenario `ek-unloadaft-T`: both legs kTLS installed after the module attached, stalled target, `rmmod` 1.2 s in. Reproduced twice. Each time the harness hung 210 s with `trial_end=0` because the guest rebooted (`panic=-1`). A third run with `panic=0 loglevel=7` captured the oops (`out/adhoc2.txt`):

```
[    2.318341] evmod: unloaded attached=1 completed=1 detached=1 aborted=0
[    2.333121] Oops: int3: 0000 [#1] SMP NOPTI
[    2.333145] RIP: 0010:0xffffffffc060fd01
[    2.333199]  ? tls_rx_msg_ready+0x18/0x30 [tls]
[    2.333218]  tls_strp_check_rcv+0x177/0x210 [tls]
[    2.333232]  tls_strp_msg_done+0x37/0x60 [tls]
[    2.333242]  tls_sw_recvmsg+0x54c/0x910 [tls]
[    2.333266]  inet_recvmsg+0x125/0x130
[    2.333466] Modules linked in: aesni_intel tls vmw_vsock_virtio_transport ... [last unloaded: evmod(OE)]
[    2.333509] Kernel panic - not syncing: Fatal exception in interrupt
```

Survivors (so the crash is hazard-dependent, not certain): a flowing unload in T (`ek-unloadaftf-T`, 5/5) and the A and G unloads (3/3 each). I did not run the module-first plus rmmod case for kTLS-before-module on a stalled T pair, but `ek-unload-T/A/G` (kTLS first, rmmod live) passed 3/3 each, rc 0, no oops.

## vendors/linux @ v7.0 citations

- vsock peek, credit and send: `virtio_transport_common.c:536, 583, 654-655, 806-809, 1440-1441`; `af_vsock.c:2215-2218`.
- TCP abort and half-close: `net/ipv4/tcp.c:3113` (`tcp_shutdown` ignores `SHUT_RD`), `:5087/5141` (`tcp_abort` GPL export); `net/ipv4/tcp_input.c:4860-4872` (`tcp_sequence`), `:7403-7416` (FIN_WAIT data reset).
- kTLS: `net/tls/tls_sw.c:1844-1860, 2062, 2511-2516, 2603-2606, 2693-2694`; `net/tls/tls_main.c:303, 314, 767-768, 933`; `include/net/tls.h:131`.

## Design implications

1. **The module must not be unloadable while any hooked socket exists.** Either build it in, or hold a module reference per hooked socket. Today's 90/90 clean stress passes fit a short race window. They do not cover the kTLS saved-callback case.
2. **Lock the install order.** A new kTLS socket must never be hooked by the module after TLS saves a callback that points into module text (module first, TLS later), unless the module can never unload.
3. **Real kTLS support needs cmsg-aware peek.** Receive with a control buffer, forward the record type, and map `close_notify` to an alert record plus FIN on the target. This is a contract-shaping DESIGN item, not a spike tweak.
4. **Abort semantics are inherited from the sockets.** A close, kill or RST propagates in under 1 ms. A peer that only shuts down its read side and keeps the fd open looks like a stalled reader. The owner needs a deadline or probe if it cares.
5. **Default to per-CPU workqueues.** Choose unbound per pair at attach time if a large single flow needs it.
6. **The pair's completion event is still missing.** The owner polls `/proc/evmod` (caveat 7 of the earlier findings).

## Gate recommendation

**Keep PROMOTE-to-DESIGN as a user-decision candidate. Do not treat the in-kernel module as accepted.** Items 1 and 2 now close the host-role bound, the lifecycle and the final-build battery, and nothing found contradicts the earlier verdict. Two blockers surfaced that DESIGN must pin before acceptance:
- **Unload and callback-chain lifetime.** A kTLS-then-rmmod panic is reproduced. The fix is to remove the unload path, or to make the module's callbacks outlive every hooked socket.
- **TLS record handling.** A close_notify currently aborts the pair. If kTLS on legs is in scope, the contract needs an alert-aware EOF.

Unbound stays optional. These need the user's explicit decision per `.claude/rules/design.md`: an out-of-tree module in the appliance image versus the userspace pump, and the kTLS scope.

## Files (all under `spike-scratch/netns-density-295-evmod/inc2/`)

- Progress record: `out/REPORT-progress.md`
- Step 0: `out/step0-journal.txt`, `out/step0-host-inv.txt`
- Fault, scale and throughput runs: `out/core-Gf.txt`, `out/scale1.txt`, `out/scale2.txt`, `out/tp-*.txt`
- kTLS: `out/ek-T1.txt`, `out/ek-T.txt`, `out/ek-A.txt`, `out/ek-G.txt`, `out/ek-after.txt`, `out/ek-unloadaft*.txt`, `out/adhoc2.txt` (the oops)
- Evidence: `evidence/vm-ek-*`, `evidence/vm-ev-fault1-G*…T182448Z`, `evidence/bench-scale1-…T192006Z`, `evidence/bench-scale2-…T193652Z`
- Patched harness: `driver/vmrun.py`, which now has a names-and-sizes module check, `ek-unloadaft*` scenarios, a `diag=1` mode, and an `adhoc=` mode with `panic=` and `loglevel=` options. `perf_table_ub.py` is the table script for the unbound cells.
- Leases: each `launch.py` run took and released the canonical lease. The final check shows the lock free.
