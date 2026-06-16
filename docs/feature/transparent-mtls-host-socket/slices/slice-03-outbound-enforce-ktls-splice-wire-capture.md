# Slice 03 — OUTBOUND enforce: kTLS arms on leg B, agent-light forward write_all copy + return zero-copy splice, wire carries TLS 1.3

> **Re-grounded to ADR-0069 (the agent-light L4 proxy).** kTLS arms on the
> **agent's peer-facing leg B**, NOT the workload's socket. "Agent exits" becomes
> "**agent-light** forward `read(legF) → write_all(legB)` COPY into kTLS-TX +
> **agent-light** return `splice(legB → legF)` zero-copy pump." The wire-capture
> observable (TLS 1.3 on the peer-facing wire) is unchanged.
>
> **REVISED 2026-06-13 (D-MTLS-13 — SHIPPED + verified 20/20, commit `bb6489ef`).**
> The forward (F→B) was originally an **agent-idle** in-kernel sockmap EGRESS
> redirect. That mechanism was proven NON-VIABLE — the `sk_skb` egress redirect
> defers delivery to a `MSG_DONTWAIT` workqueue that `-EAGAIN`-stalls ~10–15% of
> records (`docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`)
> — and is REPLACED by an **agent-light bounded `read(legF) → write_all(legB)`
> COPY** into leg B's kTLS-TX (the kernel `tls_sw_sendmsg` encrypts each blocking
> `write`; the agent does ZERO crypto but DOES copy each record's plaintext through
> a userspace buffer — per-record `read`+`write`, **NOT zero-copy, NOT agent-idle,
> NOT symmetric to the return's zero-copy splice**). A `splice` INTO kTLS-TX loses
> records (the same `MSG_DONTWAIT` loss class), so the forward is a synchronous
> blocking `write_all`, not a splice. The whole sockmap apparatus
> (`MTLS_SOCKMAP`/`MTLS_FPORT`/`MTLS_ARMED`, the `sk_skb/stream_verdict` verdict,
> the `sock_ops_mtls_enroll` enroll program, the ARMED gate) and the
> `SOCKMAP`-before-`TCP_ULP` Tier-3 invariant are DELETED. This slice ALSO drains
> 0.5-RTT early application_data from `conn.reader()` before arming kTLS-RX
> (`mtls::drain_early_plaintext`).

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-03

## Goal (one sentence)

After the outbound handshake (Slice 02), the agent installs the rustls handshake's
extracted secrets into kTLS on **leg B** (the agent's peer-facing leg —
auth-session == data-session) and hands steady state to the kernel: **forward**
(F→B) is agent-LIGHT but a COPY (a bounded `read(legF) → write_all(legB)` pump into
leg B's kTLS-TX; the kernel `tls_sw_sendmsg` encrypts each blocking `write`;
per-record `read`+`write`, NOT zero-copy) and **return** (B→F) is agent-light
zero-copy (a bounded `splice(legB → legF)` pump on a plain kTLS-RX leg, ~1 splice
per record) — and a `tcpdump` capture on the peer-facing wire proves TLS 1.3
records. (D-MTLS-13: the forward is NOT symmetric to the return — a splice into
kTLS-TX loses records, so the forward is a `write_all` copy; the agent-idle sockmap
egress redirect was retired.)

## IN scope

- Installing the rustls handshake's extracted secrets into kTLS on **leg B**
  (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — the auth-session's secrets
  (auth-session == data-session), not a separately negotiated session.
- The **kTLS 0.5-RTT early-data drain** (D-MTLS-13): before arming kTLS-RX, drain
  `conn.reader()` of any 0.5-RTT early application_data the peer already sent and
  forward it ahead of the splice pump (`mtls::drain_early_plaintext`) — the `rx`
  `rec_seq` already accounts for the over-read records, so it would otherwise be
  dropped.
- **Forward steady state (F→B), agent-light COPY** (D-MTLS-13): the agent drives a
  bounded `read(legF) → write_all(legB)` COPY into leg B's **kTLS-TX**; the kernel
  `tls_sw_sendmsg` encrypts each blocking `write` → encrypted egress. The agent
  does ZERO crypto, but it DOES copy each record's plaintext through a userspace
  buffer and issues a `read`+`write` per record — **NOT zero-copy, NOT agent-idle,
  NOT symmetric to the return** (a `splice` INTO kTLS-TX loses records, so the
  forward is a synchronous blocking `write_all`). **Replaced the agent-idle sockmap
  egress redirect (`MSG_DONTWAIT`-stall, retired 2026-06-13;
  `crates/overdrive-dataplane/src/mtls/splice.rs`).**
- **Return steady state (B→F), agent-light zero-copy**: leg B is a **plain kTLS-RX
  socket with NO sockmap/psock**; the agent drives a bounded
  `splice(legB → pipe → legF)` pump, `tls_sw_splice_read` decrypting each record
  into clean plaintext (~1 splice per record, no userspace copy).
- `ss -tie` showing the kTLS ULP installed on leg B (`tcp-ulp-tls version: 1.3
  cipher: aes-gcm-256 rxconf:sw txconf:sw`).
- A `tcpdump` capture on the peer-facing wire showing TLS 1.3 Application Data
  records (content type 0x17) and NEVER the cleartext payload — **the K1
  North-Star observable** (the workload's plaintext lives only on the
  host-internal leg F, by design).

## OUT scope

- The INBOUND (server) enforce path (TPROXY → server-mTLS → kTLS-RX →
  splice-to-server) → Slice 04.
- Fail-closed negatives, resource limits, pump supervision, intercept-exemption
  negatives, the authn-vs-authz boundary → Slice 05.
- The kTLS `crypto_info` struct construction details are DESIGN's to pin (the slice
  pins the observable, not the struct).

## Learning hypothesis

- **Disproves if it fails**: "the negotiated session installs into kTLS on the
  agent's peer-facing leg B (auth-session == data-session), the kernel carries BOTH
  the forward path (agent-light `read → write_all` COPY into kTLS-TX) and the return
  path (agent-light zero-copy `splice` on a plain kTLS-RX leg), the 0.5-RTT early data is not
  dropped, and the wire carries TLS 1.3 records." If the install fails, a splice
  pump breaks, early data is lost, or the wire shows cleartext, the "in-kernel,
  agent-light" property (the thing distinguishing Overdrive from ztunnel) does not
  hold.
- **Confirms if it succeeds**: the North-Star observable (TLS 1.3 on the wire,
  in-kernel, agent-light) is achieved for the outbound half; Slice 04 mirrors it
  inbound and Slice 05 adds the guardrails.

## Acceptance criteria

- [ ] The agent installs the rustls handshake's extracted secrets into kTLS on **leg B** (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — auth-session == data-session, not a separately negotiated session. Anchor: `findings.md` A.
- [ ] **kTLS 0.5-RTT early-data drain** (D-MTLS-13): the 0.5-RTT early application_data the peer sent before the agent's first record reaches leg F byte-exact, in order (drained from `conn.reader()` before kTLS-RX arm; `rx` rec_seq accounts for the over-read). Anchor: shipped `mtls::drain_early_plaintext`.
- [ ] **Forward agent-light** (D-MTLS-13): `tcpdump` on the peer-facing wire shows `1703 03` (0x17) records; `strace` shows the agent moves forward bytes via `splice`/`ppoll` ONLY (zero per-byte `read`/`write` of plaintext), ~1 `splice` per TLS record; leg B carries NO psock on its TX. Anchor: `findings-splice-return.md` (symmetric splice) / `sockmap-egress-redirect-into-ktls-tx-delivery-research.md` (why the redirect was retired).
- [ ] **Return agent-light**: `strace` shows ONLY `splice`/`ppoll` (zero payload `read`/`write`), byte-exact plaintext on leg F, ~1 `splice` per TLS record, `einval_on_B=0`. Anchor: `findings-splice-return.md`.
- [ ] `ss -tie` shows the kTLS ULP on leg B (`tcp-ulp-tls 1.3 aes-gcm-256 rxconf:sw txconf:sw`) — NOT `ss -K` (which is `--kill`). Anchor: `findings.md` A.
- [ ] A `tcpdump` capture on the peer-facing wire shows TLS 1.3 Application Data records (0x17) and NEVER the cleartext payload; cleartext count on leg B = 0 (the workload's plaintext is on leg F, host-internal, by design) — the K1 North-Star observable. Anchor: `findings-userspace-relay.md` Unknown 2 / `findings-splice-return.md`.
- [ ] (RETIRED 2026-06-13, D-MTLS-7/13) ~~`SOCKMAP`-insert AFTER `TCP_ULP "tls"` returns `EINVAL`~~ — no sockmap insert on any path now; this invariant is no longer tested.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the outbound wire-capture + cost-tier acceptance tests (real kernel, not `--no-run`).

## Dependencies

- Slice 02 (the completed outbound handshake's extracted secrets) + Slice 01 (leg F / leg B).
- In-kernel TLS 1.3 TX+RX (`tls_sw_sendmsg` encrypt-on-splice) + `tls_sw_splice_read` on the 6.18 kernel (ADR-0068). (No sockmap dependency — D-MTLS-13.)
- The Lima/LVH harness with `tcpdump` + `ss` + `strace` on the real kernel.

## Effort estimate

~1 day (≤6h). Reference class: the kTLS install + the agent-light `splice` pump
(both directions) + the early-data drain were proven (`findings-splice-return.md`
+ the shipped `crates/.../mtls/`); the new part is composing them on leg B behind
the real intercept + the wire-capture/cost-tier acceptance harness. (D-MTLS-13:
the forward agent-light splice REPLACES the original sockmap-egress-redirect —
that mechanism was proven non-viable, `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`.)

## Pre-slice SPIKE

Not needed — the agent-light `splice` pump (both directions), the early-data
drain, and the `ss -tie` kTLS ULP are all proven on the real kernel (and SHIPPED
20/20, commit `bb6489ef`). This slice productionises them on leg B behind the
intercept. (The original sockmap-egress-redirect forward path was retired,
D-MTLS-13.)

## Taste-test note

The outbound North-Star slice: ships the kTLS-TX arm on leg B + the agent-light
forward `read → write_all` copy + return agent-light zero-copy `splice` + the
early-data drain + the headline wire capture. Production-data observable (real
`tcpdump` TLS 1.3 records + `ss -tie` ULP + strace agent-light — Tier-3, the
security-reviewer-facing proof).
Disproves a real assumption (the encryption is in-kernel with the agent
AGENT-LIGHT, not a userspace proxy). One value story (US-MTLS-03); the observable
IS the value (principle 3 made real on the wire, outbound) — a headline
deliverable, not infra-only.
