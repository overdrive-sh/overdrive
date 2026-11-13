# DELIVER Review — Step 03-02: Broker submit wakes the convergence loop

## Review metadata

| Field | Value |
|---|---|
| Feature | `service-allocation-readiness-and-ending` |
| Step | `03-02` — Broker submit wakes the convergence loop |
| Iteration | 1–3 (current final remediation re-review: 3) |
| Reviewer | Codex, fresh isolated DELIVER reviewer; iteration 3 is the fresh final Clippy remediation reviewer |
| Implementation commit | `97e7e5d7855b49e6456347e1454b6b47e97c2c9c` |
| COMMIT audit commit | `b046718aee011be6333f14a9578c5dc5c0b6f9a0` |
| F-01 documentation remediation commit | `09b052b39bf7bf543651ab81d1f8ffd987a9d539` |
| Final Clippy remediation commit | `ff29358a80fc10e112b7b7f6ba1a86dcf0711c20` |
| Final remediation DES audit commit | `32a60dd17e5d28d8a5fe4fceb9d978bca80335be` |
| Comparison base | `641f546aec009cfa4a5af416ea919948d76334e` (approved step 03-01) |
| Authoritative contract | Approved roadmap step 03-02; feature-delta D17 and D17-S1; ADR-0273; approved architecture brief |
| Upstream prerequisite | Step 03-01 review is `APPROVED`; the D17-S1 signatures are user-approved and included in the reviewed DESIGN contract |
| Verdict history | Iteration 1: **NEEDS_REVISION**; iteration 2: **APPROVED**; iteration 3: **APPROVED** |

## Executive summary

The D17 implementation and its accepted API remain unchanged. Iteration 1's F-01 documentation finding was closed in iteration 2. The final Clippy remediation commit `ff29358a8` restricts the private action-shim `EvaluationSubmission::Standalone` adapter and its `EvaluationBroker` import and match arm to `cfg(test)`; the only constructors are in source-local test modules. This removes the feature-enabled non-test-library dead-code failure while preserving the standalone test composition. The production runtime path and D17 ordering, stored-permit, wake-selection, drain-only broker, and cadence invariants are unchanged.

The remediation's recorded focused strict Clippy run and five D17 convergence-loop wake tests pass. The full-workspace Clippy run reached a separate scanner `doc_markdown` issue, which the 05-02 owner subsequently fixed and independently approved; it is unrelated and is not required for this bounded review. The user explicitly waived mutation testing, and the roadmap records the final mutation gate as `required: false`, `status: not_required_by_user`. No mutation run or witness is required here. No new finding remains; the current verdict is **APPROVED — iteration 3**.

## D17 contract and production-path review (iteration 1 baseline)

This section preserves the initial review snapshot. Its descriptions of the
pre-remediation Standalone cfg and the then-pending mutation gate reflect the
state reviewed in iteration 1; the final cfg and current mutation-gate status
are recorded in iteration 3 below.

### Exact interface contract

The new public `BrokerDrain<'a>` type has a private guard and only the three pinned methods: `drain_pending(&mut self, usize, &BTreeSet<TargetResource>, Instant, UnixInstant) -> Vec<(Evaluation, Duration)>`, `next_eligible_at(&self, &BTreeSet<TargetResource>) -> Option<UnixInstant>`, and `counters(&self) -> BrokerCounters` (`crates/overdrive-control-plane/src/reconciler_runtime.rs:174–200`). `ReconcilerRuntime::broker(&self) -> BrokerDrain<'_>` matches D17-S1 (`:427–430`). There is no `Deref`, `AsRef`, public guard field, or submission method on the wrapper.

`ReconcilerRuntime::submit_evaluation(&self, Evaluation, Instant, EvaluationEligibility)` matches the approved D17 signature (`reconciler_runtime.rs:432–444`). `submitted(&self)` remains `pub(crate)` and is used by the loop. `broker_mutex()` is removed. The only new stored runtime state is the private `Notify`; no public types, method parameters, enum variants, wire or persisted shapes, configuration, or scheduling policies were added beyond the approved interface.

### Submission ownership, ordering, and admission

The production entry inserts while holding the runtime broker mutex, explicitly drops the guard, then signals (`reconciler_runtime.rs:434–449`). The loop submits due resync evaluations through that same entry, drains through `BrokerDrain`, and preserves the existing cadence-derived sleep duration (`crates/overdrive-control-plane/src/lib.rs:5129–5183`). Its biased `select!` keeps shutdown and completion first, places `submitted()` before the clock timer, and disables wake admission only during shutdown drain (`lib.rs:5201–5229`). The drain guard is out of scope before any await.

Caller audit of the changed production owners:

- The handler and runtime self-re-enqueue use `submit_evaluation` (`handlers.rs:77–82`; `reconciler_runtime.rs:1791–1796`).
- The interest router's runtime-backed capability uses `submit_evaluation` (`lib.rs:5285–5293`). Its standalone shared-broker adapter is a separately owned harness path, explicitly unchanged by D17 (`lib.rs:5296–5308`); its only test caller uses that adapter (`tests/acceptance/interest_router.rs:327–329`).
- Production action-shim dispatch creates `EvaluationSubmission::Runtime(&state.runtime)` (`action_shim/mod.rs:1364–1398`); `Action::EnqueueEvaluation` routes through it (`:3492–3498`). Production reclamation receives `&state.runtime` and submits each of its three downstream evaluations through the same runtime submission interface (`vm_reclamation_boot.rs:131–143`; `action_shim/reclamation.rs:174–199, 270–296`). Gateway identity's production owner wraps `ReconcilerRuntime` and its `WakeSubmit` implementation calls `submit_evaluation` (`gateway_identity_lifecycle.rs:58–62, 81–89, 110`).
- The no-wake `Standalone` action-shim submission variant is gated by `test` or `integration-tests` (`action_shim/mod.rs:85–104`); the direct-broker dispatch forms are similarly cfg-gated (`:1049–1155`). The gateway lifecycle's standalone constructor is cfg-gated to tests (`gateway_identity_lifecycle.rs:71–79`). These are test adapters, not production runtime broker submit paths.
- Remaining direct `EvaluationBroker::submit` calls are internal to `submit_evaluation`, within the cfg-gated standalone adapters, or the explicitly standalone `InterestRouterBroker::from_shared_broker` capability. No production caller retains access to the runtime's raw broker mutex.

The implementation leaves `EvaluationBroker`'s FIFO, same-key collapse, blocked-target handling, `NotBefore`, counters, capacity handling, and per-target exclusion unchanged. `notify_one` is issued after insertion and after lock release. The stored-permit behavior covers a submit while the loop is between draining and parking; a completion remains ahead of the wake arm and a dropped notified future does not discard the permit. A deferred evaluation remains governed by its existing eligibility timestamp after unrelated wakes. The due-resync table and cadence timer remain the level-triggered backstop. No additional scheduling policy or architecture was introduced.

### Executable coverage and test integrity

The four D17 scenarios in `convergence_loop_wake.rs` are activated and exercise `spawn_convergence_loop` with the production `ReconcilerRuntime`, registered `noop-heartbeat`, and a `SimClock` that does not auto-advance (`convergence_loop_wake.rs:68–119, 171–281`). They cover admission while parked without a clock advance, a distinct-target burst plus same-key collapse, `NotBefore` remaining deferred across an unrelated wake, and reproducible admission order for the same seed. The pre-existing cadence-only control remains live (`:213–225`). The four newly active test functions carry per-test `CONTRACT_SHAPE: bounded-change` declarations (`:173–175, 191–193, 230–232, 250–252`).

The S-48(b) drain-to-park-window case remains explicitly waived and backed by the accepted Tokio stored-permit rationale plus the final mutation gate; S-48(e) remains waived because it would require a blocked production reconciler or a new seam. The acceptance fixture documents those exact limits (`convergence_loop_wake.rs:11–15`), and the final roadmap gate retains the required signal-position and stored-permit mutants (`deliver/roadmap.json`, `final_wave_gate.d17_waiver_evidence`). No Quint model or test seam was added, consistent with D17-S6 and the user-approved handoff.

The test-file diff activates the four pre-authored D17 cases and mechanically routes existing runtime submissions through `submit_evaluation`. The `svid_lifecycle_reenqueues_on_dispatch_failure` helper now scopes the drain guard so it is dropped before re-submitting through the runtime (`tests/acceptance/svid_lifecycle_reenqueues_on_dispatch_failure.rs:178–197`); its behavioral oracle is unchanged. Other changed acceptance tests only replace the former `broker().submit` calls with the pinned runtime entry. No assertion was removed, weakened, skipped, or rewritten to make the feature pass. The action-shim unit fixtures use the cfg-gated standalone adapter where they intentionally own and inspect their own broker.

There are four newly activated D17 scenario tests for the four D17 acceptance behaviors, plus one unchanged cadence control. The composition tests are not an over-budget set of isolated unit tests; no budget blocker applies. The tests remain in-process and do not start the built Overdrive binary or act as expectation runners.

### Typed error mapping

| Designed semantic condition | Approved and implemented representation |
|---|---|
| A broker submission is made | `submit_evaluation` returns `()` and inserts the evaluation, then notifies; the existing `EvaluationBroker::submit` has no semantic error result. |
| A wake is signalled or awaited | `Notify::notify_one` and the crate-private `submitted()` wait are infallible; there is no designed typed failure or error variant for this wake path. |
| The convergence loop drains broker state | `BrokerDrain` returns the exact approved drain/read result types; no new error representation is introduced. |

No new semantic failure distinction is required by D17, and none is encoded in strings or an invented generic error carrier. There is no `SPECIFICATION_AMBIGUITY` on the D17 error contract.

## Finding and remediation disposition

### F-01 — The runtime-mechanics rule still specifies the pre-D17 tick and submit surface

**Severity:** High  
**Status:** Blocking; remediation required in this step  
**Location:** `.claude/rules/development.md:1044–1048, 1090–1101, 1108–1131`

The section labels the steady-state loop as running “every `tick_period_ms`, default 100 ms” and shows it draining as the cadence tick. It also directs resync and interest-router submissions to `broker.submit` and calls that the router's production effect. After this step, a production submit wakes the loop immediately through `ReconcilerRuntime::submit_evaluation`; the loop can iterate before the timer, while the cadence timer remains the backstop. The direct production submission API is no longer exposed through `broker()`.

This is an accepted landing obligation, not a speculative production failure: ADR-0273's consequences explicitly state that `.claude/rules/development.md` § “Reconciler I/O” describes the steady-state tick as the drain period and “at landing that sentence needs the wake added” (ADR-0273:128–130). The code demonstrates the behavior through the production owner path: insert/release/notify in `reconciler_runtime.rs:434–449`, then the loop selects `submitted()` before `clock.sleep` in `lib.rs:5201–5229`. The unchanged rule currently contradicts that accepted contract and presents the retired direct-submit surface to future maintainers.

**Required bounded remediation:** update this runtime-mechanics section to describe immediate submit-triggered loop iterations, the retained cadence backstop, and the single production `submit_evaluation` path. Keep the existing cadence/resync and interest-router ownership and relist guarantees intact. No new runtime mechanism or test seam is needed. This documentation conformance finding does not allege a defect in the current runtime behavior.

## Contract Shape compliance

The four newly activated D17 tests each have a per-test bounded-change declaration. The cadence control was pre-existing and unchanged. The only changes in existing test bodies are the mechanical API migration and the drain-guard scope correction required by the new drain-only interface; their behavioral assertions remain intact. No source-local pure-function Rust property was added or transitioned, so the exact `/// CONTRACT_SHAPE: pure-function.` rustdoc requirement does not apply. No banned test-name pattern or testing-theater assertion was introduced in the reviewed diff.

## TDD, DES, and quality-gate evidence

The step's execution log records the required order and outcomes: RED executed/FAIL at `2026-11-10T05:45:30Z`, GREEN executed/PASS at `06:00:26Z`, and COMMIT executed/PASS at `06:20:48Z` (`deliver/execution-log.json`). The crafter's handoff reports that the four D17 scenarios fail against the submit scaffold and pass after implementation; the step run reports 35 focused tests across seven binaries, all-target control-plane/integration tests, clippy, and scoped formatting passing. The recorded whole-workspace format check differed only in untouched `fwd_guest_suite.rs` and `xtask/tests/acceptance.rs`. These commands were not rerun during this review because the focused and broad step evidence is already recorded and no unresolved code failure required reproduction.

Mutation testing was not run for this individual step, as required by the repository DELIVER rule. The approved roadmap retains one final mutation gate after all 17 step reviews and includes each production file touched by this step plus the required D17 signal-position and stored-permit targets.

`git diff --check` passed for both implementation and audit commits. The implementation commit preserves Marcus as Git author and contains exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 03-02`, and `Task-Id: service-allocation-readiness-and-ending`. The audit commit changes only the execution log and carries the same attribution. The workspace was clean before this review artifact was written. No production, test, model, DES, roadmap, or commit changes were made by the reviewer.

## Iteration 1 verdict

**NEEDS_REVISION.** The D17 production implementation and exact API pass review. Close F-01 by updating the explicitly named runtime-mechanics rule, then return the step to the same crafter for a fresh reviewer re-review. No code change is requested by this finding.

## Iteration 2 — F-01 remediation re-review

### Revision reviewed

Commit `09b052b39bf7bf543651ab81d1f8ffd987a9d539` updates only
`.claude/rules/development.md`. It replaces the clock-only “Steady-state tick” description with a convergence-loop iteration that re-enters on broker submission, evaluation completion, timer, or shutdown (`:1044–1061`). The revised text names `ReconcilerRuntime::submit_evaluation` as the production submission entry, explains insert → unlock → `Notify::notify_one`, and describes `broker()` as the drain-only `BrokerDrain` surface (`:1063–1070`).

The cadence and event-interest sections now route resync through the runtime entry, retain the one-per-period re-arm, and explain that the production `InterestRouterBroker` uses the same wake entry while its standalone-broker test composition does not wake the runtime (`:1102–1141`). The accepted relist period, accepted-write filtering, and sole-drainer ownership remain stated. These edits close each point in F-01 without changing production code, tests, DES, the roadmap, or the D17 mechanism.

### Finding disposition

| Finding | Disposition |
|---|---|
| F-01 — runtime-mechanics rule described the pre-D17 clock-only tick and direct `broker.submit` surface | **Closed.** The rule now documents submit-triggered loop iterations, the cadence backstop, the exact runtime submit entry, the drain-only accessor, and the unchanged cadence and interest-router guarantees. |

No new finding was introduced by the documentation revision. The update is consistent with D17 and ADR-0273 and does not relax any accepted outcome or invariant.

### Re-review verification

| Check | Result |
|---|---|
| Exact F-01 locations compared with revised rule | **PASS** — the clock-only cadence description and direct production submit instructions are corrected. |
| Cadence and interest-router guarantees | **PASS** — periodic re-arm, relist behavior, accepted-write filtering, and runtime sole-drainer ownership are retained. |
| Commit scope and attribution | **PASS** — documentation-only change; Marcus remains author; exactly `Co-Authored-By: Codex <codex@openai.com>`, `Step-Id: 03-02`, and `Task-Id: service-allocation-readiness-and-ending` are present. |
| Whitespace | **PASS** — `git show --check` and `git diff --check 09b052b39bf7bf543651ab81d1f8ffd987a9d539^ 09b052b39bf7bf543651ab81d1f8ffd987a9d539`. |
| Executable tests | Not rerun. The remediation changes only documentation, and the reviewed D17 production code and tests are unchanged from iteration 1. |

The workspace contained only this reviewer artifact as untracked work before this re-review. No production, test, model, DES, roadmap, or commit changes were made by the reviewer.

## Iteration 2 final verdict (historical)

**APPROVED — iteration 2.** F-01 is closed. The step's D17 implementation, exact public API, production submission ownership, test integrity, and required runtime-mechanics documentation now match the accepted contract. The final DELIVER-wave mutation gate remains pending after all roadmap step reviews as specified.

## Iteration 3 — final Clippy remediation re-review

### Revision reviewed

The final remediation consists of implementation commit
`ff29358a80fc10e112b7b7f6ba1a86dcf0711c20` and DES audit commit
`32a60dd17e5d28d8a5fe4fceb9d978bca80335be`. The implementation commit
changes only three conditional-compilation attributes in
`crates/overdrive-control-plane/src/action_shim/mod.rs` and adds the
step-specific remediation record. The DES commit appends this replacement
crafter's RED, GREEN, and COMMIT events. The review preserves the iteration 1
finding and iteration 2 closure above; this is the current final review of the
additional Clippy remediation.

### Approved contract and implementation review

The authoritative D17 interface remains the approved feature-delta contract:
`ReconcilerRuntime::submit_evaluation(&self, eval, now, eligibility)` is the
only production submit path; `BrokerDrain` has only its three approved
drain/read methods; `broker_mutex()` remains removed; and `EvaluationBroker`
remains unchanged. D17 also requires insert → unlock → signal ordering,
`Notify::notify_one` stored-permit behavior, a `submitted()` select arm before
the cadence timer, unchanged deferred eligibility and admission semantics, and
the cadence timer as the level-triggered backstop.

The reviewed source diff changes no runtime submission or convergence-loop
code. In `action_shim/mod.rs:25–26`, the `EvaluationBroker` import is now
under `#[cfg(test)]`. At `:73–95`, the crate-internal
`EvaluationSubmission::Standalone` variant and its `submit` match arm use
the same `cfg(test)` gate. The production `Runtime` variant and its
`submit_evaluation` call remain unconditional. This is a private compilation
correction; it adds no public or cross-crate API.

The constructor audit found all `Standalone` uses within test-only owners:

| Constructor owner | Gate | Result |
|---|---|---|
| `action_shim/reclamation.rs` test module (`:343–345`, constructors at `:513`, `:667`, `:746`, `:811`, and `:848`) | `#[cfg(test)]` | Available in the source-local unit-test build. |
| `action_shim/replacement_acceptance.rs` (constructor at `:427`) | Included from `action_shim/mod.rs:3329–3330` under `#[cfg(all(test, feature = "integration-tests"))]` | Available when the integration-tests feature participates in the source-local unit-test build. |

Repository search found no other `EvaluationSubmission::Standalone`
constructors or call sites. Therefore the library built with
`integration-tests` but without `cfg(test)` has only the `Runtime` variant
and a complete one-arm match, while the unit-test composition keeps the
standalone adapter and its broker import. This matches the precise compiler
failure recorded in the remediation report: the prior
`cfg(any(test, feature = "integration-tests"))` exposed an unconstructed
variant in the feature-enabled non-test library.

The focused all-target Clippy command includes
`integration-tests,kvm-tests`, and the recorded `--lib --features
integration-tests` nextest command compiles and executes the source-local
test composition. Both are relevant evidence that the gated import, enum
variant, match arm, and test constructors remain buildable in their required
configurations.

The production D17 invariants remain intact. The remediation diff does not
touch `ReconcilerRuntime::submit_evaluation` (insert, release the lock, then
signal), `BrokerDrain`, the runtime's `Notify`, the convergence loop's
`submitted()` select arm or its position relative to completion and the
cadence timer, resync routing, `NotBefore`, FIFO/same-key collapse, or
shutdown drain behavior. The current runtime mechanics documentation still
matches the iteration 2 F-01 closure. No design premise or accepted invariant
is weakened.

### Test integrity and Contract Shape

No acceptance or unit test source changed in the final remediation commit, and
no test was added, removed, skipped, or weakened. The standalone constructors
remain in their existing source-local test compositions. The existing five
D17 convergence-loop wake tests pass in the recorded run. The prior review's
acceptance behavior coverage and per-test Contract Shape declarations remain
unchanged; this remediation introduces no new or transitioned test requiring
a declaration. The test budget is unchanged.

No D17 semantic error path or error representation changed. The remediation
adds no error type, variant, payload, mapping, or string-carried distinction;
the prior typed-error review remains valid.

### TDD, DES, commit, and verification evidence

The execution log contains the replacement crafter's own ordered
`03-02` events, distinct from earlier step events:

| Phase | Recorded outcome | Timestamp |
|---|---|---|
| RED | Executed; expected strict Clippy failure reproduced | `2026-11-13T02:05:18Z` |
| GREEN | Executed; remediation checks passed | `2026-11-13T02:05:22Z` |
| COMMIT | Executed; pass | `2026-11-13T02:06:59Z` |

The review treats only these recorded events as the replacement remediation's
phases and makes no claim that this reviewer or crafter inherited or reran
earlier step phases. The remediation record names the reproducing RED failure
(`Standalone` dead-code lint in the non-test library) and the focused GREEN
evidence.

| Check | Result |
|---|---|
| Focused strict Clippy: `cargo clippy -p overdrive-control-plane --all-targets --features integration-tests,kvm-tests -- -D warnings` via canonical Metal foreground | **PASS**, recorded Metal run `b3116b9963afeb5071cfbc4a`. |
| Five D17 convergence-loop wake tests via `cargo nextest run -p overdrive-control-plane --lib --features integration-tests -E 'test(/convergence_loop_wake::/)'` and canonical Metal foreground | **PASS**, recorded Metal run `42302d54ea1a3dca6b4d5f79`. |
| Full-workspace strict Clippy | Reached a separate `clippy::doc_markdown` report in `xtask/src/dst_lint.rs`; the 05-02 owner subsequently fixed and independently approved that issue. It is unrelated to this cfg-only remediation and is not a blocking requirement here. |
| Commit scope and attribution | **PASS** — `ff29358a8` changes the action-shim cfg gates and its remediation record; `32a60dd17` changes the step execution log. Both preserve Marcus as Git author and contain exactly `Co-Authored-By: Codex <codex@openai.com>`, plus the expected `Step-Id: 03-02` and task trailer. |
| Commit whitespace | **PASS** — `git show --check` for both commits reported no whitespace errors. |
| Mutation testing | **Not required** — the user's explicit instruction “no mutation testing is required” is recorded in the roadmap as `required: false`, `status: not_required_by_user`, including the D17 mutation-witness requirements. No mutation test or witness is requested by this review. |

The recorded focused checks were not rerun; their Metal run identities and
results are documented, and the source diff is limited to the three cfg
attributes. The full-workspace Clippy result is not promoted into a blocker
because the only reported issue is an unrelated scanner lint already fixed
and independently approved.

### Finding and remediation disposition

| Review item | Current disposition |
|---|---|
| F-01 — `.claude/rules/development.md` described the pre-D17 clock-only tick and direct production submit surface | **Closed in iteration 2; remains closed.** |
| Final Clippy failure — feature-enabled non-test library exposed an unconstructed private `Standalone` variant | **Remediated and verified.** Its enum variant, import, and match arm are now gated by `cfg(test)`; source-local test constructors remain available. |

No new blocking or advisory finding was found in this final remediation review.
The user-waived mutation gate creates no outstanding step requirement.
Reviewer write scope is this review artifact only; no implementation, test,
model, roadmap, evidence, DES, or commit changes were made by the reviewer.

## Final verdict

**APPROVED — iteration 3.** The private standalone broker adapter is present
only in test builds, all existing constructor owners remain test-gated, and
the strict Clippy failure is resolved without changing D17 runtime behavior
or API. F-01 remains closed. The user-waived final mutation gate is recorded
as not required.
