# Slice 04 — INBOUND enforce: orig-dst → server-mTLS → kTLS-RX → agent-light splice-to-server

> **New INBOUND slice (F3).** Re-grounded to ADR-0069 (the BIDIRECTIONAL
> agent-light L4 proxy). The inbound/server half is first-class in host-socket v1
> and fully spike-proven (`findings-inbound-intercept.md`, increment-i, kernel
> 7.0). Every AC is anchored on a proven observable from §1–§5.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-04

## Goal (one sentence)

After the inbound intercept (Slice 01) selects the server workload's identity from
the TPROXY-recovered original destination, the agent completes the server-side
mutual-TLS handshake (Slice 02 server role), arms kTLS-RX on **leg C** (the
agent's client-facing leg), and **`splice`s the decrypted plaintext to the
identity-unaware server workload** (leg S, agent-light) — so the server workload
reads the byte-exact request as plaintext while the client-facing wire carries
TLS `0x17` ciphertext only.

## IN scope

- **Original-destination → identity selection**: the TPROXY-recovered original
  destination (`getsockname()` on leg C, Slice 01) selects the **server**
  workload's `AllocationId` → its held SVID via `IdentityRead`.
- **Server-side mutual-TLS** on leg C: present the server SVID, `WebPkiClientVerifier`
  REQUIRE+VERIFY the client's presented SVID chains to `IdentityRead::current_bundle()`,
  `dangerous_extract_secrets` → arm **kTLS-RX** (+ TX for the response leg).
- **Agent-light deliver (C→S)**: leg C is a plain (no-psock) kTLS-RX leg; the agent
  drives a bounded `splice(legC → pipe → legS)` pump, `tls_sw_splice_read`
  decrypting each record → clean plaintext to the server workload S (the same
  return-direction primitive the outbound half uses, applied to the request
  direction).
- **Byte-exact plaintext at S**: S reads the byte-exact request as plaintext (S
  holds no cert/key, is identity-unaware); the client-facing leg carries TLS
  `0x17` app_data only; the decrypted plaintext appears ONLY on the agent→S leg.
- The two server-config mechanics: suppress `NewSessionTicket`
  (`send_tls13_tickets = 0`); read `peer_certificates()` for the fail-closed guard
  BEFORE `dangerous_extract_secrets()` consumes the connection.

## OUT scope

- The OUTBOUND (client) enforce path (kTLS-TX on leg B + agent-light forward
  `read → write_all` copy + return agent-light zero-copy splice) → Slice 03.
  (D-MTLS-13: the forward is an agent-light `write_all` copy into kTLS-TX, NOT a
  splice into kTLS-TX and NOT the retired sockmap egress redirect.)
- The inbound fail-closed negatives (nocert / wrongca) — the dedicated negative ACs
  → Slice 05 (the verifier REQUIRE+VERIFY is wired here; the dedicated negative
  proofs with distinct reasons are S05; the composed WS Slice 00 already touches
  the happy inbound path).
- Resource limits / pump supervision / the authn-vs-authz boundary → Slice 05.
- The server's response leg (re-encrypt S's reply onto leg C's kTLS-TX)
  steady state — reuses the outbound forward primitive (the agent-light
  `read(legS) → write_all(legC)` COPY into kTLS-TX, NOT a splice into kTLS-TX,
  D-MTLS-13); the spike did NOT exercise
  it (`findings-inbound-intercept.md` § "What was NOT tested"), so it is composed in
  Slice 00 (the WS bidirectional transfer) and productionised here only to the
  extent the WS requires.

## Learning hypothesis

- **Disproves if it fails**: "the inbound half works agent-light — orig-dst →
  server-SVID selection → server-side mutual-TLS (verify the client SVID) → kTLS-RX
  arm → `splice` the decrypted plaintext to the identity-unaware server workload,
  byte-exact." If the server workload does not read the byte-exact plaintext, the
  client leg leaks request cleartext, or the agent copies payload bytes in
  userspace, the inbound enforcement (and the "BIDIRECTIONAL v1" claim) does not
  hold.
- **Confirms if it succeeds**: the inbound/server half is productionised on the
  real kernel; combined with the outbound half (S03) the agent-light proxy
  enforces both directions.

## Acceptance criteria

- [ ] **Orig-dst → identity**: the TPROXY-recovered original destination selects the server workload's `AllocationId` → its held SVID via `IdentityRead`. Anchor: `findings-inbound-intercept.md` §1 + § "Design implications" #3.
- [ ] **Server-mTLS**: the agent presents the server SVID and `WebPkiClientVerifier` REQUIRE+VERIFY the client's SVID chains to the bundle; a valid client cert → handshake succeeds, kTLS-RX armed (`ss -tie` `rxconf:sw`). Anchor: `findings-inbound-intercept.md` §2/§3 (`MTLS_OK client_auth=VERIFIED ktls_rx=ARMED`).
- [ ] **Byte-exact plaintext to S**: the server workload S reads the byte-exact request as plaintext; the client-facing leg carries `0x17` app_data only (cleartext-marker hits on the client leg = 0); the decrypted plaintext appears ONLY on the agent→S leg. Anchor: `findings-inbound-intercept.md` §3 (`PLAINTEXT_EXACT`; client leg `0x17` records=2, cleartext-hits=0).
- [ ] **Agent-light**: `strace` shows the agent moves the inbound payload via `splice`/`ppoll` only — zero per-byte `read`/`write`/`recv`/`send` of the payload; leg C carries no psock on its RX (same plain-kTLS-RX invariant as the outbound return). Anchor: `findings-inbound-intercept.md` §5 (`splice_in=1 splice_out=1`, no payload socket I/O).
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the inbound enforce acceptance test (real kernel, not `--no-run`). The loopback-only spike topology is re-proven in the real netns/veth topology (`findings-inbound-intercept.md` § "What was NOT tested").

## Dependencies

- Slice 01 (the inbound TPROXY intercept + recovered orig-dst) + Slice 02 (the
  server-role handshake).
- The shipped `IdentityRead` port (server-SVID selection by `AllocationId`; bundle
  for client-auth) (#35) — exists.
- `nft_tproxy` / `IP_TRANSPARENT` + `WebPkiClientVerifier` + in-kernel kTLS-RX +
  `tls_sw_splice_read` on the 6.18 kernel (ADR-0068).

## Effort estimate

~1 day (≤6h). Reference class: the entire inbound path (TPROXY intercept +
orig-dst + server-mTLS + kTLS-RX + agent-light splice-to-S) is proven end-to-end in
`findings-inbound-intercept.md` §1–§5; the cost is productionising the
identity-selection lookup (orig-dst → `AllocationId` → SVID via `IdentityRead`,
which the spike hardcoded) and re-proving the loopback spike topology in the real
netns/veth shape.

## Pre-slice SPIKE

Not needed — `findings-inbound-intercept.md` (increment-i) proved the whole inbound
half on a real 7.0 kernel (TPROXY intercept + orig-dst recovery + server-mTLS +
kTLS-RX + agent-light splice-to-S, fail-closed on nocert/wrongca). This slice
productionises it (real identity lookup + real netns/veth topology).

## Taste-test note

The inbound enforce slice: ships the server-side half of the proxy (orig-dst →
server-mTLS → kTLS-RX → agent-light splice-to-S). Production-data observable (real
`tcpdump` `0x17` on the client leg + byte-exact plaintext at S + `strace` splice-only
+ `ss -tie` kTLS-RX — Tier-3, the proof the proxy is BIDIRECTIONAL). Disproves a
real assumption (the server workload reads byte-exact plaintext while the wire
carries ciphertext, agent-light, server holds nothing). One value story
(US-MTLS-04); the observable is the inbound enforced flow — the second direction
that makes "between two workloads" real. Not infra-only.
