# evmod findings: event-driven in-kernel vsock/TCP forwarder (netns-density-295)

## Verdict

**WORKS for the measured contract. The host-role bound, the abort fix and `rmmod` with a live pair are NOT verified, because the metal host became unreachable.**

- **Bounded:** with a non-reading target the pipeline plateaus at about 2.7 MiB and stays flat for the whole stall. The sender blocks for the whole stall (3 s and 10 s tested) and resumes 1.3–3.5 ms after the reader wakes. Guest `SUnreclaim` rose at most 0.6 MiB in any trial. The V-3/valve sockmap control queued 160 MiB in 0.36 s with `SUnreclaim` +179 MiB.
- **Correct:** 504 trials and 64.6 GiB of splitmix stream verified with zero mismatches. The harness caught 27 of 27 injected loss/duplicate/reorder faults, so the verifier has teeth. FIN propagates both ways inside the module.
- **Performance:** at 16,384 peers, 65,536-byte request/reply, evmod beats Aya: 1.198 vs 0.955 GiB/s, 2.71 vs 3.18 core-s/GiB, p99 0.780 vs 0.966 ms. At 64 peers it is at parity. It runs 16,466 tasks against shmproxy's 49,233. `SUnreclaim` rose 2.76 GiB against shmproxy's 5.38 GiB and Aya's 2.79 GiB.
- **Single-flow TCP→TCP streaming** is slower than sockmap with the default per-CPU workqueue (667 vs 1,092 MiB/s). With `unbound=1` it reached 1,256 MiB/s at 1.65 core-s/GiB, which is on par with the sockmap and userspace-pump controls. Unbound mode at 16,384 peers was not measured.

**Not done.**
- The metal host stopped answering SSH and ping around 14:45 CEST and was still down at 16:53 CEST, after about 2h10m of polling. The last-observed state was `uptime` showing 62 days up.
- The host-role V-3 run (`bench-v3`, vsock source on the vhost transport) never ran. The first attempt failed because `sync.sh` omitted `bench-v3`; the retry was lost to the outage.
- Three things were never checked on the final module build:
  - the `tcp_abort` fix;
  - `rmmod` with a live pair;
  - the tight-buffer scenarios.
- The final-build battery and the unbound/256 KiB-peek throughput variants did not run, apart from the first two tp runs.
- These were queued in `chain4.sh`, which I killed rather than leave unattended on a shared host.

## Substrate

| Item | Value |
|---|---|
| Host | AMD EPYC 8024P, 16 CPUs, `systemd-detect-virt`=`none`, KVM API 12 (fail-closed preflight passed every run) |
| Host `uname -r` | `7.0.0-29-generic` |
| Guest `uname -r` (Cloud Hypervisor fork, `--vsock backend=vhost-kernel`, 4 vCPU, 1 GiB) | `7.0.0-29-generic`, kernel sha `b51367c7…` |
| Private QEMU/KVM host-role VM (perf), 16 GiB, 4 vCPU | `7.0.0-29-generic` |
| Guest TCP limits | `tcp_rmem` max 7,285,312; `tcp_wmem` max 4,194,304 |
| Module placement | loaded only inside the CH guest or the private QEMU VM. The preflight records physical `/proc/modules` and links, and the run asserts they are unchanged. |

**Rust-for-Linux:** the kernel config has `CONFIG_RUST=y`, but `vendors/linux/rust/kernel/net.rs` only contains `pub mod phy` (line 6). There are no socket abstractions, so I wrote C against the stock headers.

## Module

Sources are in `spike-scratch/netns-density-295-evmod/module/` (`evmod.c`, `evmod_uapi.h`). The control surface is a spike-only ioctl on `/dev/evmod`: `EVMOD_ATTACH(fd_a, fd_b)` and `EVMOD_DETACH(id)`.

- **Hooks:** each socket's `sk_data_ready`, `sk_write_space`, `sk_state_change` and `sk_error_report` are overridden. The originals are saved and restored. `sk_user_data` is set via RCU.
- **Wake-up:** a callback only sets a pending bit and queues one `work_struct` per pair on a `WQ_PERCPU` workqueue. There is no per-peer thread.
- **Work item:**
  - It runs `kernel_recvmsg(MSG_PEEK|MSG_DONTWAIT)` into a pooled 64 KiB scratch buffer.
  - It then runs `kernel_sendmsg(MSG_DONTWAIT)` to the target.
  - It then consumes exactly the accepted bytes with `sock_recvmsg` on an `iov_iter_discard` iterator.
  - On EAGAIN or a short send it stops and resumes on the target's `write_space`.
  - The budget is 256 KiB per direction per run, then requeue.
- **Teardown:** EOF is forwarded as `shutdown(SHUT_WR)`. When both directions have seen EOF the pair tears itself down.
- **Memory held:** each pair is 248 B. The scratch pool is 8 preallocated buffers. `pool_miss=0` over the whole 16K run (3.65M work runs, `requeue=0`, `err=0`).
- **Negative-control knob:** module param `fault`: 1 drops one byte, 2 duplicates 8 bytes, 3 swaps two 8-byte words.
- **Module params:** `unbound=1` selects `WQ_UNBOUND`. `buf_kb` sets the peek size.

## Experiments

These were written before the runs in my working plan, not as a committed file. The fault-injection controls and the unbound/`buf_kb` variants were added after the first results.

| # | Hypothesis | Prediction | Falsified if |
|---|---|---|---|
| E1 stall/long | A byte leaves the source only after the target accepted it, so vsock credit and the TCP window close | Pipeline plateaus near 256 KiB plus the TCP buffers (about 3 MiB). Sender blocks for the whole stall. `SUnreclaim` is flat (<1 MiB). Resume under 10 ms. | The pipeline grows with time, `SUnreclaim` rises more than 8 MiB, or the sender never blocks |
| E2 slow-steady | Backpressure throttles the sender to the reader's rate | Sender rate equals reader rate (20 MiB/s) | The sender runs ahead of the reader for more than 1 s |
| E3 cycles/burst | No stranded tail and no re-insert deadlock | Stream byte-exact, no stall after resume | A bad byte, or stuck for more than 1 s after resume |
| E4 bidir+FIN | Both directions are independent; FIN propagates both ways | Reverse stream flows during a forward stall. Target sees EOF with 0 extra bytes. | Anything else |
| E5 fault injection | The verifier detects loss, duplication and reorder | 100% detected | A fault goes undetected, in which case E1–E4 prove nothing |
| E6 perf | Equal to or better than Aya per CPU and p99 | ≥0.9× Aya throughput. Tasks near 16.5k. Memory well under shmproxy. | Below 0.9× Aya, or tasks/memory near shmproxy |
| E7 lifecycle | Aborts and cuts tear the pair down with no leak | `live=0`, memory returned | A hang or a leak |

## Kernel facts (vendors/linux @ v7.0)

1. **Peek does not return credit.** `virtio_transport_stream_dequeue` dispatches `MSG_PEEK` to `do_peek` (`net/vmw_vsock/virtio_transport_common.c:806-809`, `do_peek` at `:536`). Only `do_dequeue` (`:583`) advances `fwd_cnt` and sends the credit update (`:654-655`).
2. **Consume-after-send is what withholds credit.** The sockmap path instead sends credit at `:1779`, before `recv_actor` at `:1781`. This is the V-3 root cause.
3. **Credit update wakes the writer.** `OP_CREDIT_UPDATE` calls `sk->sk_write_space(sk)` (`:1440-1441`). Data arrival calls `vsock_data_ready` (`:1435`, `af_vsock.c:1062-1070`).
4. **Non-blocking vsock send.** It returns -EAGAIN when there is no credit (`af_vsock.c:2215-2218`). A partial send is accepted.
5. **vsock read.** `MSG_PEEK` breaks after one pass (`af_vsock.c:2433`). EOF comes from `RCV_SHUTDOWN` (`:2441`) or `peer_shutdown & SEND_SHUTDOWN` in the wait (`:2305+`). Default buffer is 256 KiB (`:199`).
6. **TCP.**
   - `tcp_recvmsg_locked` honors `MSG_PEEK` through `peek_seq` (`net/ipv4/tcp.c:2731-2735`).
   - A failed non-blocking send sets `SOCK_NOSPACE` (`:1412`).
   - `tcp_check_space` calls `sk_write_space` only if `SOCK_NOSPACE` is set (`include/net/tcp.h:770-776`, `tcp_input.c:6118`).
   - `sk_stream_write_space` clears it only if writeable (`net/core/stream.c:34`), so the override calls the original and then tests `sk_stream_is_writeable`.
   - `tcp_data_ready` fires on `SOCK_DONE` (`tcp_input.c:5621`).
7. **APIs used.**
   - `sock_recvmsg` (`net/socket.c:1102`), `kernel_sendmsg` (`:791`), `kernel_sock_shutdown` (`:3681`), `sockfd_lookup` (`:570`).
   - `iov_iter_discard` (`include/linux/uio.h:297`).
   - `rcu_dereference_sk_user_data` (`include/net/sock.h:691`).
   - `WQ_PERCPU` (`include/linux/workqueue.h:405`).
   - `tcp_abort` is `EXPORT_SYMBOL_GPL` (`net/ipv4/tcp.c:5141`).
8. **Why the sockmap path is unbounded.** `skmsg.c:989` is an unconditional `skb_queue_tail(&psock_other->ingress_skb, …)`.

## (a) Memory over time, non-reading target

Directions:
- **A:** host sender → guest vsock source → evmod → guest TCP target.
- **T:** guest TCP source → evmod → guest TCP target.
- **G:** guest TCP source → evmod → vsock target → host reader.

"Pipeline" is the source rx queue plus target send queue plus target app rx queue, from `SIOCINQ`/`SIOCOUTQ`. Module build `9eebe7fe`. Cells are min/p50/max.

| scenario | trials | stream ok | pipeline MiB | guest SUnreclaim rise, max (MiB) | longest block (ms) | accepted before first block (MiB) | resume (ms, p50) |
|---|---:|---:|---|---:|---|---|---:|
| ev-stall-A (3 s) | 20 | 20/20 | 2.62/2.75/2.77 | 0.4 | 2995/2998/3021 | 2.62/2.75/2.75 | 2.59 |
| ev-stall-T | 20 | 20/20 | 2.65/2.76/8.05 | 0.5 | 2965/2996/3028 | 4.38/5.06/5.19 | 3.11 |
| ev-stall-G | 20 | 20/20 | 0.25/0.52/1.68 | 0.6 | 2973/2998/2999 | 2.69/2.75/2.88 | 1.58 |
| ev-long-A (10 s) | 3 | 3/3 | 2.69/2.69/2.76 | 0.1 | 9997/9997/10008 | 2.69/2.69/2.75 | 2.71 |
| ev-long-T | 3 | 3/3 | 2.58/2.62/3.95 | 0.1 | 9995/9997/10028 | 4.69/5.0/5.06 | 2.95 |
| ev-long-G | 3 | 3/3 | 0.86/1.0/2.95 | 0.0 | 9974/9997/9998 | 2.75/2.75/2.81 | 1.68 |
| ev-slow-A (20 MiB/s) | 5 | 5/5 | 2.81/2.86/2.90 | 0.3 | 44–47 | n/a | n/a |
| ev-slow-T | 5 | 5/5 | 2.68/2.96/4.45 | 0.0 | 76–92 | n/a | n/a |
| ev-slow-G | 5 | 5/5 | 0.10/0.20/0.42 | 0.1 | 49–61 | n/a | n/a |
| ev-cyc-A/T/G (3 stall/fast cycles) | 10 each | 10/10 each | 2.82–5.12 / 2.67–6.37 / 0.24–1.94 | ≤0.3 | about 1000 | n/a | n/a |
| **ctl-A** (sockmap, same VM) | 3 | n/a | 160.4 queued, hit the 160 MiB abort cap in 0.36–0.44 s | **+179.4** | 0–1 (never blocked) | n/a | n/a |
| **ctl-T** (sockmap, same VM) | 3 | n/a | 162 queued in 0.12 s | +5.1; MemAvailable −139 | n/a | n/a | n/a |

- Sender throughput in ev-slow-A was 20.4 MiB/s against the 20 MiB/s reader.
- **G host side:** the host vsock rx queue sat at exactly 256 KiB (`host_peak_vsock_inq` 256/256/256 KiB) and host `SUnreclaim` rose at most 0.6 MiB.
- **T worst case:** the 8.05 MiB outlier is TCP autotune on the two loopback legs, under the guest ceilings of 7.2 MiB rx and 4 MiB tx. It does not grow with stall time: the 10 s runs peaked at 3.95 MiB.
- **cyc rows:** ignore the "resume ms" and "accepted before block" columns. The harness picks the longest of several blocks.
- **ev-burst-A/T/G** (bursty sender with a 2 s stall): 30/30 byte-exact, pipeline 2.6–3.4 MiB for A and T, 0.06–0.42 for G.

Trial-0 memory over time, ev-stall-A (the stall ends at about 3 s):

```
t= 0.00s pipe= 1.889MiB moved=  1.56MiB app_read=  0.00 sunrec= 19.5MiB avail=802.4MiB
t= 0.36s pipe= 2.775MiB moved=  2.53MiB app_read=  0.00 sunrec= 19.7MiB avail=800.7MiB
t= 1.43s pipe= 2.775MiB moved=  2.53MiB app_read=  0.00 sunrec= 19.8MiB avail=800.4MiB
t= 2.86s pipe= 2.775MiB moved=  2.53MiB app_read=  0.00 sunrec= 19.8MiB avail=802.0MiB
t= 3.21s pipe= 0.077MiB moved=138.44MiB app_read=138.44 sunrec= 19.7MiB avail=806.5MiB
t= 3.57s pipe= 0.000MiB moved=256.00MiB app_read=256.00 sunrec= 19.8MiB avail=805.4MiB
```

Control, ctl-A trial 0: `queued` goes 16 → 35 → 53 → 72 → 91 → 109 → 134 MiB with `sunreclaim` 38.9 → 180 MiB in 0.36 s, while `sent` stays at 2.57 MiB.

The sender is not throttled early; it blocks only once about 2.6–5 MiB is in flight, because the target's TCP buffers fill first. Setting the target socket buffers explicitly would tighten that. The `ev-tight-*` scenarios that test this did not run.

## (b) Correctness

Every row is a fresh pair per trial. The main stream is checked against an independently generated splitmix64 stream (the order-sensitive hash plus `first_bad`).

| group | trials | result |
|---|---:|---|
| stall, long, slow, cyc, burst (A, T, G) | 135 | 135/135 byte-exact |
| bidir (both streams concurrent, forward reader stalled 2 s, FIN both ways) | 30 | 30/30 forward and 30/30 reverse byte-exact. FIN seen at target 30/30, 0 extra bytes. |
| eof (1 MiB + FIN) | 120 | 120/120, FIN seen 120/120 |
| tiny (exactly 8 bytes + FIN) | 180 | 180/180, FIN seen 180/180 |
| **fault 1/2/3 on A, T, G** | **27** | **27/27 detected.** `first_bad` is 1,048,576–1,113,000, i.e. just past `fault_after`=1 MiB. |

Total: 504 non-fault trials, 64.6 GiB verified, 0 bad, `module_err=0`, `short_consume=0`.

- The Aya variant of this harness needs the controller to propagate the FIN (`aya_controller_only_shutdown_control`). evmod does it in the module (`module_propagated_fin:true`).
- In the 16K perf harness every request/reply round trip is `assert_eq`-checked in both directions (TCP 64/1,024/65,536 B). The preflight passed in all 9 cells: half-close, backpressure hold, and owned-mapping fail-closed via `EVMOD_DETACH`.

### Lifecycle (partial, module build `9eebe7fe`)

- **ev-cut-A** (target app shuts down mid-stream): the pair aborted 5/5 with `live=0`. The host sender got EPIPE (`err_32`) within 8–219 ms. No memory leak.
- **ev-cut-G**: the pair aborted 5/5 with `live=0`. The guest source writer was still blocked until the harness shut it down.
- **ev-sclose-A** (sender closes at 8 MiB): `completed=5`, `live=0`, target read exactly 8,388,608 bytes.
- **ev-cut-T found a defect.** On error the module used `shutdown(SHUT_RDWR)`, which does not wake a blocked TCP peer sender. The writer stayed blocked forever, and the harness sink (which only force-unblocked a writer when the trial had not finished) hung for 260 s with no `trial_end`.
- **Fix, not verified:** `tcp_abort(sk, ECONNABORTED)` for TCP halves on error, which sends RST. I also made the harness force-unblock the writer whenever it has not finished.
- **ev-unload-T was invalid.** `rmmod` returned EBUSY because the hung ev-cut-T sink still held `/dev/evmod` open. It does show the module refuses to unload while the control device is open, but `rmmod` with a live pair was not tested. No kernel BUG or oops appeared in any run.

## (c) Performance, same harness and workload as kernel-vs-aya-benchmark

Setup:
- Each cell runs in a fresh private QEMU VM (16 GiB, 4 online / 64 possible CPUs, queue size 8, 256 KiB guest memory per peer).
- Synthetic peers are real independent `/dev/vhost-vsock` owners.
- There is no userspace payload relay.
- Cohorts were AB/BA/AB over `kernel`, `aya`, `evmod`, three cells each, all in one session.

Cell values are median [min–max] over the 3 cohorts. CPU is whole-VM busy (user, nice, system, irq, softirq; idle, iowait and steal excluded), which includes the kworkers.

**16,384 peers, all participating, 65,536-byte request/reply, unrestricted, 4 outstanding:**

| proxy | GiB/s | busy cores | core-s/GiB | p50 ms | p99 ms |
|---|---:|---:|---:|---:|---:|
| shmproxy.ko (2 kthreads/peer) | 0.199 [0.194–0.203] | 0.69 [0.58–0.81] | 3.39 [2.91–4.15] | 1.987 | 7.185 [6.89–7.42] |
| Aya sockhash stream verdict | 0.955 [0.947–0.992] | 3.06 [3.04–3.11] | 3.18 [3.13–3.23] | 0.385 | 0.966 [0.959–0.979] |
| **evmod.ko** | **1.198 [1.181–1.208]** | 3.24 [3.24–3.27] | **2.71 [2.71–2.74]** | **0.270** | **0.780 [0.771–0.788]** |

**64 peers, 65,536 B:**

| proxy | GiB/s | busy cores | core-s/GiB | p50 ms | p99 ms |
|---|---:|---:|---:|---:|---:|
| shmproxy.ko | 0.098 [0.098–0.101] | 0.72 | 7.31 [7.12–7.51] | 2.067 | 43.56 |
| Aya | 0.143 [0.137–0.143] | 0.86 | 5.98 [5.86–7.09] | 0.779 | 41.79 |
| evmod.ko | 0.146 [0.140–0.151] | 0.85 | 6.05 [5.11–6.08] | 0.545 | 41.80 |

- At 64 peers evmod is at parity with Aya; its lower p50 (0.545 vs 0.779 ms) is the only difference.
- The 41–44 ms p99 at 64 peers comes from the harness leaving Nagle on for the proxy leg (`x.set_nodelay(false)`). It is the same for Aya and evmod.
- At 16,384 peers evmod is 1.25× Aya on throughput, 0.85× on CPU/GiB, and 0.81× on p99.
- 1,024 B and 64 B rows track Aya to within noise. For example 16,384 peers at 1,024 B: evmod 0.049 GiB/s and p99 0.341 ms, Aya 0.043 GiB/s and p99 0.328 ms.
- These runs reproduce the published rows: shmproxy 0.199 vs 0.195 GiB/s, Aya 0.955 vs 0.970 GiB/s, shmproxy tasks 49,233 vs 49,231.

**Kernel tasks and memory, idle with the population held, increase over the loaded baseline:**

| proxy | peers | all tasks | proxy-owned kernel threads | SUnreclaim +MiB | KernelStack +MiB | kworkers |
|---|---:|---:|---|---:|---:|---:|
| shmproxy.ko | 64 | 273 | 128 (`shmps-`+`shmprx-`) + 1 accept | 22 | 3 | 43 |
| Aya | 64 | 145 | 0 | 12 | 1 | 44 |
| evmod.ko | 64 | 144 | 0 | 12 | 1 | 43 |
| shmproxy.ko | 16,384 | **49,233** | 32,768 + 1 | **5,514** (5.38 GiB) | 768 | 43 |
| Aya | 16,384 | 16,465 | 0 | 2,853 | 256 | 44 |
| evmod.ko | 16,384 | **16,466** | **0** | **2,822** (2.76 GiB) | 256 | 45 |

- The 16,384 tasks common to Aya and evmod are the stock vhost workers from the 16,384 held peers. The module adds only kworkers (43 → 45).
- The module's own state is 248 B per pair.
- Setup of the 4,096 → 16,384 step took 32–35 s for evmod and Aya, against 76–81 s for shmproxy.
- Retirement of the 16,384 owners took 208–218 s for evmod and 238–244 s for Aya.

**Single flow, fast reader, count only (guest `SUnreclaim` flat, ≤0.2 MiB rise). MiB/s min/p50/max, and core-s/GiB where shown:**

| dir | evmod per-CPU (build `9eebe7fe`) | evmod per-CPU (final build `1271015c`) | evmod `unbound=1` (final build) | sockmap control | userspace-pump control |
|---|---|---|---|---|---|
| A | 460/569/661, 2.12 | 442/483/597 | 515/550/602 | 410/468/497, 3.51 | 625/646/700, 2.42 |
| T | 554/667/755, 2.20 | 525/569/846 | **1166/1256/1401**, 1.65 | 1003/1092/1697, 1.35 | 1215/1261/1309, 1.46 |
| G | 802/927/1178 | 758/846/1258 | not run (outage) | no equivalent | no equivalent |

- **Cause of the T gap:** the per-CPU workqueue runs the worker on the CPU that queued it, which is the softirq CPU of the writer. Producer and forwarder share one CPU.
- The spike used the default per-CPU mode for the 16K runs, where every CPU is already busy.
- The `perf-unbound` and `buf_kb=256` runs did not execute. Use `unbound` for large single flows and per-CPU for many flows, or pick at attach time.

## What was not run, and caveats

1. **Host-role V-3** (`bench-v3/src/v3.rs`, credit-aware synthetic guest sender into evmod, and the TCP-source-to-vsock-target mirror) was never executed. Directions A, T and G were measured with the module in the guest. Sections 1–3 of the kernel facts and `virtio_transport_recv_pkt` are shared with the vhost transport, so the bound very likely holds there too. That is an inference, not a measurement.
2. The `tcp_abort` fix, `rmmod` with a live pair, and `ev-tight-A/T` (target buffers set to 64 KiB) are unverified.
3. The evmod rows in perf cohorts 1–3 are build `f4228aa0…`, which predates the `eof_fwd` counter, `tcp_abort` and `buf_kb`. This is documented but unavoidable, because the data path is unchanged. The battery A/T/G, ctl and life runs used `9eebe7fe…`.
4. The bound depends on explicit socket buffers. The target loopback legs autotune up to about 11 MiB (rx 7.2 MiB, tx 4 MiB) in the guest. Pin `SO_SNDBUF`/`SO_RCVBUF` on the target leg to tighten it.
5. **kTLS and psock coexistence:** the module takes over `sk_user_data` and four callbacks (-EBUSY if `sk_user_data` is set) and does not call the original `sk_data_ready`. It would conflict with kTLS RX, sockmap or psock on the same socket. kTLS coexistence is untested.
6. **Unload race:** callbacks are not called under RCU or BH-disable (the vsock rx path runs in process context), so a callback could execute module text after unload. `synchronize_rcu` does not close this. A built-in or pinned-lifetime design is needed.
7. **No completion event** reaches the owner. The harness polls `/proc/evmod`. A pair that finishes (both EOF) or aborts is torn down by the module, which drops its file references. The owner needs a poll or eventfd to know to close its fds.
8. **Verifier scope:** the byte-exact checks cover the main stream and the reverse stream, not the TCP/vsock stack around them. The splitmix verifier detects loss, duplication and reorder of the exact 8-byte words.
9. **Process note:** I ran `pkill -f launch.py` at cleanup, which matches any `launch.py` owned by the same macOS user. Nothing else was found running, but other workspaces' launchers could have been hit.

## Design implications

- Both bounds come from consume-after-accept. Peek then send then discard-consume keeps the byte in the source's receive queue until the target accepted it. vsock credit therefore returns only at consumption (fact 1 vs fact 2), and the TCP window stays closed natively.
- The bound is a sum of explicit socket buffers plus a fixed scratch pool. It is independent of stall time and of bytes sent. Per-flow bound ≈ vsock buf (256 KiB, or what the owner sets) + target `wmem` + `rmem` (set explicitly to tighten) + a module scratch pool of at most ncpu × 64 KiB.
- evmod removes the userspace pump the V-3/valve work recommended, along with its per-flow owner work. It also removes the sockmap re-insert deadlock and the one-skb-per-arrival drain (valve facts 7 and 10): ev-burst and ev-cyc are 60/60 byte-exact.
- FIN/half-close is handled in the module, so the controller no longer needs `shutdown`, as Aya did.
- Error propagation differs by socket type. A TCP half needs an RST on abort (`tcp_abort`). A vsock half takes `shutdown(SHUT_RDWR)`, which gave the sender EPIPE in 8–219 ms.
- The control surface must be re-specified in DESIGN. The spike ioctl is not a contract.

## Production caveats (not blockers)

- The module must be signed or built in. The kernel has `CONFIG_MODULE_SIG=y` but `CONFIG_MODULE_SIG_FORCE` is not set. The appliance would have to build it against the pinned 7.0 kernel; it taints the spike kernel as unsigned.
- Lifecycle: refcount against in-flight callbacks (caveat 6), ownership and permissions on the control device, behavior across control-plane restart, and who closes the fds on abort (caveat 7).
- The spike has no RLIMIT, no quota and no multi-tenant accounting. It does not charge the target legs to a memory cgroup.
- Out-of-tree kernel code is a security and maintenance surface that the userspace pump does not have.

## Gate recommendation

**PROMOTE evmod to DESIGN as a user-decision candidate for the bounded fast path. Do not treat the in-kernel module as accepted.**

It is the only mechanism tested that is bounded, byte-exact, FIN-correct, and matches or beats Aya at 16,384 peers. It is also a contract-shaping choice (an out-of-tree module in the appliance image versus the userspace pump), so it needs an explicit user decision per `.claude/rules/design.md`.

Before DESIGN accepts it, re-run on the metal host once it is reachable:
1. host-role V-3 (`bench-v3`);
2. the final build's lifecycle set, including `tcp_abort`, `rmmod` with a live pair, and the tight-buffer scenarios;
3. unbound mode at 16,384 peers;
4. kTLS coexistence.

## Commands and evidence

Everything is under `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-evmod/`, uncommitted. `crates/`, `specs/` and `docs/` were not touched.

```
./sync.sh                                   # tar to /tmp/evmod-work on the host; no rsync --delete
python3 launch.py <name> '<remote command>' # canonical lease protocol (infra/metal/lease-holder.sh)
python3 launch.py build4 'bash /tmp/evmod-work/src/driver/build.sh'
python3 launch.py vm-A "python3 /tmp/evmod-work/src/driver/vmrun.py ev-stall-A,ev-long-A,…"
python3 launch.py perf-c1 "cd /tmp/evmod-work && python3 src/bench/run.py --max 16384 --cells kernel-tcp:1,aya-tcp:1,evmod-tcp:1 --tag perf-c1 --no-build"
python3 perf_table.py evidence/bench-perf-c1-* evidence/bench-perf-c2-* evidence/bench-perf-c3-*
python3 final_tables.py; python3 report_vm.py out/vm-A.txt
```

- **Sources:** `module/`, `probe/` (evprobe), `bench/` (frozen perf harness derived from kernel-vs-aya-benchmark increment-r), `bench-v3/` (untested), `driver/vmrun.py`.
- **Raw evidence:** `evidence/vm-*` (battery, controls, lifecycle, tp variants) and `evidence/bench-*` (perf cohorts).
- **Artifact hashes:**
  - guest kernel `b51367c7…`;
  - Cloud Hypervisor `eadddf1b…`;
  - `valveprobe` `da0b304d…`, `valve-bpf.o` `f3a5e587…`;
  - evprobe `8dd6fa47…` (battery) and `27a4ca7b…` (final);
  - `evmod.ko` `f4228aa0…` (perf), `9eebe7fe…` (battery), `1271015c…` (final).
- **Provenance:** `host-admission-budget.json` and `physical-before/after.json` in each bench dir. The physical host's `/proc/modules` and links were asserted unchanged after every run.
