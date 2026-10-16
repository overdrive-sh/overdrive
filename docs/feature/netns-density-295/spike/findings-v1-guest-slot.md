# V-1 findings: a non-reading application and the drain on a vsock-source direction

**Verdict: HOLDS for the ≤128 KiB claim, with an important qualification. DOESN'T-WORK for "never reads, any tail".**
- Every tail ≤128 KiB drained to `sent ≥ goal` with an application that never reads. That is 100/100 trials in each of 4 KiB, 16 KiB, 64 KiB, 96 KiB and 128 KiB, in both directions.
- The same held for every tail up to 2.5 MiB on a loopback-MTU path and up to 384 KiB on a 1500-MTU path.
- Beyond a threshold the drain never completes. It plateaus at a fixed `sent` and stays there until the application closes (`sent` unchanged between samples 2.5 s and 5 s after the source EOF, 100/100 per cell).
- The threshold is set by the target socket's send buffer (`sk_sndbuf`, not `sk_rcvbuf`) plus the unread window of the application's receive buffer.

Probe code is in `spike-scratch/netns-density-295-v1/`. It is a copy of the v27 drain probe, which is the aya `sk_skb` `SockHash` forwarder with the dual-counter fexit. I added a never-read application, a settle sampler, and `--dest-mss`.
- Nothing under `crates/`, `specs/` or `docs/` was touched. It is untracked and nothing was committed.
- I did not reuse the ka5 harness. ka5 only has a guest agent for the K-A5 close cases, with no `sk_skb` verdict or drain machinery. I reused its `launch.py` and substrate-preflight pattern, and ran the v27 `vmrun.py` driver on top.

## Substrate

- **Hardware:** native x86_64 AMD EPYC 8024P. `systemd-detect-virt` = `none`, KVM API 12. The fail-closed preflight passed before every VM boot (`evidence/*/substrate-probes.json`).
- **Kernels:** host `uname -r` = `7.0.0-29-generic`, guest `uname -r` = `7.0.0-29-generic`. Source cites below are `vendors/linux` at tag `v7.0`.
- **VMM:** Cloud Hypervisor sha256 `eadddf1b…` (fork `41a619d19`), `--vsock cid=3360,backend=vhost-kernel`, 4 vCPU, 2 GiB. The guest kernel is the stock `/srv/vm/overdrive-testing/kernel`, sha256 `b51367c7…`.
- **Probe artifacts:** `drain-bpf.o` sha256 `7dfb4344…` (same object as the T1–T6 spike), `drainprobe` sha256 `b53afa9d…`.
- **Sysctls in the guest and on the host:** `tcp_rmem=4096 131072 15350272` (guest) and `4096 131072 33554432` (host), `tcp_wmem=4096 16384 4194304`, `tcp_adv_win_scale=1`, `tcp_moderate_rcvbuf=1`, `lo_mtu=65536`.
- **How it ran:** the metal lease was acquired through `launch.py`, with no `--delete` rsync, all files in `/tmp/v1-work`. Only my own CH processes were started. The VM was reaped and I confirmed no processes, `v1-*` cgroups or lease owner file were left behind.

## Hypothesis, prediction, falsification

- **Hypothesis:** the target of a vsock-source direction is a TCP socket whose peer application never reads. `sk_psock_backlog` retries with `-EAGAIN` while the target's send queue is full, so `sent` can only reach the goal while the tail fits in the target's send queue plus the application's receive window.
- **Prediction:** `sent == goal` for small tails. For a large tail, `sent` plateaus at roughly the target's unacknowledged send queue plus the application's unread bytes, independent of the tail size. The plateau should be larger on a 64 KiB-MTU loopback path (larger `sk_sndbuf` autotune) than on a 1500-MTU path.
- **Falsified if:** a tail ≤16 KiB failed to reach `sent ≥ goal` (that would pin a slot), or the plateau moved with the tail size.

## What was driven (production-shaped)

The probe is the v27 forwarder. An `SK_SKB` stream verdict redirects through a `SockHash` from the vsock source into the TCP target. `sent` is the sum of `fexit(skb_send_sock)` return values, taken as the maximum of two independent counters. The owner sets `goal` and waits for a ringbuf record. This is the same instrumentation the T1–T6 drain probes used.

Each cell is a direction on the real microVM, vsock over vhost-kernel:

- **G:** the guest application never reads.
  - The host server sends M bytes, then a FIN.
  - Source is the guest vsock, target is the guest intake child (guest TCP socket), and the app is that socket's peer.
- **H:** the host destination application never reads past its 56-byte header.
  - The guest client writes N bytes, then a FIN.
  - Source is the host vsock, target is the host destination socket.
  - The client's `write` of the full N bytes always succeeded. The vsock credit is returned at `read_skb` time, so the guest-side write is not throttled (F3 of the research, confirmed).
- **Hold and sampling:**
  - The never-reading application holds 7 s with its socket open.
  - `sent`, `goal` and `fwd` for the draining direction are sampled 2.5 s and 5 s after the source EOF.
  - The role also records the target socket's `SO_SNDBUF` and `SIOCOUTQ` and the application's `SO_RCVBUF` and `FIONREAD`.
- **Reader controls:** both applications read, 1 MiB and 16 MiB tails.
- **Trials:** N = 100 per cell, 50 for the receive-buffer cells, so about 6,300 trials in total. VIOLATION counters = 0 across all cells.

## Commands

The pilots are `evidence/vm-sweep-20261015T101735Z` and `T102417Z`.

```
python3 spike-scratch/netns-density-295-v1/sync.sh            # builds on the metal host in /tmp/v1-work
python3 spike-scratch/netns-density-295-v1/gen_spec.py A|B|C > spec-{A,B,C}.json
python3 spike-scratch/netns-density-295-v1/launch.py runA 'cd /tmp/v1-work && python3 vmrun.py sweep /tmp/v1-work/spec-A.json'   # evidence/vm-sweep-20261015T102556Z
... runB (spec-B.json)   # evidence/vm-sweep-20261015T104129Z
... runC (spec-C.json)   # evidence/vm-sweep-20261015T105016Z
python3 spike-scratch/netns-density-295-v1/analyze.py <evidence dir>/    # full tables in evidence/analysis.txt
```

## Results

Columns: trials where the direction reached `sent ≥ goal` ("complete"), trials stuck at the settle sample, and the stuck `sent` plateau. A plateau means `sent` was identical in the 2.5 s and 5 s samples in every stuck flow.

### Series A: default stack (guest `lo` MTU 65536, host destination unclamped)

| Tail | G complete / stuck | H complete / stuck | `sent` plateau (G / H) |
|---|---|---|---|
| 4 KiB | 100 / 0 | 100 / 0 | n/a |
| 16 KiB | 100 / 0 | 100 / 0 | n/a |
| 64 KiB | 100 / 0 | 100 / 0 | n/a |
| 96 KiB | 100 / 0 | 100 / 0 | n/a |
| **128 KiB** | **100 / 0** | **100 / 0** | n/a |
| 192 KiB | 100 / 0 | 100 / 0 | n/a |
| 256 KiB | 100 / 0 | 100 / 0 | n/a |
| 512 KiB | 100 / 0 | 100 / 0 | n/a |
| 1 MiB | 100 / 0 | 100 / 0 | n/a |
| 2 MiB | 100 / 0 | 100 / 0 | n/a |
| 2.375 MiB | 100 / 0 | 100 / 0 | n/a |
| 2.5 MiB (2,621,440) | 100 / 0 | 100 / 0 | n/a |
| 2.625 MiB (2,752,512) | 0 / 100 | 0 / 100 | 2,695,828 / 2,695,141 |
| 2.75 MiB | 0 / 100 | 0 / 100 | 2,695,828 / 2,695,141 |
| 3 MiB | 0 / 100 | 0 / 100 | 2,695,828 / 2,695,141 |
| 4 MiB | 0 / 100 | 0 / 100 | 2,695,828 / 2,695,141 |
| 16 MiB | 0 / 100 | 0 / 100 | 2,695,828 / 2,695,141 |

Per-flow plateaus are between 2,695,141 and 2,754,560, medians identical to the cell minimum, and `fwd` equals the goal in every stuck flow. Reader controls (both applications read): 100/100 complete at 1 MiB and at 16 MiB, `sent == goal` in 100% of the 400 vsock-source directions.

### Series B: 1500-MTU path (guest `lo` MTU 1500, host destination `TCP_MAXSEG` 1448)

This emulates a typical NIC path. The `TCP_MAXSEG` clamp is an emulation, not a real 1500-MTU interface on the host.

| Tail | G complete / stuck | H complete / stuck |
|---|---|---|
| 4 KiB – 384 KiB (4, 16, 64, 96, 128, 160, 192, 224, 256, 320, 384 KiB) | 100 / 0 in every cell | 100 / 0 in every cell |
| 512 KiB | 12 / 88 (plateau median 479,600) | 98 / 2 (plateau ~510,152) |
| 1 MiB | 0 / 100 (median 479,600) | 0 / 100 (median 709,924) |
| 4 MiB | 0 / 100 (median 479,600) | 0 / 100 (median 709,800) |

### Series C: receive-buffer sweep at an 8 MiB tail (50 trials per cell, default loopback path)

| App `SO_RCVBUF` set | G `sent` plateau, `outq`, app unread | H `sent` plateau, `outq`, app unread |
|---|---|---|
| 16 KiB (reads back 32,768) | 2,412,738 = 2,386,614 + 26,124 | 2,443,264 (outq 2,414,592, unread 28,616) |
| 128 KiB (reads back 262,144) | 2,815,822 = 2,596,504 + 219,318 | 2,829,250 |
| 1 MiB (reads back 2,097,152) | 6,222,475 = 4,156,000 + 2,066,596 | 6,237,522 |
| 4 MiB (reads back 8,388,608) | no plateau, 50/50 complete (unread 8,302,377) | no plateau, 50/50 complete |

## Analysis

- **The plateau is exact.** `sent` equals the target's `SIOCOUTQ` (unacknowledged bytes in its send queue) plus the application's `FIONREAD` (bytes unread in its receive queue).
  - 2,585,600 + 110,228 = 2,695,828 for G on the default stack.
  - 355,408 + 124,192 = 479,600 for G on 1500 MTU.
- **The send buffer dominates.** The target's `SO_SNDBUF` was 2,626,560 on the loopback path (356,864 on G at 1500 MTU, 574,464 on H). Autotuned `sk_sndbuf` for a local TCP connection is about 2 × max(10, cwnd) × per_mss, capped by `tcp_wmem[2]` (4 MiB). The per-MSS size follows the MSS, so a 65483-byte loopback MSS gives about 2.6 MiB and a 1448-byte MSS gives hundreds of KiB.
  - `tcp_sndbuf_expand` is `net/ipv4/tcp_input.c:605-635 @ v7.0`; it runs from `tcp_init_buffer_space` (`:741-747`) and `tcp_new_space` (`:6094-6099`).
  - The application's `tcp_rmem[1]` default of 131072 matters little: it contributes only the 110–125 KB unread window. With an app `SO_RCVBUF` of 16 KiB the plateau is still about 2.4 MB.
  - Only an application receive buffer of several MiB (the 4 MiB cell, reading back 8 MiB) lets an 8 MiB tail drain fully.
- **Why `sent` stops.** `sk_psock_backlog` calls `skb_send_sock` and, on `-EAGAIN`, requeues the skb and reschedules with a one-jiffy delay (`net/core/skmsg.c:710-728`). `__skb_send_sock` sends with `MSG_DONTWAIT`, and a TCP send is refused once `sk_wmem_queued >= sk_sndbuf` (`include/net/sock.h:1409-1412`, `net/ipv4/tcp.c:1261`). The skbs already forwarded sit in the psock's `ingress_skb` queue, which `sk_psock_skb_redirect` appends to with no accounting limit (`skmsg.c:985-992`).
  - That is the deficit: `fwd - sent` is exactly `goal - plateau` in the stuck flows (`fwd` = goal in every one).
  - For the 16 MiB tail, about 14 MB per flow is held in the kernel.
- **The client's write is not throttled in H.** The guest client's `send` succeeded for the full 16 MiB with the host server not reading (`sent == N`, no `send_err` in 100/100 rows). Vsock credit is returned at `read_skb`, before the target accepts anything (research F3, now measured).
- **The plateau is not a constant.** It varies from about 0.48 MiB to about 2.7 MiB depending on MSS and autotune state, and on the receiver's window. A mean of 710 KB in H at 1500 MTU versus 480 KB in G shows the spread.
  - The "128 KiB or less" claim holds on all paths measured, with margin: smallest failing tail 512 KiB (1500 MTU, 88/100 stuck in G) and 2.625 MiB (loopback), both at least 4× the claim.
  - The claim is wrong as a general statement ("a tail the pool bounds is always drainable"). It is true only for tails below roughly 400 KiB on a NIC-like path.

## Per-claim table

| Claim | Result |
|---|---|
| Tail ≤128 KiB drains with a non-reading application | HOLDS, 0 stuck in 1,200 trials in series A and 1,000 in series B (G and H, every size 4–128 KiB) |
| A ≤16 KiB tail pins the slot (the falsification condition) | NOT FALSIFIED, 0 stuck in 800 trials in A and B |
| Drain completes at any tail without a reader | FALSIFIED. Stops completing above about 2.6 MiB (loopback) or about 400–500 KiB (1500 MTU), 100% stuck, no deadline |
| Plateau scales with tail size | NO. It is flat from 2.75 MiB to 16 MiB |
| Plateau follows target `sk_rcvbuf` | NO, it follows the target's `sk_sndbuf` plus the peer application's unread window |
| Both directions behave the same | YES within a path (G vs H plateau within about 0.5% on loopback, about 1.5× apart on 1500 MTU) |

## Design implications

- **PSO-1's trigger needs only one extra fact.** A probe or app that half-closes cleanly after an unread tail ≤128 KiB does not hold a slot by itself. If a probe tail exceeds the send-buffer-plus-window capacity and the application never reads, the drain never completes. With no deadline the slot is held until the application closes or the flow is aborted.
- **The bound the pool relies on is a function of kernel autotune.** The plateau is the target's `sk_sndbuf` (up to `tcp_wmem[2]`) plus the application's receive window. A tail of ≲400 KiB is safe on a NIC-like path, and ≲2.5 MiB on loopback, but neither is a constant to design against. A byte-size bound on the drain would have to be conservative (under a few hundred KiB) or read the target's actual `SIOCOUTQ` and `SO_SNDBUF`.
- **F3 is real.** In every stuck cell the unaccepted bytes sit in the kernel's psock queue: `fwd − sent` up to about 14 MB per flow in the 16 MiB case. The quota/slot count does not bound memory.
- **Ground-the-premise note:** every non-completing flow here comes from an application that does not read at all while a tail larger than the plateau is in flight. That is the state V-1 asked about; I did not check whether `serve` plus `deploy` can produce it.

## Gate recommendation

**PROMOTE the finding, do not PROMOTE a "≤128 KiB always drains" contract as stated.**
- It is safe to say "tails ≤128 KiB drained to `sent ≥ goal` in 100% of trials, with margin of at least 4× on both paths".
- It is not safe to say the pool bounds draining bytes. State the plateau as `sk_sndbuf + unread rcv window` with a measured range (about 0.48–2.7 MiB), and open the F1/F2 byte-bound item (D-4) rather than withdrawing it.
- Whether to withdraw PSO-1 depends on whether the HTTP probe half-closes with a tail above about 400 KiB, and on whether production TCP targets look like the loopback or the 1500-MTU cells. Both are still open.

## Not run / caveats

- The 1500-MTU series is an emulation: the guest `lo` MTU set to 1500 and `TCP_MAXSEG` 1448 on the host destination leg. A real NIC/veth path, or a clamped intake child on the host side, was not measured.
- The host's `tcp_rmem` default is 131072 as in the guest. The reader controls ran only at 1 MiB and 16 MiB.
- I did not read the 7.0 `tcp_select_window` path, so the 110–125 KB unread window is measured, not derived.
- Memory growth (V-3) was not recorded. The 14 MB per-flow figure is `fwd − sent`, not a `Slab`/`SUnreclaim` reading.
- The settle sample at 2.5 s and 5 s is bounded by the application's 7 s hold. The plateau was never observed beyond 5 s after EOF.
- Reader hold-end data (`app_unread`) is sampled at the end of the 7 s hold, so it can exceed the value at the 2.5 s sample by a few KB (spread within a cell is about 110–130 KB).

Key files, all under `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-v1/`:
- `probe/src/fwd.rs` and `probe/src/apps.rs` (settle sampler, never-read apps)
- `vmrun.py`
- `gen_spec.py`
- `analyze.py`
- `evidence/analysis.txt`
- `evidence/vm-sweep-20261015T102556Z`, `T104129Z` and `T105016Z`
