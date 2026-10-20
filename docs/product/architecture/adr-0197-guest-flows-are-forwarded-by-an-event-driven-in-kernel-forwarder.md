# ADR-0197 — Guest flows are forwarded by an event-driven in-kernel forwarder, not by a sockmap redirect

## Status

**Accepted — data-path ruling EVMOD-ADOPT, approved by user 2026-10-16**
("ADOPT evmod as #295's vsock data path, replacing the BPF sockmap
redirect"). GH #295. Recorded in the #295 feature delta, § *[REF] vsock data
path — the in-kernel event-driven forwarder (EVMOD)*, which also carries the
interface contract (the forwarder port, the control device, the end record)
and the decisions this one leaves open. The independent choices that follow
from it are separate ADRs, each approved by the user 2026-10-17: the
module's lifetime (ADR-0198), its build per kernel release (ADR-0203), who
loads it (ADR-0205), the guest kit that pairs a Linux guest kernel with its
module (ADR-0206), the late-callback rules for an ended pair's state
(ADR-0204), the pairs' lifetime bound to their control context (ADR-0199),
the one lossless end record per pair (ADR-0202), no forwarded socket carries
kTLS (ADR-0200), datagram associations (ADR-0201) and the guest-neutral host
side (ADR-0207). How a vsock end is classified is ADR-0180; how a half-close
completes is ADR-0183.

## Context

ADR-0145 forwarded every guest flow with Aya `sk_skb` stream verdicts that
redirect each skb into the paired socket's egress. Guest-slot spike V-3
(`spike/findings-v3-guest-slot.md`, host and guest `7.0.0-29-generic`)
measured that redirect unbounded toward a target that does not read: about
14 MB queued per 16 MiB flow, the guest OOM-killed at about 0.4 GiB queued,
the queue not charged to any memory cgroup. The causes are in the kernel the
redirect runs on: vsock credit is returned before the verdict runs
(`net/vmw_vsock/virtio_transport_common.c:1779`, `:1781 @ v7.0`), and the
egress redirect queues at the target with no limit (`net/core/skmsg.c:989 @
v7.0`). The valve spike (`spike/findings-v3-valve.md`) found no
sockmap-resident mechanism that bounds a vsock source without losing data,
and the valve design built on owner moves and verdict parking was never
approved. The redirect also needed its own completion machinery: a drain
cell per direction, two `fexit(skb_send_sock)` counters and an
`fentry(sock_poll)` backstop, because a redirect queue reports nothing when
it empties.

The evmod spike (`spike/findings-evmod.md`, `spike/findings-evmod-followup.md`;
module source `spike-scratch/netns-density-295-evmod/inc2/module/evmod.c`;
host and guest `7.0.0-29-generic`, physical host `7.0.0-38-generic`) built
an event-driven forwarder in a kernel module. It overrides each socket's
`sk_data_ready`, `sk_write_space`, `sk_state_change` and `sk_error_report`;
a callback only sets a pending bit and queues one work item per pair; the
work item peeks the source (`MSG_PEEK | MSG_DONTWAIT`), sends to the target
(`MSG_DONTWAIT`) and consumes from the source exactly the bytes the target
accepted. Measured:

- **Bounded.** With a target that does not read, the pipeline plateaued at
  about 2.7 MiB (guest), 4.69 MiB and 0.65 MiB (host role) and stayed flat
  for 3 s and 10 s stalls; the sender blocked for the whole stall and
  resumed 1.3–3.5 ms after the reader; unreclaimable slab rose at most
  0.6 MiB. The sockmap control on the same VM queued 160 MiB in 0.36 s.
- **Correct.** 504 trials and 64.6 GiB byte-exact, FIN propagated in both
  directions, 26 of 27 injected faults detected (the 27th did not take
  effect); the final build passed the core battery on all three directions.
- **Fast.** At 16,384 peers and 64 KiB request/reply: 1.198 GiB/s against
  Aya's 0.955, 2.71 against 3.18 core-s/GiB, p99 0.780 against 0.966 ms; at
  parity at 64 peers; 16,466 kernel tasks (no per-pair thread).

The bound follows from the kernel: a peek does not return vsock credit; only
a dequeue advances `fwd_cnt` and sends the credit update
(`net/vmw_vsock/virtio_transport_common.c:806-809`, `:536`, `:583`,
`:654-655 @ v7.0`), and a TCP receive window closes while bytes stay
unconsumed in the receive queue. A byte leaves the source only after the
target accepted it.

## Decision

Every guest flow's bytes are moved, on each side of the VM boundary, by one
event-driven in-kernel forwarder that joins the flow's two sockets and moves
bytes between them by peek, non-blocking send, and consumption of exactly
the bytes the target accepted.

- **No byte leaves a source before its target accepted it.** The source's
  own receive queue is the only place a stalled flow's bytes wait, so vsock
  credit and the TCP window are the backpressure, as for an unforwarded
  socket.
- **A clean end of a source is forwarded as `shutdown(SHUT_WR)` on its
  target after the last byte the target accepted**; there is no separate
  drain mechanism and no deadline (ADR-0183).
- **An error or an abort on either socket aborts the pair**: a TCP socket is
  reset, a vsock socket is shut down in both directions (ADR-0180 for how a
  vsock end is classified).
- **No userspace process reads or writes the payload of a forwarded pair.**
  Both owners keep only control I/O (ADR-0147).
- **Forwarding work runs on a per-CPU workqueue**, one work item per pair,
  queued from the socket callbacks; no kernel thread per pair.
- **On the guest side this is the forwarder of a Linux guest.** The host
  forwarder is guest-neutral (ADR-0207); a guest that runs no Linux kernel
  implements its side of the protocol in its own image
  ([GH #321](https://github.com/overdrive-sh/overdrive/issues/321)).

## Alternatives considered

- **Keep the sockmap redirect and add the valve** (owner moves of every
  vsock-source direction and verdict parking of every TCP source, the
  withdrawn VALVE-1 to VALVE-6). Lossless and bounded in the valve spike's
  separate probes, but every vsock-source byte crosses Overdrive userspace,
  a stalled TCP source needs a park–move–resume protocol with its own
  records and counters, and it keeps the drain cells and counters. Rejected
  by the user's ruling.
- **A userspace pump for every flow** (peek, send, consume in the owner).
  Bounded and correct, but copies every byte through userspace on both sides
  and costs owner CPU per byte. Rejected by the user (CPU cost).
- **A kernel patch bounding the psock backlog (P1).** Overdrive does not
  patch Linux. Rejected by the user.
- **BPF only.** vsock credit is returned before any verdict runs
  (`net/vmw_vsock/virtio_transport_common.c:1779`, `:1781 @ v7.0`), so a
  verdict cannot hold a vsock byte until the target accepted it; parking,
  `SK_PASS`, native knobs and sockmap removal were each measured unbounded
  or lossy (valve spike). Rejected (shown impossible).
- **The shared-memory kernel proxy (`shmproxy.ko`, two kernel threads per
  peer).** At 16,384 peers: 0.199 GiB/s, 49,233 tasks and 5.38 GiB of
  unreclaimable slab. Rejected on cost.

## Consequences

**Positive:**

- The bytes a flow holds are bounded per flow by its sockets' buffers and
  independent of how long a reader stalls; ADR-0183's "a slow reader is
  backpressure" holds.
- No drain cell, `fexit` counter, `fentry` backstop, recursion-miss read,
  flow-event ring or sockmap verdict exists on either side.
- At 16,384 peers it is faster per core and lower in tail latency than the
  Aya redirect, at the same kernel memory and task count.

**Negative:**

- Overdrive ships out-of-tree C kernel code built for the pinned kernel
  (ADR-0068), on the host and in the guest. Rust-for-Linux on 7.0 has no
  socket abstractions (`vendors/linux/rust/kernel/net.rs:6` declares only
  `phy`). It is a security and maintenance surface the redirect did not
  have; it is exercised only through Tier-3 tests.
- The module replaces each forwarded socket's four callbacks and its
  `sk_user_data`; a socket can carry only one such owner (ADR-0200 keeps
  kTLS off forwarded sockets).
- A forwarded single flow on the per-CPU workqueue runs on the CPU of the
  writer's softirq; one large flow reached 890–1,093 MiB/s (TCP to TCP) where
  an unbound workqueue reached 1,242–1,258 MiB/s, but the unbound queue was
  worse for many flows and small messages (0.041 against 0.106 GiB/s at 64
  peers and 1 KiB) (`spike/findings-evmod-followup.md`, item 3).
- With buffers left to autotune, a stalled flow holds up to the TCP autotune
  maxima on its TCP leg plus the vsock buffer (2.6–8 MiB measured in the
  guest); this is a per-flow constant, not growth.
