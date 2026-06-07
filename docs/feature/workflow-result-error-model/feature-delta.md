<!-- markdownlint-disable MD024 -->
# Feature Delta — workflow-result-error-model (amends ADR-0064; resolves #217, unblocks #40)

Wave: DESIGN (Morgan / nw-solution-architect) · Date: 2026-06-06 · Mode:
PROPOSE. **No DISCUSS wave** — the evidence base is the completed research
investigation `docs/research/workflow-durable-execution/result-error-retry-semantics-research.md`
(High confidence, 4 platforms). Paradigm: object-oriented Rust (project
CLAUDE.md — not re-litigated). Greenfield single-cut: the old
`WorkflowResult`-as-body-return shape is deleted and the new contract lands
in the same change; no migration/versioning bridge (the only registered
workflow is the `provision-record` test fixture — no production instances).

ADR: **ADR-0065** (NEW; amends ADR-0064 §2/§3/§5/§6). Amendment-vs-
supersession call + rationale in ADR-0065 § "Considered alternatives →
Alternative A".

---

## [REF] DDD — decisions + verdicts

The four in-scope priorities, each as a numbered decision with the recommended
verdict. Trade-off content is under [WHY] below.

- **[D1] Object safety — typed author trait + CBOR-erasing adapter at the
  registration edge; engine holds `Box<dyn ErasedWorkflow>`.** VERDICT:
  ACCEPTED. The author writes `Workflow { type Output; type Input; async fn run(&self, ctx, input: Self::Input) -> Result<Self::Output, TerminalError> }`;
  a generic `ErasedWorkflowAdapter<W>` blanket-erases `Output`/`Input` to CBOR
  into an object-safe `ErasedWorkflow { run_erased(&self, ctx, input_bytes: &[u8]) -> Result<Vec<u8>, TerminalError> }`.
  Mirrors the existing `ctx.run<T>` typed-edge / CBOR-erased-interior split.
- **[D2] `TerminalError` — concrete core type, thiserror-adjacent, serde.**
  VERDICT: ACCEPTED. `TerminalError { kind: TerminalErrorKind, detail: String }`
  with `kind ∈ {Explicit, BudgetExhausted, MalformedInput, OutputEncode}`,
  `detail` length-capped at construction (closes the free-text replay-
  determinism hazard ADR-0064 §3 worked around). `BudgetExhausted` is
  engine-minted; the author cannot construct it.
- **[D3] Control-plane terminal-status projection — engine-owned
  `WorkflowStatus`, distinct from the body return AND from
  `TerminalCondition`.** VERDICT: ACCEPTED.
  `WorkflowStatus ∈ {Completed{output: Vec<u8>}, Failed{terminal: TerminalError}, Cancelled, TimedOut}`
  (`#[non_exhaustive]`; `Cancelled`/`TimedOut` are forward variants the
  Phase-1 engine never writes but the reconciler matches exhaustively). Lives
  in `overdrive-core::workflow`; carried by the journal `Terminal` command
  and `ObservationRow::WorkflowTerminal`. Engine maps `Ok` → `Completed`,
  `Err(TerminalError)` / budget-exhausted → `Failed`.
- **[D4] Retryable-vs-terminal model + retry budget in the engine/journal,
  NOT the body.** VERDICT: ACCEPTED. Retryable = engine absorbs + re-drives;
  terminal = explicit `Err(TerminalError)` or engine-minted `BudgetExhausted`.
  Budget POLICY is an engine constant (not persisted); budget INPUTS
  (attempts, last-failure) derive from journal `RetryAttempted` entries
  (additive command, ADR-0066 §2) recomputed against the live policy. The
  full re-drive loop is Slice 04 (types + success/explicit-terminal paths
  land Slices 01–03; body contract stable from Slice 01).
- **[D5] Typed `WorkflowStart.input` crossing Raft with rkyv-envelope
  discipline; resolves #217.** VERDICT: ACCEPTED.
  `WorkflowStart { name: WorkflowName, input: Vec<u8> }` (opaque CBOR
  `W::Input`); the durable desired-intent persists the FULL spec via a
  `WorkflowStartEnvelope` (V1) + co-located typed codec
  (`archive_for_store`/`from_store_bytes`, ADR-0048 `Job` precedent), NOT the
  name bytes; `started_digests` derives `input_digest = ContentHash::of(&spec.input)`
  (discharges the `TODO(#217)`).

## [REF] Component decomposition (paths + change type)

| Component | Path | Change |
|---|---|---|
| `Workflow` trait (typed `Output`/`Input`/`run`) | `crates/overdrive-core/src/workflow/mod.rs` | MODIFY (signature reshape) |
| `ErasedWorkflow` trait + `ErasedWorkflowAdapter<W>` | `crates/overdrive-core/src/workflow/mod.rs` | CREATE |
| `TerminalError` + `TerminalErrorKind` | `crates/overdrive-core/src/workflow/mod.rs` | CREATE |
| `WorkflowStatus` (control-plane projection) | `crates/overdrive-core/src/workflow/mod.rs` | CREATE |
| `WorkflowResult` enum | `crates/overdrive-core/src/workflow/mod.rs` | DELETE |
| `WorkflowStart` (+ `input`, envelope, typed codec) | `crates/overdrive-core/src/workflow/mod.rs` | MODIFY |
| `WorkflowStartEnvelope` (V1) + schema-evolution fixture | `crates/overdrive-core/src/workflow/mod.rs` + `crates/overdrive-core/tests/schema_evolution/workflow_start.rs` | CREATE |
| `WorkflowFactory` / `WorkflowRegistry` (→ `ErasedWorkflow`) | `crates/overdrive-control-plane/src/workflow_runtime/mod.rs` | MODIFY |
| `WorkflowEngine::start` (run_erased, status write, short-circuit, started_digests) | `crates/overdrive-control-plane/src/workflow_runtime/mod.rs` | MODIFY |
| `JournalCommand::Terminal` (`result` → `status`); `JournalCommand::RetryAttempted` (D4, additive) | `crates/overdrive-control-plane/src/journal/mod.rs` | MODIFY |
| `ObservationRow::WorkflowTerminal` (`result` → `status`) | `crates/overdrive-core/src/traits/observation_store.rs` | MODIFY |
| `WorkflowInstanceState.terminal: Option<WorkflowStatus>` | `crates/overdrive-core/src/reconcilers/workflow_lifecycle.rs` | MODIFY |
| `persist_workflow_intents` (persist full spec via envelope codec) | `crates/overdrive-control-plane/src/action_shim/mod.rs` | MODIFY |
| workflow-lifecycle `hydrate_desired` (read spec via `from_store_bytes`) | `crates/overdrive-control-plane/src/reconciler_runtime.rs` | MODIFY |
| `ProvisionRecord` / `*WithSleep` / `*WithSignalEmit` fixtures (→ `Output=()`, `Input=()`, `Result<(), TerminalError>`) | `crates/overdrive-core/src/testing/workflow.rs` | MODIFY |
| `replay_equivalence_provision_record` + new `WorkflowTerminalStatusProjection` invariant | `crates/overdrive-sim/src/invariants/{mod.rs,evaluators.rs}` | MODIFY/CREATE |

## [REF] Driving / driven ports

- **Driving (into the hexagon):** `Action::StartWorkflow { start: WorkflowStart, correlation }`
  — the reconciler→engine trigger (unchanged variant; `spec` now typed-param-
  bearing). The action-shim is the driving adapter that persists intent +
  drives the engine off the shim.
- **Driven (out of the hexagon):** `JournalStore` (durable `await`-point +
  `Terminal` status + `RetryAttempted`); `ObservationStore::write`
  (`WorkflowTerminal { status }` projection row). Both unchanged in trait
  shape — only the carried types reshape. The `Clock`/`Transport`/`Entropy`
  ports on `WorkflowCtx` are untouched.
- **Author surface (not a port — a core trait):** `Workflow` (typed) +
  `ErasedWorkflow` (object-safe engine surface). The erasure adapter is the
  anti-corruption layer between the typed author edge and the `dyn`-dispatched
  engine interior.

## [REF] Technology choices

| Choice | Tech | Rationale | License |
|---|---|---|---|
| Body output/input erasure codec | `ciborium` (CBOR) | Already the journal step-result + `ctx.run<T>` codec (ADR-0066 §2); homogeneous durable surface; runtime-memory class | Apache-2.0/MIT (workspace dep) |
| Durable `WorkflowStart` intent codec | `rkyv` versioned envelope (ADR-0048) | Intent-class durable aggregate read across restarts — the `Job` precedent, NOT CBOR (the two codecs stay separate per `development.md`) | MIT (workspace dep) |
| `TerminalError`/`WorkflowStatus` derive | `serde` + `thiserror` (errors) | typed-error discipline (core libraries never eyre); serde for journal/obs carriage | workspace deps |
| Async author trait | `async_trait` | already a core dep; no tokio in core (ADR-0064 §1 carried forward) | MIT/Apache-2.0 |

No new dependencies. No proprietary tech.

## [REF] Decisions table

| ID | Decision | Status | ADR |
|---|---|---|---|
| D1 | Object safety: typed trait + CBOR-erasing adapter | Accepted | 0065 §1 |
| D2 | `TerminalError` concrete core type | Accepted | 0065 §2 |
| D3 | `WorkflowStatus` engine-owned projection | Accepted | 0065 §3 |
| D4 | Retry budget in engine/journal (not body, not View) | Accepted | 0065 §4 |
| D5 | Typed `WorkflowStart.input` + rkyv envelope; #217 | Accepted | 0065 §5 |
| D6 | New ADR (0065) amending 0064, not in-place edit | Accepted | 0065 § Alt A |

## [REF] Reuse Analysis (mandatory hard gate)

| Existing component | File | Overlap | Decision | Justification |
|---|---|---|---|---|
| `ctx.run<T>` CBOR typed-edge/erased-interior | `overdrive-core/src/workflow/mod.rs:714` | The exact "type at the author edge, CBOR at the journal boundary" pattern D1 needs | **REUSE (pattern)** | D1's `ErasedWorkflowAdapter` is the same erasure `ctx.run<T>` already performs for step results — not a new mechanism. |
| `WorkflowStart` placeholder (`name` only) | `overdrive-core/src/workflow/mod.rs:1160` | The spec the trigger carries | **EXTEND** | Add `input: Vec<u8>` + envelope/codec; already core because `Action` is core. |
| `Action::StartWorkflow { start, correlation }` | `overdrive-core/src/reconcilers/mod.rs:380` | The reconciler→engine trigger | **REUSE (unchanged variant)** | `spec` reshapes; the variant shape is exactly D-INH-3. No new Action. |
| `JournalCommand::Terminal { result }` | `overdrive-control-plane/src/journal/mod.rs:326` | The durable terminal | **EXTEND** (`result: WorkflowResult` → `status: WorkflowStatus`) | The lossless-terminal short-circuit already depends on this carrying the full terminal; swap the carried type. |
| `JournalCommand` (additive variants) | `overdrive-control-plane/src/journal/mod.rs:236` | Retry bookkeeping (D4) | **EXTEND** | `RetryAttempted` is an additive `#[serde(default)]` command per ADR-0066 §2 — the established additive-variant pattern. |
| `ObservationRow::WorkflowTerminal { result }` | `overdrive-core/src/traits/observation_store.rs:561` | The observable terminal | **EXTEND** (`result` → `status`) | Same plumbing; swap the carried type to the projection. |
| `WorkflowInstanceState.terminal: Option<WorkflowResult>` | `overdrive-core/src/reconcilers/workflow_lifecycle.rs:66` | Reconciler convergence signal | **EXTEND** (`Option<WorkflowStatus>`) | The `terminal.is_some()` convergence check is unchanged; only the carried type. |
| `WorkflowEngine::started_digests` `TODO(#217)` | `overdrive-control-plane/src/workflow_runtime/mod.rs:542` | `input_digest` derivation | **MODIFY** (discharge #217) | `input_digest = ContentHash::of(&spec.input)`; the marker exists precisely for this. |
| `persist_workflow_intents` (persists `spec.name` bytes) | `overdrive-control-plane/src/action_shim/mod.rs:466` | Durable desired-intent write | **MODIFY** | Persist `spec.archive_for_store()?` (full spec) — the #217 value-side fix. |
| `hydrate_workflow_desired_instances` (parses name bytes) | `overdrive-control-plane/src/reconciler_runtime.rs:2110` | Rehydrate spec from intent | **MODIFY** | Read `WorkflowStart::from_store_bytes(value)?` — the #217 read-side fix. |
| `Job::archive_for_store`/`from_store_bytes` + envelope | `overdrive-core` (ADR-0048 §4b) | The rkyv typed-codec precedent | **REUSE (pattern)** | `WorkflowStart`'s codec is the same shape — co-located typed codec on the value, byte-level store unchanged. |
| `RETRY_BACKOFFS` / `RetryMemory` reconciler precedent | `overdrive-control-plane/src/worker/exit_observer.rs`; `development.md` Reconciler I/O | Retry-budget shape | **CONTRAST, do NOT reuse** | Reconciler has no engine → View-memory; workflow HAS an engine → journal-derived budget (D4). Deliberately the opposite home. |
| `WorkflowResult` enum | `overdrive-core/src/workflow/mod.rs:84` | The old body return | **DELETE** | Greenfield single-cut; replaced by `Result<Output, TerminalError>` (body) + `WorkflowStatus` (projection). |

No component is created where an existing one can be extended; every CREATE
(`ErasedWorkflow`, `TerminalError`, `WorkflowStatus`, `WorkflowStartEnvelope`)
is a genuinely new concept with no existing alternative (verified by the
search above).

## [REF] Open questions

- **OQ-1 (BLOCKER — needs user resolution before DELIVER): #217 / #40 comment
  thread not read.** This subagent has no `gh` access; the design is grounded
  in the in-repo references (the `TODO(#217)` marker, ADR-0064 §1, the
  persist/rehydrate loop) but the issue *comment* threads (where ratified
  scope corrections live, per CLAUDE.md "always fetch comments") were not
  verified. The orchestrator must run `gh issue view 217 --comments` and
  `gh issue view 40 --comments` and confirm D5 matches #217's ratified scope
  (input_digest off parameter bytes) and that #40 needs no additional input/
  output shape this design omits.
- **OQ-2:** D4's `RetryAttempted` journal command — is the attempt count best
  derived from journal entries, or does the engine need a separate in-memory
  attempt counter for the live (pre-crash) re-drive loop that the journal
  count reconstructs on resume? Slice-04 design detail; does not affect the
  body contract (Slices 01–03).
- **OQ-3:** `WorkflowStatus::Completed { output: Vec<u8> }` carries erased CBOR
  — should the observable terminal row ALSO carry the workflow-kind name so an
  operator-facing reader can decode `output` without resolving the registry?
  Deferred; the Phase-1 observable surface is the DST invariant + structured
  events, no operator CLI (ADR-0064 D4 carried forward).

---

## [WHY] Propose-mode trade-offs (per priority)

### P1 — Object safety

- **Option A (RECOMMENDED): typed trait + `ErasedWorkflowAdapter` → `dyn ErasedWorkflow`.**
  Trade-off: one extra blanket-impl adapter type; in exchange the author keeps
  full typing (`Result<CertOutput, TerminalError>`), the engine keeps its
  `dyn` interior unchanged, and the durable surface stays homogeneous CBOR —
  the same split `ctx.run<T>` already proves.
- Option B: single `type Output` + engine `dyn Any` downcast. Trade-off: no
  adapter type, but `Any + Send + serde` compose badly, the registry loses
  the compile-time output type, and the journal needs per-workflow result
  schemas. Rejected.
- Option C: non-generic `run(&self, ctx) -> Result<Vec<u8>, TerminalError>`
  the author implements directly. Trade-off: object-safe with no adapter, but
  every author hand-writes CBOR encode/decode — the boilerplate the adapter
  removes. Rejected.

### P2 — Typed input crossing Raft

- **Option A (RECOMMENDED): `WorkflowStart { name, input: Vec<u8> }`, durable
  intent via rkyv envelope (ADR-0048 `Job` precedent).** Trade-off: one new
  durable schema to evolve (envelope + golden fixture), but it rides the
  established precedent and correctly treats the persisted desired-intent as
  intent-class durable state (read across restarts).
- Option B: keep persisting raw CBOR spec bytes (no rkyv envelope). Trade-off:
  simpler now, but an input-bearing durable intent with no versioned
  envelope violates `development.md` § "rkyv schema evolution" the moment the
  input shape evolves — the exact additive-`Option<T>`-will-be-fine trap.
  Rejected.
- Option C: carry input out-of-band (a separate intent key). Trade-off:
  splits one logical desired-intent across two keys, breaking the atomic
  persist-then-dispatch the action-shim relies on. Rejected.

### P3 — Control-plane status projection

- **Option A (RECOMMENDED): engine-owned `WorkflowStatus` in core, carried by
  journal `Terminal` + obs row, distinct from the body return.** Trade-off: a
  third terminal-modelling type (with `TerminalError`, `TerminalCondition`),
  but it is exactly the two-layer split all four researched platforms use; the
  body return and the observable status are genuinely different concerns.
- Option B: reuse `TerminalCondition` (ADR-0037). Trade-off: one fewer type,
  but `TerminalCondition` is the reconciler's allocation claim — conflating it
  with a workflow's terminal repeats the exact mistake ADR-0064 §2 already
  warned against. Rejected (inherit the SemVer convention, not the type).
- Option C: keep `WorkflowResult` as the body return + add `WorkflowStatus`
  separately. Trade-off: minimal body-side change, but retains all three
  anti-patterns (contentless success, retryable-as-terminal, body-authored
  cancel) the research refutes. Rejected.

### P4 — Retry model + budget location

- **Option A (RECOMMENDED): engine absorbs retryable; budget inputs from the
  journal, policy an engine constant; `BudgetExhausted` engine-minted.**
  Trade-off: the journal grows a `RetryAttempted` command and the re-drive
  loop is a follow-on slice (04), but the journal stays the single durable
  SSOT and the budget is recomputed-from-inputs against the live policy.
- Option B: reconciler-style `RetryMemory` View. Trade-off: reuses a known
  pattern, but a workflow has an engine (the reconciler pattern exists because
  a reconciler does NOT) — a second durable store duplicates the journal's
  role. Rejected.
- Option C: budget in the body. Trade-off: simplest types, but it puts retry
  policy in the author's hands — the exact inversion the research refutes (the
  engine owns retry; the body owns only terminal). Rejected.

---

## [REF] Slice breakdown (ordered DELIVER slices — NOT roadmap.json)

Sequenced for a single-cut landing. The compiler is the forcing function:
deleting `WorkflowResult` and reshaping the `Workflow` trait breaks every
consumer at once, so Slices 01–03 are tightly coupled and land as one PR arc
(each a coherent landable commit, RED→GREEN per the project's TDD discipline);
Slice 04 (the retry loop) is genuinely additive on a stable body contract and
MAY land in a follow-on PR. C4 L2+L3 diagrams are in `brief.md` § "Phase 1
workflow-result-error-model extension".

- **Slice 01 — core types + the typed/erased trait split.** CREATE
  `TerminalError`/`TerminalErrorKind`, `WorkflowStatus`, `ErasedWorkflow` +
  `ErasedWorkflowAdapter<W>`; reshape the `Workflow` trait to
  `{ type Output; type Input; run(ctx, input) -> Result<Output, TerminalError> }`;
  DELETE `WorkflowResult`. Grow `WorkflowStart { name, input }` + the
  `WorkflowStartEnvelope` (V1) + typed codec + the golden-bytes
  schema-evolution fixture (ADR-0048). **Acceptance intent:** the trait + ctx
  module compiles in `overdrive-core` with no tokio; `ErasedWorkflowAdapter`
  round-trips a typed `Output`/`Input` through CBOR; the `WorkflowStartEnvelope`
  V1 golden fixture decodes via `into_latest`; `TerminalError`/`WorkflowStatus`
  newtype-completeness (serde round-trip, bounded `detail`). Pure-core slice,
  default test lane.
- **Slice 02 — engine drives `ErasedWorkflow`; body-return → `WorkflowStatus`
  mapping; journal/obs carry `WorkflowStatus`.** Reshape `WorkflowRegistry`/
  `WorkflowFactory` to `ErasedWorkflow`; `WorkflowEngine::start` calls
  `run_erased(&ctx, &input_bytes)` and maps `Ok(bytes)` → `Completed{output}`,
  `Err(TerminalError)` → `Failed{terminal}`; `JournalCommand::Terminal` +
  `ObservationRow::WorkflowTerminal` + `WorkflowInstanceState.terminal` carry
  `WorkflowStatus`. Migrate the `ProvisionRecord`/`*WithSleep`/`*WithSignalEmit`
  fixtures to `Output=()`, `Input=()`, `Result<(), TerminalError>`. **Acceptance
  intent:** a workflow returning `Ok(())` writes `WorkflowStatus::Completed`;
  one returning `Err(TerminalError::explicit(...))` writes
  `Failed{terminal}` round-tripping byte-equal through the journal `Terminal`
  command and the obs row; the terminal short-circuit re-publishes the typed
  status losslessly; an author-body **panic** (the engine `catch_unwind` path,
  carried forward from ADR-0064) maps to `Failed{TerminalError::explicit(...)}`
  with the deterministic downcast detail (no behavior regression, retargeted
  type). NEW DST invariant `WorkflowTerminalStatusProjection`.
  `replay_equivalence_provision_record` updated to assert `Completed{output}`
  (output round-trips to `()`). Lima integration lane for the engine.
- **Slice 03 — typed `WorkflowStart` crosses Raft; #217 discharged.**
  action-shim `persist_workflow_intents` persists the full spec via
  `spec.archive_for_store()?`; lifecycle reconciler `hydrate_desired` reads
  `WorkflowStart::from_store_bytes(value)?`; `started_digests` derives
  `input_digest = ContentHash::of(&spec.input)`. **Acceptance intent:** two
  instances of the same kind with different `input` persist + rehydrate with
  distinct `input_digest`s; a round-trip `StartWorkflow → intent → hydrate →
  engine` preserves the input; a malformed/undecodable intent refuses
  (intent SSOT asymmetry, ADR-0048). The `TODO(#217)` is removed. Closes #217.
- **Slice 04 (follow-on PR; additive) — retry-re-drive loop + engine-minted
  `BudgetExhausted`.** Add the `JournalCommand::RetryAttempted` additive
  command; the engine classifies transient errors, re-drives from the journal
  up to the engine-constant budget, recomputes attempts from journal entries,
  and mints `TerminalError::budget_exhausted()` → `WorkflowStatus::Failed` on
  exhaustion. **Acceptance intent:** a forced-transient workflow re-drives up
  to the budget then terminates `Failed{BudgetExhausted}` (the body authored
  no failure); NEW DST invariant `WorkflowBudgetExhaustionMintsTerminal`. The
  body contract is unchanged from Slice 01 — this is pure engine growth.

**Single-cut landing note:** Slices 01–03 MUST land together (the compiler
will not accept a half-reshaped trait); they are sliced for review/TDD
coherence, not for independent shipping. Slice 04 is the one genuinely
deferrable unit (the engine's pre-Slice-04 behaviour — "explicit
`Err(TerminalError)` ends the instance" — is correct, just less sophisticated
than the final retry contract). The DELIVER roadmap (`/nw-roadmap`, the next
step) sequences these into ACs; this breakdown is the input to it, NOT a
`roadmap.json`.

---

## Wave: DISTILL

Wave: DISTILL (Quinn / nw-acceptance-designer) · Date: 2026-06-06 · Mode:
REUSE-FIRST triage (not green-field authoring). **No DISCUSS / DEVOPS dirs**
for this feature → graceful degradation: ACs derived from DESIGN (ADR-0065 +
this feature-delta + `brief.md` § "Phase 1 workflow-result-error-model
extension"); default env. DESIGN present → not blocked. Reconciliation HARD
GATE: only the DESIGN wave exists for this feature; 0 cross-wave
contradictions → passed.

**Rust project — no `.feature` files, no pytest, no `__SCAFFOLD__`.** Per
`.claude/rules/testing.md`: acceptance tests are Rust `#[test]` /
`#[tokio::test]` in `crates/{crate}/tests/acceptance/*.rs`; RED scaffolds use
`#[should_panic(expected = "RED scaffold")]` with a self-contained `panic!`
body that imports no unbuilt type. The driving "ports" here are the
`Workflow` trait + `ErasedWorkflow` engine surface + `WorkflowEngine` (all
in-process) + the DST/sim harness — there is NO CLI/HTTP surface for this
feature, so Mandate-1 driving-adapter (subprocess/HTTP) coverage is N/A; the
port-to-port discipline is satisfied at the trait/engine boundary.

**DISTILL deliverable is the PLAN + the NEW scaffolds, NOT the migration.**
The contract change (`WorkflowResult` DELETE; `run` reshape; `Terminal`/obs
`result → status`; `WorkflowStart { name → name, input }`) breaks every
existing consumer at compile time. Migrating each broken test is DELIVER
work (each slice migrates what its production change breaks). The triage
below tells DELIVER exactly what breaks and how each broken assertion maps.

### [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DESIGN#D1 | Object safety via typed author trait + `ErasedWorkflowAdapter<W>` → object-safe `ErasedWorkflow`; engine holds `Box<dyn ErasedWorkflow>` | D1 | NEW-1 scaffold pins typed `Output`/`Input` round-tripping through the CBOR erasure boundary — the genuinely-new author-edge behaviour |
| DESIGN#D2 | `TerminalError { kind, detail }` concrete core type; `BudgetExhausted` engine-minted; `detail` length-capped (closes the `reason: String` replay hazard) | D2 | NEW-4 (MalformedInput) + the panic-test MIGRATE both assert the typed terminal; `reason: String` assertion sites are deleted |
| DESIGN#D3 | Engine-owned `WorkflowStatus { Completed{output} \| Failed{terminal} \| Cancelled \| TimedOut }` carried by journal `Terminal` + `ObservationRow::WorkflowTerminal`, distinct from body return | D3 | 7 terminal-asserting tests MIGRATE `result: WorkflowResult` → `status: WorkflowStatus`; the body-return → status mapping is pinned by NEW-1/NEW-4 + the migrated panic test |
| DESIGN#D4 | Retryable absorbed/re-driven by engine; budget in engine/journal (`RetryAttempted`-derived + engine-constant policy), NOT body, NOT `View`; `BudgetExhausted` engine-minted | D4 | NEW-5 scaffolds (Slice 04, stay RED) pin re-drive-to-budget + engine-minted exhaustion + "retryable never reaches the return type" |
| DESIGN#D5 | Typed `WorkflowStart { name, input: Vec<u8> }` crosses Raft via rkyv `WorkflowStartEnvelope` (V1) + co-located typed codec; `input_digest = ContentHash::of(&spec.input)` — resolves #217 | D5 | NEW-2 scaffold pins the executable #217 acceptance (distinct inputs ⇒ distinct digests); NEW-3 schema-evolution fixture pins the V1 rkyv layout; 3 existing `input_digest` tests MIGRATE their assertion target |
| DESIGN (single-cut) | `WorkflowResult` DELETED greenfield; only the `provision-record` test fixture is a registered consumer | n/a | The keystone fixture (`testing/workflow.rs`) signature migration cascades to all 24 tests; no production workflow instances exist to migrate |

### [REF] Reuse / Migrate / New triage

**Counts (over the 27 enumerated existing tests — the dispatch said "24",
but the enumerated list is 27: core 4 + cp-acceptance 8 + cp-e2e/journal 3 +
sim 12): REUSE 3 · MIGRATE 24.** MIGRATE breakdown: 3 core author-surface
(M-c1..M-c3) + 8 cp engine/projection (M1..M8) + 3 cp e2e/integration
(M9..M11) + 8 sim mechanics (M12..M19) + 1 DST-invariant (M-inv) + 1
MIGRATE-shallow-REUSE-leaning (`sleep_records_deadline`, body unchanged) =
24. (3 REUSE + 24 MIGRATE = 27, all enumerated tests classified.)

**NEW: 5 scaffold files** — 4 active `#[should_panic]` acceptance files (7
`#[test]`s: NEW-1 ×2, NEW-2 ×2, NEW-4 ×1, NEW-5 ×2) + 1 schema-evolution
fixture file (NEW-3: 3 `#[test]`s + 1 `#[ignore]` regenerator, shipped
DELIVER-ready, un-wired). Plus 1 NEW DST invariant
(`WorkflowTerminalStatusProjection`) authored in DELIVER Slice 02 (not a
`#[should_panic]` scaffold) + its Slice-04 sibling
`WorkflowBudgetExhaustionMintsTerminal` (NEW-5's DST counterpart).

The keystone fixture `crates/overdrive-core/src/testing/workflow.rs` (the
three `ProvisionRecord*` workflows) is the cascade root: its `run(&self, ctx)
-> WorkflowResult` → `run(&self, ctx, _input: ()) -> Result<(),
TerminalError>` migration (Output=(), Input=()) breaks every test that
constructs or drives those fixtures. This is MIGRATE-shallow for the
fixture itself (a mechanical signature + body rewrite: each
`WorkflowResult::Success` → `Ok(())`, each `WorkflowResult::Failed { reason }`
→ `Err(TerminalError::explicit(<detail>))`, add `spec.input` via
`ciborium::into_writer(&(), ..)` in each `spec()` helper) and is DELIVER
Slice 02 work (component table row `testing/workflow.rs` MODIFY).

**The canonical assertion mapping (applies to every MIGRATE row below):**

| Old (deleted contract) | New (ADR-0065 contract) | Where |
|---|---|---|
| body returns `WorkflowResult::Success` | body returns `Ok(())` (or `Ok(output)`) | fixture / inline-workflow `run` bodies |
| body returns `WorkflowResult::Failed { reason }` | body returns `Err(TerminalError::explicit(detail))` | fixture / inline-workflow `run` bodies |
| body returns `WorkflowResult::Cancelled` | (gone — `Cancelled` is engine-authored `WorkflowStatus::Cancelled`, never a body return) | n/a (no fixture returns it) |
| terminal row `ObservationRow::WorkflowTerminal { correlation, result }` | `{ correlation, status }` (pattern + binding rename) | obs-row match sites |
| `assert_eq!(result, WorkflowResult::Success)` on a terminal row | `assert!(matches!(status, WorkflowStatus::Completed { .. }))` (output round-trips to `()` for unit fixtures) | terminal-row assertions |
| `matches!(result, WorkflowResult::Failed { .. })` on a terminal row | `matches!(status, WorkflowStatus::Failed { terminal } if terminal.kind() == TerminalErrorKind::Explicit)` | panic / failure terminal assertions |
| `JournalCommand::Terminal { result: WorkflowResult::Success }` | `JournalCommand::Terminal { status: WorkflowStatus::Completed { .. } }` | journal `Terminal` construction/match |
| `WorkflowInstanceState.terminal == Some(WorkflowResult::Success)` | `== Some(WorkflowStatus::Completed { .. })` | lifecycle-reconciler hydrate assertions |
| `obs.workflow_terminal_rows()` → `Vec<(CorrelationKey, WorkflowResult)>` | `Vec<(CorrelationKey, WorkflowStatus)>` (return-type shift; all callers) | every `workflow_terminal_rows()` caller |
| `expected_input_digest = ContentHash::of(ProvisionRecord::PAYLOAD)` | `ContentHash::of(&spec.input)` (CBOR of `()` for the unit fixture — NOT the step payload) | the 3 `input_digest`-pinning tests |
| `JournalCommand::Terminal { .. }` (type-agnostic matcher) | UNCHANGED — survives both contracts verbatim | `terminal_count` / `has_terminal` helpers |

#### REUSE (unchanged — survive the reshape verbatim)

| # | Test | Crate / path | Why REUSE |
|---|---|---|---|
| R1 | `workflow_ctx_run_and_sleep` | overdrive-core/tests/acceptance | Pure `ctx.run` / `ctx.sleep` cursor mechanics + `WorkflowCtxError::NonDeterministic`; never constructs a `Workflow::run`, never names `WorkflowResult`/`WorkflowStart`. `WorkflowCtx` + `WorkflowCtxError` are untouched by ADR-0065. |
| R2 | `workflow_committed_step_survives_crash` | overdrive-sim/tests/acceptance | Journal commit/crash mechanics; zero migration touch-points in the scan. Drives `ctx` directly, asserts journal entries — the result-model reshape does not reach it. |
| R3 | `workflow_journal_write_ordering` | overdrive-sim/tests/acceptance | Journal append/fsync ordering invariant; zero touch-points. |

**REUSE caveat (transitive compile coupling, NOT a content change):** R1-R3
live in test binaries (`tests/acceptance.rs`) that ALSO compile the migrated
modules. The binary won't *link* until the sibling migrations + the keystone
fixture compile under the new contract — but R1-R3's own SOURCE is unchanged.
DELIVER does not edit them; it only needs the binary to compile around them.
`workflow_sleep_records_deadline_not_remaining` (overdrive-sim) is also
effectively REUSE-shaped (drives `ctx.sleep` directly, no `WorkflowResult`),
but is grouped under MIGRATE-shallow M-sim below out of caution — it imports
the sim adapters whose `journal.rs` proptest references `WorkflowResult`
(adapter-side migration), so its binary coupling is tighter; its own body is
unchanged.

#### MIGRATE — author-surface cluster (overdrive-core)

| # | Test | Path | Migration (delta only) |
|---|---|---|---|
| M-c1 | `workflow_trait_drives_to_terminal` | core/tests/acceptance | `ProvisionRecord` (keystone fixture) sig migrates → drive `run(&ctx, ())`; `assert_eq!(result, WorkflowResult::Success)` → `assert!(matches!(result, Ok(())))` (the body returns `Result<(), TerminalError>` now — there is no separate terminal-row here, the author surface returns the typed result directly) |
| M-c2 | `workflow_body_has_no_step_machine` | core/tests/acceptance | This is a SYN-SCAN test (it parses the `impl Workflow for ProvisionRecord` `run` body as source text). The embedded reference-body string (`PROVISION_RECORD_SOURCE`-style const showing `async fn run(&self, ctx) -> WorkflowResult { ... WorkflowResult::Success }`) MIGRATES to the new signature/body so the syn-scan asserts against the post-reshape clean body. No runtime behaviour; the assertion is "the body has no step enum / transition match" — unchanged intent, updated reference text |
| M-c3 | `workflow_body_routes_nondeterminism_through_ctx` | core/tests/acceptance | Same syn-scan shape as M-c2 (parses the `run` body for direct `Instant::now`/`rand` use). The embedded reference-body const MIGRATES to `run(&self, ctx, ()) -> Result<(), TerminalError>` with `Ok(())`; the routes-through-ctx assertion intent is unchanged |

#### MIGRATE — engine / terminal-projection cluster (control-plane)

| # | Test | Path | Migration (delta only) |
|---|---|---|---|
| M1 | `workflow_engine_writes_terminal_row` | cp/tests/acceptance | obs-row `{ result }` → `{ status }`; `WorkflowResult::Success` → `WorkflowStatus::Completed`; **`expected_input_digest`: `ContentHash::of(ProvisionRecord::PAYLOAD)` → `ContentHash::of(&spec.input)`** (the #217 fix lands HERE — substantive, not shallow); `Started { spec_digest, input_digest }` matcher unchanged shape |
| M2 | `workflow_engine_terminal_short_circuit` | cp/tests/acceptance | `CountingSuccess::run` sig + `WorkflowResult::Success` → `Ok(())`; `WorkflowStart { name }` → `{ name, input: cbor(()) }`. `terminal_count` helper (type-agnostic `Terminal { .. }`) UNCHANGED — already authored to survive both contracts |
| M3 | `workflow_panic_converges_to_failed_terminal` | cp/tests/acceptance | **The `reason: String` replay-hazard closure lands here.** `matches!(result, WorkflowResult::Failed { .. })` → `matches!(status, WorkflowStatus::Failed { terminal } if terminal.kind() == TerminalErrorKind::Explicit)` with the deterministic downcast `detail`; obs-row `result → status`; `PanickingWorkflow::run` sig + `WorkflowStart` |
| M4 | `workflow_engine_replay_cursor` | cp/tests/acceptance | `ctx.run` replay-cursor unit mechanics; `result_digest`/`value_digest`/`action_digest` are `ContentHash::of(bytes)` (UNCHANGED). Migrates ONLY if it constructs a `Terminal` command or a `Workflow::run` — scan shows it does not name `WorkflowResult`; MIGRATE-shallow (compile-fixup if it touches the reshaped `JournalCommand::Terminal` field name via a shared helper) |
| M5 | `workflow_engine_live_instance_registration` | cp/tests/acceptance | `BlockingWorkflow::run` sig + `WorkflowResult::Success` → `Ok(())`; `WorkflowStart` |
| M6 | `action_shim_dispatches_start_workflow_to_engine` | cp/tests/acceptance | `JournalCommand::Terminal { result } if *result == WorkflowResult::Success` → `{ status } if matches!(status, WorkflowStatus::Completed { .. })`; `WorkflowStart` |
| M7 | `lifecycle_reconciler_rehydrates_on_restart` | cp/tests/acceptance | `WorkflowInstanceState.terminal: Some(WorkflowResult::Success)` → `Some(WorkflowStatus::Completed { .. })`; `provision_spec()` `WorkflowStart { name }` → `{ name, input }` |
| M8 | `workflow_emit_action_lands_in_raft_channel` | cp/tests/acceptance | `EmittingWorkflow::run` sig + `Ok(()) => WorkflowResult::Success` / `Err(_) => WorkflowResult::Failed { reason }` → `Ok(())` / `Err(TerminalError::explicit(..))`; `WorkflowStart` |

#### MIGRATE — e2e / integration cluster (control-plane, Lima lane)

| # | Test | Path | Migration (delta only) |
|---|---|---|---|
| M9 | `reconciler_emit_drives_workflow_to_terminal` | cp/tests/integration/workflow_e2e | obs-row `{ result }` → `{ status }`; `obs.workflow_terminal_rows()` return-type shift; `WorkflowResult::Success` → `WorkflowStatus::Completed` (2 sites: terminal row + `actual_instance.terminal`); `FixtureTriggerReconciler { spec }` carries reshaped `WorkflowStart`; correlation derivation off `spec.name` UNCHANGED |
| M10 | `workflow_emit_action_drives_through_production_composition` | cp/tests/integration/workflow_e2e | `EmittingWorkflow::run` sig + 3 `WorkflowResult::{Success,Failed}` sites; `obs.workflow_terminal_rows()` + helper return type → `WorkflowStatus`; `WorkflowStart` (both provision + emitting specs) |
| M11 | `journal_writes_to_redb` | cp/tests/integration/workflow_journal | **`input_digest: ContentHash::of(PROVISION_RECORD_PAYLOAD)` → `ContentHash::of(&spec.input)`** (#217); `result_digest` unchanged; `Terminal` field rename if constructed |

#### MIGRATE — sim crash / resume / sleep / signal / journal cluster (overdrive-sim, MIGRATE-shallow)

These test JOURNAL + CTX mechanics that the result-model reshape leaves
structurally unchanged. The delta is (a) the keystone-fixture signature
cascade (they construct `ProvisionRecord*`), (b) terminal-row `result →
status` + `WorkflowResult::Success` → `WorkflowStatus::Completed` at the
final assertion, (c) `obs.workflow_terminal_rows()` return-type shift. The
crash/resume/exactly-once/deadline SCENARIOS are reused verbatim; only the
assertion/fixture lines change. Confirms the user's hypothesis.

| # | Test | Migration (delta only) |
|---|---|---|
| M12 | `workflow_crash_resume_exactly_once` (WALKING SKELETON) | terminal-result helper return `WorkflowResult` → `WorkflowStatus`; obs-row `{ result }` → `{ status }`; final `assert_eq!(resumed_result, WorkflowResult::Success)` → `matches!(.., WorkflowStatus::Completed { .. })`; `WorkflowStart`. Exactly-once / replay-byte-equality core UNCHANGED |
| M13 | `workflow_sleep_resumes_to_original_deadline` | `run_body() -> WorkflowResult` → `Result<(), TerminalError>` (returns `Ok(())`); `assert_eq!(terminal, WorkflowResult::Success)` → `Completed`; obs-row rename. Deadline-recompute core UNCHANGED |
| M14 | `workflow_sleep_crash_pre_sleep_step_not_repeated` | obs-row `{ result }` → `{ status }`; final `assert_eq!(result, WorkflowResult::Success)` → `Completed`; `WorkflowStart`. Pre-sleep-step-not-repeated core UNCHANGED |
| M15 | `workflow_signal_wait_reblocks_after_crash` | terminal helper return `WorkflowResult` → `WorkflowStatus`; obs-row rename; `Some(WorkflowResult::Success)` → `Some(WorkflowStatus::Completed { .. })`; `obs.workflow_terminal_rows()` shift. Re-block-on-absent-signal core UNCHANGED |
| M16 | `workflow_signal_already_seen_not_rewaited` | same shape as M15 (terminal helper + obs-row + `Some(Completed)`); already-seen-not-rewaited core UNCHANGED |
| M17 | `workflow_emit_action_not_re_emitted_after_crash` | keystone-fixture cascade; `Terminal { .. }` type-agnostic matcher UNCHANGED; obs-row rename if it reads the terminal row. Not-re-emitted core UNCHANGED |
| M18 | `workflow_emit_action_at_least_once_on_failed_record` | keystone-fixture cascade (`ProvisionRecordWithSignalEmit`); `value_digest` unchanged. At-least-once-on-failed-record core UNCHANGED |
| M19 | `journal_records_inputs_not_derived` | **`input_digest: ContentHash::of(ProvisionRecord::PAYLOAD)` → `ContentHash::of(&spec.input)`** (#217, 2 sites); `Terminal { result: WorkflowResult::Success }` → `{ status: WorkflowStatus::Completed { .. } }`; keystone cascade. Inputs-not-derived core UNCHANGED |

**MIGRATE-shallow (REUSE-leaning):** `workflow_sleep_records_deadline_not_remaining`
(overdrive-sim) — body unchanged (drives `ctx.sleep` directly, no
`WorkflowResult`); only its binary's transitive compile of the migrated
sim-adapter proptest couples it. Counted within M-sim as a no-body-change
compile-fixup.

**Production-side MIGRATE (NOT tests, but the contract-change sites DELIVER
must touch — listed so the count is honest):** `testing/workflow.rs` (the 3
fixtures), `workflow_runtime/mod.rs:438` (panic→`Failed` mapping →
`TerminalError::explicit`), `journal/mod.rs:326,662` (`Terminal { result }` →
`{ status }` + the CBOR-roundtrip test), `traits/observation_store.rs:561`
(`WorkflowTerminal { result }` → `{ status }` + `workflow_terminal_rows`
return type), `reconcilers/workflow_lifecycle.rs:66`
(`terminal: Option<WorkflowStatus>`), `sim/adapters/journal.rs:198,248-250`
(the `Terminal`/`WorkflowResult` proptest generator), `sim/invariants/
evaluators.rs` (8 `terminal(WorkflowResult::Success)` sites in the
replay-equivalence evaluator + the NEW `WorkflowTerminalStatusProjection`
evaluator). These are component-table rows already in the DESIGN section;
the triage surfaces them as the migration's blast radius.

#### MIGRATE — DST invariant (overdrive-sim)

| # | Test | Path | Migration (delta only) |
|---|---|---|---|
| M-inv | `replay_equivalence_provision_record_invariant` | sim/tests/acceptance | **MIGRATE-shallow at the test surface, substantive in the evaluator.** The test asserts `InvariantStatus::Pass` + seed-reproducibility via the DST harness — it does NOT name `WorkflowResult`, so the test SOURCE is largely unchanged. The underlying `replay_equivalence_provision_record` EVALUATOR (`evaluators.rs`, 8 `terminal(WorkflowResult::Success)` sites) MIGRATES its terminal assertion from "terminal is `Success`" to "the `WorkflowStatus` projection is `Completed { output }` and the erased output round-trips to `()`" (ADR-0065 § "DST invariants", carried-forward invariant). The NEW `WorkflowTerminalStatusProjection` invariant (below) is authored alongside |

### [REF] NEW scenarios + RED scaffolds (authored this wave)

All authored as Rust `#[should_panic(expected = "RED scaffold")]` per
`.claude/rules/testing.md`, EXCEPT NEW-3 (schema-evolution fixture — see the
decision below). Each scaffold imports NO unbuilt production type and is
green-at-the-bar (nextest PASS / clippy clean / no `--no-verify`).

| ID | Scenario | File (authored) | Slice | Tags | Layer / mode |
|---|---|---|---|---|---|
| NEW-1 | Non-unit typed `Output` round-trips through `ErasedWorkflowAdapter` CBOR erasure (+ NEW-1b: typed `Input` decode side) | `crates/overdrive-core/tests/acceptance/workflow_typed_output_roundtrip.rs` | 01 | `@in-memory @property @D1` | Layer 1; PBT-full in DELIVER (output roundtrip is a universal property) |
| NEW-2 | `input_digest` divergence — two distinct inputs of one kind ⇒ distinct digests (+ NEW-2b: same input ⇒ stable digest + persist→rehydrate fidelity) | `crates/overdrive-control-plane/tests/acceptance/workflow_input_digest_divergence.rs` | 03 | `@in-memory @property @D5 @issue-217` | Layer 1-2; the executable #217 acceptance |
| NEW-3 | `WorkflowStartEnvelope` V1 golden-bytes + discriminant triangulation + unknown-version probe | `crates/overdrive-core/tests/schema_evolution/workflow_start.rs` | 01 | `@property @D5 @issue-217 @error` | Layer 1 rkyv; mandatory golden-bytes obligation (ADR-0048) |
| NEW-4 | Undecodable start input ⇒ engine-minted `WorkflowStatus::Failed { MalformedInput }`, body never entered | `crates/overdrive-control-plane/tests/acceptance/workflow_malformed_input_terminal.rs` | 03 | `@in-memory @error @D2 @D3` | Layer 1-2; example-based sad path (Mandate 11) |
| NEW-5 | Transient re-driven to budget ⇒ engine-minted `BudgetExhausted` (+ NEW-5b: transient clearing within budget ⇒ `Completed`, "retryable never reaches the return type") | `crates/overdrive-control-plane/tests/acceptance/workflow_budget_exhaustion_mints_terminal.rs` | **04** | `@in-memory @error @D4 @slice-04` | Layer 1-2; **STAYS RED until Slice 04** (DELIVER 01-03 do NOT activate) |

**NEW DST invariant (authored in DELIVER Slice 02, not a `#[should_panic]`
scaffold):** `WorkflowTerminalStatusProjection` (ADR-0065 § "DST invariants")
— drives a workflow returning `Err(TerminalError::explicit(..))` and asserts
the engine writes `WorkflowStatus::Failed { terminal }` (NOT a contentless
variant) with the `TerminalError` round-tripping byte-equal through the
journal `Terminal` command + the observation row. Pins the body-return →
status-projection mapping (D3) as a structural property. Lives in
`sim/invariants/{mod.rs,evaluators.rs}` (a named enum variant per the house
no-inline-literal convention). DST invariants are not test-binary
`#[should_panic]` scaffolds — they are authored as the evaluator + a named
variant; DELIVER Slice 02 lands the variant returning a `todo!("RED
scaffold: ...")` body if needed to keep the harness exhaustive-match intact
(`.claude/rules/testing.md` § "Production-side scaffolds"). Its companion
Slice-04 invariant `WorkflowBudgetExhaustionMintsTerminal` is NEW-5's DST
sibling.

### [REF] Schema-evolution fixture decision (NEW-3)

**Decision: author the fixture FILE now (DELIVER-ready), wire it in DELIVER
Slice 01.** The `WorkflowStartEnvelope` golden-bytes fixture is MANDATORY
(ADR-0048 § 1 — every rkyv envelope ships a per-version golden fixture). But
unlike the four self-contained `#[should_panic]` acceptance scaffolds, a
schema-evolution fixture CANNOT be green-at-the-bar: the harness
(`assert_envelope_v_roundtrip::<WorkflowStartEnvelope>`) needs the REAL
envelope type, which lands in DELIVER Slice 01. Wiring `mod workflow_start;`
into `tests/schema_evolution.rs` now would fail the WHOLE `overdrive-core`
schema-evolution test binary compile (breaking every OTHER fixture's ability
to run) against a not-yet-existing type.

Resolution (the honest Rust analogue of a skip marker for a fixture that
cannot compile standalone): the fixture file ships complete — canonical V1
payload, the three harness assertions (golden-bytes roundtrip, discriminant
triangulation, unknown-version probe), and the `print_fixture_v1_bytes`
regeneration aid — with `FIXTURE_V1 = "__RED_SCAFFOLD__..."` and
`GOLDEN_DISCRIMINANT_OFFSET_V1 = 0` placeholders. Its `mod workflow_start;`
line in `tests/schema_evolution.rs` is COMMENTED OUT with an explicit
DELIVER-Slice-01 activation marker (3 steps: mint `FIXTURE_V1` via the aid,
pin the offset, uncomment). This matches how `root_ca_key.rs` /
`issued_certificate_row.rs` were wired only AFTER their `src/ca/` envelope
types existed — here the type does not exist yet, so the wiring waits one
slice. The contract (the V1 payload shape, the `input`-bearing aggregate, the
"rkyv wraps the outer `WorkflowStart` only, inner `input` stays opaque CBOR"
rule) is fully specified by the shipped file.

### [REF] trybuild non-substitutability — RECOMMEND, do not author

ADR-0065 § Consequences (Negative) warns three terminal-modelling types must
not be conflated: `TerminalError` (body failure channel), `WorkflowStatus`
(engine projection), `TerminalCondition` (reconciler claim, ADR-0037). The
project HAS a trybuild precedent (`tests/compile_fail/intent_vs_observation.rs`,
`reconciler_trait_is_not_dyn_compatible.rs`) for exactly this
"prove-non-substitutability-at-compile-time" shape.

**Recommendation: DELIVER Slice 01 SHOULD add a `tests/compile_fail/
workflow_status_vs_terminal_condition.rs` case** proving a function typed
`fn(WorkflowStatus)` rejects a `TerminalCondition` (and vice versa), so a
future refactor cannot blur the workflow projection into the reconciler
claim. **NOT authored in DISTILL** because a trybuild case's load-bearing
artifact is its `.stderr` fixture, which can only be generated against the
REAL compiler output once both types exist (DELIVER) — a RED-scaffold
`.stderr` would not match and cannot be green-at-the-bar. This is the
"recommend, don't over-author" call. The reciprocal `WorkflowStatus` vs
`TerminalError` separation is weaker-value (they already differ structurally
— one is an enum, one a struct) and is left to the crafter's judgement.

### [REF] Test placement + pre-requisites

- **NEW-1, NEW-3** → `crates/overdrive-core/tests/` (the author/core surface:
  the erasure adapter + the `WorkflowStartEnvelope` are core types).
  Precedent: `ca_cert_spec_policy.rs` (most recent core DISTILL RED-scaffold)
  and `root_ca_key.rs` / `alloc_status_row.rs` (schema-evolution harness).
- **NEW-2, NEW-4, NEW-5** → `crates/overdrive-control-plane/tests/acceptance/`
  (the engine-side surface: `started_digests`, the adapter decode path, the
  retry loop). Precedent: the existing 8 control-plane `workflow_*`
  acceptance tests + the `should_panic.*RED scaffold` precedent already in
  `tests/acceptance.rs`.
- **Wiring:** NEW-1 added to `overdrive-core/tests/acceptance.rs` mod block;
  NEW-2/4/5 added to `overdrive-control-plane/tests/acceptance.rs` mod block;
  NEW-3's mod line COMMENTED in `overdrive-core/tests/schema_evolution.rs`
  (DELIVER Slice 01 uncomments).
- **Pre-requisites (DESIGN driving ports the scaffolds depend on):** the
  `Workflow` typed trait + `ErasedWorkflow`/`ErasedWorkflowAdapter<W>` +
  `TerminalError`/`TerminalErrorKind` + `WorkflowStatus` +
  `WorkflowStart { name, input }` + `WorkflowStartEnvelope` (Slice 01); the
  engine `run_erased` drive + status mapping + `started_digests` off
  `spec.input` (Slices 02-03); the retry-re-drive loop +
  `JournalCommand::RetryAttempted` + `TerminalError::budget_exhausted()`
  (Slice 04, NEW-5 only). No CLI/HTTP/hook driving adapter (N/A for this
  feature). No DEVOPS env matrix (default; pure-core + Lima engine lanes).
- **Test lanes:** NEW-1/NEW-3 default lane (pure-core, no I/O). NEW-2/4/5
  default lane (Sim* in-process engine); if any DELIVER body grows to real
  redb, it moves under `integration-tests` per `.claude/rules/testing.md`.

### [REF] Surprises vs the dispatch hypothesis (corrections)

1. **`input_digest` migration is SUBSTANTIVE, not shallow (correction).** The
   hypothesis treated the sim journal tests as uniformly MIGRATE-shallow. But
   3 tests (M1 `workflow_engine_writes_terminal_row`, M11 `journal_writes_to_redb`,
   M19 `journal_records_inputs_not_derived`) currently assert
   `input_digest = ContentHash::of(ProvisionRecord::PAYLOAD)` — the transport
   STEP payload `b"provision-record"`, which is NEITHER the old `spec.name`
   digest NOR the new `spec.input` digest; it is a coincidental third value.
   Per D5 these MIGRATE to `ContentHash::of(&spec.input)` (CBOR of `()` for
   the unit fixture). This is the #217 fix landing in those tests — a
   substantive assertion-target change, and the reason NEW-2 exists to pin
   the divergence property the migrated tests alone would not assert.
2. **3 genuinely-REUSE tests exist (refinement).** The hypothesis said "most
   sim tests are MIGRATE-shallow." Three (`workflow_ctx_run_and_sleep`,
   `workflow_committed_step_survives_crash`, `workflow_journal_write_ordering`)
   have ZERO migration touch-points — they drive `ctx`/journal directly and
   never name the result model. They are REUSE (source unchanged), coupled to
   the migration only by test-binary linkage.
3. **`workflow_engine_terminal_short_circuit` is ALREADY contract-agnostic on
   the journal matcher (confirmation + nuance).** Its `terminal_count` helper
   was deliberately authored with a payload-type-agnostic `Terminal { .. }`
   matcher ("compiles against BOTH the current String-based Terminal (RED)
   and the post-fix WorkflowResult-based one") — so the helper survives the
   `result → status` rename verbatim. Only its `CountingSuccess::run` body +
   `WorkflowStart` migrate. A small, pleasant confirmation that the codebase
   already anticipated terminal-payload churn.
4. **The `replay_equivalence` invariant test is MIGRATE-shallow at the test
   surface (refinement).** The hypothesis implied the invariant migrates. The
   TEST asserts `InvariantStatus::Pass` via the harness and never names
   `WorkflowResult` — so the test source barely changes. The substantive
   migration is in the EVALUATOR (`evaluators.rs`, 8 `terminal(WorkflowResult
   ::Success)` sites), which is production-side sim code, not the acceptance
   test. The DST-invariant migration is real but lives outside the 24 test
   files.
5. **`Cancelled` has no body-return migration (confirmation).** No existing
   fixture returns `WorkflowResult::Cancelled` (only the sim journal proptest
   GENERATES it as a value). Post-reshape `Cancelled` is an engine-authored
   `WorkflowStatus` forward variant the body cannot return — so there is
   nothing to migrate on the body side; the proptest generator migrates to
   generate `WorkflowStatus::Cancelled` (production-side sim).

### [REF] Definition of Done (DISTILL → DELIVER gate)

- [x] Every existing workflow test classified (REUSE 3 / MIGRATE 19 / NEW 6+1).
- [x] Per-MIGRATE-row assertion mapping given (the canonical mapping table + per-row deltas).
- [x] NEW scenarios authored as green-at-the-bar `#[should_panic]` scaffolds (NEW-1/2/4/5) importing no unbuilt type.
- [x] Schema-evolution fixture (NEW-3) shipped DELIVER-ready, un-wired with explicit activation marker (decision recorded).
- [x] Slice-04-only scaffolds (NEW-5) flagged to STAY RED until Slice 04.
- [x] trybuild non-substitutability RECOMMENDED (not over-authored), with rationale.
- [x] No migration of existing tests executed (DELIVER work — the contract change breaks them at compile time).
- [x] No compile/test gate run (scaffolds compile standalone; full build awaits DELIVER); no GitHub issues created; no reviewers dispatched (orchestrator owns the review gate).
- [x] Wave-Decision Reconciliation HARD GATE passed (0 contradictions — only DESIGN exists for this feature).
- [x] Business/domain language at the trait+engine boundary; port-to-port via `Workflow`/`ErasedWorkflow`/`WorkflowEngine` (no internal-component entry).
