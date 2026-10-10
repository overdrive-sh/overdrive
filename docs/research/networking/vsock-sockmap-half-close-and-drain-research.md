# Research: Half-close and drain across a SockHash TCP↔AF_VSOCK forwarder

**Date**: 2026-10-10 (re-grounded on Linux v7.0 the same day) | **Researcher**: nw-researcher (Nova) | **Confidence**: High on kernel facts, Medium on the recommended mechanism (unproven on 7.0 until T2) | **Sources**: 22

Scope: GH #295 (`netns-density-295`), the vsock attachment replacement. This answers whether the
drain wait in feature-delta § *Flow end* → *Drain wait (DG-5-D)* (`feature-delta.md` L19357–19388,
`FLOW_DRAIN_RECHECK_PERIOD` = 10 ms at L18231) is needed, and if so how to do it without polling.
This document is research only. It changes no design, spec, code or test file.

**Kernel baseline.** Overdrive's metal host and every spike run Ubuntu `7.0.0-29-generic` on both
host and guest (`guest-vsock-capture-findings.md:20-21,196-198`). Every kernel claim below is
grounded in the upstream **v7.0** tag (annotated tag object `3131ff5a117498bb4b9db3a238bb311cbf8383ce`,
commit `028ef9c96e96197026887c0f092424679298aae8`, "Linux 7.0", 2026-04-12). Citations are
`path:line @ v7.0`. An earlier draft of this document was written against v6.18; § "v6.18 → v7.0
delta" lists what changed.

## Executive Summary

**The drain problem is real on 7.0.** It follows from how `sk_skb` redirects work.
`sk_psock_skb_redirect()` never sends the skb itself. It appends the skb to the *target's*
`psock->ingress_skb` queue and schedules the target's delayed work
(`net/core/skmsg.c:989-990 @ v7.0`). That worker, `sk_psock_backlog()`, later calls
`skb_send_sock()` on the target. When the target is not writeable it re-queues itself with a
one-jiffy delay (`skmsg.c:719-729 @ v7.0`). The source TCP socket meanwhile reports `EPOLLRDHUP` as
soon as `sk_shutdown & RCV_SHUTDOWN` is set (`net/ipv4/tcp.c:593-594 @ v7.0`). Nothing ties that bit
to the target's backlog being empty. Worse, for a FIN-carrying data skb the kernel sets
`RCV_SHUTDOWN` and wakes poll waiters **before** the verdict program has even seen that skb
(`tcp_data_queue`: `tcp_fin` at `net/ipv4/tcp_input.c:5694`, `tcp_data_ready` at `:5714 @ v7.0`).
The earlier draft marked this order as inference; it is now verified. So when the owner sees EOF,
bytes can still be queued in the kernel between the two sockets, or not yet through the verdict at
all. Calling `shutdown(SHUT_WR)` on the target then makes the worker's later `skb_send_sock()` fail
with `EPIPE` (`net/vmw_vsock/af_vsock.c:2176-2178 @ v7.0`), which truncates the stream. ADR-0160
already rejected half-close without a drain check for this reason.

**7.0 still gives no kernel FIN carry-over and no drain-complete event:**

- A FIN-only (length 0) skb redirected to egress makes `skb_send_sock` return 0. The backlog treats
  that as a hard error, reports `EPIPE` and disables the target's TX (`skmsg.c:731-732 @ v7.0`).
  This matches the spike's finding on 7.0.0-29.
- `sk_msg` redirects to a vsock target are refused in both directions, and `sk_skb` redirects to a
  vsock target are refused with `BPF_F_INGRESS` (`net/core/sock_map.c:656,686,1262,1292 @ v7.0`).
  Only `sk_skb` **egress** into vsock is allowed. The earlier draft wrongly said an `sk_skb` ingress
  redirect into vsock works; that has been refused since v6.12 (commit `9c5bd93edf7b`).
- `skmsg.c` has no "backlog empty" hook. Its only tracepoint is `trace_sk_data_ready`
  (`skmsg.c:1165,1274 @ v7.0`).

Production redirectors either stay TCP→TCP and let the real TCP connection carry the FIN (Cilium's
`sk_msg` + `BPF_F_INGRESS`), or copy in userspace so that "write all, then shut down" is ordered by
construction (ztunnel, Firecracker). None of them gives a kernel-only answer for a TCP→vsock egress
redirect.

**Recommendation (unchanged in substance by 7.0).** Keep the drain condition, but make it an event
the kernel raises. Today the owner polls the counters. Instead, the BPF program that already counts
sent bytes at the final hop (`fexit(skb_send_sock)`, ADR-0151) compares its running total with a
per-direction *goal* and, when the total reaches the goal, writes one record to a BPF ringbuf
(`BPF_RB_FORCE_WAKEUP`, `include/uapi/linux/bpf.h:6257 @ v7.0`), which the owner already epolls. A
set-goal-then-check protocol with a fire-once compare-and-swap flag rules out a lost wakeup. The
goal is exact:

- for a TCP source: `tcpi_bytes_received − 1`. `bytes_received` grows by `end_seq − rcv_nxt`
  (`tcp_input.c:3941 @ v7.0`), and `end_seq` includes the FIN (`net/ipv4/tcp_ipv4.c:2133 @ v7.0`).
  This is now verified in source, not only observed by the spike.
- for a vsock source: the byte count the peer owner puts in its `HalfClose` (it knows its own sent
  total exactly).

New on re-reading 7.0 (the facts are not new to 7.0, only to this document): for a TCP source the
goal can be armed **in the kernel**, with no owner step. A `sock_ops` `BPF_SOCK_OPS_STATE_CB` fires
inside `tcp_fin` on ESTABLISHED→CLOSE_WAIT (`tcp_input.c:4960`; `tcp.c:3035 @ v7.0`). At that point
`bytes_received` already includes the FIN and the verdict has not yet run for the final skb, and
`bpf_sock_ops` exposes `bytes_received` (`bpf.h:6939 @ v7.0`). This is an option (R1-alt), not a
replacement.

No timer is part of the mechanism. A hard error at the target already surfaces as `EPOLLERR`
(`sk_psock_report_error` sets `sk_err`, `include/linux/skmsg.h:421 @ v7.0`). A slow reader is
legitimate backpressure and not a fault. So a drain deadline is not needed for correctness. If one
is kept, it is a separate resource policy. One 7.0-specific point strengthens that policy case
without changing the mechanism: on vanilla v7.0 a vsock peer that exceeds its credit has the excess
packet **silently dropped** (`virtio_transport_common.c:1378,1433-1435 @ v7.0`), so a misbehaving
guest can strand a drain forever. Upstream changed this to a connection reset after v7.0
(`a4f0b001782b`, 2026-05-21).

ADR-0180's acknowledged `HalfClose` is still justified, but for **crash discrimination**, not for
byte ordering. A kernel fact might replace it: process exit on a vsock sends one `SHUTDOWN` packet
carrying both `RCV` and `SEND` flags, while `shutdown(SHUT_WR)` sends `SEND` only. Correcting the
earlier draft's Gap 3, that bit **is** observable without payload I/O and without BTF field reads:
the stable tracepoint `vsock:virtio_transport_recv_pkt` carries `op` and `flags` for every received
packet (`include/trace/events/vsock_virtio_transport_common.h:89`;
`virtio_transport_common.c:1622 @ v7.0`). It is still a probe candidate (T5), not a basis for a
decision now.

## Correction to the question's premise

The brief says V-13 found that vsock "did not carry TCP half-close correctly". The V-13 record says
otherwise:

- A clean half-close worked. The clean-close regression was "CLEAN 80/80 byte-exact"
  (`spike/findings-v13.md:59`).
- What V-13 falsified was the *meaning* of a vsock end-of-stream. When an owner dies, its vsocks
  end with an ordinary end-of-stream at the survivor. That end-of-stream arrived before
  control-session loss in 415 of 415 deaths. The survivor took it as a half-close and sent a FIN on
  a truncated stream (`findings-v13.md:5`, `:45-52`; ADR-0180 Context).

So ADR-0180 answers "is this end-of-stream a crash or a half-close?". It does not answer "are the
bytes in order?". The drain wait (ADR-0160, K2) is a separate, older requirement from the
guest-capture spike (`spike/guest-vsock-capture-findings.md:70-74`).

## Research Methodology

**Search strategy.**
- Read the Linux source for `net/core/{skmsg.c,sock_map.c,skbuff.c}`, `include/linux/skmsg.h`,
  `net/ipv4/{tcp.c,tcp_input.c,tcp_ipv4.c,tcp_bpf.c}`,
  `net/vmw_vsock/{af_vsock.c,vsock_bpf.c,virtio_transport_common.c}`,
  `include/uapi/linux/bpf.h`, `include/trace/events/{sock.h,vsock_virtio_transport_common.h}`,
  `net/socket.c`.
- Read the per-file commit history between v6.18, v7.0 and current master (GitHub commits API with
  `path=` at `sha=v7.0` and `sha=master`) to find what changed in each window.
- Read LKML threads for the vsock sockmap series, the NVD/OSV record for CVE-2023-52523, the
  production sources (Cilium `bpf_redir.c`, ztunnel `copy.rs`, Firecracker `csm/connection.rs`),
  the Cloudflare SOCKMAP article, and the local ADRs and spike findings.

**How the v7.0 source was read (and a deviation from the brief).** The brief asked for
`git -C /Users/marcus/git/linux show v7.0:<path>`. This agent had no shell tool, so `git show`
could not be run. What was done instead:
1. The v7.0 tag was confirmed in the local repo (`.git/packed-refs:1841`, tag object `3131ff5a…`).
   The local worktree is `master` at `502d45774af0` (Makefile `7.2.0`), read-only and untouched.
2. Function bodies were read from the local worktree, then checked against v7.0 with verbatim
   fetches of `raw.githubusercontent.com/torvalds/linux/v7.0/<path>` wherever post-v7.0 commits
   touched the function (`vsock_poll`, `virtio_transport_close`, `virtio_transport_read_skb`,
   `virtio_transport_recv_enqueue`, the `recv_connected` RW/SHUTDOWN cases).
3. **v7.0 line numbers** come from Sourcegraph code search pinned to `@v7.0`. Its `lineNumber` is
   0-based and was converted to 1-based. For `skmsg.c` they were cross-checked independently by
   reverse-applying the four post-v7.0 commits' hunk headers to the worktree (`ddf8029623a1`,
   `406e8a651a7b`, `79511603a65b`; `490a83d6386e` is not in the local worktree). Every checked line
   agreed.
4. One line, `tcp_input.c:5004` (`sk_state_change` in `tcp_fin`), was found by search and also
   matches the uniform +21 offset of `tcp_fin` between v7.0 and the worktree.

**Source selection.** Primary kernel source at the pinned tag (High). Kernel docs, LKML and NVD
(High). Upstream project source (High). Vendor engineering blog (Medium-High).

**Limitation — upstream v7.0 is not byte-identical to Ubuntu 7.0.0-29.** Ubuntu's `-29` build
(2026-07-17) carries 7.0.y stable backports, which may include post-v7.0 fixes listed in the delta
section, for example `aae9d8a5528b` (vsock poll snapshot) or `a4f0b001782b` (reset on rx overflow).
None of them changes a conclusion here. Where a backport would matter, it is flagged. T1–T6 run on
the real 7.0.0-29 host and guest and are the authority.

## v6.18 → v7.0 delta (what changed for this document)

| Area | Change between v6.18 and v7.0 | Effect on a conclusion |
|---|---|---|
| `skmsg.c` | `b40cc5adaa80` copied_seq fix (adds `copied_from_self` to `sk_msg_recvmsg`), `929e30f93125` FIONREAD, `ad8391d37f33` UAF fix in `sk_psock_verdict_data_ready` (`READ_ONCE(sk->sk_socket)`/`ops`), `2ef2b20cf4e0` data-race annotations, `69050f8d6d07` kmalloc_obj | None. Redirect, backlog, retry and hard-error paths are unchanged. |
| `af_vsock.c` / `virtio_transport_common.c` | vsock netns support (`eafb64f40ca4`, `a69686327e42`), TX credit capped to local buffer (`8ee784fdf006`), `get_credit` underflow fix (`3ef3d52a1a98`), coalesce only linear skbs (`0386bd321d0f`) | None for drain. The credit cap can make `skb_send_sock` into vsock return short or `-EAGAIN` more often: more backpressure, same mechanism. |
| `vsock_bpf.c`, `sock_map.c` vsock rules | unchanged (the vsock redirect restrictions date from v6.12 / v6.14) | The earlier draft's "vsock can be an `sk_skb` ingress target" was already wrong at v6.18 (see F4). |
| `tcp.c`, `tcp_input.c` | line shifts only for the functions cited | None. |

**After v7.0** (in v7.1/v7.2 or later, **not** in vanilla v7.0; maybe in Ubuntu backports):
`aae9d8a5528b` "vsock: keep poll shutdown state consistent" (`vsock_poll` now takes one
`peer_shutdown` snapshot under the lock); `a4f0b001782b` "vsock/virtio: reset connection on
receiving queue overflow"; `ddf8029623a1` and `79511603a65b` (sockmap/kTLS verdict paths; kTLS
sockmap removed); `2a12c05aef21` (collapse rx queue under memory pressure); `490a83d6386e`
(self-redirect copied_seq) and `a463f55c791b` (vsock `read_skb` accounting, 2026-10-06). None adds a
drain or completion primitive.

## Findings

### F1 — `sk_skb` redirect into the target is always asynchronous through the target's backlog

**Evidence** (`net/core/skmsg.c @ v7.0`):
- `sk_psock_skb_redirect` (`:957-993`) queues and schedules under `ingress_lock`:
  `skb_queue_tail(&psock_other->ingress_skb, skb);` (`:989`),
  `schedule_delayed_work(&psock_other->work, 0);` (`:990`), `return 0;`. If the target's
  `SK_PSOCK_TX_ENABLED` is clear, the skb is dropped with `-EIO` (`:982-987`).
- `sk_psock_verdict_apply` (`:1030`) handles `__SK_REDIRECT` with
  `tcp_eat_skb(psock->sk, skb); err = sk_psock_skb_redirect(psock, skb);` (`:1076-1078`). The
  source has consumed the bytes at this point, but the target has not sent them.
- `sk_psock_handle_skb` (`:646-656`) egress is
  `if (!sock_writeable(psock->sk)) return -EAGAIN; return skb_send_sock(psock->sk, skb, off, len);`
  (`:650-652`).
- `sk_psock_backlog` (`:670-747`) handles `-EAGAIN` by saving `len/off` in `psock->work_state` and
  re-arming `schedule_delayed_work(&psock->work, 1)` (`:719-729`). The kernel itself retries on a
  one-jiffy delay. The target's `sk_write_space` (`sk_psock_write_space`, `:1091-1106`) also
  re-schedules the work at delay 0.

**Confidence**: High. Primary source at v7.0, consistent with the spike's 7.0.0-29 reading
(`guest-vsock-capture-findings.md:85`).

**Analysis**: There is a kernel-held FIFO, the target `psock->ingress_skb` plus a partial-send
cursor in `psock->work_state`, between "the verdict took the bytes" and "the target's sendmsg
accepted them". Its length depends on the target's writeability, so on the far reader. That FIFO is
the drain gap.

### F2 — Source EOF is reported before, and independently of, that FIFO (order now verified)

**Evidence** (`net/ipv4/tcp.c @ v7.0`, `tcp_poll` at `:542`):
`if (shutdown == SHUTDOWN_MASK || state == TCP_CLOSE) mask |= EPOLLHUP;` (`:591`);
`if (shutdown & RCV_SHUTDOWN) mask |= EPOLLIN | EPOLLRDNORM | EPOLLRDHUP;` (`:593-594`).

**Evidence — in-softirq order** (`net/ipv4/tcp_input.c @ v7.0`, `tcp_data_queue` at `:5627`, in
sequence path):
1. `eaten = tcp_queue_rcv(sk, skb, &fragstolen);` (`:5690`). The skb is on `sk_receive_queue` and
   `rcv_nxt`/`bytes_received` advance to `end_seq` (`tcp_queue_rcv` → `tcp_rcv_nxt_update`,
   `:5561`, `:3936-3941`).
2. `if (TCP_SKB_CB(skb)->tcp_flags & TCPHDR_FIN) tcp_fin(sk);` (`:5693-5694`). `tcp_fin` (`:4947`)
   sets `RCV_SHUTDOWN` (`:4953`), moves ESTABLISHED→CLOSE_WAIT through `tcp_set_state` (`:4960`;
   this runs the `sock_ops` `STATE_CB`, `tcp.c:3035`), then calls `sk->sk_state_change(sk)`
   (`:5004`), which wakes poll waiters.
3. `if (!sock_flag(sk, SOCK_DEAD)) tcp_data_ready(sk);` (`:5713-5714`). `tcp_data_ready` (`:5621`)
   calls `sk_data_ready`, which for a psock is `sk_psock_verdict_data_ready` (`skmsg.c:1268`). That
   runs `ops->read_skb(sk, sk_psock_verdict_recv)` (`skmsg.c:1284`), and `tcp_read_skb`
   (`tcp.c:1770-1797`) hands each receive-queue skb, FIN skb included, to the actor
   (`used = recv_actor(sk, skb);` `:1785`) and stops after the FIN (`:1793-1794`).

**Confidence**: High. The earlier draft's *[inference]* is resolved by source.

**Analysis**: For a FIN-on-data skb the owner can be woken with `EPOLLRDHUP` at step 2, on another
CPU (`tcp_poll` takes no socket lock), before step 3 has run the verdict on the final bytes. Later
the bytes can also sit in the target backlog (F1). Neither appears in any poll bit of the *source*.
A correct design must wait for "the target has accepted every byte the source received".

### F3 — The kernel does not propagate FIN through the redirect, and cannot

**Evidence** (`skmsg.c @ v7.0`, `sk_psock_backlog`):
```c
			if (ret <= 0) {
				if (ret == -EAGAIN) { ... goto end; }
				/* Hard errors break pipe and stop xmit. */
				sk_psock_report_error(psock, ret ? -ret : EPIPE);   /* :731 */
				sk_psock_clear_state(psock, SK_PSOCK_TX_ENABLED);   /* :732 */
```
- `__skb_send_sock` (`net/core/skbuff.c:3302 @ v7.0`; `skb_send_sock` at `:3422`) returns
  `orig_len - len`. For `len == 0` neither send loop runs, so it returns 0. A FIN-only skb therefore
  becomes `EPIPE` and disables TX. A partial send returns the positive partial count; the error code
  is returned only when nothing was sent.
- `sk_psock_report_error` (`include/linux/skmsg.h:421 @ v7.0`) is
  `sk->sk_err = err; sk_error_report(sk);`.
- `skb_send_sock` is a data `sendmsg` (`sendmsg_unlocked` → `sock_sendmsg`, `skbuff.c:3292`), so
  even a successful call carries no FIN.
- The guest-capture spike observed exactly this on 7.0.0-29 ("FIN-only (len 0) skb redirected to
  egress → `skb_send_sock` returns 0 → … `EPIPE`, TX disabled", `guest-vsock-capture-findings.md:85`).
  That is why the verdict drops length-0 skbs.

**Confidence**: High.

**Analysis**: The owner must issue the target's `shutdown(SHUT_WR)`, and it must do so after the
FIFO in F1 has emptied for that flow. There is no BPF helper that half-closes a socket.
`bpf_sock_destroy` is an iterator kfunc that aborts; it does not FIN. *[from knowledge, not
re-checked at v7.0]*

### F4 — vsock sockmap at v7.0: what is allowed

**Evidence**:
- `net/vmw_vsock/vsock_bpf.c @ v7.0` (© 2022 Bobby Eshleman, "Based off of net/unix/unix_bpf.c",
  `:4`). `vsock_bpf_update_proto` (`:150`) requires `vsk->transport->read_skb` and returns
  `-EOPNOTSUPP` otherwise (`:164-165`). It swaps in `vsock_bpf_prot`, whose `recvmsg`
  (`vsock_bpf_recvmsg`, `:76`; installed at `:130`) serves the psock `ingress_msg`.
- `virtio_transport_read_skb` (`virtio_transport_common.c:1751 @ v7.0`) dequeues **one** skb per
  call (`__skb_recv_datagram(...)` `:1765`, then `return recv_actor(sk, skb);` `:1781`).
- The sockmap support commit is `634f1a7110b4` "vsock: support sockmap" (net-next 2023-03-29), so
  it landed in v6.4. The cover letter says "Only the virtio transport has been tested" [LKML v3].
- Redirect rules (`net/core/sock_map.c @ v7.0`):
  - `sock_map_redirect_allowed` (`:528`): a connectible vsock target must be ESTABLISHED (`:547-549`).
  - `bpf_sk_redirect_map` / `bpf_sk_redirect_hash` (`:645`, `:1251`):
    `if ((flags & BPF_F_INGRESS) && sk_is_vsock(sk)) return SK_DROP;` (`:656`, `:1262`).
  - `bpf_msg_redirect_map` / `_hash` (`:673`, `:1279`):
    `if (!(flags & BPF_F_INGRESS) && !sk_is_tcp(sk)) return SK_DROP;` (`:684`, `:1290`; this is the
    CVE-2023-52523 fix `b80e31baa436`), then `if (sk_is_vsock(sk)) return SK_DROP;` (`:686`,
    `:1292`; `9c5bd93edf7b`, v6.12).
  - On the TCP sender, an `sk_msg` `__SK_DROP` surfaces as `-EACCES` from `sendmsg`
    (`net/ipv4/tcp_bpf.c:499 @ v7.0`).

**What vsock can do as a sockmap member at v7.0**:
- the *source* of an `sk_skb` verdict, through `read_skb`;
- the *target* of an `sk_skb` **egress** redirect, through `skb_send_sock` → `sendmsg`, while
  ESTABLISHED. The spike shows this works on 7.0.0-29.
- **not** the target of an `sk_skb` ingress redirect (corrects the earlier draft, which claimed it
  could be);
- **not** the target of any `sk_msg` redirect, ingress or egress.

**Confidence**: High (source at v7.0 and commit history).

**Analysis**: The synchronous `sk_msg` egress path, which would push bytes straight into the target
from the sender's `sendmsg` with no backlog FIFO, is unavailable for TCP→vsock. The design is forced
onto the asynchronous `sk_skb` egress path in F1. One more consequence of the ESTABLISHED rule: once
the target vsock leaves ESTABLISHED (for example `virtio_transport_do_close` moving it to
`TCP_CLOSING`; function cited by name, line not re-pinned at v7.0), new redirects to it return
`SK_DROP` at the helper. The source has already eaten the bytes, so they are lost. That is a
failure path T6 must cover, not a drain case.

### F5 — vsock end-of-stream: the kernel distinguishes close from half-close; poll does not, a tracepoint does

**Evidence** (`net/vmw_vsock/virtio_transport_common.c @ v7.0`):
- `virtio_transport_shutdown(vsk, mode)` (`:1084`) sends `VIRTIO_VSOCK_OP_SHUTDOWN` with
  `SHUTDOWN_RCV` and/or `SHUTDOWN_SEND` according to `mode` (`:1089`, `:1091`). `vsock_shutdown`
  (`af_vsock.c:1179`) passes the user's mode through (`vsock_send_shutdown`, `:1221`), so
  `shutdown(SHUT_WR)` sends `SEND` only.
- `virtio_transport_close()` (`:1280`), on `close()` and on process exit, does
  `if ((sk->sk_shutdown & SHUTDOWN_MASK) != SHUTDOWN_MASK) (void)virtio_transport_shutdown(vsk,
  SHUTDOWN_MASK);` (`:1294-1295`). This is one packet carrying both flags. Linger is skipped when
  `current->flags & PF_EXITING` (`:1297`).
- The receiver ORs the flags into `vsk->peer_shutdown` (`OP_SHUTDOWN` case, `:1443-1447`). Vanilla
  v7.0 does a plain `vsk->peer_shutdown |= …`; post-v7.0 `aae9d8a5528b` switched to
  `READ_ONCE`/`WRITE_ONCE`. Same semantics.

**Evidence** (`net/vmw_vsock/af_vsock.c @ v7.0`, `vsock_poll` at `:1230`):
- `EPOLLHUP` if `sk->sk_shutdown == SHUTDOWN_MASK` or (local `SEND_SHUTDOWN` and peer
  `SEND_SHUTDOWN`) (`:1250-1253`).
- `EPOLLRDHUP` if `sk->sk_shutdown & RCV_SHUTDOWN || vsk->peer_shutdown & SEND_SHUTDOWN`
  (`:1256-1258`).
- Nothing reports peer `RCV_SHUTDOWN` through poll. It shows up as `-EPIPE` from
  `vsock_connectible_sendmsg`
  (`if (sk->sk_shutdown & SEND_SHUTDOWN || vsk->peer_shutdown & RCV_SHUTDOWN) { err = -EPIPE; …}`,
  `:2176-2178`).
- The tracepoint `vsock:virtio_transport_recv_pkt`
  (`include/trace/events/vsock_virtio_transport_common.h:89 @ v7.0`) records `src_cid, src_port,
  dst_cid, dst_port, len, type, op, flags, buf_alloc, fwd_cnt`. It fires for every packet entering
  `virtio_transport_recv_pkt` (`virtio_transport_common.c:1608`, trace call `:1622`). With
  vhost-kernel vsock, which the spike uses (`guest-vsock-capture-findings.md:12,25`), that is the
  host kernel for guest→host packets and the guest kernel for host→guest packets.

**Verification**: V-13 variant B, "`EPOLLHUP` never delivered (79/79 events were `mask 8192`)"
(`findings-v13.md:58`), is what `vsock_poll` predicts for a peer close while the survivor's own
send side is open.

**Confidence**: High for the code. Medium for practical use. The tracepoint makes the bit
observable (this corrects the earlier Gap 3), but correlating a trace event with the owner's
`EPOLLRDHUP` without a race is unproven (T5).

**Analysis**: V-13's "a vsock end-of-stream does not state why the peer ended the stream" is true
*at the poll interface*. At the transport level it is false: an owner crash or close puts peer
`RCV_SHUTDOWN` in the same packet, and a clean `shutdown(SHUT_WR)` does not. This is the "make the
fact observable" option in `.claude/rules/design.md` that ADR-0180's alternatives section does not
list. A BPF program on the tracepoint can record `(5-tuple, flags)` into a map or ringbuf with no
payload I/O and no reliance on the non-UAPI `peer_shutdown` field layout. One `SHUTDOWN` with both
flags implies the peer closed or exited. A `SEND`-only one implies a deliberate half-close.

### F6 — Production precedent: nobody gets FIN propagation from the redirect itself

| System | Mechanism | How end-of-stream is handled | Source |
|---|---|---|---|
| Cilium sockmap acceleration | `sk_msg` `msg_redirect_hash(..., BPF_F_INGRESS)` between two **local TCP** sockets | Data skips the stack. FIN travels on the real loopback TCP connection, so the kernel's own TCP state carries it. Not applicable to vsock (F4). | Cilium v1.13 `bpf/sockops/bpf_redir.c` (`__u64 flags = BPF_F_INGRESS;`) |
| Cloudflare SOCKMAP splice demo | `sk_skb` verdict redirect | Userspace "hangs … until the socket is closed" with `poll(POLLRDHUP)`. Half-close and drain are not addressed. | Majkowski, 2019 |
| Istio ambient ztunnel | userspace copy loop | On a read that returns 0: `poll_flush` the writer, then `shutdown()`. Ordering comes from the single copy loop; there is no drain race. | ztunnel `src/copy.rs` |
| Firecracker vsock muxer | userspace copy | A host EOF sends `SHUTDOWN` with **both** `RCV` and `SEND` (no host-side half-close). Guest shutdown → `PeerClosed`. With data still in `tx_buf` it "arm[s] the kill timer" before RST. | firecracker `csm/connection.rs` |

**Confidence**: High (each row is primary source; not kernel-version dependent). Coverage is
partial. Envoy, Kata and gVisor were not read (Gap 4).

**Analysis**: Every precedent either keeps a real TCP connection under the redirect or copies in
userspace. Firecracker, the closest vsock proxy, sidesteps half-close and drains with a timer.
Overdrive's TCP→vsock egress `sk_skb` design has no prior art that solves this. The drain signal
has to be built, and building it from kernel-sourced counters (ADR-0151) is sound. Only the
*notification* is wrong today, because it is polled.

### F7 — What the spikes proved vs assumed

Proved (Ubuntu 7.0.0-29 host and guest, native metal, CH `41a619d19`, vhost-kernel vsock):
- K2 drain-before-shutdown with exact counters gave 120,008 drained shutdowns, 0 truncations, and
  0 drain timeouts with RX withheld 300 ms each way (`guest-vsock-capture-findings.md:242`,
  `:302-303`).
- LRU-evicted counters stall a half-close (`:344-347`; ADR-0151:35-36).
- FIN-only redirect → `EPIPE` (`:85`).
- vsock `read_skb` takes one skb per `data_ready`, with no re-arm (`:96-99`). This matches
  `virtio_transport_read_skb` at v7.0 (F4).
- V-13 measured the crash and clean-half-close facts above.

Assumed or untested:
- The 10 ms re-check period. The spike used its own loop and a 10 s timeout
  (`guest-vsock-capture-findings.md:344`).
- The production tokio owner.
- That the counters are exact across cell reuse and multi-hop chains.
- Whether a stranded vsock skb (the one-skb-per-`data_ready` lag) can block a drain forever.
- Whether the tracepoint in F5 can be correlated with the owner's EOF without a race.
- Which post-v7.0 fixes the Ubuntu `-29` build carries (Methodology, Limitation).

The spike's K2 drain condition was a *poll-loop over a kernel fact*. The kernel fact was proven.
The poll was never a requirement.

## Answers to the research questions

**RQ1.** FIN on the source is not propagated to the target. If redirected as a zero-length skb, it
breaks the target with `EPIPE` (F3). There is no guarantee that redirected bytes have reached the
target's `sendmsg` before the source reports `EPOLLRDHUP` or `read()==0`. For a TCP source the
wakeup can even come before the verdict has run on the final skb (F2, verified at v7.0). The bytes
may then sit in the target psock `ingress_skb` and be retried there indefinitely under backpressure
(F1). For a vsock source, packets are processed in order by one rx path: `OP_RW` →
`recv_enqueue` → `vsock_data_ready` (`virtio_transport_common.c:1433-1435 @ v7.0`) before the
following `OP_SHUTDOWN` is handled (`:1443`). But `vsock_data_ready` (`af_vsock.c:1062`) wakes the
verdict only at `rcvlowat`, and `read_skb` takes one skb per call. So the verdict has been
*notified* of the preceding data, but may not have *consumed* all of it when `EPOLLRDHUP` appears.
*[inference from the source; the stranded case is T4]*

**RQ2.** A drain wait is needed. No existing kernel signal reports it at v7.0:
- Readiness on the target (`EPOLLOUT`) reflects send-buffer space, not the psock FIFO.
- vsock `SIOCOUTQ` (`af_vsock.c:1544`, via `unsent_bytes`, `virtio_transport_common.c:1138`) and
  TCP `SIOCOUTQ` measure the target's own send queue *after* the FIFO.
- There is no psock state bit or tracepoint for "backlog empty". The only skmsg tracepoint is
  `sk_data_ready` (`skmsg.c:1165,1274`). `include/trace/events/sock.h` adds `sock_send_length`,
  which fires on every `sock_sendmsg` (`net/socket.c:719-733 @ v7.0`, so on each `skb_send_sock`
  hop too). That is a per-send event, not a drain event.
- Sending the FIN through the redirect fails (F3).

The primitive that works is a **BPF-raised completion**: a program at the final hop compares a
running sent total with an armed goal and emits a ringbuf record. See the recommendation below.

**RQ3.** See F6. The only kernel-only systems keep a real TCP connection that carries the FIN
(Cilium). Everyone else copies in userspace, where "flush then shutdown" is ordered by
construction. Firecracker drains with a kill timer.

**RQ4.** The acknowledged `HalfClose` is not needed for *byte ordering*. Ordering is the drain plus
vsock's in-order packet path: a `SHUTDOWN` is queued by `virtio_transport_send_pkt_info` after data
the target already accepted. That last point is *[inference]*, to be confirmed by probe T3.
`HalfClose` is needed today for *crash discrimination*, and its acknowledgement removes the
pending-timer hazard of the unacknowledged variant (ADR-0180 Alternatives). That reasoning is
sound. F5 shows the kernel exposes a discriminator through a stable tracepoint. Whether it can
replace the side channel is unproven (T5).

## Recommendation

The recommendation is unchanged in substance by the move to 7.0. Re-checked against v7.0:

- the gap still exists (F1, F2);
- no drain or completion primitive was added in v7.0 or after it, up to the current master;
- `skb_send_sock` still returns exact sent byte counts, including partial sends (F3);
- `BPF_RB_FORCE_WAKEUP` exists (`bpf.h:6257`).

Two additions: R1-alt (an in-kernel goal for TCP sources) and the tracepoint route for R2.

**R1 (adopt, subject to the probes below) — event-driven drain, no polling.**

1. **One exact, non-evicting cell per (flow, direction)** in the existing drain counter map,
   preferably a `BPF_F_MMAPABLE` array indexed by a flow slot. The cell holds `sent` (u64), `goal`
   (u64), `armed` (u32) and `fired` (u32).
2. **Kernel side.** The existing `fexit(skb_send_sock)` program at the *final hop's target* (the
   flow vsock for outer→vsock; the outer leg for vsock→outer) does:
   - `__sync_fetch_and_add(&c->sent, ret)` when `ret > 0`. `ret` is the exact byte count sent,
     including partial sends (`skbuff.c:3302 @ v7.0`, F3).
   - then, if `c->armed && c->sent >= c->goal && cmpxchg(&c->fired, 0, 1) == 0`,
     `bpf_ringbuf_output({flow, dir}, BPF_RB_FORCE_WAKEUP)`.

   Ringbuf supports epoll notification, and `FORCE_WAKEUP` exists for explicit control
   [kernel ringbuf doc; `bpf.h:6257 @ v7.0`]. Counting at the final hop alone is sufficient for a
   multi-hop chain such as the guest parking cell P1→P2→vsock: bytes are conserved and FIFO, and
   the final target carries only this flow.
3. **Owner side, on EOF for a direction.**
   - Set `goal`. For a TCP outer leg it is `TCP_INFO.tcpi_bytes_received − 1`
     (`tcp.c:4342 @ v7.0`; FIN counted per `tcp_input.c:3941`, `tcp_ipv4.c:2133`). Edge case: a TFO
     socket seeds `bytes_received` from the SYN payload (`tcp_fastopen.c`), which is still data, so
     the formula holds. *[read in the worktree only]* For a flow vsock it is the byte count carried
     in the peer's `HalfClose`, a field added to that existing message, because the peer owner
     knows its own final-hop `sent` exactly.
   - Store `armed = 1` with a sequentially consistent atomic.
   - Read `sent`. If `sent >= goal` and `cmpxchg(fired, 0, 1)` succeeds, proceed in the same
     dispatch. Otherwise wait for the ringbuf record, epolled with the flow's other inputs.

   Set-then-check on both sides, with a fire-once CAS, cannot lose a wakeup or fire twice. This is
   the same structure as a futex wait. It replaces `FLOW_DRAIN_RECHECK_PERIOD` and the re-check
   timers outright.
4. **Failure without a timer.** A target hard error sets `sk_err` (`skmsg.h:421`), which raises
   `EPOLLERR` on a socket the owner already watches (`af_vsock.c:1230-…`, the `sk_err` check;
   `tcp_poll`) → abort. Flow error, `Abort`, control loss, teardown and quiescence end the wait as
   they already do. A target that has left ESTABLISHED makes new redirects `SK_DROP` (F4); the
   owner sees that through the target's own `EPOLLHUP`/`EPOLLERR`, not through the drain.
5. **Deadline.** It is **not needed for correctness**. A drain that does not complete because the
   far reader is slow is ordinary TCP backpressure; a TCP half-closer in the kernel has no such
   deadline either. A drain that does not complete because bytes are stranded is a *defect* to
   make unrepresentable, not time out. Stranding causes: the vsock one-skb-per-`data_ready` lag,
   counter eviction, cell-epoch confusion, and on vanilla v7.0 an over-credit vsock packet silently
   dropped by `virtio_transport_recv_enqueue` (`virtio_transport_common.c:1365-1378`, return
   ignored at `:1434`).

   That last cause is guest-controllable. Together with the fact that a guest can always be a slow
   reader, it argues for a per-VM **resource bound** on half-closed flows, decided as its own
   policy, akin to `TCP_USER_TIMEOUT` or the quota. It is never the drain mechanism.
   `FLOW_DRAIN_DEADLINE` as an abort-on-slow-reader would turn legitimate slow consumers into
   resets. *[interpretation]*

**R1-alt (consider; not required) — arm the TCP goal in the kernel.** For a TCP source, a cgroup
`sock_ops` program with `BPF_SOCK_OPS_STATE_CB_FLAG` (`bpf.h:6979`) set on the outer-leg socket
receives `BPF_SOCK_OPS_STATE_CB` (`bpf.h:7078`) from `tcp_set_state` (`tcp.c:2997`, call at
`:3035`). `tcp_fin` makes that call on ESTABLISHED→CLOSE_WAIT (`tcp_input.c:4960`). At that point
`bytes_received` already includes the FIN (step 1 of F2), and the verdict has not yet run for the
final skb (step 3). The program can write `goal = ctx->bytes_received − 1` (`bpf.h:6939`), set
`armed`, and do the same `sent >= goal` check. The owner then only consumes the ringbuf record and
issues `shutdown(SHUT_WR)`, with no `TCP_INFO` call.

Costs: a `sock_ops` attachment on the outer leg's cgroup, a per-socket flag set, and a socket→cell
mapping (for example `sk_storage`). It does not apply to vsock sources, which still need the peer's
`HalfClose` count. *[interpretation; the kernel facts are verified, the design is not]*

**R2 (investigate, do not adopt yet) — kernel crash discrimination instead of `HalfClose`.** Read
the peer's `SHUTDOWN` flags payload-free through the `vsock:virtio_transport_recv_pkt` tracepoint
(F5). If T5 holds, an unannounced end can be classified by the kernel fact, saving one control
round trip per half-close. The `HalfClose` byte count used in R1 would then come from the
survivor's own `fwd` counter at `EPOLLRDHUP` instead, which T4 must prove exact. Until then
ADR-0180 stands.

### Tier-3 probes required on 7.0 (native metal host and guest on Ubuntu 7.0.0-29, the same kernel the spikes ran; record `uname -r` and the Ubuntu package version)

**T1 — the drain gap exists on 7.0.**
- *Hypothesis*: with the target's reader paused, EOF on the source precedes the target having
  accepted all bytes.
- *Predicted*: after `EPOLLRDHUP`, `sent < tcpi_bytes_received − 1` for at least the pause; a
  premature `shutdown(SHUT_WR)` truncates and the target reports `EPIPE`. Also predicted, from F2:
  with the reader *not* paused, a FIN-on-data final segment still shows `sent < goal` at the first
  `EPOLLRDHUP` in some trials.
- *Falsified if*: `sent` always equals the goal at `EPOLLRDHUP` across 10k trials with a 0 ms,
  300 ms and 3 s pause. That would mean no drain wait is needed on 7.0.

**T2 — R1 event correctness.**
Matrix: TCP→vsock and vsock→TCP; guest intake through the P1→P2 parking cell; host destination
legs; reader paused 0 / 300 ms / 3 s; 2 MiB each way; ≥100k half-closes.
- *Predicted*:
  - exactly one ringbuf record per armed direction;
  - zero records before the receiver has every byte (the receiver checks its byte count when it
    reads FIN);
  - zero truncations;
  - zero stuck directions.
- *Adversarial cases*:
  - arm when `sent` already equals the goal (the owner fires and the kernel does not);
  - arm racing the final `skb_send_sock`, pinning CPUs to force interleaving;
  - cell reuse across flows;
  - if R1-alt is built: kernel arming from `STATE_CB` racing the final verdict.
- *Falsified by* any missed, duplicate or early record.

**T3 — vsock shutdown ordering.**
- *Hypothesis*: `shutdown(SHUT_WR)` on a flow vsock is delivered after all bytes previously
  accepted by `sendmsg`, including when peer credit is exhausted.
- *Probe*: exhaust peer credit (stall the guest reader), then shut down.
- *Falsified by* an end-of-stream observed before the final byte.

**T4 — goal exactness.**
- For FIN-only skbs and FIN-on-data skbs: `tcpi_bytes_received − 1` equals the verdict's
  forwarded total at completion.
- For a vsock source: the peer's final-hop `sent` equals the local `fwd` at `EPOLLRDHUP`.
- Include a flow whose vsock had a backlog at install time, to expose the one-skb lag.
- Include a sub-`rcvlowat` tail, to test whether `vsock_data_ready`'s gate strands it.

**T5 — R2 discriminator.**
- SIGKILL of an owner, `shutdown(SHUT_WR)`, and clean close; 110 trials per cell, in V-13's
  matrix shape.
- *Predicted*: the `vsock:virtio_transport_recv_pkt` event for the flow shows `op=SHUTDOWN` with
  `flags=0x3` for kill and close, and `flags=0x2` (SEND only) for `SHUT_WR`.
- Read it payload-free from a BPF program on that tracepoint, keyed by the 4-tuple. The fallback is
  a zero-length `send()` returning `EPIPE`.
- *Falsified by* any kill that is classified as a half-close, or a trace record that arrives after
  the owner has acted on `EPOLLRDHUP`.

**T6 — failure path.** Reset the target mid-drain, and separately move the target vsock out of
ESTABLISHED mid-drain.
- *Predicted*: the owner sees `EPOLLERR` or `EPOLLHUP` on the target and aborts, and no ringbuf
  record arrives.

## Source Analysis

| Source | Domain | Reputation | Type | Access | Cross-verified |
|---|---|---|---|---|---|
| Linux v7.0 `net/core/skmsg.c` | kernel source (local repo + github.com/torvalds + Sourcegraph @v7.0) | High | primary source | 2026-10-10 | Y (diff arithmetic; spike 7.0 reading) |
| Linux v7.0 `net/core/sock_map.c` | same | High | primary source | 2026-10-10 | Y (commit history) |
| Linux v7.0 `net/core/skbuff.c` (`__skb_send_sock`, `skb_send_sock`) | same | High | primary source | 2026-10-10 | Y (spike F3) |
| Linux v7.0 `include/linux/skmsg.h` | same | High | primary source | 2026-10-10 | Y |
| Linux v7.0 `net/ipv4/tcp.c`, `tcp_input.c`, `tcp_ipv4.c`, `tcp_bpf.c` | same | High | primary source | 2026-10-10 | Y |
| Linux v7.0 `net/vmw_vsock/af_vsock.c` | same (+ raw v7.0 fetch of `vsock_poll`) | High | primary source | 2026-10-10 | Y (V-13 variant B) |
| Linux v7.0 `net/vmw_vsock/vsock_bpf.c` | same | High | primary source | 2026-10-10 | Y (LKML) |
| Linux v7.0 `net/vmw_vsock/virtio_transport_common.c` | same (+ raw v7.0 fetch) | High | primary source | 2026-10-10 | Y (spike) |
| Linux v7.0 `include/uapi/linux/bpf.h`, `include/trace/events/vsock_virtio_transport_common.h`, `include/trace/events/sock.h`, `net/socket.c` | same | High | primary source | 2026-10-10 | Y |
| GitHub commits API, per-path history at `v7.0` and `master` | api.github.com | High | VCS metadata | 2026-10-10 | Y |
| LKML "vsock: add support for sockmap" v3 cover | lkml.iu.edu | High | mailing list | 2026-10-10 | Y (commit `634f1a7110b4`) |
| CVE-2023-52523 | nvd.nist.gov, osv.dev | High | vuln DB | 2026-10-10 | Y (`sock_map.c:684`) |
| docs.kernel.org BPF ring buffer | kernel.org | High | official docs | 2026-10-10 | Y (`bpf.h`) |
| Cilium v1.13 `bpf_redir.c` | github.com/cilium | High | primary source | 2026-10-10 | N |
| istio/ztunnel `src/copy.rs` | github.com/istio | High | primary source | 2026-10-10 | N |
| Firecracker `csm/connection.rs` | github.com/firecracker-microvm | High | primary source | 2026-10-10 | N |
| Cloudflare "SOCKMAP – TCP splicing of the future" | blog.cloudflare.com | Medium-High | industry | 2026-10-10 | N |
| Local: ADR-0151, ADR-0160, ADR-0180; `findings-v13.md`; `guest-vsock-capture-findings.md`; `feature-delta.md` L18231, L19330-19410 | repo | High (project SSOT) | internal | 2026-10-10 | Y |

## Knowledge Gaps

**Gap 1 — No probe of the drain mechanism itself.** The spikes ran on 7.0.0-29 and proved the
polled K2 drain, but R1/R1-alt are unbuilt. *Do*: T1–T6 on the 7.0.0-29 metal host and guest.

**Gap 2 — Ubuntu 7.0.0-29 vs upstream v7.0.** The backports in the Ubuntu build were not
enumerated. The relevant candidates (`aae9d8a5528b`, `a4f0b001782b`, `ddf8029623a1`) change no
conclusion, but `a4f0b001782b` would turn the silent over-credit drop (R1 §5) into a reset. *Do*:
check the Ubuntu 7.0 changelog for those commit subjects.

**Gap 3 — Correlating the vsock tracepoint with owner EOF.** The `SHUTDOWN` flags are now known to
be observable payload-free (F5). Whether the trace record reliably arrives before the owner reads
`EPOLLRDHUP` was not studied. The tracepoint fires in `virtio_transport_recv_pkt` before
`sk_state_change`, which suggests yes *[inference]*. T5 checks it.

**Gap 4 — Envoy, Kata agent and gVisor netstack** half-close handling were not read. All are
userspace copy paths, so they are unlikely to change the conclusion.

**Gap 5 — Memory ordering between userspace atomics on a `BPF_F_MMAPABLE` map and BPF atomics.**
This is assumed sequentially consistent for `BPF_FETCH`/cmpxchg operations, from knowledge only.
T2's race cases are the check.

**Gap 6 — Cilium removing sockmap acceleration.** I could not find the deprecation record (one
search, one upgrade-guide read). It does not affect the conclusion.

**Gap 7 — `git show` not used.** v7.0 was read through the local worktree plus v7.0-pinned remote
reads (Methodology). A reader with a shell can re-verify any citation with
`git -C /Users/marcus/git/linux show v7.0:<path> | sed -n '<line>p'`.

## Conflicting Information

**V-13 / ADR-0180 vs F5.** "A vsock end-of-stream does not state why the peer ended the stream."
- Position A (ADR-0180): true, measured through poll flags.
- Position B (kernel source): the transport records it (one `SHUTDOWN_MASK` packet on close or
  exit), and the `virtio_transport_recv_pkt` tracepoint exposes it.
- Assessment: both are correct at their own layer. The ADR's alternatives should add "observe the
  peer's SHUTDOWN flags via the vsock tracepoint" and state why it lost, or adopt it after T5.

**Earlier draft of this document vs v7.0 source.** The draft said vsock can be the target of an
`sk_skb` ingress redirect. `sock_map.c:656,1262 @ v7.0` (and since v6.12) drop it. Resolved in
favour of the source. No design in #295 depends on that path.

## Full Citations

[1] Linux kernel, tag v7.0 (commit `028ef9c96e96`). `net/core/skmsg.c`: `sk_psock_handle_skb`
    :646, `sk_psock_backlog` :670, `sk_psock_skb_redirect` :957, `sk_psock_verdict_apply` :1030,
    `sk_psock_verdict_recv` :1236, `sk_psock_verdict_data_ready` :1268. Read via the local repo and
    https://raw.githubusercontent.com/torvalds/linux/v7.0/net/core/skmsg.c. Accessed 2026-10-10.
[2] Linux v7.0 `include/linux/skmsg.h`: `sk_psock_report_error` :421.
[3] Linux v7.0 `net/ipv4/tcp.c`: `tcp_poll` :542, `tcp_read_skb` :1770, `tcp_set_state` :2997,
    `tcpi_bytes_received` :4342.
[4] Linux v7.0 `net/ipv4/tcp_input.c`: `tcp_rcv_nxt_update` :3936, `tcp_fin` :4947,
    `tcp_data_ready` :5621, `tcp_data_queue` :5627. `net/ipv4/tcp_ipv4.c` `end_seq` :2133.
    `net/ipv4/tcp_bpf.c` `-EACCES` :499.
[5] Linux v7.0 `net/core/sock_map.c`: `sock_map_redirect_allowed` :528, `bpf_sk_redirect_map`
    :645, `bpf_msg_redirect_map` :673, `bpf_sk_redirect_hash` :1251, `bpf_msg_redirect_hash` :1279.
[6] Linux v7.0 `net/core/skbuff.c`: `__skb_send_sock` :3302, `skb_send_sock` :3422.
[7] Linux v7.0 `net/vmw_vsock/af_vsock.c`: `vsock_data_ready` :1062, `vsock_shutdown` :1179,
    `vsock_poll` :1230, `SIOCOUTQ` :1544, connectible sendmsg `-EPIPE` :2176-2178.
[8] Linux v7.0 `net/vmw_vsock/vsock_bpf.c`: `vsock_bpf_recvmsg` :76, `vsock_bpf_update_proto` :150.
[9] Linux v7.0 `net/vmw_vsock/virtio_transport_common.c`: `virtio_transport_shutdown` :1084,
    `virtio_transport_unsent_bytes` :1138, `virtio_transport_close` :1280,
    `virtio_transport_recv_enqueue` :1365, `virtio_transport_recv_connected` :1425,
    `virtio_transport_recv_pkt` :1608, `virtio_transport_read_skb` :1751.
[10] Linux v7.0 `include/uapi/linux/bpf.h`: `BPF_RB_FORCE_WAKEUP` :6257, `struct bpf_sock_ops`
     :6898 (`bytes_received` :6939), `BPF_SOCK_OPS_STATE_CB_FLAG` :6979, `BPF_SOCK_OPS_STATE_CB`
     :7078. `include/trace/events/vsock_virtio_transport_common.h` `virtio_transport_recv_pkt` :89.
     `net/socket.c` `trace_sock_send_length` :722.
[11] Commit history (GitHub commits API, `path=` filter, `sha=v7.0` and `sha=master`) for the files
     above; commits cited by 12-char SHA in the text. https://api.github.com/repos/torvalds/linux/commits.
     Accessed 2026-10-10.
[12] Eshleman, B. "[PATCH net-next v3 0/3] vsock: add support for sockmap". LKML, 2023-02.
     https://lkml.rescloud.iu.edu/2302.3/04420.html; RFC v2 https://lkml.rescloud.iu.edu/2301.3/08433.html.
     Accessed 2026-10-10.
[13] NIST NVD. "CVE-2023-52523 — bpf, sockmap: Reject sk_msg egress redirects to non-TCP sockets".
     https://nvd.nist.gov/vuln/detail/CVE-2023-52523; https://osv.dev/vulnerability/CVE-2023-52523.
     Accessed 2026-10-10.
[14] Linux kernel documentation. "BPF ring buffer". https://docs.kernel.org/bpf/ringbuf.html.
     Accessed 2026-10-10.
[15] Cilium. `bpf/sockops/bpf_redir.c`, v1.13.0.
     https://raw.githubusercontent.com/cilium/cilium/v1.13.0/bpf/sockops/bpf_redir.c. Accessed 2026-10-10.
[16] Istio. ztunnel `src/copy.rs`. https://raw.githubusercontent.com/istio/ztunnel/master/src/copy.rs.
     Accessed 2026-10-10.
[17] Firecracker. `src/vmm/src/devices/virtio/vsock/csm/connection.rs`.
     https://raw.githubusercontent.com/firecracker-microvm/firecracker/main/src/vmm/src/devices/virtio/vsock/csm/connection.rs.
     Accessed 2026-10-10.
[18] Majkowski, M. "SOCKMAP - TCP splicing of the future". Cloudflare blog, 2019-02-18.
     https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/. Accessed 2026-10-10.
[19] Overdrive. ADR-0180, ADR-0160, ADR-0151 (`docs/product/architecture/`).
[20] Overdrive. `docs/feature/netns-density-295/spike/findings-v13.md`;
     `.../spike/guest-vsock-capture-findings.md`; `.../feature-delta.md` L18231, L19330–19410.

## Research Metadata

- Examined: about 35 sources; cited: 22, counting each kernel file group, the commit history and
  each local artifact.
- Kernel baseline: upstream v7.0 (`028ef9c96e96`); local repo `master` `502d45774af0` (7.2.0) read
  only, never modified. v7.0 line numbers from Sourcegraph `@v7.0`, cross-checked for `skmsg.c` by
  diff arithmetic.
- Resolved from the earlier draft: the `tcp_input.c` FIN/data-ready order (now High), the
  `bpf_sock_ops` fields (now High), the vsock sockmap merge version (v6.4).
- Confidence: High on F1–F5 kernel facts and F6; Medium on R1/R1-alt until T2; Low on R2 until T5.
- Output: `docs/research/networking/vsock-sockmap-half-close-and-drain-research.md`.
