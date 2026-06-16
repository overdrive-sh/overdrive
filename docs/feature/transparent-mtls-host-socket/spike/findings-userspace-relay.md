# Spike findings — transparent userspace relay / splice-out (transparent-mtls-host-socket, GH #26)

**nw-spike Phase 1 (PROBE) — follow-up, throwaway, real-kernel.** Kernel
`7.0.0-15-generic` (Ubuntu 26.04, aarch64; ≥ the pinned 6.18 floor, ADR-0068).
aya 0.13.1 / aya-ebpf 0.1.1 / bpf-linker / rustc 1.95.0 (relay) + nightly
1.98.0 (BPF target) / ktls crate 6.0.2 / rustls 0.23 / rcgen P-256.
Throwaway code lives (gitignored) in
`spike-scratch/increment-e-userspace-relay/{dupread-probe,bpf,relay}/`.
**Not promoted; no `overdrive-*` API touched; Phase-2 gate NOT run.**

Captured: 2026-06-12. Loopback only; software kTLS; AES-256-GCM TLS 1.3 only;
agent-as-TLS-client shape.

This file does NOT clobber `findings.md` (spike 1: in-band kTLS is real, the
`sk_msg` DROP gate is fail-closed but lossy) or `findings-lossless-hybrid.md`
(increment-d: reinject+rec_seq WORKS, sockmap-redirect-of-the-workload's-own-
socket is foreclosed by cilium#6431 / source-TX-bypass). It probes the **one
remaining lossless option both prior artifacts named**: a *transient userspace
splice/relay that does NOT sockmap-redirect the workload's own socket* —
transparent interception (workload → agent) + handshake-window relay +
in-kernel splice-out (research `sockmap-redirect-live-socket-liveness-research.md`
§ "Path 1", and `findings-lossless-hybrid.md` § "Design implications" #4(b)).

---

## Overall verdict: **(c) PARTIAL — and the lossless path collapses into #222**

The handshake-window relay is **lossless** (transparent intercept + lossless
capture + arm-on-a-second-leg + flush captured plaintext to the peer as TLS 1.3
records, in order, zero plaintext on the peer wire — all proven). The
**in-kernel splice-out does NOT work** on a runtime-loadable BPF path, for the
same structural reason `findings-lossless-hybrid.md` found, now seen from the
other side: `BPF_F_INGRESS` redirect lands on the *target's RECV queue*, not its
kTLS *TX*, so the post-handshake-window bytes never reach the peer encrypted; an
egress-flag redirect into a kTLS socket's TX is unproven and the steady-state
byte path could not be demonstrated end-to-end in this harness.

**The headline (the #222-collapse answer): YES — a lossless host-socket mTLS
path is, by construction, a host L4 transparent-mTLS proxy (#222), NOT
kTLS-on-the-workload's-own-socket (#26).** To capture the pre-arm plaintext
losslessly you MUST make the workload connect to the agent (transparent
interception) and have the agent own *two* sockets — a workload-facing plaintext
leg and a peer-facing kTLS leg. Bytes then traverse two agent-owned kernel
sockets (a kernel/userspace proxy), which is exactly the #222 host-L4-proxy
shape and fundamentally unlike #26's "kTLS on the workload's own single socket,
workload owns the fd, restart-survivable" model. "Lossless host-socket mTLS for
exec/WASM" reduces to "route the workload through the #222 proxy."

| # | Crux unknown | Verdict | One-line |
|---|--------------|---------|----------|
| 1 | Transparent interception (workload unaware) | **WORKS** | `cgroup/connect4` rewrites the dest to the agent; the workload `connect()`s to a fake peer and lands on the agent; lossless `recv()` drains the workload's plaintext (banner 80/80). |
| 2 | Lossless end-to-end (client-speaks-first) | **PARTIAL** | The **handshake-window** bytes (the pre-arm CLIENT_BANNER) arrive at the peer exactly once, in order, as a TLS 1.3 `0x17` record, zero plaintext on the peer leg. **Steady-state POST never delivered** (blocked by splice-out + a harness intercept-lifecycle RST). |
| 3 | Splice-out (userspace leaves the per-byte path) | **DOESN'T-WORK** (BPF-only) | kTLS coexists with sockmap membership (`ss -K` shows kTLS on the spliced leg). But `BPF_F_INGRESS` redirect lands on the partner's RECV queue, NOT its kTLS TX → bytes never egress encrypted; the splice verdict never carried the steady-state direction (`REDIRECT(spliced)=0`). |
| 4 | #222-collapse (the architecture finding) | **YES — collapses to #222** | The lossless path is a two-agent-socket proxy = the #222 host L4 transparent-mTLS proxy, not #26's kTLS-on-the-workload's-own-socket. Characterized below. |

**Q1 does NOT move.** This upholds spike 1 + increment-d + the liveness
research: ship the lossy DROP-RESET gate as v1 for the **#26 in-band path**
(kTLS on the workload's own socket, server-speaks-first named). A *lossless*
client-speaks-first path is not achievable on the #26 single-socket model with
runtime-loadable BPF; the only lossless option is the **#222 two-socket proxy**
(separate feature) or an out-of-tree kernel patch.

---

## What was built (throwaway)

A 2-stage probe over one shared BPF object:

- **`dupread-probe/`** (Stage 1, 238 L) — forecloses the dup-read-of-egress dead
  end: does `recv()` on a `pidfd_getfd` dup of the workload's socket drain the
  workload's TX (egress) or the peer's RX? Confirms the topology choice by
  evidence.
- **`bpf/`** (kernel-side, 4 programs) — `cgroup/connect4` (transparent dest
  rewrite), `sock_ops` (track ESTABLISHED → `SOCKHASH` + ringbuf), `sk_msg`
  splice verdict (port-keyed, ingress/egress-mode-selectable), hand-rolled
  `sk_skb/stream_verdict` (aya-ebpf 0.1.1 has no `#[sk_skb]` macro; emitted via
  `#[link_section = "sk_skb/stream_verdict"]`).
- **`relay/`** (Stage 2, ~600 L) — the agent/orchestrator: loads+attaches the
  BPF, programs `REDIRECT_DEST`, spawns a real TLS peer + a cgroup-confined
  workload, `accept()`s leg A (the transparently-redirected workload), drains
  the pre-arm plaintext, opens leg B to the real peer, rustls-handshakes, arms
  kTLS on leg B, flushes the captured plaintext, then (default) splices the two
  agent legs in-kernel and exits the per-byte path. Carries three modes
  (`RELAY_MODE=splice|userspace|pure`, `SPLICE_MODE=0|1`) to A/B the splice
  direction and isolate the failure.

---

## Stage 1 — dup-read-of-egress foreclosure (`dupread-probe/`): **CONFIRMED**

The load-bearing reason the lossless capture needs transparent interception (not
a `pidfd_getfd` dup of the workload's socket): `recv()` on the dup drains the
**RX** queue (bytes FROM the peer), never the workload's **TX** writes.

```
WORKLOAD: wrote WORKLOAD_EGRESS to my TX queue; pid=58204 fd=3
DUPREAD_BEFORE_PEER_WRITE: recv() = EAGAIN/err (os error 11) — RX queue empty,
                           workload's TX egress NOT readable via dup
PEER_RESULT: received 56 bytes from workload; contains WORKLOAD_EGRESS = true   <-- egress went OUT to the peer
DUPREAD_AFTER_PEER_WRITE: recv() returned 61 bytes; is PEER_TO_WORKLOAD(RX)? true ;
                          is WORKLOAD_EGRESS(TX)? false
VERDICT: dup-read-of-egress is IMPOSSIBLE (recv on dup drains RX, never the workload's TX)
         — Stage 2 transparent interception is REQUIRED for lossless capture.
```

Before the peer writes, the dup `recv()` returns `EAGAIN` (RX empty) — the
workload's own egress is not readable on the dup. After the peer writes, the dup
`recv()` returns the **peer's** bytes (RX), never the workload's egress (which
already left for the peer, who received it). **Capturing the workload's pre-arm
egress losslessly therefore requires the workload to be talking to the AGENT —
transparent interception.** This is what makes the lossless path a proxy.

## Unknown 1 — transparent interception (`relay/`, connect4): **WORKS**

`cgroup/connect4` rewrites the workload's `connect()` destination to the agent's
listener, transparently:

```
AGENT: REDIRECT_DEST[127.0.0.2:42658] -> 127.0.0.1:36737 programmed
WORKLOAD: connect() to 127.0.0.2:42658 -> kernel peer_addr=127.0.0.1:36737 (rewritten if != fake)
AGENT: accepted leg A from 127.0.0.1:51336 (the workload thinks it reached the peer)
AGENT: drained 80 pre-arm plaintext bytes from leg A: "CLIENT_BANNER_speaks_first_..._0001"
AGENT: held == CLIENT_BANNER ? true
```

The workload aims at `127.0.0.2:42658` (a fake peer), the kernel `peer_addr` is
`127.0.0.1:36737` (the agent) — rewritten, the workload unaware. The agent
`accept()`s leg A and drains the workload's pre-arm plaintext via an **ordinary
lossless `recv()`** (80/80 bytes, `held == CLIENT_BANNER`). No sockmap redirect
of a live socket, no cilium#6431 — a userspace buffer is trivially lossless.
**This is the lossless-capture primitive `findings-lossless-hybrid.md` #4(b)
named, and it works.** (It is also exactly what makes this a proxy — see
Unknown 4.)

## Unknown 2 — lossless end-to-end (client-speaks-first): **PARTIAL**

**Handshake-window bytes: LOSSLESS.** The captured CLIENT_BANNER is flushed
through leg B's kTLS and arrives at the real peer decrypted, in order:

```
RELAY: kTLS armed on leg B; TX rec_seq=0, RX rec_seq=0
AGENT: flushed 80 buffered bytes through kTLS leg B -> Ok(()) flush=Ok(())
PEER: kTLS installed; reading app stream
PEER: got 80 bytes: "CLIENT_BANNER_speaks_first_transparent_relay_must_arrive_decrypted_in_order_0001"
```

Wire oracle (`tcpdump -i lo`), **populations separated** (the load-bearing
distinction — leg A is host-internal, leg B is the peer-facing wire):

- **Leg A** (workload↔agent, `…> 36737`): CLIENT_BANNER appears as **plaintext**
  — **by design**; this is the workload→agent loopback intercept hop, the proxy
  ingress. It never leaves the host. (This plaintext hop IS the #222 shape.)
- **Leg B** (agent↔real-peer, `…> 38645`): **CLEAN — CLIENT_BANNER never appears
  as plaintext.** The agent→peer egress records, in order:

  ```
  16 03 01 …  ClientHello (leg-B rustls handshake)
  14 03 03 …  ChangeCipherSpec
  17 03 00 45 (len 0x45=69)  application_data — client Finished
  17 03 00 61 (len 0x61=97)  application_data — the flushed 80-byte CLIENT_BANNER
                              (80 plaintext + 1 inner-type + 16 GCM tag = 97 = 0x61)
  ```

  Leg-B record census: 2 handshake records, 3 `17 03 …` application_data records.
  The 97-byte app-data record is the captured banner, encrypted as a single
  TLS 1.3 record on the peer-facing wire. **Zero plaintext banner on leg B.**

So the handshake-window relay is lossless, in order, confidentiality-correct on
the wire that matters.

**Steady-state bytes (post-window POST): NOT delivered.** A second application
write (`POST_DATA`) after the splice never reaches the peer in any mode:
`PEER_RESULT: MISMATCH … post_present=false len=80 want=162`. Two compounding
causes, isolated below (Unknown 3 + the harness note). The lossless guarantee is
therefore proven for the **pre-arm captured bytes**, NOT yet for arbitrary
steady-state traffic — hence PARTIAL.

## Unknown 3 — splice-out (userspace leaves the per-byte path): **DOESN'T-WORK (BPF-only)**

**kTLS + sockmap coexist** — the ordering invariant from `findings.md` (sockmap
insert BEFORE `TCP_ULP "tls"`) holds, and `ss -K` proves kTLS lives on the
sockmap-member leg:

```
AGENT: PROXY.insert(leg_a, pre-kTLS) -> Ok(())
AGENT: PROXY.insert(leg_b, pre-kTLS) -> Ok(())  <<< sockmap-BEFORE-kTLS
RELAY: kTLS armed on leg B; ...                  <<< kTLS arm AFTER sockmap insert: Ok
--- ss -K for leg B (peer-facing, kTLS) ---
  ESTAB ... 127.0.0.1:38645 127.0.0.1:60240
  ... tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw txconf: sw   <<< kTLS on a PROXY member
```

So the composition (sockmap membership + kTLS on the same socket, insert-first)
is sound. **But the splice-out does not carry the steady-state direction:**

```
AGENT: SPLICED_BY_PORT=1 for leg_A(36737) + leg_B(60240) — kernel now owns the per-byte path
AGENT: spliced out — released both legs; doing NO per-byte I/O now
WORKLOAD: pre-POST socket health: SO_ERROR=103 TCP_INFO.state=7 (1=ESTAB 7=CLOSE)
WORKLOAD: POST_DATA write -> Err(BrokenPipe)
AGENT(post-GO): sk_msg egress invocations since splice: leg_A=0 leg_B=171
DEBUG counters: ... REDIRECT(spliced)=0 redirect_err=0 spliced_flag_hit=0 partner_found=0
```

Two structural facts, the first kernel-confirmed, the second harness-confirmed:

1. **`BPF_F_INGRESS` redirect lands on the partner's RECV queue, not its kTLS
   TX.** The research (`sockmap-redirect-live-socket-liveness-research.md`
   SQ3 / Finding 3.1, kernel source `net/ipv4/tcp_bpf.c`
   `tcp_bpf_sendmsg_redir`) pins it: `BPF_F_INGRESS` set → `bpf_tcp_ingress()`
   queues onto the *target's* psock ingress queue (to be `read()` from the
   target); only the **unset (egress) flag** drives `tcp_sendmsg_locked()` on the
   target. So redirecting leg A's plaintext egress into leg B with
   `BPF_F_INGRESS` deposits it in leg B's **recv** buffer — it is never
   transmitted to the peer and never sees leg B's kTLS **TX** encryption. This is
   the architectural reason the post-window direction cannot reach the peer
   encrypted via an ingress splice into a kTLS leg. (An egress-flag redirect into
   a kTLS socket's TX was wired as `SPLICE_MODE=1` but could not be demonstrated
   — see below.)

2. **The splice verdict never fired on the steady-state direction**
   (`REDIRECT(spliced)=0`, `leg_A egress=0`). The workload's intercepted client
   socket was **already RST (`SO_ERROR=103/104`, `TCP_INFO.state=7 TCP_CLOSE`)
   before the POST write** — so POST never reached leg A's egress path at all.
   This RST of the workload's intercepted socket reproduces in **every** mode,
   **including `pure` mode** (legs entirely OUT of the sockmap, no
   `stream_verdict`, no splice surface): isolation falsifies "the
   sockmap/kTLS/verdict machinery breaks the socket." The cause is the
   transparent-intercept lifecycle in this throwaway harness (the agent's leg-A
   accept-socket handling while it pivots to the multi-ms leg-B handshake), not
   the kTLS+splice composition. It blocked an end-to-end steady-state
   demonstration but is **not** a kernel finding — it is a harness limitation,
   named honestly.

**Splice-out proof (agent idle while bytes flow): NOT obtained.** After the
splice the agent does no per-byte I/O (`spliced out — doing NO per-byte I/O`),
and `ss -K` confirms kTLS on leg B — but because the steady-state bytes never
flowed (cause 1 + 2), there was no "bytes flowing while the agent is idle"
moment to capture. The splice-out is therefore **unproven**, leaning
DOESN'T-WORK for the BPF-only path: even with a perfect key match and a clean
intercept, the `BPF_F_INGRESS` direction is architecturally wrong (cause 1), and
the egress-flag-into-kTLS-TX variant is unverified.

## Unknown 4 — #222-collapse: **YES — the lossless path IS the #222 host L4 proxy**

This is the architecture headline, and it is **structural, not incidental**:

- **Lossless pre-arm capture REQUIRES transparent interception** (Stage 1
  forecloses every alternative: you cannot read the workload's egress off a
  `pidfd` dup; `findings-lossless-hybrid.md` forecloses sockmap-redirecting the
  workload's own socket via cilium#6431). The only lossless way to get the
  workload's pre-arm plaintext is to make the workload **connect to the agent**.
- Once the workload connects to the agent, the agent owns **two** sockets — a
  workload-facing plaintext leg (leg A) and a peer-facing kTLS leg (leg B) — and
  every workload byte traverses **both agent-owned kernel sockets**. Whether the
  per-byte copy between them rides a userspace loop or an in-kernel sockmap
  splice is an *optimization detail*; either way it is **a two-socket kernel
  proxy**.
- That is **exactly the #222 shape**: per the feature-delta, #222 is the *"host
  L4 tap proxy, SEPARATE feature"* for the guest-stack case (TCP in the guest
  kernel). It is **fundamentally unlike #26**, whose model is kTLS installed on
  the **workload's OWN single socket** (`pidfd_getfd` → `setsockopt` on the
  workload's fd; the workload owns the fd; restart-survivable because kTLS state
  is socket-owned and the workload owns it). In the relay path the workload owns
  NOTHING crypto-relevant and holds a plaintext socket to the agent; the SVID,
  the kTLS, and both legs live on the agent. Workload-owns-fd / restart-survival
  (the #26 DESIGN target) is **gone**.

**Conclusion: "lossless host-socket mTLS for exec/WASM" = "route the workload's
traffic through the #222 host L4 transparent-mTLS proxy."** It unifies the two
mechanisms by collapsing one into the other: the lossless variant of #26 is not
a variant of #26 at all — it is #222. The two mechanisms the prompt hoped to
unify are unified only in the sense that the lossless #26 ceases to be #26.

---

## Design implications (for DESIGN / Q1)

1. **Q1 does NOT move; v1 stays the lossy DROP-RESET gate for #26.** The #26
   in-band model (kTLS on the workload's own socket, workload-owns-fd,
   restart-survivable) has **no lossless client-speaks-first path** on
   runtime-loadable BPF — confirmed from both directions now: you cannot HOLD
   (spike 1), you cannot losslessly redirect-and-keep-live the source (increment-d
   / liveness research), and you cannot losslessly capture the source's egress
   without turning the topology into a proxy (this probe). Ship the
   confidentiality-correct lossy DROP-then-RESET gate, server-speaks-first
   assumption named (unchanged from `findings.md` #3 and `wave-decisions.md`).

2. **A lossless client-speaks-first path means adopting #222, not extending #26.**
   If a lossless host-socket path is wanted for exec/WASM, the honest design move
   is to route those workloads through the **#222 host L4 transparent-mTLS proxy**
   (the two-socket proxy this probe is, made robust), NOT to bolt a lossless
   capture onto #26. DESIGN should treat "lossless #26" as a #222 decision, with
   #222's costs: agent in the data path for the connection's life (or until a
   working splice-out), workload owns no crypto, and the restart-survival story
   changes (kernel/proxy-held, the #26-coupled Tier-3 question in CLAUDE.md).

3. **The handshake-window relay is lossless and reusable.** Transparent intercept
   + lossless `recv()` capture + arm-on-a-second-leg + flush-captured-plaintext
   delivers the pre-arm bytes to the peer as in-order TLS 1.3 records with zero
   plaintext on the peer wire. Whichever lossless path DESIGN picks (#222 proxy
   or a kernel patch) inherits this working handshake-window capture+flush, and
   inherits the proven reinject+rec_seq=0 half from `findings-lossless-hybrid.md`.

4. **In-kernel splice-out is the wrong tool for the encrypting direction.**
   `BPF_F_INGRESS` sockmap redirect delivers to the target's RECV queue, which
   bypasses the target leg's kTLS TX — so an ingress splice into a kTLS leg can
   never produce ciphertext on the peer wire. An egress-flag redirect into a
   kTLS socket's TX is the only candidate splice-out and was **not** demonstrated
   here (the intercept-lifecycle RST blocked the steady-state run). If a #222
   proxy wants userspace out of the per-byte path, that egress-into-kTLS-TX splice
   is the specific unproven primitive to settle on a clean harness before relying
   on it; until then a userspace copy loop (agent in the per-byte path) is the
   honest baseline.

5. **Two load-bearing mechanics for whoever revisits this** (each cost a debugging
   detour):
   - The `sk_msg` context's `local_ip4` and the `sock_ops` context's
     `ctx.local_ip4()` **disagree on byte order** on 7.0 (observed:
     `7f000001` vs `0100007f`), so a full-`FlowKey` `SPLICED` lookup never hits.
     `local_port` is host-order in both contexts — gate/route the splice by
     `local_port` only.
   - The EVENTS ringbuf drain must be **hard-capped**: the `sk_msg` verdict emits
     an event per egress invocation, so once any cgroup socket keeps sending the
     producer outruns an uncapped `while let Some(rb.next())` consumer and the
     agent livelocks.

## Scope / honesty notes

Loopback only; software kTLS; AES-256-GCM TLS 1.3 only; agent-as-TLS-client
shape; client-speaks-first. **PROVEN:** dup-read foreclosure (Stage 1);
transparent interception (Unknown 1); lossless capture + handshake-window flush
to peer as encrypted in-order TLS 1.3 records, zero plaintext on the peer leg
(Unknown 2, the pre-arm bytes); kTLS coexists with sockmap membership, insert-
first (Unknown 3, the composition); the #222-collapse argument (Unknown 4).
**NOT PROVEN / blocked:** steady-state (post-window) byte delivery end-to-end —
blocked by (a) the architectural `BPF_F_INGRESS`→RECV-queue fact (kernel-source
confirmed) and (b) a harness intercept-lifecycle RST of the workload's
intercepted socket that reproduces even with zero sockmap surface (a throwaway-
harness limitation, NOT a kernel finding); the egress-flag-into-kTLS-TX splice
variant; agent-idle-while-bytes-flow splice-out; server-speaks-first; NIC
offload; restart-survival. The intercept-RST was reproduced across splice /
userspace / pure modes; the lossless banner relay + wire oracle were reproduced
cleanly. Kernel state cleaned up after every run and at the end (no stray
cgroups, no stray BPF progs/maps, no XDP, no bpffs pins; loopback healthy —
verified).

**Stopped after findings per the brief — did NOT run the Phase-2 gate or
promote.** `spike-scratch/increment-e-userspace-relay/` left in place
(gitignored) for review.
