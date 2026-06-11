# Wave Decisions — transparent-mtls-host-socket (GH #26 · roadmap step 2.4)

**Wave**: DISCUSS (wave 2 of 6) · **Agent**: Luna (nw-product-owner) · **Date**: 2026-06-11
· **Density**: `lean` + `ask-intelligent` (DISCUSS hard default)

This feature is the **host-socket transparent-mTLS ENFORCEMENT mechanism** — the
consumer that finally encrypts the wire using the SVID that the CA mints
(J-SEC-001 / #28) and the IdentityMgr holds + exposes via `IdentityRead`
(J-SEC-002 / #35).

---

## Locked decisions (stated, not re-litigated)

- **[D1] Feature type = infrastructure / security-primitive.** No operator-facing
  verb this phase. There is no `overdrive` subcommand to "encrypt this workload";
  encryption is automatic and undisableable (vision principle 2). The HONEST
  observables are TEST-tier (see § Honest observables). Per CLAUDE.md the workload
  verb is `overdrive deploy <SPEC>`, never `job submit`.
- **[D2] Walking skeleton = SPIKE-FIRST.** The issue mandates a Tier-3 spike
  BEFORE the design locks. WS = the thinnest end-to-end slice that disproves the
  core hypothesis: prove `sockops ACTIVE_ESTABLISHED → pidfd_getfd → rustls
  handshake presenting the held SVID (via IdentityRead) → kTLS install → tcpdump
  shows TLS 1.3 records` for ONE process→process flow on the 6.18 Lima/LVH kernel,
  with the plaintext race window closed (fail-closed: no plaintext egress before
  kTLS install). **Named falsification**: "disproves in-band sidecarless kTLS on
  our kernel if the handoff cannot be made race-free for one process flow → fall
  back to Cilium out-of-band-auth + separate encryption." This is Slice 00.
- **[D3] Research depth = lightweight.** Matches the sibling security-primitive
  journeys (built-in-ca, workload-identity-manager). Three dataplane research docs
  + ADR-0068 already cover the mechanism, the race window, and the kernel pin; no
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
3. **"fd acquisition differs per workload kind" → resolved by the host-socket vs
   guest-stack taxonomy.** #26 handles host-socket only (process via `pidfd_getfd`;
   WASM in-process). microVM/unikernel are **#222's** problem (host L4 tap proxy).

---

## The genuine, still-open risk DISCUSS honors (the WS hypothesis)

Two load-bearing hypotheses the walking-skeleton spike (Slice 00) must validate:

- **In-band sidecarless kTLS is unshipped anywhere.** No production mesh does
  it: Cilium does out-of-band auth + WireGuard/IPsec (separate encryption); Istio
  ztunnel keeps a userspace proxy in the data path; Linkerd same. Overdrive's
  target — auth-session == data-session, agent then EXITS the data path — has zero
  shipping precedent (race-window research Finding 6; CP-restart research Gap 1).
- **The plaintext race window.** Between sockops `ACTIVE_ESTABLISHED` firing and
  kTLS install completing, a workload `write()` can leak cleartext: sk_msg has only
  PASS/DROP/REDIRECT, no lossless HOLD; `bpf_msg_cork_bytes` does not buffer
  (race-window research Findings 1–2).

**Documented fallback** (if in-band kTLS doesn't pan out): the Cilium model —
out-of-band auth + separate encryption (or a userspace proxy that stays in the
data path, à la Istio ztunnel / Architecture C). The 6.18 pin also legitimises an
out-of-tree write-block patch (lossless backpressure) as an appliance-OS option,
but that is a DESIGN-wave call, not pinned here.

---

## Scope Assessment: PASS — 5 stories + 1 spike, 1 bounded context (workload-identity / dataplane mTLS), estimated ~7–9 days

Run BEFORE journey-visualization investment (Elephant Carpaccio gate, Phase 1.5).
Oversized-signal check (oversized = any 2+ firing):

| Signal | Threshold | This feature | Fires? |
|---|---|---|---|
| User stories | >10 | 5 (US-MTLS-01..05) + 1 spike (US-MTLS-00) | No |
| Bounded contexts / modules | >3 | 1 context (host-socket dataplane mTLS); touches sockops + agent rustls + kTLS install but one enforcement concern | No |
| WS integration points | >5 | the WS is a SPIKE (one process flow): sockops detect → fd acquire → handshake (reads IdentityRead) → kTLS install → wire capture = ~5, and deliberately thin | No (at the boundary; the spike IS the thinning lever) |
| Estimated effort | >2 weeks | ~7–9 days (spike ~2d, then 5 × ≤1-day slices) — under 2 weeks | No |
| Independent shippable outcomes | multiple | 1 coherent outcome (host-socket workloads' wire is encrypted with their own SVID, in-kernel) | No |

**Zero signals fire.** The feature is one coherent capability (host-socket
workloads carry TLS 1.3 on the wire with their own SVID, in-kernel, agent out of
the path), already correctly carved from the guest-stack path (#222) and the
rekey/rotation/revocation concerns (#229/#40/Phase 5). It is **not** split
further; it IS sliced thinly (carpaccio) — 1 spike + 5 ≤1-day slices, each
end-to-end against the wire-capture observable, each with a named learning
hypothesis. The spike-first WS keeps the riskiest assumption cheapest to learn.

> Note: `story-map` does not exist yet at this phase (it is authored in § Story
> Map of the feature-delta). The scope verdict is recorded here per the Phase-1.5
> gate.

---

## Risk: NO DIVERGE wave was run for this feature

Unlike the sibling #35 (which had a DIVERGE wave that minted J-SEC-002 and locked
Option 1), **#26 has no DIVERGE artifacts** (`docs/feature/transparent-mtls-host-socket/diverge/`
does not exist). The job-grounding therefore rests on:

- The three dataplane research docs (mechanism, race window, recommended
  architecture) + the CP-restart survival research — all High-confidence, primary
  -kernel-sourced.
- ADR-0068 (the pinned-kernel decision that settles the kernel floor).
- The J-SEC-001 / J-SEC-002 jobs + journeys this feature consumes.

**Consequence / mitigation**: the option space (in-band kTLS [Arch A] vs proxy
[Arch C] vs out-of-band auth + separate encryption [Cilium]) is NOT pre-narrowed
by a DIVERGE recommendation. DISCUSS deliberately does NOT pin the mechanism — it
pins the WHAT (the wire carries TLS 1.3 with the workload's own SVID, in-kernel,
fail-closed) and the acceptance OBSERVABLES, and runs the **spike-first walking
skeleton (Slice 00)** to settle the riskiest mechanism question empirically before
DESIGN locks. The DESIGN wave (solution-architect) owns the mechanism choice,
informed by the spike outcome. This is the honest substitute for a DIVERGE
recommendation, recorded here as a risk so DESIGN does not assume a pre-validated
option.

---

## Honest observables (foundation feature — TEST-tier security evidence)

There is NO operator CLI verb for "encrypt this workload" this phase. The HONEST
observable is TEST-tier security evidence, exactly as the J-SEC-002 journey did it:

- `tcpdump` / wire-capture on the veth shows **TLS 1.3 Application Data records**
  (content type 0x17), not cleartext, between two host-socket workloads.
- `ss -K` shows the **kTLS ULP installed** on the workload's socket.
- A **negative test** shows a handshake **fails closed** on a wrong/absent SVID
  (no TLS Application Data and no cleartext on the wire).
- A **race-window probe** shows **no cleartext byte egresses** before kTLS install
  (write() immediately on connect() return + deliberately delayed install).

Each slice's "After/sees" is framed against these concrete observables (the
security-reviewer-facing wire capture IS the value), so slices are not empty
`@infrastructure` shells. The slice-composition hard gate is respected: every
slice has a genuine observable. The walking-skeleton spike's observable is the
wire capture for one process flow.

---

## SSOT updates produced by this wave

- **`docs/product/jobs.yaml`**: appended **J-SEC-003** (`served_by_phase: 2`,
  `status: active`, `relates_to: J-SEC-002`, full dimensions + four forces) +
  changelog entry.
- **`docs/product/journeys/enforce-transparent-mtls-on-the-wire.yaml`**: NEW
  product-level journey mapping J-SEC-003, Sam persona, lightweight depth, happy
  path + the load-bearing error/race paths (plaintext race window;
  handshake-fail-closed on absent/wrong SVID; CP/agent-restart survival of
  in-flight kTLS sessions; KeyUpdate-unsupported → teardown+reconnect deferred to
  #229). Header states it does NOT extend the J-SEC-002 journey and names the
  #222/#229/#40 carve-outs.
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

- **Trigger: NO DIVERGE wave** → the mechanism option space is un-narrowed.
  Reported as the § Risk above; the spike-first WS is the mitigation. DESIGN may
  want a focused options pass (Arch A vs Arch C vs Cilium-fallback) if the spike
  is inconclusive.
- **Trigger: a novel/unshipped core hypothesis** (in-band sidecarless kTLS) →
  surfaced as the WS falsification + the documented Cilium fallback. No
  auto-expansion; the spike settles it.
- **Trigger: a server-speaks-first protocol scope question** (SMTP/FTP/SSH have an
  irreducible data-loss window without a write-block) → surfaced as an open
  question / DESIGN scope call, not resolved in DISCUSS.

No Tier-2 expansions were auto-rendered.

---

## DoR validation

See `feature-delta.md` § Wave: DISCUSS / [REF] Definition of Ready — the 9-item
checklist with per-item evidence.
