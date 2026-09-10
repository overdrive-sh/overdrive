# Adversarial review — step 01-01

## Current review state

| Field | Value |
|---|---|
| Latest iteration | 6 |
| Current reviewed commit | `1918489f2c5b1592276d1155e98c1012cffe95e1` (DES persistence `f628b95f26767f822d0ff7550db81cbade202c72`) |
| Current verdict | **APPROVED** |
| Open findings | 0 (F1-F6 closed) |

## Iteration 1 metadata

| Field | Value |
|---|---|
| Feature | `vm-lifecycle-latency` |
| Step | `01-01` — Convergence ownership |
| Base | `7378fbd32a6082e7637feac2fa5df9f25d6d3a47` |
| Implementation commit | `083ef6e6ac6e29843e741df05868029385b42aea` |
| DES-log completion commit | `f3c6250faf96c237de2ed14104838c0cfdd0594e` |
| Reviewer | `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated selection) |
| Review date | 2026-09-10 |
| Iteration | 1 |
| Verdict | **CHANGES_REQUESTED** |

## Executive summary

The bounded convergence owner, exact ADR-0102 public signatures, broker FIFO/age
policy, target lease, admission close, active-result drain, locked
`pending_at_exit` snapshot, and owner tracing are implemented coherently. The
step's seeded production-server suite passes 12/12, the selected preservation
suite passes 37/37, E10 preservation passes 2/2, and the full workspace/all-target
check passes. The S06a tests now match all eight target/tick admissions to the
Driver ledger, shim-authored rows, owner-consumed completions, and sole drain
event. S06b observes one locked broker snapshot and excludes a submission
linearized after it. No wire shape, `Evaluation`, `BrokerCounters`,
`run_convergence_tick`, or `spawn_convergence_loop` signature changed.

Two bounded defects prevent approval. One transitioned broker property is
missing its mandatory per-test Contract Shape declaration. Separately, the
commit reverses the production exit observer's existing biased cancellation
priority even though ADR-0102 explicitly preserves that priority and declines
an unread-queue drain guarantee. The latter is an exact design/scope mismatch,
not a proposed new shutdown mechanism or an allegation based on an imagined
failure schedule.

## Contract and review scope

The review used these authoritative sources:

- `docs/product/architecture/adr-0102-bounded-convergence-evaluation-ownership.md`
- the VM-lifecycle amendment in `docs/product/architecture/brief.md`
- `docs/feature/vm-lifecycle-latency/feature-delta.md`, including the Remaining
  S06 owner oracle
- step `01-01` in `docs/feature/vm-lifecycle-latency/deliver/roadmap.json`
- `.claude/rules/testing.md`, `.claude/rules/development.md`, and the remaining
  repository-mandated project rules

The committed diff contains 26 files, 1,599 insertions, and 322 deletions. The
broader file set is predominantly exact signature fallout in production callers,
tests, and the Sim harness. The review excluded and preserved the unrelated dirty
`AGENTS.md` plus later-step work in
`crates/overdrive-worker/tests/acceptance/vm_driver_stop_totality.rs` and
`crates/overdrive-cli/tests/integration/vm_stop_restart_and_vmm_death.rs`.
This reviewer wrote only this artifact.

## Inspected implementation evidence

### Exact public contract

| Contract | Result | Evidence |
|---|---|---|
| `EvaluationBroker::submit(&mut self, Evaluation, Instant)` | PASS | `crates/overdrive-core/src/eval_broker.rs:94-106` |
| `EvaluationBroker::drain_pending(&mut self, usize, &BTreeSet<TargetResource>, Instant) -> Vec<(Evaluation, Duration)>` | PASS | `crates/overdrive-core/src/eval_broker.rs:111-140` |
| Both `InterestRouterBroker` constructors receive `Arc<dyn Clock>` | PASS | `crates/overdrive-control-plane/src/lib.rs:3510-3532` |
| `enqueue_evaluation::dispatch` receives `Instant` | PASS | `crates/overdrive-control-plane/src/action_shim/enqueue_evaluation.rs:51-59` |
| `Evaluation` and `BrokerCounters` shape unchanged | PASS | Base-to-target diff changes neither public struct's fields nor derives. |
| Wire shapes unchanged | PASS | No wire/schema file is in the step diff. |
| `run_convergence_tick` and `spawn_convergence_loop` signatures unchanged | PASS | The owner body changed; neither signature did. |
| No compatibility submit/drain method or new public scheduling type | PASS | Complete production diff audit found none. |

### Broker and owner behavior

`EvaluationBroker` retains the first timestamp and FIFO ordinal when a pending
key is replaced, admits by the private ordinal, skips externally blocked targets,
adds every admitted target to the per-result unavailable set, and increments
`dispatched` only by actual admissions (`eval_broker.rs:94-139`). Its private
metadata adds no persisted or wire shape.

The convergence owner keeps at most eight futures in one `FuturesUnordered`,
tracks exact active `TargetResource`s, admits only free capacity, assigns one
monotonic tick per evaluation, and retains the target until the complete
`run_convergence_tick` result is yielded and consumed (`lib.rs:3321-3457`). On
cancellation it closes admission, records the eight active owners, continues to
consume success and `ConvergenceError` results, takes one broker-locked queued
snapshot after the active set becomes empty, emits the one drain event after the
guard is released, and returns. Immediate refill occurs on the next owner-loop
turn after a completion; no cadence sleep is interposed.

### Acceptance-test honesty

- The 12 S-VLL tests use the real in-process server, HTTPS handlers, broker,
  convergence owner, runtime, durable View path, and action shim. Only external
  Driver delay/failure and simulated time are injected.
- S06a compares all eight admissions and completions by exact
  `(reconciler, target, tick)`, matches Driver return and shim-row sets after each
  release, checks shutdown remains pending until the final result, and proves the
  ninth target crosses none of admission, completion, Driver, or allocation-row
  boundaries (`vm_lifecycle_latency_283_spike.rs:590-791`).
- S06b gates the trace callback after the owner has read the broker snapshot,
  submits through HTTPS while the event call is blocked, and proves the late
  target is excluded from the unchanged report and every execution surface
  (`vm_lifecycle_latency_283_spike.rs:1214-1409`).
- The two source-local S-VLL-05 properties use the exact
  `/// CONTRACT_SHAPE: pure-function.` declaration, generated operation traces,
  mandatory 0/1/7/8/9 limit boundaries, none/some/all blocked masks,
  before/equal/after clock values, complete counter/reap comparisons, and a
  fairness witness that cannot be satisfied by lexical key order
  (`eval_broker.rs:239-497`).
- The preservation selection retains View write-before-dispatch, unchanged-View
  elision, error re-enqueue, LWW/session, and reclamation evidence. No expectation
  runner was absorbed into the Rust tests.

## Contract Shape Compliance

**Overall: FAIL.**

| Check | Result | Evidence |
|---|---|---|
| Per-test Contract Shape declaration | **BLOCKER** | F1: transitioned property at `eval_broker_collapse.rs:329-349` has no declaration. |
| Exact source-local pure-property spelling | PASS | All live source-local properties in `eval_broker.rs` use exactly `/// CONTRACT_SHAPE: pure-function.`. |
| Bounded-change delta/complement | PASS for mapped S-VLL bodies | The server tests assert their complete declared target/View/row/Driver/trace universes, including negative complements. |
| Pure-function property mechanism | PASS | Generated traces and explicit boundary partitions compare full outputs and state deltas. |
| Banned implementation-detail test-name pattern | PASS | No step-owned test matches the reviewer mandate's banned regex. |
| Outcome-anchor generic check | Repository-specific contract governs | The accepted feature delta and repository rule for this feature mandate the per-test Contract Shape declaration and do not define an exact outcome-anchor string. No new string was invented during review. |

The mapped feature has nine scenario-contract identities. Twelve server tests
cover required start/stop and healthy partitions, and two source-local properties
cover S-VLL-05a/b. Fourteen focused bodies remain below the `2 × 9 = 18` review
budget; preservation tests are existing independent evidence and are not counted
as newly authored behavior tests.

## Findings

### F1 — transitioned broker property lacks its per-test Contract Shape declaration

- **Severity:** Blocker
- **Dimension:** Mechanical Contract Shape compliance
- **Location:** `crates/overdrive-control-plane/tests/acceptance/eval_broker_collapse.rs:329-349`
- **Governing contract:** `feature-delta.md:312-314` and `AGENTS.md:258-260`
- **Reachability:** Not applicable to production behavior. This is mandatory
  executable-test metadata, so the defect is present whenever the transitioned
  property is compiled or reviewed.
- **Reproducer:** Direct diff/source audit. Commit `083ef6e6` transitions
  `duplicate_evaluations_collapse_invariant_holds_after_every_submit` to the new
  timestamped `submit` API, but the `#[test]` at line 329 has no preceding
  `CONTRACT_SHAPE` rustdoc. The focused preservation command executed this live
  property successfully; a green body does not supply the missing declaration.
- **Remediation disposition:** OPEN. Add the exact bounded broker declaration
  `/// CONTRACT_SHAPE: bounded-change.` immediately before this transitioned
  acceptance property. Do not change its behavior, add a compatibility API, or
  broaden its oracle.

### F2 — exit-observer cancellation priority changes despite an explicit preserve contract

- **Severity:** Blocker
- **Dimension:** Exact DESIGN conformance and scope control
- **Locations:**
  - `crates/overdrive-control-plane/src/worker/exit_observer.rs:199-212`
  - `crates/overdrive-control-plane/src/lib.rs:1446-1459`
  - ADR-0102 lines 175-181
  - `feature-delta.md:79,128,306,443`
- **Production entry point and owner path:** `run_server_with_obs_and_driver`
  creates one observer per registered Driver through
  `spawn_with_runtime` (`lib.rs:3018-3050`). `ServerHandle::shutdown` cancels the
  shared observer token and awaits every observer task (`lib.rs:1446-1459`). The
  observer's sole receive owner is the biased `tokio::select!` at
  `exit_observer.rs:199-212`.
- **Exact state and ordering:** In the base, cancellation is the first biased
  arm, followed by `rx.recv()`. The reviewed commit moves `rx.recv()` first. If
  cancellation and an unread queued event are both ready, the new ordering
  selects the unread event; on the next loop it again prefers another queued
  event. That is the opposite of the retained cancellation-first selection. The
  current shutdown comment at `lib.rs:1446-1455` still describes
  cancellation-biased exit.
- **Current shutdown/cancellation/retry behavior:** An event already selected
  before cancellation continues through the existing four-attempt retry owner,
  because cancellation is not selected inside `run_with_retry`. That already
  satisfies the accepted consumed-event promise. The committed arm reversal
  additionally changes which unread event becomes consumed after cancellation.
- **Reproducer:**
  `git diff 7378fbd32a6082e7637feac2fa5df9f25d6d3a47 083ef6e6 -- crates/overdrive-control-plane/src/worker/exit_observer.rs`
  deterministically shows the cancellation-first arm removed and the receive
  arm placed first. This is a source-level preservation/scope failure, not a
  hypothetical runtime-hang claim; no new timing defect or architecture is
  inferred. The passing S06b test does not require the change: it waits for all
  four retries at lines 1298-1333 before releasing the remaining seven active
  convergence results, while observer cancellation occurs only later in
  `ServerHandle::shutdown`.
- **Remediation disposition:** OPEN. Restore the existing cancellation-first
  biased arm order and remove/update the contradictory new comment. Retain the
  existing behavior in which an event selected before cancellation finishes its
  retry sequence. Do not add an observer drain protocol, new barrier, new API,
  or persistence/recovery mechanism.

## Non-findings and rejected hypotheses

- No finding is raised for a pre-cancelled convergence token admitting work
  before its first `select!`: no seeded production-owner invariant was supplied
  or failed for that hypothesized startup race, so repository reachability rules
  prohibit promoting it.
- No finding is raised for active-future cancellation or detached evaluation
  work. The convergence task owns the futures directly and `ServerHandle`
  awaits that owner; no per-evaluation Tokio task was introduced.
- No finding is raised for View, LWW/session, or reclamation behavior. The
  selected preservation suites remain green and the reviewed owner retains the
  full runtime result as its lease boundary.
- The broad caller/test fallout is necessary for the exact atomic signature cut.
  No new public method, type, enum variant, trait, parameter beyond the five
  approved signatures, or compatibility path was found.

## Verification results

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests --test vm_lifecycle_latency_283_spike --no-fail-fast` | PASS — 12/12, nextest run `0d9637b5-6821-48f9-95a2-c70f26794bf4` |
| `cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-control-plane --features integration-tests -E 'test(bounded_admission_contract) or test(eval_broker_collapse) or test(reconciler_runtime_view_store) or test(runtime_convergence_loop) or test(vm_reclamation_claim_lifecycle)'` | PASS — 37/37, nextest run `3c0fb776-b55d-4887-bd01-16fdf2c92c84` |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests --test e10_vm_early_exit_spike` | PASS — 2/2 |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `git diff --check 7378fbd3..f3c6250f` | PASS |

The implementation and DES commits both have the exact required
`Co-Authored-By: Codex <codex@openai.com>` trailer and `Step-Id: 01-01`.
`execution-log.json` schema 3.0 records chronological successful RED, GREEN,
GREEN, GREEN, and COMMIT events for `01-01`. No per-step mutation run or mutation
configuration change is present.

## Iteration disposition

| Finding | Status | Required next action |
|---|---|---|
| F1 | OPEN | Add the one exact per-test bounded-change declaration. |
| F2 | OPEN | Restore the existing cancellation-first observer arm order; do not expand the mechanism. |

## Final verdict

**CHANGES_REQUESTED.** Return both findings to the original step-01-01 crafter.
After the tightly bounded remediation is committed, this same step-specific
reviewer must re-review the new target and append iteration 2 to this artifact.
Step 01-02 must not start until the verdict is **APPROVED**.

---

## Iteration 2 — remediation re-review

### Metadata

| Field | Value |
|---|---|
| Review date | 2026-09-11 |
| Remediation commit | `78f189b743f1e55c32c1c00fc4024b52dd3d18e6` |
| Parent | `f3c6250faf96c237de2ed14104838c0cfdd0594e` |
| Cumulative implementation | `7378fbd32a6082e7637feac2fa5df9f25d6d3a47..78f189b743f1e55c32c1c00fc4024b52dd3d18e6` |
| Reviewer | Same step-specific `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated selection) |
| Verdict | **APPROVED** |

### Remediation scope and integrity

The remediation is exactly two files: two insertions and six deletions. It adds
the one missing test-metadata line and restores the exit-observer source to its
pre-step cancellation ordering. There is no public API, type, enum, trait,
parameter, wire, schema, persistence, test seam, lifecycle mechanism, roadmap,
DES-log, or mutation-configuration change.

Commit `78f189b7` has parent `f3c6250f`, retains Marcus Schack Abildskov as both
author and committer, and contains exactly the required
`Co-Authored-By: Codex <codex@openai.com>` trailer plus `Step-Id: 01-01`. The
remediation diff passes `git diff --check`. The existing unrelated dirty
`AGENTS.md` and later-step acceptance files were preserved; this reviewer again
changed only this Markdown review artifact.

### Prior-finding dispositions

#### F1 — RESOLVED

`crates/overdrive-control-plane/tests/acceptance/eval_broker_collapse.rs:329`
now places the exact `/// CONTRACT_SHAPE: bounded-change.` rustdoc immediately
before the transitioned property's `#[test]` attribute. Its generated input,
broker calls, and assertions are otherwise unchanged. The focused preservation
selection executes the property and passes.

**Disposition:** Closed. The mandatory per-test Contract Shape declaration is
present with the correct bounded broker classification; no compatibility API or
oracle expansion was introduced.

#### F2 — RESOLVED

`crates/overdrive-control-plane/src/worker/exit_observer.rs:199-208` again has
the exact retained biased owner order:

1. `shutdown_token.cancelled()` first;
2. `rx.recv()` second.

The explanatory text introduced with the reversed order is removed. A cumulative
diff of this file from base `7378fbd3` through remediation `78f189b7` is empty,
so the step no longer changes exit-observer production behavior at all.

The accepted consumed-event behavior also remains intact. Once the receive arm
has selected an event, `run_with_retry` executes outside the `tokio::select!` at
line 208; later token cancellation does not interrupt that event's existing
four-attempt retry owner. The complete 12-test S-VLL suite, including S06b's
50/100/200 ms backoff and fourth-attempt assertions, passes with cancellation
priority restored. Thus the remediation preserves both halves of ADR-0102:
finish an already consumed event, but do not promise to consume unread queued
events after shutdown cancellation.

**Disposition:** Closed. Exact DESIGN preservation is restored without adding a
drain protocol, barrier, API, persistence owner, or recovery mechanism.

### Cumulative contract check

| Check | Iteration 2 result | Evidence |
|---|---|---|
| Exact five ADR-0102 public signature changes only | PASS | Remediation contains no API change; iteration-1 cumulative audit remains valid. |
| `Evaluation`, `BrokerCounters`, wire, `run_convergence_tick`, and `spawn_convergence_loop` signatures unchanged | PASS | No remediation change to these surfaces. |
| Eight complete evaluations, target exclusion, FIFO age, immediate refill | PASS | Seeded S-VLL suite 12/12 and preservation suite 37/37. |
| Admission close, active-result drain, owner snapshot, observer ordering | PASS | S06a/S06b remain green; cancellation-first source equals base. |
| S-VLL-13 View, re-enqueue, LWW/session, reclamation preservation | PASS | Selected preservation suite remains 37/37; remediation does not touch those owners. |
| Contract Shape Compliance | PASS | F1 exact declaration present; all iteration-1 mapped checks remain satisfied. |
| Test integrity | PASS | One metadata addition only; no assertion, fixture, input domain, expected value, test attribute, or scenario body changed. |
| Scope control | PASS | Both changes are direct, minimal dispositions of F1/F2. |

No new finding is proven. The remediation does not reach any new production
path, and the cumulative implementation paths affected by its two changes are
fully covered by the prior review plus the focused reruns below. The unproven
hypotheses rejected in iteration 1 remain rejected; this review does not turn
them into requirements.

### Iteration 2 verification

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests --test vm_lifecycle_latency_283_spike --no-fail-fast` | PASS — 12/12, nextest run `2ab31097-7d4e-45be-a7f4-38712bb2e94b` |
| `cargo xtask lima run -- cargo nextest run -p overdrive-core -p overdrive-control-plane --features integration-tests -E 'test(bounded_admission_contract) or test(eval_broker_collapse) or test(reconciler_runtime_view_store) or test(runtime_convergence_loop) or test(vm_reclamation_claim_lifecycle)'` | PASS — 37/37, nextest run `a3d85397-8521-47e9-b3fd-e6220215e8c8` |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo xtask lima run -- cargo fmt --all -- --check` | PASS |
| `cargo xtask lima run -- cargo clippy -p overdrive-control-plane --features integration-tests --test acceptance -- -D warnings` | PASS |
| `git diff --check f3c6250f..78f189b7` | PASS |
| `git diff --exit-code 7378fbd3..78f189b7 -- crates/overdrive-control-plane/src/worker/exit_observer.rs` | PASS — empty cumulative diff |

### Iteration history

| Iteration | Reviewed commit | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `083ef6e6` plus `f3c6250f` DES completion | **CHANGES_REQUESTED** | 2 | F1 and F2 returned to the original step crafter. |
| 2 | `78f189b7` | **APPROVED** | 0 new; 2 prior closed | F1 exact declaration present; F2 exact cancellation-first preservation restored. |

### Final verdict after iteration 2

**APPROVED.** No required remediation remains for step `01-01`. The exact
ADR-0102 API and owner contract, mapped acceptance bodies, Contract Shape
metadata, observer shutdown semantics, preservation suites, commit scope, and
attribution all pass. Step `01-01` may advance.

---

## Iteration 3 — post-approval affected-suite failure

### Metadata

| Field | Value |
|---|---|
| Review date | 2026-09-11 |
| Reopened target | `78f189b743f1e55c32c1c00fc4024b52dd3d18e6` |
| Trigger | Pre-commit affected-test gate failed before any `01-02` commit |
| Reviewer | Same step-specific `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated selection) |
| Verdict | **CHANGES_REQUESTED** |

### Reproduction

The reported full affected-test command was:

`cargo xtask lima run -- cargo nextest run -E 'rdeps(overdrive-cli) | rdeps(overdrive-init) | rdeps(overdrive-worker)'`

This review reproduced the two failures with the bounded focused command:

`cargo xtask lima run -- cargo nextest run -p overdrive-control-plane -E 'test(execute_reclaim_allocation_authorised_kills_discards_writes_and_submits_three_evaluations) or test(execute_reclaim_allocation_terminal_and_non_terminal_rows_are_not_interchangeable)' --no-fail-fast`

Nextest run `290d6211-6a26-4d11-a413-4b1cd1b47cc6` ran two tests and failed both:

- `execute_reclaim_allocation_authorised_kills_discards_writes_and_submits_three_evaluations`
  observed one admitted reconciler (`workload-lifecycle`) at
  `reclamation.rs:549`, while its assertion expected all three in one drain.
- `execute_reclaim_allocation_terminal_and_non_terminal_rows_are_not_interchangeable`
  observed a first-drain length of one at `reclamation.rs:715`, while its
  assertion expected three.

The independent broker-policy complement passed:

`cargo xtask lima run -- cargo nextest run -p overdrive-control-plane -E 'test(drain_pending_is_deterministic_across_two_brokers) or test(submits_with_same_target_different_reconciler_dont_collapse)'`

Nextest run `cd84670c-b613-4251-aa47-9b04ea22c94c` passed 2/2. It proves the
two relevant halves separately: same-target/different-reconciler submissions
remain distinct pending keys, and only one such key is admitted in a round while
the other remains pending for a later round.

### Production caller and owner analysis

This is not a production loss or coalescing defect.

The real path is:

1. `VmReclamation::plan_reclamation` emits `Action::ReclaimAllocation` for an
   authorised non-terminal VM allocation
   (`crates/overdrive-reconcilers/src/vm_reclamation.rs:175-180`).
2. The runtime dispatches that action through the production action shim, whose
   `Action::ReclaimAllocation` arm calls `execute_reclaim_allocation`
   (`crates/overdrive-control-plane/src/action_shim/mod.rs:3076-3090`).
3. After the supervision lease, terminality guard, host cleanup, and durable
   terminal write, the executor submits exactly three evaluations under one
   broker lock: `workload-lifecycle`, `service-lifecycle`, and `svid-lifecycle`,
   all for the same `workload/<id>` target
   (`action_shim/reclamation.rs:246-273`).
4. `EvaluationBroker::submit` keys pending work by
   `(ReconcilerName, TargetResource)` (`eval_broker.rs:94-105`). Because the
   three reconciler names differ, all three entries remain pending; none is a
   duplicate and `cancelled` does not increase.
5. ADR-0102 requires targets within one admission result to be distinct. During
   `drain_pending`, the first eligible entry is removed and its target is added
   to the round-local blocked set; the two later keys with that same target are
   skipped without removal (`eval_broker.rs:108-139`). Thus the first call
   returns one evaluation and retains two pending entries.
6. The production convergence owner also supplies its active target set to that
   drain (`lib.rs:3357-3371`), inserts the admitted target before running the
   complete evaluation (`lib.rs:3373-3403`), and removes it only after consuming
   the full result (`lib.rs:3432-3451`). The next owner-loop turn then admits the
   next same-target reconciler, followed by the third after the second lease is
   released. Under ordinary continued service they execute sequentially; on
   shutdown, nonadmitted entries remain pending by the separately accepted
   admission-close contract.

The exact deterministic state transition is therefore:

| Point | Pending | Admitted result | Queued | Dispatched | Cancelled |
|---|---|---|---:|---:|---:|
| After authorised reclamation submits | workload + service + svid for one target | none | 3 | 0 | 0 |
| First empty-block-set drain | service + svid | workload | 2 | 1 | 0 |
| After first lease release, second drain | svid | service | 1 | 2 | 0 |
| After second lease release, third drain | empty | svid | 0 | 3 | 0 |

The tests' one-shot `drain_pending(usize::MAX, empty, now)` calls conflate
submission/pending identity with one target-exclusive admission round. The
`usize::MAX` limit does not override target exclusion. The failure is fully
explained by the pure deterministic broker policy; no timing, ordering race,
crash/restart, retry, or convergence schedule is alleged. The repository's
seeded-Sim prerequisite for a control-plane timing finding is therefore not
triggered, and production must not be changed to manufacture three concurrent
same-target admissions.

### Finding F3 — step-transitioned reclamation tests retain the superseded drain-all oracle

- **Severity:** Blocker
- **Dimension:** Test integrity, exact ADR-0102 semantics, and affected-suite gate
- **Locations:**
  - `crates/overdrive-control-plane/src/action_shim/reclamation.rs:539-562`
  - `crates/overdrive-control-plane/src/action_shim/reclamation.rs:715-722`
- **Production reachability:** The complete production caller/owner path is
  established above. It submits all three distinct keys and later admits them
  sequentially under the shared `TargetResource` lease.
- **Failure reproduction:** Focused Lima nextest run
  `290d6211-6a26-4d11-a413-4b1cd1b47cc6`, 0/2 passed. The assertion failures are
  exact and repeat the pre-commit affected-suite result.
- **Root cause:** Commit `083ef6e6` mechanically converted the two existing
  assertions from old `drain_pending()` to ADR-0102's new
  `drain_pending(usize::MAX, empty, now)` without transitioning their oracle.
  Before ADR-0102, drain emptied every pending key. Under ADR-0102, each drain
  returns distinct targets, so three reconciler keys sharing one target require
  three admission rounds. The production submit code itself changed only to
  supply the required timestamp and still submits the same three named keys.
- **Consequence:** The implementation's affected suite is red, so iteration 2's
  approval cannot stand. Changing production to make the tests green would
  violate ADR-0102's exact same-target exclusion and could overlap three
  reconcilers over one workload View/target.
- **Correct remediation owner:** This is an **acceptance/test-oracle correction,
  not an original-crafter production correction**. The test owner must transition
  the assertions to ADR-0102 semantics. Under the repository's same-step
  remediation protocol, the original `01-01` crafter may integrate/commit that
  bounded test correction, but must not alter production behavior or public API;
  test-design ownership remains with the acceptance-test owner.
- **Required bounded correction:** Preserve the full reclamation contract while
  distinguishing pending from admission. Immediately after the authorised
  executor returns, assert the broker has three pending distinct reconciler keys
  and no cancellation; then observe them through three target-exclusive
  admission rounds (or an equivalently complete public-boundary oracle), proving
  the exact three names, common target, queued/dispatched deltas, and no loss.
  For the terminal/non-terminal contrast, assert the authorised broker contains
  three pending entries and the refused broker contains zero; do not require one
  admission result to contain duplicate targets. No new broker accessor, test
  seam, method, type, scheduling lane, or production behavior is authorized.
- **Disposition:** OPEN.

### Prior findings and cumulative implementation

F1 and F2 remain closed. The exact Contract Shape declaration is still present,
and the exit observer remains cancellation-first with an empty cumulative diff
from the pre-step base. No new public API or production defect is proven. The
convergence owner, broker admission policy, S06 shutdown oracle, and S-VLL-13
reclamation intent are mutually consistent; only the two step-transitioned
source-local assertions are stale.

### Iteration history

| Iteration | Reviewed target | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `083ef6e6` plus `f3c6250f` DES completion | **CHANGES_REQUESTED** | F1, F2 | Returned to original step crafter. |
| 2 | `78f189b7` | **APPROVED** | 0 new; F1/F2 closed | Superseded after the broader affected-suite gate exposed F3. |
| 3 | `78f189b7` | **CHANGES_REQUESTED** | F3 | Acceptance/test-oracle correction required; production/API change prohibited. |

### Final verdict after iteration 3

**CHANGES_REQUESTED.** Step `01-01` is reopened with one blocking test-integrity
finding. The implementation must not be changed to admit same-target evaluations
together. Correct the two stale reclamation assertions at their existing public
broker boundary, rerun the focused failures plus the complete affected gate, and
return to this reviewer for iteration 4. Step `01-02` must remain uncommitted
until step `01-01` is again **APPROVED**.

---

## Iteration 4 — distinct-period cadence affected-suite failure

### Metadata

| Field | Value |
|---|---|
| Review date | 2026-09-11 |
| Committed target | `78f189b743f1e55c32c1c00fc4024b52dd3d18e6` |
| Additional reviewed state | Uncommitted F3 test-only correction in `action_shim/reclamation.rs` |
| Trigger | Next failure exposed by the required full affected-test gate |
| Reviewer | Same step-specific `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated selection) |
| Verdict | **CHANGES_REQUESTED** |

### F3 status carried into iteration 4

The uncommitted F3 correction is bounded to the two stale reclamation assertions.
It now distinguishes three pending `(ReconcilerName, TargetResource)` intentions
from three sequential same-target admission rounds, and distinguishes an
authorised broker with `queued = 3` from a refused broker with `queued = 0`.
Per the orchestrator's retained evidence, focused reclamation is 2/2, adjacent
broker policy is 2/2, S-VLL is 12/12, preservation is 37/37, E10 is 2/2, and
workspace check, format, clippy, and diff checks pass.

**Disposition:** F3 is REMEDIATED IN THE WORKTREE but remains open until its
test-only correction is included in the bounded step-01-01 remediation commit.
No production change is present or required.

### Iteration 4 reproduction

The next affected-suite failure was independently reproduced with:

`cargo xtask lima run -- cargo nextest run -p overdrive-control-plane -E 'test(s_266_05_distinct_periods_fire_independently_over_60s)' --no-fail-fast`

Nextest run `eebefd2d-c9ae-4b7d-ac9b-b8bdce9b275d` ran the one test and failed:

```text
Y (30s period) fires 2 times over 60s
left: 1
right: 2
```

The complete focused cadence selection was also run:

`cargo xtask lima run -- cargo nextest run -p overdrive-control-plane -E 'test(cadence_resync)' --no-fail-fast`

Nextest run `7a8e9379-13be-4edc-8fa9-5b1975f4f145` ran six tests: five passed and
only `s_266_05_distinct_periods_fire_independently_over_60s` failed with the same
`Y = 1` observation. The adjacent single-schedule cadence properties
`s_266_02_k_periods_yield_exactly_k_broker_routed_submits` and
`s_266_03_fires_once_per_period_never_per_tick` independently passed 2/2 in
nextest run `d84c21da-9c2f-493e-b394-59eecbd34192`.

### Exact cadence, broker, and production-owner path

This failure is another stale test oracle, not a production cadence defect.

The tested and production-composed path is:

1. `build_cadence_table` retains one declared schedule per reconciler
   (`crates/overdrive-control-plane/src/lib.rs:3206-3221`).
2. `arm_next_wake` independently initializes each reconciler to
   `registration_time + its_period` (`lib.rs:3223-3237`).
3. `due_resync_evaluations` iterates every schedule, emits one evaluation when
   its own `next_wake <= now`, and re-arms that reconciler from its prior wake by
   its own period (`lib.rs:3239-3278`). Every emitted evaluation goes through the
   same broker submission path in the production convergence owner
   (`lib.rs:3353-3371`).
4. The broker retains pending identity by
   `(ReconcilerName, TargetResource)`, so cadence-X and cadence-Y do not coalesce.
   ADR-0102 separately requires one admitted evaluation per exact target in a
   result; after the first admission, later keys sharing `node/nfive` remain
   pending (`crates/overdrive-core/src/eval_broker.rs:94-139`).
5. The production owner keeps `node/nfive` in `active_targets` until the complete
   evaluation result is consumed, then removes the target and immediately loops
   to refill the released capacity (`lib.rs:3373-3403,3424-3454`).

The unchanged S-266-05 test declares X at 10 seconds and Y at 30 seconds, then
advances a logical clock one second at a time through the inclusive 60-second
boundary (`crates/overdrive-control-plane/tests/acceptance/cadence_resync.rs:203-249`).
Its exact schedule is:

| Logical second | Due submissions | First target-exclusive admission | Pending after first round |
|---:|---|---|---|
| 10 | X | X | empty |
| 20 | X | X | empty |
| 30 | X, Y | X | Y |
| 31 | none | Y | empty |
| 40 | X | X | empty |
| 50 | X | X | empty |
| 60 | X, Y | X | Y |

Thus the cadence helper emits exactly the accepted six X submissions and two Y
submissions. At the two coincident boundaries, FIFO admits X first because it
was inserted first; Y is a distinct pending key, not lost or cancelled. The test
observes the first deferred Y at second 31, but stops immediately after the first
drain at second 60 and never supplies the subsequent lease-release/admission
round that would observe the second Y. It then labels admitted counts as
"fires", conflating the declarative due/submission contract with completion of
one target-exclusive admission result.

The externally relevant production outcome remains correct: both schedules fire
at their declared boundaries and both broker intentions survive; evaluations
for the shared node target execute sequentially after each complete lease is
released. ADR-0102 does not promise that two reconcilers targeting the same node
are admitted concurrently or that both complete before the exact 60-second clock
instant. On shutdown, a still-pending Y is governed by the explicitly accepted
nonadmitted-work disposition rather than being reported as completed.

This analysis accepts no timing or liveness finding against production. The
red test drives pure cadence helpers and a broker manually; it does not drive the
real asynchronous convergence owner or model evaluation completion. Therefore
it cannot prove a production control-plane schedule failure. Under the
repository rule, any future claim that the real owner fails to make the deferred
Y evaluation progress must first fail a seeded `overdrive-sim` liveness
invariant through that owner and print its seed. No such production failure is
present here, so production remediation is prohibited.

### Finding F4 — distinct-period cadence test counts only first-round admissions as fires

- **Severity:** Blocker
- **Dimension:** Test integrity, cadence semantics, and exact ADR-0102 target exclusion
- **Location:** `crates/overdrive-control-plane/tests/acceptance/cadence_resync.rs:203-249`
- **Production entry point and owner:** The exact table → next-wake → due
  evaluation → broker → convergence-owner path is cited above. It submits X and
  Y independently and admits shared-target work sequentially.
- **Reproducer:** Focused Lima run
  `eebefd2d-c9ae-4b7d-ac9b-b8bdce9b275d`, 0/1; complete cadence selection
  `7a8e9379-13be-4edc-8fa9-5b1975f4f145`, 5/6 with only this test red.
- **Root cause:** Commit `083ef6e6` mechanically replaced old
  `drain_pending()` with one
  `drain_pending(usize::MAX, empty, now)` call per logical second. The old call
  emptied all pending keys; the ADR-0102 call returns distinct targets and
  retains Y when X and Y share `node/nfive`. The test ends at second 60 before a
  second round can admit the retained Y.
- **Consequence:** The required affected suite remains red, invalidating the
  prior approval. Making the first drain return X and Y would violate the
  accepted one-target lease and could overlap reconcilers against the same node
  target.
- **Correct remediation owner:** This is an **acceptance-test/oracle correction,
  not a production correction**. The acceptance-test owner must preserve the
  S-266-05 independent-period contract while representing the new admission
  boundary. The original step crafter may integrate and commit the bounded test
  correction under same-step remediation protocol, but must not alter cadence,
  broker, convergence-owner, or public API behavior.
- **Required bounded correction:** Separate the two due/submission counts from
  admission counts, and prove the helper emitted X six times and Y twice. Then
  give every retained same-target key a later admission round after the modeled
  lease release (including the Y left at second 60), proving all eight submitted
  keys are eventually admitted, none is cancelled, and no round contains the
  target twice. An equivalent complete public-boundary oracle is acceptable.
  Because this step transitions the test's semantic oracle, retain/add its
  required per-test `CONTRACT_SHAPE` declaration. Do not add a broker accessor,
  compatibility drain, priority lane, reserved cadence slot, or test-only
  production seam.
- **Disposition:** OPEN.

### Iteration 4 cumulative disposition

| Finding | Status | Owner/action |
|---|---|---|
| F1 | CLOSED | Exact Contract Shape declaration remains present. |
| F2 | CLOSED | Exit observer remains cancellation-first. |
| F3 | REMEDIATED, UNCOMMITTED | Include the already-green test-only reclamation correction in the bounded remediation commit. |
| F4 | OPEN | Acceptance-test owner corrects the distinct-period submission/admission oracle; no production change. |

No new production defect is proven. All cumulative API and owner conclusions
from iterations 1-3 remain unchanged. The new evidence expands only the set of
step-transitioned tests whose old drain-all assumptions must be migrated.

### Iteration history

| Iteration | Reviewed state | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `083ef6e6` plus `f3c6250f` DES completion | **CHANGES_REQUESTED** | F1, F2 | Returned to original step crafter. |
| 2 | `78f189b7` | **APPROVED** | 0 new; F1/F2 closed | Superseded by the full affected-suite failures. |
| 3 | `78f189b7` | **CHANGES_REQUESTED** | F3 | Test-only reclamation oracle correction required. |
| 4 | `78f189b7` plus uncommitted F3 correction | **CHANGES_REQUESTED** | F4 | F3 green but uncommitted; cadence test correction remains open. |

### Final verdict after iteration 4

**CHANGES_REQUESTED.** Step `01-01` remains reopened. Correct the stale S-266-05
acceptance oracle without changing production, retain the green F3 correction,
and run both focused cadence evidence and the complete affected-test gate. The
step may return for re-review only after those test-only changes are committed;
step `01-02` must remain uncommitted until `01-01` is again **APPROVED**.

---

## Iteration 5 — complete non-fail-fast affected-suite failure set

### Metadata

| Field | Value |
|---|---|
| Review date | 2026-09-11 |
| Committed target | `78f189b743f1e55c32c1c00fc4024b52dd3d18e6` |
| Additional reviewed state | Uncommitted F3/F4 test-only corrections in `action_shim/reclamation.rs` and `tests/acceptance/cadence_resync.rs` |
| Trigger | Complete `--no-fail-fast` affected-test gate exposed four interest-router failures and one trybuild mismatch |
| Reviewer | Same step-specific `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated selection) |
| Iteration | 5 |
| Verdict | **CHANGES_REQUESTED** |

### Carried remediation state

F3 and F4 are now green in the current worktree. The complete affected run below
executes the current uncommitted corrections and reports neither reclamation nor
cadence among its failures. Source inspection confirms that both changes remain
test-only: F3 asserts the three distinct pending keys and admits them through
three target-exclusive rounds; F4 separately counts six X and two Y cadence
submissions, admits the shared target sequentially, and carries the exact
`/// CONTRACT_SHAPE: bounded-change.` declaration. Neither changes broker,
cadence, reclamation, or convergence-owner production behavior.

**Disposition:** F3 and F4 are REMEDIATED IN THE WORKTREE but remain open until
the bounded step-01-01 remediation commit includes them. F1 and F2 remain
closed.

### Complete affected-suite reproduction

This review independently ran the exact required command:

`cargo xtask lima run -- cargo nextest run --no-fail-fast -E 'rdeps(overdrive-cli) | rdeps(overdrive-init) | rdeps(overdrive-worker)'`

Nextest run `b8bd7a2e-b52e-4914-94e7-37c3334620fe` ran 2,212 tests: 2,207
passed and exactly five failed. The four control-plane failures were:

- `accepted_alloc_change_wakes_exactly_the_current_three_consumers` — one
  first-round admission was compared with three submitted owners
  (`interest_router.rs:852-885`, assertion at line 880).
- `every_accepted_write_wakes_all_current_consumers` — the exact source and
  runner spelling has one `all`; the reported `every_accepted_write_wakes_all_all_current_consumers`
  spelling was not present. Its one drain returned one evaluation per one of
  three targets and therefore lacked `service-lifecycle` for `workload/exitobs`
  (`interest_router.rs:897-931`, assertion at line 925).
- `fan_out_reaches_a_fixpoint_and_does_not_re_wake_forever` — one drain left
  two legitimate same-target pending owners, so the test's immediate
  `queued == 0` assertion failed (`interest_router.rs:946-974`, assertion at
  line 970).
- `interested_reconciler_wakes_on_accepted_alloc_status_change` — the
  two-owner `payments` case returned only `r-a` in the first admission result
  and retained `r-b` (`interest_router.rs:378-414`, assertion at line 410).

The fifth failure was
`overdrive-worker::compile_fail::compile_fail_fixtures`, where trybuild rejected
the expected diagnostic for `exec_driver_missing_fs.rs`. The failure set is
complete because the run used `--no-fail-fast`; no reclamation or cadence test
failed.

The four interest-router failures were also reproduced together with the
bounded focused command:

`cargo xtask lima run -- cargo nextest run -p overdrive-control-plane --no-fail-fast -E 'test(accepted_alloc_change_wakes_exactly_the_current_three_consumers) or test(every_accepted_write_wakes_all_current_consumers) or test(fan_out_reaches_a_fixpoint_and_does_not_re_wake_forever) or test(interested_reconciler_wakes_on_accepted_alloc_status_change)'`

Nextest run `41fc5689-5e02-4a33-a904-7f5fe3418c51` reproduced all four, 0/4.
The focused broker-policy complement
`test(submits_with_same_target_different_reconciler_dont_collapse) or test(drain_pending_is_deterministic_across_two_brokers)`
passed 2/2 in nextest run `ffa4327c-3f77-46ae-b327-41afe252ec9a`.

### Accepted-write, routing, submission, and owner trace

The four failures share one proven stale-oracle root. They do not demonstrate
lost or coalesced router submissions.

1. Each affected test subscribes before spawning the real
   `spawn_interest_router`, then performs an accepted
   `ObservationStore::write_alloc_lifecycle` through `write_alloc`
   (`interest_router.rs:268-274,304-325`). The concrete
   `SimObservationStore` applies the LWW transition, installs the current row,
   and broadcasts exactly the accepted `AllocStatus` row
   (`overdrive-sim/src/adapters/observation_store.rs:311-357,713-729`).
2. The real router receives that `SubscriptionEvent::Row` and calls
   `route_observation_row` (`overdrive-control-plane/src/lib.rs:3729-3735`).
   For `AllocStatus`, it derives the one exact
   `workload/<WorkloadId>` target and loops over every declared interested
   reconciler, submitting one `Evaluation` per name
   (`lib.rs:3547-3594`). Production creates this same interest table, watch,
   restricted broker handle, and convergence owner in
   `run_server_with_obs_and_drivers` (`lib.rs:3066-3107`).
3. `InterestRouterBroker::from_runtime` submits into the runtime's shared
   broker with the captured clock (`lib.rs:3500-3531`). Broker identity remains
   `(ReconcilerName, TargetResource)`, so different interested reconciler names
   for one workload are distinct pending entries; replacement occurs only at
   the complete duplicate key (`overdrive-core/src/eval_broker.rs:90-106`).
4. `drain_pending` sorts by first-submit FIFO, seeds the unavailable set from
   active targets, and adds each admitted target to that set. Later keys naming
   the same target are skipped without removal, and only returned admissions
   increment `dispatched` (`eval_broker.rs:108-139`). An unlimited numeric
   limit does not disable exact-target exclusion.
5. The production convergence owner passes its `active_targets` set into that
   drain, inserts each admitted target before running its complete evaluation,
   and removes the target only when it consumes the runtime result
   (`overdrive-control-plane/src/lib.rs:3353-3403,3424-3454`). The next loop
   turn sees the newly free capacity and immediately attempts another drain.
   Thus retained same-target owners execute sequentially while service
   continues. On shutdown, any not-yet-admitted key has the explicitly accepted
   pending-at-exit disposition; it is not falsely reported as completed.

The exact states exercised by the four tests are:

| Test state after routing | First target-exclusive drain | Retained pending | Why the old assertion fails |
|---|---|---|---|
| `interested_reconciler...`, `w1`: one key | `r-a` | 0 | This first one-owner case passes. |
| `interested_reconciler...`, `payments`: `r-a`, `r-b` at one target | `r-a` | `r-b` (queued 1) | A first admission result is compared with both submitted names. |
| `accepted_alloc_change...`: workload/service/svid at `workload/payments` | `workload-lifecycle` | service + svid (queued 2) | One admission is compared with all three routed owners. |
| `every_accepted_write...`: three owners for each of three distinct workload targets (queued 9) | the oldest owner for each target (three admissions) | two owners per target (queued 6) | Three admissions are treated as the complete nine-entry router submit set. |
| `fan_out_reaches...`: three owners at `workload/w1` | `workload-lifecycle` | service + svid (queued 2) | Pending convergent work is mislabeled as an infinite re-wake/busy loop. |

The tests themselves establish that routing completed before draining: their
`eventually` guards require `queued >= names.len()`, `queued >= 3`, or
`queued >= 9` (`interest_router.rs:394-400,861-865,909-914,956-959`). The
adjacent focused broker tests independently prove that same-target/different-
reconciler entries do not collapse and that a later empty-blocked-set round
admits the deferred key (`eval_broker_collapse.rs:111-119,383-430`). There is
therefore no loss, cancellation, or missing current consumer at the production
submission boundary.

Commit `083ef6e6` changed only the shared test `drain` helper from the old
drain-all call to one ADR-0102 admission call; it left these four bodies' old
complete-set/quiescence assumptions unchanged. The failures are deterministic
policy consequences, not a timing race. No production ordering defect is
proposed, so the repository's seeded-Sim prerequisite is not used to manufacture
one. The existing seed-283001 S-VLL owner evidence remains green; any future
claim that the real owner fails to progress a retained interested consumer must
first fail the seeded `overdrive-sim` liveness/convergence invariant through
that owner.

### Finding F5 — four interest-router tests confuse submitted consumers with one admission result

- **Severity:** Blocker
- **Dimension:** Test integrity, exact ADR-0102 target exclusion, and affected-suite gate
- **Locations:** `crates/overdrive-control-plane/tests/acceptance/interest_router.rs:350-357,378-414,852-978`
- **Production entry and full owner path:** Accepted allocation write → store
  LWW accept/broadcast → `spawn_interest_router` watch →
  `route_observation_row` complete interested-name loop → restricted broker
  submit → distinct pending `(ReconcilerName, TargetResource)` keys →
  convergence-owner target-exclusive admission/result consumption, as cited
  above.
- **Reachable exact state/order:** Each failing test waits until all intended
  keys are pending, then calls the shared `drain` helper exactly once. The helper
  now requests one ADR-0102 admission result, in which a target may occur only
  once. FIFO therefore returns the first owner for each distinct workload and
  retains later owners for later lease-release rounds. The table above records
  each exact pending/admitted state.
- **Reproducer:** Full affected nextest run
  `b8bd7a2e-b52e-4914-94e7-37c3334620fe` and focused nextest run
  `41fc5689-5e02-4a33-a904-7f5fe3418c51`; all four fail. The adjacent policy
  complement passes 2/2 in `ffa4327c-3f77-46ae-b327-41afe252ec9a`.
- **Consequence:** The full required gate is red and the assertions falsely
  diagnose retained same-target work as absent wakeups or non-quiescence.
  Changing production to empty all same-target keys in one result would violate
  ADR-0102 and permit overlapping owners of the same workload target/View.
- **Correct remediation owner:** Acceptance-test/oracle correction. The
  interest-router acceptance-test owner must update these four test bodies; the
  original 01-01 crafter may integrate and commit that bounded correction under
  same-step remediation. Broker, router, convergence-owner, and public API
  production code must remain unchanged.
- **Required bounded correction:** Preserve the accepted-write and complete
  interested-consumer contracts by first asserting the full pending count and
  no unintended cancellation, then model lease release through sequential
  admission rounds until every exact `(reconciler, target)` is observed once.
  Assert each round has distinct targets, exact queued/dispatched deltas, and
  eventual quiescence only after every retained key is admitted. Preserve each
  live test's required `CONTRACT_SHAPE` declaration. No accessor, compatibility
  drain, priority lane, production seam, or weakened consumer universe is
  authorized.
- **Disposition:** OPEN.

### Trybuild comparison and contract analysis

The worker failure was independently reproduced in isolation with:

`cargo xtask lima run -- cargo nextest run -p overdrive-worker -E 'test(compile_fail_fixtures)' --no-fail-fast`

Nextest run `a98d2f45-43dd-4295-aca0-3f54f1cd78e6` ran the one harness and
failed because one of its two fixtures mismatched;
`exec_driver_no_default.rs` still passed. For `exec_driver_missing_fs.rs`, the
expected and actual diagnostics agree on every semantic and positional fact:

- error `E0061`;
- the constructor takes three arguments but the fixture supplies two;
- the failure is at `exec_driver_missing_fs.rs:19:19`;
- argument 3 is the required `Arc<(dyn CgroupFs + 'static)>`;
- rustc's help supplies a third `Arc<dyn CgroupFs>` placeholder.

They differ on exactly one rendered source-note line at
`exec_driver_missing_fs.stderr:10`:

```text
EXPECTED: |     pub fn new(cgroup_root: PathBuf, clock: Arc<dyn Clock>, fs: Arc<dyn...
ACTUAL:   |     pub fn new(cgroup_root: PathBuf, clock: Arc<dyn Clock>, fs: Arc<dyn CgroupFs>) -> Self {
```

The production signature is exactly the actual full line at
`crates/overdrive-worker/src/driver.rs:285`. Git confirms that neither that
signature, the compile-fail source, nor its `.stderr` file changed between base
`7378fbd3` and implementation `083ef6e6`. The expected truncated line was
introduced independently in commit `373b335c9`; the pinned current Lima toolchain
reports `rustc 1.95.0`, and trybuild is pinned at `1.0.101`.

This is not compiler fallout from any ADR-0102 signature: ADR-0102 changes
broker/router/enqueue signatures only, and the `ExecDriver::new` three-argument
contract is unchanged. It is also not a compile-fail contract regression. The
fixture still fails for precisely the intended missing mandatory filesystem
dependency; only the snapshot's elided-versus-full rendering is stale for the
required current command.

### Finding F6 — stale trybuild rendering snapshot keeps the affected gate red

- **Severity:** Blocker for verification; no production defect
- **Dimension:** Compile-fail fixture integrity and affected-suite gate
- **Locations:** `crates/overdrive-worker/tests/compile_fail.rs:18-32`,
  `crates/overdrive-worker/tests/compile_fail/exec_driver_missing_fs.rs:16-20`,
  `crates/overdrive-worker/tests/compile_fail/exec_driver_missing_fs.stderr:10`,
  and unchanged production signature `crates/overdrive-worker/src/driver.rs:285`
- **Reachability:** This is a compile-time contract fixture, not a runtime
  control-plane path. The harness invokes the fixture directly at
  `compile_fail.rs:26`; rustc rejects the prohibited two-argument call exactly
  as intended.
- **Reproducer:** Full affected nextest run
  `b8bd7a2e-b52e-4914-94e7-37c3334620fe` and isolated worker run
  `a98d2f45-43dd-4295-aca0-3f54f1cd78e6` both show the same one-line mismatch.
- **Consequence:** Product/API behavior is preserved, but trybuild's exact-text
  comparison and therefore the required affected suite remain red.
- **Correct remediation owner:** Worker compile-fail expected-diagnostic
  fixture owner, not the interest router, evaluation broker, convergence owner,
  VM driver production implementation, or ADR-0102 crafter production scope.
- **Required bounded correction:** Replace only the stale truncated note line
  in `exec_driver_missing_fs.stderr` with the independently observed full
  signature line, after which rerun this isolated harness and the full affected
  gate. Do not change `ExecDriver::new`, make `fs` optional, alter the fixture's
  invalid call, or blindly bless unrelated diagnostics.
- **Disposition:** OPEN.

### Iteration 5 verification results

| Command | Result |
|---|---|
| Exact full affected `--no-fail-fast` command | FAIL — 2,207/2,212 passed; exactly F5's four tests plus F6's trybuild harness failed; nextest `b8bd7a2e-b52e-4914-94e7-37c3334620fe` |
| Focused four interest-router tests | FAIL — 0/4, all four reproduced; nextest `41fc5689-5e02-4a33-a904-7f5fe3418c51` |
| Focused same-target retention/determinism complement | PASS — 2/2; nextest `ffa4327c-3f77-46ae-b327-41afe252ec9a` |
| Focused worker compile-fail harness | FAIL — missing-fs diagnostic snapshot mismatch; no-default fixture passed; nextest `a98d2f45-43dd-4295-aca0-3f54f1cd78e6` |
| `cargo xtask lima run -- rustc --version --verbose` | PASS — `rustc 1.95.0 (59807616e 2026-04-14)`, `aarch64-unknown-linux-gnu` |
| `git diff --check` | PASS for the current worktree |

The full run is also focused preservation evidence: all 2,207 tests outside the
five enumerated failures passed, including the current F3/F4 corrections. The
orchestrator additionally reports the roadmap-focused S-VLL 12/12,
preservation 37/37, E10 2/2, workspace check, format, clippy, and diff gates
green on this same worktree state. Those green layers do not override the five
independently reproduced red tests.

### Iteration 5 cumulative disposition

| Finding | Status | Owner/action |
|---|---|---|
| F1 | CLOSED | Exact Contract Shape declaration remains present. |
| F2 | CLOSED | Exit observer remains cancellation-first. |
| F3 | REMEDIATED, UNCOMMITTED | Include the green reclamation test-oracle correction in the bounded remediation commit. |
| F4 | REMEDIATED, UNCOMMITTED | Include the green cadence test-oracle correction in the bounded remediation commit. |
| F5 | OPEN | Correct four interest-router admission oracles; production remains unchanged. |
| F6 | OPEN | Correct the one stale trybuild expected-diagnostic rendering line; production remains unchanged. |

No new production finding is proven. F5 is the same deterministic
submitted-versus-admitted distinction already established for F3/F4, now found
in four more mechanically transitioned direct consumers. F6 is unrelated stale
test-fixture text exposed by the broader gate, not ADR-0102 API fallout. Neither
finding authorizes an architecture, API, scheduling, persistence, or lifecycle
change.

### Iteration history

| Iteration | Reviewed state | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `083ef6e6` plus `f3c6250f` DES completion | **CHANGES_REQUESTED** | F1, F2 | Returned to original step crafter. |
| 2 | `78f189b7` | **APPROVED** | 0 new; F1/F2 closed | Superseded by the broader affected-suite failures. |
| 3 | `78f189b7` | **CHANGES_REQUESTED** | F3 | Reclamation test-oracle correction required. |
| 4 | `78f189b7` plus uncommitted F3 correction | **CHANGES_REQUESTED** | F4 | Cadence test-oracle correction required. |
| 5 | `78f189b7` plus uncommitted F3/F4 corrections | **CHANGES_REQUESTED** | F5, F6 | Four interest-router oracles and one trybuild snapshot line require test-only correction. |

### Final verdict after iteration 5

**CHANGES_REQUESTED.** Step `01-01` remains reopened because its exact affected
suite is red. Retain the green F3/F4 changes, correct F5's four stale
interest-router oracles using complete sequential admission evidence, and
correct F6's single expected-diagnostic rendering line. Production code, public
API, accepted target exclusion, and architecture must remain unchanged. Commit
the bounded test-only remediation, rerun the focused complements and exact full
`--no-fail-fast` affected gate, then return to this reviewer. Step `01-02` must
remain uncommitted until this step is again **APPROVED**.

---

## Iteration 6 — F3-F6 remediation re-review

### Metadata

| Field | Value |
|---|---|
| Review date | 2026-09-11 |
| Remediation commit | `1918489f2c5b1592276d1155e98c1012cffe95e1` |
| Remediation parent | `df90fe87410c19e4a6b9ed6e4e807ece8ed31467` |
| DES-log persistence commit | `f628b95f26767f822d0ff7550db81cbade202c72` |
| Cumulative step implementation | `083ef6e6ac6e29843e741df05868029385b42aea`, `78f189b743f1e55c32c1c00fc4024b52dd3d18e6`, and `1918489f2c5b1592276d1155e98c1012cffe95e1` |
| Base | `7378fbd32a6082e7637feac2fa5df9f25d6d3a47` |
| Reviewer | Same step-specific `nw-software-crafter-reviewer` |
| Reviewer model | GPT-5.6 Sol, high reasoning (repository-mandated selection) |
| Iteration | 6 |
| Verdict | **APPROVED** |

### Remediation scope, attribution, and DES evidence

Commit `1918489f` changes exactly the four artifacts authorized by iteration 5:

1. the two source-local reclamation test bodies in
   `crates/overdrive-control-plane/src/action_shim/reclamation.rs`;
2. the S-266-05 cadence acceptance test in
   `crates/overdrive-control-plane/tests/acceptance/cadence_resync.rs`;
3. the four interest-router acceptance bodies plus their shared admission
   helper in
   `crates/overdrive-control-plane/tests/acceptance/interest_router.rs`;
4. the one expected diagnostic line in
   `crates/overdrive-worker/tests/compile_fail/exec_driver_missing_fs.stderr`.

There is no production behavior, public API, wire shape, persistence shape,
roadmap, feature-delta, ADR, mutation configuration, or unrelated test change in
the remediation commit. The `action_shim/reclamation.rs` edits are wholly inside
its `#[cfg(test)]` module. A targeted cumulative comparison confirms no change
from `78f189b7` through `1918489f` in the production convergence owner,
`EvaluationBroker`, exit observer, or `ExecDriver` constructor.

The intervening parent commit `df90fe87` modifies only `lefthook.yml` to make an
existing affected-test gate non-fail-fast. It is not attributed as a step-01-01
remediation file and is excluded from the four-file remediation-scope
comparison. The remediation commit and DES persistence commit retain Marcus
Schack Abildskov as author and committer and each contains exactly
`Co-Authored-By: Codex <codex@openai.com>` plus `Step-Id: 01-01`.

Commit `f628b95f` changes only `execution-log.json`, appending chronological
successful `GREEN` and `COMMIT` events for `sid: 01-01` at
`2026-09-10T22:58:38Z` and `2026-09-10T22:59:04Z`. Those events follow the
existing completed step trace and accurately represent the remediation's green
and commit phases; the prior review failures supplied the remediation RED
evidence. Both commit diffs pass `git diff --check`.

### F3 disposition — RESOLVED

The authorised reclamation scenario now inspects the broker before admission
and proves the complete submission boundary:

- `queued = 3`, `dispatched = 0`, and `cancelled = 0` immediately after the
  real `execute_reclaim_allocation` path submits workload-, service-, and
  svid-lifecycle evaluations (`reclamation.rs:540-548`);
- three successive admission rounds each return exactly one evaluation for the
  shared `workload/<id>` target (`reclamation.rs:550-580`);
- queued depth falls exactly `3 → 2 → 1 → 0`, dispatched rises
  `0 → 1 → 2 → 3`, and cancelled remains zero;
- the accumulated set is exactly the three named lifecycle owners, all at the
  reclaimed workload target (`reclamation.rs:582-598`).

The terminal/non-terminal complement no longer uses a one-round drain as a
submission oracle. Its authorised non-terminal path proves three pending,
zero dispatched, zero cancelled entries, while the refused terminal path proves
zero across the same complete counter universe (`reclamation.rs:740-799`). The
production path remains the one established in iteration 3: the reclamation
executor submits all three distinct keys, the broker retains them, and the
convergence owner releases the shared target only after consuming each complete
runtime result.

**Disposition:** Closed. Complete distinct submissions, same-target retention,
sequential eventual admission, exact target/name universe, and terminal refusal
are all represented honestly without a production change.

### F4 disposition — RESOLVED

S-266-05 now carries the exact
`/// CONTRACT_SHAPE: bounded-change.` declaration and separates cadence due
events from broker admissions (`cadence_resync.rs:209-243`). Advancing the
declarative clock through seconds 1-60 counts X at its six 10-second boundaries
and Y at its two 30-second boundaries independently of which evaluation the
shared node target can admit.

The test then models the accepted owner boundary faithfully:

- at seconds 30 and 60, X and Y are both submitted, FIFO admits X, and Y
  remains pending (`cadence_resync.rs:246-275`);
- after the first modeled lease release, Y is admitted at second 31;
- after X at second 60 is modeled complete, an immediate second drain at the
  same broker time admits the retained Y, requiring no new cadence tick
  (`cadence_resync.rs:283-303`);
- the exact admission trajectory is six X plus two Y, with `queued = 0`,
  `dispatched = 8`, and `cancelled = 0` at completion
  (`cadence_resync.rs:305-313`).

This matches the production path already traced in iteration 4: each schedule
owns its due calculation; the broker preserves both distinct keys; and the
convergence owner immediately refills a released target/capacity slot.

**Disposition:** Closed. Cadence firing and target-exclusive admission are now
independent, complete oracles; no cadence, broker, or owner behavior changed.

### F5 disposition — RESOLVED

The new shared test helper first proves that the complete interested-consumer
set is pending and submission itself admitted nothing. It then performs
successive target-exclusive rounds, treating each new call as completion and
lease release of the prior round (`interest_router.rs:359-388`). On every
round, it asserts:

- nonempty progress while eligible keys remain;
- no duplicate target in the admission result;
- queued depth decreases by exactly the admission count;
- dispatched increases by exactly the admission count; and
- cancelled does not change, proving target exclusion retained rather than
  cancelled deferred keys (`interest_router.rs:389-420`).

At the complement, the helper requires `queued = 0`, exactly the expected
cumulative dispatched count, and the unchanged initial cancellation count
(`interest_router.rs:424-435`). Preserving rather than forcing the initial
cancelled count is correct: subscribe-first List-then-Watch may legitimately
deliver the same accepted row through both the LIST and WATCH legs and collapse
that exact duplicate key. Exact initial queued cardinality plus the later full
identity comparison proves that no distinct interested owner was lost.

Each former failure now adds its scenario-specific complete oracle:

- the 1/2/3-owner generated cases require one shared-target admission per owner
  and compare every exact reconciler name and target
  (`interest_router.rs:455-499`);
- the current three-consumer scenario admits three sequential rounds and
  compares exactly workload-lifecycle, service-lifecycle, and svid-lifecycle at
  `workload/payments` (`interest_router.rs:935-975`);
- the three-write scenario requires three rounds of three distinct workload
  targets and compares all nine `(reconciler, target)` pairs
  (`interest_router.rs:982-1031`);
- the fixpoint scenario admits all three retained shared-target consumers,
  then proves `queued == 0` remains stable without another accepted write
  (`interest_router.rs:1041-1080`).

The tests continue to drive the real accepted-write store fan-out and real
interest-router code. Their manual admission rounds match the production owner
path proven in iteration 5: `route_observation_row` submits all interested
names; the broker retains later same-target keys; `spawn_convergence_loop`
removes a target only after result consumption and immediately attempts refill.
No timing or production ordering defect is asserted, so no new seeded-Sim
finding is needed. The independently green seed-283001 S-VLL suite retains the
production-owner safety/liveness evidence.

**Disposition:** Closed. Complete fan-out submission, target exclusion,
pending retention, eventual admission, and the no-re-wake fixpoint are now
non-vacuously distinguished.

### F6 disposition — RESOLVED

The remediation changes only line 10 of
`exec_driver_missing_fs.stderr`, replacing the stale elided source note with
the exact current source signature:

```text
|     pub fn new(cgroup_root: PathBuf, clock: Arc<dyn Clock>, fs: Arc<dyn CgroupFs>) -> Self {
```

Every semantic portion of the expected diagnostic remains intact: rustc error
`E0061`, the invalid two-argument call at fixture line 19, the required third
`Arc<(dyn CgroupFs + 'static)>` argument, and the suggested third placeholder.
`ExecDriver::new` itself remains the exact mandatory three-argument constructor
at `crates/overdrive-worker/src/driver.rs:285`; no default, builder,
compatibility method, or optional filesystem dependency was introduced.

**Disposition:** Closed. The precise compile-fail contract still rejects the
prohibited call, and the expected rendering now matches the pinned Lima
toolchain without weakening production API shape.

### Cumulative contract and test-honesty check

| Contract | Iteration 6 result | Evidence |
|---|---|---|
| Exact ADR-0102 public API and unchanged `Evaluation`, `BrokerCounters`, wire, `run_convergence_tick` | PASS | Remediation contains no production/API edit; iteration-1 exact-shape audit remains valid. |
| Eight complete owners, target exclusion, FIFO age/order, immediate refill | PASS | Seeded S-VLL 12/12 plus F3-F5 sequential-admission complements. |
| Admission close, active-result drain, owner snapshot, observer ordering | PASS | S-VLL remains 12/12; F2's cancellation-first restoration remains unchanged. |
| Complete reclamation submissions and terminal refusal | PASS | F3 assertions cover full pending/admitted/counter/name/target universes. |
| Cadence due counts independent of admission | PASS | F4 counts 6 X/2 Y submissions and exact eight-entry admission trajectory. |
| Complete accepted-write interest fan-out and fixpoint | PASS | F5 covers 1/2/3 owners, the exact current-three set, nine cross-target entries, and stable quiescence. |
| Compile-fail mandatory filesystem dependency | PASS | F6 expected/actual match; invalid two-argument constructor remains rejected. |
| Contract Shape declarations | PASS | All changed acceptance bodies retain/add their exact bounded-change declaration. |
| S-VLL-13 View, re-enqueue, LWW/session, reclamation preservation | PASS | No production change; independent S-VLL run 12/12 and reported preservation selection 37/37. |
| Scope and attribution | PASS | Four authorized remediation files, one DES-only commit, exact trailers, no invented mechanism. |

No new finding is proven. Review of the changed assertions found no vacuous
membership-only oracle, hidden compatibility path, weakened target universe, or
production behavior change. The corrections test both the necessity of the
ADR-0102 policy and the remediation: reverting to drain-all would violate their
per-round target uniqueness, while losing a deferred key would violate their
exact final identities and counter deltas.

### Iteration 6 verification

| Command | Result |
|---|---|
| `cargo xtask lima run -- cargo nextest run --no-fail-fast -E 'rdeps(overdrive-cli) | rdeps(overdrive-init) | rdeps(overdrive-worker)'` | PASS — 2,212/2,212; nextest `aef88868-bf41-4996-8a55-5ef4058b5e3d` |
| Combined focused F3/F4/F5, two broker complements, and trybuild selection | PASS — 10/10; nextest `711a520b-253d-455c-b6a6-1751b4e5f669` |
| `cargo xtask lima run -- cargo nextest run -p overdrive-sim --features integration-tests --test vm_lifecycle_latency_283_spike --no-fail-fast` | PASS — 12/12; nextest `1d3f38ea-3981-46a5-a518-a4fed54524f9` |
| `cargo xtask lima run -- cargo check --workspace --all-targets --features integration-tests` | PASS |
| `cargo fmt --all -- --check` | PASS |
| `git diff --check df90fe87..f628b95f` | PASS |
| Targeted production-file cumulative no-diff check from `78f189b7..1918489f` | PASS — convergence owner, broker, exit observer, and `ExecDriver` unchanged |

The reported preservation selection 37/37, E10 2/2, workspace clippy, and the
more granular F3 2/2, F4 1/1, F5 4/4, broker 3/3, and trybuild 1/1 results are
consistent with the independently run full affected suite and focused checks.
No contradictory result was found.

### Iteration 6 cumulative disposition

| Finding | Status | Evidence |
|---|---|---|
| F1 | CLOSED | Exact transitioned property Contract Shape declaration remains present. |
| F2 | CLOSED | Exit observer remains cancellation-first; consumed-event retry remains intact. |
| F3 | CLOSED | Reclamation proves three pending intentions and three sequential admissions. |
| F4 | CLOSED | Cadence counts due submissions separately and admits both deferred Y evaluations. |
| F5 | CLOSED | Four interest-router scenarios prove complete fan-out across target-exclusive rounds and stable fixpoint. |
| F6 | CLOSED | Trybuild snapshot exactly preserves the mandatory third filesystem dependency diagnostic. |

### Iteration history

| Iteration | Reviewed state | Verdict | Findings | Disposition |
|---:|---|---|---:|---|
| 1 | `083ef6e6` plus `f3c6250f` DES completion | **CHANGES_REQUESTED** | F1, F2 | Returned to original step crafter. |
| 2 | `78f189b7` | **APPROVED** | 0 new; F1/F2 closed | Superseded by broader affected-suite failures. |
| 3 | `78f189b7` | **CHANGES_REQUESTED** | F3 | Reclamation oracle correction required. |
| 4 | `78f189b7` plus uncommitted F3 correction | **CHANGES_REQUESTED** | F4 | Cadence oracle correction required. |
| 5 | `78f189b7` plus uncommitted F3/F4 corrections | **CHANGES_REQUESTED** | F5, F6 | Interest-router oracles and trybuild snapshot required correction. |
| 6 | `1918489f` plus `f628b95f` DES persistence | **APPROVED** | 0 new; F3-F6 closed | All findings closed; terminating affected gate green. |

### Final verdict after iteration 6

**APPROVED.** No required remediation remains for step `01-01`. The exact
ADR-0102 API and production owner remain unchanged by the test-only F3-F6
correction; the corrected tests now distinguish complete submission from
target-exclusive admission, prove retained work eventually enters later rounds,
and preserve cadence, fan-out, fixpoint, reclamation, and compile-fail
contracts. The cumulative step, attribution, DES evidence, focused gates, seeded
S-VLL suite, workspace check, and complete 2,212-test affected suite pass. Step
`01-01` may advance.
