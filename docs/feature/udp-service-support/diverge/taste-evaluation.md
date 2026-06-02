# Taste Evaluation — udp-service-support `update_service` proto-threading

> Generation is complete (`options-raw.md`); this file is the evaluation
> phase. DVF filter first, then weights LOCKED before scoring, then the
> 4-criterion taste matrix, then the ranking. The recommendation
> (`../recommendation.md`) is derivable from this matrix.

## Evaluation lens for THIS decision

The four taste criteria are read through an **architectural-surface**
lens, since the "user" of `update_service` is a Rust engineer (the
dataplane/reconciler author — J-PLAT-004 persona), and the "first
interaction" is *reading the trait and writing a call site*. Concretely:

- **T1 Subtraction** → can the option deliver the proto-fix with one
  fewer concept / argument / type?
- **T2 Concept Count** → how many NEW mental concepts must an engineer
  learn to call `update_service` correctly (and to read the lockstep)?
- **T3 Progressive Disclosure** → does the common call (single-listener
  TCP/UDP) stay simple, with multi-listener complexity revealed only when
  needed?
- **T4 Speed-as-Trust** → "speed" = *reviewer/maintainer velocity and the
  PR-time signal*. Does the option keep the lockstep gate a fast,
  single-set-equality check, and keep the blast radius reviewable in one
  PR? (Latency here is human, not wire — the wire latency is identical
  across all options once the right key set is installed.)

This lens is locked before scoring; it is not adjusted per option.

---

## Phase 1: DVF filter — primary triage

Elimination threshold: DVF total < 6 → eliminated before taste scoring.

| Option | Desirability (addresses the job?) | Feasibility (buildable in scope?) | Viability (sustainable surface?) | DVF total | Verdict |
|---|---|---|---|---|---|
| **1 Positional proto** | 4 — fully fixes O1–O3; serves operator + author | 5 — smallest change; one scalar arg | 4 — sustainable but multi-listener (O5) needs upstream fan-out | **13** | SURVIVES |
| **2 Typed aggregate** | 5 — fixes O1–O3 + O4 (single SSOT) + extensible | 4 — buildable; touches every call site (single-cut) | 5 — industry-validated surface (Cilium/Katran); extends to SCTP | **14** | SURVIVES |
| **3 Per-listener** | 5 — fixes O1–O3 + native multi-listener (O5) | 3 — reshapes hydrator emission granularity + trait | 4 — clean but couples trait to listener granularity early | **12** | SURVIVES |
| **4 No-sig / Action-carried** | 3 — fixes O1–O3 but reconstructs `(vip,port,proto)` from scattered `service_id` lookup (fights C2) | 2 — proto is *intent*, not packet-derivable; no honest side-channel (prior art: no system re-derives declared proto at install) | 3 — `service_id`-coupled recovery is fragile to evolve | **8** | SURVIVES (barely) |
| **5 Vec<Listener> aggregate** | 5 — fixes everything + adapter-internal multi-listener | 3 — richest aggregate; shared fan-out loop in BOTH adapters | 4 — powerful but front-loads multi-listener before US-04 needs it | **12** | SURVIVES |
| **6 ServiceFrontend newtype** | 5 — fixes O1–O4; `(vip,port,proto)` typed, backends separate | 4 — buildable; touches call sites but `backends` arg unchanged | 5 — Katran `VipKey` shape exactly; extends to SCTP | **14** | SURVIVES |

**No eliminations** — all six clear the < 6 threshold. Option 4 is the
weakest (DVF 8): its feasibility is dragged down because the protocol is
*declared intent* that must be carried, and prior art confirms no named
system re-derives a declared protocol at install time (competitive-research
§ Reference 4 synthesis). It survives DVF but, per the skill, **DVF is a
filter, not a tiebreaker** — option 4 is scored on taste like the rest and
its weakness will surface there too, honestly.

---

## Phase 2 + 3: Locked weights + scoring matrix

### Weights — LOCKED before any taste score (developer-tool profile)

This is a **developer/platform tool** surface (a Rust trait + DST
invariant consumed by engineers), so the skill's **Developer Tool** weight
column applies. Rationale recorded per weight:

| Criterion | Weight | Rationale (locked) |
|---|---|---|
| DVF (avg of D/F/V, normalized to /5) | **25%** | Standard dev-tool weight. All options fix the wire bug (O1–O3) — DVF mostly separates feasibility/viability, which matters but is not the discriminator. |
| T1 Subtraction | **15%** | Dev-tool weight. Subtraction matters but a slightly larger surface that is *industry-standard* is acceptable on a trait read by few engineers. |
| T2 Concept Count | **20%** | The discriminator between "proto as a field of an existing key shape" vs "a brand-new aggregate concept." Weighted full per the skill's dev-tool column. |
| T3 Progressive Disclosure | **15%** | The single-listener common case must stay simple; multi-listener complexity should not be front-loaded (US-04 ships before US-05). Dev-tool weight. |
| T4 Speed-as-Trust (reviewer/gate velocity) | **25%** | Elevated dev-tool weight. This is a CI-gated dataplane surface: the lockstep gate's clarity and the single-PR reviewability of the blast radius ARE the trust signal (J-PLAT-004). |

Weights sum to 100%. **No weight is adjusted after scoring** (anti-pattern
guard). If the recommendation contradicts the matrix, the weight change is
documented explicitly in `recommendation.md` — it is not.

### Per-criterion scores (1–5) with one-line justification

**DVF (avg/5):** 1 → 13/15=4.33 · 2 → 14/15=4.67 · 3 → 12/15=4.0 ·
4 → 8/15=2.67 · 5 → 12/15=4.0 · 6 → 14/15=4.67.

**T1 Subtraction** (can it deliver with one fewer concept?):
- O1 = **5** — nothing removable: one scalar proto is the irreducible
  carrier; no new type.
- O2 = **3** — the descriptor adds a wrapping concept beside `BackendKey`;
  a reviewer could argue `(vip,port,proto)` is already expressible without
  a new aggregate.
- O3 = **3** — listener-as-unit adds a granularity concept earlier than
  the single-listener path needs.
- O4 = **2** — adds a `service_id`-keyed recovery path AND keeps the proto
  implicit at the trait — accumulation of indirection, not subtraction.
- O5 = **2** — richest aggregate + an adapter-internal fan-out loop; most
  removable parts for the US-04 single-listener milestone.
- O6 = **4** — one newtype, `backends` arg untouched; minor removable
  element (the newtype could be a positional pair) but the frontend
  identity is genuinely one thing.

**T2 Concept Count** (new mental concepts to call it correctly):
- O1 = **4** — one new concept (`proto` is now an argument), well-anchored
  to the existing `Proto` enum the engineer already knows.
- O2 = **3** — two concepts: the `ServiceDescriptor` type AND the
  service_id-reconciliation question it forces (does it re-absorb
  service_id/ServiceVip?). Anchored to Katran/Cilium but still a new
  aggregate to learn.
- O3 = **3** — two concepts: listener-as-update-unit AND the
  (VIP,port)-granularity it implies.
- O4 = **2** — three interdependent concepts: implicit-proto-at-trait +
  service_id-keyed recovery + the lookup the adapter must hold.
- O5 = **2** — three concepts: the aggregate + `Vec<Listener>` +
  adapter-internal fan-out semantics.
- O6 = **4** — one new concept (`ServiceFrontend` newtype) that is the
  *forward twin of the `BackendKey` the engineer already knows from the
  reverse side* — strongly anchored (competitive-research: Katran `VipKey`
  is byte-identical shape).

**T3 Progressive Disclosure** (common case simple; depth on demand):
- O1 = **4** — single-listener call is `update_service(vip, proto,
  backends)`; multi-listener stays an upstream-hydrator concern (revealed
  only at US-05).
- O2 = **4** — single-listener descriptor is flat; `Vec<Listener>` is NOT
  forced into the descriptor (option 2 is per-listener-or-per-service by
  design choice, defaults to the simple shape).
- O3 = **3** — multi-listener granularity is exposed at the trait from
  slice 01, before US-04 needs it.
- O4 = **3** — the recovery indirection is present for every call,
  including the simple single-listener one.
- O5 = **2** — `Vec<Listener>` is in the signature from slice 01; the
  single-listener US-04 milestone must construct a one-element Vec —
  multi-listener complexity front-loaded.
- O6 = **4** — single-listener call is `update_service(frontend,
  backends)`; multi-listener stays upstream like option 1.

**T4 Speed-as-Trust** (gate clarity + single-PR reviewable blast radius):
- O1 = **4** — smallest blast radius (5 call sites), reviewable in one PR;
  lockstep stays a single set-equality over the passed proto. Minor: the
  4th positional arg slightly erodes call-site readability.
- O2 = **3** — every call site reconstructs/constructs the descriptor;
  larger PR; the service_id-reconciliation question (B2) must be resolved
  in the same PR or the gate can false-green (review B2). Gate is clean
  once resolved.
- O3 = **3** — reshapes hydrator emission + trait in one PR; the lockstep
  must assert per-listener — clear but a wider change.
- O4 = **2** — the lockstep gate becomes hard to express against BOTH
  adapters: the proto is recovered via a `service_id` lookup the real
  Ebpf adapter holds differently than Sim, so the "identical
  `(ip,port,proto)` set" assertion fights the indirection (worsens H1).
- O5 = **3** — the shared adapter-internal fan-out loop is production
  logic both adapters must implement identically (C5); reviewable but the
  largest single-PR surface.
- O6 = **4** — blast radius comparable to option 1 (newtype swap on one
  arg; `backends` untouched); lockstep is a clean set-equality over the
  `ServiceFrontend`→`BackendKey` projection (the twin shape makes the gate
  assertion *trivial*).

### Weighted scoring matrix

`Final = DVF×0.25 + T1×0.15 + T2×0.20 + T3×0.15 + T4×0.25`

| Option | DVF | T1 Sub | T2 Concept | T3 Prog | T4 Speed | **Weighted Total** |
|---|---|---|---|---|---|---|
| **6 ServiceFrontend newtype** | 4.67 | 4 | 4 | 4 | 4 | **4.17** |
| **1 Positional proto** | 4.33 | 5 | 4 | 4 | 4 | **4.13** |
| **2 Typed aggregate** | 4.67 | 3 | 3 | 4 | 3 | **3.57** |
| **3 Per-listener** | 4.0 | 3 | 3 | 3 | 3 | **3.25** |
| **5 Vec<Listener> aggregate** | 4.0 | 2 | 2 | 2 | 3 | **2.75** |
| **4 No-sig / Action-carried** | 2.67 | 2 | 2 | 3 | 2 | **2.32** |

**Arithmetic (top three, for audit):**
- Option 6: 4.67×.25 + 4×.15 + 4×.20 + 4×.15 + 4×.25 = 1.168 + 0.60 +
  0.80 + 0.60 + 1.00 = **4.17**.
- Option 1: 4.33×.25 + 5×.15 + 4×.20 + 4×.15 + 4×.25 = 1.083 + 0.75 +
  0.80 + 0.60 + 1.00 = **4.13**.
- Option 2: 4.67×.25 + 3×.15 + 3×.20 + 4×.15 + 3×.25 = 1.1675 + 0.45 +
  0.60 + 0.60 + 0.75 = **3.5675 ≈ 3.57**. *(An earlier draft of the
  matrix cell rounded this to 3.62; corrected to 3.57 so the table, this
  arithmetic, and the ranking agree. The ordering 6 > 1 > 2 > 3 > 5 > 4 is
  unaffected — option 2 is third on either value. Review FIX-1.)*

---

## Phase 4 ranking summary

1. **Option 6 — ServiceFrontend newtype — 4.17**
2. **Option 1 — Positional proto — 4.13** (Δ 0.04 from the top)
3. **Option 2 — Typed aggregate — 3.57** (the user's standing preference)
4. Option 3 — Per-listener — 3.25
5. Option 5 — Vec<Listener> aggregate — 2.75 *(Prism PRISM-1 erratum fix: 4.0×.25+2×.15+2×.20+2×.15+3×.25 = 2.75; was 2.85. Rank-5, decision-unaffected.)*
6. Option 4 — No-sig / Action-carried — 2.32

### The load-bearing finding

The matrix produces a **two-way near-tie at the top** (6: 4.17 vs 1: 4.13,
Δ 0.04 — inside noise) and places the **user's standing preference
(option 2) a clear third at 3.57**. Both top options carry proto as a
*typed-or-scalar field threaded into the existing call shape* rather than
re-modelling the whole call as an aggregate. The aggregate's cost
(T1=3, T2=3, T4=3) is what drops it 0.56–0.60 below the leaders.

**This is exactly the H3 finding made quantitative:** the simpler
alternatives (1 and 6) were never weighed against the aggregate, and on a
locked developer-tool matrix they outscore it. The recommendation
(`../recommendation.md`) does NOT silently follow the user's preference;
it reports the matrix honestly and frames the decision the DISCUSS
revision must make.

### Why option 6 noses ahead of option 1

Both fix the bug with a tiny blast radius. Option 6 wins T2 (concept
anchoring: `ServiceFrontend` is the forward twin of the `BackendKey` the
engineer already reads on the reverse side — prior art: Katran `VipKey`)
and T4 (the twin shape makes the lockstep set-equality *trivial* to
express). Option 1 wins T1 (one scalar, zero new types — maximal
subtraction). The Δ 0.04 means the choice between them is a *secondary
DESIGN preference*, not a divergence-level decision — both are
"thread proto into the existing call without an aggregate."

## Anti-pattern self-audit

| Anti-pattern | Check |
|---|---|
| Cherry-picking criteria | All 6 options scored on all 4 criteria + DVF. PASS. |
| Retroactive justification | Scores derived from mechanism facts in options-raw.md BEFORE the ranking; recommendation written after. PASS. |
| Weight manipulation | Dev-tool weights locked with rationale before scoring; not touched after. The user's preferred option (2) is NOT advantaged by the weights. PASS. |
| "It feels right" override | The matrix does NOT pick the user's preference; the recommendation follows the matrix and documents the dissent. PASS. |
| Feasibility as tiebreaker only | Option 4's low feasibility surfaces in DVF AND in T-scores; not kept "for aesthetics." PASS. |
