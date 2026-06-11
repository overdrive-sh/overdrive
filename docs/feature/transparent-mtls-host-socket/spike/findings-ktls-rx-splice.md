# Spike findings — kTLS-RX decrypt → sockmap splice (RETURN direction) (transparent-mtls-host-socket, GH #26)

**nw-spike Phase 1 (PROBE) — follow-up, throwaway, real-kernel.** Kernel
`7.0.0-15-generic` (Ubuntu 26.04, aarch64; ≥ the pinned 6.18 floor, ADR-0068).
`CONFIG_TLS=m` (loaded), `CONFIG_BPF_STREAM_PARSER=y`, `CONFIG_NET_HANDSHAKE=y`,
`CONFIG_TLS_DEVICE=y`. aya 0.13.1 / aya-ebpf 0.1.1 / bpf-linker / rustc 1.95.0
(agent) + nightly 1.98.0 (BPF target) / ktls crate 6.0.2 / rustls 0.23 / rcgen
P-256. Throwaway code lives (gitignored) in
`spike-scratch/increment-g-ktls-rx-splice/{bpf,agent}/`. Pinned at HEAD
`55fe4038`. **Not promoted; no `overdrive-*` API touched; Phase-2 gate NOT run.**

Captured: 2026-06-12. Loopback only; software kTLS; AES-256-GCM TLS 1.3 only;
agent-as-TLS-client shape.

This file does NOT clobber `findings.md` (spike 1: in-band kTLS is real, the
`sk_msg` DROP gate is fail-closed but lossy), `findings-lossless-hybrid.md`
(increment-d: reinject+rec_seq WORKS; sockmap-redirect of the workload's *own*
live socket is foreclosed by source-TX-bypass), `findings-userspace-relay.md`
(increment-e: the lossless path collapses to a #222 two-socket proxy; an
egress-flag splice into a kTLS socket's TX was wired but never demonstrated;
putting a psock verdict on a kTLS-RX socket hit `PEER: ConnectionAborted`), or
`findings-egress-ktls-splice.md` (increment-f: the **forward** F→B(kTLS-TX)
direction WORKS agent-idle, 15/15 — egress `bpf_sk_redirect_map(flags=0)` into a
kTLS-armed target drives `tcp_sendmsg_locked` → kTLS encrypt, agent does ZERO
per-byte I/O). increment-f explicitly left the **return** direction open
(mechanic #2 / design-implication #4: "a design that also splices the decrypt
direction (B-RX → F) needs the verdict on leg B's RX, which conflicted with kTLS
RX in every variant tried here … Settle this on a clean harness before relying on
a bidirectional kernel splice"). **This probe settles the return direction.**

---

## THE ONE THING UNDER TEST

> Can leg B's kTLS-RX **decrypted** plaintext be redirected in-kernel (egress
> sockmap redirect) into leg F's TX, with the agent doing ZERO per-byte I/O — so
> a reader on F_peer receives the peer's TLS-encrypted response as plaintext,
> agent idle? (The RETURN half of an agent-light #222 proxy. increment-f proved
> the forward half.)

## Overall verdict: **(b) NO — the return-direction splice is FUNCTIONAL but NOT agent-idle**

**The kTLS-RX decrypt → `sk_skb/stream_verdict` → egress sockmap-redirect → leg F
chain WORKS** (the canonical `test_sockmap.c --ktls --txmsg-skb-redir` pattern,
matched exactly: dedicated verdict-governed sockmap, **stream_parser + verdict**,
sockmap-insert-before-kTLS-arm). The peer's TLS-encrypted `SERVER_BANNER` is
kTLS-RX-decrypted on leg B and the decrypted plaintext is redirected in-kernel to
leg F, where a reader reads it byte-exact (deterministic 5/5). **BUT the redirect
fires if and only if the agent drives `recvmsg`/`read` on leg B** — and that is
kernel-pinned, not a harness artifact:

- The kTLS-RX→BPF-verdict integration runs the verdict on the **decrypted** skb
  only inside `tls_sw_recvmsg` (`net/tls/tls_sw.c`), via
  `sk_psock_tls_strp_read` (`net/core/skmsg.c`), gated by
  `bpf_strp_enabled = sk_psock_strp_enabled(psock)`.
- **`tls_sw_recvmsg` is the userspace `read()`/`recvmsg()` path.** The canonical
  selftest drives it with `msg_loop(rx_fd, …)` — the consuming process actively
  reads the kTLS socket, and the verdict + redirect happen *inside that read*.
- The autonomous / push read_sock path **`tls_sw_read_sock` returns `-EINVAL`
  when a psock is attached** (`net/tls/tls_sw.c`: `psock = sk_psock_get(sk); if
  (psock) { sk_psock_put(sk, psock); return -EINVAL; }`). There is **no
  kernel path that decrypts-then-redirects autonomously while the agent is idle.**

So the **decisive NO**: an agent-idle kTLS-RX→sockmap→leg-F splice does **not**
exist on 7.0 — the kTLS-RX decrypted-verdict path is **pull-driven** (requires a
userspace read on the kTLS socket), unlike the forward kTLS-**TX** splice
(increment-f), which is push-driven by the redirect's own `tcp_sendmsg_locked` on
the target and IS agent-idle. The asymmetry is structural: encrypt-on-egress is
driven by the redirect; decrypt-on-ingress is driven by a `recvmsg` that the
kernel refuses to issue on the psock's behalf.

| # | Assertion (from the brief) | Verdict | Evidence (inline below) |
|---|-----------|---------|--------------------------|
| 1 | Decrypt+redirect works (reader reads exact `SERVER_BANNER`) | **PASS — but only when leg B is read-driven** | driven 5/5: `READER: got 114 bytes` containing the exact 92-byte banner; `REDIRECT(B->F egress)=1` |
| 2 | In-kernel / agent idle (ZERO read of the data on leg B) | **FAIL** | agent-idle 3/3: `REDIRECT=0`, `READER: got 0 bytes`. The redirect requires the agent's `read()` on leg B (kernel-pinned) |
| 3 | kTLS-RX actually engaged (peer sent ciphertext; F_peer got plaintext) | **PASS** | leg B↔P wire: `1703 0300` TLS app_data records, `SERVER_BANNER` cleartext = **0**; on the redirect (driven), `SERVER_BANNER` cleartext on leg F = **1** → genuine decrypt-then-redirect, not passthrough; `ss -tie`: `tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw` on leg B |

**Is #222 fully bidirectionally agent-light? NO.** The **forward** direction
(workload-plaintext → peer-kTLS-TX, increment-f) is agent-light. The **return**
direction (peer-kTLS-RX-decrypt → workload-plaintext) is **NOT**: the agent must
sit in the per-byte path issuing `recvmsg` on the peer-facing kTLS leg to drive
the decrypt-then-redirect. So #222's return path stays **userspace** (the agent
reads decrypted from leg B and writes to leg F — exactly the userspace-copy
baseline `findings-userspace-relay.md` named). #222 is agent-light **one
direction only**.

---

## Anchored on the kernel's own canonical pattern (matched FIRST, not blind-retried)

The brief's load-bearing instruction was to match the kernel's supported kTLS +
sockmap wiring before retrying. The authoritative pattern is
`tools/testing/selftests/bpf/{test_sockmap.c, progs/test_sockmap_kern.h}` — the
`--ktls --txmsg-skb-redir` mode (fetched at the **v7.0** tag via `curl` from
`raw.githubusercontent.com/torvalds/linux/v7.0/…` inside the VM). What it pins:

1. **A DEDICATED verdict-governed sockmap for the kTLS socket** (`tls_sock_map`,
   `map_fd[8]`), distinct from the plaintext sockmap. The kTLS socket (`p2`) is
   inserted into it (`bpf_map_update_elem(map_fd[8], &i, &p2, …)`).
2. **stream_parser (`bpf_prog1`) + stream_verdict (`bpf_prog3`) both attached**
   to that map (`run_options` L1078–1095). The parser is toggleable
   (`txmsg_omit_skb_parser`) — but the selftest's `txmsg_omit_skb_parser=1` runs
   are the **non-kTLS** (`ktls=0`) ones; every **kTLS** skb-redir run keeps the
   parser. (We confirmed empirically *why* — see "Root cause".)
3. **Verdict attached + socket in the map BEFORE kTLS RX is armed.**
   `run_options` attaches the verdict and populates `tls_sock_map[0]=p2`;
   `forward_msg` arms kTLS on `rx_fd` (`sockmap_init_ktls`) **afterwards**. Same
   sockmap-insert-before-kTLS-arm ordering invariant `findings.md` / increment-f
   found for the TX direction, now applied to the RX verdict path.
4. **The verdict runs on the kTLS-DECRYPTED bytes** and calls
   `bpf_sk_redirect_map(skb, &tls_sock_map, idx, flags)`. The selftest's
   `txmsg_ktls_skb_redir` uses `flags=BPF_F_INGRESS` (deliver to the target's RX
   so `rx_fd` reads it); we use **`flags=0` (egress)** so the target (leg F)
   **transmits** the plaintext to its peer (F_peer reader) — the same egress flag
   increment-f proved drives `tcp_sendmsg_locked` on the target.
5. **The consumer drives the redirect with `recvmsg` on the kTLS socket**
   (`msg_loop(rx_fd, …)`, L893). This is the line that turns out to be decisive
   (see "Root cause") — the selftest's redirect happens *inside the rx_fd
   process's read*, never autonomously.

This probe replicates 1–4 exactly (verdict-governed SOCKMAP, parser+verdict,
sockmap-insert-before-kTLS, egress redirect on the decrypted stream). The
agent-idle test omits 5 (the agent does not read leg B) — and that omission is
exactly what makes it fail, which is the finding.

---

## The isolated topology (built exactly; no cgroup/connect4 intercept)

Every socket is agent-controlled; nothing can RST out from under the test (the
confound that blocked `findings-userspace-relay.md`'s egress mode):

- **PEER P** — a rustls TLS 1.3 server that arms **kTLS TX** (ktls crate) and
  **SENDS** `SERVER_BANNER` as TLS 1.3 application_data (ciphertext on the wire).
- **Leg B (kTLS RX+TX, peer-facing)** — the agent's client socket to P. Inserted
  into `SOCKMAP[0]` with the verdict (+parser) attached **BEFORE** `TCP_ULP
  "tls"`; rustls handshake; arm kTLS RX+TX (`rec_seq=0`).
- **Leg F (plaintext target)** — a self-connected TCP pair the agent owns. The
  ACCEPT end (`f_target`) → `SOCKMAP[1]` (the redirect **target**). The CONNECT
  end (`f_peer`) is handed (via `SCM_RIGHTS`) to a separate **reader** process
  that reads the redirected plaintext.
- **The verdict** — a hand-rolled `sk_skb/stream_verdict` on the SOCKMAP. For a
  skb arriving on leg B (matched by `local_port`, post-arm) it calls
  `bpf_sk_redirect_map(skb, &SOCKMAP, F_IDX=1, 0)` — **`flags=0` = EGRESS** —
  driving leg F's TX. For leg F's own RX it `SK_PASS`es. An optional companion
  `sk_skb/stream_parser` (the selftest's `bpf_prog1`) is attached in
  parser+verdict mode (`PARSER=1`).

`DRIVE_LEGB_READ` selects whether the agent stays idle (the agent-light test) or
pumps `read()` on leg B (the control that proves the chain is functional).

(`aya-ebpf 0.1.1` ships no `#[sk_skb]` proc macro — the verdict + parser sections
are hand-rolled via `#[link_section = "sk_skb/stream_verdict"|"…/stream_parser"]`,
the names aya's loader recognises. The redirect helper is the *typed*
`SockMap::redirect_skb` → `bpf_sk_redirect_map`. Both sections confirmed present
in the built object via `llvm-objdump -h`.)

---

## Root cause — kernel-source pinned (the WHY behind the NO)

The kTLS-RX decrypted-verdict path is **pull-driven**, and the pull is a userspace
`recvmsg` the kernel will not issue on the psock's behalf. Three primary-source
facts (read at the v7.0 tag):

1. **The decrypted-verdict runs ONLY inside `tls_sw_recvmsg`.**
   `net/tls/tls_sw.c` `tls_sw_recvmsg`:
   ```c
   bpf_strp_enabled = sk_psock_strp_enabled(psock);          // L2075
   …
   if (bpf_strp_enabled) {
       released = true;
       err = sk_psock_tls_strp_read(psock, skb);             // L2190 — verdict on DECRYPTED skb
       …
   }
   ```
   `sk_psock_tls_strp_read` (`net/core/skmsg.c` L1009) runs
   `psock->progs.stream_verdict` on the decrypted skb and on `__SK_REDIRECT`
   calls `sk_psock_skb_redirect`. This is the ONLY call site of the decrypted
   verdict — and `tls_sw_recvmsg` is the `read()`/`recvmsg()` syscall body.

2. **The decrypted-verdict requires the stream_parser** (`bpf_strp_enabled`).
   `sk_psock_strp_enabled(psock)` = `!!psock->saved_data_ready`
   (`include/linux/skmsg.h` L549). `saved_data_ready` is set by
   `sk_psock_start_strp`, called from the **stream_parser** attach path
   (`sk_psock_init_strp` → `strp_init` → `SK_PSOCK_RX_STRP_ENABLED`). **Without a
   stream_parser, the verdict runs on the raw `skb_verdict`/`sk_psock_verdict_recv`
   path — which sees CIPHERTEXT before kTLS decrypt** (empirically confirmed — see
   "What verdict-only does"). This is exactly why the selftest keeps the parser for
   every kTLS skb-redir run. The brief's hypothesis that verdict-only is the
   likely-correct shape is **falsified for kTLS-RX**: parser+verdict is required.

3. **No autonomous (push) path: `tls_sw_read_sock` refuses a psock.**
   `net/tls/tls_sw.c` `tls_sw_read_sock`:
   ```c
   psock = sk_psock_get(sk);
   if (psock) {
       sk_psock_put(sk, psock);
       return -EINVAL;                                       // <-- bails when a psock is attached
   }
   ```
   `tls_sw_read_sock` is the proto `read_sock` that a BPF-driven autonomous pull
   would use; it deliberately **returns `-EINVAL` when a sockmap psock is
   present**. So when the agent is idle (no `recvmsg` on leg B), nothing drives
   `sk_psock_tls_strp_read`, the decrypted record sits in leg B's kTLS `rx_list`,
   and the verdict never runs → no redirect.

**The asymmetry with the forward direction (increment-f).** The forward kTLS-**TX**
splice is push-driven: the source skb arriving on leg F triggers the
`sk_skb/stream_verdict`, whose `__SK_REDIRECT` calls `tcp_bpf_sendmsg_redir` →
`tcp_bpf_push_locked` → `tcp_sendmsg_locked` on the kTLS-armed target — the kernel
drives the encrypt+transmit inside the redirect, agent idle. The reverse kTLS-**RX**
splice cannot be push-driven because the kTLS decrypt is wired to `tls_sw_recvmsg`
(a syscall body), and the one autonomous entry (`tls_sw_read_sock`) is closed to
psocks. Encrypt-on-egress rides the redirect; decrypt-on-ingress rides a
`recvmsg` the agent must issue.

---

## What verdict-only does (the empirical confirmation of root-cause fact #2)

Run with the brief's hypothesised "likely-correct" verdict-only shape (no
stream_parser): the verdict fired and redirected, but the reader received the
**raw TLS record (ciphertext)** verbatim:

```
READER: got 114 bytes off F_peer: "\u{17}\u{3}\u{3}\0m… (a 17 03 03 00 6d … TLS application_data record)"
READER_RESULT: MISMATCH banner_present=false
stream_verdict: invocations=4 REDIRECT(B->F egress)=1 redir_err=0
```

`17 03 03 00 6d` = a TLS 1.3 application_data record, length 0x6d=109 (92 banner
+ 1 inner-type + 16 GCM tag). The verdict ran on the **pre-decrypt** raw RX
(`sk_psock_verdict_recv`) because `bpf_strp_enabled` was false (no parser → no
`saved_data_ready`). This pins fact #2: **the parser is what routes the verdict to
the decrypted stream.** With the parser attached, the same redirect carries the
**decrypted** plaintext (Assertion 1).

---

## Assertion 1 — decrypt+redirect works (driven): **PASS (5/5 deterministic)**

With the canonical pattern (PARSER=1, sockmap-insert-before-kTLS) and the agent
**pumping `read()` on leg B** during the transfer:

```
READER: got 114 bytes off F_peer:
  "\0\0\0\0\0SERVER_BANNER_ktls_rx_decrypt_must_redirect_to_legF_as_plaintext_agent_idle_return_path_0001\u{17}\0…\0"
stream_verdict: invocations=4 REDIRECT(B->F egress)=1 redir_err=0 PASS(legF/other)=0 PASS_prearm=3 parser_inv=3
```

The exact 92-byte `SERVER_BANNER` plaintext is present in the redirected output,
in order, byte-identical. (The 114 bytes = a 5-byte leading run + the 92-byte
banner + the `0x17` inner-content-type marker + GCM-tag-region padding: the
strparser hands the verdict the whole decrypted TLS-record region; the **banner
payload is intact and correct** — that is the load-bearing fact. A production
design would strip the record framing, trivial in userspace; the kernel splice
redirects the framed decrypted region as-is.) Reproduced **5/5** with the tight
read-pump (a sparse pump hit the verdict-engagement race ~1/4 — increment-f
mechanic #3; a back-to-back read loop eliminates it).

## Assertion 2 — agent idle: **FAIL (3/3 deterministic — the decisive NO)**

With the identical setup but the agent **idle** (no `read()` on leg B — the
agent-light test):

```
stream_verdict: invocations=3 REDIRECT(B->F egress)=0 redir_err=0 PASS(legF/other)=0 PASS_prearm=3 parser_inv=3
READER: got 0 bytes off F_peer: ""
READER_RESULT: MISMATCH banner_present=false len=0 want=92
```

`REDIRECT=0`, reader **0 bytes**, deterministic **3/3**. The `parser_inv=3` are
the handshake-record parses during the pre-arm window; the post-arm data record's
decrypt-and-verdict **never runs** because no `recvmsg` drives `tls_sw_recvmsg`.
The decrypted banner sits undelivered in leg B's kTLS `rx_list`.

**Population comparison** (the diagnosis — only the read-pump differs):

| Variable | Idle (`DRIVE_LEGB_READ=0`) | Driven (`DRIVE_LEGB_READ=1`) |
|---|---|---|
| `REDIRECT(B->F egress)` | **0** (3/3) | **1** (5/5) |
| reader bytes | **0** (3/3) | **114, banner_present=true** (5/5) |
| `SERVER_BANNER` cleartext on leg F | **0** | **1** |

The redirect fires **iff** the agent drives `recvmsg` on leg B. This is the
empirical face of the kernel mechanism above.

## Assertion 3 — kTLS-RX actually engaged: **PASS**

`ss -tie` (the correct kTLS-introspection flag, NOT `ss -K` = `--kill`) on leg B
throughout:

```
tcp-ulp-tls version: 1.3 cipher: aes-gcm-256 rxconf: sw txconf: sw
```

Wire oracle (`tcpdump -i lo`), populations separated (leg B↔P is the peer-facing
wire; leg F is the host-internal target pair):

```
leg B<->P (peer-facing): TLS application_data records (17 03 03 / 17 03 00):
   2  1703 0300     (handshake/Finished + the encrypted banner record)
   1  1703 0302
SERVER_BANNER cleartext on the leg B<->P wire?  count = 0   <<< the banner was ENCRYPTED

driven: SERVER_BANNER cleartext on the leg-F loopback pair?  count = 1   <<< the DECRYPTED, redirected plaintext
idle:   SERVER_BANNER cleartext on the leg-F loopback pair?  count = 0   <<< never redirected
```

The banner crosses the peer-facing wire as ciphertext (count 0 cleartext on leg
B↔P) and — in the driven case only — appears as cleartext on leg F. That is
genuine **decrypt-then-redirect** (kTLS-RX decrypted it; the verdict redirected
the plaintext), not a plaintext passthrough.

---

## Why this is a clean NO where the kernel-canonical pattern was correctly matched

The brief's caution — do not let a wrong-attach-order artifact masquerade as the
kernel verdict — is honoured: the pattern was matched to the selftest *before*
the agent-idle test, and the two prior failure modes were both reproduced and
explained, then the canonical shape was used:

- **`findings-userspace-relay.md`'s `PEER: ConnectionAborted`** was a wrong-attach
  shape: a psock verdict on an already-kTLS-RX socket without the
  insert-before-arm ordering and/or without the parser. This probe matched the
  selftest (parser+verdict, insert-before-arm) and got **no abort** — the
  handshake completes, kTLS RX arms, and the decrypted-verdict path is reachable.
- **Verdict-only (the brief's "likely-correct shape")** redirected **ciphertext**:
  it landed on the raw `sk_psock_verdict_recv` path (pre-decrypt), confirmed by
  the reader receiving a `17 03 03 00 6d …` TLS record verbatim. This empirically
  pins root-cause fact #2 (the parser is what enables the *decrypted*-verdict
  path) — and confirms the selftest keeps the parser for kTLS.
- With the **canonical** shape (parser+verdict), the redirect carries the
  **decrypted** plaintext — but only when the agent reads leg B (facts #1/#3).

So the NO is not a mis-attach: the supported pattern was matched, the chain is
demonstrably functional, and it is the *agent-idle requirement specifically* that
the kernel forecloses for the RX direction. A correctly-attached NO.

---

## Load-bearing mechanics (each cost a debugging detour; for whoever revisits)

1. **kTLS-RX decrypted verdict ⇒ parser REQUIRED.** `bpf_strp_enabled =
   !!psock->saved_data_ready`, set only by the **stream_parser** attach. Without
   the parser the verdict runs on raw ciphertext (`sk_psock_verdict_recv`), not
   the decrypted stream. Verdict-only is the wrong shape for kTLS-RX (it is the
   selftest's *non-kTLS* `txmsg_omit_skb_parser` variant). The selftest keeps the
   parser for every kTLS skb-redir run.

2. **kTLS-RX decrypted verdict ⇒ a userspace read on the kTLS socket REQUIRED.**
   The verdict on the decrypted skb runs only inside `tls_sw_recvmsg`
   (`sk_psock_tls_strp_read`); `tls_sw_read_sock` returns `-EINVAL` when a psock
   is attached, so there is no autonomous pull. The selftest drives it via
   `msg_loop(rx_fd, …)`. **This is the agent-idle killer for the RETURN direction.**

3. **Insert-before-kTLS-arm, RX side too.** Leg B joins the verdict-governed
   sockmap (verdict + parser attached) BEFORE `TCP_ULP "tls"` + `TLS_RX`. Out of
   order, the strparser/verdict and kTLS RX fight (the `ConnectionAborted` shape).

4. **Verdict-engagement race on a freshly-enrolled kTLS socket** (increment-f
   mechanic #3 recurs). A sparse read-pump hits it ~1/4; a back-to-back
   `read()` loop (40 ms timeout, no sleep) keeps the agent continuously in
   `tls_sw_recvmsg` and makes the driven redirect deterministic (5/5).

5. **`strace -f` perturbs the engagement timing** (increment-f mechanic #5
   recurs) — it flipped the driven redirect to a miss. The functional path is
   deterministic only WITHOUT strace; the agent-idle proof here is the
   counter+reader delta (idle 0/0 vs driven 1/1), not a strace histogram. The
   read-driving distinction is the load-bearing evidence, captured by the
   `DRIVE_LEGB_READ` control, no strace needed.

6. **`ss -tie`, not `ss -K`** (`-K` = `--kill`) for kTLS introspection — as
   corrected in `findings-egress-ktls-splice.md`.

---

## Design implications (for DESIGN / Q1)

1. **#222 (host L4 transparent-mTLS proxy) is agent-light in ONE direction only.**
   - **Forward** (workload-plaintext → peer-kTLS-TX): agent-light, kernel splice
     (`findings-egress-ktls-splice.md`, proven 15/15).
   - **Return** (peer-kTLS-RX-decrypt → workload-plaintext): **NOT agent-light** —
     the agent must sit in the per-byte path issuing `recvmsg` on the peer-facing
     kTLS leg to drive the decrypt-then-redirect. **#222's return path stays
     userspace** (the agent reads decrypted from leg B, writes to leg F — the
     userspace-copy baseline). A full kernel splice-out of *both* directions is
     foreclosed on 7.0 by the kTLS-RX pull-driven decrypted-verdict path.

2. **The return path's userspace cost is bounded but real.** The agent stays in
   the read loop on the peer-facing kTLS leg for the connection's life (it
   `recvmsg`s the decrypted bytes and the kernel verdict redirects them to leg F —
   the agent does not itself copy bytes to leg F, but it MUST issue the reads). So
   "agent idle for steady state" holds for the forward direction only; the return
   direction is "agent issuing reads (kernel does the byte movement on redirect),"
   which is lighter than a full userspace copy loop but is NOT idle and keeps the
   agent scheduled per-record on the return path.

3. **This does NOT revive lossless #26.** As with increment-f, the result is about
   redirecting an **agent-owned** kTLS leg, the #222 two-socket shape — not the
   workload's own socket. `findings-userspace-relay.md`'s "#26 lossless collapses
   to #222" stands.

4. **Q1 does NOT move for #26.** v1 for the #26 in-band path stays the
   spike-1-proven lossy DROP-RESET gate. Unchanged from `findings.md` /
   `wave-decisions.md`.

5. **The bidirectional-kTLS-splice open question (increment-f design-implication
   #4) is now ANSWERED: the decrypt-direction splice-out does NOT exist
   agent-idle on 7.0.** The encrypt direction (F→B) is agent-light; the decrypt
   direction (B→F) requires the agent to drive `recvmsg` on leg B. A fully
   agent-idle bidirectional kernel splice would require a kernel patch (a
   push-driven kTLS-RX→sockmap path, or relaxing `tls_sw_read_sock`'s psock
   refusal) — out-of-tree, kernel-version-gated.

---

## Scope / honesty notes

Loopback only; software kTLS (`rxconf: sw txconf: sw`); AES-256-GCM TLS 1.3 only;
agent-as-TLS-client shape; single 92-byte banner (one record). **PROVEN:**
the canonical kTLS-RX + sockmap pattern was matched to `test_sockmap.c --ktls
--txmsg-skb-redir` (v7.0 source, fetched and read); the kTLS-RX decrypt →
stream_verdict → egress redirect → leg F chain is FUNCTIONAL (driven 5/5,
banner byte-exact, `SERVER_BANNER` cleartext on leg F = 1, on leg B↔P = 0);
the chain is NOT agent-idle (idle 3/3: `REDIRECT=0`, 0 bytes) — kernel-source
pinned to `tls_sw_recvmsg` / `sk_psock_tls_strp_read` (decrypted verdict only
inside recvmsg), `sk_psock_strp_enabled` (parser required), and
`tls_sw_read_sock` returning `-EINVAL` on a psock (no autonomous pull); kTLS-RX
genuinely engaged (`ss -tie` tcp-ulp-tls 1.3 aes-gcm-256; ciphertext on the
peer wire). **NOT exercised:** record-framing strip (the 114-vs-92 framing is
left raw — trivial in userspace, not settled here); multi-record / large
transfers; NIC/hardware kTLS offload; the full #222 transparent-intercept
compose (deliberately omitted to isolate the primitive); restart-survival;
`BPF_F_INGRESS` (deliver-to-target's-RX) variant of the kTLS-RX redirect (we
used egress `flags=0` so leg F transmits; the ingress variant would still be
pull-driven by the same recvmsg). The encrypt/forward direction's agent-idle
result is in `findings-egress-ktls-splice.md` and is unchanged. Kernel state
cleaned up after the run: no stray `sk_skb`/`sockmap` (aya auto-detaches on
process exit), no cgroups, no bpffs pins, no XDP; loopback healthy (refused `:1`
fast, did not hang); no stray procs — verified.

**Stopped after findings per the brief — did NOT run the Phase-2 gate or
promote.** `spike-scratch/increment-g-ktls-rx-splice/` left in place (gitignored)
for review.
