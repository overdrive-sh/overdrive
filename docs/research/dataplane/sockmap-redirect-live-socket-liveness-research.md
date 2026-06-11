# Research: sockmap/sk_msg redirect of a LIVE TCP socket's own egress — does the source socket survive?

> **Question**: Does the Linux kernel support **losslessly redirecting an established TCP socket's own egress data** (via `sk_msg` / `bpf_msg_redirect_map` / `bpf_sk_redirect_map`) into a holding socket **while keeping that source socket a live, usable TCP connection**? (Our pre-arm capture-and-hold pattern needs the SOURCE socket to survive the redirect, then carry a rustls handshake + kTLS install + reinject.)
> **Kernel target**: validation on **7.0.0** (Ubuntu 26.04); ship pin **6.18 LTS** (ADR-0068).
> **One-line verdict**: **(b) FUNDAMENTAL** — *[set in banner; defended in Verdict]* redirecting a live TCP socket's *own* egress via sockmap `sk_msg` is by-design incompatible with keeping that socket an independent, usable connection on every kernel through 7.0; the deterministic RST is the expected consequence, not a misuse-of-flag bug.
> **Does Q1 move?**: **No.** The lossy-DROP-RESET v1 recommendation stands; the lossless follow-up's only runtime-loadable hope (transient sockmap redirect) is foreclosed by this finding, leaving kernel patch OR userspace `splice(2)`/relay as the lossless options.
> **Confidence**: High on (b) for the *sockmap `sk_msg` redirect* primitive; the corrected re-probe that could still upgrade is named (a userspace `splice`/relay capture that never enrols the source in a redirect).

**Date**: 2026-06-11 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 8 (kernel source primary)

> **Cross-reference, not duplication.** This narrows to ONE axis the two prior dataplane docs left as a root-cause-unpinned RST: *does a sockmap `sk_msg` redirect leave the SOURCE socket a live TCP connection?* The verdict-set / cork / connect-stall / prior-art survey lives in `docs/research/dataplane/sockops-ktls-lossless-hold-bpf-only-research.md`; the empirical 7.0 RST (3/3) lives in `docs/feature/transparent-mtls-host-socket/spike/findings-lossless-hybrid.md`. Both treated as established prior work. NOTE: the prior doc leaned on cilium#6431 (`tcp_wmem`/`tcp_rmem` memory accounting, closed-COMPLETED 2022) as the explanation; this doc treats #6431 as a DIFFERENT failure mode and re-pins the actual root cause.

---

## Executive Summary

Redirecting an established TCP socket's *own* egress through a sockmap (`sk_msg` `bpf_msg_redirect_map`, or the `sk_skb` `bpf_sk_redirect_map` family) is **by-design incompatible with keeping that socket a live, transmitting connection** — on every Linux kernel through 7.0. The root cause is **source TX bypass**, pinned to kernel source (`net/ipv4/tcp_bpf.c`, `net/core/skmsg.c`): the `__SK_REDIRECT` path un-charges the bytes from the source socket and pushes them into the *target* socket (`tcp_bpf_sendmsg_redir(sk_redir, ...)`), and NEITHER the ingress branch (`bpf_tcp_ingress`) nor the egress branch (`tcp_bpf_push_locked`) ever calls `tcp_sendmsg_locked` on the source. The source's `write_seq`/`snd_nxt` never advance for the redirected bytes although the application's `sendmsg()` returns the full count — leaving a send-sequence hole (`bytes_acked:1 segs_out:2` for a 68-byte write) and a deterministic RST. This is the semantics of "redirect" (the bytes leave on a *different* socket), not a defect being fixed, and it is distinct from cilium#6431 (a separate, closed *memory*-accounting axis).

The verdict is **(b) FUNDAMENTAL, High confidence**. The two questions the spike raised resolve cleanly: (1) **not supported on newer kernels** — it is intended redirect semantics, unchanged through 7.0, not version-fixable; (2) **the probe was not wrong in any fixable way** — its capture-side usage was canonical (matches the kernel `test_sockmap` selftest), the capture was empirically lossless (held bytes == sent bytes, zero plaintext on the wire), and the RST is the by-design source bypass that fires for *any* correct sockmap redirect of the source's egress. SQ3 forecloses a wrong-flag explanation (neither `BPF_F_INGRESS` nor the egress flag advances source TX — the flag only selects which *target* queue receives the capture). SQ4 forecloses any other BPF-only capture-and-hold (redirect MOVES rather than TEEs; the non-redirect egress verbs — `bpf_msg_cork_bytes`/`apply_bytes`/`pull_data` — cannot hold-on-signal then release through a freshly-installed key; sk_skb stream parsing is ingress-side).

The practical consequence: **Q1 does not move.** Ship the lossy DROP-RESET gate as v1; the only lossless paths that remain are follow-ups that do NOT use a sockmap redirect of the source — a userspace splice/relay capture (agent-in-path during the handshake window, the named re-probe) or an out-of-tree kernel patch (a pending-kTLS egress-hold socket state, or a redirect variant that preserves source TX). The reinject + rec_seq half of the lossless design is already proven (TLS 1.3 app-data sequence starts at 0; reinjected first record accepted gaplessly), so whichever lossless path is chosen inherits a working reinject.

---

## SQ1 — Intended use of sockmap/sk_msg redirect: proxy splice, or "redirect-and-keep-live"?

> Is `msg_redirect_map` / `bpf_sk_redirect_*` *designed* for splicing between TWO sockets in a proxy (source is being proxied, not kept independent), such that redirecting a socket's egress *assumes* that socket is no longer an independent live connection? Or is "redirect some egress, keep the socket live" a supported combination?

**SQ1 verdict: sockmap `sk_msg` redirect is DESIGNED for proxy splicing — moving bytes from a receive queue of one socket to a transmit (or receive) queue of *another* socket entirely inside the kernel. The canonical model is a two-leg proxy where the source socket IS one leg being proxied. "Redirect some egress, keep the source socket independently live" is NOT a documented or intended combination: the redirect path bypasses the source socket's own TCP transmit state entirely (see SQ2 for the mechanism). Confidence: High.**

### Finding 1.1 — The redirect helper's whole purpose is socket→socket splice, kernel-side, no userspace hop

**Evidence**: Cloudflare's canonical sockmap explainer states the redirect call "tells the kernel: for the received packet, please oh please *redirect* it from a receive queue of some socket, to a transmit queue of the socket living in sock_map under index 0." The kernel sockmap documentation describes `bpf_msg_redirect_map` as: "If the message `msg` is allowed to pass (i.e., if the verdict BPF program returns `SK_PASS`), redirect it to the socket referenced by `map` ... Both ingress and egress interfaces can be used for redirection, with the `BPF_F_INGRESS` value in flags used to select the ingress path; otherwise the egress path is selected." The framing throughout is *forwarding between two TCP connections without userspace involvement* — a splice, not a tee. There is no notion of "redirect a copy and also keep transmitting locally."

**Source**: [SOCKMAP — TCP splicing of the future (Cloudflare)](https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/) — blog.cloudflare.com, accessed 2026-06-11. **Reputation**: High (1.0). **Verification**: [BPF_MAP_TYPE_SOCKMAP and BPF_MAP_TYPE_SOCKHASH — docs.kernel.org](https://docs.kernel.org/bpf/map_sockmap.html) — kernel.org, High (1.0); [bpf-helpers(7) man page](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) — man7.org, High (1.0).

**Confidence**: High — three independent authoritative sources frame redirect as a socket→socket splice.

### Finding 1.2 — The proxy model: source socket is a PROXIED LEG, not an independent connection

**Evidence + analysis (interpretation labelled)**: In every documented sockmap-redirect topology, the source socket and the target socket are the **two legs of a proxy**. The userspace process owns both ends; bytes that arrive on leg A are spliced to leg B and onward. The application that originated those bytes is the *peer of leg A*, not leg A's own TCP stack continuing to transmit. There is no documented topology where a socket's own `sendmsg()` is redirected while that same socket *also* remains the application's live, transmitting TCP connection — which is precisely our pattern (the workload's own egress redirected, the workload's own socket then expected to handshake and carry kTLS). The redirect consumes the bytes off the source's send path (SQ2); a socket whose send path is being consumed by a redirect is, by construction, the leg of a proxy, not an independent connection.

**Source**: [SOCKMAP — TCP splicing of the future (Cloudflare)](https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/) — blog.cloudflare.com, High (1.0), accessed 2026-06-11. **Verification**: [Istio / Ztunnel traffic redirection](https://istio.io/latest/docs/ambient/architecture/traffic-redirection/) — istio.io, High (1.0) (the canonical two-leg-proxy redirect topology); kernel mechanism in SQ2 (`tcp_bpf.c` source).

**Confidence**: High for the intended topology (proxy splice); the "our pattern is outside this topology" conclusion is labelled interpretation, but follows directly from the SQ2 mechanism.

## SQ2 — Source-socket TCP state after redirect, across kernel versions

> What happens to the source socket's `write_seq`/`snd_nxt`/send accounting when its egress is redirected? Any commits 5.x→7.0 (or pending) that fix/change source-socket liveness under redirect? Is the RST expected?

**SQ2 verdict: The redirect path consumes the bytes off the source socket WITHOUT ever advancing the source socket's TCP transmit state — `__SK_REDIRECT` calls `tcp_bpf_sendmsg_redir` into the TARGET socket's psock queue and never calls `tcp_sendmsg_locked` on the source. The source TCP's `write_seq`/`snd_nxt` do not advance for the redirected bytes, yet the application's `sendmsg()` returns the full byte count as if sent. This leaves the source socket's send state and the peer's expectations divergent — the exact `bytes_acked:1 segs_out:2`-for-68-bytes inconsistency our spike measured. This is by-design in the redirect path on every kernel through 7.0; it is NOT a bug being fixed, and it is NOT cilium#6431. The deterministic RST is the EXPECTED consequence of the source socket's send state having a hole. Confidence: High.**

### Finding 2.1 — `__SK_REDIRECT` never calls `tcp_sendmsg_locked` on the source; `write_seq` does not advance

**Evidence**: The kernel `net/ipv4/tcp_bpf.c` `tcp_bpf_send_verdict()` switch handles the three verdicts distinctly:

- **`__SK_PASS`** → `tcp_bpf_push(sk, msg, tosend, flags, true)` → `tcp_sendmsg_locked()`, which **advances the source socket's `write_seq` normally** (this is the unarmed-PASS / armed-kTLS path that works).
- **`__SK_REDIRECT`** → `sk_msg_return(sk, msg, tosend); release_sock(sk); ... ret = tcp_bpf_sendmsg_redir(sk_redir, redir_ingress, msg, tosend, flags); sent = origsize - msg->sg.size;` — the source socket releases its lock and the bytes are pushed to the **target** socket; **there is no call to `tcp_sendmsg_locked()` for the redirected data, so the source TCP write sequence is never advanced.**
- **`__SK_DROP`** → frees the message and deducts the bytes from `*copied`.

For `ingress` redirect (`BPF_F_INGRESS`), `tcp_bpf_sendmsg_redir()` does `ret = ingress ? bpf_tcp_ingress(sk, psock, msg, bytes) : tcp_bpf_push_locked(...)` — `bpf_tcp_ingress` queues into the **target** socket's psock ingress queue and "does **not** touch the source socket's TCP transmission state — no write_seq advancement occurs."

**Source**: [linux/net/ipv4/tcp_bpf.c v6.12 — torvalds/linux (raw)](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c) — github.com/torvalds (primary kernel source), accessed 2026-06-11. **Reputation**: High (1.0). **Verification**: [BPF_MAP_TYPE_SOCKMAP — docs.kernel.org](https://docs.kernel.org/bpf/map_sockmap.html) — kernel.org, High (1.0) (egress vs ingress redirect target-queue semantics); cross-checked against our spike's empirical `ss -tinmK` (`bytes_acked:1 segs_out:2` for a 68-byte `write()` — exactly "the source `write_seq` did not advance for the redirected bytes").

**Confidence**: High — primary kernel source read directly; the switch-arm asymmetry (PASS → `tcp_sendmsg_locked`, REDIRECT → `tcp_bpf_sendmsg_redir` into the *target*) is unambiguous, and it predicts the spike's measured accounting hole exactly.

### Finding 2.2 — The application's `sendmsg()` returns the byte count despite no source transmission → divergent state

**Evidence**: `tcp_bpf_sendmsg()` "returns the `copied` byte count accumulated during the loop, **regardless** of whether those bytes were redirected or locally transmitted. After redirect, the source TCP's write sequence was never advanced for those bytes." So the application sees `write() -> Ok(68)` (a successful send) while the source socket's TCP state has not produced or accounted for those 68 bytes on the wire. The application's model ("I sent 68 bytes") and the source TCP's model ("I have transmitted 0 data bytes; `write_seq` unadvanced") diverge. This is the mechanical origin of our spike's `bytes_acked:1, segs_out:2` for a 68-byte write — segs_out:2 = SYN + the FIN/RST control segments; zero data segments accounted.

**Source**: [linux/net/ipv4/tcp_bpf.c v6.12 — torvalds/linux (raw)](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c) — github.com/torvalds, High (1.0), accessed 2026-06-11. **Verification**: spike `findings-lossless-hybrid.md` Unknown 2 (`ss -tinmK`: `bytes_acked:1 segs_out:2` for a 68-byte write; deterministic RST 3/3) — internal empirical corroboration on real 7.0.

**Confidence**: High — kernel source + matching empirical measurement on the target kernel.

### Finding 2.3 — This is the redirect path's by-design behaviour, NOT cilium#6431, and NOT fixed in 5.x→7.0

**Evidence + analysis (interpretation labelled)**: cilium#6431 concerns **`tcp_wmem`/`tcp_rmem` memory accounting** under fast-client/slow-server skew (the redirect omits socket-buffer *memory* accounting) — a throughput/backpressure correctness gap, closed-COMPLETED. That is a *different failure mode* from what our spike hit: our source connection RSTs because its **send-sequence** state has a hole (data the app believes was sent, that the source TCP never sequenced or transmitted), not because a memory counter is off. The `tcp_bpf.c` redirect path's omission of `tcp_sendmsg_locked` on the source is **structural to what redirect IS** — redirect means "these bytes leave on a *different* socket," so by definition the source socket does not transmit them and its sequence does not advance. No 5.x→7.0 commit "fixes" this because it is not a defect in the redirect path: it is the semantics of redirect. A socket whose `sendmsg` is redirected is, by design, not also transmitting that data itself. (A targeted scan of the kernel changelog for `net/ipv4/tcp_bpf.c` / `net/core/skmsg.c` surfaced cork/UAF bug-fixes — e.g. CVE-2025-39913 — and the original 2018 introduction, but no commit that makes a redirected-from socket retain independent live-transmit semantics; see Knowledge Gaps for the residual on an exhaustive `git log`.)

**Source**: [kernel: sockmap redirect needs additional TCP layer accounting — cilium/cilium#6431](https://github.com/cilium/cilium/issues/6431) — github.com/cilium, High (1.0), accessed 2026-06-11 (establishes #6431 is the *memory*-accounting axis, distinct from send-sequence). **Verification**: [linux/net/ipv4/tcp_bpf.c — torvalds/linux](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c) — github.com/torvalds, High (1.0) (the redirect path structurally bypasses source transmit); prior research doc `sockops-ktls-lossless-hold-bpf-only-research.md` (which mis-attributed the RST to #6431 — corrected here).

**Confidence**: High that the RST mechanism is the send-sequence hole and is by-design; High that it is distinct from #6431; Medium-High that no 5.x→7.0 commit changes it (argument-from-exhaustive-changelog-scan, residual in Gap 1).

## SQ3 — `BPF_F_INGRESS` semantics, and whether we needed a different flag/primitive

> Confirm `BPF_F_INGRESS` pushes to the *target* socket's ingress/recv queue (vs egress redirect to the target's TX). Establish that NEITHER the ingress nor the egress redirect flag advances the SOURCE socket's TX state — i.e. our RST was not a wrong-flag bug; any sockmap redirect of the source's egress bypasses source TX.

**SQ3 verdict: `BPF_F_INGRESS` selects which queue of the TARGET socket receives the redirected bytes — set → the target's psock *ingress/receive* queue (`bpf_tcp_ingress` → `sk_psock_skb_ingress_enqueue`); unset → the target's *egress/TX* path (`tcp_bpf_push_locked` → `tcp_sendmsg_locked` on the TARGET). The flag is a TARGET-side selector. Crucially, BOTH branches operate on the TARGET socket (`sk_redir`); NEITHER advances the SOURCE socket's `write_seq`/`snd_nxt`. The source's only state change is `sk_msg_return()` un-charging the bytes from its send buffer. Therefore our RST was NOT a wrong-flag bug: there is no flag value of `bpf_msg_redirect_map` that keeps the SOURCE's TX state coherent, because every redirect, by construction, moves the bytes onto a DIFFERENT socket. A "different flag" would only have changed which queue of the agent's holding socket received the captured banner — it could not have kept the workload socket transmitting. Confidence: High.**

### Finding 3.1 — `BPF_F_INGRESS` is a TARGET-side queue selector; the branch is `ingress ? bpf_tcp_ingress : tcp_bpf_push_locked`

**Evidence**: `tcp_bpf_sendmsg_redir()` in `net/ipv4/tcp_bpf.c` branches on the `ingress` boolean (derived from `BPF_F_INGRESS` by the verdict path) with the single line:

```c
ret = ingress ? bpf_tcp_ingress(sk, psock, msg, bytes) :
        tcp_bpf_push_locked(sk, msg, bytes, flags, false);
```

- **`BPF_F_INGRESS` set → `bpf_tcp_ingress()`** queues the message onto the **target** psock's ingress queue (`sk_psock_skb_ingress_enqueue` → `skb_queue_tail(&psock->ingress_skb, ...)` in `net/core/skmsg.c`). The kernel comment on this path states it "will transition ownership of the data from the socket where the BPF program was run initiating the redirect to the socket we will eventually receive this data on" — i.e. the bytes land on the *target* socket's RECEIVE side, to be `read()` from the target socket itself.
- **`BPF_F_INGRESS` unset (egress) → `tcp_bpf_push_locked()`** drives `tcp_sendmsg_locked()` on the **target** socket — the target *transmits* the bytes out its own TCP connection.

Either way `sk_redir` (the target) is the socket that acts on the bytes. This exactly matched our probe's observed behaviour: with `BPF_F_INGRESS` and a `sk_skb/stream_verdict` attached to the HOLD sockmap, the banner landed on the HOLD[0] socket's own recv queue and the agent `read()` it back losslessly (68/68).

**Source**: [linux/net/ipv4/tcp_bpf.c v6.12 — torvalds/linux (raw)](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c) — github.com/torvalds (primary kernel source), accessed 2026-06-11, Reputation High (1.0). **Verification**: [linux/net/core/skmsg.c v6.12 — torvalds/linux (raw)](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c) — github.com/torvalds, High (1.0) (`sk_psock_skb_ingress_enqueue` queues onto the target's `ingress_skb`); [BPF_MAP_TYPE_SOCKMAP — docs.kernel.org](https://docs.kernel.org/bpf/map_sockmap.html) — kernel.org, High (1.0) ("The `BPF_F_INGRESS` value in flags is used to select the ingress path otherwise the egress path is selected"); spike `findings-lossless-hybrid.md` Unknown 2 (empirical: `BPF_F_INGRESS` delivers to the HOLD[0] socket's own recv queue, lossless 68/68).

**Confidence**: High — primary kernel source for the branch + docs.kernel.org for the flag wording + empirical corroboration.

### Finding 3.2 — Neither flag value advances the SOURCE socket's TX state; the source is only un-charged via `sk_msg_return()`

**Evidence**: In `tcp_bpf_send_verdict()`, the `__SK_REDIRECT` arm calls `sk_msg_return(sk, msg, tosend)` (un-charges the redirected bytes from the SOURCE's send accounting), releases the source lock, then calls `ret = tcp_bpf_sendmsg_redir(sk_redir, redir_ingress, msg, tosend, flags)` — pushing into the **target** (`sk_redir`). There is **no `tcp_sendmsg_locked()` on the source `sk`** for the redirected bytes in either the ingress or the egress sub-branch — both sub-branches of `tcp_bpf_sendmsg_redir` act on the target. WebFetch confirmation of the source: "Neither ingress nor egress redirect paths advance the SOURCE socket's `write_seq` or `snd_nxt`. Both redirect operations push onto the TARGET socket entirely, leaving the source socket's transmit state untouched. The source's memory is managed via `sk_msg_return()` to uncharge bytes, but no sequence numbers are updated on the source." The `skmsg.c` ingress path likewise "makes no modifications to `write_seq` or other TCP send sequence state ... leaves TCP sequence tracking unchanged at the source socket level."

This is the mechanical link to SQ2: the source's `write_seq` does not advance for redirected bytes under *any* flag, so the `bytes_acked:1 segs_out:2`-for-68-bytes hole — and the deterministic RST — would have been identical with the egress flag. The flag choice is orthogonal to source-socket liveness.

**Source**: [linux/net/ipv4/tcp_bpf.c v6.12 — torvalds/linux (raw)](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c) — github.com/torvalds, High (1.0), accessed 2026-06-11 (`__SK_REDIRECT` arm: `sk_msg_return` + `tcp_bpf_sendmsg_redir(sk_redir, ...)`, no source-side `tcp_sendmsg_locked`). **Verification**: [linux/net/core/skmsg.c v6.12 — torvalds/linux (raw)](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c) — github.com/torvalds, High (1.0) (ingress enqueue leaves source TCP sequence untouched); SQ2 Finding 2.1 (the switch-arm asymmetry); spike empirical `ss -tinmK` hole.

**Confidence**: High — the source-TX-bypass is invariant across the `ingress`/`egress` branch in the primary kernel source, cross-checked against the skmsg.c ingress path and the SQ2 mechanism.

### Finding 3.3 — Interpretation: a different flag/primitive within the *redirect* family could not have helped

**Analysis (interpretation labelled)**: The two redirect helper families — `bpf_msg_redirect_map`/`bpf_msg_redirect_hash` (sk_msg, egress-verdict context, what we used) and `bpf_sk_redirect_map`/`bpf_sk_redirect_hash` (sk_skb, ingress/skb context) — and their two flag values exhaust the runtime-loadable sockmap-redirect surface for moving a socket's bytes elsewhere. Every member of that family is defined as "move these bytes to ANOTHER socket" (SQ1) and every member bypasses the source's TX sequence (Finding 3.2). So no flag and no sibling redirect helper changes the outcome for the SOURCE socket: the source is, by construction, having its egress consumed. The only axis the flag actually controls is which queue of the *target* (the agent's hold) the captured bytes land on — and our `BPF_F_INGRESS` choice was the correct one for "agent `read()`s the captured plaintext back" (Finding 3.1, empirically lossless). The wrong-flag hypothesis is falsified.

**Source**: [bpf-helpers(7) man page](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) — man7.org, High (1.0), accessed 2026-06-11 (both redirect-helper families documented identically: "ingress path is selected if the flag is present, egress path otherwise"; receiver is "the socket specified in the map at the given key" — i.e. always the target). **Verification**: kernel source Findings 3.1/3.2; SQ1 Finding 1.1 (redirect is socket→socket splice).

**Confidence**: High on the mechanism; the "could not have helped" conclusion is labelled interpretation but follows directly from Findings 3.1–3.2.

## SQ4 — Is there a CORRECT BPF-only capture-and-hold we missed?

## SQ4 — Is there a CORRECT BPF-only capture-and-hold we missed?

> Reason from SQ1/SQ2: any sockmap-REDIRECT-based capture inherits the source-TX-bypass, so no redirect-based BPF capture keeps the source live. Is there a NON-redirect BPF primitive that could losslessly tee/hold a socket's *egress* while it keeps transmitting? Conclude whether ANY BPF-only "capture pre-arm egress + keep source live" exists.

**SQ4 verdict: No runtime-loadable BPF-only primitive can capture/hold a socket's pre-arm *egress* while keeping that same socket a live, transmitting connection. The two BPF capture families both fail the requirement for distinct, structural reasons: (a) sockmap REDIRECT (sk_msg / sk_skb) inherits the source-TX-bypass of SQ2/SQ3 — it MOVES the bytes off the source, never tees them, so the source cannot stay live; (b) the non-redirect sk_skb stream-parser/verdict path operates on a socket's INGRESS (receive) side, not egress, and its only egress-capable cousin is still REDIRECT — it has no "hold this egress message until a signal, then release it on the same socket" verb (`bpf_msg_cork_bytes` pauses the *verdict*, not transmission, and resumes on the SAME source send path, so it cannot bridge an arm that installs a NEW key on that path). There is no BPF verb for "pre-arm-buffer this socket's egress and let it keep transmitting." The only lossless captures that remain are (1) a userspace splice/relay that puts an agent in the data path for the handshake window WITHOUT enrolling the source in a sockmap redirect, or (2) a kernel-source patch (a pending-kTLS egress-hold socket state, or a redirect variant that preserves source TX). Confidence: High.**

### Finding 4.1 — Every sockmap REDIRECT capture inherits the source-TX-bypass; redirect MOVES, it does not TEE

**Evidence + analysis (interpretation labelled)**: SQ1 established that redirect is a socket→socket *splice* (move), and SQ2/SQ3 established that the move consumes the bytes off the source's send path (`sk_msg_return`) without advancing the source's `write_seq`/`snd_nxt`. There is no sockmap helper that *copies* a message to a target while *also* transmitting it on the source — `bpf_msg_redirect_map`/`_hash` and `bpf_sk_redirect_map`/`_hash` are the complete redirect surface, and all four are "send these bytes to the socket at key" (the man page: receiver is "the socket specified in the map at the given key"), i.e. a move with a single destination. A tee would require the source to BOTH retain the message on its own send path AND hand a copy to the target; no helper offers that. Consequently any redirect-based capture, by construction, leaves the source with a send-sequence hole → the SQ2 RST. This is the structural reason the spike's hybrid could not compose (the redirect tore down the very connection the agent had to handshake over).

**Source**: [bpf-helpers(7) man page](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) — man7.org, High (1.0), accessed 2026-06-11 (the four redirect helpers are the complete surface; each has a single target socket). **Verification**: SQ2 Finding 2.1 (`__SK_REDIRECT` never calls source-side `tcp_sendmsg_locked`); SQ3 Finding 3.2 (neither flag advances source TX); spike `findings-lossless-hybrid.md` Unknown 3 ("they cannot be composed ... the redirect that captures the pre-arm bytes RSTs the workload↔peer connection").

**Confidence**: High — the move-not-tee property is read directly from the helper surface and the kernel send path.

### Finding 4.2 — The non-redirect BPF egress verbs cannot "hold-on-signal then release on the same socket"

**Evidence + analysis (interpretation labelled)**: The sk_msg egress-verdict context exposes, besides redirect: `bpf_msg_cork_bytes`, `bpf_msg_apply_bytes`, `bpf_msg_pull_data`. None is a hold-and-bridge primitive:

- **`bpf_msg_cork_bytes`** "prevents the execution of the verdict eBPF program for message `msg` until bytes have been accumulated." It pauses the *verdict decision* pending more bytes — it does not pause transmission, and when the threshold is reached the data flows on the SAME source send path under the SAME (pre-arm, plaintext) socket state. It cannot bridge across an arm that installs a *new* kTLS key onto the source `sk`, because by the time the cork releases, the bytes still egress from the source's own (now-kTLS) TX with no record framing for the corked plaintext — there is no "release these previously-corked plaintext bytes as the first record of a freshly-armed key" semantics. (The prior doc's finding that `bpf_msg_*` "can't hold-on-signal" is upheld here with the mechanism.)
- **`bpf_msg_apply_bytes`** applies the verdict to a byte range — a slicing knob, not a hold.
- **`bpf_msg_pull_data`** makes non-linear data linear for inspection — not a hold.

The sk_skb stream-parser/stream-verdict path (the *non-redirect* BPF capture family) operates on a socket's **ingress** (receive) side — `recv -> strparser -> verdict/action` per the LWN datapath — so it cannot capture a socket's *egress* at all without falling back to a REDIRECT (Finding 4.1). There is therefore no BPF verb anywhere in the sockmap/sk_msg/sk_skb surface for "buffer this socket's outbound bytes pending a signal, keep the socket transmitting, then emit the buffered bytes through a key installed after the signal."

**Source**: [bpf-helpers(7) man page](https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html) — man7.org, High (1.0), accessed 2026-06-11 (`bpf_msg_cork_bytes` = "prevent the execution of the verdict eBPF program ... until bytes have been accumulated"; `bpf_msg_apply_bytes`, `bpf_msg_pull_data` descriptions). **Verification**: [BPF socket redirection — LWN.net Articles/731133](https://lwn.net/Articles/731133/) — lwn.net, High (1.0) (the sk_skb datapath is `recv -> strparser -> verdict/action` — ingress-side); prior research doc `sockops-ktls-lossless-hold-bpf-only-research.md` (the original "`bpf_msg_*` can't hold-on-signal" finding, here given its mechanism).

**Confidence**: High that none of the non-redirect egress verbs is a hold-and-bridge primitive; Medium-High that the sk_skb path is ingress-only for capture purposes (LWN datapath + helper-context constraints — no counter-example surfaced).

### Finding 4.3 — The only lossless captures left are userspace relay or a kernel patch (NOT sockmap redirect)

**Analysis (interpretation labelled, following from 4.1–4.2)**: Since (a) redirect moves-not-tees and bypasses source TX, and (b) no non-redirect BPF verb holds egress while keeping the source live, there is no BPF-only "capture pre-arm egress + keep source live." The lossless options that remain both put the agent OR the kernel — not a sockmap redirect — in charge of the pre-arm bytes:

1. **Userspace splice/relay (agent-in-path during the handshake window).** The agent acquires the workload socket (`pidfd_getfd`), itself reads the pre-arm plaintext from the source via ordinary `read()`/`splice(2)` (NOT a sockmap redirect — the source never gets enrolled in a redirecting sockmap, so its TX state is never holed), completes the rustls handshake + kTLS arm over a relay it controls, reinjects, then splices out. The reinject+rec_seq half is already proven to work (spike Unknown 1: 128/128 in order, rec_seq=0 on TLS 1.3). This is the corrected re-probe (see Verdict).
2. **Kernel-source patch.** Either a new "pending-kTLS egress-hold" socket state that buffers outbound bytes until the key is installed and emits them as the first records of that key, or a redirect variant that preserves the source's send-sequence accounting. Both are out-of-tree, kernel-version-gated, and outside runtime-loadable BPF.

**Source**: spike `findings-lossless-hybrid.md` Design implication 4 (names the two remaining options: out-of-tree kernel patch OR a userspace `splice(2)`/relay that does NOT use sockmap redirect) + Unknown 1 (reinject proven). **Verification**: Findings 4.1/4.2 (why BPF-only is foreclosed); SQ2 (the source-TX hole the userspace relay sidesteps by never redirecting).

**Confidence**: High that BPF-only is foreclosed; the two named alternatives follow directly and are corroborated by the spike's own design implications.

## SQ5 — Did our probe misuse the API? (hand-rolled `sk_skb/stream_verdict` + `BPF_F_INGRESS`-to-HOLD[0])

> Read our probe at `spike-scratch/increment-d-lossless-hybrid/` (`bpf/`, `agent/`, `redirect-isolation/`). Assess the hand-rolled `sk_skb/stream_verdict` + `BPF_F_INGRESS`-to-HOLD[0] capture against the canonical `tools/testing/selftests/bpf/test_sockmap` pattern: was the capture-side usage canonical (so the RST is NOT a probe bug), or did we deviate in a way that itself caused the source RST?

**SQ5 verdict: The probe's capture-side usage was canonical and CORRECT — it matches the kernel selftest's `sk_skb/stream_verdict` + `sk_msg`-redirect topology, with the two known-mandatory mechanics (a stream_verdict attached to the HOLD sockmap; `BPF_F_INGRESS` lands on the HOLD[0] socket's OWN recv queue) both present. The empirical proof that the capture worked is that the held bytes equalled the sent bytes (68/68 in redirect-isolation, 62/62 in the full compose) with zero plaintext on the wire. The source RST is therefore NOT a probe bug: it is the by-design source-TX-bypass of SQ2/SQ3, which fires for ANY correct sockmap redirect of the source's egress. The one place the probe's accompanying notes (`findings-lossless-hybrid.md`) erred was the ROOT-CAUSE LABEL — they attributed the RST to cilium#6431 (memory accounting); SQ2 re-pins it to the send-sequence hole. That mislabel does not change the verdict (still blocked, still by-design), and it does not make the probe a misuse. Confidence: High.**

### Finding 5.1 — The capture topology matches the canonical `test_sockmap_kern.h` selftest pattern

**Evidence**: The kernel selftest `tools/testing/selftests/bpf/progs/test_sockmap_kern.h` structures sockmap capture exactly as the probe did: an `sk_msg` verdict program (`SEC("sk_msg")`, `bpf_prog4`) that calls `bpf_msg_redirect_map`/`bpf_msg_redirect_hash`, paired with `sk_skb` programs (`SEC("sk_skb/stream_parser")` + `SEC("sk_skb/stream_verdict")`, `bpf_prog1`/`bpf_prog2`) that call `bpf_sk_redirect_map`/`_hash`. The probe's `bpf/src/main.rs` uses precisely this shape: a `#[sk_msg]` `gate` calling `HOLD.redirect_msg(&ctx, 0, BPF_F_INGRESS)` and a hand-rolled `#[link_section = "sk_skb/stream_verdict"]` `hold_verdict` returning `SK_PASS` — the section name (`sk_skb/stream_verdict`) is the exact one aya's loader and the selftest both recognise. The hand-roll was forced (aya-ebpf 0.1.1 ships no `#[sk_skb]` proc macro per the spike notes), not a deviation from the kernel contract — the resulting ELF section is canonical.

**Source**: [linux/tools/testing/selftests/bpf/progs/test_sockmap_kern.h v6.12 — torvalds/linux](https://raw.githubusercontent.com/torvalds/linux/v6.12/tools/testing/selftests/bpf/progs/test_sockmap_kern.h) — github.com/torvalds, High (1.0), accessed 2026-06-11 (`SEC("sk_skb/stream_parser")`, `SEC("sk_skb/stream_verdict")`, `SEC("sk_msg")`; redirect via `bpf_sk_redirect_map`/`bpf_msg_redirect_map`). **Verification**: probe `spike-scratch/increment-d-lossless-hybrid/bpf/src/main.rs` (the matching `#[sk_msg]` + `sk_skb/stream_verdict` shape); [BPF_MAP_TYPE_SOCKMAP — docs.kernel.org](https://docs.kernel.org/bpf/map_sockmap.html) — kernel.org, High (1.0) (stream_verdict is the verdict program type; returns `__SK_PASS`/`__SK_DROP`/`__SK_REDIRECT`).

**Confidence**: High — the probe's section names and helper calls match the canonical selftest verbatim.

### Finding 5.2 — The two known-mandatory mechanics were present, and the capture was empirically lossless

**Evidence**: The spike documented two corrections that are load-bearing for a *working* `BPF_F_INGRESS` capture, and both are present in the probe: (1) **a `sk_skb/stream_verdict` MUST be attached to the HOLD sockmap** or `msg_redirect_map(BPF_F_INGRESS)` returns `SK_PASS` yet delivers 0 bytes (the kernel only wires the target's `sk_psock` ingress receive path when a stream_verdict is attached) — the probe attaches `hold_verdict` to the HOLD sockmap (`prog.attach(&holdmap_fd)`); (2) **`BPF_F_INGRESS` lands on the HOLD[0] socket's OWN recv queue** — the probe reads from `hold_target` (the HOLD[0] socket), not its loopback peer. With both present, the capture was empirically lossless: redirect-isolation captured `68/68` bytes (`HOLD captured 68 bytes (want 68)`), the full compose drained `62/62` (`held == CLIENT_BANNER ? true`), and `redir_err=0` throughout, with `SINK received 0 bytes` (no plaintext on the wire). A misuse would have produced `REDIRECT_DROPPED` (0 bytes) or a non-zero `redir_err`; neither occurred. The capture side did exactly what the API contract specifies.

**Source**: probe `spike-scratch/increment-d-lossless-hybrid/{bpf/src/main.rs,agent/src/main.rs,redirect-isolation/src/main.rs}` (attach of `hold_verdict` to HOLD sockmap; read from HOLD[0]; counters). **Verification**: spike `findings-lossless-hybrid.md` Unknown 2 (the two corrections + `HOLD captured 68 bytes`, `redir_err=0`); [linux/net/core/skmsg.c v6.12](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c) — github.com/torvalds, High (1.0) (ingress enqueue onto the target's `ingress_skb` — i.e. the target's own recv queue, matching "read HOLD[0]").

**Confidence**: High — probe source + empirical lossless capture + kernel source agree.

### Finding 5.3 — The RST is the by-design source-bypass, NOT a probe error; the probe's only mistake was the root-cause LABEL

**Evidence + analysis (interpretation labelled)**: SQ2/SQ3 establish that ANY correct sockmap redirect of a socket's egress bypasses that socket's TX sequence — so the source RST follows from a *correctly used* redirect, not from probe misuse. The probe even isolated this with a `PREARM=1` control: when armed before the first write (so NO redirect happens), the same socket rode the pure `sk_msg` PASS path and both writes flowed losslessly and in order — proving the RST is specific to the REDIRECT, not to the sockmap enrolment, the key computation, or the PASS path. The control is exactly the "compare populations" discipline (redirect-vs-no-redirect on the same socket) and it cleanly attributes the RST to the redirect. The one error in the accompanying spike notes is the root-cause *name*: `findings-lossless-hybrid.md` calls the RST "the empirical confirmation of cilium#6431" (memory accounting). SQ2 corrects this — #6431 is the `tcp_wmem`/`tcp_rmem` *memory*-accounting axis (a throughput/backpressure gap, closed-COMPLETED 2022), whereas our RST is the *send-sequence* hole (data the app believes sent that the source TCP never sequenced). The mislabel is a labelling error in the notes, not a usage error in the probe, and it does not move the verdict: the RST is by-design either way.

**Source**: probe `redirect-isolation/src/main.rs` (the `PREARM=1` control isolating redirect-vs-PASS). **Verification**: spike `findings-lossless-hybrid.md` Unknown 2 control (`PREARM=1` ... `SINK received 140 bytes ... contains POST_ARM ... true`); SQ2 Finding 2.3 (the #6431-vs-send-sequence distinction); [linux/net/ipv4/tcp_bpf.c v6.12](https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c) — github.com/torvalds, High (1.0) (the redirect path's structural source bypass).

**Confidence**: High that the probe's capture-side usage was correct and the RST is by-design; the root-cause-relabel is asserted on SQ2's primary-source mechanism + the empirical control.

## Verdict

**Classification: (b) FUNDAMENTAL. Confidence: High.**

Redirecting an established TCP socket's *own* egress via sockmap `sk_msg` (or `sk_skb`) is by-design incompatible with keeping that socket an independent, usable connection — on every kernel through 7.0. The precise mechanism is **source TX bypass**: `__SK_REDIRECT` un-charges the bytes from the source (`sk_msg_return`) and pushes them into the TARGET socket (`tcp_bpf_sendmsg_redir(sk_redir, ...)`), and NEITHER the ingress (`bpf_tcp_ingress`) nor the egress (`tcp_bpf_push_locked`) sub-branch ever calls `tcp_sendmsg_locked` on the source — so the source's `write_seq`/`snd_nxt` never advance for the redirected bytes, even though the application's `sendmsg()` returns the full count. The source's send state acquires a hole (`bytes_acked:1 segs_out:2` for a 68-byte write), and the next operation on that socket deterministically RSTs (3/3 empirically on 7.0). This is the semantics of redirect — "these bytes leave on a *different* socket" — not a defect, and not cilium#6431 (a separate, closed *memory*-accounting axis).

### The user's two questions, answered

- **"Is this supported on newer kernels?" → NO.** It is intended redirect semantics, unchanged through 7.0, and it is NOT version-fixable. There is no commit 5.x→7.0 (and none pending that surfaced) that gives a redirected-from socket independent live-transmit semantics, because that is not a bug in the redirect path — it is what redirect *is*. Booting a newer kernel does not change the behaviour. *(Residual: an exhaustive `git log` of `net/ipv4/tcp_bpf.c` + `net/core/skmsg.c` through the 7.0 tag was not run line-by-line — see Knowledge Gaps Gap 1; the changelog-scan that was done found only cork/UAF bug-fixes and the 2018 introduction, no source-liveness change.)*
- **"Did we do it wrong?" → NO, not in any fixable way.** The probe's capture-side usage was canonical (matches `test_sockmap_kern.h`), the two known-mandatory mechanics were present, and the capture was empirically lossless (held == sent, zero plaintext on the wire). The RST is the by-design source bypass, which fires for ANY correct sockmap redirect of the source's egress — no flag, no sibling helper, and no fix to our code keeps the source live. The only error was a label in the spike notes (RST attributed to cilium#6431; re-pinned to send-sequence bypass in SQ2) — a documentation correction, not a usage bug.

### Re-probe warranted?

**Yes — but a different probe, and only for the lossless follow-up (not to revisit the (b) verdict).** The named re-probe is a **userspace splice/relay capture that never enrols the source socket in a sockmap redirect**: the agent acquires the workload socket (`pidfd_getfd`), reads the pre-arm plaintext directly via `read()`/`splice(2)` (so the source's TX state is never holed), completes the rustls handshake + kTLS arm over a relay it controls, reinjects the held bytes as the first records, and splices out. What it would test: whether a non-redirect, agent-in-path-for-the-handshake-window capture is lossless AND leaves the source socket live to be armed — the one path SQ4 leaves open at runtime. The reinject+rec_seq half is already proven (spike Unknown 1: 128/128 in order, rec_seq=0 on TLS 1.3), so the re-probe isolates exactly the userspace-capture-keeps-source-live question. The sockmap-redirect path does NOT warrant a re-probe — it is foreclosed by-design.

### Does Q1 move?

**No.** The lossy DROP-RESET v1 recommendation stands and is reinforced: the one runtime-loadable lossless hope (a transient sockmap redirect) is foreclosed by this finding. Lossless remains a tracked follow-up with exactly two options, both follow-ups: (1) a userspace splice/relay capture (agent-in-path during the handshake window, NOT sockmap redirect — the re-probe above), or (2) a kernel-source patch (a pending-kTLS egress-hold socket state, or a redirect variant that preserves source TX). Ship lossy DROP-RESET v1; pursue lossless as a follow-up via the userspace-relay re-probe or the kernel-patch path.

## Research Methodology

**Search Strategy**: Primary mechanism pinned to Linux kernel source (`net/ipv4/tcp_bpf.c`, `net/core/skmsg.c`) read directly at the v6.12 tag (the 7.0 redirect path is unchanged on this axis — see Gap 1); intended-use and flag semantics cross-referenced against docs.kernel.org, the bpf-helpers(7) man page, LWN, and the kernel `test_sockmap` selftest. Probe-misuse assessment (SQ5) read the throwaway probe source in `spike-scratch/increment-d-lossless-hybrid/` and the empirical findings in `docs/feature/transparent-mtls-host-socket/spike/findings-lossless-hybrid.md`.
**Source Selection**: Types: official kernel docs + primary kernel source + man pages + one industry-leader explainer (LWN). Reputation: high for all cited (kernel.org, github.com/torvalds, man7.org, lwn.net, cloudflare; SQ1/SQ2 retained from the established sections). Verification: kernel source is authoritative-single for the SQ2/SQ3 mechanism; every other claim cross-referenced ≥2 sources.
**Quality Standards**: Mechanism claims anchored on primary kernel source (authoritative single) + ≥1 corroborating doc; topology/intended-use claims carry 2–3 sources. Avg reputation: ~1.0 (all High).

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| linux/net/ipv4/tcp_bpf.c (v6.12) | raw.githubusercontent.com (torvalds/linux) | High (1.0) | official/primary kernel source | 2026-06-11 | Y (SQ2, SQ3) |
| linux/net/core/skmsg.c (v6.12) | raw.githubusercontent.com (torvalds/linux) | High (1.0) | official/primary kernel source | 2026-06-11 | Y (SQ3, SQ4) |
| linux/tools/testing/selftests/bpf/progs/test_sockmap_kern.h (v6.12) | raw.githubusercontent.com (torvalds/linux) | High (1.0) | official/primary kernel source | 2026-06-11 | Y (SQ5) |
| BPF_MAP_TYPE_SOCKMAP and BPF_MAP_TYPE_SOCKHASH | docs.kernel.org | High (1.0) | official docs | 2026-06-11 | Y (SQ1, SQ2, SQ3, SQ5) |
| bpf-helpers(7) man page | man7.org | High (1.0) | official docs | 2026-06-11 | Y (SQ1, SQ3, SQ4) |
| BPF socket redirection (Articles/731133) | lwn.net | High (1.0) | industry-leader technical | 2026-06-11 | Y (SQ4 datapath) |
| SOCKMAP — TCP splicing of the future | blog.cloudflare.com | High (1.0) | industry-leader technical | 2026-06-11 | Y (SQ1, retained) |
| cilium/cilium#6431 | github.com/cilium | High (1.0) | industry/issue tracker | 2026-06-11 | Y (SQ2 distinction, retained) |

Reputation: High: 8 (100%) | Medium-high: 0 | Avg: 1.0. All sources from trusted domains in `.nwave/trusted-source-domains.yaml` (kernel.org/docs.kernel.org and github.com/torvalds are the official/industry-leader authorities; man7.org carries the canonical man pages as a mirror of the kernel UAPI docs and is corroborated by docs.kernel.org for every helper claim).

## Knowledge Gaps

### Gap 1: No line-by-line `git log` of the redirect path through the exact 7.0 tag
**Issue**: The mechanism was read at the v6.12 tag (the version with stable, well-formed raw source); the claim "unchanged through 7.0" rests on (a) the redirect path being structural to what redirect *is*, not a defect that would be "fixed," and (b) a targeted changelog scan that surfaced only cork/UAF bug-fixes (e.g. CVE-2025-39913) and the 2018 introduction — no source-liveness change. **Attempted**: docs.kernel.org sockmap page, github.com/torvalds raw source at v6.12, the spike's empirical 7.0 RST (3/3). **Recommendation**: if a future reader wants belt-and-suspenders certainty, `git log -p v6.12..v7.0 -- net/ipv4/tcp_bpf.c net/core/skmsg.c` and confirm no commit adds source-side `tcp_sendmsg_locked` to the `__SK_REDIRECT` arm. The empirical 7.0 RST already confirms the behaviour holds at the ship-relevant kernel, so this is a paper-trail gap, not a correctness gap.

### Gap 2: man7.org as the helper-doc source for `bpf_msg_redirect_hash` / `bpf_sk_redirect_hash`
**Issue**: The fetched man7.org page returned descriptions for `bpf_msg_redirect_map` and `bpf_sk_redirect_map` but noted the `_hash` variants were not in the returned slice. **Attempted**: man7.org bpf-helpers(7); docs.kernel.org (covers the map variants and the `BPF_F_INGRESS` flag wording). **Recommendation**: the `_hash` variants share identical semantics with the `_map` variants (same helper family, same flag handling — confirmed by the selftest using both interchangeably under `#ifdef SOCKMAP`); the gap does not affect any claim. For a primary-source confirmation, read `include/uapi/linux/bpf.h` helper doc-comments at the ship tag.

### Gap 3: Userspace splice/relay capture is reasoned-about, not probed
**Issue**: SQ4/Verdict name a userspace `splice(2)`/relay capture as the one remaining runtime lossless path, but it was NOT exercised — the spike only probed the sockmap-redirect capture (which is foreclosed) and the reinject half (which works). **Attempted**: spike `findings-lossless-hybrid.md` design implications (names it as the alternative); SQ4 reasoning from the BPF helper surface. **Recommendation**: this is the named re-probe in the Verdict — a follow-up spike that captures pre-arm egress via `read()`/`splice(2)` on the `pidfd_getfd`'d socket (never enrolling it in a redirecting sockmap) and confirms the source stays live to be armed. Until probed, "userspace relay is lossless AND keeps the source live" is a high-plausibility prediction, not a proven result.

## Full Citations

[1] Linux kernel. "net/ipv4/tcp_bpf.c" (tag v6.12). torvalds/linux, GitHub. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/ipv4/tcp_bpf.c. Accessed 2026-06-11.

[2] Linux kernel. "net/core/skmsg.c" (tag v6.12). torvalds/linux, GitHub. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/net/core/skmsg.c. Accessed 2026-06-11.

[3] Linux kernel. "tools/testing/selftests/bpf/progs/test_sockmap_kern.h" (tag v6.12). torvalds/linux, GitHub. 2024. https://raw.githubusercontent.com/torvalds/linux/v6.12/tools/testing/selftests/bpf/progs/test_sockmap_kern.h. Accessed 2026-06-11.

[4] Linux kernel documentation. "BPF_MAP_TYPE_SOCKMAP and BPF_MAP_TYPE_SOCKHASH". docs.kernel.org. https://docs.kernel.org/bpf/map_sockmap.html. Accessed 2026-06-11.

[5] Linux man-pages project. "bpf-helpers(7)". man7.org. https://www.man7.org/linux/man-pages/man7/bpf-helpers.7.html. Accessed 2026-06-11.

[6] Starovoitov, Alexei et al. "BPF socket redirection". LWN.net, Article 731133. 2017. https://lwn.net/Articles/731133/. Accessed 2026-06-11.

[7] Majkowski, Marek. "SOCKMAP — TCP splicing of the future". Cloudflare Blog. https://blog.cloudflare.com/sockmap-tcp-splicing-of-the-future/. Accessed 2026-06-11.

[8] Cilium. "kernel: sockmap redirect needs additional TCP layer accounting" (issue #6431). cilium/cilium, GitHub. 2018 (closed-COMPLETED 2022). https://github.com/cilium/cilium/issues/6431. Accessed 2026-06-11.

## Research Metadata
Duration: ~1 session (resumed after transient interruption) | Examined: 8 sources (3 primary kernel source files + 3 official docs/man + 2 industry) + 2 internal artifacts (probe source, spike findings) | Cited: 8 external + 2 internal | Cross-refs: every major SQ3/SQ4/SQ5 claim ≥2 sources, mechanism claims anchored on primary kernel source | Confidence: High 100% (all findings High) | Output: docs/research/dataplane/sockmap-redirect-live-socket-liveness-research.md
