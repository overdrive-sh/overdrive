# ADR-0065 — Workflow body returns `Result<T, TerminalError>`; status enum becomes an engine-owned control-plane projection; typed `WorkflowStart` input crosses Raft with rkyv-envelope discipline

## Status

Proposed. 2026-06-06. PROPOSE mode (subagent context — surfaced for user
ratification in the DESIGN return summary). Decision-makers: Morgan
(proposing). Tags: phase-1, workflow-primitive, application-arch,
durable-execution, result-error-model, dst.

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

### 2. `TerminalError` — a concrete core type, thiserror, analogous to `WorkflowCtxError` (D2)

```rust
/// The terminal-failure channel of a workflow body. A workflow that returns
/// `Err(TerminalError)` ends with a terminal failure; a workflow that
/// returns `Ok(Output)` succeeds. RETRYABLE failures never construct this — a
/// body that hits a transient error returns it through its own `Output` typing
/// inside a `ctx.run` step (the step's `Err` is re-driven by the engine), or
/// propagates a `WorkflowCtxError` the engine treats as retryable (D4). A
/// `TerminalError` is the explicit "do not retry; fail the workflow" signal.
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
`Terminal` command and the terminal observation row as an input). It is the
workflow analogue of `WorkflowCtxError` (the await-op error) but models a
different thing: `WorkflowCtxError` is an *engine-internal* await failure
(journal record failed, non-deterministic replay); `TerminalError` is the
*body's authored terminal-failure outcome*. They do not substitute.
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

- **Retryable** (engine absorbs, re-drives internally; never reaches the
  body's return type): a `ctx.run` step whose closure future resolves to a
  caller-defined error folded into the step's own `T` (the existing
  `Result<usize, String>` pattern), OR a `WorkflowCtxError` the engine
  classifies as transient. The engine re-drives the workflow from the
  journal (completed steps replay; the failed step re-fires) against a
  bounded budget.
- **Terminal** (ends the workflow): an explicit `Err(TerminalError)` the
  body returns, OR an engine-minted `TerminalError { BudgetExhausted }` when
  the retry budget is exhausted.

**Retry budget location — the engine/journal, contrasting the reconciler
`RetryMemory` View precedent.** A reconciler has no engine; ADR-0035 puts
its retry memory in the `View` (`RetryMemory { attempts, last_failure_seen_at }`,
recompute-the-deadline-on-read). A **workflow HAS an engine** — the budget
belongs there, not in the body and not in a reconciler-style View. Concretely:

- The budget POLICY (max attempts, backoff schedule) is an engine constant
  for Phase 1 (a `WORKFLOW_RETRY_BUDGET` analogous to the reconciler
  `RETRY_BACKOFFS` table), consulted by the engine — NOT persisted (per
  `development.md` § "Persist inputs, not derived state"; the policy is a
  function, the inputs are persisted).
- The budget INPUTS (attempts-so-far, last-failure-time) are derived from
  the **journal** — the count of re-drive entries the engine has recorded
  for the instance — not from a separate store. This keeps the journal the
  single durable SSOT for the instance's progress AND its retry state, and
  keeps the budget recomputed-from-inputs (the journal is the input; the
  attempt count and next-retry deadline are recomputed against the live
  policy on each re-drive). A dedicated retry-bookkeeping journal entry
  (a `RetryAttempted` command, additive per ADR-0063 §2) records the
  attempt input; the engine recomputes `attempts` and the backoff deadline
  from the count of these entries + the policy on each re-drive.

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
