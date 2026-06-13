# Wave Decisions — transparent-mtls-host-socket (GH #26 · roadmap step 2.4)

**Wave**: DISCUSS (wave 2 of 6) · **Agent**: Luna (nw-product-owner) · **Date**: 2026-06-11
· **Density**: `lean` + `ask-intelligent` (DISCUSS hard default)

This feature is the **transparent-mTLS ENFORCEMENT mechanism** — the consumer
that finally encrypts the wire using the SVID that the CA mints (J-SEC-001 / #28)
and the IdentityMgr holds + exposes via `IdentityRead` (J-SEC-002 / #35).

---

## RE-GROUNDING ADDENDUM (2026-06-12) — the DISCUSS mechanism below is SUPERSEDED by ADR-0069

> **Read this first.** The DISCUSS-wave decisions recorded below (D2's
> spike-first in-band walking skeleton; the "in-band sidecarless kTLS,
> auth-session == data-session, agent EXITS the data path, kTLS on the
> workload's OWN socket" hypothesis; the Cilium fallback) describe the
> mechanism **as it stood at DISCUSS (2026-06-11), with the mechanism
> deliberately un-pinned**. The DESIGN wave **settled it**: **ADR-0069
> (2026-06-12)** locked a **universal agent-light L4 proxy** as the v1
> mechanism for #26 and **superseded** the in-band kTLS-on-the-workload's-own-
> socket model (out of v1 scope — a post-v1 optimization tracked in #231; ADR-0069 A1). The
> spike-first risk was **resolved by 6 committed Tier-3 spikes** (verdict:
> proxy, not in-band) — so D2's "FAIL → Cilium fallback" branch did NOT fire;
> the proxy is the answer.
>
> What this changes for the artifacts (all re-grounded 2026-06-12 by Luna per
> `design/upstream-changes.md` + `design/review-adversarial-2026-06-12.md` F1):
> kTLS on the **agent's peer-facing leg** (not the workload's socket); agent is
> **agent-LIGHT not OUT** (the kernel kTLS engine does all crypto; agent runs no
> cipher — but the pumps are NOT symmetric: decrypt/RX = zero-copy `splice` out of
> kTLS-RX, encrypt/TX = `read → write_all` copy into kTLS-TX, because a splice into
> kTLS-TX loses records; the forward was originally agent-idle via a sockmap egress
> redirect, retired 2026-06-13 / D-MTLS-13 — see `design/wave-decisions.md`
> § "Forward-mechanism pivot"); **BIDIRECTIONAL** v1 (outbound `cgroup_connect4` + inbound TPROXY
> server-mTLS); **NO restart-survival** (2 sockets/connection); **process/exec
> ONLY** (WASM dropped as a distinct path — auto-covered by the same proxy when
> a WASM driver lands); **#222 folded in** as the STAGED guest-stack intercept
> adapter of the ONE proxy (not a separate mechanism); **honest claim** =
> chain-to-bundle authn + encryption, NO intended-peer pinning (#178 upgrade);
> **authorization out of scope** (#27/#38). The re-grounded artifacts are
> `docs/product/jobs.yaml` § J-SEC-003, both journeys, and the re-scoped slices
> 00–05 (slice-00 = the composed proxy walking skeleton; the in-band
> spike/restart-survival/WASM slices are superseded/deleted).
>
> The CORE job is UNCHANGED (consume the held SVID to encrypt the wire
> in-kernel, auth-session == data-session, workload holds nothing, provable on
> the wire, `IdentityRead` reader). The DISCUSS decisions below are retained as
> the **historical record of how the mechanism question was framed and de-risked**
> — NOT as the current mechanism. Where they describe the in-band model, ADR-0069
> governs.

---

## Locked decisions (stated, not re-litigated)

- **[D1] Feature type = infrastructure / security-primitive.** No operator-facing
  verb this phase. There is no `overdrive` subcommand to "encrypt this workload";
  encryption is automatic and undisableable (vision principle 2). The HONEST
  observables are TEST-tier (see § Honest observables). Per CLAUDE.md the workload
  verb is `overdrive deploy <SPEC>`, never `job submit`.
- **[D2] Walking skeleton = the BLOCKING composed proxy WS (re-grounded to
  ADR-0069).** The DISCUSS-era spike-first framing was settled by the 6 committed
  Tier-3 spikes (verdict: the agent-light L4 proxy). The walking skeleton is now the
  COMPOSED proxy WS: the thinnest end-to-end cut that proves the full path holds on a
  real transparent intercept — `cgroup_connect4` (outbound) / nft-TPROXY (inbound)
  intercept → agent drains pre-arm plaintext losslessly → rustls handshake on the
  agent's peer-facing leg presenting the held SVID (via IdentityRead) → kTLS arm →
  post-arm bidirectional multi-record transfer with NO RST, under normal AND delayed
  timing, on the 6.18 Lima/LVH kernel. **Named falsification**: the composition does
  not hold (post-arm RST on the intercept lifecycle, the increment-e failure mode) →
  every later slice is blocked until the RST is engineered around. This is Slice 00,
  the BLOCKING first DELIVER slice.
- **[D3] Research depth = lightweight.** Matches the sibling security-primitive
  journeys (built-in-ca, workload-identity-manager). Three dataplane research docs +
  ADR-0068 + the 6 committed spike findings cover the mechanism and the kernel pin; no
  new research wave.
- **[D4] JTBD = yes; new job J-SEC-003.** This is the on-the-wire ENFORCEMENT peer
  of mint(J-SEC-001)/hold(J-SEC-002) — a distinct job (different progress: "the
  wire is actually encrypted with the workload's own SVID, in-kernel, agent out of
  the path"; different failure mode: cleartext on the wire / a plaintext race
  window / a handshake that does not fail closed). `relates_to: J-SEC-002`
  (J-SEC-003 is a READER of the `IdentityRead` port J-SEC-002 ships). Authored in
  `docs/product/jobs.yaml` § J-SEC-003 with full dimensions + four forces.
- **[Density] `lean` + `ask-intelligent`.** Tier-1 `[REF]` sections only; a scoped
  expansion menu is emitted at wave end ONLY if a trigger fires (see §
  Density & Triggers).

---

## Post-ADR-0068 grounding (the stale #26 issue body is corrected on three axes)

DISCUSS is grounded in the corrected reality, NOT the stale issue body:

1. **Kernel floor "5.10" → pinned 6.18 LTS (ADR-0068).** 6.18 guarantees in-kernel
   TLS 1.3 TX+RX and `CONFIG_NET_HANDSHAKE`. Kernel-version anxiety is REMOVED — it
   is a controlled constant, not an axis to design against.
2. **"TLS 1.3 KeyUpdate must be handled" → kernel-side IS present at v6.18**; the
   SOLE blocker is the userspace `rustls/ktls` bridge (rustls/ktls#59 / #62),
   tracked in **#229**. In-place rekey is OUT of v1 scope (teardown + reconnect);
   this is a TRACKED DEPENDENCY, not an open design risk for #26.
3. **"fd acquisition differs per workload kind" → resolved by the universal proxy
   (ADR-0069).** The agent owns its own legs (it does not acquire the workload's
   socket), so fd-acquisition no longer varies by workload kind. v1 ships process/exec
   (the only driver that exists); a future WASM driver's host-socket workloads are
   auto-covered by the same proxy, and guest-stack (microVM/unikernel) routes through
   the SAME mechanism via the STAGED guest-stack intercept adapter (**#222**, repurposed
   by ADR-0069 — not a separate mechanism).

---

## The genuine, still-open risk (the composed walking-skeleton gate)

The mechanism risk DISCUSS deliberately left un-pinned was settled by the 6 committed
Tier-3 spikes (verdict: the agent-light L4 proxy, ADR-0069) — sidecarless in-kernel
mTLS where the auth-session IS the data-session is proven on a real kernel, and the
agent-light L4 proxy is lossless for every protocol kind (the userspace handshake
buffer captures pre-arm plaintext and flushes it after the handshake; no kernel patch).

The ONE load-bearing risk the walking skeleton (Slice 00) still validates is the
**composition under a real transparent intercept**: the spikes proved every primitive
in isolation, but increment-e's composed harness RST'd on the intercept lifecycle and
increments-f/h removed the intercept to prove their primitive. Slice 00 — the BLOCKING
first DELIVER slice — proves the full composed path (real intercept → handshake on the
agent's leg → kTLS arm → post-arm bidirectional multi-record transfer, NO RST, both
directions, under normal AND delayed timing). FAIL → the composition does not hold and
every later slice is blocked until the RST is engineered around. There is no fallback
mechanism to adopt; the proxy is the answer.

---

## Scope Assessment: PASS — 6 stories (Slice 00 = the BLOCKING composed WS), 1 bounded context (workload-identity / dataplane mTLS), estimated ~7–9 days

Run BEFORE journey-visualization investment (Elephant Carpaccio gate, Phase 1.5).
Oversized-signal check (oversized = any 2+ firing):

| Signal | Threshold | This feature | Fires? |
|---|---|---|---|
| User stories | >10 | 6 (US-MTLS-00 composed WS + US-MTLS-01..05) | No |
| Bounded contexts / modules | >3 | 1 context (host-socket dataplane mTLS); touches the intercept + agent rustls + kTLS arm + splice pumps but one enforcement concern | No |
| WS integration points | >5 | the WS is the composed proxy path, one flow each way: transparent intercept → lossless capture → handshake on the agent's leg (reads IdentityRead) → kTLS arm → bidirectional transfer = ~5, and deliberately thin | No (at the boundary; the composed WS IS the thinning lever) |
| Estimated effort | >2 weeks | ~7–9 days (6 × ≤1–1.5-day slices) — under 2 weeks | No |
| Independent shippable outcomes | multiple | 1 coherent outcome (host-socket workloads' wire is encrypted with their own SVID, in-kernel, both directions) | No |

**Zero signals fire.** The feature is one coherent capability (host-socket
workloads carry TLS 1.3 on the peer-facing wire with their own SVID, in-kernel,
both directions, agent-light), already correctly carved from the staged guest-stack
adapter (#222) and the rekey/rotation/revocation concerns (#229/#40/Phase 5). It is
**not** split further; it IS sliced thinly (carpaccio) — 6 ≤1–1.5-day slices, each
end-to-end against the wire-capture observable, each with a named learning
hypothesis. The BLOCKING composed walking skeleton (Slice 00) keeps the riskiest
remaining assumption (the composition) cheapest to learn.

> Note: `story-map` does not exist yet at this phase (it is authored in § Story
> Map of the feature-delta). The scope verdict is recorded here per the Phase-1.5
> gate.

---

## Risk: NO DIVERGE wave was run for this feature

Unlike the sibling #35 (which had a DIVERGE wave that minted J-SEC-002 and locked
Option 1), **#26 has no DIVERGE artifacts** (`docs/feature/transparent-mtls-host-socket/diverge/`
does not exist). The job-grounding therefore rests on:

- The dataplane research docs + the 6 committed Tier-3 spike findings — all
  High-confidence, primary-kernel-sourced.
- ADR-0068 (the pinned-kernel decision that settles the kernel floor).
- The J-SEC-001 / J-SEC-002 jobs + journeys this feature consumes.

**Consequence / mitigation**: DISCUSS deliberately did NOT pin the mechanism — it
pinned the WHAT (the wire carries TLS 1.3 with the workload's own SVID, in-kernel,
fail-closed, both directions) and the acceptance OBSERVABLES, and left the mechanism
for the DESIGN wave to settle empirically. The DESIGN wave settled it: **ADR-0069
locked the universal agent-light L4 proxy** on the strength of the 6 committed Tier-3
spikes (no DIVERGE-recommended option was needed; the spikes ARE the empirical
narrowing). The one residual risk — the COMPOSITION under a real transparent intercept
— is gated by the BLOCKING composed walking skeleton (Slice 00), not assumed.

---

## Honest observables (foundation feature — TEST-tier security evidence)

There is NO operator CLI verb for "encrypt this workload" this phase. The HONEST
observable is TEST-tier security evidence, exactly as the J-SEC-002 journey did it
(re-grounded to the agent-light L4 proxy, ADR-0069):

- `tcpdump` / wire-capture on the **peer-facing leg** shows **TLS 1.3 Application Data
  records** (content type 0x17), not cleartext of the payload, both directions.
- `ss -tie` shows the **kTLS ULP installed** on the **agent's peer-facing leg**
  (`tcp-ulp-tls 1.3 aes-gcm-256`), NOT the workload's socket.
- `strace` shows the agent **agent-light** (the kernel kTLS engine does all crypto;
  the agent runs no cipher), but the cost is per-direction, NOT symmetric: the
  decrypt/RX directions (outbound return, inbound deliver) show only `splice`/`ppoll`
  (~1 splice per record, zero per-byte plaintext copy — zero-copy); the encrypt/TX
  directions (outbound forward, inbound response) show a per-record `read`+`write`
  into kTLS-TX (a userspace plaintext copy — the kernel `tls_sw_sendmsg` encrypts
  each write). NOT a userspace-crypto proxy. (D-MTLS-13: the forward is now an
  agent-light `read → write_all` copy into kTLS-TX, not the retired agent-idle
  sockmap egress redirect and NOT a splice into kTLS-TX — which loses records — so
  it shows per-record `read`+`write`, not "zero forward syscalls" and not
  `splice`-only.)
- A **negative test** shows a handshake **fails closed** cause-distinct on an absent SVID
  (outbound) or a missing/untrusted client cert (inbound `nocert`/`wrongca`) — no TLS
  Application Data and no cleartext.

Each slice's "After/sees" is framed against these concrete observables (the
security-reviewer-facing wire capture IS the value), so slices are not empty
`@infrastructure` shells. The slice-composition hard gate is respected: every
slice has a genuine observable. The composed walking skeleton's observable is the
lossless, RST-free, TLS-1.3-on-the-peer-wire capture for one composed flow each way.

---

## SSOT updates produced by this wave

- **`docs/product/jobs.yaml`**: appended **J-SEC-003** (`served_by_phase: 2`,
  `status: active`, `relates_to: J-SEC-002`, full dimensions + four forces) +
  changelog entry.
- **`docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml`**: product-level
  journey mapping J-SEC-003, Sam persona, lightweight depth, happy path (both
  directions) + the load-bearing error/resource paths (handshake-fail-closed on
  absent/wrong/untrusted creds; resource limits + pump-stall supervision;
  agent-restart → new-connection re-handshake, NO in-flight survival in v1;
  KeyUpdate-unsupported → teardown+reconnect deferred to #229). RE-GROUNDED 2026-06-12
  to ADR-0069 (the agent-light L4 proxy). Header states it does NOT extend the J-SEC-002
  journey and names the #222/#178/#27/#38/#229/#40 carve-outs.
- **`docs/product/personas/sam-platform-security-engineer.yaml`**: added
  `J-SEC-003` to `related_jobs` + a `j_sec_003_lens` (on-the-wire enforcement
  questions / success-signals / frustrations).
- **`docs/product/outcomes/registry.yaml`**: added 4 outcomes —
  `OUT-MTLS-SPIKE-INBAND-KTLS` (the spike), `OUT-MTLS-WIRE-TLS13`,
  `OUT-MTLS-NO-PLAINTEXT-PRE-KTLS`, `OUT-MTLS-HANDSHAKE-FAIL-CLOSED`.
- **Outcome KPIs**: in `feature-delta.md` § Outcome KPIs (the DISCUSS SSOT for KPI
  definitions/baselines/targets). `docs/product/kpi-contracts.yaml` is the
  docs-platform feature's single-feature contract and is NOT extended here (per
  its own scope note — other features record KPI baselines in their evolution
  records, not there).

---

## Density & Triggers

Tier-1 `[REF]` sections emitted (lean default). `ask-intelligent` triggers that
fired this wave are reported to the orchestrator (NOT auto-expanded):

- **Trigger: NO DIVERGE wave** → the mechanism was left for DESIGN to settle.
  Reported as the § Risk above; the 6 committed Tier-3 spikes were the empirical
  narrowing, and ADR-0069 locked the agent-light L4 proxy. The residual risk (the
  composition) is gated by the BLOCKING composed walking skeleton (Slice 00).
- **Trigger: a novel/unshipped core hypothesis** (sidecarless in-kernel mTLS where the
  auth-session IS the data-session) → settled empirically by the 6 committed Tier-3
  spikes on a real kernel; no fallback was needed.
- **Trigger: lossless capture for all protocol kinds** → resolved by the agent's
  userspace handshake buffer, which is lossless for every protocol kind (client- or
  server-first); no kernel patch. The question is closed.

No Tier-2 expansions were auto-rendered.

---

## DoR validation

See `feature-delta.md` § Wave: DISCUSS / [REF] Definition of Ready — the 9-item
checklist with per-item evidence.
