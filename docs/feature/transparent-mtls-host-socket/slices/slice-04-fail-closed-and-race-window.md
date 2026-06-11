# Slice 04 — fail-closed on absent/wrong SVID; no cleartext before kTLS install (the security guardrails)

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-04

## Goal (one sentence)

The encryption guarantee cannot be bypassed: a handshake against an absent or wrong
SVID FAILS CLOSED (no plaintext fallback), and no cleartext byte egresses before
kTLS is armed (the plaintext race window) — a gate armed before connect()/accept()
returns keeps the wire fail-closed (the gate mechanism is a DESIGN/spike choice, not
pinned here), and the residual data-loss window is closed by connection reset if
drops > 0.

## IN scope

- A handshake where `IdentityRead::svid_for` returns absent for the allocation
  FAILS CLOSED — the agent refuses rather than presenting a stale credential; no
  cleartext egresses.
- A handshake where the peer does not chain to `IdentityRead::current_bundle()`
  aborts (rustls) — no TLS Application Data, no cleartext.
- A gate armed before connect()/accept() returns, so no cleartext byte reaches the
  wire before kTLS is armed (the confidentiality fail-closed property). The gate
  mechanism (sk_msg DROP-until-armed vs sockmap redirect vs out-of-tree write-block)
  is a DESIGN/spike choice — pinned in OUT scope below, not here.
- A race-window probe: `write("PLAINTEXT_PROBE")` immediately on `connect()` return
  + a deliberately delayed kTLS install → `tcpdump` shows EITHER TLS 1.3 records OR
  zero application bytes, NEVER the plaintext probe string; a drop-counter signals
  the window; the connection resets if drops > 0 and a request-first app retries.

## OUT scope

- Restart survival + the WASM variant → Slice 05.
- The server-speaks-first protocol (SMTP/FTP/SSH) data-loss window + the out-of-tree
  write-block patch decision → DESIGN scope call (Open Questions), NOT this slice.
- The exact gate mechanism (sk_msg DROP-until-armed vs sockmap proxy redirect vs the
  write-block patch) → DESIGN's, informed by the Slice-00 spike (DISCUSS pins the
  observable, not the mechanism).

## Learning hypothesis

- **Disproves if it fails**: "the encryption cannot be bypassed — an absent/wrong
  SVID fails closed (no cleartext), and no cleartext byte egresses before kTLS is
  armed." If a wrong/absent SVID falls back to plaintext, or a write-before-install
  leaks cleartext, the encryption guarantee is bypassable and the security claim is
  hollow.
- **Confirms if it succeeds**: the two security invariants (fail-closed handshake,
  no cleartext before install) hold — the guarantee is enforced, not just
  configured.

## Acceptance criteria

- [ ] A handshake where `IdentityRead::svid_for` returns absent for the allocation fails closed — the agent refuses rather than presenting a stale credential; no cleartext egresses (TEST tier).
- [ ] A handshake where the peer does not chain to `IdentityRead::current_bundle()` aborts (rustls) — no TLS Application Data, no cleartext on the wire.
- [ ] No cleartext byte reaches the wire before kTLS is armed (the confidentiality fail-closed property): the gate is armed before connect()/accept() returns. The gate mechanism (sk_msg DROP-until-armed vs sockmap redirect vs out-of-tree write-block) is a DESIGN/spike choice — NOT pinned here.
- [ ] A race-window probe (write immediately on connect() return + deliberately delayed kTLS install) captured by `tcpdump` shows EITHER TLS 1.3 records OR zero application bytes — NEVER the plaintext probe string; a drop-counter signals the window; the connection resets if drops > 0 and a request-first app retries (TEST tier, via Lima).
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the fail-closed + race-window acceptance tests (real kernel, not `--no-run`).

## Dependencies

- Slice 03 (the install path to gate) + Slice 02 (the handshake to fail closed).
- The shipped `IdentityRead` port returning absence explicitly (`None`) for an
  unheld allocation (#35) — exists.
- sk_msg semantics on the 6.18 kernel (PASS/DROP/REDIRECT — no lossless HOLD;
  `bpf_msg_cork_bytes` does not buffer) — the race-window research established this.

## Effort estimate

~1 day (≤6h). Reference class: the negative tests are straightforward (absent-SVID,
wrong-peer); the race-window probe needs a deliberately-delayed-install harness +
the sk_msg drop-counter (the race-window research §7 specifies the exact test shape).

## Pre-slice SPIKE

Not needed — Slice 00 (the spike) exercised the race-window no-cleartext-before-
install proof; this slice adds the fail-closed-on-wrong/absent-SVID negative paths
and productionises the gate.

## Taste-test note

A thin vertical cut: ships the two security guardrails (fail-closed handshake +
no-cleartext-before-install). Production-data observable (real `tcpdump` showing no
cleartext / no plaintext probe + the drop-counter — Tier-3, the security invariants
a reviewer pushes hardest on). Disproves a real assumption (the encryption cannot be
bypassed). One value story (US-MTLS-04); the observable is the fail-closed / no-leak
proof — the security teeth of the feature, not infra-only.
