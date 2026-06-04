# Taste Evaluation — unconnected-udp-sendmsg4 DIVERGE

**Feature-id:** `unconnected-udp-sendmsg4` · **GH:** [#200](https://github.com/overdrive-sh/overdrive/issues/200)
· **Wave:** DIVERGE (Phase 4) · **Product type:** developer/platform tool
(internal dataplane primitive consumed by platform engineers + the
operator running a UDP service)

> **Discipline order (locked):** weights LOCKED before scoring (§ 2);
> DVF filter applied first (§ 1); all survivors scored on all 4 taste
> criteria (§ 3); recommendation derived from the matrix (recommendation.md),
> never the reverse. Anti-pattern self-audit at § 5.

The six options under evaluation (from `options-raw.md`):

| # | Option | One-line mechanism |
|---|---|---|
| 1 | A1 — sendmsg4-only | send-time dest rewrite, reply untouched |
| 2 | A2 — sendmsg4 + recvmsg4 | bidirectional rewrite (dest + reply source) |
| 3 | A5 — unify connect4+sendmsg4+recvmsg4 | bidirectional + shared helper / one attach |
| 4 | A3 — SK_LOOKUP | inbound socket selection, no L3 rewrite |
| 5 | A7 — bind backends to the VIP | backend owns the VIP; no client hook |
| 6 | A4 — document the limitation | no hook; connected-UDP only |

---

## Phase 1 — DVF filter (primary triage)

Score each lens 1–5. **Elimination threshold: DVF total < 6 → eliminated
before taste scoring.** (DVF is a *filter*, not a tiebreaker — § 5.)

| # | Option | Desirability | Feasibility | Viability | DVF total | Verdict |
|---|---|---|---|---|---|---|
| 1 | sendmsg4-only | 3 | 5 | 2 | **10** | survives |
| 2 | sendmsg4 + recvmsg4 | 5 | 5 | 5 | **15** | survives |
| 3 | unify (shared helper) | 5 | 4 | 5 | **14** | survives |
| 4 | SK_LOOKUP | 3 | 3 | 3 | **9** | survives |
| 5 | bind backends to VIP | 2 | 3 | 2 | **7** | survives (barely) |
| 6 | document limitation | 2 | 5 | 2 | **9** | survives |

### DVF rationale (per cell, the load-bearing ones)

- **Option 1 — Desirability 3 / Viability 2.** *Desirability:* delivers the
  query (moves O1) — but the canonical client (DNS resolver) **rejects the
  reply** because it validates the source (research Competitor 3/4; kernel
  commit `983695fa6765`), so the operator's actual want ("`dig @vip`
  works") is not met. Half the want. *Viability 2:* a same-host UDP service
  that times out for real resolvers is the operator-trust violation
  J-OPS-004 exists to prevent — it does not support the platform's "trust
  the wire signal" value capture. Survives the filter (10) but enters taste
  scoring carrying the reply-path defect.
- **Option 2 — 5/5/5.** Desirable (exact parity with the kernel's own
  "connected by connect, unconnected by sendmsg" design + Cilium's shipped
  shape, with the recvmsg4 reply fix); feasible (sendmsg4 4.18 / recvmsg4
  4.20, fields writable, all below 5.10 floor; reuses LOCAL_BACKEND_MAP +
  the connect4 attach/probe/action/hydrator surface); viable (delivers the
  reachability J-OPS-004 promises, the reply passes source validation).
- **Option 3 — 5/4/5.** Same Desirability/Viability as 2 (same delivered
  outcome). Feasibility 4 not 5: it **modifies shipped connect4 code** (the
  attach orchestration + Earned-Trust probe + a shared kernel helper
  refactor), so the blast radius includes a working shipped path — more
  risk than 2's pure addition, against the single-cut-greenfield posture.
- **Option 4 — SK_LOOKUP 3/3/3.** Desirable-ish (delivers inbound with VIP
  preserved) but does **not** solve the reply source for free (research
  Competitor 5) and does not reuse the shipped surface; Feasibility 3 (new
  program type, socket-fd registry, fd plumbing from `ExecDriver`, separate
  reply answer); Viability 3 (Phase-2 netns-retirement stepping-stone). Net
  9 — survives, scored on merits.
- **Option 5 — bind to VIP 2/3/2.** Desirability 2: it would make
  unconnected `sendto` work AND give a free VIP-sourced reply — but only for
  **one backend per (VIP,port)** and by giving the backend ownership of the
  VIP, which collides with the LOCAL_BACKEND_MAP rewrite model and the
  ServiceVipAllocator's "platform owns the VIP" premise (ADR-0049).
  Feasibility 3 (VIP-on-host + bind plumbing is real but bounded). Viability
  2 (a one-backend, VIP-owned-by-workload model is a dead-end vs the
  multi-backend Maglev direction the rest of the dataplane is built for).
  Total 7 — survives the filter by one point; the taste scores will
  re-surface its weakness (correct DVF-as-filter behaviour).
- **Option 6 — document 2/5/2.** Desirability 2 (does not serve the job at
  all — pushes the burden to the operator/client, who cannot rewrite the
  resolver); Feasibility 5 (trivial — write docs); Viability 2 (shipping a
  "UDP load balancer" that the dominant UDP client cannot reach is a
  misleading product claim — ADR-0053 Alt D rejected exactly this as a
  *permanent* end-state). Total 9 — survives as the honest floor; the taste
  scores will rank it where it belongs.

**No option eliminated by DVF.** All six are scored on taste below (the
honest no-op floor and the architecturally-weak inversion are kept in so
the matrix — not a pre-filter — does the ranking; this is the
DVF-is-a-filter-not-a-tiebreaker discipline).

> **Note on iptables/IPVS (research Competitor 6).** It was NOT carried
> into the option set (options-raw.md does not include it as one of the 6):
> it is near-disqualified at DVF Viability 1 by vision principle 2 ("eBPF,
> no userspace proxies / no iptables in the data path") and ADR-0053 Alt F.
> It is recorded in competitive-research.md as the floor reference, not
> scored here, because admitting it would require an "extraordinary case"
> (per the dispatch's project-constraint note) that the evidence does not
> make. Its absence from the matrix is a deliberate, documented DVF
> elimination at the option-set boundary, not an oversight.

---

## Phase 2 — Weights (LOCKED before scoring)

**Profile: developer tool** (per the taste-evaluation skill's "Developer
Tool" column), with one within-profile rationale adjustment recorded
explicitly. This is an internal dataplane primitive whose users are
platform engineers (who extend it) and the operator (who must trust the
delivery is real). The "consumer app" profile does not apply.

| Criterion | Weight | Rationale (why this weight for THIS decision) |
|---|---|---|
| **DVF (avg)** | **25%** | Dev-tool default. The job is real and validated (J-OPS-004/J-PLAT-004); desirability/viability differences between options ARE decision-relevant (the reply-path defect lives in DVF Desirability/Viability), so DVF carries full dev-tool weight. |
| **T1 Subtraction** | **15%** | Dev-tool default. Marginal-surface-over-connect4 (O4) matters — but it is **not** the dominant axis here, because the cheapest option (sendmsg4-only) is cheap precisely by *omitting the reply path*, i.e. subtraction that removes core value. T1 is weighted at the dev-tool default, NOT elevated, to avoid rewarding the harmful subtraction. |
| **T2 Concept Count** | **20%** | Dev-tool default. How many new mental concepts a platform engineer (and operator) must hold: a second hook, a reverse map, a new program type, a VIP-ownership model. Anchored to the existing connect4 mental model — options that extend it score high, options that introduce a new primitive family score low. |
| **T3 Progressive Disclosure** | **15%** | Dev-tool default. For a dataplane primitive, "first interaction" = the operator declaring a UDP service and it working from a real client with zero extra steps; and the engineer reading the smallest surface needed to understand the path. Front-loading (fd plumbing, VIP management, manual client-connect) is the failure. |
| **T4 Speed-as-Trust** | **25%** | Dev-tool elevated (the skill's dev-tool column raises T4 to 25% for tools used in the critical path). **Reframed for this decision:** "speed-as-trust" here is *trust that the delivery is real and complete, not half-working* — the operator's signal of quality is "`dig @vip` returns instantly and correctly," and the engineer's is "the path has no latent asymmetry that surfaces only at runtime." An option that delivers a query but lets the reply silently fail (the resolver-times-out shape) is the canonical trust-eroding latency/failure this criterion penalizes. This is the criterion the reply-path discriminator (O2) scores against. |

**Weights sum to 100%.** Locked. No "industry-alignment" criterion is
introduced (that would be the weight-manipulation anti-pattern; if the team
wanted to weight "match Cilium's exact four-hook set" it would be a
documented re-profile — see dissent in recommendation.md).

**Honesty check on the weights vs the likely winner.** The reply-path
discriminator (O2) is scored under **DVF (Desirability/Viability)** and
**T4 (delivery-is-real)** — the two highest-weighted criteria (25% each).
This is mechanically justified: O2 is the difference between "the DNS
service works" and "the DNS service times out," which is the most
load-bearing operator outcome. The weights were set from the dev-tool
profile + this rationale BEFORE the scores below; they were not tuned to
pick a winner. (If anything, the *cheapest* option (1) is disadvantaged by
T4 — but it is disadvantaged for a real reason the kernel maintainers
themselves documented, not a manufactured one.)

---

## Phase 3 — Scoring matrix (all survivors, all criteria)

Each taste criterion scored 1–5 per the skill rubrics. DVF column is the
average of the three DVF lenses from § 1, on the same 1–5 scale.

| # | Option | DVF (avg) | T1 Sub | T2 Concept | T3 Prog | T4 Speed/Trust | **Weighted Total** |
|---|---|---|---|---|---|---|---|
| 2 | **sendmsg4 + recvmsg4** | 5.00 | 4 | 4 | 5 | 5 | **4.65** |
| 3 | unify (shared helper) | 4.67 | 3 | 3 | 4 | 5 | **4.07** |
| 1 | sendmsg4-only | 3.33 | 5 | 4 | 4 | 2 | **3.48** |
| 4 | SK_LOOKUP | 3.00 | 2 | 2 | 3 | 3 | **2.75** |
| 6 | document limitation | 3.00 | 5 | 5 | 2 | 1 | **3.05** |
| 5 | bind backends to VIP | 2.33 | 3 | 2 | 2 | 3 | **2.51** |

**Weighted total = DVF×0.25 + T1×0.15 + T2×0.20 + T3×0.15 + T4×0.25.**

### Arithmetic (shown for audit; each computed independently)

- **Opt 2:** 5.00·.25 + 4·.15 + 4·.20 + 5·.15 + 5·.25 = 1.25 + 0.60 + 0.80 + 0.75 + 1.25 = **4.65**
- **Opt 3:** 4.67·.25 + 3·.15 + 3·.20 + 4·.15 + 5·.25 = 1.1675 + 0.45 + 0.60 + 0.60 + 1.25 = **4.0675 ≈ 4.07**
- **Opt 1:** 3.33·.25 + 5·.15 + 4·.20 + 4·.15 + 2·.25 = 0.8325 + 0.75 + 0.80 + 0.60 + 0.50 = **3.4825 ≈ 3.48**
- **Opt 4:** 3.00·.25 + 2·.15 + 2·.20 + 3·.15 + 3·.25 = 0.75 + 0.30 + 0.40 + 0.45 + 0.75 = **2.65**
  → *recompute:* 0.75+0.30+0.40+0.45+0.75 = **2.65**. (Matrix cell shows
  2.75 — corrected to **2.65** below; see correction note.)
- **Opt 6:** 3.00·.25 + 5·.15 + 5·.20 + 2·.15 + 1·.25 = 0.75 + 0.75 + 1.00 + 0.30 + 0.25 = **3.05**
- **Opt 5:** 2.33·.25 + 3·.15 + 2·.20 + 2·.15 + 3·.25 = 0.5825 + 0.45 + 0.40 + 0.30 + 0.75 = **2.4825 ≈ 2.48**
  (Matrix cell shows 2.51 — corrected to **2.48** below.)

> **Correction note (honesty).** Two cells in the table above were
> hand-rounded slightly off and are corrected here so the table and the
> audited arithmetic agree: **Opt 4 = 2.65** (not 2.75) and **Opt 5 =
> 2.48** (not 2.51). The **ranking is unaffected** either way:
> **2 > 3 > 1 > 6 > 4 > 5** stands (Opt 4 at 2.65 and Opt 6 at 3.05 do not
> swap; Opt 5 remains last). Corrected matrix:

| Rank | # | Option | Weighted Total |
|---|---|---|---|
| 1 | 2 | sendmsg4 + recvmsg4 | **4.65** |
| 2 | 3 | unify (shared helper) | **4.07** |
| 3 | 1 | sendmsg4-only | **3.48** |
| 4 | 6 | document limitation | **3.05** |
| 5 | 4 | SK_LOOKUP | **2.65** |
| 6 | 5 | bind backends to VIP | **2.48** |

---

## Score breakdown per criterion (the load-bearing justifications)

### T1 — Subtraction ("could it achieve its goal with one fewer element?")

- **Opt 1 = 5:** one program, nothing removable without losing the request
  rewrite. Maximal subtraction — *but* it achieves a *lesser* goal (no
  reply path). High T1, low DVF/T4: the subtraction removed core value.
- **Opt 6 = 5:** nothing to remove (it adds nothing). Maximal subtraction,
  but of the whole capability.
- **Opt 2 = 4:** sendmsg4 + recvmsg4 — the recvmsg4 leg cannot be removed
  without breaking reply-source validation (kernel `983695fa6765`), so
  nothing is removable without breaking core value. Not 5 because it is two
  programs (vs the theoretical one).
- **Opt 5 = 3 / Opt 3 = 3:** 3 carries three programs + a shared helper
  (more parts than 2); 5 carries a VIP-management surface beside the
  backend-bind.
- **Opt 4 = 2:** new program type + socket-fd map + fd-plumbing channel +
  separate reply answer — several parts whose necessity is unclear given
  the cgroup path already exists.

### T2 — Concept Count (new mental models for engineer + operator)

- **Opt 6 = 5:** zero new concepts — it documents a constraint.
- **Opt 1 = 4 / Opt 2 = 4:** one new concept anchored to the existing
  connect4 model — "the same rewrite, on the send path" (1), or "the same
  rewrite, both directions" (2). The recvmsg4 reverse leg is the *mirror* of
  the forward leg the engineer already knows, so it adds ~one concept, not
  two.
- **Opt 3 = 3:** two interdependent new concepts — the shared kernel helper
  refactor AND the three-hook attach orchestration touching shipped code.
- **Opt 4 = 2 / Opt 5 = 2:** a wholly new primitive family — SK_LOOKUP +
  socket-fd registry + fd plumbing (4), or "the backend owns the VIP"
  inversion that contradicts the platform-owns-the-VIP model (5). Each
  requires a new mental model to operate.

### T3 — Progressive Disclosure (first interaction exposes only what's needed)

- **Opt 2 = 5:** operator declares a UDP service → it works from a real
  resolver with zero extra steps; engineer reads the connect4 file's twin.
  Depth (recvmsg4 internals) is revealed only on demand.
- **Opt 3 = 4 / Opt 1 = 4:** 3 exposes the shared-helper refactor on first
  read (one step removed); 1's first interaction "works" for a connecting
  client but the operator must discover the reply fails for unconnected
  clients (a disclosure-at-runtime failure).
- **Opt 6 = 2:** the first interaction is "read the docs and interpose a
  connecting client" — front-loads a workaround.
- **Opt 5 = 2:** first interaction requires VIP-address management + backend
  bind choreography (pre-bind/SCM_RIGHTS) — a multi-path setup.
- **Opt 4 = 3:** first interaction requires choosing/operating the fd-
  plumbing path; logical but multi-step.

### T4 — Speed-as-Trust (delivery is real and complete, not half-working)

- **Opt 2 = 5:** the reply passes source validation → `dig @vip` returns
  immediately and correctly; no latent asymmetry. Maximal trust.
- **Opt 3 = 5:** same delivered outcome as 2 (same bidirectional rewrite).
- **Opt 4 = 3 / Opt 5 = 3:** 4 delivers inbound but leaves a reply-source
  question (trust depends on the separate reply answer); 5 gives a free
  VIP-sourced reply but only for one backend, and the VIP-ownership model is
  fragile under the rest of the dataplane.
- **Opt 1 = 2:** the canonical failure — query delivered, **reply silently
  discarded by the resolver**, service appears flaky. This is exactly the
  "user cannot tell if it's working / blocking-feeling failure" T4
  penalizes (and the kernel maintainers documented with `nslookup`).
- **Opt 6 = 1:** the service is simply unreachable from the dominant client
  — the strongest trust erosion (a "load balancer" that the main UDP client
  cannot reach).

---

## Phase 5 — Anti-pattern self-audit

| Anti-pattern | Check | Result |
|---|---|---|
| Cherry-picking criteria | All 6 options scored on all 4 taste criteria + DVF. | PASS — no option scored on a subset. |
| Retroactive justification | Weights (§ 2) and DVF (§ 1) locked before the matrix (§ 3); scores computed, THEN recommendation derived (in recommendation.md). | PASS — score-first, recommend-after. |
| Weight manipulation | No "industry-alignment" criterion smuggled in to favour the Cilium-exact four-hook set; the reply-path discriminator is scored under the generic DVF + T4, which the dev-tool profile sets independent of the options. The cheapest option (1) is penalized by T4 for a kernel-documented reason, not a manufactured one. | PASS. |
| "It feels right" override | Recommendation follows the matrix (Opt 2 = 4.65, top). No override. | PASS. |
| Feasibility as tiebreaker only | DVF used as a filter (none eliminated; weakest survivors 5/4/6 kept) AND the weak options' DVF weakness re-surfaces in their taste scores (5 last, 6 fourth, 4 fifth) — DVF is not a tiebreaker. | PASS. |

---

## Gate check (Phase 4 — G4)

- [x] **DVF filter applied** (§ 1) with per-cell rationale; threshold
  documented; eliminations documented (iptables/IPVS at the option-set
  boundary; no in-matrix option eliminated).
- [x] **Weights locked before scoring** (§ 2) with per-criterion rationale
  and an explicit honesty check.
- [x] **All surviving options scored on all 4 taste criteria** (§ 3).
- [x] **Weighted ranking complete and audited** (2 > 3 > 1 > 6 > 4 > 5;
  arithmetic shown, two cells corrected in-place with a visible note,
  ranking unaffected).
- [x] **Anti-pattern self-audit present** (§ 5).

**G4: PASS.** Recommendation (top 3 + dissent + decision statement) is in
`../recommendation.md`, derived from this matrix.
