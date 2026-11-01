# DELIVER Review — Step 11-02

## Review metadata

| Field | Value |
|---|---|
| Feature / step | `netns-density-295` / `11-02`, RESTART-SSOT |
| Review type | Fresh independent current-source closure review |
| Reviewer | Fresh isolated DELIVER reviewer |
| Review date | 2026-11-01T20:57:53Z |
| Implementation commit | `c60ccedd946e32b6c40e9303719d96fb55ed62fd` |
| Bounded remediation | `98e16fb0e58c027aab19a51327294496727a5fad` |
| Current source boundary | `HEAD 00ce922c43b9743651cfda535af006f2393abbe3` |
| DES completion record | `execution-log.json`: RED/GREEN/COMMIT PASS on 2026-10-22; lint-only remediation RED SKIPPED, GREEN/COMMIT PASS on 2026-10-23 |
| Iteration | 1 |
| Verdict | **APPROVED** |

This review was performed at the date above. It records a current independent
review of the implementation and its bounded remediation; it does not claim
that a reviewer executed historically on the implementation or remediation
dates. No source, test body, roadmap, DES event, commit or existing review was
changed. The only authored path is this Markdown artifact.

The working tree already contained the user's dirty `AGENTS.md`, the user's
roadmap edit, and the untracked whole-slice review artifact. Those paths were
preserved and are outside this review.

## Authority and review method

The accepted contract was read from `AGENTS.md`, the step 11-02 roadmap entry,
`feature-delta.md` § “Restart decision — one predicate for
`WorkloadLifecycle` and its accessor,” DISTILL's RESTART-SSOT classification,
ADR-0087's GH #324 amendment, and the current bounded consumers. The review
applied the repository's development, testing, Rust, and reconcilers rules,
the installed `nw-sc-review-dimensions`, `nw-tdd-review-enforcement`,
`nw-tdd-methodology`, `nw-quality-framework`, `nw-review`, and
`nw-review-output-format` guidance, subject to the repository overrides.

The review compared the exact public signatures, semantic restart rule,
ownership and view-read boundary, production stream path, `cid_lease`
projection, test assertions and test-layer boundaries. It also inspected the
actual commit scope, attribution, DES events, retained verification receipts,
and the current source after the remediation. No mutation, KASAN, benchmark,
native-kernel, VM, or duplicate test run was started; the existing bounded
receipts are sufficient for this current-source closure review.

## Step contract and implementation assessment

The step's five obligations are satisfied by the current source:

| Contract obligation | Current implementation and evidence | Assessment |
|---|---|---|
| One ceiling predicate and three reconciler checks | `restart_decision` is the exact `pub const fn` in `crates/overdrive-reconcilers/src/workload_lifecycle.rs:47-53`; the next-wake, pending-restart-owner, and main failed-allocation checks call it at `:388`, `:445`, and `:1132`. No production ceiling expression remains outside that predicate. | PASS |
| Reclaim and generation exemptions | `restart_decision` implements `attempts < RESTART_BACKOFF_CEILING || platform_reclaimed || generation_replacement`; the runtime boundary tests cover `CEILING - 1`, `CEILING`, platform reclaim at and beyond the ceiling, and pending generation at the ceiling (`reconciler_runtime.rs:1897-1967`). | PASS |
| Reconciler-equivalent failed-allocation decision | `restart_after_failure` at `workload_lifecycle.rs:1634-1651` follows the accepted order: run intent, `Draining`, Job natural exit, candidate filter, then the shared predicate. The seeded proptest at `:4000-4043` compares it with a real `WorkloadLifecycle::reconcile` over all three workload kinds, intents, row classes, generations, attempts and lineage. | PASS |
| Job stream decision | `build_workload_stream` enters `workload_event_from_lifecycle_with_state` (`streaming.rs:193-350, 425-462`) for intermediate failed rows. The path hydrates the same desired intent/generation, reads the persisted event row when available, and calls `restart_status_for_alloc` through `attempt_failed_event` (`:499-523`). The acceptance and integration tests assert `will_restart == false` and no delay for a Job crash. | PASS |
| `cid_lease` projections | `quint_cid_lease.rs:797-810` projects `retry`/`ex` through `restart_status_for_alloc` with `Service`, `run`, the hydrated current row and generation; `:1534-1548` projects `att` through `attempt_index_for_alloc`; `Reading::current` uses the re-exported `current_alloc` at `:662-666`. | PASS |

The exact approved public API is present and bounded:

- `restart_decision(attempts, platform_reclaimed, generation_replacement) -> bool`;
- `restart_after_failure(kind, run_desired, failed, attempts,
  generation_replacement) -> bool`;
- `ReconcilerRuntime::restart_status_for_alloc(target, kind, run_desired,
  failed, desired_generation) -> (u32, bool)`;
- `ReconcilerRuntime::attempt_index_for_alloc(target, alloc_id) -> u32`; and
- `current_alloc(&[&AllocStatusRow]) -> Option<&AllocStatusRow>`, re-exported at
  the reconcilers crate root.

No new public type, enum variant, trait, parameter, wire field or persisted
field was invented. The restart budget remains the `WorkloadLifecycle` view's
`restart_counts` input; the accessor recomputes its decision from that view,
the current row, desired intent and generation. It does not persist a derived
decision or read a second reconciler's private view.

## Production path and lifecycle evidence

The reconciler's decision is pure and synchronous. `restart_after_failure`
does not perform I/O, mutate a store, or bypass the action boundary. The
runtime accessor reads the view through `view_for_workload_lifecycle` and
returns only the attempt index and approved decision. The production Job stream
uses the state-aware path, whose hydration is the reconciler's own
`hydrate_desired` through `build_hydration_context`; failures are returned as
`std::io::Error` to the stream rather than absorbed into a default decision.

The remediation closes the event-row fact boundary. The action shim documents
and implements row-write-before-broadcast ordering (`action_shim/mod.rs:253-269,
464-505, 620-642, 741-744, 826-829`). `streaming.rs:442-447` reads that
persisted row when it exists; the event-only fallback uses the approved
`build_alloc_status_row` at `:474-496`, so `CrashFacts::advance` owns
`last_terminated` and `restart_count` rather than a raw production struct
literal. This preserves ADR-0078 crash facts while keeping the stream's
decision tied to the event's allocation.

The `cid_lease` driver remains a projection and driver of the production
composition: it hydrates through the production reconciler, calls the runtime
accessors, and performs placement through the convergence tick/action shim.
It does not call the pool directly to manufacture `retry`, `ex`, or `att`, and
it does not introduce a second restart authority.

## Typed error and interface mapping

This step has no new semantic error contract. The approved restart interfaces
return a boolean decision and a bounded attempt index; they do not collapse a
designed typed failure into a string. The state-aware stream's existing
`std::io::Error` item is used only for hydration or observation-read failures,
while the restart decision itself has no error path. The action shim's typed
terminal claims remain unchanged and continue to be carried on both the
durable row and lifecycle event from the same action value.

## Test integrity and acceptance coverage

The authored tests exercise behavior at the permitted boundaries:

- `workload_lifecycle::restart_ssot_agreement` is a source-local proptest with
  `/// CONTRACT_SHAPE: pure-function.` and compares the new pure decision with
  the real reconciler output, including a declared row-class universe.
- The three runtime accessor tests cover the ceiling and both exemptions at the
  runtime view boundary (`reconciler_runtime.rs:1897-1967`).
- The Job acceptance test drives the production router/stream and asserts the
  operator-visible `attempt_failed` fields (`streaming_submit.rs:1273-1349`).
- The integration projection tests assert attempt index, `will_restart`, exit
  code and delay (`streaming_attempt_failed.rs:72-213`).

Assertions are behavioral and falsifiable. The tests do not mock the SUT,
recompute the implementation's predicate as their oracle, or assert only call
counts. The acceptance body was transitioned by removing only its reasoned
pending marker; the production implementation changed in the same step, and
the remediation changed only the production stream projection. No assertion
was weakened, deleted, skipped or replaced to accommodate the implementation.

The retained receipts show the required evidence layers passed:

| Check | Retained receipt | Result |
|---|---|---|
| RED semantic failure | `.context/netns-density-295/11-02/red-streaming-submit.log` | The old Job stream reports `will_restart: true`; the intended regression is demonstrated. |
| Reconciler property | `green-reconcilers-property.log` | 1 selected test passed; 62 unrelated tests skipped by filter. |
| Full reconcilers package | `green-full-reconcilers.log` | 62 passed, 1 skipped. |
| Focused control-plane stream/accessor suite | `green-control-plane-tests-final2.log` | 22 passed, 1107 skipped by filter. |
| Remediation stream suite | `remediation-stream-tests.log` | 19 passed, 1110 skipped by filter. |
| Workspace compile/clippy | `green-workspace-check-final.log`, `green-workspace-clippy-final.log` | Both exit 0. |
| Remediation compile/clippy/format | `remediation-check.log`, `remediation-clippy.log`, `remediation-format.log` | All exit 0. |
| Crash-observability lint | `remediation-dst-lint-after.log` | `cargo xtask dst-lint` exit 0. |

The historical execution log records the step's RED → GREEN → COMMIT order and
the remediation's explicitly recorded lint-only RED skip followed by GREEN →
COMMIT. This review does not reinterpret those events as a review approval.

## Commit and scope audit

`c60ccedd946e32b6c40e9303719d96fb55ed62fd` changes the five roadmap source/
test paths plus `execution-log.json`; `98e16fb0e58c027aab19a51327294496727a5fad`
changes only `crates/overdrive-control-plane/src/streaming.rs` for the bounded
crash-fact projection remediation. Both commits preserve the Marcus author,
contain exactly `Co-Authored-By: Codex <codex@openai.com>`, and carry
`Step-Id: 11-02`. `git show --check` is clean for both commits. No unrelated
source, test, formal input, roadmap, or DES history was swept into the step.

## Findings and dispositions

`praise:` The implementation makes the accepted single-authority rule literal:
the reconciler's three production ceiling checks and the retained runtime
accessor converge on the same shared predicate, while the Job stream and
`cid_lease` consumers use the accessor rather than copying restart policy.

`praise:` The remediation preserves the durable allocation facts at the
event-only stream boundary. Reading the persisted row and using the existing
allocation-row builder closes the concrete crash-fact regression without
adding a public seam, a second owner, or a new error variant.

No blocking, high, medium, or low implementation findings remain. No
reachable production failure was reproduced or evidenced against the current
source. The previously recorded pre-remediation stream failure is historical
RED evidence and is correctly followed by the bounded remediation receipts;
it is not a current failure.

## Iteration 1 verdict

**APPROVED.** The current implementation matches the exact RESTART-SSOT API
and semantic contract, routes all bounded consumers through the approved
authority, preserves typed lifecycle facts and ownership, and retains honest
test/DES evidence. No source or test remediation is required for step 11-02.
