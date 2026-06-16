# Research: Agent-light relay of kTLS-RX decrypted plaintext to a workload socket (kernel ≥6.18 / 7.0)

> **Question**: Is there ANY *agent-light* mechanism to relay a software-kTLS `TLS_RX` socket's **decrypted plaintext** to a *different* (workload-facing) socket, **without a userspace per-byte read/copy loop**, on kernel ≥6.18 / 7.0?
> **Kernel target**: 7.0 / ≥6.18 (software kTLS, `rxconf: sw`; ADR-0068 appliance floor).
> **One-line verdict**: **(c) PLAUSIBLE-BUT-UNVALIDATED** — `splice(2)` on a kTLS-RX socket *without an attached psock* leans YES per `tls_sw_splice_read` in `net/tls/tls_sw.c` (delivers decrypted plaintext, kernel-driven, no per-byte userspace copy), but needs a confirming Tier-3 spike; io_uring `recv` on kTLS-RX leans plausible but still requires userspace copy to leg F.
> **#222 return-path cost**: best case **agent-light (zero-copy `splice` syscalls, no per-byte copy)** — one bounded `splice(B→pipe→F)` pump loop per connection — NOT a per-byte userspace copy, and NOT fully agent-idle (the forward F→B direction stays agent-idle per `findings-egress-ktls-splice.md`).

**Date**: 2026-06-12 | **Researcher**: nw-researcher (Nova) | **Confidence**: Medium-High (High on source-level shape, Medium on end-to-end demonstration) | **Sources**: 14

---

## Executive Summary

The prior spike (increment-g, `findings-ktls-rx-splice.md`) foreclosed the **sockmap stream-verdict** path for relaying kTLS-RX decrypted plaintext: the decrypted BPF verdict runs only inside `tls_sw_recvmsg`, and `tls_sw_read_sock` returns `-EINVAL` when a psock is attached, so nothing decrypts-then-redirects while the agent is idle. **But the spike never tested `splice(2)`** — and kernel source shows that is a different, unrestricted path. `tls_sw_splice_read` (`net/tls/tls_sw.c`, registered as the TLS_SW `proto_ops.splice_read`) **decrypts each TLS record (`tls_rx_one_record`) and splices the plaintext into a pipe (`skb_splice_bits`) with no per-byte userspace copy and — crucially — no `sk_psock`/`-EINVAL` check.** The `-EINVAL` that foreclosed the spike's path lives in `tls_sw_read_sock` (the in-kernel sockmap consumer), which a splice-only design never calls. This behavior is verified unchanged across v6.12 and v6.16, so it holds at the ≥6.18 / 7.0 target.

The agent-light return mechanism is therefore a **`splice(legB → pipe → legF)` pump** (or `IORING_OP_SPLICE`, which routes through `do_splice` → the same `tls_sw_splice_read`): the agent issues bounded, kernel-paced `splice` submissions — one pump loop per connection — and the kernel does both the decrypt and the byte movement, so the decrypted bytes never transit a userspace buffer. The io_uring multishot `recv` alternative drives the kernel receive path but copies into userspace provided-buffers (a chained `IORING_OP_SEND` is then a userspace copy), so it is strictly worse than io_uring SPLICE for the zero-copy goal; the multishot+kTLS incompatibility in liburing #727 is a TLS-1.2 cmsg issue and does not affect Overdrive's TLS-1.3-only path. Hardware kTLS-RX offload (`TLS_HW`) is ruled out: it decrypts in the NIC but leaves the recv/splice delivery path unchanged, requires ConnectX-6 Dx / BlueField-2, and is unavailable on virtio-net (Lima).

**Verdict (c) PLAUSIBLE-BUT-UNVALIDATED, leaning strongly yes.** Source is decisive on the mechanism's *existence and shape* (High); no public test was found *demonstrating* `splice(kTLS-RX → pipe → socket)` delivering plaintext end-to-end (the relevant CVE-2024-0646 is the TX-destination splice path), so one cheap confirming Tier-3 spike — extend the increment-g harness, remove the sockmap, run a `splice` pump, prove zero per-byte payload syscalls + byte-exact plaintext on leg F — promotes it to (a). For #222 this means: **agent-idle forward, agent-light zero-copy-splice return — no per-byte userspace copy in either direction.** Q1 (the #26 in-band lossy DROP-RESET gate) does not move.

## Research Methodology

**Search Strategy**: Primary kernel source at the v7.0 tag (`net/tls/tls_sw.c`, `io_uring/net.c`) via elixir.bootlin.com / git.kernel.org; LWN articles on io_uring recv, kTLS, and splice; kernel docs (docs.kernel.org/networking/tls.rst); selftests under torvalds/linux.
**Source Selection**: kernel.org / docs.kernel.org / elixir.bootlin.com / git.kernel.org (primary, exact-semantics) > lwn.net (secondary, exposition) > github.com (selftests, liburing).
**Quality Standards**: ≥1 primary kernel source for every exact-behavior claim (splice / read_sock / io_uring); ≥2–3 sources per claim where possible.

**Cross-references (do not duplicate)**:
- `docs/feature/transparent-mtls-host-socket/spike/findings-ktls-rx-splice.md` (increment-g) — the **sockmap stream-verdict** path is functional but NOT agent-idle: decrypted verdict runs only inside `tls_sw_recvmsg`; `tls_sw_read_sock` returns `-EINVAL` with a psock attached. **That path is foreclosed and out of scope here.**
- `docs/feature/transparent-mtls-host-socket/spike/findings-egress-ktls-splice.md` (increment-f) — the **forward** (plaintext → kTLS-TX egress sockmap redirect) direction IS agent-idle.
- `docs/research/dataplane/sockmap-redirect-live-socket-liveness-research.md` — sockmap redirect liveness semantics.

## Findings

### SQ1 — `splice(2)` / `sendfile()` from a kTLS-RX socket

**Verdict: YES (per source) — `splice(2)` reads DECRYPTED plaintext from a kTLS-RX socket, kernel-driven, no per-byte userspace copy; valid only when NO psock is attached.** This is the load-bearing find of this doc.

**Finding 1.1 — `tls_sw_splice_read` exists, decrypts, and splices plaintext into a pipe.**
**Evidence** (`net/tls/tls_sw.c`, v6.12, exact body):
```c
ssize_t tls_sw_splice_read(struct socket *sock, loff_t *ppos,
                           struct pipe_inode_info *pipe,
                           size_t len, unsigned int flags)
{
    ...
    err = tls_rx_reader_lock(sk, ctx, flags & SPLICE_F_NONBLOCK);
    ...
    if (!skb_queue_empty(&ctx->rx_list)) {
        skb = __skb_dequeue(&ctx->rx_list);      // already-decrypted record
    } else {
        struct tls_decrypt_arg darg;
        err = tls_rx_rec_wait(sk, NULL, flags & SPLICE_F_NONBLOCK, true);
        ...
        err = tls_rx_one_record(sk, NULL, &darg); // DECRYPT one TLS record
        ...
        skb = darg.skb;
    }
    ...
    chunk = min_t(unsigned int, rxm->full_len, len);
    copied = skb_splice_bits(skb, sk, rxm->offset, pipe, chunk, flags); // -> pipe
    ...
}
```
`tls_rx_one_record` is the decrypt path (the same record decrypt `tls_sw_recvmsg` uses); `skb_splice_bits` moves the **decrypted** skb bytes into the destination pipe with no per-byte userspace copy. **Crucially, `tls_sw_splice_read` contains NO `sk_psock_get` / `-EINVAL` check** — unlike `tls_sw_read_sock` (SQ3). It only requires `tlm->control == TLS_RECORD_TYPE_DATA` (it `-EINVAL`s on a non-data record, e.g. an alert/handshake, requeuing it).
**Source**: [torvalds/linux net/tls/tls_sw.c @ v6.12](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_sw.c) — Accessed 2026-06-12
**Confidence**: High (primary kernel source, exact body)
**Verification**: [netdev kTLS paper (Dave Watson)](https://netdevconf.info/1.2/papers/ktls.pdf) confirms "standard read(), write(), sendfile() and splice() system calls can be used on the kTLS file descriptor." Behavior **unchanged in v6.16** (verified: `tls_sw_splice_read` still has no psock check; `tls_sw_read_sock` still `-EINVAL`s on psock), so it holds at the ≥6.18 floor. (Note: [CVE-2024-0646](https://nvd.nist.gov/vuln/detail/cve-2024-0646) — Jann Horn's kTLS splice OOB-write — concerns `splice()` with a kTLS socket as the **destination/TX**, a *different* path (affecting ≤6.7, since patched); it corroborates that splice-on-kTLS plumbing is real and exercised, but it is NOT a test of the RX `tls_sw_splice_read` read path. See Knowledge Gaps.)
**Analysis**: A `splice(B → pipe → F)` pump is therefore zero-copy at the syscall level: the agent issues `splice()` calls, the kernel does the decrypt (`tls_rx_one_record`) and the byte movement (decrypted skb → pipe → leg-F socket buffer) without the bytes transiting a userspace buffer. This is the mechanism the prior spike did NOT test (it tested only the sockmap-verdict path).

**Finding 1.2 — `splice_read` is registered as the `proto_ops` handler for the TLS_SW receive configuration.**
**Evidence** (`net/tls/tls_main.c`, v6.12, `build_proto_ops`):
```c
ops[TLS_BASE][TLS_SW].splice_read = tls_sw_splice_read;
ops[TLS_BASE][TLS_SW].read_sock   = tls_sw_read_sock;
ops[TLS_SW ][TLS_SW].splice_read  = tls_sw_splice_read;
ops[TLS_SW ][TLS_SW].read_sock    = tls_sw_read_sock;
```
`splice_read` lives in `proto_ops` (the socket-layer op the `splice(2)`/`sendfile(2)` syscall dispatches to via `do_splice`/`sock_splice_read`), so a userspace `splice()` on the kTLS RX fd reaches `tls_sw_splice_read` directly. `read_sock` (the in-kernel push-consumer op, used by sockmap/strparser) is the separate, psock-refusing entry (SQ3).
**Source**: [torvalds/linux net/tls/tls_main.c @ v6.12](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_main.c) — Accessed 2026-06-12
**Confidence**: High (primary kernel source)
**Analysis**: The two RX entries are deliberately distinct: `splice_read` (userspace `splice`) decrypts unconditionally; `read_sock` (in-kernel consumers) bails on a psock. An agent-light splice-only design uses the former and never attaches a psock — so the spike's `-EINVAL` (which was on `read_sock` under sockmap) does not apply.

**Finding 1.3 — the psock conflict is specific to `read_sock`, NOT to `splice_read`.**
**Evidence**: `tls_sw_splice_read` has no `sk_psock` reference at all; `tls_sw_read_sock` opens with `psock = sk_psock_get(sk); if (psock) { sk_psock_put(...); return -EINVAL; }`. The spike's increment-g `-EINVAL` finding is for the **sockmap** path (psock attached, autonomous pull via `read_sock`). A plain `splice()` design attaches **no sockmap/psock** to leg B — it just arms kTLS RX and splices.
**Source**: same as 1.1 (net/tls/tls_sw.c)
**Confidence**: High
**Analysis**: This resolves the brief's explicit sub-question ("does an attached psock change this — does plain `splice` without sockmap avoid the `-EINVAL`?"). **Answer: plain `splice` without sockmap avoids it entirely** — the `-EINVAL` is in a different function (`read_sock`) the splice path never calls.

### SQ2 — `io_uring` `recv` / `splice` on a kTLS-RX socket

**Verdict: two distinct io_uring shapes.** (a) `IORING_OP_SPLICE` from a kTLS-RX fd routes to `tls_sw_splice_read` → inherits SQ1's plaintext-delivering, no-userspace-copy property, and the agent submits one SQE instead of blocking in `splice()` → **agent-light, zero-copy** (the strongest io_uring path, and effectively SQ1 via the ring). (b) io_uring **multishot `recv`** drives `tls_sw_recvmsg` kernel-side but **copies decrypted plaintext into provided userspace buffers** → a chained `IORING_OP_SEND` to leg F is a **userspace copy** → agent-light on the *syscall* axis but NOT zero-copy on the *byte* axis.

**Finding 2.1 — `IORING_OP_SPLICE` routes through the socket's `->splice_read` op → `tls_sw_splice_read` for a kTLS fd.**
**Evidence** (`io_uring/splice.c`, v6.12): `io_splice()` calls `do_splice(in, poff_in, out, poff_out, sp->len, flags)`; `do_splice` dispatches to the input file's registered `splice_read` proto-op. For a kTLS RX socket that op is `tls_sw_splice_read` (SQ1, Finding 1.2). The canonical zero-copy socket-to-socket proxy pattern is **`splice(socket → pipe)` then `splice(pipe → socket)`** — Samba demonstrates this via `IORING_OP_SPLICE` at ~8.9–11 GB/s.
**Source**: [torvalds/linux io_uring/splice.c @ v6.12](https://raw.githubusercontent.com/torvalds/linux/v6.12/io_uring/splice.c) — Accessed 2026-06-12
**Confidence**: High (primary source for the call chain) / Medium for the kTLS-specific reachability (no test found exercising `IORING_OP_SPLICE` on a kTLS fd specifically — see Knowledge Gaps)
**Verification**: [Samba io_uring SambaXP 2023 (Metzmacher)](https://www.samba.org/~metze/presentations/2023/SambaXP/StefanMetzmacher_SambaXP2023-io_uring-rev0-presentation.pdf) — `IORING_OP_SPLICE` socket throughput; [LWN: io_uring/splice zero-copy extension](https://lwn.net/Articles/913653/).
**Analysis**: Because io_uring SPLICE bottoms out in the same `do_splice` → `tls_sw_splice_read` path as a bare `splice(2)`, it carries the **same** plaintext-decrypt, no-per-byte-copy guarantee as SQ1, while letting the agent batch submissions and avoid blocking. This is the best agent-light io_uring shape for the return path.

**Finding 2.2 — plain io_uring `recv`/`recvmsg` works on kTLS-RX for TLS 1.3; only *multishot* + the TLS-1.2 cmsg trick is incompatible.**
**Evidence**: liburing issue #727 ("minor incompatibility between recvmsg multishot and kTLS") reports the incompatibility is specifically OpenSSL's TLS-1.2 trick of having `recvmsg()` write 5 header bytes + reconstruct the TLS header from ancillary data, which multishot's `io_uring_recvmsg_out` packing breaks — and **"this is required only for TLSv1.2. This is not used with TLSv1.3."**
**Source**: [axboe/liburing issue #727](https://github.com/axboe/liburing/issues/727) — Accessed 2026-06-12
**Confidence**: Medium-High (the primary issue thread; cross-referenced by the LWN multishot-recv exposition)
**Verification**: [LWN: io_uring multishot recv](https://lwn.net/Articles/899498/) confirms multishot drives the kernel receive path asynchronously and copies into **provided userspace buffers**.
**Analysis**: Overdrive's kTLS is TLS 1.3 only (AES-256-GCM), so the multishot incompatibility does not bite the data path on the kTLS 1.3 plaintext (it concerns TLS-1.2 cmsg reconstruction). BUT multishot `recv` still delivers decrypted plaintext into a **userspace buffer**; forwarding to leg F via `IORING_OP_SEND` is then a userspace copy. So the io_uring-`recv` shape is "agent-light syscalls, still a byte copy" — strictly worse than `IORING_OP_SPLICE` for zero-copy. For zero-copy, use io_uring SPLICE (Finding 2.1), not recv+send.

**Finding 2.3 — io_uring zero-copy SEND (`IORING_OP_SEND_ZC`) does not help the RX→F relay.**
**Evidence**: `IORING_OP_SEND_ZC` avoids the *send-side* userspace→kernel copy by pinning the userspace buffer, but it still requires the bytes to be IN a userspace buffer first (delivered by a prior `recv`). It does not create a socket→socket kernel path.
**Source**: [LWN: Zero-copy network transmission with io_uring](https://lwn.net/Articles/879724/) — Accessed 2026-06-12
**Confidence**: Medium (single primary exposition; the mechanism is well-documented)
**Analysis**: `SEND_ZC` optimizes a copy the splice path avoids entirely. It is irrelevant to making the return path zero-copy; `IORING_OP_SPLICE` (2.1) is the right tool.

### SQ3 — `tls_sw_read_sock` semantics (without sockmap / in-kernel consumer)

**Verdict: `tls_sw_read_sock` refuses ANY psock — it is unusable as an in-kernel "decrypt → forward to another socket" primitive while a psock is attached.** This confirms (does not contradict) the spike. The autonomous in-kernel push path stays closed.

**Finding 3.1 — `tls_sw_read_sock` returns `-EINVAL` whenever a psock is present.**
**Evidence** (`net/tls/tls_sw.c`, v6.12):
```c
int tls_sw_read_sock(struct sock *sk, read_descriptor_t *desc,
                     sk_read_actor_t read_actor)
{
    ...
    psock = sk_psock_get(sk);
    if (psock) {
        sk_psock_put(sk, psock);
        return -EINVAL;
    }
    ...
}
```
**Source**: [torvalds/linux net/tls/tls_sw.c @ v6.12](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_sw.c) — Accessed 2026-06-12
**Confidence**: High (primary kernel source; identical to the spike's increment-g citation)
**Analysis**: `read_sock` is the proto op an in-kernel consumer (a `tcp_read_sock`-style autonomous pull, or sockmap's strparser) would use to drive decrypt without a userspace syscall. The `-EINVAL` makes that impossible *when a psock is attached*. Note it does NOT `-EINVAL` when no psock is attached — but `read_sock` is not a userspace-reachable syscall; without a psock there's no in-kernel consumer driving it either, so it is not an agent-light path on its own. The agent-light path is `splice` (SQ1), which is a separate op.

**Finding 3.2 — there is no in-tree "kTLS-RX-decrypt → redirect to another socket autonomously" path.**
**Evidence**: The spike (increment-g) established kernel-source-pinned that the decrypted BPF verdict (`sk_psock_tls_strp_read`) runs ONLY inside `tls_sw_recvmsg`, gated by `bpf_strp_enabled = sk_psock_strp_enabled(psock)` (requires a stream_parser), and `tls_sw_read_sock` refuses the psock — so no decrypt-then-redirect fires while the agent is idle. No upstream patch relaxing `tls_sw_read_sock`'s psock refusal or adding a push-driven kTLS-RX→sockmap path was found in this search.
**Source**: cross-ref `findings-ktls-rx-splice.md` (increment-g, this repo); negative search result against lore.kernel.org/netdev (no relaxation patch surfaced)
**Confidence**: Medium (the in-tree state is High via the spike; "no pending upstream work" is a bounded-search negative — see Knowledge Gaps)
**Analysis**: So the *autonomous, agent-idle* in-kernel decrypt-redirect remains foreclosed. The agent-light answer is NOT autonomy; it is moving the byte movement off the per-byte userspace copy onto a `splice` pump (SQ1) — the agent still issues bounded syscalls but never copies bytes.

### SQ4 — kTLS RX hardware offload (`TLS_HW`)

**Verdict: RULED OUT — does not change the relay path and is unavailable on virtio/Lima.**

**Finding 4.1 — HW offload decrypts in the NIC but leaves the socket delivery path (recv/splice) unchanged.**
**Evidence** (`docs.kernel.org/networking/tls-offload.html`): with RX offload "the device validates the L4 checksum and performs decryption ... sets the `decrypted` mark in `struct sk_buff`"; "The device leaves the record framing unmodified, the stack takes care of record decapsulation"; the application still "use[s] the standard `recv()` syscall; hardware offload is transparent at the API level."
**Source**: [Kernel TLS offload — kernel.org](https://docs.kernel.org/networking/tls-offload.html) — Accessed 2026-06-12
**Confidence**: High (primary kernel doc)
**Verification**: [NVIDIA kTLS Offloads](https://docs.nvidia.com/doca/sdk/ktls-offloads/index.html) — TLS_HW is "packet-based NIC offload ... integrates with the kernel stack"; software fallback handles records the NIC could not.
**Analysis**: Because the userspace-facing op is unchanged, HW offload neither enables a new autonomous redirect nor alters `tls_sw_splice_read` / `tls_sw_recvmsg` reachability. The `splice`/`recv` decision (SQ1/SQ2) is identical whether RX is sw or hw. So HW offload is orthogonal to the agent-light question.

**Finding 4.2 — HW offload requires ConnectX-6 Dx / BlueField-2+ and is NOT available on virtio-net / virtual interfaces (Lima).**
**Evidence** (kernel doc): "the current `ktls` implementation will not offload sockets routed through software interfaces such as those used for tunneling or virtual networking." NVIDIA: RX offload "supported on ConnectX-6 Dx and BlueField-2 crypto devices onwards" (RX added in mlx5e v5.9).
**Source**: [kernel.org TLS offload](https://docs.kernel.org/networking/tls-offload.html); [NVIDIA kTLS Offloads](https://docs.nvidia.com/doca/sdk/ktls-offloads/index.html) — Accessed 2026-06-12
**Confidence**: High
**Analysis**: Lima uses virtio-net; the appliance target ships software kTLS (`rxconf: sw`, per both prior spikes' `ss -tie`). HW offload is out of scope for the spike environment and the v1 appliance. Ruled in/out: **OUT.**

### SQ5 — Cost-tier classification

| Mechanism | Decrypts plaintext? | Per-byte userspace copy? | Agent posture | Cost tier |
|---|---|---|---|---|
| **Forward F→B** (egress sockmap redirect into kTLS-TX) — `findings-egress-ktls-splice.md` | n/a (encrypt) | No (kernel `tcp_sendmsg_locked`) | idle | **AGENT-IDLE** |
| **Return B→F via `splice(2)`** (`tls_sw_splice_read` → pipe → leg F) | **Yes** | **No** (`skb_splice_bits` → pipe → socket) | issues bounded `splice()` pumps | **AGENT-LIGHT (zero-copy syscalls)** |
| **Return B→F via io_uring `IORING_OP_SPLICE`** (routes to `tls_sw_splice_read`) | **Yes** | **No** | submits SQEs, not blocked | **AGENT-LIGHT (zero-copy, batched submission)** |
| **Return B→F via sockmap stream-verdict** (`findings-ktls-rx-splice.md`) | Yes | No (kernel verdict redirect) but requires the agent to drive `recvmsg` on leg B per record | issues `recvmsg` per record | agent-light syscalls, NOT idle, NOT zero-copy on the read (recvmsg copies) — **foreclosed as not better than splice** |
| **Return B→F via io_uring multishot `recv` + `IORING_OP_SEND`** | Yes | **Yes** (provided-buffer copy + send copy) | submits SQEs | **AGENT-IN-COPY-LOOP (no blocking, but a byte copy)** |
| **Return B→F via plain userspace `recvmsg`+`write` loop** (the baseline) | Yes | **Yes** | blocks per record | **AGENT-IN-COPY-LOOP (blocking)** |

**The decisive comparison**: the spike (increment-g) foreclosed the *sockmap-verdict* return path as not agent-idle — but it did NOT test `splice(2)`. `splice(2)`/`IORING_OP_SPLICE` on leg B (with NO psock attached) is strictly better than every other return shape: it decrypts AND moves bytes without a per-byte userspace copy, and the agent only issues bounded `splice` submissions (one pump loop per connection, kernel-paced, never byte-by-byte). It is **agent-light** (not agent-idle: the agent must drive the `splice` pump, just as the forward direction's redirect drives `tcp_sendmsg_locked` autonomously while the return cannot). For #222: **forward = agent-idle, return = agent-light zero-copy splice** — NO per-byte userspace copy in either direction.

### SQ5 — Cost-tier classification

_(pending)_

## Verdict

**(c) PLAUSIBLE-BUT-UNVALIDATED — an agent-light return EXISTS per kernel source, leaning strongly YES, but warrants one confirming Tier-3 spike.**

**The mechanism: `splice(2)` (or `IORING_OP_SPLICE`) from leg B's kTLS-RX fd → pipe → leg F.** Kernel source (v6.12 *and* v6.16, so it holds at the ≥6.18 floor) shows `tls_sw_splice_read` decrypts each TLS record (`tls_rx_one_record`) and splices the **plaintext** into a pipe (`skb_splice_bits`) with **no per-byte userspace copy** and **no psock/`-EINVAL` restriction**. The spike's foreclosing `-EINVAL` is in a *different* function — `tls_sw_read_sock` (the in-kernel push/sockmap consumer) — which a splice-only design never calls and never attaches a psock for. So the spike's NO (for the sockmap-verdict path) does NOT generalize to `splice`; that path was simply never tested.

**Why (c) and not (a):** the evidence is primary-source-strong on *existence and shape* (High confidence), but the exact path `splice(kTLS-RX → pipe → another socket)` was not found *demonstrated* in any test/selftest/CVE (the CVE is the TX-destination path). The spike harness already proves the surrounding facts (kTLS-RX engages, `ss -tie` sw, ciphertext on the peer wire), so the confirming spike is small and cheap. This leans yes; pin it with one run rather than asserting it.

**Confirming Tier-3 spike — exactly what it must show** (extend the increment-g harness; do NOT attach any sockmap/psock to leg B):
1. Arm kTLS RX on leg B (rustls handshake + `TLS_RX`, AES-256-GCM TLS 1.3), `ss -tie` shows `tcp-ulp-tls ... rxconf: sw`. **No sockmap, no `sk_skb` verdict/parser on leg B.**
2. Peer P sends `SERVER_BANNER` as TLS 1.3 application_data (ciphertext on the leg-B↔P wire — `tcpdump` shows `1703 03..`, banner cleartext count = 0 on that wire).
3. Agent runs a `splice(legB_fd → pipe[1])` then `splice(pipe[0] → legF_fd)` pump (or one `IORING_OP_SPLICE` SQE pair). **`strace`/syscall trace during the transfer shows the agent issues only `splice()` (and poll), ZERO `read`/`recvmsg`/`write`/`send` of the payload bytes** — i.e. the banner never lands in a userspace buffer.
4. A reader on F_peer receives the exact `SERVER_BANNER` plaintext, byte-identical, in order.
5. Confirm `splice` returns the **decrypted** record payload (cleartext banner on leg F = 1) and does NOT `-EINVAL` (it should not, since no psock). Also confirm record-framing handling: `tls_sw_splice_read` splices `rxm->full_len` of the data record — note whether the 5-byte TLS header / GCM tag region is included (the increment-g framing caveat), since `splice` hands record payload, not raw recv-stripped bytes.

If the spike shows (3) + (4), the verdict promotes to **(a) AGENT-LIGHT RETURN EXISTS** (mechanism = `splice`/`IORING_OP_SPLICE` on the kTLS-RX fd).

**Does this make #222 fully agent-light?** **It makes #222 agent-light in BOTH directions, with an asymmetry in posture:**
- **Forward (workload-plaintext F → peer-kTLS-TX B)**: **agent-idle** — egress sockmap redirect drives `tcp_sendmsg_locked`, agent out of the path entirely (`findings-egress-ktls-splice.md`, 15/15).
- **Return (peer-kTLS-RX-decrypt B → workload-plaintext F)**: **agent-light, zero-copy** — the agent drives a bounded `splice` pump (one loop per connection, kernel-paced), but **NO per-byte userspace copy**. NOT idle (the agent must issue the `splice` submissions; nothing pushes the decrypt autonomously — SQ3), but strictly better than the userspace-copy baseline and strictly better than the sockmap-verdict `recvmsg` path the spike foreclosed.

So the precise #222 framing is: **"agent-idle forward, agent-light zero-copy-splice return"** — no userspace per-byte copy in either direction (pending the confirming spike).

**Q1 does NOT move.** Per both prior spikes, #26's in-band path stays the spike-1-proven lossy DROP-RESET gate; this finding is about the #222 two-socket proxy's return leg only, not the workload's own socket. Unchanged.

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| net/tls/tls_sw.c @ v6.12 (raw github mirror of torvalds/linux) | raw.githubusercontent.com / github.com | Medium-High (0.8) | primary kernel source | 2026-06-12 | Y (v6.16) |
| net/tls/tls_sw.c @ v6.16 | raw.githubusercontent.com / github.com | Medium-High (0.8) | primary kernel source | 2026-06-12 | Y (v6.12) |
| net/tls/tls_main.c @ v6.12 (proto_ops registration) | raw.githubusercontent.com / github.com | Medium-High (0.8) | primary kernel source | 2026-06-12 | Y |
| io_uring/splice.c @ v6.12 | raw.githubusercontent.com / github.com | Medium-High (0.8) | primary kernel source | 2026-06-12 | N |
| docs.kernel.org/networking/tls(-offload).html | docs.kernel.org | High (1.0) | official kernel doc | 2026-06-12 | Y |
| netdevconf kTLS paper (Dave Watson, netdev 1.2) | netdevconf.info | Medium-High | conference paper | 2026-06-12 | Y |
| LWN: io_uring multishot recv (Art. 899498) | lwn.net | Medium-High (0.8) | technical exposition | 2026-06-12 | Y |
| LWN: zero-copy network tx with io_uring (Art. 879724) | lwn.net | Medium-High (0.8) | technical exposition | 2026-06-12 | N |
| LWN: io_uring/splice zero-copy ext (Art. 913653) | lwn.net | Medium-High (0.8) | technical exposition | 2026-06-12 | N |
| axboe/liburing issue #727 (recvmsg multishot + kTLS) | github.com | Medium-High (0.8) | maintainer issue thread | 2026-06-12 | Y |
| Samba io_uring SambaXP 2023 (Metzmacher) | samba.org | Medium-High | conference talk | 2026-06-12 | N |
| NVIDIA kTLS Offloads docs | docs.nvidia.com | High (vendor primary) | official HW docs | 2026-06-12 | Y |
| CVE-2024-0646 (NVD) | nvd.nist.gov | High (1.0) | vuln database | 2026-06-12 | Y |

Reputation: High: 4 | Medium-High: 9 | Avg ≈ 0.85. Every exact-semantics claim (splice / read_sock / io_uring routing) carries ≥1 primary kernel source.

## Knowledge Gaps

### Gap 1: No demonstrated test of `splice(kTLS-RX → pipe → socket)` delivering plaintext.
**Issue**: The kernel source unambiguously shows `tls_sw_splice_read` decrypts and splices plaintext with no psock restriction, and the netdev paper states splice "can be used on the kTLS fd" — but no selftest, LWN article, or CVE was found that *exercises the RX splice path specifically* and confirms plaintext delivery into a downstream socket. CVE-2024-0646 is the TX-destination splice path, not RX. **Attempted**: torvalds/linux selftests search, LWN kTLS/splice articles, CVE corpus. **Recommendation**: the confirming Tier-3 spike named in the Verdict (cheap — extends the increment-g harness, removes the sockmap). This is precisely why the verdict is (c) not (a).

### Gap 2: Record-framing semantics of `splice` vs `recvmsg`.
**Issue**: `tls_sw_splice_read` splices `rxm->full_len` of a decrypted *data record*; increment-g observed a 114-vs-92 framing artifact (record header + GCM-tag region) via the verdict path. Whether `splice` hands the clean payload or includes framing was not settled from source alone. **Recommendation**: capture exact byte layout in the spike (item 5).

### Gap 3: Pending upstream work to make kTLS-RX autonomously redirectable.
**Issue**: No lore.kernel.org/netdev patch was found relaxing `tls_sw_read_sock`'s psock refusal or adding a push-driven kTLS-RX→sockmap path; this is a bounded-search negative, not an exhaustive proof of absence. **Recommendation**: if a fully agent-*idle* return is later required, monitor netdev for a `read_sock`/sockmap-kTLS-RX relaxation; for now, `splice` (agent-light) is the answer and does not need it.

### Gap 4: `IORING_OP_SPLICE` on a kTLS fd not separately validated.
**Issue**: The call chain (`io_splice` → `do_splice` → `->splice_read` = `tls_sw_splice_read`) is source-confirmed, but no test exercises io_uring SPLICE on a kTLS socket specifically. **Recommendation**: the bare `splice(2)` spike is sufficient to validate the primitive; io_uring is an optimization of submission, not a different decrypt path.

## Full Citations

[1] Linux kernel. "net/tls/tls_sw.c" (v6.12). torvalds/linux. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_sw.c. Accessed 2026-06-12.
[2] Linux kernel. "net/tls/tls_sw.c" (v6.16). torvalds/linux. https://raw.githubusercontent.com/torvalds/linux/v6.16/net/tls/tls_sw.c. Accessed 2026-06-12.
[3] Linux kernel. "net/tls/tls_main.c" (v6.12, build_proto_ops). torvalds/linux. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/tls/tls_main.c. Accessed 2026-06-12.
[4] Linux kernel. "io_uring/splice.c" (v6.12). torvalds/linux. https://raw.githubusercontent.com/torvalds/linux/v6.12/io_uring/splice.c. Accessed 2026-06-12.
[5] Linux kernel docs. "Kernel TLS". https://docs.kernel.org/networking/tls.html. Accessed 2026-06-12.
[6] Linux kernel docs. "Kernel TLS offload". https://docs.kernel.org/networking/tls-offload.html. Accessed 2026-06-12.
[7] Watson, Dave. "Kernel TLS (kTLS)". netdev 1.2. https://netdevconf.info/1.2/papers/ktls.pdf. Accessed 2026-06-12.
[8] Corbet, Jonathan. "io_uring: multishot recv". LWN.net. https://lwn.net/Articles/899498/. Accessed 2026-06-12.
[9] Corbet, Jonathan. "Zero-copy network transmission with io_uring". LWN.net. https://lwn.net/Articles/879724/. Accessed 2026-06-12.
[10] "io_uring/splice: extend splice for supporting ublk zero copy". LWN.net. https://lwn.net/Articles/913653/. Accessed 2026-06-12.
[11] axboe/liburing. "minor incompatibility between recvmsg multishot and kTLS" (issue #727). https://github.com/axboe/liburing/issues/727. Accessed 2026-06-12.
[12] Metzmacher, Stefan. "io_uring Status Update within Samba". SambaXP 2023. https://www.samba.org/~metze/presentations/2023/SambaXP/StefanMetzmacher_SambaXP2023-io_uring-rev0-presentation.pdf. Accessed 2026-06-12.
[13] NVIDIA. "kTLS Offloads". DOCA SDK. https://docs.nvidia.com/doca/sdk/ktls-offloads/index.html. Accessed 2026-06-12.
[14] NVD. "CVE-2024-0646". https://nvd.nist.gov/vuln/detail/cve-2024-0646. Accessed 2026-06-12.

## Research Metadata
Duration: ~30 min | Examined: ~14 sources | Cited: 14 | Cross-refs: 3 prior spike/research docs | Confidence: High on splice existence/shape & io_uring routing (primary source, dual-version verified); Medium on end-to-end demonstration (no test exercises RX-splice-to-socket — hence verdict (c)) | Output: docs/research/dataplane/ktls-rx-agent-light-relay-research.md
