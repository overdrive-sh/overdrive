# V-3 findings: does a vsock-source sockmap redirect into a non-reading TCP target grow kernel memory without bound?

## Verdict

**V-3 FALSIFIED on the "memory flat near `buf_alloc`" side. The queue is unbounded and vsock credit does not stop the sender.**

- With a vsock source, an `sk_skb` stream verdict that redirects (egress) into a TCP socket whose peer never reads grew linearly with the bytes the peer sent. It grew at roughly 480 MiB/s (host to guest), with no plateau and no sender throttling.
- The guest OOM-killed its own processes at about 0.4 GiB queued, on a 1 GiB guest.
- The research doc's inference that a TCP source is bounded is also **FALSIFIED** (case C3). It had been marked "INFERENCE; not measured".
- A guest-sends / host-queues run shows the same shape on the host kernel (C4).
- The queue is not charged to the owner's memory cgroup. A cgroup `memory.max` cannot bound it.

Evidence directory: `/Users/marcus/conductor/workspaces/helios/wellington-v2/spike-scratch/netns-density-295-v3/`. It holds `evidence/vm-*/` (raw serial logs, `run.log`, `result.json`, artifact pins) and the sources in `bpf/`, `probe/`, `vmrun.py` and `launch.py`.

## Substrate and versions

| Item | Value |
|---|---|
| Host | AMD EPYC 8024P, `systemd-detect-virt` = `none`, KVM API 12 (fail-closed preflight passed every run) |
| Host `uname -r` | `7.0.0-29-generic` |
| Guest `uname -r` | `7.0.0-29-generic` (stock `/srv/vm/overdrive-testing/kernel`, sha256 `b51367c7…`) |
| VMM | Cloud Hypervisor fork `41a619d19`, binary sha256 `eadddf1b…`, `--vsock cid=3370,backend=vhost-kernel`, 4 vCPU, 1024M |
| Probe | `v3-bpf.o` sha256 `746e211f…`: aya-ebpf 0.1.1, `stream_verdict` plus `fexit(skb_send_sock)`. `v3probe` sha256 `3da1395a…`; later runs `6b109ad1…`, `2e9a30e3…` and `cacb8850…` add only the `--cgroup`, `--valve-*` and `--valve-kick` flags |
| Kernel source | `vendors/linux`, tag v7.0 (`028ef9c96e96`). The runtime kernel is Ubuntu 7.0.0-29, so backports are not enumerated. |

**Harness:** this reuses the v27 pattern, not ka5's `launch.py` directly. Reason: ka5's guest has no BPF loader, and v27 already runs aya inside the guest. It runs under the canonical lease (`infra/metal/lease-holder.sh`), with private work dir `/tmp/v3-work`, and no rsync of the shared tree. Nothing under `crates/`, `specs/` or `docs/` was touched. The `crates/` changes in `git status` were already there.

## Method (hypothesis / prediction / falsification, written first)

- **H:** the egress redirect has no byte bound, and vsock credit is returned at `read_skb` before the target accepts anything. So `queued = fwd − sent` grows with what the peer sends.
- **Predicted:** a sender is never throttled, and kernel memory rises about 1.2× the queued bytes. Memory returns on close.
- **Falsified if:** growth stops near 256 KiB, or the sender stalls at the credit window.

**Probe design:**
- The sink accepts a vsock flow, puts it in sockmap `VERD` with the verdict attached, and puts one end of a loopback TCP pair in `SOCKS` key 1.
- The other end of the TCP pair is never read.
- The verdict always redirects.
- Counters are `fwd` (bytes entering the verdict) and `sent` (the return of `skb_send_sock`).
- Samples every 50–250 ms: `/proc/meminfo`, `/proc/slabinfo`, `/proc/net/sockstat`, `SIOCINQ`/`SIOCOUTQ`, memory-cgroup stats.
- An independent 1 s heartbeat from init prints `MemAvailable` and `SUnreclaim` to the serial console.

**Bounds:** the guest is a 1 GiB VM (the OOM blast radius). The host-side run (C4) is capped at 256 MiB queued, 384 MiB sent. The host has 62 GB, and memory returned fully after close.

## Per-case results

| Case | Setup | Result |
|---|---|---|
| **C0 control** (no sockmap) | Host vsock sender, guest app never reads | Sender stops at **exactly 262,144 B** (= `VSOCK_DEFAULT_BUFFER_SIZE`). Blocked until the 5 s `SO_SNDTIMEO` (`outcome stall_5s`, `max_send_ms 5398`). The vsock rx queue holds 262,144 B. Credit works when nothing is in a sockmap. |
| **C1 trials, N=100** | 16 MiB per trial, host to guest, target peer never reads, 100 sequential flows | See the next table. |
| **C1long ×3, uncapped** | Host sends up to 4 GiB; the third run's guest sink in a 64 MiB memory cgroup, the second also (see C1long-cg below) | Queue grew linearly to **338–398 MiB** (last good samples at 0.6–0.8 s). Sender pushed 406 / 422 / 415 MB (`send_end err_32` = EPIPE when the sink was OOM-killed). `max_send_ms` ≤ 5, `slow_sends` 0. **No credit throttle.** Guest OOM killed the sink (pid 104/115) and then the agent. The VM survived. |
| **C1long-cg** | Same, sink inside a 64 MiB memory cgroup | While `SUnreclaim` went 19→467 MiB and the queue reached 383 MiB, the cgroup stayed flat: `memory.current` **6.1 MiB** (`sock` 2.6 MiB, the loopback TCP leg only), `oom_kill 0`. The queue is **not charged to the owner's cgroup.** |
| **C3 TCP source** | Guest-local TCP app pushes into a verdict-redirected TCP source; target peer never reads | Uncapped: sink OOM-killed in under 0.5 s (two runs). Capped at 100 MiB: stopped at `queued_end` **129 MiB**, `app_sent` 132,382,720, `app_state 0` (**never stalled**), ~3.4 GB/s. |
| **C4 host direction** | Guest sends 384 MiB to a host sink with a non-reading TCP target | Guest sent all 402,653,184 B in 0.18 s (2.1 GB/s), `max_send_ms 0`. Host queued **381–400 MiB** (the 256 MiB cap overshot at the 250 ms sample interval). Host `MemAvailable` dropped 390 MiB and fully recovered after close. |
| **C5 valve** | Remove the source from `VERD` at 8 MiB queued | Queue stopped at 8.37 MiB. `src_inq` jumped to 262,144. Sender sent exactly `fwd + 262,144` = 11,691,532 B, then blocked (`max_send_ms 10005`). **Credit then works.** Re-insert alone did **not** resume forwarding (7 s idle, `insert_ok true`). Re-insert plus `recv()` of the 262,144 B residue resumed at once: `fwd` 11.7 MiB to 73 MiB in about 120 ms, sender blocked exactly 3000 ms. |

**C1 trial summary (N=100):**

| Measure | All 100 trials |
|---|---|
| `fwd` | 16,777,216 B in 100/100 |
| `sent` (reached target) | 2,693,256 B in 100/100 (target `rcvbuf` plus `sndbuf`) |
| `queued_end` | 14,083,960 B (13.4 MiB) in 100/100 |
| Sender outcome | `done` 100/100; `max_send_ms` 1; 0 slow sends; median 481 MB/s |
| Verdict drops | 0 |
| `SUnreclaim` rise / queued | 1.11–1.21, median 1.155 |
| `SUnreclaim` after close − start | median 0.0 MiB, range −0.55 to +0.81 MiB, no drift over 100 trials |

The memory rise is `kmalloc-rnd-*-4k` (about 4 KiB per queued vsock rx skb), plus `skbuff_head_cache` (3.5 KiB average payload per skb). In C1long, `skbuff_head_active` reached 100,776 and `SUnreclaim` 435–467 MiB.

**Independent heartbeat (C1long, third run):** guest uptime 1.41 s, `SUnreclaim` 19.5 MB; 2.43 s, 176 MB; OOM kills at 2.95 s; **3.43 s, 20.2 MB**. Memory is released when the owner's sockets close.

A tool problem in run 1: an OOM message got interleaved into a serial JSON line, so that line failed to parse. Its last sample (queued 398.5 MiB at 765 ms) is in the raw serial log only.

## Citations (vendors/linux @ v7.0)

- `net/vmw_vsock/virtio_transport_common.c:1776` `virtio_transport_dec_rx_pkt`, `:1779` `virtio_transport_send_credit_update`, `:1781` `return recv_actor(sk, skb)`. Credit is returned before the verdict runs, so credit means the verdict took the bytes.
- `net/core/skmsg.c:989` `skb_queue_tail(&psock_other->ingress_skb, skb)` has no limit check. `:650-652` the egress handler returns `-EAGAIN` unless `sock_writeable`. `:727` retries with `schedule_delayed_work(&psock->work, 1)`. This matches `send_fail` rising by about 100–150/s per stuck flow and `last_errno` 11 (EAGAIN).
- Credit window: `virtio_transport_common.c:825-832` (`min(peer_buf_alloc, buf_alloc)`) and `:889-903` (`has_space`). `net/vmw_vsock/af_vsock.c:199-201` sets the default to 256 KiB. C0 and C5 confirm this to the byte.
- **TCP source (new):** `net/ipv4/tcp.c:1782-1785` `tcp_read_skb` unlinks the skb from `sk_receive_queue`, then `skb_set_owner_sk_safe`. `include/net/sock.h:2456-2465` that call runs `skb_orphan()`, which invokes the old `sock_rfree` destructor and uncharges the source's `sk_rmem_alloc`. The `tcp_bpf.c:15-31` `tcp_eat_skb` advance of `copied_seq` then reopens the window. So the source's `rcvbuf` never fills, and the research doc's claim that "the skb stays charged to the source's `sk_rmem_alloc`" is wrong. C3 measured this.

## Design implications

1. **The slot pool bounds the number of draining flows, not their bytes.** The ADR-0183 statement "a slow reader is backpressure, not a fault" is false for both TCP-source and vsock-source directions through an egress sockmap redirect. The research doc's F3 (finding) applies to both source types, and the "TCP-source directions are fine" carve-out should be removed.
2. **Guest-held memory is attackable by the host and by any peer.** A host sender can OOM the guest in about 1 s at about 0.4 GiB of queue. A guest sender can inflate host kernel memory the same way (C4). Per `.claude/rules/design.md` ("Design for Overdrive's own failures"), the probe runner and any `GuestStream` whose app does not read are reachable inside Overdrive's own composition.
3. **Memory-cgroup limits do not help.** The vsock rx skb is allocated outside the owner's cgroup (C1long-cg). Any "bound it with `memory.max`" design is unsound.
4. **The OOM killer targets the owner.** It killed the sink and the guest agent, leaving the guest running without its control plane. This is a degraded failure, not a clean one.
5. **Candidate F2 (byte cap with abort) is the cheap, working bound,** using `fwd − sent` from the existing BPF counters. The counters are accurate; the `fwd` field already exists in the v27 cells. A pause/resume valve (C5) is also viable and restores real vsock credit backpressure. Two things must be designed:
   - Removing the source from the sockmap leaves up to `buf_alloc` (262,144 B) in the vsock rx queue.
   - Re-insert does not wake it. The owner must drain the residue itself, and ordering against the bytes still queued at the target has to be preserved. My `recv()` and discard probe loses those bytes and is not a design.
6. **Re-insert alone deadlocks.** The sender is blocked on credit, so no new packet arrives to trigger `data_ready`. This is a hazard for any resume design.

## Gate recommendation

**PROMOTE V-3 to a contract-shaping finding. Do not carry it as Open into DISTILL.**

- Withdraw "credit cannot make the host's quota the source of truth" as the only concern. The sharper statement is that an unbounded queue exists in both directions and both source types.
- Treat a per-flow byte bound as required (D-4). F2 is cheap, and a valve (pause plus owner drain of the 256 KiB residue) is the correct alternative.
- Re-run the `flow_quota` and `guest-flow-owner` Quint checks with a queue-bound invariant once D-4 is decided.

## Not run, and caveats

- **Production verdict and netns:** the verdict is the minimal always-redirect program, and the target is a loopback TCP pair, not the production verdict program or veth/netns path. Ingress redirect (`BPF_F_INGRESS`, bounded by `sk_rmem_alloc` at `skmsg.c:535`) was not tested.
- **Larger guests:** the guest is 1 GiB, so the absolute OOM point (about 0.4 GiB queued) scales with guest RAM.
- **Peak queue not captured in C1long:** the OOM hits between the last 50 ms sample and the kill, so the exact peak is lower-bounded by 398.5 MiB.
- **Host-side hard cap:** the C4 cap overshot because of the 250 ms check interval, but host memory fully recovered.
- **Not measured:** the production tokio owner, and a hostile guest using SO_VM_SOCKETS_BUFFER_SIZE to raise `buf_alloc`.
