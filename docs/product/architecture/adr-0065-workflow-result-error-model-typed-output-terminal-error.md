# ADR-0065 — Workflow body returns `Result<T, TerminalError>`; status enum becomes an engine-owned control-plane projection; typed `WorkflowStart` input crosses Raft with rkyv-envelope discipline

## Status

Accepted. 2026-06-06 (proposed); accepted and implemented across Slices 01–04.
**Amended 2026-06-07** — ctx-op error surface reconciled with the final
implemented contract ("Model Z"), THEN extended to full Restate Rust SDK
`ContextSideEffects::run` parity: Gap 1 (the `ctx.run` step-closure error
becomes a `retryable | terminal` union, `StepError`, with `TerminalError`
flipped to `!std::error::Error`) and Gap 2 (a per-`ctx.run` `RunRetryPolicy`).
See § "Amendment (2026-06-07) — ctx-op error surface (Model Z) + Restate-parity
step-error union and per-run retry policy" and the corrected § 2 / § 4. The
Gap-1 step-error and Gap-2 per-run-policy surfaces are accepted-but-not-yet-
implemented (the crafter lands them to this ratified contract). Decision-makers:
Morgan (proposing). Tags: phase-1, workflow-primitive, application-arch,
durable-execution, result-error-model, restate-parity, dst.

**Amends** ADR-0064 §2 (`WorkflowResult` as body return), §3 (terminal
record + determinism inputs), §5 (composition path's terminal write +
`input_digest`), and §6 (DST invariants). ADR-0064 §1 (crate placement),
the §3 cursor/replay partition (CA-5), the §3 determinism gate Layers 1+2
(CA-6), the §4 `ctx` await-surface mechanics, and the §5 reconciler
purity boundary are **unchanged and carried forward verbatim**.

**Companion**: ADR-0063 (the redb journal — `Terminal` command + `Started`
digests are touched here). **Composes with**: ADR-0035 (`Reconciler`
retry-budget-in-`View` precedent the engine's retry-location contrasts
with), ADR-0037 (`TerminalCondition` — the control-plane terminal-status
enum's SemVer-convention sibling), ADR-0048 (rkyv versioned-envelope
discipline `WorkflowStart` input now requires), ADR-0003 (core-has-no-
tokio).

**Resolves** [#217](https://github.com/overdrive-sh/overdrive/issues/217)
(`input_digest` must hash the start-input bytes, not the workflow
name) by giving `WorkflowStart` a typed input surface and routing it
durably through Raft. **Unblocks** [#40](https://github.com/overdrive-sh/overdrive/issues/40)
(cert-rotation as the first internal workflow — the validating consumer
of typed input + typed output).

## Context

The accepted ADR-0064 §2 fixed the workflow body's return type as

```rust
async fn run(&self, ctx: &WorkflowCtx) -> WorkflowResult
enum WorkflowResult { Success, Failed { reason: String }, Cancelled }
```

A High-confidence research investigation across **four** durable-execution
platforms — Restate, Temporal, DBOS, AWS Step Functions
(`docs/research/workflow-durable-execution/result-error-retry-semantics-research.md`)
— concludes this is the wrong shape for a body return, on three axes that
all four platforms agree on:

1. **Success is a typed value `T`, never a contentless `Success` variant.**
   Restate `Result<U, HandlerError>` where `U` is the real output;
   Temporal returns its typed result; Step Functions passes data output
   downstream. `WorkflowResult::Success` discards the workflow's actual
   output — no surveyed platform does this.
2. **Retryable errors never reach the return type.** They are absorbed and
   re-driven by the engine; only a *terminal* error ends the workflow. A
   body-returned `Failed { reason }` for an ordinary failure collapses the
   engine's most important distinction (retryable vs terminal) into one
   variant the body authors.
3. **Cancellation is a control-plane operation delivered *into* the body,
   not a variant the body returns.** In Restate cancellation is an external
   API/CLI operation surfaced as a `TerminalError` thrown at the next await
   point; in Temporal it is a control-plane terminal status the body
   *handles*. A body-authored `Cancelled` inverts the ownership.

The research's load-bearing structural finding: there IS a real status
enum in these systems, but it lives at the **control-plane / observable-
history layer** (Temporal's 6 statuses, Step Functions' 4) — never in the
body's signature. Several statuses (`Terminated`, `TimedOut`,
`ContinuedAsNew`) *cannot* be produced by the body at all; they arise from
engine events. **The error to avoid is using one enum for both jobs** —
which is exactly what `WorkflowResult` does today.

Two project-specific facts sharpen this:

- **The `reason: String` is itself a replay-determinism hazard.** ADR-0064
  §3 / ADR-0063 require bit-identical replay; the panic-containment path
  in the engine already had to derive a body-`Failed`'s `reason` from a
  *deterministic* downcast (never the address-bearing panic box) to keep
  the `Terminal` command's bytes stable. A free-text body-authored `reason`
  is a standing invitation to embed a non-deterministic value into the
  durable terminal — the structural fix is to take terminal-failure
  authoring away from the body and give it a typed, bounded shape.
- **#217 is open and gated on the spec input.** `WorkflowStart` today
  carries only `name: WorkflowName` (identity). The engine's
  `started_digests` currently hashes `spec.name` bytes for BOTH
  `spec_digest` AND `input_digest` (a `TODO(#217)` marks the gap); the
  action-shim persists `spec.name.as_str().as_bytes()` as the durable
  desired-intent; the lifecycle reconciler rehydrates a `WorkflowStart`
  by parsing those bytes back into a `WorkflowName`. A workflow with
  *parameters* (cert-rotation's `CertSpec`, #40) has nowhere to put them,
  and `input_digest` cannot distinguish two instances of the same kind
  with different inputs. The body-return reshape and the input reshape are
  one coherent change: both are about giving the workflow primitive a
  *typed value surface* at its two boundaries (input in, output out) while
  keeping the engine's `dyn`-dispatched, CBOR-erased interior.

Four design questions follow (the four priorities in scope):

1. **Object safety** — the engine drives `Box<dyn Workflow>`. A typed
   `Result<T, TerminalError>` return collides with `dyn` dispatch.
2. **Typed input on `WorkflowStart`** crossing `Action::StartWorkflow` →
   Raft (durable intent → rkyv-envelope discipline; resolves #217).
3. **Control-plane terminal-status projection** — the engine-owned status
   enum, where it lives, distinct from the body's return type.
4. **Retryable-vs-terminal error model + retry-budget location.**

## Decision

### 1. The body returns `Result<T, TerminalError>`; object safety via author-edge typing + engine-boundary CBOR erasure (D1)

**Author-facing trait — typed, generic over the output:**

```rust
/// A durable-async workflow. The author writes one ordinary `async fn run`
/// returning its typed output `Output` on success, or a `TerminalError` on an
/// unrecoverable failure. Retryable failures never reach this signature —
/// the engine absorbs and re-drives them (D4).
#[async_trait]
pub trait Workflow: Send + Sync {
    /// The workflow's typed success output. CBOR-serializable so the engine
    /// can erase it to the journal `Terminal` command + the terminal
    /// observation row. `()` for a workflow whose terminal carries no payload.
    type Output: serde::Serialize + serde::de::DeserializeOwned + Send + Sync;

    /// The workflow's typed start input. CBOR-serializable so it
    /// crosses `Action::StartWorkflow` → Raft and seeds `input_digest` (#217).
    /// `()` for a parameterless workflow.
    type Input: serde::Serialize + serde::de::DeserializeOwned + Send + Sync;

    async fn run(&self, ctx: &WorkflowCtx, input: Self::Input)
        -> Result<Self::Output, TerminalError>;
}
```

A trait with an associated `type Output` / `type Input` is **not object-safe**
as written (the method mentions associated types). The engine never names
`dyn Workflow` directly. Instead, an **erasing adapter** at the
registration edge bridges the typed author trait to a `dyn`-safe
engine-facing trait whose method speaks **CBOR bytes only** — mirroring
exactly how `ctx.run<T>` already erases step results to CBOR at the journal
boundary:

```rust
/// The object-safe surface the engine drives. The author's typed `Output` /
/// `Input` are erased to CBOR here; `T` is typed only at the author edge
/// (the `ErasedWorkflow<W>` adapter). The engine holds `Box<dyn ErasedWorkflow>`.
#[async_trait]
pub trait ErasedWorkflow: Send + Sync {
    /// Drive the workflow to terminal. `input_bytes` is the CBOR-decoded
    /// start input (the engine decodes nothing — it hands the raw recorded
    /// bytes; the adapter decodes into `Self::Input`). Returns the CBOR-
    /// encoded `Output` on success, or the `TerminalError` on terminal failure.
    async fn run_erased(&self, ctx: &WorkflowCtx, input_bytes: &[u8])
        -> Result<Vec<u8>, TerminalError>;
}

/// Generic adapter: blanket-erases any typed `Workflow` to the engine surface.
pub struct ErasedWorkflowAdapter<W: Workflow>(pub W);

#[async_trait]
impl<W: Workflow> ErasedWorkflow for ErasedWorkflowAdapter<W> {
    async fn run_erased(&self, ctx: &WorkflowCtx, input_bytes: &[u8])
        -> Result<Vec<u8>, TerminalError> {
        let input: W::Input = ciborium::from_reader(input_bytes)
            .map_err(|e| TerminalError::malformed_input(e.to_string()))?;
        let output: W::Output = self.0.run(ctx, input).await?;     // typed body
        let mut bytes = Vec::new();
        ciborium::into_writer(&output, &mut bytes)
            .map_err(|e| TerminalError::output_encode(e.to_string()))?;
        Ok(bytes)                                               // erased output
    }
}
```

The registry then maps `WorkflowName → Box<dyn Fn() -> Box<dyn ErasedWorkflow>>`
(today: `Box<dyn Workflow>`). The composition root registers a typed
workflow via `registry.register::<CertRotation>(name)` and the adapter is
applied internally — the author never writes the erasure. The engine's
`start` path is unchanged in shape: resolve the factory, get a
`Box<dyn ErasedWorkflow>`, call `run_erased(&ctx, &input_bytes)`.

**Why this shape (D1):** it keeps `T` typed where it matters (the author
writes `Result<CertOutput, TerminalError>`, not `Result<Vec<u8>, _>`) and
erased where the engine needs `dyn` dispatch — the *same* typed-edge /
erased-interior split `ctx.run<T>` already uses for step results. The
journal `Terminal` command and the terminal observation row both carry the
erased CBOR `Output` bytes, so the durable surface is homogeneous (no
per-workflow journal-schema explosion). Considered and rejected: a single
associated `type Output` with the engine holding `dyn Any` and downcasting
(loses the compile-time output type at the registry, and `Any` + `Send` +
serde do not compose cleanly); a non-generic `run(&self, ctx) -> Result<Vec<u8>, TerminalError>`
trait the author implements directly (forces every author to hand-write
CBOR encode/decode — exactly the boilerplate the adapter removes).

### 2. `TerminalError` — a concrete core type, `!std::error::Error`, analogous to `WorkflowCtxError` (D2)

```rust
/// The terminal-failure channel of a workflow body. A workflow that returns
/// `Err(TerminalError)` ends with a terminal failure; a workflow that
/// returns `Ok(Output)` succeeds. RETRYABLE failures never construct this — a
/// `ctx.run` step whose closure resolves to `Err(StepError::Retryable { .. })`
/// records a transient the engine ABSORBS and re-drives; that transient is
/// structurally invisible to the body (it never reaches the
/// `Result<Output, TerminalError>` return type; D4 / Amendment 2026-06-07). A
/// `TerminalError` is the explicit "do not retry; fail the workflow" signal,
/// and is PURELY TERMINAL — all four kinds are genuinely terminal.
///
/// `TerminalError` is **`!std::error::Error`** (Restate-parity, Amendment
/// 2026-06-07 Gap 1). It hand-writes `Display` but deliberately does NOT
/// derive `thiserror::Error`. This is mandatory, not stylistic: the step-closure
/// error `StepError` (below) carries a blanket `impl<E: std::error::Error>
/// From<E>` (so any std error `?`-folds to retryable) AND an
/// `impl From<TerminalError>` (so a `?`'d `TerminalError` routes to terminal).
/// Those two `From`s coexist coherently ONLY if `TerminalError` is not itself an
/// `Error` — otherwise the blanket would already cover it and the two impls
/// collide. `TerminalError` is the workflow analogue of Restate's `TerminalError`
/// (which is likewise not a blanket-colliding `Error`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TerminalError {
    /// A bounded, structured reason kind — NOT free-text. The replay-
    /// determinism hazard of a free `String` reason (ADR-0064 §3) is closed
    /// by making the cause a typed, bounded enum; an author-supplied detail
    /// is carried separately and is part of the durable terminal's inputs.
    kind: TerminalErrorKind,
    /// Author-supplied detail. Bounded (length-capped at construction) and
    /// recorded as an INPUT in the journal `Terminal` command — deterministic
    /// because it is author-data, not engine-derived state. Distinct from the
    /// engine-derived `reason` the OLD `WorkflowResult::Failed` carried.
    detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum TerminalErrorKind {
    /// The author explicitly threw a terminal failure (the Restate
    /// `TerminalError` / Temporal `non_retryable` ApplicationFailure shape).
    Explicit,
    /// The engine minted this terminal because the retry budget was
    /// exhausted (D4). Author code never constructs this variant — the
    /// engine does, on exhaustion.
    BudgetExhausted,
    /// The start input could not be CBOR-decoded into `Self::Input`
    /// (the `ErasedWorkflowAdapter` decode failure). A malformed-input
    /// terminal — not retryable (the bytes will not change on re-drive).
    MalformedInput,
    /// The typed `Output` could not be CBOR-encoded (the adapter encode
    /// failure). A programming error in the `Output` type's serde impl.
    OutputEncode,
}
```

`TerminalError` is `serde::Serialize/Deserialize` (it rides in the journal
`Terminal` command and the terminal observation row as an input). It is **not**
`std::error::Error` (Gap 1 above) — it hand-writes `Display` rendering the
structured kind plus the bounded detail. It is the workflow analogue of
`WorkflowCtxError` (the engine-internal await failure) but models a different
thing: `WorkflowCtxError` is an *engine-internal* await failure (journal record
failed, non-deterministic replay, transient-step carrier); `TerminalError` is
the *body's terminal-failure outcome*. **They do not structurally substitute**
— you cannot pass a `WorkflowCtxError` where a `TerminalError` is expected, or
vice versa. The relationship is a one-way *classification*, not a type
substitution: an *infra* `WorkflowCtxError` raised inside a ctx await-op is
**projected** to `TerminalError::Explicit` at the ctx-op boundary (Amendment
2026-06-07, point 4) — a deliberate one-way mapping, not a bidirectional `From`
that would let the two types stand in for each other. The non-substitutability
is unchanged; only the projection direction is newly explicit.

`TerminalError` does, however, gain ONE deliberate inbound conversion under Gap
1: `impl From<TerminalError> for StepError` (→ `StepError::Terminal`), so a
`?`'d `TerminalError` inside a `ctx.run` closure routes to the *terminal* arm
of the step-closure error rather than (under the old retryable-only shape)
silently folding to retryable. This is coherent precisely because `TerminalError`
is `!Error` (it does not also match `StepError`'s blanket `From<E: Error>`). It
is NOT a conversion *into* `TerminalError` and does not weaken the
`WorkflowCtxError`↔`TerminalError` non-substitutability above — `StepError` is
the step-closure error type (Restate's `HandlerError` analogue), a third type
distinct from both.

Construction is via validating constructors (`TerminalError::explicit(detail)`,
`TerminalError::malformed_input(detail)`, …) that length-cap `detail` per
the newtype-completeness discipline; `BudgetExhausted` has an
engine-only constructor (`pub(crate)` from the engine's vantage — concretely,
a `TerminalError::budget_exhausted()` the author cannot reach because budget
is engine-owned, D4).

### 3. The control-plane terminal-status projection — engine-owned, distinct from the body return (D3)

The status enum the research says is legitimate **as a control-plane
projection** becomes a new engine-owned type, the workflow analogue of
ADR-0037's `TerminalCondition` (which it does NOT reuse — same SemVer
convention, different type):

```rust
/// The externally-observable terminal status of a workflow INSTANCE — the
/// engine's projection of the body's `Result<Output, TerminalError>` PLUS the
/// engine-observed events the body cannot author (cancel, timeout). Written
/// to the ObservationStore `workflow_terminal` row keyed by the instance
/// `CorrelationKey`. The workflow-lifecycle reconciler observes it to
/// converge the instance. DISTINCT from the body's return type (the crux
/// of the research finding) and from `TerminalCondition` (ADR-0037).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum WorkflowStatus {
    /// The body returned `Ok(Output)`. Carries the erased CBOR `Output` bytes (the
    /// workflow's real output — replaces the contentless `Success`).
    Completed { output: Vec<u8> },
    /// The body returned `Err(TerminalError)` OR the engine minted a terminal
    /// on budget exhaustion. Carries the `TerminalError` (kind + detail).
    Failed { terminal: TerminalError },
    /// The control plane cancelled the instance (delivered INTO the body as a
    /// terminal at the next await point — D4 forward; the cancel surface is a
    /// later slice). Engine-authored; the body cannot return this.
    Cancelled,
    /// The instance exceeded its wall-clock deadline (engine-observed; forward
    /// — the deadline surface is a later slice). The body cannot return this.
    TimedOut,
}
```

**Mapping the engine applies** on `run_erased` returning, mirroring Temporal/
Step Functions:

| Engine observes | `WorkflowStatus` written |
|---|---|
| `Ok(output_bytes)` | `Completed { output: output_bytes }` |
| `Err(TerminalError { kind: Explicit \| MalformedInput \| OutputEncode, .. })` | `Failed { terminal }` |
| retry budget exhausted (engine-minted) | `Failed { terminal: BudgetExhausted }` |
| external cancel (forward slice) | `Cancelled` |
| deadline exceeded (forward slice) | `TimedOut` |
| author body **panic** (caught by the engine's `catch_unwind`) | `Failed { terminal: TerminalError::explicit(<deterministic downcast detail>) }` |

The panic-containment path (ADR-0064's engine `catch_unwind` around the
author future) is carried forward and re-targeted: today it maps a panic to
`WorkflowResult::Failed { reason }`; it now maps to
`WorkflowStatus::Failed { terminal: TerminalError::explicit(detail) }` where
`detail` is the SAME deterministic downcast payload (the `&str` / `String`
panic message, never the address-bearing box) the existing path already
derives — so the durable terminal stays byte-stable across runs. The
length-cap in `TerminalError::explicit` bounds an over-long panic message.

**Where it lives.** `WorkflowStatus` is a core type (`overdrive-core::workflow`)
because the `ObservationRow::WorkflowTerminal` variant carries it and
`ObservationRow` is core. The variant's `result: WorkflowResult` field
becomes `status: WorkflowStatus`. The journal `JournalCommand::Terminal`'s
`result: WorkflowResult` field likewise becomes `status: WorkflowStatus`
(the durable terminal the start-time short-circuit re-publishes losslessly
— it must carry `Completed`'s output bytes and `Failed`'s `TerminalError`).
`Cancelled` / `TimedOut` are `#[non_exhaustive]` forward variants the
Phase-1 engine never writes (no cancel/deadline surface yet) — they are
declared now so the projection's shape is honest about what the control
plane will record, and the lifecycle reconciler's match is exhaustive
against them from day one.

**`WorkflowResult` is deleted** (greenfield single-cut — no deprecation, no
bridge). Every site that named it (the engine terminal write + short-circuit,
the journal `Terminal` command, the `ObservationRow::WorkflowTerminal`
variant, the lifecycle reconciler's `WorkflowInstanceState::terminal`, the
panic-containment path) moves to the body's `Result<Output, TerminalError>` at
the body edge and `WorkflowStatus` at the projection edge.

### 4. Retryable-vs-terminal model + retry budget in the engine/journal, NOT the body (D4)

**The error taxonomy the engine applies:**

- **Retryable / transient** (engine absorbs, re-drives internally;
  **structurally invisible to the body**): a `ctx.run` step whose closure
  future resolves to `Err(StepError::Retryable { .. })` (the retryable arm of
  the step-closure error union — Amendment 2026-06-07 Gap 1). The transient is
  NOT folded into the step's own `T` and is NOT a `WorkflowCtxError` the body
  sees — the ctx records a `WorkflowCtxError::TransientStep` and **parks the
  body** (the body's `ctx.run` await never returns on the transient path); the
  `ErasedWorkflowAdapter` detects the recorded transient and surfaces
  `WorkflowDriveError::Transient`, cancelling the parked body. The engine
  re-drives the workflow from the journal (completed steps replay; the failed
  step re-fires) against a bounded budget — **now the FAILING step's own
  `RunRetryPolicy`** (Amendment 2026-06-07 Gap 2), defaulting to
  `WORKFLOW_RETRY_BUDGET` when the step set none. See Amendment (2026-06-07),
  Gap 1 + points 2–3 — this STRENGTHENS the original "step's `Err` re-driven by
  the engine" model: the transient is now structurally off the body's
  `Result<Output, TerminalError>`, not merely conventionally ignored. The old
  `Result<usize, String>`-folding shape is deleted (greenfield single-cut).
- **Terminal** (ends the workflow): an explicit `Err(TerminalError)` the
  body returns, a `ctx.run` step whose closure resolves to
  `Err(StepError::Terminal(t))` (the terminal arm of the step-closure union —
  the step surfaces `t` as `Err(TerminalError)` from `ctx.run(...).await` with
  NO re-drive; Gap 1), an engine-minted `TerminalError { BudgetExhausted }`
  when the retry budget is exhausted, OR an *infra* `WorkflowCtxError` raised
  inside a ctx await-op, projected at the ctx-op boundary to
  `TerminalError::Explicit` (Amendment 2026-06-07, point 4). An infra failure
  inside a ctx op is a terminal outcome for the body — not a transient — so the
  body's `?` observes a `TerminalError`, never a `WorkflowCtxError`.

**Retry budget location — the engine/journal, contrasting the reconciler
`RetryMemory` View precedent.** A reconciler has no engine; ADR-0035 puts
its retry memory in the `View` (`RetryMemory { attempts, last_failure_seen_at }`,
recompute-the-deadline-on-read). A **workflow HAS an engine** — the budget
*mechanism* belongs there, not in the body and not in a reconciler-style View.
Concretely:

- The budget POLICY (max attempts, backoff schedule) is the FAILING STEP's
  `RunRetryPolicy` (Amendment 2026-06-07 Gap 2 — a per-`ctx.run` policy set on
  the `RunStep` builder, defaulting to `RunRetryPolicy::default()` — whose
  fields encode the `50/100/200ms`-clamped schedule and `max_attempts ==
  WORKFLOW_RETRY_BUDGET` directly — when the step sets none). It is **derived from the
  workflow CODE** (the builder call), so it is recomputed each drive from the
  body and **NOT persisted** (per `development.md` § "Persist inputs, not
  derived state"; the policy is a function, the inputs are persisted). The
  engine learns the failing step's policy because it is **carried on the
  transient signal** (`WorkflowCtxError::TransientStep` →
  `WorkflowDriveError::Transient` → `DriveOutcome::Transient`), not read from a
  store. See Amendment 2026-06-07 Gap 2 for the full plumbing.
- The budget INPUTS (attempts-so-far, the retry window's start instant) are
  derived from the **journal** — the count of re-drive entries the engine has
  recorded for the instance, plus the journaled start timestamp — not from a
  separate store. This keeps the journal the single durable SSOT for the
  instance's progress AND its retry state, and keeps the budget
  recomputed-from-inputs (the journal is the input; the attempt count and
  next-retry deadline are recomputed against the live policy on each re-drive).
  A dedicated retry-bookkeeping journal entry (a `RetryAttempted` command,
  additive per ADR-0063 §2) records the attempt input; the engine recomputes
  `attempts` and the backoff deadline from the count of these entries + the
  policy on each re-drive.

**Phase-1 scope of the budget mechanism.** The full retry-re-drive loop
(transient-error classification, backoff parking, `RetryAttempted` recording,
`BudgetExhausted` minting) is its own slice (Slice 04 below). The
result/error-model reshape (Slices 01–03) lands the *types and the success/
explicit-terminal paths* first; the engine's existing behaviour (a body that
returns `Err` ends the instance) maps `Err(TerminalError)` → `Failed`
immediately, with the retry loop layered on top in Slice 04 without changing
the body contract. This is the honest carpaccio: the body contract is stable
from Slice 01; the engine's retry sophistication grows additively.

### 5. Typed `WorkflowStart` input crossing Raft — rkyv-envelope discipline; resolves #217 (D5)

`WorkflowStart` grows from identity-only to identity + typed input:

```rust
/// The concrete workflow spec carried by `Action::StartWorkflow`. Now
/// carries the start INPUT in addition to the kind identity — the
/// `input_digest` hashes it (#217) and the `ErasedWorkflowAdapter`
/// CBOR-decodes into `W::Input`.
pub struct WorkflowStart {
    /// Identity of the workflow kind to start (resolves the factory).
    pub name: WorkflowName,
    /// The CBOR-encoded start input (the erased `W::Input`). Opaque to
    /// the engine; decoded by the adapter into the typed `Input`. An input,
    /// per `development.md` § "Persist inputs, not derived state".
    pub input: Vec<u8>,
}
```

**The durability path (the #217 resolution):**

1. `Action::StartWorkflow { start, correlation }` carries the typed
   `WorkflowStart { name, input }`. `Action` is core, `input` is opaque CBOR.
2. The action-shim's `persist_workflow_intents` persists the **full spec**
   (name + input) as the durable desired-intent under
   `IntentKey::for_workflow_instance(correlation)` — NOT just the name bytes
   (the current `spec.name.as_str().as_bytes()` is the #217 bug). Because the
   persisted desired-intent is a **durable, replayable-across-restart
   value**, it crosses the rkyv-persistence boundary discipline.

   **Decision (D5a): the persisted `WorkflowStart` intent uses the rkyv
   versioned-envelope + typed-codec discipline (ADR-0048).** A `WorkflowStart`
   that now carries arbitrary input bytes is a durable intent aggregate
   read back on every restart; per `development.md` § "rkyv schema evolution"
   and ADR-0048 § 4b (the `Job` typed-codec precedent), it gets a
   `WorkflowStartEnvelope` enum (`V1(WorkflowStartV1)`) and a co-located typed
   codec (`WorkflowStart::archive_for_store` / `WorkflowStart::from_store_bytes`)
   on the typed value, with the byte-level `IntentStore` surface unchanged.
   The action-shim writes `spec.archive_for_store()?` bytes; the lifecycle
   reconciler's `hydrate_desired` reads `WorkflowStart::from_store_bytes(bytes)?`
   — replacing the current `WorkflowName::new(from_utf8(value))` parse. A
   decode failure on intent is load-bearing (intent is SSOT): refuse with a
   structured `health.startup.refused`-class surface, per ADR-0048's intent
   asymmetry.

   *Why rkyv, not CBOR, for the intent value:* the persisted desired-intent
   is **intent-class durable state** read back across restarts — the
   ADR-0048 envelope case, the same class as the `Job` aggregate. (Contrast:
   the journal `input`/`Output`/`Terminal` bytes are runtime-memory CBOR per
   ADR-0063 §2 — they are NOT re-aliased through rkyv. The two codecs stay
   separate per the `development.md` rule; do not conflate.) The
   `input: Vec<u8>` inside `WorkflowStart` is itself opaque CBOR (the erased
   `W::Input`); the rkyv envelope wraps the OUTER `WorkflowStart`, not the
   inner input — exactly the "aggregate envelopes wrap the outer type only"
   rule.
3. `started_digests` (engine) now derives `input_digest = ContentHash::of(&spec.input)`
   — the start-input bytes — and `spec_digest = ContentHash::of(spec.name…)`
   — the kind identity. The two digests **diverge as intended** (the
   `TODO(#217)` is discharged). Two instances of the same kind with different
   inputs get different `input_digest`s; the journal `Started` command
   records both as inputs.

This is the one decision that pulls in a durable-schema discipline:
`WorkflowStart` was identity-only (no envelope needed); with input it
becomes a versioned durable intent aggregate.

## Amendment (2026-06-07) — ctx-op error surface (Model Z) + Restate-parity step-error union and per-run retry policy

This amendment reconciles the ADR's written `ctx`/step error surface with the
final accepted contract ("Model Z"), and then closes the two remaining gaps to
full **Restate Rust SDK `ContextSideEffects::run` parity** (Gap 1 — the
step-closure error becomes a `retryable | terminal` union; Gap 2 — a
per-`ctx.run` retry policy). It supersedes the *original* § 2 / § 4 descriptions
of how a transient flows and what type a `ctx` await-op returns, in place (those
sections' prose and snippets are corrected above). § 1 (adapter / object
safety), § 3 (`WorkflowStatus` projection), and § 5 (`WorkflowStart` / #217) are
**unchanged**. Two intervening `refactor/fix(workflow)!:` commits landed code
ahead of the ADR and are folded in here:

- `40fb7772` (Option A) — the transient signal is a `ctx.run` step concern,
  **not** a `TerminalErrorKind` variant. A `TerminalErrorKind::Retryable` was
  tried and rejected: "retryable never reaches the return type," so a
  *terminal* error that is secretly retryable is a contradiction in terms.
- `6dd50299` (collapse) — there is a **single** `ctx.run` (no
  `ctx.run_retryable`); its closure returns the step-closure error
  (`Result<T, StepError>` post-Gap-1; was `Result<T, RetryableStepError>`),
  not the old "fold the error into the step's own `T`" / `Result<usize, String>`
  shape.

**Reference (Restate Rust, verified against docs.rs).** `restate_sdk`'s
`ContextSideEffects::run` returns `impl RunFuture<Result<T, TerminalError>>`;
its closure returns `HandlerResult<T> = Result<T, HandlerError>`, where
`HandlerError` carries EITHER a `TerminalError` (terminal, no retry) OR any
`std::error::Error` (retryable, retried with backoff). The `RunFuture` builder
exposes `.retry_policy(RunRetryPolicy)` / `.name(..)`; `RunRetryPolicy` is
`{ initial_delay, exponentiation_factor, max_delay, max_attempts, max_duration }`,
and on `max_attempts` exhaustion the run resolves to a `TerminalError`. Model Z
already matched the RETURN type (`ctx.run(...).await -> Result<T, TerminalError>`)
and the std-error-`?`-folds-to-retryable ergonomic. Gap 1 + Gap 2 below close
the two remaining deltas.

### The two core invariants — restated as STILL TRUE (under Model Z AND Gap 1 + Gap 2)

Model Z changes the *types* at the ctx-op boundary, not the *guarantees*; Gap 1
+ Gap 2 below change the step-closure error type and add a per-run policy, but
**both invariants the original ADR established hold unchanged** — they are
restated here as the binding contract for everything that follows:

1. **A retryable/transient failure NEVER reaches the body's
   `Result<Output, TerminalError>` return type.** Model Z *strengthens* this:
   the transient now structurally parks the body (the `ctx.run` await never
   returns on the transient path) and the engine cancels the parked body — the
   body's `?` cannot observe the transient even in principle. Gap 1 keeps this
   intact: the `StepError::Retryable` arm is absorbed by the engine (park +
   re-drive); only the `StepError::Terminal` arm surfaces from `ctx.run` — and
   it is a genuine `TerminalError`, never a disguised retry. Gap 2's per-step
   policy governs only HOW MANY times the engine re-drives, not WHETHER the
   transient reaches the body (it never does).
2. **`TerminalError` is purely terminal.** Its four kinds — `Explicit`,
   `BudgetExhausted`, `MalformedInput`, `OutputEncode` — are all genuinely
   terminal. No retryable variant exists or may be added. Gap 1 puts the
   `retryable | terminal` union on the **step-closure error** (`StepError`),
   NOT on `TerminalError`: `TerminalError` itself gains no variant and stays
   four-kinds-all-terminal.

### The final contract (Model Z + Gap 1 + Gap 2)

1. **`WorkflowCtx` await-ops surface `Result<_, TerminalError>`** (was
   `Result<_, WorkflowCtxError>`), so a workflow body composes them with a
   clean `?` against its own `Result<Output, TerminalError>` return. Post-Gap-1
   the `ctx.run` closure returns the `StepError` union (was `RetryableStepError`);
   post-Gap-2 `ctx.run` returns a `RunStep` builder that `IntoFuture`s to
   `Result<T, TerminalError>` (so `ctx.run(name, fut).await?` and
   `ctx.run(name, fut).retry_policy(p).await?` are both valid):

   ```rust
   /// `T: Serialize + DeserializeOwned + Send`,
   /// `F: Future<Output = Result<T, StepError>> + Send`.   // Gap 1 union
   /// `RunStep<T>: IntoFuture<Output = Result<T, TerminalError>>`,
   ///   with `.retry_policy(RunRetryPolicy) -> Self`.       // Gap 2 builder
   fn run<T, F>(&self, name: &str, f: F) -> RunStep<T, F>;
   async fn sleep(&self, duration: Duration) -> Result<(), TerminalError>;
   async fn wait_for_signal(&self, signal_key: SignalKey)
       -> Result<SignalValue, TerminalError>;
   async fn emit_action(&self, action: Action) -> Result<(), TerminalError>;
   ```

2. **`StepError` is the closure error type for `ctx.run` — a `retryable |
   terminal` union (Gap 1; the analogue of Restate's `HandlerError`).** It is
   named `StepError` (not `HandlerError`) because in our system it is
   specifically the `ctx.run` step-closure error, not a whole-handler error. It
   REPLACES the prior retryable-only `RetryableStepError` (greenfield
   single-cut — `RetryableStepError` is renamed/replaced, not kept alongside):

   ```rust
   pub enum StepError {
       /// Transient — the engine re-drives the workflow (per the step's retry
       /// policy; Gap 2). NEVER reaches the body's return type.
       Retryable { detail: String },
       /// Permanent — surfaces as `Err(TerminalError)` from `ctx.run(...).await`
       /// with NO retry and NO re-drive.
       Terminal(TerminalError),
   }
   ```

   The contract on each property:

   - **`StepError` is `!std::error::Error`** (the anyhow/eyre coherence trick),
     with a hand-written `Display`. This is what makes the blanket `From` below
     coherent.
   - **`impl<E: std::error::Error> From<E> for StepError`** →
     `Retryable { detail: e.to_string() }`. This preserves the `Ok(op().await?)`
     ergonomic: any std error `?`-folds into a retryable transient. The captured
     error's `Display` form becomes the (length-capped) transient `detail`.
   - **`impl From<TerminalError> for StepError`** → `Terminal(..)`. Coherent
     with the blanket above ONLY because `TerminalError` is `!Error` (§ 2,
     flipped under Gap 1). A `?`'d `TerminalError` inside a step routes to the
     terminal arm.
   - **Constructors:** `StepError::retryable(&str)` (replaces
     `RetryableStepError::new`), `StepError::terminal(TerminalError)` (or rely
     on `From` / `.into()`), and `detail(&self) -> &str` (the `Terminal` arm's
     detail is its `TerminalError`'s `detail()`; the `Retryable` arm's is its
     own). `retryable`'s `detail` is length-capped at construction
     (deterministic, UTF-8-safe — the same `cap_detail` discipline
     `TerminalError` uses).

   A step author therefore never names `StepError` on the happy path — any std
   error folds to retryable via the blanket, and an explicit terminal is `?`'d
   or `.into()`'d:

   ```rust
   ctx.run("charge", || async {
       Ok(stripe.charge(&card).await?)   // any std::error::Error → Retryable
   }).await?;                            // ctx.run yields Result<_, TerminalError>
   ```

   **`ctx.run` Err handling (the union's two arms):**
   - `Err(StepError::Terminal(t))` → `return Err(t)` from `ctx.run(...).await` —
     propagate the terminal; NO transient-slot record, NO body-park, NO
     re-drive.
   - `Err(StepError::Retryable { detail })` → record
     `WorkflowCtxError::TransientStep { name, detail }` in the transient slot +
     park the body (the existing Model Z mechanism, unchanged).

   **Footgun this FIXES.** Under the retryable-only shape, `?`-ing a
   `TerminalError` inside a step *silently became retryable* — because
   `TerminalError: Error` matched the blanket `From<E: Error>`, so an author's
   "fail terminally" inside a step was absorbed as a transient and re-driven
   forever (until budget). Under the union, `From<TerminalError>` routes it to
   `Terminal`: a `?`'d `TerminalError` inside a step now correctly fails the
   step terminally. This is the single behavioural reason `TerminalError` MUST
   drop `Error` (§ 2): the two `StepError` `From`s only coexist, and only route
   a terminal to the terminal arm, when `TerminalError` is not itself an
   `Error`.

   **`WorkflowDriveError` ripple.** `WorkflowDriveError::Terminal` currently
   embeds `#[from] TerminalError` with `#[error(transparent)]`, which requires
   `TerminalError: Error` and BREAKS once `TerminalError` is `!Error`. The fix:
   change that arm's attribute to `#[error("{0}")]` (Display via `TerminalError`'s
   hand-written `Display`), drop the `#[from]`, and add a manual
   `impl From<TerminalError> for WorkflowDriveError`. The
   `Transient(#[from] WorkflowCtxError)` arm is **unchanged** —
   `WorkflowCtxError` stays a `thiserror::Error` (it is the engine-internal infra
   type, not on the `StepError` coherence path).

   **Restate-parity author example** (the canonical shape the contract enables):

   ```rust
   let id = ctx.run("charge", async move {
       let resp = stripe.charge(&card).await?;            // network error → Retryable (engine re-drives)
       if resp.declined {
           return Err(TerminalError::explicit("card declined").into()); // permanent → Terminal, no retry
       }
       Ok(resp.id)
   }).await?;                                              // retryable absorbed; terminal (decline OR exhaustion) propagates
   ```

3. **Transient channel (D4) — the engine absorbs it; the body never sees it.**
   A `ctx.run` closure that resolves to `Err(StepError::Retryable { .. })`
   records a `WorkflowCtxError::TransientStep` in the ctx's transient slot
   **and parks the body** (the body's await never returns on the transient
   path). The `ErasedWorkflowAdapter::run_erased` detects the recorded
   transient (via `WorkflowCtx::take_transient_step`) and surfaces
   `WorkflowDriveError::Transient(WorkflowCtxError::TransientStep)`,
   **cancelling the parked body**; the engine re-drives from the journal
   against the failing step's `RunRetryPolicy` (Gap 2; default
   `WORKFLOW_RETRY_BUDGET`). The body's `?` can never observe the transient.
   This is the structural realisation of invariant 1 above — replacing the
   original § 4 language about a caller-defined error "folded into the step's
   own `T`" and "a `WorkflowCtxError` the engine classifies as transient"
   reaching anything body-visible. The `Err(StepError::Terminal(t))` arm does
   NOT park: it returns `Err(t)` from `ctx.run` directly, so the body's `?`
   observes the terminal and the body returns `Err(TerminalError)` — projected
   to `WorkflowStatus::Failed` (§ 3), never re-driven.

4. **ctx INFRA failures → `TerminalError::explicit`.** A ctx await-op that
   hits an engine-internal infra failure
   (`WorkflowCtxError::{NonDeterministic, JournalRecord, Serialize,
   Deserialize, Signal, ActionChannel}`) is projected **at the ctx-op
   boundary** to `TerminalError::explicit(<detail>)` — the same observable
   terminal the body's prior hand-folding
   (`.unwrap_or_else(|e| Err(TerminalError::explicit(...)))`) produced, and
   consistent with the existing panic → `Failed { Explicit }` containment path
   (§ 3). **No new `TerminalErrorKind` variant is added** — the four kinds
   above are exhaustive.

5. **`WorkflowCtxError` is RETAINED** in two engine-internal roles, but is no
   longer a ctx-op *return* type:
   - (a) the **transient-slot carrier** —
     `WorkflowDriveError::Transient(WorkflowCtxError::TransientStep { name, detail })`
     (Gap 2 adds a `policy: RunRetryPolicy` field to this variant so the failing
     step's retry policy reaches the engine's re-drive decision — see Gap 2; the
     carrier ROLE is otherwise unchanged from the committed code); and
   - (b) the **internal infra-error type** produced *inside* a ctx op before
     projection to `TerminalError` (point 4).

   It is still the type the journal-cursor surface returns
   (`replay_run`/`record_run`/… on the engine-facing journal trait); the
   projection to `TerminalError` happens at the public `WorkflowCtx` method
   boundary that wraps those internal calls.

6. **Non-substitutability is preserved** (see corrected § 2). `WorkflowCtxError`
   and `TerminalError` do not structurally substitute; the infra projection in
   point 4 is a deliberate one-way classification, not a bidirectional `From`.

### What this STRENGTHENS vs the original ADR

The original § 4 left the transient *conventionally* off the body's return
type (the body was expected to not propagate a transient `WorkflowCtxError`).
Model Z makes it *structurally* impossible: the body parks, the engine cancels
it, and the body's only error type is `TerminalError`. The "do not retry; fail
the workflow" channel (`TerminalError`) and the "retry me" channel (the
`StepError::Retryable` arm, engine-absorbed) are now disjoint and cannot be
confused at a call site — which is the whole point of the retryable-vs-terminal
split (D4). Gap 1 carries this one step further INTO the step closure: the
step's own outcome is a `retryable | terminal` union (`StepError`), so a step
can deliberately fail *terminally* (`StepError::Terminal`, surfaced as
`Err(TerminalError)` from `ctx.run`) OR *transiently* (`StepError::Retryable`,
engine-absorbed) — and the old footgun where a `?`'d `TerminalError` silently
became retryable is closed (point 2 above).

### Gap 2 — per-`ctx.run` retry policy (`RunRetryPolicy` + the `RunStep` builder)

Restate exposes a per-run retry policy via its `RunFuture` builder
(`.retry_policy(RunRetryPolicy)`); pre-Gap-2 our `ctx.run` re-drove against a
single engine-global `WORKFLOW_RETRY_BUDGET` constant + a standalone
`backoff_for_attempt` schedule fn with no per-step override. Gap 2 adds the
per-step policy while keeping the whole-workflow re-drive model ADR-0064
specified — and, as landed, the standalone `backoff_for_attempt` engine fn was
deleted (the re-drive path now reads its schedule from `RunRetryPolicy`, leaving
the fn unused-except-by-its-own-test, the named anti-pattern in
`.claude/rules/development.md` § "Deletion discipline"). `RunRetryPolicy::default()`
is the sole backoff SSOT post-Gap-2; `WORKFLOW_RETRY_BUDGET` is retained
(`RunRetryPolicy::default().max_attempts` derives from it).

#### Pinned surface

- **`RunRetryPolicy`** — a type mirroring Restate's fields, with `Default`:

  ```rust
  pub struct RunRetryPolicy {
      pub initial_delay:         Duration,
      pub exponentiation_factor: f64,
      pub max_delay:             Duration,
      pub max_attempts:          u32,
      pub max_duration:          Duration,
  }
  ```

  **`Default` MUST reproduce the prior behaviour exactly** — the
  `WORKFLOW_RETRY_BUDGET` attempt count (`max_attempts = WORKFLOW_RETRY_BUDGET`,
  i.e. `3`) and the `50ms / 100ms / 200ms` schedule (clamped to the last entry).
  `RunRetryPolicy::default()`'s fields encode that schedule **directly** —
  `initial_delay 50ms`, `exponentiation_factor 2.0`, `max_delay 200ms` yield the
  `50/100/200ms`-clamped window — and `max_duration` defaults to the saturating
  sum of that schedule over `max_attempts` windows (effectively unbounded
  relative to it, so the attempt-count gate is what fires first — exactly as
  before). The binding requirement is **observable equivalence when no policy is
  set**, so every existing test and DST invariant
  (`WorkflowBudgetExhaustionMintsTerminal`, `replay_equivalence_*`) holds
  unchanged. **`RunRetryPolicy::default()` is the sole backoff SSOT.**
  `WORKFLOW_RETRY_BUDGET` is retained (live —
  `RunRetryPolicy::default().max_attempts` derives from it, and the budget tests
  assert on it); the engine-side standalone `backoff_for_attempt` schedule fn was
  **deleted** as landed (after Gap 2 the re-drive path reads its window from
  `RunRetryPolicy::backoff_window`, leaving the fn unused-except-by-its-own-test
  and gated `#[expect(dead_code)]` — the named anti-pattern in
  `.claude/rules/development.md` § "Deletion discipline"). The engine-side
  `default_policy_reproduces_engine_constants` test pins
  `RunRetryPolicy::default()` against `WORKFLOW_RETRY_BUDGET` (still live) and the
  `50/100/200ms` schedule **as concrete literals** (the standalone fn no longer
  exists to compare against). NOTE: the *reconciler* `backoff_for_attempt` in
  `overdrive-core::reconcilers::workload_lifecycle` is a separate, LIVE,
  unrelated fn — only the workflow-engine copy was deleted; do not conflate.

- **`RunStep` builder** — `ctx.run(name, fut)` returns a builder (the analogue
  of Restate's `RunFuture`; named `RunStep`) that implements
  `IntoFuture<Output = Result<T, TerminalError>>` and exposes
  `.retry_policy(self, p: RunRetryPolicy) -> Self`. Existing call sites
  `ctx.run(name, fut).await?` stay valid (the default policy applies via
  `IntoFuture`); new sites add `.retry_policy(p)` before `.await`. `name` stays
  **positional** — the cosmetic `.name()` builder is out of scope.

  ```rust
  // default policy (today's behaviour) — unchanged call site:
  let id = ctx.run("charge", fut).await?;
  // explicit per-step policy:
  let id = ctx.run("charge", fut)
      .retry_policy(RunRetryPolicy { max_attempts: 10, ..Default::default() })
      .await?;
  ```

#### Architecture — KEEP whole-workflow re-drive; the failing step's policy governs the engine's re-drive decision

The recommended (and ratified) architecture **keeps the existing
whole-workflow re-drive model** (ADR-0064 durable model; § 4 above). We do NOT
switch to Restate's step-local in-place retry. Instead the FAILING step's
`RunRetryPolicy` governs the engine's re-drive decision:

1. The `RunStep` builder holds the policy. When the step's closure resolves to
   `Err(StepError::Retryable { detail })`, `ctx.run` records the transient in
   the slot **together with the step's policy** — the carrier
   `WorkflowCtxError::TransientStep` (or the transient slot alongside it) gains
   a `policy: RunRetryPolicy` field. (A `StepError::Terminal` arm does not carry
   a policy — it is terminal, never re-driven.)
2. The policy rides the existing transient signal path unchanged in shape:
   `WorkflowCtxError::TransientStep { name, detail, policy }` →
   `WorkflowDriveError::Transient` → `DriveOutcome::Transient { detail, policy }`.
3. `drive_to_terminal` / `redrive_decision` consult **that policy's
   `max_attempts` + backoff schedule** instead of a single engine-global
   default (when no per-step policy is set, that default is
   `RunRetryPolicy::default()` — the sole backoff SSOT post-Gap-2; the prior
   standalone `backoff_for_attempt` schedule fn was deleted). The signature
   becomes `redrive_decision(outcome, attempts, started_at, now)` (the policy
   travels inside `outcome`'s `Transient` arm); the `clock.sleep(...)` backoff
   window is computed from the policy
   (`initial_delay * exponentiation_factor^attempts`, clamped to `max_delay`)
   rather than the fixed schedule.

Observable behaviour = Restate parity: the step is retried per its policy; on
`max_attempts` (or `max_duration`) exhaustion `ctx.run(...).await` yields
`Err(TerminalError::budget_exhausted(..))` (the engine-minted terminal,
projected to `WorkflowStatus::Failed { BudgetExhausted }`). The body contract is
UNCHANGED — this is pure engine growth, the same shape the Slice-04 retry loop
already lands. The journal-derived attempt-count discipline (§ 4) is unchanged:
`attempts_from_journal` still counts `RetryAttempted` commands.

**Considered-and-rejected alternative — step-local in-place retry (Restate's
literal mechanism).** Restate retries a `run` closure *in place* — it re-invokes
the closure within the same handler invocation, without re-executing the
workflow from the top. **Rejected** because ADR-0064 specified the durable model
as **whole-workflow re-drive from the journal** (completed steps replay
byte-equal; the failed step re-fires) — the canonical Temporal/Restate
*re-execute-from-top-and-short-circuit* shape our `drive_to_terminal` loop
already implements. Switching to step-local in-place retry would re-architect
the durable model: the engine would have to suspend *inside* a single drive at
the failed step, hold the partially-driven body across the backoff park, and
resume the same future — incompatible with the "fresh ctx + reload journal per
drive" crash-resume contract (§ 4; the parked-body-cancel mechanism of Model Z)
and with the replay-equivalence DST invariant that asserts a re-drive replays
the *whole* trajectory. The per-step policy gives Restate-parity *observable
behaviour* (the step is retried per its policy) without disturbing the
re-drive-from-journal mechanism. (This is the analogue of D4's rejected
"reconciler-style View" alternative: parity of behaviour, not parity of
internal mechanism.)

#### Resolved sub-decisions

1. **`max_duration` measurement needs a start instant — journal it as an
   input.** Per `development.md` § "Persist inputs, not derived state", the
   retry window's START timestamp is an input that must survive crash-resume (so
   elapsed can be recomputed against `clock.unix_now()` each drive). It is
   journaled on the FIRST `RetryAttempted` for the step: the `RetryAttempted`
   command (ADR-0063 §2; additive `#[serde(default)]`) gains a
   `started_at_unix: Option<std::time::Duration>` field — `Some(clock.unix_now())`
   on the first attempt, `None` thereafter — exactly mirroring the
   `SleepArmed { deadline_unix: Duration }` absolute-wall-clock shape. The engine
   recovers the start instant by scanning the loaded run for the first
   `RetryAttempted` carrying `Some(started_at_unix)`, and recomputes
   `elapsed = clock.unix_now() − started_at_unix` on each drive; a re-drive is
   gated on BOTH `attempts < policy.max_attempts` AND `elapsed <
   policy.max_duration`. No derived "deadline" field is persisted — only the
   start instant (an input) and the attempt count (recomputed from the
   `RetryAttempted` count). This addition is additive-`#[serde(default)]` per
   ADR-0063 §2 and is engine-side only; the body contract is untouched.
2. **Multi-step policy resolution — the FAILING step's policy governs.** When a
   workflow has several `ctx.run`s with different policies, the policy of the
   step that FAILED on the current drive governs the re-drive decision.
   Completed steps replay (they do not fail), so only the one step whose closure
   resolved to `StepError::Retryable` on this drive contributes a policy — it is
   the policy carried on this drive's transient signal. There is no merging or
   precedence across steps: at most one step fails-and-parks per drive (the body
   short-circuits there), so exactly one policy is in play per re-drive.
3. **`RunRetryPolicy` is NOT persisted — it is derived from the workflow CODE.**
   The policy is set by the `RunStep` builder call in the body, so it is
   recomputed each drive from the body (the builder re-runs when the body
   re-executes the failing step), NOT persisted. The journal persists only the
   attempt INPUTS — the `RetryAttempted` count and the first attempt's
   `started_at_unix`. This aligns with "persist inputs, not derived state": the
   policy is a function of the code (like `RunRetryPolicy::default()`, derived
   from `WORKFLOW_RETRY_BUDGET` + the literal schedule, is); the inputs it consumes are what
   persist. A consequence the crafter must preserve: a re-drive re-runs the body
   up to the failing step, so the policy is re-derived identically each drive —
   a body that changes its `.retry_policy(..)` across drives would be
   non-deterministic and is already forbidden by the replay-equivalence contract
   (§ 4; `development.md` § "Workflow contract").

#### Phasing note (surfaced, not self-deferred)

All `RunRetryPolicy` fields are in scope for the crafter to implement to this
ratified surface. One field's *implementation* carries a journal-schema
ripple worth the orchestrator's explicit attention rather than a silent
crafter decision: **`max_duration` requires the additive
`RetryAttempted.started_at_unix` field** (sub-decision 1) — a journal command
schema change. The journal (`JournalCommand`) is **CBOR (`ciborium`) `#[serde]`
with additive `#[serde(default)]` evolution — NO golden-bytes fixture and NO
`#[serde(tag="v")]` version envelope** (per the journal module's own codec doc,
`crates/overdrive-control-plane/src/journal/mod.rs` header, and ADR-0063 §2;
greenfield single-cut, no surviving on-disk journals). The golden-bytes /
versioned-envelope schema-evolution ceremony (testing.md "Archive
schema-evolution roundtrip") applies to **rkyv** envelopes (ADR-0048:
observation rows, intent aggregates, `WorkflowStart`), NOT the CBOR journal.
`started_at_unix: Option<Duration>` is added under `#[serde(default)]`
(additive, ADR-0063 §2); the sim CBOR round-trip generator exercises both
`Some`/`None` arms — no fixture. This is **not a deferral** (no scope is being
pushed to a future slice; the field is specified here and lands with the Gap-2
implementation) and therefore needs no GitHub issue. It is flagged so the
orchestrator dispatches the Gap-2 crafter with the journal-schema change in
explicit scope (the additive `#[serde(default)]` field + the sim CBOR
round-trip covering both arms as the deliverable), not as a surprise discovered
mid-implementation. If the
crafter finds the journal-schema change genuinely warrants its own slice, that
is a blocker to surface to the user for approval — never a unilateral split.

## Considered alternatives

### Alternative A — Amend ADR-0064 in place (rejected for the ADR-vs-supersession call)

ADR-0064 already carries a "Changed Assumptions" amendment block
(2026-06-06, CA-5..CA-7). One option is to add a CA-8..CA-11 block here.
**Rejected** because: (a) the result/error model is a *cohesive new
decision* spanning four named priorities (object safety, input surface,
status projection, retry model) that a future reader must cite as a unit —
burying it in a fourth amendment block to an already 700-line ADR harms
discoverability; (b) the project ADR convention is "single decision per
ADR; immutable, supersede rather than modify" (nw-architecture-patterns
§ ADR Templates) — a new decision of this size earns its own number; (c) it
*partially* supersedes ADR-0064 (§2 wholesale, §3/§5/§6 in part) while
leaving §1/§3-cursor/§4 intact, which is exactly the "amends specific
sections" shape a sibling ADR expresses more honestly than an in-place
edit that would force re-reading the whole document to see what changed.
ADR-0064 gets a one-line Changed-Assumptions pointer to ADR-0065; the
brief.md ADR index marks ADR-0064 "Accepted (§2/§3/§5/§6 amended by 0065)".

### Alternative B — Single associated `type Output` + engine holds `dyn Any` (rejected)

The engine downcasts the body's output at the terminal boundary.
**Rejected**: `Any + Send + serde::Serialize` do not compose into a
`dyn`-stored value cleanly; the registry loses the compile-time output type;
the journal would need a per-workflow result schema (no homogeneous
`Terminal` command). The CBOR-erasure adapter (D1) gives the same author
ergonomics with a homogeneous durable surface and no `Any`.

### Alternative C — Keep `WorkflowResult` as the body return, add `WorkflowStatus` only at the projection (rejected)

Keep the body returning the three-variant enum; add the control-plane
status separately. **Rejected**: this retains all three anti-patterns the
research identifies (contentless success, retryable-as-terminal conflation,
body-authored cancellation) and is the weakest evidence alignment (no
surveyed platform returns a status enum from the body). The whole point of
the finding is that the body return and the control-plane status are *two
different types*; keeping the old enum as the body return defeats it.

### Alternative D — Put the retry budget in a reconciler-style `View` (rejected)

Mirror the reconciler `RetryMemory` View. **Rejected**: a workflow has an
engine; the reconciler pattern exists *because* a reconciler has no engine.
The journal is already the instance's durable progress SSOT — deriving the
attempt count from journal `RetryAttempted` entries keeps one durable store,
not two, and keeps the budget recomputed-from-inputs against the live policy
(D4). A second View store for workflow retry would duplicate the journal's
role.

### Alternative E — Step-local in-place retry, Restate's literal mechanism (rejected; Gap 2)

Adopt Restate's actual implementation: retry the failing `ctx.run` closure
*in place* — re-invoke it within the same handler invocation, suspending inside
a single drive at the failed step rather than re-executing the workflow from the
top. **Rejected** because ADR-0064 specified the durable model as
**whole-workflow re-drive from the journal** (completed steps replay byte-equal;
the failed step re-fires) — the Temporal/Restate
*re-execute-from-top-and-short-circuit* shape `drive_to_terminal` already
implements. Step-local in-place retry would re-architect that model: the engine
would suspend *inside* a drive, hold the partially-driven body across the
backoff park, and resume the same future — incompatible with the "fresh ctx +
reload journal per drive" crash-resume contract (§ 4), with the Model Z
parked-body-cancel mechanism, and with the replay-equivalence DST invariant that
asserts a re-drive replays the *whole* trajectory. The per-step `RunRetryPolicy`
(Gap 2) delivers Restate-parity *observable behaviour* — the step is retried per
its policy; exhaustion mints `BudgetExhausted` — by governing the engine's
existing re-drive decision, without disturbing the re-drive-from-journal
mechanism. (Parity of behaviour, not of internal mechanism — the same shape
Alternative D rejects for the budget *location*.)

## Consequences

### Positive

- **Body contract matches all four surveyed platforms.** Typed success
  output, terminal-error failure channel, retryable absorbed by the engine,
  cancellation/timeout engine-owned. The author writes ordinary Rust
  `Result<CertOutput, TerminalError>`.
- **Replay determinism strengthened.** The free-text body `reason: String`
  is gone; the durable terminal carries a typed `WorkflowStatus` whose
  `Failed` arm is a bounded `TerminalError` (kind + length-capped author
  detail) — no engine-derived non-deterministic value can leak into the
  durable terminal (closes the hazard the panic-containment path worked
  around).
- **#217 resolved.** `input_digest` hashes the start-parameter bytes; two
  instances of the same kind with different inputs are distinguishable in
  the journal.
- **#40 unblocked.** Cert-rotation can express its typed `CertSpec` input
  and typed cert output through the new surface — the validating consumer.
- **Object safety preserved with no author boilerplate.** The
  `ErasedWorkflowAdapter` blanket impl erases typed `Output`/`Input` to CBOR
  the same way `ctx.run<T>` already does; the engine's `dyn` interior is
  unchanged in shape.
- **Full Restate Rust `ContextSideEffects::run` parity (Gap 1 + Gap 2).** The
  step closure is a `retryable | terminal` union (Restate `HandlerError`
  analogue), a `?`'d std error folds to retryable while a `?`'d `TerminalError`
  fails the step terminally (footgun closed), and a per-`ctx.run`
  `RunRetryPolicy` mirrors Restate's `RunFuture::retry_policy` — all while the
  `ctx.run(...).await -> Result<T, TerminalError>` return type and the
  whole-workflow re-drive durable model are unchanged. An author who knows
  Restate's `run` knows ours.

### Negative

- **Two terminal-modelling types remain, now three with `TerminalCondition`.**
  `TerminalError` (body's failure channel), `WorkflowStatus` (engine's
  observable projection), `TerminalCondition` (reconciler's allocation
  claim, ADR-0037). They model genuinely different things; the ADR pins the
  distinctions, but a reader must not conflate them.
- **`WorkflowStart` crosses an rkyv-envelope boundary now.** Identity-only
  needed no envelope; input-bearing durable intent does (ADR-0048
  discipline + a golden-bytes schema-evolution fixture). One more durable
  schema to evolve carefully — but it rides the established `Job` codec
  precedent.
- **The retry-re-drive loop is deferred to Slice 04.** Slices 01–03 land the
  types + success/explicit-terminal paths; the `BudgetExhausted` minting and
  backoff parking land after. Until Slice 04 the engine's behaviour is
  "explicit `Err(TerminalError)` ends the instance" (no transient re-drive)
  — a smaller behaviour than the final contract, but the body contract is
  stable from Slice 01 (additive engine growth, no body re-litigation).
- **`TerminalError` MUST stay `!std::error::Error` — a coherence constraint a
  future maintainer can silently break (Gap 1).** Re-deriving
  `thiserror::Error` on `TerminalError` "for convenience" would make
  `StepError`'s blanket `From<E: Error>` cover it, colliding with
  `impl From<TerminalError> for StepError` AND re-opening the footgun (a `?`'d
  `TerminalError` would fold to retryable again). The constraint is load-bearing
  but invisible from `TerminalError`'s own site; the `WorkflowStepTerminalShortCircuits`
  DST invariant is the structural guard (it fails if the terminal arm starts
  re-driving), and a doc-comment on `TerminalError` names the constraint.
- **One additive journal-schema field (`RetryAttempted.started_at_unix`).**
  Gap 2's `max_duration` needs the retry window's start instant journaled (an
  input). The journal is CBOR (`ciborium`); the field is additive
  (`#[serde(default)]`, ADR-0063 §2) and needs **no golden-bytes fixture and no
  version envelope** — the sim CBOR round-trip exercising both `Some`/`None`
  arms is the coverage. (The golden-bytes ceremony is rkyv-only — ADR-0048;
  it applies to `WorkflowStart`, not the CBOR journal.) One more durable field
  to evolve carefully, but on the established additive-`#[serde(default)]`
  journal precedent.

### Quality-attribute impact

- **Maintainability — modifiability**: positive (typed body surface; author
  writes domain types, not status variants).
- **Maintainability — testability**: positive (the erasure adapter is unit-
  testable in isolation; the DST replay-equivalence invariant gains a
  typed-output assertion).
- **Reliability — recoverability**: positive (typed `WorkflowStatus` in the
  durable terminal replays losslessly including `Completed`'s output bytes).
- **Reliability — fault tolerance**: positive (retryable-vs-terminal split is
  now structural; the engine owns retry, the body owns only terminal).
- **Functional suitability — correctness**: positive (the free-text
  determinism hazard is closed).

## DST invariants (amending ADR-0064 §6)

- **`replay_equivalence_provision_record`** (carried forward) — the
  `ProvisionRecord` fixture moves from returning `WorkflowResult::Success`
  to returning `Ok(())` (its `Output = ()`); the invariant's terminal assertion
  changes from "terminal is `Success`" to "the `WorkflowStatus` projection
  is `Completed { output }` and the erased output round-trips to `()`". The
  byte-identical-trajectory + bounded-progress core is unchanged.
- **`WorkflowExactlyOnceEffectOnResume`** (carried forward) — unchanged; the
  `ctx.run` step semantics are untouched by this ADR.
- **`WorkflowBudgetExhaustionMintsTerminal`** (carried forward, default-policy
  invariant) — holds UNCHANGED under Gap 2 because `RunRetryPolicy::default()`
  (the sole backoff SSOT) reproduces the `WORKFLOW_RETRY_BUDGET` attempt count
  and the `50/100/200ms` schedule exactly: a forced-transient step that sets no
  policy still re-drives the same number of times and mints the same
  `BudgetExhausted`. This invariant is the binding guard for the "Default ==
  prior behaviour" requirement.
- **NEW `WorkflowTerminalStatusProjection`** — drives a workflow that returns
  `Err(TerminalError::explicit(...))` and asserts the engine writes
  `WorkflowStatus::Failed { terminal }` (NOT a contentless variant) with the
  `TerminalError` round-tripping byte-equal through the journal `Terminal`
  command and the observation row. Pins the body-return → status-projection
  mapping (D3) as a structural property.
- **NEW (Slice 04) `WorkflowBudgetExhaustionMintsTerminal`** — under a
  forced-transient-error workflow, assert the engine re-drives up to the
  budget and then mints `WorkflowStatus::Failed { terminal: BudgetExhausted }`
  — the body never authored a failure (D4). Lands with the retry-loop slice.
- **NEW (Gap 1) `WorkflowStepTerminalShortCircuits`** — drives a workflow whose
  `ctx.run` closure resolves to `Err(StepError::Terminal(TerminalError::explicit(..)))`
  and asserts: (a) `ctx.run(...).await` yields `Err(TerminalError)` (NOT a
  re-drive), (b) NO `RetryAttempted` command is journaled (the terminal arm does
  not park or re-drive), and (c) the projection is `WorkflowStatus::Failed`. Pins
  the union's terminal arm (the footgun-fix: a `?`'d `TerminalError` inside a
  step fails terminally, never silently retries).
- **NEW (Gap 2) `WorkflowPerStepRetryPolicyGovernsRedrive`** — drives a
  forced-transient step carrying a non-default `RunRetryPolicy`
  (`max_attempts = N`, N ≠ `WORKFLOW_RETRY_BUDGET`) and asserts the engine
  re-drives exactly `N` times (the journal holds `N` `RetryAttempted` commands)
  before minting `BudgetExhausted` — the per-step policy, not the global
  constant, gates the re-drive count. A companion assertion drives a
  `max_duration`-bounded policy and asserts exhaustion on the elapsed-window gate
  (recomputed from the journaled first-attempt `started_at_unix`) even when
  `attempts < max_attempts`.

## References

- ADR-0064 — the amended base (§2/§3/§5/§6); §1/§3-cursor/§4/§5-boundary
  carried forward.
- ADR-0063 — the journal (`Terminal` command's `status` field; `Started`
  digests; the additive `RetryAttempted` command for D4).
- ADR-0048 — rkyv versioned-envelope + typed-codec discipline `WorkflowStart`
  input adopts (the `Job` aggregate precedent).
- ADR-0037 — `TerminalCondition` (the control-plane terminal-status sibling
  `WorkflowStatus` mirrors in convention, not type).
- ADR-0035 — reconciler `RetryMemory`-in-`View` (the precedent D4 contrasts:
  workflows have an engine, reconcilers do not).
- `docs/research/workflow-durable-execution/result-error-retry-semantics-research.md`
  — the four-platform investigation (Restate/Temporal/DBOS/Step Functions),
  High confidence, the evidence base replacing a DISCUSS wave.
- Restate Rust SDK — `restate_sdk::context::ContextSideEffects::run`,
  `RunFuture`, `RunRetryPolicy`, `HandlerError` / `TerminalError` (docs.rs) —
  the parity reference for the Amendment 2026-06-07 Gap 1 (`StepError` union)
  and Gap 2 (`RunRetryPolicy` + `RunStep` builder).
- [#217](https://github.com/overdrive-sh/overdrive/issues/217) — resolved
  (input_digest off the input bytes).
- [#40](https://github.com/overdrive-sh/overdrive/issues/40) — unblocked
  (cert-rotation, the validating consumer).
- `.claude/rules/development.md` § "Workflow contract", § "rkyv schema
  evolution", § "Persist inputs, not derived state", § "Trait definitions
  specify behavior, not just signature".

## Changelog

- 2026-06-06 — Initial proposed version. Amends ADR-0064 §2/§3/§5/§6.
  Body return → `Result<Output, TerminalError>`; status enum → engine-owned
  `WorkflowStatus` control-plane projection; typed `WorkflowStart.input`
  crossing Raft with rkyv-envelope discipline (resolves #217); retry budget
  in engine/journal (D4). Greenfield single-cut — `WorkflowResult` deleted.
- 2026-06-07 — In-place amendment reconciling the ctx-op error surface with
  the final implemented contract ("Model Z"). Status flipped to Accepted.
  `WorkflowCtx` await-ops (`run`/`sleep`/`wait_for_signal`/`emit_action`) now
  return `Result<_, TerminalError>` (was `Result<_, WorkflowCtxError>`); the
  `ctx.run` closure error is `RetryableStepError` (`Display` + `!Error`, with
  the blanket `From<E: Error>`); the transient parks the body and is surfaced
  as `WorkflowDriveError::Transient` by the adapter (structurally invisible to
  the body); infra `WorkflowCtxError`s inside ctx ops are projected to
  `TerminalError::explicit` at the ctx-op boundary (no new `TerminalErrorKind`
  variant); `WorkflowCtxError` retained as transient-slot carrier + internal
  infra type only. Folds in commits `40fb7772` (transient is a step concern,
  not a `TerminalErrorKind::Retryable`) and `6dd50299` (single `ctx.run`,
  closure → `Result<T, RetryableStepError>`). Corrects stale § 2 / § 4 prose +
  snippets in place; § 1 / § 3 / § 5 unchanged. Both core invariants restated
  as still-true (transient never reaches the body's return type; `TerminalError`
  is purely terminal). Greenfield single-cut — old `Result<usize, String>`
  step-folding shape deleted. (The `RetryableStepError` retryable-only closure
  error from this entry is itself superseded within the same 2026-06-07
  amendment by the Gap-1 `StepError` union — see the next entry.)
- 2026-06-07 — Restate-parity extension of the Model Z amendment (Gap 1 +
  Gap 2), accepted-but-not-yet-implemented. **Gap 1:** the `ctx.run`
  step-closure error becomes a `retryable | terminal` union `StepError`
  (Restate `HandlerError` analogue), REPLACING the retryable-only
  `RetryableStepError` (greenfield single-cut). `StepError` is `!std::error::Error`
  with a blanket `impl<E: std::error::Error> From<E>` (→ `Retryable`) and an
  `impl From<TerminalError>` (→ `Terminal`); `TerminalError` is flipped to
  `!std::error::Error` (hand-written `Display`, no `thiserror::Error` derive) so
  the two `From`s coexist coherently. `ctx.run` routes `StepError::Terminal(t)`
  to `Err(t)` (no re-drive) and `StepError::Retryable{..}` to the transient
  slot + body park. Fixes the footgun where a `?`'d `TerminalError` inside a
  step silently became retryable. `WorkflowDriveError::Terminal` ripple: drop
  `#[from]` + `#[error(transparent)]`, use `#[error("{0}")]` + a manual
  `From<TerminalError>` (the `Transient(#[from] WorkflowCtxError)` arm is
  unchanged). **Gap 2:** a per-`ctx.run` `RunRetryPolicy`
  (`{ initial_delay, exponentiation_factor, max_delay, max_attempts,
  max_duration }`, whose `Default` encodes the prior `WORKFLOW_RETRY_BUDGET`
  attempt count + the literal `50/100/200ms` schedule directly and is the sole
  backoff SSOT — the standalone engine `backoff_for_attempt` schedule fn was
  deleted as landed; `WORKFLOW_RETRY_BUDGET` retained) set on a `RunStep` builder
  (`ctx.run(name, fut).retry_policy(p).await?`; `IntoFuture` keeps existing call
  sites valid). KEEPS the whole-workflow re-drive model (ADR-0064); the FAILING
  step's policy rides the transient signal
  (`WorkflowCtxError::TransientStep` gains `policy`) and
  `drive_to_terminal`/`redrive_decision` consult it instead of the global
  constant. `max_duration`'s start instant is journaled as an additive
  `RetryAttempted.started_at_unix: Option<Duration>` input (ADR-0063 §2; first
  attempt only). Step-local in-place retry is the considered-and-rejected
  alternative (would re-architect the durable re-drive model). Both core
  invariants restated as still-true under Gap 1 + Gap 2. Greenfield single-cut.
