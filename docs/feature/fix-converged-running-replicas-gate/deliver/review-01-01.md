# Review: Fix Converged Running Replicas Gate (Step 01-01)

**Reviewer:** nw-software-crafter-reviewer
**Review Date:** 2026-05-24
**Commit:** 28956fc7 (branch `marcus-sa/converged-running-gate`)
**Issue:** [#140](https://github.com/overdrive-sh/overdrive/issues/140) — Streaming `check_terminal` ignores `replicas_desired`
**Verdict:** **APPROVED**

---

## Executive summary

Focused, well-disciplined bugfix closing issue #140 with precision. Implementation conforms to the RCA specification in every material aspect: three function signatures updated (`build_stream`, `check_terminal`, `lagged_recover`), both emission sites gated on `running_count >= replicas_desired`, fail-fast semantics preserved, `TODO(#140)` deferral marker removed, trait-contract docstrings added, cohesive regression test landing alongside the fix in a single commit. **No defects detected.**

---

## RCA conformance (critical path)

### 1. Three function signatures — exact match

| Function | Location | Status |
|---|---|---|
| `build_stream` | `streaming.rs:171–176` | `replicas_desired: NonZeroU32` added per RCA §6 |
| `check_terminal` | `streaming.rs:398–403` | Parameter added |
| `lagged_recover` | `streaming.rs:514–518` | Parameter added |

### 2. Count-based gate in `check_terminal` — exact match to RCA §6 ad-hoc code block

`streaming.rs:425–436`:
```rust
let running_count: u32 = rows
    .iter()
    .filter(|r| r.workload_id == *workload_id && r.state == AllocState::Running)
    .count()
    .try_into()
    .unwrap_or(u32::MAX);
if running_count >= replicas_desired.get() { return Some(ConvergedRunning { ... }); }
```

### 3. `lagged_recover` non-terminal projection — correct refactor

`streaming.rs:520–569`: aggregate count over `job_rows` (workload-filtered subset) + `max_by_key(|r| r.updated_at.counter)` for the wire-event's `alloc_id` / `started_at` source. Implementation correctly picks the most-recently-updated Running row (NOT `latest`, which may itself be a non-Running transition while sibling rows have already met the gate).

### 4. Terminal branches unchanged (fail-fast preserved)

Both functions evaluate `event.terminal.is_some()` / `latest.terminal.is_some()` BEFORE the running-count gate. Any single terminal claim closes the stream regardless of sibling state, per RCA §7 + user Q1/Q2 confirmation.

### 5. `TODO(#140)` removal

`git grep 'TODO.*140' crates/` returns zero matches. Deferral marker at the previous `streaming.rs:358` site has been deleted per `CLAUDE.md` § "Deferrals require GitHub issues".

### 6. Handler Service-branch extraction

`handlers.rs:479–490` is **more defensive than the RCA template** — two explicit `unreachable!()` arms (Schedule + Job) instead of one combined `_`. This is **better**: it pins the exact invariants with clear citations (Schedule rejected at validation, Job routed to `build_workload_stream` in the sibling arm).

---

## Test quality

**Test name:** `streaming_lane_does_not_emit_converged_running_until_running_count_meets_replicas_desired` (streaming_submit.rs:1376–1483)

**Assertion sequence:**
1. Submit Service spec with `replicas: 2` via `Accept: application/x-ndjson`; spawn request in tokio task; wait for subscription.
2. Inject first replica reaching Running; yield/sleep 50ms.
3. Assert `request_task.is_finished() == false` — stream has NOT yet closed.
4. Inject second replica reaching Running.
5. Assert request completes within 5s timeout; last line has `kind == "converged_running"`; **exactly one** `converged_running` line in stream.

**Observable outcomes (not internal state):**
- Stream does NOT emit `converged_running` until `running_count >= replicas_desired`.
- Stream DOES emit exactly one `converged_running` once threshold met.
- Stream terminates cleanly.

**Deletion-test litmus:** if `check_terminal`'s count gate were removed, the test would fail at assertion 3 (`is_finished()` would be true after first Running row). **If the comparison were `>` instead of `>=`, the test would fail** (would need 3 replicas to emit).

**Harness reuse:** `build_app_state`, `emit_lifecycle`, `body_ndjson_lines`, `write_row`, `SimClock` — all existing fixtures per RCA §8. **No new harness primitive needed**, as predicted.

**Port-to-port wiring:** test enters through `POST /v1/jobs` (driving port) and asserts on the NDJSON response body (observable boundary). No internal class instantiation.

**Testing theater patterns:** none detected (no tautological assertions, no over-mocked SUT, no zero-assertion shape, no weakened comparisons).

---

## Rustdoc contract discipline

Per `.claude/rules/development.md` § "Trait definitions specify behavior, not just signature", both functions carry the four required properties.

### `check_terminal` (streaming.rs:360–397)

- **Preconditions:** `obs` is the live observation store; `workload_id` is the Service; `replicas_desired` carries the `NonZero` invariant from validated `ServiceV1.replicas`.
- **Postconditions:** Returns `Some(ConvergedRunning)` only when `event.terminal.is_none()`, `event.to == Running`, AND at least `replicas_desired.get()` rows carry `(workload_id == self, state == Running)`. Returns `Some(<terminal variant>)` whenever `event.terminal.is_some()`, bypassing the running-count gate.
- **Edge cases:** `replicas_desired == 1` behaves identically to the prior single-row shortcut by construction. Obs read error returns `None`.
- **Invariant:** A single terminal claim closes the stream regardless of running-count gate state.

### `lagged_recover` (streaming.rs:481–513)

- **Preconditions:** `obs` is the live observation store; `replicas_desired` carries the `NonZero` invariant.
- **Postconditions:** Returns `Some(<terminal variant>)` when the LWW-winner row carries `Some(TerminalCondition)` (fail-fast). Returns `Some(ConvergedRunning)` when Running-row count meets threshold; emitted `alloc_id` / `started_at` identify the **most-recently-updated Running row** (NOT necessarily the LWW winner).
- **Edge cases:** `replicas_desired == 1` behaves identically to prior shortcut. Zero rows → `None`.
- **Invariant:** Terminal-projection bypasses the running-count gate.

---

## Diff precision

`git diff bce69e44..28956fc7 -- crates/overdrive-control-plane/src/streaming.rs`:
- Three function signatures changed (build_stream, check_terminal, lagged_recover)
- Two body changes (Running-detection inline in check_terminal; non-terminal projection in lagged_recover)
- Rustdoc updates
- `TODO(#140)` deleted
- One inline unit test call site updated
- **No incidental edits** to `JobSubmitEvent`/`ServiceSubmitEvent` type declarations
- **No refactoring** of unrelated functions

`git diff bce69e44..28956fc7 -- crates/overdrive-control-plane/src/handlers.rs`:
- Service-branch extraction at `:479–490`
- `build_stream` call updated with new fourth argument
- **No edits** to Job branch
- **No edits** to Schedule rejection

`git grep -n 'has_running'` on `streaming.rs` returns zero matches — deleted single-row shortcut is gone.

---

## Quality gates (all pass)

| Gate | Status |
|---|---|
| Acceptance fails on current code | PASS (RCA §8 + issue #140 evidence) |
| RED_UNIT skipped as `NOT_APPLICABLE` | PASS (justified — inline logic) |
| No domain mocks | PASS (real `SimObservationStore`, real `LocalIntentStore`) |
| Business language in tests | PASS (`replicas_desired`, `running_count`, `converged_running`) |
| All tests green | PASS (148 acceptance, 202 default lane) |
| Test budget met | PASS (1 test ≤ 6 budget) |
| Single-cut discipline | PASS (no shims, no fallbacks) |

---

## Praise

- **`praise:` Defensive `unwrap_or(u32::MAX)` cast.** `.count().try_into().unwrap_or(u32::MAX)` converts `usize → u32` gracefully; an unreachable >4B-row case becomes a sentinel that always satisfies `>= replicas_desired` rather than panicking. Sound choice.

- **`praise:` Handler dual `unreachable!()` arms with explicit citations.** More defensive than the RCA template — two explicit arms (Schedule + Job) instead of one `_`, each with clear citations to validation step and sibling handler. Makes invariants crystal clear at the match site.

- **`praise:` `lagged_recover` correctly picks most-recently-updated Running row, not `latest`.** The subtle distinction from RCA §6 (where `latest` may be a non-Running transition while siblings have already met the gate) is correctly implemented. Easy to miss, correctly caught.

- **`praise:` Rustdoc updates pin the contract, not the implementation.** Postconditions stated in terms of observable behavior (`ConvergedRunning` returned only when X), not in terms of internal data structures. This is exactly the trait-contract discipline the project rule prescribes.

---

## Non-blocking observations

- **`thought (non-blocking):` Branch surprise.** The crafter created `marcus-sa/converged-running-gate` rather than committing onto `marcus-sa/doha` where work started. Not a defect — one-PR-per-fix is a reasonable convention — but it's an unsolicited workflow choice. Future bugfix dispatches could either explicitly request the branching shape or have the crafter ask.

- **`nitpick (non-blocking):` Duplicated `filter(|r| r.state == AllocState::Running)` in `lagged_recover`.** The filter appears twice (once for the count, once for the `max_by_key` selection). Three occurrences would be the rule-of-three breakpoint for extracting a helper; at two, the inline shape is more readable than a named helper would be. Leave as-is.

---

## Risk assessment

- **Scope:** Three function signatures, two body changes, one handler call site, one test.
- **Wire changes:** None — `SubmitEvent` variants unchanged; only *when* they fire changes.
- **Reconciler / observation store changes:** None.
- **Replica-count staleness:** None — Phase 1 aggregates are immutable post-submit (`PutOutcome::Conflict` on re-submit per `handlers.rs:432-434`).

**Overall risk:** minimal. Surgical, localized to streaming emission path. Ready to merge.

---

## Approval

**Verdict: APPROVED**

Full RCA conformance, zero defects, test budget satisfied, quality gates all pass, external validity confirmed, trait contracts documented, single-cut discipline observed, no testing theater. Ready to merge.
