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

- **[D-INH-1] Distinct durable-async `Workflow` primitive.** `async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult` with `ctx.call`/`ctx.sleep`/`ctx.wait_for_signal`/`ctx.activity`. NOT reconciler-as-step-machine (Option C is runner-up). (wave-decisions § RATIFIED, R3.)
- **[D-INH-2] Step/await journal in redb**, per-instance append-only, distinct layout from the reconciler `View` store, same backend. NOT libSQL. (R2; whitepaper §17/§18.)
- **[D-INH-3] Instance lifecycle owned by the workflow-lifecycle reconciler** (spec→running→journaled→terminated); reconcilers emit `Action::StartWorkflow { spec, correlation }`. (R3; whitepaper §18.)
- **[D-INH-4] All non-determinism through `ctx`** (injected Clock/Transport/Entropy, DST-controllable). Workflow→cluster mutations via typed Actions through Raft, never direct IntentStore writes. Cross-workflow coordination via typed signals in the ObservationStore.
- **[D-INH-5] Correctness = deterministic replay + bounded progress** — replay the journal twice under DST → bit-identical; reach terminal within a declared step budget. §21 DST obligation, ties O5.
- **[D-INH-6] Version-skew mitigation deferred WITH the app SDK** (R1) — first-party Rust workflows ship journal + code in one binary, recompiled as a unit; the hazard is minor and its fix (code-graph hashing) rides with the deferred WASM SDK. No story may hinge on it.

DISCUSS-made:

- **[D1] Feature type: Backend** (platform-internal control-plane primitive). Users = platform engineers (authors) + operators. (Orchestrator-set.)
- **[D2] Walking skeleton: YES, thinnest end-to-end durable workflow** — `Workflow` trait + `WorkflowCtx` with ONE durable op, redb journal write at the await, lifecycle reconciler brings up one instance via `Action::StartWorkflow`, single-node crash-resume under DST, observable terminal. First consumer: **`ProvisionRecord`** (a minimal 2-step `ctx.call → terminal` sequence with a real non-idempotent-to-repeat effect) — NOT cert-rotation (needs slice-02/03 surface). (slice-01.)
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
production-data AC (real `ctx.call` effect, real SimTransport call-count
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
After: write `impl Workflow for ProvisionRecord { async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult { let _ = ctx.call(write_record_req).await?; Ok(WorkflowResult::Success) } }` → `cargo dst --only replay_equivalence_provision_record` shows `ok · step 0 executed once · terminal result == uninterrupted-run terminal`.
Decision enabled: Devon decides she can model the next platform sequence (cert rotation) as ordinary control flow rather than a bespoke state machine.

#### Acceptance Criteria
- [ ] AC1: A `Workflow` trait exists with `async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult`; an author writes `ProvisionRecord` with NO step enum and NO transition match (O3). (Verifies the "After" author surface.)
- [ ] AC2: The workflow body performs its side effect through `ctx.call(...).await` only; a `dst-lint`-style check / review confirms no `Instant::now()` / `reqwest` / `tokio::time::sleep` / `rand` in the body (D-INH-4).
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
- [ ] AC1 (O1): Killing the instance AFTER `ctx.call` records but BEFORE terminal, then restarting, executes the `ctx.call` external effect EXACTLY ONCE (SimTransport call count == 1), not twice.
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
| K6 (O3) | the workflow author | hand-writes step-machine boilerplate per sequence | 0 step-enum / transition-match lines in a workflow body | code review of `ProvisionRecord` (one `async fn run`, no step enum) | full state machine per sequence |

K4 is the load-bearing KPI: a green, seeded, reproducible
`replay_equivalence_*` invariant on the CI critical path IS the proof of O5
and the gate the feature exists to deliver. (Note O2 cross-node resume is NOT
a Phase-1 KPI — K2 is scoped single-node per D3.)

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy B (thinnest end-to-end vertical slice).** Slice 01 IS the walking
skeleton: `Workflow` trait + `WorkflowCtx` (one durable `ctx.call`) + redb
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
3. **3+ domain examples with real data** — PASS. `ProvisionRecord` (record-write effect), cert rotation (ACME CSR — the non-idempotent example), region migration (quiesce source) named throughout; `ctx.call` with a concrete request shape in US-WP-1's pitch.
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
