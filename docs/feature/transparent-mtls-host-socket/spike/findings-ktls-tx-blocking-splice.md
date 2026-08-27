# Spike findings — blocking `splice(2)` INTO a kTLS-TX socket delivers losslessly (increment M)

> **nw-spike Phase-1 PROBE — THROWAWAY, real-kernel, NOT promoted.** TX-direction mirror of
> increment-h (`findings-splice-return.md`). Confirms-or-refutes the unprobed closing
> recommendation of
> `docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`
> ("ideally via `splice()` for zero userspace copy" — blocking — never tested).

**GH**: [#26](https://github.com/overdrive-sh/overdrive/issues/26) (transparent mTLS host socket).
**Date**: 2026-08-26. **Kernel**: 7.0.0-29-generic, Ubuntu 26.04 LTS, Lima VM `overdrive`, aarch64.
**Scope**: loopback only; software kTLS; AES-256-GCM TLS 1.3 only; agent-as-TLS-client;
single-record (86 B) + multi-record (100 000 B) + backpressure-stress (1 000 000 B,
`SO_SNDBUF=2048`, slow peer reader) payloads.
**Probe code** (committed per the 2026-08-11 ruling): `spike-scratch/increment-m-ktls-tx-splice/`.
strace evidence: `spike-scratch/increment-m-ktls-tx-splice/evidence-strace-run2.txt`.

---

## VERDICT

**WORKS — a blocking `splice(legF → pipe → legB kTLS-TX)` pump delivers application bytes
losslessly and byte-exact to the TLS peer, 15/15 under the exact send-buffer-exhaustion
condition where the retired non-blocking (`MSG_DONTWAIT`) paths lost ~10–15% of records.**
`eagain=0` on every run; no `-EINVAL` on the kTLS leg; zero short splice-out returns; no stalls.

---

## Hypothesis / prediction / falsification (as dispatched, verbatim)

- **Hypothesis**: userspace splice into a kTLS-TX socket rides `sendmsg(MSG_SPLICE_PAGES)`
  (kernel ≥6.5 path; sendpage removed), and with blocking semantics `tls_sw_sendmsg` waits for
  send-buffer space — the `MSG_DONTWAIT` loss class structurally cannot fire.
- **Predicted**: every run ends `PEER_RESULT: EXACT`; `eagain=0`; no mid-stream zero returns;
  run 3 is 15/15 EXACT.
- **Falsified if**: any MISMATCH/short delivery at P, any `WATCHDOG: STALL`, or splice returning
  `-1 EINVAL` on the kTLS leg (which would mean the kernel refuses splice-to-kTLS — a decisive
  DOESN'T-WORK worth recording precisely).

**Outcome: the prediction held in full.** 20/20 executed runs (1 + 1 + 15 + 3) ended
`PEER_RESULT: EXACT`; `eagain=0` and `eintr=0` everywhere; no watchdog fire; no `EINVAL`.

## Topology

```
W (writer thread, plain TCP client) ──▶ legF_peer (accepted socket, agent-owned)
      pump (main thread):  splice(legF_peer → pipe[1], SPLICE_F_MOVE)   [BLOCKING]
                           splice(pipe[0]   → legB,    SPLICE_F_MOVE)   [BLOCKING]  ← kernel encrypts
legB (agent's TCP client to P, kTLS TLS_TX armed, AES-256-GCM TLS 1.3, BLOCKING,
      optional small SO_SNDBUF set BEFORE connect)
      ──0x17 ciphertext──▶  P (rustls TLS 1.3 SERVER thread, suite PINNED to
                               TLS13_AES_256_GCM_SHA384, byte-compares to EOF)
```

- No `O_NONBLOCK`, no `SPLICE_F_NONBLOCK`, no `MSG_DONTWAIT` anywhere — the mechanism under
  test is *blocking* splice, the exact semantics the deferred-workqueue redirect path could
  never have (research Finding 2: `__skb_send_sock` hardcodes `MSG_DONTWAIT` per fragment).
- The kTLS arm is the production `arm_ktls_tx_rx` logic COPIED from
  `crates/overdrive-dataplane/src/mtls/ktls.rs` (error type adapted; **TLS_TX only** armed —
  the agent never reads leg B). `send_tls13_tickets = 0` mirrored from production
  `tls_config.rs`.
- P does NOT arm kTLS: its rustls session successfully decoding every record IS the proof the
  kernel emitted well-formed TLS 1.3 ciphertext.
- Writer W sends `byte[i] = i % 251` in odd cycling chunks (1, 777, 1777, 13, 4096, 25, 999),
  then `shutdown(SHUT_WR)`, and holds the socket open until the run completes.
- Every run carries an in-program watchdog (30 s; 60 s for the 1 MB stress runs) that exits 2
  with a tally dump on stall — no run may hang the harness.

---

## Run 1 — 86 B single record (parity with increment-h), defaults

```
RUN: mode=splice payload=86 sndbuf=None slow_reader=false watchdog=30s
KTLS: armed TLS_TX only (tx_seq=0) suite=TLS13_AES_256_GCM_SHA384
PEER_RESULT: EXACT n=86
PUMP: mode=splice splice_in calls=1 bytes=86  splice_out calls=1 bytes=86 eagain=0 eintr=0
RUN_RESULT: PASS
```

One splice in, one splice out, byte-exact — the single-record shape increment-h proved for the
RX direction holds for TX.

## Run 2 — 100 000 B multi-record, defaults

```
RUN: mode=splice payload=100000 sndbuf=None slow_reader=false watchdog=30s
KTLS: armed TLS_TX only (tx_seq=0) suite=TLS13_AES_256_GCM_SHA384
PEER_RESULT: EXACT n=100000
PUMP: mode=splice splice_in calls=2 bytes=100000  splice_out calls=2 bytes=100000 eagain=0 eintr=0
RUN_RESULT: PASS
```

`splice_in` granularity is the 64 KiB default pipe capacity (65 536 + 34 464 = 100 000), and —
notable — each `splice(pipe → legB)` landed its **full** requested length in ONE call: the
kernel consumed 64 KiB into kTLS-TX per blocking splice (multiple 16 KiB TLS records built
inside one sendmsg).

## Run 3 — 15× 1 000 000 B, `SNDBUF=2048 SLOW_READER=1` (the adversarial buffer-full case)

The condition under which the retired redirect lost ~10–15% of records probabilistically. The
prior arc's bar is a 15/15 reproduction. All 15 runs printed identically (runs 2–14 elided
here as byte-identical to runs 1/15; reproducible via the loop below):

```
=== STRESS RUN 1 ===
RUN: mode=splice payload=1000000 sndbuf=Some(2048) slow_reader=true watchdog=60s
KTLS: armed TLS_TX only (tx_seq=0) suite=TLS13_AES_256_GCM_SHA384
PEER_RESULT: EXACT n=1000000
PUMP: mode=splice splice_in calls=16 bytes=1000000  splice_out calls=16 bytes=1000000 eagain=0 eintr=0
RUN_RESULT: PASS
   …
=== STRESS RUN 15 ===
RUN: mode=splice payload=1000000 sndbuf=Some(2048) slow_reader=true watchdog=60s
KTLS: armed TLS_TX only (tx_seq=0) suite=TLS13_AES_256_GCM_SHA384
PEER_RESULT: EXACT n=1000000
PUMP: mode=splice splice_in calls=16 bytes=1000000  splice_out calls=16 bytes=1000000 eagain=0 eintr=0
RUN_RESULT: PASS
STRESS_TALLY: 15/15 PASS
```

**15/15 `PEER_RESULT: EXACT n=1000000`.** Reproduction command (inside Lima, as root):

```
for i in $(seq 1 15); do SNDBUF=2048 SLOW_READER=1 WATCHDOG_SECS=60 ./target/release/probe 1000000; done
```

With `SO_SNDBUF=2048` (kernel-clamped to its floor) and a 3 ms-per-4 KiB peer reader, every
16 KiB TLS record push necessarily hits the buffer-full wait — and the blocking sendmsg
*waited* instead of dropping: `eagain=0`, `splice_out bytes=1000000`, zero loss, on all 15
runs. This is the structural absence of the `MSG_DONTWAIT` loss class the hypothesis predicted.

## Run 4 — COPY control (the production `read → write_all` shape), same adversarial params, 3×

```
=== COPY CONTROL RUN 1 ===
RUN: mode=copy payload=1000000 sndbuf=Some(2048) slow_reader=true watchdog=60s
KTLS: armed TLS_TX only (tx_seq=0) suite=TLS13_AES_256_GCM_SHA384
PEER_RESULT: EXACT n=1000000
PUMP: mode=copy splice_in calls=17 bytes=1000000  splice_out calls=17 bytes=1000000 eagain=0 eintr=0
RUN_RESULT: PASS
   … (runs 2, 3 identical modulo calls=16/17)
COPY_TALLY: 3/3 PASS
```

Equivalence baseline: the blocking splice pump and the production blocking COPY pump are
delivery-equivalent under the adversarial condition. The splice pump buys the removal of the
per-byte userspace copy, not a delivery difference.

## Run 5 — strace agent-light proof (shape 2 under `strace -f -tt`)

`strace -f -e trace=read,write,splice,sendmsg,recvmsg,sendto,recvfrom` on the 100 000 B splice
run (evidence file: `spike-scratch/increment-m-ktls-tx-splice/evidence-strace-run2.txt`).
The run under strace still ended `PEER_RESULT: EXACT n=100000` (`splice_in calls=49` — strace
timing perturbation de-coalesces the writer's chunks; granularity varies, losslessness does
not). Everything the pump TID (2637421) did in the transfer window (first `splice` → exit):

```
2637421 23:05:21.254730 splice(8, NULL, 10, NULL, 65536, SPLICE_F_MOVE <unfinished ...>
2637421 23:05:21.255028 <... splice resumed>) = 10243
2637421 23:05:21.255599 splice(9, NULL, 5, NULL, 10243, SPLICE_F_MOVE <unfinished ...>
2637421 23:05:21.256128 <... splice resumed>) = 10243
   … (splice pairs continue; no other syscall shapes appear) …
2637421 23:05:21.314677 splice(9, NULL, 5, NULL, 56, SPLICE_F_MOVE) = 56
2637421 23:05:21.315076 splice(8, NULL, 10, NULL, 65536, SPLICE_F_MOVE) = 0     ← legF EOF
2637421 23:05:21.318604 write(1, "PUMP: mode=splice splice_in call"..., 101) = 101
2637421 23:05:21.319667 write(1, "RUN_RESULT: PASS\n", 17) = 17
2637421 23:05:21.321224 +++ exited with 0 +++
```

Per-syscall totals for the pump TID in the window:

```
     99 splice     ← 49 in + 49 out + 1 EOF-returning splice(=0)
      2 write      ← BOTH write(1, …) stdout logging (pasted above), NOT socket payload
```

**In the transfer window the pump thread issued ONLY `splice()` on the payload path** — zero
`read`/`recvmsg`/`recvfrom`/`sendmsg`/`sendto` and zero socket `write` of payload bytes. The
mirror of increment-h's ASSERTION 1, TX direction.

---

## Edge cases observed

- **splice-out never returns short.** Even with `SNDBUF=2048` + slow reader, every
  `splice(pipe → legB)` consumed its full requested length (up to 64 KiB) in one call — the
  blocking `tls_sw_sendmsg` waits for send-buffer space *inside* the call. The probe's inner
  drain loop (loop on short positive returns) never iterated; keep it anyway as a defensive
  invariant, since nothing in the splice contract forbids a short return.
- **Splice granularity is arrival-paced, not record-paced.** Non-strace runs coalesced to
  64 KiB per splice-in (pipe capacity); under strace the same payload took 49 smaller
  splice-ins (per-chunk arrival). Cost tier: ~1 splice-in + 1 splice-out per readiness event,
  bounded by pipe capacity — kernel-paced, never per-byte.
- **EOF ordering**: `shutdown(legB, SHUT_WR)` after legF EOF sends a bare FIN — **no TLS
  close_notify** (nothing userspace holds the TLS state anymore; kTLS does not synthesize one
  on shutdown). The rustls peer surfaces this as `UnexpectedEof`, which the probe (and any
  consumer) must treat as EOF. The production pumps already live with this shape.
- **No `EINVAL` on splice-to-kTLS.** The kernel ≥6.5 `splice_to_socket → sendmsg(MSG_SPLICE_PAGES)`
  path accepts a kTLS-TX socket as a splice destination on 7.0.0-29-generic. (The retired
  hazard was loss under `MSG_DONTWAIT`, never a refusal.)
- **Ticket suppression matters for hygiene**: with `send_tls13_tickets = 0` (mirroring
  production) nothing lands unread in legB's RX queue; the probe armed TX-only and never reads
  leg B.

## Design implications (on promote)

Swapping the two production COPY pumps in `crates/overdrive-dataplane/src/mtls/` — forward
(`legF → legB`, `PumpHandle::spawn_encrypt`) and response (`legS → legC`) — for this blocking
splice pump would make all four pump directions the same primitive (splice), removing the
per-byte userspace copy on the encrypt side:

- The **pre-arm prelude** bytes (captured before the kTLS arm) are an in-memory `Vec` and
  cannot ride `splice`; the encrypt pump would still `write_all` the prelude first (single
  writer per leg preserved), then splice steady-state. The single-writer-per-kTLS-leg
  discipline in `outbound.rs` step 6 carries over unchanged.
- Three stale claims need correcting in the same change (per
  `feedback_behavior_change_must_mark_stale_adjacent_docs`):
  - `crates/overdrive-dataplane/src/mtls/mod.rs:21-23` — "a `splice` INTO a kTLS-TX socket
    loses records the same way" (true only for NON-blocking splice; unqualified as written).
  - `crates/overdrive-dataplane/src/mtls/outbound.rs:11-12` — "A `splice` INTO a kTLS-TX
    socket is NOT used — it loses records the same way the abandoned sockmap egress redirect
    did" (same missing qualifier).
  - `website/content/docs/concepts/transparent-mtls.mdx:76-79` — "a `splice` into a kTLS-TX
    leg reports success but silently drops records under `MSG_DONTWAIT`, so the blocking
    `write_all` is the only primitive that delivers every record" — the `MSG_DONTWAIT`
    qualifier is accurate, but "the only primitive" is now falsified: blocking splice also
    delivers every record.
- The research doc's closing recommendation ("ideally via `splice()`") is now validated rather
  than speculative for the measured kernel.

## Kernel caveat

Measured on the dev-VM kernel **7.0.0-29-generic** (aarch64, Lima), NOT the pinned 6.18
appliance kernel (ADR-0068). The verdict is pinned to the measured kernel. The
`splice → sendmsg(MSG_SPLICE_PAGES)` path exists since 6.5 (sendpage removal) and 6.18 > 6.5,
but 6.18 confirmation would happen at Tier 3 if promoted — same caveat discipline as
increment-h.

## Gate recommendation

**PROMOTE** — blocking splice into kTLS-TX is validated lossless 15/15 under the exact
adversarial condition that killed the redirect path; it is delivery-equivalent to the shipped
COPY pump while removing the per-byte userspace copy, and the swap is a bounded change to the
two encrypt pumps plus three named doc corrections.
