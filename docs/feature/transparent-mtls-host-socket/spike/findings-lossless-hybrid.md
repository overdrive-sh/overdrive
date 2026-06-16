# Spike findings — lossless redirect-reinject hybrid (transparent-mtls-host-socket, GH #26)

**nw-spike Phase 1 (PROBE) — follow-up, throwaway, real-kernel.** Kernel
`7.0.0-15-generic` (Ubuntu 26.04, aarch64; ≥ the pinned 6.18 floor, ADR-0068).
`CONFIG_TLS=m` (loaded), `CONFIG_BPF_STREAM_PARSER=y`, `CONFIG_NET_HANDSHAKE=y`.
aya 0.13.1 / aya-ebpf 0.1.1 / bpf-linker / rustc 1.95.0 / ktls crate 6.0.2 /
rustls 0.23. Throwaway code lives (gitignored) in
`spike-scratch/increment-d-lossless-hybrid/{bpf,agent,redirect-isolation,recseq-isolation}/`.
**Not promoted; no `overdrive-*` API touched; Phase-2 gate NOT run.**

Captured: 2026-06-11. Loopback only; software kTLS; AES-256-GCM TLS 1.3 only.

> **⚠️ CORRECTION (2026-06-12) — the RST cause is source-TX-bypass, NOT cilium#6431.**
> Every reference below attributing the deterministic source-connection RST to
> **cilium#6431** is superseded. The follow-up research
> `docs/research/dataplane/sockmap-redirect-live-socket-liveness-research.md` (SQ2,
> kernel-source-pinned to `net/ipv4/tcp_bpf.c`) established the actual cause: a
> sockmap **redirect of a live socket's own egress bypasses the source socket's TX
> path** — `__SK_REDIRECT` never calls `tcp_sendmsg_locked()` on the source, so
> `write_seq`/`snd_nxt` never advance for the redirected bytes (`bytes_acked:1
> segs_out:2` for a 68-byte write) → deterministic RST. This is the **by-design
> semantics of redirect**, invariant across the ingress/egress flag and every
> kernel through 7.0. **cilium#6431 is a SEPARATE, CLOSED-completed (2022) issue
> about `tcp_wmem`/`tcp_rmem` *memory*-budget accounting** — adjacent prior art that
> sockmap redirect has TCP-layer accounting gaps, but a different axis, not this
> RST. Read every "(cilium#6431)" below as "(source-TX-bypass)". The empirical
> finding (RST 3/3 on 7.0; the redirect-capture half blocked) is UNCHANGED and
> correct — only the named root cause was wrong.

This file does NOT clobber `findings.md` (the first spike: in-band kTLS is real,
the `sk_msg` DROP gate is fail-closed but lossy). It resolves the two unsolved
gaps the follow-up research
(`docs/research/dataplane/sockops-ktls-lossless-hold-bpf-only-research.md`, Gap 2
+ SQ3) flagged as **PLAUSIBLE-BUT-UNVALIDATED** for the
`sk_msg` REDIRECT-to-holding-socket → re-inject-after-arm hybrid: **rec_seq
continuity** and **TCP send-accounting (cilium#6431)**.

## THE ASSUMPTION UNDER TEST

> The `sk_msg` REDIRECT-to-holding-socket → re-inject-after-arm hybrid can close
> the sockops→kTLS plaintext race window LOSSLESSLY — preserving TLS rec_seq
> continuity and TCP send-accounting — with the agent in the data path ONLY
> during the handshake window, then spliced out.

## Overall verdict: **(c) PARTIAL**

**The two halves of the hybrid were tested in isolation. The *reinject + rec_seq*
half WORKS cleanly. The *redirect-capture* half is BLOCKED by a kernel-side TCP
send-accounting corruption (cilium#6431, confirmed empirically on 7.0): the
sockmap `BPF_F_INGRESS` redirect captures the workload's pre-arm bytes losslessly
into the holding socket, but leaves the *source* (workload↔peer) TCP connection's
send-side accounting inconsistent — the redirected bytes are never counted
(`bytes_acked:1, segs_out:2` for a 68-byte write) and the source connection
DETERMINISTICALLY RESETS (3/3 runs). Because that reset kills the very connection
the agent must complete its rustls handshake over, the two halves cannot be
composed on a runtime-loadable-BPF-only path.**

| # | Crux unknown | Verdict | One-line |
|---|--------------|---------|----------|
| 1 | rec_seq continuity (reinject as first app-data) | **WORKS** | Post-handshake TLS 1.3 TX rec_seq=0; reinjected banner accepted gaplessly by peer kTLS RX, decrypted, in order, never plaintext. |
| 2 | TCP source send-state (**source-TX-bypass**; NOT cilium#6431 — see Correction) | **DOESN'T-WORK** | `msg_redirect_map` never advances the source's `write_seq` (`__SK_REDIRECT` skips `tcp_sendmsg_locked` on the source); source connection RSTs deterministically (3/3). |
| 3 | Ordering + zero-loss end-to-end | **PARTIAL** | Capture lossless (68/68→HOLD); reinject lossless+in-order (128/128 at peer); cannot compose — redirect kills the handshake connection. |
| 4 | Splice-out (agent leaves; steady-state rides kTLS) | **WORKS** (in isolation; unreachable in full hybrid) | Agent closes dup fd; workload writes post-arm via kTLS in-kernel, agent out of path. |

**Blocking primitive (names what's left):** a **lossless sockmap-redirect that
preserves the *source* socket's TCP send-side accounting** (`sndbuf` /
`write_seq` / `snd_nxt` / `bytes_sent`). This is cilium#6431, classified upstream
as a kernel bug requiring kernel-level fixes — i.e. NOT closable from
runtime-loadable BPF. With it, the hybrid would compose (the reinject+rec_seq
half is proven). Without it, the runtime-loadable hybrid is not viable, which
upholds the research's (b) classification for a *BPF-only* lossless hold and
pushes Q1 to **either** lossy-DROP-RESET v1 **or** an out-of-tree kernel patch.

---

## What was built (throwaway)

Three composable harnesses over one shared BPF object, to test the two crux
unknowns *in isolation* (compare populations) plus the full compose:

- **`bpf/`** — `sockops` (ESTABLISHED → `SOCKHASH` + ringbuf) + `sk_msg` egress
  verdict that, while UNARMED, **REDIRECTs** into a holding `SOCKMAP` (`HOLD[0]`,
  `BPF_F_INGRESS`) instead of DROP, and PASSes once `ARMED[key]=1`. Plus a
  **hand-rolled `sk_skb/stream_verdict`** section (aya-ebpf 0.1.1 ships no
  `#[sk_skb]` proc macro — the macro source exists in `aya-ebpf-macros` but is
  unwired in `lib.rs`; emitted via `#[link_section = "sk_skb/stream_verdict"]`).
- **`redirect-isolation/`** — tests JUST the redirect→hold→capture→post-flip-PASS
  mechanism against a plain TCP sink (no TLS, no pidfd). `PREARM=1` control mode
  arms before the first write (no redirect) to isolate the RST cause.
- **`recseq-isolation/`** — tests JUST the reinject+rec_seq half against a real
  TLS peer: agent `pidfd_getfd`s the workload socket, rustls handshakes, arms
  kTLS, REINJECTs a held banner as the first application_data, splices out;
  the peer reconstructs the byte stream.
- **`agent/`** — the full client-speaks-first compose (workload writes
  `CLIENT_BANNER` immediately on `connect()`), which fails exactly where the
  isolation predicts (the redirect RSTs the connection before the agent can
  handshake).

---

## Unknown 1 — rec_seq continuity (`recseq-isolation/`): **WORKS**

The heart of the probe. A clean TLS 1.3 connection; the agent `pidfd_getfd`s the
workload's socket, drives the rustls client handshake, calls
`dangerous_extract_secrets()`, arms kTLS (`TCP_ULP "tls"` + `TLS_TX`/`TLS_RX`
with the post-handshake `rec_seq` seeded big-endian into
`tls12_crypto_info_aes_gcm_256.rec_seq`), then **writes the held banner as the
FIRST application_data** through the now-kTLS socket. Splices out; the workload
then writes POST via kTLS in-kernel.

```
AGENT: kTLS armed; TX rec_seq handoff = 0, RX rec_seq = 0
AGENT: reinjected 68 held bytes as first app-data (tx_seq was 0) -> Ok(68) flush=Ok(())
WORKLOAD: POST write -> Ok(60) flush=Ok(())
SERVER: got 128 bytes: "RECSEQ_BANNER_reinjected_as_FIRST_appdata_must_decrypt_in_order_0001RECSEQ_POST_written_by_workload_via_ktls_after_reinject_0002"
SERVER_RESULT: REINJECT_LOSSLESS_IN_ORDER — BANNER(reinjected) ++ POST decrypted exactly, in order
```

Wire oracle (tcpdump on `lo`):

```
--- BANNER as plaintext on the wire?  CLEAN: BANNER never plaintext
--- POST as plaintext?                CLEAN: POST never plaintext
handshake records (1603 0x): 2
app-data  records (1703 0x): 4
sample app-data on wire: 1703 0300 4526 3325 f3eb 98ac   (0x17 app_data, ver 0x0303, len 0x45)
packet flags: 1×S 1×S. 9×[.] 8×P. 2×F.    (clean FIN close — NO RST)
```

**The rec_seq handoff is a non-issue on TLS 1.3.** After a TLS 1.3 handshake the
application_data record sequence starts fresh at **0** (the counter is keyed off
the freshly-derived application traffic secret; handshake records ride a separate
key schedule and do NOT advance the app-data counter). `dangerous_extract_secrets`
hands back `rec_seq = 0` for both TX and RX; the agent seeds the kernel with 0;
the peer's kTLS RX accepts the reinjected first record at sequence 0 with **no
`EBADMSG`, no decrypt failure, no gap**. The full 128 bytes (`BANNER ++ POST`)
arrive decrypted, in order, and the connection closes cleanly with FIN. The
research's Gap 2 ("rec_seq handoff has no demonstrated working implementation")
is now demonstrated for the TLS-1.3 case: **it works, and is simpler than feared
because the post-handshake starting sequence is 0.**

(Caveat, named: this is the *agent-handshakes-as-client* shape — the agent is the
TLS client over the workload's connection, so the reinjected bytes are the
client's first app-data. A server-speaks-first variant where the agent must
reinject onto an already-advanced counter was not exercised; on TLS 1.3 the
app-data counter still starts at 0 per direction, so the same result is expected,
but it is not separately proven here.)

## Unknown 2 — TCP send-accounting / cilium#6431 (`redirect-isolation/`): **DOESN'T-WORK**

The redirect mechanism itself was made to work first (two corrections were
load-bearing):

1. **`sk_skb/stream_verdict` on the HOLD sockmap is mandatory.** Without it,
   `msg_redirect_map(BPF_F_INGRESS)` returns `SK_PASS` (`redir_err=0`) but
   delivers **0 bytes** to the target — the kernel only wires the target's
   `sk_psock` ingress receive path when a STREAM_VERDICT is attached to the
   sockmap. (`ISO_RESULT: REDIRECT_DROPPED` until the verdict was added.)
2. **`BPF_F_INGRESS` lands data on the recv queue of the `HOLD[0]` socket
   ITSELF** — the agent must `read()` the HOLD[0] socket, not its peer.

With both, the **capture is lossless**:

```
sk_msg: invocations=1 PASS(armed)=0 REDIRECT(hold)=1 redir_err=0
HOLD captured 68 bytes (want 68): "HOLD_PROBE_pre_arm_bytes_to_be_captured_in_holding_socket_0123456789"
SINK received 0 bytes   (the pre-arm bytes never reached the wire — confidentiality-correct)
```

**But the source (workload↔sink) TCP connection is corrupted by the redirect and
resets deterministically.** `ss -tinmK` on the source connection, captured live
immediately after the redirect:

```
ESTAB 0 0 127.0.0.1:51410 127.0.0.1:43837
  skmem:(...) ... bytes_acked:1 segs_out:2 segs_in:1 ...
```

`bytes_acked:1` + `segs_out:2` for a flow that just `write()`-returned **68 bytes
successfully** — the 68 redirected bytes were pulled out of the egress path but
NEVER accounted in the source TCP send state. The next operation on the source
connection then hits the inconsistent state and the kernel RSTs:

```
WORKLOAD: POST_ARM write (after expected arm) -> Err(Os { code: 104, ConnectionReset })   [run 1]
WORKLOAD: POST_ARM write (after expected arm) -> Err(Os { code: 104, ConnectionReset })   [run 2]
WORKLOAD: POST_ARM write (after expected arm) -> Err(Os { code: 104, ConnectionReset })   [run 3]
```

Wire (tcpdump on `lo`) — the source connection ends in RST, not FIN:

```
IP 127.0.0.1.54252 > 127.0.0.1.48998: Flags [S] ...
IP 127.0.0.1.48998 > 127.0.0.1.54252: Flags [S.] ...
IP 127.0.0.1.54252 > 127.0.0.1.48998: Flags [.] ack 1 ...
IP 127.0.0.1.48998 > 127.0.0.1.54252: Flags [R.] seq 1 ack 1   <<< RST, no data record ever sent
IP 127.0.0.1.54252 > 127.0.0.1.48998: Flags [R.] seq 1 ack 69
```

**Control (isolates the RST to the redirect, not the sockmap/PASS path).** With
`PREARM=1` the gate is armed BEFORE the first write, so NO redirect ever happens
and the same socket rides the pure `sk_msg` PASS path. Back-to-back writes then
flow losslessly and in order — proving the PASS path and multi-write delivery are
sound, and the RST is specific to the REDIRECT:

```
sk_msg: invocations=2 PASS(armed)=2 REDIRECT(hold)=0 redir_err=0
SINK received 140 bytes: "HOLD_PROBE...0123456789POST_ARM_after_flip_should_PASS_to_sink_if_conn_survived_AND_key_matches"
  -> contains POST_ARM (post-flip pass)?  true
```

This is the empirical confirmation of **cilium#6431** on a real 7.0 kernel:
sockmap redirect performs socket-level accounting but omits the source-side
`tcp_wmem`/send-sequence accounting, leaving the redirected-from connection
inconsistent. It is classified upstream as a kernel bug requiring kernel-level
fixes — i.e. NOT closable from runtime-loadable BPF.

## Unknown 3 — ordering + zero-loss end-to-end: **PARTIAL**

Both halves are individually lossless and in-order:
- **Capture**: 68/68 bytes into HOLD, 0 leaked to the wire (Unknown 2).
- **Reinject**: 128/128 bytes (`BANNER ++ POST`) at the peer, decrypted, in
  order, all as `0x17` records (Unknown 1).

They **cannot be composed** on the BPF-only path: the redirect that captures the
pre-arm bytes (Unknown 2) RSTs the workload↔peer connection, and that is the
exact connection the agent must complete its rustls handshake over to arm kTLS
and reinject. The full compose (`agent/`, client-speaks-first) fails precisely
there:

```
AGENT: drained 62 bytes of pre-arm plaintext from hold: "CLIENT_BANNER_speaks_first_must_arrive_decrypted_in_order_0001"
AGENT: held == CLIENT_BANNER ? true                              <<< capture worked
AGENT: gate flipped to PASS for the flow
AGENT: pidfd_getfd OK dup fd=26
AGENT: dup fd health: getsockopt(SO_ERROR) so_error=103 (ECONNABORTED); TCP_INFO state=7 (TCP_CLOSE)
AGENT: HANDSHAKE FAILED: EOF during handshake (peer closed — handshake never reached server)
DEBUG sk_msg counters: invocations=2 PASS(armed)=0 REDIRECT(hold)=2
```

The connection is already `TCP_CLOSE` (state=7) with `ECONNABORTED` pending by the
time the agent acquires it — the redirect tore it down. The full wire capture
shows the workload↔server connection reaching only SYN/SYN-ACK/ACK then RST,
with zero TLS records exchanged.

## Unknown 4 — splice-out: **WORKS (in isolation), unreachable in the full hybrid**

The rec_seq isolation proves the splice-out cleanly: after reinject the agent
`drop`s its `pidfd_getfd` dup (the workload keeps its own fd; kTLS lives on the
shared `struct sock`), and the **workload then writes POST straight through kTLS
in-kernel with the agent out of the relay** — the peer decrypts it in order
(`RECSEQ_POST...` arrives after `RECSEQ_BANNER...`). The connection closes with
FIN, not RST. So splice-out is sound *given* an armed connection — but in the full
hybrid the connection never survives to be armed (Unknown 2), so this is
unreachable on the BPF-only path.

---

## Design implications (for DESIGN / Q1)

1. **The reinject + rec_seq half is SOLVED and simpler than feared.** On TLS 1.3
   the post-handshake application_data sequence starts at 0 per direction;
   `dangerous_extract_secrets` returns `rec_seq=0`; the kernel accepts a
   reinjected first record at 0; the peer decrypts it gaplessly. **Any** future
   lossless design (the kernel-patch path included) inherits a working reinject —
   the rec_seq handoff is not a blocker. This retires research Gap 2 for the
   TLS-1.3 / agent-as-client shape.

2. **The redirect-capture half is BLOCKED by cilium#6431, confirmed on 7.0.**
   `msg_redirect_map(BPF_F_INGRESS)` captures losslessly into the holding socket
   but corrupts the *source* connection's TCP send-accounting, RSTing it
   deterministically (3/3). The source connection is exactly what the agent needs
   to handshake over — so the runtime-loadable hybrid does NOT compose. This
   **upholds the research's (b) classification** ("not achievable in BPF-only with
   the agent out of the data path") and demotes SQ3's transient-redirect hybrid
   from "the runtime-loadable lossless option" to "blocked on a kernel-side
   accounting bug."

3. **Two load-bearing mechanics for whoever revisits this** (both cost a
   debugging detour here):
   - A `sk_skb/stream_verdict` MUST be attached to the HOLD sockmap or
     `msg_redirect_map(BPF_F_INGRESS)` returns `SK_PASS` and silently delivers
     nothing. aya-ebpf 0.1.1 has no `#[sk_skb]` macro; hand-roll the section.
   - `BPF_F_INGRESS` delivers to the **HOLD[0] socket's own recv queue** — read
     that socket, not its socketpair/loopback peer.

4. **Q1 recommendation is unchanged and now empirically reinforced:** ship the
   spike-1-proven **lossy DROP-RESET gate as v1** (confidentiality-correct,
   server-speaks-first assumption named), and treat lossless as a tracked
   follow-up whose ONLY remaining options are (a) an out-of-tree kernel patch
   — either a "pending-kTLS write-block" socket state OR a fix for cilium#6431's
   source-side redirect accounting — or (b) accepting a transient agent data-path
   hop that does NOT use sockmap redirect (e.g. a userspace splice via
   `pidfd_getfd` + `splice(2)`/relay during the handshake window, not probed
   here). The reinject+rec_seq half is ready for whichever path is chosen.

## Scope / honesty notes

Loopback only; software kTLS; AES-256-GCM TLS 1.3 only; agent-as-TLS-client shape.
The cilium#6431 RST was reproduced 3/3 in the redirect-isolation and 1/1 in the
full compose; the rec_seq reinject was reproduced cleanly with the wire oracle.
NOT exercised: a non-sockmap userspace splice/relay capture (the alternative
capture primitive that sidesteps #6431), server-speaks-first reinject onto an
advanced counter, kernel-patch paths, NIC offload, restart-survival. The
`agent/` full compose remains red by construction (it is blocked at Unknown 2);
it is retained as the executable demonstration of the blocker, not as a passing
artifact. Kernel state cleaned up (no stray cgroups, no stray BPF progs, no XDP).

**Stopped after findings per the brief — did NOT run the Phase-2 gate or
promote.** `spike-scratch/increment-d-lossless-hybrid/` left in place (gitignored)
for review.
