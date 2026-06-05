<!-- markdownlint-disable MD024 -->
# Feature Delta — workflow-primitive (GH #39, roadmap [3.2])

Wave: DISCUSS (Luna / nw-product-owner) · Date: 2026-06-05 · Density: lean +
ask-intelligent. Job: **J-PLAT-005** (DIVERGE-validated; NOT re-derived).
Architecture: **locked to B′** (distinct durable-async `Workflow` primitive,
redb journal) per `wave-decisions.md` § "RATIFIED DIRECTION" — designed OVER,
not re-litigated.

---

## Wave: DISCUSS / [REF] Persona ID

**`devon-platform-engineer`** (Devon Reyes) — Overdrive platform engineer
authoring first-party durable control-plane sequences (cert rotation, region
migration, staged rollout, microVM snapshot/restore). Primary = **workflow
author** (O3 authoring ergonomics). Secondary actor = **Ana, operator**
observing a running/terminal instance via ObservationStore rows + structured
lifecycle events (NO CLI verb exists — see Deferrals). New persona kind:
platform-internal, distinct from the app-developer/evaluator J-DOCS personas.
File: `docs/product/personas/devon-platform-engineer.yaml`.

## Wave: DISCUSS / [REF] JTBD one-liner

When a platform subsystem must perform a finite, ordered, multi-step operation
whose steps take externally-visible side effects unsafe to repeat (issue a
cert, quiesce a region, snapshot a microVM, ratify a rollout), **I want** to
express the sequence as ordinary control flow and have the platform persist
its progress, resume it on any node after a crash from the first incomplete
step, and drive it to a single terminal result, **so I can** rely on the
operation completing exactly-once without hand-rolling a state machine, a step
cursor, a crash-resume path, and a correctness proof for each one. (J-PLAT-005;
ODI outcomes O1–O6 — `diverge/job-analysis.md`.)

## Wave: DISCUSS / [REF] Locked Decisions

Inherited (from DIVERGE / RATIFIED DIRECTION — design over these, do NOT
re-litigate):

- **[D-INH-1] Distinct durable-async `Workflow` primitive.** `async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult` with `ctx.run<T>`/`ctx.sleep`/`ctx.wait_for_signal`/`ctx.activity`. NOT reconciler-as-step-machine (Option C is runner-up). (wave-decisions § RATIFIED, R3.)
- **[D-INH-2] Step/await journal in redb**, per-instance append-only, distinct layout from the reconciler `View` store, same backend. NOT libSQL. (R2; whitepaper §17/§18.)
- **[D-INH-3] Instance lifecycle owned by the workflow-lifecycle reconciler** (spec→running→journaled→terminated); reconcilers emit `Action::StartWorkflow { spec, correlation }`. (R3; whitepaper §18.)
- **[D-INH-4] All non-determinism through `ctx`** (injected Clock/Transport/Entropy, DST-controllable). Workflow→cluster mutations via typed Actions through Raft, never direct IntentStore writes. Cross-workflow coordination via typed signals in the ObservationStore.
- **[D-INH-5] Correctness = deterministic replay + bounded progress** — replay the journal twice under DST → bit-identical; reach terminal within a declared step budget. §21 DST obligation, ties O5.
- **[D-INH-6] Version-skew mitigation deferred WITH the app SDK** (R1) — first-party Rust workflows ship journal + code in one binary, recompiled as a unit; the hazard is minor and its fix (code-graph hashing) rides with the deferred WASM SDK. No story may hinge on it.

DISCUSS-made:

- **[D1] Feature type: Backend** (platform-internal control-plane primitive). Users = platform engineers (authors) + operators. (Orchestrator-set.)
- **[D2] Walking skeleton: YES, thinnest end-to-end durable workflow** — `Workflow` trait + `WorkflowCtx` with ONE durable op, redb journal write at the await, lifecycle reconciler brings up one instance via `Action::StartWorkflow`, single-node crash-resume under DST, observable terminal. First consumer: **`ProvisionRecord`** (a minimal 2-step `ctx.run → terminal` sequence with a real non-idempotent-to-repeat effect) — NOT cert-rotation (needs slice-02/03 surface). (slice-01.)
- **[D3] O2 scoped to SINGLE-NODE crash-resume for Phase 1.** "Resume on a DIFFERENT node" (full O2) is a multi-node property the Phase-1 single-node codebase cannot honour; surfaced as a sequencing dependency on HA/multi-node. NO Phase-1 AC promises cross-node resume. The redb-journal design must not PRECLUDE it. (Scope note; back-propagated to jobs.yaml changelog.)
- **[D4] Observable surface for the operator = ObservationStore terminal-result row + structured lifecycle event + the `replay_equivalence_*` DST invariant as executable evidence.** NO `overdrive workflow` CLI verb is invented (cli.rs has none). (D-INH-3; honesty constraint.)
- **[D5] Every story `job_id: J-PLAT-005`.** N:1 mapping (no infrastructure-only stories — the engine slice ships WITH its observable skeleton consumer).

## Wave: DISCUSS / [REF] Scope Assessment

**Verdict: OVERSIZED → SLICED (user-approved direction is the B′ primitive;
carpaccio split applied).** Oversized signals met (≥2): the primitive spans
the durable-async engine + redb journal layout + replay-equivalence DST
machinery + lifecycle reconciler integration + signals + parent-child
composition (multiple independent outcomes that ship separately); estimated
effort > 2 weeks for the whole surface. Per the carpaccio taste test "if every
slice depends on a new abstraction, ship the abstraction FIRST as its own
slice," the **durable engine + journal + replay core is slice 01** and every
later await-surface slice is additive on it. Sliced into 3 thin end-to-end
vertical slices (briefs in `slices/`); parent-child composition named as a
forward slice-04, not specified here (keeps DISCUSS right-sized). **Taste
tests:** (a) no slice ships 4+ new components — slice 01 is the one heavy
slice and is explicitly flagged 1–1.5 days with a de-risking SPIKE option;
(b) the new abstraction (engine) ships FIRST as slice 01; (c) slice 01's
learning hypothesis disproves the locked B′ direction's central premise if it
fails (real disproof, not decoration); (d) every slice carries a
production-data AC (real `ctx.run` effect, real SimTransport call-count
assertion — not synthetic plumbing); (e) no two slices are scale-duplicates.

## Wave: DISCUSS / [REF] User Stories

All stories: `job_id: J-PLAT-005`. Each elevator pitch is honest about the
real observable surface (Rust author surface + ObservationStore + DST test —
NOT a CLI verb that does not exist).

### US-WP-1 — Express a durable sequence as ordinary control flow

**Story.** Devon, a platform engineer, must write cert rotation / region
migration as a finite side-effecting sequence. Today she would hand-roll a
step-cursor enum + transition match + crash-resume path per sequence. She
wants to write one ordinary `async fn run(&self, ctx)` and let the platform
own durability.

#### Elevator Pitch
Before: Devon cannot author a crash-resumable sequence without hand-writing a state machine, a step cursor, and a recovery path for each one.
After: write `impl Workflow for ProvisionRecord { async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult { let _ = ctx.run("write_record", async { ctx.transport().send(write_record_req).await }).await?; Ok(WorkflowResult::Success) } }` → `cargo dst --only replay_equivalence_provision_record` shows `ok · step 0 executed once · terminal result == uninterrupted-run terminal`.
Decision enabled: Devon decides she can model the next platform sequence (cert rotation) as ordinary control flow rather than a bespoke state machine.

#### Acceptance Criteria
- [ ] AC1: A `Workflow` trait exists with `async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult`, and `ProvisionRecord` implements it; the impl **compiles** and a test drives `run` to a terminal `WorkflowResult` (verifies the "After" author surface — one ordinary `async fn`, no bespoke runtime). The O3 *structural* property (zero step-enum / transition-match lines in the body) is the **K6 metric**, asserted by an AST/grep check over the workflow impl — NOT free-hand review. (Per Eclipse review H1: AC made mechanically verifiable.)
- [ ] AC2: The workflow body performs its side effect through `ctx.run(name, f).await` only (the closure `f` reaching the outside world via `ctx.transport()`); a `dst-lint`-style check / review confirms no `Instant::now()` / `reqwest` / `tokio::time::sleep` / `rand` in the body (D-INH-4).
- [ ] AC3 (O5): `cargo dst --only replay_equivalence_provision_record` is green and prints a reproducible seed. (Verifies the "sees" output end-to-end.)

→ O3, O5.

### US-WP-2 — Journal the await-point in redb so a completed step is durable

**Story.** Devon needs the platform to persist a completed step BEFORE it can
be lost to a crash, on the substrate she already operates — not a second
storage engine.

#### Elevator Pitch
Before: a completed step in Devon's sequence exists only in process memory and is lost the instant the control-plane node crashes.
After: when `run` reaches its durable await, the runtime writes a per-instance append-only checkpoint to redb before suspending; the test harness reads the journal handle and shows the recorded step present in redb (not libSQL).
Decision enabled: Devon decides she does not need a bespoke persistence layer — the existing redb substrate carries her sequence's progress (O6).

#### Acceptance Criteria
- [ ] AC1 (O6): The await checkpoint is written to the runtime-owned redb substrate, a per-instance append-only layout distinct from the reconciler `View` store but on the same backend; asserted via the journal handle. NO libSQL journal exists. (D-INH-2.)
- [ ] AC2 (durability ordering): The checkpoint is fsync'd BEFORE the await suspends (mirrors reconciler write-through ordering); asserted by the crash-resume test in US-WP-3.
- [ ] AC3 (inputs-not-derived): The journal records step inputs/results, not a derived deadline cache (development.md "Persist inputs, not derived state").

→ O1, O6.

### US-WP-3 — Resume exactly-once after a single-node crash

**Story.** Devon needs the headline guarantee: kill the process mid-run,
restart, and the completed external effect is NOT repeated, the committed
result is NOT lost, and the run drives to terminal — proven, not asserted by
inspection.

#### Elevator Pitch
Before: Devon cannot trust that killing a node mid-sequence won't double-issue a cert / double-write a record / lose a committed step.
After: under DST, kill the instance after step 0 records but before terminal, restart → `replay_equivalence_provision_record` shows `step 0 executed once (not repeated on resume) · terminal result == uninterrupted-run terminal`; the SimTransport call-count for the effect is exactly 1.
Decision enabled: Devon decides a control-plane crash during this sequence is survivable — she ships it without a bespoke recovery path.

#### Acceptance Criteria
- [ ] AC1 (O1): Killing the instance AFTER the `ctx.run` step records but BEFORE terminal, then restarting, does NOT re-fire the `ctx.run` closure's external effect on the replay path (SimTransport call count == 1, not twice). Honest scope: exactly-once on the replay path — once journaled, the closure is not re-polled; a crash in the fire→fsync window (before the step journals) re-runs the effect (at-least-once), mitigated by the remote idempotency key (the Restate `ctx.run` caveat).
- [ ] AC2 (O4): The resumed run reaches a `WorkflowResult` byte-identical to the uninterrupted run for the same inputs + seed.
- [ ] AC3 (O2, SINGLE-NODE SCOPE): The crash-resume is demonstrated by killing the PROCESS and restarting on the SAME (single) node, resuming from the redb journal. **No AC claims cross-node resume** — that is a multi-node dependency (D3). The journal design does not preclude cross-node resume.
- [ ] AC4: The workflow-lifecycle reconciler re-hydrates the instance from `Action::StartWorkflow { spec, correlation }` on restart (D-INH-3).

→ O1, O2 (single-node), O4.

### US-WP-4 — Prove replay-equivalence from a seed before shipping

**Story.** Devon will not adopt a primitive whose correctness she can only
validate by code review. She needs replay-equivalence + bounded progress as a
named DST invariant on the CI critical path, reproducible from a seed — the
same discipline she already trusts for reconcilers.

#### Elevator Pitch
Before: Devon can only check her sequence's resume path by reading code and hoping — no mechanical evidence before merge.
After: `cargo dst --only replay_equivalence_provision_record` runs a named `SimInvariant` (replay the journal twice → bit-identical) paired with `assert_eventually!(is_terminal)` (bounded progress), green on the CI critical path, reproducing bit-for-bit from the printed seed.
Decision enabled: Devon decides the resume path is correct on the evidence of a seeded DST run — the same bar as her reconcilers — and merges.

#### Acceptance Criteria
- [ ] AC1 (O5): A named `replay_equivalence_provision_record` `SimInvariant` is exported from `overdrive-sim` (no inline string literal, per house convention) and runs on the CI critical path.
- [ ] AC2 (D-INH-5): Replaying the journal twice produces a bit-identical trajectory (`assert_replay_equivalent!`); a paired `assert_eventually!(is_terminal)` proves the run reaches terminal within a declared step budget (bounded progress).
- [ ] AC3: The invariant prints a seed and reproduces bit-for-bit on a second run on the same SHA + toolchain (trust-the-sim discipline).

→ O4, O5.

### US-WP-5 — Coordinate via typed signals + emit cluster mutations through Raft

**Story (slice 03).** Devon needs a sequence to wait on a typed signal from
another workflow and to push a cluster mutation — both crash-safe, both
through the sanctioned channels, never a direct IntentStore write.

#### Elevator Pitch
Before: Devon cannot make a durable sequence wait on another workflow or mutate cluster intent without hand-rolling coordination and risking a Raft bypass.
After: write `ctx.wait_for_signal(key).await` and `ctx.emit_action(action).await` → under DST, crash while blocked re-blocks on the SAME signal on resume; the emitted Action lands in the Raft channel (asserted) with the workflow performing NO direct IntentStore write.
Decision enabled: Devon decides she can compose cross-workflow coordination (and, later, parent-child) on a proven, crash-safe signal + emit surface.

#### Acceptance Criteria
- [ ] AC1 (O1): Crash while blocked on `ctx.wait_for_signal` → on resume the workflow blocks on the SAME signal; no duplicate downstream effect.
- [ ] AC2 (D-INH-4, no Raft bypass): `ctx.emit_action` lands the typed Action in the Raft channel; asserted the workflow performs NO direct IntentStore write.
- [ ] AC3 (idempotent emit): Crash AFTER an emit records but before terminal → the Action is NOT re-emitted on resume.
- [ ] AC4 (O5): `replay_equivalence_*` green across a signal wait + an emit, seeded, reproducible.

→ O1, O3, O4, O5.

## Wave: DISCUSS / [REF] Outcome KPIs

Numeric targets + measurement method, tied to O1–O6. Baseline is greenfield
(no incumbent Overdrive mechanism — satisfaction ≈ 1 across the board per
job-analysis §4).

| KPI | Who | Does what | By how much | Measured by | Baseline |
|---|---|---|---|---|---|
| K1 (O1) | the durable engine | re-executes a recorded external effect on resume | 0 occurrences (call count == 1) | SimTransport call-count assertion in the crash-resume DST scenario | none (hand-rolled, untested) |
| K2 (O2, single-node) | a crashed instance | loses a committed step on single-node restart | 0 lost steps | journal-read assertion on resume (recorded step present) | none |
| K3 (O4) | the resumed run | diverges from the uninterrupted terminal | 0 byte-divergences for same inputs+seed | byte-equality of `WorkflowResult` (resumed vs uninterrupted) | none |
| K4 (O5) | the platform engineer | proves resume-equivalence before ship | 1 named `replay_equivalence_*` SimInvariant on the CI critical path, reproducible from seed | presence + green status of the invariant in `cargo dst`; bit-for-bit seed reproduction | DST exists for reconcilers, NOT sequences |
| K5 (O6) | the platform | operates distinct persistence/recovery mechanisms for terminal sequences | +0 NEW stores (journal on existing redb; no libSQL journal) | grep/dep-graph: no libSQL journal table; journal handle is redb-backed | redb already serves reconcilers |
| K6 (O3) | the workflow author | hand-writes step-machine boilerplate per sequence | 0 step-enum / transition-match lines in a workflow body | AST/grep check over the `Workflow` impl body (zero step-enum decls, zero state-transition `match` arms) — automatable, not free-hand review (per Eclipse L1) | full state machine per sequence |

K4 is the load-bearing KPI: a green, seeded, reproducible
`replay_equivalence_*` invariant on the CI critical path IS the proof of O5
and the gate the feature exists to deliver. (Note O2 cross-node resume is NOT
a Phase-1 KPI — K2 is scoped single-node per D3.)

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy B (thinnest end-to-end vertical slice).** Slice 01 IS the walking
skeleton: `Workflow` trait + `WorkflowCtx` (one durable `ctx.run<T>`) + redb
journal + lifecycle-reconciler bring-up via `Action::StartWorkflow` +
single-node crash-resume under DST + observable ObservationStore terminal.
First consumer `ProvisionRecord` (real non-idempotent-to-repeat effect).
Ships end-to-end (engine + observable consumer together) — NOT an
`@infrastructure`-only engine, satisfying the slice-composition hard gate.

## Wave: DISCUSS / [REF] Driving Ports

- **Author surface (primary):** the Rust `Workflow` trait + `WorkflowCtx` in the Overdrive workspace. This is the inbound surface platform engineers use.
- **Lifecycle trigger:** `Action::StartWorkflow { spec, correlation }` emitted by a reconciler onto the existing Action channel → workflow-lifecycle reconciler.
- **Observable surfaces (no CLI):** ObservationStore terminal-result row (keyed by `CorrelationKey`) + structured lifecycle events + the `replay_equivalence_*` DST invariant as executable evidence.
- **NOT a driving port:** there is NO `overdrive workflow` CLI subcommand (cli.rs: deploy/job/node/alloc/cluster only). Surfaced as a forward concern (Deferrals).

## Wave: DISCUSS / [REF] Pre-requisites

EXISTS (brownfield, verified): reconciler runtime, redb ViewStore, Action
channel, ObservationStore, DST harness, `Action::StartWorkflow` placeholder
(`reconcilers/mod.rs:373`), `replay_equivalence_empty_workflow` already a named
DST invariant (trust-the-sim step 1). NEEDED by this feature: concrete
`WorkflowSpec`, `Workflow` trait + `WorkflowCtx`, per-instance redb journal
layout, workflow-lifecycle reconciler instance bring-up. No external
dependency outside the workspace.

## Wave: DISCUSS / [REF] Definition of Done

- [ ] All 5 stories' ACs green (slices 01–03).
- [ ] Slice 01 walking skeleton: one durable op, redb journal, lifecycle bring-up, single-node crash-resume under DST, observable terminal — all demonstrated.
- [ ] `replay_equivalence_provision_record` (+ later variants) named SimInvariant(s) green on the CI critical path, seed-reproducible (K4).
- [ ] Journal on redb; no libSQL journal table (K5).
- [ ] No story / AC promises cross-node resume (D3 honesty).
- [ ] No invented CLI verb (D4 honesty).
- [ ] DoR passed (9/9 below).
- [ ] SSOT updated: persona + journey created, jobs.yaml changelog (done).

## Wave: DISCUSS / [REF] Out of Scope

- **WASM workflow SDK** + SDK load-time version-skew rejection via code-graph hashing (dispatch-forbidden; rides with the deferred SDK) — [#209](https://github.com/overdrive-sh/overdrive/issues/209).
- **Cross-NODE resume** (full O2) — multi-node/HA dependency (D3) — [#205](https://github.com/overdrive-sh/overdrive/issues/205).
- **Operator `overdrive workflow` CLI verb** — none in cli.rs — [#206](https://github.com/overdrive-sh/overdrive/issues/206).
- **Parent-child workflow composition** (`ctx` awaiting a child result) — forward slice-04, post-signals.
- **Journal retention / compaction policy** — whitepaper names "compacted on a declared retention policy"; the policy itself is a forward concern, not in the skeleton — [#208](https://github.com/overdrive-sh/overdrive/issues/208).
- **`ctx.activity`** beyond what slices 02/03 need — the full activity surface is post-skeleton.

## Wave: DISCUSS / [REF] DoR Validation (9/9)

1. **Problem clear, domain language** — PASS. JTBD one-liner + per-story Problem in domain terms (cert rotation, region migration, exactly-once side effect). Evidence: job-analysis.md, this delta.
2. **Persona with specific characteristics** — PASS. `devon-platform-engineer.yaml` (author primary + operator secondary), emotional arc, frustrations, success signals.
3. **3+ domain examples with real data** — PASS. `ProvisionRecord` (record-write effect), cert rotation (ACME CSR — the non-idempotent example), region migration (quiesce source) named throughout; `ctx.run` with a concrete closure shape in US-WP-1's pitch.
4. **UAT in Given/When/Then (3–7 scenarios)** — PASS. 5 stories, each with testable ACs in observable-outcome form; scenario titles describe business outcomes (resume exactly-once, prove replay-equivalence), not implementation. Journey YAML carries the GWT-shaped step checkpoints.
5. **AC derived from UAT** — PASS. Every AC traces to a journey step / ODI outcome and is verifiable (call-count == 1, byte-equality, named-invariant-green).
6. **Right-sized (1–3 days, 3–7 scenarios)** — PASS with one flag. 3 slices; slice 01 flagged 1–1.5 days with a de-risking SPIKE option (the one heavy slice); slices 02/03 ≤1 day. Each story 3–4 ACs.
7. **Technical notes: constraints/dependencies** — PASS. Locked B′ direction (D-INH-1..6), redb-not-libSQL, no-Raft-bypass, all-through-ctx, single-node O2 scope, no CLI verb — all stated.
8. **Dependencies resolved or tracked** — PASS. Brownfield substrate verified present; cross-node resume + CLI verb + signals-under-partition + retention + SDK filed as #205/#206/#207/#208/#209 (user-approved 2026-06-05).
9. **Outcome KPIs with measurable targets** — PASS. K1–K6, numeric targets + measurement method, tied O1–O6.

**DoR: 9/9 PASS.**

## Wave: DISCUSS / [REF] Wave Decisions

- [D1] Backend feature type (orchestrator-set).
- [D2] Walking skeleton = slice 01, consumer `ProvisionRecord` (not cert-rotation — justified: cert-rotation needs slice-02/03 await surface).
- [D3] O2 scoped single-node for Phase 1; cross-node resume is a sequencing dependency on HA/multi-node. Back-propagated to jobs.yaml changelog (job statement unchanged).
- [D4] Observable surface = ObservationStore + structured events + DST invariant; NO invented CLI verb.
- [D5] All stories `job_id: J-PLAT-005`; no infrastructure-only stories (engine ships with its observable skeleton consumer).

### Constraints Established
- redb journal, not libSQL (K5/O6). No Raft bypass. All non-determinism through `ctx`. Single-node crash-resume only (D3). No `overdrive workflow` CLI verb invented (D4).

### Upstream Changes
- None to DIVERGE artifacts (architecture locked). One SCOPE clarification back-propagated to `jobs.yaml` changelog: J-PLAT-005's O2 is Phase-1-scoped to single-node; the job statement (served_by_phase: 3) is unchanged.

## Wave: DISCUSS / [REF] Deferrals / Forward Concerns (tracked)

All five surfaced during DISCUSS and **filed with user approval (2026-06-05)**:

1. **Cross-node workflow resume (full O2)** — [#205](https://github.com/overdrive-sh/overdrive/issues/205). Multi-node/HA dependency; the redb journal must not preclude it. Not promised in any Phase-1 AC.
2. **Operator `overdrive workflow` CLI verb** — [#206](https://github.com/overdrive-sh/overdrive/issues/206). cli.rs has none; operators currently observe via ObservationStore rows. An inspect/list verb is a real gap for the operator journey.
3. **Typed-signal scope under partition** — [#207](https://github.com/overdrive-sh/overdrive/issues/207). Slice 03 delivers in-process single-node signal delivery; cross-node signal semantics under partition is a multi-node concern.
4. **Journal retention / compaction policy** — [#208](https://github.com/overdrive-sh/overdrive/issues/208). Whitepaper names "compacted on a declared retention policy" — the policy is undefined.
5. **WASM workflow SDK + version-skew code-graph hashing** — [#209](https://github.com/overdrive-sh/overdrive/issues/209). Deferred with the app SDK; no story hinges on it.

## Wave: DISCUSS / [REF] Peer Review (Eclipse)

**Reviewer**: Eclipse (nw-product-owner-reviewer) · **Date**: 2026-06-05 ·
**Model**: inherit (session) · **Verdict**: **APPROVED** (handoff to DESIGN cleared).

Blocking issues: **none**. Hard gates (JTBD traceability, slice composition,
DoR 9/9) all satisfied. One HIGH-severity finding was actionable and has been
resolved in this artifact; remaining findings are non-blocking.

### Dimension results

| Dimension | Verdict |
|---|---|
| Journey coherence + emotional arc | PASS |
| Job traceability (hard gate) | PASS — all stories `job_id: J-PLAT-005`, mapped to O1–O6 |
| Elevator-pitch honesty | PASS — real surfaces (DST, redb journal, ObservationStore, Raft); no invented CLI verb |
| Slice composition (hard gate) | PASS — no `@infrastructure`-only slice; slice 01 ships with `ProvisionRecord` consumer |
| AC testability | PASS (US-WP-1 AC1 rephrased per H1) |
| Outcome KPIs | PASS — K1–K6 numeric + method; K4 load-bearing |
| Scope honesty / deferral discipline | PASS — #205–#209 cited by real issue numbers; no Phase-1 AC promises cross-node resume |
| LeanUX antipatterns / sizing | PASS |

### Findings

- `praise:` Honest scope and deferral discipline — every forward concern
  surfaced and cited by real issue number (#205–#209); Phase-1 single-node O2
  scope explicit and back-propagated; no invented CLI verb. Model for future specs.
- `praise:` Strong, earned emotional arc — Devon "wary → reassured → trusting,"
  each persona frustration mapped to a specific O1–O6 outcome and a preventing AC.
- `praise:` Clean carpaccio slicing — engine ships with a real value consumer;
  no orphan infrastructure; slice-01 learning hypothesis has teeth (failure
  threatens the locked B′ direction).
- `praise:` DST is first-class — K4 (`replay_equivalence_*` on the CI critical
  path) is a real gate, not aspirational; all non-determinism routed through `ctx`.
- `issue (resolved, H1):` US-WP-1 AC1 was code-review-phrased ("author writes …
  with NO step enum"). **Resolved**: AC1 now asserts compile + drive-to-terminal
  (mechanical), with the structural "no step machine" property delegated to the
  K6 metric. K6 measurement upgraded from free-hand review to an AST/grep check
  (also resolves L1).
- `suggestion (non-blocking, M1):` Slice-01 effort framing ("OPTIMISTIC ≤1 day")
  hedges; the honest estimate is ≤1.5 days + optional SPIKE. Narrative polish for
  the DELIVER dispatcher; no functional impact.
- `suggestion (non-blocking, M2):` The Phase-1 operator surface (Ana) is weaker
  than the author surface — no `overdrive workflow` verb; she reads ObservationStore
  rows. Correctly tracked as #206; flag to DESIGN that the operator journey is
  Phase-1-incomplete by design.

### Handoff
**APPROVED.** DESIGN may proceed. The architecture (B′ distinct durable-async
`Workflow`, redb journal, lifecycle-reconciler-managed) is locked from DIVERGE;
DISCUSS defines requirements/AC/slices over it and does not re-open the choice.

---

## Wave: DESIGN / [REF] Design Decisions (DDD)

Density: lean (per `~/.nwave/global-config.json`). Architecture **locked to
B′** (D-INH-1..6); these DDDs design OVER it. PROPOSE mode — each material
sub-decision was weighed 2–3 ways (see ADRs for the option matrices); the
recommended call is recorded here, the 3 warranting user ratification flagged.

- **[DDD-1] Journal store = second redb table layout, NOT an extension of
  `RedbViewStore`.** Distinct `JournalStore` port + `RedbJournalStore`/
  `SimJournalStore`, sharing the `RedbViewStore` redb **file + `Arc<Database>`
  + codec + fsync-ordering + probe discipline**, with a distinct table layout.
  Rationale: append-only-ordered-per-instance ≠ single-blob-overwrite; one
  trait must not carry two contracts. Substrate reuse (O6/K5) is identical to
  the "extend" option, without the trait coupling. ADR-0063 §1. **(RATIFY — the
  central reuse call.)**
- **[DDD-2] Journal codec = CBOR (`ciborium`, ADR-0035 §3 discipline), NOT the
  ADR-0048 rkyv envelope.** The journal is mutable runtime memory (ADR-0035's
  case), not a content-addressed/hashed type (ADR-0048's case); replay needs
  deterministic *decode* (CBOR gives it), not zero-copy archived-byte
  canonicality (buys nothing — never hashed). Additive entry-variants per
  await-surface slice ride `#[serde(default)]`; rkyv would force a per-slice
  version-bump + golden-fixture. ADR-0063 §2. **(RATIFY — codec choice.)**
- **[DDD-3] Replay = engine-owned journal cursor; `ctx.*` check-then-record.**
  The general durable-step primitive is `ctx.run<T>(name, f)` (Restate `ctx.run`
  model — wrap any side-effecting future, journal its `T`, replay the journaled
  result without re-running `f`). Engine re-executes `run` from the top each
  (re)start; replay returns recorded results without re-firing effects
  (exactly-once on the replay path — K1), live performs + appends
  (fsync-before-suspend) + advances cursor. Step identity is positional (the
  cursor); `name` is a diagnostic label + replay determinism check (fail-closed
  on mismatch). Honest semantics: at-least-once for the effect (a fire→fsync
  crash re-runs the closure), exactly-once on replay. All non-determinism
  through `ctx` ⇒ bit-identical replay (K4). ADR-0064 §3.
- **[DDD-4] `WorkflowCtx` surface additive per slice.** Machinery (cursor +
  suspend/resume) whole in slice 01; methods grow `run<T>`(01)→`sleep`(02)→
  `wait_for_signal`+`emit_action`(03), each an additive journal variant.
  ADR-0064 §4.
- **[DDD-5] Engine↔lifecycle-reconciler boundary: reconciler stays pure-sync;
  engine runs the async body off the action-shim.** The workflow-lifecycle
  reconciler is a normal ADR-0035 pure reconciler (emits `StartWorkflow`,
  observes terminal rows, never `.await`); the engine is the async executor
  driven off the shim, exactly as `StartAllocation`→`Driver::start`. The engine
  is to workflows what `Driver` is to allocations. ADR-0064 §5. **(RATIFY — the
  subtlest boundary.)**
- **[DDD-6] Crate placement: trait+ctx in `overdrive-core` (no tokio), engine +
  journal in `overdrive-control-plane`, sim journal + replay invariant in
  `overdrive-sim`.** Mirrors the reconciler trait-in-core/runtime-in-control-plane
  split; respects the `core`-has-no-tokio dst-lint rule. ADR-0064 §1.
- **[DDD-7] `WorkflowResult` is a new core enum, distinct from
  `TerminalCondition`.** Inherits the `#[non_exhaustive]` + K8s-Condition SemVer
  *convention* (ADR-0037 §5), not the type — they model different things
  (workflow return value vs reconciler allocation claim). ADR-0064 §2.

## Wave: DESIGN / [REF] Component Decomposition

| Component | Path | Class | Change |
|---|---|---|---|
| `Workflow` trait, `WorkflowCtx`, `WorkflowResult`, concrete `WorkflowSpec` | `overdrive-core/src/workflow/` | core | **CREATE NEW** module (`WorkflowSpec` replaces `reconcilers/mod.rs:562` placeholder) |
| `Action::StartWorkflow` | `overdrive-core/src/reconcilers/mod.rs:373` | core | **EXTEND** (already the locked shape; made live) |
| `WorkflowEngine` | `overdrive-control-plane/src/workflow_runtime/` | adapter-host | **CREATE NEW** |
| `JournalStore` port + `RedbJournalStore` | `overdrive-control-plane/src/journal/` | adapter-host | **CREATE NEW** |
| workflow-lifecycle reconciler | `overdrive-control-plane` (registration) + `overdrive-core/src/reconcilers` (state/`AnyState` variant) | core/adapter-host | **CREATE NEW** (pure-sync ADR-0035 reconciler) |
| action-shim `StartWorkflow` arm | `overdrive-control-plane/src/action_shim/mod.rs:446` | adapter-host | **EXTEND** (no-op `Ok(())` → `WorkflowEngine::start`) |
| `SimJournalStore` | `overdrive-sim/src/adapters/journal` | adapter-sim | **CREATE NEW** |
| `replay_equivalence_provision_record` (+ ordering + exactly-once invariants) | `overdrive-sim/src/invariants/` | adapter-sim | **EXTEND** (graduate `ReplayEquivalentEmptyWorkflow`) |

## Wave: DESIGN / [REF] Driving Ports

- **Author surface (primary):** `impl Workflow for X` against the core
  `Workflow` trait (Rust). The inbound surface for platform engineers.
- **Lifecycle trigger:** `Action::StartWorkflow { spec, correlation }` emitted
  by a reconciler onto the existing Action channel → workflow-lifecycle
  reconciler → action-shim → `WorkflowEngine::start`.
- **Observable surfaces (no CLI):** ObservationStore terminal-result row (keyed
  by `CorrelationKey`) + structured lifecycle events + the
  `replay_equivalence_provision_record` DST invariant as executable evidence.
- **NOT a driving port:** no `overdrive workflow` CLI verb (#206).

## Wave: DESIGN / [REF] Driven Ports + Adapters

| Driven port | Adapter (prod) | Adapter (sim) | Effect |
|---|---|---|---|
| `JournalStore` (NEW) | `RedbJournalStore` (shared redb file) | `SimJournalStore` | Durable append-only `await` journal |
| `Transport` (REUSE) | `TcpTransport` | `SimTransport` | `ctx.run` closure's transport effect (via `ctx.transport()`) |
| `Clock` (REUSE) | `SystemClock` | `SimClock` | `ctx.sleep` deadline park |
| `Entropy` (REUSE) | `OsEntropy` | `SeededEntropy` | any `ctx` RNG need |
| `ObservationStore` (REUSE) | `LocalObservationStore` | `SimObservationStore` | terminal-result row; typed signals (slice 03) |
| Action channel → Raft (REUSE) | reconciler-runtime commit path | sim harness | `ctx.emit_action` (slice 03; no IntentStore bypass) |

## Wave: DESIGN / [REF] Technology Choices (pinned)

- **Language/runtime:** Rust 2024; `tokio` (engine only, in control-plane);
  `async_trait` (the `Workflow` trait — already a core dep, declares async
  signature, pulls no runtime into core).
- **Journal store:** `redb` 2.x (shared substrate); `ciborium` (CBOR codec —
  already in graph from ADR-0035). **No new external dependency.**
- **DST:** `turmoil` + `Sim*` adapters; the K4 invariant on the CI critical path.
- **No proprietary deps; no contract tests this phase** (no external boundary —
  ACME/DNS lands Phase 3+ with real first-party workflows).

## Wave: DESIGN / [REF] Decisions Table

| ID | Decision | ADR |
|---|---|---|
| DDD-1 | Journal = second redb table layout, distinct `JournalStore` port | 0063 §1 |
| DDD-2 | Journal codec = CBOR (`ciborium`), not rkyv envelope | 0063 §2 |
| DDD-3 | Replay = engine cursor; `ctx.*` check-then-record | 0064 §3 |
| DDD-4 | `WorkflowCtx` surface additive per slice | 0064 §4 |
| DDD-5 | Engine runs off the action-shim; reconciler stays pure-sync | 0064 §5 |
| DDD-6 | Trait+ctx in core (no tokio); engine+journal in control-plane | 0064 §1 |
| DDD-7 | `WorkflowResult` distinct from `TerminalCondition` | 0064 §2 |

## Wave: DESIGN / [REF] Reuse Analysis

| Existing component | File | Overlap | Decision | Justification |
|---|---|---|---|---|
| `Action::StartWorkflow` placeholder | `reconcilers/mod.rs:373` | lifecycle trigger | **EXTEND** | Already the exact D-INH-3 shape; engine consumes it off the shim |
| `WorkflowSpec` placeholder | `reconcilers/mod.rs:562` | the spec | **EXTEND** (make concrete) | Already in core (Action is core); replace empty struct |
| `ReplayEquivalentEmptyWorkflow` invariant + evaluator | `overdrive-sim/.../mod.rs:136`,`evaluators.rs:584` | replay DST invariant | **EXTEND** (graduate) | Placeholder says "Phase 2 replaces with actual journal replay"; K4 is that |
| `RedbViewStore`/`ViewStore`/`SimViewStore` | `view_store/{mod,redb}.rs` | redb durable memory; fsync ordering; bulk-load; probe; CBOR | **REUSE substrate+discipline; CREATE NEW port** | THE central call (ADR-0063 §1). Substrate shared; trait+layout differ — distinct `JournalStore` avoids two-contracts-on-one-trait, zero reuse loss |
| action-shim `dispatch` + reconciler runtime | `action_shim/mod.rs:446`, `reconciler_runtime` | per-tick async-effect pipeline | **EXTEND** | Engine driven off the same shim; `StartWorkflow` no-op arm → `engine.start` |
| `Clock`/`Transport`/`Entropy` port traits | `traits/` | injected non-determinism | **REUSE** | `WorkflowCtx` is a new wrapper over existing ports; no new port |
| `CorrelationKey`/`HttpCall` machinery | `id.rs:538`, `reconcilers/mod.rs:357` | instance correlation + remote idempotency-key precedent | **REUSE** | instance-level `CorrelationKey` keys the terminal row; `HttpCall`'s idempotency-key shape is the precedent for making a `ctx.run` closure's remote effect exactly-once |
| `TerminalCondition` | `overdrive-core` | terminal modelling | **DO NOT REUSE (relate)** | `WorkflowResult` ≠ allocation claim; inherits SemVer convention not type |
| `TickContext` | `overdrive-core/reconcilers` | injected bundle | **DO NOT REUSE (analogue)** | `WorkflowCtx` is the analogue; carries full ctx surface, not just time |
| `JournalStore`/`RedbJournalStore`/`SimJournalStore` | NEW | journal layout | **CREATE NEW** | No existing trait hosts append-only-ordered point-access; mirrors ViewStore line-for-line |
| `WorkflowEngine` | NEW | durable-async executor | **CREATE NEW** | No existing component runs an async body with journaled awaits; reconciler is pure-sync (R3) |

**Verdict: 6 EXTEND/REUSE, 2 DO-NOT-REUSE-(relate), 2 CREATE NEW (justified).**

## Wave: DESIGN / [REF] Open Questions / Deferrals

Deferred to DISTILL/DELIVER or forward phases (no NEW deferrals introduced by
DESIGN; all cite real issues per CLAUDE.md):

- **Cross-node resume** — [#205](https://github.com/overdrive-sh/overdrive/issues/205) (journal `WorkflowId`-keyed behind a trait — not precluded).
- **Operator `overdrive workflow` CLI** — [#206](https://github.com/overdrive-sh/overdrive/issues/206) (ad-hoc journal-query view is a deferrable read-only projection, not a replay requirement).
- **Signals under partition** — [#207](https://github.com/overdrive-sh/overdrive/issues/207) (slice 03 ships in-process single-node delivery).
- **Journal retention/compaction** — [#208](https://github.com/overdrive-sh/overdrive/issues/208) (a range-delete per terminal `WorkflowId`; layout supports it).
- **WASM SDK + version-skew code-graph hashing** — [#209](https://github.com/overdrive-sh/overdrive/issues/209) (no design element hinges on it — R1/D-INH-6).
- **`JournalEntry` digest resolution for full-body observability** — a future #206/#208 concern; replay needs only the digest.
- **Outcome Collision Check:** registry `docs/product/outcomes/registry.yaml`
  not present — skipped (no fabrication).

---

## Wave: DISTILL / [REF] Reconciliation result

Wave: DISTILL (Quinn / nw-acceptance-designer) · Date: 2026-06-05 · Density:
lean. Architecture **locked to B′**; ATs designed OVER it (not re-litigated).

**Reconciliation passed — 0 contradictions.** Confirmed (orchestrator
pre-cleared): no DEVOPS wave (WARN — DST-internal primitive, no env-matrix
dependency; defaults used). DISCUSS vs DESIGN are mutually consistent on every
load-bearing decision — redb-not-libSQL (D-INH-2 / DDD-1/2 / K5), CBOR codec
(DDD-2), single-node crash-resume only (D3 / #205), all non-determinism through
`ctx` (D-INH-4), no Raft bypass (slice-03 AC2), no `overdrive workflow` CLI verb
(D4 / #206), engine-off-shim + reconciler-pure (DDD-5). No NEW contradiction
found beyond the orchestrator's clearance.

## Wave: DISTILL / [REF] Scenario list with tags

`.feature` files are forbidden here (`.claude/rules/testing.md`); the scenario
SSOT is `distill/test-scenarios.md` (GWT spec, prose-only) and the executable
RED scaffolds are Rust `#[should_panic(expected = "RED scaffold")]` tests.

| Scenario | Title | Tags | AC | KPI/ODI | Scaffold file |
|---|---|---|---|---|---|
| S-WP-01-01 | Author writes one ordinary async sequence → terminal | `@driving_port @in-memory @kpi` | US-WP-1 AC1 | K6/O3 | `overdrive-core/tests/acceptance/workflow_trait_drives_to_terminal.rs` |
| S-WP-01-02 | A durable body has zero step-machine boilerplate | `@driving_port @in-memory @property @kpi` | US-WP-1 AC1 (K6) | K6/O3 | `overdrive-core/tests/acceptance/workflow_body_has_no_step_machine.rs` |
| S-WP-01-03 | Non-determinism flows through `ctx`, never the runtime | `@driving_port @in-memory @property @error` | US-WP-1 AC2 | O5 | `overdrive-core/tests/acceptance/workflow_body_routes_nondeterminism_through_ctx.rs` |
| S-WP-01-04 | A completed step is recorded in real redb before suspend | `@driving_port @real-io @kpi` | US-WP-2 AC1/AC2 | K5/O6 | `overdrive-control-plane/tests/integration/workflow_journal/journal_writes_to_redb.rs` |
| S-WP-01-05 | The journal records inputs, not a derived cache | `@in-memory @property` | US-WP-2 AC3 | O6 | `overdrive-sim/tests/acceptance/journal_records_inputs_not_derived.rs` |
| S-WP-01-06 | **WS:** kill mid-run → completed step not repeated | `@walking_skeleton @driving_port @dst @in-memory @error @property @kpi` | US-WP-3 AC1/2/3/4; slice-01 AC1/2/5 | K1/K2/K3 (O1/O2/O4) | `overdrive-sim/tests/acceptance/workflow_crash_resume_exactly_once.rs` |
| S-WP-01-07 | A committed step survives the crash (not lost) | `@dst @in-memory @error @kpi` | US-WP-3 AC2 | K2/O2 | `overdrive-sim/tests/acceptance/workflow_committed_step_survives_crash.rs` |
| S-WP-01-08 | Lifecycle reconciler re-hydrates running instance on restart | `@driving_port @in-memory @kpi` | US-WP-3 AC4 | O2 | `overdrive-control-plane/tests/acceptance/lifecycle_reconciler_rehydrates_on_restart.rs` |
| S-WP-01-09 | Replay-equivalence is a named DST invariant, seeded | `@dst @in-memory @property @kpi` | US-WP-4 AC1/2/3; slice-01 AC3 | **K4/O5 (load-bearing)** | `overdrive-sim/tests/acceptance/replay_equivalence_provision_record_invariant.rs` |
| S-WP-01-10 | fsync failure does not advance the journal cursor | `@dst @in-memory @error @property @kpi` | US-WP-2 AC2 | O1/O6 | `overdrive-sim/tests/acceptance/workflow_journal_write_ordering.rs` |
| S-WP-01-11 | Action-shim dispatches StartWorkflow to the engine off the shim, not a reconcile loop | `@driving_port @in-memory @kpi` | DDD-5 / ADR-0064 §5 | O3 (R3) | `overdrive-control-plane/tests/acceptance/action_shim_dispatches_start_workflow_to_engine.rs` |
| S-WP-02-01 | Crash spanning the sleep window does not repeat pre-sleep step | `@driving_port @dst @in-memory @error @property @kpi` | slice-02 AC1 | K1/O1 | `overdrive-sim/tests/acceptance/workflow_sleep_crash_pre_sleep_step_not_repeated.rs` |
| S-WP-02-02 | Post-sleep step fires only at/after the original deadline | `@dst @in-memory @error @property @kpi` | slice-02 AC2 | K3/O4 | `overdrive-sim/tests/acceptance/workflow_sleep_resumes_to_original_deadline.rs` |
| S-WP-02-03 | Sleep entry records deadline (input), not "remaining" | `@in-memory @property` | slice-02 AC4 | O3/O6 | `overdrive-sim/tests/acceptance/workflow_sleep_records_deadline_not_remaining.rs` |
| S-WP-02-04 | Replay-equivalence holds across the sleep, seeded | `@dst @in-memory @property @kpi` | slice-02 AC3 | K4/O5 | `overdrive-sim/tests/acceptance/replay_equivalence_holds_across_sleep.rs` |
| S-WP-03-01 | Blocked-on-signal re-blocks on the SAME signal after crash | `@driving_port @dst @in-memory @error @property @kpi` | US-WP-5 AC1 | K1/O1 | `overdrive-sim/tests/acceptance/workflow_signal_wait_reblocks_after_crash.rs` |
| S-WP-03-02 | A satisfied signal is not re-waited on resume | `@dst @in-memory @error @property` | slice-03 AC1 | O1 | `overdrive-sim/tests/acceptance/workflow_signal_already_seen_not_rewaited.rs` |
| S-WP-03-03 | `ctx.emit_action` lands in Raft channel, no IntentStore write | `@driving_port @in-memory @property @kpi` | US-WP-5 AC2 | O3 | `overdrive-control-plane/tests/acceptance/workflow_emit_action_lands_in_raft_channel.rs` |
| S-WP-03-04 | An emitted Action is not re-emitted after a crash | `@dst @in-memory @error @property @kpi` | US-WP-5 AC3 | K1/O1 | `overdrive-sim/tests/acceptance/workflow_emit_action_not_re_emitted_after_crash.rs` |
| S-WP-03-05 | Replay-equivalence holds across signal wait + emit, seeded | `@dst @in-memory @property @kpi` | US-WP-5 AC4 | K4/O5 | `overdrive-sim/tests/acceptance/replay_equivalence_holds_across_signal_and_emit.rs` |

**20 scenarios** · **9 `@error` scenarios (~45%, ≥40% gate met)** · **1
`@walking_skeleton`** · every US-WP-1..5 AC mapped · S-WP-01-11 added in the
consolidated review to give the action-shim `StartWorkflow` dispatch arm
(DDD-5, the RATIFY-flagged engine↔reconciler boundary) dedicated coverage.

## Wave: DISTILL / [REF] WS strategy

**Strategy B (thinnest end-to-end vertical slice).** Slice 01 IS the walking
skeleton; the ONE `@walking_skeleton` scenario is **S-WP-01-06** — "Devon kills
the process mid-run and the completed step is not repeated on restart" — which
closes the full durable-execution loop (author surface → `ctx.run` → redb
journal write → lifecycle-reconciler bring-up via `Action::StartWorkflow` →
single-node crash-resume under DST → exactly-once effect → byte-identical
terminal → observable ObservationStore terminal row). Justification: a
non-technical stakeholder reads it and confirms "yes, that is what durable
execution must do" (Mandate 5 litmus); it is the demo-able user-value E2E, not
a layer-connectivity proof.

## Wave: DISTILL / [REF] Adapter coverage table

Per Mandate 6, every DRIVEN port from DESIGN mapped to ≥1 scenario. In this DST
primitive the in-process "real" driven-internal adapter is the `Sim*` adapter
(it honours the same trait contract — per the project ATDD Infrastructure
Policy). The `JournalStore` (NEW) additionally has a **real-redb** persistence
scenario (`@real-io`, `integration-tests`) per AC4/O6.

| Driven port | Sim adapter (default lane) | Real-IO scenario | Covered by |
|---|---|---|---|
| `JournalStore` (NEW) | `SimJournalStore` | **YES** — S-WP-01-04 (`@real-io`, real `RedbJournalStore`) | S-WP-01-04, 01-05, 01-07, 01-10 |
| `Transport` (REUSE) | `SimTransport` | covered via DST (call-count == 1) | S-WP-01-06, 02-01 |
| `Clock` (REUSE) | `SimClock` | covered via DST (deadline park) | S-WP-02-02 |
| `Entropy` (REUSE) | `SeededEntropy` | covered via seed reproduction | S-WP-01-09, 02-04, 03-05 |
| `ObservationStore` (REUSE) | `SimObservationStore` | covered via DST (terminal row + signals) | S-WP-01-06, 03-01, 03-02 |
| Action channel → Raft (REUSE) | sim harness | covered via DST (emit lands in channel) | S-WP-03-03, 03-04 |

Zero "NO — MISSING" rows. The one NEW driven port (`JournalStore`) carries the
real-redb integration scenario; the REUSE ports are exercised through their
existing `Sim*` adapters in the DST lane (their real-adapter integration tests
already exist in the brownfield substrate).

## Wave: DISTILL / [REF] Scaffolds

21 RED scaffold files (all `#[should_panic(expected = "RED scaffold")]`,
RED-not-BROKEN, import no unbuilt production type):

- **overdrive-core** `tests/acceptance/`: `workflow_trait_drives_to_terminal.rs`, `workflow_body_has_no_step_machine.rs`, `workflow_body_routes_nondeterminism_through_ctx.rs` (S-WP-01-01/02/03).
- **overdrive-sim** `tests/acceptance/`: `journal_records_inputs_not_derived.rs`, `workflow_crash_resume_exactly_once.rs`, `workflow_committed_step_survives_crash.rs`, `replay_equivalence_provision_record_invariant.rs`, `workflow_journal_write_ordering.rs` (slice 01); `workflow_sleep_crash_pre_sleep_step_not_repeated.rs`, `workflow_sleep_resumes_to_original_deadline.rs`, `workflow_sleep_records_deadline_not_remaining.rs`, `replay_equivalence_holds_across_sleep.rs` (slice 02); `workflow_signal_wait_reblocks_after_crash.rs`, `workflow_signal_already_seen_not_rewaited.rs`, `workflow_emit_action_not_re_emitted_after_crash.rs`, `replay_equivalence_holds_across_signal_and_emit.rs` (slice 03).
- **overdrive-control-plane** `tests/acceptance/`: `action_shim_dispatches_start_workflow_to_engine.rs` (S-WP-01-11), `lifecycle_reconciler_rehydrates_on_restart.rs` (S-WP-01-08), `workflow_emit_action_lands_in_raft_channel.rs` (S-WP-03-03).
- **overdrive-control-plane** `tests/integration/workflow_journal/`: `journal_writes_to_redb.rs` (S-WP-01-04, `@real-io`, `integration-tests`).

Wired into `overdrive-core/tests/acceptance.rs`, `overdrive-sim/tests/acceptance.rs`,
`overdrive-control-plane/tests/acceptance.rs`, and
`overdrive-control-plane/tests/integration.rs` (the `mod integration { mod
workflow_journal { … } }` subtree). RED-classification: `distill/red-classification.md`.

## Wave: DISTILL / [REF] Test placement

- **Default DST lane** (`tests/acceptance/*.rs`, `Sim*` in-process, `cargo dst`): all 17 non-real-IO scenarios. Precedent: `reconciler_is_pure_with_workload_lifecycle.rs` and `sim_view_store.rs` — sibling `Sim*`-adapter DST acceptance tests in the same `overdrive-sim/tests/acceptance/` directory. The DST invariant scenarios NAME the future `ReplayEquivalenceProvisionRecord` / `WorkflowJournalWriteOrdering` / `WorkflowExactlyOnceEffectOnResume` variants.
- **Real-redb lane** (`tests/integration/workflow_journal/journal_writes_to_redb.rs`, gated `integration-tests`): the ONE journal-persistence scenario that opens a real redb file (S-WP-01-04), per `.claude/rules/testing.md` § "Integration vs unit gating" (real filesystem I/O → `integration-tests` feature + `tests/integration/`). Precedent: `tests/integration/redb_view_store.rs` (the sibling real-redb `ViewStore` test).

## Wave: DISTILL / [REF] Driving Adapter coverage

- **Author surface (`impl Workflow` / `ctx.*`):** S-WP-01-01/02/03 (the `Workflow` trait + `WorkflowCtx` surface), S-WP-02-* (`ctx.sleep`), S-WP-03-* (`ctx.wait_for_signal`, `ctx.emit_action`).
- **Lifecycle trigger (`Action::StartWorkflow { spec, correlation }`):** S-WP-01-06 (bring-up), S-WP-01-08 (re-hydrate on restart), S-WP-01-11 (the action-shim dispatch arm hands the instance to `WorkflowEngine::start` off the shim, not a reconcile loop — the DDD-5 engine↔reconciler boundary).
- **NO CLI verb exists (#206).** There is NO `overdrive workflow` subcommand (cli.rs: deploy/job/node/alloc/cluster only). No scenario invents one. Ana's (operator) Phase-1 observable surface is the ObservationStore terminal-result row + structured lifecycle event + the `replay_equivalence_*` DST invariant as executable evidence — asserted by S-WP-01-06 (terminal row) and S-WP-01-09 (the named invariant).

## Wave: DISTILL / [REF] Pre-requisites

DESIGN driving/driven ports + the brownfield substrate the scenarios depend on:

- **Driving:** `Workflow` trait + `WorkflowCtx` (core, NEW — DELIVER), `Action::StartWorkflow` (core, EXTEND placeholder `reconcilers/mod.rs:373`).
- **Driven (NEW):** `JournalStore` port + `RedbJournalStore` (control-plane) + `SimJournalStore` (sim) — ADR-0063.
- **Driven (REUSE, brownfield-verified):** reconciler runtime, redb ViewStore (shared `Arc<Database>`), Action channel → Raft, `ObservationStore`, `Clock`/`Transport`/`Entropy` port traits, `CorrelationKey`/`HttpCall` machinery, the DST harness.
- **Graduated:** the `ReplayEquivalentEmptyWorkflow` placeholder invariant (`overdrive-sim/src/invariants/mod.rs:136`) becomes `ReplayEquivalenceProvisionRecord` (K4) — DELIVER slice 01.
- **No external dependency outside the workspace.** No DEVOPS env-matrix (DST-internal). No new external/non-deterministic driven port (no clock/email/SMS/payment/LLM/API fake needed).

## Wave: DISTILL / [REF] DST invariant catalogue delta

Three `overdrive-sim::invariants::Invariant` variants the DISTILL scaffolds
NAME; they LAND (graduate) in DELIVER (ADR-0064 §6, ADR-0063 §6):

1. **`ReplayEquivalenceProvisionRecord`** — graduates the placeholder `ReplayEquivalentEmptyWorkflow` (`mod.rs:136`); replaces the `evaluate_replay_equivalent_empty_workflow` two-SimEntropy-transcript stub with a real journal replay against the engine + `SimJournalStore`. Uninterrupted-vs-crash-resumed trajectory byte-equality + `assert_eventually!(is_terminal)` bounded progress. **K4, on the CI critical path.** (S-WP-01-09; extended for S-WP-02-04 / S-WP-03-05.)
2. **`WorkflowJournalWriteOrdering`** — under `SimJournalStore` with injected fsync-failure on the next append, the engine does not advance the cursor / suspend (mirrors ADR-0035 `WriteThroughOrdering`). (S-WP-01-10.)
3. **`WorkflowExactlyOnceEffectOnResume`** — asserts the replay-path guarantee: crash AFTER a `ctx.run` step records but before terminal → resume → the journaled result is returned and the closure is NOT re-fired → `SimTransport` call count == 1 (K1). (Exactly-once on replay, not unconditional — a fire→fsync-window crash re-runs the closure at-least-once; S-WP-01-06.)

DISTILL scaffolds the acceptance tests that NAME these; DELIVER lands the enum
variants + evaluators.

## Wave: DISTILL / [REF] Self-review + mandate compliance

- **WS strategy declared:** Strategy B; WS scenario S-WP-01-06 tagged `@walking_skeleton @driving_port @dst`. ✔
- **Every driven port has a scenario; `JournalStore` (NEW) has a `@real-io` real-redb scenario** (S-WP-01-04). ✔
- **What `Sim*` doubles CANNOT model (documented):** wall-clock fidelity (DST drives logical time — production `SystemClock` is the same trait, no extra surface, slice-02 OUT-of-scope note); real-kernel/multi-node behaviour (single-node only, #205); cross-node signal delivery under partition (#207). These are honestly OUT of Phase-1 scope, not silently asserted.
- **RED-not-BROKEN confirmed:** all 3 crates compile `--no-run` in Lima; the 20 scaffolds run and PASS at the bar via `#[should_panic(expected = "RED scaffold")]`. No IMPORT_ERROR / SETUP_FAILURE. ✔
- **Scaffold convention applied:** project `.claude/rules/testing.md` § "RED scaffolds" shape (`#[should_panic(expected = "RED scaffold")]` + `panic!("Not yet implemented -- RED scaffold (S-WP-NN-NN / …)")`), NOT the generic `__SCAFFOLD__` marker (Rust override). ✔
- **Honesty constraints:** no `overdrive workflow` CLI verb (#206); no cross-node resume claimed (#205, D3 — every crash-resume scenario notes single-node); journal on redb-CBOR not libSQL (K5 asserted S-WP-01-04); all non-determinism through `ctx` (S-WP-01-03 + negative test); `ctx.emit_action` → Raft channel, no IntentStore bypass (S-WP-03-03). ✔
- **CM-A (Mandate 1, hexagonal):** scaffolds enter through the author surface (`impl Workflow`/`ctx.*`), the lifecycle trigger (`Action::StartWorkflow`), or the named DST invariant — never an internal component. **CM-B (Mandate 2, business language):** scenario titles are business outcomes (resume exactly-once, prove replay-equivalence, signal re-blocks). **CM-C (Mandate 3, journeys):** the WS is the complete crash-resume journey with observable terminal. **Mandate 9 (PBT mode):** the `replay_equivalence_*` invariant IS the property (any seed → bit-identical); slice-03 sad paths are example-pinned (Mandate 11). **Mandate 10:** Tier B state-machine PBT is NOT emitted — the journey's rich behaviour is already covered by the DST replay invariant (the project's native equivalent), and the input space is digest/seed-shaped, not domain-rich free-text; Tier A (DST + named invariant) is the correct single tier here.

### Consolidated review (the mandatory end-of-DISTILL gate)

Two reviewers dispatched in parallel (Haiku) against the full `feature-delta.md`
+ the `.feature`-replacement GWT spec + the 21 scaffolds (the product-owner and
platform-architect reviewers were scoped out — DISCUSS was already APPROVED by
Eclipse and unchanged in DISTILL, and this feature has no DEVOPS wave):

- **`nw-acceptance-designer-reviewer` (Sentinel) — APPROVED**, 0 blockers / 0
  high / 0 low. AC + KPI completeness, WS integrity, adapter coverage, error-path
  %, RED convention, traceability, honesty all PASS.
- **`nw-solution-architect-reviewer` (Architect) — CONDITIONALLY_APPROVED**, 0
  blocker / 1 high / 0 low. The high finding: the engine↔reconciler boundary
  (DDD-5, the RATIFY-flagged "subtlest decision") was under-covered — the
  action-shim `StartWorkflow` dispatch arm (a DESIGN EXTEND component) had only
  *implicit* coverage via the walking skeleton. **Resolved in-wave** (not
  deferred to DELIVER): added dedicated scenario **S-WP-01-11** asserting the
  shim hands the instance to `WorkflowEngine::start` off the shim (not a
  reconcile loop), and strengthened **S-WP-01-08** to name that `ReconcilerIsPure`
  holds with the workflow-lifecycle reconciler registered. Both new/edited
  scaffolds re-compiled RED-not-BROKEN in Lima. No remaining blocker or high.

DELIVER handoff cleared: both verdicts APPROVED / CONDITIONALLY_APPROVED with the
sole high finding resolved in-wave.
