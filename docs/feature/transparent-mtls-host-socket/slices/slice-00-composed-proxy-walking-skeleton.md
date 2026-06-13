# Slice 00 — the COMPOSED proxy walking skeleton (BLOCKING): real netns/veth bidirectional round-trip → handshake → kTLS → splice, NO RST

> **WALKING SKELETON — the FIRST, BLOCKING DELIVER slice (F2).** Must pass
> before any other slice lands. Re-grounded to **ADR-0069** (the universal
> agent-light L4 proxy). This **supersedes** the old in-band kTLS-on-the-
> workload's-own-socket spike slice. The 6 committed Tier-3 spikes settled the
> MECHANISM (verdict: proxy, not in-band) AND proved the **composed INBOUND
> flow end-to-end** under a real transparent intercept: increment-i drove a real
> nft-TPROXY intercept → `getsockname` orig-dst → server-side mutual-TLS
> (presents S's SVID, `WebPkiClientVerifier` VERIFIES C's client SVID chains to
> the bundle) → kTLS-RX arm → agent-light splice → S reads byte-exact plaintext,
> with the client leg carrying TLS `0x17` ciphertext and fail-closed on
> nocert/wrongca (distinct reasons, 0 bytes to S). The OUTBOUND primitives are
> equally spike-proven — intercept + lossless pre-arm capture + handshake flush
> (increment-e) and the kTLS-TX steady-state encrypt-on-`splice` — but on SEPARATE
> harnesses. So the MECHANISM and the composed inbound flow are PROVEN; what
> remains is **integration**, not mechanism. This slice composes the spike-proven
> pieces into ONE bidirectional walking skeleton in the real netns/veth topology,
> closing the three NARROW gaps the spikes left open: (1) outbound composed in ONE
> flow, (2) bidirectional steady-state round-trip, (3) real netns/veth +
> cgroup-isolated workloads.
>
> **REVISED 2026-06-13 (D-MTLS-13).** The OUTBOUND forward steady state is an
> **agent-light `splice(legF → legB)`** into leg B's kTLS-TX, NOT the agent-idle
> sockmap egress redirect (increment-f, 15/15) the original text named — that
> redirect was proven non-viable (`MSG_DONTWAIT`-backlog stall, ~10–15% loss;
> `docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`)
> and the whole sockmap apparatus is retired. The composed flow's "post-arm
> bidirectional multi-record transfer" is now a `splice` pump in BOTH directions.
> Reader legs drain 0.5-RTT early data before arming kTLS-RX. SHIPPED + verified
> 20/20 (commit `bb6489ef`).

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-00
**Walking skeleton**: YES — the thinnest composed end-to-end cut, BOTH directions, one flow each
**Type**: Composed Tier-3 acceptance test (production code, NOT a spike — the spike already ran)

## Goal (one sentence)

On the pinned 6.18 kernel, COMPOSE the spike-proven pieces into ONE bidirectional
walking skeleton in the real netns/veth topology with cgroup-isolated workloads
and **NO RST post-arm** — OUTBOUND (real `cgroup_connect4` intercept → workload
pre-arm write → leg-B rustls handshake presenting the held SVID → kTLS arm →
post-arm **bidirectional** multi-record transfer, composing increment-e's
intercept+capture+flush with increment-f's kTLS-TX splice in ONE flow) AND
INBOUND (the increment-i composed flow — real nft-TPROXY intercept →
`getsockname` orig-dst → server-mTLS → kTLS-RX arm → splice-to-server, byte-exact
plaintext at S — extended with the S→C response leg) — closing the three narrow
gaps the spikes left open (outbound-one-flow, bidirectional round-trip,
netns/veth), under **BOTH** normal AND traced/delayed timing.

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

- The per-direction agent-light cost assertions (forward + return strace —
  `splice`/`ppoll` only, both directions; D-MTLS-13) → Slices 03 (outbound) / 04
  (inbound) — this slice proves the composition holds + no RST; the cost-tier
  observables are productionised there.
- Fail-closed negatives, resource limits, pump supervision, the intercept-exemption
  negative, the authn-vs-authz boundary → Slice 05.
- WASM (no WASM driver exists in v1) and guest-stack (#222, the staged adapter) —
  out of v1 scope entirely.
- Restart-survival — GONE in v1 (ADR-0069); not a deliverable.

## Learning hypothesis

This is an **integration / walking-skeleton gate, NOT a "prove the mechanism"
gate** — the mechanism is spike-proven (the composed inbound flow end-to-end in
increment-i; the outbound primitives in increment-e/f). The open question is
whether the proven pieces COMPOSE into one bidirectional flow on the real
netns/veth topology.

- **Disproves if it fails**: "the spike-proven pieces compose into ONE
  bidirectional walking skeleton in the real netns/veth topology with NO RST —
  (1) the OUTBOUND increment-e intercept+capture+flush and increment-f kTLS-TX
  splice, never wired together before, hold in ONE flow; (2) a bidirectional
  steady-state round-trip (the S→C / B→F response leg never composed in the
  spikes) holds; (3) cgroup-isolated workloads over veth (all spikes were
  loopback + sibling processes) behave as the loopback spikes did — under normal
  AND delayed timing." A FAIL here is an integration defect (the pieces don't
  compose, or netns/veth changes the behaviour the loopback spikes saw), not a
  mechanism unknown.
- **Confirms if it succeeds**: the spike-proven pieces compose into the
  bidirectional netns/veth walking skeleton; Slices 01–05 productionise the
  composed path (intercept-exemption, handshake, outbound enforce, inbound
  enforce, guardrails).

> The mechanism — including the composed INBOUND flow end-to-end — is
> spike-proven (increment-i §1–§4: real TPROXY intercept → orig-dst → server-mTLS
> verifying the client SVID → kTLS-RX → agent-light splice → byte-exact plaintext
> at S, fail-closed on nocert/wrongca). These ACs do NOT re-prove the mechanism;
> they demonstrate the THREE gap-closures (outbound-one-flow, bidirectional
> round-trip, netns/veth) that compose the proven pieces into the walking
> skeleton.

- [ ] **GAP 1 — OUTBOUND composed in ONE flow**: on the 6.18 kernel, a real `cgroup_connect4` intercept routes a workload's `connect()` to the agent's leg-F listener; the agent drains the pre-arm plaintext losslessly (the increment-e intercept+capture+flush), completes a rustls TLS 1.3 CLIENT handshake on leg B presenting the held SVID (read via `IdentityRead`), drains 0.5-RTT early data, arms kTLS on leg B, and the post-arm **agent-light forward `splice(legF → legB)` into kTLS-TX** (D-MTLS-13 — `tls_sw_sendmsg` encrypts on splice-in; NOT the retired sockmap egress redirect) carries the steady-state bytes — intercept+capture+flush wired to the forward splice in ONE flow for the first time, with **NO RST**. Anchor: `findings-userspace-relay.md` (intercept+capture+flush) + `findings-splice-return.md` (the symmetric splice primitive) + `sockmap-egress-redirect-into-ktls-tx-delivery-research.md` (why the redirect was retired); ADR-0069 § Enforcement "Composed walking-skeleton gate."
- [ ] **GAP 2 — bidirectional steady-state round-trip**: post-arm transfer is **bidirectional** in BOTH directions — outbound forward F→B AND the return B→F, inbound request C→S AND the response S→C (the response leg increment-i drove only one way of; increment-i §"What was NOT tested" names the S→C leg as unproven) — all multi-record, with **NO RST**. EVERY direction is the agent-light `splice` pump (D-MTLS-13: forward and return are the same primitive). Anchor: `findings-inbound-intercept.md` § "What was NOT tested" (bidirectional steady-state) + `findings-splice-return.md` (the splice primitive composed in every direction).
- [ ] **GAP 3 — real netns/veth + cgroup-isolated workloads**: the composed flow runs over a real netns/veth topology with cgroup-isolated workloads, NOT loopback + sibling processes (all spikes were loopback). The nft-TPROXY prerouting intercept, the `cgroup_connect4` rewrite, and the splice-to-workload all hold in the real topology. Anchor: `findings-inbound-intercept.md` § "What was NOT tested" (the cgroup/netns shape would need re-proving in the real netns/veth topology).
- [ ] The peer-facing leg carries TLS 1.3 Application Data records (`tcpdump` shows `1703 03` / 0x17) in both directions; the workload's plaintext appears only on the host-internal leg F / leg S, NEVER on the peer leg. Anchor: `findings-splice-return.md` (the agent-light splice into/out of kTLS) / `findings-inbound-intercept.md` §3.
- [ ] The composed path holds under **BOTH** normal AND traced/delayed timing — the post-arm transfer never RSTs in either timing regime (the increment-e harness's intercept-lifecycle RST — a throwaway-harness artifact, not a kernel finding — is defeated by the production intercept lifecycle). Anchor: `findings-userspace-relay.md` § Crux 2 (the steady-state-blocking harness RST).
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

~1–1.5 days. The mechanism is proven (the composed inbound flow end-to-end in
increment-i; the outbound primitives in increment-e/f); the cost is INTEGRATION —
wiring increment-e's intercept+capture+flush with increment-f's kTLS-TX splice
into ONE outbound flow (gap 1), composing the bidirectional round-trip including
the response legs (gap 2), and standing the whole thing up on the real netns/veth
topology with cgroup-isolated workloads (gap 3) — plus the dual (outbound +
inbound) harness on the real kernel.

## Pre-slice SPIKE

Not needed — the 6 committed Tier-3 spikes already settled the mechanism, proved
the composed INBOUND flow end-to-end (increment-i), and de-risked every outbound
primitive on a real 7.0 kernel. This slice is the PRODUCTION INTEGRATION of the
spike-proven pieces — composing them into the bidirectional netns/veth walking
skeleton and closing gaps 1–3. It is a composed acceptance test, not a spike — a
FAIL here is a real integration defect to fix, not a mechanism-learning outcome.

## Taste-test note

The walking skeleton, thinned to ONE composed flow per direction + the no-RST
invariant. Touches every backbone activity (intercept → handshake → kTLS arm →
bidirectional transfer) for both directions. Production-data observable (real
`tcpdump` TLS 1.3 records + `ss -tie` kTLS ULP, NOT synthetic). The mechanism and
the composed inbound flow are spike-proven; this slice closes the three NARROW
integration gaps the spikes left (outbound-one-flow, bidirectional round-trip,
netns/veth). Carries one value story (US-MTLS-00); its observable is the
lossless, RST-free, TLS-1.3-on-the-peer-wire capture for both halves over the
real netns/veth topology — the proof the proven pieces compose. BLOCKING: no
other slice lands until this passes.
