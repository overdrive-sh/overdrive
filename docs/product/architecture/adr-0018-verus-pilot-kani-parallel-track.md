# ADR-0018 — Verus pilot on cert-rotation reconciler; Kani parallel track for `unsafe` interior; whitepaper §18 unchanged pending pilot

## Status

Proposed. 2026-04-23.

## Context

Whitepaper §18 commits Overdrive's reconcilers to **Eventually Stable
Reconciliation (ESR)** and workflows to **deterministic replay +
bounded progress**, explicitly naming "USENIX OSDI '24 *Anvil*
demonstrates this is mechanically checkable in Verus against a Rust
implementation" as the evidence anchor. `.claude/rules/testing.md`
further commits: "First-party reconcilers ship with ESR
specifications" and "Both obligations are gated by DST (§21).
Reconcilers additionally have an Anvil-style ESR verification target."

The commitment is public. As of 2026-04-23, Overdrive has *zero*
proved reconcilers. Whether the commitment can be met on Overdrive's
actual code, schedule, and engineering bench is an open question that
the research doc
`docs/research/verification/verus-for-overdrive-applicability-research.md`
(Nova, 2026-04-23) investigates in depth.

The research's verdict is unambiguous: **Experiment — do not commit
outright, do not defer indefinitely**. The three reasons:

- **Commit outright is not justified.** Anvil's proof-to-code ratios
  (4.5–7.4×), first-controller investment (2 person-months), and
  subsequent-controller investment (2 person-weeks) come from a team
  that **included Verus co-authors** (Finding 4.2). The cost profile
  for a team without that background is unquantified (research Gap 4).
  Verus async support was merged 2026-04-10 and a basic `Send`-bounded
  Future call currently ICEs (Finding 1.5). Verifying workflow
  replay-equivalence today would be riding research-grade tooling in
  production.
- **Defer indefinitely is not justified.** Anvil's evidence is that ESR
  **precludes 69% of the bug classes detected by state-of-the-art
  fault-injection testing of Kubernetes controllers** (Finding 2.7).
  Deferring means forgoing that defense. Proof automation research
  (AutoVerus, RAG-Verus, AlphaVerus — Finding 4.6) is on a rapidly
  improving trajectory that Overdrive forfeits if it stays
  disengaged.
- **Pivot to an alternative tool is not justified.** Creusot has no
  ESR-shape case study and no async support (Finding 3.1). Kani
  cannot express ESR — it is a bounded model checker (Finding 3.2).
  Prusti is maintenance-stalled (Finding 3.3). None of the
  alternatives can carry the ESR obligation.

The research recommends a *four-engineer-week pilot* on the
certificate-rotation reconciler with explicit pass/fail criteria, and
a *parallel Kani investment* on `unsafe` interior in `overdrive-bpf`
and `overdrive-fs`. GH issue #127 tracks five revisit triggers; this
ADR codifies both the pilot plan and the trigger set.

## Decision

### 1. Verus pilot — cert-rotation reconciler, 4 engineer-weeks, explicit gates

**Target**: The certificate-rotation reconciler from whitepaper §18
"Built-in Primitives." Not the certificate-rotation *workflow* — the
reconciler that drives SVID issuance, expiry tracking, and rotation
scheduling. Sync by whitepaper contract (ADR-0013). Anvil's FluentBit
precedent is the closest analog in the published evidence base (§7
incremental-verification case study).

**Budget**: 4 engineer-weeks (= 2× Anvil's reported 2 person-months
for first controller, halved on the expectation that Anvil's TLA
embedding can be consumed as a dependency rather than re-derived; see
Gap 5 in research).

**Sub-workspace layout**:

```
crates/overdrive-reconcilers-verified/
  cert-rotation/
    rust-toolchain.toml          # pins Verus's supported toolchain
    Cargo.toml                   # crate_class = "core" per ADR-0003
    src/
      reconcile.rs               # the verified reconciler body
      spec.rs                    # ESR specification
    proof/                       # Verus proof structure
    tests/
      esr_specs.rs               # Verus-spec-name round-trip against
                                 # overdrive-invariants::InvariantName
```

The sub-workspace isolates Verus's rustc-fork toolchain from the main
workspace (research Finding 1.6) — the main workspace tracks stable
Rust unchanged. CI invokes Verus via a new `cargo xtask verify`
subcommand (Finding 4.3).

**Pilot pass criteria** (evaluate at end of 4 engineer-weeks):

1. **ESR proved**. The fundamental "it works" checkpoint.
2. **Proof-to-code ratio ≤ 10×**. Loosened from Anvil's 4.5–7.4× to
   account for learning curve. Exceeded → tool is not productive for
   Overdrive-shape code.
3. **Verification wall-clock ≤ 20 minutes** on CI-class hardware.
   Within nightly budget (research Finding 4.3 — Anvil runs 154–520s
   per controller parallel; Overdrive allowance is generous).
4. **Concrete bug caught** — seeded (a deliberately introduced
   regression) or real (a bug the DST suite did not catch). Validates
   the 69% bug-class claim holds for Overdrive-shape code, not just
   Kubernetes controllers.
5. **Incremental proof cost ≤ 2× feature code cost** on one added
   feature. Projects ongoing maintenance burden within budget
   (Finding 4.5 — Anvil FluentBit measured 1.1× per-feature ratio).

**Pilot fail criteria** (any one triggers halt-and-reassess):

1. No ESR proof after 4 engineer-weeks elapsed.
2. Trait-object refactor required across `overdrive-core` (research
   Finding 1.7 — Verus has known `dyn Trait` panics; Anvil uses
   static dispatch). If the cert-rotation reconciler pilot requires
   >200 LOC of non-proof refactor in `overdrive-core` to accommodate
   static trait dispatch, the blast radius is unacceptable.
3. Proof brittleness: >20% of reconciler code changes require proof
   rewrites disproportionate to the code change. The "proof flake"
   failure mode (research cites Cazamariposas CADE '25 as evidence
   proof flakes are a named research problem).

### 2. Kani parallel track — `unsafe` interior, 5–10 harnesses, 1 engineer-month

**Target**: `unsafe` blocks in `overdrive-bpf` (aya-rs map wrappers)
and `overdrive-fs` (chunk-store raw pointers, rkyv archived-access).

**Budget**: 1 engineer-month (research Finding 3.2 — AWS Firecracker
ships 27 Kani harnesses in 15-minute CI; 5–10 harnesses for Overdrive
is a scoped first bite).

**Layout**: Kani harnesses live in the owning crate's `tests/` under
a `kani` feature flag, per the AWS Firecracker pattern. `cargo xtask
kani` runs them; nightly CI consumes (not per-PR — Firecracker's CI
is 15 minutes for 27 harnesses, which is nightly territory for a
project of Overdrive's scope).

**Go/no-go independent of Verus pilot**. The research is explicit
(Executive Summary): "None of the outcomes leaves Overdrive worse off
than the 'defer indefinitely' status quo." Kani's AWS production
precedent (Finding 3.2) is direct evidence that the investment pays
off for `unsafe` code regardless of how the Verus pilot concludes.

**Kani pass criteria**:

1. **≥5 harnesses land with no-panic proofs**. Minimum to demonstrate
   the pattern.
2. **CI wall-clock ≤ 15 minutes** for the harness suite. Matches
   Firecracker's operating envelope.
3. **One concrete `unsafe`-code panic caught** (seeded or real).

### 3. Whitepaper §18 — unchanged pending pilot

The current whitepaper text reads (verbatim, §18 Correctness
Guarantees):

> USENIX OSDI '24 *Anvil* demonstrates this is mechanically checkable
> in Verus against a Rust implementation. First-party reconcilers
> ship with ESR specifications; WASM extensions declare ESR
> preconditions the runtime enforces at load time.

**This ADR does not propose whitepaper edits.** The commitment
remains. The ADR backstops it with a pilot plan. If the pilot
succeeds, the text is validated by evidence. If the pilot fails, a
*separate* superseding ADR will propose softening language based on
the concrete failure modes encountered; the research doc offers a
draft of that softening (§Q6) for reference, but this ADR does not
pre-authorise its use.

**Rationale for not pre-softening**: softening before evidence is a
credibility cost for a speculative gain. The pilot is explicitly
designed to be bounded in scope and calendar; the downside-case write-
off is 4 engineer-weeks, not months of lost commitment. Softening now
signals "we never meant it"; softening after a pilot signals "we tried
and learned."

### 4. TLA embedding — evaluate, defer the port/rebuild decision

Anvil's TLA embedding is 85 lines at core, inside 5353 lines of
reusable lemmas (Finding 4.2, Gap 5). Porting vs reusing vs
re-implementing has different cost profiles. Before the pilot kicks
off, a 2-day spike assesses whether the Anvil TLA library (`anvil/src/
temporal_logic/` and `anvil/src/state_machine/`) can be consumed
directly as a Rust dependency, or must be forked.

**This ADR does not decide port-vs-rebuild**. The 2-day spike is a
pilot input; the actual decision is made at spike-end and recorded
inline in the pilot dev journal. A future ADR can ratify the outcome
if it turns out to be long-lived enough to warrant one.

**Why defer**: the research doc is explicit (Gap 5): "Whether the
lemmas are Kubernetes-specific or generally reusable is not clearly
documented. Porting vs reusing has different cost profiles." Forcing
a decision in this ADR ahead of the spike's evidence is
architectural bet-making without grounds.

### 5. Revisit triggers — five, tracked in GH #127

The research doc's recommendation is to re-evaluate the Verus stance
on any of the following triggers. This ADR codifies them inline for
durability (GH issues can be closed, renumbered, or lost):

1. **A Verus-verified async production case study is published** with
   tokio-backed futures of the `Workflow` shape. Lifts the research
   Finding 1.5 block on workflow verification.
2. **Verus GitHub issues #2321 and #2323** (unit-future name bug and
   `Send`-bounded Future ICE) are both resolved. Lifts the basic
   async-production-pattern block.
3. **The Anvil TLA embedding is published as an independent crate**
   (not coupled to `kubernetes_cluster/`). De-risks the Overdrive
   pilot's temporal-logic bootstrap by ~50% per research Gap 5.
4. **The Overdrive Verus pilot completes** (4 engineer-weeks elapsed
   or all pass/fail criteria hit). This is the primary trigger — the
   pilot's outcome drives the whitepaper §18 revision decision.
5. **A new Rust verification tool** (neither Verus, Creusot, Kani,
   nor Prusti) reaches Anvil-class production maturity on async or
   ESR-shape properties. Unlikely in the 12-month window but the
   door stays open.

GH #127 is the authoritative tracker; this ADR is the authoritative
record of the five triggers that GH #127 points at.

## Alternatives considered

### Alternative A — Pilot a different reconciler (not cert-rotation)

Other candidates from whitepaper §18 that were considered:

- **Job-lifecycle reconciler** — Large state space (pending → running
  → draining → terminated → migrating), touches every driver. Too
  much surface for a pilot. **Rejected**: pilot would confound tool
  tractability with scope complexity.
- **Evaluation-broker reaper** — Small, but trivial ESR shape
  ("pending evals eventually drain"). Too small to stress the
  temporal-logic machinery. **Rejected**: would give false confidence
  if the pilot succeeds (the real ESR targets are harder).
- **Operator cert revocation sweep** — Small and bounded, but the ESR
  property is a safety invariant (revoked certs never re-appear)
  rather than the full liveness+safety pair. **Rejected**: does not
  exercise the liveness machinery the research is most uncertain
  about.
- **Cert rotation (chosen)** — Middle complexity, exercises full ESR
  (cert expiry → issuance → propagation → retirement), directly
  analogous to Anvil's FluentBit controller shape. **Accepted**: the
  research recommends it explicitly (§Q6 Pilot design); no counter-
  evidence surfaced in alternatives consideration.

The risk — flagged by the research doc and this ADR openly — is that
cert-rotation is **too easy** and gives false confidence. Mitigation:
pass criterion 4 (concrete bug caught, seeded or real) explicitly
tests whether ESR finds bugs DST does not, on Overdrive-shape code. A
cert-rotation pilot that trivially proves ESR but finds no bugs is a
fail on criterion 4, not a pass.

### Alternative B — Commit to Verus without a pilot

**Rejected.** Research Finding 4.2 shows Anvil's 2 person-months /
2 person-weeks figures come from a Verus-native team. Community
anecdote (research Gap 4) suggests 2–4× for teams without that
background. Committing to verify 15 reconcilers (whitepaper §18
built-ins) at those ratios without a pilot is an 18–36 person-month
bet on unvalidated assumptions. The pilot exists precisely to
convert that bet into measurement.

### Alternative C — Defer Verus indefinitely; rely entirely on DST

**Rejected.** Research Finding 2.7 shows ESR precludes 69% of bug
classes detected by fault-injection testing of Kubernetes
controllers. If that figure translates to Overdrive, DST is covering
≤31% of the reconciler-bug space, and deferring means living with
that gap permanently. The pilot's fail case still costs only 4
engineer-weeks; the defer-indefinitely case costs the ongoing bug
exposure.

### Alternative D — Pivot to Creusot for ESR, Kani for `unsafe`

**Rejected.** Research Finding 3.1: Creusot has no temporal-logic or
ESR-shape case study. "NOT a replacement for Anvil-style ESR
verification — no published Creusot case study in that shape exists."
Pivoting to Creusot for the reconciler obligation loses the Anvil
precedent and gains nothing — Creusot is a candidate only for
*subset* obligations (hash determinism, snapshot roundtrip, `FromStr`
validation) where its trait story is stronger than Verus's. If the
Verus pilot fails, a follow-up ADR can revisit Creusot for those
subset obligations; but on the reconciler-ESR path specifically,
Creusot is not an alternative.

### Alternative E — Prusti

**Rejected.** Research Finding 3.3: Prusti's last release was August
2023 at research time. Maintenance cadence is an order of magnitude
slower than Verus/Creusot/Kani. Not in the Rust Foundation
standard-library verification challenge tool list. A project of
Overdrive's scope should not depend on a tool with stalled
maintenance.

### Alternative F — Sequence pilot then Kani (not parallel)

**Rejected.** The research is explicit that Kani's go/no-go is
independent of the Verus verdict — the evidence base (AWS Firecracker
production adoption, Finding 3.2) is complete without reference to
Verus's outcome. Sequencing would waste one engineer-month of
feasible work on a synthetic dependency. Run both tracks in parallel;
they share no review surface.

### Alternative G — Make the pilot pass criteria looser

**Rejected.** Every pass criterion in §1 above is grounded in a
specific research finding. Loosening criterion 2 (proof-to-code
ratio) risks "we proved it but the cost model is unworkable"; loosen
criterion 4 (concrete bug caught) risks "we proved it but it was
trivial and DST would have caught it." The criteria are already more
permissive than Anvil's reported numbers; further loosening converts
the pilot from a hypothesis test into a confirmation exercise.

## Consequences

### Positive

- **Whitepaper §18 commitment stays credible** by being backstopped
  with a concrete plan that can be executed in a bounded timeframe.
  A public commitment without an execution plan is vapourware;
  with this pilot, it is a measured bet.
- **Four-engineer-week downside case is a known cost**. No
  open-ended verification investment; no sunk cost on proofs of
  unclear value; clean exit if the pilot fails.
- **Kani's Firecracker-pattern `unsafe` verification is a near-term
  win regardless of Verus's outcome**. Overdrive gets better `unsafe`
  safety from month 1 of the parallel track.
- **Principle 12 (Earned Trust) served at the verification-tool
  boundary**. The pilot asks the tool to prove it works on
  Overdrive's shape of code before the project depends on that
  capability. A probe on the verifier itself.
- **Research-trajectory alignment**. Overdrive stays engaged with
  Verus / Anvil improvements (AutoVerus, RAG-Verus, AlphaVerus per
  Finding 4.6) rather than re-engaging cold in 24 months when they
  have moved on.

### Negative

- **4 engineer-weeks is a real budget**. For a project of Overdrive's
  phase-1-foundation scope, that is measurable engineer-time. The
  pilot must be protected from scope creep or it eats the budget
  without producing a verdict.
- **Sub-workspace toolchain pin adds operational burden**. The
  `rust-toolchain.toml` in `crates/overdrive-reconcilers-verified/`
  will drift from the main workspace toolchain; upgrading the main
  workspace does not auto-update the verified sub-workspace. An
  explicit update step is required on every Verus rolling-release
  bump.
- **No async / workflow verification**. This ADR explicitly scopes
  out workflows. The whitepaper §18 commitment on workflow
  replay-equivalence remains a DST-only gate until a future ADR
  lifts the research Finding 1.5 block. Revisit trigger 1 or 2 in
  GH #127 is the signal.
- **Proof maintenance is a PR-review burden once the pilot
  concludes and verified code is in the codebase**. A reconciler
  PR that passes DST but breaks a Verus proof must be caught at
  review. This ADR commits to a policy: **a reconciler PR that
  breaks a verified proof and does not ship a proof update is
  rejected on review**. No "we'll fix the proof later" — the whole
  point of verification is that the proof is always live.

### Quality-attribute impact (ISO 25010)

- **Reliability — fault tolerance**: strongly positive **if the
  pilot succeeds**. ESR verification precludes 69% of bug classes
  per Finding 2.7. Neutral if the pilot fails (DST remains the
  floor).
- **Maintainability — modifiability**: mixed. Verified reconcilers
  carry proof-churn burden on every code change. Anvil's
  FluentBit evidence (1.1× per-feature ratio, Finding 4.5) is
  encouraging but is one case study.
- **Maintainability — testability**: neutral. Verification is
  complementary to testing, not a substitute. DST and real-kernel
  integration stay the floor.
- **Portability — replaceability**: mild negative. Verus binds to
  a rustc fork; the verified sub-workspace cannot be compiled on
  arbitrary Rust toolchains. Mitigated by sub-workspace isolation.
- **Performance efficiency**: neutral. Verus adds no runtime
  overhead (Finding 1.1).

### Enforcement

- **Pilot pass/fail assessment is a structured review, not a
  subjective call**. The pass criteria are numeric (ratios,
  wall-clock, bug count) and the fail criteria are boolean. A
  2-hour pilot retrospective at end of 4 engineer-weeks reviews
  each criterion against evidence; the verdict is written inline
  in the pilot dev journal and linked from this ADR's changelog.
- **Proof maintenance gate on PR review**. A PR touching the
  verified reconciler that does not land a corresponding proof
  update is rejected at review. `cargo xtask verify` runs on the
  verified sub-workspace in nightly CI; the verify-fails-build
  policy is the same shape as the dst-lint policy (ADR-0006).
- **Revisit-trigger review cadence**: GH #127 is checked
  quarterly. Each check answers: has any of the five triggers
  fired? If yes, this ADR is re-opened; if no, a quick "no
  change" comment lands on the issue.

## Supersedes / Relates

- **Does not supersede any existing ADR.** Whitepaper §18 remains
  the public commitment; this ADR is its execution plan.
- **Relates to ADR-0017** — the pilot consumes `InvariantName` /
  `InvariantClass` from the `overdrive-invariants` crate as the
  name-and-class bridge between executable and verified
  specifications. If ADR-0017 is rejected, the pilot is re-scoped to
  depend on whatever sharing surface the project lands in its place,
  or pauses until one exists. The pilot does not ship a parallel
  private name registry — that would introduce exactly the dual-path
  state the project's single-cut-migration policy forbids.
- **Relates to ADR-0003** — the verified sub-workspace crate
  declares `crate_class = "core"`. No new class needed.
- **Relates to ADR-0013** — the cert-rotation reconciler (the pilot
  target) is a built-in reconciler in the sense of ADR-0013 and
  lands on the `Reconciler` trait surface ADR-0013 defined. Verus
  verifies a concrete impl of that trait, not the trait itself.
- **Relates to ADR-0006** — the `cargo xtask verify` subcommand
  follows the same shape as `cargo xtask dst` / `dst-lint`,
  including seed surfacing (for Verus's solver-randomness paths)
  and structured failure artifacts.
- **Informs a future whitepaper §18 amendment** — either "pilot
  succeeded, commitment validated" (no text change; this ADR's
  changelog is the evidence) or "pilot failed, commitment softened"
  (superseding ADR drafts replacement text). Neither outcome is
  predetermined.

## Open Questions (for user decision)

1. **Pilot start gate — ADR-0017 first?** The pilot consumes
   `overdrive-invariants::InvariantName`. ADR-0017 lands single-cut
   (sim invariants deleted and new crate landed in the same PR), so
   "the crate exists" is a binary event, not a phased rollout. The
   pilot either waits for that PR to merge or starts against a private
   name registry and re-imports on merge. Architect recommendation:
   wait for the ADR-0017 PR — the landing is expected to be bounded
   (not a phased migration), and a private name registry followed by
   a migration later would itself violate the project's single-cut
   preference. User can override if timeline pressure differs.
2. **Kani scope — only `unsafe` panic-freedom, or also bounded ESR?**
   Research Finding 3.2 flags that Kani *could* reason about some
   bounded ESR-shape properties (the scheduler bin-pack against
   fixed capacity, for example). The architect recommendation is
   **keep Kani strictly scoped to `unsafe` interior** — the
   Firecracker precedent is direct there, and mixing scopes confounds
   the go/no-go criteria. The user can override if Kani-for-bounded-ESR
   strikes them as a genuine hedge.
3. **Proof maintenance policy — harder or softer than "reject PR
   without proof update"?** Options: (a) reject outright (architect
   recommendation — matches dst-lint gate); (b) allow with a `TODO:
   update proof` and a 7-day SLA; (c) track proof debt as a backlog
   item. Option (a) is strictest; (c) is most flexible at the cost of
   proof rot risk.
4. **TLA embedding — port, rebuild, or slim equivalent?** Deferred to
   the 2-day spike at pilot start (see §4 of the Decision). The user
   may want to force an upfront choice before the spike; the
   architect recommendation is to let the spike data drive.
5. **Pilot exit on fail — soften §18, or retry with different
   reconciler?** If the cert-rotation pilot fails on criterion 1 or
   2, is the next step a softening ADR, or is there a second pilot
   attempt on a different reconciler (e.g. tombstone sweep)? The
   architect recommendation is **one pilot, not two** — a second
   attempt with the same tooling on different code is not
   additional evidence, it is the same bet rolled twice. The user
   may disagree if they believe cert-rotation was specifically
   ill-chosen.
6. **Revisit-trigger quarterly review — who owns it?** GH #127 needs
   a cadence owner. Architect recommendation: the DEVOPS-wave
   platform architect (not the DESIGN-wave solution architect — the
   triggers fire on tool-ecosystem events, not architecture events).
   Alternative: the user themselves. This is a project-operations
   call.

## References

- Research: `docs/research/verification/verus-for-overdrive-applicability-research.md`
  (Nova, 2026-04-23). All findings cited are traceable to that
  doc's section numbers.
- Research: `docs/research/testing/invariant-observer-patterns-comprehensive-research.md`
  (Nova, 2026-04-23). Finding 1.4 (Anvil ESR) and Finding 2.4
  (offline vs runtime verification).
- ADR-0003 — core-crate labelling.
- ADR-0006 — CI wiring pattern for `cargo xtask` subcommands.
- ADR-0013 — reconciler primitive (cert-rotation reconciler is a
  concrete implementation of the ADR-0013 trait).
- ADR-0017 — `overdrive-invariants` crate (name-and-class contract
  with the Verus pilot).
- Whitepaper §18 — Reconciler and Workflow Primitives.
- `.claude/rules/testing.md` — Tier 1 DST (baseline the verification
  complements, not replaces).
- Sun, X. et al. "Anvil: Verifying Liveness of Cluster Management
  Controllers." OSDI '24, Best Paper. Jay Lepreau Award. [PDF](https://www.usenix.org/system/files/osdi24-sun-xudong.pdf).
- Lattuada, A. et al. "Verus: A Practical Foundation for Systems
  Verification." SOSP '24, Distinguished Artifact Award.
- Kani Verifier. "Using Kani to Validate Security Boundaries in AWS
  Firecracker." 2023.
- GH #127 — revisit-trigger tracker for this ADR.
