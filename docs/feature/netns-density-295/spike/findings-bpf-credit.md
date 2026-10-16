# bpf-credit spike: no BPF hook can defer vsock credit; a 35-line kernel change can (measured, guest receive side only)

Both host and guest ran `uname -r` = `7.0.0-29-generic`. The host is an AMD EPYC 8024P with `systemd-detect-virt` = `none`, and the fail-closed preflight passed on every run. The VMM was the Cloud Hypervisor fork (sha `eadddf1b…`) with `--vsock backend=vhost-kernel`, 4 vCPU and 1 GiB. All kernel citations are `vendors/linux @ v7.0`.

## Verdict

- **BPF-only: DOESN'T-WORK.** No BPF program type on stock 7.0.0-29 can make vsock return credit only when redirected bytes are consumed downstream. None can bound the redirect queue without killing the flow either.
- **A 35-line kernel change in `virtio_transport_read_skb` WORKS (measured, one direction only).**
  - It is a rebuilt out-of-tree `vmw_vsock_virtio_transport_common.ko` (sha `b6eae8ec…`, vermagic matches 7.0.0-29, loaded only into my own guest). The patch is shown under "Smallest kernel change" below.
  - Direction A is a host sender into a guest owner with the sockmap redirect to a non-reading TCP target.
  - Direction B (guest sender, host owner) needs the host module reloaded, which I did not do on the shared host.
  - Direction T (TCP source) is unaffected and already has a BPF-only fix, the park valve from `findings-v3-valve.md`.
- **Comparison (a)–(c) with the P1 prototype:**

| Measure | Unmodified path (`mod=stock`) | P1 patch (`mod=p1`) |
|---|---|---|
| Queue with a non-reading target (b) | hits the 160 MiB abort cap in 0.28 s | holds at exactly `buf_alloc` (0.25 MiB at the default 256 KiB window; 1.0 and 4.0 MiB at 1 and 4 MiB windows) |
| Guest memory (`SUnreclaim`) rise | +176 to +187 MiB | +0.1 MiB (max 0.8) |
| Sender | never blocks | stalls through credit (blocked 2.96 s after 2.81 MiB) |
| Stall/resume, slow-steady and mid-stall-close scenarios | not usable (see tables) | all clean, no kernel BUG/WARN |
| Single-flow throughput vs stock | — | −35 to −40% at 256 KiB; about −5 to −20% at 1 MiB |

- **Not measured:**
  - Direction B on the host kernel.
  - (c) at 64 and 16,384 peers on the kernel-vs-aya-benchmark harness. That harness is six private VMs and 72 minutes per run, and P1 would need the patched module in its host-role VM kernel. The single-flow rows below stand in for it.
  - Task counts: the patch adds only system-workqueue use.
- **Deviation from the brief:** the question said "without a kernel patch or module". P1 was an extra step to validate the "smallest kernel change" I was asked to name. It lives only under `spike-scratch/`. The gate section lists what needs a user decision.

## Substrate, harness and deviations

- Everything ran from `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-bpf-credit/` (own workspace). `crates/`, `specs/` and `docs/` were not touched.
- I used the valve launcher: the canonical lease (`infra/metal/lease-holder.sh`), a private work dir `/tmp/bpf-credit-work`, and no rsync of the shared tree. One job at a time; the lease was free on every acquisition.
- **eBPF.**
  - The sockmap scenarios use the valve's aya-ebpf program unchanged (`valve-bpf.o` sha `f3a5e587…`, the same pin as in `findings-v3-valve.md`).
  - The eligibility probe (`elig/`, Rust and libc only, no C) builds raw `bpf(2)` programs with 2–4 hand-assembled instructions. Aya 0.13.1 cannot express `fmod_ret` or a module `attach_btf_obj_fd`.
  - The verifier decides eligibility at load from `prog_type`, `expected_attach_type` and `attach_btf_id`, so the body does not matter.
- **Kernel C.** The P1 change is kernel C; it is not eBPF. It is built out of tree in `kmod/` against the host's own `/lib/modules/7.0.0-29-generic/build`. I built two variants from the v7.0 source: `rebuilt` (unmodified, as a control) and `p1`.

## Commands

```
cd spike-scratch/netns-density-295-bpf-credit
./sync.sh                                         # push bpf/ probe/ elig/ to /tmp/bpf-credit-work, build there
ssh <metal> bash /tmp/bpf-credit-work/src/kmod/build.sh   # rebuilt + p1 module variants
python3 launch.py <tag> "python3 /tmp/bpf-credit-work/vmrun.py <scenario,...> [mod=stock|rebuilt|p1]"
python3 report.py evidence/<run-dir>              # condense result.json
```

Runs:

- `elig`
- `ctl-A,ctl-B,ctl-T,pass-A,hard-A,knobs-A,ingress-T,park-A` with `mod=stock`
- `p1stall-A`, `p1long-A`, `p1slow-A`, `p1slow2-A`, `p1abort-A`, `p1cyc-A` with `mod=p1`
- `tp-sockmap-A,lat-sockmap-A,fast-off-A` in the order stock, p1, rebuilt, p1, stock
- `p1tp{1m,4m,16m}-A`, `p1stall{1m,4m}-A` on p1 and stock
- `p1ver1m-A`, `lat1m-A`, `p1cyc1m-A`, `p1slow1m-A`, `p1abort1m-A` on p1, stock, p1

Raw JSONL and serial logs are in `evidence/vm-*/`.

## Experiment 1: hook eligibility on the live kernel

**Hypothesis.** `fmod_ret` is allowed only for `ALLOW_ERROR_INJECTION` functions or names starting `security_` (`kernel/bpf/verifier.c:24928-24935`, call site `:25278`). `ALLOW_ERROR_INJECTION` is not set on any vsock or skmsg function. The two credit-return statics (`virtio_transport_send_credit_update` at `virtio_transport_common.c:524` and `virtio_transport_dec_rx_pkt` at `:455`) are probably inlined, so they would not exist as symbols at all.

**Predicted.** `fmod_ret` is rejected with `is not modifiable` for all vsock and skmsg targets. `fentry` and `fexit` load fine. Writes through a traced pointer are rejected. `kprobe` plus `bpf_override_return` is rejected at attach.

**Falsified if.** Any vsock or skmsg function accepts `fmod_ret` or the override.

**Result.** Same on the host (modules loaded) and in the guest (modules insmod'd). The positive controls loaded and attached, so the probe itself is sound.

| Target | fentry | fexit | fmod_ret | kprobe + override | BTF write |
|---|---|---|---|---|---|
| `virtio_transport_read_skb` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `virtio_transport_inc_tx_pkt` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `virtio_transport_recv_pkt` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `virtio_transport_notify_set_rcvlowat` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `virtio_transport_get_credit` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `virtio_transport_recv_enqueue` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `vsock_data_ready` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `vsock_set_rcvlowat` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `sk_psock_verdict_recv` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `sk_psock_verdict_data_ready` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `sk_psock_skb_redirect` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `sk_psock_verdict_apply` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `sk_psock_backlog` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `skb_send_sock` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `tcp_read_skb` | ok | ok | rejected EINVAL | attach EINVAL | rejected |
| `virtio_transport_send_credit_update` | absent (no BTF, not in kallsyms) | absent | absent | absent | absent |
| `virtio_transport_dec_rx_pkt` | absent | absent | absent | absent | absent |
| `sk_psock_handle_skb` | absent | absent | absent | absent | absent |
| control `security_socket_recvmsg`, `security_socket_sendmsg` | ok | ok | ok (name exemption) | attach EINVAL | rejected |
| control `page_pool_alloc_netmems`, `__x64_sys_iopl`, `__x64_sys_sendto` | ok | ok | ok | ok | rejected |

Pasted lines (host; the guest is identical):

```
{"row":"symbol","func":"virtio_transport_send_credit_update","where":"vmw_vsock_virtio_transport_common","btf_id":-1,"linkage":"-","kallsyms":"absent"}
{"row":"tracing","func":"virtio_transport_read_skb","mode":"fmod_ret","load":"rejected","errno":"22 (Invalid argument ...)","verifier":"virtio_transport_read_skb() is not modifiable | processed 0 insns ..."}
{"row":"tracing","func":"virtio_transport_read_skb","mode":"fentry-btf-write","load":"rejected","errno":"13 ...","verifier":"1: (62) *(u32 *)(r2 +0) = 0 | only read is supported | ..."}
{"row":"kprobe_override","func":"virtio_transport_read_skb","load":"ok","attach":"rejected","errno":"22 (Invalid argument ...)"}
{"row":"tracing","func":"security_socket_recvmsg","mode":"fmod_ret","load":"ok","attach":"attached"}
{"row":"kprobe_override","func":"page_pool_alloc_netmems","load":"ok","attach":"ok"}
```

- **Ground truth from the running kernel.** The stock `error_injection/list` has 1020 entries on the host and 1013 in the guest. `grep -E 'vsock|psock|skmsg|virtio|sock_map|tcp_|skb_send|page_pool|net_realloc'` returns exactly one line, `page_pool_alloc_netmems NULL`. The host config has `CONFIG_BPF_KPROBE_OVERRIDE=y`. The active LSM list is `lockdown,capability,landlock,yama,apparmor,ima,evm`, so BPF-LSM is not active.
- **Source for each rejection.**
  - fmod_ret gating: `kernel/bpf/verifier.c:24928-24935`; error-injection annotations in the networking tree are only `net/core/page_pool.c:671` and `net/core/skb_fault_injection.c:31`.
  - The only other fmod_ret path is a kfunc set registered by mptcp (`net/mptcp/bpf.c:34`) and the test code (`net/bpf/test_run.c:1836`).
  - Kprobe override requires the opt-in list and function entry: `kernel/trace/bpf_trace.c:1932-1936`.
  - Tracing programs cannot write kernel memory: `kernel/bpf/verifier.c:7435-7452`.
- **Why fentry and fexit are not enough.** They can only observe (the valve already uses them for counters). They cannot change the `:1779` credit call or the `:1776` `fwd_cnt` advance.

## Experiment 2: other hook families (source plus live BTF)

- **struct_ops.** Registrations in v7.0 are `net/sched/bpf_qdisc.c`, `net/ipv4/bpf_tcp_ca.c`, `net/smc/smc_hs_bpf.c`, `net/bpf/bpf_dummy_struct_ops.c`, `drivers/hid/bpf/hid_bpf_struct_ops.c` and `kernel/sched/ext.c`. The live kernel's BTF lists `Qdisc_ops`, `bpf_dummy_ops`, `hid_bpf_ops`, `sched_ext_ops` and `tcp_congestion_ops`. None touches vsock or skmsg.
- **Helpers in `sk_skb`.**
  - `sk_skb_func_proto` (`net/core/filter.c:8638-8676`) has no `setsockopt` and no `probe_read`.
  - It does reach `bpf_skc_to_tcp_sock` through the base protos (`:12081-12090`), so a verdict can read the TCP target's `sk_wmem_queued`.
  - The verdict already runs after credit is returned. The valve measured this as `pass-A` and `hard-A`, and I re-measured both below.
- **Setting `SO_RCVLOWAT` from BPF.**
  - `bpf_sk_setsockopt_proto` is exposed only to cgroup sockopt SETSOCKOPT (`kernel/bpf/cgroup.c:2522`), the tcp and unix iterators (`net/ipv4/tcp_ipv4.c:3690`, `net/unix/af_unix.c:3890`), `tcp_congestion_ops` (`net/ipv4/bpf_tcp_ca.c:171`), and LSM-cgroup on three socket-creation hooks (`kernel/bpf/bpf_lsm.c:264-267`).
  - None fires on the vsock rx path, and the lowat gate still has the race the valve measured.
- **`BPF_PROG_TYPE_SK_MSG` and sockmap ingress.**
  - vsock overrides only `close`, `recvmsg` and `sock_is_readable` (`net/vmw_vsock/vsock_bpf.c:129-131`). There is no `sk_psock_msg_verdict` anywhere in `af_vsock.c` or `virtio_transport_common.c`, so `sk_msg` never runs for vsock.
  - Ingress mode is measured in `ingress-T` below.
- **`SO_VM_SOCKETS_BUFFER_SIZE`.** Measured in `knobs-A` below: `buf_alloc` is the size of the vsock rx queue, but credit returns at `read_skb`, so it does not bound the redirect queue.
- **LSM and syscall `fmod_ret` hooks** (`security_socket_sendmsg`, `__x64_sys_sendto`) are attachable. They only veto the local sender syscall with an error, not a credit stall, and cannot see the receiver's queue across the VM boundary. I tested load and attach only.

## Experiment 3: (a)/(b) baseline and the verdict-side options on the unmodified path

**Hypothesis.** Credit is returned at `virtio_transport_read_skb` before the verdict runs (`virtio_transport_common.c:1776` dec_rx_pkt, `:1779` credit update, `:1781` `recv_actor`). The sockmap redirect queue is then unbounded (`net/core/skmsg.c:989`).

**Predicted.** A non-reading target fills memory in both directions. SK_PASS, a hard drop, buffer tuning and ingress mode do not bound it while keeping the flow.

**Falsified if.** The queue plateaus near `buf_alloc` or the sender blocks.

**Result** (`mod=stock`, `evidence/vm-stock-ctl-A+…-20261016T083423Z`, 256 MiB streams, 3 trials each):

| Scenario | Outcome | Peak queue | `SUnreclaim` rise | Sender | Intact |
|---|---|---|---|---|---|
| ctl-A (vsock source) | `abort_cap` 3/3 at 0.28 s | 160.2 MiB | +176 to +187 MiB | `max_send_ms` 0–1, never blocked | 0/3 |
| ctl-B (host owner) | `abort_cap` 3/3 at 0.14 s | 160.4–161.5 MiB | host `MemAvailable` −114 to −118 MiB | `max_send_ms` 0 | 0/3 |
| ctl-T (TCP source) | `abort_cap` 3/3 at 0.11 s | 161.9–169.3 MiB | `MemAvailable` −137 to −164 MiB | — | 0/3 |
| pass-A (SK_PASS at 4 MiB) | `timeout` | 4.0 MiB then parked | +372 MiB, 215,508 skbs passed | `max_send_ms` ≤ 3 | 0/3 |
| hard-A (drop at 8 MiB) | `timeout` | 8.0 MiB exactly | +9 MiB | never throttled | 0/3 (212,052 skbs dropped, flow dead) |
| knobs-A (64 KiB buf, 4 KiB `SNDBUF`) | `abort_cap` | 160.4 MiB | +172 to +178 MiB | never blocked | 0/3 |
| ingress-T (`BPF_F_INGRESS`) | `timeout` | 128 MiB | `MemAvailable` −128 MiB | — | 0/1 |
| park-A | `abort_cap` | 4 MiB then unbounded | +201 to +236 MiB | never blocked | 0/3 |

V-3 scenario over time, ctl-A trial 0 (the sender is never throttled; the target holds 2.6 MiB):

```
t=0.00s fwd=1.5   sent=1.5 q=0.12   sunrec=19.0
t=0.05s fwd=27.6  sent=2.6 q=24.31  sunrec=47.9
t=0.11s fwd=53.6  sent=2.6 q=50.56  sunrec=79.3
t=0.21s fwd=117.1 sent=2.6 q=113.80 sunrec=155.2
t=0.26s fwd=150.0 sent=2.6 q=146.86 sunrec=194.4
```

- **Answer to "does credit still leak at `:1779` when the verdict stops consuming?"** Yes. `pass-A` and `hard-A` never throttled the sender. The queue is capped by dropping data, not by backpressure.
- **Even a slow but reading target is a problem.** At a 1 MiB window with a 20 MiB/s reader (`p1slow1m-A`, stock), the redirect queue peaked at 120 MiB and `SUnreclaim` rose by 144 MiB.

## Experiment 4: smallest kernel change (P1, credit released on skb free)

**Hypothesis.**
- Credit is the peer-visible `fwd_cnt`. It is advanced by `virtio_transport_dec_rx_pkt` (`virtio_transport_common.c:460`) and stamped on every outgoing header (`:468-469`).
- `read_skb` advances it before the redirect runs (`:1776-1779`).
- The receiving skb already holds a socket reference and `sock_efree` as its destructor (`skb_set_owner_sk_safe`, `:1654`; `include/net/sock.h:2456-2465`).
- So if `buf_used` and `fwd_cnt` advance from the skb's destructor instead, credit returns only when the redirect has finished with the skb. That is a send into the TCP target, a drop, or a purge.

**Predicted.** The queue stalls at `buf_alloc`, the sender blocks through credit, memory stays flat, the stream is intact and resumes.

**Falsified if.** The queue exceeds `buf_alloc`, any byte is lost, reordered or duplicated, a kernel BUG or WARN fires, or the flow deadlocks.

**The change** (v7.0 `virtio_transport_common.c`; the full diff is `kmod/virtio_transport_common.c` against `.v7.0`):

```c
/* in virtio_transport_read_skb(), replacing :1776-1781 */
virtio_transport_dec_rx_pkt(vvs, pkt_len, 0);          /* rx_bytes only */
spin_unlock_bh(&vvs->rx_lock);
skb->destructor = virtio_transport_credit_free;        /* skb->sk already holds the sock ref */
return recv_actor(sk, skb);

/* destructor: queue a work (GFP_ATOMIC; context may be atomic) that does
 *   rx_lock { vvs->buf_used -= len; vvs->fwd_cnt += len; }
 *   virtio_transport_send_credit_update(vsk); sock_put(sk); */
```

- **Where it applies.** `read_skb` is common code for both guest virtio-vsock and host vhost-vsock, so one change should cover both directions. I measured only the guest side.
- **Only the receiver controls the bound.** The sender's credit is limited by the receiver's `buf_alloc` (`virtio_transport_get_credit`), so a hostile sender cannot raise it.
- **Prototype limits.** The work allocation uses `GFP_ATOMIC`. If it fails, that skb's credit is lost (never hit in the runs, but a flow could stall). It also needs a lifetime and `rcu` review before anyone upstreams it.

**Result, direction A, p1 module** (all runs `outcomes complete` and byte-verified via splitmix hash equality, unless noted):

| Scenario | Trials | Intact | Peak queue | `SUnreclaim` rise (max) | Sender behaviour |
|---|---|---|---|---|---|
| p1stall-A (256 MiB, 3 s stall) | 10 | 10/10 | 0.25 MiB | 0.8 MiB | blocked 2.96 s after 2.81 MiB sent; `resume_latency` p50 2.36 ms |
| p1long-A (10 s stall) | 3 | 3/3 | 0.25 MiB | 0.6 MiB | blocked 9.96 s, flat for the whole stall |
| p1slow-A (20 MiB/s reader) | 10 | 10/10 | 0.25 MiB | 0.3 MiB | sender 20.45 MiB/s, reader 20.0 |
| p1slow2-A (2 MiB/s reader) | 5 | 5/5 | 0.25 MiB | 0.2 MiB | sender 2.4 MiB/s |
| p1cyc-A (4 stall/resume cycles) | 100 | 100/100 (25.6 GiB) | 0.25 MiB | 0.3 MiB | `max_send_ms` 259–293 |
| p1abort-A (flow closed mid-stall) | 20 | n/a (timeout by design) | 0.25 MiB | 0.3 MiB; after close −0.2 to +0.2 | no BUG/WARN lines in `serial.log`, no drift |
| p1cyc1m-A (1 MiB window) | 100 | 100/100 (25.6 GiB) | 1.0 MiB | 1.2 MiB | — |
| p1slow1m-A (1 MiB window) | 10 | 10/10 | 1.0 MiB | 0.8 MiB | — |
| p1abort1m-A (1 MiB window) | 20 | n/a | 1.0 MiB | 1.1 MiB | — |
| p1stall1m-A / p1stall4m-A | 5 each | 5/5 | 1.0 / 4.0 MiB | 0.9 / 4.2 MiB | blocked 2.96 s |

The same scenarios on the stock path (1 MiB window) failed:

- `p1cyc1m-A`: 71 of 100 trials hit `abort_cap` at 160 MiB.
- `p1stall1m-A` and `p1stall4m-A`: all hit the cap.
- `p1abort1m-A`: peak queue 61 MiB.

Stock stall trace versus P1 (trial 0, 3 s app stall):

```
stock ctl-A:  t=0.26s q=146.86 MiB sunrec=194.4
p1 stall-A:   t=0.16s..2.91s  fwd=2.8 sent=2.6 q=0.25 (flat)   sunrec=19.5 (flat)
              t=3.06s  fwd=16.0 sent=16.0 q=0.13   (resumed)   t=3.84s fwd=256.0 sent=256.0 q=0.00
```

**Not covered.** There was no artificial loss, reorder or duplication injection; the splitmix verifier would have flagged any. I did not test peer RST under load or host-side vhost.

## Experiment 5: performance on the valve harness (not the kernel-vs-aya harness)

Direction A, single flow, 10 × 1 GiB trials, p50. Whole-VM busy CPU is ms per GiB. The runs alternated variants (stock, p1, rebuilt, p1, stock).

| Window | Variant | tp (verify off) MiB/s | verified fast MiB/s | busy ms/GiB (tp / verified) |
|---|---|---|---|---|
| 256 KiB | stock (2 runs) | 463 / 441 | 466 / 530 | 3460–3695 / 3555–3820 |
| 256 KiB | rebuilt, unmodified | 426 | 483 | 3790 / 3755 |
| 256 KiB | p1 (2 runs) | 280 / 299 | 313 / 302 | 4165–4435 / 4755–4875 |
| 1 MiB | stock | 456 | 567 | 3485 / 3585 |
| 1 MiB | p1 (2 runs) | 438 | 455 / 443 | 3925 / 4210–4300 |
| 4 MiB | stock / p1 | 444 / 426 | — | 3625 / 3710 |
| 16 MiB | stock / p1 | 421 / 404 | — | 3700 / 3980 |

Small-record latency (16-byte records every 300 µs, 2000 per trial, 10 trials), p50 µs: stock 75–79 (256 KiB window) and 81 (1 MiB); rebuilt 72; p1 62–65 (256 KiB) and 65–73 (1 MiB). p99 is dominated by guest scheduling noise, as the valve already found: stock 148, 267 and 11,498; rebuilt 1,660; p1 1,600, 17,886, 10,490 and 172. I did not use it as a signal.

- **At the default 256 KiB window P1 pays 35 to 40% single-flow throughput.** The queue bound becomes the true bandwidth-delay product. At 1 MiB the cost is about 5 to 20%, because the stock path's own unbounded buffering partly hides the receiver's pace.
- **Whole-VM CPU per GiB is about +15 to 20%.** I attribute it to the extra credit-update packet and workqueue wake. I did not profile it.

**Reference rows I did not re-run.** The Aya and shmproxy rows of `kernel-vs-aya-benchmark.md`: Aya 0.970 GiB/s and 3.12 core-s/GiB at 16,384 held peers (65,536-byte request/reply), and the shmproxy C module at 0.195 GiB/s. They come from a different workload and cannot be compared with the table above.

## Design implications

1. **The BPF-only design space is closed for a vsock source.** There is no hook, helper or struct_ops that can hold credit past `:1779`. The valve's earlier statement ("only a userspace pump is lossless and bounded") is confirmed at the hook level, and the "any attachable type" question is answered.
2. **TCP source: the park valve (BPF-only) remains correct.** It is not affected by this spike.
3. **vsock source has two bounded, lossless options:**
   - Userspace pump. No kernel change, about 0.5–0.8 owner CPU-s/GiB, roughly 17% slower than the sockmap fast path in direction B, 35% faster in A (valve numbers).
   - P1 kernel change. Keeps the in-kernel fast path and bounds each stalled flow at `buf_alloc`.
4. **P1's memory bound is `flows × buf_alloc` worst case.** Only stalled flows hold it. At the default 256 KiB, 16,384 stalled flows would be 4 GiB; at 1 MiB, 16 GiB. The slot pool therefore needs a per-flow `buf_alloc` budget and a per-flow cap, not just a flow count. I did not measure many flows.
5. **The memory bound sits in the owner's control.** A peer cannot raise it (see Experiment 4).

## Gate recommendation

- **Do not carry this as Open into DISTILL.** BPF hook verdict: DOESN'T-WORK.
- **A decision is needed from the user before DESIGN picks a vsock-source mechanism.** Per `.claude/rules/design.md`, surface it as numbered decisions:
  1. Ship P1 as a patch to the pinned appliance kernel's vsock module (guest virtio and host vhost paths), or
  2. keep the userspace pump for vsock sources and accept its CPU and latency cost.
- **If P1 is chosen, these remain to be proven before DESIGN accepts it:**
  - Direction B on the host kernel (reload `vmw_vsock_virtio_transport_common` under an exclusive lease, or use a dedicated host).
  - Many-flow memory and CPU at 64 and 16,384 flows.
  - Hardening of the destructor: allocation failure, `rcu` and lock context, socket-teardown ordering, and whether `GFP_ATOMIC` plus `schedule_work` is the right shape.
  - The default `buf_alloc` choice, given the 256 KiB throughput cost.
- A `kprobe`-free alternative (a new kernel `ALLOW_ERROR_INJECTION` plus a wake kfunc) is strictly larger than P1 and was not pursued.

## Files

- Probe and sources: `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-bpf-credit/` — `elig/src/main.rs`, `kmod/virtio_transport_common.c` (+ `.v7.0` original, `Kbuild`, `build.sh`), `vmrun.py`, `launch.py`, `sync.sh`, `build.sh`, `report.py`, `bpf/`, `probe/` (copied unchanged from the valve spike).
- Evidence: `evidence/vm-elig-20261016T082729Z/`, `evidence/vm-stock-ctl-A+…-20261016T083423Z/`, `evidence/vm-p1-*`, `evidence/vm-rebuilt-*`, `evidence/vm-stock-*`. Each has `result.json`, `run.log` and `vm/serial.log`. Local run logs are in `out/` (gitignored).
