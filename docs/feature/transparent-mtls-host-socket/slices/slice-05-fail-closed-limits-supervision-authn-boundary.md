# Slice 05 — the guardrails: fail-closed (distinct reasons) · resource limits (F4/F7) · pump supervision (F6) · intercept-exemption negatives (F5) · the honest authn-vs-authz boundary (F1)

> **Re-grounded to ADR-0069 (the agent-light L4 proxy).** This **replaces** the
> old "restart-survival + WASM variant" slice — **both are GONE in v1**:
> restart-survival was the superseded in-band model's property (the agent owns
> both legs + the kTLS state; v1 = new-connection re-handshake), and there is no
> distinct WASM path (only `ExecDriver` exists; WASM is auto-covered by the same
> proxy when a WASM driver lands). This slice is the security teeth: the
> guardrails that make the enforcement un-bypassable AND the honest v1 claim.

**Job**: J-SEC-003 | **Feature**: transparent-mtls-host-socket (GH #26) | **Stories**: US-MTLS-05

## Goal (one sentence)

The encryption guarantee cannot be bypassed and its boundary is honest: a
handshake against an absent SVID (outbound) or a missing/untrusted client cert
(inbound) **FAILS CLOSED** with a cause-distinct reason and no plaintext; the
bounded pre-arm buffer / handshake deadline / per-allocation in-flight ceiling are
enforced fail-closed at their **concrete** values; a stalled return/deliver pump is
torn down (no leak); the agent's leg-B dial is provably not re-intercepted and a
workload provably **cannot self-exempt**; and v1 is documented as
**chain-to-bundle transport authn + encryption ONLY — NO intended-peer pinning**
(the wrong-but-valid-peer case is the #178 upgrade, NOT "protected" in v1).

## IN scope

- **Fail-closed (both directions, cause-distinct)**:
  - Outbound: `IdentityRead::svid_for` returns `None` → the agent refuses the
    handshake (no stale credential), leg B never armed, no cleartext to the peer
    → `MtlsEnforcementError::AbsentSvid`.
  - Outbound: the dialed peer's server cert does not chain to the bundle → rustls
    aborts → `PeerVerificationFailed`; no TLS app data, no cleartext.
  - Inbound: the client presents no cert (`nocert`) or an untrusted-CA cert
    (`wrongca`) → `WebPkiClientVerifier` rejects with a **distinct reason** per
    case (`peer sent no certificates` vs `invalid peer certificate: BadSignature`),
    BEFORE any splice → the server workload receives **0 bytes**.
- **Resource limits (F4/F7 — assert the CONCRETE values, not field existence)**:
  - `max_prearm_bytes = 256 KiB` exceeded → `BufferLimitExceeded` (buffer dropped,
    leg reset, no cleartext).
  - `handshake_deadline = 5 s` exceeded → `HandshakeTimeout`.
  - `max_inflight_per_alloc = 128` exceeded → `InFlightLimitExceeded` (refuse the
    new intercept; the workload's `connect()` fails, no cleartext).
  - Cleanup leaks nothing (no fd/kTLS state; no sockmap state exists post-D-MTLS-13);
    re-querying `liveness` returns `Gone`.
- **Pump supervision (F6)**: a return/deliver pump whose bytes-spliced counter has
  not advanced for `pump_stall_deadline = 30 s` WHILE a record is pending is
  `Stalled` → the node-agent/worker tears the connection down (teardown +
  fail-closed reset) → `Gone`, no leak; telemetry `mtls.pump.stalled` /
  `mtls.pump.teardown_on_stall`.
- **Intercept-exemption negatives (F5)**: (a) the agent's leg-B dial is NOT
  re-intercepted (no recursion) — proven; AND (b) a workload CANNOT self-exempt —
  the agent-private `SO_MARK`/cgroup bypass is unreachable from the workload's
  sockets (a workload setting it on its own socket is STILL intercepted).
- **The honest authn-vs-authz boundary (F1)**: documented + tested that v1
  authenticates **chain-to-trust-bundle ONLY** (BOTH directions), with **NO
  intended-peer pinning**. A reserved negative test for the wrong-but-valid-peer
  case (`PeerIdentityMismatch`) is **`#[ignore]`-gated on #178** (which supplies the
  expected-peer identity; VIP path #61). No AC/doc/test calls the wrong-but-valid-
  peer case "protected" / "pinned" until #178 lands.

## OUT scope

- **Authorization (allow/deny who-may-connect-to-whom)** — the BPF-LSM
  `socket_connect` hook (**#27**) fed by compiled `policy_verdicts` (**#38**;
  related **#49**), a SEPARATE subsystem this feature MUST NOT duplicate, embed, or
  read. The proxy does authn + encryption only.
- **Intended-peer SAN-match** (the wrong-but-valid-peer guard) — the **#178**
  upgrade (VIP path #61). Reserved as an `#[ignore]`-gated placeholder, NOT wired
  in v1.
- **Restart-survival + WASM + guest-stack** — restart-survival is GONE in v1; WASM
  has no distinct path (auto-covered when a WASM driver lands); guest-stack is the
  staged #222 adapter. None are v1 deliverables.
- **Operator-tunability of the limits** — v1 limits are compile-time defaults; a
  separate deferral (surfaced as a blocker, no issue created here).

## Learning hypothesis

- **Disproves if it fails**: "the encryption cannot be bypassed and the boundary is
  honest — absent/missing/untrusted creds fail closed cause-distinct, the resource
  limits + pump supervision hold at their concrete values with no leak, the
  intercept cannot be self-exempted, and the v1 claim is exactly chain-to-bundle
  authn." If a wrong/absent/missing cred falls back to plaintext, a limit is not
  enforced, a stalled pump leaks, a workload can self-exempt, or a doc/test
  overclaims intended-peer protection, the security guarantee is hollow.
- **Confirms if it succeeds**: the guardrails are enforced (not just configured)
  and the v1 claim is honest — the feature is safe to hand to a security reviewer.

## Acceptance criteria

- [ ] **Outbound fail-closed**: `IdentityRead::svid_for` `None` → `AbsentSvid` (agent refuses, no cleartext to the peer); a peer not chaining to the bundle → `PeerVerificationFailed` (no TLS app data, no cleartext). Anchor: ADR-0069 § Enforcement "Authn-only boundary"; contract `AbsentSvid` (consumes `identity_read.rs` clause 3).
- [ ] **Inbound fail-closed, distinct reasons**: `nocert` and `wrongca` each reject with their DISTINCT reason (`peer sent no certificates` vs `invalid peer certificate: BadSignature`), BEFORE any splice; the server workload receives 0 bytes. Anchor: `findings-inbound-intercept.md` §4.
- [ ] **Resource limits (concrete values)**: `max_prearm_bytes = 256 KiB` → `BufferLimitExceeded` (buffer dropped, leg reset, no cleartext); `handshake_deadline = 5 s` → `HandshakeTimeout`; `max_inflight_per_alloc = 128` → `InFlightLimitExceeded`; cleanup leaks no fd/sockmap/kTLS state (re-query `liveness` → `Gone`). Assert the CONCRETE values, not field existence. Anchor: feature-delta contract `MtlsLimits` (F7 defaults) + the three variants; ADR-0069 § "Resource & robustness constraints".
- [ ] **Pump supervision (F6)**: inject a stalled pump (pause the `splice` task while a record is pending); `liveness` transitions to `Stalled` within `pump_stall_deadline = 30 s`; the worker tears the connection down; no fd/kTLS leak (re-query → `Gone`; no sockmap state exists post-D-MTLS-13); `mtls.pump.stalled` / `mtls.pump.teardown_on_stall` emitted. Anchor: ADR-0069 § ATAM "Pump supervision policy (F6)".
- [ ] **Intercept-exemption negatives (F5)**: the agent's leg-B dial is NOT re-intercepted (no recursion); a workload that sets the bypass on its own socket is STILL intercepted (the bypass is agent-private, unreachable from the workload). Anchor: ADR-0069 § "intercept-recursion / agent-leg-B exemption" (the `cgroup_connect4_service` attach boundary).
- [ ] **Honest authn boundary (F1)**: a test asserts v1 verifies chain-to-bundle ONLY (both directions, fail-closed on non-chaining peers); the wrong-but-valid-peer `PeerIdentityMismatch` negative test is present but `#[ignore]`-gated on #178 with a `reason` naming #178; NO AC/doc/test calls the wrong-but-valid-peer case "protected" until #178 lands. Anchor: ADR-0069 § Decision "The honest v1 security claim".
- [ ] `cargo xtask lima run -- cargo nextest run -p <crate> --features integration-tests` green for the fail-closed + limits + supervision + exemption + boundary acceptance tests (real kernel, not `--no-run`).

## Dependencies

- Slice 03 (the outbound enforce path to guard) + Slice 04 (the inbound enforce path) + Slice 02 (the handshakes to fail closed) + Slice 01 (the intercept-exemption mechanism).
- The shipped `IdentityRead` port returning `None` explicitly for an unheld allocation (#35) — exists.
- The `MtlsLimits` defaults + `MtlsEnforcementError` variants + `PumpLiveness::Stalled` + the `PeerIdentityMismatch` reserved variant (ADR-0069 / feature-delta DESIGN contract) — pinned by DESIGN.
- #178 (the intended-peer SAN-match upgrade — the gate for the reserved negative test); #27/#38 (authorization, the SEPARATE subsystem). **Verify before citing**; create NO issues.

## Effort estimate

~1–1.5 days. Reference class: the fail-closed negatives are straightforward (the
inbound nocert/wrongca distinct-reason rejections are proven in
`findings-inbound-intercept.md` §4); the resource-limit + pump-supervision tests
need the deliberately-exceeded-buffer / stalled-handshake / paused-pump harnesses;
the exemption negatives + the `#[ignore]`-gated boundary placeholder are small.

## Pre-slice SPIKE

Not needed — the inbound fail-closed (nocert/wrongca, distinct reasons) is proven
in `findings-inbound-intercept.md` §4; the resource limits / pump supervision /
exemption are productionisation of the ADR-0069 contract, not open mechanism
questions.

## Taste-test note

The security-teeth slice: ships fail-closed (both directions, cause-distinct) +
the concrete resource limits + pump supervision + the intercept-exemption negatives
+ the honest authn-vs-authz boundary. Production-data observable (real
cause-distinct errors, no leak on cleanup, a self-exempt attempt still intercepted,
no overclaim of intended-peer protection — Tier-3, the invariants a security
reviewer pushes hardest on). Disproves a real assumption (the encryption cannot be
bypassed AND the platform claims exactly what it proves). One value story
(US-MTLS-05); the observable is the fail-closed / no-leak / no-evade / honest-claim
proof — the security teeth of the feature, not infra-only. The #178/#27/#38
boundaries keep the scope (authn + encryption only) and the claim (chain-to-bundle,
not intended-peer) honest.
