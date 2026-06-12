# Slice 00 — the COMPOSED proxy walking skeleton (BLOCKING): real intercept → handshake → kTLS → bidirectional transfer, NO RST

> **WALKING SKELETON — the FIRST, BLOCKING DELIVER slice (F2).** Must pass
> before any other slice lands. Re-grounded to **ADR-0069** (the universal
> agent-light L4 proxy). This **supersedes** the old in-band kTLS-on-the-
> workload's-own-socket spike slice: the 6 committed Tier-3 spikes settled the
> MECHANISM (verdict: proxy, not in-band) and proved every PRIMITIVE in
> isolation, but did **NOT** prove the **composition** under a real transparent
> intercept (increment-e RST'd on the intercept lifecycle; increments-f/h
> removed the intercept to prove their primitive). This slice closes that gap.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-00
**Walking skeleton**: YES — the thinnest composed end-to-end cut, BOTH directions, one flow each
**Type**: Composed Tier-3 acceptance test (production code, NOT a spike — the spike already ran)

## Goal (one sentence)

On the pinned 6.18 kernel, prove the COMPOSED agent-light L4 proxy path holds
end-to-end with **NO RST post-arm** — OUTBOUND (real `cgroup_connect4` intercept
→ workload pre-arm write → leg-B rustls handshake presenting the held SVID →
kTLS arm → post-arm **bidirectional** multi-record transfer) AND INBOUND (real
nft-TPROXY intercept → `getsockname` orig-dst → server-mTLS → kTLS-RX arm →
splice-to-server, byte-exact plaintext at S) — under **BOTH** normal AND
traced/delayed timing.

## IN scope

- A composed Tier-3 acceptance test on the 6.18 kernel exercising BOTH halves of
  one composed flow each:
  - **OUTBOUND**: real `cgroup_connect4` rewrite to the agent's leg-F listener →
    the agent drains the pre-arm plaintext losslessly → rustls TLS 1.3 CLIENT
    handshake on leg B presenting the held SVID (read via `IdentityRead`) → kTLS
    arm on leg B → post-arm **bidirectional** multi-record transfer (forward F→B
    and return B→F) with **NO RST**.
  - **INBOUND**: real nft-TPROXY redirect to the agent's `IP_TRANSPARENT` leg-C
    listener → `getsockname()` recovers the original destination → server-side
    mutual-TLS (present the server SVID, `WebPkiClientVerifier` REQUIRE+VERIFY
    the client SVID) → kTLS-RX arm → `splice`-to-server → S reads the byte-exact
    plaintext, with **NO RST**.
- The test repeats under **normal** AND **traced/delayed** timing (e.g. a
  deliberate handshake-window delay) — the increment-e RST mode is the thing to
  defeat.
- Reading the held SVID + trust bundle via the shipped `IdentityRead` port (#35) —
  NO #26-local issuance or cache. kTLS arms on the **agent's** peer-facing leg
  (leg B / leg C), NEVER the workload's socket.

## OUT scope

- The per-direction agent-idle/agent-light cost assertions (forward strace, return
  strace) → Slices 03 (outbound) / 04 (inbound) — this slice proves the
  composition holds + no RST; the cost-tier observables are productionised there.
- Fail-closed negatives, resource limits, pump supervision, the intercept-exemption
  negative, the authn-vs-authz boundary → Slice 05.
- WASM (no WASM driver exists in v1) and guest-stack (#222, the staged adapter) —
  out of v1 scope entirely.
- Restart-survival — GONE in v1 (ADR-0069); not a deliverable.

## Learning hypothesis

- **Disproves if it fails**: "the agent-light L4 proxy COMPOSES under a real
  transparent intercept — a real `cgroup_connect4` (outbound) / nft-TPROXY
  (inbound) intercept → pre-arm capture → handshake → kTLS arm → post-arm
  bidirectional multi-record transfer holds with NO RST, under normal AND delayed
  timing." If the composed intercept lifecycle RSTs (the increment-e failure
  mode), the composition does not hold and every later slice is blocked until the
  RST is engineered around.
- **Confirms if it succeeds**: the primitives the spikes proved in isolation
  compose on a real intercept; Slices 01–05 productionise the proven composition
  (intercept-exemption, handshake, outbound enforce, inbound enforce, guardrails).

## Acceptance criteria

- [ ] **OUTBOUND composed**: on the 6.18 kernel, a real `cgroup_connect4` intercept routes a workload's `connect()` to the agent's leg-F listener; the agent drains the pre-arm plaintext losslessly, completes a rustls TLS 1.3 CLIENT handshake on leg B presenting the held SVID (read via `IdentityRead`), arms kTLS on leg B, and post-arm **bidirectional** multi-record transfer (F→B forward AND B→F return) completes with **NO RST**. Anchor: ADR-0069 § Enforcement "Composed walking-skeleton gate" (the composition increment-e did NOT prove).
- [ ] **INBOUND composed**: a real nft-TPROXY intercept routes a connection aimed at the server workload's logical address to the agent's `IP_TRANSPARENT` leg-C listener; `getsockname()` recovers the original destination; the agent completes a server-side mutual-TLS handshake (presents the server SVID, `WebPkiClientVerifier` REQUIRE+VERIFY the client SVID), arms kTLS-RX, and `splice`s the decrypted plaintext to the server workload S, which reads the **byte-exact** request as plaintext, with **NO RST**. Anchor: `findings-inbound-intercept.md` §1–§3.
- [ ] The peer-facing leg carries TLS 1.3 Application Data records (`tcpdump` shows `1703 03` / 0x17) in both directions; the workload's plaintext appears only on the host-internal leg F / leg S, NEVER on the peer leg. Anchor: `findings-egress-ktls-splice.md` Assertion 1 / `findings-inbound-intercept.md` §3.
- [ ] The composed path holds under **BOTH** normal AND traced/delayed timing — the post-arm transfer never RSTs in either timing regime (the increment-e RST mode is defeated). Anchor: ADR-0069 § Consequences "Composition is unproven."
- [ ] The agent reads SVID + bundle ONLY via `IdentityRead` (#26 is a READER, never an issuer/cache); kTLS arms on the **agent's** leg (leg B / leg C), NOT the workload's socket.
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the composed acceptance test (real 6.18 kernel, actually executing — NOT `--no-run`, which would not exercise the composed intercept lifecycle).

## Dependencies

- The shipped `IdentityRead` port + `Arc<IdentityMgr>` held store (#35) — exists.
- The shipped `Ca` hierarchy + `SvidMaterial` (cert + leaf key, ADR-0063 D9) (#28) — exists.
- The pinned 6.18 LTS kernel (ADR-0068) on Lima/LVH — `tls.ko`, `nft_tproxy` /
  `xt_TPROXY` / `nf_tproxy_ipv4`, in-kernel TLS 1.3 TX+RX, `CONFIG_NET_HANDSHAKE`.
- The 6 committed spike findings (the proven primitives this slice composes):
  `findings.md`, `findings-egress-ktls-splice.md`, `findings-splice-return.md`,
  `findings-userspace-relay.md`, `findings-lossless-hybrid.md`,
  `findings-inbound-intercept.md`.
- The `MtlsEnforcement` port contract (ADR-0069 / feature-delta DESIGN) — the
  enforcement surface this slice composes; pinned by DESIGN, NOT improvised.

## Effort estimate

~1–1.5 days. The primitives are proven; the cost is engineering the composed
intercept lifecycle so it does NOT RST (the increment-e failure mode) and wiring
the bidirectional transfer + the dual (outbound + inbound) harness on the real
kernel.

## Pre-slice SPIKE

Not needed — the 6 committed Tier-3 spikes already settled the mechanism and
de-risked every primitive on a real 7.0 kernel. This slice is the PRODUCTION
composition the spikes did NOT prove (they proved the primitives in isolation;
increment-e's composed harness RST'd). It is a composed acceptance test, not a
spike — a FAIL here is a real defect to fix, not a learning outcome.

## Taste-test note

The walking skeleton, thinned to ONE composed flow per direction + the no-RST
invariant. Touches every backbone activity (intercept → handshake → kTLS arm →
bidirectional transfer) for both directions. Production-data observable (real
`tcpdump` TLS 1.3 records + `ss -tie` kTLS ULP, NOT synthetic). Closes the ONE
thing the spikes did not prove (the composition under a real intercept). Carries
one value story (US-MTLS-00); its observable is the lossless, RST-free,
TLS-1.3-on-the-peer-wire capture for both halves — the proof the whole feature
composes. BLOCKING: no other slice lands until this passes.
