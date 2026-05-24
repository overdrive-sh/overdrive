# fix-converged-running-replicas-gate — Feature Evolution

**Feature ID**: fix-converged-running-replicas-gate
**Type**: Bug fix (`/nw-bugfix` → `/nw-deliver`)
**Branch**: `marcus-sa/converged-running-gate`
**Date**: 2026-05-24
**Issue**: [#140](https://github.com/overdrive-sh/overdrive/issues/140) — Streaming `check_terminal`: respect `replicas_desired` before emitting `ConvergedRunning`
**PR**: [#188](https://github.com/overdrive-sh/overdrive/pull/188)
**Commits**:
- `28956fc7` — `fix(streaming): gate ConvergedRunning on running_count >= replicas_desired`
- `d23fce6d` — `chore(nwave): record DES log and adversarial review for #140 bugfix`

**Status**: Delivered.

---

## Symptom

`crates/overdrive-control-plane/src/streaming.rs::check_terminal` (and
its snapshot-path twin `lagged_recover`) emitted
`SubmitEvent::ConvergedRunning` the moment **any** `state == Running`
row existed for the workload, regardless of how many replicas were
desired. The function docstring already carried a `TODO(#140)`
acknowledging this as a Phase 1 walking-skeleton concession.

Phase 1 has `replicas == 1` for every workload spec, so the defect
had not yet manifested in production. Once a Service spec carries
`replicas > 1`, the streaming surface would report
`ConvergedRunning` after the FIRST allocation reaches Running while
the reconciler is still spinning up the rest. Operator tooling
(`overdrive job submit --wait`, the CLI streaming path) would
prematurely close the stream and report success on a job that has
not yet converged.

The reconciler itself was already replica-aware
(`Action::StartAllocation` is gated on
`desired.replicas > actual.replicas_running` per
`crates/overdrive-core/src/reconciler.rs:495-497`). The gap was
exclusively on the streaming convergence-detection side.

## Root cause

Single-row shortcut in two adjacent code paths:

- **Live broadcast path** (`check_terminal` at `streaming.rs:362`):
  `let has_running = rows.iter().any(|r| r.workload_id == *workload_id
  && r.state == Running)` — any single Running row triggered
  `ConvergedRunning` emission.
- **Snapshot / lagged-recovery path** (`lagged_recover` at
  `streaming.rs:444`): `match latest.state { AllocState::Running =>
  ... }` — the LWW-winner row's state alone gated emission.

Both inherited the walking-skeleton assumption (`replicas == 1`)
without a structural defense against the spec growing.

A material clarification surfaced during the RCA audit that the issue
body did not fully capture: the bug lives **exclusively on the
Service streaming lane**. Job kind was already moved off
`ConvergedRunning` semantics by ADR-0047 §3 [D7] — its typed-sibling
`JobSubmitEvent` enum has no `ConvergedRunning` variant ("Running" is
informational, not terminal, for run-to-completion Jobs). Schedule
kind rejects at the submit boundary (HTTP 400). So the
legacy-flat-`SubmitEvent` path on the Service lane is the only live
caller of the buggy code.

## Fix

Three signature additions, two body refactors, one handler call-site
extraction, one new acceptance test — landed in a single cohesive
commit (`28956fc7`).

1. **`build_stream` / `check_terminal` / `lagged_recover` signatures**
   (`crates/overdrive-control-plane/src/streaming.rs`) — each gains a
   new `replicas_desired: NonZeroU32` parameter, threaded into the
   streaming task closure (`NonZeroU32: Copy`). The type carries the
   "must be ≥ 1" invariant from the validating `ServiceV1::from_submit`
   constructor through to the streaming layer with no defensive
   re-check needed.

2. **`check_terminal` Running-detection body** —
   `let has_running = rows.iter().any(...)` replaced with:

   ```rust
   let running_count: u32 = rows.iter()
       .filter(|r| r.workload_id == *workload_id
           && r.state == AllocState::Running)
       .count()
       .try_into()
       .unwrap_or(u32::MAX);
   if running_count >= replicas_desired.get() {
       return Some(SubmitEvent::ConvergedRunning { ... });
   }
   ```

   The `unwrap_or(u32::MAX)` graceful-degrade handles the
   never-reachable >4B-row case as a sentinel that always satisfies
   `>= replicas_desired`, rather than panicking. Terminal-projection
   branch unchanged.

3. **`lagged_recover` non-terminal body** — single-row inspection
   replaced with aggregate count over the workload's row subset, plus
   explicit most-recently-updated Running row selection for the wire
   event:

   ```rust
   let job_rows: Vec<_> = rows.into_iter()
       .filter(|r| r.workload_id == *workload_id)
       .collect();
   // ... terminal-projection arm unchanged ...
   let running_count: u32 = job_rows.iter()
       .filter(|r| r.state == AllocState::Running)
       .count()
       .try_into()
       .unwrap_or(u32::MAX);
   if running_count >= replicas_desired.get() {
       let running = job_rows.iter()
           .filter(|r| r.state == AllocState::Running)
           .max_by_key(|r| r.updated_at.counter)
           .unwrap_or_else(|| unreachable!("running_count >= 1 guarantees at least one Running row"));
       return Some(SubmitEvent::ConvergedRunning {
           alloc_id: running.alloc_id.to_string(),
           started_at: format!("{}@{}",
               running.updated_at.counter,
               running.updated_at.writer.as_str()),
       });
   }
   ```

   The most-recently-updated Running row (NOT `latest`) sources the
   wire event's `alloc_id` and `started_at`. The distinction is
   load-bearing: `latest` is the LWW winner across all states for the
   workload, which may itself be a non-Running transition (e.g. an
   intermediate Pending update) while sibling Running rows have
   already met the count threshold. Picking `latest` would emit
   `alloc_id` belonging to a non-Running allocation.

4. **Handler call-site extraction** (`handlers.rs:479–490`) — the
   Service branch extracts `replicas_desired` from the validated
   `WorkloadIntent::Service(s).replicas` (already in scope from
   `ServiceV1::from_submit`) and passes it to `build_stream`. The
   implementation landed more defensive than the RCA template: two
   explicit `unreachable!()` arms (Schedule + Job) with citations to
   the validation step and sibling handler, instead of one combined
   `_`. Each arm pins the invariant at the match site:

   ```rust
   let replicas_desired = match &intent {
       WorkloadIntent::Service(s) => s.replicas,
       WorkloadIntent::Schedule(_) => unreachable!(
           "Schedule rejected at submit (handlers.rs validation step \
            returns HTTP 400 before reaching this branch); \
            Job uses build_workload_stream"),
       WorkloadIntent::Job(_) => unreachable!(
           "Job dispatch is the sibling arm above; this branch is \
            Service-or-Schedule only"),
   };
   ```

5. **`TODO(#140)` removal** — the deferral marker at the previous
   `streaming.rs:358` site is gone (`git grep 'TODO(#140)' crates/`
   returns zero matches). Per `CLAUDE.md` § "Deferrals require
   GitHub issues", a forward-pointer comment cannot survive the
   issue it tracks.

6. **Rustdoc contract update** — both `check_terminal` and
   `lagged_recover` carry full four-property contracts per
   `.claude/rules/development.md` § "Trait definitions specify
   behavior, not just signature": preconditions on `replicas_desired`,
   postconditions on when `ConvergedRunning` is returned, edge case
   for `replicas_desired == 1` (behaves identically to the prior
   single-row shortcut by construction), and the invariant that
   terminal-projection bypasses the count gate.

7. **Regression test** — new `#[tokio::test]`
   `streaming_lane_does_not_emit_converged_running_until_running_count_meets_replicas_desired`
   in `crates/overdrive-control-plane/tests/acceptance/streaming_submit.rs`.
   Submits a Service spec with `replicas: 2` via
   `Accept: application/x-ndjson`, asserts zero `kind ==
   "converged_running"` lines after the first allocation reaches
   Running, asserts exactly one `converged_running` line after the
   second reaches Running. Reuses the existing harness
   (`build_app_state`, `emit_lifecycle`, `body_ndjson_lines`,
   `SimClock`) — no new fixture primitive needed. Port-to-port
   shape: enters through the public HTTP submit lane, not by direct
   calls to `check_terminal` / `lagged_recover`.

## Why fail-fast on terminal-during-convergence

The terminal-projection branches (BackoffExhausted / Stopped /
Custom) fire unconditionally BEFORE the running-count gate. With
`replicas: 3`, if 2 are Running and 1 hits `BackoffExhausted`, the
stream emits `ConvergedFailed { BackoffExhausted }` and ends
immediately — it does NOT wait for the remaining allocations to
reach a terminal state.

Considered alternative: aggregate-outcome semantics (wait for all 3
to reach terminal, then emit a composite outcome). Rejected per RCA
§7 Q1 recommendation and user confirmation: an operator running
`submit --wait` against a Service whose one replica has exhausted
its restart budget wants `ConvergedFailed` now, not after the
timeout cap. The aggregate-outcome design is a deeper streaming-loop
restructure and is out of scope per issue #140.

## What was NOT changed

- **Reconciler logic.** Already replica-aware via
  `Action::StartAllocation` gating; touching it would have been scope
  creep.
- **`SubmitEvent` wire variants.** Unchanged — only *when* they fire
  changes. No client-side migration concern.
- **Observation store / IntentStore.** Untouched. The `replicas`
  field on `ServiceV1` was already correctly persisted (`NonZeroU32`,
  validated at construction).
- **Job kind streaming.** Already exempt per ADR-0047 §3 [D7]
  (`JobSubmitEvent` has no `ConvergedRunning` variant). The audit
  confirmed Job is structurally out of reach for the bug.
- **`ServiceSubmitEvent::ConvergedRunning`** (`streaming.rs:1020`) —
  declared but never emitted; a future-slice scaffold. If a later
  PR wires its emission path, the same replicas-gate logic must be
  applied there too. Tracked by inheritance from this fix; no
  separate issue.
- **Multi-replica scheduling.** Out of scope per issue #140 —
  already in the reconciler.
- **Replica-count CHANGES mid-life** (rolling deployment, scale).
  Phase 2 concern per whitepaper §15; explicitly deferred. Phase 1
  spec is immutable (`PutOutcome::Conflict` on re-submit with a
  different spec hash), so `replicas_desired` hydrated once at
  stream start IS the value for the stream's entire lifetime.

## Key decisions

- **Threading shape: option (A) — pass from the handler, not
  hydrate-in-streaming-task.** The handler already had the validated
  `ServiceV1` aggregate in scope at the `build_stream` call site;
  passing `replicas_desired: NonZeroU32` through one parameter
  required zero new I/O. Option (B) — one-shot `IntentStore` read
  inside the streaming task — was rejected because it would (1)
  re-read bytes the handler had just produced, (2) introduce a new
  `Job::from_store_bytes` decode failure surface, and (3) couple the
  streaming task to the codec module that was deliberately kept out
  of the streaming path.

- **`lagged_recover` picks most-recently-updated Running row, not
  `latest`.** This is the subtle correctness point of the fix. The
  audit caught it because the function body had to be rewritten —
  the single-row shortcut hid the question entirely.

- **Single-commit GREEN-on-fix landing.** Per RCA §8.5 and
  `.claude/rules/testing.md` § "RED scaffolds and intentionally-
  failing commits", a focused one-file production change where the
  RED state is documented by the issue body does not warrant a
  separate RED-scaffold commit. Test + fix land cohesively;
  `#[should_panic(expected = "RED scaffold")]` scaffolding is for
  multi-step DELIVER waves where intermediate commits would
  otherwise be GREEN-on-incomplete.

## Lessons

- **In-source `TODO(#issue)` markers are load-bearing forward
  pointers.** The `TODO(#140)` at `streaming.rs:358` was the single
  source of truth that any future engineer touching `check_terminal`
  could trip on. It survived from the original Phase 1 walking-
  skeleton landing through multiple intervening edits to the
  function. The discipline of writing structured TODO markers paid
  off — the audit took ~10 minutes because the trail was already
  laid. The complementary discipline (`CLAUDE.md` § "Deferrals
  require GitHub issues" — remove the marker when the issue closes)
  is what keeps the trail honest over time.

- **Audit before fix, even when the issue body is already an
  RCA.** The issue body was high-fidelity — it named symptom,
  mechanism, both call sites, and the preferred fix shape. The
  troubleshooter audit still found one material clarification: the
  bug only lives on the Service lane (Job is structurally exempt
  per ADR-0047). Without that audit, the regression test would have
  been a Job-kind test that wouldn't have actually exercised the
  buggy code path. The premise the issue body presented (Job
  framing) was reasonable but stale relative to ADR-0047's
  intervening landing.

- **`unreachable!()` arms with citations document the invariant
  better than a combined `_`.** The handler's two explicit
  `unreachable!()` arms (Schedule + Job) read better than a single
  catch-all and survive future readers grepping for "why is this
  unreachable." Each citation (Schedule rejected at validation; Job
  routed to sibling handler) makes the invariant audit-able without
  context-loading the surrounding handler logic.

## References

- **GitHub issue**: #140
- **PR**: #188
- **In-tree RCA**: `docs/feature/fix-converged-running-replicas-gate/bugfix/rca.md`
- **Adversarial review**: `docs/feature/fix-converged-running-replicas-gate/deliver/review-01-01.md`
- **Roadmap**: `docs/feature/fix-converged-running-replicas-gate/deliver/roadmap.json`
- **Execution log**: `docs/feature/fix-converged-running-replicas-gate/deliver/execution-log.json`
- **ADR-0047**: Workload-kind typed streaming siblings (Job kind's
  exemption from `ConvergedRunning` semantics)
- **Whitepaper §15**: Zero Downtime Deployments (defers replica-count
  changes mid-life to Phase 2)
- **Whitepaper §18**: Reconciler primitive — desired vs actual on
  replicas
- **Reconciler**: `crates/overdrive-core/src/reconciler.rs:495-497`
  (`Action::StartAllocation` gate, already replica-aware before this
  fix)
