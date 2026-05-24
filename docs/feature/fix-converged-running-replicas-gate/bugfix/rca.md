# Bug fix RCA — Streaming `check_terminal` ignores `replicas_desired`

Tracking issue: [overdrive-sh/overdrive#140](https://github.com/overdrive-sh/overdrive/issues/140)

The issue body is the high-fidelity RCA. This document is the in-tree
audit confirming the issue's claims against HEAD, surfacing the one
material nuance the issue body's framing does not capture (the
affected lane is **Service**, not Job), and pinning the threading
shape the crafter will implement in Phase 3.

## 1. Two emission sites in `streaming.rs` — both confirmed

The two single-Running-row shortcuts that need the replicas gate live
in `crates/overdrive-control-plane/src/streaming.rs`:

**(a) Live broadcast path — `check_terminal`** at `streaming.rs:362`.
The Running-detection branch reads at lines 384–397:

```
if matches!(event.to, AllocStateWire::Running) {
    if let Ok(rows) = obs.alloc_status_rows().await {
        let has_running = rows.iter().any(|r| {
            r.workload_id == *workload_id
                && r.state == overdrive_core::traits::observation_store::AllocState::Running
        });
        if has_running {
            return Some(SubmitEvent::ConvergedRunning {
                alloc_id: event.alloc_id.to_string(),
                started_at: event.at.clone(),
            });
        }
    }
}
```

The `has_running = rows.iter().any(...)` is the bug — emits
`ConvergedRunning` the moment **any** Running row exists for the
workload, regardless of how many are desired.

**(b) Snapshot / lagged-recovery path — `lagged_recover`** at
`streaming.rs:444`. The non-terminal projection at lines 477–486:

```
match latest.state {
    AllocState::Running => Some(SubmitEvent::ConvergedRunning {
        alloc_id: latest.alloc_id.to_string(),
        started_at: format!(
            "{}@{}",
            latest.updated_at.counter,
            latest.updated_at.writer.as_str()
        ),
    }),
    _ => None,
}
```

Same shape — `latest.state == Running` emits `ConvergedRunning` with
no replica-count comparison. `lagged_recover` is called from
**two** sites in `build_stream` per `streaming.rs:208` (pre-subscribe
window bridge) and `streaming.rs:281` (`broadcast::error::RecvError::
Lagged(_)` recovery). Both inherit the bug.

Both functions also carry a pre-existing structured docstring TODO
naming this issue:

> `streaming.rs:358` — `TODO(#140): gate ConvergedRunning on
> running_count >= replicas_desired once a multi-replica workload
> lands. Hydrate replicas_desired once at stream start rather than
> reading the IntentStore per broadcast event.`

The docstring's "hydrate once at stream start" recommendation matches
the issue body's preferred shape (issue § "Scope" 2).

## 2. Audit of every `ConvergedRunning` emission site in the crate

Repo-wide `rg ConvergedRunning` against
`crates/overdrive-control-plane/src/` returns ten matches plus tests:

| Site | File:line | Classification |
|---|---|---|
| `SubmitEvent::ConvergedRunning` variant decl | `api.rs:670` | **type definition** — the wire variant itself (legacy flat enum) |
| `check_terminal` emission | `streaming.rs:391` | **(a) live/broadcast path** — affected |
| `lagged_recover` emission | `streaming.rs:478` | **(b) snapshot / recover path** — affected |
| `JobSubmitEvent` (no Converged) docstrings | `streaming.rs:925, 945, 1003` | **doc-only** — ADR-0047 rationale |
| `ServiceSubmitEvent::ConvergedRunning` variant decl | `streaming.rs:1020` | **type definition** — typed-sibling enum per ADR-0047 |
| CLI docstrings | `api.rs:451, 603` | **doc-only** — exit-code mapping reference |
| Handler comment | `handlers.rs:444` | **doc-only** — explaining `build_workload_stream` divergence |

Test surfaces (`tests/acceptance/submit_event_serialization.rs` lines
172, 345, 373; `tests/acceptance/streaming_submit.rs` lines 284, 408,
1004) are **(c) test fixture / expectation** classifications.

**Material finding: `ServiceSubmitEvent::ConvergedRunning`
(`streaming.rs:1020`) is declared but never emitted anywhere in
`src/`.** A repo-wide grep finds zero `ServiceSubmitEvent::Converged`
constructor expressions; the variant is a future-slice scaffold per
ADR-0047 §3 [D7]. No Phase 3 work needs to land on it as part of
this fix — the active emission surface today is the legacy
`SubmitEvent::ConvergedRunning` flat-enum path only.

**Two real-code emission sites, both in `streaming.rs`.** No third
real emission surface exists.

## 3. `Job::replicas` and `ServiceV1::replicas` — both `NonZeroU32`, both validated

Issue body cites `Job::replicas: NonZeroU32` at
`crates/overdrive-core/src/aggregate/mod.rs:94`. Current tree:

- `JobV1.replicas: NonZeroU32` at `aggregate/mod.rs:215` (the issue's
  cited `:94` is stale post-ADR-0050 line drift; field is on the
  `JobV1` aggregate as expected).
- `ServiceV1.replicas: NonZeroU32` at `aggregate/mod.rs:393`.
- `JobSpecInput.replicas: u32` (wire-shape, line 738) validated by
  `JobV1::from_submit` at line 239–242: `NonZeroU32::new(replicas)
  .ok_or_else(|| AggregateError::Validation { ... })`.
- `ServiceV1::from_submit` at line 451 (`crate::api::submit::
  ServiceSpecInput { id, replicas, ... }`) carries the equivalent
  non-zero validation.

The streaming side's bug is symmetric for both kinds in principle —
but **only Service is structurally reachable** today (see § 4 below).

## 4. The Service-vs-Job distinction — what the issue's framing under-emphasises

Issue body uses Job as the running example and cites
`Action::StartAllocation` (which fires for both kinds). The reality
in the current tree:

- The submit handler at `handlers.rs:446-479` branches by
  `WorkloadKind` and routes each kind to a distinct streaming
  builder:
  - **Job kind** → `crate::streaming::build_workload_stream(...)`
    (`handlers.rs:454`). Emits the typed-sibling `JobSubmitEvent`
    enum which **has no `ConvergedRunning` variant** by design
    (per ADR-0047 §3 [D7] — Jobs are run-to-completion; "Running"
    is informational, not terminal). The handler comment at
    `handlers.rs:444` says this verbatim: "no `ConvergedRunning`
    variant — RCA root causes B+C+D structurally unreachable".
  - **Service / Schedule kind** → `crate::streaming::build_stream(...)`
    (`handlers.rs:476`). This is the legacy flat `SubmitEvent` path
    that emits `SubmitEvent::ConvergedRunning` via `check_terminal`
    / `lagged_recover` — the buggy surface.

**Conclusion: the bug is on the Service streaming lane.** Job
streaming was already moved off `ConvergedRunning` semantics by
ADR-0047. The fix scope is the legacy flat-`SubmitEvent` path the
Service kind still rides. Schedule kind also rides `build_stream` but
its submit handler rejects with HTTP 400 today (`handlers.rs:258-264`),
so the Service lane is the sole live caller of `build_stream` that
can actually reach the Running path.

This distinction does not change the fix shape (gate
`ConvergedRunning` on `running_count >= replicas_desired`) — it just
narrows the regression-test surface to Service-kind submits.

## 5. Reconciler gate — confirmation only

`Action::StartAllocation` carries the docstring at
`reconciler.rs:529-531`:

> Start a fresh allocation for a job. Emitted by the
> `WorkloadLifecycle` reconciler when `desired.replicas >
> actual.replicas_running`.

The reconciler is the gate that decides when to mint a new alloc; the
streaming side is purely observational. The issue's framing is
correct: **the gap is exclusively on the streaming surface**. The
fix changes streaming only; no reconciler change is needed.

## 6. Threading shape — recommended (A): pass `replicas_desired` from the handler

### Survey of existing context

- The submit handler at `handlers.rs:215` (`submit_workload`) calls
  `Job::from_submit` / `ServiceV1::from_submit` at lines 243 / 251,
  which means the **validated `Job` / `ServiceV1` aggregate is in
  scope** — including the `replicas: NonZeroU32` field — at the
  exact point where `build_stream` is invoked (line 476).
- `AppState.store: Arc<LocalIntentStore>` (`lib.rs:123`) is also
  reachable from the streaming task, so option (B) — one-shot
  hydrate inside the streaming task — is mechanically possible
  (the handler uses the same surface at `handlers.rs:401` and
  `handlers.rs:654` to read intent bytes).
- `check_terminal`'s signature `(obs, workload_id, event)` and
  `lagged_recover`'s `(obs, workload_id)` both flow through
  `build_stream(state, workload_id, accepted)` at `streaming.rs:158`
  — no context-struct refactor is required; `replicas_desired` can
  be added as a new positional / named parameter to both helpers and
  to `build_stream`.

### Recommendation: option (A) — pass through from the handler

The handler ALREADY has the validated aggregate with `replicas:
NonZeroU32` in scope BEFORE it calls `build_stream`. Passing it
through is one parameter on three function signatures and zero new
I/O. Option (B) — one-shot IntentStore read inside the streaming
task — adds:

1. A redundant read of bytes the handler just produced.
2. A decode-via-`Job::from_store_bytes` path that can fail (UI-03
   amendment in development.md adds a `redb_path` and operator-
   facing error message), introducing a new failure mode the
   streaming task currently does not have to handle.
3. A coupling between the streaming task and the codec module that
   was deliberately kept out of the streaming code path.

Option (A) avoids all three. The issue body's preference language
("hydrate `replicas_desired` once at stream start rather than reading
the IntentStore per broadcast event") was specifically warning
against the *per-event* read, not against the threading approach —
both (A) and (B) satisfy that constraint, and (A) is structurally
cheaper.

### Concrete signatures

```
// streaming.rs

pub fn build_stream(
    state: AppState,
    workload_id: WorkloadId,
    accepted: SubmitEvent,
    replicas_desired: NonZeroU32,    // NEW
) -> impl Stream<...>

async fn check_terminal(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
    event: &LifecycleEvent,
    replicas_desired: NonZeroU32,    // NEW
) -> Option<SubmitEvent>

async fn lagged_recover(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
    replicas_desired: NonZeroU32,    // NEW
) -> Option<SubmitEvent>
```

`NonZeroU32` (not `u32`) per the aggregate's declaration — the type
carries the "must be >= 1" invariant from the validating constructor
through to the streaming layer with no defensive re-check needed.

### Handler call site

At `handlers.rs:476` (Service branch — the actively-buggy lane), the
handler extracts `replicas_desired` from the validated aggregate
already in scope:

```
WorkloadKind::Service | WorkloadKind::Schedule => {
    let replicas_desired = match &intent {
        WorkloadIntent::Service(s) => s.replicas,
        // Schedule never reaches build_stream — submit rejects at
        // line 258. Pass a sentinel or refactor to JobV1 if/when
        // Schedule streaming lands. For Phase 1 the unreachable
        // arm can NonZeroU32::new(1).unwrap() with a comment, or
        // we can structurally restrict build_stream to Service-only.
        _ => unreachable!("Schedule kind rejected at submit; Job uses build_workload_stream"),
    };
    let accepted = crate::streaming::build_accepted(...);
    let stream = crate::streaming::build_stream(
        state.clone(),
        workload_id.clone(),
        accepted,
        replicas_desired,
    );
    ...
}
```

### `check_terminal` and `lagged_recover` after the fix

The single-Running-row shortcut becomes a count comparison:

```
async fn check_terminal(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
    event: &LifecycleEvent,
    replicas_desired: NonZeroU32,
) -> Option<SubmitEvent> {
    if let Some(cond) = &event.terminal {
        return Some(submit_event_from_terminal(cond, event));
    }
    if matches!(event.to, AllocStateWire::Running) {
        if let Ok(rows) = obs.alloc_status_rows().await {
            let running_count: u32 = rows.iter()
                .filter(|r| r.workload_id == *workload_id
                    && r.state == AllocState::Running)
                .count()
                .try_into()
                .unwrap_or(u32::MAX);
            if running_count >= replicas_desired.get() {
                return Some(SubmitEvent::ConvergedRunning {
                    alloc_id: event.alloc_id.to_string(),
                    started_at: event.at.clone(),
                });
            }
        }
    }
    None
}
```

`lagged_recover` is more interesting — `latest` is a single row, not
the full slice. The fix swaps single-row inspection for an aggregate
count over all rows belonging to `workload_id`:

```
async fn lagged_recover(
    obs: &dyn ObservationStore,
    workload_id: &WorkloadId,
    replicas_desired: NonZeroU32,
) -> Option<SubmitEvent> {
    let rows = obs.alloc_status_rows().await.ok()?;
    let job_rows: Vec<_> = rows.into_iter()
        .filter(|r| r.workload_id == *workload_id)
        .collect();
    let latest = job_rows.iter()
        .max_by_key(|r| r.updated_at.counter)?;

    if let Some(cond) = &latest.terminal {
        // ... terminal projection unchanged ...
    }

    // Non-terminal — count Running rows; emit only when ≥ desired.
    let running_count: u32 = job_rows.iter()
        .filter(|r| r.state == AllocState::Running)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    if running_count >= replicas_desired.get() {
        // pick the most-recently-updated Running row as the
        // alloc_id seed for the wire event
        let running = job_rows.iter()
            .filter(|r| r.state == AllocState::Running)
            .max_by_key(|r| r.updated_at.counter)?;
        return Some(SubmitEvent::ConvergedRunning {
            alloc_id: running.alloc_id.to_string(),
            started_at: format!(
                "{}@{}",
                running.updated_at.counter,
                running.updated_at.writer.as_str()
            ),
        });
    }
    None
}
```

Note the `lagged_recover` change picks the most-recent Running row's
metadata for the wire event — `latest` may be a transition that is
itself not Running, even though the count of Running siblings has
already met the threshold. The single-row shortcut today happens to
work because there is only one alloc; under multi-replica the
selection has to be explicit.

## 7. Restart-budget interaction — design question for the user

`check_terminal`'s terminal-projection branch fires unconditionally
when `event.terminal.is_some()` (line 372-374), BEFORE the
running-count gate. With `replicas_desired > 1`, this means:

- If alloc 1 reaches Running and alloc 2 reaches Running → wait for
  count ≥ desired → emit `ConvergedRunning` (new behaviour, correct).
- If alloc 1 reaches Running and alloc 2 fires
  `TerminalCondition::BackoffExhausted` → `check_terminal` projects
  the BackoffExhausted into `SubmitEvent::ConvergedFailed` and ends
  the stream.

The pre-existing "BackoffExhausted fires on first failure" behaviour
preserves: any single alloc's terminal condition closes the stream,
regardless of how many other allocs are still pending or running.
That seems semantically correct — a Service with `replicas: 3` whose
second replica has exhausted its restart budget is failing the
deployment overall, and the operator wants to know now, not after
the timeout cap fires.

**Open question for the user (§5 of the dispatch prompt):** confirm
this semantics is desired. Specifically:

- **Q1:** With `replicas: 3`, if 2 are Running and 1 hits
  `BackoffExhausted`, should the stream emit `ConvergedFailed` (fail
  fast — current behaviour with the trivial fix) or wait until ALL
  in-flight allocs reach a terminal state and report aggregate
  outcome (more nuanced — out of scope for this issue)?
  - **Recommendation: keep current "fail fast on first terminal"
    semantics.** It's the conservative choice, matches what an
    operator running `submit --wait` actually wants
    (failure → exit non-zero immediately), and is purely additive
    behavior. The nuanced aggregate-outcome design is a separate
    concern that the issue body says is out of scope.

- **Q2:** Stopped (operator-initiated) vs Stopped (reconciler-
  initiated) — same answer? `TerminalCondition::Stopped { by }`
  projects to `SubmitEvent::ConvergedStopped` today on the first
  Stopped row. With multi-replica, the same fail-fast semantics
  apply by default. Confirm.

The recommended fix shape preserves the existing terminal-projection
behaviour unchanged — the Running gate only applies to the *success*
path. If the user wants aggregate-outcome semantics instead, that's
a deeper streaming-loop restructure and warrants its own issue.

## 8. Regression test outline

### Location

`crates/overdrive-control-plane/tests/acceptance/streaming_submit.rs`
(append a new `#[tokio::test]` alongside `s_cp_01_*`). The existing
file provides the harness — `build_app_state`, `build_router`,
`emit_lifecycle`, `make_lifecycle_event`, `body_ndjson_lines`,
`SimClock` — and they all already operate on the streaming flow.

### Why this file rather than a new one

- The existing `s_cp_01_streaming_lane_emits_*` test at line 287
  uses the Job-kind `payments_spec()`, which routes through
  `build_workload_stream`. Adding a Service-kind sibling here keeps
  related streaming acceptance scenarios co-located.
- The harness's `emit_lifecycle` helper at line 181 is the right
  altitude — directly drives `state.lifecycle_events.send(event)`
  to inject broadcast events without going through the reconciler
  or driver.

### Fixture shape

A new helper `payments_service_spec()` returning
`ServiceSpecInput { replicas: 2, ... }` (analogous to the existing
`payments_spec` but using the Service variant) plus a new
`build_submit_request` overload that routes through
`SubmitSpecInput::Service(...)`.

### RED-assertion sequence (the load-bearing test logic)

```
1. Submit a Service spec with replicas == 2 via the streaming lane
   (Accept: application/x-ndjson). Spawn the request in a tokio task.
2. Wait until state.lifecycle_events.receiver_count() >= 1 (handler
   has subscribed).
3. Inject one allocation reaching Running:
   - Write an AllocStatusRow to obs with workload_id = "payments",
     alloc_id = "alloc-payments-0", state = Running.
   - Emit LifecycleEvent { from: Pending, to: Running,
     alloc_id: "alloc-payments-0", workload_id: "payments",
     terminal: None }.
4. Yield enough times (or advance SimClock by a small ε) for the
   streaming task to process the event.
5. ASSERT (RED — fails on current code):
   The stream has NOT emitted ConvergedRunning yet. Read available
   lines from the body and assert no line has kind == "converged_running".
   Equivalently: the request_task has not yet completed (still polling
   for more events).
6. Inject the second allocation reaching Running (alloc-payments-1
   row + lifecycle event).
7. ASSERT (GREEN after fix): the stream NOW emits ConvergedRunning
   and the request_task completes within a short timeout.
```

### Test name

Suggested: `s_cp_NN_streaming_lane_does_not_emit_converged_running_until_running_count_meets_replicas_desired`
(or shorter — the existing file uses long descriptive names).

### Missing primitives

Looking at the harness:

- `build_app_state(tmp, clock)` already gives a working `AppState`
  with a `SimClock`, a `SimDriver`, and a real `LocalObservationStore`.
- `emit_lifecycle(state, event)` already drives broadcast injection.
- `obs.write_alloc_status_row(...)` is reachable through
  `state.obs.as_ref()`. The existing Service-kind acceptance tests
  (`service_workload_emits_start_allocation.rs`) already drive
  `obs.write_alloc_status(...)` directly — there's a working
  reference pattern.

**No new harness primitive needed.** The test can be written entirely
against existing fixture surface.

### RED-scaffold shape

Per `.claude/rules/testing.md` § "RED scaffolds and intentionally-
failing commits", a Phase 3 crafter landing the RED test before the
fix must mark it `#[should_panic(expected = "RED scaffold")]` with a
panic body naming the scenario, then drop the attribute and replace
the panic with the assertion sequence in the GREEN-fix commit.

Alternatively — and more cleanly given this is a one-commit bugfix
landing test + fix together — the test can land GREEN against the
fixed implementation in a single commit, with the issue body's
description as the RED-state evidence.

## 9. Risk assessment

Tight scope. Three signature changes (`build_stream`, `check_terminal`,
`lagged_recover`), one handler call-site change
(`handlers.rs:476`), one new test. No reconciler change. No
observation-store change. No wire-protocol change (`SubmitEvent`
variants unchanged — only the *when* they fire changes).

The pre-subscribe window referenced in the streaming docstring at
`streaming.rs:195-219` interacts with `lagged_recover`: at stream
start, before the broadcast subscription is live, `lagged_recover`
is called to bridge events that may have fired during the
`put_if_absent` → `broker.enqueue` window. The hydration of
`replicas_desired` happens BEFORE this call (in `build_stream`'s
parameter list, passed from the handler), so the snapshot's
running-count comparison has the correct desired value from the
first instruction of the streaming task. No ordering hazard.

Replica-count cannot change mid-stream in Phase 1 — `Job` and
`ServiceV1` are immutable aggregates once persisted (re-submit with
a different spec hash is `PutOutcome::Conflict` at
`handlers.rs:432-434`). The whitepaper §15 rolling-deployment concern
is explicitly Phase 2; the issue body and § 5 of this audit confirm
it's out of scope. So `replicas_desired` hydrated once at stream
start IS the value for the stream's entire lifetime — no staleness
concern under Phase 1 invariants.

`ServiceSubmitEvent::ConvergedRunning` at `streaming.rs:1020` is
declared but never emitted (§ 2 finding). If Phase 3 work introduces
its emission path, the same replicas gate logic from this fix should
be applied there too — but that's not in scope for this fix, since
no emission site exists yet.

## 10. Open questions for the user

**Q1 (load-bearing for fix shape):** With `replicas: 3`, if 2 are
Running and 1 hits `TerminalCondition::BackoffExhausted`, should the
stream:

- (a) Fail-fast — emit `ConvergedFailed { BackoffExhausted }` on the
  first terminal event, regardless of replica progress. **(Current
  behaviour, preserved by the trivial fix in § 6.)**
- (b) Aggregate — wait until all 3 allocs reach a terminal state and
  emit a composite outcome (`ConvergedRunning` if all eventually
  reached Running; `ConvergedFailed` only if quorum cannot be met).

Recommendation: (a). It matches operator expectations and the
existing terminal-projection semantics. (b) is a deeper redesign and
the issue body explicitly defers it.

**Q2 (informational — defaults to Q1's answer):** Same question for
`TerminalCondition::Stopped { by }` and
`TerminalCondition::Custom { ... }`. Recommendation: same as Q1.

**Q3 (sanity check):** Confirm the Service streaming lane is the only
affected surface — Job kind has no `ConvergedRunning` variant
structurally (per ADR-0047 §3 [D7]), and `ServiceSubmitEvent::
ConvergedRunning` has zero current emission sites. The fix scope is
the legacy flat `SubmitEvent::ConvergedRunning` only. (This is § 4
of the audit, restated as a confirmation question.)
