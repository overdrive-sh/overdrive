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

## Intercept-INPUT provenance pin (D-MTLS-15, 2026-06-14 — 05-01 worker-seam)

DELIVER step `05-01` (the BLOCKING external-validity gate) wires
`HostMtlsEnforcement` into the production node/worker boot path and productionises
the D-MTLS-14 intercept-setup free functions. The crafter correctly halted: D-MTLS-14
pinned the function *signatures* (`make_transparent_listener(addr)` /
`install_inbound_tproxy(virt, agent_port)` / `accept_outbound_leg(listener, alloc, peer)`
/ `accept_inbound_leg(listener, alloc)`) but NOT the *inputs* that drive them
per-allocation — what tells the worker an allocation needs an intercept, and where it
gets the per-allocation listener bindings and the orig-dst→server resolution. This
section pins those three inputs. **Nothing in the locked contract moves**: the
`MtlsEnforcement` trait stays exactly `probe`/`enforce`/`liveness`/`teardown` (no
`intercept()`, no new method/field/variant), and the D-MTLS-14 free-function
signatures are UNCHANGED. This pins only the *provenance* of their inputs, verified
against the shipped source.

### (1) The needs-intercept signal — DERIVED, no new spec field

**Decision: every host-socket allocation is intercepted by definition; the signal is
`DriverType::Exec`, derived from facts the worker already holds — NOT a new
`AllocationSpec` field.** `AllocationSpec`
(`crates/overdrive-core/src/traits/driver.rs:131`) carries exactly `alloc` /
`identity` / `command` / `args` / `resources` / `probe_descriptors` — no host-socket
flag, and **none is added**. Per ADR-0069 + the feature-delta scope table, v1 is
process/exec ONLY and the agent-light L4 proxy is **universal and undisableable**
(System Constraint "Workload holds NOTHING; the platform does mTLS" — there is no
per-workload opt-in/opt-out). For an `ExecDriver` workload, TCP terminates in the host
kernel, so *every* such allocation is a host-socket workload and is intercepted. The
predicate is therefore `spec.driver_type() == DriverType::Exec` (equivalently: the
worker only runs `ExecDriver` in v1, so the predicate is *unconditionally true* on the
worker's allocation-lifecycle path — guest-stack/#222 and a future WASM driver are
out of v1 scope and route through their own staged adapter when they land).

- **Read-site (the seam):** the existing `Driver::on_alloc_running(&AllocationSpec)`
  hook on `ExecDriver` (`crates/overdrive-worker/src/driver.rs:783`) — the same
  lifecycle seam that already fires `ProbeRunner::start_alloc` after the action-shim
  commits `AllocStatusRow{state: Running}`. The intercept-install + leg-acquire is
  wired here (or in the worker startup path that owns this hook, per the `05-01`
  `implementation_scope` "node/worker startup + allocation lifecycle"). No predicate
  beyond "this is the exec driver" is consulted; `spec.alloc` is the `AllocationId`
  passed straight into `accept_outbound_leg(..., alloc, ...)` /
  `accept_inbound_leg(..., alloc)`.
- **Set-site:** none. There is no new field to set anywhere — the signal is the driver
  class, which the worker already knows by construction (it IS the `ExecDriver`).
- **Why not a spec field:** adding `AllocationSpec.host_socket_mtls: bool` would be a
  derived-state persistence (the value is a pure function of the driver class + the
  ADR-0069 universality decision — "would editing the ADR-0069 scope change the
  field's correctness?" yes ⇒ derived, `development.md` § "Persist inputs, not derived
  state"), AND it would contradict the "undisableable, no per-workload opt-in"
  constraint by making non-interception representable. The driver class is the input;
  interception is computed from it.

### (2) Per-allocation leg-binding source — agent-chosen ephemeral legs; `virt`/`peer` provenance differs by direction

**Decision: the worker chooses BOTH listener bindings (agent-private, ephemeral
loopback `127.0.0.1:0` — the kernel assigns the port); `agent_port` is read back from
the bound listener. The `virt`/`peer` the intercept matches on is direction-specific
and is NOT agent-chosen.**

- **leg-F (outbound) listener:** the worker calls `make_transparent_listener` (or a
  plain bound `TcpListener` for leg F — leg F needs no `IP_TRANSPARENT`, only leg C
  does) on an agent-private ephemeral loopback addr; the OS assigns the port. The
  worker then programs the outbound `cgroup_connect4` rewrite to point at that
  leg-F addr.
- **leg-C (inbound) listener:** the worker calls
  `make_transparent_listener(127.0.0.1:0)` → `IP_TRANSPARENT` + bind + listen; reads
  the assigned port back via the listener's local addr; that port is the `agent_port`
  passed to `install_inbound_tproxy(virt, agent_port)`. (`make_transparent_listener`'s
  signature is UNCHANGED — `addr: SocketAddrV4`; the worker passes `127.0.0.1:0` and
  reads `listener.local_addr()` for `agent_port`. The choice of ephemeral-vs-fixed is
  the *caller's*, not a signature change.)
- **`peer` (outbound `Routed::Outbound { peer }`):** the workload's *intended*
  destination, recovered by the OUTBOUND intercept itself — the
  `cgroup_connect4_mtls` program is keyed per intended-peer:
  `MTLS_REDIRECT_DEST[real_peer] = leg_f_listener` (the userspace adapter programs the
  entry; on a map MISS the program passes the connect through unchanged —
  `crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs:62-65`). So `peer` is the
  pre-programmed `real_peer` key the worker installed, handed verbatim into
  `accept_outbound_leg(leg_f_listener, alloc, peer)`. **This is the load-bearing
  subtlety the residual gap below turns on:** the worker must know *which destination(s)
  to intercept* and program `MTLS_REDIRECT_DEST[peer]` **before** the workload connects
  — see (3) / the residual.
- **`virt` (inbound `install_inbound_tproxy(virt, agent_port)`):** the server
  workload's *logical* address (the addr a client aims at). In single-node v1 this is
  the loopback addr the server workload listens on; the TPROXY prerouting rule matches
  `ip daddr <virt-ip> tcp dport <virt-port> → tproxy to 127.0.0.1:<agent_port>`
  (`findings-inbound-intercept.md` §Architecture). `getsockname()` on the accepted
  leg-C socket then recovers that same `virt` as the orig-dst
  (`accept_inbound_leg` → `Routed::Inbound { orig_dst }`,
  `crates/overdrive-dataplane/src/mtls/inbound.rs:30-37,56-65`).

### (3) orig-dst → server-listener resolution — DEFERRED to #178; GAP-3 stays test-only; AC5 is OUTBOUND-proven

**Decision: the production inbound orig-dst → server-real-listener map is east-west
SPIFFE-ID/service resolution, which is #178 (OPEN, out of v1 scope) — explicitly
DEFERRED. The shipped inbound adapter dials the orig-dst VERBATIM
(`server_dial_addr(orig_dst) = orig_dst`,
`crates/overdrive-dataplane/src/mtls/inbound.rs:96-98`), which is correct for the
single-node v1 topology where the server workload's real listener IS its logical
orig-dst (loopback). The test-only nft DNAT/masquerade that fakes a distinct
server-real-listener hop (the harness "GAP-3" in
`crates/overdrive-dataplane/tests/integration/helpers/mtls_netns_topology.rs::install_tproxy`)
does NOT productionise** — `install_inbound_tproxy`'s body productionises only the
TPROXY-prerouting + `ip rule fwmark` + `ip route local … table` half (the D-MTLS-14
docstring already says this). **The single production-inbound-routing site that #178
will eventually supply is `server_dial_addr` in `mtls/inbound.rs`** — today the
identity function (orig-dst verbatim); when #178 lands it consults the local
ObservationStore east-west map to translate a service VIP/logical orig-dst into the
selected backend's real listener. No code change is made now; this names the site.

**AC5 (the `05-01` external-validity gate) is proven on the OUTBOUND peer-facing leg,
which needs ZERO #178 dependency.** The e2e gate
(`mtls_production_activation.rs`, `criteria[4]`: "a normal exec workload deployed via
`overdrive deploy <SPEC>` produces TLS 1.3 records on its peer-facing leg") is the
OUTBOUND client path: the deployed workload connects out to a known peer, the worker
programs `MTLS_REDIRECT_DEST[peer]`, intercepts to leg F, the agent client-handshakes
on leg B presenting the held SVID, and `tcpdump` shows `0x17` on the peer wire. East-west
resolution (#178) is an INBOUND-server concern (which real backend to dial after
TPROXY); the outbound proof does not touch it.

### Residual (surfaced as a scoped BLOCKER for the orchestrator, NOT invented past)

The OUTBOUND intercept is **per-destination** (`MTLS_REDIRECT_DEST[real_peer] =
leg_f`; miss ⇒ pass-through). For an *arbitrary* outbound workload, "the set of peers
this workload will dial" has **no production source in v1** — that enumeration is
east-west service resolution (#178/#61), DEFERRED. The shipped composed-WS / outbound
harnesses sidestep this by being *handed* the peer address by the test
(`OutboundWorkload::run(topo, peer.addr(), …)` programs the one
`MTLS_REDIRECT_DEST[peer]` entry itself —
`mtls_composed_walking_skeleton.rs:175-184`). **Consequence for `05-01`:** the e2e AC5
gate must drive a workload that connects to a **known/declared peer address** (the
deploy fixture names the destination; the worker programs that one
`MTLS_REDIRECT_DEST` entry on the alloc-lifecycle path), exactly as the proven harness
does — it CANNOT prove "every arbitrary outbound connection is auto-intercepted"
without #178/#61. This is sufficient for the external-validity gate (it proves the
*production boot path* enforces a real deployed workload's connection end-to-end), but
the orchestrator must confirm the AC5 fixture is shaped as "deploy a workload that
dials a known peer," not "deploy a workload and assert all egress is intercepted." If
the intended AC5 scope was the latter (auto-intercept of undeclared peers), that is a
#178/#61-blocked scope the worker-seam pin cannot satisfy and the step is over-scoped
— surface to the user before implementing. **No GH issue is created here; #178 and #61
already exist and cover the deferred east-west resolution. No new public API, spec
field, trait method, or aggregate surface is introduced by this pin.**

### Contract-unchanged statement

This pin changes NOTHING in the locked contract. The `MtlsEnforcement` trait remains
exactly `probe` / `enforce` / `liveness` / `teardown` (no `intercept()`; no new
method, field, or variant). The D-MTLS-14 free-function signatures
(`make_transparent_listener` / `install_inbound_tproxy` / `accept_outbound_leg` /
`accept_inbound_leg` / `TproxyInterceptGuard`) are UNCHANGED — this section specifies
only the INPUT provenance that drives them per-allocation. `InterceptedConnection`,
`Routed::{Outbound{peer}, Inbound{orig_dst}}`, and `expected_peer: None` (v1
authn-only) are the existing pinned contract types, CONSTRUCTED (never extended) by
these functions. No new `AllocationSpec` field is added — the needs-intercept signal
is derived from `DriverType::Exec`.

| Decision | Status | Where reconciled |
|---|---|---|
| **D-MTLS-15** (NEW) | `05-01` intercept-INPUT provenance: (1) needs-intercept = `DriverType::Exec`, derived (no new `AllocationSpec` field), read at `Driver::on_alloc_running`; (2) legs = agent-chosen ephemeral `127.0.0.1:0`, `agent_port` from the bound listener, outbound `peer` from the pre-programmed `MTLS_REDIRECT_DEST[real_peer]` key, inbound `virt` = server logical addr recovered as orig-dst via `getsockname`; (3) orig-dst→server-real-listener DEFERRED to #178 (`server_dial_addr` is the named site; GAP-3 stays test-only), AC5 OUTBOUND-proven. 4-method contract + D-MTLS-14 signatures UNCHANGED. | this section; `feature-delta.md` (D-MTLS-14 input-provenance note) |

## Connection-liveness supervision shape (D-MTLS-16, 2026-06-14 — supersedes the SD-4 / F6 central-point-query shape; ADR-0070)

A user-ratified decision settles **how transparent-mTLS connection liveness is
supervised in v1**. This supersedes the supervision *shape* the prior F6
amendment (RE-review F6, 2026-06-12) and SD-4 pinned — "the worker
point-queries `liveness(&handle)` on its reconciler-tick cadence (SD-4
point-query)" — which was shape **(A)**, a central tick enumerator over the
live-connection set. **Recorded in ADR-0070.** Decided on
`docs/research/dataplane/transparent-mtls-connection-supervision-research.md`
(22 sources): per-connection self-supervision is the **universal** production
pattern (Envoy/ztunnel/linkerd2-proxy/Cilium); **no surveyed dataplane uses a
central liveness enumerator**; and `.claude/rules/reconcilers.md` independently
disqualifies (A) (a stalled connection is not desired-vs-actual *config* drift,
the connection's own task is the natural owner of its death, per-tick
enumeration is the wrong granularity).

**Nothing in ADR-0069's locked core moves.** The universal/undisableable
agent-light proxy model (D-MTLS-1), the fold, OQ-2, **SD-1(a)**, **SD-2
(port-owns-pump — UNCHANGED)**, **SD-3**, the 4-method `MtlsEnforcement`
contract, F3, F4/F7, F5, the authn-only boundary, and D-MTLS-13/14/15 are ALL
unchanged. This refines exactly one thing: the **F6 supervision shape**.

### The decision — (C) + (B), reject (A)

- **(C) kernel TCP timeouts on the spliced legs** — the host adapter sets
  `TCP_USER_TIMEOUT` + keepalive on each enforced connection's legs during
  `enforce` (before starting the SD-2 pumps); the kernel reaps the entire
  **transport-dead** class (peer gone, unacked-past-deadline, half-open) with
  no userspace loop. Direct production precedent: Linkerd's `TCP_USER_TIMEOUT`
  fix (#13023), ztunnel's default-on keepalive (1.24+). The pump task observes
  the resulting `ETIMEDOUT`/EOF/RST and self-resolves.
- **(B) per-connection self-supervision** — each connection's own SD-2
  port-owned enforce task owns its full lifecycle and **self-tears-down
  fail-closed** on EOF/error/`ETIMEDOUT` (close both legs, stop both pumps,
  reclaim kTLS state — the same fail-closed teardown F6 specified, now
  triggered by the connection's own task, not a central worker query). No
  central registry, no `supervise_tick`, no tick cadence, no enumeration.
- **(A) central tick enumerator — REJECTED and retired.** The
  `MtlsSupervisor` (step 04-01) is the concrete instance; it is deleted (see
  below).

**The genuinely-hard residual is DEFERRED, not solved.** The
**kernel-invisible progress-stall** (a `splice`/kTLS pump stuck while the
sockets look transport-healthy, a record pending but not advancing) is the one
class neither (C) nor a transport signal covers. The kernel cannot detect it
(research Finding 5.3), and the app-level progress predicate for a
**kTLS-spliced** pump (`tcpi_notsent_bytes` vs kTLS record sequence vs `splice`
return) is **undocumented upstream** (research Gap 2) — a kernel-mediated
mechanism with no test backstop, so **Tier-3-spike before locking** (the
standing project rule). **Deferred to
[#232](https://github.com/overdrive-sh/overdrive/issues/232).** v1 ships
(C)+(B), which covers transport-death + crashed-pump for real. The
`PumpLiveness::Stalled` predicate is RETAINED on the contract as the reserved
hook for that deferred per-connection watchdog (#232; NOT a central loop).

**The policy plane is the future home of a central registry — NOT v1 liveness
(forward design rationale, not tracked v1 work).** A central connection
registry + control loop IS the right shape for the FUTURE revocation /
policy-driven force-close concern (Phase 5; the ztunnel `ConnectionManager`
precedent — graceful drain on authz/identity change). That is config
reconciliation projected onto connections, not liveness reaping. This note is
forward design rationale (why the central-registry shape is right for that
future concern), not a tracked unit of deferred work — no dedicated issue; the
future home is the existing
[#37](https://github.com/overdrive-sh/overdrive/issues/37) (central per-alloc
live-connection registry + drain detector) and
[#82](https://github.com/overdrive-sh/overdrive/issues/82) (gossip-propagated
revocation), cross-referenced as the related future mechanisms, NOT claimed to
cover "revocation-driven mTLS force-close" as planned work today. Do NOT build
it now; do NOT resurrect the central loop for liveness on the strength of
"we'll need a registry for revocation later" — the two concerns are separate,
and the registry, when it lands, is named for policy.

### 1. `MtlsEnforcement` contract reconciliation — 4-method shape UNCHANGED; `liveness` STAYS (reserved)

**Decision: keep all 4 methods; keep `PumpLiveness`'s three variants; reframe
the F6 supervision *consumer* in the `liveness` docstring from "central worker
point-query (SD-4)" to "(C) kernel + (B) per-connection self-teardown; `liveness`
is the SD-2 observe surface (the equivalence harness re-queries it for the
`Gone` no-leak assertion) + the reserved predicate for the deferred
progress-stall watchdog." Signatures are byte-for-byte unchanged.**

Justification (against `development.md` § Documentation "no aspirational/dead
surface" AND the single-cut greenfield-migration discipline): `liveness` is NOT
dead surface — it has **live v1 consumers independent of the retired central
loop**:
- the **post-teardown `Gone` observable** the `mtls_enforcement_equivalence`
  harness and the F4 `mtls_guardrails` tests re-query to assert *no fd/kTLS
  leak after teardown* (the SD-2 observe surface + the F4 leak-free invariant —
  genuinely asserted today);
- the **(B) self-supervision verdict** `PumpLiveness::Stalled` (derived by the
  retained pure `derive_liveness` in
  `crates/overdrive-dataplane/src/mtls/supervision.rs`), the predicate the
  per-connection task consumes to self-tear-down + the reserved
  deferred-watchdog hook.

Dropping to a 3-method contract would (a) rip the `Gone` no-leak observable out
of the equivalence harness + the 04-01 guardrail tests, and (b) force a *second*
contract churn (re-adding `liveness` + re-rippling `HostMtlsEnforcement`,
`SimMtlsEnforcement`, the equivalence tests) the moment the Tier-3 spike lands
the watchdog — two churns and a lost observable vs. one docstring reword.
Keeping 4 methods is the cleaner single-cut.

| Surface | Status under (C)+(B) | What changes |
|---|---|---|
| `teardown` | **STAYS, unchanged** | the (B) per-connection task calls it on self-teardown; still Phase-4 close |
| `liveness` | **STAYS (4 methods kept)** | **docstring only** — the "F6 supervision policy" block's "worker point-queries on reconciler-tick cadence (SD-4)" → "(C) kernel `TCP_USER_TIMEOUT`/keepalive + (B) per-connection self-teardown; `liveness` is the SD-2 observe surface (equivalence harness re-queries for `Gone`) + reserved hook for the deferred progress-stall watchdog (Tier-3 spike). No central point-query, no `supervise_tick`, no tick cadence in v1." SD-4's point-query-vs-stream sub-decision is moot for v1 liveness (neither runs) |
| `enforce` | **STAYS, unchanged signature** | gains the (C) `TCP_USER_TIMEOUT`/keepalive leg-setup as an adapter postcondition (an SD-2 HOW, before the pumps start) |
| `probe` | **UNCHANGED** | — |
| `InterceptedConnection` / `EnforcedConnection` / `Routed` / `Direction` | **UNCHANGED** | `EnforcedConnection` stays the opaque `liveness`/`teardown` key |
| `MtlsLimits` (incl. `pump_stall_deadline`) | **UNCHANGED** | `pump_stall_deadline` now the (B) verdict + deferred-watchdog threshold, not a central-tick threshold |
| `PumpLiveness` (`Running`/`Stalled`/`Gone`) | **UNCHANGED — all three variants kept** | `Gone` = post-teardown observable (live); `Running`/`Stalled` = (B) verdict + reserved watchdog predicate |

### 2. Retire the central `MtlsSupervisor` (04-01) — DELETE, not refactor (the crafter deletes; this is the direction)

`crates/overdrive-worker/src/mtls_supervisor.rs` (`MtlsSupervisor` +
`supervise_tick(&[EnforcedConnection])`) is the concrete shape-(A) enumerator.
Per `.claude/rules/development.md` § "Deletion discipline" (removed is removed —
no gate, no salvage, no stub, no relocation), DELIVER **deletes the production
code AND its tests in the same commit**:

- **Delete** `crates/overdrive-worker/src/mtls_supervisor.rs` (full file) and
  its `pub mod mtls_supervisor;` in `overdrive-worker`'s `lib.rs`.
- **Delete** `crates/overdrive-worker/tests/acceptance/mtls_supervisor_teardown_on_stall.rs`
  (both tests) and its module wiring in the acceptance entrypoint.
- This is a **delete, not a refactor-in-place** — the enumerator does NOT
  migrate into the worker boot path. (B) lives inside the SD-2 port-owned
  enforce task (the host adapter), NOT in `overdrive-worker`.
- **Retain** `crates/overdrive-dataplane/src/mtls/supervision.rs`
  (`derive_liveness`) + `PumpLiveness` + `MtlsLimits::pump_stall_deadline` —
  these are the (B) verdict + deferred-watchdog predicate, NOT the enumerator.
  The telemetry events (`mtls.pump.stalled` / `mtls.pump.teardown_on_stall`)
  re-home from the retired `MtlsSupervisor` to the per-connection self-teardown
  path — events survive, emitter moves.

### 3. The 05-01 worker composition under (C)+(B) — pinned (unblocks the crafter)

With (A) gone the registry/tick-loop architecture gap evaporates; 05-01 is the
D-MTLS-14/15 shape + enforce-port injection.

- **Enforce-port injection seam (mandatory param, NOT a builder).** The worker
  component owning the `enforce` call holds `Arc<dyn MtlsEnforcement>` as a
  **required constructor parameter** per `development.md` § "Port-trait
  dependencies" (port deps are mandatory `new()` params; builders are the
  anti-pattern *for `dyn` port traits*). The `ProbeRunner` precedent uses a
  `.with_probe_runner(...)` builder because `ProbeRunner` is a *concrete* type;
  a `dyn` port like `MtlsEnforcement` takes the required-param path. **Name the
  seam:** the field is the `ExecDriver`-owning worker component's
  `Arc<dyn MtlsEnforcement>`; the construction site is the binary composition
  root — `compose_production_driver` / the `run_server` boot path in
  `crates/overdrive-control-plane/src/lib.rs` (~1147–1214, where `ExecDriver` +
  `ProbeRunner` compose today). There the host adapter `HostMtlsEnforcement`
  (over `overdrive-dataplane`'s mTLS surface + `IdentityRead` + `MtlsLimits`)
  is constructed, **probed** (wire → probe → use; `probe()` Ok → usable, fail →
  node refuses to start with `health.startup.refused`), and threaded in as the
  mandatory `new()` param — structurally mirroring
  `compose_and_probe_runner_gate` → `with_probe_runner`, but a required port
  param, not a builder. Test composition injects
  `Arc::new(SimMtlsEnforcement::new(identity, MtlsLimits::default()))`.
- **Lifecycle drive (the established sync-seam → async-spawn precedent).**
  `Driver::on_alloc_running(&AllocationSpec)` (sync, `driver.rs:783` — the same
  hook that fires `ProbeRunner::start_alloc` after the action-shim commits
  `AllocStatusRow{state: Running}`) spawns the per-alloc intercept-and-enforce
  work: the D-MTLS-14/15 intercept-setup primitives accept the intercepted leg
  → build `InterceptedConnection` → `enforce`; the adapter's `enforce` sets the
  (C) `TCP_USER_TIMEOUT`/keepalive on its legs and starts the SD-2 port-owned
  pumps; the pump task self-tears-down (B) on EOF/error. Needs-intercept signal
  = `DriverType::Exec`-derived (D-MTLS-15; no new `AllocationSpec` field).
  `Driver::on_alloc_terminal(&AllocationId)` (`driver.rs:796`) tears down the
  alloc's connections.
- **Per-alloc teardown bookkeeping (NOT a central liveness registry).** Who
  owns the handle set for terminal teardown: a **per-alloc teardown set** — the
  worker component holds, per `AllocationId`, the `EnforcedConnection` handles
  it enforced (a `BTreeMap<AllocationId, Vec<EnforcedConnection>>`-shape set,
  deterministic per `development.md` § "Ordered-collection choice"), drained on
  `on_alloc_terminal`. This is **lifecycle bookkeeping (keyed by alloc
  start/terminal), NOT a liveness loop** — never enumerated each tick, never
  point-queries `liveness` for stall. It is the direct analogue of
  `ProbeRunner`'s per-alloc supervisor map, NOT of the retired `supervise_tick`.
- **State plainly: no central registry, no `supervise_tick`, no tick cadence in
  v1.** The worker holds per-alloc teardown bookkeeping for `on_alloc_terminal`
  and nothing else; liveness is (C) kernel + (B) per-connection task.

### Deferrals (NAMED here)

1. **Kernel-invisible progress-stall watchdog — deferred to #232 (Tier-3
   spike)** — the kTLS-spliced progress predicate is undocumented upstream
   (research Gap 2); v1 ships (C)+(B); `PumpLiveness::Stalled` is the reserved
   hook. Tracked as
   [#232](https://github.com/overdrive-sh/overdrive/issues/232) ("Tier-3
   spike: kernel-invisible progress-stall watchdog for the kTLS-spliced mTLS
   pump (F6 residual)").
2. **Phase-5 policy-plane force-close (revocation / authz drain) — forward
   design rationale, NOT a tracked unit of v1 deferred work** — a central
   registry IS the right shape THERE (ztunnel `ConnectionManager`), NOT for v1
   liveness; out of #26 v1 scope. This is forward design rationale (why the
   central-registry shape is right for a *future* policy-plane concern), not a
   tracked unit of deferred work, and gets no dedicated issue. Future home is
   the existing [#37](https://github.com/overdrive-sh/overdrive/issues/37)
   (central per-alloc live-connection registry + drain detector) and
   [#82](https://github.com/overdrive-sh/overdrive/issues/82) (gossip-
   propagated revocation) — cross-referenced as the related future mechanisms,
   NOT claimed to cover "revocation-driven mTLS force-close" as planned work
   today.

| Decision | Status | Where reconciled |
|---|---|---|
| **D-MTLS-16** (NEW) | Connection-liveness supervision shape: **(C) kernel `TCP_USER_TIMEOUT`/keepalive + (B) per-connection self-supervision; reject (A) the central tick enumerator** (supersedes the SD-4 / F6 central-point-query shape). 4-method `MtlsEnforcement` contract UNCHANGED; `liveness`/`PumpLiveness`/`pump_stall_deadline` RETAINED (the `Gone` no-leak observable + the (B) verdict + the reserved deferred-watchdog hook) — docstring-only reframe. `MtlsSupervisor` (04-01) + tests DELETED (delete, not refactor); `derive_liveness` RETAINED. 05-01: `Arc<dyn MtlsEnforcement>` mandatory-param injection at the `compose_production_driver` root + `on_alloc_running` spawn + per-alloc teardown bookkeeping (NOT a central loop). Two NAMED deferrals (Tier-3 progress watchdog → #232; Phase-5 policy force-close — forward design rationale cross-referencing #37/#82, not tracked v1 work). ADR-0069 locked core UNCHANGED. | **ADR-0070**; this section; `feature-delta.md` (the `MtlsEnforcement` `liveness`/F6/`PumpLiveness` docstrings); `crates/overdrive-worker/` (`MtlsSupervisor` deletion — DELIVER) |

## Production mTLS dataplane integration (D-MTLS-17, 2026-06-14 — the missing production layer the single 05-01 concealed)

A re-plan finds that the OUTBOUND transparent-mTLS intercept has **no
production dataplane integration** — it exists only as test-harness glue. The
single step `05-01` ("activation") silently concealed a whole missing layer:
between the shipped adapter (`HostMtlsEnforcement`, `mtls/*.rs`) and the
shipped kernel-side program (`cgroup_connect4_mtls`, `MTLS_REDIRECT_DEST`),
there is **no production loader that loads/attaches the mTLS program, no
production map-programming surface, and no composition-root construction of the
enforcement port** (which cannot even be built where D-MTLS-16 assumed, because
`IdentityMgr` is constructed AFTER the driver-composition point). This decision
pins the production integration as a coherent unit. **Nothing in the locked
contract moves**: the 4-method `MtlsEnforcement` trait
(`probe`/`enforce`/`liveness`/`teardown`), ADR-0069's locked core, ADR-0070's
(C)+(B) supervision, and the D-MTLS-14/15 worker free-function signatures are
ALL UNCHANGED. D-MTLS-17 specifies the NEW *dataplane-integration API* (on
`overdrive-dataplane`) the feature genuinely needs and that no prior pin
specified — it is the missing production layer, not a contract change. Grounded
in the shipped source: `EbpfDataplane::new_with_pin_dir`
(`crates/overdrive-dataplane/src/lib.rs:386`, attaches at `:529–765`);
`cgroup_connect4_mtls` (`crates/overdrive-bpf/src/programs/cgroup_connect4_mtls.rs`);
`MTLS_REDIRECT_DEST` (`crates/overdrive-bpf/src/maps/mtls_redirect_dest.rs`);
the test-only load/attach/program glue (`mtls_roles.rs:567–600, 1242–1255`); the
composition root (`crates/overdrive-control-plane/src/lib.rs:1147` driver compose
/ `:1467` `ebpf_dataplane.probe()` / `:1673` `IdentityMgr::new`).

### The verified gap (what is test-only vs production today)

| Concern | Shipped state | Production gap |
|---|---|---|
| `cgroup_connect4_mtls` kernel program + `MTLS_REDIRECT_DEST` map | EXIST kernel-side, compiled into the shared `overdrive_bpf.o` | The production loader `EbpfDataplane::new_with_pin_dir` loads/attaches ONLY `cgroup_connect4_service` / `cgroup_sendmsg4_service` / `cgroup_recvmsg4_service` (`lib.rs:691–765`) — **never `cgroup_connect4_mtls`** |
| Per-workload-cgroup attach of `cgroup_connect4_mtls` (the F5-exempt attach point) | Test-only: `mtls_roles.rs:579–591` attaches to a per-test `.scope` | No production per-alloc attach; the service program attaches to the GLOBAL `workloads.slice` ancestor (`lib.rs:709`), which is the WRONG scope for the mTLS F5 exemption |
| `MTLS_REDIRECT_DEST[peer] = leg_f` programming | Test-only: `mtls_roles.rs:1242 program_redirect_dest`; no `src/` handle exists (Grep of `src/` = empty) | No production typed map-programming surface on `overdrive-dataplane` |
| `HostMtlsEnforcement` construction + wire→probe→use | Adapter built (`mtls/mod.rs:119`), `enforce`/`liveness`/`teardown` shipped; consumed ONLY by Tier-3 adapter tests | Never constructed in `run_server`; never `probe()`d at boot; never injected into the worker |
| Inbound nft-TPROXY install + `IP_TRANSPARENT` leg-C + `getsockname` orig-dst | Test-only: `mtls_netns_topology.rs::install_tproxy`, `mtls_roles.rs::{make_transparent_listener, accept_leg_c_and_orig_dst}` | D-MTLS-14 already homed these to `overdrive-worker/src/mtls_intercept.rs` (composition-root free fns), un-productionised |

### (1) Outbound production intercept-install — the `MtlsDataplane` surface on `overdrive-dataplane`

**Decision: a NEW production surface — `MtlsDataplane` — owns (a) loading +
per-alloc attaching `cgroup_connect4_mtls`, and (b) a typed
`MTLS_REDIRECT_DEST` programming handle. It is a SEPARATE worker-owned
userspace handle from `EbpfDataplane`, NOT new methods on the `Dataplane`
port trait, AND it owns its OWN `aya::Ebpf` (its own ELF load).** The
separation has TWO distinct, independently-sound justifications, which the
user's "why separate?" challenge surfaced and which this revision pins
explicitly:

- **Separate HANDLE (off the `Dataplane` trait, worker-owned, per-alloc
  lifecycle):** the `Dataplane` trait is the LB/service surface
  (`update_service` etc.) consumed by the control-plane as `Arc<dyn
  Dataplane>`; the mTLS intercept-install has a DIFFERENT consumer (the
  worker), a DIFFERENT lifecycle (per-alloc attach/detach vs global), and
  needs `program_mut("cgroup_connect4_mtls")` + the `MTLS_REDIRECT_DEST`
  handle, neither of which belongs on `dyn Dataplane` (OQ-2). This is sound
  and UNCHANGED.
- **Separate LOAD (its own `aya::Ebpf`):** forced by aya 0.13.x's
  single-owner `aya::Ebpf` model — per-alloc attach needs ongoing `&mut
  Ebpf`, which is unreachable on `EbpfDataplane`'s `Ebpf` once it is
  `Arc<dyn Dataplane>`-wrapped, and sharing one `Ebpf` would force
  `Arc<Mutex<Ebpf>>` taxing the hot LB path. See "Load model" below for the
  full reasoning and the bounded duplicate-load cost. This was previously
  CONTRADICTED by the item's own prose; this revision resolves it in favour
  of the separate load (the landed 06-01 shape) and documents WHY.

The `cgroup_connect4_mtls` program is **load-once, attach-per-alloc**: the
BPF object is loaded once at boot into `MtlsDataplane`'s own `aya::Ebpf` (the
program FD lives for the process), and a fresh `CgroupSockAddrLink` is taken
per allocation against that alloc's own `.scope` cgroup.

**Load model — `MtlsDataplane` performs its own `EbpfLoader` load of
`overdrive_bpf.o`; this is JUSTIFIED BY aya 0.13.x's single-owner
`aya::Ebpf` ownership model, NOT an arbitrary second loader.** This
resolves a prior internal contradiction in this item (the original prose said
"fold into the existing single ELF load, recover from the SAME `aya::Ebpf`
`EbpfDataplane` already holds" while the pinned API was `MtlsDataplane::load(pin_dir)`
— its own second load). The user challenged the separate load directly. On
investigation the **separate load is the correct shape** and the single-load
prose was unsatisfiable. The deciding reasoning, against aya 0.13.x:

1. **Per-alloc attach needs ongoing `&mut aya::Ebpf`.**
   `CgroupSockAddr::attach(&mut self, …)` and `take_link(&mut self, …)`
   (aya 0.13.1 `programs/cgroup_sock_addr.rs:71,111`) both require `&mut`
   on the program, reachable only via `Ebpf::program_mut(name) -> &mut`.
   `attach_alloc` `take_link`s a fresh `CgroupSockAddrLink` per allocation
   (the worker owns it, drops it on teardown), so the mTLS consumer needs
   `&mut Ebpf` **repeatedly over the node's lifetime** — once per alloc —
   not once at boot.
2. **`EbpfDataplane`'s `aya::Ebpf` is unreachable for `&mut` after
   composition.** `EbpfDataplane` is moved into `Arc<dyn Dataplane>` at the
   composition root (`control-plane/lib.rs:1481`). Past that line `&mut`
   access to its inner `aya::Ebpf` (struct field `bpf`, `lib.rs:768`) is
   STRUCTURALLY GONE: `Arc` yields `&` only, and `dyn Dataplane` — the
   LB/service trait, which by this feature's ratified scope MUST NOT grow
   mTLS methods — erases the concrete type. `EbpfDataplane` itself attaches
   its three service cgroup programs **once, globally** to the
   `workloads.slice` ancestor at boot (`lib.rs:709–765`) and never re-touches
   `&mut Ebpf` again; it has no reason to expose one.
3. **Sharing ONE `aya::Ebpf` across both consumers would force
   `Arc<Mutex<aya::Ebpf>>` and tax the LB dataplane.** The only way one
   loaded `Ebpf` serves both the global-attach LB surface AND the
   per-alloc-attach mTLS surface under aya 0.13.x is to hold the `Ebpf`
   behind `Arc<Mutex<…>>` so the per-alloc mutator can re-acquire `&mut`
   through the lock. That puts a `Mutex` around the LB/service dataplane's
   entire `aya::Ebpf` — a lock it does not need, for a consumer it does not
   own, with a lifecycle (per-alloc) it does not share — purely to satisfy
   mTLS. aya 0.13.x offers no "extract an owned program handle that retains
   its FD independently of the `Ebpf`" escape; programs are borrowed via
   `program_mut`, never moved out. So shared-single-load is either
   impossible (no owned-program extraction) or a coupling tax on the hot
   LB path. The two consumers' attach lifecycles are genuinely divergent —
   global-once vs per-alloc-repeated — and the clean expression of that
   divergence is two independent `aya::Ebpf` owners.

**Therefore `MtlsDataplane` owns its OWN `aya::Ebpf`, loaded by its own
`EbpfLoader::new().map_pin_path(pin_dir).allow_unsupported_maps().load_file(…)`**
(landed in `mtls/dataplane.rs::load`). It recovers `cgroup_connect4_mtls` +
the native-`HASH` `MTLS_REDIRECT_DEST` from THAT object; `program.load()` (the
verifier pass) runs once at boot; `.attach(&alloc_scope_file,
CgroupAttachMode::Single)` runs per-alloc against that owner's `&mut Ebpf`.

**Duplicate-load cost — bounded and mitigated.** The cost of the second load
is duplicate **program** instances and a re-walk of the ELF, NOT duplicate
map state on the shared maps:

- **The shared SERVICE_MAP HoM is NOT duplicated.** It is pinned by-name
  (`pinning = ByName`); the `MtlsDataplane` load passes the same `pin_dir`
  and `allow_unsupported_maps()`, so aya reuses the already-pinned outer FD
  via `BPF_OBJ_GET` rather than creating a second one — exactly the
  pin-by-name reuse the `EbpfDataplane` load relies on. `load()` does a
  `BPF_OBJ_GET`-or-first-pin match and **NEVER unlinks** the pin. In
  production `EbpfDataplane` (constructed in `run_server` before `IdentityMgr`,
  `lib.rs:450`) is the FIRST pinner of the live SERVICE_MAP, so by the time
  `MtlsDataplane::load` runs (post-`IdentityMgr`, item 3) the `BPF_OBJ_GET`
  reuse branch is taken: it reuses the existing pin and **never creates or
  unlinks** it. The defensive first-pin (`HashOfMapsHandle` pin +
  `std::mem::forget` so the bpffs pin persists) fires **only** when no prior
  owner has pinned SERVICE_MAP — i.e. the standalone Tier-3 test path
  (`BPF_OBJ_GET` → `ENOENT` → become the first pinner). Either way the
  `MtlsDataplane` surface never touches SERVICE_MAP through this map; and
  because `load()` never unlinks, it cannot clobber `EbpfDataplane`'s live pin
  (an earlier `remove_file` the design never sanctioned would have orphaned
  the kernel LB program bound to that pin by name).
- **The duplicated cost is bounded to the mTLS load's own programs/maps.**
  `MtlsDataplane` recovers and verifier-loads ONLY `cgroup_connect4_mtls` +
  takes ONLY `MTLS_REDIRECT_DEST`; it does NOT attach the service XDP/cgroup
  programs the second `aya::Ebpf` also parsed. The service programs in the
  mTLS-owned `Ebpf` are loaded-but-never-attached dead weight (verifier
  instruction budget is per-program and already gated by Tier-4; the
  unused programs cost a one-time verifier pass at boot, no steady-state
  cost). `MTLS_REDIRECT_DEST` is a per-node table the LB path never reads,
  so there is no shared-map double-instantiation hazard.
- **Boot-once, not per-alloc.** The second load happens exactly once, at the
  post-`IdentityMgr` composition point (item 3), behind the same
  wire→probe→use fail-closed gate as `EbpfDataplane`. It is not on any hot
  path.

This makes the user's "why a separate load?" a **documented answer** —
aya's single-owner model — rather than an accident, and pins the cost as
one extra boot-time verifier pass over the mTLS program plus an ELF re-walk,
with no duplicated shared-map state.

**Why a separate handle, not `EbpfDataplane` methods.** `EbpfDataplane` is
constructed in `run_server` at `:1404` and wrapped as `Arc<dyn Dataplane>` at
`:1481` — a trait object that exposes ONLY the LB/service surface. The mTLS
intercept-install needs `program_mut("cgroup_connect4_mtls")` + the
`MTLS_REDIRECT_DEST` map handle, neither of which belongs on `dyn Dataplane`.
The worker needs to call attach-per-alloc + program-the-map on
`on_alloc_running`, so the handle must be reachable from the worker as its own
injected type. Pinned surface:

```rust
//! crates/overdrive-dataplane/src/mtls/dataplane.rs — the production mTLS
//! intercept-install surface. Loads the shared overdrive_bpf.o ONCE into its
//! OWN aya::Ebpf value, owns the cgroup_connect4_mtls program handle + the
//! MTLS_REDIRECT_DEST typed map, and exposes per-alloc attach + per-destination
//! redirect programming. SEPARATE from EbpfDataplane (the LB/service Dataplane
//! port); NOT a Dataplane method.
//!
//! Why its OWN aya::Ebpf (not EbpfDataplane's): per-alloc attach needs ongoing
//! `&mut aya::Ebpf` (CgroupSockAddr::attach/take_link are `&mut self`), but
//! EbpfDataplane is moved into `Arc<dyn Dataplane>` at composition, after which
//! `&mut` on its inner Ebpf is structurally unreachable. Sharing one Ebpf would
//! force `Arc<Mutex<Ebpf>>` and tax the hot LB path for a per-alloc consumer it
//! does not own. The separate load is justified by aya 0.13.x's single-owner
//! model — see D-MTLS-17 item 1 "Load model". The duplicate load reuses the
//! pinned-by-name SERVICE_MAP (no second outer FD) and only verifier-loads the
//! mTLS program; no shared-map state is duplicated.

use std::net::SocketAddrV4;
use std::os::fd::OwnedFd;
use std::path::Path;

use overdrive_core::AllocationId;

/// Cause-distinct failure modes for the production mTLS intercept-install.
/// Typed (`thiserror`), no `Internal(String)`; `Display` names the kernel /
/// privilege remediation (`.claude/rules/development.md` § Errors).
#[derive(Debug, thiserror::Error)]
pub enum MtlsDataplaneError {
    /// The shared BPF object failed to load, or `cgroup_connect4_mtls` /
    /// `MTLS_REDIRECT_DEST` was absent from it (a build/embed regression).
    #[error("mTLS BPF load failed: {reason}")]
    Load { reason: String },
    /// `cgroup_connect4_mtls.load()` (the verifier pass) was rejected.
    #[error("cgroup_connect4_mtls verifier load failed: {reason}")]
    ProgramLoad { reason: String },
    /// Per-alloc attach to the allocation's `.scope` cgroup failed (the scope
    /// dir is missing, or `CAP_BPF`/`CAP_NET_ADMIN` is absent).
    #[error("cgroup_connect4_mtls attach to {scope} failed: {source}")]
    Attach { scope: std::path::PathBuf, #[source] source: std::io::Error },
    /// `MTLS_REDIRECT_DEST` update/delete syscall failed.
    #[error("MTLS_REDIRECT_DEST {op} failed: {source}")]
    MapProgram { op: &'static str, #[source] source: std::io::Error },
}

pub type Result<T, E = MtlsDataplaneError> = std::result::Result<T, E>;

/// The production mTLS intercept-install surface. Constructed ONCE at boot
/// (load-once); `attach_alloc` is called per-allocation (attach-per-alloc).
/// Owns its OWN `aya::Ebpf` (see module docs + D-MTLS-17 item 1 "Load model"
/// for the aya-ownership justification). Because `attach_alloc` is `&mut self`
/// (per-alloc `CgroupSockAddr::attach`/`take_link` need `&mut Ebpf`), the
/// worker holds the `MtlsDataplane` mutably (the `MtlsInterceptWorker` owns it
/// behind whatever interior-mutability the worker seam in item 3 establishes —
/// the per-alloc attach is serialised, which is correct: alloc lifecycle
/// events are not a hot path). `program_redirect`/`unprogram_redirect` are
/// `&self` (the `MTLS_REDIRECT_DEST` handle sits behind a `Mutex`).
pub struct MtlsDataplane { /* OWN aya::Ebpf (program FD owner) + MTLS_REDIRECT_DEST handle */ }

impl MtlsDataplane {
    /// Load the shared `overdrive_bpf.o` into THIS surface's OWN `aya::Ebpf`,
    /// recover the `cgroup_connect4_mtls` program handle and the
    /// `MTLS_REDIRECT_DEST` typed map, and run the program's verifier load ONCE.
    /// Mirrors `EbpfDataplane::new_with_pin_dir`'s recover-from-the-loaded-ELF
    /// shape (`lib.rs:529–765`) — but into a DISTINCT `aya::Ebpf` value, NOT
    /// `EbpfDataplane`'s (which is unreachable for `&mut` post-`Arc`-wrap; see
    /// "Load model"). Reuses the pinned-by-name SERVICE_MAP via the same
    /// `pin_dir` (no second outer FD); takes only `MTLS_REDIRECT_DEST` and
    /// verifier-loads only `cgroup_connect4_mtls`. No attach yet — attach is
    /// per-alloc.
    pub fn load(pin_dir: &Path) -> Result<Self>;

    /// Attach `cgroup_connect4_mtls` to ONE allocation's own `.scope` cgroup
    /// (the F5-exempt per-workload subtree — NOT the global `workloads.slice`
    /// ancestor the service program uses). Returns the owned link; the worker
    /// holds it per-alloc and drops it on teardown to detach. This IS the F5
    /// exemption made structural: the program sees only THIS workload's
    /// `connect()`s, never the agent's own leg-B dial (which runs on the host,
    /// outside any workload scope).
    pub fn attach_alloc(&mut self, alloc_scope: &Path) -> Result<MtlsCgroupLink>;

    /// Program `MTLS_REDIRECT_DEST[real_peer] = leg_f_listener` (host-order
    /// keys; the kernel program converts to NBO on rewrite). Called by the
    /// worker BEFORE the workload connects, so the workload's `connect(real_peer)`
    /// is transparently rewritten to the agent's leg-F listener. Idempotent
    /// overwrite (re-programming the same peer replaces the leg-F target).
    pub fn program_redirect(&self, real_peer: SocketAddrV4, leg_f: SocketAddrV4) -> Result<()>;

    /// Remove the `MTLS_REDIRECT_DEST[real_peer]` entry (on alloc teardown).
    /// Absent key → Ok (idempotent remove).
    pub fn unprogram_redirect(&self, real_peer: SocketAddrV4) -> Result<()>;
}

/// RAII owner of one allocation's `cgroup_connect4_mtls` attach link. `Drop`
/// detaches the program from that alloc's `.scope`. Held by the worker per-alloc.
pub struct MtlsCgroupLink { /* private: the aya CgroupSockAddrLink */ }
```

The `MtlsDestKey`/`MtlsAddrPort` userspace PODs (8-byte host-order mirrors of
the kernel-side structs in `mtls_redirect_dest.rs`) productionise the test-only
mirrors in `mtls_roles.rs:1218–1238`, moved into `mtls/dataplane.rs`. The map
handle is a plain `aya::maps::HashMap<_, MtlsDestKey, MtlsAddrPort>` (the map is
a `BPF_MAP_TYPE_HASH`, NOT the service HoM — aya supports it natively via
`bpf.take_map("MTLS_REDIRECT_DEST")`, simpler than the HoM `pinning = ByName`
dance). **Per-alloc attach lifecycle owner:** the worker holds the
`MtlsCgroupLink` per `AllocationId` (alongside the per-alloc teardown
bookkeeping D-MTLS-16 already pins) and drops it on `on_alloc_terminal`.

### (2) Inbound production intercept-install — already homed to the worker; needs NO `EbpfDataplane`/`MtlsDataplane` loader change

**Decision: the inbound path needs no BPF loader change at all. It is
nft-TPROXY (shell/`nft`, no BPF program) + an `IP_TRANSPARENT` leg-C listener +
the shipped `inbound.rs` adapter (`establish`/`dial_leg_s`). Its production home
is `overdrive-worker/src/mtls_intercept.rs` (`install_inbound_tproxy` +
`make_transparent_listener`), already pinned by D-MTLS-14.** The
`cgroup_connect4_mtls` BPF program is OUTBOUND-only; inbound interception is
purely kernel-routing (TPROXY prerouting + `ip rule fwmark` + `ip route local …
table`) installed via `nft`/`ip`, plus the agent's `IP_TRANSPARENT` accept
socket. `getsockname` recovers the orig-dst. The leg-S dial exemption
(`MTLS_LEG_S_DIAL_MARK`) is already production in `mtls/mod.rs::dial_leg_s`.

State plainly:
- **Already production:** lossless pre-arm capture (`mtls/mod.rs::drain_prearm`);
  the inbound `establish` flow (`mtls/inbound.rs`); leg-S `SO_MARK` F5 bypass
  (`dial_leg_s` / `MTLS_LEG_S_DIAL_MARK`); the 0.5-RTT early-data drain.
- **Un-productionised (D-MTLS-14 worker free fns — land in the worker step):**
  `make_transparent_listener`, `install_inbound_tproxy` (+`TproxyInterceptGuard`),
  `accept_inbound_leg` (`getsockname` orig-dst → `InterceptedConnection`).
- **NO `EbpfDataplane`/`MtlsDataplane` loader change for inbound** — inbound
  rides `nft` + the existing `inbound.rs` adapter + `dial_leg_s`. The only BPF
  loader change is the OUTBOUND `MtlsDataplane` (item 1).

### (3) Composition sequencing — resequence `IdentityMgr` BEFORE the enforcement-port construction, NOT before the driver

**Decision: the D-MTLS-16 assumption ("construct `HostMtlsEnforcement` at the
`compose_production_driver` root, ~1147") is unsatisfiable as shipped — `IdentityMgr`
is built at `lib.rs:1673`, AFTER `compose_production_driver` (1147) AND after
`ebpf_dataplane.probe()` (1467). The fix is NOT to drag the whole CA/identity
boot earlier (it depends on `boot_ca` / `bootstrap_node_intermediate` /
`store`, which have their own ordering). The fix is to construct
`HostMtlsEnforcement` + `MtlsDataplane` + wire them into the worker at a NEW
composition point AFTER `IdentityMgr` is built (after `:1673`), and inject the
enforcement port into the worker via the driver/worker seam — NOT at the 1147
driver-compose point.**

Two viable resequencings; **(3a) is chosen** (least movement, mirrors the
shipped `ebpf_dataplane.probe()` precedent):

- **(3a) — CHOSEN: construct + probe the mTLS port AFTER `IdentityMgr`
  (post-`:1673`), inject into the worker via a setter the action-shim/worker
  seam already supports OR via `AppState`.** The `ExecDriver` is composed at
  1147 WITHOUT the enforcement port; the enforcement port + `MtlsDataplane` are
  constructed at the new point (after `:1673`, where `identity: Arc<IdentityMgr>`
  exists), `probe()`d (wire→probe→use, fail-closed `health.startup.refused`
  mirroring `:1467`), and threaded to the worker component that owns
  `on_alloc_running`. Because `IdentityMgr` (`Arc<IdentityMgr>`) implements
  `IdentityRead`, `HostMtlsEnforcement::new(Arc::clone(&identity) as Arc<dyn
  IdentityRead>, MtlsLimits::default())` constructs cleanly at this point.
  - **The worker-injection seam.** D-MTLS-16 named the seam as "the
    `ExecDriver`-owning worker component's `Arc<dyn MtlsEnforcement>`, a required
    `new()` param at `compose_production_driver`." Because `IdentityMgr` is built
    LATER than `compose_production_driver`, the enforcement port CANNOT be a
    `compose_production_driver` param without also moving `IdentityMgr` earlier.
    **Resolution: split the worker's mTLS-enforcement role out of `ExecDriver`
    construction into a SECOND worker component (`MtlsInterceptWorker`)
    constructed at the post-`:1673` point with `Arc<dyn MtlsEnforcement>` +
    `MtlsDataplane` as required params, and have the `ExecDriver` lifecycle hooks
    (`on_alloc_running`/`on_alloc_terminal`) delegate to it.** The cleanest shape
    that honors the "mandatory port param, no builder" rule without forcing the
    CA/identity boot to move: `ExecDriver::with_mtls_intercept(Arc<MtlsInterceptWorker>)`
    is NOT acceptable (builder anti-pattern for a port-bearing component); instead
    the `MtlsInterceptWorker` is its OWN lifecycle observer the action-shim fires
    alongside the driver, OR `ExecDriver::new` grows the `Arc<MtlsInterceptWorker>`
    as a required param and `compose_production_driver` is resequenced to run
    AFTER `IdentityMgr`. **This sub-decision (worker-injection mechanism) is the
    one genuine design question the crafter must NOT improvise — see the BLOCKER
    below.**

- **(3b) — REJECTED: move the entire CA/identity boot (`:1616–1673`) above
  `compose_production_driver` (1147).** Rejected: `boot_ca` /
  `bootstrap_node_intermediate` depend on `store`, `config.kek`, `store_path`,
  `node_id` — several of which are derived between 1147 and 1616. Moving the
  whole block is a large, risky reorder of the boot sequence with cross-cutting
  ordering constraints (the cgroup-subtree ordering rule, the dataplane-provision
  ordering) for no benefit over (3a). The narrow fix (construct the mTLS port
  where its dependency already exists) is strictly simpler.

**Resequencing, named exactly:**
1. Keep `compose_production_driver` at 1147 unchanged for the probe-runner
   threading. The `ExecDriver` it returns does NOT yet hold the mTLS port.
2. After `IdentityMgr::new` (`:1673`), at a new composition block, construct:
   `let mtls_dataplane = MtlsDataplane::load(pin_dir)?;` then
   `let mtls_enforcement: Arc<dyn MtlsEnforcement> = Arc::new(HostMtlsEnforcement::new(Arc::clone(&identity) as Arc<dyn IdentityRead>, MtlsLimits::default()));`
   then `mtls_enforcement.probe().await` with the `health.startup.refused`
   fail-closed branch (mirroring `:1467`).
3. Construct the worker mTLS-intercept component with `mtls_enforcement` +
   `mtls_dataplane` as REQUIRED params and wire it into the `ExecDriver`
   lifecycle (the exact mechanism is the BLOCKER below).
4. Test composition injects `Arc::new(SimMtlsEnforcement::new(identity,
   MtlsLimits::default()))` + a sim/no-op `MtlsDataplane` equivalent.

### (4) Per-alloc lifecycle + supervision — compose the install + enforce + (C)+(B) at `on_alloc_running`/`on_alloc_terminal`

Reusing D-MTLS-15 (inputs) + ADR-0070/D-MTLS-16 (C+B supervision, no central
loop). The composed per-alloc flow, on `Driver::on_alloc_running(&AllocationSpec)`
(the sync seam, `driver.rs:783`) — spawning the async work per the established
sync-seam → async-spawn precedent:

1. **needs-intercept** = `DriverType::Exec` (D-MTLS-15; unconditionally true on
   the worker's exec path — no new spec field).
2. **Outbound install:** `mtls_dataplane.attach_alloc(&alloc_scope)` (the
   alloc's own `overdrive.slice/workloads.slice/<alloc>.scope` — the F5-exempt
   subtree); create the leg-F listener (`make_transparent_listener(127.0.0.1:0)`
   — leg F needs no `IP_TRANSPARENT`, a plain bound `TcpListener` suffices);
   `mtls_dataplane.program_redirect(declared_peer, leg_f_addr)` for the
   workload's DECLARED peer(s) (D-MTLS-15 residual: the peer set is the
   declared-mesh-peer from the deploy spec; arbitrary-peer auto-intercept is
   #178/#61-deferred). Hold the `MtlsCgroupLink` per-alloc.
3. **Inbound install:** `make_transparent_listener(127.0.0.1:0)` (leg C,
   `IP_TRANSPARENT`); read `agent_port` back; `install_inbound_tproxy(virt,
   agent_port)` (hold the `TproxyInterceptGuard` per-alloc).
4. **accept → enforce:** on each accepted leg, `accept_outbound_leg` /
   `accept_inbound_leg` builds the `InterceptedConnection`; `enforce(conn)` sets
   the (C) `TCP_USER_TIMEOUT`/keepalive on the legs (an SD-2 adapter
   postcondition, before the pumps start) and spawns the SD-2 port-owned pumps;
   the pump task self-tears-down (B) on EOF/error/`ETIMEDOUT`. The returned
   `EnforcedConnection` is recorded in the per-alloc teardown set.
5. **`on_alloc_terminal(&AllocationId)`** (`driver.rs:796`): drain the alloc's
   teardown set (`teardown` each `EnforcedConnection`); drop the
   `MtlsCgroupLink` (detach the cgroup program); drop the `TproxyInterceptGuard`
   (remove the nft rule/route/table); `unprogram_redirect` the alloc's peers.
   This is lifecycle bookkeeping keyed by alloc start/terminal — **NOT** a
   central liveness loop (D-MTLS-16).

No re-decision of supervision: (C) kernel timeouts + (B) per-connection
self-teardown, exactly as ADR-0070 pins. No `MtlsSupervisor`, no
`supervise_tick`.

### BLOCKER surfaced (worker-injection mechanism — the one design question to pin before the crafter starts step 06-03)

D-MTLS-16's "`Arc<dyn MtlsEnforcement>` is a required `new()` param at
`compose_production_driver`" is **not literally satisfiable** because
`IdentityMgr` (the only `IdentityRead`) is built AFTER `compose_production_driver`.
The enforcement port must be constructed post-`:1673`. The exact worker-injection
mechanism — **(α)** resequence `compose_production_driver` to run after
`IdentityMgr` and add the mTLS port as a required `ExecDriver::new` param; vs
**(β)** a separate `MtlsInterceptWorker` lifecycle component the action-shim
fires alongside the driver (so `ExecDriver` is unchanged) — is a genuine design
choice with a contract-adjacent consequence (whether `ExecDriver::new`'s
signature grows a mandatory port param). The decomposition below routes this to
the composition-root step (06-03) and the orchestrator must pin (α) vs (β) in
the dispatch (the crafter must NOT improvise a builder/`Option` to dodge it —
that is the exact "invent API surface" failure CLAUDE.md forbids). **My
recommendation: (β)** — a `MtlsInterceptWorker` constructed post-`:1673` with
both ports as required params, registered as a lifecycle observer, leaving
`ExecDriver::new` untouched and avoiding the CA-boot reorder. This keeps the
mTLS concern out of `ExecDriver` (separation) and satisfies "mandatory port
param, no builder" cleanly.

| Decision | Status | Where reconciled |
|---|---|---|
| **D-MTLS-17** (NEW) | Production mTLS dataplane integration (the missing layer 05-01 concealed): (1) a NEW `MtlsDataplane` surface on `overdrive-dataplane` (load-once the shared ELF, recover `cgroup_connect4_mtls` + `MTLS_REDIRECT_DEST`; `attach_alloc` per-alloc `.scope` = F5-exempt; `program_redirect`/`unprogram_redirect` typed map handle; `MtlsCgroupLink` RAII) — SEPARATE handle, NOT a `Dataplane` trait method; (2) inbound needs NO loader change (nft-TPROXY + `IP_TRANSPARENT` + shipped `inbound.rs` + `dial_leg_s`; D-MTLS-14 worker fns); (3) composition resequencing — construct + probe `HostMtlsEnforcement` + `MtlsDataplane` AFTER `IdentityMgr::new` (`:1673`), NOT at `compose_production_driver` (1147), and inject into a worker mTLS-intercept component; (4) per-alloc compose at `on_alloc_running`/`on_alloc_terminal` (install → enforce → (C)+(B)). 4-method contract + ADR-0069/0070 core + D-MTLS-14/15/16 UNCHANGED. **BLOCKER**: worker-injection mechanism (α vs β) to pin in dispatch; recommend (β). | this section; `deliver/decomposition-proposal-05.md` (the step breakdown replacing single 05-01); `feature-delta.md` (the `MtlsDataplane` surface note — DELIVER); `crates/overdrive-dataplane/src/mtls/dataplane.rs` (NEW — DELIVER); `crates/overdrive-control-plane/src/lib.rs` (resequence — DELIVER) |
