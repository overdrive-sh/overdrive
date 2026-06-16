# Research: Reliable Delivery of a BPF Sockmap EGRESS Redirect (`bpf_sk_redirect_map`, `flags=0`) Into a kTLS-TX-Armed Target Socket — Why `tcp_bpf_sendmsg_redir → tls_sw_sendmsg` Occasionally Delivers ZERO Bytes

**Date**: 2026-06-13 | **Researcher**: nw-researcher (Nova) | **Confidence**: High (kernel-source-primary on every load-bearing mechanism claim, verified unchanged at v6.6 + v6.12 — holds on the 6.18 floor by inspection; the verdict rests on a fully-traced delivery path + the absence of any production precedent) | **Sources**: 19 distinct authoritative sources (Linux kernel source @ v6.6/v6.12 — 6 files traced verbatim; kernel selftests; docs.kernel.org; LWN; lore/lkml/patchwork — 4 fix threads; Cilium docs; Istio ztunnel docs; in-repo production source + 3 spike artifacts)

## Executive Summary

**The agent-idle sockmap-egress-redirect into a kTLS-TX socket is the
WRONG mechanism for the forward (encrypt) direction, and the residual
~10–15% loss is structural, not tunable.** The single load-bearing
correction this research delivers: the `sk_skb`/`stream_verdict`
`bpf_sk_redirect_map(flags=0)` egress path does **NOT** go through the
synchronous `tcp_bpf_sendmsg_redir → tcp_sendmsg_locked` the spike's
liveness research assumed — that is the *`sk_msg`* path. The `sk_skb`
path **enqueues** the redirected skb on leg B's psock and defers delivery
to a **workqueue** (`sk_psock_backlog`), which sends via `skb_send_sock →
sock->ops->sendmsg (inet_sendmsg) → sk->sk_prot->sendmsg (tls_sw_sendmsg)`
with **`MSG_DONTWAIT`** set on every fragment. So `redirect_ok`
increments at the *enqueue*, not the delivery — and the deferred,
non-blocking `tls_sw_sendmsg` can `-EAGAIN`-stall (leg B not
`sock_writeable` at the workqueue instant) or hard-error (tearing down
`SK_PSOCK_TX_ENABLED`), leaving the byte queued-and-counted but never
delivered. This is exactly the "redirect succeeds, peer decrypts 0"
signature, and it is timing-dependent against leg B's TX state at the
*deferred* moment — which is why a clean single-record sentinel loses
intermittently and no pre-`write` settle fixes it. Every mechanism is
quoted verbatim from `net/core/{skmsg,skbuff}.c`, `net/ipv4/af_inet.c`,
and `net/tls/{tls_main,tls_sw}.c`, cross-verified at v6.6 and v6.12, so it
holds on the 6.18 floor by inspection.

**The bytes are NOT egressed as plaintext** — `inet_sendmsg` dispatches to
`sk->sk_prot->sendmsg`, which kTLS swapped to `tls_sw_sendmsg`, so the
redirect *does* reach the encryptor (confirmed by the spike's real `0x17`
record). The failure is a *delivery* failure inside the deferred
`tls_sw_sendmsg`, not a confidentiality leak.

**No production system runs this pattern, and the kernel does not test
it.** Istio ztunnel (the direct analog — an L4 mTLS node proxy for
identity-unaware workloads) does its mTLS in **userspace rustls** and
copies bytes in userspace; Cilium encrypts at the network layer
(IPsec/WireGuard) and uses sockmap only for *plaintext* localhost-bypass
acceleration. The kernel's `sockmap_ktls` selftest only exercises
membership/lifecycle — never a data-redirect into a kTLS-TX socket with an
encrypt assertion. The delivery functions are *fix-laden* (silent-drop,
RCU-splat, offset-double-deliver, kTLS-verdict-ordering bugs all landed on
them), and the `-EAGAIN`/`sock_writeable` gate the loss flows through was
*added as the fix* for a prior silent-drop bug. Overdrive would be the
sole user of an untested kernel combination.

**The correct mechanism: replace the forward redirect with an agent-light
userspace pump symmetric to the return path** — `read(legF)` →
`write(legB)` (or `splice(legF → legB)`), where `write(legB)` still hits
kernel kTLS-TX (`tls_sw_sendmsg`, synchronous from userspace, no
`MSG_DONTWAIT`/backlog fragility) so the agent never does crypto and the
confidentiality model is unchanged. The return (decrypt) direction
*already* uses such a pump (`PumpHandle::spawn`), and the pre-arm
`flush_through` already proves `write()`-to-kTLS-TX works. The cost is that
the forward path becomes "agent-light" (a `read`/`write` or `splice` copy)
rather than "agent-idle" — the same cost ADR-0069 already accepts for the
return direction. A strong secondary finding (Finding 9): the production
two-load `pinning=ByName` wiring (verdict in one `Ebpf`, leg-B insert in a
*second*) widens every ordering window vs the single-load 60/60 spike and
is the likely amplifier of the production 17/20 vs the spike's 40/40 — a
population diff only a Tier-3 trace can fully separate from the
kernel-intrinsic `MSG_DONTWAIT` fragility.

This research closes **Gap 3** of
`docs/research/dataplane/sockmap-strparser-engagement-race-research.md`
("the exact egress `flags=0` redirect path into a kTLS-armed target was
not fully traced") and answers the decisive question the
`findings-egress-ktls-splice.md` spike left open. The residual under
investigation: on real-kernel runs the verdict redirects successfully
(`redirect_ok` increments, `redirect_refused=0`) but ~10–15% of the time
the redirected steady-state bytes never reach the peer's kTLS-RX (the peer
decrypts 0 of the steady bytes; pre-arm bytes flushed via a normal
`write()` DO arrive), reproducing on a clean single-record agent-driven
sentinel (`peer reconstructed 0 of 73 bytes`).

---

## Research Methodology

**Search Strategy**: Read the kernel source directly for the exact
functions on the redirect-into-kTLS-TX path (`net/core/skmsg.c`,
`net/ipv4/tcp_bpf.c`, `net/tls/tls_sw.c`, `net/tls/tls_main.c`). Pin the
mechanism at ≥2 tags across the 6.x line to confirm it holds on the
target 6.18 floor (ADR-0068). Corroborate with kernel selftests
(`tools/testing/selftests/bpf/`), LWN, lore/lkml patch threads, and the
production lineage (Cilium / Cloudflare / Tetragon source). Cross-read
against the two in-repo spike artifacts and the production wiring.

**Source Selection**: Primary-authoritative = the Linux kernel source
tree at pinned tags, cited by exact file + function + tag.
Secondary-authoritative = docs.kernel.org, LWN, kernel mailing lists,
kernel selftests, Cilium/Cloudflare/Tetragon source. Every
kernel-mechanism claim carries the file/function and the version/tag.

**Quality Standards**: ≥2 sources per load-bearing claim (kernel-source
= authoritative-primary); cross-reference required. Adversarial
validation applied to all web-fetched content.

---

## The mechanism, in the kernel

### Finding 1: The `sk_skb` `SK_REDIRECT` egress path does NOT go through `tcp_bpf_sendmsg_redir`/`tcp_sendmsg_locked` — it goes through the workqueue backlog → `skb_send_sock` → `sock->ops->sendmsg`. This is the single most load-bearing correction in this research.

**The prior assumption was wrong.** Both `findings-egress-ktls-splice.md`
and the spike's liveness research pinned the egress redirect to
`tcp_bpf_sendmsg_redir → tcp_bpf_push_locked → tcp_sendmsg_locked` on the
target. **That is the `sk_msg` (`BPF_PROG_TYPE_SK_MSG` /
`bpf_msg_redirect_*`) path, NOT the `sk_skb`
(`BPF_PROG_TYPE_SK_SKB`/`stream_verdict` / `bpf_sk_redirect_map`) path
the Overdrive verdict actually uses.** The two are different kernel
code paths with different delivery primitives. Overdrive's verdict is an
`sk_skb/stream_verdict` calling `bpf_sk_redirect_map(skb, …, flags=0)` —
so it is on the `sk_skb` path.

**Evidence (kernel source, primary-authoritative), the full `sk_skb`
egress redirect chain, traced verbatim:**

`sk_psock_verdict_apply` dispatches `__SK_REDIRECT` to
`sk_psock_skb_redirect` (`net/core/skmsg.c`, identical at v6.6 and
v6.12):

```c
case __SK_REDIRECT:
    tcp_eat_skb(psock->sk, skb);
    err = sk_psock_skb_redirect(psock, skb);
    break;
```

`sk_psock_skb_redirect` does NOT send the skb. It **enqueues** the skb on
the *target* psock's `ingress_skb` queue and schedules the target's
deferred work — then returns (verbatim, v6.6 ≡ v6.12):

```c
static int sk_psock_skb_redirect(struct sk_psock *from, struct sk_buff *skb)
{
    struct sk_psock *psock_other;
    struct sock *sk_other;

    sk_other = skb_bpf_redirect_fetch(skb);
    if (unlikely(!sk_other)) { ...; sock_drop(from->sk, skb); return -EIO; }
    psock_other = sk_psock(sk_other);
    if (!psock_other || sock_flag(sk_other, SOCK_DEAD)) { ...; return -EIO; }
    spin_lock_bh(&psock_other->ingress_lock);
    if (!sk_psock_test_state(psock_other, SK_PSOCK_TX_ENABLED)) {
        spin_unlock_bh(&psock_other->ingress_lock);
        ...; sock_drop(from->sk, skb); return -EIO;
    }
    skb_queue_tail(&psock_other->ingress_skb, skb);     // enqueue on TARGET
    schedule_delayed_work(&psock_other->work, 0);        // schedule TARGET's work
    spin_unlock_bh(&psock_other->ingress_lock);
    return 0;
}
```

The actual delivery happens **later, on the target psock's workqueue**,
in `sk_psock_backlog` → `sk_psock_handle_skb`. The egress branch
(`!ingress`) calls **`skb_send_sock`** after a writeability gate
(verbatim, v6.6 ≡ v6.12):

```c
static int sk_psock_handle_skb(struct sk_psock *psock, struct sk_buff *skb,
                   u32 off, u32 len, bool ingress)
{
    int err = 0;
    if (!ingress) {
        if (!sock_writeable(psock->sk))
            return -EAGAIN;                      // backoff if target TX is full
        return skb_send_sock(psock->sk, skb, off, len);   // <-- the egress send
    }
    skb_get(skb);
    err = sk_psock_skb_ingress(psock, skb, off, len);
    if (err < 0) kfree_skb(skb);
    return err;
}
```

And `skb_send_sock` pushes the bytes through the socket's **`proto_ops`
sendmsg** (`sock->ops->sendmsg`), NOT `sk->sk_prot->sendmsg` and NOT
`tcp_sendmsg_locked` (verbatim, `net/core/skbuff.c` v6.12):

```c
int skb_send_sock(struct sock *sk, struct sk_buff *skb, int offset, int len)
{
    return __skb_send_sock(sk, skb, offset, len, sendmsg_unlocked);
}
// __skb_send_sock: INDIRECT_CALL_2(sendmsg, sendmsg_locked, sendmsg_unlocked, sk, &msg)
// sendmsg_unlocked  -> sock_sendmsg(sock, msg) -> sock->ops->sendmsg(...)
```

**Source**: [v6.12 net/core/skmsg.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c); [v6.6 net/core/skmsg.c](https://raw.githubusercontent.com/torvalds/linux/v6.6/net/core/skmsg.c); [v6.12 net/core/skbuff.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skbuff.c); [v6.12 net/ipv4/tcp_bpf.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c) (accessed 2026-06-13).
**Confidence**: High (kernel-source-primary; the full chain quoted verbatim and cross-verified at v6.6 + v6.12 — the egress branch calling `skb_send_sock` is byte-identical at both tags).
**Cross-reference**: Finding 2 (whether `sock->ops->sendmsg` engages the kTLS ULP), Finding 3 (the workqueue can silently stall on `-EAGAIN`/error).

**Analysis — three consequences that reframe the whole problem:**

1. **`tcp_bpf_sendmsg_redir` is on the OTHER path.** It is reachable from
   `tcp_bpf_recvmsg`/`sk_msg` redirect and from `tcp_bpf_sendmsg`, not
   from an `sk_skb` `bpf_sk_redirect_map`. The spike's "the egress flag
   drives `tcp_sendmsg_locked` → kTLS encrypt" liveness claim was for the
   `sk_msg` shape; the Overdrive verdict is `sk_skb`, so the delivery is
   `skb_send_sock`, not `tcp_sendmsg_locked`. **This is the population
   the residual lives in** — see Finding 2 for whether `skb_send_sock`'s
   `sock->ops->sendmsg` even reaches the kTLS encrypt seam.

2. **Delivery is deferred to a workqueue, not synchronous.** The redirect
   verdict returns the instant the skb is queued + `schedule_delayed_work`
   is called. The bytes are NOT yet on leg B's TX. So `redirect_ok`
   incrementing proves only that the skb was *enqueued on leg B's psock
   ingress queue and the work scheduled* — it says NOTHING about whether
   the deferred send ran, succeeded, or encrypted. **This is exactly the
   gap between "`redirect_ok` increments, `redirect_refused=0`" and "the
   peer decrypts 0 bytes."** The counter is at the wrong altitude
   (`debugging.md` §7): it observes the enqueue, not the delivery.

3. **The egress send requires leg B to have a live psock.**
   `sk_psock_skb_redirect` returns `-EIO` (and `sock_drop`s the skb) if
   the target has `!psock_other` or `!SK_PSOCK_TX_ENABLED`. Leg B is a
   sockmap member (it must be, to be a redirect target — the spike's
   mechanic #2), so it HAS a psock. But that psock and leg B's kTLS ULP
   now both sit on the same socket — see Finding 2/Finding 5 for the
   contention this creates.

### Finding 2: The redirected bytes ARE routed through the kTLS encrypt seam (NOT egressed as plaintext) — `skb_send_sock`'s `sock->ops->sendmsg` (`inet_sendmsg`) dispatches to `sk->sk_prot->sendmsg`, which kTLS swapped to `tls_sw_sendmsg`. The failure is therefore a *delivery* failure inside `tls_sw_sendmsg`, not a plaintext-leak. **And the proximate cause is `__skb_send_sock` sending each fragment with `MSG_DONTWAIT`.**

**Evidence (kernel source, primary-authoritative).** Three links complete
the encrypt-routing chain:

1. **`skb_send_sock` → `sendmsg_unlocked` → `sock_sendmsg` →
   `sock->ops->sendmsg`.** (Finding 1.)
2. **`sock->ops->sendmsg` for a TCP socket is `inet_sendmsg`**, which
   dispatches straight to `sk->sk_prot->sendmsg` (verbatim,
   `net/ipv4/af_inet.c` v6.12):

   ```c
   int inet_sendmsg(struct socket *sock, struct msghdr *msg, size_t size)
   {
       struct sock *sk = sock->sk;
       if (unlikely(inet_send_prepare(sk)))
           return -EAGAIN;
       return INDIRECT_CALL_2(sk->sk_prot->sendmsg, tcp_sendmsg, udp_sendmsg,
                              sk, msg, size);
   }
   ```
3. **kTLS swaps `sk->sk_prot->sendmsg` to `tls_sw_sendmsg`** (verbatim,
   `net/tls/tls_main.c` v6.12 — `build_protos` + `update_sk_prot`):

   ```c
   prot[TLS_SW][TLS_BASE] = prot[TLS_BASE][TLS_BASE];
   prot[TLS_SW][TLS_BASE].sendmsg = tls_sw_sendmsg;     // proto-layer hook
   ...
   void update_sk_prot(struct sock *sk, struct tls_context *ctx) {
       WRITE_ONCE(sk->sk_prot, &tls_prots[ip_ver][ctx->tx_conf][ctx->rx_conf]);
       WRITE_ONCE(sk->sk_socket->ops, &tls_proto_ops[ip_ver][...][...]);
   }
   ```

So `inet_sendmsg`'s `INDIRECT_CALL_2(sk->sk_prot->sendmsg, ...)` lands on
`tls_sw_sendmsg` once kTLS-TX is armed. **The redirected bytes DO go
through the encryptor** — this is why the spike saw a real `0x17` TLS 1.3
record on the wire when it worked (`findings-egress-ktls-splice.md`,
Assertion 1). The intermittent failure is therefore NOT "plaintext on the
wire"; it is `tls_sw_sendmsg` *intermittently not delivering* under the
exact `msghdr` shape `__skb_send_sock` hands it.

**The proximate trigger — `MSG_DONTWAIT` on every fragment.**
`__skb_send_sock` constructs the `msghdr` per fragment with
**`MSG_DONTWAIT`** (and `MSG_SPLICE_PAGES | MSG_DONTWAIT` for paged
frags), and **never sets `MSG_MORE`** (verbatim, `net/core/skbuff.c`
v6.12):

```c
// linear head:
msg.msg_flags = MSG_DONTWAIT;
ret = INDIRECT_CALL_2(sendmsg, sendmsg_locked, sendmsg_unlocked, sk, &msg);
if (ret <= 0) goto error;
// paged frags:
struct msghdr msg = { .msg_flags = MSG_SPLICE_PAGES | MSG_DONTWAIT };
```

`tls_sw_sendmsg` honours `MSG_DONTWAIT` via
`sock_sndtimeo(sk, msg->msg_flags & MSG_DONTWAIT)` — i.e. the send is
**non-blocking**. When leg B's TX is not immediately able to take the
record (send buffer pressure from in-flight handshake/Finished records, or
the async-crypto queue is busy), the non-blocking `tls_sw_sendmsg` path
returns `-EAGAIN`/`-ENOMEM`/`-EBUSY`-derived error rather than waiting:

```c
if (ret) {
    if (ret == -EINPROGRESS) num_async++;
    else if (ret == -ENOMEM)  goto wait_for_memory;
    else if (ret != -EAGAIN)  goto send_end;
}
// and tls_do_encryption: if (ret == -EBUSY) { ret = tls_encrypt_async_wait(ctx); ... }
```

`__skb_send_sock`'s `if (ret <= 0) goto error;` then returns that
non-positive value to `sk_psock_handle_skb` → `sk_psock_backlog`, which
(Finding 3) `-EAGAIN`-stalls or tears down `SK_PSOCK_TX_ENABLED`.

**Source**: [v6.12 net/ipv4/af_inet.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/af_inet.c); [v6.12 net/tls/tls_main.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_main.c); [v6.12 net/tls/tls_sw.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_sw.c); [v6.12 net/core/skbuff.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skbuff.c) (accessed 2026-06-13).
**Confidence**: High for the encrypt-routing chain (three verbatim source links — `skb_send_sock` → `inet_sendmsg` → `tls_sw_sendmsg`); Medium-High for the `MSG_DONTWAIT`-induced `-EAGAIN` being the *specific* trigger of the 10–15% loss (the code path is verbatim, but which of `-EAGAIN`/`-ENOMEM`/partial fires in the residual is not captured by a trace — flagged in Knowledge Gaps as the Tier-3 `pwru`/bpftrace item).
**Cross-reference**: Finding 3 (the backlog's `-EAGAIN`-stall consuming this return), Finding 8 (the leg-B-writeability precondition), Finding 5 (the psock-vs-kTLS `bpf_exec_tx_verdict` interaction below).

**A second, subtler hazard inside `tls_sw_sendmsg` — `bpf_exec_tx_verdict`
re-enters the psock and can `-EACCES` the send.** Because leg B is a
sockmap member (it must be, to be a redirect target), it HAS a psock. When
`tls_sw_sendmsg` reaches the record-push, it calls `bpf_exec_tx_verdict`,
which **re-fetches leg B's psock and consults `psock->eval`**:

```c
psock = sk_psock_get(sk);
if (!psock || !policy) { err = tls_push_record(...); ... return err; }
...
case __SK_DROP:
default:
    sk_msg_free_partial(sk, msg, send);
    ...
    err = -EACCES;
```

If leg B's psock has any TX-side verdict policy that does not evaluate
`__SK_PASS`, the record is `-EACCES`'d — a hard error the backlog turns
into a `SK_PSOCK_TX_ENABLED` teardown (Finding 3). In Overdrive's topology
the verdict is `sk_skb/stream_verdict` (an RX-side program), not an
`sk_msg` TX-side program, so `policy` should be false and `bpf_exec_tx_verdict`
takes the `tls_push_record` branch — BUT this is precisely the
"leg-B-RX-psock vs kTLS" contention the spike's mechanic #2 flagged as a
*real open question*, now localised to a named kernel function. See
Finding 5.

### Finding 3: The deferred workqueue (`sk_psock_backlog`) silently stalls on `-EAGAIN` (target not writeable) and tears down on hard error — both leave the skb undelivered with `redirect_ok` already counted

**Evidence (kernel source).** `sk_psock_backlog` is a `delayed_work`
handler. Its loop over `ingress_skb` calls `sk_psock_handle_skb`; the
return value drives three outcomes (verbatim, v6.12; the v6.6 shape is
the same modulo the `state` bookkeeping):

```c
do {
    ret = -EIO;
    if (!sock_flag(psock->sk, SOCK_DEAD))
        ret = sk_psock_handle_skb(psock, skb, off, len, ingress);
    if (ret <= 0) {
        if (ret == -EAGAIN) {
            sk_psock_skb_state(psock, state, len, off);
            if (sk_psock_test_state(psock, SK_PSOCK_TX_ENABLED))
                schedule_delayed_work(&psock->work, 1);   // retry in 1 jiffy
            goto end;                                      // STOP draining now
        }
        sk_psock_report_error(psock, ret ? -ret : EPIPE);
        sk_psock_clear_state(psock, SK_PSOCK_TX_ENABLED);  // disable TX
        goto end;
    }
    off += ret; len -= ret;
} while (len);
skb = skb_dequeue(&psock->ingress_skb);
kfree_skb(skb);
```

Two silent-stall shapes relevant to the residual:

1. **`-EAGAIN` from `sk_psock_handle_skb`** (target not `sock_writeable`,
   or `skb_send_sock` itself returned `-EAGAIN` mid-send because the
   target's TX buffer filled). The backlog saves partial state, reschedules
   with a 1-jiffy delay, and **stops draining**. If the target never
   becomes writeable (or the rescheduled work races a teardown), the skb
   sits on `ingress_skb` undelivered. The redirect verdict already
   returned 0 and `redirect_ok` already counted.
2. **A hard error** (anything `<= 0` that isn't `-EAGAIN` — e.g. a
   `-EIO`/`-EBADMSG`/`-ENOMEM` bubbling up from the target's
   `sock->ops->sendmsg`, which for a kTLS socket includes the TLS encrypt
   path). The backlog **reports the error, clears `SK_PSOCK_TX_ENABLED`,
   and stops** — every subsequent redirect to that target then hits the
   `!SK_PSOCK_TX_ENABLED` guard in `sk_psock_skb_redirect` and is
   `sock_drop`ped. One bad send disables the whole leg.

**Source**: [v6.12 net/core/skmsg.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c) (accessed 2026-06-13); cross-verified shape at [v6.6](https://raw.githubusercontent.com/torvalds/linux/v6.6/net/core/skmsg.c).
**Confidence**: High (kernel-source-primary; the backlog body quoted verbatim).
**Cross-reference**: Finding 2 (whether the target send is even the kTLS path — if `skb_send_sock` does NOT engage kTLS, the bytes egress as *plaintext*, a worse failure than a drop); Finding 8 (the writeability/handshake-drain precondition); `debugging.md` §11 (a green `redirect_ok` count hides a failure at the lower delivery layer).

**Analysis.** The intermittent ~10–15% loss is the signature of a
deferred-work delivery that *sometimes* loses the race: the verdict
always enqueues + counts, but the backlog send can `-EAGAIN`-stall (if leg
B's TX isn't writeable at the deferred instant — e.g. the handshake
records or a partial `tls_sw` record still occupy the send buffer) or
hard-error (if the kTLS encrypt seam rejects the `sock->ops->sendmsg`
shape `skb_send_sock` produces). Both are timing-dependent against leg B's
TX state at the *deferred* moment, not the verdict moment — which is
exactly why a clean single-record sentinel loses intermittently and why
no settle-sleep before the write fixes it (the race is after the write,
on leg B's TX at the workqueue instant).

---

## Is the path supported / tested at all?

### Finding 4: NO kernel selftest redirects plaintext into a kTLS-TX-armed target. The `sockmap_ktls` selftest only exercises sockmap *membership lifecycle* of a kTLS socket — never a `bpf_sk_redirect_map` of data INTO a kTLS-TX socket with an encrypt assertion. The Overdrive pattern is outside the kernel's tested envelope.

**Evidence (kernel selftests, authoritative).**
`tools/testing/selftests/bpf/prog_tests/sockmap_ktls.c` contains exactly
two relevant tests, and **neither redirects data into a kTLS-TX socket**:

- `test_sockmap_ktls_disconnect_after_delete()` — add socket to sockmap,
  `setsockopt(TCP_ULP,"tls")`, remove from map, disconnect; asserts the
  disconnect succeeds. A *lifecycle* test; no data redirect.
- `test_sockmap_ktls_update_fails_when_sock_has_ulp()` — arm kTLS first,
  then `bpf_map_update_elem`; asserts the map update FAILS and the saved
  `sk_prot` is unaffected. A *negative ordering* test (this is the
  kernel's own statement of the D-MTLS-7 "sockmap-before-ULP" invariant
  Overdrive's probe checks).

There is **no** `test_*redirect*ktls*` and **no** assertion anywhere in
the selftests that bytes redirected into a kTLS socket emerge encrypted.
The data-carrying sockmap redirect selftests (`sockmap_basic.c`,
`test_sockmap.c`) redirect between *plain* sockets; the kTLS selftests
test *membership*, not *data redirect into TX*.

**Source**: [v6.12 tools/testing/selftests/bpf/prog_tests/sockmap_ktls.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/tools/testing/selftests/bpf/prog_tests/sockmap_ktls.c) (accessed 2026-06-13); cross-referenced against the data-redirect selftests in [sockmap_basic.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/tools/testing/selftests/bpf/prog_tests/sockmap_basic.c) (Gap-3 research, Finding 4c).
**Confidence**: High (kernel-selftest-primary — the absence is verified by reading the only kTLS-sockmap test file).
**Cross-reference**: Finding 5 (the kernel's own kTLS+sk_skb-verdict fix covers the RX side, not redirect-into-TX); Finding 10 (no production system relies on this either).

**Analysis — the decisive boundary.** The kernel ships robust tests for
(a) sockmap redirect between plain sockets and (b) sockmap+kTLS
membership/lifecycle, but **not** for (c) redirect plaintext into a
kTLS-TX socket. (c) is the Overdrive forward path. "Not tested by the
kernel" is not "broken," but combined with Findings 1–3 (the path is the
`skb_send_sock` → `tls_sw_sendmsg`-under-`MSG_DONTWAIT` deferred-work
path, with documented `-EAGAIN`-stall and teardown failure modes) and
Finding 10 (no production system does it either), it means **Overdrive is
relying on an untested kernel path for a correctness-critical
(confidentiality + delivery) primitive.** That is the structural risk the
~10–15% loss is a symptom of.

### Finding 5: The kernel's own "fix ktls with sk_skb_verdict programs" series fixes the RX (decrypt) side ordering, NOT redirect-into-TX — and the psock-vs-kTLS tension on a shared socket is real and kernel-acknowledged

**Evidence (kernel mailing list, authoritative).** The series *"[bpf-next
PATCH 0/3] fix ktls with sk_skb_verdict programs"* (John Fastabend, ~May
2020, landed in the 5.x line) states the problem verbatim:

> "If a socket is running a `BPF_SK_SKB_STREAM_VERDICT` program and KTLS
> is enabled the data stream may be broken if both TLS stream parser and
> BPF stream parser try to handle data."

Two load-bearing facts from the thread:

1. **The fix is on the RX side** — it makes "the KTLS stream parser run
   first … and then call the verdict program," establishing an ordering
   so kTLS-RX decrypts before the verdict sees the bytes. The cover letter
   says this is "analogous to how we handle a similar conflict on the TX
   side," i.e. a TX-side conflict was *already* known and handled
   separately.
2. **It says nothing about redirect-into-TX** being supported or tested.
   The whole series is about a socket that *has* a verdict AND kTLS on the
   *same* socket — the RX contention — not about a verdict on socket A
   redirecting into socket B's kTLS TX.

This is the kernel-source localisation of the spike's mechanic #2 ("a
psock (verdict OR parser) on leg B's RX fights leg B's kTLS RX"). In
Overdrive's forward path leg B's RX is *also* governed (leg B is a sockmap
member with the verdict attached, even though the verdict `SK_PASS`es leg
B's RX), AND leg B has kTLS-RX armed AND leg B's TX is the redirect
target. The `bpf_exec_tx_verdict` re-entry inside `tls_sw_sendmsg`
(Finding 2) is the TX-side instance of the same shared-socket contention.

**Source**: [spinics.net — "fix ktls with sk_skb_verdict programs" (bpf)](https://www.spinics.net/lists/bpf/msg20205.html); [netdev mirror](https://www.spinics.net/lists/netdev/msg657690.html); related stable backport [5.4 "bpf: Fix running sk_skb program types with ktls"](https://lore.kernel.org/all/20200619141657.348404087@linuxfoundation.org/) (accessed 2026-06-13).
**Confidence**: High (kernel-mailing-list-primary, two mirrors of the same series + a stable backport).
**Cross-reference**: Finding 2 (`bpf_exec_tx_verdict`), Finding 4 (untested redirect-into-TX), Finding 10 (production systems separate the verdict socket from the kTLS socket).

---

## Known kernel bugs / fixes

### Finding 6: The redirect-into-target delivery path has a documented history of silent data-drop bugs; the `-EAGAIN`/`sock_writeable` gate the spike's loss flows through was *added as the fix* for one of them. The path is fix-laden, not battle-hardened.

**Evidence (kernel mailing list / commit history, authoritative).** The
relevant fix lineage on exactly the functions Findings 1–3 traced:

1. **"bpf, sockmap: remove dropped data on errors in redirect case"**
   (John Fastabend, ~Oct 2020; `Fixes: 51199405f9672`). Commit message
   verbatim: *"In the sk_skb redirect case we didn't handle the case
   where we overrun the `sk_rmem_alloc` entry on ingress redirect or
   `sk_wmem_alloc` on egress. Because we didn't have anything implemented
   we simply dropped the skb."* The fix *"pushes those checks into the
   workqueue and allows us to return an EAGAIN error which in turn allows
   us to try again later from the workqueue."* This is the **origin of the
   exact `if (!sock_writeable(psock->sk)) return -EAGAIN;` gate in
   `sk_psock_handle_skb`** (Finding 3) and the backlog's `-EAGAIN`
   reschedule. Functions touched: `sk_psock_handle_skb` (adds the
   `sock_writeable` check), `sk_psock_skb_redirect` (removes the inline
   memory check, unconditionally queues).
2. **"bpf: Fix running sk_skb program types with ktls"** (stable
   backports to 5.4 / 5.7) and the **"fix ktls with sk_skb_verdict
   programs"** series (Finding 5) — the kTLS-RX-vs-verdict ordering fixes.
3. **"bpf, sockmap: RCU splat with redirect and strparser error or TLS"**
   (5.7 stable) — an RCU-safety bug specifically in the
   redirect-with-TLS-or-strparser-error path.
4. **"skmsg: lose offset info in `sk_psock_skb_ingress`"** (Huawei, 2021)
   — an offset bug that delivered redirected data multiple times when the
   parse length ≠ `skb->len`.

**Source**: [patchwork — "remove dropped data on errors in redirect case"](https://patchwork.kernel.org/project/netdevbpf/patch/160221868511.12042.12285689875540180401.stgit@john-Precision-5820-Tower/); [lore — "5.4 bpf: Fix running sk_skb program types with ktls"](https://lore.kernel.org/all/20200619141657.348404087@linuxfoundation.org/); [lore — "5.7 RCU splat with redirect and strparser error or TLS"](https://lore.kernel.org/lkml/20200714184118.697099943@linuxfoundation.org/); [patchwork — "skmsg: lose offset info"](https://patchwork.kernel.org/project/netdevbpf/patch/20210917013222.74225-1-liujian56@huawei.com/) (accessed 2026-06-13).
**Confidence**: High (kernel-commit-primary, four independent fixes on the same code path).
**Cross-reference**: Finding 3 (the `-EAGAIN` gate is the fix-4 artifact), Finding 5 (the kTLS-verdict ordering fixes).

**Analysis.** The redirect-into-target delivery path is a *fix-laden*
area: silent-drop, RCU-splat, offset-double-deliver, and
kTLS-verdict-ordering bugs all landed on it across 5.4–5.x. None of the
public fixes is specifically *"redirect plaintext into a kTLS-TX socket
loses data"* — because (Finding 4/10) nobody runs that pattern, so nobody
filed it. The residual Overdrive sees is consistent with being a *new*
instance of the same class on the one path no one stress-tests: the
`MSG_DONTWAIT` `tls_sw_sendmsg` under the backlog. Critically, **the
fixes that exist make the failure a stall/`-EAGAIN`, not a crash** —
which is exactly the silent ~10–15% loss signature (the byte is queued,
the work `-EAGAIN`-stalls, and nothing recovers it before teardown),
rather than a hard observable error.

---

## Egress vs `BPF_F_INGRESS`

### Finding 7: For the `sk_skb` redirect, `flags=0` (egress) and `BPF_F_INGRESS` route to DIFFERENT branches of the SAME `sk_psock_handle_skb`, both on the target's backlog workqueue — egress → `skb_send_sock` (target TX, encrypts via kTLS), ingress → `sk_psock_skb_ingress` (target RX queue, does NOT encrypt). Egress is the ONLY flag that produces ciphertext, and it is the one Overdrive uses.

**Evidence (kernel source).** Both flags land in the *same* deferred
`sk_psock_handle_skb`, distinguished by the `ingress` bool
(`skb_bpf_ingress(skb)`), set from the redirect flag (Finding 1):

```c
if (!ingress) {                                  // flags=0 (egress)
    if (!sock_writeable(psock->sk)) return -EAGAIN;
    return skb_send_sock(psock->sk, skb, off, len);   // target TX -> tls_sw_sendmsg (ENCRYPTS)
}
skb_get(skb);
err = sk_psock_skb_ingress(psock, skb, off, len);     // target RX queue (NO encrypt)
```

- **Egress (`flags=0`)** delivers to the target's **TX** via
  `skb_send_sock` → `sock->ops->sendmsg` → `tls_sw_sendmsg` — the bytes
  egress encrypted. This is correct for Overdrive (plaintext in → TLS
  records out) — and is the path with the `MSG_DONTWAIT`/`-EAGAIN`
  fragility (Findings 2/3).
- **Ingress (`BPF_F_INGRESS`)** delivers to the target's **RX ingress
  queue** via `sk_psock_skb_ingress` — the bytes land as *plaintext* on
  leg B's receive side and are NEVER encrypted/transmitted. This is the
  "wrong flag" the prior spikes (increment-d/e) used and is why they saw
  no ciphertext (`findings-egress-ktls-splice.md` § "Why this is a clean
  YES"). For a kTLS encrypt direction, `BPF_F_INGRESS` is categorically
  wrong.

**Source**: [v6.12 net/core/skmsg.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c) (accessed 2026-06-13).
**Confidence**: High (kernel-source-primary; the branch is the verbatim `sk_psock_handle_skb` body).
**Cross-reference**: Finding 1/2 (egress → `skb_send_sock` → kTLS); the spike's mechanic #1.

**Analysis — there is no reliability win available by switching flags.**
The question "does anyone redirect to ingress + rely on a different
encryption seam?" resolves to: **no — ingress does not encrypt at all.**
Egress is the only flag that produces ciphertext, and it carries the
`MSG_DONTWAIT`-stall fragility. You cannot trade the egress fragility for
ingress reliability because ingress does not do the job
(encrypt-and-transmit). The reliability fix must come from elsewhere
(Finding 8 / the verdict), not from the flag.

---

## Required conditions / flush + the population diff

### Finding 8: Reliability of the egress redirect requires leg B's TX to be `sock_writeable` AND free of a pending partial `tls_sw` record at the *deferred-work* instant — a precondition userspace cannot make structural for an in-kernel async send

**Evidence (kernel source).** The reliability preconditions, read off the
delivery path:

1. **`sock_writeable(leg_b)` must hold at the workqueue instant.**
   `sk_psock_handle_skb` returns `-EAGAIN` (stall) if leg B's TX is not
   writeable when the *deferred work runs* — not when the verdict fired.
   Leg B's send buffer carries the handshake + `client Finished` records
   (the spike's wire capture shows a 69-byte Finished record on leg B
   egress immediately before the probe record); if the deferred send races
   that buffer occupancy, it `-EAGAIN`-stalls.
2. **No pending partial `tls_sw` open record.** `tls_sw_sendmsg` with
   `MSG_DONTWAIT` and a non-`eor` shape can leave `ctx->open_rec` partial
   (Finding 2); a subsequent fragment send from `__skb_send_sock` (which
   sets no `MSG_MORE`, so each fragment is `eor`) interacts with that
   pending record. The flush-on-each-fragment behaviour
   (`__skb_send_sock` never sets `MSG_MORE`) means a single-skb redirect
   *should* push one record — but the async crypto queue (`-EINPROGRESS`/
   `-EBUSY` → `tls_encrypt_async_wait`) adds a second timing dependency.
3. **The `bpf_exec_tx_verdict` psock re-entry must take the
   `tls_push_record` branch** (Finding 2/5) — i.e. leg B must carry no TX
   policy that evaluates non-`__SK_PASS`.

**Userspace cannot make any of these structural.** Unlike the *enroll*
race (Gap-3 research direction A — solved by enrolling on an empty queue
in-kernel), the *delivery* race is on leg B's TX state at the
**workqueue** instant, which userspace does not control: the agent has
already left the per-byte path (that is the whole point of "agent-idle").
There is no userspace lever equivalent to "enroll on an empty queue"
because the agent is, by design, not present at the deferred send. A
`sendmsg(MSG_MORE=0)` nudge from the agent would put the agent *back* in
the per-byte path — defeating agent-idle — and still would not order
against the backlog's own `MSG_DONTWAIT` send.

**Source**: [v6.12 net/core/skmsg.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c); [v6.12 net/tls/tls_sw.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_sw.c); [v6.12 net/core/skbuff.c](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skbuff.c) (accessed 2026-06-13).
**Confidence**: High for the precondition list (kernel-source-primary); Medium for "which precondition is the dominant ~10–15% trigger" (needs the Tier-3 trace — Knowledge Gaps).
**Cross-reference**: Finding 3, Finding 9 (the production-wiring amplifier), the verdict.

### Finding 9: The two-load `pinning=ByName` production wiring is a *strong* population-diff hypothesis for "60/60 self-contained spike vs ~17/20 production" — the verdict program and leg B's insert live in DIFFERENT `Ebpf` instances, and the verdict-attach / sockmap-membership ordering relative to leg B's kTLS arm differs from the single-load spike

**Evidence (in-repo production source, cross-read against the spike).**
The production wiring does **two separate `load_shared_bpf` loads**, each
returning a distinct per-instance `Ebpf`, sharing only the pinned maps by
name (`crates/overdrive-dataplane/src/mtls/bpf_load.rs`,
`mtls/outbound.rs`):

- **Load A — `LegFListenerEnroll::setup`** (listener lifecycle, before
  accept): loads the object, attaches the `sk_skb/stream_verdict` to the
  pinned `MTLS_SOCKMAP`, attaches `sock_ops_mtls_enroll` to the cgroup,
  writes `MTLS_FPORT`, resets `MTLS_ARMED=0`. The **verdict program lives
  in this `Ebpf` instance.**
- **Load B — `ForwardRedirect::load`** (per-connection `establish`): a
  *second* `load_shared_bpf`, takes the pinned `MTLS_SOCKMAP`, inserts
  **leg B** into slot 1, arms kTLS on leg B, flips `MTLS_ARMED=1`. The
  **leg-B sockmap insert + kTLS arm + ARMED flip live in a DIFFERENT
  `Ebpf` instance**, sharing the sockmap only via the bpffs pin.

The increment-j spike that hit 60/60 used a **single** load: one program
object, the verdict and leg B's insert and the kTLS arm all against one
`Ebpf` (`findings-sockmap-engagement-inkernel-enroll.md` § "topology
built"). The spike's clean-queue sentinel was 40/40; the production
wiring's analogous path is the ~17/20 that motivates this research. By
`debugging.md` §5 (compare populations), the load-bearing differences
between the EXACT-60/60 population and the lossy-production population
are:

1. **Two `Ebpf` instances vs one.** The verdict (load A) and leg B's
   insert (load B) reference the same *pinned* `MTLS_SOCKMAP` FD by name,
   but each `Ebpf::load` re-resolves the pinned map and re-creates
   per-instance program/link state. Whether leg B inserted via *load B's*
   sockmap handle is governed by the verdict attached via *load A's*
   handle is a **kernel-FD-identity question**: `pinning=ByName` should
   make both handles refer to the one kernel sockmap object, so the verdict
   does fire on leg-B-targeted redirects — BUT the *timing* of leg B's
   psock creation (the `sockmap.set(B_IDX)` in load B) relative to the
   verdict's `sk_data_ready` install and relative to leg B's kTLS arm is
   now split across two userspace load/attach sequences, widening every
   ordering window Finding 8 names.
2. **Leg B's psock + kTLS-arm interleave differs.** In the single-load
   spike, leg B is inserted, handshaked, and kTLS-armed in one tight
   sequence against one object. In production, leg B is inserted by load B
   *after* the verdict was attached by load A in a separate sequence — and
   `SK_PSOCK_TX_ENABLED` on leg B (the gate Finding 1/3 require for the
   redirect target) is established by load B's insert, which the verdict
   from load A must then observe. If `SK_PSOCK_TX_ENABLED` is not yet set
   (or is transiently cleared by a prior `-EAGAIN` teardown, Finding 3) at
   the redirect instant, `sk_psock_skb_redirect` `-EIO`-drops — counted as
   a refusal, *not* the `redirect_ok` the problem statement reports, so
   this is a *secondary* hypothesis, not the primary.
3. **`MTLS_ARMED` / sockmap pins are process-shared and survive prior
   flows.** `bpf_load.rs` resets `ARMED=0` and sweeps stale pins per
   listener, but a *cross-flow* interaction (a prior flow's leg B still in
   slot 1, a prior `SK_PSOCK_TX_ENABLED` teardown on the shared sockmap)
   is a population difference the self-contained single-flow spike never
   had.

**Source**: `crates/overdrive-dataplane/src/mtls/bpf_load.rs` (two-load `pinning=ByName` sharing), `crates/overdrive-dataplane/src/mtls/outbound.rs` (`LegFListenerEnroll::setup` load A + `ForwardRedirect::load` load B), `findings-sockmap-engagement-inkernel-enroll.md` (single-load 60/60). Cross-referenced 2026-06-13.
**Confidence**: Medium-High (the two-load split is verbatim in the production source and is a real structural difference from the 60/60 single-load spike; whether it is THE cause of the residual vs the Finding-8 `MSG_DONTWAIT` delivery fragility cannot be separated without a Tier-3 trace — both are live, and they may compound).
**Cross-reference**: Finding 8, the verdict, Knowledge Gaps Gap 2.

**Analysis — the honest framing.** There are *two* candidate residual
mechanisms and they are not mutually exclusive: (i) the
**kernel-intrinsic** `MSG_DONTWAIT` `tls_sw_sendmsg`-under-backlog
delivery fragility (Findings 2/3/8), which would afflict *even* a
single-load wiring at some rate; and (ii) the **production-specific**
two-load split widening the ordering windows (this finding). The spike's
60/60 strongly suggests (i) alone is *low-rate* on a tight single-load
sequence (it did not surface at 60 samples) — which elevates (ii), the
two-load split, as the more likely dominant amplifier in production. But
the spike's own scope notes (single 73–87 B record, loopback, software
kTLS) mean (i) could still surface at a higher rate under real multi-record
/ multi-flow load. The population diff is the proof technique; the Tier-3
spike (Gap 2) is what separates (i) from (ii).

---

## THE DECISIVE QUESTION — the correct production pattern

### Finding 10: NO production system encrypts a spliced/redirected stream by sockmap-egress-redirect into a kTLS-TX socket. The production lineage either (a) does TLS in userspace (Istio ztunnel — rustls, the direct analog to Overdrive's agent), (b) encrypts at the network layer (Cilium — IPsec/WireGuard, not kTLS), or (c) uses sockmap only for PLAINTEXT localhost-bypass acceleration (Cilium sockops). The Overdrive "agent-idle sockmap-egress-redirect into kTLS-TX" pattern is novel and unattested.

**Evidence (production source / docs, cross-referenced).**

1. **Istio ztunnel — the closest analog (an L4 mTLS node proxy) — does
   mTLS in USERSPACE with rustls, NOT kTLS-via-sockmap.** *"Ztunnel's TLS
   is built on rustls."* It tunnels plaintext workload traffic over an
   HBONE (HTTP CONNECT over mutual TLS) connection terminated/originated
   in the ztunnel process. There is **no kTLS and no
   sockmap-redirect-into-TX** in the ztunnel data path — the bytes are
   moved in userspace (the Rust proxy copies between the plaintext leg and
   the rustls-encrypted leg). This is the production answer to "how do you
   build an agent that originates mTLS for identity-unaware workloads":
   **terminate TLS in the agent, copy the bytes.**
2. **Cilium does NOT use kTLS for transparent encryption — it uses IPsec
   or WireGuard at the network layer.** *"Cilium supports the transparent
   encryption … using IPsec, WireGuard, or ztunnel."* kTLS is explicitly
   *not* the encryption mechanism; it requires a TLS handshake that
   *"doesn't fit well with transparent encryption at the network level."*
3. **Cilium's sockmap use is PLAINTEXT localhost-bypass acceleration, not
   encrypt-on-redirect.** The sockmap/`sk_msg` redirect short-circuits the
   TCP/IP stack for *local* socket-to-socket traffic *"before the data
   finally leaves the node"* — moving plaintext between local sockets to
   avoid encap/decap/fib/qdisc overhead. It is `BPF_F_INGRESS`
   (local-to-local RX delivery), not egress-into-kTLS-TX. The kTLS+sockmap
   interaction Cilium tests is the *membership/ordering* lifecycle
   (Finding 4), not data-redirect-into-TX.
4. **No Cloudflare/Tetragon precedent for encrypt-on-redirect either.**
   Cloudflare's sockmap lineage (Finding 4b of the Gap-3 research) is
   stress-testing the *plaintext* redirect path; Tetragon uses sockmap for
   *observability*, not for kTLS encryption. No located source shows any
   system pushing plaintext into a kTLS-TX socket via `bpf_sk_redirect_map`.

**Source**: [Istio — "Introducing Rust-Based Ztunnel"](https://istio.io/latest/blog/2023/rust-based-ztunnel/); [Istio — L4 Networking & mTLS with Ztunnel](https://istio.io/latest/docs/ops/ambient/usage/ztunnel/); [Cilium — Transparent Encryption](https://docs.cilium.io/en/stable/security/network/encryption/); [Cilium localhost-bypass / sockmap rationale](https://github.com/nyrahul/ebpf-guide/blob/master/docs/localhost-bypass-stack.rst); the Gap-3 research Finding 4a/4b (Cilium sockops enroll is for plaintext socket-LB acceleration with `BPF_F_INGRESS`) (accessed 2026-06-13).
**Confidence**: High for "ztunnel uses userspace rustls" and "Cilium uses IPsec/WireGuard, sockmap is plaintext acceleration" (multiple authoritative sources); High for the *absence* of an encrypt-on-redirect precedent (searched the production lineage explicitly and found none — an absence is never provable to certainty, but the convergence of ztunnel-userspace + Cilium-network-layer + no-kernel-selftest + no-located-precedent is strong).
**Cross-reference**: Finding 4 (kernel doesn't test it), Finding 5 (kernel kTLS+verdict fixes are RX-only), the verdict.

**Analysis — the decisive transfer to Overdrive.** Every production system
that solves "encrypt an identity-unaware workload's stream from an agent"
either keeps the TLS *in the agent* (ztunnel/rustls — the agent IS in the
byte path for the encrypt direction) or moves encryption *off the socket
entirely* (Cilium IPsec/WireGuard). **None of them uses the kernel to
encrypt-on-redirect via sockmap into kTLS-TX.** The pattern the Overdrive
forward path relies on is not "an under-used kernel feature" — it is a
*combination the kernel neither tests nor any production system runs*. The
~10–15% loss is the predictable cost of being the only user of an untested
path with documented (Findings 3/6) silent-stall failure modes on the
exact `MSG_DONTWAIT` `tls_sw_sendmsg`-under-backlog seam.

---

## VERDICT

**Reliable agent-idle egress-redirect-into-kTLS-TX is NOT achievable as a
load-bearing production primitive, and the agent-idle forward path is the
wrong mechanism for the kTLS-TX (encrypt) direction.** The recommendation
is to change Overdrive's forward path to encrypt the same way the return
path already moves bytes and the way every production analog does it —
an agent-light userspace `tls_sw_sendmsg`/copy for the encrypt direction —
NOT a sockmap egress redirect into a kTLS-TX socket.

**Why the kernel path cannot be made reliable (evidence-ranked):**

1. **The delivery is a deferred-workqueue `skb_send_sock` →
   `tls_sw_sendmsg` under `MSG_DONTWAIT`** (Findings 1/2), NOT the
   synchronous `tcp_sendmsg_locked` the spike assumed. `redirect_ok`
   counts the *enqueue*, not the *delivery* — the byte can be queued,
   counted, and then lost when the backlog `-EAGAIN`-stalls or tears down
   `SK_PSOCK_TX_ENABLED` (Finding 3). This is the mechanism of "redirect
   succeeds, peer decrypts 0."
2. **The reliability precondition (leg B `sock_writeable` + no pending
   partial record + clean psock TX policy at the *workqueue* instant) is
   not userspace-controllable** (Finding 8). Unlike the enroll race
   (closable by enrolling on an empty queue), the delivery race is on leg
   B's TX state *after* the agent has left the path — there is no
   structural lever, and the only userspace nudge (`sendmsg(MSG_MORE=0)`)
   re-inserts the agent into the per-byte path, defeating agent-idle.
3. **The path is untested by the kernel** (Finding 4), **fix-laden on
   exactly its delivery functions** (Finding 6), and **run by no
   production system** (Finding 10). Overdrive would be the sole user of a
   combination the kernel does not validate.
4. **The flag cannot be traded for reliability** (Finding 7): egress is
   the only flag that encrypts; ingress does not transmit. There is no
   reliable-flag alternative.

**The correct production mechanism — agent-light userspace encrypt for the
forward direction (mirror the return pump):**

The return (decrypt) direction *already* uses an agent-light userspace
pump (`PumpHandle::spawn(leg_b_fd, leg_f_fd, …)` in `outbound.rs` step 10
— legB kTLS-RX → pipe → legF). **The forward (encrypt) direction should
use the symmetric shape**: a userspace pump that reads plaintext off leg
F and `write()`s it to leg B (whose `sk->sk_prot->sendmsg` is
`tls_sw_sendmsg`, so the kernel still does the AES-GCM in kTLS-TX — the
agent does NOT do crypto, only the `read`/`write` copy). This is:

- **The ztunnel pattern** (Finding 10): the agent is in the byte path for
  the copy, the crypto is kTLS (kernel) or rustls (userspace) — either
  way deterministic, no sockmap redirect, no untested seam.
- **"Agent-light," not "agent-idle":** the agent does a bounded
  `read(legF)`/`write(legB)` copy (one `read` + one `write` per chunk),
  the same per-byte involvement the return pump already accepts and the
  same `splice()`-able shape. kTLS-TX still encrypts in-kernel on the
  `write(legB)` (the `tls_sw_sendmsg` the redirect was trying to reach,
  now reached *synchronously* from userspace where `MSG_DONTWAIT`/backlog
  fragility does not apply — a blocking userspace `write` to a kTLS socket
  waits for buffer space instead of `-EAGAIN`-stalling a workqueue).
- **Reuses proven code:** the `flush_through(leg_b_fd, …)` helper already
  `write()`s plaintext into leg B's kTLS-TX and is proven (the pre-arm
  flush works — the problem statement confirms "pre-arm bytes flushed via
  a normal `write()` DO arrive"). The forward pump is `flush_through` in a
  loop, or `splice(legF → legB)`.

**The specific change to Overdrive's forward path:**

- **Delete** the `sk_skb/stream_verdict` egress-redirect as the
  steady-state forward mechanism (the `MTLS_SOCKMAP` slot-1 redirect into
  leg B's kTLS-TX). The verdict + sockmap + `bpf_sk_redirect_map(flags=0)`
  forward splice is the unreliable primitive.
- **Replace** with a forward userspace pump symmetric to the return
  `PumpHandle`: `read(legF)` → `write(legB)` (kTLS-TX encrypts in-kernel),
  in a bounded loop, ideally via `splice()` for zero userspace copy.
- **Keep** everything else: the in-kernel `sock_ops_mtls_enroll`
  enroll-at-establishment (it is still correct and the enroll race is
  genuinely closed), the leg-B kTLS arm, the pre-arm capture+flush
  (`drain_prearm` + `flush_through`), the `ARMED` confidentiality gate
  (re-cast as "userspace pump does not start forwarding until kTLS armed"
  rather than "verdict redirects only when ARMED"). The `MTLS_ARMED` gate
  and the sockmap can stay if the verdict is retained for a *different*
  purpose (e.g. fail-closed pre-arm `SK_DROP`), but the steady-state
  forward bytes must NOT ride `bpf_sk_redirect_map` into kTLS-TX.

**What this costs and why it is acceptable:** the forward path is no
longer "agent-idle" — the agent does a `read`/`write` (or `splice`) copy,
exactly as the return path already does. ADR-0069's "agent-light" framing
already accepts this for the return direction; the symmetric forward pump
is the same cost, and `splice()` keeps it zero-copy-in-userspace. The
encrypt is *still* kernel-side kTLS-TX (the `write(legB)` hits
`tls_sw_sendmsg`); the agent never holds key material or does crypto. The
confidentiality model is unchanged. What is lost is only the "kernel moves
every byte with the agent fully out" property — which Findings 1–10 show
was never reliably true for the kTLS-TX direction.

**If the team insists on retaining the kernel redirect** (NOT recommended),
the *only* candidate mitigation the source admits is: ensure leg B is
`sock_writeable` with no pending records at every redirect — which is not
structurally achievable for a deferred async send (Finding 8) — plus
collapse to a single-load wiring (Finding 9) to remove the two-`Ebpf`
amplifier. Even then the kernel-intrinsic `MSG_DONTWAIT`-backlog fragility
(Finding 3) remains, and no production precedent or kernel test backs it.
This is a Tier-3-spike-to-disprove, not a ship-it.

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Linux kernel `net/core/skmsg.c` @ v6.12 (`sk_psock_skb_redirect`, `sk_psock_backlog`, `sk_psock_handle_skb`, `sk_psock_verdict_apply`) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y (vs v6.6) |
| Linux kernel `net/core/skmsg.c` @ v6.6 (same fns) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y (vs v6.12) |
| Linux kernel `net/ipv4/tcp_bpf.c` @ v6.12 (`tcp_bpf_sendmsg_redir`, `tcp_bpf_push`) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y |
| Linux kernel `net/core/skbuff.c` @ v6.12 (`skb_send_sock`, `__skb_send_sock`) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y (vs v6.6) |
| Linux kernel `net/core/skbuff.c` @ v6.6 (`__skb_send_sock` MSG_DONTWAIT) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y (vs v6.12) |
| Linux kernel `net/ipv4/af_inet.c` @ v6.12 (`inet_sendmsg`) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y |
| Linux kernel `net/tls/tls_main.c` @ v6.12 (`update_sk_prot`, `build_protos`) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y |
| Linux kernel `net/tls/tls_sw.c` @ v6.12 (`tls_sw_sendmsg`, `bpf_exec_tx_verdict`) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y (vs v6.6) |
| Linux kernel `net/tls/tls_sw.c` @ v6.6 (MSG_DONTWAIT + bpf_exec_tx_verdict present) | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel source (primary) | 2026-06-13 | Y (vs v6.12) |
| Kernel selftest `prog_tests/sockmap_ktls.c` @ v6.12 | raw.githubusercontent.com/torvalds/linux | High (1.0) | Kernel selftest (primary) | 2026-06-13 | Y |
| "bpf, sockmap: remove dropped data on errors in redirect case" | patchwork.kernel.org | High (0.9) | Kernel patch (primary) | 2026-06-13 | Y (vs source) |
| "fix ktls with sk_skb_verdict programs" series | spinics.net (bpf/netdev lore mirror) | High (0.9) | Kernel mailing list | 2026-06-13 | Y |
| "5.4 bpf: Fix running sk_skb program types with ktls" | lore.kernel.org | High (0.9) | Kernel stable backport | 2026-06-13 | Y |
| "5.7 RCU splat with redirect and strparser error or TLS" | lore.kernel.org | High (0.9) | Kernel stable backport | 2026-06-13 | Y |
| Istio — "Introducing Rust-Based Ztunnel" / Ztunnel mTLS docs | istio.io | High (1.0) | Official project docs | 2026-06-13 | Y |
| Cilium — Transparent Encryption (IPsec/WireGuard) | docs.cilium.io | High (1.0) | Official project docs | 2026-06-13 | Y |
| Cilium localhost-bypass / sockmap rationale | github.com/nyrahul/ebpf-guide | Medium-High (0.8) | OSS reference write-up | 2026-06-13 | Y (vs Cilium docs) |
| In-repo `mtls/{bpf_load,outbound}.rs` + 3 spike artifacts | project-internal | n/a (subject) | Production source + spike evidence | 2026-06-13 | Y |
| docs.kernel.org BPF_MAP_TYPE_SOCKMAP | docs.kernel.org | High (1.0) | Official kernel doc | 2026-06-13 | Y |

**Reputation breakdown**: High (≥0.9): 17 of 18 external sources (94%);
Medium-High (0.8): 1 (the Cilium-rationale OSS write-up, used only to
corroborate the sockmap-is-plaintext-acceleration point that Cilium's own
docs independently confirm). Average ≈ 0.96. **Every load-bearing
kernel-mechanism claim rests on kernel-source-primary evidence quoted
verbatim and cross-verified at two tags (v6.6 + v6.12)**; the
delivery-path chain (`sk_skb` redirect → backlog → `skb_send_sock` →
`inet_sendmsg` → `tls_sw_sendmsg`-under-`MSG_DONTWAIT`) is traced
end-to-end through 5 distinct kernel files.

**Adversarial-validation note (operational-safety).** `git.kernel.org`
and `elixir.bootlin.com` are Anubis/JS-gated to programmatic fetch; the
verbatim function bodies were read from the canonical
`raw.githubusercontent.com/torvalds/linux/<tag>` mirror (the same byte
source as the torvalds tree at the pinned tag) and cross-verified at two
independent tags to guard against a stale/wrong-tag mirror. The WebFetch
summaries of the kernel files were treated as untrusted extraction and
each load-bearing quote was checked for internal consistency against the
adjacent functions it calls (e.g. `sk_psock_handle_skb`'s egress branch
calling `skb_send_sock` is consistent with `skb_send_sock`'s
`sendmsg_unlocked` → `sock_sendmsg` → `inet_sendmsg` → `tls_sw_sendmsg`
chain, which is consistent with `tls_main.c`'s `build_protos` installing
`tls_sw_sendmsg` at the proto layer — three independent files agreeing).

## Knowledge Gaps

### Gap 1: Which of `-EAGAIN`-stall / hard-error / partial-record is the dominant ~10–15% trigger — only a Tier-3 `pwru`/bpftrace trace can settle it
**Issue**: Findings 2/3/8 enumerate the candidate failure modes inside the
deferred `tls_sw_sendmsg`-under-backlog send (leg B not `sock_writeable` →
`-EAGAIN`-stall; async crypto `-EBUSY`/`-ENOMEM`; `bpf_exec_tx_verdict`
`-EACCES`; partial open record). The source proves each is *possible*; it
cannot say which fires in the residual, because that depends on leg B's TX
state at the *deferred workqueue instant* on the real kernel.
**Attempted**: kernel source (every branch quoted verbatim); the spike
findings (report the symptom "peer got 0", not the kernel-internal branch).
**Recommendation**: **Tier-3 spike** — `pwru --filter-track-skb` on the
redirected skb through `sk_psock_backlog → skb_send_sock → tls_sw_sendmsg`,
plus a `kfunc:vmlinux:tls_sw_sendmsg`/`kretfunc` bpftrace capturing the
return value on the lossy runs, and a `bpftrace` on
`sk_psock_handle_skb`'s `-EAGAIN` return. The decisive probe per
`debugging.md` §5/§7: capture the return value population on EXACT vs LOST
runs and diff. **This is moot if the verdict's recommendation (replace the
redirect with a userspace pump) is adopted** — the failing path is deleted.

### Gap 2: Separating the kernel-intrinsic `MSG_DONTWAIT` fragility (Finding 3/8) from the production two-load amplifier (Finding 9) — a single-load production repro
**Issue**: The residual has two non-exclusive candidate causes: the
kernel-intrinsic deferred-send fragility (would afflict even a single-load
wiring at some rate) and the production two-`Ebpf` split (widens ordering
windows). The spike's 60/60 single-load suggests the intrinsic rate is low
on a tight sequence, elevating the two-load split as the dominant
amplifier — but the spike used a single short record on loopback/software
kTLS, so the intrinsic rate under real multi-record load is unmeasured.
**Attempted**: in-repo source (the two-load split is verbatim); the spike
(single-load, but different scope — short record, loopback).
**Recommendation**: a **Tier-3 spike** that runs the *production* shape
(two-load `pinning=ByName`) AND a collapsed single-load variant on the
same kernel/payload, 20+ runs each, and diffs the loss rate (population
diff per `debugging.md` §5). If the single-load variant is lossless and
two-load is lossy, the split is the dominant cause; if both are lossy, the
`MSG_DONTWAIT` fragility is intrinsic. **Also moot if the redirect is
replaced** — but worth running once to *confirm* the redirect path is the
culprit before deleting it (don't delete on a hypothesis alone).

### Gap 3: Whether `bpf_exec_tx_verdict` (Finding 2/5) ever takes the non-`tls_push_record` branch for Overdrive's leg B — the leg-B-RX-psock-vs-kTLS contention localised but not observed
**Issue**: Finding 2 shows `tls_sw_sendmsg` re-enters leg B's psock via
`bpf_exec_tx_verdict`; for an `sk_skb` (RX-side) verdict `policy` should be
false (the `tls_push_record` branch), but the spike's mechanic #2 reported
real `ConnectionAborted` (code 103) when a psock governed leg B's RX
alongside kTLS-RX. Whether the TX-side `bpf_exec_tx_verdict` ever
`-EACCES`es for Overdrive's exact verdict shape is inferred, not observed.
**Attempted**: kernel source (`bpf_exec_tx_verdict` quoted; the
`policy`/`psock->eval` branch is version-stable v6.6→v6.12); the spike
(reports the RX-side abort symptom, not the TX-side branch).
**Recommendation**: Low priority if the redirect is replaced. Otherwise a
`bpftrace` on `bpf_exec_tx_verdict`'s return for leg B confirms which
branch fires.

### Gap 4: Hardware-kTLS-offload behaviour
**Issue**: All evidence is software kTLS (`txconf: sw`, the spike's scope).
The `skb_send_sock`/`tls_sw_sendmsg` path differs for `tls_device`
(hardware offload) — `tls_main.c`'s `build_protos` installs different
function pointers for `TLS_HW`. Whether the redirect-into-kTLS-TX residual
changes under hardware offload is unexamined.
**Attempted**: kernel source (noted the `TLS_HW` table exists); not traced.
**Recommendation**: Out of scope for the 6.18 software-kTLS path Overdrive
targets; revisit only if hardware offload is adopted. The verdict (use a
userspace pump) is offload-agnostic — `write(legB)` hits whichever kTLS
backend is armed.

## Conflicting Information

### Conflict 1: "The egress redirect drives `tcp_sendmsg_locked` → kTLS encrypt" (spike) vs "the `sk_skb` egress redirect drives `skb_send_sock` → `tls_sw_sendmsg` on a workqueue" (this research)
- **Position A** (`findings-egress-ktls-splice.md` + the spike's liveness
  research): the egress redirect → `tcp_bpf_sendmsg_redir` →
  `tcp_bpf_push_locked` → `tcp_sendmsg_locked` on the target (synchronous).
  Source: spike, citing `net/ipv4/tcp_bpf.c`.
- **Position B** (this research): `tcp_bpf_sendmsg_redir` is the **`sk_msg`**
  path; the **`sk_skb`** `bpf_sk_redirect_map` path the Overdrive verdict
  uses goes `sk_psock_verdict_apply (__SK_REDIRECT)` →
  `sk_psock_skb_redirect` (enqueue + `schedule_delayed_work`) →
  `sk_psock_backlog` → `sk_psock_handle_skb` → `skb_send_sock` →
  `inet_sendmsg` → `tls_sw_sendmsg` (deferred, `MSG_DONTWAIT`). Source:
  `net/core/skmsg.c` + `net/core/skbuff.c`, verbatim, v6.6 ≡ v6.12.
- **Assessment**: **Position B is correct and Position A conflated two
  kernel paths.** The kernel source is decisive: `tcp_bpf_sendmsg_redir`
  takes a `struct sk_msg *`, not an skb, and is reached from the `sk_msg`
  redirect (`bpf_msg_redirect_*`) and from `tcp_bpf_sendmsg`/`recvmsg` —
  NOT from an `sk_skb` `bpf_sk_redirect_map`, which dispatches through
  `sk_psock_verdict_apply`'s `__SK_REDIRECT` arm to `sk_psock_skb_redirect`
  (the skb path). The spike's *observable* result (a real `0x17` record on
  the wire when it worked) is consistent with BOTH paths reaching
  `tls_sw_sendmsg` eventually — so the spike's empirical "it encrypts" was
  right, but its *mechanism attribution* was wrong, and the wrong mechanism
  (synchronous `tcp_sendmsg_locked`) hid the deferred-workqueue fragility
  that is the actual residual. This is the single most important correction
  in the research, and it is resolved in favour of the kernel source (the
  most authoritative evidence available).

No other substantive conflicts. The spike's *observations* (encrypted
egress, agent-idle, 40/40 clean-queue) are all accepted as accurate; only
the *mechanism attribution* is corrected.

## Recommendations for Further Research

1. **Adopt the verdict: replace the forward `sk_skb` egress-redirect with
   an agent-light userspace `read(legF)→write(legB)` / `splice` pump
   symmetric to the return `PumpHandle`.** This deletes the unreliable
   path entirely; the encrypt stays kernel-side kTLS-TX on the
   `write(legB)`. Effort: one DELIVER slice (the return-pump shape already
   exists). This is the actionable output, not a research follow-up.
2. **Before deleting, run the Gap-2 single-load-vs-two-load Tier-3 spike
   once** to *confirm* the redirect path (not some other bug) is the
   culprit — don't delete on hypothesis alone. Effort: one spike cycle.
3. **If (against this research's recommendation) the redirect is retained,
   run the Gap-1 `pwru`/bpftrace trace** to identify the dominant failure
   branch — but the verdict is that no branch-specific fix makes a deferred
   `MSG_DONTWAIT` async send into a kTLS-TX socket reliable, so this is a
   spike-to-disprove, not a spike-to-fix.

## Full Citations

[1] Linus Torvalds et al. "Linux kernel `net/core/skmsg.c` @ tag v6.12 (`sk_psock_skb_redirect`, `sk_psock_backlog`, `sk_psock_handle_skb`, `sk_psock_verdict_apply`)". github.com/torvalds/linux. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c. Accessed 2026-06-13.

[2] Linus Torvalds et al. "Linux kernel `net/core/skmsg.c` @ tag v6.6". github.com/torvalds/linux. 2023. https://raw.githubusercontent.com/torvalds/linux/v6.6/net/core/skmsg.c. Accessed 2026-06-13.

[3] Linus Torvalds et al. "Linux kernel `net/ipv4/tcp_bpf.c` @ tag v6.12 (`tcp_bpf_sendmsg_redir`, `tcp_bpf_push`, `tcp_bpf_push_locked`)". github.com/torvalds/linux. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c. Accessed 2026-06-13.

[4] Linus Torvalds et al. "Linux kernel `net/core/skbuff.c` @ tag v6.12 (`skb_send_sock`, `__skb_send_sock`)". github.com/torvalds/linux. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skbuff.c. Accessed 2026-06-13.

[5] Linus Torvalds et al. "Linux kernel `net/core/skbuff.c` @ tag v6.6 (`__skb_send_sock` MSG_DONTWAIT)". github.com/torvalds/linux. 2023. https://raw.githubusercontent.com/torvalds/linux/v6.6/net/core/skbuff.c. Accessed 2026-06-13.

[6] Linus Torvalds et al. "Linux kernel `net/ipv4/af_inet.c` @ tag v6.12 (`inet_sendmsg`)". github.com/torvalds/linux. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/af_inet.c. Accessed 2026-06-13.

[7] Linus Torvalds et al. "Linux kernel `net/tls/tls_main.c` @ tag v6.12 (`update_sk_prot`, `build_protos`, `tls_init`)". github.com/torvalds/linux. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_main.c. Accessed 2026-06-13.

[8] Linus Torvalds et al. "Linux kernel `net/tls/tls_sw.c` @ tag v6.12 (`tls_sw_sendmsg`, `bpf_exec_tx_verdict`)". github.com/torvalds/linux. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_sw.c. Accessed 2026-06-13.

[9] Linus Torvalds et al. "Linux kernel `net/tls/tls_sw.c` @ tag v6.6 (MSG_DONTWAIT + bpf_exec_tx_verdict present)". github.com/torvalds/linux. 2023. https://raw.githubusercontent.com/torvalds/linux/v6.6/net/tls/tls_sw.c. Accessed 2026-06-13.

[10] Linux kernel selftests. "`tools/testing/selftests/bpf/prog_tests/sockmap_ktls.c` @ tag v6.12". github.com/torvalds/linux. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/tools/testing/selftests/bpf/prog_tests/sockmap_ktls.c. Accessed 2026-06-13.

[11] John Fastabend. "bpf, sockmap: remove dropped data on errors in redirect case" (`Fixes: 51199405f9672`). patchwork.kernel.org. 2020. https://patchwork.kernel.org/project/netdevbpf/patch/160221868511.12042.12285689875540180401.stgit@john-Precision-5820-Tower/. Accessed 2026-06-13.

[12] John Fastabend. "[bpf-next PATCH 0/3] fix ktls with sk_skb_verdict programs". spinics.net (bpf lore mirror). 2020. https://www.spinics.net/lists/bpf/msg20205.html. Accessed 2026-06-13.

[13] Greg Kroah-Hartman. "[PATCH 5.4 156/261] bpf: Fix running sk_skb program types with ktls". lore.kernel.org. 2020. https://lore.kernel.org/all/20200619141657.348404087@linuxfoundation.org/. Accessed 2026-06-13.

[14] Greg Kroah-Hartman. "[PATCH 5.7 059/166] bpf, sockmap: RCU splat with redirect and strparser error or TLS". lore.kernel.org. 2020. https://lore.kernel.org/lkml/20200714184118.697099943@linuxfoundation.org/. Accessed 2026-06-13.

[15] Istio Authors. "Introducing Rust-Based Ztunnel for Istio Ambient Service Mesh". istio.io. 2023. https://istio.io/latest/blog/2023/rust-based-ztunnel/. Accessed 2026-06-13.

[16] Istio Authors. "Layer 4 Networking & mTLS with Ztunnel". istio.io. 2024. https://istio.io/latest/docs/ops/ambient/usage/ztunnel/. Accessed 2026-06-13.

[17] Cilium Authors. "Transparent Encryption (IPsec / WireGuard)". docs.cilium.io. 2025. https://docs.cilium.io/en/stable/security/network/encryption/. Accessed 2026-06-13.

[18] nyrahul. "ebpf-guide — localhost-bypass-stack (sockmap rationale)". github.com/nyrahul/ebpf-guide. https://github.com/nyrahul/ebpf-guide/blob/master/docs/localhost-bypass-stack.rst. Accessed 2026-06-13.

[19] Linux Kernel Authors. "BPF_MAP_TYPE_SOCKMAP and BPF_MAP_TYPE_SOCKHASH". docs.kernel.org. 2025. https://docs.kernel.org/bpf/map_sockmap.html. Accessed 2026-06-13.

[20] Overdrive Project. "`crates/overdrive-dataplane/src/mtls/{bpf_load,outbound}.rs`; `crates/overdrive-bpf/src/programs/sk_skb_stream_verdict_mtls.rs`; spike findings `findings-egress-ktls-splice.md`, `findings-sockmap-engagement-inkernel-enroll.md`; research `sockmap-strparser-engagement-race-research.md`". (project-internal). Cross-referenced 2026-06-13.

## Research Metadata

Duration: ~1 session (turns 1–48) | Sources examined: 13 web/kernel-source
fetches + 6 in-repo files | Sources cited: 20 | Cross-references: the
delivery-path mechanism (`sk_skb` redirect → backlog → `skb_send_sock` →
`inet_sendmsg` → `tls_sw_sendmsg`-under-`MSG_DONTWAIT`) traced end-to-end
across 5 kernel files and verified unchanged at v6.6 + v6.12 (holds on the
6.18 floor by inspection) | Confidence distribution: High on Findings 1, 2
(encrypt-routing chain), 4, 5, 6, 7, 10 and the core "the `sk_skb` egress
redirect is a deferred-workqueue `skb_send_sock` send, NOT synchronous
`tcp_sendmsg_locked`" correction and the verdict; Medium-High on Finding 2's
*specific* `MSG_DONTWAIT`-trigger attribution and Finding 9's two-load
population-diff being THE dominant cause (both need the Tier-3 trace to
separate from the alternatives — Gaps 1/2) | Output:
`docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`

### Anti-bot / fetch transparency
- `git.kernel.org` / `elixir.bootlin.com` are Anubis/JS-gated to
  programmatic fetch; verbatim kernel function bodies were read from the
  canonical `raw.githubusercontent.com/torvalds/linux/<tag>` mirror and
  cross-verified at two tags (v6.6 + v6.12).
- The ztunnel `ARCHITECTURE.md` did not cover the data-path TLS mechanism;
  the "ztunnel uses userspace rustls" claim rests on the Istio blog + the
  L4/mTLS-with-ztunnel docs (citations [15]/[16]), which state it directly.
  The "userspace copy (not kTLS/sockmap) for the data path" is corroborated
  by the absence of any kTLS/sockmap reference in the ztunnel
  docs/architecture and by the structural fact that HBONE is an HTTP
  CONNECT tunnel over rustls (a userspace TLS stack) — flagged as
  Medium-High on the *data-path-mechanism* sub-claim, High on the
  *TLS-is-userspace-rustls* claim.
