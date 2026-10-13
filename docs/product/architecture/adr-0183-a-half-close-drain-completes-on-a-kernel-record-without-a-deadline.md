# ADR-0183 — A half-close drain completes on a kernel ring record, with no deadline

## Status

**Accepted — decision R2-DRAIN approved by user 2026-10-11.** GH #295.
Recorded in the #295 feature delta, § *[REF] vsock Attachment Replacement
DESIGN* (§ *[REF] Driven port — host forwarder
`overdrive-dataplane::guest_flow`* for the drain cells, programs and ring;
§ *[REF] Owner and provisioner — `overdrive-control-plane::guest_network`*
and § *[REF] Guest adaptation contract* for the owner rules). How an end is
classified as a clean half-close is ADR-0180; stop and abort semantics are
ADR-0160; the per-allocation flow quota is ADR-0147.

## Context

A flow's bytes move between its sockets inside the kernel (SK_SKB verdict and
redirect, ADR-0147). When an owner sees one direction end cleanly — a FIN on
its TCP socket, or a `0x2` SHUTDOWN classified on its flow vsock (ADR-0180) —
it must shut down the write side of that direction's target socket. Doing so
before the target has accepted every forwarded byte truncates the stream.

Drain probes T1–T6 (`spike/findings-drain-t1-t6.md`, 7.0.0-29 host and guest,
real microVM and loopback) established:

- **T1 — the gap exists.** At the first `EPOLLRDHUP` the target had not yet
  sent every byte in 8–97 % of TCP-source directions, even for 2 kB flows
  with no pause. Half-closing at `EPOLLRDHUP` passed 345, 216 and 11 of 10,000
  1 MiB flows at 0, 300 and 3,000 ms reader pauses.
- **T2 — an event-driven completion is correct, but not with one counter.** An
  owner-armed byte goal plus a kernel ring record completed 776,000 loopback
  and 152,000 microVM half-closes with no missed, duplicate or early record.
  With one `fexit(skb_send_sock)` counter, 3 of 96,000 flows hung forever:
  a fexit program call skipped for recursion
  (`kernel/bpf/trampoline.c:1150-1154 @ v7.0`) loses its increment for good.
  Two independent programs, each with its own recursion guard: 0 of 96,000.
- **T3:** vsock `shutdown(SHUT_WR)` never overtakes accepted bytes (0 early
  EOF in 42,000 trials, credit exhausted in 4,201).
- **T4 — goal exactness**, with three exceptions: (a) at a default receive
  window about 5e-5 of guest intake flows forward a receive-queue overlap
  twice (`fwd = goal + 47,616`), absent with a 4 MiB `SO_RCVBUF` on the
  intake listener (0 in 180,000 flows); (b) a flow vsock installed after
  `Paired` with data already queued strands (32 of 400), installed at
  `connect()` it does not (400 of 400); (c) `SO_RCVLOWAT` above 1 on a flow
  vsock stops completion (0 of 900).
- **T6 — a refused redirect is invisible to poll.** When a vsock target leaves
  `ESTABLISHED`, `bpf_sk_redirect_hash` returns `SK_DROP`
  (`net/core/sock_map.c:528-534`, `:1259-1261 @ v7.0`) and the bytes are
  consumed from the source; the target shows only `EPOLLRDHUP`, because the
  peer's close sets `peer_shutdown` but not `sk_shutdown`
  (`net/vmw_vsock/virtio_transport_common.c:1443-1452`, `do_close` at
  `:1244-1252`,
  `net/vmw_vsock/af_vsock.c:1247-1259 @ v7.0`). 81 of 3,000 flows stuck
  without a kernel notice, 0 with it.

V-31 (`spike/findings-v31-r2-halfclose.md`) settled the counter form under
forced recursion skips: per-call counters completed 0 of 3,000 at a 30 % skip
rate, a cumulative read alone 1,936 of 3,000, a cumulative read plus a level
backstop 3,000 of 3,000 (also at 70 % and 90 %, and 6 of 6 exact 5 GiB flows
across 32-bit wrap); at real volume, 77 million fexit calls over 96,000 flows
left none stuck. A cumulative read repairs a skipped middle call; only a
skipped last call can strand a goal, and the backstop re-reads it.

Kernel facts the counters rest on: the psock backlog sends through
`skb_send_sock` (`net/core/skmsg.c:650-652`, `net/core/skbuff.c:3422
@ v7.0`); the target's cumulative send position advances on the accept path —
TCP `write_seq` (`net/ipv4/tcp.c:1390 @ v7.0`), vsock `tx_cnt`
(`net/vmw_vsock/virtio_transport_common.c:504 @ v7.0`) — and is valid when
`skb_send_sock` returns; a program's recursion misses are counted for every
program (`kernel/bpf/core.c:160`, `kernel/bpf/syscall.c:2445-2457 @ v7.0`)
and readable from the program descriptor's fdinfo (`kernel/bpf/syscall.c:2503
@ v7.0`).

## Decision

- **One exact cell per (flow, direction).** Drain state lives in a
  non-evicting `BPF_F_MMAPABLE` array indexed by flow slot, holding per
  direction the target's `sent`, the owner's `goal`, `armed`, `fired`, the
  verdict's forwarded count `fwd`, the cell `epoch`, and the drop and lost
  counters. The owner maps the array and uses atomics on the same page.
- **The goal.**
  - TCP source: `tcpi_bytes_received − 1` (the FIN counts one).
  - vsock source: the owner's own forwarded count `fwd` for that direction at
    the classified `0x2` end. T4 measured it equal to the peer's final-hop
    sent count; a byte this side's kernel never received is never waited for.
- **Completion.** On a direction's clean end the owner stores `goal`, then
  sets `armed` (sequentially consistent), then reads `sent`. Whoever wins
  `cmpxchg(fired, 0, 1)` — the owner, or a kernel program seeing
  `armed && sent ≥ goal` — completes it; a kernel win writes one record to the
  flow-event ring with `BPF_RB_FORCE_WAKEUP` (`kernel/bpf/ringbuf.c:581-582
  @ v7.0`), which the owner waits for in the same event loop as the flow's
  other inputs. On completion the owner calls `shutdown(SHUT_WR)` on the
  target.
- **A1 — two cumulative counters and a level backstop.** Two independent
  `fexit(skb_send_sock)` programs each read the target socket's cumulative
  send position and store the maximum into the cell, so a call one program
  misses is covered by the other's next run. An `fentry(sock_poll)` program
  performs the same read; the owner triggers it with a zero-event
  `poll(fd, 0, 0)` on the target of every armed, incomplete direction when
  either fexit program's `recursion_misses` has risen since the owner's last
  read, and once at arm time when its own read of `sent` is short of the
  goal. A poll with no requested events still calls the file's poll method
  (`fs/select.c:849-870`, `include/linux/poll.h:78-83`, `net/socket.c:160`
  `@ v7.0`). Two programs are kept because every measurement of the backstop
  ran with both (V-31: the 3,000-of-3,000 results at 30, 70 and 90 % forced
  skips injected skips into both programs); one cumulative program with the
  backstop was not measured.
- **A2 — epochs.** Every record carries the cell epoch; a record of another
  epoch is discarded (the slot was reused).
- **A3 — refused redirects are reported.** When a redirect returns `SK_DROP`,
  the verdict increments the cell's drop counter and, on the first refusal,
  writes one "redirect refused" record; the owner aborts the flow.
- **A4, A5, A6** — the unannounced-end abort, the raw-tracepoint classifier
  and the TCP-source end check — are ADR-0180.
- **A7 — a lost flow record aborts.** Every ring producer counts its failed
  `bpf_ringbuf_output` calls globally and per cell, split by record class: a
  lost flow-end, drain or redirect-refused record increments the cell's
  `ring_lost`; a lost empty-datagram notice increments its `empty_lost`
  (ADR-0165). A failed write increments exactly one of them. When a cell's
  `ring_lost` rises the owner aborts that flow, since a drain or
  classification record may have been lost; a rise of `empty_lost` alone
  never aborts (ADR-0165 compensates it).
- **When the counters are read.** The kernel raises no event for a rise of
  `recursion_misses` (readable only from the program descriptor's fdinfo), a
  failed ring write or a cell's lost counts. Each owner, host and guest,
  reads them at every flow-event dispatch and on the fixed one-second audit
  period of ADR-0124. This is a counter read, not a drain deadline: a
  direction completes only on its record or on the backstop read. A
  both-skip of the last call, or a lost record, is therefore detected within
  one audit period.
- **No deadline.** A drain waits until it completes or the flow is aborted; a
  slow reader is backpressure, not a fault. The per-allocation flow quota
  (ADR-0147, 4,096) counts draining (half-closed, not yet completed) flows. A
  guest that withholds credit therefore holds at most its quota of node flow
  capacity — what an untrusted guest can already hold with open flows — so
  draining adds no new exposure class.
- **The owners' socket rules live in the owner contract.** The rules that
  keep the drain exact on both owners — when a flow vsock is installed into
  the forwarding maps, its receive watermark, re-arming after accept and at
  pairing, event keying, which poll events abort a flow, and the guest TCP
  intake listener's receive buffer — are pinned in the feature delta's owner
  contract (§ *[REF] Owner and provisioner —
  `overdrive-control-plane::guest_network`* and § *[REF] Guest adaptation
  contract*), with T1–T6 (Context) as their evidence. This decision rests
  on them; it does not restate them.
- **Control-session loss** aborts every flow of the session (DG-5-F,
  ADR-0166); no drain depends on the session.

## Alternatives considered

- **Poll the counters on a period, with a drain deadline that aborts.** The
  deadline aborts slow but healthy flows, the period adds latency to every
  half-close, and the single counter the poll read strands flows on a
  recursion miss (T2) — the deadline only hid that. Rejected.
- **One `fexit` per-call counter.** 3 of 96,000 flows stuck (T2). Rejected.
- **One cumulative `fexit` program plus the backstop.** A cumulative read
  repairs a skipped middle call, and any skip raises that program's
  `recursion_misses`, which triggers the backstop for the last call, so a
  second program only lowers the backstop rate. Not chosen: V-31 measured the
  backstop only with two programs, and at real volume both programs missed
  calls (8 and 71 misses over 77 million fexit calls across 96,000 flows;
  one program's counter was short of the other's in 4 and 28 flows) with no
  flow stuck (`spike-scratch/netns-density-295-v31-r2-halfclose/analysis.txt`,
  `d-full`).
- **Per-call counters in two programs.** 0 of 3,000 under forced skips
  (V-31). Rejected in favour of cumulative reads.
- **A kernel-armed goal from `sock_ops` state callbacks.** Needs three TCP
  transitions, one of which also fires on a reset, a per-socket cell map and
  a `sock_ops` attach on every TCP leg, for no measured saving (T2). Rejected.
- **Half-close at `EPOLLRDHUP` without a drain.** Truncates (T1). Rejected.
- **A drain deadline as a resource bound for a guest withholding credit.**
  The quota already bounds what such a guest holds. Rejected.

## Consequences

- Each side runs two `fexit(skb_send_sock)` programs and one
  `fentry(sock_poll)` program, plus one cell per flow slot sized from the pair
  capacity. The programs run on every `skb_send_sock` and `sock_poll` call
  on the kernel and act only on sockets that map to a flow cell.
- A half-close completes as soon as the target has accepted its last byte;
  it has no drain deadline. The only period is the one-second counter read,
  which bounds detection of a both-skip or a lost record and never ends a
  healthy drain.
- Residual: both fexit programs skipping the same last call, with the
  backstop not yet triggered, was not observed in 96,000 flows; it is a
  validation item in the feature delta's validation plan. Its detection is
  bounded by the one-second counter read.
- The mechanism of T4(a)'s receive-queue overlap has not been read in the
  kernel source; the guest intake listener's receive buffer in the owner
  contract is a measured mitigation (0 in 180,000 flows with 4 MiB), and
  reading the mechanism in `vendors/linux` is a validation item.
- All V-31 microVM runs used the perf-attached classifier build; one microVM
  batch of the integrated owner on the raw build is open before DELIVER of the
  steps that ship this decision. It is not contract-shaping.
- The protocol is covered by the feature delta's § *Formal protocol model*.
