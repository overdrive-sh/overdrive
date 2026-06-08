<!-- markdownlint-disable MD024 -->
# Feature Delta — workload-identity-manager (GH #35 · roadmap step 2.13)

**Wave**: DISCUSS (wave 2 of 6) · **Agent**: Luna (nw-product-owner) · **Density**: `lean` + `ask-intelligent` (DISCUSS hard default)

This is the single narrative artifact for the workload-identity-manager feature.
All DISCUSS content lives here under `## Wave: DISCUSS / [REF|WHY|HOW] <Section>`
headings. Tier-1 `[REF]` sections are emitted (lean default); no Tier-2
expansions were auto-rendered — the fired `ask-intelligent` triggers are reported
to the orchestrator rather than auto-expanded (see § Wave: DISCUSS / [REF] Density
& Triggers).

The **job is already validated in DIVERGE** (J-SEC-002) — JTBD was NOT re-run
this wave. The **architecture is LOCKED** (DIVERGE Option 1) — stories, slices,
and ACs are written against it and do not re-open it.

---

## Wave: DISCUSS / [REF] Feature Summary

**What**: The platform's workload `IdentityMgr` subsystem — the loop that binds a
live, chain-verifiable SVID to the **exact set of currently-running
allocations**, holds it where the dataplane can read it, and drops it the moment
a workload stops. A standalone **`SvidLifecycle` reconciler** observes allocation
`Running ↔ Stopped` and emits typed **`Action::IssueSvid` / `Action::DropSvid`**;
an **action-shim executor** mints via the shipped `ca_issuance::issue_and_audit`
(which already binds issuance + the `issued_certificates` audit row, ADR-0063 D6)
and writes a shared **`Arc<IdentityMgr>`** (held-SVID map keyed by
`AllocationId` + the current `TrustBundle`); dataplane consumers
(sockops/gateway/telemetry) read via sync getters behind an **`IdentityRead`
port trait**. Convergence is proven by an
`assert_eventually!("running allocs hold a valid SVID")` DST invariant.

**Why** (J-SEC-002): so design principle 3 — "every packet carries cryptographic
workload identity" — is **operationally true for the running set**, not merely
mintable in principle (which is J-SEC-001 / #28). A CA can be perfect and this
job entirely unmet: the SVID is mintable but never held, never readable by the
mTLS layer, never dropped on stop. Overdrive is **sidecarless** (whitepaper §7) —
there is no in-pod agent to fetch/hold/drop a credential, so the credential's
lifecycle can *only* be driven from the allocation lifecycle the control plane
already owns.

**Feature type**: Cross-cutting (security primitive spanning `overdrive-core`
[`SvidLifecycle` reconciler + `IdentityRead` port trait + `Action::IssueSvid` /
`DropSvid` variants], `overdrive-control-plane` [`IdentityMgr` struct +
action-shim executor + runtime wiring], `overdrive-sim` [`SimIdentityRead` +
related sim doubles]).

**Evidence base**: `docs/feature/workload-identity-manager/diverge/` —
`job-analysis.md` (J-SEC-002 mint, §3 three-reason justification; six ODI
outcomes §5), `recommendation.md` (Option 1 locked, the 5 design-sensitive
surfaces DESIGN must pin), `competitive-research.md`, `taste-evaluation.md`
(Option 1 = 4.45, rank 1). Brownfield: the shipped `Ca` port trait (#28,
ADR-0063, `crates/overdrive-core/src/traits/ca.rs`) + `ca_issuance::issue_and_audit`
(`crates/overdrive-control-plane/src/ca_issuance.rs`) already mint + audit; this
feature builds the *holder/reader/dropper* on top.

---

## Wave: DISCUSS / [REF] Persona

- **`sam-platform-security-engineer`** (Sam Okafor) — platform/security engineer
  who builds AND operates Overdrive's identity layer; has run SPIRE + Vault and
  hated it; threat-models by default; verifies with `openssl verify` rather than
  trusting the platform's word. SSOT:
  `docs/product/personas/sam-platform-security-engineer.yaml`. **Reused** from
  built-in-ca (the persona file already says "and future J-SEC-* jobs"); this
  wave adds `J-SEC-002` to its `related_jobs` and a J-SEC-002 *liveness* lens
  (is the running set consistently identity-bearing? no leak on stop? can the
  dataplane read it the instant it must present?). Same skeptical→confident
  security-review arc — no rich human emotional arc (D3 Lightweight).

---

## Wave: DISCUSS / [REF] JTBD One-liner

**J-SEC-002** — *"Keep every running workload holding a live, readable identity
the dataplane can present — and nothing held for a workload that has stopped."*
`relates_to: J-SEC-001`.

> When a workload I run transitions into (or out of) the Running state — and my
> sockops/kTLS mTLS layer, my L7 gateway, and my telemetry sink all need to
> present or stamp that workload's identity *right now* — and Overdrive is
> sidecarless (no in-workload agent), **I want** the platform to bind a live,
> chain-verifiable SVID to that exact allocation (issued the moment it starts,
> held where my dataplane consumers can read it, dropped the moment it stops),
> **so** design principle 3 is operationally true for the running set — the
> window between Running and identity-held bounded to one reconcile tick and
> closed by convergence (a workload with no held SVID cannot present identity, so
> the mTLS consumer fails closed — the bounded window is not an exposure), no leak
> where a stopped workload's leaf key lingers, identity availability a DST-proven
> converged invariant rather than a hope.

**Validated in DIVERGE — NOT re-run this wave.** Full job (functional/emotional/
social dimensions + the six ODI outcomes) is in the SSOT
`docs/product/jobs.yaml` § `J-SEC-002` and `diverge/job-analysis.md` §4–5.

### ODI outcomes (from DIVERGE §5 — every story traces to these)

| # | Outcome | Opp. | Status |
|---|---|---|---|
| **O1** | Minimize likelihood a workload is Running without a held, chain-verifiable SVID. **North Star** — the `assert_eventually!("running allocs hold a valid SVID")` invariant. | 18.0 | Under-served |
| **O2** | Minimize likelihood `SvidMaterial` (incl. leaf private key) is held for an allocation no longer Running (drop-on-stop / leak-resistance). | 16.0 | Under-served |
| **O3** | Minimize time a dataplane consumer takes to read the current SVID + trust bundle it must present. | 16.0 | Under-served |
| **O4** | Minimize likelihood a control-plane restart leaves a running workload with no held SVID, or re-issues without an audit trail (bounded, audited restart re-issue — no stale or silent credential). | 13.0 | Under-served |
| **O5** | Minimize likelihood an SVID is handed out without an observable `issued_certificates` audit row (no silent issuance). | 10.0 | Appropriately served (reuse `issue_and_audit`) |
| **O6** | Minimize additional concurrency/storage mechanisms beyond the shipped reconciler runtime + `Ca` port + ObservationStore (mechanism economy). | 10.0 | Appropriately served |

---

## Wave: DISCUSS / [REF] Brownfield Evaluation (Walking Skeleton — D2)

**This is a brownfield feature: a net-new subsystem consuming already-shipped
seams. There is NO greenfield walking-skeleton proposal.**

| Already shipped (consumed, not rebuilt) | Where | This feature adds |
|---|---|---|
| `Ca` port trait (`issue_svid` / `issue_intermediate` / `trust_bundle`) | `overdrive-core/src/traits/ca.rs` (#28, ADR-0063) | The *holder/reader/dropper* of what `Ca` mints |
| `ca_issuance::issue_and_audit` (mint leaf + write `issued_certificates` row, bound so audit-write failure refuses issuance) | `overdrive-control-plane/src/ca_issuance.rs` | The executor that *calls* it from the issue action |
| Reconciler runtime (pure `reconcile()` → `(Vec<Action>, View)`; bulk-load + write-through View store) | `.claude/rules/development.md` § "Reconciler I/O" | A new `SvidLifecycle` reconciler on it |
| Action-shim executor pattern (`ServiceMapHydrator` → `Action::DataplaneUpdateService` → executor) | `action_shim/dataplane_update_service.rs` | A mirror `action_shim/issue_svid.rs` executor |
| `SvidMaterial` (cert PEM/DER + serial + spiffe_id + node-held `leaf_key`, redacted Debug) · `TrustBundle` · `IssuedCertificateRow` | `overdrive-core/src/traits/ca.rs`, `ca/issued_certificate_row.rs` | The in-process map that holds `SvidMaterial`; the read surface |

**Walking skeleton (D2 brownfield — the feature's own thinnest cut)**: realised
in **Slice 01**. Alloc reaches Running → `SvidLifecycle` emits `IssueSvid` →
executor mints via `issue_and_audit` → `SvidMaterial` held in `IdentityMgr` →
alloc stops → `DropSvid` → entry (and leaf key) dropped → `issued_certificates`
audit row observable → the `assert_eventually!` convergence invariant holds. The
**`IdentityRead` port + consumer read surface is the first enhancement past the
skeleton** (Slice 02), not part of it.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate — Phase 1.5)

**Verdict: PASS — right-sized as ONE feature, sliced into 3 thin vertical cuts.**

Oversized-signal check (oversized = any 2+ firing):

| Signal | Threshold | This feature | Fires? |
|---|---|---|---|
| User stories | >10 | 3 (US-WIM-01..03) | No |
| Bounded contexts | >3 | 1 (workload identity) — touches ~3 crates but one context | No |
| WS integration points | >5 | ~4 (alloc-lifecycle observe → issue action → executor/CA → hold/drop → audit) | No |
| Estimated effort | >2 weeks | ~3–4 days (3 × ≤1-day slices, Slice 03 ~1.5d) | No |
| Independent shippable outcomes | multiple | 1 coherent outcome (the running set holds live identity) | No |

Zero signals fire. The feature is one coherent capability (the running set holds
a live, readable, lifecycle-bound identity) and is **not** split into multiple
features. Same shape as the sibling #28 (built-in-ca): one primitive, sliced
thinly internally. It IS sliced thinly (carpaccio) — 3 slices, each ≤1 day,
each end-to-end, each with a learning hypothesis (see § Story Map). All carpaccio
taste tests pass (documented per-slice in the slice briefs).

---

## Wave: DISCUSS / [REF] Story Map

**Persona**: Sam (platform/security engineer) · **Goal**: the set of running
workloads consistently holds a live, readable, chain-verifiable identity, and a
stopped workload holds none.

### Backbone (platform activities, left → right)

| A. Notice a workload's lifecycle | B. Bind/unbind its identity | C. Make identity readable | D. Prove & audit the binding |
|---|---|---|---|
| Observe alloc Running↔Stopped (S01) | Issue SVID on Running, hold it (S01) | Hold in `Arc<IdentityMgr>` (S01) | Write `issued_certificates` row (S01) |
| Re-issue every running alloc on restart (held set = `actual`, empty on boot — S03, DESIGN rev 2) | Drop SVID + leaf key on Stop (S01) | Read via `IdentityRead` getters (S02) | Prove the `assert_eventually!` convergence invariant (S01) |
| | (gated) signal #40 rotation near-expiry (S03) | Read current trust bundle (S02) | Bounded, audited restart re-issue; no stale/silent credential (S03) |

### Walking Skeleton (thinnest end-to-end, all activities)

**Slice 01**: observe Running → issue + hold → drop-on-stop (key purged) → audit
row → convergence invariant holds. The minimum slice that touches every
backbone activity (notice → bind/unbind → hold → prove/audit).

### Release 1 (first enhancement past the skeleton)

**Slice 02**: the `IdentityRead` port trait + consumer-facing sync read surface
(SVID + trust bundle) with a `SimIdentityRead` double. Targets O3 (consumer read
latency) — the read surface the whole subsystem exists to serve.

### Release 2 (durability + the rotation seam)

**Slice 03**: restart **recovery** (DESIGN rev 2 corrected the mechanism — see §
Changed Assumptions + `design/upstream-changes.md`): the held set is the
reconciler's `actual`, so on boot `actual = ∅` → re-issue every still-Running
alloc (bounded, audited); the View is **retry memory** so a *failed* re-issue
backs off. Includes the **pre-wired-but-gated #40 rotation seam** (near-expiry
branch present, keyed off `actual.not_after`, emit dormant until #40 registers
`cert_rotation`). Targets O4/K3 (reframed: bounded, audited restart re-issue).

### Slice list (each = one ≤1-day vertical cut)

| Slice | Stories | Learning hypothesis (disproves X if it fails) | Brief |
|---|---|---|---|
| 01 (**walking skeleton**) | US-WIM-01 (core), US-WIM-03 | a standalone `SvidLifecycle` reconciler + `IssueSvid`/`DropSvid` actions + executor can bind the held-SVID set to the running-alloc set, drop the leaf key on stop, audit each issuance, and the binding converges (DST `assert_eventually!`) | `slices/slice-01-issue-hold-drop-audit-converge.md` |
| 02 | US-WIM-02 | dataplane consumers can read the current SVID + trust bundle through an `IdentityRead` port (sync getters, in-process, no re-issue per read), mockable via a sim double | `slices/slice-02-identity-read-port-and-consumer-surface.md` |
| 03 | US-WIM-01 (O4 + seam) | a workload running across a control-plane restart is re-issued a fresh, audited SVID (bounded — `running ∧ ¬held → IssueSvid`, held set = `actual`, empty on boot), a *failed* re-issue backs off via the retry-memory View, and the #40 rotation seam is a clean no-op (no `UnknownWorkflow`-per-tick) until #40 registers the kind (DESIGN rev 2) | `slices/slice-03-restart-idempotence-and-gated-rotation-seam.md` |

---

## Wave: DISCUSS / [REF] Priority Rationale

Execution order = **learning leverage first** (highest-uncertainty slices early,
so failures cost least) then **dependency chain** then **dogfood cadence**.

| Order | Slice | Why this position |
|---|---|---|
| 1 | S01 | **Riskiest assumption + walking skeleton** — does the whole "identity warrants its own convergence target separate from `WorkloadLifecycle`" thesis hold? If the `SvidLifecycle`-reconciler + actions + executor + held-store + `assert_eventually!` invariant cannot be made to converge cleanly, the locked Option 1 is wrong and everything downstream is moot. Cheapest place to learn it. Delivers the headline dogfood moment (a running workload demonstrably holds a verifiable identity; a stopped one holds none). |
| 2 | S02 | Depends on S01 (needs a populated `IdentityMgr` to read from). Resolves the O3 read-surface risk — the `IdentityRead` port shape the unbuilt consumers (#26 sockops / gateway / telemetry) will bind to. Moderate uncertainty (port-trait + sim double is well-trodden). |
| 3 | S03 | Depends on S01 (needs the issue path + View to persist inputs into). Lowest crypto/convergence uncertainty (additive durability on a proven loop) but carries the **load-bearing #40-seam gating caveat** — must be a clean no-op, not `UnknownWorkflow`-per-tick. Resolves O4 + lands the rotation seam last so #40 has a stable surface. |

Dependency chain is **S01 → {S02, S03}** (S02 and S03 both depend on S01 but not
on each other — they could parallelise after S01). The order above puts the
read-surface (S02) before durability (S03) because the read surface is the
higher-opportunity outcome (O3 opp 16 vs O4 opp 13) and de-risks the consumer
seam earlier. No deeper parallelism is available pre-S01.

---

## Wave: DISCUSS / [REF] System Constraints (cross-cutting)

These apply to every story; stated once here rather than repeated per story.

- **Reconciler purity (CORRECTNESS constraint, not taste — DIVERGE D-WIM-3)**:
  CA I/O lives in the **action-shim executor**, NEVER in `reconcile()`. The
  `SvidLifecycle` reconciler is a pure `reconcile(desired, actual, view, tick) →
  (Vec<Action>, View)` — no `.await`, no `Ca` handle, no `ObservationStore`
  handle, no wall-clock read except `tick.now`. It *emits* `Action::IssueSvid` /
  `Action::DropSvid`; the executor (`action_shim/issue_svid.rs`, mirroring
  `dataplane_update_service.rs`) calls `ca_issuance::issue_and_audit`
  (`.claude/rules/development.md` § "Reconciler I/O").
- **Persist INPUTS, not derived state** (`.claude/rules/development.md` §
  "Persist inputs, not derived state"): the `SvidLifecycle` View is **retry
  memory only** — `IssueRetry { attempts, last_failure_seen_at }` (ADR-0067
  rev-2 D8). It carries **no** `serial`, `issued_at`, `spiffe_id`, `expires_at`,
  or `next_renewal_at`: the held leaf key is non-persistable (ADR-0063), so
  issuance success facts live in the `issued_certificates` observation, never
  the View. Near-expiry (for the gated #40 seam) reads the **held cert's
  `not_after` from `actual`** (the held-set projection), not a persisted field.
  A success-fact or derived-deadline field in the View is a review-rejection smell.
- **State-layer hygiene** (whitepaper §4, ADR-0063 D2/D6): the **held
  `SvidMaterial`** (incl. the node-held leaf private key) lives **in-process** in
  `IdentityMgr` — it is neither intent nor observation; it is ephemeral runtime
  state bounded to the running set, intentionally never persisted (the leaf key
  is not an audit fact and must not reach disk). The **`issued_certificates`
  audit row** is **observation** (gossiped when #36 lands; single-node = local),
  written via the `ObservationStore` exactly like `alloc_status` / `node_health`.
  The **`SvidLifecycle` View** is **reconciler memory** (the runtime-owned
  ViewStore), persisting only retry memory (`attempts` + `last_failure_seen_at`). These three never merge.
- **`BTreeMap`, not `HashMap`** (`.claude/rules/development.md` §
  "Ordered-collection choice"): the held-SVID map IS iterated — the
  `assert_eventually!("running allocs hold a valid SVID")` DST invariant walks it
  — so its iteration order must be deterministic across seeds. The View's
  per-allocation map is `BTreeMap` for the same reason (bulk-loaded + observed).
- **Port-trait discipline for `IdentityRead`** (`.claude/rules/development.md` §
  "Port-trait dependencies"): the `IdentityRead` trait lives in `overdrive-core`;
  consumers take it as a **required constructor parameter** (never defaulted); a
  `SimIdentityRead` test double exists for tests. A DST equivalence test drives
  the real `IdentityMgr` read surface and the sim double through the same calls.
- **Secret hygiene on drop (O2)**: `SvidMaterial` holds `leaf_key: CaKeyPem`
  whose `Debug` is redacted (`CaKeyPem(<redacted>)`, ADR-0063). Drop-on-stop MUST
  remove the entry from the held map so the leaf key is no longer reachable; the
  acceptance proof is that the held set no longer contains the stopped
  allocation. (Memory-scrubbing the key bytes beyond removing the held entry is
  **explicitly NOT in #35 — accepted residual risk**: O2 is *reachability*
  (drop-on-stop removes the entry → the key is no longer reachable), not memory
  zeroization. Do not invent a zeroizing wrapper on initiative.)
- **Convergence window is fail-closed, NOT a race (O1)**: the gap between an
  allocation reaching Running and its `IssueSvid` executing is bounded to one
  reconcile tick and closed by the `assert_eventually!` convergence loop — it is
  **not** an exposure. A workload with no held SVID cannot present identity, so
  the mTLS consumer (**#26** sockops/kTLS — the enforcer, itself out of scope and
  unbuilt this phase) **fails closed** on the missing credential. This feature
  does NOT add a registration-gating AC (that over-reaches into
  BackendDiscovery/#26 scope); fail-closed at the consumer already covers it. In
  Phase 2 there is no consumer at all, so the window is doubly inert.
- **No operator CLI verb in this phase; #35 is a FOUNDATION feature (F2)**: SVID
  hold/read/drop is an internal platform mechanism — there is **no** `overdrive`
  subcommand to "issue/hold an SVID". #35's own HONEST observable surfaces are
  TEST-tier: (a) the `issued_certificates` row is WRITTEN per issuance (read back
  via the ObservationStore in a gated `integration-tests` test), and (b)
  `openssl verify -CAfile <root> -untrusted <intermediate> <svid.pem>` exiting 0
  on the minted leaf chain at the TEST tier (built-in-ca's `rcgen_ca_chain_verify`
  shape). The **operator** surfaces — `overdrive alloc status --job <id>`
  rendering the `issued_certificates` row, and the deployed-SVID operator-verify
  flow — are **#215's** deliverable (O05/E03), **blocked on #35**: the current
  `AllocStatusResponse` has no issued-cert field and the renderer shows no certs,
  and there is no SVID consumer (#26) yet. So #35 does NOT claim the operator
  render as its own observable. Per CLAUDE.md the workload verb is `overdrive
  deploy <SPEC>`, **never** `job submit`. Do NOT invent a CLI verb.
- **Single-node (Phase 2)**: one co-located node; the held set is one node's
  running allocations. Multi-node (per-node held sets, gossiped audit rows, node
  attestation) is owned by **#36 [2.14]** (node enrollment / admission handler).
- **Rotation deferred to #40 with a gated pre-wired seam** (DIVERGE D-WIM-5/D-WIM-8):
  the near-expiry branch is structurally present and targets
  `Action::StartWorkflow(cert_rotation)`, but the emit MUST be **gated/dormant**
  until #40 registers `cert_rotation`. A committed `StartWorkflow` for an
  *unregistered* kind surfaces `WorkflowEngineError::UnknownWorkflow`
  (`overdrive-control-plane/src/lib.rs:417-418`), isolated per-action by the shim
  (`action_shim/mod.rs:429`) but **re-emitted each tick the condition holds** — so
  a naïve emit raises `UnknownWorkflow` every tick. The seam stays a *clean*
  no-op until #40 flips the gate. NO throwaway synchronous sync-rotate path
  (single-cut violation #40 would delete).

---

## Wave: DISCUSS / [REF] User Stories

Every story traces to `job_id: J-SEC-002`. Every story has an Elevator Pitch.
ACs are embedded and derived from the UAT scenarios. None are `@infrastructure`
(each delivers a verifiable identity-availability property — see Elevator Pitches).

> **Elevator-Pitch "After" caveat (SAME as built-in-ca — a security primitive
> with NO operator CLI verb)**: hold/read/drop is an internal platform mechanism;
> there is no `overdrive` subcommand to "issue/hold an SVID". Each pitch's "After"
> references a real, executable verification entry point — `openssl verify` on the
> minted leaf chain (exit 0) — which is the honest user-invocable observable
> output for this feature, not an invented subcommand (exactly built-in-ca's
> shape). The DECISION enabled is Sam's trust decision (the genuine J-SEC-002
> connection: is the running set consistently identity-bearing, with no leak on
> stop, readable when it must be presented?).
>
> **Foundation-feature framing (F2, 2026-06-08 revision)**: in Phase 2, #35 is a
> FOUNDATION feature — it builds the lifecycle, the held store, the read port,
> WRITES the `issued_certificates` rows, and proves convergence. But its
> **operator surface** is **#215's** (compose built-in CA into the operator
> surface — the `alloc status` render of `issued_certificates` and the
> deployed-SVID operator-verify flow), and #215 is **blocked on #35**. The current
> `AllocStatusResponse` has no issued-cert field and the renderer shows no certs;
> there is also no SVID consumer (#26 sockops) yet, so the operator-facing render
> is genuinely future work that #215 owns. #35's own observable proof is therefore
> the **built-in-CA shape**: (a) the `issued_certificates` row is WRITTEN per
> issuance (testable via the ObservationStore in a gated integration test), (b)
> the leaf chain verifies under `openssl verify -CAfile <root> -untrusted
> <intermediate> <svid>` exit 0 at the **TEST tier** (gated `integration-tests`,
> exactly like built-in-ca's `rcgen_ca_chain_verify`), and (c) the DST
> `assert_eventually!("running allocs hold a valid SVID")` convergence invariant.
> The operator `alloc status` render + the deployed-SVID operator-verify flow are
> deferred to **#215 (its O05/E03 EDD expectations — `pending`, blocked on #35)**,
> NOT claimed as #35's own AC.
>
> **Foundation-feature exception to the strict elevator-pitch gate (recorded
> explicitly, NOT a silent pass — pass-2 F2, 2026-06-08)**: the strict nWave
> elevator-pitch gate requires *a real user-invocable entry point — not internal
> state, not "tests green."* #35 does **not** strictly satisfy that on its own:
> under the foundation framing (user-decided Option A), **none** of its three
> stories has a Phase-2 *operator-invocable* observable. Every Phase-2 proof is
> **TEST-tier** — `openssl verify` the chain in a gated `integration-tests` run,
> ObservationStore readback of the `issued_certificates` row, and the DST
> `assert_eventually!` convergence invariant. The operator-invocable surface is
> deferred to **#215** (the `alloc status` render, blocked on #35) and the
> consumer to **#26** (sockops/kTLS). This is **exactly built-in-ca's situation**
> — a security primitive with no operator verb, proven by `openssl verify` at the
> test tier (its O05/E03 finalized `pending` → #215). The gate is therefore met
> **by a deliberate, documented foundation-feature exception mirroring
> built-in-ca, NOT by a live operator surface and NOT by an invented CLI verb**
> (CLAUDE.md § "Implement to the design" — inventing a verb to dodge the gate is
> the dishonest move; recording the exception is the honest one). The exception
> is recorded in three places — here, in § Wave Decisions (D-WIM2-8), and in the
> DoR validation (the elevator-pitch / slice-composition items note it
> explicitly with this evidence pointer).

### US-WIM-01 — Issue-on-start, hold, drop-on-stop (the core lifecycle)

**Problem**: Sam has a CA that *can mint* an SVID (#28) but **nothing holds it**.
A workload reaches Running and there is no live credential bound to it that the
mTLS layer can present; when the workload stops, any minted credential (and its
leaf private key) would linger with no one to drop it. He finds it untenable that
"every packet carries identity" is true only *in principle* — mintable but never
held, never dropped — with no in-workload agent to manage the lifecycle.

**Who**: Platform/security engineer | operating the running set | wants identity
bound to the *lifetime* of each running workload, issued on start and dropped on
stop, with no in-workload agent.

**Solution**: A standalone `SvidLifecycle` reconciler observes alloc
`Running ↔ Stopped` and emits `Action::IssueSvid` (on Running) / `Action::DropSvid`
(on Stop). An action-shim executor mints via the shipped
`ca_issuance::issue_and_audit` and writes the `SvidMaterial` into a shared
`Arc<IdentityMgr>` (held-SVID `BTreeMap` keyed by `AllocationId`); drop removes
the entry, so the leaf private key is no longer held. The subsystem owns the
allocation's SPIFFE URI assignment.

#### Elevator Pitch

- **Before**: a workload can reach Running with no live, held identity the
  dataplane can present, and a stopped workload's credential would linger
  unmanaged (Overdrive is sidecarless — no in-workload agent to hold/drop it).
- **After**: deploy a workload (`overdrive deploy payments.toml`); the running
  alloc's held leaf chain verifies under `openssl verify -CAfile root.pem
  -untrusted intermediate.pem <svid.pem>` → exits 0, an `issued_certificates`
  row is written for the issuance (observable via the ObservationStore in a
  gated integration test), and after stop the held set no longer contains the
  allocation (no leak). (The operator `alloc status` render of that row + the
  deployed-SVID operator-verify flow are **#215's** O05/E03, blocked on #35.)
- **Decision enabled**: Sam decides the running set is consistently
  identity-bearing and stop genuinely drops the credential — or refuses to ship
  if a running alloc has no held SVID, or a stopped alloc's leaf key lingers.

#### Domain Examples

1. **Happy path** — Sam runs `overdrive deploy payments.toml`; allocation
   `a1b2c3` reaches Running. `SvidLifecycle` emits `IssueSvid`; the executor
   mints a leaf with SAN `spiffe://overdrive.local/job/payments/alloc/a1b2c3` via
   `issue_and_audit`, holds the `SvidMaterial` in `IdentityMgr`, and the
   `issued_certificates` row is written (read back via the ObservationStore in a
   gated test); `openssl verify` of the minted leaf chain exits 0.
2. **Drop-on-stop (O2)** — Sam stops `payments`; allocation `a1b2c3` goes
   Stopped. `SvidLifecycle` emits `DropSvid`; the executor removes the
   `AllocationId` from the held map. A read of the held set for `a1b2c3` returns
   nothing — the leaf private key is no longer held in memory.
3. **Race-before-held (O1)** — A second allocation `d4e5f6` of job `orders`
   reaches Running but its `IssueSvid` has not yet been executed (one tick
   behind). The held set transiently lacks `d4e5f6`; the convergence loop issues
   on the next tick and the `assert_eventually!` invariant holds (the window is
   bounded, not permanent). This is the bounded convergence window the model
   exists to close — identity availability is *eventually* consistent for the
   running set, the bound is one reconcile tick (never "never"), and because a
   workload with no held SVID cannot present identity the mTLS consumer (#26)
   fails closed during the window — so it is not an exposure.

#### UAT Scenarios (BDD)

##### Scenario: A running workload holds a chain-verifiable identity it did not have before
Given a deployed workload whose allocation reaches the Running state
When the platform reconciles the workload's identity lifecycle
Then the running allocation has a held SVID whose leaf chain verifies to the root
And an `issued_certificates` row is written for that allocation (observable via the ObservationStore)

##### Scenario: Stopping a workload drops its held identity and leaf key
Given a running workload that holds an SVID
When the workload stops and the platform reconciles
Then the stopped allocation has no held SVID
And the leaf private key for that allocation is no longer held in memory

##### Scenario: Identity availability converges for the running set
Given a set of allocations transitioning into and out of the Running state
When the platform reconciles repeatedly
Then eventually every Running allocation holds a valid SVID and no stopped allocation holds one

#### Acceptance Criteria

- [ ] A standalone `SvidLifecycle` reconciler emits `Action::IssueSvid` when an allocation reaches Running and `Action::DropSvid` when it stops; `reconcile()` is pure (no `.await`, no CA/observation handle, wall-clock only via `tick.now`) and passes dst-lint.
- [ ] The executor mints via the shipped `ca_issuance::issue_and_audit` (NOT re-implemented) and writes the `SvidMaterial` into the shared `Arc<IdentityMgr>` held map (keyed by `AllocationId`).
- [ ] After Running, the `issued_certificates` row is written for the issuance (read back via the ObservationStore in a gated `integration-tests` test) and `openssl verify -CAfile <root> -untrusted <intermediate> <svid.pem>` exits 0 at the TEST tier (gated `integration-tests`, via Lima — exactly built-in-ca's `rcgen_ca_chain_verify` shape). (The operator `alloc status` render of that row + the deployed-SVID operator-verify flow are deferred to **#215** — its O05/E03 EDD expectations, `pending`, blocked on #35 — NOT #35's own AC.)
- [ ] After Stop, the held map no longer contains the allocation (drop-on-stop; the leaf private key is no longer reachable in the held set).
- [ ] DST: `assert_eventually!("running allocs hold a valid SVID")` holds across a sequence of Running/Stopped transitions at a fixed seed; the held map is `BTreeMap` (deterministic iteration).

#### Technical Notes

- Mirrors the shipped `ServiceMapHydrator` → `Action::DataplaneUpdateService` →
  `action_shim/dataplane_update_service.rs` executor pattern (DIVERGE Option 1).
- `ca_issuance::issue_and_audit(ca, observation, clock, node, request)` is `async`,
  returns `SvidMaterial`, and **already** binds the audit row (refuses issuance on
  audit-write failure) — the executor reuses it wholesale (O5 served).
- **SPIFFE URI derivation** — building `spiffe://overdrive.local/job/<name>/alloc/<id>`
  from the allocation: the **public constructor** `SpiffeId::for_allocation` is
  unbuilt (`SpiffeId::new(raw)` validates a raw string), BUT the derivation
  **already exists twice as private helpers** — `mint_alloc_identity`
  (`backend_discovery_bridge.rs:424`) + `mint_identity` (`workload_lifecycle.rs:808`).
  **DESIGN rev 2 (ADR-0067 D5) resolved this as a CONSOLIDATION**: `for_allocation`
  is the canonical extraction, the reconciler builds it (pure, into the `IssueSvid`
  field), and DELIVER migrates both existing call sites (single-cut — no third
  implementation).
- **#40 rotation seam** lands in Slice 03 (the near-expiry branch + gated emit);
  US-WIM-01's core issue/hold/drop carries no rotation logic.
- The exact `Action::IssueSvid` / `Action::DropSvid` field set and the
  `IdentityMgr` concurrency primitive are **DESIGN's to pin** (design-surfaces #1, #3) — not decided here.

---

### US-WIM-02 — Shared read surface behind the `IdentityRead` port

**Problem**: Sam's dataplane consumers — the kernel-side sockops/kTLS mTLS layer
(#26), the L7 gateway, the telemetry sink — need to read a workload's current
SVID + the trust bundle the instant they must present or stamp identity, **on
the connection hot path**, with no gRPC, no IPC, and no re-issuing per use. Today
there is no read surface at all.

**Who**: Platform/security engineer | wiring the dataplane consumers that present
identity | wants an in-process, low-latency, testable read surface for the held
SVID + current trust bundle.

**Solution**: An `IdentityRead` port trait (core) exposing sync getters — the
current `SvidMaterial` for an `AllocationId` and the current `TrustBundle` —
implemented by `IdentityMgr` (reads its `RwLock`ed held map + bundle) with a
`SimIdentityRead` double for tests. The read **contract** is proven by the port +
`SimIdentityRead` + a **test consumer/fixture** that takes `Arc<dyn IdentityRead>`
as a required constructor parameter (demonstrating the port-trait discipline);
the **production** consumers that take the port for real (sockops #26 / gateway /
telemetry) are deferred to those features, not wired here.

#### Elevator Pitch

- **Before**: there is no surface for the dataplane to read a workload's held
  SVID or the current trust bundle — the mTLS layer (#26), gateway, and telemetry
  have nothing to bind to.
- **After**: the held identity is exposed behind an `IdentityRead` port whose
  getters return the current SVID + trust bundle in-process (no re-issue per
  read); the observable proof is `openssl verify -CAfile <root> -untrusted
  <intermediate> <svid.pem>` → exit 0 on the leaf the getter returns (the same
  material a consumer would read), proven through a **test consumer/fixture**
  driving the port (no production consumer exists this phase). (The operator
  `alloc status` render of that allocation's `issued_certificates` row is
  **#215's** O05, blocked on #35.)
- **Decision enabled**: Sam decides the dataplane consumers (sockops/gateway/
  telemetry) have a sound, low-latency, mockable seam to read identity from — or
  rejects a read surface that re-issues per read or cannot be exercised in tests.

#### Domain Examples

1. **Happy path** — A **test consumer** holds `Arc<dyn IdentityRead>`; for a
   running allocation `a1b2c3` it calls the getter and receives the held
   `SvidMaterial` (cert + leaf key for the handshake) plus `current_bundle()` —
   **without re-issuing** (no `issue_svid` on the read path; the SVID is served
   from the held map). Bundle currency is served from the **hydrated** bundle in
   `IdentityMgr` (DESIGN rev 2 resolved Open-Questions #5 as HYDRATED — ADR-0067
   D6; zero CA I/O on the read hot path); the SVID itself is never re-issued per
   read. The returned leaf chain verifies under `openssl verify` at the TEST tier.
2. **Read for an absent allocation** — A consumer reads identity for an
   allocation that is not currently held (stopped, or not yet issued). The getter
   returns `None` (absence is explicit), so the consumer can refuse the handshake
   rather than present a stale credential.
3. **Test double (DST)** — A consumer test wires `SimIdentityRead` preloaded with
   a fixture `SvidMaterial` + bundle; the consumer behaves identically to the
   real `IdentityMgr` read path. A DST equivalence test drives both through the
   same calls and asserts identical observable reads.

#### UAT Scenarios (BDD)

##### Scenario: A consumer reads a running workload's current identity without re-issuing it
Given a running workload whose SVID is held
When a dataplane consumer reads that workload's identity through the read surface
Then it receives the current SVID and the current trust bundle
And no new certificate is issued as a result of the read

##### Scenario: Reading identity for a workload that is not held is explicit
Given an allocation that is not currently held (stopped or not yet issued)
When a dataplane consumer reads that allocation's identity
Then the read surface reports the identity as absent rather than returning a stale credential

#### Acceptance Criteria

- [ ] An `IdentityRead` port trait in `overdrive-core` exposes sync getters for the current SVID (by `AllocationId`) and the current trust bundle; the trait docstring pins observable behaviour (incl. that a read never triggers issuance).
- [ ] `IdentityMgr` implements `IdentityRead` by reading its held map + current bundle; a **test consumer/fixture** takes `Arc<dyn IdentityRead>` as a required constructor parameter (never defaulted), proving the port-trait discipline as a contract. (Production consumer wiring — sockops #26 / gateway / telemetry — is deferred to those features, not an AC here.)
- [ ] A read for a held allocation returns the current `SvidMaterial` + `TrustBundle` **without re-issuing** — no `issue_svid` on the read path; the SVID is served from the held map and the bundle from the **hydrated** `IdentityMgr` (DESIGN rev 2 resolved Open-Questions #5 as HYDRATED — ADR-0067 D6; zero CA I/O on the read hot path). Re-issuing the SVID per read is forbidden.
- [ ] A read for an absent allocation returns an explicit "absent" (e.g. `None`), not a stale or empty-but-present credential.
- [ ] A `SimIdentityRead` double exists; a DST equivalence test drives the real read path and the sim double through the same calls and asserts identical observable reads.

#### Technical Notes

- The exact `IdentityRead` signatures (`svid_for(&AllocationId) ->
  Option<SvidMaterial>` + `current_bundle() -> TrustBundle` is the *recommended*
  shape) and the `SimIdentityRead` double are **DESIGN's to pin**
  (design-surface #2). Do NOT finalise the signatures here.
- O3 (read latency) is served by the in-process `Arc` + sync getter (whitepaper
  §7 "no gRPC, no IPC"). The getter→`watch`-channel push upgrade (DIVERGE
  Option 3) is a **non-breaking future change behind this port** — explicitly
  NOT in scope (no consumer demands change-notification yet).
- The **production** consumers (#26 sockops, gateway, telemetry) are **out of
  scope** — this story ships the *read port + its sim double + a test consumer
  that proves the read contract*, not the production consumers. The required-ctor-
  param discipline is demonstrated by the test consumer, not by wiring a real one.

---

### US-WIM-03 — Convergence proven + issuance audited (no silent issuance)

**Problem**: Sam will not accept "the running set holds identity" as an
assertion. He needs it **mechanically checked** — a DST convergence invariant
that fails the build if a Running allocation lacks a held SVID — and he needs
every issuance to leave an observable audit trail (no SVID handed to a consumer
without a corresponding `issued_certificates` row), so he can tell a security
reviewer "identity availability is proven, and nothing was issued silently" and
have it be true.

**Who**: Platform/security engineer | defending the identity story to a security
reviewer | wants identity availability proven by a convergence invariant and
every issuance auditable.

**Solution**: A DST `assert_eventually!("running allocs hold a valid SVID")`
invariant over the held map vs the running-allocation set; reuse of
`ca_issuance::issue_and_audit` so every issuance writes an `issued_certificates`
observation row and an audit-write failure refuses the issuance.

#### Elevator Pitch

- **Before**: "the running set holds identity" is an unverified claim, and there
  is no guarantee every issuance is recorded — an SVID could in principle be
  handed out with no audit trail.
- **After**: a DST invariant fails the build if any Running allocation lacks a
  held valid SVID, an `issued_certificates` row (serial / spiffe_id /
  issuer_serial / validity) is written for every issuance (read back via the
  ObservationStore in a gated test), and `openssl verify` confirms the audited
  leaf chains to the root at the TEST tier. (The operator `alloc status` render of
  that row is **#215's** O05/E03, blocked on #35.)
- **Decision enabled**: Sam decides identity availability is *mechanically*
  guaranteed (not asserted) and issuance is fully auditable — defensible in a
  security review — or rejects the feature if the invariant is decorative or an
  issuance can escape its audit row.

#### Domain Examples

1. **Convergence invariant (O1)** — Under the seeded DST harness, allocations
   churn Running↔Stopped; `assert_eventually!("running allocs hold a valid
   SVID")` walks the held `BTreeMap` against the running set and holds at every
   stable point. A deliberately broken executor (drops the hold) fails the
   invariant — proving it has teeth.
2. **Audit on issuance (O5)** — Each `IssueSvid` execution writes an
   `issued_certificates` row `{serial, spiffe_id, issuer_serial, not_before,
   not_after, node_id, issued_at}`; a test reads it back via the ObservationStore
   surface and the serial matches the held cert. No issuance without a row.
3. **Audit-write failure (O5)** — The `issued_certificates` write fails; because
   the executor reuses `issue_and_audit`, the issuance is **refused** — no
   unaudited `SvidMaterial` is ever placed in the held map. The held set does not
   gain an unaudited entry.

#### UAT Scenarios (BDD)

##### Scenario: Identity availability is proven by a convergence invariant
Given the seeded simulation harness with allocations churning into and out of Running
When the platform reconciles repeatedly
Then at every stable point every Running allocation holds a valid SVID and no stopped allocation holds one
And a deliberately broken hold fails the invariant

##### Scenario: Every issuance is audited; an unauditable issuance is refused
Given a workload whose allocation reaches Running
When the platform issues its SVID
Then an `issued_certificates` row records the serial, SPIFFE ID, issuer serial, and validity window
And if that audit row cannot be written, the issuance is refused and no unaudited SVID is held

#### Acceptance Criteria

- [ ] A DST `assert_eventually!("running allocs hold a valid SVID")` invariant compares the held `BTreeMap` against the running-allocation set and holds at every stable point across Running/Stopped churn at a fixed seed.
- [ ] The invariant has teeth: a deliberately broken executor (fails to hold, or fails to drop) fails the invariant.
- [ ] Every `IssueSvid` execution writes an `issued_certificates` observation row via `issue_and_audit`; a test reads it back via the observation surface and matches serial + spiffe_id + issuer_serial against the held cert.
- [ ] An issuance whose audit row cannot be written is refused (reuse of `issue_and_audit`'s binding); no unaudited `SvidMaterial` is placed in the held map (no silent issuance).
- [ ] `openssl verify` on the audited leaf chain exits 0 at the TEST tier (gated `integration-tests`, via Lima — the audited cert is the verifiable cert; built-in-ca's `rcgen_ca_chain_verify` shape). (The operator `alloc status` render is deferred to **#215** O05/E03, blocked on #35.)

#### Technical Notes

- `issue_and_audit` already refuses issuance on audit-write failure
  (`CaIssuanceError::Audit`) — US-WIM-03 *relies on* that binding rather than
  re-implementing it (O5 appropriately served per DIVERGE §5).
- The `assert_eventually!` invariant is the North-Star (O1) acceptance surface;
  it is the convergence target the whole subsystem is built to satisfy. It walks
  the held map, so the map MUST be `BTreeMap` (deterministic iteration across
  seeds, per System Constraints).
- This story shares Slice 01 with US-WIM-01's core (the convergence invariant +
  audit are proven on the same thin cut that lands issue/hold/drop) — it is not
  a separate slice, keeping the walking skeleton genuinely end-to-end (notice →
  bind → hold → **prove/audit**).

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Objective

By the end of #35, the set of workloads the platform is running consistently
holds a live, chain-verifiable, readable SVID bound to each running allocation,
and a stopped workload holds none — with identity availability proven by a DST
convergence invariant, every issuance audited, and idempotence across restart —
reusing the shipped reconciler runtime + `Ca` port + ObservationStore rather than
new mechanisms.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| K1 | Running allocations | hold a valid, chain-verifiable SVID | 100% of Running allocations hold a valid SVID at every stable convergence point (the `assert_eventually!` invariant) | 0% (no holder exists today — the CA mints, nothing holds) | Seeded DST `assert_eventually!("running allocs hold a valid SVID")` over the held `BTreeMap` vs the running set (Slice 01) | Leading (North Star) |
| K2 | Stopped allocations | retain a held SVID / leaf private key in memory | 0 stopped allocations hold an SVID after the drop reconciles (drop-on-stop; no leak) | n/a (no holder ⇒ no drop today) | DST + acceptance test: held map contains no stopped allocation; leaf key no longer reachable in the held set (Slice 01) | Guardrail |
| K3 | A control-plane restart | leaves a Running allocation with no held SVID, or re-issues without leaving an `issued_certificates` audit row | 0 missing-after-restart holds and every restart re-issue leaves an audit row (bounded — one re-issue per still-Running alloc) | n/a (no held-state recovery exists today) | DST: restart the control plane mid-run; every still-Running alloc is re-issued (`running ∧ ¬held → IssueSvid`), each writing an `issued_certificates` row; a surviving leaf verifies (`openssl verify`) (Slice 03) | Guardrail |
| K4 | SVIDs handed to a consumer | without a corresponding observable `issued_certificates` audit row | 0 unaudited issuances (every held SVID has an audit row; an unauditable issuance is refused) | partial (the `issue_and_audit` binding exists; this feature is its first holder-side consumer) | Test: every held cert has a matching `issued_certificates` row read back via the ObservationStore (gated `integration-tests`); audit-write-failure refuses issuance (Slice 01/03). The operator `alloc status` render of the row is #215's O05, blocked on #35. | Guardrail |
| K5 | The held-identity subsystem | composes deterministically under DST | 100% of identity-lifecycle DST scenarios reproduce bit-identically from a seed | n/a | Seeded DST twin-run identity (`BTreeMap` iteration + serials via `Entropy`; fixture keys) reproduces identically (all slices) | Guardrail |

### Metric hierarchy

- **North Star**: K1 — % of Running allocations that hold a valid,
  chain-verifiable SVID. The single signal that identity availability is
  *operationally true for the running set* (the whole reason J-SEC-002 exists,
  distinct from J-SEC-001's "mintable in principle").
- **Guardrails** (must NOT degrade as slices land): K2 (no leak on stop), K3
  (idempotent across restart), K4 (no silent issuance), K5 (DST determinism).

### Measurement plan

| KPI | Data source | Collection method | Frequency | Owner |
|---|---|---|---|---|
| K1, K2, K5 | Seeded DST harness | `cargo dst` — `assert_eventually!` invariant + held-set assertions + twin-run identity | Per PR | crafter / CI |
| K3 | Seeded DST harness | Restart-mid-run scenario; assert every still-Running alloc is re-issued (`running ∧ ¬held → IssueSvid`), each writing an `issued_certificates` audit row (Slice 03) | Per PR | crafter / CI |
| K4 | Observation surface + host-adapter acceptance test | `issued_certificates` row read back via the ObservationStore; audit-write-failure refuses issuance; `openssl verify` on the minted leaf at the TEST tier (via Lima, `integration-tests`). The operator `alloc status` render → #215 O05/E03, blocked on #35. | Per PR | crafter / CI |

### Hypothesis

We believe that a standalone `SvidLifecycle` reconciler + `Arc<IdentityMgr>` held
store + `IdentityRead` port will achieve K1 (100% of Running allocations hold a
valid SVID at convergence) and K2 (0 leaked credentials on stop). We will know
this is true when the `assert_eventually!("running allocs hold a valid SVID")`
invariant holds across Running/Stopped churn at a fixed seed and the held set
contains no stopped allocation.

---

## Wave: DISCUSS / [REF] Out-of-scope (explicit non-goals)

Each non-goal cites its owning issue/phase/roadmap line. No hand-wavy forward
pointers; no invented issue numbers.

| Non-goal | Owner | Note |
|---|---|---|
| Certificate **rotation lifecycle** (scheduled near-expiry → mint-fresh → swap → retire) | **GH #40** [3.3] (a `cert_rotation` workflow), depends on workflow primitive **GH #39** [3.2] + this #35 | The near-expiry branch + View input are **pre-wired but gated** here (Slice 03); the emit is a clean no-op until #40 registers `cert_rotation`. NO throwaway synchronous sync-rotate path. |
| The dataplane **consumers** that present identity — kernel-side sockops/kTLS mTLS, the L7 gateway, the telemetry sink | **GH #26** (sockops/kTLS) + gateway/telemetry features | This feature ships the `IdentityRead` *read port + its sim double* (Slice 02), not the consumers. Building the consumers here would be a single-cut violation (#26 owns the kernel surface). |
| **Multi-node** held sets, gossiped audit rows across nodes, per-node identity, node attestation at bootstrap | **GH #36** [2.14] (node enrollment / admission handler; `Depends on #28`) | Single-node Phase 2: one node's running set. The `issued_certificates` row is already "gossiped when #36 lands; single-node = local" (ADR-0063). |
| **ACME / public-trust gateway certs** unified into `IdentityMgr` (the unified-`IdentityMgr`-across-SVID+ACME store) | **Roadmap step 4.7** (whitepaper §11) | The 4.7 commitment is firm (DIVERGE B1) and is *why* a dedicated `IdentityMgr` is the right seam — but 4.7's ACME certs (no allocation behind them) are not built here. The `IdentityMgr` is shaped to admit them later, not to hold them now. |
| The `watch`/`broadcast` **push** read surface (consumers notified on change) | **Future (DIVERGE Option 3)** — a non-breaking change behind the `IdentityRead` port | Speculative until a real consumer demands change-notification; the getter surface (Slice 02) is the sound first step and #40 rotation can push down the same port later. No new issue (an evolution of this feature's own port). |
| SVID **revocation** (CRL / OCSP / `revoked_operator_certs`) | **Phase 5** (whitepaper §8) | Revocation-by-expiry (1h TTL) is the model; gossip revocation is later. |
| Leaf-key **zeroization** on drop (memory-scrubbing beyond removing the held entry) | **NOT in #35 — accepted residual risk** | O2 is *reachability*: drop-on-stop removes the entry so the key is no longer reachable (O2 met). Zeroizing the key bytes is separate memory-scrubbing hardening, out of #35 scope — an explicit scoping decision, not an open question. |

---

## Wave: DISCUSS / [REF] Driving Ports & Pre-requisites

**Driving ports (inbound surfaces that trigger identity-lifecycle behaviour)**:
- The **allocation lifecycle** (the existing reconciler runtime observing alloc
  `Running ↔ Stopped`) → triggers `SvidLifecycle` → `IssueSvid` / `DropSvid`
  (US-WIM-01). This is the *only* trigger — there is no absence-of-CA trigger
  (that is J-SEC-001) and no operator verb.
- The **dataplane consumers** (sockops #26 / gateway / telemetry — themselves out
  of scope) → read via the `IdentityRead` port (US-WIM-02).
- **No operator CLI verb** — by design (System Constraints). #35's own observables
  are TEST-tier (the `issued_certificates` row written to the ObservationStore +
  `openssl verify` on the minted leaf). The **operator-visible** read surfaces —
  `overdrive alloc status --job <id>` rendering the row, and the deployed-SVID
  operator-verify flow — are **#215's** (O05/E03), blocked on #35.

**Pre-requisites (all satisfied today)**:
- `overdrive-core`: the `Ca` port trait (`issue_svid` / `issue_intermediate` /
  `trust_bundle`), `SvidMaterial`, `SvidRequest`, `TrustBundle`,
  `IssuedCertificateRow`, `SpiffeId` / `CertSerial` / `AllocationId` / `NodeId`
  newtypes. ✓ confirmed present (ADR-0063).
- `overdrive-control-plane`: `ca_issuance::issue_and_audit` (mint + bind audit
  row). ✓ The reconciler runtime (pure `reconcile()` + ViewStore) + action-shim
  executor pattern. ✓
- `overdrive-sim` / ObservationStore: `SimObservationStore` for the audit row +
  DST. ✓
- **Unbuilt (DESIGN handoff, NOT a blocker)**: the `Action::IssueSvid` /
  `DropSvid` variants, the `IdentityMgr` struct, the `IdentityRead` port, the
  `SvidLifecycle` reconciler, and `SpiffeId::for_allocation` derivation — these
  are the *feature*, pinned by DESIGN (see Handoff).

---

## Wave: DISCUSS / [REF] Definition of Ready (9-item gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | ✅ PASS | Each US has a Problem in security-engineer domain language; J-SEC-002 (liveness/availability of identity) frames the job. |
| 2 | User/persona with specific characteristics | ✅ PASS | `sam-platform-security-engineer` (reused; 10+ yr platform/security, threat-models, verifies with openssl), `related_jobs` extended to J-SEC-002 with the liveness lens. |
| 3 | 3+ domain examples with real data | ✅ PASS | Each US has 3 examples with real data (`spiffe://overdrive.local/job/payments/alloc/a1b2c3`, alloc `d4e5f6` of `orders`, `issued_certificates` fields, race-before-held, drop-on-stop). |
| 4 | UAT in Given/When/Then (3-7 scenarios) | ✅ PASS | 2–3 scenarios per story, 7 total across 3 stories; happy + race + drop + absent + audit-failure + convergence coverage. |
| 5 | AC derived from UAT | ✅ PASS | Each US's AC checklist maps to its scenarios. |
| 6 | Right-sized (1-3 days, 3-7 scenarios) | ✅ PASS | 3 slices, each ≤1 day (Slice 03 ~1.5d), 2–3 scenarios each (slice briefs). |
| 7 | Technical notes: constraints/dependencies | ✅ PASS | § System Constraints (reconciler purity, persist-inputs, state-layer hygiene, BTreeMap, port-trait, secret-hygiene, no CLI verb, single-node, gated #40 seam) + per-story Technical Notes. |
| 8 | Dependencies resolved or tracked | ✅ PASS | Pre-reqs all present (Ca port, issue_and_audit, runtime, sim stores confirmed in code); non-goals cite #40/#39/#26/#36/roadmap-4.7/Phase-5. The 5 design-sensitive surfaces are explicitly DESIGN's (not crafter-invented). |
| 9 | Outcome KPIs defined with measurable targets | ✅ PASS | K1–K5 with numeric targets + measurement method. |

**DoR verdict: PASS (9/9).** No item is blocked: the job is validated (DIVERGE),
the architecture is locked (DIVERGE Option 1), every deferral maps to an existing
issue/roadmap line, and the under-specified surfaces are explicitly handed to
DESIGN rather than guessed.

> **Gate note — elevator-pitch & slice-composition satisfied via the recorded
> foundation-feature exception (pass-2 F2, 2026-06-08).** The 9 DoR items above
> all PASS on their own evidence. The two *gate* checks that sit alongside DoR —
> the **elevator-pitch gate** ("a real user-invocable entry point — not internal
> state, not 'tests green'") and the **slice-composition hard gate** (no
> silent infra-only slice) — are **NOT** satisfied by a live operator surface for
> #35: under the foundation framing (Option A) #35 has no Phase-2
> operator-invocable observable, and every Phase-2 proof is TEST-tier (`openssl
> verify` the chain + ObservationStore readback of the `issued_certificates` row
> + the DST `assert_eventually!` convergence invariant). They are satisfied **via
> the deliberate, documented foundation-feature exception** recorded at
> § User Stories (Elevator-Pitch caveat → "Foundation-feature exception to the
> strict elevator-pitch gate") and § Wave Decisions (**D-WIM2-8**), mirroring
> built-in-ca (a security primitive proven by `openssl verify` at the test tier;
> its O05/E03 → #215). The operator surface is **#215** (blocked on #35); the
> consumer is **#26**. This note makes the DoR verdict honest: the gate items are
> met **by exception, not by a live surface**, and the evidence pointer is named.

---

## Wave: DISCUSS / [REF] Density & Triggers

**Resolved density**: `lean` + `ask-intelligent` (DISCUSS hard default; the
project `.nwave/des-config.json` rigor does not override the wave default).
Tier-1 `[REF]` sections emitted; no Tier-2 expansions auto-rendered.

**Triggers evaluated (`ask-intelligent` mode)** — one fired; reported here rather
than auto-expanded (per the lean discipline):

| Trigger | Fired? | Detail | Suggested expansion (NOT auto-applied) |
|---|---|---|---|
| AC ambiguity | No | ACs are crisp invariants (held-on-Running, dropped-on-Stop, audited, converges) — no reasonable-reader disagreement. | — |
| Cross-context complexity | No | One bounded context (workload identity); touches ~3 crates but the tech surface is the reconciler runtime + the already-shipped `Ca`/audit seams — not ≥3 distinct novel technologies. | — |
| Multi-stakeholder need | No | One persona (Sam). | — |
| Compliance / regulatory | **YES** | ACs reference audit (`issued_certificates`, no silent issuance), encryption-at-rest-adjacent secret hygiene (leaf-key drop), and the `openssl verify` chain proof. | `journey-deep-dive` (the held-identity lifecycle path with its load-bearing error/race states: race-before-held, drop-on-stop leak, restart, audit-write failure, issuance failure) |
| WS strategy = D (Configurable) | No | Brownfield walking skeleton (Slice 01); not env-switching. | — |

The orchestrator may request `--expand journey-deep-dive` (full error/race-path
map) if the downstream DESIGN wave would benefit. The SSOT journey
(`docs/product/journeys/hold-identity-for-the-running-set.yaml`, created this
wave) already covers those error/race paths — so the lean default is defensible
and expansion is optional.

---

## Wave: DISCUSS / [REF] Wave Decisions

### Key decisions

- **[D-WIM2-1]** Feature is right-sized as ONE feature, sliced into 3 thin
  vertical cuts (not split). Rationale: scope assessment — zero oversized signals;
  one coherent outcome (the running set holds live identity). (See § Scope
  Assessment.)
- **[D-WIM2-2]** Architecture is **LOCKED (DIVERGE Option 1)** — standalone
  `SvidLifecycle` reconciler + typed `Action::IssueSvid`/`DropSvid` + action-shim
  executor → shared `Arc<IdentityMgr>`; consumers read via sync getters behind an
  `IdentityRead` port; **the held set is the reconciler's `actual` and the View
  carries retry memory** (DESIGN rev 2 corrected the rev-1 "View persists issuance
  INPUTS" mechanism — ADR-0067 D1/D4/D8). This wave writes stories/slices/ACs
  *against* it and does not re-open it (DIVERGE B1 resolved — 4.7
  unified-`IdentityMgr` firm; Option 2 does not re-open).
- **[D-WIM2-3]** Reconciler purity is a **CORRECTNESS constraint** (DIVERGE
  D-WIM-3): CA I/O is in the action-shim executor, never in `reconcile()`. Any
  shape putting `Ca` I/O in `reconcile()` is excluded structurally.
- **[D-WIM2-4]** Job is **J-SEC-002**, validated in DIVERGE — JTBD NOT re-run.
  Every story carries `job_id: J-SEC-002` (N:1). (See `diverge/job-analysis.md`.)
- **[D-WIM2-5]** **Rotation deferred to #40 with a gated pre-wired seam** (DIVERGE
  D-WIM-8): the near-expiry branch + View input are present (Slice 03) but the
  `Action::StartWorkflow(cert_rotation)` emit is dormant until #40 registers the
  kind — a clean no-op, never `UnknownWorkflow`-per-tick. NO throwaway sync-rotate
  path.
- **[D-WIM2-6]** **No operator CLI verb; #35 is a FOUNDATION feature (F2,
  2026-06-08 revision)** — hold/read/drop is internal mechanism. #35's own
  observables are TEST-tier (the `issued_certificates` row written to the
  ObservationStore + `openssl verify` on the minted leaf, built-in-ca's
  `rcgen_ca_chain_verify` shape). The **operator** surfaces — `alloc status`
  rendering the row + the deployed-SVID operator-verify flow — are **#215's**
  (O05/E03), **blocked on #35** (the current `AllocStatusResponse` has no
  issued-cert field; no consumer #26 exists yet). Per CLAUDE.md the workload verb
  is `overdrive deploy`, never `job submit`.
- **[D-WIM2-7]** **Single-node (Phase 2)** — one node's running set; multi-node
  owned by existing **#36 [2.14]**. No new issue.
- **[D-WIM2-8]** **Strict elevator-pitch gate met via a documented
  foundation-feature exception (pass-2 F2, 2026-06-08)** — under the foundation
  framing (Option A), #35 has **no Phase-2 operator-invocable observable**; all
  three stories' Phase-2 proofs are **TEST-tier** (`openssl verify` the chain in a
  gated `integration-tests` run + ObservationStore readback of the
  `issued_certificates` row + the DST `assert_eventually!` convergence invariant).
  The strict gate's "real user-invocable entry point — not internal state, not
  'tests green'" requirement is therefore satisfied **by a deliberate, recorded
  exception mirroring built-in-ca** (a security primitive proven by `openssl
  verify` at the test tier; its O05/E03 finalized `pending` → #215), **NOT** by a
  live operator surface and **NOT** by an invented CLI verb (CLAUDE.md §
  "Implement to the design"). Justification: the operator surface is **#215**
  (the `alloc status` render of `issued_certificates`, **blocked on #35**) and the
  consumer is **#26** (sockops/kTLS) — both existing, tracked issues; neither is
  in #35's Phase-2 scope. The exception is recorded in three places (the
  Elevator-Pitch caveat, this decision, and the DoR validation).

### Requirements summary

- Primary job: J-SEC-002 — every running workload holds a live, readable,
  chain-verifiable identity; a stopped workload holds none.
- Walking skeleton: Slice 01 (issue → hold → drop → audit → converge). Release 1:
  Slice 02 (`IdentityRead` port + consumer read surface). Release 2: Slice 03
  (restart-idempotence + gated #40 rotation seam).
- Feature type: cross-cutting security primitive (brownfield over the shipped
  `Ca` port + `issue_and_audit`).

### Constraints established

See § System Constraints (reconciler purity; persist inputs not derived
`expires_at`; state-layer hygiene — held material in-process, audit observation,
View reconciler-memory; `BTreeMap` not `HashMap`; `IdentityRead` port-trait
discipline; secret hygiene on drop; no CLI verb; single-node; gated #40 seam).

### Upstream changes

- **None to DIVERGE** — the job (J-SEC-002), the locked architecture (Option 1),
  and the resolved blockers (B1 4.7-firm, B2 pre-wire-the-seam) are consumed
  as-is. JTBD was NOT re-run.
- SSOT additions/edits (this wave): `docs/product/journeys/hold-identity-for-the-running-set.yaml`
  (NEW), `docs/product/personas/sam-platform-security-engineer.yaml`
  (`related_jobs` += J-SEC-002 + J-SEC-002 liveness lens). `docs/product/jobs.yaml`
  already carries J-SEC-002 (added in DIVERGE) — not re-added.

---

## Wave: DISCUSS / [REF] Open Questions / BLOCKERS for the orchestrator

> Surfaced per the project rule: a subagent cannot create GH issues or message
> the user. These are relayed for the orchestrator to put to the user.

**No blockers.** The job is validated (DIVERGE J-SEC-002), the architecture is
locked (DIVERGE Option 1, B1+B2 resolved), and every deferral maps to an EXISTING
issue or roadmap line — **#40** (rotation, depends on **#39** workflow primitive),
**#26** (sockops/kTLS consumer), **#36 [2.14]** (multi-node), **roadmap 4.7**
(ACME unification), **Phase 5** (revocation), **#215** (the verification surface
O05/E03 the observable ACs satisfy). **No new GH issue is required; no invented
issue numbers; no hand-wavy forward pointers.**

**DESIGN handoff items (NOT blockers — design-sensitive surfaces DESIGN must pin,
per `recommendation.md` § "What DESIGN must still pin" + CLAUDE.md § "Implement
to the design")** — named here so no crafter invents them.

> **ALL FIVE ARE NOW RESOLVED in ADR-0067 (rev 2)** — #1 → D2/D5 (`for_allocation`
> is the canonical extraction of two existing private helpers); #2 → D7; #3 → D4
> (+ `held_snapshot` for the held-set-as-`actual`); **#4 → D8 — RESOLVED as RETRY
> MEMORY (`attempts`, `last_failure_seen_at`), NOT issuance-success inputs** (the
> rev-1 "issued-at / validity-window" framing was the High-1 finding: `serial` is a
> post-dispatch output, success lives in `issued_certificates`, held-ness is
> `actual`); #5 → D6 (HYDRATED). Plus the rev-2 additions: the held-set-as-`actual`
> (D1/D4) and the enqueue/handoff trigger (D5b). Implement ADR-0067 rev 2; the list
> below is the DISCUSS-era statement of what was open.

1. **`Action::IssueSvid` / `Action::DropSvid` exact field set** (e.g. `{ alloc_id,
   spiffe_id, node_id, correlation }`) — incl. **who builds the `SpiffeId`** for
   the allocation (the `for_allocation(job, alloc)` derivation is unbuilt;
   `SpiffeId::new` only validates).
2. **`IdentityRead` port-trait signatures** + the `SimIdentityRead` double
   (`svid_for(&AllocationId) -> Option<SvidMaterial>` + `current_bundle() ->
   TrustBundle` recommended).
3. **`IdentityMgr` concurrency primitive** (`parking_lot::RwLock<BTreeMap<…>>`
   recommended; `BTreeMap` mandatory because the held map is iterated by the
   `assert_eventually!` invariant).
4. **The View's retry-memory shape** — `IssueRetry { attempts,
   last_failure_seen_at }`, never issuance success facts and never a derived
   `expires_at` / `next_renewal_at`.
5. **Trust-bundle currency mechanism** — resolved as HYDRATED into `IdentityMgr`
   at boot and refreshed through the executor / #40 seam.

---

## Wave: DISCUSS / [REF] SSOT Artifacts Produced

| Artifact | Path | Change |
|---|---|---|
| Job register | `docs/product/jobs.yaml` | J-SEC-002 already present (added in DIVERGE) — NOT re-added this wave |
| Persona | `docs/product/personas/sam-platform-security-engineer.yaml` | EDIT — `related_jobs` += J-SEC-002 + a J-SEC-002 liveness lens |
| Journey | `docs/product/journeys/hold-identity-for-the-running-set.yaml` | NEW (product-level summary; maps J-SEC-002; relates_to built-in-ca + this feature) |
| Slice briefs | `docs/feature/workload-identity-manager/slices/slice-0{1..3}-*.md` | NEW (3 briefs) |
| Feature delta | `docs/feature/workload-identity-manager/feature-delta.md` | THIS file |

---

## Wave: DISCUSS / [REF] Handoff

- **To DESIGN (nw-solution-architect)**: full artifact set — this feature-delta
  (3 stories + ACs + story map + system constraints + KPIs) + the 3 slice briefs
  + the SSOT job (J-SEC-002, validated) / persona / journey + the DIVERGE
  `recommendation.md`. **Pin the 5 design-sensitive surfaces** (Open Questions §,
  items 1–5): the `Action` field set + `SpiffeId` derivation, the `IdentityRead`
  signatures + sim double, the `IdentityMgr` concurrency primitive, the View
  input shape, and the trust-bundle currency mechanism. **Carry the gated #40
  rotation-seam caveat** (a committed `StartWorkflow` for an unregistered kind
  raises `UnknownWorkflow` per tick — keep the emit dormant until #40 registers
  `cert_rotation`).
- **To DEVOPS (nw-platform-architect)**: the Outcome KPIs (K1–K5) — instrument
  the running-set-holds-valid-SVID convergence rate (North Star), the
  no-leak-on-stop guardrail, the restart-idempotence guardrail, the
  no-silent-issuance guardrail, and DST determinism.
- **To DISTILL (nw-acceptance-designer)**: the UAT scenarios (embedded above — no
  standalone `.feature` file per `.claude/rules/testing.md`), the load-bearing
  error/race paths (SSOT journey: bounded convergence window / fail-closed at the
  consumer, drop-on-stop leak, restart idempotence, audit-write failure, issuance
  failure), and the KPIs. The test surface crosses Tier-1 (DST: the
  `assert_eventually!` convergence invariant, `BTreeMap` determinism, serials via
  `Entropy`, `SimObservationStore` audit, `SimIdentityRead` equivalence) and
  host-adapter acceptance tests (the `issued_certificates` row read back via the
  ObservationStore + `openssl verify` on the minted leaf at the TEST tier — gated
  behind `integration-tests`, run via Lima; built-in-ca's `rcgen_ca_chain_verify`
  shape). The **operator** `alloc status` render of the row + the deployed-SVID
  operator-verify flow are **#215's** O05/E03 EDD expectations (`pending`, blocked
  on #35), NOT #35's own surface.

---

## Wave: DISCUSS / [REF] Review Revisions (2026-06-08)

Two review rounds; both resolutions user-decided and applied surgically (the rest
of the artifact set — the J-SEC-002 separation, the locked Option-1 handoff, the
#40 `UnknownWorkflow` gated-seam caveat, the KPIs, the DESIGN handoff items — was
found sound and left untouched). Recorded here for an honest revision trail.

### Round 1

A `[REF]` review of this DISCUSS artifact set raised 3 findings; the user decided
the resolutions and they were applied surgically.

| # | Severity | Finding | Resolution |
|---|---|---|---|
| **F1** | Blocking (race semantics) | The JTBD one-liner prose overstated the promise as *"no race where a workload serves before its identity is held"* — contradicting O1 ("minimize likelihood"), the bounded-window domain example, the UAT, and the ACs, all of which correctly model a **bounded one-tick convergence window**. | **Reworded** to the honest **convergence + fail-closed** framing: the window between Running and identity-held is bounded to one reconcile tick and closed by convergence; a workload with no held SVID cannot present identity, so the mTLS consumer (#26) **fails closed** — the bounded window is therefore not an exposure (and in Phase 2 there is no consumer at all). Added a fail-closed note to System Constraints cross-referencing **#26** as the enforcer. The same correction was applied to the journey YAML's "no race"/"no window…serves" phrasing. No registration-gating AC was added (that over-reaches into BackendDiscovery/#26 scope; fail-closed covers it). |
| **F2** | High (observable surface) | The feature-delta + Slice 01 over-claimed `overdrive alloc status --job <id>` showing the `issued_certificates` row as #35's own AC / Elevator-Pitch "After". That render is **#215's** deliverable (compose built-in CA into the operator surface) and **#215 is blocked on #35**: the current `AllocStatusResponse` has no issued-cert field, the renderer shows no certs, and there is no SVID consumer (#26) yet. | **Foundation framing (user chose Option A).** #35 is re-grounded as a FOUNDATION feature whose own observable proof is the **built-in-CA shape**: (a) the `issued_certificates` row is WRITTEN per issuance (ObservationStore-testable in a gated integration test), (b) `openssl verify` on the minted leaf chain → exit 0 at the **TEST tier** (gated `integration-tests`, built-in-ca's `rcgen_ca_chain_verify` shape), (c) the DST `assert_eventually!` convergence invariant. The operator `alloc status` render + the deployed-SVID operator-verify flow are deferred to **#215** as its O05/E03 EDD expectations (`pending`, blocked on #35), NOT #35's own AC. Elevator-pitch "After" lines now use `openssl verify`-on-the-chain as the real executable entry point (built-in-ca's "Elevator-Pitch After caveat" precedent), never `alloc status`-shows-certs and never an invented verb. |
| **F3** | High (Slice 02 consumer AC) | US-WIM-02 / Slice 02 required *"Consumers take `Arc<dyn IdentityRead>`"* while declaring those consumers OUT of scope — untestable + scope-conflicting. | **Contract proof, not production wiring.** The AC is reworded so the `IdentityRead` port + `SimIdentityRead` double + a **test consumer/fixture** prove the read contract (sync getters, no re-issue per read, explicit-absent), and the required-ctor-param port-trait discipline is a property the **test consumer** demonstrates. **Production** consumer wiring (sockops #26 / gateway / telemetry) is deferred to those features, not an AC here. |

### Round 2

A second `[REF]` review raised 3 further findings; the user/orchestrator decided
the resolutions and they were applied surgically.

| # | Severity | Finding | Resolution |
|---|---|---|---|
| **F1** | (Round-2) | A residual over-promise in the Round-1 outputs (the bounded-window/fail-closed wording in `jobs.yaml`'s J-SEC-002 emotional dimension + `job-analysis.md:149`). | **Fixed by the orchestrator** ahead of this pass — the `jobs.yaml` emotional dimension and `job-analysis.md:149` now carry the honest bounded-window/fail-closed framing. Not re-touched by this pass (out of the surgical scope: the feature-delta + slice briefs). |
| **F2** | Blocking (elevator-pitch gate honesty) | Round-1 F2 re-grounded #35 as a foundation feature with a test-tier observable, but the strict nWave elevator-pitch gate ("a real user-invocable entry point — not internal state, not 'tests green'") is **not** strictly satisfied: under the foundation framing **none** of #35's three stories has a Phase-2 *operator-invocable* observable. Round-1 reported the gate as a bare PASS via the built-in-ca `openssl verify` precedent — but that precedent is **test-tier**, so the gate is met only **via a documented exception**, which Round-1 left implied rather than recorded. | **Foundation-feature exception recorded explicitly in three places** (do NOT silently pass; do NOT invent a verb — CLAUDE.md § "Implement to the design"): (1) § User Stories — the Elevator-Pitch caveat now carries a "**Foundation-feature exception to the strict elevator-pitch gate**" paragraph stating plainly that #35's Phase-2 verification is test-tier (`openssl verify` the chain + ObservationStore row readback + DST convergence), the operator-invocable surface is **#215** (render, blocked on #35) and the consumer **#26** (sockops), and the gate is met **by a deliberate, documented foundation-feature exception mirroring built-in-ca, NOT a live operator surface**; (2) § Wave Decisions — new **D-WIM2-8** recording the exception + justification (#215 operator surface, #26 consumer, both tracked; built-in-ca precedent); (3) § DoR validation — a gate note stating the elevator-pitch / slice-composition gates are satisfied **via the recorded exception**, with the evidence pointer, so the DoR verdict is honest. |
| **F3** | High (trust-bundle currency AC over-constrains a DESIGN-open choice) | US-WIM-02's AC + Domain Example 1 + Slice-02's AC said a read returns `SvidMaterial` + `TrustBundle` **"with NO call to the `Ca` port"** — which FORBIDS the pull-on-demand-via-`Ca::trust_bundle()` option that `recommendation.md:134` leaves OPEN to DESIGN (Open-Questions #5). The AC's real intent is O3 (**no re-issue on read**), not "no `Ca` call at all." | **Narrowed** every "no call to the `Ca` port" phrasing → **"without re-issuing — no `issue_svid` on the read path; the SVID is served from the held map."** Added an explicit clause that the **trust-bundle currency mechanism (pull-on-demand via `Ca::trust_bundle()` vs hydrated into `IdentityMgr`) stays DESIGN's call (Open-Questions #5)**. The O3 "no re-issue per read" guarantee is intact; only what is *permitted* is widened (a cheap bundle pull is allowed; re-issuing the SVID is not). Applied to: feature-delta US-WIM-02 Domain Example 1 + its Acceptance Criteria, and Slice-02's AC. |

**Re-validated gates after Round 2** (full verdicts in the orchestrator return):
- **Elevator-pitch gate — MET VIA THE DOCUMENTED FOUNDATION-FEATURE EXCEPTION**
  (NOT a bare PASS). #35 does not strictly satisfy the "real user-invocable entry
  point" requirement — under the foundation framing none of its three stories has
  a Phase-2 operator-invocable observable; every Phase-2 proof is test-tier
  (`openssl verify` exit 0 on the chain + ObservationStore row readback + the DST
  `assert_eventually!` convergence invariant). The gate is met **by the
  deliberate, documented foundation-feature exception** (recorded at § User
  Stories, § Wave Decisions D-WIM2-8, and the § DoR gate note), mirroring
  built-in-ca — not by a live operator surface and not by an invented verb. The
  operator surface is **#215** (blocked on #35); the consumer is **#26**.
- **Slice-composition hard gate — PASS under the recorded exception.** Every slice's
  value story rests on the foundation-feature (test-tier) value made releasable by
  the recorded exception; none is a silent infra-only failure and none is
  relabelled `@infrastructure` (Slice 01 issue/hold/drop/audit/converge; Slice 02
  the read port + sim double + test consumer; Slice 03 restart-idempotence + the
  gated #40 seam — each carries a user-visible value story).
- **DoR — 9/9 PASS**, with the elevator-pitch / slice-composition *gate* items
  noted as satisfied **via the recorded foundation-feature exception** (§ DoR gate
  note); the 9 DoR items themselves pass on their own evidence and no item
  regressed (F3's narrowing keeps item-3 examples / item-4 UAT intact and widens
  only the permitted DESIGN surface).

---

# DESIGN sections (Wave 3 of 6)

**Wave**: DESIGN · **Agent**: Morgan (nw-solution-architect) · **Mode**: GUIDE
(PASS-1 menu; every design-sensitive surface user-pinned) · **Date**: 2026-06-08

The architecture was **LOCKED in DIVERGE (Option 1)** and the DISCUSS wave wrote
stories/slices/ACs against it. This DESIGN wave **pins the 5 design-sensitive
surfaces** DISCUSS handed off (Open-Questions #1–#5) and the **found wiring**
(`AppState` / shim). All decisions are recorded in **ADR-0067** and integrated
into the SSOT (`docs/product/architecture/brief.md` § "workload-identity-manager
extension", `c4-diagrams.md` § "Workload Identity Manager" — L1+L2+L3 Mermaid).
The DISCUSS sections above are untouched.

---

## Wave: DESIGN / [REF] DDD Decisions

Each is a locked design decision with one-line rationale. The full record is
ADR-0067 (D-points) + brief.md.

- **[DDD-1]** One bounded context — **workload identity (holder)**, spanning
  `overdrive-core` + `overdrive-control-plane` + `overdrive-sim`. *Rationale*:
  the holder/reader/dropper is one coherent capability over the shipped `Ca`
  port (#28); not split. (DISCUSS Scope Assessment; ADR-0067 Context.)
- **[DDD-2]** A **standalone `SvidLifecycle` reconciler** owns identity as its
  own desired-vs-actual convergence target (NOT folded into `WorkloadLifecycle`),
  converging **`desired` = running allocs** vs **`actual` = the `IdentityMgr` held
  set** (rev 2: the held-set-as-`actual`). `running ∧ ¬held → IssueSvid` (incl.
  restart re-issue), `¬running ∧ held → DropSvid`, `running ∧ held(valid) → Noop`.
  *Rationale*: identity availability has its own North-Star invariant (O1);
  coupling entangles two convergence relations; the held set as `actual` makes
  restart recovery fall out for free (`actual = ∅` on boot → re-issue all running).
  (ADR-0067 D1 / D4; A1.)
- **[DDD-3]** **Reconciler purity is a CORRECTNESS constraint** — CA I/O lives in
  the action-shim executor, never in `reconcile()`. *Rationale*: DST replay +
  dst-lint require a pure `reconcile(desired, actual, view, tick) → (Vec<Action>,
  View)`. (DISCUSS D-WIM-3; ADR-0067 D1/D3.)
- **[DDD-4]** The **pure reconciler builds the `SpiffeId`** (via the new
  infallible `SpiffeId::for_allocation`) and passes it in `Action::IssueSvid`.
  `for_allocation` is the **canonical extraction** (rev 2: consolidation, not
  net-new) of two existing private helpers — `mint_alloc_identity`
  (`backend_discovery_bridge.rs:424`) + `mint_identity`
  (`workload_lifecycle.rs:808`) — which already derive the identical string;
  DELIVER migrates both call sites (single-cut). *Rationale*: identity *derivation*
  is pure and belongs in `reconcile()`; identity *issuance* (CA I/O) is the
  executor's; one canonical derivation prevents a third implementation. (ADR-0067
  D2/D5.)
- **[DDD-5]** **Two typed Actions** — `IssueSvid { alloc_id, spiffe_id, node_id,
  correlation }` / `DropSvid { alloc_id, correlation }` — additive on a plain
  enum; **`node_id` KEPT** (self-describing, #36-forward-compat). *Rationale*:
  the issuance request names the node it is issued on; the executor MAY read
  `AppState.node_id` in Phase 2 but the action stays the SSOT. (ADR-0067 D2; A6.)
- **[DDD-6]** **`IdentityRead` is a sync, owned-clone port** with 5
  behaviour-pinning rustdoc clauses + a `SimIdentityRead` double + a DST
  equivalence test. *Rationale*: in-process low-latency read surface (O3); the
  contract is enforced, not conventional. (ADR-0067 D7.)
- **[DDD-7]** **`IdentityMgr` holds the set in `parking_lot::RwLock<BTreeMap>`**
  + an `Option<TrustBundle>`, and exposes **`held_snapshot()`** — the sync
  `actual`-projection reader the runtime folds into `SvidLifecycle`'s `actual`
  (mirroring `WorkflowEngine::live_instances()`). *Rationale*: sync critical
  section (guard never crosses `.await`); `BTreeMap` mandatory because the
  invariant AND `held_snapshot` iterate it (K5); `held_snapshot` is sync so the
  `hydrate_actual` arm needs no `.await`. (ADR-0067 D4; A7/A8.)
- **[DDD-8]** **Trust-bundle currency is HYDRATED** into `IdentityMgr` (set at
  boot, refreshed by the executor, pushed by #40 via `set_bundle`). *Rationale*:
  zero CA I/O on the read hot path (O3); `set_bundle` is #40's push seam.
  (ADR-0067 D6; A4.)
- **[DDD-9]** The **View is RETRY MEMORY** (`IssueRetry{ attempts,
  last_failure_seen_at }`, 6 derive bounds, manual `Default`) — request inputs so
  a *failed* `IssueSvid` backs off — NOT issuance success facts. *Rationale* (rev
  2): `serial` is a post-dispatch executor output the pure reconciler cannot know
  AND the runtime persists `next_view` BEFORE dispatch
  (`reconciler_runtime.rs:1222-1226` vs `:1324`); held-ness is `actual` and the
  success fact lives in `issued_certificates`. NO derived `expires_at` —
  near-expiry reads the held cert's real `not_after` off `actual`. (ADR-0067 D8;
  A3.)
- **[DDD-10]** The **#40 rotation seam is pre-wired but EMIT-GATED**
  (`ROTATION_ENABLED = false` / absent emit). *Rationale*: production wires an
  empty-registry engine; a naïve `StartWorkflow(cert_rotation)` emit raises
  `UnknownWorkflow` every tick. Near-expiry reads the held cert's `not_after`
  from `actual` when #40 flips the gate — the View is **retry memory** and does
  NOT carry `issued_at`. NO throwaway sync-rotate path. (ADR-0067 D8; A5.)
- **[DDD-11]** **`AppState` is extended** with `ca: Arc<dyn Ca>` + `identity:
  Arc<IdentityMgr>` (the found wiring), threaded into `dispatch`/`dispatch_single`;
  production composes `Arc<dyn Ca>` from `ca_boot` (lib.rs:50). *Rationale*: the
  executor needs both to do CA I/O + hold; additive, existing consumers
  untouched. (ADR-0067 D3.)
- **[DDD-12]** **`SvidLifecycle` is level-triggered via `Action::EnqueueEvaluation`**
  (rev 2 — the missing trigger). `WorkloadLifecycle::reconcile`
  (`workload_lifecycle.rs:181` alloc-mutating block) emits a third
  `EnqueueEvaluation` (ungated by kind) keyed `job/<workload_id>`; the exit observer
  (`exit_observer.rs:230-256`) submits a sibling `Evaluation` on an observed exit;
  broker LWW dedups. *Rationale*: a reconciler that is never enqueued never ticks —
  rev 1 left this implicit, so the reconciler would build but not run at the moments
  the feature depends on. DELIVER regression: Running AND Stopped transitions tick
  `SvidLifecycle` with no manual broker poke. (ADR-0067 D5b.)

---

## Wave: DESIGN / [REF] Component Decomposition

| Component | Path | Crate (class) | Change |
|---|---|---|---|
| `SvidLifecycle` reconciler (converges `desired`=running vs `actual`=held set) | `crates/overdrive-core/src/reconcilers/svid_lifecycle.rs` | `overdrive-core` (core) | **CREATE NEW** |
| `SvidLifecycleView` + `IssueRetry` (retry memory) | `crates/overdrive-core/src/reconcilers/svid_lifecycle.rs` | `overdrive-core` (core) | **CREATE NEW** |
| `hydrate_actual` `SvidLifecycle` arm (held-set projection via `held_snapshot`) | `crates/overdrive-control-plane/src/reconciler_runtime.rs` (`:2190`) | `overdrive-control-plane` (adapter-host) | **EXTEND** (one new match arm) |
| `WorkloadLifecycle::reconcile` enqueue handoff (third `EnqueueEvaluation`) + exit-observer sibling submit | `crates/overdrive-core/src/reconcilers/workload_lifecycle.rs` (`:181`) + `crates/overdrive-control-plane/src/worker/exit_observer.rs` (`:230-256`) | `overdrive-core` (core) + `overdrive-control-plane` (adapter-host) | **EXTEND** (additive emissions) |
| `Action::IssueSvid` / `Action::DropSvid` (+3 dispatch-enum variants) | `crates/overdrive-core/src/reconcilers/mod.rs` | `overdrive-core` (core) | **EXTEND** (additive: +2 variants + dispatch triple) |
| `SpiffeId::for_allocation` | `crates/overdrive-core/src/id.rs` (`impl SpiffeId`) | `overdrive-core` (core) | **EXTEND** (impl) |
| `IdentityRead` port trait | `crates/overdrive-core/src/traits/identity_read.rs` | `overdrive-core` (core) | **CREATE NEW** |
| `IdentityMgr` + `IdentityState` | `crates/overdrive-control-plane/src/identity_mgr.rs` | `overdrive-control-plane` (adapter-host) | **CREATE NEW** |
| `action_shim/issue_svid.rs` executor (+2 `dispatch_single` arms) | `crates/overdrive-control-plane/src/action_shim/issue_svid.rs` (+ `mod.rs`) | `overdrive-control-plane` (adapter-host) | **CREATE NEW** + EXTEND dispatch |
| `AppState` (`+ca`, `+identity`) + shim signature | `crates/overdrive-control-plane/src/lib.rs` | `overdrive-control-plane` (adapter-host) | **EXTEND** |
| `SimIdentityRead` | `crates/overdrive-sim/src/adapters/identity_read.rs` | `overdrive-sim` (adapter-sim) | **CREATE NEW** |
| `identity_read_equivalence` DST test | `crates/overdrive-control-plane/tests/integration/identity_read_equivalence.rs` | `overdrive-control-plane` (test) | **CREATE NEW** |

Paradigm: **OOP / ports-and-adapters** (project default — use `@nw-software-crafter`).
Architecture-rule enforcement: **dst-lint** keeps `reconcile()` pure (no `.await`
/ no CA handle on the `core` compile path); the `identity_read_equivalence` DST
test is the `IdentityRead` contract enforcement; the
`assert_eventually!("running allocs hold a valid SVID")` invariant is the
North-Star convergence gate.

---

## Wave: DESIGN / [REF] Driving Ports

Inbound surfaces that trigger identity-lifecycle behaviour:

- **Allocation lifecycle** (the existing reconciler runtime observing alloc
  `Running ↔ Stopped`) → `SvidLifecycle` → `Action::IssueSvid` / `DropSvid`.
  The **only** trigger.
- **Dataplane consumers** (sockops #26 / gateway / telemetry — themselves out of
  scope) → read via the `IdentityRead` port (`svid_for` / `current_bundle`).
- **No operator CLI verb** — by design. #35's observables are TEST-tier; the
  operator-visible `alloc status` render is **#215's** (blocked on #35). Per
  CLAUDE.md the workload verb is `overdrive deploy <SPEC>`, never `job submit`.

---

## Wave: DESIGN / [REF] Driven Ports & Adapters

Outbound side-effects (the executor reaches out; the reconciler does not):

| Driven port | Production adapter | Sim adapter | What crosses |
|---|---|---|---|
| `Ca` (REUSE, #28) | `RcgenCa` (`overdrive-host`) | `SimCa` (`overdrive-sim`) | `issue_and_audit` (mint leaf + write audit row + refuse-on-audit-failure); `trust_bundle()` (boot hydrate + per-issuance refresh) |
| `ObservationStore` (REUSE) | `LocalObservationStore` | `SimObservationStore` | `issued_certificates` row — written **inside** `issue_and_audit` (ADR-0063 D6) |
| `IdentityRead` (NEW, this feature) | `IdentityMgr` (`overdrive-control-plane`) | `SimIdentityRead` (`overdrive-sim`) | `svid_for(&AllocationId)` / `current_bundle()` — sync, owned clones |
| `ViewStore` (REUSE, runtime-owned) | `RedbViewStore` | `SimViewStore` | `SvidLifecycleView` = `IssueRetry` retry memory (`attempts`, `last_failure_seen_at`) — runtime persists end-to-end |

Both `Ca` and `IdentityMgr` are **required constructor parameters** on the
consuming types — never defaulted (`.claude/rules/development.md` § "Port-trait
dependencies"). The `IdentityRead` consumer/test-fixture takes `Arc<dyn
IdentityRead>` as a required ctor param (the port-trait discipline proven by the
Slice-02 test consumer).

**External-integration / contract-test note**: there are **no third-party or
cross-team external API integrations** in this feature — every driven port is an
in-process Overdrive port (`Ca`, `ObservationStore`, `IdentityRead`, `ViewStore`)
backed by host/sim adapters and exercised by DST equivalence tests. No
consumer-driven contract testing (Pact etc.) is warranted; the equivalence tests
(`identity_read_equivalence`, the reused `ca_equivalence`) are the cross-adapter
contract guard.

---

## Wave: DESIGN / [REF] Technology Choices

All reuse; nothing new enters the dependency graph. OSS-first; no proprietary.

| Choice | License | Rationale |
|---|---|---|
| `parking_lot::RwLock` (`IdentityMgr`) | MIT/Apache-2.0 | Project default for sync critical sections — the held-map mutate/clone-out never crosses `.await`; faster uncontended path, no poisoning. Already in-graph. (ADR-0067 D4; A7.) |
| `std::collections::BTreeMap` (held map + View map) | std | **Mandatory** — the held map is iterated by the `assert_eventually!` North-Star invariant; deterministic iteration across DST seeds (K5). NOT `HashMap`. (`.claude/rules/development.md` § "Ordered-collection choice"; A8.) |
| `serde` + `ciborium` (View, via the runtime ViewStore) | MIT/Apache-2.0 | The `SvidLifecycleView` is CBOR-encoded reconciler memory (ADR-0035); additive fields ride `#[serde(default)]`. Reused, not introduced. |
| `Ca` port + `ca_issuance::issue_and_audit` (REUSE, #28) | (workspace) | Mints + audits + refuses-on-audit-failure wholesale (O5 by reuse). `RcgenCa`/`SimCa`/`ring` carry forward from ADR-0063 unchanged. |
| `async_trait` (executor is async at the shim boundary) | MIT/Apache-2.0 | The executor `.await`s `issue_and_audit` at the ADR-0023 sanctioned shim boundary; already a workspace dep. The reconciler stays sync. |

No proprietary technology. No new crate is added — the feature is a
*composition* of shipped primitives behind a new reconciler + port trait.

---

## Wave: DESIGN / [REF] Decisions Table

| DDD-N | Decision | ADR-0067 ref |
|---|---|---|
| DDD-1 | One bounded context — workload identity (holder), ~3 crates | Context |
| DDD-2 | Standalone `SvidLifecycle` reconciler; `desired`=running vs `actual`=held set (held-set-as-`actual`) | D1 / D4 / A1 |
| DDD-3 | Reconciler purity = CORRECTNESS constraint (CA I/O in executor) | D1 / D3 |
| DDD-4 | Pure reconciler builds the `SpiffeId` via `for_allocation` (extraction of 2 private helpers) | D2 / D5 |
| DDD-5 | Two typed Actions; `node_id` KEPT on `IssueSvid` | D2 / A6 |
| DDD-6 | `IdentityRead` sync owned-clone port; 5 clauses; sim double + equivalence test | D7 |
| DDD-7 | `IdentityMgr` = `parking_lot::RwLock<BTreeMap>` + `Option<TrustBundle>` + `held_snapshot()` | D4 / A7 / A8 |
| DDD-8 | Trust-bundle currency HYDRATED into `IdentityMgr` | D6 / A4 |
| DDD-9 | View = RETRY MEMORY (`attempts`, `last_failure_seen_at`); NO success facts; 6 bounds; manual `Default` | D8 / A3 |
| DDD-10 | #40 rotation seam pre-wired but EMIT-GATED; keys off `actual.not_after` | D8 / A5 |
| DDD-11 | `AppState` extended (`+ca`, `+identity`); composed from `ca_boot` | D3 |
| DDD-12 | `SvidLifecycle` level-triggered via `Action::EnqueueEvaluation` (WorkloadLifecycle + exit observer) | D5b |

---

## Wave: DESIGN / [REF] Reuse Analysis

The HARD-GATE reuse table from PASS-1. Every overlapping component classified
EXTEND / REUSE / CREATE NEW.

| Existing Component | File | Overlap | Decision | Justification |
|---|---|---|---|---|
| `Ca` port + `ca_issuance::issue_and_audit` | `crates/overdrive-core/src/traits/ca.rs`, `crates/overdrive-control-plane/src/ca_issuance.rs` | Mint + audit + refuse-on-audit-failure | **REUSE AS-IS** | Executor *calls* `issue_and_audit` wholesale; O5 served by reuse. No signature change. |
| `SvidMaterial` / `TrustBundle` / `IssuedCertificateRow` | `crates/overdrive-core/src/traits/ca.rs` | The held material, the read return, the audit row | **REUSE AS-IS** | All three exist (ADR-0063, incl. D9 node-held `leaf_key`). No change. |
| Reconciler runtime (pure `reconcile()` + ViewStore) | `.claude/rules/development.md` § "Reconciler I/O"; ADR-0035/0036 | A new reconciler on the runtime | **REUSE AS-IS** | `SvidLifecycle` is one more `Reconciler`; runtime owns View persistence. No runtime change. |
| Action-shim executor pattern (`ServiceMapHydrator` → `DataplaneUpdateService` → executor) | `crates/overdrive-control-plane/src/action_shim/dataplane_update_service.rs` | The issue/drop executor shape | **REUSE (pattern), new executor** | `action_shim/issue_svid.rs` mirrors it exactly; no shim *mechanism* change. |
| `Action` enum | `crates/overdrive-core/src/reconcilers/mod.rs` | The reconciler→executor trigger | **EXTEND** (additive) | +2 plain-enum variants + 2 `dispatch_single` arms + the 3 dispatch-enum variants — same shape `DataplaneUpdateService`/`StartWorkflow` were added. No existing variant changes. |
| `AppState` + shim signature | `crates/overdrive-control-plane/src/lib.rs` | The executor's CA + holder access | **EXTEND** | Gains `ca: Arc<dyn Ca>` + `identity: Arc<IdentityMgr>`, threaded into the 2 new arms. Additive; existing consumers untouched; `Arc<dyn Ca>` is the *same* adapter `ca_boot` builds (lib.rs:50). |
| `SpiffeId` + the 2 private derivation helpers | `crates/overdrive-core/src/id.rs`; `…/reconcilers/backend_discovery_bridge.rs:424` (`mint_alloc_identity`); `…/reconcilers/workload_lifecycle.rs:808` (`mint_identity`) | Allocation → SPIFFE-URI derivation (already duplicated twice) | **EXTEND (impl) — CONSOLIDATION** | Add infallible `for_allocation(&WorkloadId, &AllocationId) -> Self` as the **canonical extraction** of the two existing private helpers (both build the identical `spiffe://overdrive.local/job/<wl>/alloc/<id>` via `SpiffeId::new(&raw).expect(…)`). Validates via `new` + `unwrap_or_else(\|\| unreachable!(…))`. Type/`new`/`Display`/`FromStr`/serde unchanged. **DELIVER migrates both call sites** (single-cut — prevents a third implementation). |
| `WorkloadLifecycle::reconcile` enqueue block + exit observer | `…/reconcilers/workload_lifecycle.rs:181`; `…/worker/exit_observer.rs:230-256` | The level-trigger handoff to a new reconciler | **EXTEND** (additive) | Add a third `Action::EnqueueEvaluation { reconciler: SVID_LIFECYCLE_NAME, target: job/<wl> }` in the existing alloc-mutating block (ungated by kind) + a sibling `broker().submit` in the exit observer. Same shape as the shipped bridge/service-lifecycle handoffs; broker LWW dedups. No mechanism change. |
| `hydrate_actual` + `AppState` (`identity` field) | `…/reconciler_runtime.rs:2190`; `…/lib.rs:153` | Project the in-process held set into `actual` | **EXTEND** (one new arm) | New `AnyReconciler::SvidLifecycle(_)` arm reads `state.identity.held_snapshot()` — identical shape to the `WorkflowLifecycle` arm reading `state.workflow_engine.live_instances()` (`:2206-2209`/`:2166`). Feasible against the runtime as written. |
| `CorrelationKey::derive` | `crates/overdrive-core/src/id.rs` | Cause→audit correlation across ticks | **REUSE AS-IS** | `derive(target, spec_hash, purpose)` exists (ADR-0035); actions reuse it. No change. |
| `AllocationId` / `NodeId` / `WorkloadId` / `CertSerial` / `UnixInstant` | `crates/overdrive-core/src/id.rs` + time types | Action / View / `for_allocation` fields | **REUSE AS-IS** | All exist; used directly. No change. |
| `Entropy` port | `crates/overdrive-core/src/traits/entropy.rs` | CSPRNG serials | **REUSE AS-IS** | Serials flow through `Entropy` inside `issue_and_audit` (ADR-0063 D7) → DST-deterministic. Unchanged. |
| `IdentityMgr` + `IdentityRead` + `SvidLifecycle` + `SvidLifecycleView` + `SimIdentityRead` + `action_shim/issue_svid.rs` | — | The holder/reader/dropper + the read port + the identity reconciler | **CREATE NEW** (justified) | No existing component holds an SVID set in process, exposes a sync identity read surface, or reconciles identity as a separate convergence target. The holder is the *feature* — no existing alternative; each mirrors a shipped precedent. |

**Verdict (rev 2): 8 REUSE AS-IS, 2 EXTEND (additive — `Action`, `SpiffeId`-as-
CONSOLIDATION), 1 EXTEND (`AppState`), 2 EXTEND (the enqueue handoff +
`hydrate_actual` held-set projection — both additive on shipped surfaces), 1
REUSE-pattern-via-new-executor, 6 CREATE-NEW (justified). Zero unjustified CREATE
NEW.** Reuse-heavy by construction — the CA, state layers, newtypes, reconciler
runtime, action-shim, AND the enqueue/hydrate-actual machinery all already exist;
this feature is the holder/reader/dropper *composition* behind a new reconciler +
`IdentityRead` port, triggered by the existing handoff mechanism and reading the
held set through the existing `actual`-projection mechanism.

---

## Wave: DESIGN / [REF] Open Questions (deferred to DISTILL / DELIVER)

All 5 DISCUSS design-sensitive surfaces (Open-Questions #1–#5) are **RESOLVED**
in ADR-0067 (D2/D5, D7, D4, D8, D6 respectively) — none remain open. **Rev 2**
additionally resolved the 5 DESIGN-review findings (ADR-0067 § Revision rev 2):
the restart model (re-issue on boot — D1), the held-set-as-`actual` (D1/D4 —
**FEASIBLE**, grounded against the `WorkflowLifecycle`/`live_instances()`
precedent), the retry-memory View (D8), the enqueue/handoff trigger (D5b), and the
`SpiffeId` consolidation (D5). What is genuinely deferred:

- **Leaf-key zeroization on drop** (memory-scrubbing beyond removing the held-map
  entry) — **explicitly NOT in #35; accepted residual risk.** Drop-on-stop removes
  the entry so the key is no longer reachable — O2 is *reachability* (met), not
  memory zeroization. Scrubbing the key bytes is separate hardening, out of #35
  scope; the scoping decision is made here (accepted residual risk), so it is
  **not** a DESIGN open question.
- **The exact near-expiry *threshold*** the gated #40 branch compares the held
  cert's `not_after` (from `actual`) against (ADR-0063's 1h SVID TTL is the issuance
  policy; the threshold the #40 seam uses to decide "near expiry" is **#40's** to
  pin when it flips the gate). Deferred to **#40**.
- **Fault-injection scenario set** for the Earned-Trust probes (audit-write
  failure refuses issuance; a broken executor fails the convergence invariant) —
  flagged for **DISTILL** (`nw-acceptance-designer`).

No new GH issue required; every deferral cites an EXISTING issue/phase
(**#40**, DESIGN-call hardening) or a DISTILL handoff. No invented numbers.

---

## Wave: DESIGN / [REF] Handoff (DESIGN → DISTILL / DEVOPS)

- **To DISTILL (nw-acceptance-designer)**: the resolved surfaces (ADR-0067 rev 2)
  + the UAT scenarios (embedded in the DISCUSS sections above) + the DST surfaces —
  `assert_eventually!("running allocs hold a valid SVID")` (North-Star K1/O1),
  `identity_read_equivalence` (the `IdentityRead` contract, incl. clause 5
  post-drop `None`), drop-on-stop (K2), twin-run determinism (K5), **the
  enqueue/handoff regression** (Running AND Stopped transitions tick `SvidLifecycle`
  for `job/<workload_id>` with no manual broker poke — rev 2 D5b), **the
  restart-recovery DST scenario** (empty the held set, retick → re-issue every
  still-Running alloc, each leaving a fresh `issued_certificates` row, no
  stale/silent credential — rev 2 D1) — and the host-adapter test-tier proofs
  (`issued_certificates` readback via the ObservationStore + `openssl verify` on
  the held leaf chain, gated `integration-tests`, via Lima — built-in-ca's
  `rcgen_ca_chain_verify` shape). The fault-injection set (audit-write failure,
  broken hold/drop) is DISTILL's to author.
- **To DEVOPS (nw-platform-architect)**: the KPIs K1–K5 (DISCUSS § Outcome KPIs).
  **No external API integrations** → no contract-testing (Pact) annotation
  needed; the DST equivalence tests are the cross-adapter contract guard. The
  one operational note: the #40 rotation-seam emit MUST stay gated until #40
  registers `cert_rotation` (a naïve emit raises `UnknownWorkflow` every tick
  against production's empty-registry engine).
- **Upstream (PO / orchestrator)**: the **O4/K3 reframe** ("no redundant re-issue"
  → "bounded, audited restart re-issue; no stale/silent credential") is recorded
  in `design/upstream-changes.md`. The product SSOT updates were applied on
  2026-06-08; this handoff treats ADR-0067 rev 2 and the applied product wording
  as authoritative.
- **Paradigm**: OOP / ports-and-adapters → `@nw-software-crafter` for DELIVER.

---

## Wave: DESIGN / [REF] Changed Assumptions (rev 2 — DESIGN-review back-propagation)

The rev-2 rework corrected one DISCUSS assumption that propagated into the
KPIs/outcomes. Per the nw-design back-propagation contract, the change is recorded
here and in `design/upstream-changes.md` (which flags the product-SSOT edits for
the PO):

- **Original (DISCUSS, feature-delta § Outcome KPIs K3 / Opportunity-Outcomes O4;
  `jobs.yaml` J-SEC-002 O4):** "Minimize likelihood a control-plane restart leaves
  a running workload with no held SVID, **or re-issues redundantly** (idempotence;
  persist issuance INPUTS)." The phrase "no redundant re-issue" + "held state
  recomputes on boot" assumed restart recovery could rebuild held state **without**
  re-issuing.
- **New (DESIGN rev 2):** that is **impossible** — the leaf key is non-persistable
  (`CaKeyPem` has no `Serialize`, ADR-0063 D9) and non-reconstructable (each
  `issue_and_audit` mints a fresh leaf, `ca_issuance.rs:34-40`). The honest outcome
  is **"bounded, audited restart re-issue; no stale or silent credential"**: on
  boot every still-Running alloc is re-issued (`running ∧ ¬held → IssueSvid`, one
  per running alloc, each audited). The guardrail shifts from "0 redundant
  re-issues" (unachievable) to "0 Running allocs left without a held SVID AND every
  restart re-issue leaves an `issued_certificates` audit row" (achievable + a
  stronger no-silent-credential guarantee).
- **Rationale:** the rev-1 K3/O4 target would have been un-meetable (DELIVER would
  have had no coherent implementation path — the Critical finding). The reframe
  keeps the *intent* (a restart must not leave a workload without identity, and
  must not silently mint) while making the target match the cryptographic reality.
- **Propagation:** `jobs.yaml` J-SEC-002 O4 + the feature-delta DISCUSS K3 row were
  updated by the orchestrator on 2026-06-08; see `design/upstream-changes.md` for
  the retained traceability record.

---

## Wave: DISTILL / [REF] Reconciliation

**Result: PASSED — 0 unresolved contradictions after pre-scenario cleanup.**

DISTILL found one stale live journey statement that still described the rev-1
"recompute held state without re-issue" restart model. That contradicted
ADR-0067 rev 2. The journey now matches the accepted design: after restart,
`IdentityMgr` starts empty; every still-Running allocation is re-issued once
during recovery convergence; every re-issue is audited. DIVERGE prose preserving
the earlier hypothesis remains historical and is superseded by ADR-0067 rev 2
plus `design/upstream-changes.md`.

## Wave: DISTILL / [REF] Scenario List

Executable scenario SSOT: `docs/feature/workload-identity-manager/distill/test-scenarios.md`.

| ID | Scenario | Tags | Scaffold |
|---|---|---|---|
| S-WIM-01 | Running alloc without held SVID emits `IssueSvid` | `@in-memory @property` | `crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs` |
| S-WIM-02 | `IssueSvid` executor audits before hold | `@in-memory` | `crates/overdrive-control-plane/tests/acceptance/issue_svid_action_shim.rs` |
| S-WIM-03 | Stopped alloc with held SVID emits `DropSvid` | `@in-memory` | `crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs` |
| S-WIM-04 | `IdentityRead` returns SVID + trust bundle without re-issue | `@in-memory` | `crates/overdrive-control-plane/tests/acceptance/identity_mgr_read_contract.rs` |
| S-WIM-05 | `IdentityRead` returns absence after drop | `@in-memory @error` | `crates/overdrive-control-plane/tests/acceptance/identity_mgr_read_contract.rs` |
| S-WIM-06 | `SimIdentityRead` matches `IdentityMgr` read contract | `@in-memory @property` | `crates/overdrive-sim/tests/acceptance/identity_read_equivalence.rs` |
| S-WIM-07 | Audit-write failure refuses hold | `@in-memory @error` | `crates/overdrive-control-plane/tests/acceptance/issue_svid_action_shim.rs` |
| S-WIM-08 | View is retry memory only | `@in-memory @property` | `crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs` |
| S-WIM-09 | Rotation seam is emit-gated until #40 | `@in-memory @error` | `crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs` |
| S-WIM-10 | Lifecycle transitions enqueue `SvidLifecycle` | `@in-memory` | `crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs` |
| S-WIM-11 | Running-set identity invariant has teeth | `@in-memory @dst_invariant @property` | `crates/overdrive-sim/tests/acceptance/identity_read_equivalence.rs` |
| S-WIM-WS | Walking skeleton: issue, hold, audit, verify, drop | `@walking_skeleton @real-io @adapter-integration` | `crates/overdrive-control-plane/tests/integration/workload_identity_manager/lifecycle.rs` |
| S-WIM-12 | Restart re-issues each still-running alloc with audit row | `@real-io @error` | `crates/overdrive-control-plane/tests/integration/workload_identity_manager/lifecycle.rs` |

## Wave: DISTILL / [REF] Adapter Coverage

| Adapter / port | Covered by |
|---|---|
| `SvidLifecycle::reconcile` | S-WIM-01, S-WIM-03, S-WIM-08, S-WIM-09 |
| Workload lifecycle / exit-observer handoff | S-WIM-10 |
| Action-shim `IssueSvid` / `DropSvid` | S-WIM-02, S-WIM-07, S-WIM-WS |
| `IdentityMgr` holder | S-WIM-02, S-WIM-04, S-WIM-05, S-WIM-WS, S-WIM-12 |
| `IdentityRead` / `SimIdentityRead` | S-WIM-04, S-WIM-05, S-WIM-06 |
| `Ca` + `issue_and_audit` | S-WIM-02, S-WIM-07, S-WIM-WS, S-WIM-12 |
| `ObservationStore` audit rows | S-WIM-02, S-WIM-07, S-WIM-WS, S-WIM-12 |
| Reconciler ViewStore / retry memory | S-WIM-08, S-WIM-12 |

## Wave: DISTILL / [REF] Scaffolds

Rust pending scaffolds created and wired into Cargo:

- `crates/overdrive-core/tests/acceptance/svid_lifecycle_reconcile.rs`
- `crates/overdrive-control-plane/tests/acceptance/issue_svid_action_shim.rs`
- `crates/overdrive-control-plane/tests/acceptance/identity_mgr_read_contract.rs`
- `crates/overdrive-sim/tests/acceptance/identity_read_equivalence.rs`
- `crates/overdrive-control-plane/tests/integration/workload_identity_manager/lifecycle.rs`

Entrypoints updated:

- `crates/overdrive-core/tests/acceptance.rs`
- `crates/overdrive-control-plane/tests/acceptance.rs`
- `crates/overdrive-control-plane/tests/integration.rs`
- `crates/overdrive-sim/tests/acceptance.rs`

## Wave: DISTILL / [REF] Test Placement

Placement follows the workspace Rust ATDD policy in
`docs/architecture/atdd-infrastructure-policy.md`: no `.feature` files, no
Python step definitions, direct Rust `#[test]` / integration tests only.

Layer mapping:

- L1 pure contracts in `overdrive-core`.
- L1/L2 control-plane action/read contracts in `overdrive-control-plane`.
- L2 sim equivalence and DST invariant in `overdrive-sim`.
- L3 real stores + real CA + `openssl verify` under
  `overdrive-control-plane/tests/integration`, gated by `integration-tests`.

## Wave: DISTILL / [REF] Pre-DELIVER RED Classification

`docs/feature/workload-identity-manager/distill/red-classification.md` records the
pending-scaffold status. The scaffolds compile as expected-panic placeholders now.
DELIVER must replace one scaffold at a time with real assertions and confirm the
resulting failure is missing functionality, not import/setup failure, before
writing production code.

## Wave: DISTILL / [REF] Outcome Registry

DISTILL registers three #35 contract surfaces in
`docs/product/outcomes/registry.yaml`:

- `OUT-WIM-SVID-LIFECYCLE` — `SvidLifecycle` issue/drop specification.
- `OUT-WIM-IDENTITY-READ` — `IdentityRead` operation.
- `OUT-WIM-RUNNING-SET-INVARIANT` — running-set identity invariant.

## Wave: DISTILL / [REF] Verification Catalogue

No new EDD expectation is created. #35 has no new operator CLI surface; its
test-tier walking skeleton unblocks existing catalogue entries:

- `E03-ca-full-chain-verifies` — full Root → Intermediate → SVID chain verifies.
- `O05-ca-issued-certificates-audit-row` — issued certificate row is observable
  once the operator render lands.

The pure reconciler, action-shim, read-port, and DST-invariant scenarios remain
in the Rust test tiers.

## Wave: DISTILL / [REF] Handoff

- **To DELIVER**: implement the scaffolds slice by slice, replacing
  `#[should_panic(expected = "RED scaffold")]` bodies with real assertions.
- **Authority**: ADR-0067 rev 2 supersedes any older wording that claims restart
  recovery recomputes held SVIDs without re-issue.
- **Boundary**: #35 remains a foundation feature. Operator-facing certificate
  rendering belongs to #215, blocked on #35.
