# V-VALVE findings: real backpressure through the sk_skb egress redirect

Workspace: `/Users/marcus/conductor/workspaces/helios/wellington-v2`
Probe, harness and raw evidence: `spike-scratch/netns-density-295-valve/` (untracked, not committed). `crates/`, `specs/` and `docs/` were not touched.

## Verdict

- **TCP source: a working, lossless mechanism exists.** I call it the park valve. It needs no sockmap removal, and the queue bound is enforced in BPF rather than by the owner.
- **vsock source: no verdict-side or sockmap-resident mechanism is both lossless and bounded.** The only lossless way to withhold vsock credit is for the owner to read the socket itself (a userspace pump). That is the recommended mechanism.
- **The sockmap fast path for a vsock source can be kept only with a gated removal.** That variant has a timing-dependent loss window and needs a loss detector with abort-on-detect. Details are below.

Substrate: host AMD EPYC 8024P, `systemd-detect-virt` = `none`, KVM API 12. Host `uname -r` is `7.0.0-29-generic`; guest `uname -r` is `7.0.0-29-generic` (1 GiB, 4 vCPU, Cloud Hypervisor fork, `--vsock backend=vhost-kernel`). Every run passed the fail-closed preflight. Artifact hashes are in each `evidence/vm-*/artifact-pins.json`. Final BPF object `valve-bpf.o` sha256 `f3a5e587…`.

Deviation from the instructions: I ran through the V-3 launcher `launch.py`, not `cargo xtask metal run`. It uses the same lease protocol (`infra/metal/lease-holder.sh`) but does not rsync the tree, which would `--delete` the shared checkout on the host. One metal job at a time.

## Method

- Every stream is the splitmix64 sequence word k = splitmix(k). The reader compares each 8-byte word with an independently generated expected word and keeps an order-sensitive hash of both. Loss, reorder and duplication show up as `first_bad` or a byte shortfall.
- The "target app" is a reader thread on the far end of the loopback TCP pair. Schedules: `s3000,f` (stall 3 s, then fast), `r20` (steady 20 MiB/s), and stall/fast cycles.
- Directions: **A** = host sender to guest owner, **B** = guest sender to host owner, **T** = guest-local TCP source app to guest owner.
- BPF counters have one kernel-side writer each, because the bpf target has no atomic RMW. The verdict program cannot delete from a sockhash (see candidate 2).

Commands:
```
cd spike-scratch/netns-density-295-valve
./sync.sh                                   # build on the metal host in /tmp/valve-work
python3 launch.py <tag> "python3 /tmp/valve-work/vmrun.py <scenario>[,<scenario>...]"
```
Scenario names are rows of `SCENARIOS` in `vmrun.py`. Local logs are in `out/b*.txt` (gitignored). Raw JSONL per scenario is in `evidence/vm-*/`.

## Kernel facts (vendors/linux @ v7.0, `028ef9c96e96`)

1. **Credit is returned before the verdict runs.** `net/vmw_vsock/virtio_transport_common.c:1779` (`virtio_transport_send_credit_update`) precedes `:1781` (`return recv_actor(sk, skb)`) in `virtio_transport_read_skb` (`:1751`). No verdict return value can withhold vsock credit.
2. **Reads are driven by `sk_data_ready`.** `net/core/skmsg.c:1268-1287` (`sk_psock_verdict_data_ready`, `read_skb` at `:1284`) is the only reader. Only `sk_psock_stop_verdict` (`:1306`), reached from `sock_map_del_link` (`net/core/sock_map.c:142-175`, `:171`) or `sk_psock_drop` (`skmsg.c:888`), restores the original `sk_data_ready`.
3. **The redirect queue has no limit.** `skmsg.c:957-993` (`sk_psock_skb_redirect`) ends in an unconditional `skb_queue_tail(&psock_other->ingress_skb, skb)` at `:989`. The egress handler only returns -EAGAIN when the target is not writable (`:650`) and retries with `schedule_delayed_work(&psock->work, 1)` at `:727`. SK_PASS falls back to the same list at `:1067`.
4. **Removal drops the in-flight skb.** `sk_psock_verdict_recv` starts with `ret = __SK_DROP` (`skmsg.c:1240`).
   - NULL psock: `len = 0; sock_drop` (`:1245-1250`).
   - Programs cleared but psock alive: `prog` is NULL at `:1251-1254`, so the verdict stays DROP. `sk_psock_verdict_apply` DROP (`:1076-1090`) returns 0, hence `len = skb->len` and the caller sees a positive return. This path is invisible to any return-value detector.
5. **A sk_skb program cannot remove its own socket.** `kernel/bpf/verifier.c:10131-10167` (`may_update_sockmap`) does not list `BPF_PROG_TYPE_SK_SKB`. Delete is allowed only for sock_ops, iterators and a few other types. `:10254-10262` is the sockhash helper allow-list.
6. **vsock read gating by SO_RCVLOWAT.** `net/vmw_vsock/af_vsock.c:1062-1068` (`vsock_data_ready`) calls `sk_data_ready` only if `vsock_stream_has_data >= sk_rcvlowat`. The rx path calls it at `virtio_transport_common.c:1435`, under `lock_sock` (`:1661`). `setsockopt` takes the same lock (`net/core/sock.c:1310`). `vsock_set_rcvlowat` (`af_vsock.c:2585`) rejects `val > buffer_size` (`:2592`). Setting lowat does not wake a waiting reader.
7. **vsock `read_skb` reads one skb per wake.** Re-inserting the socket into the sockmap never wakes (`sock_map.c:299-320` has no kick). A residue in the vsock rx queue is only drained one skb per later arrival, and a tail can strand.
8. **TCP source.**
   - `tcp_read_skb` (`net/ipv4/tcp.c:1770`) unlinks the skb at `:1782`.
   - `tcp_eat_skb` (`net/ipv4/tcp_bpf.c:15-31`) advances `copied_seq`. `sk_psock_verdict_apply` calls it only on the redirect and drop paths (`skmsg.c:1077`, `:1084`), not on SK_PASS.
   - A PASSed skb therefore stays charged to the source's `sk_rmem_alloc`, so the TCP window closes natively.
   - `tcp_set_rcvlowat` (`tcp.c:1846`) calls `tcp_data_ready` at `:1859`, which is the TCP wake.
9. **strparser is TCP-only.** `sock_map.c:305-310` returns -EOPNOTSUPP for non-TCP. A parser returning 0 makes strparser consume and accumulate the data into `strp->skb_head` (`net/strparser/strparser.c:213-222`), not hold it in the receive queue.
10. **vsock rx overflow drops.** `virtio_transport_common.c:447` (`buf_used + len > buf_alloc`) drops the packet. `notify_buffer_size` (`:949-958`) sets `buf_alloc` equal to `buffer_size`. Shrinking the buffer is therefore lossy.

## Per-candidate results

N is trials. All trials are byte-verified unless stated. "Peak q" is the exact BPF peak of fwd minus sent.

### Controls: no valve (reproduces V-3, now with verified streams)

| Scenario | Result |
|---|---|
| ctl-A | 3/3 hit the 160 MiB abort cap in 0.27-0.33 s. Sender `max_send_ms` 0-2. SUnreclaim +175-192 MiB. |
| ctl-B | 3/3 hit the cap in 0.11-0.14 s at 1.2-1.6 GB/s. Host MemAvailable -116 to -164 MiB. |
| ctl-T | 3/3 hit the cap in 0.10-0.14 s. MemAvailable -88 to -122 MiB. |

```
ctl-A trial 0:  t=0.06s q=21.4 MiB  sunrec=45.3  skb=6584
                t=0.16s q=86.6 MiB  sunrec=122.9 skb=25288
                t=0.26s q=160.9 MiB sunrec=211.6 skb=46764   (sender never blocked)
```

### Candidate 1: remove the source from the sockmap above HIGH, re-insert below LOW

H: removal restores `sk_data_ready`, so vsock credit or the TCP window stops the sender, and the owner drains the residue in order.
P: the queue stops at about HIGH, the sender stalls, memory is flat, the stream is intact and resumes.
F: any lost, reordered or duplicated byte, or a deadlock.

Pump variant (owner receives from the source and sends to the target after queued == 0). HIGH = 4 MiB, LOW = 0.

| Scenario | Result |
|---|---|
| detect-A (N=20) | 8/20 intact. 12/20 lost exactly one skb (3732 B; one trial 2092 B). `first_bad` equals FWD, the point where the pump starts. |
| nogate-A (N=40) | 20/40 intact, 20 lost 3732 B. |
| nogatefast-A (N=40, sender at memcpy speed) | 20/40 intact. |
| detect-B (N=20) | 20/20 intact. |
| nogatefast-B (N=40) | 36/40 intact, 4 lost 65536 B. |
| detect-T (N=20) and nogate-T (N=40, slower sender) | all intact. |
| nogatefast-T (N=100) | 92/100 intact, 8 lost one skb (65483 B; one trial 53 B). |

Peak q in all of these is 4.00-4.8 MiB, with the sender stalled at about 6.75 MiB sent. The stall itself works, but removal loses an skb on all three paths, so it is unsafe.

Loss detector (exact): BPF counts verdict runs; fexit on `sk_psock_verdict_recv` counts reads. The number of reads that never ran the verdict is `vr_ok + vr_zero - verdicts`.
```
detect-A, 20 trials:  12 trials with lost_bytes=3732 have unrun=1; 8 trials with lost_bytes=0 have unrun=0.
                      Of the 12 losses, 10 hit the "programs cleared, psock alive" path (vr_zero=0) and 2 hit the NULL-psock path.
```
The kretprobe on `virtio_transport_read_skb` returning 0 caught only 2 of 12, because the "programs cleared" path returns a positive value. Do not use a return-value detector.

**Variant: gate with SO_RCVLOWAT before removal** (set lowat = buffer_size, then remove immediately; `--gate lowat`). The `setsockopt` takes `lock_sock`, which serialises against an in-flight rx read, and later arrivals are suppressed until the queue is exactly full.

| Scenario | Result |
|---|---|
| gate-A (N=40, 256 KiB buffer) | 40/40 intact |
| gate1m-A (N=40, 1 MiB buffer) | 40/40 |
| gatefast-A (N=100, fast sender) | 100/100 |
| gate-B (N=40) | 40/40 |
| gatefast-B (N=100) | 100/100 |

That is 320/320 intact. Peak q 4.0-4.8 MiB. Resume latency, measured as sender unblock minus app resume: A 6.3-7.7 ms, B 3.5-3.9 ms, in clock-synchronised time.

The residual hazard is timing:
- At exactly full, the gate opens and reads chain at sender rate (the leak). A removal that races a chained read loses an skb.
- Injected owner gap between lowat and removal (fast sender):
  - 1 ms, 256 KiB: 26/40 intact (14 lost).
  - 5 ms, 256 KiB: 37/40.
  - 1 ms, 1 MiB: 36/40, peak q up to 10.6 MiB.
- With a 10-30 us gap, none of 320 trials lost data. Fill time at memcpy-rate senders is roughly 85 us at 256 KiB, so the margin is small.
- lowat cannot be made unreachable. The set-time check is `val > buffer_size` (`af_vsock.c:2592`) and `buf_alloc == buffer_size`. A lossless shrink is impossible (fact 10).

Resume failure modes:
- **Re-insert without a wake** (`naive-A`, `naive-T`): 3/3 deadlock each. In A the source rx queue stays at `src_inq` 262144 and the sender blocks 25 s. In T the sender stalls with `src_inq` 0 and a closed window.
- **TCP re-insert plus SO_RCVLOWAT kick** (`kick-T`, N=20): 16/20 intact. 4 lost whole skbs (106, 65483, 130966, 159 B) over 234 close/reopen cycles.
- **Re-join after the pump** (`rejoingate-A`):
  - Left the gate raised: 20/20 deadlock (`src_inq` stuck at 258412, just under `buf_alloc`).
  - With lowat undone and a post-insert `SIOCINQ` check: 19/20 intact. One trial had the right byte count but a reordering at `first_bad` 7354960.
  - Cause: after a failed re-join the pump sends directly while redirected bytes are still in the target psock backlog. The pump must wait for queued == 0 on every resume.
  - `rejoingate-B` was 20/20.
- **Owner latency is not bounded** (`delay20-A`, `delay100-A`, ungated): peak q 11 MiB at +20 ms owner delay and 40 MiB at +100 ms. The overshoot is rate times owner latency, so this valve has no hard bound.

Verdict: **bounded and resumes, but lossy as shipped** (50% on A, 8-10% on T and B with fast senders). Gated removal brings observed loss to zero but keeps a timing hazard. The overshoot is owner-latency dependent.

### Candidate 2: the verdict itself stops consuming

- **Cannot delete its own source** (fact 5).
- **Cannot withhold vsock credit** (fact 1). The skb is already dequeued and credit already returned when the verdict returns.
- **SK_PASS once queued >= HIGH**:

| Scenario | Result |
|---|---|
| pass-A (vsock) | Unbounded. 3/3, SUnreclaim +373 MiB, 215,508 PASSed skbs, sender never throttled (`max_send_ms` <= 2). |
| pass-T (TCP) | Bounded. SUnreclaim +0.1-0.2 MiB, only 15 skbs parked before the TCP window closed. |

- **SK_DROP at a hard cap** (`hard-A`): the bound is exact (8.00 MiB) but 212,052 skbs were dropped and the flow is dead. It is a bound, not backpressure.

**Park valve for the TCP source** (`--mode 3 --resume park`, TCP only). The sequence:
1. Once queued + len >= HIGH, or while any parked bytes are unforwarded, the verdict returns SK_PASS. The skb stays charged to the source and the TCP window closes.
2. The owner waits for queued <= LOW, then receives the parked bytes and sends them to the target in order. HOLD is set during a receive-but-unsent window, and FORWARDED is published.
3. The verdict resumes redirecting by itself once PARKED == FORWARDED.

No sockmap removal and no re-insert are involved.

| Scenario | N | Result |
|---|---|---|
| park-T (`s3000,f`, 256 MiB) | 20 | 20/20 intact, peak q 3.96-4.00 MiB, SUnreclaim rise 0.0, 24,110 skbs parked, resume latency about 7.9 ms |
| parkstress-T (4 stall/fast cycles) | 100 | 100/100 intact (25.6 GiB verified), 185,030 skbs parked, peak q <= 4.00 MiB, SUnreclaim rise <= 0.5 MiB |
| parkslow-T (`r20`) | 10 | 10/10, reader exactly 20.0 MiB/s |
| parkslow2-T (`r2`) | 5 | 5/5 |
| parkcyc-T | 10 | 10/10 |
| parklong-T (10 s stall) | 3 | 3/3 |

```
parklong-T trial 0 (10 s stall, flat for the whole stall):
t=0.46s parked  fwd=6.4 sent=2.6 q=3.85 parked=0.43 src_inq=448044 avail=798.8 sunrec=20.8
t=5.04s parked  ... same ...                                      avail=800.0 sunrec=20.8
t=9.62s parked  ... same ...                                      avail=801.9 sunrec=20.8
t=10.08s open   fwd=38.1 sent=38.1 q=0.00 app_read=64.0 pumped=25.9
```

- The hard bound is deterministic because the verdict decides before adding to the queue.
- Parked memory is bounded by the TCP window: 0.43 MiB in the 10 s stall, and 15 skbs in `pass-T`.
- After the first episode the flow stays in park mode until the owner catches up with the sender (parked 65-250 MiB over a 256 MiB transfer). It is a userspace-copy pump through the psock ingress queue.
- **park for a vsock source** (`park-A`): unbounded, parked 160 MiB at the cap, SUnreclaim +231 MiB. Credit is already returned, so parking has nothing to throttle.

Verdict: **works for a TCP source; cannot work for a vsock source.**

### Candidate 3: native kernel knobs

| Knob | Result |
|---|---|
| vsock `buf_alloc` 64 KiB plus target SNDBUF 4 KiB (`knobs-A`) | Unbounded, 3/3 hit the 160 MiB cap. Credit comes back at read time. |
| TCP source SO_RCVBUF 4 KiB plus target SNDBUF 4 KiB (`knobs-T`) | Queue 0.06 MiB, but only 1.5 MiB of 256 MiB moved in 40 s, and it did not recover after the reader resumed. A throttle, not a valve. |
| `BPF_F_INGRESS` redirect (`ingress-T`, 128 MiB) | All 128 MiB held in kernel memory (MemAvailable -126.6 MiB). Ingress still goes through the unbounded `ingress_skb` (`skmsg.c:989`); the rcvbuf check at `:535` only delays moving it onward. |
| strparser | Not measured; source shows it accumulates and is TCP-only (facts 9). |
| sk_psock backlog limits | None exist in 7.0 (`grep` of `ingress_skb`: the only enqueue sites are `skmsg.c:989` and `:1067`). |

Verdict: nothing native bounds the egress queue.

### Candidate 4 (a kernel reason none can work): partly true

For a **vsock source**, no sockmap-resident mechanism is both lossless and bounded.
- Credit is returned in `read_skb` (fact 1), and verdicts, PASS, parking, buffer sizes and ingress redirect cannot undo that.
- The only way to withhold it is to stop `sk_data_ready` from running. Removal does that (fact 2) but races the in-flight read and drops an skb (fact 4).
- SO_RCVLOWAT can only narrow that window (fact 6).

For a **TCP source** it is false: the park valve works.

## Pure owner pump (no sockmap for the flow)

The owner receives from the source and sends to the target itself, in order.
- A source read happens only when the target is writable.
- vsock credit is withheld natively and TCP window natively.
- The owner's `recv` is a normal recvmsg (also usable while the socket is in a sockmap, `vsock_bpf.c:76-98`).

| Scenario | N | Result |
|---|---|---|
| purepump-A / B / T (`s3000,f`) | 10 each | 10/10 intact each. Sender `max_send_ms` 2735-3020, sent at stall about 2.7-2.8 MiB. SUnreclaim rise 0.0-0.1 MiB. Resume 1.7 ms (A), 1.9 ms (B), 2.4 ms (T). |
| purepumpslow-A / B / T (`r20`) | 10 each | 10/10 each. Reader 20.0 MiB/s, sender throttled to 20.4 MiB/s. |

## Throughput, CPU and latency (fast sender, verify off, N=10 x 1 GiB)

p50 MiB/s, all-CPU busy ms and owner-thread CPU ms per GiB:

| Path | Throughput | sys_busy | owner CPU |
|---|---|---|---|
| A sockmap | 427 | 3875 | 39 |
| A pump | 659 | 2360 | 761 |
| A sockmap + valve armed (gate) | 484 | 3385 | 45 |
| B sockmap | 1857 | 2310 | 8 |
| B pump | 1536 | 3125 | 459 |
| B valve armed (gate) | 1325 | 3335 | 346 |
| T sockmap | 1954 | 1100 | 9 |
| T pump | 1238 | 1570 | 826 |
| T park | 1196 | 1435 | 724 |

Small-message latency (16-byte records every 300 us, 2000 per trial, N=10; clock sync error about +/-25 us), p50 / p99 in us:
- A: sockmap 78 / 517, pump 96 / 1674.
- B: sockmap 78 / 90, pump 77 / 90.
- T: sockmap 41 / 77, pump 72 / 112.
- p99 tails of 20-30 ms in both A and T come from guest scheduling noise, not from either path.

## Recommendation

1. **TCP source: park valve** (`--mode 3`). BPF-side hard bound at HIGH, native TCP window backpressure, lossless, no removal. Owner forwards parked bytes in order after queued reaches LOW, and the verdict resumes by itself. 100/100 under four stall cycles.
2. **vsock source: a userspace pump.** Do not redirect a vsock source whose target can stall. This is the only lossless path with no timing assumption. In this guest it was about 35% faster on A and about 17% slower on B, with roughly 0.5-0.8 owner CPU-seconds per GiB.
3. **If the vsock sockmap fast path is required:**
   - Use the lowat gate, then immediately remove, then drain through the pump with a queued == 0 barrier at every pump start.
   - Demote the flow for its lifetime. Re-join is fragile (reordering and gate-left-raised deadlock above).
   - Keep the verdict and read counters and treat `vr_ok + vr_zero - verdicts > 0` as fatal for the flow.
   - Size the buffer large relative to owner latency. Expect 0 loss at 10-30 us gaps and about 35% loss at 1 ms.
4. **ADR-0183 "slow reader is backpressure"** is true only on the park path (TCP source) or the pump path. It is false for any sockmap egress redirect from a vsock source.
5. Nothing in this evidence supports `bpf_map_delete_elem` from the verdict, SK_PASS as a vsock stop, or a native knob as a hard bound.

## Not tested or limits

- Single flow per probe. Production needs per-flow counter cells and a per-flow target key. Fexit currently counts every `skb_send_sock` call.
- Half-close and FIN propagation through park and pump were not exercised.
- The verdict is an always-redirect program. There is no mTLS and no kTLS target, only a loopback TCP pair. The guest is small (4 vCPU, reader and sender share them).
- Host MemAvailable is noisy for direction B. SUnreclaim is not a good signal there because host skb data is in page frags. B uses the exact BPF queue counters.
- The `tp-*` numbers use `--fast-gen 1`, which skips pattern generation and verification. Correctness numbers above come from the verified runs.

Key files:
- `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-valve/bpf/src/main.rs`
- `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-valve/probe/src/main.rs`
- `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-valve/vmrun.py`
- `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-valve/launch.py`
- `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-valve/evidence/vm-*/` (raw JSONL, serial logs, `result.json`, `artifact-pins.json`)
