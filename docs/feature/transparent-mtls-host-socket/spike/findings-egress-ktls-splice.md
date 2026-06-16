# Spike findings — egress sockmap-redirect into kTLS TX (transparent-mtls-host-socket, GH #26)

**nw-spike Phase 1 (PROBE) — follow-up, throwaway, real-kernel.** Kernel
`7.0.0-15-generic` (Ubuntu 26.04, aarch64; ≥ the pinned 6.18 floor, ADR-0068).
`CONFIG_TLS=m` (loaded), `CONFIG_BPF_STREAM_PARSER=y`, `CONFIG_NET_HANDSHAKE=y`,
`CONFIG_TLS_DEVICE=y`. aya 0.13.1 / aya-ebpf 0.1.1 / bpf-linker / rustc 1.95.0
(agent) + nightly 1.98.0 (BPF target) / ktls crate 6.0.2 / rustls 0.23 / rcgen
P-256. Throwaway code lives (gitignored) in
`spike-scratch/increment-f-egress-ktls-splice/{bpf,agent}/`. Pinned at HEAD
`55fe4038`. **Not promoted; no `overdrive-*` API touched; Phase-2 gate NOT run.**

Captured: 2026-06-12. Loopback only; software kTLS; AES-256-GCM TLS 1.3 only;
agent-as-TLS-client shape.

This file does NOT clobber `findings.md` (spike 1: in-band kTLS is real, the
`sk_msg` DROP gate is fail-closed but lossy), `findings-lossless-hybrid.md`
(increment-d: reinject+rec_seq WORKS; sockmap-redirect of the workload's *own*
live socket is foreclosed by source-TX-bypass / cilium#6431), or
`findings-userspace-relay.md` (increment-e: the lossless path collapses to a
#222 two-socket proxy; the **egress-flag-into-kTLS-TX splice was wired but never
demonstrated** — blocked by a transparent-intercept-lifecycle RST). It settles
exactly the **one unproven primitive** those three artifacts left open and that
`findings-userspace-relay.md` § "Design implications" #4 named verbatim:

> "An egress-flag redirect into a kTLS socket's TX is the only candidate
> splice-out and was **not** demonstrated here … that egress-into-kTLS-TX splice
> is the specific unproven primitive to settle on a clean harness before relying
> on it."

This probe settles it on a clean, isolated harness with no transparent-intercept
confound.

---

## THE ONE THING UNDER TEST

> Does a BPF sockmap **EGRESS** redirect (`BPF_F_INGRESS` UNSET, `flags=0`)
> deliver bytes into a **kTLS-armed target socket's TX path** so they egress
> ENCRYPTED (TLS 1.3 records), with the agent process doing ZERO per-byte I/O?

Kernel mechanism the liveness research pins, kernel-source-pinned to
`net/ipv4/tcp_bpf.c` `tcp_bpf_sendmsg_redir` (SQ2 Finding 2.1 / SQ3 Finding 3.1
of `sockmap-redirect-live-socket-liveness-research.md`): `__SK_REDIRECT` with
`BPF_F_INGRESS` **unset** → `tcp_bpf_push_locked()` → `tcp_sendmsg_locked()` **on
the TARGET socket**, which is "the unarmed-PASS / **armed-kTLS path that works**."
So an egress redirect into a kTLS-armed target IS the documented kTLS-encrypting
path. **This probe verifies it empirically on 7.0 — and it WORKS.**

## Overall verdict: **(a) YES — agent-light splice-out works**

**An egress sockmap-redirect (`bpf_sk_redirect_map`, `flags=0`) into a kTLS-armed
target socket's TX produces ENCRYPTED TLS 1.3 egress with the agent OUT of the
per-byte path.** Plaintext arriving on an isolated source socket's RX is
egress-redirected by an `sk_skb/stream_verdict` into the kTLS leg's TX; the
kernel's `tcp_sendmsg_locked` on the target drives the kTLS encrypt; the peer's
kTLS RX decrypts the exact bytes in order; and `strace` attached to the agent
*during the transfer* shows the agent performs ZERO `read`/`write`/`sendmsg`/
`recvmsg` of the payload — the kernel moved every byte. **Reproduced 15/15** on
the no-strace functional path (deterministic).

**This is the missing primitive that `findings-userspace-relay.md` could not
demonstrate.** With it, an "agent-light" lossless host-socket proxy IS reachable:
the agent does the handshake + kTLS arm, then the kernel splices the plaintext
leg → the kTLS leg in-kernel and the agent leaves the per-byte path. The earlier
"DOESN'T-WORK (BPF-only)" verdict for splice-out was about the **`BPF_F_INGRESS`
(ingress) flag** landing on the target's RECV queue (never its TX); the **egress
flag** is the correct one and it drives the kTLS TX as the kernel source predicts.

| # | Assertion | Verdict | Evidence (inline below) |
|---|-----------|---------|--------------------------|
| 1 | Encrypted egress (`17 03 03` records; never plaintext on the peer wire) | **PASS** | leg-B egress carries a `1703 0300 67` (103-byte) app_data record = the encrypted probe; `PLAINTEXT_PROBE` cleartext count on leg B = **0** |
| 2 | Correct decrypt (peer kTLS RX reconstructs the exact bytes, in order) | **PASS** | `PEER_RESULT: DECRYPT_EXACT` — 86/86 bytes, in order, no loss |
| 3 | Agent idle / spliced out (ZERO per-byte I/O during transfer) | **PASS** | strace attached during the transfer window: agent's only syscall is `sendto(16,"GO",2)` (coordination); **zero** probe-byte read/write; `ss -tie` shows kTLS on leg B |
| 4 | Reverse direction (P→F) | **NOT RUN** | secondary per brief; the F→B encrypt direction is THE result and is decisive |

**Can #222 be agent-light? YES.** The kernel can carry the steady-state
plaintext→ciphertext direction with the agent out of the per-byte path, *provided
the source of the bytes is an isolated agent-controlled socket* (leg F here), NOT
the workload's own live socket (which the liveness research foreclosed via
source-TX-bypass). In the #222 two-socket-proxy shape, leg F = the agent's
workload-facing plaintext leg and leg B = the agent's peer-facing kTLS leg — both
agent-owned — so this primitive applies directly and #222's steady-state path can
be a kernel splice, not a userspace copy loop.

---

## The isolated topology (built exactly; no cgroup/connect4 intercept)

Every socket is agent-controlled; nothing can RST out from under the test:

- **PEER P** — a rustls TLS 1.3 server that arms **kTLS RX** (ktls crate) and so
  DECRYPTS what it receives and verifies the exact `PLAINTEXT_PROBE`.
- **Leg B (kTLS, peer-facing)** — the agent's client socket to P. Inserted into
  `SOCKMAP[1]` **BEFORE** `TCP_ULP "tls"` (the ordering invariant from
  `findings.md`); rustls handshake; arm kTLS (TX+RX, `rec_seq=0`).
- **Leg F (plaintext source)** — a self-connected TCP pair the agent owns. The
  ACCEPT end (`f_target`) → `SOCKMAP[0]`; the CONNECT end (`f_peer`) is handed
  (via `SCM_RIGHTS`) to a separate **writer** process that pushes
  `PLAINTEXT_PROBE` into it. The probe arrives on `f_target`'s RX.
- **The verdict** — a hand-rolled `sk_skb/stream_verdict` on the sockmap. For a
  skb arriving on leg F (matched by `local_port`) it calls
  `bpf_sk_redirect_map(skb, &SOCKMAP, B_IDX=1, 0)` — **`flags=0` = EGRESS** —
  driving leg B's TX. For leg B's own RX it `SK_PASS`es. Pre-arm it `SK_DROP`s
  (fail-closed; no plaintext leak before kTLS).

The agent then performs NO `read`/`write` on `f_target` or leg B; the writer
(separate process) pushes the probe; the bytes flow **F → B(kTLS) → P encrypted,
agent idle**.

(`aya-ebpf 0.1.1` ships no `#[sk_skb]` proc macro — the macro source exists in
`aya-ebpf-macros` but is unwired in `lib.rs` — so the section is hand-rolled via
`#[link_section = "sk_skb/stream_verdict"]`. The redirect helper is the *typed*
`SockMap::redirect_skb(&SkBuffContext, index, flags)` → `bpf_sk_redirect_map`,
not a raw syscall.)

---

## Assertion 1 — encrypted egress: **PASS**

Wire oracle (`tcpdump -i lo`), **populations separated** (the load-bearing
distinction — leg F is a host-internal loopback pair, leg B is the peer-facing
wire):

```
connections:
   4  leg F pair  55922 -> 47598 ... (host-internal source; plaintext BY DESIGN)
   9  leg B       47598 -> 55922   (agent leg B -> peer P)
  10  leg B       55922 -> 47598   (peer P -> agent leg B)

PLAINTEXT_PROBE cleartext on leg B (the peer-facing wire)?  count = 0   <<< CLEAN
PLAINTEXT_PROBE cleartext on the leg-F loopback pair?       count = 1   (by design)

leg B egress (dst 47598) application_data records (17 03 03):
  - 1703 0300 45  (len 0x45 = 69 B)  -> client Finished
  - 1703 0300 67  (len 0x67 = 103 B) -> THE ENCRYPTED PROBE
                                        (86 plaintext + 1 inner-type + 16 GCM tag)
```

The encrypted-probe record on the wire (leg B egress, 108-byte packet):

```
02:12:20 IP 127.0.0.1.55922 > 127.0.0.1.47598: Flags [P.], seq 304:412, length 108
    0x0030:  e73d e97f 1703 0300 67bc 811c b62b 8858   .=......g....+.X
    0x0040:  e4a0 2892 68f7 0ad7 253c 7a3b f682 ea61   ..(.h...%<z;...a
    0x0050:  4ce4 3208 c96a a9b2 44e0 d8ee 2727 350a   L.2..j..D...''5.
    0x0060:  ad8d fbcb 9934 83e6 ca24 52f1 06bd 3d0a   .....4...$R...=.
```

`1703 0300 67` = TLS 1.3 `application_data` (`0x17`), legacy version `0x0303`,
length `0x0067` = 103 bytes of ciphertext. The 86-byte `PLAINTEXT_PROBE` never
appears as cleartext on leg B; it appears ONLY inside this encrypted record.

The kernel verdict counters confirm the egress redirect fired exactly once with
no error:

```
stream_verdict: invocations=4 REDIRECT(F->B egress)=1 redir_err=0 PASS(legB/other)=3 DROP(pre-arm)=0
```

(`invocations=4` = 1 redirect of leg F's probe skb + 3 PASSes of leg B's own
handshake/Finished RX records. `redir_err=0` = `bpf_sk_redirect_map` returned
`SK_PASS`.)

## Assertion 2 — correct decrypt: **PASS**

The peer's kTLS RX reconstructs the exact probe, in order, no loss:

```
PEER: kTLS installed; reading app stream
PEER: got 86 bytes: "PLAINTEXT_PROBE_egress_redirect_must_become_TLS13_appdata_never_cleartext_on_wire_0001"
PEER_RESULT: DECRYPT_EXACT — PLAINTEXT_PROBE reconstructed by kTLS RX, in order, no loss
```

The 86 bytes that arrived on leg F's RX as plaintext came out of leg B's kTLS TX
as one `0x17` record and were decrypted byte-identically by P. The redirect drove
`tcp_sendmsg_locked` on the kTLS-armed target — exactly the
"armed-kTLS path that works" the liveness research named.

## Assertion 3 — agent idle (spliced out): **PASS**

`ss -tie` (the correct kTLS-introspection flag — NOT `ss -K`, which is `--kill`)
shows kTLS live on leg B throughout the idle window:

```
tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw txconf: sw
```

`strace` attached to the agent process **during the transfer window** (setup /
psock-engagement already complete; attaching mid-idle does not perturb it). While
the 86 bytes flowed F → B(kTLS) → P, the agent's ENTIRE syscall trace of
`read`/`write`/`sendmsg`/`recvmsg`/`recvfrom`/`sendto` was:

```
sendto(16, "GO", 2, MSG_NOSIGNAL, NULL, 0) = 2      <- the writer-coordination signal only

-> CLEAN: the agent did ZERO per-byte read/write/sendmsg/recvmsg of the probe;
   the kernel moved every byte. (No socket-data syscall touched PLAINTEXT_PROBE.)
```

The agent never `read()` the bytes off leg F and never `write()` them to leg B —
the `bpf_sk_redirect_map` did it in-kernel. This is the splice-out:
`tcp_sendmsg_locked` on the target ran inside the redirect, not from a userspace
relay. The agent's `read()` syscalls during *setup* (the rustls handshake on leg
B, before the idle window) are expected and are not in the transfer-window trace.

## Assertion 4 — reverse direction (P→F): **NOT RUN** (secondary, per brief)

The F→B *encrypt* direction is THE result and is decisive. The reverse
(P sends TLS → leg B kTLS RX decrypts → redirect to F → F_peer reads plaintext)
was not exercised; it is a secondary confirmation the brief de-prioritised. NOTE
the asymmetry surfaced below: in this probe leg B is a *redirect target* and the
peer sends nothing back, so leg B's RX is idle. A reverse-direction probe would
have to redirect leg B's *decrypted RX* into leg F, which puts a verdict on leg
B's RX — and that path interacts with leg B's kTLS RX (see "Load-bearing
mechanics" #2). Not settled here.

---

## Why this is a clean YES where the prior two probes were NO/blocked

The prior probes' negative results were about the **wrong flag** and a **harness
confound**, not the egress-into-kTLS-TX mechanism:

- `findings-lossless-hybrid.md` (increment-d) and `findings-userspace-relay.md`
  (increment-e) used **`BPF_F_INGRESS`** for the splice — that lands on the
  target's **RECV** queue (`bpf_tcp_ingress`), never its kTLS **TX**, so bytes
  never egress encrypted. The liveness research SQ3 pins this:
  `BPF_F_INGRESS` set → target's psock ingress queue; unset (egress) →
  `tcp_bpf_push_locked` → `tcp_sendmsg_locked` on the target. **This probe used
  `flags=0` (egress)** — the correct flag — and the bytes drove the kTLS TX.
- increment-e *did* wire an egress mode (`SPLICE_MODE=1`) but could never run the
  steady-state path: a transparent-intercept-lifecycle RST of the workload socket
  tore the connection down before any steady-state byte flowed. **This probe has
  NO transparent intercept** — leg F is an isolated agent-owned socket pair, so
  nothing RSTs out from under the test. The steady-state byte path ran cleanly,
  15/15.

The source-TX-bypass that foreclosed increment-d (redirecting the workload's
*own* live socket corrupts *its* send accounting → RST) does **not** apply here:
leg F is not a connection that must "stay live as the workload's TCP" — it is a
controlled pipe end whose RX is the redirect *source*. The redirect's TX-bypass
hits leg F's send state (irrelevant — leg F never transmits to a real peer), not
leg B's (leg B is the *target*, whose `tcp_sendmsg_locked` runs normally and
advances its own `write_seq`). The asymmetry the research documents (source
bypassed, target transmits) is exactly what makes the *target* side work.

---

## Load-bearing mechanics (each cost a debugging detour; for whoever revisits)

1. **The flag is `0` (EGRESS), and it is the whole point.** `flags=0` →
   `tcp_bpf_push_locked` → `tcp_sendmsg_locked` on the target → kTLS encrypt.
   `BPF_F_INGRESS` → target RECV queue → NOT encrypted. Every prior probe's
   "splice-out DOESN'T-WORK" used the ingress flag; the egress flag works.

2. **A psock (verdict OR parser) on leg B's RX fights leg B's kTLS RX.** Putting
   leg B in a sockmap is *required* (a redirect target must be a sockmap member;
   a verdict-less/program-less sockmap rejects the insert with **EOPNOTSUPP**).
   But when a `stream_verdict` (or even a `stream_parser`) governs leg B's RX, it
   competes with kTLS RX and the peer's decrypt intermittently aborts
   (`PEER: ConnectionAborted`, code 103). In THIS probe leg B is only a redirect
   *target* and P sends nothing back, so leg B's RX is idle and the conflict does
   not bite — leg B stays in the same verdict-governed sockmap and the verdict
   `SK_PASS`es leg B's (handshake-only) RX. **A reverse-direction or
   bidirectional design must solve leg-B-RX-psock vs kTLS-RX before relying on
   it** — a two-map split (parser-only on the target map) was tried and still
   conflicted; this is a real open question for bidirectional kTLS splice, NOT
   settled here.

3. **`sk_skb/stream_verdict` engagement on a freshly-enrolled socket is timing-
   sensitive (verdict-only mode).** The verdict hooks `sk_data_ready` when the
   socket joins the sockmap; if the source's first skb arrives in the same
   instant, it can land on the recv queue pre-engagement (`invocations=0`). A
   ~300 ms settle after the sockmap insert before the writer pushes makes it
   deterministic (15/15). Adding a `stream_parser` companion made it *worse* on
   7.0 (suppressed verdict delivery entirely) — verdict-only is the working
   shape here.

4. **`ss -K` is `--kill`, NOT a kTLS-introspection flag.** The correct flag for
   `tcp-ulp-tls` is **`ss -tie`** (extended + internal). (Prior findings labelled
   this `ss -K`; that was a misnomer — `-K` forcibly closes sockets.) Use
   `ss -tie` to observe kTLS on a live socket.

5. **`strace` perturbs the engagement timing.** Tracing the agent from launch
   slowed setup enough to flip the verdict-engagement race (~40% success under
   `strace -ff`). The no-strace functional path is deterministic (15/15); the
   agent-idle proof is obtained by *attaching* strace to the agent only during
   the transfer window (after engagement), which does not perturb it.

---

## Design implications (for DESIGN / Q1)

1. **#222 (host L4 transparent-mTLS proxy) CAN be agent-light.** The steady-state
   plaintext→ciphertext direction can ride an in-kernel `bpf_sk_redirect_map`
   (egress, `flags=0`) from the workload-facing plaintext leg into the
   peer-facing kTLS leg's TX, with the agent out of the per-byte path. This is
   the splice-out `findings-userspace-relay.md` could not demonstrate; it is now
   demonstrated on a clean harness. The #222 proxy does NOT have to keep a
   userspace copy loop in the data path for steady state — it can hand off to the
   kernel after arming kTLS. (The handshake-window capture+flush stays userspace,
   per the prior probe; only the steady-state direction is the kernel splice.)

2. **This does NOT revive lossless #26 (kTLS on the workload's OWN socket).**
   The result is about redirecting an **agent-owned** plaintext leg into an
   **agent-owned** kTLS leg — the #222 two-socket shape. Redirecting the
   *workload's own* live socket remains foreclosed (source-TX-bypass /
   `findings-lossless-hybrid.md`). `findings-userspace-relay.md`'s "#26 lossless
   collapses to #222" stands; this probe makes the #222 steady-state path
   agent-light, it does not make #26 lossless.

3. **Q1 does NOT move for #26.** v1 for the #26 in-band path stays the
   spike-1-proven lossy DROP-RESET gate (confidentiality-correct, server-speaks-
   first assumption named). Unchanged from `findings.md` / `wave-decisions.md`.

4. **Open question for a bidirectional kTLS splice:** leg-B-RX-psock vs kTLS-RX
   (mechanic #2). The encrypt direction (F→B) is clean because leg B's RX is
   idle; a design that also splices the decrypt direction (B-RX → F) needs the
   verdict on leg B's RX, which conflicted with kTLS RX in every variant tried
   here (single-map verdict, two-map parser-only target). Settle this on a clean
   harness before relying on a bidirectional kernel splice. The reinject +
   rec_seq half (from `findings-lossless-hybrid.md`) and the
   transparent-intercept capture half (from `findings-userspace-relay.md`) are
   already proven; the encrypt-direction splice-out is now proven; the
   decrypt-direction splice-out is the remaining unknown.

---

## Scope / honesty notes

Loopback only; software kTLS (`rxconf: sw txconf: sw`); AES-256-GCM TLS 1.3 only;
agent-as-TLS-client shape; single 86-byte probe (one record). **PROVEN
(deterministic 15/15 on the no-strace functional path):** egress
`bpf_sk_redirect_map(flags=0)` into a kTLS-armed target drives `tcp_sendmsg_locked`
→ kTLS encrypt → one `0x17` TLS 1.3 record on the peer-facing wire; the peer's
kTLS RX decrypts the exact bytes in order; zero plaintext on leg B; kTLS live on
leg B (`ss -tie`); the verdict's egress redirect fired (`redir_err=0`); and the
agent did ZERO per-byte I/O of the probe during the transfer (strace attached
during the window — only the `sendto(...,"GO",...)` coordination syscall, no
probe-byte read/write). **NOT exercised:** the reverse P→F decrypt-direction
splice; a bidirectional splice with a verdict on leg B's RX (mechanic #2 — known
to conflict with kTLS RX); multi-record / large transfers; NIC/hardware kTLS
offload; the full #222 transparent-intercept compose (deliberately omitted to
isolate the primitive — that was the prior probe's confound); restart-survival.
The encrypt-direction result was reproduced 15/15; the agent-idle strace and the
`ss -tie`/wire oracles were each reproduced cleanly. The two RSTs in the wire
capture are **post-success teardown** (the F-pair and leg B closing at process
exit, AFTER the encrypted record was sent AND acked AND decrypted by P — verified
by packet ordering: the `length 108` encrypted record at `seq 304:412` is acked
by P before any RST), NOT a mid-transfer failure. Kernel state cleaned up after
the run: no stray `sk_skb`/`sockmap`, no cgroups, no bpffs pins, no XDP;
loopback healthy (refused `:1` fast, did not hang) — verified.

**Stopped after findings per the brief — did NOT run the Phase-2 gate or
promote.** `spike-scratch/increment-f-egress-ktls-splice/` left in place
(gitignored) for review.
