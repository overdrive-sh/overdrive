# Upstream changes — transparent-mtls-host-socket DESIGN (GH #26 folds #222)

Back-propagation flagged for the **product-owner** by Morgan (DESIGN wave,
2026-06-12). The architect does **NOT** edit `jobs.yaml` or the DISCUSS slice files
— those are the product-owner's artifacts. This file records what must change
upstream and why, for the product-owner to action.

## Context

The DESIGN wave formalized the user's LOCKED decision (ADR-0069): ONE universal
**transparent mTLS via an agent-light L4 proxy** for ALL workload kinds, folding
#222 into #26. The previously-primary **in-band kTLS-on-the-workload's-own-socket**
model is SUPERSEDED as v1 (retained as a tracked future optimization). The DISCUSS
job J-SEC-003 and slices 00–05 were authored on the in-band model's properties —
several of which **no longer hold in v1**.

## What changed (the properties that no longer hold)

| DISCUSS premise (in-band model) | v1 reality (proxy model) |
|---|---|
| "the agent EXITS the data path" / "agent fully out" | **Agent is LIGHT, not OUT.** Forward steady state is agent-idle (kernel splice); **return** steady state is agent-LIGHT (`splice` pump, ~1/record) — the agent stays scheduled per-record on the return path for the connection's life. |
| "kTLS on the workload's OWN socket" / "workload-owns-fd" | **kTLS lives on the agent's leg B**, not the workload's socket. The workload holds a plaintext socket to the agent (leg F). |
| "restart-survivable" (kTLS state socket-owned + workload owns fd) | **No restart-survival in v1.** The agent owns both legs + the kTLS state; an agent restart drops in-flight sessions (re-handshake on reconnect). Restart-survival is the in-band model's unique win → tracked future optimization (DEFER-1). |
| "1 socket per connection" | **2 sockets per connection** (leg F + leg B). |
| "host-socket ONLY; guest-stack is #222, a SEPARATE feature" | **#222 folds into #26 as a STAGED ADAPTER, not a separate mechanism.** The proxy is universal — guest-stack (microVM/unikernel) routes through the SAME mechanism via a guest-stack intercept adapter (tap/TPROXY/TC source → same `InterceptedConnection`), STAGED to #222 (repurposed). Host-socket #26 v1 is BIDIRECTIONAL (outbound + inbound). |
| "host-socket = outbound/client only" (implicit in the in-band framing) | **Host-socket is BIDIRECTIONAL v1.** The inbound/server half (TPROXY intercept → `getsockname` orig-dst → server mTLS verifying the client → splice-to-server) is designed + spike-proven (`findings-inbound-intercept.md`), a first-class path alongside the outbound/client half. |
| "mTLS with workload identity" (implies intended-peer) | **v1 = chain-to-bundle transport authn + encryption, NO intended-peer pinning.** A routing bug / VIP collision / valid-but-unintended SVID is NOT prevented in v1; intended-peer SAN-match is the #178 UPGRADE. |
| race-window "lossy DROP-then-RESET under a named server-speaks-first assumption" | **Lossless for all kinds.** The handshake-window capture is a userspace buffer; no dropped pre-arm bytes, no RESET, NO server-speaks-first assumption. |

## Anchor the re-grounding on PROVEN spike observables (not hypotheticals)

The DISCUSS wave authored J-SEC-003 + slices 00–05 with the mechanism un-pinned
("show me a packet capture"), so several ACs are framed as *hypotheses to test*.
The mechanism is now **empirically settled** by 6 committed Tier-3 spikes
(`../spike/findings*.md`, `353cdc52`). The re-grounding must therefore **anchor
each AC on the concrete spike observable that PROVED it** — the AC becomes "assert
the proven observable holds for the productionised path," not "discover whether the
mechanism works." The committed findings are the foundation of record; the
gitignored `spike-scratch/` probe code is a non-durable convenience only (see the
feature-delta § "Proven-Mechanism Traceability" → Durability). The anchors:

| Re-grounded AC | Proven spike observable to cite | Committed finding |
|---|---|---|
| Forward steady state (agent-idle) | `tcpdump` shows `1703 03` (0x17) records on the peer-facing wire, agent issues ZERO per-byte syscalls (strace), `redir_err=0` — reproduced 15/15 | `findings-egress-ktls-splice.md` |
| Return steady state (agent-light) | `strace` shows ONLY `splice`/`ppoll`, zero payload read/write; byte-exact plaintext on leg F; ~1 `splice` per TLS record; `einval_on_B=0` | `findings-splice-return.md` |
| Arming invariant (Tier-3 AC) | `SOCKMAP`-insert AFTER `TCP_ULP "tls"` returns `EINVAL` (the natural detect→gate→install order passes; the reverse must fail) | `findings.md` Increment D |
| kTLS armed on leg B | `ss -tie` shows `tcp-ulp-tls 1.3 aes-gcm-256 rxconf:sw txconf:sw` (NOT `ss -K`, which is `--kill`) | `findings.md` A; `findings-egress-ktls-splice.md` mechanic #4 |
| No cleartext on the peer wire | `strings pcap \| grep <marker>` / cleartext count on leg B = 0; the workload's plaintext is on leg F (host-internal) BY DESIGN | `findings-userspace-relay.md` Unknown 2; `findings-egress-ktls-splice.md` Assertion 1 |
| Lossless handshake-window capture | pre-arm plaintext arrives at the peer exactly once, in order, as the first `application_data` (rec_seq 0); no dropped bytes, no RESET | `findings-userspace-relay.md` Unknown 1+2; `findings-lossless-hybrid.md` |
| Fail-closed (absent/wrong SVID) | `IdentityRead::svid_for == None` → handshake refused, leg closed, no bytes egress (the `MtlsEnforcementError::AbsentSvid` path) | contract § (consumes `identity_read.rs` clause 3) |
| **Composed walking skeleton (F2)** | real `cgroup_connect4` intercept → pre-arm write → handshake → kTLS arm → post-arm bidirectional multi-record transfer, NO RST, under normal AND traced/delayed timing — the composition the spikes did NOT prove (increment-e RST'd) | ADR-0069 § Enforcement "Composed walking-skeleton gate"; the FIRST DELIVER slice (BLOCKING) |
| **Inbound / server half (F3)** | TPROXY intercept → `getsockname` orig-dst recovery (`127.0.0.2:18443`); server-side mutual-TLS (present server SVID + `WebPkiClientVerifier` REQUIRE+VERIFY client SVID); kTLS-RX armed (`ss -tie` `rxconf:sw`); byte-exact plaintext spliced to the identity-unaware server workload; client leg carries `0x17` only, agent-light (`strace`: splice/ppoll only); fail-closed on `nocert`/`wrongca` (distinct reasons, 0 bytes to S) | `findings-inbound-intercept.md` §1–§5 (increment-i, kernel 7.0) |
| **Resource limits (F4/F7)** | bounded pre-arm buffer → `BufferLimitExceeded` (256 KiB); handshake deadline → `HandshakeTimeout` (5 s); per-alloc in-flight ceiling → `InFlightLimitExceeded` (128); cleanup leaks nothing — all fail-closed. **Assert the CONCRETE values, not field existence.** | feature-delta contract `MtlsLimits` (F7 defaults + `Default` impl) + the three variants; ADR-0069 § "Resource & robustness constraints" |
| **Pump supervision (F6)** | pump `Stalled` after `pump_stall_deadline` (30 s) with a record pending → worker tears down (teardown + fail-closed reset) → `Gone`, no leak; telemetry `mtls.pump.stalled` / `mtls.pump.teardown_on_stall` | feature-delta `liveness` § "F6 supervision policy" + `PumpLiveness::Stalled` + `MtlsLimits::pump_stall_deadline`; ADR-0069 § ATAM |
| **Intercept exemption (F5)** | agent leg B NOT re-intercepted (no recursion); workload CANNOT self-exempt (bypass agent-private); inbound leg-S dial not TPROXY-re-intercepted | `cgroup_connect4_service` attach boundary; ADR-0069 § "intercept-recursion exemption" |
| **Authn-only boundary (F1/F5)** | v1 authenticates **chain-to-bundle ONLY** (BOTH directions; inbound proven fail-closed on `nocert`/`wrongca`); **NO intended-peer pinning** — a valid-but-unintended SVID is NOT prevented in v1; authorization is #27/#38; wrong-but-valid-peer SAN-match (`PeerIdentityMismatch`) reserved, `#[ignore]` gated on #178; docs/tests MUST NOT call that case "protected" until #178 lands | ADR-0069 § Decision "The honest v1 security claim" + "What this does NOT do"; `findings-inbound-intercept.md` §4; #27/#38 (authz), #178/#61 (expected destination) |

State the observable + the finding in each re-grounded AC so the DISTILL test
scenarios and the DELIVER Tier-3 tests assert the SAME thing the spike already
proved — closing the loop from proven evidence → acceptance criterion → test.

## Action items for the product-owner

1. **Re-ground J-SEC-003** (`docs/product/jobs.yaml` § J-SEC-003) on the proxy
   mechanism. The `functional`/`emotional`/`social` dimensions and the `pull`/
   `anxiety` forces reference "agent exits the data path", "kTLS on the workload's
   socket", and "restart-survivable" — re-word to the proxy reality (agent-light
   return; kTLS on the agent's peer-facing leg; no v1 restart-survival; lossless;
   universal across kinds). The CORE job (transparent in-kernel mTLS with the
   workload's own SVID, auth-session == data-session, workload holds nothing,
   provable on the wire) is UNCHANGED and still holds. **Product-owner edits
   `jobs.yaml`.**

2. **Re-ground slices 00–05** (`docs/feature/transparent-mtls-host-socket/slices/`)
   on the proxy mechanism. Specifically:
   - **Slice 00 (spike)** — the 6 committed spikes settled the MECHANISM (verdict:
     "proxy", not "in-band"). But they proved the *primitives in isolation*, NOT
     the *composition*: increment-e's composed harness RST'd on the intercept
     lifecycle, and increments-f/h removed the transparent intercept to prove
     their primitive. **The new walking skeleton is a composed Tier-3 acceptance
     test (F2 — the FIRST DELIVER slice, BLOCKING)**: real `cgroup_connect4`
     intercept → workload pre-arm write → leg-B handshake → kTLS arm → **post-arm
     bidirectional multi-record transfer with NO RST**, under BOTH normal AND
     traced/delayed timing. This **supersedes the old in-band walking skeleton**
     (and the spike-as-walking-skeleton framing) — the spike de-risked the
     primitives; the composed slice proves they compose. Anchor: ADR-0069 §
     Consequences "Composition is unproven" + § Enforcement "Composed
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
     on **leg B**; "agent exits" → "agent-idle forward splice + agent-light return
     splice"; the wire capture observable is unchanged (TLS 1.3 on the peer-facing
     wire). Anchor each direction on its proven observable: forward AC ← `tcpdump`
     shows 0x17 records + agent idle (`findings-egress-ktls-splice.md`, 15/15);
     return AC ← `strace` shows only `splice`/`ppoll` (`findings-splice-return.md`);
     `ss -tie` shows the kTLS ULP (`findings.md` A). Add a Tier-3 AC for the arming
     invariant: `SOCKMAP`-after-`TCP_ULP "tls"` == `EINVAL` (`findings.md` D).
   - **Slice 04 (fail-closed + race-window + resource limits + authn boundary)** —
     fail-closed is unchanged (`IdentityRead` `None` → refuse handshake; the
     `AbsentSvid` path). The "no-cleartext-before-kTLS" observable is now satisfied
     by the userspace capture being lossless + confidentiality-correct (workload
     never reaches the peer un-proxied; cleartext count on leg B = 0,
     `findings-userspace-relay.md` Unknown 2), NOT by a lossy DROP gate. Drop the
     server-speaks-first assumption. **ADD the F4 resource-limit ACs** (the pre-arm
     buffer is bounded — `BufferLimitExceeded` fail-closed; the handshake has a
     deadline — `HandshakeTimeout`; the per-allocation in-flight ceiling refuses
     over-limit — `InFlightLimitExceeded`; cleanup leaks no fd/sockmap/kTLS state),
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
     v1** (DEFER-1). Re-scope to "new connections re-handshake after an agent
     restart" (which the proxy gives unconditionally) + "the WASM and guest-stack
     variants route through the same proxy". The in-flight-survival AC is removed
     (it was the in-band model's property).
   **Product-owner edits the slice files.**

3. **Approve (or reject) the deferral GH issues** — the architect created NONE
   (CLAUDE.md: deferrals need user approval BEFORE creation). Pending the
   product-owner's decision:
   - **DEFER-1** — in-band restart-survival + 1-socket-density future optimization.
   - **DEFER-2** — multi-node reachability — verify the existing **#36** covers it
     (`gh issue view 36 --comments`) before citing; create a new issue only if not.

   (The agent-light splice return is the design; a fully-agent-idle bidirectional
   return is a non-goal, not pursued — NO kernel patch is or will be required.)

4. **Update the scope-boundary table** in the feature-delta DISCUSS section ("In
   scope (#26) / Out of scope") — #222 is no longer "out of scope, a SEPARATE
   feature"; it is folded in as the **STAGED guest-stack intercept ADAPTER** of
   the ONE universal proxy (re-grounded by ADR-0069; #222 repurposed to "the
   guest-stack intercept adapter for the #26 universal proxy"). Host-socket #26 v1
   is **BIDIRECTIONAL** (outbound + inbound). Also **add the authn-vs-authz
   boundary AND the honest v1 claim** to the out-of-scope column: authorization
   (allow/deny) is the BPF-LSM `socket_connect` hook (**#27**) fed by compiled
   `policy_verdicts` (**#38**; related **#49**); **intended-peer identity pinning**
   is downstream via east-west SPIFFE-ID resolution (**#178**; VIP path **#61**) —
   v1 does **chain-to-bundle transport authn + encryption only, NO intended-peer
   pinning** (a valid-but-unintended SVID is NOT prevented in v1). (The architect
   left the DISCUSS sections intact; the product-owner owns the DISCUSS
   re-grounding. No GH issues created.)

5. **Land the review-revision ACs across the slices (F1/F4/F5 + the F2 gate).**
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
   **Product-owner threads these into the slice ACs; the architect named them but
   does not edit the slice files.**

6. **Land the RE-review ACs across the slices (F3/F6/F7 + the bidirectional +
   honest-claim re-grounding).** The adversarial RE-review (2026-06-12;
   `design/review-adversarial-2026-06-12.md`; resolutions in
   `design/wave-decisions.md` § "RE-review revisions"). The Luna pass must
   re-ground **BOTH directions** + the **honest v1 claim** + the **limit values**
   into the slices:
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
   **Product-owner / Luna threads these into the slice ACs; the architect named
   them but does not edit the slice files (only the journey's #222-separate
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
