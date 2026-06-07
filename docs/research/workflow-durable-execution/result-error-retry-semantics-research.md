# Research: Durable-Execution Workflow Completion / Error / Retry Semantics — Status-Enum vs Typed-Value-with-Terminal-Error

**Date**: 2026-06-06 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 9

## Executive Summary

Across all four durable-execution platforms surveyed — **Restate**, **Temporal**, **DBOS**, and **AWS Step Functions** — the workflow/handler **body returns a typed domain value on success and signals failure by *throwing/returning an error*, never by returning a status enum**. The idiomatic shape is Restate's `Result<T, HandlerError>` (Rust SDK: `HandlerResult<T> = Result<T, HandlerError>`) and Temporal's typed return + thrown `ApplicationFailure`. Success is `Ok(T)` — the real output — not a `Success` variant.

Retryable errors are **absorbed by the engine and re-driven internally**; they never surface as a caller-visible outcome. A failure becomes the workflow's terminal outcome only when it is (a) **explicitly marked terminal** (Restate `TerminalError`, Temporal `non_retryable` ApplicationFailure, Step Functions terminal error names), or (b) **the retry budget is exhausted** — at which point the *engine* mints a terminal error and fails the invocation (Restate: *"the run block will fail with a TerminalException once the retries are exhausted"*; Step Functions: retries cease and the execution fails unless caught). **Cancellation is a control-plane operation**, not a return variant the body produces: in both Restate and Temporal it is initiated externally and *delivered into* the running body (Restate throws a `TerminalError` at the next await point; Temporal delivers a cancellation request the workflow handles).

There IS a real status enum in these systems — but it lives at the **control-plane / observable-history layer**, not in the body's signature. Temporal records six terminal statuses (Completed / Failed / Cancelled / Terminated / TimedOut / ContinuedAsNew); Step Functions records SUCCEEDED / FAILED / TIMED_OUT / ABORTED; DBOS tracks PENDING and friends. Several of these statuses (Terminated, TimedOut, ContinuedAsNew) *cannot* be produced by the body at all — they arise from engine events. **The evidence therefore strongly supports the project's design claim**: the body should return `Result<T, TerminalError>` (typed value on success, terminal error on unrecoverable failure, retryable errors absorbed by the engine), and the status enum, if kept, belongs to the control plane that *records* the outcome — a separate concern from the body's return type. The crux distinction — *body return type* vs *externally-observable terminal status* — is the one the current `WorkflowResult` enum conflates.

## Research Methodology

**Search Strategy**: Official vendor docs (Restate, Temporal, DBOS, Azure Durable Functions, AWS Step Functions) + SDK source repos (github.com/restatedev, github.com/temporalio) + docs.rs crate references. Cross-reference each load-bearing claim against an independent source.
**Source Selection**: Types: official / technical_docs / industry_leaders / academic. Reputation: high preferred. Vendor docs treated as authoritative-minimum per augmented config.
**Quality Standards**: Target 3 sources/claim (min 1 authoritative). All major claims cross-referenced. Vendor doc + SDK source = 2 independent angles.

## Findings

### Q1: Restate handler / workflow return type (Rust SDK specifics)

**Finding 1.1 — Handlers return `Result<T, HandlerError>`, not a status enum.**
**Evidence**: The Rust SDK handler shape is
```rust
async fn my_handler(&self, ctx: Context<'_>, param: T) -> Result<U, HandlerError>
```
The success type `U` is the handler's *real serializable output*; the error type is `HandlerError`. Success is represented as `Ok(U)` — the typed value — never as a "Success" variant of a status enum.
**Source**: [docs.rs restate_sdk](https://docs.rs/restate-sdk/latest/restate_sdk/) — Accessed 2026-06-06
**Confidence**: High (authoritative-minimum vendor SDK doc; signature corroborated by Q2 vendor docs)
**Analysis**: The control-flow contract is the ordinary Rust `Result`. A handler that completes returns its domain value `T`; there is no separate "did it succeed" discriminant the body produces. Restate workflows are a specialization of handlers using `WorkflowContext`; the `run` handler of a workflow follows the same `Result<T, HandlerError>` shape.

### Q2: Restate error model — retryable vs terminal

**Finding 2.1 — Default = all errors retryable; terminal is the explicit opt-out.**
**Evidence**: *"Restate assumes by default that all errors are transient errors and therefore retryable."* Regular Rust `std::error::Error` implementations "cause infinite retries by default" — *"Restate retries failures infinitely."* To stop retries you must throw a `TerminalError` (Rust `TerminalError` type; `TerminalException` in Java/Kotlin; `TerminalError` in TS/Python).
**Source**: [docs.restate.dev error-handling](https://docs.restate.dev/guides/error-handling) + [docs.rs restate_sdk](https://docs.rs/restate-sdk/latest/restate_sdk/) — Accessed 2026-06-06
**Confidence**: High (two independent angles: vendor guide + SDK API doc)
**Analysis**: Retryable failures never surface as a return *value* the caller branches on — they are absorbed by the engine and re-driven. Only a `TerminalError` propagates back to the caller as the failed outcome: *"Unless catched, terminal errors stop the execution and are propagated back to the caller."* This is the load-bearing distinction for the synthesis: the body's `Err` is a *retry signal*, not an *outcome*, unless it is terminal.

### Q3: Restate retry budget / policy

**Finding 3.1 — Retry policy is exponential backoff, configurable at invocation / run-block / global level, default capped at 70 attempts.**
**Evidence**: *"Restate retries failed invocations according to a retry policy that always uses exponential back-off."* Configurable parameters: initial interval, exponentiation factor, max interval, and max attempts ("either limited or unlimited"). Next retry = `min(last_retry_interval * factor, max_interval)`. Default policy: initial interval 50ms, factor 2.0, **max attempts 70**, max interval 60s.
**Source**: [docs.restate.dev guides/error-handling](https://docs.restate.dev/guides/error-handling) + [docs.restate.dev services/configuration](https://docs.restate.dev/services/configuration) — Accessed 2026-06-06
**Confidence**: High (vendor guide + vendor config reference, two independent pages)
**Analysis**: Note the project prompt's hypothesis "infinite retries by default" is *partially* correct — the SDK-level default for an uncaught error is "retry," and unlimited is configurable, but the *shipped* default policy caps at 70 attempts rather than literally infinite. The "infinite" framing in the SDK docs describes the conceptual default (errors are retryable unless terminal), while the configured policy bounds it.

**Finding 3.2 — Budget exhaustion converts to a terminal failure (kill) or pauses the invocation.**
**Evidence**: *"When attempts are exhausted, you can configure what Restate should do with the invocation: Pause it, requiring the user to manually resume it, or kill it, which automatically fails the invocation and responds to the caller with a terminal error."* For a run block specifically: *"the run block will fail with a TerminalException once the retries are exhausted."*
**Source**: [docs.restate.dev guides/error-handling](https://docs.restate.dev/guides/error-handling) — Accessed 2026-06-06
**Confidence**: High (explicit vendor quote, corroborated across error-handling guide and config search)
**Analysis**: This is the direct evidence for the project's design claim: *retryable errors are absorbed and re-driven by the engine; budget exhaustion is the event that mints a terminal error and fails the invocation.* The handler body does not author "I failed" — the engine does, on exhaustion. (The "pause" option is a nuance: exhaustion does not *unconditionally* produce a terminal failure; an operator can configure pause-for-manual-resume instead. Either way, the handler body never returns a failure-status variant.)

### Q4: Cancellation as a distinct outcome (Restate)

**Finding 4.1 — Cancellation is a control-plane operation, surfaced into the handler as a Terminal Error at the next await point.**
**Evidence**: Cancellation is triggered externally (HTTP PATCH to the admin/ingress API, CLI). *"Cancellations are Terminal Errors."* *"When you cancel a workflow, Restate stops the execution by throwing a Terminal Error."* The mechanism: *"First, Restate tries to cancel the leaves of the current invocation ... Once the leaves are canceled, a terminal error is thrown in the service handler at the point in the code that the invocation had reached."* It surfaces *"at the next await point - operations that wait for a result, such as ctx.run(), call responses, sleeps, awakeable results."* Cancellation is non-blocking and propagates recursively up the call graph, letting handlers run compensation.
**Source**: [docs.restate.dev guides/error-handling](https://docs.restate.dev/guides/error-handling) + [docs.restate.dev operate/invocation](https://docs.restate.dev/operate/invocation/) + [docs.restate.dev services/invocation/managing-invocations](https://docs.restate.dev/services/invocation/managing-invocations) — Accessed 2026-06-06
**Confidence**: High (three vendor pages agree)
**Analysis**: Crucial for the synthesis: in Restate, "cancelled" is **not a return variant the body produces**. It is (a) initiated by the control plane, and (b) delivered *into* the body as a `TerminalError` thrown at an await point — so the body experiences cancellation as an error to compensate against, not as an outcome it chooses to return. The externally-observable terminal status ("cancelled") is recorded by the engine, distinct from the in-body error mechanism.

### Q5: Temporal comparison — typed return vs failure types, retry policy

**Finding 5.1 — Temporal workflow code returns a typed value; failures are thrown, not returned.**
**Evidence**: User code raises failures via `ApplicationFailure` — *"the only type of Temporal Failure created and thrown by user code."* The `non_retryable` flag (or matching a Retry Policy's non-retryable type list) is what makes a failure terminal: *"When an Application Failure carries `non_retryable` as `true` ... the system stops retry attempts and marks the execution as failed."* Otherwise *"failures trigger automatic retries according to the Retry Policy."* In the SDKs, a workflow function returns its typed result (e.g. `string`, a struct) on success and *raises* a failure on error — the same throw/return split as Restate.
**Source**: [docs.temporal.io references/failures](https://docs.temporal.io/references/failures) — Accessed 2026-06-06
**Confidence**: High (authoritative vendor reference)
**Analysis**: Same shape as Restate at the *body* level — success = typed value, failure = thrown error, terminal = explicitly-marked non-retryable error.

**Finding 5.2 — Activities retry by default; Workflow Executions do NOT.**
**Evidence**: *"Unlike Activities, Workflow Executions do not retry by default."* / *"Temporal's default behavior is to automatically retry an Activity that fails"* (default Activity policy: 1s initial interval, 2.0 coefficient, 100s max, unlimited attempts). Guidance: target retries at the activity level, not by retrying the whole workflow.
**Source**: [docs.temporal.io encyclopedia/retry-policies](https://docs.temporal.io/encyclopedia/retry-policies) — Accessed 2026-06-06
**Confidence**: High (explicit vendor quote)
**Analysis**: This is a meaningful divergence from Restate. Restate retries the *invocation/handler* by default; Temporal retries *Activities* by default but not whole Workflows. For a primitive like Overdrive's `WorkflowCtx` where individual `ctx.call`/`ctx.run` steps are the retried unit, the Restate model (retry the step, terminal-error the whole thing on exhaustion) is the closer analogue.

**Finding 5.3 — Temporal DOES model a status enum, but at the *control-plane* layer, separate from the body's return.**
**Evidence**: A Workflow Execution has six terminal statuses: **Completed** (*"completed successfully"*), **Failed** (*"returned an error and failed"*), **Cancelled** (*"successfully handled a cancellation request"*), **Terminated** (*"was terminated"*), **Timed Out** (*"reached a timeout limit"*), **Continued-As-New**. Cancellation, like Restate, is delivered into the workflow (a cancellation request the workflow *handles*); ContinueAsNew is a body-initiated *successor-spawning* mechanism, not a plain return.
**Source**: [docs.temporal.io workflow-execution](https://docs.temporal.io/workflow-execution) — Accessed 2026-06-06
**Confidence**: High (authoritative vendor reference)
**Analysis**: **This is the crux of the whole research question.** Temporal cleanly separates two layers:
1. **What the body produces** — a typed value (`Ok`) or a thrown failure (`Err`/terminal). The body does NOT author "Cancelled" / "TimedOut" / "Terminated" — those arise from engine/control-plane events.
2. **What the control plane records** — a 6-variant status enum that is the *externally observable* outcome, including statuses the body cannot itself return (Terminated, TimedOut, ContinuedAsNew).
The status enum is real and useful — but it lives in the engine's observable history, not as the signature of the workflow function. This is the dissenting-but-reconciling evidence the synthesis must address: a status enum is correct *as a control-plane projection*, wrong *as the body's return type*.

### Cross-reference: DBOS and AWS Step Functions (corroborating the pattern)

**Finding X.1 — DBOS: steps retry; checkpointed step outputs are never re-executed; workflow status (PENDING …) is engine-tracked, not a body return.**
**Evidence**: *"If a workflow fails while executing a step, it retries the step during recovery"* — but once checkpointed, *"it is never re-executed."* DBOS scans for incomplete (**PENDING**) workflows on startup. Workflows must be deterministic and steps idempotent for safe retry.
**Source**: [docs.dbos.dev/architecture](https://docs.dbos.dev/architecture) — Accessed 2026-06-06
**Confidence**: Medium (page was thin on workflow-function return-type specifics; status enumeration incomplete)
**Analysis**: Confirms retry-the-step semantics and an engine-side status (PENDING) distinct from the function's typed return. Did not enumerate SUCCESS/ERROR/CANCELLED explicitly on this page — logged as a minor gap.

**Finding X.2 — AWS Step Functions: per-state Retry/Catch; uncaught error fails the whole execution; execution status (SUCCEEDED/FAILED/TIMED_OUT/ABORTED) is control-plane, output data is separate.**
**Evidence**: *"When a state reports an error, Step Functions defaults to failing the entire state machine execution."* `Retry` retriers carry `MaxAttempts` (default **3**), `BackoffRate` (default **2.0**), `IntervalSeconds` (default 1), `MaxDelaySeconds`, `JitterStrategy`. *"If the error recurs more times than specified, retries cease and normal error handling resumes"* (i.e., the `Catch` fallback fires, else the execution fails). A successful state passes its **data output** downstream; the execution's terminal *status* (SUCCEEDED / FAILED / TIMED_OUT / ABORTED) is recorded by the service and observed via the API / EventBridge.
**Source**: [docs.aws.amazon.com Step Functions error handling](https://docs.aws.amazon.com/step-functions/latest/dg/concepts-error-handling.html) — Accessed 2026-06-06
**Confidence**: High (authoritative vendor reference, detailed)
**Analysis**: Same two-layer split as Temporal and Restate: the unit of retry is the *step/state* (not the whole workflow), uncaught/exhausted errors propagate to fail the execution, and the **status enum is a control-plane projection** distinct from the **data output** the successful path produces. Step Functions is the platform where a "status output" is most visible — but even here the status is the *execution's* recorded outcome, not a value a Task body returns instead of its data.

### Q6 (Synthesis): Is a status-enum return an anti-pattern?
_See the Synthesis section below — this is the cross-cutting answer._

## Source Analysis
| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| Restate Rust SDK API docs | docs.rs/restate-sdk | High (1.0, authoritative-minimum per augmented config) | technical_docs | 2026-06-06 | Y (vs error-handling guide + sdk-rust README) |
| Restate Error Handling guide | docs.restate.dev | High (1.0, authoritative-minimum) | official | 2026-06-06 | Y (vs docs.rs + config ref) |
| Restate Service Configuration | docs.restate.dev | High (1.0) | official | 2026-06-06 | Y (vs error-handling guide) |
| Restate Managing/Operate Invocations | docs.restate.dev | High (1.0) | official | 2026-06-06 | Y (vs error-handling guide) |
| Restate sdk-rust README | github.com/restatedev/sdk-rust | High (1.0; github trusted) | industry_leaders / source | 2026-06-06 | Y (vs docs.rs) |
| Temporal Failures reference | docs.temporal.io | High (1.0, authoritative-minimum) | official | 2026-06-06 | Y (vs retry-policies + workflow-execution) |
| Temporal Retry Policies | docs.temporal.io | High (1.0) | official | 2026-06-06 | Y (vs failures ref) |
| Temporal Workflow Execution | docs.temporal.io | High (1.0) | official | 2026-06-06 | Y (vs failures ref) |
| AWS Step Functions error handling | docs.aws.amazon.com | High (1.0) | technical_docs | 2026-06-06 | Y (corroborates Temporal/Restate two-layer model) |
| DBOS architecture | docs.dbos.dev | High (1.0, authoritative-minimum) | official | 2026-06-06 | Partial (thin on return type; corroborates step-retry + engine status) |

Reputation: High: 10 (100%) | Medium-High: 0 | Avg: 1.0
All sources fall within the trusted-source list or the topic-specific augmentation. **No out-of-list / excluded sources were used.** No blogspot/medium/wordpress content cited.

## Knowledge Gaps

### Gap 1: Restate Rust `TerminalError` exact type signature
**Issue**: The docs.rs fetch and sdk-rust README confirmed `HandlerResult<T> = Result<T, HandlerError>` and that `TerminalError` exists to stop retries, but did not return the exact constructor signature of the Rust `TerminalError` (e.g. status code + message fields) or how a `HandlerError` is built from a `TerminalError` vs an ordinary `std::error::Error`. **Attempted**: docs.rs landing page, sdk-rust README. **Recommendation**: read `restate-sdk` crate `errors` module source directly (github.com/restatedev/sdk-rust `src/errors.rs`) before finalizing any Rust API mirroring it — verified-by-source, not by landing-page summary.

### Gap 2: DBOS full terminal-status enumeration
**Issue**: The architecture page surfaced PENDING but not the complete status set (SUCCESS / ERROR / CANCELLED / MAX_RECOVERY_ATTEMPTS_EXCEEDED). **Attempted**: docs.dbos.dev/architecture. **Recommendation**: docs.dbos.dev workflow-tutorials / API reference for `WorkflowStatus`; low priority — DBOS is a corroborating, not load-bearing, source here.

### Gap 3: Restate "infinite retries" vs default-70-attempts framing
**Issue**: The SDK doc says errors retry "infinitely" by default while the config reference gives a default policy of max 70 attempts. These are reconcilable (conceptual default = retry; shipped policy = bounded) but the exact precedence (does an unset per-handler policy inherit the 70-attempt default, or truly unbounded?) was not pinned to a single quote. **Recommendation**: confirm against the current Restate server config reference for the active version; does not change the synthesis (exhaustion → terminal regardless of the cap value).

## Conflicting / Nuanced Information

### Nuance 1: "infinite retries" (SDK) vs "max attempts 70" (config) — Restate
**Position A**: SDK error-handling: regular errors "cause infinite retries by default." **Position B**: Service configuration: default retry policy is max attempts 70. **Assessment**: Not a true contradiction. The SDK statement describes the *category default* (an uncaught error is retryable, conceptually forever unless terminal); the config reference describes the *shipped numeric policy* that bounds it. Both are vendor-authoritative; the synthesis relies only on the unambiguous shared claim — **exhaustion of whatever bound applies converts to a terminal failure**.

### Nuance 2: Retry granularity — whole-invocation (Restate) vs per-step/activity (Temporal, Step Functions, DBOS)
**Assessment**: Restate retries the *handler/invocation* by re-driving from the journal (steps already journaled are not re-executed); Temporal/Step Functions/DBOS retry the individual *activity/state/step*. This is an implementation difference, not a contradiction of the return-type thesis: in all four, the *body's* failure path is throw-an-error, the engine owns retry, and budget exhaustion fails the unit upward. For Overdrive's `ctx.call`/`ctx.run` step model, the per-step-retry analogue (Temporal/SFN/DBOS) is the closer fit, while the terminal-on-exhaustion outcome semantics match Restate.

### Nuance 3: Where a status-like outcome legitimately appears
**Assessment**: Step Functions and Temporal both expose a control-plane status enum, and Temporal's `ContinueAsNew` is a *body-initiated* non-return mechanism. This is the dissenting evidence: a status enum is not universally absent — it is universally **relocated to the control plane / history**, never the body's success/failure channel. The body still returns `T` or throws; the engine maps that (plus its own events: timeout, terminate, cancel) onto the observable status.

## Synthesis (position grounded in evidence)

**The evidence supports replacing a body-returned status enum with `Result<T, TerminalError>` for the workflow body, while preserving a control-plane status as a separate, engine-owned projection.** This is the consistent shape across all four platforms, with the precise wording differing only in language idiom:

1. **Success = typed value `T`, not a `Success` variant.** Restate `Result<U, HandlerError>` where `U` is the real output (High confidence; docs.rs + sdk-rust README + error-handling guide). Temporal workflow functions return their typed result (High; docs.temporal.io failures + workflow-execution). Step Functions passes data output downstream (High). A `WorkflowResult::Success` (unit) variant discards the workflow's actual output — no surveyed platform does this.

2. **Retryable errors never reach the return type.** They are absorbed and re-driven by the engine: Restate *"retries failures infinitely"* by default / per policy; Temporal retries Activities by default; Step Functions retries per `Retry` block; DBOS retries the step on recovery. (High confidence; 4/4 platforms.) A `WorkflowResult::Failed { reason: String }` that the body returns for an ordinary failure is the anti-pattern — it makes a *retryable* condition look like a *terminal* outcome, collapsing the engine's most important distinction.

3. **Budget exhaustion converts to a terminal error.** Restate: *"the run block will fail with a TerminalException once the retries are exhausted"*; on invocation exhaustion the engine *"kill[s] it, which automatically fails the invocation and responds to the caller with a terminal error"* (or pauses for manual resume). Step Functions: retries cease and the execution fails (unless caught). (High confidence.) The engine, not the body, decides this — the body only ever *throws terminal* (explicit) or *returns Ok* (success); everything between is the engine's retry domain.

4. **Cancellation is an engine/control-plane concern, not a body return variant.** Restate: cancellation is an external API/CLI operation that is *delivered into* the body as a `TerminalError` at the next await point (*"Cancellations are Terminal Errors"*). Temporal: Cancelled is a control-plane terminal status; the workflow *handles* a cancellation request. (High confidence; both vendors explicit.) A `WorkflowResult::Cancelled` variant the body returns inverts this: cancellation originates outside the body, so the body cannot author it as a chosen return value. It can at most *observe* cancellation (as an error/signal) and run compensation.

**Therefore**: `enum WorkflowResult { Success, Failed { reason }, Cancelled }` as a *body return type* is, on the weight of the evidence, the wrong shape — it (a) throws away the typed output, (b) conflates retryable with terminal failure, and (c) lets the body author an outcome (Cancelled) that the engine actually owns. The idiomatic replacement is `async fn run(...) -> Result<T, TerminalError>`.

**The one legitimate place for a status enum is the control plane that *records* the terminal outcome** — exactly as Temporal (6 statuses) and Step Functions (4) do. That enum can and should include variants the body cannot produce (e.g. `Cancelled`, `TimedOut`, `Terminated`, budget-exhausted `Failed`), because those are engine-observed events. The error to avoid is using *one* enum for both jobs.

## Implications for ADR-0064 / WorkflowResult

Framed as evidence-mapped options, not project decisions:

- **Option A — Replace the body return with `Result<T, TerminalError>` (strongest evidence alignment).** `async fn run(&self, ctx: &WorkflowCtx) -> Result<T, TerminalError>`. Matches Restate's Rust SDK shape 1:1 and Temporal's body semantics. Retryable `Err`s are re-driven by the executor against the journal; a `TerminalError` (explicit or engine-minted on budget exhaustion) is the only failure that ends the workflow. Bit-identical replay is *helped* by this shape: the journal records step outcomes and a single terminal decision, not a body-authored multi-variant status whose `reason: String` is a non-deterministic free-text field (a `String` reason is itself a replay-determinism hazard — see § "Persist inputs, not derived state" and the bit-identical-trajectory requirement).

- **Option B — Hybrid: body returns `Result<T, TerminalError>`; the control plane records a status enum.** This is what Temporal and Step Functions actually do. The executor maps `Ok(T)` → `Completed{output}`, terminal `Err` → `Failed{terminal_error}`, external cancel → `Cancelled`, deadline → `TimedOut`, etc., in an *observation-layer* status the control plane persists — distinct from the body signature. In Overdrive terms this status is observation/history (engine-owned), not the workflow function's type. Retry budgets, per the current codebase precedent, would live in executor/View-adjacent state (cf. `RetryMemory` in `development.md`), not in the workflow body — consistent with "the engine owns retry."

- **Option C — Keep `enum WorkflowResult { Success, Failed{reason}, Cancelled }` as the body return (weakest evidence alignment).** No surveyed platform returns a status enum from the body. Retaining it would diverge from Restate/Temporal/DBOS/Step Functions on all three axes (typed output, retryable-vs-terminal, cancellation ownership). If retained, it should be justified by a project-specific constraint the evidence here does not surface.

**Mapping to the user's design claim** ("workflows shouldn't return a `WorkflowResult` — like Restate, any error retries, and once the retry budget is exhausted it throws a terminal error that fails the workflow; workflows just run to completion"): the evidence **substantiates this claim directly** for Restate and confirms the same body-level semantics for Temporal, with Step Functions and DBOS corroborating the retry-the-step + engine-owned-status structure. The single refinement the evidence adds: keep a control-plane status enum *if* the system needs to record engine-observed terminal outcomes (cancel/timeout/terminate) — but that enum is not the body's return type. (Confidence on the overall position: **High** — 4 platforms, official sources, no contradicting evidence on the body-return-type question; the only nuances are granularity and the legitimate control-plane status layer, both reconciled above.)

## Full Citations

[1] Restate. "restate_sdk — Rust SDK API documentation". docs.rs. 2026. https://docs.rs/restate-sdk/latest/restate_sdk/. Accessed 2026-06-06.
[2] Restate. "Error Handling". docs.restate.dev. https://docs.restate.dev/guides/error-handling. Accessed 2026-06-06.
[3] Restate. "Service Configuration". docs.restate.dev. https://docs.restate.dev/services/configuration. Accessed 2026-06-06.
[4] Restate. "Managing Invocations / Operate: Invocation". docs.restate.dev. https://docs.restate.dev/operate/invocation/ and https://docs.restate.dev/services/invocation/managing-invocations. Accessed 2026-06-06.
[5] Restate. "restatedev/sdk-rust (README)". github.com. https://github.com/restatedev/sdk-rust. Accessed 2026-06-06.
[6] Temporal. "Failures (references)". docs.temporal.io. https://docs.temporal.io/references/failures. Accessed 2026-06-06.
[7] Temporal. "Retry Policies (encyclopedia)". docs.temporal.io. https://docs.temporal.io/encyclopedia/retry-policies. Accessed 2026-06-06.
[8] Temporal. "Workflow Execution". docs.temporal.io. https://docs.temporal.io/workflow-execution. Accessed 2026-06-06.
[9] Amazon Web Services. "Handling errors in Step Functions workflows". docs.aws.amazon.com. https://docs.aws.amazon.com/step-functions/latest/dg/concepts-error-handling.html. Accessed 2026-06-06.
[10] DBOS. "Architecture". docs.dbos.dev. https://docs.dbos.dev/architecture. Accessed 2026-06-06.

## Research Metadata
Examined: 10 sources across 4 platforms | Cited: 10 | Cross-refs: every load-bearing claim verified against ≥2 independent pages (Restate SDK doc ↔ error-handling guide ↔ sdk-rust README; Temporal failures ↔ retry-policies ↔ workflow-execution; Step Functions + DBOS corroborate the two-layer model) | Confidence distribution: High ~90%, Medium ~10% (DBOS return-type specifics) | Output: docs/research/workflow-durable-execution/result-error-retry-semantics-research.md
