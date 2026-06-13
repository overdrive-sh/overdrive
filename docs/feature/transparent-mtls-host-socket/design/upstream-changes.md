# Upstream changes — transparent-mtls-host-socket DESIGN (GH #26 folds #222)

Back-propagation **record** kept by Morgan (DESIGN wave, 2026-06-12). The architect
did **NOT** edit `jobs.yaml` or the DISCUSS slice files — those are the
product-owner's artifacts. This file is the **completed-back-propagation record**:
it documents what changed upstream and why, after the changes landed. It is **not a
live TODO list** — every item below has been actioned in the product/slice/journey
files.

> **Status (2026-06-12):** the back-propagation is **COMPLETE**. The product job
> J-SEC-003, the product + DISCUSS journeys, slices 00–05, the persona lens, the
> outcome registry, and the scope-boundary table have all been re-grounded on the
> ADR-0069 proxy model. The deferral re-grounding also landed — in-band
> restart-survival + 1-socket density is recorded as out of v1 scope, a post-v1
> optimization tracked in **#231** (ADR-0069 A1); multi-node transparent mTLS is
> recorded as out of v1 scope (no forward-pointer issue). The C4 outbound-L3 peer
> endpoint (F3) and the `InterceptedConnection` inbound-leg wording (F4) have been
> aligned to the proxy model. The sections below are the **rationale of record**
> (why each change was made), past-tense — not edits still pending.

## Context

The DESIGN wave formalized the user's LOCKED decision (ADR-0069): ONE universal
**transparent mTLS via an agent-light L4 proxy** for ALL workload kinds, folding
#222 into #26. The previously-primary **in-band kTLS-on-the-workload's-own-socket**
model is SUPERSEDED as v1 (out of v1 scope — a post-v1 optimization tracked in **#231**; ADR-0069 A1). The DISCUSS
job J-SEC-003 and slices 00–05 were authored on the in-band model's properties —
several of which **no longer hold in v1**.

## What changed (the properties that no longer hold)

| DISCUSS premise (in-band model) | v1 reality (proxy model) |
|---|---|
| "the agent EXITS the data path" / "agent fully out" | **Agent is LIGHT, not OUT.** Forward, return, and deliver steady state are all agent-LIGHT (`splice` pumps, ~1/record, zero userspace plaintext copy) — the agent stays scheduled per-record on every pump for the connection's life (D-MTLS-13). |
| "kTLS on the workload's OWN socket" / "workload-owns-fd" | **kTLS lives on the agent's leg B**, not the workload's socket. The workload holds a plaintext socket to the agent (leg F). |
| "restart-survivable" (kTLS state socket-owned + workload owns fd) | **No restart-survival in v1.** The agent owns both legs + the kTLS state; an agent restart drops in-flight sessions (re-handshake on reconnect). Restart-survival is the in-band model's unique win → out of v1 scope, a post-v1 optimization tracked in **#231** (ADR-0069 A1). |
| "1 socket per connection" | **2 sockets per connection** (leg F + leg B). |
| "host-socket ONLY; guest-stack is #222, a SEPARATE feature" | **#222 folds into #26 as a STAGED ADAPTER, not a separate mechanism.** The proxy is universal — guest-stack (microVM/unikernel) routes through the SAME mechanism via a guest-stack intercept adapter (tap/TPROXY/TC source → same `InterceptedConnection`), STAGED to #222 (repurposed). Host-socket #26 v1 is BIDIRECTIONAL (outbound + inbound). |
| "host-socket = outbound/client only" (implicit in the in-band framing) | **Host-socket is BIDIRECTIONAL v1.** The inbound/server half (TPROXY intercept → `getsockname` orig-dst → server mTLS verifying the client → splice-to-server) is designed + spike-proven (`findings-inbound-intercept.md`), a first-class path alongside the outbound/client half. |
| "mTLS with workload identity" (implies intended-peer) | **v1 = chain-to-bundle transport authn + encryption, NO intended-peer pinning.** A routing bug / VIP collision / valid-but-unintended SVID is NOT prevented in v1; intended-peer SAN-match is the #178 UPGRADE. |
| race-window "lossy DROP-then-RESET under a named server-speaks-first assumption" | **Lossless for all kinds.** The handshake-window capture is a userspace buffer; no dropped pre-arm bytes, no RESET, NO server-speaks-first assumption. |

## How the re-grounding was anchored — on PROVEN spike observables (not hypotheticals)

The DISCUSS wave authored J-SEC-003 + slices 00–05 with the mechanism un-pinned
("show me a packet capture"), so several ACs were framed as *hypotheses to test*.
The mechanism is now **empirically settled** by 6 committed Tier-3 spikes
(`../spike/findings*.md`, `353cdc52`). The re-grounding therefore **anchored
each AC on the concrete spike observable that PROVED it** — each AC now reads "assert
the proven observable holds for the productionised path," not "discover whether the
mechanism works." The committed findings are the foundation of record; the
gitignored `spike-scratch/` probe code is a non-durable convenience only (see the
feature-delta § "Proven-Mechanism Traceability" → Durability). The anchors used:

| Re-grounded AC | Proven spike observable to cite | Committed finding |
|---|---|---|
| Forward steady state (agent-light, D-MTLS-13) | `tcpdump` shows `1703 03` (0x17) records on the peer-facing wire; `strace` shows ONLY `splice`/`ppoll` on the forward pump (`splice(legF → legB)` into kTLS-TX, the kernel `tls_sw_sendmsg` encrypting on splice-in), ~1 splice/record, zero userspace plaintext copy. (The earlier sockmap EGRESS-redirect — `redir_err=0`, 15/15 — was retired for a `MSG_DONTWAIT`-stall, `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`.) | `findings-egress-ktls-splice.md`; `sockmap-egress-redirect-into-ktls-tx-delivery-research.md` |
| Return steady state (agent-light) | `strace` shows ONLY `splice`/`ppoll`, zero payload read/write; byte-exact plaintext on leg F; ~1 `splice` per TLS record; `einval_on_B=0` | `findings-splice-return.md` |
| ~~Arming invariant (Tier-3 AC)~~ MOOT (D-MTLS-13) | ~~`SOCKMAP`-insert AFTER `TCP_ULP "tls"` returns `EINVAL`~~ — no sockmap insert on any path now; retained as a historical kernel fact only | `findings.md` Increment D (historical) |
| kTLS armed on leg B | `ss -tie` shows `tcp-ulp-tls 1.3 aes-gcm-256 rxconf:sw txconf:sw` (NOT `ss -K`, which is `--kill`) | `findings.md` A; `findings-egress-ktls-splice.md` mechanic #4 |
| No cleartext on the peer wire | `strings pcap \| grep <marker>` / cleartext count on leg B = 0; the workload's plaintext is on leg F (host-internal) BY DESIGN | `findings-userspace-relay.md` Unknown 2; `findings-egress-ktls-splice.md` Assertion 1 |
| Lossless handshake-window capture | pre-arm plaintext arrives at the peer exactly once, in order, as the first `application_data` (rec_seq 0); no dropped bytes, no RESET | `findings-userspace-relay.md` Unknown 1+2; `findings-lossless-hybrid.md` |
| Fail-closed (absent/wrong SVID) | `IdentityRead::svid_for == None` → handshake refused, leg closed, no bytes egress (the `MtlsEnforcementError::AbsentSvid` path) | contract § (consumes `identity_read.rs` clause 3) |
| **Composed walking skeleton (F2)** | real `cgroup_connect4` intercept → pre-arm write → handshake → kTLS arm → post-arm bidirectional multi-record transfer, NO RST, under normal AND traced/delayed timing, in the real netns/veth topology — the three narrow composition gaps the spikes left (outbound composed in ONE flow; bidirectional round-trip; real netns/veth topology). Mechanism is spike-verified incl. the inbound flow composed end-to-end (increment-i §2); increment-e's steady-state RST was a throwaway-harness limitation, NOT a kernel finding, superseded by increment-f. Integration gate, not mechanism. | ADR-0069 § Enforcement "Composed walking-skeleton gate"; the FIRST DELIVER slice (BLOCKING) |
| **Inbound / server half (F3)** | TPROXY intercept → `getsockname` orig-dst recovery (`127.0.0.2:18443`); server-side mutual-TLS (present server SVID + `WebPkiClientVerifier` REQUIRE+VERIFY client SVID); kTLS-RX armed (`ss -tie` `rxconf:sw`); byte-exact plaintext spliced to the identity-unaware server workload; client leg carries `0x17` only, agent-light (`strace`: splice/ppoll only); fail-closed on `nocert`/`wrongca` (distinct reasons, 0 bytes to S) | `findings-inbound-intercept.md` §1–§5 (increment-i, kernel 7.0) |
| **Resource limits (F4/F7)** | bounded pre-arm buffer → `BufferLimitExceeded` (256 KiB); handshake deadline → `HandshakeTimeout` (5 s); per-alloc in-flight ceiling → `InFlightLimitExceeded` (128); cleanup leaks nothing — all fail-closed. **Assert the CONCRETE values, not field existence.** | feature-delta contract `MtlsLimits` (F7 defaults + `Default` impl) + the three variants; ADR-0069 § "Resource & robustness constraints" |
| **Pump supervision (F6)** | pump `Stalled` after `pump_stall_deadline` (30 s) with a record pending → worker tears down (teardown + fail-closed reset) → `Gone`, no leak; telemetry `mtls.pump.stalled` / `mtls.pump.teardown_on_stall` | feature-delta `liveness` § "F6 supervision policy" + `PumpLiveness::Stalled` + `MtlsLimits::pump_stall_deadline`; ADR-0069 § ATAM |
| **Intercept exemption (F5)** | agent leg B NOT re-intercepted (no recursion); workload CANNOT self-exempt (bypass agent-private); inbound leg-S dial not TPROXY-re-intercepted | `cgroup_connect4_service` attach boundary; ADR-0069 § "intercept-recursion exemption" |
| **Authn-only boundary (F1/F5)** | v1 authenticates **chain-to-bundle ONLY** (BOTH directions; inbound proven fail-closed on `nocert`/`wrongca`); **NO intended-peer pinning** — a valid-but-unintended SVID is NOT prevented in v1; authorization is #27/#38; wrong-but-valid-peer SAN-match (`PeerIdentityMismatch`) reserved, `#[ignore]` gated on #178; docs/tests MUST NOT call that case "protected" until #178 lands | ADR-0069 § Decision "The honest v1 security claim" + "What this does NOT do"; `findings-inbound-intercept.md` §4; #27/#38 (authz), #178/#61 (expected destination) |

Each re-grounded AC states the observable + the finding, so the DISTILL test
scenarios and the DELIVER Tier-3 tests assert the SAME thing the spike already
proved — closing the loop from proven evidence → acceptance criterion → test.

## Back-propagation record (what was re-grounded, and why)

1. **J-SEC-003 re-grounded** (`docs/product/jobs.yaml` § J-SEC-003) on the proxy
   mechanism. The `functional`/`emotional`/`social` dimensions and the `pull`/
   `anxiety` forces previously referenced "agent exits the data path", "kTLS on the
   workload's socket", and "restart-survivable" — these were re-worded to the proxy
   reality (agent-light return; kTLS on the agent's peer-facing leg; no v1
   restart-survival; lossless; universal across kinds). The CORE job (transparent
   in-kernel mTLS with the workload's own SVID, auth-session == data-session,
   workload holds nothing, provable on the wire) is UNCHANGED and still holds. **The
   product-owner has edited `jobs.yaml` accordingly.**

2. **Slices 00–05 re-grounded** (`docs/feature/transparent-mtls-host-socket/slices/`)
   on the proxy mechanism. The slice files now reflect the re-scopes below; the
   per-slice text records what changed and why:
   - **Slice 00 (composed walking skeleton)** — the Tier-3 spikes settled the
     MECHANISM (verdict: "proxy", not "in-band"). They proved the *primitives in
     isolation* AND the *INBOUND flow composed end-to-end* in one direction
     (`findings-inbound-intercept.md` increment-i §2: real TPROXY intercept →
     orig-dst recovery → server-mTLS verifying C's client SVID → kTLS-RX arm →
     agent-light splice-to-S byte-exact, fail-closed on `nocert`/`wrongca`). Three
     NARROW composition gaps remain: (1) the OUTBOUND path composed in ONE flow
     (its pieces proven on SEPARATE harnesses — increment-f removed the intercept
     to isolate the splice; increment-e's steady-state RST was a *throwaway-harness
     intercept-lifecycle limitation, NOT a kernel finding*, superseded by
     increment-f's clean-harness proof); (2) bidirectional steady-state round-trip;
     (3) the real netns/veth topology with cgroup-isolated workloads. **The new
     walking skeleton is a composed Tier-3 acceptance test (F2 — the FIRST DELIVER
     slice, BLOCKING)**: real `cgroup_connect4` intercept → workload pre-arm write →
     leg-B handshake → kTLS arm → **post-arm bidirectional multi-record transfer
     with NO RST**, under BOTH normal AND traced/delayed timing, in the real
     netns/veth topology. This **supersedes the old in-band walking skeleton** (and
     the spike-as-walking-skeleton framing) — it is an integration gate that closes
     gaps 1–3, NOT a "prove-the-mechanism" gate. Anchor: ADR-0069 § Consequences
     "Three narrow composition gaps remain" + § Enforcement "Composed
     walking-skeleton gate".
   - **Slice 01 (sockops detect + fd acquire + intercept exemption)** — re-scope to
     "intercept (`cgroup_connect4` rewrite) + sockops detect + agent accepts leg
     F". Anchor: the `cgroup/connect4`-rewrite-to-agent-listener + lossless
     `accept()`/`recv()` proven in `findings-userspace-relay.md` Unknown 1 (route
     the splice by `local_port` only — the `local_ip4` byte-order disagreement).
     **ADD the F5 intercept-exemption AC**: the agent's own leg-B dial is NOT
     re-intercepted (no infinite recursion) via the `SO_MARK`/cgroup-scoping bypass,
     AND the workload cannot self-exempt (the bypass is agent-private) — reference
     the existing `cgroup_connect4_service` attach boundary (program attaches to the
     *workload* subtree, not the agent's).
   - **Slice 02 (handshake present held SVID)** — re-scope the handshake to **leg B**
     (the agent's peer-facing leg), not the workload's own socket. The `IdentityRead`
     read is unchanged. Anchor: rustls 1.3 handshake + `dangerous_extract_secrets`
     proven driving a real handshake in `findings.md` A (a minimal rcgen P-256 drove
     every spike; real `IdentityRead`/SPIFFE-SAN SVID is the productionisation).
   - **Slice 03 (kTLS install + agent exits + wire capture)** — re-scope: kTLS arms
     on **leg B**; "agent exits" → "agent-light forward splice + agent-light return
     splice"; the wire capture observable is unchanged (TLS 1.3 on the peer-facing
     wire). Anchor each direction on its proven observable: forward AC ← `tcpdump`
     shows 0x17 records + `strace` shows only `splice`/`ppoll` on the forward pump
     (`splice(legF → legB)` into kTLS-TX; D-MTLS-13 replaced the sockmap
     EGRESS-redirect, `sockmap-egress-redirect-into-ktls-tx-delivery-research.md`);
     return AC ← `strace` shows only `splice`/`ppoll` (`findings-splice-return.md`);
     `ss -tie` shows the kTLS ULP (`findings.md` A). (The earlier arming-invariant
     Tier-3 AC — `SOCKMAP`-after-`TCP_ULP "tls"` == `EINVAL` — is MOOT under
     D-MTLS-13; no sockmap insert on any path now.)
   - **Slice 04 (fail-closed + race-window + resource limits + authn boundary)** —
     fail-closed is unchanged (`IdentityRead` `None` → refuse handshake; the
     `AbsentSvid` path). The "no-cleartext-before-kTLS" observable is now satisfied
     by the userspace capture being lossless + confidentiality-correct (workload
     never reaches the peer un-proxied; cleartext count on leg B = 0,
     `findings-userspace-relay.md` Unknown 2), NOT by a lossy DROP gate. Drop the
     server-speaks-first assumption. **ADD the F4 resource-limit ACs** (the pre-arm
     buffer is bounded — `BufferLimitExceeded` fail-closed; the handshake has a
     deadline — `HandshakeTimeout`; the per-allocation in-flight ceiling refuses
     over-limit — `InFlightLimitExceeded`; cleanup leaks no fd/pump/kTLS state),
     and the F5 intercept-exemption ACs (agent leg B NOT re-intercepted; workload
     CANNOT self-exempt — referencing the `cgroup_connect4_service` attach
     boundary). **ADD the F1 authn-vs-authz boundary**: v1 authenticates
     chain-to-trust-bundle ONLY; authorization (allow/deny) is the BPF-LSM
     `socket_connect` hook (#27) fed by compiled `policy_verdicts` (#38; related
     #49), a SEPARATE subsystem this feature MUST NOT duplicate. Reserve a negative
     test for the wrong-but-valid-peer case (`PeerIdentityMismatch`) **gated on
     #178** (native east-west SPIFFE-ID resolution, which supplies the expected
     peer; VIP path #61) — `#[ignore]` until #178 lands. The proxy never embeds a
     policy engine. (No GH issues created here.)
   - **Slice 05 (restart-survival + WASM variant)** — **restart-survival is GONE in
     v1**. Re-scope to "new connections re-handshake after an agent
     restart" (which the proxy gives unconditionally) + "the WASM and guest-stack
     variants route through the same proxy". The in-flight-survival AC is removed
     (it was the in-band model's property).
   **The product-owner has edited the slice files accordingly.**

3. **Deferrals — RESOLVED as out of v1 scope.** The architect created NO GH issues
   directly (CLAUDE.md: deferrals need user approval BEFORE creation):
   - **(a) In-band restart-survival + 1-socket density** is not pursued in v1
     (the superseded in-band model's unique win; ADR-0069 A1) — a post-v1
     optimization tracked in **#231**.
   - **(b) Multi-node / cross-node transparent mTLS** is out of v1 scope (Phase 1 is
     single-node), no forward-pointer issue. **#36 is generic node
     enrollment/admission and does NOT cover cross-node transparent mTLS, so it is
     NOT cited for it.**

   (The agent-light splice return is the design; a fully-agent-idle bidirectional
   return is a non-goal, not pursued — NO kernel patch is or will be required.)

4. **Scope-boundary table updated** in the feature-delta DISCUSS section ("In
   scope (#26) / Out of scope") — #222 is no longer "out of scope, a SEPARATE
   feature"; it is folded in as the **STAGED guest-stack intercept ADAPTER** of
   the ONE universal proxy (re-grounded by ADR-0069; #222 repurposed to "the
   guest-stack intercept adapter for the #26 universal proxy"). Host-socket #26 v1
   is **BIDIRECTIONAL** (outbound + inbound). The authn-vs-authz boundary AND the
   honest v1 claim were **added** to the out-of-scope column: authorization
   (allow/deny) is the BPF-LSM `socket_connect` hook (**#27**) fed by compiled
   `policy_verdicts` (**#38**; related **#49**); **intended-peer identity pinning**
   is downstream via east-west SPIFFE-ID resolution (**#178**; VIP path **#61**) —
   v1 does **chain-to-bundle transport authn + encryption only, NO intended-peer
   pinning** (a valid-but-unintended SVID is NOT prevented in v1). (The product-owner
   owns the DISCUSS re-grounding; it has landed. No GH issues created.)

5. **Review-revision ACs landed across the slices (F1/F4/F5 + the F2 gate).**
   The adversarial review (rejected pending revisions, 2026-06-12; recorded in
   `design/wave-decisions.md` § "Review revisions") added, additively on the
   unchanged contract:
   - **F2 composed walking-skeleton gate** → Slice 00's re-grounding (the FIRST,
     BLOCKING DELIVER slice — supersedes the old in-band walking skeleton).
   - **F4 resource-limit ACs** (`BufferLimitExceeded` / `HandshakeTimeout` /
     `InFlightLimitExceeded` + no-leak cleanup) → Slice 04.
   - **F5 intercept-exemption ACs** (leg B not re-intercepted; workload cannot
     self-exempt) → Slice 01 (mechanism) + Slice 04 (the negative/self-exempt
     test).
   - **F1 authn-vs-authz boundary** + the reserved `PeerIdentityMismatch`
     negative-test placeholder (gated on **#178**) → Slice 04; authorization
     stays with **#27/#38**, never this feature.
   **The product-owner has threaded these into the slice ACs; the architect named
   them but did not edit the slice files.**

6. **RE-review ACs landed across the slices (F3/F6/F7 + the bidirectional +
   honest-claim re-grounding).** The adversarial RE-review (2026-06-12;
   `design/review-adversarial-2026-06-12.md`; resolutions in
   `design/wave-decisions.md` § "RE-review revisions"). The Luna pass re-grounded
   **BOTH directions** + the **honest v1 claim** + the **limit values** into the
   slices:
   - **F3 inbound/server half** → a new INBOUND slice (or an inbound arm of the
     existing handshake/kTLS/wire slices): TPROXY intercept → `getsockname`
     orig-dst → server-SVID selection → `WebPkiClientVerifier` client-auth →
     kTLS-RX arm → splice-to-server; fail-closed on `nocert`/`wrongca`. Anchor
     every AC on `findings-inbound-intercept.md` §1–§5 (the proven observables).
     The walking skeleton (Slice 00) must exercise BOTH directions.
   - **F5 honest v1 claim** → re-word the security ACs EVERYWHERE to "chain-to-bundle
     transport authn + encryption; NO intended-peer pinning." The wrong-but-valid-peer
     test stays `#[ignore]`-gated on **#178**; no AC/doc calls that case "protected"
     until #178 lands.
   - **F6 pump-stall supervision** → an AC for `Stalled` → worker teardown →
     `Gone`, no leak (Slice 04 or the steady-state slice).
   - **F7 concrete limit values** → the F4 resource-limit ACs assert the CONCRETE
     values (256 KiB / 5 s / 128 / 30 s), not field existence (Slice 04).
   - **F4 guest-stack staging** → the scope-boundary update (item 4) reflects #222
     as the STAGED guest-stack ADAPTER, not a separate mechanism.
   **The product-owner / Luna have threaded these into the slice ACs; the architect
   named them but did not edit the slice files (only the journey's #222-separate
   contradiction, which directly contradicted the ADR, was corrected here).**

## What does NOT change

- The identity model: one CA, one SVID set, one trust bundle, the `IdentityRead`
  port. #26 remains a READER (never mints/caches).
- The auth-session == data-session property (rustls secrets → leg B kTLS).
- The workload-holds-nothing property.
- The wire-capture acceptance observable (TLS 1.3 records, zero cleartext on the
  peer-facing wire).
- The pinned 6.18 kernel (ADR-0068).
- **The scope was always authn + encryption, never authorization** — the review
  made this boundary EXPLICIT (it was previously silent), it did not remove scope.
  Authorization is #27/#38; expected-destination identity is #178. The core
  fail-closed authn behaviour (`AbsentSvid` / `PeerVerificationFailed`) is
  unchanged.

## Cross-references

- ADR-0069 (the decision; § References → "Evidence durability"); `brief.md`
  § "Transparent mTLS … extension"; the feature-delta § "Wave: DESIGN / [REF]
  Proven-Mechanism Traceability" (the design-element → committed-finding matrix
  the anchors above draw from); `design/c4-diagrams.md`; `design/wave-decisions.md`.
- The 6 committed spike findings (`../spike/findings*.md`, `353cdc52` — the
  foundation of record) + 3 research docs (`docs/research/dataplane/`). The
  gitignored `spike-scratch/increment-{a..h}/` probe code is a non-durable
  convenience only — cite the committed finding, never the probe dir.
