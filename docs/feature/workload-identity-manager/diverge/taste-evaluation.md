# Taste Evaluation — workload-identity-manager (GH #35)

**Wave**: DIVERGE (Phase 4 of 4) · **Agent**: Flux (nw-diverger) · **Date**: 2026-06-08

> Discipline guards (from the taste-evaluation skill): **weights are locked BEFORE
> scoring** (§ Phase 3); DVF is a **filter, not a tiebreaker** (eliminate < 6 before any
> taste score); scores are assigned **before** the recommendation is written; the
> recommendation **must be derivable from the matrix** or any deviation must be an
> explicit, documented weight change. The six options under evaluation are defined in
> `options-raw.md` (Options 1–6); X2 was logged un-promoted there and is not scored.

---

## What "taste" means for a platform-internal subsystem

This feature ships no end-user UI. The "user" of the design is the **platform engineer
who maintains, extends, and DST-tests the code** (persona: Devon/Sam), and the
**downstream consumer authors** (#26 sockops, gateway 4.4, telemetry). So the Apple/Jobs
taste criteria translate as:

- **T1 Subtraction** → how few *new mechanisms/components* the design adds beyond the
  shipped runtime/CA/observation primitives (the O6 outcome made operational).
- **T2 Concept Count** → how many *new mental concepts* a maintainer or consumer author
  must learn to use/extend the subsystem.
- **T3 Progressive Disclosure** → does the *first* use (issue-hold-read-drop, #35's scope)
  expose only what #35 needs, with the #40 rotation seam revealed later — not front-loaded.
- **T4 Speed-as-Trust** → the consumer **read-path latency** (the mTLS handshake hot path,
  O3) and the convergence latency to a held SVID (O1). "Speed" here is real wire latency,
  not perceived UI snappiness.

---

## Phase 1: DVF filter (primary triage — eliminate total < 6)

DVF lenses scored 1–5. **Feasibility here is load-bearing**: it encodes the project's
*correctness constraints* (reconciler purity, state-layer hygiene, single-cut, port-trait
discipline, `BTreeMap`-not-`HashMap`). A design that violates a binding rule scores low on
Feasibility — that is the filter doing its job, not a taste judgment.

| Option | Desirability (serves J-SEC-002?) | Feasibility (buildable under the binding rules?) | Viability (sustains the platform's model + #40/#26/4.7 path?) | DVF total | Survives? |
|---|---|---|---|---|---|
| **1** Shared `Arc` store + `SvidLifecycle` reconciler + actions | 5 — directly the issue's job; held+readable+dropped | 5 — exact mirror of `ServiceMapHydrator`→action→executor; reconciler stays pure; `BTreeMap`; port-trait-clean | 5 — matches whitepaper §7 `Arc<IdentityMgr>`; clean #40 + #26 + 4.7 seam | **15** | **Yes** |
| **2** No new reconciler (fold into `WorkloadLifecycle` / executor bolt) | 4 — serves the job; identity lifecycle rides workload lifecycle | 4 — buildable and pure, but couples two concerns into one reconciler/executor; the X1 "no-Action, executor-side-effect" end strains the ADR-0023 action boundary (issuance becomes an un-actioned side effect, weakening DST observability of issue/drop) | 4 — works, but folding identity into `WorkloadLifecycle` muddies the per-reconciler-View boundary and complicates the 4.7 ACME-lane extension | **12** | **Yes** |
| **3** istio-SDS-style `watch`-channel push read surface | 5 — serves the job + change-notification | 4 — buildable; but a `watch`/`broadcast` read surface adds an async channel the *consumers don't need yet* (#26/gateway are not built; no consumer to subscribe), and channel state across `.await` invites the "production shaped by simulation" / lock-across-await hazards the rules call out | 5 — strongest #40 rotation-seam alignment (push rotated SVID down the channel) | **14** | **Yes** |
| **4** Kernel `IDENTITY_MAP` BPF map read surface | 4 — serves the kernel consumer well, less the userspace gateway/telemetry consumers | 2 — `IDENTITY_MAP` is **unbuilt**; requires a new BPF map + kernel-side program + dataplane port + verifier/Tier-2/3 surface — most of which is #26 (sockops mTLS) scope, not #35. Building it here is a single-cut violation (ships kernel surface #26 owns) and a large mechanism the issue does not call for | 3 — viable eventually, but couples identity *storage* to the eBPF stack; userspace consumers (gateway/telemetry) can't read a kernel map without extra plumbing | **9** | **Yes** (≥6) — but low |
| **5** No store — reconciler `View` is store-of-record | 4 — serves O1/O4 well; the leaf-key split complicates O2/O3 | 2 — the leaf **private key cannot be persisted** (O2, ADR-0063 D6); a persisted View *cannot* be the read surface for the one piece of material consumers most need (the key), forcing a parallel out-of-band key holder anyway — which *reintroduces* the very store this option claims to eliminate. State-layer hygiene: CA material is *intent*, not View memory | 3 — viable for the *facts*, not for the live key+cert consumers present | **9** | **Yes** (≥6) — but low |
| **6** Observation-row-driven rebuild on boot | 3 — serves recovery, not the steady-state read surface | 2 — rebuilding held identity from **gossiped, eventually-consistent** observation rows makes the held set's correctness depend on convergence timing; re-minting on boot from audit rows is a heavier recovery path than persisting issuance inputs; state-layer hygiene strain (observation rows driving intent-side held material) | 4 — the audit rows exist; recovery is *possible* this way | **9** | **Yes** (≥6) — but low |

**Elimination threshold (< 6):** none scored below 6, so none is filter-eliminated. **All
6 survive to taste scoring.** (The DVF spread already separates the field: Options 1/2/3
cluster at 12–15; Options 4/5/6 cluster at 9. The taste pass refines within and across
these bands — it does not pre-judge.)

> **Why nothing was eliminated despite weak options.** Per the skill's anti-pattern table,
> DVF is a *filter, not a tiebreaker* — I do not drop an option just for clustering low; I
> drop it only below the threshold. 4/5/6 clear 6 (each is *buildable* in principle), so
> they earn a taste score. Their correctness-rule strain is captured *in their Feasibility
> score* (the honest place for it), and the taste pass will show why they do not win.

---

## Phase 2 + 3: Locked weights and the scoring matrix

### Weights — LOCKED before scoring (Developer-Tool profile, adjusted)

This is platform-internal infrastructure on a connection hot path. I use the skill's
**Developer-Tool** weighting and adjust for this feature's specifics, **documenting the
adjustment now, before any taste score is read**:

| Criterion | Skill Dev-Tool default | **Locked weight (this feature)** | Adjustment rationale (pre-scoring) |
|---|---|---|---|
| DVF (avg) | 25% | **30%** | Raised: the binding correctness rules (reconciler purity, state-layer hygiene, single-cut) are *the* dominant constraint for a security primitive — a design that strains them is disqualifying in a way that matters more than for a typical dev tool. |
| T1 Subtraction | 15% | **20%** | Raised: O6 (mechanism economy) is an explicit job outcome; reusing shipped primitives vs inventing parallel ones is a first-order concern here. |
| T2 Concept Count | 20% | **20%** | Held at default — maintainer/consumer-author cognitive load is genuinely important and the default weight reflects it. |
| T3 Progressive Disclosure | 15% | **15%** | Held — the #35-now / #40-later staging matters but is secondary to subtraction/feasibility. |
| T4 Speed-as-Trust | 25% | **15%** | **Lowered** from the Dev-Tool default. Rationale: the read-path latency *difference* across the surviving options is small in absolute terms (all are in-process; the contrast is getter-vs-watch-vs-kernel-map, all sub-microsecond-class for #35's single-node scope), so weighting it at 25% would over-reward a dimension where the options barely differ. It stays material (15%) because the handshake hot path is real, but it is not the discriminator the correctness rules are. |

Weights sum to 100%. **These are now frozen for the rest of Phase 4.**

### Taste scores (1–5 per criterion, per the skill rubrics) — assigned before the recommendation

| Option | T1 Subtraction | T2 Concept Count | T3 Progressive Disclosure | T4 Speed-as-Trust | (rationale anchors below) |
|---|---|---|---|---|---|
| **1** Shared `Arc` store + reconciler + actions | 4 | 4 | 5 | 4 | — |
| **2** No new reconciler (fold) | 5 | 4 | 4 | 4 | — |
| **3** `watch`-channel push | 2 | 3 | 2 | 5 | — |
| **4** Kernel `IDENTITY_MAP` | 2 | 2 | 2 | 4 | — |
| **5** View-as-store | 3 | 2 | 3 | 3 | — |
| **6** Observation-row rebuild | 3 | 3 | 3 | 2 | — |

**Per-criterion rationale (the rubric application):**

- **T1 Subtraction** — *Option 2 = 5* (no new reconciler, possibly no new Action — the
  fewest new mechanisms). *Option 1 = 4* (one reconciler + 2 actions + 1 struct, all on
  shipped shapes; one element — the new struct — is the minimum a shared store needs).
  *Options 5/6 = 3* (reuse a store but add a fact/key split or a boot re-issue path — a
  removable-but-load-bearing extra). *Options 3/4 = 2* (a whole channel subsystem / a
  whole BPF-map+port subsystem the #35 scope does not require yet).
- **T2 Concept Count** — *Options 1/2 = 4* (one new concept: "the platform holds your
  SVID in a shared store keyed by alloc"; maps onto the existing `ServiceMapHydrator`
  mental model). *Options 3/6 = 3* (adds "subscribe to identity changes" / "held set is
  rebuilt from audit rows" — a second concept). *Options 4/5 = 2* (kernel-map identity, or
  "the View is the store but the key lives elsewhere" — two interdependent concepts).
- **T3 Progressive Disclosure** — *Option 1 = 5* (first use is exactly issue-hold-read-drop;
  the #40 rotation seam is a *later-revealed* `Action::StartWorkflow` branch, not
  front-loaded). *Option 2 = 4* (same, but the folded reconciler exposes workload+identity
  decisions together). *Options 5/6 = 3*. *Options 3/4 = 2* (force the channel/kernel-map
  complexity into the first cut before any consumer needs it).
- **T4 Speed-as-Trust** — *Option 3 = 5* (push means consumers never poll; rotated creds
  arrive instantly). *Options 1/2/4 = 4* (sync getter / kernel-map read — fast, in-process,
  O(1)). *Option 5 = 3* (View-backed read may touch the ViewStore read path / key-split
  indirection). *Option 6 = 2* (held-set correctness gated on gossip convergence; a read
  during reconvergence may see a stale/absent entry).

---

## Phase 3 (cont.): Weighted totals

DVF normalized to a 1–5 scale = (DVF total ÷ 3). Weighted total = Σ(score × weight),
weights = {DVF 30%, T1 20%, T2 20%, T3 15%, T4 15%}.

| Option | DVF/3 | T1 (20%) | T2 (20%) | T3 (15%) | T4 (15%) | **Weighted total** | Rank |
|---|---|---|---|---|---|---|---|
| **1** Shared `Arc` + reconciler + actions | 5.00 | 4 | 4 | 5 | 4 | **4.45** | **1** |
| **2** No new reconciler (fold) | 4.00 | 5 | 4 | 4 | 4 | **4.20** | **2** |
| **3** `watch`-channel push | 4.67 | 2 | 3 | 2 | 5 | **3.45** | 3 |
| **6** Observation-row rebuild | 3.00 | 3 | 3 | 3 | 2 | **2.85** | 4 |
| **5** View-as-store | 3.00 | 3 | 2 | 3 | 3 | **2.80** | 5 |
| **4** Kernel `IDENTITY_MAP` | 3.00 | 2 | 2 | 2 | 4 | **2.60** | 6 |

**Worked totals (the arithmetic, shown so the ranking is auditable):**

- **Option 1** = (5.00×.30)+(4×.20)+(4×.20)+(5×.15)+(4×.15) = 1.500+.800+.800+.750+.600 = **4.45**
- **Option 2** = (4.00×.30)+(5×.20)+(4×.20)+(4×.15)+(4×.15) = 1.200+1.000+.800+.600+.600 = **4.20**
- **Option 3** = (4.67×.30)+(2×.20)+(3×.20)+(2×.15)+(5×.15) = 1.401+.400+.600+.300+.750 = **3.45**
- **Option 6** = (3.00×.30)+(3×.20)+(3×.20)+(3×.15)+(2×.15) = .900+.600+.600+.450+.300 = **2.85**
- **Option 5** = (3.00×.30)+(3×.20)+(2×.20)+(3×.15)+(3×.15) = .900+.600+.400+.450+.450 = **2.80**
- **Option 4** = (3.00×.30)+(2×.20)+(2×.20)+(2×.15)+(4×.15) = .900+.400+.400+.300+.600 = **2.60**

The top two — **Option 1 (4.45)** then **Option 2 (4.20)** — are stable under any
reasonable rounding; the 4/5/6 band (2.60–2.85) is well separated below them.

---

## Phase 4: Top-3 analysis

### Option 1 — Shared `Arc<IdentityMgr>` + `SvidLifecycle` reconciler + actions — **4.45**

- **Why it scores well:** Top DVF (exact mirror of the shipped `ServiceMapHydrator` →
  `Action` → executor pattern; reconciler stays pure; `BTreeMap`; port-trait-clean;
  matches whitepaper §7 `Arc<IdentityMgr>` verbatim). Top Progressive Disclosure (first
  use is exactly issue-hold-read-drop; #40 rotation is a later-revealed
  `Action::StartWorkflow` branch). Strong Subtraction/Concept-Count (one new convergence
  target, one familiar concept).
- **Core trade-off:** Adds a new struct + reconciler + 2 actions + 2 executors — slightly
  more new surface than Option 2's "fold." Pays a small Subtraction cost for a clean
  separation of concerns.
- **Key risk (must be true):** That identity deserves its **own** convergence target
  (not folded into `WorkloadLifecycle`) — i.e. that the per-reconciler-View boundary and
  the future 4.7 ACME-lane extension justify a dedicated reconciler. Evidence says yes
  (SPIRE/istio both keep identity in a *dedicated* manager, not the workload supervisor).
- **Hire criteria:** A maintainer chooses this when they want identity to be an
  independently-testable, independently-evolvable subsystem with a clean seam for #40
  rotation and 4.7 ACME — the "do it the way the rest of the platform's reconcilers are
  done" choice.

### Option 2 — No new reconciler (fold into `WorkloadLifecycle` / executor) — **4.20**

- **Why it scores well:** Top Subtraction (fewest new mechanisms — no new reconciler,
  possibly no new Action). Strong Concept-Count and Speed. The thinnest possible wiring.
- **Core trade-off:** Couples identity lifecycle into `WorkloadLifecycle`'s reconcile body
  / the alloc executor. The X1 "executor side-effect, no Action" end weakens DST
  observability of issue/drop (issuance becomes an un-actioned side effect, contrary to
  the ADR-0023 spirit that *every* cluster-affecting effect is a typed `Action`).
- **Key risk (must be true):** That identity and workload lifecycle are *the same concern*
  and won't diverge — e.g. that 4.7 (ACME gateway certs, a *non-allocation* credential)
  won't need an identity path that has nothing to do with `WorkloadLifecycle`. The
  whitepaper's "unified `IdentityMgr` across SVID + ACME" suggests they *will* diverge,
  which is the case against folding.
- **Hire criteria:** A maintainer chooses this under strong time/surface pressure, or if
  the platform were *certain* identity will never need a path independent of the workload
  lifecycle (it isn't — 4.7 contradicts that).

### Option 3 — `watch`-channel push read surface — **3.45**

- **Why it scores well:** Top Speed-as-Trust (push, no poll) and the strongest **#40
  rotation-seam** alignment — a rotated SVID pushes down the same channel, exactly the
  istio-SDS no-restart-swap precedent (research L2).
- **Core trade-off:** Adds a whole async-channel subsystem (low Subtraction/Progressive-
  Disclosure) **before any consumer exists to use it** — #26 sockops, the gateway, and
  telemetry are all unbuilt. Channel state across `.await` invites the lock-across-await /
  production-shaped-by-simulation hazards the rules warn about.
- **Key risk (must be true):** That consumers need *push* notification now — but with no
  consumer built, the push channel is speculative; a sync getter can be *upgraded* to a
  watch channel when #26/gateway land and a real need appears.
- **Hire criteria:** A maintainer chooses this once a real consumer demands change-
  notification, or to pre-build the #40 rotation seam — i.e. it is the natural **evolution
  of Option 1**, not a competing foundation.

---

## Recommendation (derivable from the matrix)

**Recommend Option 1** — Shared `Arc<IdentityMgr>` store + a standalone `SvidLifecycle`
reconciler emitting `Action::IssueSvid` / `Action::DropSvid`, with an action-shim executor
that calls the shipped `ca_issuance::issue_and_audit` and writes the held store; consumers
read via sync getters (with the `IdentityRead` port-trait as a recommended read-surface
refinement — the folded-M dimension). **Weighted total 4.45, rank 1, no weight adjustment
required** — the recommendation follows the matrix directly.

This is the honest expected outcome the dispatch anticipated: DIVERGE **confirms** the
issue-pinned direction (standalone `SvidLifecycle` reconciler + `Arc<IdentityMgr>` +
persist-inputs + the `Action::StartWorkflow` rotation seam deferred to #40), because that
direction *is* the highest-taste choice once the binding correctness rules (reconciler
purity → emit-Action-not-call-CA; state-layer hygiene; single-cut) and the shipped
primitives (`ServiceMapHydrator` pattern, `ca_issuance::issue_and_audit`) are taken as
given. The competitive research independently lands on the same convergent shape
(shared identity-keyed in-memory store; rotation as a decoupled trigger that updates the
store — research L1/L3/L4). The recommendation is **not** manufactured contrarianism; it
is what the job → research → scores actually produce.

### Documented dissent — when Option 2 wins instead

**Option 2 (4.20) is the closest contender** and would be the right call under one
specific condition: **if the team is under hard surface/time pressure for #35 AND is
willing to accept that identity never needs a lifecycle independent of the workload
allocation.** Option 2's Subtraction advantage (5 vs 4) is real — it ships less code. The
case *against* it (and why Option 1 wins by 0.25) is **4.7**: the whitepaper commits
`IdentityMgr` to *also* hold ACME gateway certs (a credential with no `WorkloadLifecycle`
allocation behind it). Folding identity into `WorkloadLifecycle` (Option 2) builds a seam
that 4.7 would have to *unbuild* — a single-cut violation deferred, not avoided. Option 1
keeps identity as its own subsystem, so 4.7 plugs a second lane into the *same* store
without touching the workload supervisor. **Option 2 wins only if 4.7's unified-store
commitment is dropped or deferred indefinitely** — which is a product decision above this
wave's pay grade and is surfaced as such.

A secondary dissent: **Option 3 is the right *next* step, not a competing foundation.** If
DISCUSS/DESIGN decides the #40 rotation seam should be pre-wired now (rather than left as
a `StartWorkflow` no-op), Option 1's sync getter becomes a `watch` channel — Option 1 and
Option 3 compose; they do not compete. The recommendation is therefore "Option 1 now,
with the read surface behind an `IdentityRead` port so the getter→watch upgrade is a
non-breaking internal change when a consumer needs it."

### Decision statement for DISCUSS

> **Proceed with Option 1** — a standalone `SvidLifecycle` reconciler emitting typed
> `Action::IssueSvid` / `Action::DropSvid`, an action-shim executor that mints via the
> shipped `ca_issuance::issue_and_audit` and writes a shared `Arc<IdentityMgr>`
> (`parking_lot::RwLock<BTreeMap<AllocationId, SvidMaterial>>` + the current trust bundle),
> consumers reading via sync getters behind an `IdentityRead` port trait, the View
> persisting **issuance inputs** (not derived `expires_at`) for restart-idempotence, and
> the near-expiry rotation seam left as a deferred `Action::StartWorkflow(cert_rotation)`
> that is a **no-op until #40** — **assuming** (key risk) that identity warrants its own
> convergence target separate from `WorkloadLifecycle`, which the whitepaper's unified-
> `IdentityMgr`-across-SVID+ACME commitment (4.7) supports. If the 4.7 unified-store
> commitment is dropped, re-evaluate Option 2.

---

## Gate G4 evaluation

- [x] **DVF filter applied** — all 6 scored on D/V/F; elimination threshold (<6) checked
      (none eliminated; rationale documented). **PASS.**
- [x] **Weights locked before scoring** — § Phase 2 weights table frozen with pre-scoring
      adjustment rationale, *then* scores assigned. **PASS.**
- [x] **All surviving options scored on all 4 criteria** — full 6×4 matrix + per-criterion
      rationale. **PASS.**
- [x] **Weighted ranking complete** — exact worked totals; top-2 stable. **PASS.**
- [x] **Recommendation traceable to scores** — Option 1 is rank 1 (4.45), no weight
      adjustment needed; the matrix → recommendation link is explicit. **PASS.**
- [x] **Dissenting case included** — Option 2 dissent (wins iff 4.7 unified-store dropped)
      + Option 3 "next-step-not-competitor" secondary dissent. **PASS.**
- [x] **Decision statement for DISCUSS explicit** — § Decision statement, with the key-risk
      assumption named. **PASS.**

**Phase 4 gate: PASS.**
