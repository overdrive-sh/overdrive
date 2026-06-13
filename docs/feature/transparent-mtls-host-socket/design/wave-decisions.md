# DESIGN Decisions — transparent-mtls-host-socket (GH #26 folds #222)

**Agent**: Morgan (nw-solution-architect) · **Date**: 2026-06-12 · **Mode**:
formalize a user-LOCKED decision on complete empirical evidence · **Density**:
`lean` + `ask-intelligent` (Tier-1 `[REF]`) · **Rigor**: `.nwave/des-config.json`
inherit; `review_enabled: true` (see § Review below); mutation N/A (docs).

## The locked decision (designed, not relitigated)

**Fold #222 into #26. Build ONE universal "transparent mTLS via an agent-light L4
proxy" as THE enforcement mechanism for ALL workload kinds** (process/exec, WASM,
microVM, unikernel). Whitepaper §7's "one identity model, two enforcement
mechanisms" collapses to ONE. In-band kTLS-on-the-workload's-own-socket is
SUPERSEDED as v1 and retained as a post-v1 optimization tracked in **#231**.

**Two USER-LOCKED scope decisions (2026-06-12 re-review):**
- **Host-socket is BIDIRECTIONAL v1** — both the outbound/client half
  (`cgroup_connect4` intercept → client mTLS) AND the inbound/server half (TPROXY
  intercept → `getsockname` orig-dst → server mTLS verifying the client → splice
  to the server workload) are designed and proven (`findings-inbound-intercept.md`,
  increment-i, kernel 7.0). The **guest-stack intercept adapter** (microVM /
  unikernel) is STAGED to **#222** (repurposed to "the guest-stack intercept
  adapter for the #26 universal proxy" — no longer a separate mechanism).
- **#178 is the upgrade, not a v1 prereq** — v1's honest security claim is
  **"chain-to-bundle transport authentication + encryption, NO intended-peer
  identity pinning."** A routing bug / VIP collision / malicious in-cluster
  endpoint presenting a valid-but-unintended SVID is NOT prevented in v1; #178
  (east-west SPIFFE-ID resolution) supplies the expected-peer SAN-match.

Recorded in **ADR-0069**. User-decided 2026-06-12 on 6 Tier-3 spikes + 3 research
docs (kernel 7.0, committed `353cdc52`). The mechanism's **primitives** are
de-risked (forward splice, return splice, kTLS arm, arming order — each proven in
isolation); the **composition under a real transparent intercept** is the
walking-skeleton gate (the FIRST DELIVER slice; increment-e's steady-state RST is
unresolved — see § "Review revisions" F2).

## Why (the evidence, one line each)

- **In-band lossless foreclosed 3 ways**: no `sk_msg` HOLD (`findings.md`);
  source-TX-bypass RST on redirecting the live socket (`findings-lossless-hybrid.md`
  + `sockmap-redirect-live-socket-liveness-research.md`); lossless capture
  structurally requires a proxy (`findings-userspace-relay.md`).
- **Proxy proven agent-light BOTH directions** (the kernel kTLS engine does all
  crypto; the agent runs no cipher) — but NOT symmetric: the DECRYPT/RX directions
  (outbound return, inbound deliver) are zero-copy `splice` out of kTLS-RX
  (`tls_sw_splice_read`, ~1/record, no userspace copy; `findings-splice-return.md`);
  the ENCRYPT/TX directions (outbound forward, inbound response) are a bounded
  `read → write_all` COPY into kTLS-TX (`tls_sw_sendmsg` encrypts each `write`;
  per-record `read`+`write`, NOT zero-copy). A `splice` into kTLS-TX loses records,
  so the encrypt directions use a blocking `write_all`. **(REVISED 2026-06-13,
  D-MTLS-13: the forward was originally agent-IDLE via a sockmap-egress-redirect,
  15/15 in `findings-egress-ktls-splice.md`; that redirect was proven non-viable —
  a `MSG_DONTWAIT`-backlog delivery stall — and retired for the `write_all` copy;
  see § "Forward-mechanism pivot" below.)**
- **Basic mechanism proven**: `sockops → rustls → kTLS`, `pidfd_getfd` handoff,
  SOCKMAP-before-`TCP_ULP` ordering, control records via `ktls::KtlsStream`
  (`findings.md`).

## What was produced

| Artifact | Path |
|---|---|
| Central ADR | `docs/product/architecture/adr-0069-transparent-mtls-universal-agent-light-l4-proxy.md` |
| Application Architecture section | `docs/product/architecture/brief.md` § "Transparent mTLS — universal agent-light L4 proxy extension" (+ ADR index row 0069 + changelog) |
| C4 diagrams (L1+L2+L3) | `docs/feature/transparent-mtls-host-socket/design/c4-diagrams.md` |
| Feature-delta DESIGN sections | `docs/feature/transparent-mtls-host-socket/feature-delta.md` § "Wave: DESIGN / [REF] …" |
| Whitepaper §7/§8 reshape | `docs/whitepaper.md` § 7 ("Transparent mTLS — one universal agent-light L4 proxy") |
| Upstream back-propagation | `docs/feature/transparent-mtls-host-socket/design/upstream-changes.md` |
| This summary | `docs/feature/transparent-mtls-host-socket/design/wave-decisions.md` |

## Key decisions (D-MTLS-1…12)

See the feature-delta § "Wave: DESIGN / [REF] Decisions Table" for the full table.
Highlights: D-MTLS-3 (NEW `MtlsEnforcement` port, `Dataplane` does not fit);
D-MTLS-4 (**REVISED 2026-06-13** — forward and return are BOTH agent-light but NOT
the same primitive: encrypt/TX = `read → write_all` copy into kTLS-TX, decrypt/RX =
zero-copy `splice` out of kTLS-RX; the original agent-idle sockmap-egress was
retired, see D-MTLS-13); D-MTLS-5
(leg B = plain kTLS-RX, NO psock — now no psock on ANY leg); D-MTLS-7 (**MOOT
2026-06-13** — sockmap-before-`TCP_ULP` invariant; no sockmap insert on any path);
D-MTLS-10 (in-process agent — no separate process, no gRPC/CSR; resolves the prior
open item); D-MTLS-11 (Earned-Trust `probe()` mandatory); D-MTLS-12 (added
2026-06-12 — `probe`'s handshake sentinel uses a THROWAWAY self-signed cert minted
in-process via `rcgen`; substrate-self-test crypto, signed by neither CA, never in
the trust bundle, never on a real wire — #26 stays a READER, NOT an issuer;
promotes `rcgen` to an `overdrive-dataplane` production dep; SD-5, user-approved.
**STILL LIVE after 2026-06-13** — the shipped `probe` still does a loopback rustls
handshake, so the rcgen-sentinel core is unchanged; only the sockmap-engagement
sub-sentinels were mooted); **D-MTLS-13 (2026-06-13 — forward sockmap-egress →
agent-light `splice` pivot + kTLS 0.5-RTT early-data drain; SHIPPED + verified
20/20, commit `bb6489ef`; see § "Forward-mechanism pivot" below).**

## Reuse Analysis verdict (hard gate)

3 REUSE-AS-IS · 5 EXTEND (incl. `overdrive-dataplane` as the `HostMtlsEnforcement`
home — OQ-2 resolved) · 1 CREATE-NEW port (`MtlsEnforcement`) · 1 CREATE-NEW dep
(`ktls`). Default-EXTEND honored. Full table in `brief.md` § 6 / feature-delta §
Reuse Analysis.

## Open questions / deferrals

- **OQ-1 — ACCEPTED (user-approved 2026-06-12)**: the EXACT `MtlsEnforcement`
  signatures are pinned (model fixed by ADR-0069; the connection-handle wire shape +
  error variants are NOT improvised). The contract is BIDIRECTIONAL (F3 —
  `direction`/`Routed`) with the F6 `pump_stall_deadline` + F7 concrete `MtlsLimits`
  values. The bidirectional 4-method contract
  (`probe`/`enforce`-dispatch-on-`Direction`/`liveness`/`teardown`,
  `InterceptedConnection { leg, routed, alloc, expected_peer }`, `MtlsLimits`, the
  cause-distinct errors) is the accepted contract DELIVER implements to. No longer a
  blocker.
- **OQ-2 — RESOLVED (user-decided 2026-06-12)**: **no new crate.**
  `HostMtlsEnforcement` EXTENDS **`overdrive-dataplane`** (the established
  `adapter-host` userspace eBPF crate hosting `EbpfDataplane` — `unsafe` already
  allowed, `aya` + BPF `build.rs` already present, so every new-crate rationale is
  already satisfied); the kernel-side sockops/`sk_skb`/`cgroup_connect4`-mtls
  programs EXTEND **`overdrive-bpf`** (one shared BPF object); `SimMtlsEnforcement`
  stays in `overdrive-sim`. **`overdrive-host` ruled out** (`src/lib.rs:21` is
  `#![forbid(unsafe_code)]`; the proxy is irreducibly `unsafe`). **Revisit trigger**
  (not a blocker): if mTLS later needs isolation from the LB/service dataplane,
  split into a dedicated `adapter-host` crate then.
- **In-band restart-survival + 1-socket density** — NOT in v1 scope (the accepted
  proxy trade, ADR-0069 A1); a post-v1 optimization tracked in **#231**.
- **Multi-node transparent mTLS** — OUT of v1 scope (Phase 1 is single-node). No
  forward-pointer issue; do NOT cite #36 (generic node enrollment/admission, not
  cross-node transparent mTLS).

  (The agent-light splice return is the design; a fully-agent-idle bidirectional
  return is a non-goal, not pursued — NO kernel patch is or will be required.)

## J-SEC-003 back-propagation (flagged, NOT self-applied)

The DISCUSS job + slices 00–05 were authored on the in-band "agent fully out,
restart-survivable, kTLS on the workload's own socket" model. Those properties no
longer hold in v1. The enforcement topology is now proxy-shaped (2 sockets/conn;
agent-light return). Flagged for the product-owner in `design/upstream-changes.md`.
The architect does NOT edit `jobs.yaml` or the slice files.

## Density & triggers

`lean` + `ask-intelligent`. Tier-1 `[REF]` sections emitted. No Tier-2 auto-render.
This is a formalize-the-locked-decision dispatch — the heavy reasoning lives in the
6 spike findings + 3 research docs + ADR-0069; the wave records the decision and
the decomposition, not a fresh investigation.

## Review

`review_enabled: true`. A per-wave peer review (solution-architect-reviewer) is
**warranted but the value is bounded** here: the central decision is user-LOCKED on
exhaustive empirical evidence (not an architect bias-prone choice), and the primary
review risks the critique dimensions target (resume-driven dev, technology bias,
missing alternatives) are pre-empted — the ADR carries 4 alternatives with rejection
rationale, all OSS, all kernel-source-pinned. The HIGH-value review target was
**OQ-1** (the `MtlsEnforcement` signature) — now **ACCEPTED (user-approved
2026-06-12)**; the contract is pinned and is what DELIVER implements to. No gating
deferrals remain (in-band restart-survival/density is out of v1 scope — a post-v1
optimization tracked in **#231**; multi-node transparent mTLS is simply out of v1
scope, no forward-pointer issue; OQ-2 is resolved — extend `overdrive-dataplane` +
`overdrive-bpf`); a full reviewer pass is optional and lower-yield than the
now-accepted contract.

## Review revisions (adversarial review — rejected pending revisions, 2026-06-12)

A peer review correctly **rejected the design pending revisions** (strong on
kernel-spike evidence, not yet safe to hand to DELIVER). The core fold decision
(ADR-0069's universal agent-light L4 proxy) is UNCHANGED; OQ-2 and SD-1…SD-4 are
UNCHANGED. The five findings are folded in as safety/scope/robustness revisions
(additive fields/variants + a gate + documentation), not a re-open. **No GH issues
were created; only verified existing issues (#27, #38, #178; #49/#61 related) are
cited.**

| Finding | Severity | Resolution | Where |
|---|---|---|---|
| **F1 — authn ≠ authz; expected-destination not pinned** | CRITICAL | **Authorization is a SEPARATE, already-tracked subsystem** — the BPF-LSM `socket_connect` hook (#27) fed by compiled `policy_verdicts` (#38; related #49); the proxy does authn + encryption, NOT authz, and MUST NOT embed a policy engine. **Expected-destination SAN-match** depends on east-west SPIFFE-ID resolution (#178, downstream of #26; VIP path #61) — v1 #26 is **chain-to-trust-bundle authn only** (keep `AbsentSvid`/`PeerVerificationFailed`, fail-closed). Added an OPTIONAL `expected_peer: Option<SpiffeId>` to `InterceptedConnection` + a reserved `PeerIdentityMismatch` variant (v1 `None`, wires with #178) + a negative-test placeholder for the wrong-but-valid-peer case (gated on #178). The policy verdict is NOT duplicated. | ADR-0069 § Decision "What this does NOT do" + § Enforcement + § References; feature-delta contract (module docstring, `InterceptedConnection.expected_peer`, `enforce` postcondition/edge-case, `PeerIdentityMismatch`) |
| **F2 — "fully de-risked" overstated; three narrow composition gaps remain** | HIGH | Softened "fully de-risked" → **the primitives are de-risked AND the composed INBOUND flow is spike-verified** (`spike/findings-inbound-intercept.md` increment-i §2: real TPROXY intercept → `getsockname` orig-dst → server-side mutual-TLS verifying C's client SVID chains to the bundle → kTLS-RX → agent-light splice → byte-exact plaintext at S; fail-closed on nocert/wrongca). What remains is **THREE NARROW composition gaps**, not "the composition": (1) outbound composed in ONE flow, (2) bidirectional steady-state round-trip, (3) real netns/veth topology + cgroup-isolated workloads. (The earlier "increment-e steady-state RST" framing was a throwaway-harness intercept-lifecycle artifact, **NOT a kernel finding** — `spike/findings-egress-ktls-splice.md` increment-f later proved the steady-state egress kTLS splice cleanly, agent-idle, 15/15, superseding it.) Slice 00 is therefore a **BLOCKING first DELIVER slice = an integration / walking-skeleton GATE that closes the three narrow gaps** (NOT a "prove the mechanism" gate): a composed Tier-3 acceptance test (real `cgroup_connect4` intercept → pre-arm write → handshake → kTLS arm → post-arm bidirectional multi-record transfer with NO RST, under normal AND traced/delayed timing) — supersedes the old in-band walking skeleton. | ADR-0069 § Context (evidence base), § Consequences/Negative, § Enforcement; feature-delta DESIGN Handoff + equivalence-harness obligations; upstream-changes.md Slice 00 |
| **F4 — pre-arm buffer has no resource contract (DoS)** | HIGH | Added the `MtlsLimits` resource contract (bounded `max_prearm_bytes`, `handshake_deadline`, `max_inflight_per_alloc`) as a construction param + cause-distinct fail-closed variants `BufferLimitExceeded` / `HandshakeTimeout` / `InFlightLimitExceeded` (no `Internal(String)`). Fail-closed cleanup total (drop buffer + reset leg, no leak); backpressure = refuse, never queue-unbounded. Metrics/observability noted. Limit + cleanup tests added to the design's test obligations. | ADR-0069 § Consequences "Resource & robustness constraints" + § Enforcement; feature-delta contract (`MtlsLimits`, the three variants, `enforce` edge-cases, equivalence-harness limit branches) |
| **F5 — intercept recursion / agent-leg-B exemption underspecified** | MEDIUM | Pinned the exemption mechanism — a narrowly-scoped `SO_MARK` socket-mark bypass the `cgroup_connect4` program checks-and-skips OR cgroup scoping (the existing `cgroup_connect4_service` attach boundary: program attaches to the *workload* subtree, not the agent's). Two Tier-3 obligations: (a) agent leg B NOT re-intercepted; (b) workload CANNOT self-exempt (bypass is agent-private). | ADR-0069 § Consequences "intercept-recursion exemption" + § Enforcement; feature-delta `enforce` postcondition + equivalence-harness F5 obligations |

**Finding 1 scope resolution (explicit).** Authorization → **#27/#38** (BPF-LSM
`socket_connect` + `policy_verdicts`; related #49), NOT this feature.
Expected-destination SAN-match → **#178** (native east-west SPIFFE-ID resolution,
downstream of #26; VIP path #61). v1 #26 = authn (chain-to-trust-bundle,
fail-closed) + encryption only; `expected_peer`/`PeerIdentityMismatch` reserved
and wired when #178 lands. The policy verdict is NOT embedded in the mTLS
contract.

**Unchanged (confirmed):** the core fold decision (D-MTLS-1, ADR-0069 universal
agent-light L4 proxy), OQ-2 (extend `overdrive-dataplane` + `overdrive-bpf`; no new
crate), the OQ-1 contract's **4-method shape** (`probe`/`enforce`/`liveness`/`teardown`),
and **SD-1…SD-4** (owned-`OwnedFd` payload; port-owns-pump; async `probe`; point-query
liveness). F1/F4/F5 are ADDITIVE fields/variants on that shape; F2 is a test gate;
nothing in the locked decision moved.

## RE-review revisions (adversarial RE-review F3–F7, 2026-06-12)

A second adversarial review (`design/review-adversarial-2026-06-12.md`) accepted
the fold + OQ-2 + SD-1…SD-4 + the prior F1/F4/F5 fixes (all LOCKED, unchanged) and
flagged five remaining gaps. The inbound mechanism is now spike-PROVEN
(`findings-inbound-intercept.md`). The core decision did NOT move; the contract is
extended bidirectionally + the F4–F7 robustness/scope gaps closed. **No GH issues
created; only verified existing issues (#222, #178, #27/#38) cited.**

| Finding | Severity | Resolution | Where |
|---|---|---|---|
| **F3 — inbound/passive half not designed** | CRITICAL | Designed the inbound half as a first-class path (now spike-PROVEN on 7.0). Fixed the model: BOTH workloads are identity-unaware; each node's agent does its side (client-side outbound + server-side inbound). The contract is now BIDIRECTIONAL — `InterceptedConnection` carries `direction: Direction { Outbound, Inbound }` + a `Routed { Outbound { peer } \| Inbound { orig_dst } }` routing fact; `enforce` dispatches on it (NOT a sibling method). Inbound mechanism = TPROXY intercept → `getsockname` orig-dst → server-SVID selection → `WebPkiClientVerifier` client-auth → kTLS-RX arm → splice-to-server (agent-light); fail-closed on `nocert`/`wrongca`. Fixed the C4 self-contradiction ("peer presents its own SVID" → the peer's AGENT presents the peer workload's SVID). | ADR-0069 § Decision (bidirectional model + inbound topology + facts 8/9) + § Enforcement (inbound Tier-3) + § References; feature-delta contract (`Direction`/`Routed`, `enforce` inbound postconditions/edge-cases, bidirectional harness); `c4-diagrams.md` (L1/L2 fix + L3 inbound diagram) |
| **F4 — guest-stack adapter handoff missing** | MEDIUM | Added a guest-stack adapter handoff section: tap/TPROXY/TC intercept source → virtio-net/tap flow → `AllocationId` lookup → orig-dst recovery → SAME `InterceptedConnection`. STAGED to **#222** (repurposed to "the guest-stack intercept adapter for the #26 universal proxy"). Fixed the stale product journey's "#222 is a SEPARATE feature" line → "the staged guest-stack adapter of the universal proxy." | feature-delta § "Guest-stack adapter handoff — STAGED to #222"; `docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml` (`deferred_outside_this_journey` line) |
| **F5 — authn-only v1 boundary must be impossible to misread** | HIGH | Scoped the claim honestly EVERYWHERE: v1 = "chain-to-bundle transport authentication + encryption; NO intended-peer identity pinning; a routing bug / VIP collision / malicious in-cluster endpoint presenting a valid-but-unintended SVID is NOT prevented in v1." Pinned **#178 as the UPGRADE** (not a v1 prereq). The wrong-but-valid-peer test stays `#[ignore]`-gated on #178; docs/tests MUST NOT call that case "protected" until #178 lands. | ADR-0069 § Decision "The honest v1 security claim" + § Enforcement; feature-delta (module docstring, `enforce` postcondition, `expected_peer` field); `brief.md` § 8; this file (locked-decision scope) |
| **F6 — return-pump supervision policy (not just observation)** | MEDIUM | Specified the policy: progress metric = bytes-spliced advancing; stall threshold = `pump_stall_deadline` (30 s) with a record pending; reactor = the worker (point-query on tick); action = **teardown + fail-closed reset** (justified over reconnect/degrade/refuse); telemetry = `mtls.pump.stalled` / `mtls.pump.teardown_on_stall`; acceptance test = inject a stalled pump → `Stalled` → worker teardown → `Gone`, no leak. Added `pump_stall_deadline` to `MtlsLimits`. | ADR-0069 § ATAM (pump supervision policy); feature-delta (`liveness` § "F6 supervision policy", `PumpLiveness::Stalled`, `MtlsLimits::pump_stall_deadline`, harness F6 branch) |
| **F7 — concrete resource limits** | MEDIUM | Pinned CONCRETE defaults + budget: `max_prearm_bytes = 256 KiB`, `handshake_deadline = 5 s`, `max_inflight_per_alloc = 128`, `pump_stall_deadline = 30 s`; per-conn ≤ 256 KiB + ~3 fds, per-alloc ≤ 32 MiB + ≤ 384 fds in-flight, per-node sized vs `RLIMIT_NOFILE`. Acceptance asserts the VALUES, not field existence. Operator-tunability of `MtlsLimits` is a SEPARATE deferral — tracked in #230 (created 2026-06-12). | ADR-0069 § "Resource & robustness constraints" (values + budget); feature-delta (`MtlsLimits` doc + `Default` impl + budget, harness value-assertions, #230) |

**Operator-tunable limits — tracked in #230 (created 2026-06-12).** The F7 values
are compile-time, NOT operator-tunable in v1. **Operator-tunability of `MtlsLimits`
is tracked in #230**; the v1 defaults stand as pinned, un-tunable, compile-time
constants until that work lands.

**Unchanged (re-confirmed):** the fold (D-MTLS-1, ADR-0069), OQ-2, SD-1…SD-4, the
4-method shape, and the prior F1/F4/F5 fixes. F3 adds the `direction`/`Routed`
fields (additive); F6 adds `pump_stall_deadline` (additive); F7 pins values
(no new fields beyond `pump_stall_deadline`); F4/F5 are scope/doc. Nothing in the
locked decision moved. The contract is **ACCEPTED (user-approved 2026-06-12)**
(bidirectional + F4–F7 revised).

## Forward-mechanism pivot (D-MTLS-13, 2026-06-13 — back-propagation to a SHIPPED + verified change)

A mechanism change has **already shipped and been verified 20/20 on the real
kernel** (commit `bb6489ef`); this section reconciles the design artifacts to it.
**This is NOT a re-open or a new decision the architect made** — it records a
mechanism the user queued for back-propagation after it was implemented and
proven. The core fold (D-MTLS-1, ADR-0069), OQ-2, SD-1…SD-4, the 4-method contract
shape, the leg-B kTLS arm, the lossless pre-arm capture, the agent-light
return/deliver/response splice pumps, the no-psock invariant (D-MTLS-5), and the
fail-closed/confidentiality model are ALL UNCHANGED.

**What changed.** The OUTBOUND forward (encrypt) direction retired the agent-idle
in-kernel **sockmap egress redirect** (`sk_skb/stream_verdict` +
`bpf_sk_redirect_map(flags=0)` into leg B's kTLS-TX) for an **agent-light bounded
`read(legF) → write_all(legB)` COPY** into leg B's kTLS-TX. The kernel
`tls_sw_sendmsg` encrypts each blocking `write`; the agent does ZERO crypto, but it
DOES copy each record's plaintext through a userspace buffer and issues a
`read`+`write` per record — **NOT zero-copy, NOT agent-idle, and NOT symmetric to
the return/deliver pumps** (those `splice` zero-copy out of kTLS-RX). A `splice`
INTO kTLS-TX is NOT used (it loses records — the same `MSG_DONTWAIT` loss class the
redirect suffered). The inbound response leg (S→C) uses the SAME `write_all`-into-
kTLS-TX copy. The whole sockmap apparatus
(`MTLS_SOCKMAP`/`MTLS_FPORT`/`MTLS_ARMED`, the verdict program, the
`sock_ops_mtls_enroll` enroll program, the ARMED gate, the engagement poll) is
DELETED. A **kTLS 0.5-RTT early-data drain** was added to every reader leg: drain
`conn.reader()` of already-decrypted early application_data before
`dangerous_extract_secrets` arms kTLS-RX (`mtls::drain_early_plaintext`; the
extracted `rx` `rec_seq` already accounts for the over-read records, so early data
left only in `conn.reader()` would otherwise be silently dropped).

**Why (the evidence — kernel-source-primary + a spike + the shipped code, the
SSOT).**
- `docs/research/dataplane/sockmap-egress-redirect-into-ktls-tx-delivery-research.md`
  (v6.6 ≡ v6.12): the `sk_skb` egress redirect ENQUEUES on leg B's psock and defers
  delivery to a `MSG_DONTWAIT` workqueue (`sk_psock_backlog → skb_send_sock →
  tls_sw_sendmsg`) that `-EAGAIN`-stalls ~10–15% of records (`redirect_ok` counts
  the *enqueue*, not the *delivery*). No production system runs the pattern (Istio
  ztunnel = userspace rustls; Cilium = network-layer IPsec/WireGuard + sockmap only
  for *plaintext* localhost-bypass); the kernel does not test it. A synchronous
  blocking userspace `write_all` into kTLS-TX (no `MSG_DONTWAIT`/backlog) is the
  correct, reliable mechanism. **(The research doc offered "`read → write` OR
  `splice`"; the implementation found `splice` INTO kTLS-TX has the SAME
  `MSG_DONTWAIT` loss — trace-confirmed `n_out=55 errno=0`, peer received 0 — so the
  shipped forward is the blocking `write_all`, NOT a splice. It is therefore NOT the
  same shape as the return pump, which `splice`s zero-copy OUT of kTLS-RX; both are
  agent-light, but the forward is a userspace copy and the return is zero-copy.)**
- `docs/research/dataplane/sockmap-strparser-engagement-race-research.md` and
  `spike/findings-sockmap-engagement-inkernel-enroll.md`: the in-kernel sockops
  enroll closed the engagement race deterministically, but the redirect-delivery
  residual above remained → the whole sockmap forward path (enroll spike included)
  is retired.
- The shipped `crates/overdrive-dataplane/src/mtls/` code is the SSOT for the
  mechanism (the contract code in `crates/overdrive-core/src/traits/mtls_enforcement.rs`
  is already aligned: `ProbeSentinel` is now only `KtlsArmRoundTrip`;
  `ArmingOrderViolation` and `ForwardRedirectFailed` are removed; the OUTBOUND
  `enforce` postcondition is agent-light).

| Decision | Status after 2026-06-13 | Where reconciled |
|---|---|---|
| **D-MTLS-4** (forward/return mechanism) | **REVISED** — forward and return are BOTH agent-light (no userspace crypto) but NOT the same primitive: encrypt/TX (forward, inbound response) = `read → write_all` COPY into kTLS-TX (per-record `read`+`write`, NOT zero-copy — a splice into kTLS-TX loses records); decrypt/RX (return, inbound deliver) = zero-copy `splice` out of kTLS-RX. The agent-idle sockmap-egress was retired | ADR-0069 (2026-06-13 amendment + Decision facts 3/4); feature-delta Decisions Table + the embedded contract + Traceability matrix + Tech Choices + glossary; slice-00/01/02/03/04 |
| **D-MTLS-5** (no psock on the kTLS-RX leg) | UNCHANGED, strengthened — now no psock on ANY leg (the sockmap is gone) | ADR-0069 fact 4; feature-delta Decisions Table |
| **D-MTLS-7** (sockmap-before-`TCP_ULP` invariant) | **MOOT / SUPERSEDED** — no sockmap insert sequenced against `TCP_ULP` on any leg; the `tls-ULP-after-sockmap == EINVAL` Tier-3 test is retired (true kernel fact, governs no code path) | ADR-0069 fact 5 + Decision fact 3 note; feature-delta Decisions Table + Traceability matrix + glossary + per-method anchor table |
| **D-MTLS-12 / SD-5** (rcgen sentinel cert) | **STILL LIVE** — VERIFIED against the shipped probe: `run_probe_sentinels` STILL does a loopback rustls handshake for the kTLS-arm round-trip, so the throwaway-`rcgen`-sentinel core is unchanged and the `overdrive-dataplane → rcgen` production-dep edge still ships. ONLY the *sockmap-engagement / ARMED-gate* portion of the probe's substrate-lie catalogue was mooted (the `ForwardEgressRedirect`/`ArmingOrderEinval` sub-sentinels) | ADR-0069 Earned-Trust probe §; feature-delta Decisions Table D-MTLS-12 note + `ProbeSentinel` enum + `probe` doc |
| **D-MTLS-13** (NEW) | the pivot itself + the kTLS 0.5-RTT early-data drain | this section; ADR-0069 (2026-06-13 amendment); feature-delta Decisions Table D-MTLS-13 |

**Probe surface (reconciled to the shipped contract).** `ProbeSentinel` is now ONE
variant, `KtlsArmRoundTrip` (kTLS arm + agent-light forward-encrypt round-trip on a
loopback sentinel — the forward `read → write_all` copy into kTLS-TX, NOT a splice
into TX; reader leg drains 0.5-RTT early data). The obsolete `ForwardEgressRedirect`
and `ArmingOrderEinval` sub-sentinels, and the `ArmingOrderViolation` /
`ForwardRedirectFailed` `MtlsEnforcementError` variants, are GONE — there is no
redirect to fire and no sockmap-insert ordering to violate.

**Code-vs-design check (no contradiction surfaced).** The shipped contract code
(`mtls_enforcement.rs`) and the shipped mechanism (`mtls/`) AGREE with everything
documented above — `ProbeSentinel::KtlsArmRoundTrip` only, no
`ArmingOrderViolation`/`ForwardRedirectFailed`, the OUTBOUND forward `enforce`
postcondition is agent-light-but-a-`write_all`-copy (NOT zero-copy, NOT a splice),
and `mtls::drain_early_plaintext` on every reader leg. No
design-vs-code disagreement was found; the back-propagation is a clean
narrative-to-shipped-code reconciliation. (Per the dispatch constraint, no
`crates/**` code was touched.)

## Intercept-surface boundary reconciliation (D-MTLS-14, 2026-06-13 — 02-01 ↔ 05-01)

DELIVER step `02-01` ("Transparent intercept + leg-acquire") was dispatched and
the crafter correctly **refused to write code**, returning a design-signature
blocker; the orchestrator verified it against the source + contract (real, not a
misread). This section reconciles the roadmap↔design inconsistency the blocker
exposed. **Nothing in the locked decision moves** — the fold (D-MTLS-1,
ADR-0069), OQ-2, SD-1…SD-5, the 4-method contract shape, and the
forward-mechanism pivot (D-MTLS-13) are ALL UNCHANGED. This is a HOME/SCOPE
reconciliation, not a contract change; no new `MtlsEnforcement` method, field, or
variant is added.

**The inconsistency.** Step `02-01`'s `implementation_scope` named a net-new
production file `crates/overdrive-dataplane/src/mtls/intercept.rs` carrying
"leg-F lossless pre-arm capture; inbound `IP_TRANSPARENT` listener + `getsockname`
orig-dst recovery; TPROXY setup." But against the SHIPPED code + the accepted
contract, every one of those is mis-homed:

- **Lossless pre-arm capture** — ALREADY production from `01-01`
  (`crates/overdrive-dataplane/src/mtls/mod.rs::drain_prearm` /
  `drain_recv_queue` / `drain_recv_queue_once`). The `drain_early_plaintext`
  0.5-RTT companion is also already there (D-MTLS-13).
- **Outbound connect-rewrite + the structural F5 exemption** — ALREADY production
  from `01-01` (`crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs`: the
  rewrite + the attach-to-workload-subtree-only exemption). The **inbound** F5
  `SO_MARK` bypass is ALREADY production (`mod.rs::dial_leg_s` +
  `MTLS_LEG_S_DIAL_MARK`).
- **`IP_TRANSPARENT` leg-C listener creation, nft-TPROXY install, `accept()`→build
  `InterceptedConnection`, `getsockname` orig-dst recovery** — exist ONLY in the
  `01-01` test harness
  (`tests/integration/mtls_composed_walking_skeleton/roles.rs::{make_transparent_listener,
  getsockname_orig, accept_leg_f, accept_leg_c_and_orig_dst}` +
  `tests/integration/helpers/mtls_netns_topology.rs::install_tproxy`). These ARE
  genuinely un-productionised — but the contract assigns their home to the
  **composition root**, NOT a `mtls/` adapter file.

**Why the composition root, not the adapter (the contract reading).** SD-1(a) is
explicit: the `InterceptedConnection` payload is "an owned accepted-leg `OwnedFd` +
a `direction`-tagged routing fact + `AllocationId`," and the decision text states
this *deliberately* "couples the contract to 'the worker does the `accept()`,'
which is exactly the proxy model (a feature, not a leak)." `InterceptedConnection`
is the **input to `enforce`** — it is produced by whoever owns the
listeners/`accept()`/orig-dst, which is the worker. There is **no `intercept()`
method** on the 4-method `MtlsEnforcement` trait, and adding one is forbidden
("Implement to the design — never invent API surface"). The shipped `inbound.rs`
confirms the split: `establish` is HANDED the already-recovered `orig_dst`; it does
NOT create the listener or call `getsockname` itself. The `01-01` test docstrings
state the same ("the intercept setup (cgroup_connect4 / nft-TPROXY) + the leg-F/leg-C
listener + the `accept()` are the WORKER's composition-root role … NOT adapter
API"; "the install is the composition root's job").

**The decision — the intercept-setup primitives are composition-root code, pinned
as free functions in `overdrive-worker`, called by the `05-01` boot path; `02-01`
is FOLDED.** The un-productionised primitives (IP_TRANSPARENT listener,
nft-TPROXY install, `accept()`→`InterceptedConnection`, orig-dst recovery) are NOT
adapter surface and NOT a `mtls/` file. They are the worker's intercept-install
+ leg-acquire role and land in `crates/overdrive-worker/src/` as part of `05-01`'s
composition-root wiring (which the external-validity review already created and
which already owns `overdrive-worker/src/`). `02-01` as a distinct "productionise
`intercept.rs`" step is **vacuous** — all three of its candidate production
concerns are either already shipped (`01-01`) or belong to `05-01`. It is folded
into `01-01` (the parts already done) + `05-01` (the parts that remain).

**Pinned signatures (composition-root intercept-setup primitives — `overdrive-worker`).**
These are free functions / a small helper type the `05-01` boot path calls; they
PRODUCE/FEED an `InterceptedConnection` for `enforce`. **None is a method on
`MtlsEnforcement`** (the trait stays exactly `probe`/`enforce`/`liveness`/`teardown`).
Newtypes per house style (`SocketAddrV4`, `OwnedFd`, `AllocationId`); a typed
`thiserror` error; `OwnedFd` ownership handed by value into `InterceptedConnection`.
The bodies productionise the proven `01-01` test-harness primitives verbatim
(`make_transparent_listener` / `getsockname_orig` / `accept_*` / `install_tproxy`).

```rust
//! crates/overdrive-worker/src/mtls_intercept.rs — the worker's intercept-install
//! + leg-acquire role (the composition-root side of SD-1(a)). Produces the
//! `InterceptedConnection` that `HostMtlsEnforcement::enforce` consumes. NOT
//! adapter API; the `MtlsEnforcement` trait is unchanged (4 methods).

use std::net::SocketAddrV4;
use std::os::fd::OwnedFd;

use overdrive_core::AllocationId;
use overdrive_core::traits::mtls_enforcement::{InterceptedConnection, Routed};

/// Cause-distinct failure modes for the worker-side intercept install +
/// leg-acquire. Typed (`thiserror`), no catch-all `Internal(String)`
/// (`.claude/rules/development.md` § Errors). `Display` names the privilege /
/// kernel-feature remediation an operator acts on.
#[derive(Debug, thiserror::Error)]
pub enum InterceptError {
    /// `socket()` / `setsockopt(IP_TRANSPARENT)` / `bind` / `listen` failed
    /// while creating the inbound leg-C listener. `IP_TRANSPARENT` needs
    /// `CAP_NET_ADMIN`; the message names the failing syscall.
    #[error("transparent leg-C listener setup failed on {addr}: {source}")]
    TransparentListener { addr: SocketAddrV4, #[source] source: std::io::Error },
    /// The nft-TPROXY prerouting install (or its `ip rule` / `ip route`
    /// companions) failed — missing `nft_tproxy`, or insufficient privilege.
    #[error("nft-TPROXY intercept install failed: {reason}")]
    TproxyInstall { reason: String },
    /// `accept()` on a leg listener errored or timed out (the intercept did
    /// not deliver a connection).
    #[error("leg accept failed on the {direction} intercept listener: {source}")]
    Accept { direction: &'static str, #[source] source: std::io::Error },
    /// `getsockname()` on the accepted leg-C socket returned no usable
    /// original destination (under TPROXY the orig-dst IS the local addr;
    /// a failure here means the TPROXY redirect did not land).
    #[error("getsockname original-destination recovery failed: {source}")]
    OrigDst { #[source] source: std::io::Error },
}

pub type Result<T, E = InterceptError> = std::result::Result<T, E>;

/// Create the agent's `IP_TRANSPARENT` inbound leg-C listener bound to `addr`
/// (the port the nft-TPROXY rule redirects to). `SO_REUSEADDR` + `IP_TRANSPARENT`
/// + `bind` + `listen`, all under the agent's `CAP_NET_ADMIN`. Productionises
/// `roles.rs::make_transparent_listener`.
pub fn make_transparent_listener(addr: SocketAddrV4) -> Result<std::net::TcpListener>;

/// Install the inbound nft-TPROXY prerouting intercept (+ the `ip rule fwmark`
/// / `ip route local … table` companions) that redirects a connection aimed at
/// `virt` to the agent's leg-C listener on `agent_port`, with the
/// `MTLS_LEG_S_DIAL_MARK` exemption ordered first so the agent's own leg-S dial
/// is not re-intercepted (F5 inbound). Productionises
/// `mtls_netns_topology.rs::install_tproxy`'s production half (the harness's
/// GAP-3 netns DNAT/masquerade is test-only and does NOT productionise — the
/// production adapter dials the orig-dst verbatim, #178). Returns a guard whose
/// `Drop` removes the rule/route/table.
pub fn install_inbound_tproxy(
    virt: SocketAddrV4,
    agent_port: u16,
) -> Result<TproxyInterceptGuard>;

/// RAII guard removing the nft-TPROXY table + `ip rule`/`ip route` on `Drop`.
pub struct TproxyInterceptGuard { /* private: the cleanup argv set */ }

/// Accept the transparently-redirected OUTBOUND workload connection on the
/// agent's leg-F listener and build the `InterceptedConnection` for `enforce`
/// (`Routed::Outbound { peer }`, the real peer leg B dials). The owned leg F is
/// handed by value (the port takes ownership; RAII-closes on teardown).
/// Productionises `roles.rs::accept_leg_f`.
pub fn accept_outbound_leg(
    leg_f_listener: &std::net::TcpListener,
    alloc: AllocationId,
    peer: SocketAddrV4,
) -> Result<InterceptedConnection>;

/// Accept the TPROXY-redirected INBOUND connection on the agent's leg-C
/// listener, recover the original destination via `getsockname` (NOT
/// `SO_ORIGINAL_DST`), and build the `InterceptedConnection`
/// (`Routed::Inbound { orig_dst }`, which selects the server SVID's
/// `AllocationId`). The owned leg C is handed by value. Productionises
/// `roles.rs::{accept_leg_c_and_orig_dst, getsockname_orig}`.
pub fn accept_inbound_leg(
    leg_c_listener: &std::net::TcpListener,
    alloc: AllocationId,
) -> Result<InterceptedConnection>;
```

(`Routed`/`InterceptedConnection` are the existing pinned contract types from
`overdrive_core::traits::mtls_enforcement`; these functions CONSTRUCT them, they
do not extend them. `expected_peer` is `None` in v1 — authn-only, F5/#178.)

**Reconciled `02-01` ↔ `05-01` scope split (no overlap).**

- **`01-01` (DONE, unchanged):** lossless pre-arm capture (`drain_prearm` et al.);
  the outbound `cgroup_connect4_mtls` connect-rewrite + structural F5 exemption;
  the inbound leg-S `SO_MARK` F5 bypass (`dial_leg_s` / `MTLS_LEG_S_DIAL_MARK`); the
  0.5-RTT early-data drain. The intercept-setup primitives proven IN the test
  harness.
- **`02-01` — FOLDED (removed as a distinct DELIVER step).** Every candidate
  production concern is either already shipped (`01-01`) or belongs to `05-01`;
  there is no non-duplicative, non-already-done residue, and the only way to make
  it net-new would be inventing a forbidden adapter `intercept()` method. The
  `02-01` ACs are re-homed: AC1 (lossless capture) → `01-01` (done); AC3 (F5
  no-recursion mechanism) → `01-01` (done); AC2 (IP_TRANSPARENT listener +
  getsockname orig-dst), AC4 (CAP_NET_ADMIN intercept/listener setup), AC5
  (accepted connection enforceable via the port without re-deriving routing from an
  unsafe tuple — SD-1(a)) → `05-01` (the composition-root primitives above + the
  e2e gate). The `crates/overdrive-dataplane/src/mtls/intercept.rs` file is NOT
  created.
- **`05-01` (the home for the remaining intercept work):** the `overdrive-worker`
  composition-root primitives above (`mtls_intercept.rs`) + the `run_server`
  wire→probe→use of `HostMtlsEnforcement` + the end-to-end Tier-3 gate. `05-01`
  now lands the intercept-listener creation / TPROXY install / accept→
  `InterceptedConnection` / orig-dst recovery that `02-01` mis-homed to the
  adapter, **as the worker's role**, and proves them through the e2e deploy gate
  (a workload deployed via `overdrive deploy <SPEC>` produces TLS 1.3 on its
  peer-facing leg). No `mtls/intercept.rs`; no new trait surface.

**Net dependency effect.** The happy-path chain is unchanged in ORDER
(intercept → handshake → enforce → guardrails → activation); `02-02` (agent
handshake) now depends on `01-01` directly (the intercept + leg-acquire foundation
`02-01` was nominally productionising is already in `01-01` for the adapter-test
path; the *production* intercept install moves to `05-01`, which is downstream of
`02-02`/`02-03`/`03-01`/`04-01` and is where the e2e activation belongs). The
re-dispatch instruction: **skip `02-01` (folded) and proceed to `02-02`**; land the
intercept-setup primitives as part of `05-01`.

| Decision | Status | Where reconciled |
|---|---|---|
| **D-MTLS-14** (NEW) | `02-01` intercept-surface boundary: the intercept-setup primitives are composition-root (`overdrive-worker`) free functions feeding `InterceptedConnection`, NOT a `mtls/` adapter file and NOT a trait method; `02-01` FOLDED into `01-01` (done) + `05-01` (remaining). The 4-method contract is UNCHANGED. | this section; `deliver/roadmap.json` (`02-01` removed, `05-01` scope + ACs extended, dependency edges); `feature-delta.md` (this primitive signature pin + Traceability); `slices/slice-01-…md` (folded marker) |

**No GH issue created.** This reconciliation creates no deferral — the remaining
intercept work has a concrete home (`05-01`) and the folded step's concerns are all
accounted for. (#178 expected-peer pinning and #230 operator-tunable limits remain
the only standing deferrals, unchanged.)
