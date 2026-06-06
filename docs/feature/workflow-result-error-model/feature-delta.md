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
  (additive command, ADR-0063 §2) recomputed against the live policy. The
  full re-drive loop is Slice 04 (types + success/explicit-terminal paths
  land Slices 01–03; body contract stable from Slice 01).
- **[D5] Typed `WorkflowSpec.input` crossing Raft with rkyv-envelope
  discipline; resolves #217.** VERDICT: ACCEPTED.
  `WorkflowSpec { name: WorkflowName, input: Vec<u8> }` (opaque CBOR
  `W::Input`); the durable desired-intent persists the FULL spec via a
  `WorkflowSpecEnvelope` (V1) + co-located typed codec
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
| `WorkflowSpec` (+ `input`, envelope, typed codec) | `crates/overdrive-core/src/workflow/mod.rs` | MODIFY |
| `WorkflowSpecEnvelope` (V1) + schema-evolution fixture | `crates/overdrive-core/src/workflow/mod.rs` + `crates/overdrive-core/tests/schema_evolution/workflow_spec.rs` | CREATE |
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

- **Driving (into the hexagon):** `Action::StartWorkflow { spec: WorkflowSpec, correlation }`
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
| Body output/input erasure codec | `ciborium` (CBOR) | Already the journal step-result + `ctx.run<T>` codec (ADR-0063 §2); homogeneous durable surface; runtime-memory class | Apache-2.0/MIT (workspace dep) |
| Durable `WorkflowSpec` intent codec | `rkyv` versioned envelope (ADR-0048) | Intent-class durable aggregate read across restarts — the `Job` precedent, NOT CBOR (the two codecs stay separate per `development.md`) | MIT (workspace dep) |
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
| D5 | Typed `WorkflowSpec.input` + rkyv envelope; #217 | Accepted | 0065 §5 |
| D6 | New ADR (0065) amending 0064, not in-place edit | Accepted | 0065 § Alt A |

## [REF] Reuse Analysis (mandatory hard gate)

| Existing component | File | Overlap | Decision | Justification |
|---|---|---|---|---|
| `ctx.run<T>` CBOR typed-edge/erased-interior | `overdrive-core/src/workflow/mod.rs:714` | The exact "type at the author edge, CBOR at the journal boundary" pattern D1 needs | **REUSE (pattern)** | D1's `ErasedWorkflowAdapter` is the same erasure `ctx.run<T>` already performs for step results — not a new mechanism. |
| `WorkflowSpec` placeholder (`name` only) | `overdrive-core/src/workflow/mod.rs:1160` | The spec the trigger carries | **EXTEND** | Add `input: Vec<u8>` + envelope/codec; already core because `Action` is core. |
| `Action::StartWorkflow { spec, correlation }` | `overdrive-core/src/reconcilers/mod.rs:380` | The reconciler→engine trigger | **REUSE (unchanged variant)** | `spec` reshapes; the variant shape is exactly D-INH-3. No new Action. |
| `JournalCommand::Terminal { result }` | `overdrive-control-plane/src/journal/mod.rs:326` | The durable terminal | **EXTEND** (`result: WorkflowResult` → `status: WorkflowStatus`) | The lossless-terminal short-circuit already depends on this carrying the full terminal; swap the carried type. |
| `JournalCommand` (additive variants) | `overdrive-control-plane/src/journal/mod.rs:236` | Retry bookkeeping (D4) | **EXTEND** | `RetryAttempted` is an additive `#[serde(default)]` command per ADR-0063 §2 — the established additive-variant pattern. |
| `ObservationRow::WorkflowTerminal { result }` | `overdrive-core/src/traits/observation_store.rs:561` | The observable terminal | **EXTEND** (`result` → `status`) | Same plumbing; swap the carried type to the projection. |
| `WorkflowInstanceState.terminal: Option<WorkflowResult>` | `overdrive-core/src/reconcilers/workflow_lifecycle.rs:66` | Reconciler convergence signal | **EXTEND** (`Option<WorkflowStatus>`) | The `terminal.is_some()` convergence check is unchanged; only the carried type. |
| `WorkflowEngine::started_digests` `TODO(#217)` | `overdrive-control-plane/src/workflow_runtime/mod.rs:542` | `input_digest` derivation | **MODIFY** (discharge #217) | `input_digest = ContentHash::of(&spec.input)`; the marker exists precisely for this. |
| `persist_workflow_intents` (persists `spec.name` bytes) | `overdrive-control-plane/src/action_shim/mod.rs:466` | Durable desired-intent write | **MODIFY** | Persist `spec.archive_for_store()?` (full spec) — the #217 value-side fix. |
| `hydrate_workflow_desired_instances` (parses name bytes) | `overdrive-control-plane/src/reconciler_runtime.rs:2110` | Rehydrate spec from intent | **MODIFY** | Read `WorkflowSpec::from_store_bytes(value)?` — the #217 read-side fix. |
| `Job::archive_for_store`/`from_store_bytes` + envelope | `overdrive-core` (ADR-0048 §4b) | The rkyv typed-codec precedent | **REUSE (pattern)** | `WorkflowSpec`'s codec is the same shape — co-located typed codec on the value, byte-level store unchanged. |
| `RETRY_BACKOFFS` / `RetryMemory` reconciler precedent | `overdrive-control-plane/src/worker/exit_observer.rs`; `development.md` Reconciler I/O | Retry-budget shape | **CONTRAST, do NOT reuse** | Reconciler has no engine → View-memory; workflow HAS an engine → journal-derived budget (D4). Deliberately the opposite home. |
| `WorkflowResult` enum | `overdrive-core/src/workflow/mod.rs:84` | The old body return | **DELETE** | Greenfield single-cut; replaced by `Result<Output, TerminalError>` (body) + `WorkflowStatus` (projection). |

No component is created where an existing one can be extended; every CREATE
(`ErasedWorkflow`, `TerminalError`, `WorkflowStatus`, `WorkflowSpecEnvelope`)
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

- **Option A (RECOMMENDED): `WorkflowSpec { name, input: Vec<u8> }`, durable
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
  DELETE `WorkflowResult`. Grow `WorkflowSpec { name, input }` + the
  `WorkflowSpecEnvelope` (V1) + typed codec + the golden-bytes
  schema-evolution fixture (ADR-0048). **Acceptance intent:** the trait + ctx
  module compiles in `overdrive-core` with no tokio; `ErasedWorkflowAdapter`
  round-trips a typed `Output`/`Input` through CBOR; the `WorkflowSpecEnvelope`
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
- **Slice 03 — typed `WorkflowSpec` crosses Raft; #217 discharged.**
  action-shim `persist_workflow_intents` persists the full spec via
  `spec.archive_for_store()?`; lifecycle reconciler `hydrate_desired` reads
  `WorkflowSpec::from_store_bytes(value)?`; `started_digests` derives
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
