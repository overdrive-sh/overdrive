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
- **Proxy proven agent-light BOTH directions**: forward agent-IDLE sockmap-egress-
  redirect → kTLS-TX, 15/15 (`findings-egress-ktls-splice.md`); return agent-LIGHT
  zero-copy `splice` via `tls_sw_splice_read`, ~1/record (`findings-splice-return.md`).
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
D-MTLS-4 (forward agent-idle sockmap-egress, return agent-light `splice`); D-MTLS-5
(leg B = plain kTLS-RX, NO psock); D-MTLS-10 (in-process agent — no separate
process, no gRPC/CSR; resolves the prior open item); D-MTLS-11 (Earned-Trust
`probe()` mandatory); D-MTLS-12 (added 2026-06-12 during DELIVER back-propagation —
`probe`'s handshake sentinel uses a THROWAWAY self-signed cert minted in-process
via `rcgen`; substrate-self-test crypto, signed by neither CA, never in the trust
bundle, never on a real wire — #26 stays a READER, NOT an issuer; promotes `rcgen`
to an `overdrive-dataplane` production dep; SD-5, user-approved).

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
