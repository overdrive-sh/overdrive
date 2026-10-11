# Drain probes T1–T6 — event-driven half-close drain across a SockHash TCP↔AF_VSOCK forwarder (netns-density-295, DG-5-D)

Probe increment: `spike-scratch/netns-density-295-v27-drain/` (raw evidence under `evidence/`, launch logs under `out/`, aggregate tables in `analysis.txt`). Research: `docs/research/networking/vsock-sockmap-half-close-and-drain-research.md`.

## Verdicts

| Probe | Verdict | Evidence |
|---|---|---|
| **T1** drain gap exists on 7.0 | **HOLDS** (not moot) | `sent < tcpi_bytes_received−1` at first `EPOLLRDHUP` in 8–97 % of TCP-source directions, depending on the pause. Even 2 kB flows with no pause show it (FIN-on-data 498/10,000, FIN-only 514/10,000). Half-closing at `EPOLLRDHUP` without a drain check passes only 345/10,000 (0 ms), 216/10,000 (300 ms) and 11/10,000 (3 s) 1 MiB flows; 2 kB flows lose 2.6–3.1 %. The target reports EPIPE (errno32: 9,732 at 3 s). |
| **T2** R1 event correctness | **HOLDS on the final build**; the research's single-counter form is **FALSIFIED** | 194,000 loopback + 38,000 microVM flows (776,000 + 152,000 half-close completions): 0 missed, duplicate or early records, 0 truncation, 0 stuck. One `fexit` counter: 3 of 96,000 flows hung forever; two independent counters: 0 of 96,000. |
| **T2 / R1-alt** kernel-armed goal | **FALSIFIED as specified**; works once widened | `ESTABLISHED→CLOSE_WAIT` alone completed 0 of 25,000 flows (512 stuck per role). Also needs `FIN_WAIT1→CLOSING` and `FIN_WAIT2→CLOSE`. |
| **T3** vsock `shutdown(SHUT_WR)` ordering | **HOLDS** | 0 early EOF in 30,000 loopback + 12,000 microVM trials (4,201 with credit exhausted, up to 19.6 MB accepted). |
| **T4** goal exactness | **HOLDS with 3 exceptions** | FIN-only and FIN-on-data exact; wire capture 300/300 of each shape. Exceptions under T4. |
| **T5** crash discriminator (tracepoint) | **HOLDS** | 0 misclassified: kill and close → `SHUTDOWN flags=0x3`, `SHUT_WR` → `0x2`. Record always precedes the `EPOLLRDHUP` wake (11–287 µs); poll bits `RDHUP` only in every cell. |
| **T6** failure path | TCP target reset **HOLDS**; target vsock leaving ESTABLISHED **FALSIFIED**, mitigated | Without a kernel signal 81 of 3,000 flows stick (vsock shows only `RDHUP`, redirects silently dropped). With A3, 0 stuck in 61,295 aborted role-flows. |

**Gate recommendation:** R1 (owner-set byte goal + BPF ringbuf completion record) — **PROMOTE with amendments A1–A4**. R1 exactly as specified (one `fexit` counter) is not safe. Optional sock_ops R1-alt — **DISCARD** as a default.

**Not run:** the 6.18 kernel (not used — Overdrive runs 7.0); the production tokio owner and the HalfCloseAck/control-loss path; an over-credit packet drop by a hostile guest; UDP, kTLS, IPv6, the `connect4` capture; the RST ambiguity in R1-alt's `FIN_WAIT2→CLOSE` trigger (reasoned only). The 500-per-cell loopback T5 sweep stopped at 1,552 trials (runner child `connect` began failing with ECONNRESET at a 15 ms pace in `shut_wr/unread`; not diagnosed); the 150-per-cell loopback and 110-per-cell microVM runs have no failures.

## Substrate

- Native x86_64 metal, AMD EPYC 8024P, `systemd-detect-virt` = `none`. Fail-closed probes (x86_64, vmx/svm, no hypervisor flag, `/dev/kvm` API 12) passed before every microVM run (`evidence/vm-drain-*/substrate-probes.json`).
- **Host `uname -r` = `7.0.0-29-generic`** (`linux-image-7.0.0-29-generic 7.0.0-29.29`; `-38` installed, not booted).
- **Guest `uname -r` = `7.0.0-29-generic`** (stock bzImage `/srv/vm/overdrive-testing/kernel`, sha256 `b51367c7…`).
- VMM: Cloud Hypervisor fork `41a619d19`, `--vsock cid=3360,backend=vhost-kernel`, 4 vCPU, 2 GiB, binary sha256 `eadddf1b…`.
- Probe build: `drain-bpf.o` sha256 `7dfb4344…` (aya-ebpf 0.1.1, `-C target-cpu=v3`, BPF atomics via `asm!`). `drainprobe` sha256 `35991bcb…` for the final T1/T2/T4/T6 runs; `4c07437b…` adds only `--drop-notice no` for the T6 negative control.
- Runs in private `/tmp/v27-work` on the host under the canonical lease (`launch.py` using `infra/metal/lease-holder.sh`). No `cargo xtask metal run`, no rsync `--delete`.

## What was built

`bpf/src/main.rs` (one ELF):
- An `SK_SKB` verdict redirects each source socket to its routed target through a `SockHash` (egress) and adds the bytes it forwarded (`fwd`) to the direction's cell.
- `fexit(skb_send_sock)` adds `ret` to the cell's final-hop `sent`. If `armed && sent >= goal` and a `cmpxchg(fired,0,1)` wins, it writes one record to a BPF ringbuf with `BPF_RB_FORCE_WAKEUP`.
- A cell is 64 bytes in a `BPF_F_MMAPABLE` array: `sent, goal, armed, fired, fwd, gen, sent_b, drops`. The owner mmaps it and uses real atomics on the same page.
- `sock_ops` installs intake children at `PASSIVE_ESTABLISHED` into a loopback parking pair P1→P2 and destination legs at `ACTIVE_ESTABLISHED`; optionally (R1-alt) STATE_CB arms `goal = bytes_received−1`.
- A tracepoint program records `vsock:virtio_transport_recv_pkt` `SHUTDOWN`/`RST` ops for T5.

`probe/src/fwd.rs`: one single-threaded epoll owner per role. `guest` role: TCP intake → parking cell → flow vsock. `host` role: flow vsock → TCP destination. The owner sets `goal`, then `armed`, then reads `sent`; whoever wins `cmpxchg(fired)` proceeds in the same dispatch, otherwise the owner waits for the record.

Substrate modes: **loopback** (both roles in one host process, flow vsock over `vsock_loopback`) and **VM** (guest role in a real CH microVM, host role and destination on the host, over vhost-kernel vsock). Owners are stand-ins, not the production tokio owner; apps are plain AF_INET with a byte-exact pattern check in both directions.

## T1 — the drain gap exists on 7.0

- **Hypothesis:** with the target's reader paused, EOF on the source precedes the target having accepted all bytes.
- **Predicted:** `sent < tcpi_bytes_received−1` after `EPOLLRDHUP`; a premature `shutdown(SHUT_WR)` truncates and the target reports EPIPE; with no pause a FIN-on-data final segment still shows a gap in some trials.
- **Falsified if:** `sent` always equals the goal at `EPOLLRDHUP` across 10k trials at 0 / 300 / 3000 ms.

Command: `python3 launch.py finalA2 'bash /tmp/v27-work/finalA.sh'` (runs `scen/t1.sh` then `scen/t4.sh`). Each cell: 10,000 trials, concurrency 512, 1 MiB each way, both app readers paused P ms. `--arm owner` = drain; `--arm none` = half-close at `EPOLLRDHUP`. TCP-source directions with `sent < goal` at the first `EPOLLRDHUP`:

```
t1-drain-p0     guest 999/10000 host 7489/10000  vsock-src guest 215 host 693   fwd<goal (verdict not yet run) 0/0  max deficit 1048576 B
t1-drain-p300   guest 856/10000 host 8608/10000  vsock-src guest 298 host 554   fwd<goal 0/0
t1-drain-p3000  guest 9576/10000 host 9670/10000 vsock-src guest 3818 host 7210  fwd<goal 0/0
t1-drain-small-findata  (2000 B, FIN on the last data segment)  guest 0/10000 host 498/10000
t1-drain-small-finonly  (2000 B, bare FIN)                      guest 0/10000 host 514/10000
WIRE t4-fin-data: FIN segments to intake port: with_payload=300 bare_FIN=0
WIRE t4-fin-only: FIN segments to intake port: with_payload=0 bare_FIN=300
```

With the drain gate, 10,000/10,000 pass at every pause with 0 violations. Naive half-close at `EPOLLRDHUP`:

```
t1-naive-p0     pass=345/10000  fail={'cli_truncated': 8888, 'srv_truncated': 754, ...}
t1-naive-p300   pass=216/10000  fail={'cli_truncated': 8977, 'srv_truncated': 783, ...}
t1-naive-p3000  pass=11/10000
t1-naive-small-findata  pass=9689/10000 fail={'cli_truncated': 311}   (no pause, 2000 B)
t1-naive-small-finonly  pass=9736/10000 fail={'cli_truncated': 264}
send errors at the tracked target, naive p3000: {'errno32/tracked_target': 9732, ...}
```

microVM (1,500 flows per cell): TCP-source `sent<goal` guest 1334/1300/1498, host 1496/1455/1500 of 1500 at 0/300/3000 ms. `vm-naive-p300` passes 0 of 1,500 (`srv_truncated` 1091).

**Verdict: HOLDS.** Edge: the research's F2 window (RCV_SHUTDOWN set before the verdict ran for the final skb) was never seen by an owner — `fwd<goal` at `EPOLLRDHUP` was 0 in every run. The drain wait is needed for F1 (the target backlog), not F2.

## T2 — R1 event correctness

- **Predicted:** exactly one record per armed direction, zero records before the receiver has every byte, zero truncation, zero stuck. **Falsified by:** any missed, duplicate or early record.

Matrix (`scen/t2.sh`, both arming modes): TCP→vsock and vsock→TCP in every flow (guest intake through the P1→P2 parking cell, host destination legs); readers paused 0/300/3000 ms at 2 MiB each way; mixed sizes 0 B–2 MiB; FIN-only and FIN-on-data; `--race-spin --pin` (owner arms into the final `skb_send_sock` calls, owners on separate CPUs); cell reuse (`--flows 32`, ~780 reuses per cell); server-speaks-first.

```
T2 loopback  trials 194000 pass 194000 completions 776000
             owner_cas 454317  record(fexit) 283082  record(state_cb) 38601  owner_lost_cas 26  VIOLATIONS 0  stuck 0
T2 microVM   trials  38000 pass  38000 completions 152000
             owner_cas  81863  record(fexit)  65174  record(state_cb)   4963  owner_lost_cas 15  VIOLATIONS 0  stuck 0
```

Completions equal 2 × role-flows and ringbuf records equal record-completions per run. Both arm/fire race outcomes occur (owner wins the CAS in ~59 % of completions; `owner_lost_cas` 41 times).

**Defect: the single-counter form is not safe.** Early runs left 1–2 flows per ~24,000 stuck forever although the target had every byte. Cause measured: `recursion_misses` lose `sent` increments permanently.

```
m2 (one fexit counter), 24 runs x 4000 flows x 2 MiB: flows=96000 fexit_calls=76,715,795 recursion_misses=11 stuck_flows=3
   m2-kernel-15 misses=2 stuck=1,1 | m2-owner-17 misses=5 stuck=1,1 | m2-owner-19 misses=4 stuck=1,1
   (every run with misses>0 had a stuck flow; every run with 0 misses had none)
m3 (two independent fexit programs, owner/kernel use max): flows=96000 calls=76,637,803 stuck_flows=0
   misses prog_a=3 prog_b=4; counters disagree in 5 flows (4 A-short, 1 B-short, never both)
```

A second program has its own recursion guard, so a call one skips the other counts. Rate ≈ 1.4e-7 per call, ≈ 1 in 32,000 two-MiB flows. ADR-0151 reads the same single counter, so the polling design carries this exposure too; its deadline hid it.

**R1-alt:** `ESTABLISHED→CLOSE_WAIT` fires for the guest intake leg but never for the host destination leg (the owner has already `shutdown(SHUT_WR)` there; transitions are `FIN_WAIT1→CLOSING` and `FIN_WAIT2→CLOSE`). First `--arm kernel` run completed 0 of 25,000. With all three transitions it passed the whole matrix (38,601 state_cb completions). `FIN_WAIT2→CLOSE` is also an RST, so the owner must ignore a record for an aborted flow. R1-alt saved one `TCP_INFO` read and gave identical results.

**Verdict: HOLDS** with dual counters and an epoch tag; single-counter variant FALSIFIED on "zero stuck".

## T3 — vsock shutdown ordering

Probe: sender non-blocking `send` until EAGAIN (credit exhausted) or a capped 0–4 MiB, then `shutdown(SHUT_WR)` (or `close()` in a third of trials); receiver stalls 20–100 ms and reads to EOF; a report connection gives the accepted total; pattern verified. **Falsified by:** EOF before the final accepted byte.

```
loopback            expected 30000 early_eof 0 excess 0 pattern_errors 0 credit_fill 3000 (all >=100 kB) close_variant 9993 max_accepted 4784128
microVM guest→host  expected 6000  early_eof 0 excess 0 pattern_errors 0 credit_fill 600  close_variant 2001 max_accepted 19595272
microVM host→guest  expected 6000  early_eof 0 excess 0 pattern_errors 0 credit_fill 601  close_variant 2001 max_accepted 4194312
```

**Verdict: HOLDS.**

## T4 — goal exactness

Predicted: `tcpi_bytes_received−1` equals the verdict's forwarded total for FIN-only and FIN-on-data; for a vsock source the peer's final-hop `sent` equals the local `fwd`; includes a vsock with a backlog at install and a sub-`rcvlowat` tail. Common case exact: `t4-findata-big` and `t4-finonly-big` (20,000 flows each) 0 mismatches; all T1/T2/T6 flows check it too.

**Exception (a): TCP receive-queue overlap duplicates.** ~5e-5 of flows at a default intake window end with `fwd = goal + 47,616`; the destination receives more bytes than the client sent, corrupt (`srv_excess_bytes`). In-kernel check (`TCP_SKB_CB(skb)->seq` from `skb->cb[0]`) shows an skb whose head overlaps bytes already handed to a verdict:

```
ovl41 {"flow":41,"goal":365823,"fwd":413439,"sent":413439,"tcpi_bytes_received":365824,"overlap_bytes_seen_by_verdict":47616}
ovl52 {"flow":8043,"goal":367842,"fwd":415458,"overlap_bytes_seen_by_verdict":47616}
ovl52 {"flow":8823,"goal":415717,"fwd":463333,"overlap_bytes_seen_by_verdict":47616}
default intake window: 8 overlap flows in 120,000 --diag-seq trials (each: fwd-goal == overlap == 47616); 10 in 200,000 post-fix trials
intake SO_RCVBUF = 4 MiB: 0 overlap and 0 mismatches in 180,000 flows (t4-bigrcvbuf-1..12 plus nsb21-26)
```

Only on the guest intake child when the client wrote more than the initial window before `accept()` (accept-time `bytes_received` 98,360 or 113,208). Zero-window probes (`TcpExtTCPWinProbe`, ~1,500 per 10k flows) present at the default window, absent with the large buffer. Consistent with a retransmission overlapping `rcv_nxt` whose head `tcp_read_skb` forwards without applying the skb offset — **interpretation, kernel source not read; to be verified against v7.0.** Stream corruption is independent of drain; drain effect is `sent > goal`, so a mid-stream duplicate could fire completion before the real tail (not captured). Measured fix: large `SO_RCVBUF` on the intake listener.

**Exception (b): a vsock leg installed with data already queued strands.** `--v-late` installs the flow vsock after `Paired` with a 100,000 B banner queued (`virtio_transport_read_skb` takes one skb per `data_ready`, nothing re-arms):

```
loopback  v-late 0 (install at connect, the design): 400/400   v-late 1 (no re-arm): 368/400 (32 stuck)   v-late 2 (SO_RCVLOWAT re-arm): 368/400 (32 stuck)
microVM   v-late 0: 400/400   v-late 1: 139/400 (64 stuck)   v-late 2: 369/400 (31 stuck)
```

Installing the vsock at `connect()` (the existing design) gives 0 stuck.

**Exception (c): `SO_RCVLOWAT` > 1 on a flow vsock.** With 65536 on both flow vsocks, 0 of 600 loopback and 0 of 300 microVM flows complete (`vsock_eof_fwd_lt_goal` 300/300).

**Verdict: HOLDS** for FIN-only, FIN-on-data and vsock-source exactness, with exceptions (a)–(c).

## T5 — crash discriminator

Predicted: `op=SHUTDOWN flags=0x3` for SIGKILL and `close()`, `flags=0x2` for `SHUT_WR`, read payload-free from BPF. Falsified by a kill classified as a half-close, or a record after the owner acted on `EPOLLRDHUP`. Probe: a peer child holds the flow vsock; it is SIGKILLed, `SHUT_WR`'d or closed, idle or with 4 KiB unread; observer drains the ringbuf the instant `EPOLLRDHUP` wakes it and 60 ms later; observers on host and guest.

```
microVM host observes guest peers (110/cell, 660 trials, 0 timeouts):
  at wake: kill/idle 0x3 110 | kill/unread 0x3 110 | close/idle 0x3 110 | close/unread 0x3 110 | shut_wr/idle 0x2 110 | shut_wr/unread 0x2 110
  no record at wake {} ; record only after wake {} ; poll bits at wake: RDHUP only in every cell ; record→wake lag 11.7–287 µs (p50 52 µs)
microVM guest observes host peers: identical table, 660 trials, lag 24.6–198 µs
loopback 900 trials (150/cell): identical table, lag 13.6–74.6 µs
```

**Verdict: HOLDS.** Not integrated into the drain; a candidate for ADR-0180's crash discrimination.

## T6 — failure path

Predicted: a target reset, or a target vsock leaving ESTABLISHED, mid-drain gives `EPOLLERR`/`EPOLLHUP` on the target, the owner aborts, no ringbuf record. Probes: `--srv-rst-permille 500`; `--chaos-v-reset-permille 300` (guest owner `shutdown(SHUT_RDWR)` on the flow vsock 1–20 ms after `Paired`). Both arming modes, 5 seeds, loopback plus microVM.

```
T6 loopback, 10 runs: 100,000 trials, 61,295 aborted role-flows, live_at_end (0,0) in all, VIOLATIONS 0, all 69,225 non-reset trials passed
   abort causes: host epollerr 21,938 | guest epollerr 9,120 | unannounced_vsock_end 18,355+6,809 | target_hup_mid_drain 3,271 | redirect_dropped 1,802
   completion records for an aborted direction: 0 ; late redirect-refused notices: 54
T6 microVM, 4 runs: live_at_end (0,0), VIOLATIONS 0
t6-clean-control (20,000 flows, no fault): 0 aborts
```

**TCP target reset: HOLDS.** **Target vsock leaving ESTABLISHED: prediction FALSIFIED.** Without the mitigation (`--drop-notice no`, 3,000 trials):

```
trials 3000 pass 2465 live_at_end guest/host 0 81
80 x host flow stuck: poll_o=IN|OUT|HUP|RDHUP poll_v=IN|OUT|RDHUP so_error_o=0 so_error_v=0 done=[False,True] fwd==goal(dir0)=True sent(dir0)=0
verdict_redirect_fail (SK_DROP at the helper): 1094
```

The target vsock shows `RDHUP` only (peer SHUTDOWN(RCV|SEND) then do_close sets `peer_shutdown` and CLOSING but not `sk_shutdown`, so no `EPOLLHUP`; no `EPOLLERR` because nothing was sent). Later redirects are refused by the helper and bytes eaten from the source (`fwd == goal`, `sent == 0`).

**Mitigation A3 validated:** on `redirect_skb` returning `SK_DROP` the verdict bumps `drops` and writes one ringbuf record (kind 3) on the first refusal; owner aborts → stuck 81 → 0. **A4 also needed:** source-side `EPOLLRDHUP|EPOLLHUP` on the flow vsock with no `HalfClose` aborts (`abort_unannounced_vsock_end` 25,164). A redirect-refused notice can arrive after abort and cell recycle (54×); the epoch makes it discardable.

## Other facts established

1. Never key events on fd numbers — a stale epoll event for a closed-and-reused fd caused a spurious early EOF and premature completion; tokens must be flow ids.
2. Intake children lose bytes arriving before `accept()` unless re-armed with `SO_RCVLOWAT` afterwards (14 of 40 stalled without); parking cell P2 also needs a re-arm at `Paired` (615/10,000 passed without).
3. `skb_send_sock` returns -EAGAIN often under backpressure (backlog retries) and -EPIPE at a half-closed target. Zero-length FIN-only skbs are still dropped by the verdict.
4. `recursion_misses` is readable from the program fd's `/proc/self/fdinfo` — usable as a runtime health counter.

## Design implications

- **DG-5-D (event instead of poll) is supportable.** Replace `FLOW_DRAIN_RECHECK_PERIOD` and `FLOW_DRAIN_DEADLINE` with: owner stores `goal`, then `armed`, then reads `sent`; `cmpxchg(fired)` decides who reports; otherwise wait for the kernel record. Measured 0 lost wake-up, 0 double fire, 0 early fire over ~232,000 flows.
- **A1:** two independent `fexit(skb_send_sock)` programs; owner and kernel use the larger `sent`. Residual both-skip case not seen in 96,000 flows — needs a validation item.
- **A2:** every record carries the cell epoch; owner discards records whose epoch is not the flow's.
- **A3:** the kernel tells the owner when it refuses a redirect (poll cannot).
- **A4:** `RDHUP|HUP` with no `HalfClose` aborts at once (matches the delta's "Unannounced vsock end").
- Owner rules: install the flow vsock at `connect()`; never `SO_RCVLOWAT` > 1 on a flow vsock; re-arm intake children after `accept()` and the parking cell at `Paired`; abort on `EPOLLERR` anywhere and on `EPOLLHUP` at the target of a draining direction.
- Intake overlap hazard (T4 a) is independent of drain but changes what the goal can equal; large `SO_RCVBUF` on the intake listener removed it in 180,000 flows. Contract-shaping for ADR-0158's intake, outside DG-5-D.
- R1-alt: three trigger transitions (one RST-ambiguous), a per-socket cell map, a sock_ops attach on every TCP leg, no measurable saving — not adopted.
- No drain deadline needed for correctness once A1–A4 are in (0 stuck over ~232,000 drain flows and 61,295 aborts). A resource bound for a guest withholding credit is its own policy; the vanilla-7.0 over-credit silent drop was not exercised.

## Limits and cleanup

Control session was an in-process queue (loopback) or a vsock stream (VM), no loss injected; host capacity sized at 1024 cells so pairing never failed. Loopback latencies not meaningful in absolute terms. Cleanup verified under the lease: no `v27-*` cgroups, no probe BPF programs, no leftover processes or VMs, `/tmp/v27-work` removed; `vsock_loopback`/`vhost_vsock` were already loaded and left loaded; pre-existing `overdrive-mtls*` nft tables, `ovd-*` links and `/sys/fs/bpf/overdrive` are not from this probe.
