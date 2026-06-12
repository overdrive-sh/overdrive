# Slice 03 — OUTBOUND enforce: kTLS arms on leg B, forward agent-idle splice + return agent-light splice, wire carries TLS 1.3

> **Re-grounded to ADR-0069 (the agent-light L4 proxy).** kTLS arms on the
> **agent's peer-facing leg B**, NOT the workload's socket. "Agent exits" becomes
> "forward **agent-idle** sockmap splice + return **agent-light** `splice` pump."
> The wire-capture observable (TLS 1.3 on the peer-facing wire) is unchanged.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-03

## Goal (one sentence)

After the outbound handshake (Slice 02), the agent installs the rustls handshake's
extracted secrets into kTLS on **leg B** (the agent's peer-facing leg —
auth-session == data-session) and hands steady state to the kernel: **forward**
(F→B) is agent-idle (an in-kernel sockmap EGRESS redirect into the kTLS-armed leg
B → encrypted egress) and **return** (B→F) is agent-light (a bounded `splice` pump
on a plain kTLS-RX leg, ~1 splice per record) — and a `tcpdump` capture on the
peer-facing wire proves TLS 1.3 records.

## IN scope

- Installing the rustls handshake's extracted secrets into kTLS on **leg B**
  (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — the auth-session's secrets
  (auth-session == data-session), not a separately negotiated session.
- **Forward steady state (F→B), agent-idle**: an in-kernel sockmap **EGRESS**
  redirect (`bpf_sk_redirect_map`, `flags=0`) from leg F's RX into leg B's kTLS-TX
  drives `tcp_sendmsg_locked` → encrypted egress; the agent issues ZERO per-byte
  syscalls.
- **Return steady state (B→F), agent-light**: leg B is a **plain kTLS-RX socket
  with NO sockmap/psock**; the agent drives a bounded `splice(legB → pipe → legF)`
  pump, `tls_sw_splice_read` decrypting each record into clean plaintext (~1 splice
  per record).
- `ss -tie` showing the kTLS ULP installed on leg B (`tcp-ulp-tls version: 1.3
  cipher: aes-gcm-256 rxconf:sw txconf:sw`).
- A `tcpdump` capture on the peer-facing wire showing TLS 1.3 Application Data
  records (content type 0x17) and NEVER the cleartext payload — **the K1
  North-Star observable** (the workload's plaintext lives only on the
  host-internal leg F, by design).
- The arming-order invariant (Tier-3): `SOCKMAP`-insert AFTER `TCP_ULP "tls"`
  returns `EINVAL` (both replace `sk->sk_prot`) — the natural detect→gate→install
  order passes; the reverse must fail.

## OUT scope

- The INBOUND (server) enforce path (TPROXY → server-mTLS → kTLS-RX →
  splice-to-server) → Slice 04.
- Fail-closed negatives, resource limits, pump supervision, intercept-exemption
  negatives, the authn-vs-authz boundary → Slice 05.
- The kTLS `crypto_info` struct construction details are DESIGN's to pin (the slice
  pins the observable, not the struct).

## Learning hypothesis

- **Disproves if it fails**: "the negotiated session installs into kTLS on the
  agent's peer-facing leg B (auth-session == data-session), the kernel carries the
  forward path agent-idle (sockmap egress redirect → kTLS-TX) and the return path
  agent-light (`splice` on a plain kTLS-RX leg), and the wire carries TLS 1.3
  records." If the install fails, the forward path is not agent-idle, the return
  `splice` breaks, or the wire shows cleartext, the "in-kernel, agent-light"
  property (the thing distinguishing Overdrive from ztunnel) does not hold.
- **Confirms if it succeeds**: the North-Star observable (TLS 1.3 on the wire,
  in-kernel, agent-light) is achieved for the outbound half; Slice 04 mirrors it
  inbound and Slice 05 adds the guardrails.

## Acceptance criteria

- [ ] The agent installs the rustls handshake's extracted secrets into kTLS on **leg B** (`setsockopt TCP_ULP "tls"` + `TLS_TX/TLS_RX`) — auth-session == data-session, not a separately negotiated session. Anchor: `findings.md` A / `findings-egress-ktls-splice.md` mechanic #4.
- [ ] **Forward agent-idle**: `tcpdump` on the peer-facing wire shows `1703 03` (0x17) records and the agent issues ZERO per-byte syscalls (strace), `redir_err=0`. Anchor: `findings-egress-ktls-splice.md` (15/15 deterministic; strace proof).
- [ ] **Return agent-light**: `strace` shows ONLY `splice`/`ppoll` (zero payload `read`/`write`), byte-exact plaintext on leg F, ~1 `splice` per TLS record, `einval_on_B=0`. Anchor: `findings-splice-return.md`.
- [ ] `ss -tie` shows the kTLS ULP on leg B (`tcp-ulp-tls 1.3 aes-gcm-256 rxconf:sw txconf:sw`) — NOT `ss -K` (which is `--kill`). Anchor: `findings.md` A.
- [ ] A `tcpdump` capture on the peer-facing wire shows TLS 1.3 Application Data records (0x17) and NEVER the cleartext payload; cleartext count on leg B = 0 (the workload's plaintext is on leg F, host-internal, by design) — the K1 North-Star observable. Anchor: `findings-userspace-relay.md` Unknown 2 / `findings-egress-ktls-splice.md` Assertion 1.
- [ ] (Tier-3 invariant) `SOCKMAP`-insert AFTER `TCP_ULP "tls"` returns `EINVAL` (the reverse, natural, order passes). Anchor: `findings.md` Increment D.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the outbound wire-capture + cost-tier acceptance tests (real kernel, not `--no-run`).

## Dependencies

- Slice 02 (the completed outbound handshake's extracted secrets) + Slice 01 (leg F / leg B).
- In-kernel TLS 1.3 TX+RX + sockmap egress redirect + `tls_sw_splice_read` on the 6.18 kernel (ADR-0068).
- The Lima/LVH harness with `tcpdump` + `ss` + `strace` on the real kernel.

## Effort estimate

~1 day (≤6h). Reference class: the kTLS install + forward sockmap-egress-redirect +
return `splice` were each proven in the spikes (`findings-egress-ktls-splice.md` /
`findings-splice-return.md`); the new part is composing them on leg B behind the
real intercept + the wire-capture/cost-tier acceptance harness.

## Pre-slice SPIKE

Not needed — the spikes exercised the forward agent-idle sockmap-egress-redirect →
kTLS-TX, the return agent-light `splice`, the `ss -tie` kTLS ULP, and the
`SOCKMAP`-before-`TCP_ULP` invariant on the real kernel. This slice productionises
them on leg B behind the intercept.

## Taste-test note

The outbound North-Star slice: ships the kTLS-TX arm on leg B + forward agent-idle
sockmap splice + return agent-light `splice` + the headline wire capture.
Production-data observable (real `tcpdump` TLS 1.3 records + `ss -tie` ULP + strace
agent-idle/agent-light — Tier-3, the security-reviewer-facing proof). Disproves a
real assumption (the encryption is in-kernel with the agent AGENT-LIGHT, not a
userspace proxy). One value story (US-MTLS-03); the observable IS the value
(principle 3 made real on the wire, outbound) — a headline deliverable, not
infra-only.
