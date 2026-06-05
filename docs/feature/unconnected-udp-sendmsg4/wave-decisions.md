# Wave Decisions — unconnected-udp-sendmsg4

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)

---

## DIVERGE Decisions

| ID | Decision | Rationale | Source |
|---|---|---|---|
| **D1** | Diverge ONLY on the *unconnected*-UDP hook; do NOT reopen ADR-0053's connect4 decision (locked, shipping). | The connected-UDP + TCP path is settled; #200 is the named, scoped follow-up for the unconnected path. | ADR-0053 Amd 4; #200; dispatch scope guard |
| **D2** | Job rides existing **J-OPS-004** (operator-trust reachability) + **J-PLAT-004** (Sim≡kernel equivalence); no new job minted. | The same-host dataplane-reachability job is implicit in J-OPS-004; minting a per-idiom job fragments it (udp-service-support D5 precedent). | `job-analysis.md` § 5; `jobs.yaml` |
| **D3** | **recvmsg4 is load-bearing, not optional** for the canonical (source-validating) client. | Kernel commit `983695fa6765` demonstrates `nslookup` rejecting the backend-sourced reply; the fix IS recvmsg4. Cilium ships recvmsg4 for this. Resolves the #200 "consider recvmsg4 (verify; may be out of scope)" hedge → required. | `competitive-research.md` Competitor 3; #200 |
| **D4** | **Recommend Option 2 — `sendmsg4` + `recvmsg4`** (bidirectional address rewrite), reusing `LOCAL_BACKEND_MAP` (forward) + a `REVERSE_LOCAL_MAP` (reply). | Top of the locked dev-tool taste matrix at **4.65**; driven by DVF (5.00) + T4 (delivery-is-real, 5) on the reply-path discriminator (O2). | `taste-evaluation.md`; `recommendation.md` |
| **D5** | Kernel viability is NOT a blocker for any cgroup-hook option. | sendmsg4 (4.18), recvmsg4 (4.20), `bpf_sock_addr.protocol`/`user_ip4`/`user_port` populated/writable for these contexts — all below the 5.10 LTS floor (fresh-verified 2026-06-05). | `competitive-research.md` Competitor 2 |
| **D6** | proto source = `bpf_sock_addr.protocol` (zero-translation), same contract as connect4. | ADR-0053 Amd 2 already pinned this for connect4; sendmsg4 exposes the same field. No translation table. | ADR-0053 Amd 2; `competitive-research.md` |
| **D7** | recvmsg4's reverse lookup = a `REVERSE_LOCAL_MAP` written atomically with the forward `register_local_backend` write (one logical write, two map entries) — NOT a conntrack table. | UDP is connectionless; there is no per-flow state to reuse. Single-write consistency keeps Sim≡kernel equivalence (J-PLAT-004) tractable. Second-map-vs-reverse-scan is a DESIGN detail. | `recommendation.md` § blast radius; `options-raw.md` A8 |

## Job Summary

- **Validated job (physical level):** A same-host client's datagram to a
  service VIP must reach a healthy backend, and the reply must appear to
  come from the VIP — regardless of whether the client connected first.
- **Rides:** J-OPS-004 + J-PLAT-004 (no new job).
- **ODI outcomes:** 5 (O1 reachability, O2 reply-source identity [the
  discriminator], O3 Sim≡kernel equivalence, O4 marginal surface, O5
  testability-below-Tier-3).

## Options Evaluated

- **6 generated/curated** (A1 sendmsg4-only, A2 sendmsg4+recvmsg4, A5 unify,
  A3 SK_LOOKUP, A7 VIP-bind, A4 document); A6/A8 merged, A9 held out;
  iptables/IPVS DVF-eliminated at the option-set boundary (vision principle
  2; ADR-0053 Alt F).
- **All 6 survived DVF** (none eliminated in-matrix); scored on all 4 taste
  criteria with locked dev-tool weights.
- **Ranking:** Option 2 (4.65) > Option 3 (4.07) > Option 1 (3.48) >
  Option 6 (3.05) > Option 4 (2.65) > Option 5 (2.48).
- **Recommended:** Option 2 (`sendmsg4` + `recvmsg4`) — the only clean
  VIP-sourced-reply, exact-kernel-design-parity, pure-addition option.
- **Dissent:** Option 1 (sendmsg4-only, 3.48) — wins ONLY if the Phase-1
  UDP client model is non-source-validating, OR as a documented
  request-path-first interim with a tracked recvmsg4 follow-up.

## Sub-deferral surfaced (needs user approval if taken)

- recvmsg4-as-separate-follow-up (the Option-1-interim path). The
  recommendation keeps recvmsg4 **in scope** (Option 2). A sendmsg4-first
  split is a *new deferral* requiring a user-approved GitHub issue — **not
  created by this DIVERGE** (CLAUDE.md). #200 already covers the
  recommended in-scope Option 2; no new issue needed for it.

## SSOT Updates

- `docs/product/jobs.yaml`: **no new job**; changelog entry dated
  2026-06-05 elevates the same-host unconnected-UDP reachability dimension
  under J-OPS-004 + J-PLAT-004, referencing feature-id
  `unconnected-udp-sendmsg4` / #200. J-OPS-004 `source` annotated to span
  unconnected-UDP wire-path reachability.

## Peer Review

- nw-diverger-reviewer (Prism) verdict recorded in `diverge/review.yaml`.
