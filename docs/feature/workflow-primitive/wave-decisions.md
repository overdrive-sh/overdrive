# Wave Decisions — workflow-primitive

## DIVERGE Decisions

### Key Decisions
- [D1] **Job extracted at strategic/physical level, not the requested
  feature shape.** The request named a solution (`Workflow` trait +
  libSQL journal + signals + reconciler). The validated job is the
  irreducible "drive a finite side-effecting sequence to a terminal
  result exactly-once, crash-resumable on any node, without hand-rolling
  a state machine per sequence." (see: `diverge/job-analysis.md`)
- [D2] **"Durable like restate.dev" reads as outcomes, not mechanism.**
  Research correction: Restate is journal-replay like Temporal (NOT
  mechanically distinct on re-execution); and DBOS *also* requires
  deterministic code and re-enters from the top. The real mechanism
  cleavage is **replay-of-authored-control-flow (inherits version-skew)**
  vs **explicit-persisted-state (dodges it)**. (see:
  `diverge/competitive-research.md` §4 Insight A/B)
- [D3] **The deferred version-skew hazard is a first-class scoring axis.**
  #39 defers code-graph-hash version-skew rejection; any replay option
  ships that hazard unmitigated into first-party platform sequences. The
  taste DVF/Viability lens penalizes inheriting options accordingly.
  (see: `diverge/taste-evaluation.md` Phase 1)
- [D4] **Journal store is an open assumption, not a given.** The
  whitepaper says libSQL; the peer reconciler primitive was deliberately
  moved OFF libSQL to redb (ADR-0035) for O6 (one-mechanism) reasons. The
  store decision is evaluated per option, not inherited. (see:
  `diverge/competitive-research.md` §4 Insight C)
- [D5] **Reconciler-as-workflow is a real contender, not a strawman.**
  Argo Workflows' controller is a production reconcile-loop step machine;
  this is the non-obvious alternative and it ranks #1. Choosing it
  requires amending the whitepaper §18 / reconcilers.md two-primitive
  doctrine. (see: `recommendation.md` Option C)

### Job Summary
- Validated job: see [D1] above (strategic/physical level).
- ODI outcomes: **6** outcome statements (O1–O6); O1/O2/O4 are the
  exactly-once-correctness core, O5 ties to DST, O6 surfaces the
  store tension.

### Options Evaluated
- **8 options generated** (SCAMPER S/C/A/M/P/E/R + 3 Crazy-8s);
  **2 set aside** as out-of-scope (P + Crazy-8 #1 both *hinge* on the
  deferred WASM SDK / WASM-component execution unit, which the dispatch
  forbids); **6 carried to taste** (A–F), **all 6 survived the DVF
  filter** (lowest = E at 9 > 6).
- **Recommended: Option C (reconciler-as-step-machine) — 4.50** —
  reuses the existing reconciler runtime + redb ViewStore + DST invariant
  (zero new mechanism, O6 maximal), dodges the deferred version-skew
  hazard; **contingent on DISCUSS ratifying the two-primitive doctrine
  amendment.**
- **Dissent: Option F (macro-lowered explicit-state) — 4.05** — wins if
  authoring ergonomics (O3) is judged the dominant outcome; a single
  defensible reweight toward O3 flips C↔F. Second, sharper dissent:
  if the deferred version-skew mitigation is brought back into scope,
  **Option B (DBOS-style on libSQL, whitepaper-faithful, "durable like
  restate")** re-enters as a live contender — a **scope** decision for
  DISCUSS, not a taste decision.

### Open questions for DISCUSS (can flip the recommendation)
1. Is the two-primitive doctrine load-bearing **beyond mechanism**
   (suspension ergonomics, parent-child composition, WASM extension
   surface)? → C vs F.
2. Does the deferred version-skew mitigation (code-graph hashing) **stay
   deferred**? If not, re-open Option B. → scope decision.
3. Journal store: libSQL (whitepaper) vs redb (peer-primitive precedent,
   O6) vs append-only log? C answers it for free; B/A/F must justify.

---

## RATIFIED DIRECTION (post-DIVERGE design dialogue — 2026-06-05)

**The user selected a distinct durable `Workflow` primitive, journaled in
redb — NOT the matrix's Option C (reconciler-as-step-machine).** This is
the "B′" synthesis: Option B's durable-async authoring model with the
journal store swapped from libSQL to redb. It **supersedes** the matrix
recommendation below; Option C is retained as the runner-up. The decision
rests on three premises corrected during the design dialogue (which the
original taste scoring did not have):

- **[R1] Version-skew is an SDK-era concern, not an architectural driver.**
  Deferring the app SDK defers its *version-skew mitigation* (load-time
  code-graph hashing) — it does **not** mean the platform must
  architecturally avoid replay. First-party Rust workflows ship journal +
  code in **one binary**, recompiled as a unit, so the hazard is minor and
  its fix arrives with the SDK. This **neutralizes [D3]** — the scoring
  penalty that [D3] applied to replay-of-control-flow options (A/B) is
  withdrawn for the platform-internal scope. Open-Q2 answered: **the
  mitigation stays deferred** (it rides with the app SDK).

- **[R2] Journal lives in redb, not libSQL.** Resolves [D4] / open-Q3 in
  favor of the redb substrate: a crash-resume journal is append-mostly,
  small-record, point-access by `(workflow_id, step)` — redb's wheelhouse,
  and already the reconciler ViewStore backend (ADR-0035, O6). SQL would
  only earn its keep for *ad-hoc* journal queries, which is an
  observability nice-to-have addable later as a read-only view, not a
  replay requirement. One durable-memory story for both primitives.

- **[R3] The two-primitive doctrine is UPHELD, not amended.** Answers
  open-Q1: a distinct primitive is justified — but **not** by "terminates
  vs runs forever" (Jobs already run-to-completion on the reconcile loop,
  ADR-0047, reaching a typed `TerminalCondition`, ADR-0037). The real
  discriminator is **ordered multi-step orchestration with await-points**
  (issue → wait → validate → swap → result) vs converging a single
  desired/actual relationship. The distinct primitive earns its place via
  the inner **await / suspension / signal / parent-child** execution
  surface the converge loop cannot express ergonomically. **Instance
  lifecycle** (spec → running → journaled → terminated) remains
  **reconciler-managed** — the workflow-lifecycle reconciler named in
  whitepaper §18 — in this design too. What the distinct primitive adds is
  only the durable-async **execution of the steps between start and
  terminal.**

**Selected option — B′:** a distinct durable-async `Workflow` primitive
(`async fn run(ctx)` with `ctx.call`/`ctx.sleep`/`ctx.wait_for_signal`/
`ctx.activity`), step **journal in redb**, instance lifecycle owned by the
**workflow-lifecycle reconciler**, all non-determinism through `ctx`
(injected `Clock`/`Transport`/`Entropy`, DST-controllable), replay-
equivalence + bounded-progress as the correctness property, version-skew
mitigation **deferred with the app SDK**.

**Carried into DISCUSS as locked:** the primitive shape (distinct,
durable-async), the journal store (redb), and the doctrine (two primitives
upheld; lifecycle reconciler-managed). DISCUSS defines the journey,
requirements, and acceptance criteria over this locked direction — it is
no longer choosing between C and B′.

### SSOT Updates
- `docs/product/jobs.yaml`: **created** job **J-PLAT-005** (durable
  multi-step terminal sequence) + changelog entry referencing this
  feature-id.

### Peer Review
- Reviewer (nw-diverger-reviewer / Prism): **not dispatchable from
  subagent context** (Task tool unavailable inside a subagent). Verdict
  surfaced to orchestrator for relay — see return summary. All 5 phase
  gates (G1–G4 + diversity) self-verified PASS.
